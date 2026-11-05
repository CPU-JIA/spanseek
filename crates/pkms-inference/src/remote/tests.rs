use super::*;
use serde_json::json;

fn decode(
    protocol: Protocol,
    input: &str,
    chunk: usize,
) -> Result<(Completion, Vec<Event>), Error> {
    let mut parser = parser::Parser::new(protocol, 32);
    let mut sse = sse::Sse::new(256 * 1024);
    let mut events = Vec::new();
    for part in input.as_bytes().chunks(chunk) {
        sse.push(part, |f| {
            parser.frame(f, &mut |e| {
                events.push(e);
                Ok(())
            })
        })?;
    }
    if !sse.is_empty() {
        return Err(Error::Incomplete);
    }
    Ok((
        parser.complete("https://example.test/v1", "fixture")?,
        events,
    ))
}
fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn chat(delta: Value, reason: Value) -> String {
    frame(json!({"id":"chat1","choices":[{"index":0,"delta":delta,"finish_reason":reason}]}))
}
fn request() -> Request {
    Request {
        model: "fixture".into(),
        messages: vec![Message::User {
            text: "hello".into(),
        }],
        tools: vec![],
        max_output_tokens: 100,
        chat_token_limit: None,
        temperature: None,
        output_format: None,
    }
}

#[test]
fn chat_utf8_sse_and_usage_after_finish() {
    let input = format!(
        "\u{feff}: heartbeat\r\n\r\n{}{}{}data: [DONE]\r\n\r\n",
        chat(json!({"role":"assistant","content":"中🙂文"}), Value::Null),
        chat(json!({}), json!("stop")),
        frame(
            json!({"id":"chat1","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}})
        )
    );
    for chunk in [1, 2, 7, 4096] {
        let (result, events) = decode(Protocol::ChatCompletions, &input, chunk).unwrap();
        assert_eq!(result.text, "中🙂文");
        assert_eq!(result.finish, Finish::Stop);
        assert_eq!(result.usage.output_tokens, Some(3));
        assert_eq!(events.len(), 1);
    }
}
#[test]
fn chat_interleaved_calls_and_reasoning_replay() {
    let input = format!(
        "{}{}{}data: [DONE]\n\n",
        chat(
            json!({"reasoning_content":"private reasoning","tool_calls":[{"index":0,"id":"a","type":"function","function":{"name":"find","arguments":"{\"q\":"}},{"index":1,"id":"b","type":"function","function":{"name":"read","arguments":"{"}}]}),
            Value::Null
        ),
        chat(
            json!({"tool_calls":[{"index":1,"function":{"arguments":"\"id\":1}"}},{"index":0,"function":{"arguments":"\"中\"}"}}]}),
            Value::Null
        ),
        chat(json!({}), json!("tool_calls"))
    );
    let (result, events) = decode(Protocol::ChatCompletions, &input, 1).unwrap();
    assert_eq!(result.tool_calls.len(), 2);
    assert_eq!(result.tool_calls[0].arguments, json!({"q":"中"}));
    assert_eq!(result.finish, Finish::ToolCalls);
    assert!(result.text.is_empty());
    assert!(!events.iter().any(|e| matches!(e, Event::TextDelta { .. })));
    let mut req = request();
    req.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    req.messages.extend([
        Message::ToolResult {
            call_id: "a".into(),
            content: "found".into(),
            is_error: false,
        },
        Message::ToolResult {
            call_id: "b".into(),
            content: "read".into(),
            is_error: false,
        },
    ]);
    let body = request::wire(Protocol::ChatCompletions, "https://example.test/v1", &req).unwrap();
    assert_eq!(
        body["messages"][1]["reasoning_content"],
        "private reasoning"
    );
    assert!(request::wire(Protocol::Responses, "https://example.test/v1", &req).is_err());
    assert!(request::wire(Protocol::ChatCompletions, "https://other.test/v1", &req).is_err());
}
#[test]
fn incomplete_or_invalid_chat_never_delivers_complete_calls() {
    for input in [
        chat(json!({"content":"partial"}), Value::Null),
        "data: [DONE]\n\n".into(),
        format!(
            "{}data: [DONE]\n\n",
            chat(
                json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"find","arguments":"{"}}]}),
                json!("tool_calls")
            )
        ),
    ] {
        assert!(decode(Protocol::ChatCompletions, &input, 2).is_err());
    }
    let input = format!(
        "{}data: [DONE]\n\n",
        chat(
            json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"find","arguments":"{"}}]}),
            json!("length")
        )
    );
    let result = decode(Protocol::ChatCompletions, &input, 4).unwrap().0;
    assert_eq!(result.finish, Finish::Length);
    assert!(result.tool_calls.is_empty());
}
#[test]
fn chat_rejects_extra_choices_changed_id_duplicate_tool_ids_and_unsignaled_calls() {
    let a = json!({"id":"x","choices":[{"index":0,"delta":{}},{"index":1,"delta":{}}]});
    assert!(decode(Protocol::ChatCompletions, &frame(a), 10).is_err());
    let input = format!(
        "{}{}",
        chat(json!({}), Value::Null),
        frame(json!({"id":"other","choices":[]}))
    );
    assert!(decode(Protocol::ChatCompletions, &input, 100).is_err());
    for reason in ["tool_calls", "stop"] {
        let input = format!(
            "{}data: [DONE]\n\n",
            chat(
                json!({"tool_calls":[{"index":0,"id":"same","function":{"name":"find","arguments":"{}"}},{"index":1,"id":"same","function":{"name":"read","arguments":"{}"}}]}),
                json!(reason)
            )
        );
        assert!(decode(Protocol::ChatCompletions, &input, 10).is_err());
    }
}
fn response_fixture() -> String {
    let reasoning = json!({"id":"r1","type":"reasoning","summary":[],"encrypted_content":"opaque"});
    let call_start =
        json!({"id":"f1","type":"function_call","call_id":"c1","name":"find","arguments":""});
    let call_done = json!({"id":"f1","type":"function_call","call_id":"c1","name":"find","arguments":"{\"q\":\"x\"}"});
    [json!({"type":"response.created","response":{"id":"resp1"}}),json!({"type":"response.output_item.added","output_index":0,"item":reasoning}),json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),json!({"type":"response.output_item.added","output_index":1,"item":call_start}),json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"f1","delta":"{\"q\":\"x\"}"}),json!({"type":"response.function_call_arguments.done","output_index":1,"item_id":"f1","arguments":"{\"q\":\"x\"}"}),json!({"type":"response.output_item.done","output_index":1,"item":call_done}),json!({"type":"response.completed","response":{"id":"resp1","status":"completed","output":[reasoning,call_done],"usage":{"input_tokens":4,"output_tokens":8}}})].into_iter().map(frame).collect()
}
#[test]
fn responses_correlates_call_id_item_id_and_preserves_reasoning() {
    let (result, _) = decode(Protocol::Responses, &response_fixture(), 1).unwrap();
    assert_eq!(result.tool_calls[0].id, "c1");
    assert_eq!(result.finish, Finish::ToolCalls);
    let mut req = request();
    req.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    req.messages.push(Message::ToolResult {
        call_id: "c1".into(),
        content: "result".into(),
        is_error: false,
    });
    let body = request::wire(Protocol::Responses, "https://example.test/v1", &req).unwrap();
    assert_eq!(body["input"][1]["encrypted_content"], "opaque");
    assert_eq!(body["input"][3]["call_id"], "c1");
    assert_eq!(body["store"], false);
    assert!(
        decode(
            Protocol::Responses,
            &response_fixture().replace("\"item_id\":\"f1\"", "\"item_id\":\"wrong\""),
            10
        )
        .is_err()
    );
}
#[test]
fn responses_normal_text_and_failed_incomplete_are_distinct() {
    let start = json!({"id":"m1","type":"message","role":"assistant","content":[]});
    let done = json!({"id":"m1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]});
    let input: String=[json!({"type":"response.created","response":{"id":"r"}}),json!({"type":"response.output_item.added","output_index":0,"item":start}),json!({"type":"response.output_text.delta","output_index":0,"item_id":"m1","delta":"hello"}),json!({"type":"response.output_item.done","output_index":0,"item":done}),json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[done]}})].into_iter().map(frame).collect();
    assert_eq!(
        decode(Protocol::Responses, &input, 3).unwrap().0.text,
        "hello"
    );
    assert!(
        decode(
            Protocol::Responses,
            &frame(json!({"type":"response.failed","response":{"error":{"message":"private"}}})),
            1
        )
        .is_err()
    );
    let input = frame(json!({"type":"response.created","response":{"id":"r"}}))
        + &frame(
            json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        );
    assert_eq!(
        decode(Protocol::Responses, &input, 7).unwrap().0.finish,
        Finish::Length
    );
}
fn anthropic_fixture() -> String {
    [json!({"type":"message_start","message":{"id":"m1","usage":{"input_tokens":9,"output_tokens":1}}}),json!({"type":"ping"}),json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private"}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),json!({"type":"content_block_stop","index":0}),json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t1","name":"find","input":{}}}),json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"中\"}"}}),json!({"type":"content_block_stop","index":1}),json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),json!({"type":"message_stop"})].into_iter().map(frame).collect()
}
#[test]
fn anthropic_tool_json_signature_and_usage_are_preserved() {
    let (result, events) = decode(Protocol::AnthropicMessages, &anthropic_fixture(), 1).unwrap();
    assert_eq!(result.finish, Finish::ToolCalls);
    assert_eq!(
        result.usage,
        Usage {
            input_tokens: Some(9),
            output_tokens: Some(7),
            reasoning_tokens: None,
        }
    );
    assert_eq!(events.len(), 1);
    assert!(result.text.is_empty());
    let mut req = request();
    req.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    req.messages.push(Message::ToolResult {
        call_id: "t1".into(),
        content: "ok".into(),
        is_error: false,
    });
    let wire = request::wire(Protocol::AnthropicMessages, "https://example.test/v1", &req).unwrap();
    assert_eq!(wire["messages"][1]["content"][0]["signature"], "signature");
    assert_eq!(
        wire["messages"][1]["content"][1]["input"],
        json!({"q":"中"})
    );
    assert_eq!(wire["messages"][2]["content"][0]["tool_use_id"], "t1");
}
#[test]
fn anthropic_requires_closed_blocks_and_message_stop() {
    let input = anthropic_fixture();
    let stop = frame(json!({"type":"message_stop"}));
    assert!(decode(Protocol::AnthropicMessages, &input.replace(&stop, ""), 10).is_err());
    assert!(
        decode(
            Protocol::AnthropicMessages,
            &input.replace(&frame(json!({"type":"content_block_stop","index":1})), ""),
            10
        )
        .is_err()
    );
    let input = input.replace("input_json_delta", "text_delta");
    assert!(decode(Protocol::AnthropicMessages, &input, 10).is_err());
}
#[test]
fn sse_multiline_cr_and_mismatched_events() {
    let mut sse = sse::Sse::new(100);
    let mut frames = Vec::new();
    sse.push(b": ping\r\rdata: one\rdata: two\r\r", |f| {
        frames.push(f);
        Ok(())
    })
    .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data, "one\ntwo");
    let mut sse = sse::Sse::new(5);
    assert_eq!(
        sse.push(b"data: too large", |_| Ok(())),
        Err(Error::ResponseTooLarge)
    );
    let mut sse = sse::Sse::new(100);
    assert_eq!(
        sse.push(b"data: \xff\n\n", |_| Ok(())),
        Err(Error::InvalidResponse)
    );
    assert!(
        decode(
            Protocol::AnthropicMessages,
            "event: message_stop\ndata: {\"type\":\"ping\"}\n\n",
            1
        )
        .is_err()
    );
}
#[test]
fn request_three_protocols_have_exact_tool_and_format_shapes() {
    let mut req = request();
    req.messages.insert(
        0,
        Message::System {
            text: "system".into(),
        },
    );
    req.tools.push(ToolDefinition {
        name: "find".into(),
        description: "search".into(),
        parameters: json!({"type":"object","properties":{}}),
    });
    req.output_format = Some(ChatFormat::EvidenceAnswer {
        citation_ids: vec!["E1".into()],
    });
    for protocol in [
        Protocol::ChatCompletions,
        Protocol::Responses,
        Protocol::AnthropicMessages,
    ] {
        let body = request::wire(protocol, "https://example.test/v1", &req).unwrap();
        assert!(body["temperature"].is_null());
        match protocol {
            Protocol::ChatCompletions => {
                assert_eq!(body["tools"][0]["function"]["name"], "find");
                assert_eq!(body["max_completion_tokens"], 100);
                assert_eq!(body["response_format"]["type"], "json_object");
                assert!(
                    body["messages"][0]["content"]
                        .as_str()
                        .unwrap()
                        .contains("E1")
                );
            }
            Protocol::Responses => {
                assert_eq!(body["tools"][0]["name"], "find");
                assert_eq!(body["text"]["format"]["type"], "json_schema");
                let schema = &body["text"]["format"]["schema"];
                assert_eq!(schema["type"], "object");
                assert_eq!(schema["additionalProperties"], false);
                assert_eq!(
                    schema["properties"]["claims"]["items"]["required"],
                    json!(["text", "citation_ids"])
                );
                assert_eq!(
                    schema["properties"]["claims"]["items"]["properties"]["citation_ids"]["items"]
                        ["enum"],
                    json!(["E1"])
                );
                for unsupported in ["anyOf", "uniqueItems", "const", "pattern", "minLength"] {
                    assert!(!schema.to_string().contains(unsupported));
                }
            }
            Protocol::AnthropicMessages => {
                assert_eq!(body["system"][1]["text"], "system");
                assert!(body["system"][0]["text"].as_str().unwrap().contains("E1"));
                assert_eq!(body["messages"].as_array().unwrap().len(), 1);
                assert!(body.get("output_config").is_none());
            }
            Protocol::Gemini => unreachable!("covered by native Gemini tests"),
        }
    }
}
#[test]
fn request_rejects_missing_duplicate_results_and_invalid_definitions() {
    let mut req = request();
    req.messages.push(Message::ToolResult {
        call_id: "orphan".into(),
        content: "ok".into(),
        is_error: false,
    });
    assert!(request::wire(Protocol::Responses, "x", &req).is_err());
    let mut req = request();
    req.messages.push(Message::Assistant {
        text: "".into(),
        tool_calls: vec![ToolCall {
            id: "c".into(),
            name: "find".into(),
            arguments: json!({}),
        }],
        continuation: None,
    });
    assert!(request::wire(Protocol::Responses, "x", &req).is_err());
    let mut req = request();
    req.temperature = Some(f64::NAN);
    assert!(request::wire(Protocol::Responses, "x", &req).is_err());
}

