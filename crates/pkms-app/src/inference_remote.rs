//! Remote generation runs on the existing background thread; secrets never cross worker IPC.
use crate::{AppError, Result, WorkspaceIdentity, inference_credentials as vault};
use pkms_core::{GenerationChannel, GenerationProfile, GenerationTransport};
use pkms_inference::remote::{self, Protocol};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn credential_error(error: vault::CredentialError) -> AppError {
    AppError::InvalidInput(match error {
        vault::CredentialError::InvalidBinding => "invalid_model_endpoint",
        vault::CredentialError::InvalidSecret => "model_invalid_key",
        vault::CredentialError::Unavailable => "model_credential_unavailable",
        vault::CredentialError::Unsupported => "model_credential_unsupported",
    })
}
pub(crate) fn protocol(profile: &GenerationProfile) -> Result<Protocol> {
    match profile.provider.as_str() {
        "openai_chat" => Ok(Protocol::ChatCompletions),
        "openai_responses" => Ok(Protocol::Responses),
        "anthropic" => Ok(Protocol::AnthropicMessages),
        "gemini" => Ok(Protocol::Gemini),
        _ => Err(AppError::InvalidInput("model_profile_unsupported")),
    }
}
pub(crate) fn credential_binding(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
) -> Result<vault::CredentialBinding> {
    pkms_core::validate_generation_profile(profile)?;
    protocol(profile)?;
    if let Some(channel_id) = &profile.channel_id {
        return vault::CredentialBinding::new_channel(
            &identity.local_instance_id,
            &identity.workspace_id,
            channel_id,
            &profile.provider,
            &profile.endpoint,
        )
        .map_err(credential_error);
    }
    vault::CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        &profile.provider,
        &profile.endpoint,
        &profile.model,
    )
    .map_err(credential_error)
}

pub fn generation_channel_credential_binding(
    identity: &WorkspaceIdentity,
    channel: &GenerationChannel,
) -> Result<vault::CredentialBinding> {
    if channel.deleted || channel.row_version == 0 {
        return Err(AppError::InvalidInput("generation_channel_missing"));
    }
    pkms_core::validate_generation_channel_spec(&channel.spec)?;
    vault::CredentialBinding::new_channel(
        &identity.local_instance_id,
        &identity.workspace_id,
        &channel.id,
        &channel.spec.provider,
        &channel.spec.endpoint,
    )
    .map_err(credential_error)
}

