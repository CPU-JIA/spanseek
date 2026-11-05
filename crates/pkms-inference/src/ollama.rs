//! Bounded, cancellation-aware transport for a local Ollama service.
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct StreamLimits {
    pub total_timeout: Duration,
    pub first_token_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_line_bytes: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            total_timeout: Duration::from_secs(300),
            first_token_timeout: Duration::from_secs(120),
            idle_timeout: Duration::from_secs(30),
            max_response_bytes: 4 * 1024 * 1024,
            max_line_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OllamaError {
    InvalidEndpoint,
    InvalidRequest,
    ContextLimit,
    Canceled,
    TotalTimeout,
    FirstTokenTimeout,
    IdleTimeout,
    Unavailable,
    ModelNotFound,
    ApiError,
    InvalidResponse,
    ResponseTooLarge,
    Incomplete,
}

impl OllamaError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidEndpoint => "model_invalid_endpoint",
            Self::InvalidRequest => "model_invalid_request",
            Self::ContextLimit => "model_context_limit",
            Self::Canceled => "model_canceled",
            Self::TotalTimeout => "model_total_timeout",
            Self::FirstTokenTimeout => "model_first_token_timeout",
            Self::IdleTimeout => "model_idle_timeout",
            Self::Unavailable => "model_unavailable",
            Self::ModelNotFound => "model_not_found",
            Self::ApiError => "model_api_error",
            Self::InvalidResponse => "model_invalid_response",
            Self::ResponseTooLarge => "model_response_too_large",
            Self::Incomplete => "model_incomplete",
        }
    }
}

impl std::fmt::Display for OllamaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for OllamaError {}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Serialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Serialize)]
pub struct ChatOptions {
    pub num_ctx: u32,
    pub num_predict: u32,
    pub temperature: f32,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            num_ctx: 2048,
            num_predict: 512,
            temperature: 0.0,
        }
    }
}

#[derive(Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub options: ChatOptions,
    #[serde(skip)]
    pub format: Option<crate::protocol::ChatFormat>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChatCompletion {
    pub done_reason: Option<String>,
    pub prompt_eval_count: Option<u64>,
    pub eval_count: Option<u64>,
}

pub struct OllamaClient {
    client: reqwest::Client,
    url: reqwest::Url,
    limits: StreamLimits,
}

#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    error: Option<serde_json::Value>,
    done: Option<bool>,
    message: Option<Message>,
    done_reason: Option<String>,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
}

#[derive(Deserialize)]
struct Message {
    content: String,
}

