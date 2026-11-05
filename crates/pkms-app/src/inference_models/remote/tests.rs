//! Synthetic local HTTP catalog contracts. No OS vault or real provider is contacted.
use super::*;
use pkms_core::{GenerationCapabilities, GenerationChannelSpec, GenerationTransport};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
};

fn channel(provider: &str, cloudflare: bool) -> GenerationChannel {
    GenerationChannel {
        id: "37ebf203-3a3b-4f3f-9eab-ac4b52bbf117".into(),
        row_version: 2,
        deleted: false,
        created_at: "2026-10-09T00:00:00Z".into(),
        updated_at: "2026-10-09T00:00:00Z".into(),
        spec: GenerationChannelSpec {
            name: "Synthetic catalog".into(),
            provider: provider.into(),
            endpoint: "https://example.test/v1".into(),
            models: vec![],
            capabilities: GenerationCapabilities {
                text_transport: GenerationTransport::Stream,
                structured_transport: Some(GenerationTransport::Json),
            },
            preset: cloudflare.then_some(GenerationChannelPreset::CloudflareWorkersAi),
        },
    }
}
fn ok(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn spawn_catalog(
    provider: &str,
    cloudflare: bool,
    replies: Vec<(Duration, String)>,
) -> (
    RemoteService,
    mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (send, receive) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        for (delay, reply) in replies {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < until,
                            "missing synthetic catalog page"
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
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 32768);
            }
            let _ = send.send(String::from_utf8(request).unwrap());
            std::thread::sleep(delay);
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    let mut service = RemoteService::new(
        &channel(provider, cloudflare),
        &vault::Secret::new("synthetic-catalog-secret".into()).unwrap(),
    )
    .unwrap();
    assert!(!format!("{:?}", service.headers).contains("synthetic-catalog-secret"));
    service.endpoint = Url::parse(&format!(
        "http://{address}{}",
        if cloudflare {
            "/client/v4/accounts/synthetic/ai/v1"
        } else if provider == "gemini" {
            "/v1beta"
        } else {
            "/v1"
        }
    ))
    .unwrap();
    // HTTP is confined to this test fixture; the production constructor is HTTPS-only.
    service.client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    (service, receive, peer)
}
fn replies(values: Vec<Value>) -> Vec<(Duration, String)> {
    values
        .into_iter()
        .map(|value| (Duration::ZERO, ok(value)))
        .collect()
}
fn gemini(name: &str) -> Value {
    json!({"name":format!("models/{name}"),"displayName":name,"inputTokenLimit":1048576,"supportedGenerationMethods":["generateContent"]})
}
fn cf(name: &str, task: &str) -> Value {
    json!({"name":name,"task":{"name":task},"context_length":8192})
}

