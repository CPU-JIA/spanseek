use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Workspace-scoped configuration that must travel with the knowledge base.
/// Both registries remain the canonical JSON strings used by the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WorkspaceMetadata {
    pub object_type_registry: String,
    pub note_template_registry: String,
    pub row_version: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkspaceMetadata {
    pub base_row_version: u32,
    pub object_type_registry: String,
    pub note_template_registry: String,
}
