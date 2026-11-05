use crate::runtime::{DesktopRuntime, emit, lock, resume_operations, resume_portable_jobs};
use pkms_app::{
    ApiError, ApiErrorCode, DesktopData, DesktopEvent, DesktopReply, DesktopRequest, GrantPurpose,
    ResourceGrant, WorkspaceIdentity,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

/// Debug-only, file-driven fault seam for real WebView regression runs.
///
/// The file named by `PKMS_TEST_FAULT_CONFIG` is re-read for every request so a
/// test can arm and disarm a fault without restarting the app. Release builds
/// never include this path.
#[cfg(debug_assertions)]
#[derive(serde::Deserialize)]
struct DebugTestFault {
    kind: String,
    mode: String,
    #[serde(default)]
    note_id: Option<String>,
    #[serde(default = "default_fault_delay_ms")]
    delay_ms: u64,
}

#[cfg(debug_assertions)]
fn default_fault_delay_ms() -> u64 {
    30_000
}

#[cfg(debug_assertions)]
async fn apply_debug_test_fault(
    command: &pkms_app::DesktopCommand,
    request_id: &str,
) -> Result<(), ApiError> {
    let Some(path) = std::env::var_os("PKMS_TEST_FAULT_CONFIG") else {
        return Ok(());
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(fault) = serde_json::from_str::<DebugTestFault>(&text) else {
        return Ok(());
    };
    let value = serde_json::to_value(command).unwrap_or(serde_json::Value::Null);
    if value.get("kind").and_then(|kind| kind.as_str()) != Some(fault.kind.as_str()) {
        return Ok(());
    }
    if let Some(note_id) = fault.note_id.as_deref()
        && value.pointer("/payload/note_id").and_then(|id| id.as_str()) != Some(note_id)
    {
        return Ok(());
    }
    match fault.mode.as_str() {
        "delay" => {
            let delay = fault.delay_ms.clamp(1, 120_000);
            tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            })
            .await
            .map_err(|_| ApiError::new(ApiErrorCode::Internal, request_id))?;
            Ok(())
        }
        "error" => Err(ApiError {
            code: ApiErrorCode::StorageBusy,
            message: "injected test fault".to_string(),
            request_id: request_id.to_string(),
            retryable: true,
            details: None,
        }),
        _ => Ok(()),
    }
}

