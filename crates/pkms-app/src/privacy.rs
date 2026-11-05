//! One place that decides whether text may leave this device.
use crate::{AppError, Result};
use pkms_core::{GenerationProfile, Library};

/// Remote metadata configuration and remote calls both require the library-level
/// acknowledgement. Keeping this check in one place prevents one entry point from
/// silently outliving the switch the user turned off.
pub(crate) fn require_remote_consent(library: &Library) -> Result<()> {
    if !library.retrieval_settings()?.params.remote_consent {
        return Err(AppError::InvalidInput("remote_consent_required"));
    }
    Ok(())
}

/// Ollama and OpenAI-compatible servers bound to a literal loopback address
/// never send text away. Everything else is remote by construction.
pub(crate) fn local_generation(profile: &GenerationProfile) -> bool {
    if profile.provider == "ollama" {
        return true;
    }
    reqwest::Url::parse(&profile.endpoint).is_ok_and(|endpoint| {
        endpoint.scheme() == "http"
            && endpoint
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
    })
}

pub(crate) fn require_generation_consent(
    library: &Library,
    profile: &GenerationProfile,
) -> Result<()> {
    if !local_generation(profile) {
        require_remote_consent(library)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn profile(provider: &str) -> GenerationProfile {
        GenerationProfile {
            provider: provider.into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "synthetic".into(),
            model_digest: String::new(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 4096,
            chat_token_limit: None,
            channel_id: None,
            capabilities: None,
        }
    }

    #[test]
    fn local_ollama_and_loopback_openai_are_exempt_but_remote_generation_requires_consent() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path()).unwrap();
        assert!(require_generation_consent(&library, &profile("ollama")).is_ok());
        let mut loopback = profile("openai_chat");
        loopback.endpoint = "http://127.0.0.1:11436/v1".into();
        assert!(require_generation_consent(&library, &loopback).is_ok());
        let mut remote = profile("openai_chat");
        remote.endpoint = "https://example.test/v1".into();
        assert!(matches!(
            require_generation_consent(&library, &remote),
            Err(AppError::InvalidInput("remote_consent_required"))
        ));
    }
}
