//! Desktop application state and bootstrap orchestration, independent of Tauri.
mod background;
mod backup;
pub use background::{BackupOperationWorker, PortableJobsWorker};
mod bootstrap;
mod control;
mod diagnostics;
pub use diagnostics::{
    DiagnosticsPreview, parse_component_issues, set_diagnostic_webview_version,
    set_parse_component_issues,
};
mod error_codes;
mod inference_agent;
mod inference_answer_format;
mod inference_chat;
pub use inference_agent::{AgentProgress, AgentStage};
pub use inference_chat::{ChatProgress, ChatStage};
mod asr;
mod voice_summary;
pub use voice_summary::summarize as summarize_voice;
mod inference_answers;
pub mod inference_credentials;
mod inference_remote;
pub use asr::{AsrOutcome, AsrProbe, VoiceTranscription};
pub use inference_remote::{generation_channel_credential_binding, verify_remote_generation_model};
pub use pkms_core::AsrTestResult;
mod inference_model_policy;
mod inference_models;
pub use inference_models::{
    GenerationModels, InstalledGenerationModel, list_generation_models,
    list_remote_generation_models,
};
mod inference_process;
mod inference_queries;
mod inference_resources;
mod output_jobs;
mod sync;
pub use sync::*;
mod privacy;
mod research;
pub use research::{
    ExternalResearchOutcome, ExternalResearchPlan, ExternalResearchPreparation,
    fetch_external_research,
};
mod source_background;
mod source_directory;
mod source_imports;
pub use source_imports::{IMPORTABLE_EXTENSIONS, format_for_extension};
mod source_process;
pub use inference_answers::verify_generation_model;
mod inference_background;
pub use inference_background::InferenceJobsWorker;
pub use inference_process::InferenceConfig;
pub use inference_process::stop_inference_session;
pub use inference_queries::InferenceConnection;
mod webpage_capture;
mod worker_process;
pub use control::AppOperationControl;
pub use pkms_worker::OcrConfig;
mod ocr_support;
pub use ocr_support::{OcrStatus, ocr_status_from};
pub use source_background::SourceJobsWorker;
pub use source_directory::*;
pub use source_process::{ParseComponentIssue, SourceWorkerConfig};
mod grants;
mod ipc;
pub use ipc::*;
mod models;
mod state;
pub use models::*;

use pkms_core::{Library, WorkspaceLease};
use rusqlite::Connection;
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};
use thiserror::Error;