impl OllamaClient {
    /// V1 deliberately accepts only HTTP loopback endpoints, without credentials.
    pub fn new(endpoint: &str, limits: StreamLimits) -> Result<Self, OllamaError> {
        if endpoint.len() > 2048 || endpoint.chars().any(char::is_control) {
            return Err(OllamaError::InvalidEndpoint);
        }
        let mut url = reqwest::Url::parse(endpoint).map_err(|_| OllamaError::InvalidEndpoint)?;
        let host = url.host_str().unwrap_or_default();
        let loopback = host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback
            || url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(OllamaError::InvalidEndpoint);
        }
        if limits.total_timeout.is_zero()
            || limits.first_token_timeout.is_zero()
            || limits.idle_timeout.is_zero()
            || limits.max_line_bytes == 0
            || limits.max_response_bytes == 0
            || limits.max_line_bytes > limits.max_response_bytes
        {
            return Err(OllamaError::InvalidRequest);
        }
        // Avoid DNS for localhost: a changed hosts file must not redirect private prompts.
        if url.host_str() == Some("localhost") {
            url.set_host(Some("127.0.0.1"))
                .map_err(|_| OllamaError::InvalidEndpoint)?;
        }
        url.set_path("/api/chat");
        let client = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| OllamaError::Unavailable)?;
        Ok(Self {
            client,
            url,
            limits,
        })
    }

    /// Partial deltas are provisional. Only `Ok` means a `done:true` frame arrived.
    /// No request is retried. Dropping this future also drops the HTTP response.
    pub async fn chat(
        &self,
        request: &ChatRequest,
        mut on_delta: impl FnMut(&str) -> Result<(), OllamaError>,
        mut canceled: impl FnMut() -> bool,
    ) -> Result<ChatCompletion, OllamaError> {
        if request.model.is_empty()
            || request.model.len() > 256
            || request.model.chars().any(char::is_control)
            || request.messages.is_empty()
            || request.messages.len() > 128
            || request
                .messages
                .iter()
                .map(|m| m.content.len())
                .sum::<usize>()
                > 1024 * 1024
            || !(256..=32768).contains(&request.options.num_ctx)
            || !(1..=8192).contains(&request.options.num_predict)
            || !request.options.temperature.is_finite()
            || !(0.0..=2.0).contains(&request.options.temperature)
        {
            return Err(OllamaError::InvalidRequest);
        }
        // Conservative admission for the managed local chat profile. UTF-8
        // bytes upper-bound byte-level text tokens; reserve framing separately.
        // This is not an exact model-token count and must not be presented as one.
        let input_bound = crate::profile::chat_context_bound(
            request.messages.iter().map(|m| m.content.len()).sum(),
            request.messages.len(),
            request.options.num_predict,
        );
        if input_bound > request.options.num_ctx as usize {
            return Err(OllamaError::ContextLimit);
        }
        let format = request
            .format
            .as_ref()
            .map(crate::protocol::ChatFormat::json_schema)
            .transpose()
            .map_err(|_| OllamaError::InvalidRequest)?;
        #[derive(Serialize)]
        struct WireRequest<'a> {
            #[serde(flatten)]
            request: &'a ChatRequest,
            stream: bool,
            think: bool,
            keep_alive: u32,
            #[serde(skip_serializing_if = "Option::is_none")]
            format: Option<serde_json::Value>,
        }
        let started = Instant::now();
        let mut last_token = started;
        let mut has_token = false;
        let pending = self
            .client
            .post(self.url.clone())
            .json(&WireRequest {
                request,
                stream: true,
                think: false,
                keep_alive: 0,
                format,
            })
            .send();
        tokio::pin!(pending);
        let mut response = loop {
            self.check(started, last_token, has_token, &mut canceled)?;
            tokio::select! {
                result = &mut pending => break result.map_err(|_| OllamaError::Unavailable)?,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        };
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(OllamaError::ModelNotFound);
        }
        if !response.status().is_success() {
            return Err(OllamaError::ApiError);
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.limits.max_response_bytes as u64)
        {
            return Err(OllamaError::ResponseTooLarge);
        }
        let mut line = Vec::new();
        let mut total = 0usize;
        loop {
            let pending = response.chunk();
            tokio::pin!(pending);
            let chunk = loop {
                self.check(started, last_token, has_token, &mut canceled)?;
                tokio::select! {
                    result = &mut pending => break result.map_err(|_| OllamaError::Incomplete)?,
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                }
            };
            let Some(chunk) = chunk else {
                if !line.is_empty() {
                    self.check(started, last_token, has_token, &mut canceled)?;
                    if let Some(done) =
                        frame(&line, &mut on_delta, &mut has_token, &mut last_token)?
                    {
                        self.check(started, last_token, has_token, &mut canceled)?;
                        return Ok(done);
                    }
                }
                return Err(OllamaError::Incomplete);
            };
            total = total
                .checked_add(chunk.len())
                .ok_or(OllamaError::ResponseTooLarge)?;
            if total > self.limits.max_response_bytes {
                return Err(OllamaError::ResponseTooLarge);
            }
            for byte in chunk {
                if byte == b'\n' {
                    self.check(started, last_token, has_token, &mut canceled)?;
                    if !line.is_empty() {
                        if let Some(done) =
                            frame(&line, &mut on_delta, &mut has_token, &mut last_token)?
                        {
                            self.check(started, last_token, has_token, &mut canceled)?;
                            return Ok(done);
                        }
                        line.clear();
                    }
                } else {
                    if line.len() == self.limits.max_line_bytes {
                        return Err(OllamaError::ResponseTooLarge);
                    }
                    line.push(byte);
                }
            }
        }
    }

    fn check(
        &self,
        started: Instant,
        last_token: Instant,
        has_token: bool,
        canceled: &mut impl FnMut() -> bool,
    ) -> Result<(), OllamaError> {
        if canceled() {
            return Err(OllamaError::Canceled);
        }
        if started.elapsed() >= self.limits.total_timeout {
            return Err(OllamaError::TotalTimeout);
        }
        if !has_token && started.elapsed() >= self.limits.first_token_timeout {
            return Err(OllamaError::FirstTokenTimeout);
        }
        if has_token && last_token.elapsed() >= self.limits.idle_timeout {
            return Err(OllamaError::IdleTimeout);
        }
        Ok(())
    }
}

fn frame(
    line: &[u8],
    on_delta: &mut impl FnMut(&str) -> Result<(), OllamaError>,
    has_token: &mut bool,
    last_token: &mut Instant,
) -> Result<Option<ChatCompletion>, OllamaError> {
    let frame: Frame = serde_json::from_slice(line).map_err(|_| OllamaError::InvalidResponse)?;
    if frame.error.is_some() {
        return Err(OllamaError::ApiError);
    }
    let done = frame.done.ok_or(OllamaError::InvalidResponse)?;
    if let Some(message) = frame.message {
        if !message.content.is_empty() {
            on_delta(&message.content)?;
            *has_token = true;
            *last_token = Instant::now();
        }
    } else if !done {
        return Err(OllamaError::InvalidResponse);
    }
    if done {
        // Only protocol-defined reasons leave the transport; arbitrary server text is not metadata.
        let reason = match frame.done_reason.as_deref() {
            Some("stop") => Some("stop".to_owned()),
            Some("length") => Some("length".to_owned()),
            None => None,
            _ => return Err(OllamaError::InvalidResponse),
        };
        Ok(Some(ChatCompletion {
            done_reason: reason,
            prompt_eval_count: frame.prompt_eval_count,
            eval_count: frame.eval_count,
        }))
    } else {
        Ok(None)
    }
}
