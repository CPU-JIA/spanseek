use crate::{AppError, Result, inference_model_policy};
use pkms_core::GenerationProfile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use ts_rs::TS;
mod remote;
pub use remote::list_remote_generation_models;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct InstalledGenerationModel {
    pub name: String,
    pub digest: String,
    #[ts(type = "number")]
    pub size: u64,
    pub supported: bool,
    pub reason: Option<String>,
    pub profile: Option<GenerationProfile>,
    pub context_min: Option<u32>,
    pub context_max: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GenerationModels {
    pub endpoint: String,
    pub runtime_version: String,
    pub models: Vec<InstalledGenerationModel>,
}

pub(crate) struct ModelService {
    client: reqwest::Client,
    endpoint: reqwest::Url,
}

impl ModelService {
    pub(crate) fn new(endpoint: &str) -> Result<Self> {
        let endpoint = reqwest::Url::parse(endpoint)
            .map_err(|_| AppError::InvalidInput("invalid_model_endpoint"))?;
        if endpoint.scheme() != "http"
            || !matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost"))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
        {
            return Err(AppError::InvalidInput("invalid_model_endpoint"));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| AppError::InvalidInput("model_unavailable"))?;
        Ok(Self { client, endpoint })
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Value> {
        model_json(
            self.client.get(
                self.endpoint
                    .join(path)
                    .map_err(|_| AppError::InvalidInput("invalid_model_endpoint"))?,
            ),
        )
        .await
    }

    pub(crate) async fn show(&self, model: &str) -> Result<Value> {
        model_json(
            self.client
                .post(
                    self.endpoint
                        .join("api/show")
                        .map_err(|_| AppError::InvalidInput("invalid_model_endpoint"))?,
                )
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(&serde_json::json!({"model": model}))?),
        )
        .await
    }
}

async fn model_json(request: reqwest::RequestBuilder) -> Result<Value> {
    let mut response = request
        .send()
        .await
        .map_err(|_| AppError::InvalidInput("model_unavailable"))?;
    if !response.status().is_success() {
        return Err(AppError::InvalidInput("model_unavailable"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::InvalidInput("model_unavailable"))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(AppError::InvalidInput("model_response_limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| AppError::InvalidInput("model_response_invalid"))
}

pub(crate) fn run_model_request<T>(
    shutdown: &AtomicBool,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    if shutdown.load(Ordering::Acquire) {
        return Err(AppError::InvalidInput("worker_interrupted"));
    }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        tokio::pin!(operation);
        loop {
            tokio::select! {
                result = &mut operation => return result,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if shutdown.load(Ordering::Acquire) { return Err(AppError::InvalidInput("worker_interrupted")); }
                }
            }
        }
    })
}

pub(crate) fn runtime_version(body: &Value) -> Result<&str> {
    body.get("version")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 80)
        .ok_or(AppError::InvalidInput("model_response_invalid"))
}

fn installed_models(body: &Value, endpoint: &str) -> Result<Vec<InstalledGenerationModel>> {
    let invalid = || AppError::InvalidInput("model_response_invalid");
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .filter(|m| m.len() <= 256)
        .ok_or_else(invalid)?;
    let mut names = std::collections::HashSet::new();
    models
        .iter()
        .map(|model| {
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or_else(invalid)?;
            if !names.insert(name) {
                return Err(invalid());
            }
            let digest = model
                .get("digest")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid());
            }
            let size = model
                .get("size")
                .and_then(Value::as_u64)
                .filter(|size| *size < (1_u64 << 53))
                .ok_or_else(invalid)?;
            let local = size > 0
                && model.get("remote_model").is_none()
                && model.get("remote_host").is_none();
            let profile = local
                .then(|| inference_model_policy::candidate_profile(endpoint, name, digest))
                .flatten();
            Ok(InstalledGenerationModel {
                name: name.into(),
                digest: digest.into(),
                size,
                supported: false,
                reason: Some(
                    if !local {
                        "model_not_local"
                    } else {
                        "model_profile_unsupported"
                    }
                    .into(),
                ),
                profile,
                context_min: None,
                context_max: None,
            })
        })
        .collect()
}

