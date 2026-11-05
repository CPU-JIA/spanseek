use super::*;
use serde_json::json;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};

fn read_request(stream: &mut TcpStream) -> (String, serde_json::Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (
        headers.to_ascii_lowercase(),
        serde_json::from_slice(&body).unwrap(),
    )
}
fn listen() -> (TcpListener, Url) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!(
        "http://{}/embeddings",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    (listener, url)
}
fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    && start.elapsed() < Duration::from_secs(3) =>
            {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => panic!("local test server accept failed: {e}"),
        }
    }
}
fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}
fn valid_body() -> String {
    json!({"model":MODEL,"data":[{"index":0,"embedding":vec![1.;DIMENSIONS]}]}).to_string()
}

#[tokio::test]
async fn pool_reuses_socket_without_reusing_credentials_or_input() {
    let (listener, url) = listen();
    let server = std::thread::spawn(move || {
        // Exactly one accepted socket must serve both requests.
        let mut stream = accept(&listener);
        for (key, text) in [
            ("synthetic-first", "first workspace"),
            ("synthetic-second", "second workspace"),
        ] {
            let (headers, body) = read_request(&mut stream);
            assert!(headers.contains(&format!("authorization: bearer {key}\r\n")));
            assert_eq!(headers.matches("authorization:").count(), 1);
            assert_eq!(body["input"], json!([text]));
            reply(&mut stream, 200, &valid_body());
        }
    });
    let client = Client::new().unwrap();
    for (key, text) in [
        ("synthetic-first", "first workspace"),
        ("synthetic-second", "second workspace"),
    ] {
        let result = client
            .embed_at(
                url.clone(),
                key,
                &[text.into()],
                Duration::from_secs(3),
                || Ok(()),
            )
            .await
            .unwrap();
        assert_eq!(result[0].len(), DIMENSIONS);
    }
    server.join().unwrap();
}

#[tokio::test]
async fn shared_pool_keeps_endpoint_requests_separate() {
    let client = Client::new().unwrap();
    for key in ["synthetic-endpoint-one", "synthetic-endpoint-two"] {
        let (listener, url) = listen();
        let server = std::thread::spawn(move || {
            let mut stream = accept(&listener);
            let (headers, _) = read_request(&mut stream);
            assert!(headers.contains(&format!("authorization: bearer {key}\r\n")));
            reply(&mut stream, 200, &valid_body());
        });
        client
            .embed_at(
                url,
                key,
                &["synthetic".into()],
                Duration::from_secs(3),
                || Ok(()),
            )
            .await
            .unwrap();
        server.join().unwrap();
    }
}

#[tokio::test]
async fn pool_preserves_http_errors_and_rejects_bad_vectors() {
    let client = Client::new().unwrap();
    for (status, body, expected) in [
        (401, "{}".to_owned(), "remote_authentication_failed"),
        (429, "{}".to_owned(), "remote_rate_limited"),
        (503, "{}".to_owned(), "embedding_api_failed"),
        (200, "not json".to_owned(), "embedding_invalid_response"),
        (
            200,
            json!({"data":[{"index":0,"embedding":[1.]}]}).to_string(),
            "embedding_invalid_vector",
        ),
    ] {
        let (listener, url) = listen();
        let server = std::thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&mut stream);
            reply(&mut stream, status, &body);
        });
        let error = client
            .embed_at(
                url,
                "synthetic",
                &["test".into()],
                Duration::from_secs(3),
                || Ok(()),
            )
            .await
            .unwrap_err();
        assert_eq!(error.0, expected);
        server.join().unwrap();
    }
}

