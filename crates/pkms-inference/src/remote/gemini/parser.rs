use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TURN: AtomicU64 = AtomicU64::new(1);

pub(in super::super) struct Parser {
    limit: usize,
    turn: u64,
    text: String,
    calls: Vec<ToolCall>,
    parts: Vec<Value>,
    finish: Option<Finish>,
    response_id: Option<String>,
    candidate_tokens: Option<u64>,
    total_tokens: Option<u64>,
    usage: Usage,
    activity: usize,
    eof: bool,
}

impl Parser {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            turn: NEXT_TURN.fetch_add(1, Ordering::Relaxed),
            text: String::new(),
            calls: Vec::new(),
            parts: Vec::new(),
            finish: None,
            response_id: None,
            candidate_tokens: None,
            total_tokens: None,
            usage: Usage::default(),
            activity: 0,
            eof: false,
        }
    }
    pub fn activity(&self) -> usize {
        self.activity
    }
    pub fn done(&self) -> bool {
        self.eof
    }

    pub fn value(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.eof || !value.is_object() {
            return Err(Error::InvalidResponse);
        }
        if value.get("error").is_some_and(|value| !value.is_null()) {
            return Err(Error::ApiError);
        }
        if let Some(id) = value.get("responseId") {
            let id = id
                .as_str()
                .filter(|id| request::identifier(id))
                .ok_or(Error::InvalidResponse)?;
            if self.response_id.as_deref().is_some_and(|old| old != id) {
                return Err(Error::InvalidResponse);
            }
            self.response_id = Some(id.to_owned());
        }
        let mut metadata = false;
        if let Some(usage) = value.get("usageMetadata") {
            if !usage.is_object() {
                return Err(Error::InvalidResponse);
            }
            for (key, slot) in [
                ("promptTokenCount", &mut self.usage.input_tokens),
                ("candidatesTokenCount", &mut self.candidate_tokens),
                ("thoughtsTokenCount", &mut self.usage.reasoning_tokens),
                ("totalTokenCount", &mut self.total_tokens),
            ] {
                if let Some(value) = usage.get(key) {
                    let count = value.as_u64().ok_or(Error::InvalidResponse)?;
                    if slot.is_some_and(|old| count < old) {
                        return Err(Error::InvalidResponse);
                    }
                    *slot = Some(count);
                }
            }
            if self.candidate_tokens.is_some() || self.usage.reasoning_tokens.is_some() {
                self.usage.output_tokens = Some(
                    self.candidate_tokens
                        .unwrap_or_default()
                        .checked_add(self.usage.reasoning_tokens.unwrap_or_default())
                        .ok_or(Error::InvalidResponse)?,
                );
            }
            metadata = true;
        }
        if let Some(feedback) = value.get("promptFeedback") {
            if !feedback.is_object() {
                return Err(Error::InvalidResponse);
            }
            if let Some(reason) = feedback.get("blockReason") {
                let reason = reason.as_str().ok_or(Error::InvalidResponse)?;
                if !matches!(
                    reason,
                    "SAFETY" | "OTHER" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "IMAGE_SAFETY"
                ) {
                    return Err(Error::UnsupportedOutput);
                }
                if self.finish.is_some()
                    || !self.parts.is_empty()
                    || value
                        .get("candidates")
                        .is_some_and(|items| items.as_array().is_none_or(|items| !items.is_empty()))
                {
                    return Err(Error::InvalidResponse);
                }
                self.finish = Some(Finish::Refusal);
                self.activity += 1;
                return Ok(());
            }
            metadata = true;
        }
        let candidates = match value.get("candidates") {
            Some(items) => items.as_array().ok_or(Error::InvalidResponse)?,
            None if metadata => return Ok(()),
            None => return Err(Error::InvalidResponse),
        };
        if candidates.len() > 1 {
            return Err(Error::UnsupportedOutput);
        }
        let Some(candidate) = candidates.first() else {
            return if metadata {
                Ok(())
            } else {
                Err(Error::InvalidResponse)
            };
        };
        if !candidate.is_object()
            || candidate
                .get("index")
                .is_some_and(|index| index.as_u64() != Some(0))
        {
            return Err(Error::InvalidResponse);
        }
        if let Some(content) = candidate.get("content") {
            if !content.is_object() || content.get("role").is_some_and(|role| role != "model") {
                return Err(Error::InvalidResponse);
            }
            let parts = content["parts"].as_array().ok_or(Error::InvalidResponse)?;
            for part in parts {
                validate_part(part)?;
                // A terminal candidate may be followed by signature-only/usage chunks.
                // Newly generated text or a call after its terminal reason is inconsistent.
                if self.finish.is_some()
                    && (part.get("functionCall").is_some()
                        || part["text"].as_str().is_some_and(|text| !text.is_empty()))
                {
                    return Err(Error::InvalidResponse);
                }
                if let Some(text) = part["text"].as_str() {
                    self.activity += text.len();
                    if part["thought"] != true && !text.is_empty() {
                        self.text.push_str(text);
                        emit(Event::TextDelta {
                            text: text.to_owned(),
                        })?;
                    }
                }
                if part.get("thoughtSignature").is_some() {
                    self.activity += 1;
                }
                if let Some(function) = part.get("functionCall") {
                    if self.calls.len() >= self.limit {
                        return Err(Error::ResponseTooLarge);
                    }
                    let id = function["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!("pkms_gemini_{}_{}", self.turn, self.calls.len())
                        });
                    if self.calls.iter().any(|call| call.id == id) {
                        return Err(Error::InvalidResponse);
                    }
                    let arguments = function.get("args").cloned().unwrap_or_else(|| json!({}));
                    self.activity += 1;
                    emit(Event::ToolArgumentsDelta {
                        index: self.calls.len() as u32,
                        delta: arguments.to_string(),
                    })?;
                    self.calls.push(ToolCall {
                        id,
                        name: function["name"]
                            .as_str()
                            .ok_or(Error::InvalidResponse)?
                            .to_owned(),
                        arguments,
                    });
                }
                self.parts.push(part.clone());
            }
        }
        if let Some(reason) = candidate.get("finishReason") {
            let finish = match reason.as_str().ok_or(Error::InvalidResponse)? {
                "STOP" => Finish::Stop,
                "MAX_TOKENS" => Finish::Length,
                "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII"
                | "IMAGE_SAFETY" => Finish::Refusal,
                "FINISH_REASON_UNSPECIFIED" => return Ok(()),
                "MALFORMED_FUNCTION_CALL"
                | "UNEXPECTED_TOOL_CALL"
                | "MISSING_THOUGHT_SIGNATURE" => return Err(Error::InvalidResponse),
                _ => return Err(Error::UnsupportedOutput),
            };
            if self.finish.is_some_and(|old| old != finish) {
                return Err(Error::InvalidResponse);
            }
            self.finish = Some(finish);
            self.activity += 1;
        }
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), Error> {
        if self.eof || self.finish.is_none() {
            return Err(Error::Incomplete);
        }
        self.eof = true;
        Ok(())
    }

    pub fn complete(mut self, endpoint: &str, model: &str) -> Result<Completion, Error> {
        if !self.eof {
            return Err(Error::Incomplete);
        }
        let mut finish = self.finish.ok_or(Error::Incomplete)?;
        if finish == Finish::Stop {
            if !self.calls.is_empty() {
                finish = Finish::ToolCalls;
            } else if self.text.trim().is_empty() {
                return Err(Error::InvalidResponse);
            }
        }
        let replayable = matches!(finish, Finish::Stop | Finish::ToolCalls);
        if !replayable {
            self.calls.clear();
            self.parts.clear();
        }
        Ok(Completion {
            continuation: Continuation {
                protocol: Protocol::Gemini,
                endpoint: endpoint.to_owned(),
                model: model.to_owned(),
                items: self.parts,
                text: self.text.clone(),
                calls: self.calls.clone(),
                replayable,
                channel_id: None,
            },
            text: self.text,
            tool_calls: self.calls,
            usage: self.usage,
            finish,
        })
    }
}