/// Debug-only: lets a real WebView test return a workspace or bibliography
/// resource grant without the native dialog. Production builds never include
/// this path.
#[cfg(debug_assertions)]
fn debug_resource_grant(purpose: &GrantPurpose) -> Option<std::path::PathBuf> {
    let expected = match purpose {
        GrantPurpose::CreateWorkspace => "create_workspace",
        GrantPurpose::OpenWorkspace => "open_workspace",
        GrantPurpose::ImportBibliography => "import_bibliography",
        GrantPurpose::ImportFile => "import_file",
        GrantPurpose::ExportFile => "export_file",
        GrantPurpose::BackupInput => "backup_input",
        GrantPurpose::HandoffInput => "handoff_input",
        GrantPurpose::HandoffOutput => "handoff_output",
        GrantPurpose::RestoreWorkspace => "restore_workspace",
        _ => return None,
    };
    let config = std::env::var_os("PKMS_TEST_FAULT_CONFIG")?;
    let text = std::fs::read_to_string(config).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value.get("grant_purpose")?.as_str()? != expected {
        return None;
    }
    Some(std::path::PathBuf::from(value.get("grant_path")?.as_str()?))
}
#[tauri::command]
pub async fn voice_capture_audio(
    app: AppHandle,
    window: WebviewWindow,
    identity: pkms_app::WorkspaceIdentity,
    capture_id: String,
) -> Result<tauri::ipc::Response, ApiError> {
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let runtime = crate::runtime::current(&app, &request_id)?;
    let lock_id = request_id.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let service = lock(&runtime.service, &lock_id)?;
        let library = service
            .library(&identity)
            .map_err(|error| ApiError::from_app(error, &lock_id))?;
        library
            .voice_capture_audio(&capture_id)
            .map_err(|error| ApiError::from_app(error.into(), &lock_id))
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, &request_id))??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn voice_capture_export(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
    capture_id: String,
) -> Result<bool, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    if runtime
        .dialog_open
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, &request_id));
    }
    let lease = DialogLease(runtime.dialog_open.clone());
    let worker_id = request_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let (capture, bytes, english) = {
            let service = lock(&runtime.service, &worker_id)?;
            let library = service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &worker_id))?;
            let capture = library
                .voice_capture(&capture_id)
                .map_err(|error| ApiError::from_app(error.into(), &worker_id))?;
            let bytes = library
                .voice_capture_audio(&capture_id)
                .map_err(|error| ApiError::from_app(error.into(), &worker_id))?;
            let english = service
                .preferences()
                .map_err(|error| ApiError::from_app(error, &worker_id))?
                .preferences
                .language
                == pkms_app::Language::English;
            (capture, bytes, english)
        };
        let extension = match capture.audio_media_type.as_str() {
            "audio/webm" => "webm",
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/ogg" => "ogg",
            "audio/mpeg" => "mp3",
            "audio/mp4" => "m4a",
            "audio/flac" => "flac",
            _ => "audio",
        };
        let choose = || -> Result<Option<std::path::PathBuf>, ApiError> {
            #[cfg(debug_assertions)]
            if let Some(path) = debug_resource_grant(&GrantPurpose::ExportFile) {
                return Ok(Some(path));
            }
            let short_id: String = capture_id.chars().take(8).collect();
            app.dialog()
                .file()
                .set_parent(&window)
                .set_title(if english {
                    "Save original audio"
                } else {
                    "保存原音频"
                })
                .set_file_name(format!("spanseek-recording-{short_id}.{extension}"))
                .add_filter(
                    if english {
                        "Original audio"
                    } else {
                        "原始音频"
                    },
                    &[extension],
                )
                .blocking_save_file()
                .map(|path| {
                    path.into_path()
                        .map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, &worker_id))
                })
                .transpose()
        };
        let Some(destination) = choose()? else {
            return Ok(false);
        };
        // A dialog may outlive a workspace transition. Recheck its scope before
        // committing an export; never hold the service lock while a user chooses.
        {
            let service = lock(&runtime.service, &worker_id)?;
            service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &worker_id))?;
        }
        persist_audio_export(&destination, &bytes)
            .map_err(|error| ApiError::from_app(error.into(), &worker_id))?;
        Ok(true)
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, &request_id))?
}