#[tokio::test]
async fn stalled_headers_and_body_cancel_or_expire_without_poisoning_pool() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let client = Client::new().unwrap();
    for body_started in [false, true] {
        for canceled in [false, true] {
            let (listener, url) = listen();
            let (release, wait) = mpsc::channel();
            let received = Arc::new(AtomicBool::new(false));
            let seen = Arc::clone(&received);
            let server = std::thread::spawn(move || {
                let mut stream = accept(&listener);
                read_request(&mut stream);
                if body_started {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n{{").unwrap();
                    stream.flush().unwrap();
                }
                seen.store(true, Ordering::Release);
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            let timeout = if canceled {
                Duration::from_secs(3)
            } else {
                Duration::from_millis(150)
            };
            let error = client
                .embed_at(url, "synthetic", &["test".into()], timeout, || {
                    if canceled && received.load(Ordering::Acquire) {
                        Err(Error("canceled"))
                    } else {
                        Ok(())
                    }
                })
                .await
                .unwrap_err();
            release.send(()).unwrap();
            server.join().unwrap();
            assert_eq!(
                error.0,
                if canceled {
                    "canceled"
                } else {
                    "inference_timeout"
                }
            );
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }
    let (listener, url) = listen();
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        read_request(&mut stream);
        reply(&mut stream, 200, &valid_body());
    });
    assert!(
        client
            .embed_at(
                url,
                "synthetic",
                &["after cancellation".into()],
                Duration::from_secs(3),
                || Ok(())
            )
            .await
            .is_ok()
    );
    server.join().unwrap();
}

/// WP9 wire-level evidence: every provider call is the documented request shape.
#[tokio::test]
async fn openai_wire_sends_documented_fields_and_keeps_reported_usage() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let (headers, body) = read_request(&mut stream);
        assert!(
            headers.starts_with("post /v1/embeddings http/1.1\r\n"),
            "unexpected request: {headers}"
        );
        assert!(headers.contains("authorization: bearer synthetic-openai\r\n"));
        assert_eq!(headers.matches("authorization:").count(), 1);
        assert!(!headers.contains("x-goog-api-key"));
        assert_eq!(body["model"], json!("text-embedding-3-small"));
        assert_eq!(body["input"], json!(["第一段", "second"]));
        assert_eq!(body["encoding_format"], json!("float"));
        assert_eq!(body["dimensions"], json!(512));
        reply(
            &mut stream,
            200,
            &json!({
                "model": "text-embedding-3-small",
                "data": [
                    {"index": 0, "embedding": vec![0.5f32; 512]},
                    {"index": 1, "embedding": vec![0.25f32; 512]},
                ],
                "usage": {"prompt_tokens": 7},
            })
            .to_string(),
        );
    });
    let wire = Wire::OpenAi {
        base,
        model: "text-embedding-3-small".into(),
        dimensions: Some(512),
        check_model: true,
    };
    let embedded = Client::new()
        .unwrap()
        .embed_wire(
            &wire,
            "synthetic-openai",
            &["第一段".into(), "second".into()],
            Some(512),
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(embedded.vectors.len(), 2);
    assert_eq!(embedded.vectors[0].len(), 512);
    assert_eq!(embedded.prompt_tokens, Some(7));
    server.join().unwrap();
}

#[tokio::test]
async fn a_compatible_endpoint_may_omit_dimensions_and_the_model_echo() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let (headers, body) = read_request(&mut stream);
        assert!(
            headers.starts_with("post /embeddings http/1.1\r\n"),
            "unexpected request: {headers}"
        );
        assert_eq!(body["model"], json!("nomic-embed-text"));
        // A server that does not implement truncation must never be asked for it.
        assert!(body.get("dimensions").is_none());
        // Nor is it required to echo the model it was asked for.
        reply(
            &mut stream,
            200,
            &json!({"data": [{"index": 0, "embedding": vec![0.5f32; 768]}]}).to_string(),
        );
    });
    let wire = Wire::OpenAi {
        base,
        model: "nomic-embed-text".into(),
        dimensions: None,
        check_model: false,
    };
    let embedded = Client::new()
        .unwrap()
        .embed_wire(
            &wire,
            "synthetic-compatible",
            &["local".into()],
            None,
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(embedded.vectors[0].len(), 768);
    assert_eq!(embedded.prompt_tokens, None);
    server.join().unwrap();
}

#[tokio::test]
async fn gemini_wire_uses_batch_embed_contents_with_api_key_and_embed_content_config() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let (headers, body) = read_request(&mut stream);
        assert!(headers.starts_with(
            "post /v1beta/models/gemini-embedding-001:batchembedcontents http/1.1\r\n"
        ));
        assert!(headers.contains("x-goog-api-key: synthetic-gemini\r\n"));
        assert!(!headers.contains("authorization:"));
        assert_eq!(body["requests"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["requests"][0]["model"],
            json!("models/gemini-embedding-001")
        );
        assert_eq!(
            body["requests"][0]["content"]["parts"][0]["text"],
            json!("查询文本")
        );
        assert_eq!(
            body["requests"][0]["embedContentConfig"],
            json!({
                "taskType": "RETRIEVAL_QUERY",
                "outputDimensionality": 768,
                "autoTruncate": false,
            })
        );
        reply(
            &mut stream,
            200,
            &json!({
                "embeddings": [{"values": vec![0.5f32; 768]}],
                "usageMetadata": {"promptTokenCount": 11},
            })
            .to_string(),
        );
    });
    let wire = Wire::Gemini {
        base,
        model: "gemini-embedding-001".into(),
        task: Some(GeminiTask::Query),
        dimensions: Some(768),
    };
    let embedded = Client::new()
        .unwrap()
        .embed_wire(
            &wire,
            "synthetic-gemini",
            &["查询文本".into()],
            Some(768),
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(embedded.vectors[0].len(), 768);
    assert_eq!(embedded.prompt_tokens, Some(11));
    server.join().unwrap();
}

