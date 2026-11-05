use crate::{MetadataReceipt, Tag};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagViewUsage {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagDetails {
    pub tag: Tag,
    pub description: String,
    pub row_version: u32,
    /// Older dictionaries did not record creation/edit times; do not invent them.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub object_count: u32,
    pub active_object_count: u32,
    pub trashed_object_count: u32,
    pub note_count: u32,
    pub source_count: u32,
    pub annotation_count: u32,
    pub collection_count: u32,
    pub saved_views: Vec<TagViewUsage>,
    /// Binds destructive confirmation to the actual affected objects and views.
    pub impact_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TagsBrowseRequest {
    pub page: u32,
    pub limit: u32,
    pub query: Option<String>,
    pub collection_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagBrowseItem {
    pub tag: Tag,
    pub description: String,
    pub row_version: u32,
    pub updated_at: Option<String>,
    /// Count in the requested collection, or all active objects without a scope.
    pub object_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagsBrowsePage {
    pub items: Vec<TagBrowseItem>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateTag {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateTag {
    pub id: String,
    pub base_row_version: u32,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeleteTag {
    pub id: String,
    pub base_row_version: u32,
    pub impact_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TagManagementCommit {
    /// For deletion this is the confirmed pre-delete snapshot.
    pub tag: TagDetails,
    pub deleted: bool,
    pub affected_objects: u32,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TaggedObjectsRequest {
    pub tag_id: String,
    pub page: u32,
    pub limit: u32,
    pub trashed: bool,
    pub collection_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TaggedObject {
    pub id: String,
    pub kind: crate::ObjectKind,
    pub title: String,
    pub archived: bool,
    pub trashed: bool,
    pub source_id: Option<String>,
    pub source_version_id: Option<String>,
    pub source_trashed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TaggedObjectsPage {
    pub items: Vec<TaggedObject>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}
