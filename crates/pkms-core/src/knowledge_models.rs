use crate::{
    AnnotationSummary, InboxState, NoteSummary, ObjectKind, ObjectMetadata, SourceSummary,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ObjectCollectionRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KnowledgeObjectHit {
    pub snippet: String,
    pub start_codepoint: u32,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub page_index: Option<u32>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub block_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KnowledgeObject {
    pub object: ObjectMetadata,
    #[serde(default)]
    pub note: Option<NoteSummary>,
    #[serde(default)]
    pub source: Option<SourceSummary>,
    #[serde(default)]
    pub annotation: Option<AnnotationSummary>,
    pub collections: Vec<ObjectCollectionRef>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub object_type: Option<String>,
    pub title: String,
    pub preview: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub hit: Option<KnowledgeObjectHit>,
    #[serde(default)]
    pub related_count: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeObjectFilter {
    /// Empty means notes, sources and annotations. Collections are categories, not index rows.
    #[serde(default)]
    pub kinds: Vec<ObjectKind>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub collection_id: Option<String>,
    #[serde(default)]
    pub uncategorized: bool,
    /// Every selected tag must be assigned (AND).
    #[serde(default)]
    pub tag_ids: Vec<String>,
    /// Exact object IDs from a grouped search; empty means no object restriction.
    #[serde(default)]
    pub object_ids: Vec<String>,
    #[serde(default)]
    pub object_types: Vec<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub inbox_state: Option<InboxState>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub trashed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeObjectPageRequest {
    pub limit: u32,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub cursor: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub query: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub filter: Option<KnowledgeObjectFilter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KnowledgeObjectPage {
    pub items: Vec<KnowledgeObject>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

/// Numbered browsing and topic statistics share the exact same visibility rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeBrowseRequest {
    pub limit: u32,
    pub page: u32,
    pub query: Option<String>,
    pub filter: Option<KnowledgeObjectFilter>,
    #[serde(default)]
    pub title_query: Option<String>,
    #[serde(default)]
    pub updated_from: Option<String>,
    #[serde(default)]
    pub updated_to: Option<String>,
    #[serde(default)]
    pub source_formats: Vec<crate::SourceFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KnowledgeTypeCount {
    pub kind: ObjectKind,
    pub object_type: Option<String>,
    pub count: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KnowledgeBrowsePage {
    pub items: Vec<KnowledgeObject>,
    pub total: u32,
    pub type_counts: Vec<KnowledgeTypeCount>,
    pub page: u32,
    pub page_size: u32,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}
