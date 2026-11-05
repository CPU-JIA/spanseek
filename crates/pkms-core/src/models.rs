use crate::NoteDocument;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct MigrationRecord {
    pub version: u32,
    pub checksum: String,
    pub applied_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub database_schema: u32,
    pub supported_document_schemas: Vec<u32>,
    pub migrations: Vec<MigrationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub revision: u32,
    pub revision_id: String,
    pub parent_revision_id: Option<String>,
    pub content_hash: String,
    pub author_kind: String,
    pub schema_version: u32,
    pub document: NoteDocument,
    pub trashed: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct Collection {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct NoteLink {
    pub source_id: String,
    pub target_id: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct SearchHit {
    pub note: Note,
    /// FTS5's BM25 score is a ranking value (lower is better), not a probability.
    pub bm25: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateNote {
    pub title: String,
    pub document_schema: u32,
    pub document: NoteDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub collection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub base_collection_row_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveNote {
    pub note_id: String,
    pub base_revision_id: String,
    pub title: String,
    pub document_schema: u32,
    pub document: NoteDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RestoreNote {
    pub note_id: String,
    pub base_revision_id: String,
    pub target_revision_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CopyNote {
    pub note_id: String,
    pub source_revision_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CommitReceipt {
    pub command_id: String,
    pub object_id: String,
    pub revision_id: String,
    pub change_seq: String,
    pub committed_at: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct NoteCommit {
    pub note: Note,
    pub receipt: CommitReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct BlobInfo {
    pub hash: String,
    pub size: u32,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct NotesPageRequest {
    pub limit: u32,
    /// Literal title substring; ASCII case-insensitive. Blank means no title filter.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub query: Option<String>,
    pub cursor: Option<String>,
    pub trashed: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub filter: Option<crate::NotesFilter>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub sort: Option<crate::NoteSort>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub revision_id: String,
    pub document_schema: u32,
    pub preview: String,
    pub trashed: bool,
    pub updated_at: String,
    pub row_version: u32,
    pub inbox_state: Option<crate::InboxState>,
    pub archived: bool,
    pub search_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct NotePage {
    pub items: Vec<NoteSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct RevisionSummary {
    pub revision_id: String,
    pub revision: u32,
    pub title: String,
    pub document_schema: u32,
    pub author_kind: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct RevisionPage {
    pub items: Vec<RevisionSummary>,
    pub next_before: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct AttachmentImport {
    pub blob: BlobInfo,
    pub original_name: String,
    pub command_id: String,
    pub change_seq: String,
    pub replayed: bool,
    /// Short-lived, process-local preview grant. Revision reads use real references.
    pub preview_token: String,
    pub preview_expires_at: String,
}
