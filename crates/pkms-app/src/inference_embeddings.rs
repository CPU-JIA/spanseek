use crate::inference_credentials::{self, CredentialBinding};
use crate::{AppError, InferenceConfig, Result, WorkspaceIdentity};
use pkms_core::{EmbeddingProfile, EmbeddingProvider};
use pkms_inference::{remote_chunks, remote_embeddings};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

// IPC queries and index workers use different blocking threads. Keep the reactor
// alive between calls so pooled connections survive either thread's lifetime.
struct Transport {
    runtime: tokio::runtime::Runtime,
    client: remote_embeddings::Client,
}
fn transport() -> Result<Arc<Transport>> {
    static SHARED: Mutex<Option<Arc<Transport>>> = Mutex::new(None);
    let mut shared = SHARED.lock().map_err(|_| AppError::AppBusy)?;
    if let Some(value) = shared.as_ref() {
        return Ok(Arc::clone(value));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("pkms-embeddings")
        .enable_all()
        .build()?;
    let client = remote_embeddings::Client::new().map_err(|e| AppError::InvalidInput(e.0))?;
    let value = Arc::new(Transport { runtime, client });
    *shared = Some(Arc::clone(&value));
    // Only initialization is locked; no vault access or network awaits under it.
    Ok(value)
}

/// The local profile is the pinned E5 small model. It has no provider because
/// its bytes are verified by `pkms-inference`; remote profiles always name one.
pub(crate) fn local(profile: &EmbeddingProfile) -> bool {
    profile.spec.provider.is_none()
        && serde_json::to_value(&profile.spec).ok()
            == serde_json::to_value(pkms_inference::profile::profile_spec()).ok()
}

/// A profile this build can actually execute: pinned local E5 or a validated
/// remote profile. Used before scheduling a rebuild or a semantic query.
pub(crate) fn supported(profile: &EmbeddingProfile) -> bool {
    local(profile) || remote_chunks::valid(profile)
}

pub(crate) fn binding(
    identity: &WorkspaceIdentity,
    provider: EmbeddingProvider,
    endpoint: &str,
    model: &str,
) -> Result<CredentialBinding> {
    remote_embeddings::endpoint(endpoint).map_err(|e| AppError::InvalidInput(e.0))?;
    // Gemini keys travel in `x-goog-api-key` rather than as a bearer token, so
    // they live in their own vault namespace. Every other provider shares the
    // OpenAI namespace; the model is part of the binding either way, so two
    // models on one endpoint never reuse each other's credential.
    let protocol = match provider {
        EmbeddingProvider::Gemini => "gemini_embeddings",
        _ => "openai_embeddings",
    };
    CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        protocol,
        endpoint,
        model,
    )
    .map_err(crate::inference_remote::credential_error)
}

/// Which side of retrieval a text belongs to. Gemini embeds the two sides with
/// different task types, so every caller states which one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Document,
    Query,
}

pub(crate) fn wire(remote: &remote_chunks::RemoteProfile, kind: Kind) -> remote_embeddings::Wire {
    match remote.provider {
        EmbeddingProvider::Gemini => remote_embeddings::Wire::Gemini {
            base: remote.endpoint.clone(),
            model: remote.model.clone(),
            // gemini-embedding-2 rejects taskType, so the task has to live in the
            // prompt there; every older Gemini embedding model accepts it.
            task: (!remote.model.starts_with("gemini-embedding-2")).then_some(match kind {
                Kind::Document => remote_embeddings::GeminiTask::Document,
                Kind::Query => remote_embeddings::GeminiTask::Query,
            }),
            dimensions: remote.output_dimensions,
        },
        _ => remote_embeddings::Wire::OpenAi {
            base: remote.endpoint.clone(),
            model: remote.model.clone(),
            dimensions: remote.output_dimensions,
            // OpenAI and Cloudflare echo the requested model; a self-hosted
            // compatible server is not required to.
            check_model: matches!(
                remote.provider,
                EmbeddingProvider::Cloudflare | EmbeddingProvider::Openai
            ),
        },
    }
}

fn credential(
    identity: &WorkspaceIdentity,
    remote: &remote_chunks::RemoteProfile,
) -> Result<inference_credentials::Secret> {
    inference_credentials::load(&binding(
        identity,
        remote.provider,
        &remote.endpoint,
        &remote.model,
    )?)
    .map_err(crate::inference_remote::credential_error)?
    .ok_or(AppError::InvalidInput("model_key_missing"))
}

