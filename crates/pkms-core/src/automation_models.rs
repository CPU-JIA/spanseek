//! Contracts for controlled automation, research tasks, audit and explainable
//! review. Every proposal is inspectable; nothing in this module writes a note
//! unless an explicit per-item command is applied.
use crate::MetadataReceipt;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_CHANGESET_ITEMS: u32 = 200;
pub const MAX_CHANGESET_PAGE: u32 = 50;
pub const MAX_RESEARCH_QUERIES: usize = 20;
pub const MAX_RESEARCH_STEPS: u32 = 50;
pub const MAX_RESEARCH_SOURCES: u32 = 100;
pub const MAX_AUDIT_PAGE: u32 = 100;
pub const MAX_EVIDENCE_EXCERPT: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetKind {
    TagSuggestions,
    DuplicateCandidates,
    RelationSuggestions,
    StaleSources,
}
impl ChangeSetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TagSuggestions => "tag_suggestions",
            Self::DuplicateCandidates => "duplicate_candidates",
            Self::RelationSuggestions => "relation_suggestions",
            Self::StaleSources => "stale_sources",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    Pending,
    Applied,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeItemStatus {
    Pending,
    Applied,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeItemKind {
    AssignTag,
    TrashDuplicate,
    AppendRelation,
    AcknowledgeStale,
}
impl ChangeItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssignTag => "assign_tag",
            Self::TrashDuplicate => "trash_duplicate",
            Self::AppendRelation => "append_relation",
            Self::AcknowledgeStale => "acknowledge_stale",
        }
    }
    pub fn requires_confirmation(self) -> bool {
        matches!(self, Self::TrashDuplicate | Self::AppendRelation)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateChangeSet {
    pub kind: ChangeSetKind,
    pub collection_id: Option<String>,
    pub limit: u32,
    pub stale_after_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeScope {
    pub collection_id: Option<String>,
    pub limit: u32,
    pub stale_after_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangePayload {
    AssignTag {
        tag: String,
        matched_text: String,
        field: String,
    },
    TrashDuplicate {
        duplicate_id: String,
        duplicate_title: String,
        similarity_basis_points: u32,
        reason: String,
    },
    AppendRelation {
        target_id: String,
        target_title: String,
        shared_tags: Vec<String>,
        shared_terms: Vec<String>,
    },
    StaleSource {
        age_days: u32,
        captured_at: String,
        origin_url: Option<String>,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeEvidence {
    pub object_id: String,
    pub revision_id: Option<String>,
    pub field: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeItem {
    pub item_id: String,
    pub ordinal: u32,
    pub kind: ChangeItemKind,
    pub target_object_id: String,
    pub target_title: String,
    pub target_revision_id: Option<String>,
    pub payload: ChangePayload,
    pub evidence: Vec<ChangeEvidence>,
    pub requires_confirmation: bool,
    pub status: ChangeItemStatus,
    pub blocked_reason: Option<String>,
    pub audit_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetSummary {
    pub id: String,
    pub kind: ChangeSetKind,
    pub status: ChangeSetStatus,
    pub scope: ChangeScope,
    pub item_count: u32,
    pub applied_count: u32,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeSet {
    pub id: String,
    pub kind: ChangeSetKind,
    pub status: ChangeSetStatus,
    pub scope: ChangeScope,
    pub item_count: u32,
    pub applied_count: u32,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub items: Vec<ChangeItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetPageRequest {
    pub limit: u32,
    pub before_id: Option<String>,
    pub kind: Option<ChangeSetKind>,
    pub status: Option<ChangeSetStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetPage {
    pub items: Vec<ChangeSetSummary>,
    pub next_before_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ApplyChangeItem {
    pub change_id: String,
    pub item_id: String,
    /// Destructive or body-rewriting items require an explicit confirmation.
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RejectChangeSet {
    pub change_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetCommit {
    pub change: ChangeSet,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ChangeItemCommit {
    pub change: ChangeSet,
    pub item: ChangeItem,
    pub audit: Option<AuditEvent>,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    OlderThanWindow,
    NoOrigin,
    MissingVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct StaleSourceFinding {
    pub source_id: String,
    pub title: String,
    pub captured_at: Option<String>,
    pub age_days: Option<u32>,
    pub origin_url: Option<String>,
    pub reason: StaleReason,
    pub evidence: Vec<ChangeEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct StaleSourceReport {
    pub checked: u32,
    pub window_days: u32,
    pub scanned_at: String,
    pub findings: Vec<StaleSourceFinding>,
    pub empty_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AuditSubjectKind {
    Note,
    Source,
    ChangeSet,
    ResearchTask,
    ReviewSetting,
    Tag,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    #[ts(type = "number")]
    pub seq: i64,
    pub id: String,
    pub occurred_at: String,
    pub kind: String,
    pub subject_kind: AuditSubjectKind,
    pub subject_id: String,
    pub summary: String,
    pub origin: String,
    pub undoable: bool,
    pub undone: bool,
    pub undone_at: Option<String>,
    pub change_id: Option<String>,
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AuditPageRequest {
    pub limit: u32,
    #[ts(type = "number | null")]
    pub before_seq: Option<i64>,
    pub subject_kind: Option<AuditSubjectKind>,
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AuditPage {
    pub items: Vec<AuditEvent>,
    #[ts(type = "number | null")]
    pub next_before_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct UndoAuditEvent {
    pub audit_id: String,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AuditUndoCommit {
    pub reversed: AuditEvent,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStatus {
    Pending,
    Running,
    Completed,
    Canceled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStepKind {
    PrivateRetrieval,
    ExternalSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStepStatus {
    Succeeded,
    Failed,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResearchOrigin {
    PrivateLibrary,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchBudget {
    pub max_steps: u32,
    pub max_sources: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchStopConditions {
    pub deadline_seconds: Option<u32>,
    pub missing_after_step: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CreateResearchTask {
    pub goal: String,
    pub collection_id: Option<String>,
    pub queries: Vec<String>,
    pub budget: ResearchBudget,
    pub stop: ResearchStopConditions,
    pub external_allowed: bool,
    pub external_search_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResearchStepInput {
    PrivateRetrieval { query: String, limit: u32 },
    ExternalSearch { query: String, endpoint: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResearchStepResult {
    PrivateRetrieval { matched: u32, returned: u32 },
    ExternalSearch { http_status: u16, body_bytes: u32 },
    Canceled { reason: String },
    Failed { code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchSource {
    pub object_id: String,
    pub revision_id: Option<String>,
    pub title: String,
    pub origin: ResearchOrigin,
    pub url: Option<String>,
    pub excerpt: String,
    pub excerpt_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchStep {
    pub task_id: String,
    pub seq: u32,
    pub kind: ResearchStepKind,
    pub status: ResearchStepStatus,
    pub origin: ResearchOrigin,
    pub input: ResearchStepInput,
    pub result: ResearchStepResult,
    pub sources: Vec<ResearchSource>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRevisionState {
    Current,
    Superseded,
    Missing,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchEvidence {
    pub id: String,
    pub task_id: String,
    pub step_seq: u32,
    pub object_id: String,
    pub object_revision_id: Option<String>,
    pub origin: ResearchOrigin,
    pub title: String,
    pub excerpt: String,
    pub excerpt_hash: String,
    pub source_url: Option<String>,
    pub captured_at: String,
    pub revision_state: EvidenceRevisionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchTask {
    pub id: String,
    pub goal: String,
    pub collection_id: Option<String>,
    pub queries: Vec<String>,
    pub budget: ResearchBudget,
    pub stop: ResearchStopConditions,
    pub external_allowed: bool,
    pub external_search_endpoint: Option<String>,
    pub status: ResearchStatus,
    pub stop_reason: Option<String>,
    pub step_count: u32,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub steps: Vec<ResearchStep>,
    pub evidence: Vec<ResearchEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchTaskSummary {
    pub id: String,
    pub goal: String,
    pub status: ResearchStatus,
    pub stop_reason: Option<String>,
    pub step_count: u32,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchPageRequest {
    pub limit: u32,
    pub before_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchPage {
    pub items: Vec<ResearchTaskSummary>,
    pub next_before_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct AdvanceResearch {
    pub task_id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct CancelResearch {
    pub task_id: String,
    pub base_row_version: u32,
    pub reason: String,
}

/// The host performs the single external GET and hands the bounded result back.
/// The endpoint and query are recorded, so the outbound request can be checked
/// against the private library without trusting the caller's summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RecordExternalStep {
    pub task_id: String,
    pub base_row_version: u32,
    pub query: String,
    pub endpoint: String,
    pub http_status: u16,
    pub final_url: String,
    pub title: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct RecordFailedExternalStep {
    pub task_id: String,
    pub base_row_version: u32,
    pub query: String,
    pub endpoint: String,
    pub error_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResearchCommit {
    pub task: ResearchTask,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewInsightSettings {
    pub enabled: bool,
    pub window_days: u32,
    pub include_completed: bool,
    pub include_stale: bool,
    pub row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct SetReviewInsightSettings {
    pub base_row_version: u32,
    pub enabled: bool,
    pub window_days: u32,
    pub include_completed: bool,
    pub include_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewSettingsCommit {
    pub settings: ReviewInsightSettings,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewInsightKind {
    Revisit,
    Gap,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewInsightBasis {
    pub kind: String,
    pub occurred_at: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewInsight {
    pub object_id: String,
    pub title: String,
    pub kind: ReviewInsightKind,
    pub reason: String,
    pub basis: Vec<ReviewInsightBasis>,
    pub caveat: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewInsights {
    pub enabled: bool,
    pub settings: ReviewInsightSettings,
    pub items: Vec<ReviewInsight>,
    pub empty_reason: Option<String>,
    pub disclaimer: String,
}
