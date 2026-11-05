use pkms_app::{
    ApiError, ApiErrorCode, AppOperationControl, AppService, DesktopEvent, OperationKind,
    WorkspaceIdentity,
};
use std::sync::{Arc, Mutex, MutexGuard, atomic::AtomicBool};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub struct DesktopRuntime {
    pub service: Arc<Mutex<AppService>>,
    pub control: Arc<Mutex<AppOperationControl>>,
    pub active: Arc<Mutex<Option<WorkspaceIdentity>>>,
    pub dialog_open: Arc<AtomicBool>,
    operation_schedule: Arc<Mutex<OperationSchedule>>,
    pub source_shutdown: Arc<AtomicBool>,
    source_threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    pub inference_threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

#[derive(Default)]
struct OperationSchedule {
    running: bool,
    requested: bool,
}
impl OperationSchedule {
    fn request(&mut self) -> bool {
        self.requested = true;
        if self.running {
            false
        } else {
            self.running = true;
            true
        }
    }
    fn finish_pass(&mut self) -> bool {
        if self.requested {
            true
        } else {
            self.running = false;
            false
        }
    }
}
struct OperationGuard {
    schedule: Arc<Mutex<OperationSchedule>>,
    active: bool,
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        if self.active {
            self.schedule
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .running = false;
        }
    }
}

#[derive(Default)]
pub struct StartupFailure(pub Mutex<Option<ApiError>>);
/// Where the application keeps its own state database. The retry command needs
/// the same location again after a startup failure, when no runtime exists yet.
#[derive(Clone)]
pub struct AppDataDirectory(pub std::path::PathBuf);

/// Opens the app-state database and installs the runtime on a healthy handle.
///
/// Startup runs this once. `retry_startup` runs it again after the user fixed
/// whatever blocked the first attempt (a locked, missing or read-only
/// location), so the "reconnect" button on the welcome screen can recover
/// instead of re-reading a remembered failure forever.
pub fn initialize(app: &AppHandle, directory: std::path::PathBuf) -> Result<(), ApiError> {
    let initialized = AppService::open(directory).and_then(|service| {
        let preferences = service.preferences()?;
        DesktopRuntime::new(service).map(|runtime| (runtime, preferences))
    });
    match initialized {
        Ok((runtime, preferences)) => {
            // Publish what this installation can use for parsing before the interface
            // asks for its state, so a missing sidecar is named instead of discovered
            // as a failed import.
            if let Ok(config) = source_config(app, &runtime) {
                record_parse_components(&config);
            }
            // A successful retry installs the runtime for the first time; a
            // repeated call must not panic on an already managed state.
            if app.try_state::<DesktopRuntime>().is_none() {
                app.manage(runtime);
            }
            crate::inference::start_scheduler(app);
            crate::appearance::apply_preferences(app, preferences);
            emit(app, DesktopEvent::WorkspaceChanged { active: None });
            resume_operations(app.clone());
            Ok(())
        }
        Err(error) => {
            let error = ApiError::from_app(error, "");
            if let Ok(mut failure) = app.state::<StartupFailure>().0.lock() {
                *failure = Some(error.clone());
            }
            emit(
                app,
                DesktopEvent::OperationFailed {
                    operation_id: String::new(),
                    error: error.clone(),
                },
            );
            Err(error)
        }
    }
}

pub fn current(app: &AppHandle, request_id: &str) -> Result<DesktopRuntime, ApiError> {
    if let Some(runtime) = app.try_state::<DesktopRuntime>() {
        return Ok(runtime.inner().clone());
    }
    if let Some(failure) = app.try_state::<StartupFailure>()
        && let Some(error) = lock(&failure.0, request_id)?.clone()
    {
        return Err(error);
    }
    Err(ApiError::new(ApiErrorCode::BackendStarting, request_id))
}

pub fn lock<'a, T>(mutex: &'a Mutex<T>, request_id: &str) -> Result<MutexGuard<'a, T>, ApiError> {
    mutex
        .lock()
        .map_err(|_| ApiError::new(ApiErrorCode::Internal, request_id))
}

