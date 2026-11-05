use pkms_app::*;
use pkms_core::{SearchMode, SearchQuery, SearchScope};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let result = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Succeeded);
    result.result.unwrap().identity
}

fn request(identity: &WorkspaceIdentity, command: Value) -> DesktopRequest {
    DesktopRequest::parse(json!({
        "api_version": 1, "request_id": id(), "identity": identity, "command": command,
    }))
    .unwrap()
}

fn allow_remote_data(app: &mut AppService, identity: &WorkspaceIdentity) {
    let reply = app
        .dispatch(request(identity, json!({"kind": "retrieval_settings_get"})))
        .unwrap();
    let DesktopData::RetrievalSettings(current) = reply.data else {
        panic!("retrieval settings reply")
    };
    assert!(!current.params.remote_consent);
    let mut params = serde_json::to_value(&current.params).unwrap();
    params["remote_consent"] = json!(true);
    app.dispatch(request(
        identity,
        json!({"kind": "retrieval_settings_set", "payload": {
            "command_id": id(),
            "request": {"base_row_version": current.row_version, "params": params},
        }}),
    ))
    .unwrap();
}

#[derive(Clone)]
struct Capture {
    head: String,
    body: String,
}

struct Canary {
    endpoint: String,
    captures: Arc<Mutex<Vec<Capture>>>,
}

impl Canary {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let captures = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captures);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(120);
            while Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                // Accepted sockets inherit the listener's non-blocking mode on
                // Windows; restore blocking reads so a slow parallel test cannot
                // turn a pending request into a spurious disconnect.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&request[..header_end]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = request[header_end + 4..].to_vec();
                while body.len() < length {
                    let read = socket.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    body.extend_from_slice(&buffer[..read]);
                }
                sink.lock().unwrap().push(Capture {
                    head,
                    body: String::from_utf8_lossy(&body).to_string(),
                });
                let body = r#"{"data":[{"index":0,"embedding":[0.1,0.2,0.3,0.4]}],"model":"canary-embed","usage":{"prompt_tokens":3}}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });
        Self { endpoint, captures }
    }

    fn count(&self) -> usize {
        self.captures.lock().unwrap().len()
    }

    fn wait_for(&self, expected: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.count() >= expected {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.count() >= expected
    }
}

#[test]
fn unconsented_configuration_makes_no_request_and_consented_probe_uses_only_the_configured_endpoint()
 {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    app.library_mut(&identity)
        .unwrap()
        .create_note("PRIVATE-SENTINEL", "PRIVATE-SENTINEL body")
        .unwrap();
    let canary = Canary::start();
    let other = Canary::start();
    let endpoint = format!("{}/v1", canary.endpoint);
    let credential = json!({"kind": "embedding_credential_set", "payload": {
        "provider": "openai", "endpoint": endpoint, "model": "canary-embed",
        "api_key": "synthetic-canary-key",
    }});
    app.dispatch(request(&identity, credential)).unwrap();
    let configure = json!({"kind": "embedding_remote_configure", "payload": {
        "command_id": id(), "provider": "openai", "endpoint": endpoint,
        "model": "canary-embed", "output_dimensions": null,
    }});
    let error = app
        .dispatch(request(&identity, configure.clone()))
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidInput);
    match error.details {
        Some(ApiErrorDetails::InferenceFailure { reason }) => {
            assert_eq!(reason, "remote_consent_required")
        }
        other => panic!("unexpected details: {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        canary.count(),
        0,
        "unconsented configuration must not connect"
    );
    assert_eq!(other.count(), 0, "no other endpoint may be contacted");
    allow_remote_data(&mut app, &identity);
    app.dispatch(request(&identity, configure)).unwrap();
    assert!(canary.wait_for(1, Duration::from_secs(5)));
    assert_eq!(canary.count(), 1);
    assert_eq!(other.count(), 0);
    let capture = canary.captures.lock().unwrap().first().cloned().unwrap();
    assert!(
        capture.head.contains("POST /v1/embeddings"),
        "{}",
        capture.head
    );
    assert!(
        capture
            .head
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-canary-key"),
        "{}",
        capture.head
    );
    assert!(
        capture.body.contains("spanseek embedding probe"),
        "{}",
        capture.body
    );
    assert!(capture.body.contains("canary-embed"), "{}", capture.body);
    assert!(
        !capture.body.contains("PRIVATE-SENTINEL"),
        "the save-time probe must not send workspace text: {}",
        capture.body
    );
}
#[test]
fn withdrawing_consent_stops_semantic_search_before_any_request() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    app.library_mut(&identity)
        .unwrap()
        .create_note("隐私", "私有正文")
        .unwrap();
    let canary = Canary::start();
    let endpoint = format!("{}/v1", canary.endpoint);
    app.dispatch(request(
        &identity,
        json!({"kind": "embedding_credential_set", "payload": {
            "provider": "openai", "endpoint": endpoint, "model": "canary-embed",
            "api_key": "synthetic-canary-key",
        }}),
    ))
    .unwrap();
    allow_remote_data(&mut app, &identity);
    app.dispatch(request(
        &identity,
        json!({"kind": "embedding_remote_configure", "payload": {
            "command_id": id(), "provider": "openai", "endpoint": endpoint,
            "model": "canary-embed", "output_dimensions": null,
        }}),
    ))
    .unwrap();
    assert!(canary.wait_for(1, Duration::from_secs(5)));
    let before = canary.count();
    let reply = app
        .dispatch(request(
            &identity,
            json!({"kind": "retrieval_settings_get"}),
        ))
        .unwrap();
    let DesktopData::RetrievalSettings(current) = reply.data else {
        panic!("retrieval settings reply")
    };
    let mut params = serde_json::to_value(&current.params).unwrap();
    params["remote_consent"] = json!(false);
    app.dispatch(request(
        &identity,
        json!({"kind": "retrieval_settings_set", "payload": {
            "command_id": id(),
            "request": {"base_row_version": current.row_version, "params": params},
        }}),
    ))
    .unwrap();
    let mut connection = app.inference_connection(&identity).unwrap();
    let config = InferenceConfig {
        executable: PathBuf::new(),
        model_directory: PathBuf::new(),
        vector_root: PathBuf::new(),
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    let results = connection
        .search(
            &config,
            SearchQuery {
                query: "私有".into(),
                mode: SearchMode::Semantic,
                scope: SearchScope::default(),
                limit: 10,
                minimum_cosine: None,
            },
        )
        .unwrap();
    assert_eq!(
        results.unavailable_reason.as_deref(),
        Some("remote_consent_required")
    );
    assert!(!results.semantic_available);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        canary.count(),
        before,
        "revoked consent must stop new requests"
    );
}