pub(crate) fn embed(
    identity: &WorkspaceIdentity,
    profile: &EmbeddingProfile,
    kind: Kind,
    input: &[String],
    config: &InferenceConfig,
    timeout: Duration,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Vec<f32>>> {
    let remote = remote_chunks::remote(profile)
        .ok_or(AppError::InvalidInput("embedding_remote_required"))?;
    let secret = credential(identity, &remote)?;
    let wire = wire(&remote, kind);
    let transport = transport()?;
    let started = Instant::now();
    let mut output = Vec::new();
    for batch in input.chunks(remote_embeddings::MAX_BATCH) {
        check()?;
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or(AppError::InvalidInput("inference_timeout"))?;
        let mut cause = None;
        let result = transport.runtime.block_on(transport.client.embed_wire(
            &wire,
            secret.expose_secret(),
            batch,
            Some(remote.dimensions as usize),
            remaining,
            || {
                if config.shutdown.load(Ordering::Acquire) {
                    return Err(remote_embeddings::Error("canceled"));
                }
                if let Err(error) = check() {
                    cause = Some(error);
                    return Err(remote_embeddings::Error("canceled"));
                }
                Ok(())
            },
        ));
        if let Some(error) = cause {
            return Err(error);
        }
        output.extend(result.map_err(|e| AppError::InvalidInput(e.0))?.vectors);
    }
    Ok(output)
}

/// What the save-time probe measured.
pub(crate) struct Probe {
    pub dimensions: u32,
    /// Provider-reported input tokens, when the provider returns them.
    pub prompt_tokens: Option<u64>,
}

/// One tiny request that proves the endpoint, the credential, the model and the
/// vector size the model really returns, before a profile is registered. The
/// size cannot be assumed: compatible servers, truncation and provider defaults
/// all change it.
pub(crate) fn probe(
    identity: &WorkspaceIdentity,
    remote: &remote_chunks::RemoteProfile,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<Probe> {
    let secret = credential(identity, remote)?;
    let wire = wire(remote, Kind::Document);
    let input = vec!["spanseek embedding probe".to_owned()];
    let transport = transport()?;
    let embedded = transport
        .runtime
        .block_on(transport.client.embed_wire(
            &wire,
            secret.expose_secret(),
            &input,
            None,
            Duration::from_secs(30),
            || {
                if shutdown.load(Ordering::Acquire) {
                    return Err(remote_embeddings::Error("canceled"));
                }
                Ok(())
            },
        ))
        .map_err(|e| AppError::InvalidInput(e.0))?;
    let dimensions = embedded
        .vectors
        .first()
        .map(Vec::len)
        .filter(|size| (1..=remote_chunks::MAX_DIMENSIONS as usize).contains(size))
        .ok_or(AppError::InvalidInput("embedding_invalid_vector"))?;
    Ok(Probe {
        dimensions: dimensions as u32,
        prompt_tokens: embedded.prompt_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> WorkspaceIdentity {
        WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    #[test]
    fn the_default_configuration_keeps_the_credential_it_already_stored() {
        let identity = identity();
        let endpoint = "https://example.invalid/ai/v1";
        let stored = CredentialBinding::new(
            &identity.local_instance_id,
            &identity.workspace_id,
            "openai_embeddings",
            endpoint,
            "@cf/baai/bge-m3",
        )
        .unwrap();
        let current = binding(
            &identity,
            EmbeddingProvider::Cloudflare,
            endpoint,
            "@cf/baai/bge-m3",
        )
        .unwrap();
        assert!(
            current == stored,
            "an upgrade must not orphan the token the user already stored"
        );
        // A second model on the same endpoint is a different credential, and
        // Gemini keys never share the bearer-token namespace.
        assert!(
            binding(
                &identity,
                EmbeddingProvider::Openai,
                endpoint,
                "text-embedding-3-small"
            )
            .unwrap()
                != stored
        );
        assert!(
            binding(
                &identity,
                EmbeddingProvider::Gemini,
                endpoint,
                "gemini-embedding-001"
            )
            .unwrap()
                != stored
        );
    }
    #[test]
    fn each_provider_maps_to_its_own_documented_wire() {
        let mut remote =
            remote_chunks::RemoteProfile::cloudflare("https://example.invalid/v1".into(), 512, 76);
        assert!(matches!(
            wire(&remote, Kind::Document),
            remote_embeddings::Wire::OpenAi {
                check_model: true,
                ..
            }
        ));
        remote.provider = EmbeddingProvider::OpenaiCompatible;
        assert!(matches!(
            wire(&remote, Kind::Document),
            remote_embeddings::Wire::OpenAi {
                check_model: false,
                ..
            }
        ));
        remote.provider = EmbeddingProvider::Gemini;
        remote.model = "gemini-embedding-001".into();
        match wire(&remote, Kind::Query) {
            remote_embeddings::Wire::Gemini { task, .. } => {
                assert_eq!(task, Some(remote_embeddings::GeminiTask::Query))
            }
            _ => panic!("Gemini must use its native batch endpoint"),
        }
        remote.model = "gemini-embedding-2".into();
        match wire(&remote, Kind::Document) {
            remote_embeddings::Wire::Gemini { task, .. } => {
                assert_eq!(task, None, "gemini-embedding-2 rejects taskType")
            }
            _ => panic!("Gemini must use its native batch endpoint"),
        }
    }
    #[test]
    fn reactor_survives_callers_and_can_drive_overlapping_requests() {
        let retained = std::thread::spawn(|| transport().unwrap()).join().unwrap();
        let shared = transport().unwrap();
        assert!(Arc::ptr_eq(&retained, &shared));
        let (signal, ready) = std::sync::mpsc::channel();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let first = std::thread::spawn(move || {
            retained.runtime.block_on(async {
                signal.send(()).unwrap();
                wait.await.unwrap();
            })
        });
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        shared.runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            release.send(()).unwrap();
        });
        first.join().unwrap();
    }
}
