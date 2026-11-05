use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StatisticCount {
    pub key: String,
    #[ts(type = "number")]
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityCheckState {
    NotRun,
}

/// Inventory counts, including trashed objects and historical registered blobs.
/// Source format counts describe each source's current version only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceStatistics {
    pub schema_version: u32,
    pub objects: Vec<StatisticCount>,
    #[ts(type = "number")]
    pub note_revisions: u64,
    #[ts(type = "number")]
    pub blobs: u64,
    #[ts(type = "number")]
    pub blob_bytes: u64,
    pub sources_by_format: Vec<StatisticCount>,
    pub source_jobs: Vec<StatisticCount>,
    pub portable_jobs: Vec<StatisticCount>,
    pub index_generations: Vec<StatisticCount>,
    /// Objects with a currently searchable index generation.\n    #[ts(type = "number")]
    pub indexed_objects: u64,
    /// Chunks inside those active generations, i.e. the searchable passage total.\n    #[ts(type = "number")]
    pub index_chunks: u64,
    pub index_rebuild_jobs: Vec<StatisticCount>,
    pub answers: Vec<StatisticCount>,
    pub review_entries: Vec<StatisticCount>,
    pub integrity_check: IntegrityCheckState,
    pub retrieval_latency_24h: RetrievalLatencyStats,
    pub model_usage_30d: ModelUsageStats,
}

/// Local retrieval latency for the statistics panel. `samples == 0` means the
/// window has no recorded retrieval yet, which the UI shows as an empty state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RetrievalLatencyStats {
    #[ts(type = "number")]
    pub samples: u64,
    #[ts(type = "number")]
    pub average_ms: u64,
    #[ts(type = "number")]
    pub p95_ms: u64,
}

/// Model call and token totals for the last 30 days. Counts are split so an
/// estimate is never presented as a provider-reported number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ModelUsageStats {
    #[ts(type = "number")]
    pub calls: u64,
    #[ts(type = "number")]
    pub estimated_calls: u64,
    #[ts(type = "number")]
    pub calls_without_usage: u64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Calls that carried a measured wall-clock duration.
    #[ts(type = "number")]
    pub timed_calls: u64,
    /// Mean wall-clock duration of those calls; absent until one is measured.
    #[ts(type = "number | null")]
    pub average_duration_ms: Option<u64>,
    /// Mean time to the first streamed token; absent for non-streamed calls.
    #[ts(type = "number | null")]
    pub average_first_token_ms: Option<u64>,
}
