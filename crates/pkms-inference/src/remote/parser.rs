use super::{sse::Frame, *};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
mod json;

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
    item_id: String,
    closed: bool,
}
pub(super) struct Parser {
    protocol: Protocol,
    limit: usize,
    text: String,
    calls: BTreeMap<u32, PartialCall>,
    blocks: BTreeMap<u32, Value>,
    closed: HashSet<u32>,
    usage: Usage,
    finish: Option<Finish>,
    done: bool,
    started: bool,
    id: Option<String>,
    reasoning: String,
    refusal: bool,
    activity: usize,
    gemini: Option<super::gemini::Parser>,
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, Error> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or(Error::InvalidResponse)
}
fn index(v: &Value, key: &str) -> Result<u32, Error> {
    v.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v < 1024)
        .ok_or(Error::InvalidResponse)
}
fn finish(reason: &str) -> Result<Finish, Error> {
    match reason {
        "stop" | "end_turn" | "stop_sequence" => Ok(Finish::Stop),
        "tool_calls" | "tool_use" => Ok(Finish::ToolCalls),
        "length" | "max_tokens" | "max_output_tokens" => Ok(Finish::Length),
        "content_filter" | "refusal" => Ok(Finish::Refusal),
        _ => Err(Error::UnsupportedOutput),
    }
}
impl Parser {
    pub fn new(protocol: Protocol, limit: usize) -> Self {
        Self {
            protocol,
            limit,
            text: String::new(),
            calls: BTreeMap::new(),
            blocks: BTreeMap::new(),
            closed: HashSet::new(),
            usage: Usage::default(),
            finish: None,
            done: false,
            started: false,
            id: None,
            reasoning: String::new(),
            refusal: false,
            activity: 0,
            gemini: (protocol == Protocol::Gemini).then(|| super::gemini::Parser::new(limit)),
        }
    }
    pub fn done(&self) -> bool {
        self.gemini
            .as_ref()
            .map_or(self.done, super::gemini::Parser::done)
    }
    pub fn activity(&self) -> usize {
        self.gemini
            .as_ref()
            .map_or(self.activity, super::gemini::Parser::activity)
    }
    pub fn end(&mut self) -> Result<(), Error> {
        self.gemini.as_mut().ok_or(Error::Incomplete)?.end()
    }
    fn text(
        &mut self,
        text: &str,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if !text.is_empty() {
            self.activity += text.len();
            self.text.push_str(text);
            emit(Event::TextDelta {
                text: text.to_owned(),
            })?;
        }
        Ok(())
    }
    fn call(&mut self, index: u32) -> Result<&mut PartialCall, Error> {
        if !self.calls.contains_key(&index) && self.calls.len() >= self.limit {
            return Err(Error::ResponseTooLarge);
        }
        Ok(self.calls.entry(index).or_default())
    }
    fn arguments(
        &mut self,
        index: u32,
        delta: &str,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let call = self.calls.get_mut(&index).ok_or(Error::InvalidResponse)?;
        if call.closed {
            return Err(Error::InvalidResponse);
        }
        call.arguments.push_str(delta);
        self.activity += delta.len();
        if !delta.is_empty() {
            emit(Event::ToolArgumentsDelta {
                index,
                delta: delta.to_owned(),
            })?;
        }
        Ok(())
    }
    fn usage(&mut self, value: &Value, input: &str, output: &str) -> Result<(), Error> {
        for (key, slot) in [
            (input, &mut self.usage.input_tokens),
            (output, &mut self.usage.output_tokens),
        ] {
            if let Some(value) = value.get(key).filter(|v| !v.is_null()) {
                let value = value.as_u64().ok_or(Error::InvalidResponse)?;
                if slot.is_some_and(|old| value < old) {
                    return Err(Error::InvalidResponse);
                }
                *slot = Some(value);
            }
        }
        Ok(())
    }
    pub fn frame(
        &mut self,
        frame: Frame,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.done {
            return Err(Error::InvalidResponse);
        }
        if self.protocol == Protocol::ChatCompletions && frame.data == "[DONE]" {
            if self.finish.is_none() {
                return Err(Error::Incomplete);
            }
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(&frame.data).map_err(|_| Error::InvalidResponse)?;
        if let Some(parser) = &mut self.gemini {
            if frame.event == "error" {
                return Err(Error::ApiError);
            }
            if !frame.event.is_empty() && frame.event != "message" {
                return Err(Error::InvalidResponse);
            }
            return parser.value(&value, emit);
        }
        if value.get("error").is_some() || value["type"] == "error" || frame.event == "error" {
            return Err(Error::ApiError);
        }
        if self.protocol != Protocol::ChatCompletions
            && !frame.event.is_empty()
            && Some(frame.event.as_str()) != value["type"].as_str()
        {
            return Err(Error::InvalidResponse);
        }
        match self.protocol {
            Protocol::ChatCompletions => self.chat(&value, emit),
            Protocol::Responses => self.responses(&value, emit),
            Protocol::AnthropicMessages => self.anthropic(&value, emit),
            Protocol::Gemini => Err(Error::InvalidResponse),
        }
    }
    fn chat(
        &mut self,
        v: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let id = string(v, "id")?;
        if self.id.as_ref().is_some_and(|old| old != id) {
            return Err(Error::InvalidResponse);
        }
        self.id = Some(id.to_owned());
        let choices = v["choices"].as_array().ok_or(Error::InvalidResponse)?;
        if choices.len() > 1 {
            return Err(Error::UnsupportedOutput);
        }
        if let Some(usage) = v.get("usage").filter(|v| !v.is_null()) {
            self.usage(usage, "prompt_tokens", "completion_tokens")?;
        }
        let Some(choice) = choices.first() else {
            return if self.finish.is_some() {
                Ok(())
            } else {
                Err(Error::InvalidResponse)
            };
        };
        if index(choice, "index")? != 0 || self.finish.is_some() {
            return Err(Error::InvalidResponse);
        }
        let delta = choice
            .get("delta")
            .filter(|v| v.is_object())
            .ok_or(Error::InvalidResponse)?;
        if delta.get("function_call").is_some_and(|v| !v.is_null()) {
            return Err(Error::UnsupportedOutput);
        }
        if let Some(text) = delta.get("content").filter(|v| !v.is_null()) {
            self.text(text.as_str().ok_or(Error::InvalidResponse)?, emit)?;
        }
        if let Some(reasoning) = delta.get("reasoning_content").filter(|v| !v.is_null()) {
            let text = reasoning.as_str().ok_or(Error::InvalidResponse)?;
            self.activity += text.len();
            self.reasoning.push_str(text);
        }
        if let Some(refusal) = delta.get("refusal").filter(|v| !v.is_null()) {
            self.refusal |= !refusal.as_str().ok_or(Error::InvalidResponse)?.is_empty();
        }
        if let Some(calls) = delta.get("tool_calls").filter(|v| !v.is_null()) {
            for item in calls.as_array().ok_or(Error::InvalidResponse)? {
                let i = index(item, "index")?;
                let call = self.call(i)?;
                if let Some(id) = item.get("id").filter(|v| !v.is_null()) {
                    let id = id.as_str().ok_or(Error::InvalidResponse)?;
                    if !call.id.is_empty() && call.id != id {
                        return Err(Error::InvalidResponse);
                    }
                    call.id = id.to_owned();
                }
                if item
                    .get("type")
                    .is_some_and(|v| !v.is_null() && v != "function")
                {
                    return Err(Error::UnsupportedOutput);
                }
                if let Some(function) = item.get("function") {
                    if let Some(name) = function.get("name").filter(|v| !v.is_null()) {
                        call.name
                            .push_str(name.as_str().ok_or(Error::InvalidResponse)?);
                    }
                    if let Some(arguments) = function.get("arguments").filter(|v| !v.is_null()) {
                        self.arguments(i, arguments.as_str().ok_or(Error::InvalidResponse)?, emit)?;
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
            self.finish = Some(finish(reason.as_str().ok_or(Error::InvalidResponse)?)?);
        }
        Ok(())
    }
    fn responses(
        &mut self,
        v: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if string(v, "type")? != "response.created" && !self.started {
            return Err(Error::InvalidResponse);
        }
        if let Some(response_id) = v.get("response_id")
            && response_id.as_str() != self.id.as_deref()
        {
            return Err(Error::InvalidResponse);
        }
        match string(v, "type")? {
            "response.created" => {
                if self.started {
                    return Err(Error::InvalidResponse);
                }
                self.started = true;
                self.id = Some(string(&v["response"], "id")?.to_owned());
            }
            "response.output_text.delta" => {
                let i = index(v, "output_index")?;
                let block = self.blocks.get(&i).ok_or(Error::InvalidResponse)?;
                if self.closed.contains(&i)
                    || block["type"] != "message"
                    || block["id"] != v["item_id"]
                {
                    return Err(Error::InvalidResponse);
                }
                self.text(string(v, "delta")?, emit)?;
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.activity += string(v, "delta")?.len();
            }
            "response.refusal.delta" => {
                self.refusal = true;
            }
            "response.output_item.added" => {
                let i = index(v, "output_index")?;
                if self.blocks.contains_key(&i) {
                    return Err(Error::InvalidResponse);
                }
                let item = &v["item"];
                match string(item, "type")? {
                    "function_call" => {
                        let call = self.call(i)?;
                        call.id = string(item, "call_id")?.to_owned();
                        call.name = string(item, "name")?.to_owned();
                        call.item_id = string(item, "id")?.to_owned();
                        call.arguments = string(item, "arguments")?.to_owned();
                    }
                    "message" | "reasoning" => {}
                    _ => return Err(Error::UnsupportedOutput),
                }
                self.blocks.insert(i, item.clone());
            }
            "response.function_call_arguments.delta" => {
                let i = index(v, "output_index")?;
                if self
                    .calls
                    .get(&i)
                    .is_none_or(|c| Some(c.item_id.as_str()) != v["item_id"].as_str())
                {
                    return Err(Error::InvalidResponse);
                }
                self.arguments(i, string(v, "delta")?, emit)?;
            }
            "response.function_call_arguments.done" => {
                let call = self
                    .calls
                    .get_mut(&index(v, "output_index")?)
                    .ok_or(Error::InvalidResponse)?;
                if call.closed
                    || call.item_id != string(v, "item_id")?
                    || call.arguments != string(v, "arguments")?
                {
                    return Err(Error::InvalidResponse);
                }
                call.closed = true;
            }
            "response.output_item.done" => {
                let i = index(v, "output_index")?;
                let old = self.blocks.get(&i).ok_or(Error::InvalidResponse)?;
                if !self.closed.insert(i)
                    || old["id"] != v["item"]["id"]
                    || old["type"] != v["item"]["type"]
                {
                    return Err(Error::InvalidResponse);
                }
                self.blocks.insert(i, v["item"].clone());
            }
            "response.completed" | "response.incomplete" => {
                let response = &v["response"];
                if !self.started || self.id.as_deref() != response["id"].as_str() {
                    return Err(Error::InvalidResponse);
                }
                let incomplete = v["type"] == "response.incomplete";
                if response["status"]
                    != if incomplete {
                        "incomplete"
                    } else {
                        "completed"
                    }
                {
                    return Err(Error::InvalidResponse);
                }
                self.usage(&response["usage"], "input_tokens", "output_tokens")?;
                if incomplete {
                    self.finish = Some(finish(string(&response["incomplete_details"], "reason")?)?);
                } else {
                    if self.closed.len() != self.blocks.len()
                        || self.calls.values().any(|c| !c.closed)
                    {
                        return Err(Error::Incomplete);
                    }
                    let output = response["output"]
                        .as_array()
                        .ok_or(Error::InvalidResponse)?;
                    if output.len() != self.blocks.len()
                        || output.iter().zip(self.blocks.values()).any(|(a, b)| a != b)
                    {
                        return Err(Error::InvalidResponse);
                    }
                    let mut text = String::new();
                    for item in output {
                        if item["type"] == "message" {
                            for part in item["content"].as_array().ok_or(Error::InvalidResponse)? {
                                match string(part, "type")? {
                                    "output_text" => text.push_str(string(part, "text")?),
                                    "refusal" => self.refusal = true,
                                    _ => return Err(Error::UnsupportedOutput),
                                }
                            }
                        }
                    }
                    if text != self.text {
                        return Err(Error::InvalidResponse);
                    }
                    self.finish = Some(if self.calls.is_empty() {
                        Finish::Stop
                    } else {
                        Finish::ToolCalls
                    });
                }
                self.done = true;
            }
            "response.failed" => return Err(Error::ApiError),
            // Semantic metadata and future events are ignored, but cannot complete a stream.
            _ => {}
        }
        Ok(())
    }
    fn anthropic(
        &mut self,
        v: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        match string(v, "type")? {
            "message_start" => {
                if self.started {
                    return Err(Error::InvalidResponse);
                }
                self.started = true;
                self.id = Some(string(&v["message"], "id")?.to_owned());
                self.usage(&v["message"]["usage"], "input_tokens", "output_tokens")?;
            }
            "content_block_start" => {
                if !self.started || self.finish.is_some() {
                    return Err(Error::InvalidResponse);
                }
                let i = index(v, "index")?;
                if self.blocks.contains_key(&i) {
                    return Err(Error::InvalidResponse);
                }
                let block = &v["content_block"];
                match string(block, "type")? {
                    "tool_use" => {
                        let call = self.call(i)?;
                        call.id = string(block, "id")?.to_owned();
                        call.name = string(block, "name")?.to_owned();
                        if block["input"] != json!({}) {
                            return Err(Error::InvalidResponse);
                        }
                    }
                    "text" => self.text(string(block, "text")?, emit)?,
                    "thinking" | "redacted_thinking" => {}
                    _ => return Err(Error::UnsupportedOutput),
                }
                self.blocks.insert(i, block.clone());
            }
            "content_block_delta" => {
                let i = index(v, "index")?;
                if self.closed.contains(&i) || self.finish.is_some() {
                    return Err(Error::InvalidResponse);
                }
                let block_type = self
                    .blocks
                    .get(&i)
                    .and_then(|b| b["type"].as_str())
                    .ok_or(Error::InvalidResponse)?
                    .to_owned();
                let delta = &v["delta"];
                match (block_type.as_str(), string(delta, "type")?) {
                    ("tool_use", "input_json_delta") => {
                        self.arguments(i, string(delta, "partial_json")?, emit)?
                    }
                    ("text", "text_delta") => {
                        let text = string(delta, "text")?;
                        self.text(text, emit)?;
                        let block = self.blocks.get_mut(&i).ok_or(Error::InvalidResponse)?;
                        block["text"] = json!(format!("{}{}", string(block, "text")?, text));
                    }
                    ("thinking", "thinking_delta") | ("thinking", "signature_delta") => {
                        let key = if delta["type"] == "thinking_delta" {
                            "thinking"
                        } else {
                            "signature"
                        };
                        self.activity += string(delta, key)?.len();
                        let block = self.blocks.get_mut(&i).ok_or(Error::InvalidResponse)?;
                        block[key] = json!(format!(
                            "{}{}",
                            block[key].as_str().unwrap_or_default(),
                            string(delta, key)?
                        ));
                    }
                    _ => return Err(Error::UnsupportedOutput),
                }
            }
            "content_block_stop" => {
                let i = index(v, "index")?;
                if !self.blocks.contains_key(&i) || !self.closed.insert(i) {
                    return Err(Error::InvalidResponse);
                }
                if let Some(call) = self.calls.get_mut(&i) {
                    // Empty-input tools may close without any parameter deltas.
                    if call.arguments.is_empty() {
                        call.arguments = "{}".into();
                    }
                    call.closed = true;
                }
            }
            "message_delta" => {
                if !self.started || self.closed.len() != self.blocks.len() {
                    return Err(Error::Incomplete);
                }
                if let Some(reason) = v["delta"].get("stop_reason").filter(|r| !r.is_null()) {
                    if self.finish.is_some() {
                        return Err(Error::InvalidResponse);
                    }
                    self.finish = Some(finish(reason.as_str().ok_or(Error::InvalidResponse)?)?);
                }
                self.usage(&v["usage"], "input_tokens", "output_tokens")?;
            }
            "message_stop" => {
                if !self.started || self.finish.is_none() || self.closed.len() != self.blocks.len()
                {
                    return Err(Error::Incomplete);
                }
                self.done = true;
            }
            _ => {}
        }
        Ok(())
    }
    pub fn complete(mut self, endpoint: &str, model: &str) -> Result<Completion, Error> {
        if let Some(parser) = self.gemini.take() {
            return parser.complete(endpoint, model);
        }
        if !self.done {
            return Err(Error::Incomplete);
        }
        let finish = if self.refusal {
            Finish::Refusal
        } else {
            self.finish.ok_or(Error::Incomplete)?
        };
        let mut calls = Vec::new();
        let mut ids = HashSet::new();
        // A truncated/refused response can contain partial tool JSON. Never expose it as a call.
        if matches!(finish, Finish::Stop | Finish::ToolCalls) {
            if (finish == Finish::ToolCalls) == self.calls.is_empty() {
                return Err(Error::InvalidResponse);
            }
            for (i, c) in &self.calls {
                if !request::identifier(&c.id) || !request::tool_name(&c.name) || !ids.insert(&c.id)
                {
                    return Err(Error::InvalidResponse);
                }
                let arguments: Value =
                    serde_json::from_str(&c.arguments).map_err(|_| Error::InvalidResponse)?;
                if !arguments.is_object() {
                    return Err(Error::InvalidResponse);
                }
                if self.protocol == Protocol::Responses {
                    let block = self.blocks.get(i).ok_or(Error::InvalidResponse)?;
                    if block["call_id"] != c.id
                        || block["name"] != c.name
                        || block["arguments"] != c.arguments
                    {
                        return Err(Error::InvalidResponse);
                    }
                } else if self.protocol == Protocol::AnthropicMessages {
                    self.blocks.get_mut(i).ok_or(Error::InvalidResponse)?["input"] =
                        arguments.clone();
                }
                calls.push(ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments,
                });
            }
        }
        let items = if matches!(finish, Finish::Length | Finish::Refusal) {
            Vec::new()
        } else if self.protocol == Protocol::ChatCompletions {
            let mut item = json!({"role":"assistant","content":self.text});
            if !calls.is_empty() {
                item["tool_calls"] = Value::Array(calls.iter().map(request::chat_call).collect());
            }
            if !self.reasoning.is_empty() {
                item["reasoning_content"] = json!(self.reasoning);
            }
            vec![item]
        } else {
            if self.protocol == Protocol::AnthropicMessages {
                for block in self.blocks.values() {
                    match block["type"].as_str() {
                        Some("thinking")
                            if block["signature"].as_str().is_none_or(str::is_empty) =>
                        {
                            return Err(Error::Incomplete);
                        }
                        Some("redacted_thinking")
                            if block["data"].as_str().is_none_or(str::is_empty) =>
                        {
                            return Err(Error::Incomplete);
                        }
                        _ => {}
                    }
                }
            }
            self.blocks.into_values().collect()
        };
        Ok(Completion {
            continuation: Continuation {
                protocol: self.protocol,
                endpoint: endpoint.to_owned(),
                model: model.to_owned(),
                items,
                text: self.text.clone(),
                calls: calls.clone(),
                replayable: matches!(finish, Finish::Stop | Finish::ToolCalls),
                channel_id: None,
            },
            text: self.text,
            tool_calls: calls,
            usage: self.usage,
            finish,
        })
    }
}
