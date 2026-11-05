use crate::{MetadataReceipt, ObjectKind, ResolveLink, ResolvedLink};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Completed,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewBucket {
    Due,
    Later,
    Completed,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Added,
    Completed,
    Snoozed,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewEntry {
    pub id: String,
    pub target: ResolveLink,
    pub kind: ObjectKind,
    pub state: ReviewState,
    pub due_at: String,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewItem {
    pub entry: ReviewEntry,
    pub resolved: ResolvedLink,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AddReview {
    pub target: ResolveLink,
    pub base_row_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewChange {
    pub entry_id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SnoozeReview {
    pub entry_id: String,
    pub base_row_version: u32,
    pub due_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewMutation {
    pub entry: ReviewEntry,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewListRequest {
    pub bucket: ReviewBucket,
    pub collection_id: Option<String>,
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewPage {
    pub items: Vec<ReviewItem>,
    pub next_cursor: Option<String>,
}

/// One count per review tab. It comes from a single aggregate read that uses the
/// same `as_of` boundary the list filters with, so a badge never has to page
/// through the queue to produce a number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ReviewCounts {
    pub due: u32,
    pub later: u32,
    pub completed: u32,
    pub removed: u32,
    /// The instant the 到期/稍后 split was taken at.
    pub as_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewEvent {
    pub entry_id: String,
    pub seq: u32,
    pub action: ReviewAction,
    pub state: ReviewState,
    pub due_at: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewHistoryRequest {
    pub entry_id: String,
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewEventPage {
    pub items: Vec<ReviewEvent>,
    pub next_cursor: Option<String>,
}
