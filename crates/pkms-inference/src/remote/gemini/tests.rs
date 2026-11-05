//! Synthetic wire fixtures only; these are not real Gemini model acceptance results.
use super::*;
use crate::remote::{parser, sse::Sse};

const BASE: &str = "https://example.test/v1beta";
const MODEL: &str = "gemini-fixture";
fn input() -> Request {
    Request {
        model: MODEL.into(),
        messages: vec![Message::User {
            text: "synthetic question".into(),
        }],
        tools: vec![ToolDefinition {
            name: "find".into(),
            description: "synthetic tool".into(),
            parameters: json!({"type":"object","properties":{}}),
        }],
        max_output_tokens: 64,
        chat_token_limit: None,
        temperature: None,
        output_format: None,
    }
}
fn candidate(parts: Value, finish: Option<&str>) -> Value {
    let mut value = json!({"responseId":"synthetic-response","candidates":[{"index":0,"content":{"role":"model","parts":parts}}]});
    if let Some(finish) = finish {
        value["candidates"][0]["finishReason"] = json!(finish);
    }
    value
}
fn decode(values: &[Value], chunk: usize) -> Result<(Completion, Vec<Event>), Error> {
    let bytes = values
        .iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect::<String>();
    let mut sse = Sse::new(256 * 1024);
    let mut parser = parser::Parser::new(Protocol::Gemini, 32);
    let mut events = Vec::new();
    for chunk in bytes.as_bytes().chunks(chunk) {
        sse.push(chunk, |frame| {
            parser.frame(frame, &mut |event| {
                events.push(event);
                Ok(())
            })
        })?;
    }
    assert!(
        !parser.done(),
        "a terminal candidate must still wait for EOF"
    );
    assert!(sse.is_empty());
    parser.end()?;
    Ok((parser.complete(BASE, MODEL)?, events))
}
fn append_turn(request: &mut Request, completion: Completion, reverse_results: bool) {
    let mut results = completion
        .tool_calls
        .iter()
        .map(|call| Message::ToolResult {
            call_id: call.id.clone(),
            content: call.arguments.to_string(),
            is_error: false,
        })
        .collect::<Vec<_>>();
    if reverse_results {
        results.reverse();
    }
    request.messages.push(Message::Assistant {
        text: completion.text,
        tool_calls: completion.tool_calls,
        continuation: Some(completion.continuation),
    });
    request.messages.extend(results);
}

#[test]
fn new_channel_gemini_tail_signatures_thoughts_and_cumulative_usage_survive_eof() {
    let first = candidate(
        json!([{"text":"private thought", "thought":true,"thoughtSignature":"private-thought-signature"}]),
        None,
    );
    let mut middle = candidate(json!([{"text":"中文🙂 answer"}]), Some("STOP"));
    middle["usageMetadata"] = json!({"promptTokenCount":10,"candidatesTokenCount":4,"thoughtsTokenCount":3,"totalTokenCount":17});
    let mut tail = candidate(
        json!([{"text":"","thoughtSignature":"private-tail-signature"}]),
        None,
    );
    tail["usageMetadata"] = json!({"promptTokenCount":10,"candidatesTokenCount":4,"thoughtsTokenCount":5,"totalTokenCount":19});
    let (result, events) = decode(&[first.clone(), middle.clone(), tail.clone()], 1).unwrap();
    assert_eq!(result.text, "中文🙂 answer");
    assert_eq!(
        events,
        vec![Event::TextDelta {
            text: "中文🙂 answer".into()
        }]
    );
    assert!(!format!("{events:?}").contains("private"));
    assert_eq!(
        result.usage,
        Usage {
            input_tokens: Some(10),
            output_tokens: Some(9),
            reasoning_tokens: Some(5)
        }
    );
    assert_eq!(
        result.continuation.items,
        vec![
            first["candidates"][0]["content"]["parts"][0].clone(),
            middle["candidates"][0]["content"]["parts"][0].clone(),
            tail["candidates"][0]["content"]["parts"][0].clone()
        ]
    );
    let mut request = input();
    append_turn(&mut request, result, false);
    request.messages.push(Message::User {
        text: "correct the previous answer".into(),
    });
    let body = request::wire(Protocol::Gemini, BASE, &request).unwrap();
    assert_eq!(
        body["contents"][1]["parts"][2]["thoughtSignature"],
        "private-tail-signature"
    );
    assert_eq!(
        body["contents"][2]["parts"][0]["text"],
        "correct the previous answer"
    );
}

