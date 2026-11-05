use crate::{
    AnnotationSummary, MetadataReceipt, NoteSummary, ObjectKind, ObjectMetadata, SourceSummary,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CollectionDetail {
    pub id: String,
    pub name: String,
    pub overview_note_id: Option<String>,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
    /// Unknown for collections created before schema 7.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub metadata: ObjectMetadata,
    /// Includes individually trashed members; membership survives trashing.
    pub member_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateCollection {
    pub name: String,
    pub overview_note_id: Option<String>,
    #[serde(default)]
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateCollection {
    pub collection_id: String,
    pub base_row_version: u32,
    pub name: String,
    pub overview_note_id: Option<String>,
    pub is_pinned: bool,
    pub pin_position: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeCollectionMembers {
    pub collection_id: String,
    pub base_row_version: u32,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CollectionPageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pinned_only: bool,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CollectionPage {
    pub items: Vec<CollectionDetail>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CollectionMembersRequest {
    pub collection_id: String,
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_trashed: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub kind: Option<ObjectKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CollectionMember {
    pub position: u32,
    pub object: ObjectMetadata,
    pub note: Option<NoteSummary>,
    #[serde(default)]
    pub source: Option<SourceSummary>,
    #[serde(default)]
    pub annotation: Option<AnnotationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CollectionMembersPage {
    pub collection_id: String,
    pub collection_row_version: u32,
    pub items: Vec<CollectionMember>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct CollectionCommit {
    pub collection: CollectionDetail,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SetObjectFlag {
    pub object_id: String,
    pub base_row_version: u32,
    pub value: bool,
}
