//! Real HTTP against an isolated synthetic peer, never a real provider or API credential.
use super::*;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

fn input() -> Request {
    Request {
        model: "fixture".into(),
        messages: vec![Message::User {
            text: "synthetic".into(),
        }],
        tools: vec![],
        max_output_tokens: 64,
        chat_token_limit: None,
        temperature: None,
        output_format: None,
    }
}
fn with_tool(mut request: Request) -> Request {
    request.tools.push(ToolDefinition {
        name: "find".into(),
        description: "synthetic".into(),
        parameters: json!({"type":"object","properties":{}}),
    });
    request
}
fn structured() -> Request {
    Request {
        output_format: Some(ChatFormat::EvidenceAnswer {
            citation_ids: vec!["E1".into()],
        }),
        ..input()
    }
}
fn spawn_peer(
    protocol: Protocol,
    transport: GenerationTransport,
    pieces: Vec<(Duration, String)>,
    limits: Limits,
) -> (Client, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (send, receive) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < until,
                        "missing synthetic request"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            assert!(head.len() < 32768);
        }
        let head = String::from_utf8(head).unwrap();
        let length = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        let _ = send.send(format!("{head}{}", String::from_utf8(body).unwrap()));
        for (delay, piece) in pieces {
            std::thread::sleep(delay);
            if stream.write_all(piece.as_bytes()).is_err() {
                break;
            }
            let _ = stream.flush();
        }
    });
    let base = format!(
        "http://{address}/{}",
        if protocol == Protocol::Gemini {
            "v1beta"
        } else {
            "v1"
        }
    );
    let suffix = match protocol {
        Protocol::ChatCompletions => "chat/completions",
        Protocol::Responses => "responses",
        Protocol::AnthropicMessages => "messages",
        Protocol::Gemini => "",
    };
    let client = Client {
        http: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap(),
        protocol,
        url: Url::parse(&format!("{base}/{suffix}")).unwrap(),
        base,
        limits,
        transport,
        channel_id: None,
    };
    (client, receive, handle)
}
fn response(body: &str, transport: GenerationTransport) -> String {
    let mime = if transport == GenerationTransport::Json {
        "application/json"
    } else {
        "text/event-stream"
    };
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn json_peer(
    protocol: Protocol,
    body: Value,
) -> (Client, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    spawn_peer(
        protocol,
        GenerationTransport::Json,
        vec![(
            Duration::ZERO,
            response(&body.to_string(), GenerationTransport::Json),
        )],
        Limits::default(),
    )
}
fn key() -> ApiKey {
    ApiKey::new("synthetic-channel-secret".into()).unwrap()
}
fn chat(text: &str) -> Value {
    json!({"id":"chat-fixture","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":5}})
}
fn gemini(text: &str, finish: Option<&str>) -> Value {
    let mut value =
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":text}]},"index":0}]});
    if let Some(finish) = finish {
        value["candidates"][0]["finishReason"] = json!(finish);
    }
    value
}
fn body(wire: &str) -> Value {
    serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn new_channel_http_chat_json_preserves_structured_parameters_and_tool_roundtrip() {
    let text = r#"{"status":"answered","claims":[{"text":"synthetic","citation_ids":["E1"]}]}"#;
    let (client, wire, peer) = json_peer(Protocol::ChatCompletions, chat(text));
    let result = client
        .generate(&structured(), &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert_eq!(result.text, text);
    assert_eq!(result.usage.output_tokens, Some(5));
    let request = wire.recv().unwrap();
    assert!(request.starts_with("POST /v1/chat/completions "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-channel-secret")
    );
    let sent = body(&request);
    assert_eq!(sent["stream"], false);
    assert_eq!(sent["response_format"]["type"], "json_object");
    assert!(sent.get("stream_options").is_none());
    peer.join().unwrap();

    let output = json!({"id":"call","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[
        {"id":"one","type":"function","function":{"name":"find","arguments":"{\"q\":1}"}},
        {"id":"two","type":"function","function":{"name":"find","arguments":"{\"q\":2}"}}
    ]},"finish_reason":"tool_calls"}]});
    let (client, _, peer) = json_peer(Protocol::ChatCompletions, output);
    let mut request = with_tool(input());
    let result = client
        .generate(&request, &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert_eq!(result.finish, Finish::ToolCalls);
    let calls = result.tool_calls.clone();
    request.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    for call in calls {
        request.messages.push(Message::ToolResult {
            call_id: call.id,
            content: "synthetic result".into(),
            is_error: false,
        });
    }
    let sent = request::wire_with_options(
        Protocol::ChatCompletions,
        &client.base,
        None,
        GenerationTransport::Json,
        &request,
    )
    .unwrap();
    assert_eq!(sent["messages"][2]["tool_call_id"], "one");
    assert_eq!(sent["messages"][3]["tool_call_id"], "two");
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_responses_json_keeps_reasoning_and_full_output_items() {
    let reasoning = json!({"type":"reasoning","id":"reason","summary":[],"encrypted_content":"private-opaque-reasoning"});
    let output = json!({"id":"r","object":"response","status":"completed","output":[reasoning.clone(),{"type":"function_call","id":"f","call_id":"call","name":"find","arguments":"{}","status":"completed"}],"usage":{"input_tokens":12,"output_tokens":7}});
    let (client, wire, peer) = json_peer(Protocol::Responses, output);
    let mut request = with_tool(input());
    let result = client
        .generate(&request, &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert_eq!(result.finish, Finish::ToolCalls);
    assert_eq!(result.continuation.items[0], reasoning);
    assert!(result.text.is_empty());
    request.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    request.messages.push(Message::ToolResult {
        call_id: "call".into(),
        content: "result".into(),
        is_error: false,
    });
    let sent = request::wire_with_options(
        Protocol::Responses,
        &client.base,
        None,
        GenerationTransport::Json,
        &request,
    )
    .unwrap();
    assert_eq!(sent["input"][1], reasoning);
    assert_eq!(sent["input"][3]["call_id"], "call");
    assert_eq!(body(&wire.recv().unwrap())["stream"], false);
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_anthropic_json_keeps_thinking_signature_and_tool() {
    let output = json!({"type":"message","id":"a","role":"assistant","content":[{"type":"thinking","thinking":"private thought","signature":"private-signature"},{"type":"tool_use","id":"call","name":"find","input":{}}],"stop_reason":"tool_use","usage":{"input_tokens":4,"output_tokens":6}});
    let (client, wire, peer) = json_peer(Protocol::AnthropicMessages, output);
    let mut events = Vec::new();
    let result = client
        .generate(
            &with_tool(input()),
            &key(),
            |event| {
                events.push(event);
                Ok(())
            },
            || false,
        )
        .await
        .unwrap();
    assert_eq!(result.finish, Finish::ToolCalls);
    assert!(result.text.is_empty() && events.is_empty());
    assert_eq!(
        result.continuation.items[0]["signature"],
        "private-signature"
    );
    assert!(
        wire.recv()
            .unwrap()
            .to_ascii_lowercase()
            .contains("x-api-key: synthetic-channel-secret")
    );
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_gemini_json_uses_native_url_auth_and_format() {
    let (client, wire, peer) = json_peer(Protocol::Gemini, gemini("synthetic text", Some("STOP")));
    let result = client
        .generate(&structured(), &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert_eq!(result.text, "synthetic text");
    let sent = wire.recv().unwrap();
    assert!(sent.starts_with("POST /v1beta/models/fixture:generateContent "));
    assert!(
        sent.to_ascii_lowercase()
            .contains("x-goog-api-key: synthetic-channel-secret")
    );
    assert!(!sent.to_ascii_lowercase().contains("authorization:"));
    assert_eq!(
        body(&sent)["generationConfig"]["responseFormat"]["text"]["mimeType"],
        "application/json"
    );
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_gemini_stream_waits_for_tail_signature_and_normal_eof() {
    let first = format!("data: {}\n\n", gemini("synthetic text", Some("STOP")));
    let tail = "data: {\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"\",\"thoughtSignature\":\"private-tail\"}]}}],\"usageMetadata\":{\"promptTokenCount\":2,\"candidatesTokenCount\":3,\"thoughtsTokenCount\":4}}\n\n";
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + tail.len()
    );
    let (client, wire, peer) = spawn_peer(
        Protocol::Gemini,
        GenerationTransport::Stream,
        vec![
            (Duration::ZERO, header),
            (Duration::from_millis(60), tail.into()),
        ],
        Limits::default(),
    );
    let started = Instant::now();
    let result = client
        .generate(&input(), &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert_eq!(
        result.continuation.items[1]["thoughtSignature"],
        "private-tail"
    );
    assert_eq!(result.usage.output_tokens, Some(7));
    assert!(
        wire.recv()
            .unwrap()
            .starts_with("POST /v1beta/models/fixture:streamGenerateContent?alt=sse ")
    );
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_json_errors_and_rejected_format_never_retry_as_plain_text() {
    for output in [
        json!({"success":false,"errors":[{"message":"private provider error"}],"result":null}),
        json!({"error":{"message":"JSON Mode could not be met: private"}}),
    ] {
        let (client, wire, peer) = json_peer(Protocol::ChatCompletions, output);
        let error = client
            .generate(&structured(), &key(), |_| Ok(()), || false)
            .await
            .err()
            .unwrap();
        assert_eq!(error, Error::ApiError);
        assert!(!error.to_string().contains("private"));
        assert_eq!(
            body(&wire.recv().unwrap())["response_format"]["type"],
            "json_object"
        );
        peer.join().unwrap();
        assert!(wire.try_recv().is_err());
    }
}

#[tokio::test]
async fn new_channel_http_json_incomplete_malformed_and_unadvertised_calls_stay_failed() {
    let mut missing_finish = chat("partial");
    missing_finish["choices"][0]
        .as_object_mut()
        .unwrap()
        .remove("finish_reason");
    let bad_call = json!({"id":"c","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"call","type":"function","function":{"name":"undeclared","arguments":"{}"}}]},"finish_reason":"tool_calls"}]});
    for (output, expected) in [
        (missing_finish, Error::InvalidResponse),
        (bad_call, Error::UnsupportedOutput),
    ] {
        let (client, _, peer) = json_peer(Protocol::ChatCompletions, output);
        assert_eq!(
            client
                .generate(&input(), &key(), |_| Ok(()), || false)
                .await
                .err(),
            Some(expected)
        );
        peer.join().unwrap();
    }
    let (client, _, peer) = json_peer(
        Protocol::Responses,
        json!({"id":"r","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"function_call","id":"f","call_id":"c","name":"find","arguments":"{","status":"incomplete"}]}),
    );
    let result = client
        .generate(&with_tool(input()), &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    assert_eq!(result.finish, Finish::Length);
    assert!(result.tool_calls.is_empty() && !result.continuation.replayable);
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_json_limits_mime_and_truncated_body_are_enforced() {
    for (wire, limits, expected) in [
        (response("{}", GenerationTransport::Stream), Limits::default(), Error::InvalidResponse),
        (response(&chat("too much").to_string(), GenerationTransport::Json), Limits { max_response_bytes: 20, max_event_bytes: 20, ..Limits::default() }, Error::ResponseTooLarge),
        (response("{\"id\":", GenerationTransport::Json), Limits::default(), Error::InvalidResponse),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 9999\r\n\r\n{\"id\":".into(), Limits::default(), Error::Incomplete),
    ] {
        let (client, _, peer) = spawn_peer(Protocol::ChatCompletions, GenerationTransport::Json, vec![(Duration::ZERO, wire)], limits);
        assert_eq!(client.generate(&input(), &key(), |_| Ok(()), || false).await.err(), Some(expected));
        peer.join().unwrap();
    }
}

#[tokio::test]
async fn new_channel_http_json_waits_obey_cancel_first_idle_and_total_deadlines() {
    for (prefix, cancel, limits, expected) in [
        ("", true, Limits::default(), Error::Canceled),
        (
            "",
            false,
            Limits {
                first_event_timeout: Duration::from_millis(40),
                ..Limits::default()
            },
            Error::FirstEventTimeout,
        ),
        (
            "{",
            false,
            Limits {
                idle_timeout: Duration::from_millis(40),
                ..Limits::default()
            },
            Error::IdleTimeout,
        ),
        (
            "{",
            false,
            Limits {
                total_timeout: Duration::from_millis(40),
                ..Limits::default()
            },
            Error::TotalTimeout,
        ),
    ] {
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\n\r\n{prefix}"
        );
        let (client, _, peer) = spawn_peer(
            Protocol::ChatCompletions,
            GenerationTransport::Json,
            vec![
                (Duration::ZERO, wire),
                (Duration::from_millis(200), String::new()),
            ],
            limits,
        );
        let started = Instant::now();
        assert_eq!(
            client
                .generate(
                    &input(),
                    &key(),
                    |_| Ok(()),
                    || cancel && started.elapsed() > Duration::from_millis(30)
                )
                .await
                .err(),
            Some(expected)
        );
        assert!(started.elapsed() < Duration::from_millis(190));
        peer.join().unwrap();
    }
}

#[tokio::test]
async fn new_channel_http_gemini_hidden_thoughts_are_progress_but_not_answer_events() {
    let thought = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"private thought\",\"thought\":true,\"thoughtSignature\":\"private\"}]}}]}\n\n";
    let wire = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 10000\r\n\r\n{thought}"
    );
    let limits = Limits {
        first_event_timeout: Duration::from_millis(40),
        idle_timeout: Duration::from_millis(40),
        ..Limits::default()
    };
    let (client, _, peer) = spawn_peer(
        Protocol::Gemini,
        GenerationTransport::Stream,
        vec![
            (Duration::ZERO, wire),
            (Duration::from_millis(200), String::new()),
        ],
        limits,
    );
    let mut events = Vec::new();
    assert_eq!(
        client
            .generate(
                &input(),
                &key(),
                |event| {
                    events.push(event);
                    Ok(())
                },
                || false
            )
            .await
            .err(),
        Some(Error::IdleTimeout)
    );
    assert!(events.is_empty());
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_gemini_cancel_or_truncated_eof_cannot_complete_tools() {
    let call = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"find\",\"args\":{}},\"thoughtSignature\":\"private\"}]}}]}\n\n";
    let (client, _, peer) = spawn_peer(
        Protocol::Gemini,
        GenerationTransport::Stream,
        vec![(Duration::ZERO, response(call, GenerationTransport::Stream))],
        Limits::default(),
    );
    assert_eq!(
        client
            .generate(&with_tool(input()), &key(), |_| Ok(()), || false)
            .await
            .err(),
        Some(Error::Incomplete)
    );
    peer.join().unwrap();
    let final_call = call.replace("}]}}]}", "}]},\"finishReason\":\"STOP\"}]}");
    let (client, _, peer) = spawn_peer(
        Protocol::Gemini,
        GenerationTransport::Stream,
        vec![(
            Duration::ZERO,
            response(&final_call, GenerationTransport::Stream),
        )],
        Limits::default(),
    );
    let canceled = AtomicBool::new(false);
    let result = client
        .generate(
            &with_tool(input()),
            &key(),
            |_| {
                canceled.store(true, Ordering::Release);
                Ok(())
            },
            || canceled.load(Ordering::Acquire),
        )
        .await;
    assert_eq!(result.err(), Some(Error::Canceled));
    peer.join().unwrap();
}

#[tokio::test]
async fn new_channel_http_continuation_cannot_cross_channel_even_at_same_endpoint() {
    let (client, _, peer) = json_peer(Protocol::ChatCompletions, chat("synthetic"));
    let client = client.with_channel_id(Some("channel-a")).unwrap();
    let mut request = input();
    let result = client
        .generate(&request, &key(), |_| Ok(()), || false)
        .await
        .unwrap();
    request.messages.push(Message::Assistant {
        text: result.text,
        tool_calls: result.tool_calls,
        continuation: Some(result.continuation),
    });
    request.messages.push(Message::User {
        text: "follow up".into(),
    });
    assert!(
        request::wire_with_options(
            Protocol::ChatCompletions,
            &client.base,
            Some("channel-a"),
            GenerationTransport::Json,
            &request
        )
        .is_ok()
    );
    assert!(
        request::wire_with_options(
            Protocol::ChatCompletions,
            &client.base,
            Some("channel-b"),
            GenerationTransport::Json,
            &request
        )
        .is_err()
    );
    assert!(
        request::wire_with_options(
            Protocol::ChatCompletions,
            &client.base,
            None,
            GenerationTransport::Json,
            &request
        )
        .is_err()
    );
    peer.join().unwrap();
}

#[test]
fn new_channel_edge_json_identity_role_and_empty_output_are_not_success() {
    fn decode(protocol: Protocol, value: &Value) -> Result<Completion, Error> {
        let mut parser = Parser::new(protocol, 32);
        parser.json(value, &mut |_| Ok(()))?;
        parser.complete("https://example.test/v1", "fixture")
    }
    for (protocol, valid) in [
        (Protocol::ChatCompletions, chat("valid")),
        (
            Protocol::Responses,
            json!({"id":"r","status":"completed","output":[{"id":"m","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"valid"}]}]}),
        ),
        (
            Protocol::AnthropicMessages,
            json!({"id":"a","type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"valid"}]}),
        ),
        (Protocol::Gemini, gemini("valid", Some("STOP"))),
    ] {
        assert_eq!(decode(protocol, &valid).unwrap().text, "valid");
        if protocol != Protocol::Gemini {
            let mut missing_id = valid.clone();
            missing_id["id"] = json!("");
            assert_eq!(
                decode(protocol, &missing_id).err(),
                Some(Error::InvalidResponse)
            );
        }
        let mut wrong_role = valid.clone();
        let mut blank = valid;
        match protocol {
            Protocol::ChatCompletions => {
                wrong_role["choices"][0]["message"]["role"] = json!("user");
                blank["choices"][0]["message"]["content"] = json!(" \n ");
            }
            Protocol::Responses => {
                wrong_role["output"][0]["role"] = json!("user");
                blank["output"][0]["content"][0]["text"] = json!(" \n ");
            }
            Protocol::AnthropicMessages => {
                wrong_role["role"] = json!("user");
                blank["content"][0]["text"] = json!(" \n ");
            }
            Protocol::Gemini => {
                wrong_role["candidates"][0]["content"]["role"] = json!("user");
                blank["candidates"][0]["content"]["parts"][0]["text"] = json!(" \n ");
            }
        }
        assert_eq!(
            decode(protocol, &wrong_role).err(),
            Some(Error::InvalidResponse)
        );
        assert_eq!(decode(protocol, &blank).err(), Some(Error::InvalidResponse));
    }
}

#[test]
fn new_channel_edge_legacy_usage_and_continuation_json_keep_the_old_binding() {
    let old_usage = json!({"input_tokens":4,"output_tokens":7});
    let usage: Usage = serde_json::from_value(old_usage.clone()).unwrap();
    assert_eq!(usage.reasoning_tokens, None);
    assert_eq!(serde_json::to_value(usage).unwrap(), old_usage);
    let old = json!({"protocol":"chat_completions","endpoint":"https://example.test/v1","model":"fixture",
        "items":[{"role":"assistant","content":"previous"}],"text":"previous","calls":[],"replayable":true});
    let continuation: Continuation = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&continuation).unwrap(), old);
    let mut request = input();
    request.messages.push(Message::Assistant {
        text: "previous".into(),
        tool_calls: vec![],
        continuation: Some(continuation),
    });
    request.messages.push(Message::User {
        text: "follow up".into(),
    });
    assert!(
        request::wire_with_options(
            Protocol::ChatCompletions,
            "https://example.test/v1",
            None,
            GenerationTransport::Json,
            &request
        )
        .is_ok()
    );
    assert!(
        request::wire_with_options(
            Protocol::ChatCompletions,
            "https://example.test/v1",
            Some("new-channel"),
            GenerationTransport::Json,
            &request
        )
        .is_err()
    );
}