#[test]
fn new_channel_discovery_gemini_paginates_filters_and_deduplicates() {
    let (mut service, wire, peer) = spawn_catalog(
        "gemini",
        false,
        replies(vec![
            json!({"models":[gemini("alpha"),{"name":"models/embedding","supportedGenerationMethods":["embedContent"]}],"nextPageToken":"page 2/test"}),
            json!({"models":[gemini("alpha"),gemini("beta")]}),
        ]),
    );
    let models = run_model_request(&AtomicBool::new(false), service.discover()).unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.model.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    assert_eq!(models[0].context_tokens, Some(1048576));
    let first = wire.recv().unwrap();
    assert!(first.starts_with("GET /v1beta/models?pageSize=100 "));
    assert!(
        first
            .to_ascii_lowercase()
            .contains("x-goog-api-key: synthetic-catalog-secret")
    );
    assert!(!first.to_ascii_lowercase().contains("authorization:"));
    assert!(wire.recv().unwrap().contains("pageToken=page+2%2Ftest"));
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_cloudflare_uses_search_and_does_not_trust_short_pages() {
    let (mut service, wire, peer) = spawn_catalog(
        "openai_chat",
        true,
        replies(vec![
            json!({"success":true,"errors":[],"result":[cf("@cf/a","Text Generation"),cf("@cf/embedding","Text Embeddings")]}),
            json!({"success":true,"errors":[],"result":[cf("@cf/a","Text Generation"),cf("@cf/b","Text Generation")]}),
            json!({"success":true,"errors":[],"result":[]}),
        ]),
    );
    let models = run_model_request(&AtomicBool::new(false), service.discover()).unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.model.as_str())
            .collect::<Vec<_>>(),
        vec!["@cf/a", "@cf/b"]
    );
    for page in 1..=3 {
        let request = wire.recv().unwrap();
        assert!(request.starts_with(&format!(
            "GET /client/v4/accounts/synthetic/ai/models/search?page={page}&per_page=100 "
        )));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-catalog-secret")
        );
        assert!(!request.contains("/ai/v1/models"));
    }
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_anthropic_uses_cursor_and_native_auth() {
    let (mut service, wire, peer) = spawn_catalog(
        "anthropic",
        false,
        replies(vec![
            json!({"data":[{"id":"claude-a","display_name":"A"}],"has_more":true,"last_id":"claude-a"}),
            json!({"data":[{"id":"claude-b","max_input_tokens":200000}],"has_more":false,"last_id":"claude-b"}),
        ]),
    );
    let models = run_model_request(&AtomicBool::new(false), service.discover()).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[1].context_tokens, Some(200000));
    let first = wire.recv().unwrap().to_ascii_lowercase();
    assert!(
        first.contains("x-api-key: synthetic-catalog-secret")
            && first.contains("anthropic-version: 2023-06-01")
    );
    assert!(wire.recv().unwrap().contains("after_id=claude-a"));
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_openai_wire_lists_actual_ids_without_claiming_capabilities() {
    for provider in ["openai_chat", "openai_responses"] {
        let (mut service, wire, peer) = spawn_catalog(
            provider,
            false,
            replies(vec![
                json!({"object":"list","data":[{"id":"remote-a"},{"id":"remote-b","context_length":262144}]}),
            ]),
        );
        let models = run_model_request(&AtomicBool::new(false), service.discover()).unwrap();
        assert_eq!(models[0].model, "remote-a");
        assert!(models[0].context_tokens.is_none());
        assert_eq!(models[1].context_tokens, Some(262144));
        assert!(wire.recv().unwrap().starts_with("GET /v1/models "));
        peer.join().unwrap();
    }
}

