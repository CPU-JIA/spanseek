//! Native Generate Content wire. Parts are opaque replay material, never flattened for tools.
mod parser;
pub(super) use parser::Parser;
#[cfg(test)]
mod tests;

use super::*;
use serde_json::json;
use std::collections::{HashMap, HashSet};

pub(super) fn model_name(model: &str) -> Result<&str, Error> {
    let name = model.strip_prefix("models/").unwrap_or(model);
    if name.is_empty()
        || name.len() > 256
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::InvalidRequest);
    }
    Ok(name)
}

pub(super) fn url(
    base: &str,
    model: &str,
    transport: GenerationTransport,
) -> Result<reqwest::Url, Error> {
    let model = model_name(model)?;
    let mut url = reqwest::Url::parse(base).map_err(|_| Error::InvalidEndpoint)?;
    if url
        .path_segments()
        .is_some_and(|mut parts| parts.any(|part| part == "models" || part.contains(':')))
    {
        return Err(Error::InvalidEndpoint);
    }
    let method = match transport {
        GenerationTransport::Stream => "streamGenerateContent",
        GenerationTransport::Json => "generateContent",
    };
    url.path_segments_mut()
        .map_err(|_| Error::InvalidEndpoint)?
        .pop_if_empty()
        .push("models")
        .push(&format!("{model}:{method}"));
    if transport == GenerationTransport::Stream {
        url.query_pairs_mut().append_pair("alt", "sse");
    }
    Ok(url)
}

struct Pending {
    call_id: String,
    name: String,
    provider_id: Option<String>,
}

pub(super) fn wire(
    endpoint: &str,
    channel_id: Option<&str>,
    request: &Request,
) -> Result<Value, Error> {
    model_name(&request.model)?;
    let mut system = Vec::new();
    let mut contents = Vec::new();
    let mut pending = Vec::<Pending>::new();
    let mut results = HashMap::new();
    let mut ids = HashSet::new();
    for message in &request.messages {
        if !matches!(message, Message::ToolResult { .. }) && !pending.is_empty() {
            return Err(Error::InvalidRequest);
        }
        match message {
            Message::System { text } => {
                if !contents.is_empty() {
                    return Err(Error::InvalidRequest);
                }
                system.push(json!({"text": text}));
            }
            Message::User { text } => {
                contents.push(json!({"role":"user","parts":[{"text":text}]}));
            }
            Message::Assistant {
                text,
                tool_calls,
                continuation,
            } => {
                if tool_calls.len() > 32 {
                    return Err(Error::InvalidRequest);
                }
                for call in tool_calls {
                    if !request::identifier(&call.id)
                        || !request::tool_name(&call.name)
                        || !call.arguments.is_object()
                        || !ids.insert(call.id.clone())
                    {
                        return Err(Error::InvalidRequest);
                    }
                }
                let parts = if let Some(saved) = continuation {
                    if !saved.replayable
                        || saved.protocol != Protocol::Gemini
                        || saved.endpoint != endpoint
                        || saved.model != request.model
                        || saved.channel_id.as_deref() != channel_id
                        || saved.text != *text
                        || saved.calls != *tool_calls
                    {
                        return Err(Error::InvalidRequest);
                    }
                    let mut visible = String::new();
                    let mut calls = tool_calls.iter();
                    for part in &saved.items {
                        validate_part(part).map_err(|_| Error::InvalidRequest)?;
                        if part["thought"] != true
                            && let Some(value) = part["text"].as_str()
                        {
                            visible.push_str(value);
                        }
                        if let Some(function) = part.get("functionCall") {
                            let call = calls.next().ok_or(Error::InvalidRequest)?;
                            let args = function.get("args").cloned().unwrap_or_else(|| json!({}));
                            let provider_id = function.get("id").and_then(Value::as_str);
                            if function["name"] != call.name
                                || args != call.arguments
                                || provider_id.is_some_and(|id| id != call.id)
                            {
                                return Err(Error::InvalidRequest);
                            }
                            pending.push(Pending {
                                call_id: call.id.clone(),
                                name: call.name.clone(),
                                provider_id: provider_id.map(str::to_owned),
                            });
                        }
                    }
                    if visible != *text || calls.next().is_some() || saved.items.is_empty() {
                        return Err(Error::InvalidRequest);
                    }
                    saved.items.clone()
                } else {
                    // A reconstructed functionCall cannot distinguish a provider ID from an
                    // internal one and cannot recover the original thought signature.
                    if !tool_calls.is_empty() || text.is_empty() {
                        return Err(Error::InvalidRequest);
                    }
                    vec![json!({"text":text})]
                };
                contents.push(json!({"role":"model","parts":parts}));
            }
            Message::ToolResult {
                call_id,
                content,
                is_error,
            } => {
                let call = pending
                    .iter()
                    .find(|call| call.call_id == *call_id)
                    .ok_or(Error::InvalidRequest)?;
                if results.contains_key(call_id) {
                    return Err(Error::InvalidRequest);
                }
                // Tool output may be plain text or JSON. Both have a fixed object envelope.
                let output = serde_json::from_str::<Value>(content)
                    .unwrap_or_else(|_| Value::String(content.clone()));
                let response = if *is_error {
                    json!({"error":output})
                } else {
                    json!({"output":output})
                };
                let mut function = json!({"name":call.name,"response":response});
                if let Some(id) = &call.provider_id {
                    function["id"] = json!(id);
                }
                results.insert(call_id.clone(), json!({"functionResponse":function}));
                if results.len() == pending.len() {
                    // Providers without IDs match by order. Reorder completed results to the
                    // original calls, including when two parallel calls share a function name.
                    let parts: Vec<Value> = pending
                        .drain(..)
                        .map(|call| results.remove(&call.call_id).ok_or(Error::InvalidRequest))
                        .collect::<Result<_, _>>()?;
                    contents.push(json!({"role":"user","parts":parts}));
                }
            }
        }
    }
    if !pending.is_empty() || contents.is_empty() {
        return Err(Error::InvalidRequest);
    }
    let mut config = json!({"candidateCount":1,"maxOutputTokens":request.max_output_tokens});
    if let Some(temperature) = request.temperature {
        config["temperature"] = json!(temperature);
    }
    if let Some(format) = &request.output_format {
        // REST structured-output guide as read 2026-10-09. Use this one documented wire;
        // do not mix deprecated responseSchema fields or retry without the format.
        config["responseFormat"] = json!({"text":{
            "mimeType":"application/json", "schema":request::evidence_schema(format)?
        }});
    }
    let mut body = json!({"contents":contents,"generationConfig":config});
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts":system});
    }
    if !request.tools.is_empty() {
        body["tools"] = json!([{"functionDeclarations":request.tools.iter().map(|tool| {
            json!({"name":tool.name,"description":tool.description,"parametersJsonSchema":tool.parameters})
        }).collect::<Vec<_>>()}]);
    }
    if serde_json::to_vec(&body)
        .map_err(|_| Error::InvalidRequest)?
        .len()
        > 2 * 1024 * 1024
    {
        return Err(Error::InvalidRequest);
    }
    Ok(body)
}