#[test]
fn chat_token_budget_is_explicit_and_mutually_exclusive() {
    let mut req = request();
    req.max_output_tokens = 2048;
    for (choice, expected, absent) in [
        (None, "max_completion_tokens", "max_tokens"),
        (
            Some(ChatTokenLimit::MaxCompletionTokens),
            "max_completion_tokens",
            "max_tokens",
        ),
        (
            Some(ChatTokenLimit::MaxTokens),
            "max_tokens",
            "max_completion_tokens",
        ),
    ] {
        req.chat_token_limit = choice;
        let body =
            request::wire(Protocol::ChatCompletions, "https://example.test/v1", &req).unwrap();
        assert_eq!(body[expected], 2048);
        assert!(body.get(absent).is_none());
        assert!(body.get("max_output_tokens").is_none());
    }
    req.chat_token_limit = None;
    let legacy = serde_json::to_value(&req).unwrap();
    assert!(legacy.get("chat_token_limit").is_none());
    let loaded: Request = serde_json::from_value(legacy).unwrap();
    assert_eq!(loaded.chat_token_limit, None);
}

#[test]
fn chat_token_selector_is_rejected_for_other_protocols() {
    let mut req = request();
    for protocol in [Protocol::Responses, Protocol::AnthropicMessages] {
        for value in [
            ChatTokenLimit::MaxTokens,
            ChatTokenLimit::MaxCompletionTokens,
        ] {
            req.chat_token_limit = Some(value);
            assert_eq!(
                request::wire(protocol, "https://example.test/v1", &req),
                Err(Error::InvalidRequest)
            );
        }
        req.chat_token_limit = None;
        let body = request::wire(protocol, "https://example.test/v1", &req).unwrap();
        assert_eq!(
            body[if protocol == Protocol::Responses {
                "max_output_tokens"
            } else {
                "max_tokens"
            }],
            100
        );
        assert!(body.get("max_completion_tokens").is_none());
    }
}