#[test]
fn new_channel_discovery_repeated_pages_and_cursor_cycles_fail_instead_of_truncating() {
    let repeated = json!({"success":true,"result":[cf("@cf/a","Text Generation")]});
    let (mut service, _, peer) = spawn_catalog(
        "openai_chat",
        true,
        replies(vec![repeated.clone(), repeated]),
    );
    assert!(matches!(
        run_model_request(&AtomicBool::new(false), service.discover()),
        Err(AppError::InvalidInput("model_response_invalid"))
    ));
    peer.join().unwrap();
    let (mut service, _, peer) = spawn_catalog(
        "gemini",
        false,
        replies(vec![
            json!({"models":[gemini("a")],"nextPageToken":"same"}),
            json!({"models":[gemini("b")],"nextPageToken":"same"}),
        ]),
    );
    assert!(matches!(
        run_model_request(&AtomicBool::new(false), service.discover()),
        Err(AppError::InvalidInput("model_response_invalid"))
    ));
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_midway_failure_never_returns_partial_models() {
    let (mut service, wire, peer) = spawn_catalog("gemini", false, vec![
        (Duration::ZERO, ok(json!({"models":[gemini("a")],"nextPageToken":"next"}))),
        (Duration::ZERO, "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\nprivate details".into()),
    ]);
    let error = run_model_request(&AtomicBool::new(false), service.discover()).unwrap_err();
    assert!(matches!(
        error,
        AppError::InvalidInput("model_authentication_failed")
    ));
    assert!(!format!("{error:?}").contains("private"));
    peer.join().unwrap();
    assert_eq!(wire.try_iter().count(), 2);
}

#[test]
fn new_channel_discovery_cloudflare_envelope_empty_and_unsupported_are_distinct() {
    for (value, expected) in [
        (
            json!({"success":false,"errors":[{"message":"private error"}],"result":[]}),
            "model_api_error",
        ),
        (
            json!({"success":true,"errors":[],"result":{}}),
            "model_response_invalid",
        ),
        (
            json!({"success":true,"errors":[],"result":[{"name":"@cf/a"}]}),
            "model_discovery_unsupported",
        ),
    ] {
        let (mut service, _, peer) = spawn_catalog("openai_chat", true, replies(vec![value]));
        let result = run_model_request(&AtomicBool::new(false), service.discover());
        assert!(matches!(result, Err(AppError::InvalidInput(code)) if code == expected));
        peer.join().unwrap();
    }
    let (mut service, _, peer) = spawn_catalog(
        "openai_chat",
        true,
        replies(vec![json!({"success":true,"errors":[],"result":[]})]),
    );
    assert!(
        run_model_request(&AtomicBool::new(false), service.discover())
            .unwrap()
            .is_empty()
    );
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_metadata_and_model_limit_must_be_consistent() {
    let (mut service, _, peer) = spawn_catalog(
        "openai_chat",
        true,
        replies(vec![
            json!({"success":true,"result":[cf("@cf/a","Text Generation")],"result_info":{"page":1,"total_pages":1}}),
        ]),
    );
    assert_eq!(
        run_model_request(&AtomicBool::new(false), service.discover())
            .unwrap()
            .len(),
        1
    );
    peer.join().unwrap();
    let (mut service, _, peer) = spawn_catalog(
        "openai_chat",
        true,
        replies(vec![
            json!({"success":true,"result":[cf("@cf/a","Text Generation")],"result_info":{"page":2,"total_pages":2}}),
        ]),
    );
    assert!(run_model_request(&AtomicBool::new(false), service.discover()).is_err());
    peer.join().unwrap();
    let values = (0..MAX_MODELS + 1)
        .map(|i| json!({"id":format!("model-{i}")}))
        .collect::<Vec<_>>();
    let (mut service, _, peer) =
        spawn_catalog("openai_chat", false, replies(vec![json!({"data":values})]));
    assert!(run_model_request(&AtomicBool::new(false), service.discover()).is_err());
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_response_bytes_and_request_cancel_are_bounded() {
    let too_large = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_PAGE_BYTES + 1
    );
    let (mut service, _, peer) =
        spawn_catalog("openai_chat", false, vec![(Duration::ZERO, too_large)]);
    assert!(matches!(
        run_model_request(&AtomicBool::new(false), service.discover()),
        Err(AppError::InvalidInput("model_response_limit"))
    ));
    peer.join().unwrap();
    let (mut service, _, peer) =
        spawn_catalog("openai_chat", false, replies(vec![json!({"data":[]})]));
    service.bytes = MAX_CATALOG_BYTES;
    assert!(matches!(
        run_model_request(&AtomicBool::new(false), service.discover()),
        Err(AppError::InvalidInput("model_response_limit"))
    ));
    peer.join().unwrap();
    let (mut service, _, peer) = spawn_catalog(
        "openai_chat",
        false,
        vec![(Duration::from_millis(250), ok(json!({"data":[]})))],
    );
    let shutdown = Arc::new(AtomicBool::new(false));
    let cancel = shutdown.clone();
    let stop = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        cancel.store(true, Ordering::Release);
    });
    assert!(matches!(
        run_model_request(&shutdown, service.discover()),
        Err(AppError::InvalidInput("worker_interrupted"))
    ));
    stop.join().unwrap();
    peer.join().unwrap();
}

#[test]
fn new_channel_discovery_pre_cancellation_never_requires_a_credential() {
    let identity = WorkspaceIdentity {
        local_instance_id: "not-read".into(),
        workspace_id: "not-read".into(),
    };
    assert!(matches!(
        list_remote_generation_models(&identity, &channel("gemini", false), &AtomicBool::new(true)),
        Err(AppError::InvalidInput("worker_interrupted"))
    ));
}