#[test]
fn new_channel_gemini_same_name_parallel_calls_without_provider_ids_keep_order() {
    let parts = json!([
        {"functionCall":{"name":"find","args":{"q":"first"}},"thoughtSignature":"signature-1"},
        {"functionCall":{"name":"find","args":{"q":"second"}}}
    ]);
    let (result, _) = decode(&[candidate(parts.clone(), Some("STOP"))], 3).unwrap();
    assert_eq!(result.finish, Finish::ToolCalls);
    assert_ne!(result.tool_calls[0].id, result.tool_calls[1].id);
    let mut request = input();
    append_turn(&mut request, result, true);
    let body = request::wire(Protocol::Gemini, BASE, &request).unwrap();
    assert_eq!(body["contents"][1]["parts"], parts);
    let responses = &body["contents"][2]["parts"];
    assert_eq!(
        responses[0]["functionResponse"]["response"]["output"]["q"],
        "first"
    );
    assert_eq!(
        responses[1]["functionResponse"]["response"]["output"]["q"],
        "second"
    );
    assert!(responses[0]["functionResponse"].get("id").is_none());
    assert!(responses[1]["functionResponse"].get("id").is_none());
}

#[test]
fn new_channel_gemini_provider_ids_and_signatures_survive_sequential_tools() {
    let mut request = input();
    for (id, signature) in [("provider-1", "sig-1"), ("provider-2", "sig-2")] {
        let (result, _) = decode(&[candidate(json!([{"functionCall":{"id":id,"name":"find","args":{}},"thoughtSignature":signature}]), Some("STOP"))], 7).unwrap();
        assert_eq!(result.tool_calls[0].id, id);
        append_turn(&mut request, result, false);
    }
    let body = request::wire(Protocol::Gemini, BASE, &request).unwrap();
    assert_eq!(body["contents"].as_array().unwrap().len(), 5);
    assert_eq!(body["contents"][1]["parts"][0]["thoughtSignature"], "sig-1");
    assert_eq!(body["contents"][3]["parts"][0]["thoughtSignature"], "sig-2");
    assert_eq!(
        body["contents"][2]["parts"][0]["functionResponse"]["id"],
        "provider-1"
    );
    assert_eq!(
        body["contents"][4]["parts"][0]["functionResponse"]["id"],
        "provider-2"
    );
}

#[test]
fn new_channel_gemini_no_id_sequential_calls_do_not_alias_prior_results() {
    let mut request = input();
    let mut ids = Vec::new();
    for round in 1..=2 {
        let (result, _) = decode(
            &[candidate(
                json!([{"functionCall":{"name":"find","args":{"round":round}},"thoughtSignature":format!("round-{round}")}]),
                Some("STOP"),
            )],
            1,
        )
        .unwrap();
        ids.push(result.tool_calls[0].id.clone());
        append_turn(&mut request, result, false);
    }
    assert_ne!(ids[0], ids[1]);
    let body = request::wire(Protocol::Gemini, BASE, &request).unwrap();
    for (turn, round) in [(1, 1), (3, 2)] {
        let part = &body["contents"][turn]["parts"][0];
        assert!(part["functionCall"].get("id").is_none());
        assert_eq!(part["thoughtSignature"], format!("round-{round}"));
        let response = &body["contents"][turn + 1]["parts"][0]["functionResponse"];
        assert!(response.get("id").is_none());
        assert_eq!(response["name"], "find");
        assert_eq!(response["response"]["output"]["round"], round);
    }
}

