//! Local HTTP fixtures test transport semantics, not model quality or successful inference.
#![cfg(feature = "ollama")]
use pkms_inference::ollama::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

fn request() -> ChatRequest {
    ChatRequest {
        model: "fixture-only".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "transport fixture".into(),
        }],
        options: ChatOptions::default(),
        format: None,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn context_budget_rejects_before_opening_a_connection() {
    let client = OllamaClient::new("http://127.0.0.1:1", StreamLimits::default()).unwrap();
    let mut request = request();
    request.messages[0].content = "证据".repeat(1000);
    let result = runtime().block_on(client.chat(&request, |_| panic!("no delta"), || false));
    assert_eq!(result.unwrap_err(), OllamaError::ContextLimit);
}

#[test]
fn invalid_format_rejects_before_opening_a_connection() {
    let client = OllamaClient::new("http://127.0.0.1:1", StreamLimits::default()).unwrap();
    let mut request = request();
    request.format = Some(pkms_inference::protocol::ChatFormat::EvidenceAnswer {
        citation_ids: vec!["E33".into()],
    });
    let result = runtime().block_on(client.chat(&request, |_| panic!("no delta"), || false));
    assert_eq!(result.unwrap_err(), OllamaError::InvalidRequest);
}

#[test]
fn constrained_schema_is_sent_without_exposing_the_format_selector() {
    let (endpoint, capture, handle) = server(
        "200 OK",
        "{\"done\":true,\"done_reason\":\"stop\"}\n",
        Duration::ZERO,
    );
    let client = OllamaClient::new(&endpoint, StreamLimits::default()).unwrap();
    let mut request = request();
    let format = pkms_inference::protocol::ChatFormat::EvidenceAnswer {
        citation_ids: vec!["E1".into(), "E32".into()],
    };
    let schema = format.json_schema().unwrap();
    request.format = Some(format);
    runtime()
        .block_on(client.chat(&request, |_| Ok(()), || false))
        .unwrap();
    handle.join().unwrap();
    let wire = capture.lock().unwrap();
    let json: serde_json::Value =
        serde_json::from_str(wire.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(json["format"], schema);
    assert!(json["format"].get("kind").is_none());
    assert_eq!(json["stream"], true);
    assert_eq!(json["think"], false);
}

fn server(
    status: &str,
    body: &'static str,
    delay: Duration,
) -> (String, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(String::new()));
    let capture = captured.clone();
    let status = status.to_owned();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut data = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let size = socket.read(&mut buffer).unwrap();
            if size == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..size]);
            if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        *capture.lock().unwrap() = String::from_utf8(data).unwrap();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n"
        );
        let _ = socket.write_all(headers.as_bytes());
        // A token before the delay supports idle timeout testing.
        if body.starts_with("DELAY_AFTER_TOKEN") {
            let _ = socket.write_all(b"{\"message\":{\"content\":\"first\"},\"done\":false}\n");
        }
        std::thread::sleep(delay);
        let _ = socket.write_all(
            body.strip_prefix("DELAY_AFTER_TOKEN")
                .unwrap_or(body)
                .as_bytes(),
        );
    });
    (endpoint, captured, handle)
}

#[test]
fn endpoint_is_only_credential_free_loopback() {
    for endpoint in [
        "https://localhost:11434",
        "http://example.com",
        "http://192.168.1.1",
        "http://user:secret@localhost",
        "http://localhost/api/chat",
        "http://localhost?key=secret",
        "http://localhost/#secret",
        "file:///tmp/model",
    ] {
        assert!(matches!(
            OllamaClient::new(endpoint, StreamLimits::default()),
            Err(OllamaError::InvalidEndpoint)
        ));
    }
    for endpoint in [
        "http://127.0.0.1:11434",
        "http://[::1]:11434",
        "http://localhost:11434",
    ] {
        assert!(OllamaClient::new(endpoint, StreamLimits::default()).is_ok());
    }
}

