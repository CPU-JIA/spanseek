use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Note,
    Collection,
    Source,
    Annotation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    Pending,
    Snoozed,
    Processed,
}

impl InboxState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Snoozed => "snoozed",
            Self::Processed => "processed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct Tag {
    pub id: String,
    pub display_name: String,
    pub normalized_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ObjectMetadata {
    pub id: String,
    pub kind: ObjectKind,
    pub row_version: u32,
    pub archived: bool,
    pub trashed: bool,
    pub inbox_state: Option<InboxState>,
    pub search_enabled: bool,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct MetadataTarget {
    pub object_id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TagSelector {
    Id { id: String },
    Name { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AssignTag {
    pub targets: Vec<MetadataTarget>,
    pub tag: TagSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RemoveTag {
    pub targets: Vec<MetadataTarget>,
    pub tag_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SetInboxState {
    pub object_id: String,
    pub base_row_version: u32,
    pub target: InboxState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct MetadataReceipt {
    pub command_id: String,
    pub change_seq: String,
    pub committed_at: String,
    pub replayed: bool,
}

/// An immutable description of this command, not a fresh metadata query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct MetadataCommit {
    pub objects: Vec<ObjectMetadata>,
    pub tag: Option<Tag>,
    pub tag_created: bool,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct TagsPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct TagPage {
    pub items: Vec<Tag>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct NotesFilter {
    pub inbox_state: Option<InboxState>,
    /// Every selected tag must be assigned (AND). An empty list imposes no filter.
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub collection_id: Option<String>,
    /// Missing/null means normal, unarchived browsing.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub archived: Option<bool>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub search_enabled: Option<bool>,
    /// Canonical or alias type tokens. `__untyped` matches notes without a type.
    #[serde(default)]
    pub object_types: Vec<String>,
}