/// Enumerate actual local tags. Only exact admitted identities are probed with /show.
pub fn list_generation_models(endpoint: &str, shutdown: &AtomicBool) -> Result<GenerationModels> {
    let service = ModelService::new(endpoint)?;
    run_model_request(shutdown, async {
        let tags = service.get("api/tags").await?;
        let version = service.get("api/version").await?;
        let runtime_version = runtime_version(&version)?.to_owned();
        let endpoint = service.endpoint.as_str().to_owned();
        let mut models = installed_models(&tags, &endpoint)?;
        for model in &mut models {
            let Some(profile) = model.profile.take() else {
                continue;
            };
            let verified = match service.show(&model.name).await {
                Ok(show) => {
                    inference_model_policy::validate_show(&profile, &show, &runtime_version)
                }
                Err(error) => Err(error),
            };
            match verified {
                Ok(()) => {
                    model.supported = true;
                    model.reason = None;
                    model.profile = Some(profile);
                    model.context_min = Some(inference_model_policy::MODEL_CONTEXT_RANGE.0);
                    model.context_max = Some(inference_model_policy::MODEL_CONTEXT_RANGE.1);
                }
                Err(AppError::InvalidInput(reason)) => model.reason = Some(reason.into()),
                Err(error) => return Err(error),
            }
        }
        models.sort_by(|a, b| b.supported.cmp(&a.supported).then(a.name.cmp(&b.name)));
        Ok(GenerationModels {
            endpoint,
            runtime_version,
            models,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};

    fn server(responses: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            for (path, response) in responses {
                let until = std::time::Instant::now() + Duration::from_secs(8);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < until, "missing request {path}");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    request.extend_from_slice(&byte);
                }
                let head = String::from_utf8(request).unwrap();
                assert!(head.lines().next().unwrap().contains(path), "{head}");
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                socket.read_exact(&mut vec![0; length]).unwrap();
                let _ = socket.write_all(response.as_bytes());
            }
        });
        (endpoint, thread)
    }
    fn ok(body: Value) -> String {
        let body = body.to_string();
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }
    #[test]
    fn installed_identity_and_show_must_both_pass() {
        let digest = "2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd";
        let (endpoint, thread) = server(vec![
            (
                "/api/tags",
                ok(json!({"models":[
                    {"name":"qwen3.5:4b", "digest":digest, "size":3000000000u64},
                    {"name":"qwen3.5:9b", "digest":"f".repeat(64), "size":6000000000u64},
                    {"name":"remote", "digest":"a".repeat(64), "size":1, "remote_model":"cloud"}
                ]})),
            ),
            ("/api/version", ok(json!({"version":"0.35.1"}))),
            (
                "/api/show",
                ok(
                    json!({"template":"{{ .Prompt }}", "requires":"0.17.1", "details":{"format":"gguf","family":"qwen35","quantization_level":"Q4_K_M"},"model_info":{"general.architecture":"qwen35","tokenizer.ggml.model":"gpt2","qwen35.context_length":262144},"capabilities":["completion"]}),
                ),
            ),
        ]);
        let result = list_generation_models(&endpoint, &AtomicBool::new(false)).unwrap();
        thread.join().unwrap();
        assert_eq!(result.models.len(), 3);
        assert!(result.models[0].supported);
        assert_eq!(
            result.models[0].profile.as_ref().unwrap().model_digest,
            digest
        );
        assert_eq!(result.models[0].context_max, Some(8192));
        assert_eq!(
            result.models[1].reason.as_deref(),
            Some("model_profile_unsupported")
        );
        assert!(result.models[1].profile.is_none());
        assert_eq!(result.models[2].reason.as_deref(), Some("model_not_local"));
    }
    #[test]
    fn wrong_runtime_remains_visible_but_not_selectable() {
        let (endpoint, thread) = server(vec![
            (
                "/api/tags",
                ok(
                    json!({"models":[{"name":"qwen3.5:4b", "digest":"2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd", "size":1}]}),
                ),
            ),
            ("/api/version", ok(json!({"version":"0.35.2"}))),
            ("/api/show", ok(json!({"template":"{{ .Prompt }}"}))),
        ]);
        let result = list_generation_models(&endpoint, &AtomicBool::new(false)).unwrap();
        thread.join().unwrap();
        assert!(!result.models[0].supported);
        assert!(result.models[0].profile.is_none());
        assert_eq!(
            result.models[0].reason.as_deref(),
            Some("model_profile_unsupported")
        );
    }
    #[test]
    fn network_boundaries_and_response_limit_fail_closed() {
        for endpoint in [
            "https://127.0.0.1:11434",
            "http://example.com",
            "http://user@localhost",
            "http://localhost/api",
            "http://localhost/?q=1",
        ] {
            assert!(ModelService::new(endpoint).is_err());
        }
        let (endpoint, thread) = server(vec![("/api/tags", "HTTP/1.1 302 Found\r\nLocation: http://example.com\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into())]);
        assert!(matches!(
            list_generation_models(&endpoint, &AtomicBool::new(false)),
            Err(AppError::InvalidInput("model_unavailable"))
        ));
        thread.join().unwrap();
        let body = "x".repeat(1024 * 1024 + 1);
        let (endpoint, thread) = server(vec![(
            "/api/tags",
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )]);
        assert!(matches!(
            list_generation_models(&endpoint, &AtomicBool::new(false)),
            Err(AppError::InvalidInput("model_response_limit"))
        ));
        thread.join().unwrap();
    }
    #[test]
    fn malformed_duplicate_and_empty_lists_are_distinct() {
        assert!(installed_models(&json!({}), "http://localhost").is_err());
        assert!(
            installed_models(&json!({"models":[]}), "http://localhost")
                .unwrap()
                .is_empty()
        );
        let model = json!({"name":"other", "digest":"a".repeat(64), "size":1});
        assert!(
            installed_models(&json!({"models":[model.clone(),model]}), "http://localhost").is_err()
        );
        assert!(matches!(
            run_model_request(&AtomicBool::new(true), async { Ok(()) }),
            Err(AppError::InvalidInput("worker_interrupted"))
        ));
        assert!(
            installed_models(
                &json!({"models":[{"name":"other","digest":"a".repeat(64),"size":u64::MAX}]}),
                "http://localhost"
            )
            .is_err()
        );
    }

    #[test]
    #[ignore = "requires isolated Ollama with verified qwen3.5:4b installed"]
    fn real_installed_models_are_discovered_without_loading_or_generating() {
        let endpoint = std::env::var("PKMS_TEST_OLLAMA_ENDPOINT")
            .expect("PKMS_TEST_OLLAMA_ENDPOINT required for real discovery probe");
        let models = list_generation_models(&endpoint, &AtomicBool::new(false)).unwrap();
        assert_eq!(models.runtime_version, "0.35.1");
        let model = models
            .models
            .iter()
            .find(|model| model.name == "qwen3.5:4b")
            .expect("4B is installed");
        assert!(model.supported, "{:?}", model.reason);
        let profile = model.profile.as_ref().unwrap();
        assert_eq!(
            profile.model_digest,
            "2a654d98e6fba55d452b7043684e9b57a947e393bbffa62485a7aac05ee4eefd"
        );
        assert_eq!(profile.context_tokens, 8192);
        assert!(
            models
                .models
                .iter()
                .filter(|model| model.name == "qwen3.5:9b")
                .all(|model| !model.supported)
        );
    }
}
