//! Detached import connections retain the workspace lease without holding the UI service lock.
use crate::{
    ActiveWorkspace, AppError, AppService, GrantPurpose, Result, SourceWorkerConfig,
    source_imports::metadata_signature, storage_identity,
};
use pkms_core::{JobStatus, Library};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Default)]
struct Schedule {
    running: bool,
    requested: bool,
}
static SCHEDULES: OnceLock<Mutex<HashMap<String, Arc<Mutex<Schedule>>>>> = OnceLock::new();
pub struct SourceJobsWorker {
    service: AppService,
    schedule: Arc<Mutex<Schedule>>,
    released: bool,
}
impl Drop for SourceJobsWorker {
    fn drop(&mut self) {
        if !self.released {
            self.schedule
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .running = false;
        }
    }
}
impl AppService {
    pub fn source_jobs_worker(&self) -> Result<Option<SourceJobsWorker>> {
        let Some(active) = self.active.as_ref() else {
            return Ok(None);
        };
        let schedule = {
            let mut schedules = SCHEDULES
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| AppError::AppBusy)?;
            schedules.retain(|_, schedule| Arc::strong_count(schedule) > 1);
            schedules
                .entry(active.storage_identity.clone())
                .or_default()
                .clone()
        };
        {
            let mut state = schedule.lock().map_err(|_| AppError::AppBusy)?;
            state.requested = true;
            if state.running {
                return Ok(None);
            }
            state.running = true;
        }
        let prepared = (|| {
            if storage_identity(Path::new(&active.entry.path))? != active.storage_identity {
                return Err(AppError::InputUnavailable);
            }
            let library = Library::open_existing(&active.entry.path)?;
            if library.info()?.workspace_id != active.entry.identity.workspace_id {
                return Err(AppError::InstanceMismatch);
            }
            let connection = Connection::open_with_flags(
                self.connection
                    .path()
                    .ok_or(AppError::UnsupportedDatabase)?,
                OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?;
            connection.busy_timeout(std::time::Duration::from_secs(5))?;
            connection.pragma_update(None, "foreign_keys", true)?;
            connection.pragma_update(None, "synchronous", "FULL")?;
            Ok(AppService {
                connection,
                app_directory: self.app_directory.clone(),
                active: Some(ActiveWorkspace {
                    library,
                    entry: active.entry.clone(),
                    storage_identity: active.storage_identity.clone(),
                    _lease: active._lease.clone(),
                    portable_schedule: active.portable_schedule.clone(),
                }),
                _lock: self._lock.clone(),
                workspace_owners: Default::default(),
            })
        })();
        match prepared {
            Ok(service) => Ok(Some(SourceJobsWorker {
                service,
                schedule,
                released: false,
            })),
            Err(error) => {
                schedule
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .running = false;
                Err(error)
            }
        }
    }
}
impl SourceJobsWorker {
    pub fn run(mut self, config: SourceWorkerConfig) -> Result<()> {
        if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let identity = self
            .service
            .active_workspace()
            .ok_or(AppError::InstanceMismatch)?
            .identity;
        // This schedule is exclusive across all connections for the physical workspace.
        self.service.library_mut(&identity)?.recover_source_jobs()?;
        loop {
            self.schedule
                .lock()
                .map_err(|_| AppError::AppBusy)?
                .requested = false;
            loop {
                let jobs = self.service.library(&identity)?.pending_source_jobs()?;
                if jobs.is_empty() {
                    break;
                }
                for id in jobs {
                    if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                        return Ok(());
                    }
                    let Some(lease) = self.service.library_mut(&identity)?.claim_source_job(&id)?
                    else {
                        continue;
                    };
                    if let Err(error) = self.execute(&identity, &lease, &config) {
                        if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                            return Ok(());
                        }
                        let current = self.service.library(&identity)?.source_job(&id)?;
                        if matches!(current.status, JobStatus::Canceled | JobStatus::Succeeded) {
                            continue;
                        }
                        let code = crate::error_codes::error_code(
                            &error,
                            crate::error_codes::ErrorDomain::SourceImport,
                        );
                        // Only transient worker/I/O failures retry, under the core's three-attempt cap.
                        let retryable =
                            matches!(code, "io_failed" | "worker_failed" | "worker_timeout");
                        self.service.library_mut(&identity)?.fail_source_job(
                            &id,
                            &lease.lease_token,
                            code,
                            retryable,
                        )?;
                    }
                }
            }
            let mut state = self.schedule.lock().map_err(|_| AppError::AppBusy)?;
            if state.requested {
                continue;
            }
            state.running = false;
            self.released = true;
            return Ok(());
        }
    }
    fn execute(
        &mut self,
        identity: &crate::WorkspaceIdentity,
        lease: &pkms_core::SourceJobLease,
        config: &SourceWorkerConfig,
    ) -> Result<()> {
        let id = &lease.job.id;
        let token = &lease.lease_token;
        // The parse environment is a precondition of the whole job, not a property of
        // the selected file: a component this installation cannot use stops the job
        // before the original is captured, so no half-finished job is left behind and
        // the record names the environment instead of a parse failure. The retry then
        // starts from the capture step again once the component is in place.
        if let Some(code) = config.missing_parse_component(lease.job.input.format) {
            return Err(AppError::InvalidInput(code));
        }
        let version = if let Some(version) = &lease.job.source_version_id {
            self.service.library(identity)?.source_version(version)?
        } else if lease.job.input.format == pkms_core::SourceFormat::Html {
            let url = lease
                .job
                .input
                .origin_url
                .as_deref()
                .ok_or(AppError::InvalidInput("webpage_invalid_url"))?;
            let mut captured = {
                let mut last_renew = std::time::Instant::now();
                let mut check = || {
                    if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(AppError::InvalidInput("worker_interrupted"));
                    }
                    let current = self.service.library(identity)?.source_job(id)?;
                    if current.status != JobStatus::Running
                        || current.cancel_requested
                        || current.attempt != lease.job.attempt
                    {
                        return Err(AppError::InvalidInput("canceled"));
                    }
                    if last_renew.elapsed() >= std::time::Duration::from_secs(20) {
                        self.service.library_mut(identity)?.source_job_checkpoint(
                            id,
                            token,
                            "capturing",
                        )?;
                        last_renew = std::time::Instant::now();
                    }
                    Ok(())
                };
                crate::webpage_capture::capture(url, &mut check)?
            };
            let mut check = |_stage: &str| -> pkms_core::Result<()> {
                if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(pkms_core::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "worker shutdown",
                    )));
                }
                Ok(())
            };
            self.service
                .library_mut(identity)?
                .capture_webpage_original(
                    id,
                    token,
                    &mut captured.file,
                    captured.capture,
                    &mut check,
                )?
        } else {
            let path = self
                .service
                .resolve_grant(&lease.job.input.input_token, &GrantPurpose::ImportSource)?;
            let expected = crate::source_imports::input_identity(&path)?;
            let mut file = std::fs::File::open(&path)?;
            let signature = metadata_signature(&file.metadata()?)?;
            let metadata_file = file.try_clone()?;
            self.service
                .resolve_grant(&lease.job.input.input_token, &GrantPurpose::ImportSource)?;
            let mut last_check = std::time::Instant::now() - std::time::Duration::from_secs(1);
            let mut input_changed = false;
            let mut check = |stage: &str| -> pkms_core::Result<()> {
                if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(pkms_core::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "worker shutdown",
                    )));
                }
                if stage != "register_original"
                    && last_check.elapsed() < std::time::Duration::from_millis(100)
                {
                    return Ok(());
                }
                last_check = std::time::Instant::now();
                let unchanged = crate::source_imports::input_identity(&path)
                    .is_ok_and(|actual| actual == expected)
                    && metadata_signature(&metadata_file.metadata()?)
                        .is_ok_and(|actual| actual == signature);
                if !unchanged {
                    input_changed = true;
                    return Err(pkms_core::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "source input changed",
                    )));
                }
                Ok(())
            };
            let captured = self
                .service
                .library_mut(identity)?
                .capture_source_original(id, token, &mut file, &mut check);
            if input_changed {
                return Err(AppError::InputUnavailable);
            }
            captured?
        };
        if self.service.library(identity)?.source_job(id)?.status == JobStatus::Succeeded {
            return Ok(());
        }
        self.service
            .library_mut(identity)?
            .source_job_checkpoint(id, token, "parsing")?;
        let (_, mut original) = self
            .service
            .library(identity)?
            .source_original_file(&version.id)?;
        let staging = tempfile::Builder::new()
            .prefix("pkms-source-worker-")
            .tempdir()?;
        let input_path = staging.path().join("original");
        let mut copied = std::fs::File::create_new(&input_path)?;
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                return Err(AppError::InvalidInput("worker_interrupted"));
            }
            if self
                .service
                .library(identity)?
                .source_job(id)?
                .cancel_requested
            {
                return Err(AppError::InvalidInput("canceled"));
            }
            let count = original.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > 256 * 1024 * 1024 {
                return Err(pkms_core::Error::IntegrityFailure.into());
            }
            hash.update(&buffer[..count]);
            copied.write_all(&buffer[..count])?;
        }
        if bytes != version.size {
            return Err(pkms_core::Error::IntegrityFailure.into());
        }
        if format!("{:x}", hash.finalize()) != version.blob_hash {
            return Err(pkms_core::Error::IntegrityFailure.into());
        }
        copied.sync_all()?;
        drop(copied);
        let mut last_renew = std::time::Instant::now();
        let mut checkpoint = || {
            if config.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                return Err(AppError::InvalidInput("worker_interrupted"));
            }
            let current = self.service.library(identity)?.source_job(id)?;
            if current.status != JobStatus::Running
                || current.cancel_requested
                || current.attempt != lease.job.attempt
            {
                return Err(AppError::InvalidInput("canceled"));
            }
            if last_renew.elapsed() >= std::time::Duration::from_secs(20) {
                self.service
                    .library_mut(identity)?
                    .source_job_checkpoint(id, token, "parsing")?;
                last_renew = std::time::Instant::now();
            }
            Ok(())
        };
        let parsed = crate::source_process::parse(
            config,
            lease,
            &version,
            &input_path,
            staging.path(),
            &mut checkpoint,
        )?;
        self.service
            .library_mut(identity)?
            .source_job_checkpoint(id, token, "indexing")?;
        self.service
            .library_mut(identity)?
            .publish_source_parse(id, token, parsed)?;
        Ok(())
    }
}