#[test]
fn new_channel_gemini_wire_uses_one_native_structured_shape_and_model_resource() {
    let mut request = input();
    request.messages.insert(
        0,
        Message::System {
            text: "system fixture".into(),
        },
    );
    request.output_format = Some(ChatFormat::EvidenceAnswer {
        citation_ids: vec!["E1".into()],
    });
    let body = request::wire(Protocol::Gemini, BASE, &request).unwrap();
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        "system fixture"
    );
    assert_eq!(body["generationConfig"]["candidateCount"], 1);
    assert_eq!(
        body["generationConfig"]["responseFormat"]["text"]["mimeType"],
        "application/json"
    );
    assert_eq!(
        body["generationConfig"]["responseFormat"]["text"]["schema"]["properties"]["claims"]["items"]
            ["properties"]["citation_ids"]["items"]["enum"],
        json!(["E1"])
    );
    assert!(body["generationConfig"].get("responseSchema").is_none());
    assert!(body["generationConfig"].get("responseMimeType").is_none());
    assert!(body.get("messages").is_none() && body.get("stream").is_none());
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"],
        request.tools[0].parameters
    );
    assert!(
        body["tools"][0]["functionDeclarations"][0]
            .get("parameters")
            .is_none()
    );
    let stream = url(BASE, "models/gemini-fixture", GenerationTransport::Stream).unwrap();
    assert_eq!(
        stream.as_str(),
        "https://example.test/v1beta/models/gemini-fixture:streamGenerateContent?alt=sse"
    );
    assert_eq!(
        url(BASE, MODEL, GenerationTransport::Json)
            .unwrap()
            .as_str(),
        "https://example.test/v1beta/models/gemini-fixture:generateContent"
    );
    for model in ["../secret", "models/models/a", "a?key=x", "a/b", "..", " a"] {
        assert!(url(BASE, model, GenerationTransport::Json).is_err());
    }
    assert!(
        url(
            "https://example.test/v1beta/models",
            MODEL,
            GenerationTransport::Json
        )
        .is_err()
    );
}

#[test]
fn new_channel_gemini_continuation_scope_and_raw_parts_are_checked() {
    let (mut result, _) = decode(
        &[candidate(
            json!([{"functionCall":{"name":"find","args":{}},"thoughtSignature":"sig"}]),
            Some("STOP"),
        )],
        10,
    )
    .unwrap();
    result.continuation.channel_id = Some("channel-a".into());
    let mut request = input();
    append_turn(&mut request, result, false);
    assert!(
        request::wire_with_options(
            Protocol::Gemini,
            BASE,
            Some("channel-a"),
            GenerationTransport::Json,
            &request
        )
        .is_ok()
    );
    for scope in [None, Some("channel-b")] {
        assert!(
            request::wire_with_options(
                Protocol::Gemini,
                BASE,
                scope,
                GenerationTransport::Json,
                &request
            )
            .is_err()
        );
    }
    assert!(
        request::wire_with_options(
            Protocol::Gemini,
            "https://other.test",
            Some("channel-a"),
            GenerationTransport::Json,
            &request
        )
        .is_err()
    );
    if let Message::Assistant {
        continuation: Some(saved),
        ..
    } = &mut request.messages[1]
    {
        saved.items[0]["functionCall"]["name"] = json!("other");
    }
    assert!(
        request::wire_with_options(
            Protocol::Gemini,
            BASE,
            Some("channel-a"),
            GenerationTransport::Json,
            &request
        )
        .is_err()
    );
}

