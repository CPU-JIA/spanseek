use crate::ObjectKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TagSyncKind {
    Tag,
    TagAssignments,
}

impl TagSyncKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::TagAssignments => "tag_assignments",
        }
    }
}

/// The deletion time is a retained dictionary tombstone, not an erased identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SyncedTag {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SyncedTagAssignments {
    pub object_id: String,
    pub object_kind: ObjectKind,
    /// Original stored UUID identifiers in ascending order, without duplicates.
    /// Empty removes all; legacy UUID text is not normalized or merged.
    pub tag_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "sync_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TagSyncPayload {
    Tag { tag: SyncedTag },
    TagAssignments { assignments: SyncedTagAssignments },
}

impl TagSyncPayload {
    pub fn kind(&self) -> TagSyncKind {
        match self {
            Self::Tag { .. } => TagSyncKind::Tag,
            Self::TagAssignments { .. } => TagSyncKind::TagAssignments,
        }
    }

    pub fn entity_id(&self) -> &str {
        match self {
            Self::Tag { tag } => &tag.id,
            Self::TagAssignments { assignments } => &assignments.object_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TagSyncPendingReason {
    LocalChanges,
    NameConflict,
    MissingObject,
    WrongObjectKind,
    MissingTags,
    DeletedTags,
    ViewDependency,
    AssignmentChanges,
    DeletedTag,
}

impl TagSyncPendingReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalChanges => "local_changes",
            Self::NameConflict => "name_conflict",
            Self::MissingObject => "missing_object",
            Self::WrongObjectKind => "wrong_object_kind",
            Self::MissingTags => "missing_tags",
            Self::DeletedTags => "deleted_tags",
            Self::ViewDependency => "view_dependency",
            Self::AssignmentChanges => "assignment_changes",
            Self::DeletedTag => "deleted_tag",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagSyncPending {
    pub payload: TagSyncPayload,
    pub reason: TagSyncPendingReason,
    pub baseline_fingerprint: Option<String>,
    pub received_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagSyncLabel {
    pub id: String,
    pub display_name: Option<String>,
    pub deleted: bool,
}

/// Thin ownership keys for one sync operation; no deferred payloads or titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSyncWireOwner {
    pub sync_object_id: String,
    pub kind: TagSyncKind,
    pub entity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagSyncSnapshot {
    pub sync_object_id: String,
    pub kind: TagSyncKind,
    pub entity_id: String,
    /// Local display metadata, excluded from the encrypted payload/fingerprint.
    pub entity_title: Option<String>,
    pub local_tags: Vec<TagSyncLabel>,
    pub remote_tags: Vec<TagSyncLabel>,
    /// None for an entity that has never existed here. Never publish a pending
    /// missing object's hypothetical empty assignment set.
    pub payload: Option<TagSyncPayload>,
    pub local_exists: bool,
    #[ts(type = "number")]
    pub local_generation: i64,
    pub local_fingerprint: Option<String>,
    /// Required for a pending dictionary deletion: binds associated objects and
    /// views without changing the dictionary's wire fingerprint.
    pub local_impact_token: Option<String>,
    pub delete_impact: Option<crate::TagDetails>,
    pub dirty: bool,
    /// Highest durably received or acknowledged server version, including pending.
    #[ts(type = "number")]
    pub remote_version: i64,
    /// Version of base_fingerprint; an older late acknowledgement cannot regress it.
    #[ts(type = "number")]
    pub base_version: i64,
    pub base_fingerprint: Option<String>,
    pub pending: Option<TagSyncPending>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TagSyncPageRequest {
    pub limit: u32,
    pub after: Option<String>,
    pub pending_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagSyncPage {
    pub items: Vec<TagSyncSnapshot>,
    pub total: u32,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TagSyncApplyStatus {
    Applied,
    Unchanged,
    Pending,
    IgnoredOlder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagSyncApplyResult {
    pub status: TagSyncApplyStatus,
    pub snapshot: TagSyncSnapshot,
    /// Metadata caches may refresh these objects; no note revision is replaced.
    pub changed_objects: Vec<String>,
}
