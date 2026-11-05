//! Complete JSON responses use the same final tool/continuation validation as SSE.
use super::*;

impl Parser {
    pub fn json(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.started || self.done || !value.is_object() {
            return Err(Error::InvalidResponse);
        }
        if value.get("error").is_some_and(|error| !error.is_null())
            || value.get("success").is_some_and(|success| success != true)
            || value
                .get("errors")
                .is_some_and(|errors| errors.as_array().is_none_or(|errors| !errors.is_empty()))
        {
            return Err(Error::ApiError);
        }
        if let Some(parser) = &mut self.gemini {
            parser.value(value, emit)?;
            return parser.end();
        }
        if !request::identifier(string(value, "id")?) {
            return Err(Error::InvalidResponse);
        }
        match self.protocol {
            Protocol::ChatCompletions => self.chat_json(value, emit)?,
            Protocol::Responses => self.responses_json(value, emit)?,
            Protocol::AnthropicMessages => self.anthropic_json(value, emit)?,
            Protocol::Gemini => return Err(Error::InvalidResponse),
        }
        if matches!(self.finish, Some(Finish::Stop | Finish::ToolCalls))
            && !self.refusal
            && self.text.trim().is_empty()
            && self.calls.is_empty()
        {
            return Err(Error::InvalidResponse);
        }
        self.done = true;
        Ok(())
    }

    fn json_usage(&mut self, value: &Value, input: &str, output: &str) -> Result<(), Error> {
        if let Some(usage) = value.get("usage").filter(|value| !value.is_null()) {
            if !usage.is_object() {
                return Err(Error::InvalidResponse);
            }
            self.usage(usage, input, output)?;
        }
        Ok(())
    }

    fn chat_json(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if value
            .get("object")
            .is_some_and(|object| object != "chat.completion")
        {
            return Err(Error::InvalidResponse);
        }
        self.id = Some(string(value, "id")?.to_owned());
        let choices = value["choices"].as_array().ok_or(Error::InvalidResponse)?;
        if choices.len() != 1 {
            return Err(Error::InvalidResponse);
        }
        let choice = &choices[0];
        if index(choice, "index")? != 0 {
            return Err(Error::InvalidResponse);
        }
        let message = choice
            .get("message")
            .filter(|message| message.is_object())
            .ok_or(Error::InvalidResponse)?;
        if message["role"] != "assistant" {
            return Err(Error::InvalidResponse);
        }
        for field in ["function_call", "audio"] {
            if message.get(field).is_some_and(|value| !value.is_null()) {
                return Err(Error::UnsupportedOutput);
            }
        }
        if let Some(text) = message.get("content").filter(|value| !value.is_null()) {
            self.text(text.as_str().ok_or(Error::UnsupportedOutput)?, emit)?;
        }
        if let Some(reasoning) = message
            .get("reasoning_content")
            .filter(|value| !value.is_null())
        {
            self.reasoning = reasoning.as_str().ok_or(Error::InvalidResponse)?.to_owned();
        }
        if let Some(refusal) = message.get("refusal").filter(|value| !value.is_null()) {
            self.refusal = !refusal.as_str().ok_or(Error::InvalidResponse)?.is_empty();
        }
        if let Some(calls) = message.get("tool_calls").filter(|value| !value.is_null()) {
            for (i, item) in calls
                .as_array()
                .ok_or(Error::InvalidResponse)?
                .iter()
                .enumerate()
            {
                if item["type"] != "function" {
                    return Err(Error::UnsupportedOutput);
                }
                let call = self.call(u32::try_from(i).map_err(|_| Error::ResponseTooLarge)?)?;
                call.id = string(item, "id")?.to_owned();
                call.name = string(&item["function"], "name")?.to_owned();
                call.arguments = string(&item["function"], "arguments")?.to_owned();
                call.closed = true;
            }
        }
        self.finish = Some(finish(string(choice, "finish_reason")?)?);
        self.json_usage(value, "prompt_tokens", "completion_tokens")
    }