impl DesktopRuntime {
    pub fn new(service: AppService) -> Result<Self, pkms_app::AppError> {
        let control = service.operation_control()?;
        Ok(Self {
            service: Arc::new(Mutex::new(service)),
            control: Arc::new(Mutex::new(control)),
            active: Arc::new(Mutex::new(None)),
            dialog_open: Arc::new(AtomicBool::new(false)),
            operation_schedule: Arc::new(Mutex::new(OperationSchedule::default())),
            source_shutdown: Arc::new(AtomicBool::new(false)),
            source_threads: Arc::new(Mutex::new(Vec::new())),
            inference_threads: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn refresh_identity(&self, service: &AppService, request_id: &str) -> Result<(), ApiError> {
        *lock(&self.active, request_id)? = service.active_workspace().map(|entry| entry.identity);
        Ok(())
    }

    pub fn shutdown_sources(&self) {
        self.source_shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        pkms_app::stop_inference_session();
        // Source checkpoints stop capture and kill/reap the active child before process exit.
        let threads = std::mem::take(
            &mut *self
                .source_threads
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for thread in threads {
            let _ = thread.join();
        }
        let inference = std::mem::take(
            &mut *self
                .inference_threads
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for thread in inference {
            let _ = thread.join();
        }
    }
}

pub fn emit(app: &AppHandle, event: DesktopEvent) {
    // Events only invalidate caches. Polling/re-query still recovers if the window closed.
    if app.emit_to("main", "pkms:event", event).is_err() {
        eprintln!("spanseek: event delivery failed");
    }
}

pub fn resume_operations(app: AppHandle) {
    let runtime = app.state::<DesktopRuntime>().inner().clone();
    let requested = lock(&runtime.operation_schedule, "").map(|mut schedule| schedule.request());
    match requested {
        Ok(false) => return,
        Ok(true) => {}
        Err(error) => {
            emit(
                &app,
                DesktopEvent::OperationFailed {
                    operation_id: String::new(),
                    error,
                },
            );
            return;
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = OperationGuard {
            schedule: runtime.operation_schedule.clone(),
            active: true,
        };
        let result = (|| {
            loop {
                lock(&runtime.operation_schedule, "")?.requested = false;
                let pending = lock(&runtime.service, "")?
                    .pending_operation_ids()
                    .map_err(|e| ApiError::from_app(e, ""))?;
                for operation_id in pending {
                    let (result, changes_workspace) = {
                        let mut service = lock(&runtime.service, &operation_id)?;
                        let operation = service
                            .operation(&operation_id)
                            .map_err(|e| ApiError::from_app(e, &operation_id))?;
                        if matches!(
                            operation.kind,
                            OperationKind::VerifyBackup | OperationKind::RestoreBackup
                        ) {
                            let worker = service
                                .backup_operation_worker(&operation_id)
                                .map_err(|e| ApiError::from_app(e, &operation_id))?;
                            drop(service);
                            (worker.run(), false)
                        } else {
                            let result = service.execute_operation(&operation_id);
                            runtime.refresh_identity(&service, &operation_id)?;
                            resume_portable_jobs(&service);
                            resume_source_jobs(&app, &service);
                            crate::inference::resume(&app, &service);
                            (result, true)
                        }
                    };
                    match result {
                        Ok(_) => emit(&app, DesktopEvent::OperationChanged { operation_id }),
                        Err(error) => emit(
                            &app,
                            DesktopEvent::OperationFailed {
                                error: ApiError::from_app(error, &operation_id),
                                operation_id,
                            },
                        ),
                    }
                    if changes_workspace {
                        let active = lock(&runtime.active, "")?.clone();
                        emit(&app, DesktopEvent::WorkspaceChanged { active });
                    }
                }
                let mut schedule = lock(&runtime.operation_schedule, "")?;
                if !schedule.finish_pass() {
                    guard.active = false;
                    break;
                }
            }
            Ok::<_, ApiError>(())
        })();
        if let Err(error) = result {
            emit(
                &app,
                DesktopEvent::OperationFailed {
                    operation_id: String::new(),
                    error,
                },
            );
        }
    });
}

pub fn resume_portable_jobs(service: &AppService) {
    // Capture ownership while the caller still holds the active-service mutex.
    // The worker itself never takes it, including if the user switches before
    // spawn_blocking starts running the queued work.
    match service.portable_jobs_worker() {
        Ok(Some(worker)) => {
            tauri::async_runtime::spawn_blocking(move || {
                if worker.run().is_err() {
                    portable_worker_failed();
                }
            });
        }
        Ok(None) => {}
        Err(_) => portable_worker_failed(),
    }
}

/// Bundled helper path. The pre-rename file name stays as a documented fallback
/// for an installation directory that was only partially updated; a directory
/// carrying both names always uses the current one.
pub(crate) fn bundled_sidecar(
    directory: &std::path::Path,
    name: &str,
    legacy: &str,
) -> std::path::PathBuf {
    let preferred = directory.join(name);
    if preferred.is_file() {
        return preferred;
    }
    let fallback = directory.join(legacy);
    // Only a directory that still carries the pre-rename helper is accepted;
    // otherwise report the current name so a failure names what is missing.
    if fallback.is_file() {
        fallback
    } else {
        preferred
    }
}

fn source_config(
    app: &AppHandle,
    runtime: &DesktopRuntime,
) -> Result<pkms_app::SourceWorkerConfig, ApiError> {
    let (executable, pdfium) = if cfg!(debug_assertions) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        (
            std::env::var_os("PKMS_SOURCE_WORKER_EXE")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    bundled_sidecar(
                        &root.join("target/debug"),
                        "spanseek-worker.exe",
                        "pkms-worker.exe",
                    )
                }),
            std::env::var_os("PKMS_PDFIUM_DLL")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    root.join(".local-archive/dependencies/pdfium-7881/bin/pdfium.dll")
                }),
        )
    } else {
        let executable = std::env::current_exe()
            .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?;
        let directory = executable
            .parent()
            .ok_or_else(|| ApiError::new(ApiErrorCode::InputUnavailable, ""))?;
        (
            bundled_sidecar(directory, "spanseek-worker.exe", "pkms-worker.exe"),
            app.path()
                .resource_dir()
                .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
                .join("pdfium/pdfium.dll"),
        )
    };
    Ok(pkms_app::SourceWorkerConfig {
        executable,
        pdfium_library_path: Some(pdfium),
        ocr: ocr_config(app),
        shutdown: runtime.source_shutdown.clone(),
    })
}