fn persist_audio_export(destination: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = destination
        .parent()
        .ok_or_else(|| std::io::Error::other("missing export directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod audio_export_tests {
    use super::persist_audio_export;

    #[test]
    fn replaces_an_export_with_exact_original_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("recording.wav");
        std::fs::write(&destination, b"previous export").unwrap();
        let original = b"RIFF\0\x01original audio bytes";
        persist_audio_export(&destination, original).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_export_preserves_the_destination_and_removes_its_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing-directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep.txt"), b"keep").unwrap();
        assert!(persist_audio_export(&destination, b"audio").is_err());
        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[tauri::command]
pub async fn desktop_request(
    app: AppHandle,
    window: WebviewWindow,
    request: serde_json::Value,
) -> Result<DesktopReply, ApiError> {
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let request = DesktopRequest::parse(request)?;
    let closes_workspace = matches!(request.command, pkms_app::DesktopCommand::WorkspaceClose);
    let updates_preferences = matches!(
        request.command,
        pkms_app::DesktopCommand::PreferencesUpdate { .. }
    );
    let request_id = request.request_id.clone();
    #[cfg(debug_assertions)]
    apply_debug_test_fault(&request.command, &request_id).await?;

    let runtime = crate::runtime::current(&app, &request_id)?;
    let source_operation = matches!(
        request.command,
        pkms_app::DesktopCommand::SourcesImport { .. }
            | pkms_app::DesktopCommand::SourcesImportWebpage { .. }
            | pkms_app::DesktopCommand::SourceJobRetry { .. }
    );
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if request.command.is_operation_control() {
            let active = lock(&runtime.active, &request.request_id)?.clone();
            return lock(&runtime.control, &request.request_id)?.dispatch(request, active.as_ref());
        }
        if matches!(request.command, pkms_app::DesktopCommand::SyncStatus) {
            // Read local configuration under the lock, then release it before the
            // health request. A slow or unreachable endpoint must not block note
            // lists, bootstrap, or any other desktop command behind this probe.
            let probe = lock(&runtime.service, &request.request_id)?
                .sync_status_probe()
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::SyncStatus(probe.resolve()),
            });
        }
        if let pkms_app::DesktopCommand::SyncWatch { timeout_ms } = &request.command {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let probe = lock(&runtime.service, &request.request_id)?
                .sync_watch_probe(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let result = match probe {
                Some(probe) => probe
                    .resolve(u64::from(*timeout_ms))
                    .map_err(|error| ApiError::from_app(error, &request.request_id))?,
                None => pkms_app::SyncWatchResult {
                    configured: false,
                    changed: false,
                    conflict_count: 0,
                },
            };
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::SyncWatch(result),
            });
        }
        if let pkms_app::DesktopCommand::VoiceCaptureTranscribe {
            capture_id,
            base_row_version,
        } = &request.command
        {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            // Prepare under the lock; the ASR upload must not block other commands.
            let transcription = lock(&runtime.service, &request.request_id)?
                .begin_voice_transcription(identity, capture_id, *base_row_version)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let outcome = transcription.run();
            let capture = lock(&runtime.service, &request.request_id)?
                .finish_voice_transcription(identity, &transcription, outcome)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let body = match capture.status {
                pkms_core::VoiceCaptureStatus::Ready => {
                    format!(
                        "转写完成：{} 秒语音，可检查后存入收件箱。",
                        capture.duration_ms / 1_000
                    )
                }
                pkms_core::VoiceCaptureStatus::Failed => {
                    "转写失败，原始音频已保留，可在语音速记中重试。".to_owned()
                }
                _ => "录音已保存。".to_owned(),
            };
            let _ = worker_app
                .notification()
                .builder()
                .title("语音速记")
                .body(body)
                .show();
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::VoiceCapture(capture),
            });
        }
        if let pkms_app::DesktopCommand::VoiceCaptureSummarize { request: summary } =
            &request.command
        {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let (profile, transcript) = lock(&runtime.service, &request.request_id)?
                .voice_summary_input(identity, &summary.capture_id)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let text = pkms_app::summarize_voice(identity, &profile, &transcript, summary.mode)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::VoiceSummary(pkms_core::VoiceSummary { text }),
            });
        }
        if matches!(request.command, pkms_app::DesktopCommand::VoiceSyncImport) {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let imported = lock(&runtime.service, &request.request_id)?
                .voice_sync_import(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::VoiceSyncImport(imported),
            });
        }
        if matches!(request.command, pkms_app::DesktopCommand::AsrSettingsTest) {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let probe = lock(&runtime.service, &request.request_id)?
                .asr_probe(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let latency_ms = probe
                .run()
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::AsrTestResult(pkms_app::AsrTestResult {
                    latency_ms,
                    accepted: true,
                }),
            });
        }
        if let pkms_app::DesktopCommand::ResearchRunExternal {
            command_id,
            task_id,
            base_row_version,
        } = &request.command
        {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let (plan, path) = {
                let service = lock(&runtime.service, &request.request_id)?;
                let plan = match service
                    .prepare_external_research(identity, command_id, task_id, *base_row_version)
                    .map_err(|error| ApiError::from_app(error, &request.request_id))?
                {
                    pkms_app::ExternalResearchPreparation::Replayed(result) => {
                        return Ok(DesktopReply {
                            request_id: request.request_id,
                            data: DesktopData::ResearchCommit(*result),
                        });
                    }
                    pkms_app::ExternalResearchPreparation::Plan(plan) => plan,
                };
                (
                    plan,
                    service
                        .workspace_path(identity)
                        .map_err(|error| ApiError::from_app(error, &request.request_id))?,
                )
            };
            // The service lock is free during the network request, so a cancel
            // command can commit while this fetch is still in flight.
            let checker = pkms_core::Library::open_existing(&path)
                .map_err(|error| ApiError::from_app(error.into(), &request.request_id))?;
            let outcome =
                pkms_app::fetch_external_research(&plan, &checker, &runtime.source_shutdown);
            let data = lock(&runtime.service, &request.request_id)?
                .record_external_research(identity, command_id, plan, outcome)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::ResearchCommit(data),
            });
        }
        if matches!(
            &request.command,
            pkms_app::DesktopCommand::GenerationModelVerify { .. }
                | pkms_app::DesktopCommand::GenerationModelsList { .. }
                | pkms_app::DesktopCommand::GenerationChannelModelsList { .. }
        ) {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            lock(&runtime.service, &request.request_id)?
                .library(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let data = match &request.command {
                pkms_app::DesktopCommand::GenerationModelVerify { profile } => {
                    if profile.provider == "ollama" {
                        pkms_app::verify_generation_model(profile, &runtime.source_shutdown)
                    } else {
                        pkms_app::verify_remote_generation_model(
                            identity,
                            profile,
                            &runtime.source_shutdown,
                        )
                    }
                    .map_err(|error| ApiError::from_app(error, &request.request_id))?;
                    DesktopData::Done
                }
                pkms_app::DesktopCommand::GenerationModelsList { endpoint } => {
                    DesktopData::GenerationModels(
                        pkms_app::list_generation_models(endpoint, &runtime.source_shutdown)
                            .map_err(|error| ApiError::from_app(error, &request.request_id))?,
                    )
                }
                pkms_app::DesktopCommand::GenerationChannelModelsList {
                    channel_id,
                    row_version,
                } => {
                    let channel = lock(&runtime.service, &request.request_id)?
                        .generation_channel_at_version(identity, channel_id, *row_version)
                        .map_err(|error| ApiError::from_app(error, &request.request_id))?;
                    // The service guard was dropped with the snapshot expression above.
                    // Discovery may paginate and must not hold the global AppService lock.
                    let catalog = pkms_app::list_remote_generation_models(
                        identity,
                        &channel,
                        &runtime.source_shutdown,
                    )
                    .map_err(|error| ApiError::from_app(error, &request.request_id))?;
                    lock(&runtime.service, &request.request_id)?
                        .generation_channel_at_version(identity, channel_id, *row_version)
                        .map_err(|error| ApiError::from_app(error, &request.request_id))?;
                    DesktopData::RemoteGenerationModels(catalog)
                }
                _ => unreachable!(),
            };
            lock(&runtime.service, &request.request_id)?
                .library(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data,
            });
        }
        if let pkms_app::DesktopCommand::SearchQuery(query) = &request.command {
            let identity = request.identity.as_ref().ok_or_else(|| {
                ApiError::new(ApiErrorCode::InstanceMismatch, &request.request_id)
            })?;
            let mut connection = lock(&runtime.service, &request.request_id)?
                .inference_connection(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            let config = crate::inference::config(&worker_app, &runtime)?;
            let result = connection
                .search(&config, query.clone())
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            // A completed old-workspace query must never enter the newly selected workspace.
            lock(&runtime.service, &request.request_id)?
                .library(identity)
                .map_err(|error| ApiError::from_app(error, &request.request_id))?;
            return Ok(DesktopReply {
                request_id: request.request_id,
                data: DesktopData::SearchResults(result),
            });
        }
        let inference_work = matches!(
            request.command,
            pkms_app::DesktopCommand::AnswersStart { .. }
                | pkms_app::DesktopCommand::AnswersAgentStart { .. }
                | pkms_app::DesktopCommand::ChatRunsStart { .. }
                | pkms_app::DesktopCommand::KnowledgeTasksStart { .. }
                | pkms_app::DesktopCommand::IndexRebuildStart { .. }
                | pkms_app::DesktopCommand::WorkspaceSelect { .. }
        );
        let mut service = lock(&runtime.service, &request.request_id)?;
        let reply = service.dispatch(request)?;
        runtime.refresh_identity(&service, &reply.request_id)?;
        if inference_work
            || matches!(
                reply.data,
                DesktopData::NoteCommit(_)
                    | DesktopData::MetadataCommit(_)
                    | DesktopData::TagManagementCommit(_)
                    | DesktopData::EmbeddingProfileMutation(_)
            )
        {
            crate::inference::resume(&worker_app, &service);
        }
        // Capture this workspace before another command can switch it.
        if source_operation {
            crate::runtime::resume_source_jobs(&worker_app, &service);
        } else if matches!(reply.data, DesktopData::JobAccepted(_)) {
            resume_portable_jobs(&service);
        }
        Ok(reply)
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, &request_id))??;
    if updates_preferences && let DesktopData::Preferences(preferences) = &result.data {
        crate::appearance::apply_preferences(&app, preferences.clone());
    }
    if let DesktopData::Bootstrap(_) = &result.data {
        resume_operations(app.clone());
    }
    if closes_workspace {
        let active = lock(&app.state::<DesktopRuntime>().active, &request_id)?.clone();
        emit(&app, DesktopEvent::WorkspaceChanged { active });
    }
    Ok(result)
}

