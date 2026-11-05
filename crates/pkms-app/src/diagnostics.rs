use crate::source_process::ParseComponentIssue;
use crate::{AppError, AppService, Result, WorkspaceIdentity};
use pkms_core::{
    DiagnosticCategory, DiagnosticError, DiagnosticSnapshot, DiagnosticVersions, JobAccepted,
    PortableJobInput,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DiagnosticsPreview {
    pub categories: Vec<DiagnosticCategory>,
    pub snapshot: DiagnosticSnapshot,
    pub fingerprint: String,
}
static WEBVIEW_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// Parse components this installation cannot use, as last observed by the native
/// layer that owns the sidecar location. This report names local paths and is read
/// only by the local interface, so it deliberately stays out of
/// `DiagnosticSnapshot`: an exported bundle must never carry local paths.
static PARSE_COMPONENTS: std::sync::OnceLock<std::sync::Mutex<Vec<ParseComponentIssue>>> =
    std::sync::OnceLock::new();

/// Records the parse component report for the local diagnostics surface.
pub fn set_parse_component_issues(issues: Vec<ParseComponentIssue>) {
    *PARSE_COMPONENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = issues;
}

/// The last parse component report, empty until the native layer has reported one.
pub fn parse_component_issues() -> Vec<ParseComponentIssue> {
    PARSE_COMPONENTS
        .get()
        .map(|issues| {
            issues
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        })
        .unwrap_or_default()
}
/// Native supplies only the actual numeric runtime version, never machine/user paths.
pub fn set_diagnostic_webview_version(version: &str) {
    if !version.is_empty()
        && version.len() <= 80
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        let _ = WEBVIEW_VERSION.set(version.into());
    }
}
fn categories(mut value: Vec<DiagnosticCategory>) -> Result<Vec<DiagnosticCategory>> {
    if value.is_empty() || value.len() > 3 {
        return Err(AppError::InvalidInput("invalid_diagnostic_categories"));
    }
    value.sort();
    if value.windows(2).any(|items| items[0] == items[1]) {
        return Err(AppError::InvalidInput("invalid_diagnostic_categories"));
    }
    Ok(value)
}
fn fingerprint(snapshot: &DiagnosticSnapshot) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(snapshot)?)
    ))
}
fn safe_error(mut item: DiagnosticError) -> Option<DiagnosticError> {
    item.id = uuid::Uuid::parse_str(&item.id).ok()?.to_string();
    item.occurred_at = chrono::DateTime::parse_from_rfc3339(&item.occurred_at)
        .ok()?
        .to_rfc3339();
    if !matches!(
        item.component.as_str(),
        "source" | "portable" | "index" | "answer"
    ) {
        return None;
    }
    item.stage = match item.stage.as_str() {
        "queued" | "running" | "failed" | "canceled" | "timed_out" | "capture" | "parsing"
        | "publishing" | "preparing" | "copy_original" | "parse" | "publish" | "generating"
        | "retrieving" => item.stage,
        _ => "unknown_state".into(),
    };
    // The vocabulary is owned by `pkms_core::Library::diagnostic_errors`, which
    // already collapsed anything unknown to `unknown_error` before this point.
    // Repeating the table here would only create a second source of truth that
    // drifts, so the interface checks the shape of the value it exports instead.
    item.code = if is_exportable_code(&item.code) {
        item.code
    } else {
        "unknown_error".into()
    };
    Some(item)
}
/// Accepts only a short lowercase slug, so the exported archive can never carry
/// an arbitrary stored message even if a caller skips the core classification.
fn is_exportable_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
impl AppService {
    pub fn diagnostics_preview(
        &self,
        identity: &WorkspaceIdentity,
        selected: Vec<DiagnosticCategory>,
    ) -> Result<DiagnosticsPreview> {
        let categories = categories(selected)?;
        let library = self.library(identity)?;
        let versions = if categories.contains(&DiagnosticCategory::Versions) {
            Some(DiagnosticVersions {
                application: env!("CARGO_PKG_VERSION").into(),
                app_schema: crate::APP_SCHEMA as u32,
                knowledge_schema: library.info()?.database_schema,
                platform: std::env::consts::OS.into(),
                architecture: std::env::consts::ARCH.into(),
                webview_version: WEBVIEW_VERSION.get().cloned(),
                source_worker_protocol: pkms_worker::PROTOCOL_VERSION,
                inference_protocol: pkms_inference::protocol::PROTOCOL_VERSION,
                embedding_model_revision: pkms_inference::profile::MODEL_REVISION.into(),
            })
        } else {
            None
        };
        let statistics = if categories.contains(&DiagnosticCategory::Statistics) {
            Some(library.workspace_statistics()?)
        } else {
            None
        };
        let errors = if categories.contains(&DiagnosticCategory::Errors) {
            Some(
                library
                    .diagnostic_errors(100)?
                    .into_iter()
                    .filter_map(safe_error)
                    .collect(),
            )
        } else {
            None
        };
        let snapshot = DiagnosticSnapshot {
            versions,
            statistics,
            errors,
        };
        Ok(DiagnosticsPreview {
            fingerprint: fingerprint(&snapshot)?,
            snapshot,
            categories,
        })
    }
    pub fn export_diagnostics(
        &mut self,
        identity: &WorkspaceIdentity,
        command: &str,
        token: &str,
        selected: Vec<DiagnosticCategory>,
        expected: &str,
    ) -> Result<JobAccepted> {
        self.library(identity)?;
        crate::uuid(command)?;
        crate::uuid(token)?;
        let selected = categories(selected)?;
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::InvalidInput("invalid_diagnostic_fingerprint"));
        }
        let old:Option<(String,String)>=self.connection.query_row("SELECT token,input_json FROM workspace_job_outputs WHERE local_instance_id=?1 AND workspace_id=?2 AND command_id=?3",params![identity.local_instance_id,identity.workspace_id,command],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let input = if let Some((bound, json)) = old {
            let input: PortableJobInput = serde_json::from_str(&json)?;
            match &input {
                PortableJobInput::ExportDiagnostics {
                    categories,
                    snapshot,
                } if bound == token
                    && categories == &selected
                    && fingerprint(snapshot)? == expected =>
                {
                    input
                }
                _ => return Err(AppError::IdempotencyMismatch),
            }
        } else {
            let preview = self.diagnostics_preview(identity, selected)?;
            if preview.fingerprint != expected {
                return Err(AppError::StateConflict);
            }
            PortableJobInput::ExportDiagnostics {
                categories: preview.categories,
                snapshot: Box::new(preview.snapshot),
            }
        };
        self.accept_output_job(identity, command, token, input)
    }
}

pub(crate) fn snapshot_files(
    snapshot: &DiagnosticSnapshot,
) -> Result<Vec<(DiagnosticCategory, Vec<u8>)>> {
    let mut files = Vec::new();
    if let Some(versions) = &snapshot.versions {
        files.push((
            DiagnosticCategory::Versions,
            serde_json::to_vec_pretty(versions)?,
        ));
    }
    if let Some(statistics) = &snapshot.statistics {
        files.push((
            DiagnosticCategory::Statistics,
            serde_json::to_vec_pretty(statistics)?,
        ));
    }
    if let Some(errors) = &snapshot.errors {
        files.push((
            DiagnosticCategory::Errors,
            serde_json::to_vec_pretty(errors)?,
        ));
    }
    Ok(files)
}