/// OCR is optional. The engine and language data stay outside the database and
/// are chosen by the installation, never by an imported document.
pub(crate) fn ocr_config(app: &AppHandle) -> Option<pkms_app::OcrConfig> {
    let resource_root = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("tesseract"));
    let debug_root = cfg!(debug_assertions).then(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.local-archive/dependencies/tesseract-5.4.0.20240606")
    });
    resolve_ocr_config(
        std::env::var_os("PKMS_TESSERACT_EXE").map(std::path::PathBuf::from),
        std::env::var_os("PKMS_TESSDATA_DIR").map(std::path::PathBuf::from),
        resource_root,
        debug_root,
        Some(std::path::PathBuf::from(r"C:\Program Files\Tesseract-OCR")),
    )
}

fn resolve_ocr_config(
    explicit_executable: Option<std::path::PathBuf>,
    explicit_tessdata: Option<std::path::PathBuf>,
    resource_root: Option<std::path::PathBuf>,
    debug_root: Option<std::path::PathBuf>,
    system_root: Option<std::path::PathBuf>,
) -> Option<pkms_app::OcrConfig> {
    // An explicit executable is authoritative: never mix it with a different
    // installation's language data or silently fall back to another engine.
    if let Some(executable) = explicit_executable {
        let tessdata = explicit_tessdata.or_else(|| {
            executable
                .parent()
                .map(|directory| directory.join("tessdata"))
        })?;
        return ocr_candidate(executable, tessdata);
    }
    for root in [resource_root, debug_root, system_root]
        .into_iter()
        .flatten()
    {
        let tessdata = explicit_tessdata
            .clone()
            .unwrap_or_else(|| root.join("tessdata"));
        if let Some(config) = ocr_candidate(root.join("tesseract.exe"), tessdata) {
            return Some(config);
        }
    }
    None
}

