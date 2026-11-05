use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One current block projection. The immutable note revision remains the
/// document truth; this row supplies stable identity, outline order and search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct NoteBlockSummary {
    pub id: String,
    pub note_id: String,
    pub note_title: String,
    pub revision_id: String,
    pub parent_block_id: Option<String>,
    pub position: u32,
    pub depth: u32,
    pub kind: String,
    pub text: String,
    pub alias: Option<String>,
    pub reference_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct BlocksPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    pub query: Option<String>,
    pub note_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BlockPage {
    pub items: Vec<NoteBlockSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct BlockHistoryRequest {
    pub note_id: String,
    pub block_id: String,
    pub before: Option<u32>,
    pub limit: u32,
}

/// A block's captured state in one immutable note revision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BlockHistoryItem {
    pub revision_id: String,
    pub revision: u32,
    pub note_title: String,
    pub created_at: String,
    pub present: bool,
    pub kind: Option<String>,
    pub text: Option<String>,
    pub alias: Option<String>,
    pub parent_block_id: Option<String>,
    pub position: Option<u32>,
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BlockHistoryPage {
    pub items: Vec<BlockHistoryItem>,
    pub next_before: Option<u32>,
}