const APP_ID: i64 = 1_347_112_257;
const APP_SCHEMA: i64 = 3;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("application database failure: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("application state encoding failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Core(#[from] pkms_core::Error),
    #[error("application state is already open in another process")]
    AppBusy,
    #[error("unsupported or unrelated application state database")]
    UnsupportedDatabase,
    #[error("application state version conflict")]
    StateConflict,
    #[error("workspace instance does not match the active handle")]
    InstanceMismatch,
    #[error("application object not found")]
    NotFound,
    #[error("command ID was reused with a different request")]
    IdempotencyMismatch,
    #[error("input grant is expired, changed, or unavailable")]
    InputUnavailable,
    #[error("output destination already exists")]
    OutputExists,
    #[error("invalid application input: {0}")]
    InvalidInput(&'static str),
    #[error("sync failure: {0}")]
    Sync(String),
}
pub type Result<T> = std::result::Result<T, AppError>;

struct ActiveWorkspace {
    library: Library,
    entry: WorkspaceEntry,
    storage_identity: String,
    _lease: std::sync::Arc<WorkspaceLease>,
    portable_schedule: std::sync::Arc<std::sync::Mutex<background::WorkerSchedule>>,
}

pub struct AppService {
    connection: Connection,
    app_directory: std::path::PathBuf,
    active: Option<ActiveWorkspace>,
    _lock: std::sync::Arc<File>,
    workspace_owners: std::collections::HashMap<String, background::WorkspaceOwner>,
}

impl AppService {
    /// The native adapter supplies its app-data directory; knowledge workspaces
    /// never carry this catalog, UI state, or local instance namespace.
    pub fn open(app_directory: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(app_directory.as_ref())?;
        let directory = app_directory.as_ref().canonicalize()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("app-state.lock"))?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => AppError::AppBusy,
            TryLockError::Error(error) => AppError::Io(error),
        })?;
        let mut connection = Connection::open(directory.join("app-state.sqlite3"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app == 0 && version == 0 {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if count != 0 {
                return Err(AppError::UnsupportedDatabase);
            }
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute(
                "INSERT INTO app_preferences VALUES(1,1,?1)",
                [serde_json::to_string(&AppPreferences::default())?],
            )?;
            tx.commit()?;
        } else if app != APP_ID || !(1..=APP_SCHEMA).contains(&version) {
            return Err(AppError::UnsupportedDatabase);
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 1 {
            // The exclusive application lease excludes other adapters. Preserve an
            // independent SQLite snapshot before changing this machine's catalog.
            let backup_path =
                directory.join(format!("app-state-v1-{}.sqlite3", uuid::Uuid::new_v4()));
            File::create_new(&backup_path)?;
            let mut backup_connection = Connection::open(&backup_path)?;
            {
                let backup = rusqlite::backup::Backup::new(&connection, &mut backup_connection)?;
                backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
            }
            backup_connection
                .close()
                .map_err(|(_, error)| AppError::Database(error))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&backup_path)?
                .sync_all()?;
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0002_attachment_commands.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 2 {
            let backup_path =
                directory.join(format!("app-state-v2-{}.sqlite3", uuid::Uuid::new_v4()));
            File::create_new(&backup_path)?;
            let mut copy = Connection::open(&backup_path)?;
            {
                let backup = rusqlite::backup::Backup::new(&connection, &mut copy)?;
                backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
            }
            copy.close().map_err(|(_, e)| AppError::Database(e))?;
            OpenOptions::new()
                .write(true)
                .open(&backup_path)?
                .sync_all()?;
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0003_backup_operations.sql"))?;
            tx.commit()?;
        }
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(AppError::UnsupportedDatabase);
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        // A running bootstrap has no live worker after acquiring this exclusive
        // application lease. Resume only through its validated checkpoint.
        connection.execute("UPDATE app_operations SET status='queued',stage='resume_pending' WHERE status='running'",[])?;
        let service = Self {
            connection,
            app_directory: directory.clone(),
            active: None,
            _lock: std::sync::Arc::new(lock),
            workspace_owners: Default::default(),
        };
        service.preferences()?;
        Ok(service)
    }

    /// Opaque-but-verifiable scope for an app-level list cursor. Only this
    /// machine''s app-state database produces it, so a page from another adapter
    /// instance is refused instead of silently continuing there.
    pub(crate) fn app_cursor_scope(connection: &Connection, tag: u32) -> Result<String> {
        use sha2::{Digest, Sha256};
        let path = connection.path().ok_or(AppError::UnsupportedDatabase)?;
        let mut hasher = Sha256::new();
        hasher.update(tag.to_string());
        hasher.update([0]);
        hasher.update(path.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Decode an app-level list cursor (`[scope, updated_at, id]`) and refuse a
    /// page that was produced by another app state or is malformed.
    pub(crate) fn decode_app_cursor(
        connection: &Connection,
        tag: u32,
        cursor: Option<&str>,
    ) -> Result<(String, Option<(String, String)>)> {
        let scope = Self::app_cursor_scope(connection, tag)?;
        let Some(value) = cursor else {
            return Ok((scope, None));
        };
        let parsed: (String, String, String) = serde_json::from_str(value)
            .map_err(|_| AppError::InvalidInput("invalid list cursor"))?;
        if parsed.0 != scope || chrono::DateTime::parse_from_rfc3339(&parsed.1).is_err() {
            return Err(AppError::InvalidInput(
                "list cursor does not match app state",
            ));
        }
        uuid::Uuid::parse_str(&parsed.2)
            .map_err(|_| AppError::InvalidInput("invalid list cursor"))?;
        Ok((scope, Some((parsed.1, parsed.2))))
    }
    pub fn active_workspace(&self) -> Option<WorkspaceEntry> {
        self.active
            .as_ref()
            .map(|workspace| workspace.entry.clone())
    }

    pub fn library(&self, identity: &WorkspaceIdentity) -> Result<&Library> {
        let active = self.active.as_ref().ok_or(AppError::InstanceMismatch)?;
        if &active.entry.identity != identity {
            return Err(AppError::InstanceMismatch);
        }
        Ok(&active.library)
    }

    pub fn library_mut(&mut self, identity: &WorkspaceIdentity) -> Result<&mut Library> {
        let active = self.active.as_mut().ok_or(AppError::InstanceMismatch)?;
        if &active.entry.identity != identity {
            return Err(AppError::InstanceMismatch);
        }
        Ok(&mut active.library)
    }
    pub fn workspace_path(&self, identity: &WorkspaceIdentity) -> Result<std::path::PathBuf> {
        let active = self.active.as_ref().ok_or(AppError::InstanceMismatch)?;
        if &active.entry.identity != identity {
            return Err(AppError::InstanceMismatch);
        }
        Ok(std::path::PathBuf::from(&active.entry.path))
    }

    pub fn close_workspace(&mut self) {
        self.active = None;
    }
}

fn uuid(value: &str) -> Result<()> {
    uuid::Uuid::parse_str(value).map_err(|_| AppError::InvalidInput("identity must be a UUID"))?;
    Ok(())
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(AppError::InvalidInput("path cannot be represented as text"))
}

fn physical_identity(path: &Path) -> Result<String> {
    let id = match file_id::get_file_id(path)? {
        file_id::FileId::Inode {
            device_id,
            inode_number,
        } => format!("inode:{device_id}:{inode_number}"),
        file_id::FileId::LowRes {
            volume_serial_number,
            file_index,
        } => format!("win-low:{volume_serial_number}:{file_index}"),
        file_id::FileId::HighRes {
            volume_serial_number,
            file_id,
        } => format!("win-high:{volume_serial_number}:{file_id}"),
    };
    let created = std::fs::metadata(path)?
        .created()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos().to_string());
    Ok(serde_json::to_string(&(id, created))?)
}

/// What a recent workspace needs before it can be opened again: the folder and
/// its `library.sqlite3` must exist *and* still carry the identity recorded when
/// the workspace was last opened. A folder reused by another library is reported
/// instead of silently opening notes that belong elsewhere.
pub fn workspace_availability(path: &Path, expected: &str) -> WorkspaceAvailability {
    // Controlled QA fault injection: a normal user cannot be given a file
    // that passes the metadata checks and then fails the file-id read on this
    // machine. Release builds never read this variable.
    #[cfg(debug_assertions)]
    if std::env::var_os("PKMS_TEST_WORKSPACE_UNREADABLE")
        .is_some_and(|value| Path::new(&value) == path)
    {
        return WorkspaceAvailability::Unreadable;
    }
    if !path.is_dir() || !path.join("library.sqlite3").is_file() {
        return WorkspaceAvailability::Missing;
    }
    classify_workspace(storage_identity(path), expected)
}

fn classify_workspace(identity: Result<String>, expected: &str) -> WorkspaceAvailability {
    match identity {
        Ok(identity) if identity == expected => WorkspaceAvailability::Available,
        Ok(_) => WorkspaceAvailability::ForeignLibrary,
        Err(_) => WorkspaceAvailability::Unreadable,
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    #[test]
    fn an_identity_read_failure_is_named_unreadable() {
        assert_eq!(
            classify_workspace(Err(crate::AppError::InvalidInput("test")), "expected"),
            WorkspaceAvailability::Unreadable
        );
    }
}

fn storage_identity(workspace: &Path) -> Result<String> {
    Ok(serde_json::to_string(&(
        physical_identity(workspace)?,
        physical_identity(&workspace.join("library.sqlite3"))?,
    ))?)
}

fn path_from_text(path: &str) -> PathBuf {
    PathBuf::from(path)
}

mod inference_embeddings;