fn validate_part(part: &Value) -> Result<(), Error> {
    let object = part.as_object().ok_or(Error::InvalidResponse)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "text" | "functionCall" | "thought" | "thoughtSignature"
        )
    }) {
        return Err(Error::UnsupportedOutput);
    }
    if part.get("thought").is_some_and(|value| !value.is_boolean()) {
        return Err(Error::InvalidResponse);
    }
    if let Some(signature) = part.get("thoughtSignature")
        && signature.as_str().is_none_or(str::is_empty)
    {
        return Err(Error::InvalidResponse);
    }
    let text = part.get("text");
    let function = part.get("functionCall");
    if text.is_some() && function.is_some() {
        return Err(Error::InvalidResponse);
    }
    if let Some(text) = text {
        if !text.is_string() {
            return Err(Error::InvalidResponse);
        }
    } else if let Some(function) = function {
        let fields = function.as_object().ok_or(Error::InvalidResponse)?;
        if fields
            .keys()
            .any(|key| !matches!(key.as_str(), "id" | "name" | "args"))
        {
            return Err(Error::UnsupportedOutput);
        }
        if function["name"]
            .as_str()
            .is_none_or(|name| !request::tool_name(name))
            || function
                .get("id")
                .is_some_and(|id| id.as_str().is_none_or(|id| !request::identifier(id)))
            || function.get("args").is_some_and(|args| !args.is_object())
        {
            return Err(Error::InvalidResponse);
        }
    } else if !object.contains_key("thoughtSignature") {
        return Err(Error::InvalidResponse);
    }
    Ok(())
}
