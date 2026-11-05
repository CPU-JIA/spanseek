pub use crate::chat_format::ChatFormat;
use pkms_core::{EmbeddingProfile, IndexInput, IndexPlan, IndexPublish, VectorCandidate};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// V1 workers ignore unknown Chat fields; version 2 prevents silently dropping
// the constrained format when an outdated executable is accidentally selected.
pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_REQUEST_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_RESULT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestIdentity {
    pub request_id: String,
    pub job_id: String,
    pub attempt: u32,
    pub lease_token: String,
}
#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub protocol: u32,
    pub identity: RequestIdentity,
    pub model_directory: PathBuf,
    pub vector_directory: PathBuf,
    pub result_path: PathBuf,
    pub operation: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    ScoreExactQuotes {
        profile: EmbeddingProfile,
        query: String,
        inputs: Vec<ExactQuoteInput>,
        limit: usize,
    },
    PrepareChunks {
        profile: EmbeddingProfile,
        input: IndexInput,
    },
    EmbedGeneration {
        profile: EmbeddingProfile,
        plan: IndexPlan,
        title: String,
        generation_id: String,
    },
    SearchVectors {
        profile: EmbeddingProfile,
        generations: Vec<GenerationExpectation>,
        query: String,
        limit: usize,
    },
    PublishVectors {
        profile: EmbeddingProfile,
        plan: IndexPlan,
        generation_id: String,
        vectors: Vec<Vec<f32>>,
    },
    SearchEmbedded {
        profile: EmbeddingProfile,
        generations: Vec<GenerationExpectation>,
        vector: Vec<f32>,
        limit: usize,
    },
    Chat {
        endpoint: String,
        model: String,
        messages: Vec<ChatMessage>,
        num_ctx: u32,
        num_predict: u32,
        temperature: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<ChatFormat>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactQuoteInput {
    pub id: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactQuoteScore {
    pub id: String,
    pub start_codepoint: u32,
    pub end_codepoint: u32,
    pub cosine_similarity: f64,
}
#[derive(Serialize, Deserialize)]
pub struct GenerationExpectation {
    pub generation_id: String,
    pub chunk_count: u32,
}
#[derive(Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}
#[derive(Serialize, Deserialize)]
pub struct WorkerEvent {
    pub protocol: u32,
    pub identity: RequestIdentity,
    pub event: Event,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Progress {
        stage: String,
        completed: u32,
        total: Option<u32>,
    },
    TextDelta {
        text: String,
    },
    Completed {
        result_path: PathBuf,
        sha256: String,
        bytes: u64,
    },
    Failed {
        code: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerResult {
    QuoteScores {
        items: Vec<ExactQuoteScore>,
    },
    Prepared {
        plan: IndexPlan,
    },
    Indexed {
        publish: IndexPublish,
    },
    Candidates {
        items: Vec<VectorCandidate>,
    },
    Answer {
        done_reason: Option<String>,
        prompt_eval_count: Option<u64>,
        eval_count: Option<u64>,
    },
}
