use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCategory {
    Versions,
    Statistics,
    Errors,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct DiagnosticFile {
    pub name: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct DiagnosticExportReport {
    pub bundle_id: String,
    pub generated_at: String,
    pub sha256: String,
    pub files: Vec<DiagnosticFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticVersions {
    pub application: String,
    pub app_schema: u32,
    pub knowledge_schema: u32,
    pub platform: String,
    pub architecture: String,
    pub webview_version: Option<String>,
    pub source_worker_protocol: u32,
    pub inference_protocol: u32,
    pub embedding_model_revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticError {
    pub component: String,
    pub id: String,
    pub stage: String,
    pub code: String,
    pub occurred_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSnapshot {
    pub versions: Option<DiagnosticVersions>,
    pub statistics: Option<crate::WorkspaceStatistics>,
    pub errors: Option<Vec<DiagnosticError>>,
}