#[tokio::test]
async fn a_model_without_task_type_support_is_never_sent_one() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let (_, body) = read_request(&mut stream);
        assert_eq!(
            body["requests"][0]["model"],
            json!("models/gemini-embedding-2")
        );
        assert!(
            body["requests"][0]["embedContentConfig"]
                .get("taskType")
                .is_none()
        );
        assert_eq!(
            body["requests"][0]["embedContentConfig"]["autoTruncate"],
            json!(false)
        );
        reply(
            &mut stream,
            200,
            &json!({"embeddings": [{"values": vec![0.5f32; 3072]}]}).to_string(),
        );
    });
    let wire = Wire::Gemini {
        base,
        model: "gemini-embedding-2".into(),
        task: None,
        dimensions: None,
    };
    let embedded = Client::new()
        .unwrap()
        .embed_wire(
            &wire,
            "synthetic-gemini",
            &["文档".into()],
            None,
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(embedded.vectors[0].len(), 3072);
    server.join().unwrap();
}

#[tokio::test]
async fn a_vector_size_that_differs_from_the_profile_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            &json!({"data": [{"index": 0, "embedding": vec![0.5f32; 1024]}]}).to_string(),
        );
    });
    let wire = Wire::OpenAi {
        base,
        model: "text-embedding-3-small".into(),
        dimensions: Some(768),
        check_model: false,
    };
    let error = match Client::new()
        .unwrap()
        .embed_wire(
            &wire,
            "synthetic-openai",
            &["probe".into()],
            Some(768),
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
    {
        Ok(_) => panic!("a vector size that differs from the profile must fail"),
        Err(error) => error,
    };
    assert_eq!(error.0, "embedding_invalid_vector");
    server.join().unwrap();
}

#[test]
fn the_endpoint_boundary_keeps_plain_http_on_this_machine_and_https_elsewhere() {
    assert!(endpoint("http://127.0.0.1:11434/v1").is_ok());
    assert!(endpoint("http://localhost:8080").is_ok());
    assert_eq!(
        endpoint("http://embedding.example.com/v1").unwrap_err().0,
        "embedding_invalid_endpoint"
    );
    assert!(endpoint("https://api.openai.com/v1").is_ok());
    assert_eq!(
        gemini_endpoint(
            "https://generativelanguage.googleapis.com/v1beta",
            "models/bad/name"
        )
        .unwrap_err()
        .0,
        "embedding_invalid_endpoint"
    );
    assert!(
        gemini_endpoint(
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-embedding-001"
        )
        .is_ok()
    );
}

#[test]
fn a_base_url_without_a_path_still_resolves_to_a_single_slash() {
    assert_eq!(
        endpoint("https://api.openai.com").unwrap().path(),
        "/embeddings"
    );
    assert_eq!(
        endpoint("https://api.openai.com/").unwrap().path(),
        "/embeddings"
    );
    assert_eq!(
        endpoint("https://api.openai.com/v1").unwrap().path(),
        "/v1/embeddings"
    );
    assert_eq!(
        endpoint("https://api.openai.com/v1/").unwrap().path(),
        "/v1/embeddings"
    );
    assert_eq!(
        endpoint("http://localhost:11434").unwrap().path(),
        "/embeddings"
    );
    assert_eq!(
        gemini_endpoint(
            "https://generativelanguage.googleapis.com",
            "gemini-embedding-001"
        )
        .unwrap()
        .path(),
        "/models/gemini-embedding-001:batchEmbedContents"
    );
    assert_eq!(
        gemini_endpoint(
            "https://generativelanguage.googleapis.com/v1beta/",
            "models/gemini-embedding-001"
        )
        .unwrap()
        .path(),
        "/v1beta/models/gemini-embedding-001:batchEmbedContents"
    );
}