    fn responses_json(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if value
            .get("object")
            .is_some_and(|object| object != "response")
        {
            return Err(Error::InvalidResponse);
        }
        self.id = Some(string(value, "id")?.to_owned());
        let incomplete = match string(value, "status")? {
            "completed" => false,
            "incomplete" => true,
            "failed" => return Err(Error::ApiError),
            "cancelled" | "canceled" => return Err(Error::Canceled),
            "queued" | "in_progress" => return Err(Error::Incomplete),
            _ => return Err(Error::InvalidResponse),
        };
        let output = value["output"]
            .as_array()
            .filter(|items| items.len() <= 1024)
            .ok_or(Error::InvalidResponse)?;
        let mut ids = HashSet::new();
        for (i, item) in output.iter().enumerate() {
            let id = string(item, "id")?;
            if !request::identifier(id) || !ids.insert(id) {
                return Err(Error::InvalidResponse);
            }
            if !incomplete
                && item
                    .get("status")
                    .is_some_and(|status| status != "completed")
            {
                return Err(Error::Incomplete);
            }
            match string(item, "type")? {
                "message" => {
                    if item["role"] != "assistant" {
                        return Err(Error::InvalidResponse);
                    }
                    for part in item["content"].as_array().ok_or(Error::InvalidResponse)? {
                        match string(part, "type")? {
                            "output_text" => self.text(string(part, "text")?, emit)?,
                            "refusal" => {
                                string(part, "refusal")?;
                                self.refusal = true;
                            }
                            _ => return Err(Error::UnsupportedOutput),
                        }
                    }
                }
                "function_call" => {
                    let call = self.call(i as u32)?;
                    call.id = string(item, "call_id")?.to_owned();
                    call.name = string(item, "name")?.to_owned();
                    call.arguments = string(item, "arguments")?.to_owned();
                    call.item_id = id.to_owned();
                    call.closed = true;
                }
                "reasoning" => {}
                _ => return Err(Error::UnsupportedOutput),
            }
            self.blocks.insert(i as u32, item.clone());
            self.closed.insert(i as u32);
        }
        self.finish = Some(if incomplete {
            let reason = finish(string(&value["incomplete_details"], "reason")?)?;
            if !matches!(reason, Finish::Length | Finish::Refusal) {
                return Err(Error::InvalidResponse);
            }
            reason
        } else if self.calls.is_empty() {
            Finish::Stop
        } else {
            Finish::ToolCalls
        });
        self.json_usage(value, "input_tokens", "output_tokens")
    }

    fn anthropic_json(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if value["type"] != "message" || value["role"] != "assistant" {
            return Err(Error::InvalidResponse);
        }
        self.id = Some(string(value, "id")?.to_owned());
        let content = value["content"]
            .as_array()
            .filter(|items| items.len() <= 1024)
            .ok_or(Error::InvalidResponse)?;
        for (i, block) in content.iter().enumerate() {
            match string(block, "type")? {
                "text" => self.text(string(block, "text")?, emit)?,
                "tool_use" => {
                    let call = self.call(i as u32)?;
                    call.id = string(block, "id")?.to_owned();
                    call.name = string(block, "name")?.to_owned();
                    if !block["input"].is_object() {
                        return Err(Error::InvalidResponse);
                    }
                    call.arguments = block["input"].to_string();
                    call.closed = true;
                }
                "thinking" => {
                    string(block, "thinking")?;
                    string(block, "signature")?;
                }
                "redacted_thinking" => {
                    string(block, "data")?;
                }
                _ => return Err(Error::UnsupportedOutput),
            }
            self.blocks.insert(i as u32, block.clone());
        }
        self.finish = Some(finish(string(value, "stop_reason")?)?);
        self.json_usage(value, "input_tokens", "output_tokens")
    }
}