#[test]
fn new_channel_gemini_finish_reasons_do_not_turn_partial_calls_into_success() {
    let call = json!([{"functionCall":{"name":"find","args":{}}}]);
    for (reason, expected) in [
        ("MAX_TOKENS", Finish::Length),
        ("SAFETY", Finish::Refusal),
        ("RECITATION", Finish::Refusal),
    ] {
        let (result, _) = decode(&[candidate(call.clone(), Some(reason))], 11).unwrap();
        assert_eq!(result.finish, expected);
        assert!(result.tool_calls.is_empty());
        assert!(!result.continuation.replayable);
    }
    for reason in [
        "MALFORMED_FUNCTION_CALL",
        "UNEXPECTED_TOOL_CALL",
        "MISSING_THOUGHT_SIGNATURE",
        "FUTURE_UNKNOWN",
    ] {
        assert!(decode(&[candidate(call.clone(), Some(reason))], 9).is_err());
    }
    assert_eq!(
        decode(&[candidate(json!([{"text":"partial"}]), None)], 1).err(),
        Some(Error::Incomplete)
    );
    let (blocked, _) = decode(&[json!({"promptFeedback":{"blockReason":"SAFETY"}})], 1).unwrap();
    assert_eq!(blocked.finish, Finish::Refusal);
    assert!(decode(&[json!({"promptFeedback":{"blockReason":"UNKNOWN"}})], 1).is_err());
    assert!(decode(&[candidate(json!([]), Some("STOP"))], 1).is_err());
}

#[test]
fn new_channel_gemini_malformed_or_unsupported_parts_fail_closed() {
    for part in [
        json!({"inlineData":{"mimeType":"image/png","data":"private"}}),
        json!({"functionResponse":{"name":"find","response":{}}}),
        json!({"text":"x","functionCall":{"name":"find","args":{}}}),
        json!({"functionCall":{"name":"find","args":"partial"}}),
        json!({"functionCall":{"name":"find","args":{},"id":""}}),
        json!({"text":"","thoughtSignature":""}),
        json!({"text":"x","thought":"true"}),
        json!({"executableCode":{"code":"private"}}),
        json!({}),
    ] {
        assert!(decode(&[candidate(json!([part]), Some("STOP"))], 13).is_err());
    }
    let mut wrong = candidate(json!([{"text":"x"}]), Some("STOP"));
    wrong["candidates"][0]["index"] = json!(1);
    assert!(decode(&[wrong], 1).is_err());
    let call = json!({"functionCall":{"id":"duplicate","name":"find","args":{}}});
    assert!(decode(&[candidate(json!([call.clone(), call]), Some("STOP"))], 1).is_err());
    let stopped = candidate(json!([{"text":"x"}]), Some("STOP"));
    assert!(decode(&[stopped, candidate(json!([{"text":"late"}]), None)], 1).is_err());
}

#[test]
fn new_channel_gemini_usage_response_identity_and_candidates_cannot_mix() {
    let mut first = candidate(json!([{"text":"x"}]), None);
    first["usageMetadata"] = json!({"promptTokenCount":5,"thoughtsTokenCount":3});
    let mut second = candidate(json!([{"text":"y"}]), Some("STOP"));
    second["usageMetadata"] = json!({"promptTokenCount":5,"thoughtsTokenCount":2});
    assert!(decode(&[first.clone(), second.clone()], 1).is_err());
    second["usageMetadata"] = json!({"thoughtsTokenCount":4});
    second["responseId"] = json!("different");
    assert!(decode(&[first, second], 1).is_err());
    let mut multiple = candidate(json!([{"text":"x"}]), Some("STOP"));
    multiple["candidates"]
        .as_array_mut()
        .unwrap()
        .push(json!({"index":1,"content":{"parts":[{"text":"y"}]}}));
    assert_eq!(decode(&[multiple], 1).err(), Some(Error::UnsupportedOutput));
}

#[test]
fn new_channel_gemini_missing_duplicate_and_reconstructed_tool_results_are_rejected() {
    let (result, _) = decode(
        &[candidate(
            json!([{"functionCall":{"name":"find","args":{}}}]),
            Some("STOP"),
        )],
        1,
    )
    .unwrap();
    let mut request = input();
    append_turn(&mut request, result, false);
    let good = request.clone();
    request.messages.pop();
    assert!(request::wire(Protocol::Gemini, BASE, &request).is_err());
    request = good.clone();
    request
        .messages
        .push(request.messages.last().unwrap().clone());
    assert!(request::wire(Protocol::Gemini, BASE, &request).is_err());
    request = good;
    if let Message::Assistant { continuation, .. } = &mut request.messages[1] {
        *continuation = None;
    }
    assert!(request::wire(Protocol::Gemini, BASE, &request).is_err());
}
