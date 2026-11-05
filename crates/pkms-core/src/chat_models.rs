use crate::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatPurpose {
    Chat,
    Summarize,
    Rewrite,
    Research,
    Review,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatRunOptions {
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
    pub total_timeout_seconds: u32,
}
impl Default for ChatRunOptions {
    fn default() -> Self {
        Self {
            max_model_calls: 8,
            max_tool_calls: 16,
            max_output_tokens: 8192,
            total_timeout_seconds: 600,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateConversation {
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StartChatRun {
    pub conversation_id: String,
    pub base_row_version: u32,
    pub text: String,
    pub purpose: ChatPurpose,
    pub materials: Vec<SearchEvidence>,
    pub scope: SearchScope,
    pub use_knowledge_tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional=nullable)]
    pub options: Option<ChatRunOptions>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunStatus {
    Queued,
    Running,
    Completed,
    Canceled,
    Interrupted,
    TimedOut,
    Failed,
    Truncated,
    Refused,
    ModelUnconfigured,
}
impl ChatRunStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatEventKind {
    TextDelta,
    Status,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct ChatUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatConversation {
    pub id: String,
    pub title: String,
    pub row_version: u32,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatConversationCursor {
    pub updated_at: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatConversationPage {
    pub items: Vec<ChatConversation>,
    pub next_cursor: Option<ChatConversationCursor>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatMessagePage {
    pub conversation: ChatConversation,
    /// Chronological messages from a bounded window, newest window first.
    pub items: Vec<ChatMessage>,
    pub next_before: Option<u32>,
    /// Only the newest page contains the latest run. It shares the messages' read snapshot,
    /// so its event sequence and the assistant text can safely seed stream resumption.
    pub latest_run: Option<ChatRun>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub ordinal: u32,
    pub role: ChatRole,
    pub content: String,
    pub created_at: String,
    pub run_id: String,
    pub run_status: ChatRunStatus,
    pub run_error_code: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatRun {
    pub id: String,
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub status: ChatRunStatus,
    pub request: StartChatRun,
    pub model: Option<GenerationProfile>,
    pub last_event_seq: u32,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub usage: ChatUsage,
    pub finish_reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatRunEvent {
    pub seq: u32,
    pub kind: ChatEventKind,
    pub text_delta: Option<String>,
    pub status: ChatRunStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatRunLease {
    pub run: ChatRun,
    pub lease_token: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatConversationMutation {
    pub conversation: ChatConversation,
    pub receipt: MetadataReceipt,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChatRunMutation {
    pub conversation: ChatConversation,
    pub run: ChatRun,
    pub receipt: MetadataReceipt,
}
impl crate::mutations::RecordedResult for ChatConversationMutation {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl crate::mutations::RecordedResult for ChatRunMutation {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