fn request_transport(
    profile: &GenerationProfile,
    request: &remote::Request,
) -> Result<GenerationTransport> {
    if request.model != profile.model {
        return Err(AppError::InvalidInput("model_invalid_request"));
    }
    let transport = match &profile.capabilities {
        None => GenerationTransport::Stream,
        Some(capabilities) if request.output_format.is_some() => capabilities
            .structured_transport
            .ok_or(AppError::InvalidInput("model_unsupported_output"))?,
        Some(capabilities) => capabilities.text_transport,
    };
    if let Ok(endpoint) = reqwest::Url::parse(&profile.endpoint)
        && endpoint.host_str() == Some("api.cloudflare.com")
        && endpoint.path().trim_end_matches('/').ends_with("/ai/v1")
        && transport == GenerationTransport::Stream
        && (profile.provider == "openai_responses" || request.output_format.is_some())
    {
        return Err(AppError::InvalidInput("cloudflare_transport_unsupported"));
    }
    Ok(transport)
}
fn generate(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    request: remote::Request,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<remote::Completion> {
    generate_with_limits(identity, profile, request, remote::Limits::default(), check)
}

pub(crate) fn generate_with_limits(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    request: remote::Request,
    limits: remote::Limits,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<remote::Completion> {
    generate_streaming(identity, profile, request, limits, &mut |_| Ok(()), check)
}

pub(crate) fn generate_streaming(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    request: remote::Request,
    limits: remote::Limits,
    on_event: &mut dyn FnMut(remote::Event) -> Result<()>,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<remote::Completion> {
    check()?;
    let transport = request_transport(profile, &request)?;
    let binding = credential_binding(identity, profile)?;
    let secret = vault::load(&binding).map_err(credential_error)?;
    let key = match secret {
        Some(secret) => remote::ApiKey::new(secret.expose_secret().to_owned()),
        // Local OpenAI-compatible servers usually ignore authorization. A
        // fixed non-secret value keeps the wire shape identical without ever
        // asking the user to store a fake credential.
        None if crate::privacy::local_generation(profile) => remote::ApiKey::new("local".into()),
        None => return Err(AppError::InvalidInput("model_key_missing")),
    }
    .map_err(|e| AppError::InvalidInput(e.code()))?;
    let client = remote::Client::new_with_transport(
        protocol(profile)?,
        &profile.endpoint,
        limits,
        transport,
    )
    .and_then(|client| client.with_channel_id(profile.channel_id.as_deref()))
    .map_err(|e| AppError::InvalidInput(e.code()))?;
    let interrupted = std::cell::RefCell::new(None);
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(client.generate(
            &request,
            &key,
            |event| {
                on_event(event).map_err(|error| {
                    *interrupted.borrow_mut() = Some(error);
                    remote::Error::Canceled
                })
            },
            || match check() {
                Ok(()) => false,
                Err(error) => {
                    *interrupted.borrow_mut() = Some(error);
                    true
                }
            },
        ));
    if let Some(error) = interrupted.into_inner() {
        return Err(error);
    }
    check()?;
    result.map_err(|e| AppError::InvalidInput(e.code()))
}

pub fn verify_remote_generation_model(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    shutdown: &AtomicBool,
) -> Result<()> {
    let request = remote::Request {
        chat_token_limit: profile.chat_token_limit,
        model: profile.model.clone(),
        messages: vec![
            remote::Message::System { text: include_str!("inference_answer_prompt.txt").trim().into() },
            remote::Message::User { text: "Question: 测试记录的颜色是什么？\nEvidence:\n[E1] 测试记录\n此测试使用的颜色是蓝色。".into() },
        ],
        tools: vec![],
        max_output_tokens: 8192.min(profile.context_tokens / 2),
        temperature: Some(profile.temperature),
        output_format: Some(pkms_inference::chat_format::ChatFormat::EvidenceAnswer { citation_ids: vec!["E1".into()] }),
    };
    let completion = generate(identity, profile, request, &mut || {
        if shutdown.load(Ordering::Acquire) {
            Err(AppError::InvalidInput("worker_interrupted"))
        } else {
            Ok(())
        }
    })?;
    if completion.finish != remote::Finish::Stop
        || completion.text.trim().is_empty()
        || !completion.tool_calls.is_empty()
    {
        return Err(AppError::InvalidInput("model_response_invalid"));
    }
    match crate::inference_answer_format::decode(&completion.text, &["E1".into()])? {
        crate::inference_answer_format::Answer::Supported { .. } => Ok(()),
        crate::inference_answer_format::Answer::InsufficientEvidence => {
            Err(AppError::InvalidInput("model_response_invalid"))
        }
    }
}

/// Returns the model text together with the provider-reported usage so the
/// statistics panel can count calls and tokens without re-deriving them.
pub(crate) fn answer(
    identity: &WorkspaceIdentity,
    profile: &GenerationProfile,
    messages: Vec<pkms_inference::protocol::ChatMessage>,
    allowed: Vec<String>,
    max_output_tokens: u32,
    total_timeout: std::time::Duration,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<(String, remote::Usage, Option<std::time::Duration>)> {
    let request = remote::Request {
        chat_token_limit: profile.chat_token_limit,
        model: profile.model.clone(),
        messages: messages
            .into_iter()
            .map(|message| match message.role {
                pkms_inference::protocol::ChatRole::System => remote::Message::System {
                    text: message.content,
                },
                pkms_inference::protocol::ChatRole::User => remote::Message::User {
                    text: message.content,
                },
                pkms_inference::protocol::ChatRole::Assistant => remote::Message::Assistant {
                    text: message.content,
                    tool_calls: vec![],
                    continuation: None,
                },
            })
            .collect(),
        tools: vec![],
        max_output_tokens,
        temperature: Some(profile.temperature),
        output_format: Some(pkms_inference::chat_format::ChatFormat::EvidenceAnswer {
            citation_ids: allowed,
        }),
    };
    // The strict evidence path publishes no partial text, but the provider's
    // first delta still measures the time to first token, which the panel
    // reports separately from the total call duration.
    let started = std::time::Instant::now();
    let mut first_token = None;
    // One cited answer is bounded by the caller's budget: the library setting
    // when present, otherwise the provider default. The first-event and idle
    // guards stay narrower so a stalled stream still fails before the total.
    let limits = remote::Limits {
        total_timeout,
        first_event_timeout: total_timeout.min(std::time::Duration::from_secs(90)),
        idle_timeout: total_timeout.min(std::time::Duration::from_secs(30)),
        ..remote::Limits::default()
    };
    let result = generate_streaming(
        identity,
        profile,
        request,
        limits,
        &mut |event| {
            if first_token.is_none()
                && matches!(&event, remote::Event::TextDelta { text } if !text.is_empty())
            {
                first_token = Some(started.elapsed());
            }
            Ok(())
        },
        check,
    )?;
    if result.finish != remote::Finish::Stop || !result.tool_calls.is_empty() {
        return Err(AppError::InvalidInput("model_output_truncated"));
    }
    if result.text.len() > 1024 * 1024 {
        return Err(AppError::InvalidInput("answer_output_limit"));
    }
    Ok((result.text, result.usage, first_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> GenerationProfile {
        GenerationProfile {
            chat_token_limit: None,
            provider: "openai_chat".into(),
            endpoint: "https://example.com/v1".into(),
            model: "test-model".into(),
            model_digest: String::new(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 8192,
            channel_id: None,
            capabilities: None,
        }
    }
    #[test]
    fn remote_binding_reuses_core_validation_and_rejects_local_profiles() {
        let identity = WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        };
        assert!(credential_binding(&identity, &profile()).is_ok());
        for endpoint in [
            "http://example.com/v1",
            "https://user@example.com",
            "https://@example.com",
            "https://example.com?key=secret",
        ] {
            let mut invalid = profile();
            invalid.endpoint = endpoint.into();
            assert!(credential_binding(&identity, &invalid).is_err());
        }
        let mut invalid = profile();
        invalid.model_digest = "invented-digest".into();
        assert!(credential_binding(&identity, &invalid).is_err());
        invalid = profile();
        invalid.context_tokens = 1;
        assert!(credential_binding(&identity, &invalid).is_err());
    }
    #[test]
    fn canceled_remote_request_never_requires_credentials_or_connects() {
        let identity = WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        };
        let request = remote::Request {
            chat_token_limit: None,
            model: "test-model".into(),
            messages: vec![remote::Message::User {
                text: "test".into(),
            }],
            tools: vec![],
            max_output_tokens: 10,
            temperature: None,
            output_format: None,
        };
        assert!(matches!(
            generate(&identity, &profile(), request, &mut || Err(
                AppError::InvalidInput("canceled-before-network")
            )),
            Err(AppError::InvalidInput("canceled-before-network"))
        ));
    }
    #[test]
    fn credential_ipc_debug_and_errors_never_expose_secrets() {
        let request: crate::DesktopCommand = serde_json::from_value(serde_json::json!({"kind":"generation_credential_set", "payload":{"profile":profile(),"api_key":"synthetic-not-a-real-key"}})).unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("synthetic-not-a-real-key"));
        assert!(debug.contains("redacted"));
        let error = crate::ApiError::from_app(
            AppError::InvalidInput("model_authentication_failed"),
            "test",
        );
        let json = serde_json::to_value(error).unwrap();
        assert!(json.to_string().contains("model_authentication_failed"));
    }

    fn channel_profile() -> GenerationProfile {
        GenerationProfile {
            channel_id: Some("680cbe03-031e-46c5-8e64-67dd5e5731d8".into()),
            capabilities: Some(pkms_core::GenerationCapabilities {
                text_transport: GenerationTransport::Stream,
                structured_transport: Some(GenerationTransport::Json),
            }),
            ..profile()
        }
    }
    fn channel_request() -> remote::Request {
        remote::Request {
            model: "test-model".into(),
            messages: vec![remote::Message::User {
                text: "synthetic".into(),
            }],
            tools: vec![],
            max_output_tokens: 64,
            chat_token_limit: None,
            temperature: None,
            output_format: None,
        }
    }

    #[test]
    fn new_channel_models_share_only_their_channel_credential() {
        let identity = WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        };
        let first = channel_profile();
        let mut second = first.clone();
        second.model = "other-model".into();
        assert!(
            credential_binding(&identity, &first).unwrap()
                == credential_binding(&identity, &second).unwrap()
        );
        second.channel_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(
            credential_binding(&identity, &first).unwrap()
                != credential_binding(&identity, &second).unwrap()
        );
        let mut legacy = first.clone();
        legacy.channel_id = None;
        legacy.capabilities = None;
        assert!(
            credential_binding(&identity, &first).unwrap()
                != credential_binding(&identity, &legacy).unwrap()
        );
        second = legacy.clone();
        second.model = "other-model".into();
        assert!(
            credential_binding(&identity, &legacy).unwrap()
                != credential_binding(&identity, &second).unwrap()
        );
    }

    #[test]
    fn new_channel_empty_catalog_can_bind_credentials_before_discovery() {
        let identity = WorkspaceIdentity {
            local_instance_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: uuid::Uuid::new_v4().to_string(),
        };
        let profile = channel_profile();
        let mut channel = GenerationChannel {
            id: profile.channel_id.clone().unwrap(),
            row_version: 1,
            deleted: false,
            created_at: "2026-10-09T00:00:00Z".into(),
            updated_at: "2026-10-09T00:00:00Z".into(),
            spec: pkms_core::GenerationChannelSpec {
                name: "Synthetic channel".into(),
                provider: profile.provider.clone(),
                endpoint: profile.endpoint.clone(),
                models: vec![],
                capabilities: profile.capabilities.clone().unwrap(),
                preset: None,
            },
        };
        assert!(
            generation_channel_credential_binding(&identity, &channel).unwrap()
                == credential_binding(&identity, &profile).unwrap()
        );
        channel.deleted = true;
        assert!(generation_channel_credential_binding(&identity, &channel).is_err());
    }

    #[test]
    fn new_channel_transport_is_selected_explicitly_per_requested_capability() {
        let mut profile = channel_profile();
        let mut request = channel_request();
        assert_eq!(
            request_transport(&profile, &request).unwrap(),
            GenerationTransport::Stream
        );
        request.output_format = Some(pkms_inference::chat_format::ChatFormat::EvidenceAnswer {
            citation_ids: vec!["E1".into()],
        });
        assert_eq!(
            request_transport(&profile, &request).unwrap(),
            GenerationTransport::Json
        );
        profile.capabilities.as_mut().unwrap().structured_transport = None;
        assert!(matches!(
            request_transport(&profile, &request),
            Err(AppError::InvalidInput("model_unsupported_output"))
        ));
        request.model = "wrong-model".into();
        assert!(matches!(
            request_transport(&profile, &request),
            Err(AppError::InvalidInput("model_invalid_request"))
        ));
    }

    #[test]
    fn new_channel_cloudflare_cannot_stream_json_mode_or_responses() {
        let mut profile = channel_profile();
        profile.endpoint = "https://api.cloudflare.com/client/v4/accounts/synthetic/ai/v1".into();
        let mut request = channel_request();
        assert_eq!(
            request_transport(&profile, &request).unwrap(),
            GenerationTransport::Stream
        );
        request.output_format = Some(pkms_inference::chat_format::ChatFormat::EvidenceAnswer {
            citation_ids: vec!["E1".into()],
        });
        profile.capabilities.as_mut().unwrap().structured_transport =
            Some(GenerationTransport::Stream);
        assert!(matches!(
            request_transport(&profile, &request),
            Err(AppError::InvalidInput("cloudflare_transport_unsupported"))
        ));
        request.output_format = None;
        profile.provider = "openai_responses".into();
        assert!(matches!(
            request_transport(&profile, &request),
            Err(AppError::InvalidInput("cloudflare_transport_unsupported"))
        ));
        profile.capabilities.as_mut().unwrap().text_transport = GenerationTransport::Json;
        assert_eq!(
            request_transport(&profile, &request).unwrap(),
            GenerationTransport::Json
        );
    }
}
