//! Default timeouts for generation work, resolved from the configured provider.
//!
//! A model that runs on this machine may legitimately take minutes, while a
//! remote provider is network-bound and should fail faster. These are defaults
//! only: a caller that already knows its own budget always wins, and the chat
//! and agent run options stay adjustable per run.

/// Which generation purpose a default is being resolved for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationPurpose {
    Answer,
    Chat,
    Embedding,
}

/// The one provider that runs on this machine instead of over the network.
const LOCAL_PROVIDER: &str = "ollama";
/// A configured remote endpoint is network-bound; the in-process embedding
/// worker keeps the budget it always had.
const LOCAL_WORKER_EMBEDDING_SECONDS: u32 = 600;

/// Seconds allowed for one unit of `purpose` work on `provider`.
///
/// Every value stays inside the documented run-option bounds (10..=3600), so a
/// caller may store it back as an explicit option without rewriting it.
pub fn default_timeout_seconds(provider: &str, purpose: GenerationPurpose) -> u32 {
    match (provider == LOCAL_PROVIDER, purpose) {
        (true, GenerationPurpose::Answer) => 600,
        (true, GenerationPurpose::Chat) => 900,
        (true, GenerationPurpose::Embedding) => 900,
        (false, GenerationPurpose::Answer) => 180,
        (false, GenerationPurpose::Chat) => 300,
        (false, GenerationPurpose::Embedding) => 120,
    }
}

/// Seconds allowed for one embedding batch. Only a configured remote endpoint is
/// resolved from a provider; the local worker keeps its historical budget.
pub fn embedding_timeout_seconds(remote_endpoint: bool) -> u32 {
    if remote_endpoint {
        default_timeout_seconds("remote", GenerationPurpose::Embedding)
    } else {
        LOCAL_WORKER_EMBEDDING_SECONDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_differ_by_provider_and_purpose() {
        assert_eq!(
            default_timeout_seconds("ollama", GenerationPurpose::Answer),
            600
        );
        assert_eq!(
            default_timeout_seconds("openai_chat", GenerationPurpose::Answer),
            180
        );
        assert_eq!(
            default_timeout_seconds("ollama", GenerationPurpose::Chat),
            900
        );
        assert_eq!(
            default_timeout_seconds("anthropic", GenerationPurpose::Chat),
            300
        );
        assert_eq!(
            default_timeout_seconds("gemini", GenerationPurpose::Embedding),
            120
        );
        // A local model is never given a shorter budget than a remote one.
        for purpose in [
            GenerationPurpose::Answer,
            GenerationPurpose::Chat,
            GenerationPurpose::Embedding,
        ] {
            assert!(
                default_timeout_seconds(LOCAL_PROVIDER, purpose)
                    > default_timeout_seconds("openai_chat", purpose)
            );
            for provider in [LOCAL_PROVIDER, "openai_chat", "gemini", "cloudflare"] {
                let value = default_timeout_seconds(provider, purpose);
                assert!(
                    (10..=3600).contains(&value),
                    "{provider} {purpose:?} resolved to {value}"
                );
            }
        }
    }

    #[test]
    fn the_local_embedding_worker_keeps_its_historical_budget() {
        assert_eq!(
            embedding_timeout_seconds(false),
            LOCAL_WORKER_EMBEDDING_SECONDS
        );
        assert_eq!(embedding_timeout_seconds(true), 120);
    }
}
