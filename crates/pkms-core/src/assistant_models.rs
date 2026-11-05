use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const INSUFFICIENT_EVIDENCE_MARKER: &str = "[[PKMS_INSUFFICIENT_EVIDENCE]]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChatTokenLimit {
    MaxTokens,
    #[default]
    MaxCompletionTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationProfile {
    /// ollama, openai_chat, openai_responses, anthropic, or gemini.
    pub provider: String,
    /// Local Ollama uses loopback HTTP; remote providers require a credential-free HTTPS base URL.
    pub endpoint: String,
    pub model: String,
    /// Required local weight identity for Ollama; empty for remote models with no verifiable digest.
    pub model_digest: String,
    pub config_revision: u32,
    pub temperature: f64,
    pub context_tokens: u32,
    /// OpenAI Chat only; absent uses max_completion_tokens, without host guessing or retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub chat_token_limit: Option<ChatTokenLimit>,
    /// Stable credential reference, frozen with the endpoint and protocol in each request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub channel_id: Option<String>,
    /// None preserves the historical protocol behavior; channel selections freeze explicit capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub capabilities: Option<crate::GenerationCapabilities>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GenerationSettings {
    pub profile: Option<GenerationProfile>,
    pub row_version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SetGenerationProfile {
    pub base_row_version: u32,
    pub profile: Option<GenerationProfile>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GenerationSettingsMutation {
    pub settings: GenerationSettings,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for GenerationSettingsMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StartAnswer {
    /// Frozen per request. Absent preserves historical 2048 remote / 384 local behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub max_output_tokens: Option<u32>,
    /// How many retrieved passages may become answer citations. Absent keeps the
    /// historical behavior of citing every candidate (still capped by evidence limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub citation_count: Option<u32>,
    pub search: crate::SearchQuery,
    pub model: Option<GenerationProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub agent: Option<AgentOptions>,
}

impl StartAnswer {
    pub fn output_tokens(&self) -> u32 {
        self.max_output_tokens.unwrap_or_else(|| {
            if self.model.as_ref().is_some_and(|p| p.provider == "ollama") {
                384
            } else {
                2048
            }
        })
    }
}
/// Explicit new budgets must fit the same output bounds as free chat. Historical
/// missing budgets retain their old wire identity and are checked at execution.
pub fn validate_answer_output_tokens(
    tokens: Option<u32>,
    model: Option<&GenerationProfile>,
) -> crate::Result<()> {
    if let Some(tokens) = tokens
        && (!(1..=32768).contains(&tokens) || model.is_some_and(|p| tokens >= p.context_tokens))
    {
        return Err(crate::Error::InvalidInput("answer_output_budget_invalid"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentOptions {
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_evidence: u32,
    pub total_timeout_seconds: u32,
}
impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            max_model_calls: 4,
            max_tool_calls: 8,
            max_evidence: 32,
            total_timeout_seconds: 240,
        }
    }
}
pub fn validate_agent_options(options: &AgentOptions) -> crate::Result<()> {
    if !(2..=4).contains(&options.max_model_calls)
        || !(1..=8).contains(&options.max_tool_calls)
        || !(1..=32).contains(&options.max_evidence)
        || !(1..=3600).contains(&options.total_timeout_seconds)
    {
        return Err(crate::Error::InvalidInput("invalid_agent_options"));
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AnswerStatus {
    Queued,
    Running,
    Completed,
    Canceled,
    TimedOut,
    NoEvidence,
    InvalidCitation,
    EvidenceChanged,
    Failed,
    ModelUnconfigured,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerEvidence {
    pub citation_id: String,
    pub evidence: crate::SearchEvidence,
    /// Small-to-big expansion: the parent block text handed to the model when
    /// the hit itself is only a narrow chunk or excerpt of that block. The
    /// citation still resolves to the precise hit range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub context: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerRecord {
    pub id: String,
    pub request: StartAnswer,
    pub status: AnswerStatus,
    pub text: String,
    pub evidence: Vec<AnswerEvidence>,
    pub cited_ids: Vec<String>,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_event_seq: u32,
    pub evidence_current: bool,
    pub feedback: Option<AnswerFeedback>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerLease {
    pub answer: AnswerRecord,
    pub lease_token: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerEvent {
    pub seq: u32,
    pub status: AnswerStatus,
    pub text_delta: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerMutation {
    pub answer: AnswerRecord,
    pub receipt: crate::MetadataReceipt,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CompleteAnswer {
    pub answer_id: String,
    pub lease_token: String,
    pub text: String,
    pub cited_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerFeedback {
    pub answer_id: String,
    pub rating: i32,
    pub comment: String,
    /// When this feedback was last changed, read from the command receipt that
    /// recorded the change. Read only: the write path rejects a supplied value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub updated_at: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Applied,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub answer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub chat_run_id: Option<String>,
    /// Set only by the local external adapter. This is the request ID used to
    /// audit a proposal that did not originate from an answer or chat run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub external_request_id: Option<String>,
    pub note_id: Option<String>,
    pub base_revision_id: Option<String>,
    pub title: String,
    pub document_schema: u32,
    pub document: crate::NoteDocument,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProposalRecord {
    pub id: String,
    pub request: CreateProposal,
    pub status: ProposalStatus,
    pub applied_revision_id: Option<String>,
    pub applied_note_id: Option<String>,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProposalMutation {
    pub proposal: ProposalRecord,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for AnswerMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}
impl crate::mutations::RecordedResult for ProposalMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}

/// One collected rating, with the question it was given for.
///
/// The change time is not a column: every rating change is recorded as a command
/// receipt, and the report reads it from there instead of keeping a second copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackRow {
    pub answer_id: String,
    pub rating: i32,
    pub comment: String,
    /// The question this answer was given for.
    pub question: String,
    pub changed_at: String,
}

/// One page of collected ratings, newest change first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackPage {
    pub items: Vec<FeedbackRow>,
    pub next_cursor: Option<String>,
}

/// Counts over every collected rating. `neutral` exists so a zero a future entry
/// point might write stays visible instead of being folded into one verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackSummary {
    pub total: u32,
    pub helpful: u32,
    pub unhelpful: u32,
    pub neutral: u32,
    pub commented: u32,
    pub newest_at: Option<String>,
}