pub(crate) struct DialogLease(pub(crate) Arc<AtomicBool>);
impl Drop for DialogLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Debug-only: grants a source file without the native dialog for real WebView
/// tests. Release builds never include this path.
#[cfg(debug_assertions)]
fn debug_source_grants() -> Option<Vec<std::path::PathBuf>> {
    let config = std::env::var_os("PKMS_TEST_FAULT_CONFIG")?;
    let text = std::fs::read_to_string(config).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value.get("grant_purpose")?.as_str()? != "import_source" {
        return None;
    }
    if let Some(paths) = value.get("grant_paths").and_then(|value| value.as_array()) {
        let paths: Vec<_> = paths
            .iter()
            .filter_map(|value| value.as_str())
            .map(std::path::PathBuf::from)
            .collect();
        return (!paths.is_empty()).then_some(paths);
    }
    Some(vec![std::path::PathBuf::from(
        value.get("grant_path")?.as_str()?,
    )])
}

#[tauri::command]
pub async fn choose_source_files(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
) -> Result<Vec<ResourceGrant>, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    if runtime
        .dialog_open
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, &request_id));
    }
    let lease = DialogLease(runtime.dialog_open.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        #[cfg(debug_assertions)]
        if let Some(paths) = debug_source_grants() {
            let mut service = lock(&runtime.service, &request_id)?;
            service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &request_id))?;
            let mut grants = Vec::with_capacity(paths.len());
            for path in paths {
                grants.push(
                    service
                        .grant_path(GrantPurpose::ImportSource, path)
                        .map_err(|error| ApiError::from_app(error, &request_id))?,
                );
            }
            return Ok(grants);
        }
        let english = {
            let service = lock(&runtime.service, &request_id)?;
            service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &request_id))?;
            service
                .preferences()
                .map_err(|error| ApiError::from_app(error, &request_id))?
                .preferences
                .language
                == pkms_app::Language::English
        };
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(if english {
                "Import sources"
            } else {
                "导入资料"
            })
            .add_filter(
                if english { "Documents" } else { "文档" },
                pkms_app::IMPORTABLE_EXTENSIONS,
            )
            .blocking_pick_files()
            .unwrap_or_default();
        if selected.len() > 100 {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
        }
        let mut service = lock(&runtime.service, &request_id)?;
        service
            .library(&identity)
            .map_err(|error| ApiError::from_app(error, &request_id))?;
        selected
            .into_iter()
            .map(|path| {
                let path = path
                    .into_path()
                    .map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, &request_id))?;
                service
                    .grant_path(GrantPurpose::ImportSource, path)
                    .map_err(|error| ApiError::from_app(error, &request_id))
            })
            .collect()
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