/// Tauri's resource directory may carry the Windows verbatim prefix. Tesseract
/// rejects that spelling for `--tessdata-dir`, so hand the process a normal
/// Win32 path without changing where the files are verified from.
fn tesseract_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return std::path::PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(rest);
        }
    }
    path
}

fn ocr_candidate(
    executable: std::path::PathBuf,
    tessdata_dir: std::path::PathBuf,
) -> Option<pkms_app::OcrConfig> {
    let executable = tesseract_path(executable);
    let tessdata_dir = tesseract_path(tessdata_dir);
    if !executable.is_file() || !tessdata_dir.is_dir() {
        return None;
    }
    // The worker requests this exact pair. Requiring both packs prevents an
    // English-only system install from looking available and then failing at
    // the first scanned page.
    if !["chi_sim.traineddata", "eng.traineddata"]
        .iter()
        .all(|name| tessdata_dir.join(name).is_file())
    {
        return None;
    }
    Some(pkms_app::OcrConfig {
        executable,
        tessdata_dir,
        languages: "chi_sim+eng".into(),
        dpi: 300,
    })
}

pub fn resume_source_jobs(app: &AppHandle, service: &AppService) {
    let Some(runtime) = app.try_state::<DesktopRuntime>() else {
        return;
    };
    if runtime
        .source_shutdown
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }
    let config = match source_config(app, &runtime) {
        Ok(config) => config,
        Err(_) => {
            source_worker_failed();
            return;
        }
    };
    record_parse_components(&config);
    match service.source_jobs_worker() {
        Ok(Some(worker)) => {
            let mut threads = runtime
                .source_threads
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            // Serialize registration against shutdown so no child can escape the final join.
            if runtime
                .source_shutdown
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return;
            }
            let mut pending = Vec::new();
            for thread in threads.drain(..) {
                if thread.is_finished() {
                    let _ = thread.join();
                } else {
                    pending.push(thread);
                }
            }
            let source_app = app.clone();
            let source_identity = service.active_workspace().map(|entry| entry.identity);
            pending.push(std::thread::spawn(move || {
                if worker.run(config).is_err() {
                    source_worker_failed();
                }
                if let Some(runtime) = source_app.try_state::<DesktopRuntime>()
                    && let Ok(service) = runtime.service.lock()
                    && service.active_workspace().map(|entry| entry.identity) == source_identity
                {
                    crate::inference::resume(&source_app, &service);
                }
            }));
            *threads = pending;
        }
        Ok(None) => {}
        Err(_) => source_worker_failed(),
    }
}

/// Publishes what this installation can use for parsing. The interface names an
/// unusable component (with its path) before a job fails on it; the exported
/// diagnostics bundle never sees these paths.
fn record_parse_components(config: &pkms_app::SourceWorkerConfig) {
    pkms_app::set_parse_component_issues(config.parse_component_issues());
}

fn source_worker_failed() {
    eprintln!("spanseek source worker stopped; durable task state is retained.");
}

fn portable_worker_failed() {
    // The durable task retains its failure or resumable checkpoint.
    // No fabricated operation ID, filesystem path, or user content.
    eprintln!("spanseek portable task worker stopped; durable task state is retained.");
}

#[cfg(test)]
mod tests {
    use super::{OperationSchedule, resolve_ocr_config, tesseract_path};
    use std::path::{Path, PathBuf};

