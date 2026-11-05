use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Lexical,
    Semantic,
    Hybrid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Note,
    Source,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchScope {
    #[serde(default)]
    pub object_ids: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<EvidenceKind>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub collection_id: Option<String>,
    pub inbox_state: Option<crate::InboxState>,
    pub reading_status: Option<crate::ReadingStatus>,
    pub source_format: Option<crate::SourceFormat>,
    pub archived: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SearchQuery {
    pub query: String,
    pub mode: SearchMode,
    pub scope: SearchScope,
    pub limit: u32,
    pub minimum_cosine: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct EvidenceTarget {
    pub object_id: String,
    pub kind: EvidenceKind,
    pub content_revision: String,
    pub metadata_revision: u32,
    pub parse_id: Option<String>,
    pub block_id: String,
    pub start_codepoint: u32,
    pub end_codepoint: u32,
    pub page_index: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchEvidence {
    pub id: String,
    pub target: EvidenceTarget,
    pub title: String,
    pub text: String,
    pub lexical_rank: Option<u32>,
    pub cosine_similarity: Option<f64>,
    pub fusion_rank: Option<f64>,
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub annotation: Option<AnnotationEvidence>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AnnotationEvidence {
    pub annotation_id: String,
    pub annotation_revision: u32,
    pub locator_hash: String,
    pub embedding_profile_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnnotationVectorCandidate {
    pub evidence: SearchEvidence,
    pub profile_id: String,
    pub cosine_similarity: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SearchResults {
    pub items: Vec<SearchEvidence>,
    pub lexical_available: bool,
    pub semantic_available: bool,
    pub unavailable_reason: Option<String>,
    pub profile_id: Option<String>,
}
/// Transport family for a remote embedding profile. Each variant is called
/// through its provider's own documented request/response schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProvider {
    /// Cloudflare Workers AI, through its documented OpenAI-compatible
    /// `/ai/v1/embeddings` endpoint.
    Cloudflare,
    /// OpenAI `/v1/embeddings`.
    Openai,
    /// Google Gemini `models/{model}:batchEmbedContents`.
    Gemini,
    /// Any other OpenAI-compatible `/v1/embeddings` server the user runs or
    /// subscribes to (Ollama, vLLM, LM Studio, TEI, gateways).
    OpenaiCompatible,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct EmbeddingProfileSpec {
    /// Remote embedding endpoint only; credentials are stored in the OS vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub endpoint: Option<String>,
    pub model: String,
    pub revision: String,
    pub dimensions: u32,
    pub normalized: bool,
    pub query_prefix: String,
    pub passage_prefix: String,
    pub tokenizer: String,
    pub chunker: String,
    pub max_tokens: u32,
    /// Overlap between consecutive chunks of the same block, in tokens. Older
    /// stored specs have no overlap, which keeps them readable.
    #[serde(default)]
    pub overlap_tokens: u32,
    /// Which provider schema this profile is called with. Profiles registered
    /// before V1.7 have no value here; they keep their exact JSON (and therefore
    /// their identity) and mean the default Cloudflare BGE-M3 configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub provider: Option<EmbeddingProvider>,
    /// Output size the user asked the provider to produce, when the model
    /// supports truncation. `dimensions` is the size the provider actually
    /// returned, measured when the profile is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub output_dimensions: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EmbeddingProfile {
    pub id: String,
    #[serde(flatten)]
    pub spec: EmbeddingProfileSpec,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexInputBlock {
    pub target: EvidenceTarget,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexInput {
    pub object_id: String,
    pub kind: EvidenceKind,
    pub content_revision: String,
    pub metadata_revision: u32,
    pub parser_revision: String,
    pub title: String,
    pub blocks: Vec<IndexInputBlock>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexChunkInput {
    pub chunk_id: String,
    pub block_id: String,
    pub start_codepoint: u32,
    pub end_codepoint: u32,
    pub text: String,
    pub token_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexPlan {
    pub profile_id: String,
    pub object_id: String,
    pub content_revision: String,
    pub metadata_revision: u32,
    pub parser_revision: String,
    pub chunks: Vec<IndexChunkInput>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IndexGenerationStatus {
    Running,
    Ready,
    Failed,
    Canceled,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexGeneration {
    pub id: String,
    pub object_id: String,
    pub content_revision: String,
    pub metadata_revision: u32,
    pub profile_id: String,
    pub parser_revision: String,
    pub status: IndexGenerationStatus,
    pub chunk_count: u32,
    pub error_code: Option<String>,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexLease {
    pub generation: IndexGeneration,
    pub lease_token: String,
    pub receipt: crate::MetadataReceipt,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EmbeddingProfileMutation {
    pub profile: EmbeddingProfile,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for EmbeddingProfileMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
impl crate::mutations::RecordedResult for IndexLease {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexPublish {
    pub generation_id: String,
    pub lease_token: String,
    pub profile_id: String,
    pub chunk_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VectorCandidate {
    pub chunk_id: String,
    pub generation_id: String,
    pub object_id: String,
    pub content_revision: String,
    pub profile_id: String,
    pub cosine_similarity: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexStatus {
    pub profile: Option<EmbeddingProfile>,
    pub eligible_objects: u32,
    pub ready_objects: u32,
    pub running_generations: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IndexedChunk {
    pub id: String,
    pub generation_id: String,
    pub target: EvidenceTarget,
    pub title: String,
    pub text: String,
    pub token_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RebuildRequest {
    pub profile_id: String,
    pub scope: SearchScope,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RebuildStatus {
    Queued,
    Running,
    Completed,
    Canceled,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RebuildJob {
    pub id: String,
    pub request: RebuildRequest,
    pub status: RebuildStatus,
    pub after_object_id: Option<String>,
    pub processed: u32,
    pub skipped: u32,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RebuildMutation {
    pub job: RebuildJob,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for RebuildMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RebuildLease {
    pub job: RebuildJob,
    pub lease_token: String,
}

/// Retrieval candidate width (Top-K) and the separate answer citation count.
/// The similarity floor is optional: `None` means no filtering at all, because
/// cosine distributions differ per model and dimension.
pub const DEFAULT_TOP_K: u32 = 8;
pub const DEFAULT_CITATION_COUNT: u32 = 5;
pub const MAX_TOP_K: u32 = 50;
pub const MAX_CITATION_COUNT: u32 = 20;
/// Bounds for the optional per-library answer timeout: a total wall-clock budget
/// for one cited answer, never a per-token limit.
pub const MIN_ANSWER_TIMEOUT_SECONDS: u32 = 10;
pub const MAX_ANSWER_TIMEOUT_SECONDS: u32 = 3600;
/// Citation ids are still capped by the historical evidence ceiling.
pub const LEGACY_EVIDENCE_LIMIT: usize = 32;
/// Upper bound for a user-entered reference price per million tokens, in the
/// currency they choose. It only guards against typos, not against prices.
pub const MAX_COST_PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RetrievalParams {
    pub top_k: u32,
    pub citation_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub minimum_cosine: Option<f64>,
    /// When false, retrieval and answering never call a remote embedding service.
    /// Older stored rows default to enabled, which is the pre-existing behavior.
    #[serde(default = "default_true")]
    pub semantic_enabled: bool,
    /// Library-level acknowledgement that enabled text may leave this device.
    /// Remote embedding and generation configuration is refused without it.
    #[serde(default)]
    pub remote_consent: bool,
    /// Reference price the user enters for their own provider account, per
    /// million input tokens. Absent means no cost estimate is shown: the app
    /// never assumes a price list of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub cost_per_million_input: Option<f64>,
    /// Same as `cost_per_million_input`, for generated output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub cost_per_million_output: Option<f64>,
    /// Total seconds allowed for one cited answer. Absent means the provider
    /// default: a model running on this machine is given a longer budget than a
    /// remote one. Saved values are bounded by MIN/MAX_ANSWER_TIMEOUT_SECONDS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub answer_timeout_seconds: Option<u32>,
}
fn default_true() -> bool {
    true
}
impl Default for RetrievalParams {
    fn default() -> Self {
        Self {
            top_k: DEFAULT_TOP_K,
            citation_count: DEFAULT_CITATION_COUNT,
            minimum_cosine: None,
            semantic_enabled: true,
            remote_consent: false,
            cost_per_million_input: None,
            cost_per_million_output: None,
            answer_timeout_seconds: None,
        }
    }
}
pub fn validate_retrieval_params(params: &RetrievalParams) -> crate::Result<()> {
    if !(1..=MAX_TOP_K).contains(&params.top_k)
        || !(1..=MAX_CITATION_COUNT).contains(&params.citation_count)
        || params.citation_count > params.top_k
        || params
            .minimum_cosine
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        || params
            .cost_per_million_input
            .is_some_and(|v| !v.is_finite() || !(0.0..=MAX_COST_PER_MILLION).contains(&v))
        || params
            .cost_per_million_output
            .is_some_and(|v| !v.is_finite() || !(0.0..=MAX_COST_PER_MILLION).contains(&v))
        || params.answer_timeout_seconds.is_some_and(|v| {
            !(MIN_ANSWER_TIMEOUT_SECONDS..=MAX_ANSWER_TIMEOUT_SECONDS).contains(&v)
        })
    {
        return Err(crate::Error::InvalidInput("invalid retrieval params"));
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RetrievalSettings {
    pub params: RetrievalParams,
    pub row_version: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SetRetrievalParams {
    pub base_row_version: u32,
    pub params: RetrievalParams,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RetrievalSettingsMutation {
    pub settings: RetrievalSettings,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for RetrievalSettingsMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
