use crate::{MetadataReceipt, ObjectKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct TrashItem {
    pub object_id: String,
    pub kind: ObjectKind,
    pub title: String,
    pub row_version: u32,
    pub preview: String,
    pub updated_at: Option<String>,
    pub parent_source_id: Option<String>,
    pub source_trashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TrashPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TrashPage {
    pub items: Vec<TrashItem>,
    pub next_cursor: Option<String>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrashSelection {
    Objects { object_ids: Vec<String> },
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TrashPreviewRequest {
    pub selection: TrashSelection,
}

/// A short-lived, read-only snapshot. Deletion rechecks the complete closure.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TrashPreview {
    pub selection: TrashSelection,
    pub snapshot_token: String,
    pub items: Vec<TrashItem>,
    pub selected_count: u32,
    pub dependent_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeleteTrash {
    pub selection: TrashSelection,
    pub snapshot_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct TrashDeletedObject {
    pub object_id: String,
    pub kind: ObjectKind,
}

/// Contains identities only, never a recoverable copy of deleted content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct TrashCommit {
    pub deleted_objects: Vec<TrashDeletedObject>,
    pub receipt: MetadataReceipt,
}

impl crate::mutations::RecordedResult for TrashCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