    #[test]
    fn operation_wakeups_during_a_pass_are_coalesced_without_being_lost() {
        let mut schedule = OperationSchedule::default();
        assert!(schedule.request());
        schedule.requested = false;
        assert!(!schedule.request());
        assert!(!schedule.request());
        assert!(schedule.finish_pass());
        schedule.requested = false;
        assert!(!schedule.finish_pass());
        assert!(schedule.request());
    }

    fn ocr_root(root: &Path, languages: &[&str]) -> PathBuf {
        let tessdata = root.join("tessdata");
        std::fs::create_dir_all(&tessdata).expect("tessdata");
        std::fs::write(root.join("tesseract.exe"), b"fixture").expect("engine");
        for language in languages {
            std::fs::write(tessdata.join(format!("{language}.traineddata")), b"fixture")
                .expect("language");
        }
        root.to_path_buf()
    }

    #[test]
    fn ocr_resolution_prefers_the_bundled_pair_and_fails_closed_on_partial_languages() {
        let root = std::env::temp_dir().join(format!(
            "spanseek-ocr-resolution-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let resource = ocr_root(&root.join("resource"), &["chi_sim", "eng"]);
        let debug = ocr_root(&root.join("debug"), &["chi_sim", "eng"]);
        let system = ocr_root(&root.join("system"), &["chi_sim", "eng"]);
        let selected = resolve_ocr_config(
            None,
            None,
            Some(resource.clone()),
            Some(debug.clone()),
            Some(system.clone()),
        )
        .expect("bundled OCR");
        assert_eq!(selected.executable, resource.join("tesseract.exe"));
        assert_eq!(selected.tessdata_dir, resource.join("tessdata"));
        assert_eq!(selected.languages, "chi_sim+eng");

        std::fs::remove_file(resource.join("tessdata/chi_sim.traineddata"))
            .expect("remove language");
        let selected = resolve_ocr_config(
            None,
            None,
            Some(resource),
            Some(debug.clone()),
            Some(system),
        )
        .expect("debug OCR fallback");
        assert_eq!(selected.executable, debug.join("tesseract.exe"));

        let partial = ocr_root(&root.join("partial"), &["eng"]);
        assert!(
            resolve_ocr_config(
                Some(partial.join("tesseract.exe")),
                None,
                Some(debug),
                None,
                None,
            )
            .is_none()
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn tesseract_paths_drop_only_the_windows_verbatim_prefix() {
        assert_eq!(
            tesseract_path(PathBuf::from(r"\\?\E:\spanseek\tesseract.exe")),
            PathBuf::from(r"E:\spanseek\tesseract.exe")
        );
        assert_eq!(
            tesseract_path(PathBuf::from(r"\\?\UNC\server\share\tesseract.exe")),
            PathBuf::from(r"\\server\share\tesseract.exe")
        );
        assert_eq!(
            tesseract_path(PathBuf::from(r"E:\spanseek\tesseract.exe")),
            PathBuf::from(r"E:\spanseek\tesseract.exe")
        );
    }
}

#[cfg(test)]
mod sidecar_tests {
    use super::bundled_sidecar;

    #[test]
    fn bundled_helper_uses_the_current_name_and_keeps_the_legacy_fallback() {
        let directory = std::env::temp_dir().join(format!(
            "spanseek-sidecar-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let current = directory.join("spanseek-worker.exe");
        let legacy = directory.join("pkms-worker.exe");
        // Neither file yet: the caller must see the current name in its error.
        assert_eq!(
            bundled_sidecar(&directory, "spanseek-worker.exe", "pkms-worker.exe"),
            current
        );
        std::fs::write(&legacy, b"legacy").expect("legacy file");
        assert_eq!(
            bundled_sidecar(&directory, "spanseek-worker.exe", "pkms-worker.exe"),
            legacy
        );
        std::fs::write(&current, b"current").expect("current file");
        assert_eq!(
            bundled_sidecar(&directory, "spanseek-worker.exe", "pkms-worker.exe"),
            current
        );
        std::fs::remove_dir_all(&directory).expect("cleanup");
    }
}