/// Grants a whole folder read-only. The scan itself happens in the app layer,
/// which re-derives every file path from this granted root.
#[tauri::command]
pub async fn choose_source_directory(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
) -> Result<Option<ResourceGrant>, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    if runtime
        .dialog_open
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, &request_id));
    }
    let lease = DialogLease(runtime.dialog_open.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;

        let english = {
            let service = lock(&runtime.service, &request_id)?;
            service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &request_id))?;
            service
                .preferences()
                .map_err(|error| ApiError::from_app(error, &request_id))?
                .preferences
                .language
                == pkms_app::Language::English
        };
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(if english {
                "Import a folder"
            } else {
                "导入文件夹"
            })
            .blocking_pick_folder();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, &request_id))?;
        let mut service = lock(&runtime.service, &request_id)?;
        if service.active_workspace().map(|entry| entry.identity) != Some(identity) {
            return Err(ApiError::new(ApiErrorCode::InstanceMismatch, &request_id));
        }
        service
            .grant_path(GrantPurpose::ImportDirectory, path)
            .map(Some)
            .map_err(|error| ApiError::from_app(error, &request_id))
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}
#[tauri::command]
pub async fn choose_resource(
    app: AppHandle,
    window: WebviewWindow,
    purpose: GrantPurpose,
    identity: Option<WorkspaceIdentity>,
) -> Result<Option<ResourceGrant>, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    if runtime
        .dialog_open
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, &request_id));
    }
    let lease = DialogLease(runtime.dialog_open.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;

        let english = {
            let service = lock(&runtime.service, &request_id)?;
            if service.active_workspace().map(|entry| entry.identity) != identity {
                return Err(ApiError::new(ApiErrorCode::InstanceMismatch, &request_id));
            }
            service
                .preferences()
                .map_err(|e| ApiError::from_app(e, &request_id))?
                .preferences
                .language
                == pkms_app::Language::English
        };
        #[cfg(debug_assertions)]
        if let Some(path) = debug_resource_grant(&purpose) {
            let grant = lock(&runtime.service, &request_id)?
                .grant_path(purpose.clone(), path)
                .map_err(|error| ApiError::from_app(error, &request_id))?;
            return Ok(Some(grant));
        }
        let dialog = app.dialog().file().set_parent(&window);
        let selected = match purpose {
            GrantPurpose::OpenWorkspace => dialog
                .set_title(if english {
                    "Open knowledge library folder"
                } else {
                    "打开知识库文件夹"
                })
                .blocking_pick_folder(),
            GrantPurpose::CreateWorkspace => dialog
                .set_title(if english {
                    "Choose a location and name for the new library"
                } else {
                    "选择新知识库的位置与名称"
                })
                .set_file_name(if english {
                    "My library"
                } else {
                    "我的知识库"
                })
                .blocking_save_file(),
            GrantPurpose::ImportFile => dialog
                .set_title(if english {
                    "Insert image"
                } else {
                    "插入图片"
                })
                .add_filter(
                    if english { "Images" } else { "图片" },
                    &["png", "jpg", "jpeg", "webp"],
                )
                .blocking_pick_file(),
            GrantPurpose::ImportSource => dialog
                .set_title(if english {
                    "Import a source"
                } else {
                    "导入资料"
                })
                .add_filter(
                    if english { "Documents" } else { "文档" },
                    pkms_app::IMPORTABLE_EXTENSIONS,
                )
                .blocking_pick_file(),
            GrantPurpose::ImportBibliography => dialog
                .set_title(if english {
                    "Import bibliography"
                } else {
                    "导入参考文献"
                })
                .add_filter(
                    if english {
                        "BibTeX, CSL JSON or CSL style"
                    } else {
                        "BibTeX、CSL JSON 或 CSL 样式"
                    },
                    &["bib", "json", "csl"],
                )
                .blocking_pick_file(),
            GrantPurpose::BackupInput => dialog
                .set_title(if english {
                    "Choose a complete spanseek backup"
                } else {
                    "选择 spanseek 完整备份包"
                })
                .add_filter(
                    if english {
                        "spanseek backup"
                    } else {
                        "spanseek 备份"
                    },
                    &["pkmsbackup", "zip"],
                )
                .blocking_pick_file(),
            GrantPurpose::HandoffInput => dialog
                .set_title(if english {
                    "Import a resume handoff"
                } else {
                    "导入续读包"
                })
                .add_filter("JSON", &["json"])
                .blocking_pick_file(),
            GrantPurpose::HandoffOutput => dialog
                .set_title(if english {
                    "Export a resume handoff"
                } else {
                    "导出续读包"
                })
                .add_filter("JSON", &["json"])
                .set_file_name("spanseek-resume-handoff.json")
                .blocking_save_file(),
            GrantPurpose::RestoreWorkspace => dialog
                .set_title(if english {
                    "Choose a location and new folder name for the restored library"
                } else {
                    "选择恢复知识库的位置与新文件夹名称"
                })
                .set_file_name(if english {
                    "Restored library"
                } else {
                    "恢复的知识库"
                })
                .blocking_save_file(),
            GrantPurpose::ExportFile => dialog
                .set_title(if english {
                    "Save export or complete backup"
                } else {
                    "保存导出或完整备份包"
                })
                .add_filter(if english { "ZIP archive" } else { "ZIP 包" }, &["zip"])
                .set_file_name("spanseek-export.zip")
                .blocking_save_file(),
            // Folder import pairs its own picker with a single-directory grant,
            // so this generic resource chooser never issues one.
            GrantPurpose::ImportDirectory => {
                return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
            }
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, &request_id))?;
        let mut service = lock(&runtime.service, &request_id)?;
        if service.active_workspace().map(|entry| entry.identity) != identity {
            return Err(ApiError::new(ApiErrorCode::InstanceMismatch, &request_id));
        }
        service
            .grant_path(purpose, path)
            .map(Some)
            .map_err(|error| ApiError::from_app(error, &request_id))
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

