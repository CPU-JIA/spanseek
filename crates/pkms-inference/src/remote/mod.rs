//! Explicit remote protocols. Credentials and provider continuation never enter answer text.
mod gemini;
mod parser;
mod request;
mod sse;
#[cfg(test)]
mod tests;
mod transport;

use crate::chat_format::ChatFormat;
pub use pkms_core::{ChatTokenLimit, GenerationTransport};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
pub use transport::Client;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
    Gemini,
}

/// Deliberately neither Debug nor Serialize. The HTTP header is marked sensitive.
pub struct ApiKey(reqwest::header::HeaderValue);
impl ApiKey {
    pub fn new(value: String) -> Result<Self, Error> {
        if value.is_empty() || value.len() > 8192 || value.bytes().any(|b| !(33..=126).contains(&b))
        {
            return Err(Error::InvalidKey);
        }
        let mut header =
            reqwest::header::HeaderValue::from_str(&value).map_err(|_| Error::InvalidKey)?;
        header.set_sensitive(true);
        Ok(Self(header))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub total_timeout: Duration,
    pub first_event_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_event_bytes: usize,
    pub max_tool_calls: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            total_timeout: Duration::from_secs(240),
            first_event_timeout: Duration::from_secs(90),
            idle_timeout: Duration::from_secs(30),
            max_response_bytes: 4 * 1024 * 1024,
            max_event_bytes: 256 * 1024,
            max_tool_calls: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidEndpoint,
    InvalidKey,
    InvalidRequest,
    Canceled,
    TotalTimeout,
    FirstEventTimeout,
    IdleTimeout,
    Unavailable,
    Authentication,
    RateLimited,
    /// The provider rejected the request itself (4xx other than auth/rate limit):
    /// a retired or misspelled model id, an unsupported parameter, or a protocol
    /// mismatch. Retrying the same request cannot help, so it must not be
    /// reported as a service-side error.
    RequestRejected,
    ApiError,
    InvalidResponse,
    ResponseTooLarge,
    Incomplete,
    UnsupportedOutput,
}
impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidEndpoint => "model_invalid_endpoint",
            Self::InvalidKey => "model_invalid_key",
            Self::InvalidRequest => "model_invalid_request",
            Self::Canceled => "model_canceled",
            Self::TotalTimeout => "model_total_timeout",
            Self::FirstEventTimeout => "model_first_token_timeout",
            Self::IdleTimeout => "model_idle_timeout",
            Self::Unavailable => "model_unavailable",
            Self::Authentication => "model_authentication_failed",
            Self::RateLimited => "model_rate_limited",
            Self::RequestRejected => "model_request_rejected",
            Self::ApiError => "model_api_error",
            Self::InvalidResponse => "model_invalid_response",
            Self::ResponseTooLarge => "model_response_too_large",
            Self::Incomplete => "model_incomplete",
            Self::UnsupportedOutput => "model_unsupported_output",
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Opaque per-assistant-turn replay material, bound to the same endpoint/model/protocol.
/// Do not show or persist this as an answer. Keep it only for an active tool loop.
#[derive(Clone, Serialize, Deserialize)]
pub struct Continuation {
    protocol: Protocol,
    endpoint: String,
    model: String,
    items: Vec<Value>,
    text: String,
    calls: Vec<ToolCall>,
    replayable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    System {
        text: String,
    },
    User {
        text: String,
    },
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
        continuation: Option<Continuation>,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_output_tokens: u32,
    /// Chat-compatible APIs differ on the budget field. None preserves the OpenAI default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_token_limit: Option<ChatTokenLimit>,
    pub temperature: Option<f64>,
    pub output_format: Option<ChatFormat>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    Stop,
    ToolCalls,
    Length,
    Refusal,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    /// Total generated tokens, including reasoning where the provider reports it separately.
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}
/// Deltas are provisional. Tool arguments are never executable until generate returns Ok.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    TextDelta { text: String },
    ToolArgumentsDelta { index: u32, delta: String },
}
pub struct Completion {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish: Finish,
    pub continuation: Continuation,
}