#[test]
fn fixture_unicode_deltas_and_explicit_done() {
    let (endpoint, capture, handle) = server(
        "200 OK",
        "{\"message\":{\"content\":\"中文\"},\"done\":false}\n{\"message\":{\"content\":\"回答\"},\"done\":true,\"done_reason\":\"stop\",\"eval_count\":2}",
        Duration::ZERO,
    );
    let client = OllamaClient::new(&endpoint, StreamLimits::default()).unwrap();
    let mut text = String::new();
    let result = runtime()
        .block_on(client.chat(
            &request(),
            |s| {
                text.push_str(s);
                Ok(())
            },
            || false,
        ))
        .unwrap();
    assert_eq!(text, "中文回答");
    assert_eq!(result.eval_count, Some(2));
    handle.join().unwrap();
    let wire = capture.lock().unwrap();
    let json: serde_json::Value =
        serde_json::from_str(wire.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(json["stream"], true);
    assert_eq!(json["think"], false);
    assert_eq!(json["keep_alive"], 0);
    assert!(json.get("format").is_none());
    assert!(!wire.to_lowercase().contains("authorization:"));
}

#[test]
fn fixture_errors_do_not_leak_server_text_or_mark_partial_success() {
    for (status, body, expected) in [
        (
            "404 Not Found",
            "secret missing model",
            OllamaError::ModelNotFound,
        ),
        (
            "500 Internal Server Error",
            "secret internal error",
            OllamaError::ApiError,
        ),
        ("302 Found", "secret redirect", OllamaError::ApiError),
        ("200 OK", "{\"error\":\"secret\"}\n", OllamaError::ApiError),
        (
            "200 OK",
            "{\"message\":{\"content\":\"partial\"},\"done\":false}\n",
            OllamaError::Incomplete,
        ),
        ("200 OK", "not-json secret\n", OllamaError::InvalidResponse),
    ] {
        let (endpoint, _, handle) = server(status, body, Duration::ZERO);
        let client = OllamaClient::new(&endpoint, StreamLimits::default()).unwrap();
        let error = runtime()
            .block_on(client.chat(&request(), |_| Ok(()), || false))
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("secret"));
        handle.join().unwrap();
    }
}

#[test]
fn fixture_limits_timeouts_and_inflight_cancel() {
    let cases = [
        (
            "no-newline-at-all",
            StreamLimits {
                max_line_bytes: 8,
                ..StreamLimits::default()
            },
            Duration::ZERO,
            None,
            OllamaError::ResponseTooLarge,
        ),
        (
            "\n\n\n\n\n\n\n\n\n",
            StreamLimits {
                max_response_bytes: 8,
                max_line_bytes: 8,
                ..StreamLimits::default()
            },
            Duration::ZERO,
            None,
            OllamaError::ResponseTooLarge,
        ),
        (
            "",
            StreamLimits {
                first_token_timeout: Duration::from_millis(50),
                ..StreamLimits::default()
            },
            Duration::from_millis(150),
            None,
            OllamaError::FirstTokenTimeout,
        ),
        (
            "DELAY_AFTER_TOKEN",
            StreamLimits {
                idle_timeout: Duration::from_millis(50),
                ..StreamLimits::default()
            },
            Duration::from_millis(150),
            None,
            OllamaError::IdleTimeout,
        ),
        (
            "",
            StreamLimits {
                total_timeout: Duration::from_millis(50),
                ..StreamLimits::default()
            },
            Duration::from_millis(150),
            None,
            OllamaError::TotalTimeout,
        ),
        (
            "",
            StreamLimits::default(),
            Duration::from_millis(150),
            Some(Duration::from_millis(50)),
            OllamaError::Canceled,
        ),
    ];
    for (body, limits, delay, cancel_after, expected) in cases {
        let (endpoint, _, handle) = server("200 OK", body, delay);
        let client = OllamaClient::new(&endpoint, limits).unwrap();
        let started = std::time::Instant::now();
        let error = runtime()
            .block_on(client.chat(
                &request(),
                |_| Ok(()),
                || cancel_after.is_some_and(|d| started.elapsed() >= d),
            ))
            .unwrap_err();
        assert_eq!(error, expected);
        handle.join().unwrap();
    }
}

#[test]
#[ignore = "requires isolated real Ollama service at 127.0.0.1:11435; does not test successful model inference"]
fn real_ollama_missing_model() {
    let client = OllamaClient::new("http://127.0.0.1:11435", StreamLimits::default()).unwrap();
    let mut req = request();
    req.model = "pkms-intentionally-absent-model-transport-check:missing".into();
    let result =
        runtime().block_on(client.chat(&req, |_| panic!("absent model emitted a token"), || false));
    assert_eq!(result.unwrap_err(), OllamaError::ModelNotFound);
}
