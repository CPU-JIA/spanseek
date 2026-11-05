use crate::{MetadataReceipt, NoteSummary, ObjectMetadata};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CanvasNode {
    pub note_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub z_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Canvas {
    pub id: String,
    pub name: String,
    pub config_schema: u32,
    pub nodes: Vec<CanvasNode>,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveCanvas {
    pub canvas_id: Option<String>,
    pub base_row_version: Option<u32>,
    pub name: String,
    pub config_schema: u32,
    pub nodes: Vec<CanvasNode>,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RemoveCanvas {
    pub canvas_id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CanvasCommit {
    pub canvas: Canvas,
    pub removed: bool,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CanvasPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub pinned_only: bool,
    pub query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CanvasPage {
    pub items: Vec<Canvas>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CanvasNote {
    pub note: NoteSummary,
    pub metadata: ObjectMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CanvasView {
    pub canvas: Canvas,
    pub notes: Vec<CanvasNote>,
    /// Hard-purged objects leave stable IDs in the layout. The canvas says so
    /// rather than silently dropping those nodes or inventing replacement text.
    pub missing_note_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConfigTombstone {
    pub object_id: String,
    pub kind: String,
}