/// Retries the application startup after the user cleared the reason it failed.
///
/// Startup failures are remembered on the handle so every later request can
/// explain them. That memory has to be dropped here, otherwise the "reconnect"
/// button on the welcome screen would only read the same failure back.
#[tauri::command]
pub async fn retry_startup(app: AppHandle) -> Result<(), ApiError> {
    let directory = app
        .try_state::<crate::runtime::AppDataDirectory>()
        .map(|state| state.0.clone())
        .ok_or_else(|| ApiError::new(ApiErrorCode::Internal, ""))?;
    if let Ok(mut failure) = app.state::<crate::runtime::StartupFailure>().0.lock() {
        *failure = None;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || crate::runtime::initialize(&handle, directory))
        .await
        .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}
/// Local OCR capability, reported to diagnostics. The engine and language packs
/// are installation facts; nothing here reads an imported document.
#[tauri::command]
pub fn ocr_status(app: AppHandle) -> pkms_app::OcrStatus {
    let config = crate::runtime::ocr_config(&app);
    let languages = config
        .as_ref()
        .map(|config| traineddata_languages(&config.tessdata_dir))
        .unwrap_or_default();
    pkms_app::ocr_status_from(config, languages)
}

fn traineddata_languages(directory: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut languages: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".traineddata").map(str::to_owned)
        })
        .collect();
    languages.sort();
    languages
}
