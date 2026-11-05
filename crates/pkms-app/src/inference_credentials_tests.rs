use super::*;

const LOCAL: &str = "cdddeec7-9e46-4fe4-9cdd-fb836c3c4639";
const WORKSPACE: &str = "041e7a20-1bbe-4c90-830f-b1cc25ab3264";

fn binding(endpoint: &str) -> CredentialBinding {
    CredentialBinding::new(LOCAL, WORKSPACE, "openai_chat", endpoint, "model-a").unwrap()
}

#[test]
fn binding_normalizes_host_port_and_trailing_slash_only() {
    assert_eq!(
        binding("https://EXAMPLE.COM:443/api/v1/").target,
        binding("https://example.com/api/v1").target
    );
    assert_eq!(
        binding("https://example.com").target,
        binding("https://example.com/").target
    );
    assert_ne!(
        binding("https://example.com/api").target,
        binding("https://example.com/API").target
    );
}

#[test]
fn binding_isolates_every_authorization_dimension() {
    let baseline = binding("https://example.com/v1");
    for (local, workspace, protocol, endpoint, model) in [
        (
            WORKSPACE,
            WORKSPACE,
            "openai_chat",
            "https://example.com/v1",
            "model-a",
        ),
        (
            LOCAL,
            LOCAL,
            "openai_chat",
            "https://example.com/v1",
            "model-a",
        ),
        (
            LOCAL,
            WORKSPACE,
            "openai_responses",
            "https://example.com/v1",
            "model-a",
        ),
        (
            LOCAL,
            WORKSPACE,
            "anthropic",
            "https://example.com/v1",
            "model-a",
        ),
        (
            LOCAL,
            WORKSPACE,
            "openai_chat",
            "https://other.example/v1",
            "model-a",
        ),
        (
            LOCAL,
            WORKSPACE,
            "openai_chat",
            "https://example.com/v2",
            "model-a",
        ),
        (
            LOCAL,
            WORKSPACE,
            "openai_chat",
            "https://example.com/v1",
            "model-b",
        ),
    ] {
        assert_ne!(
            baseline.target,
            CredentialBinding::new(local, workspace, protocol, endpoint, model)
                .unwrap()
                .target
        );
    }
    assert!(!baseline.target.contains("example"));
    assert!(!baseline.target.contains(LOCAL));
}

#[test]
fn credentials_reject_header_injection_and_invalid_bindings() {
    for value in ["", "key\r\nInjected: yes", " key", "key ", "☃", "key\0"] {
        assert!(matches!(
            Secret::new(value.into()),
            Err(CredentialError::InvalidSecret)
        ));
    }
    assert!(Secret::new("x".repeat(2561)).is_err());
    assert_eq!(
        Secret::new("sk-test_ABC-123".into())
            .unwrap()
            .expose_secret(),
        "sk-test_ABC-123"
    );
    for endpoint in [
        "http://example.com",
        "https://u:p@example.com",
        "https://example.com?key=x",
        "https://example.com/#x",
        " https://example.com",
        "https://exa\nmple.com",
    ] {
        assert!(matches!(
            CredentialBinding::new(LOCAL, WORKSPACE, "openai_chat", endpoint, "model"),
            Err(CredentialError::InvalidBinding)
        ));
    }
    for (local, workspace, protocol, model) in [
        ("invalid", WORKSPACE, "openai_chat", "model"),
        (
            LOCAL,
            "00000000-0000-0000-0000-000000000000",
            "openai_chat",
            "model",
        ),
        (LOCAL, WORKSPACE, "unknown", "model"),
        (LOCAL, WORKSPACE, "openai_chat", " model"),
    ] {
        assert!(
            CredentialBinding::new(local, workspace, protocol, "https://example.com", model)
                .is_err()
        );
    }
}

#[cfg(windows)]
#[test]
#[ignore = "writes a random synthetic credential to the current Windows user's vault"]
fn windows_vault_round_trip_and_endpoint_isolation() {
    let local = uuid::Uuid::new_v4().to_string();
    let binding = CredentialBinding::new(
        &local,
        WORKSPACE,
        "openai_chat",
        "https://pkms-credential-test.invalid",
        "test-only",
    )
    .unwrap();
    struct Cleanup(CredentialBinding);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = delete(&self.0);
        }
    }
    let cleanup = Cleanup(binding);
    assert!(!exists(&cleanup.0).unwrap());
    store(
        &cleanup.0,
        &Secret::new("synthetic-test-value".into()).unwrap(),
    )
    .unwrap();
    assert!(exists(&cleanup.0).unwrap());
    assert_eq!(
        load(&cleanup.0).unwrap().unwrap().expose_secret(),
        "synthetic-test-value"
    );
    let other = CredentialBinding::new(
        &local,
        WORKSPACE,
        "openai_chat",
        "https://different.invalid",
        "test-only",
    )
    .unwrap();
    assert!(load(&other).unwrap().is_none());
    store(
        &cleanup.0,
        &Secret::new("replacement-test-value".into()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        load(&cleanup.0).unwrap().unwrap().expose_secret(),
        "replacement-test-value"
    );
    delete(&cleanup.0).unwrap();
    assert!(!exists(&cleanup.0).unwrap());
    delete(&cleanup.0).unwrap();
}

#[cfg(not(windows))]
#[test]
fn unsupported_platform_never_falls_back_to_files() {
    let binding = binding("https://example.com");
    assert!(matches!(load(&binding), Err(CredentialError::Unsupported)));
    assert_eq!(
        store(&binding, &Secret::new("test".into()).unwrap()),
        Err(CredentialError::Unsupported)
    );
    assert_eq!(delete(&binding), Err(CredentialError::Unsupported));
}

#[test]
fn embedding_credentials_are_separate_from_generation() {
    let endpoint = "https://example.com/ai/v1";
    let embedding = CredentialBinding::new(
        LOCAL,
        WORKSPACE,
        "openai_embeddings",
        endpoint,
        "@cf/baai/bge-m3",
    )
    .unwrap();
    let generation =
        CredentialBinding::new(LOCAL, WORKSPACE, "openai_chat", endpoint, "@cf/baai/bge-m3")
            .unwrap();
    assert_ne!(embedding.target, generation.target);
    assert_eq!(
        embedding.target,
        CredentialBinding::new(
            LOCAL,
            WORKSPACE,
            "openai_embeddings",
            &format!("{endpoint}/"),
            "@cf/baai/bge-m3"
        )
        .unwrap()
        .target
    );
}

#[test]
fn new_channel_credential_namespace_preserves_the_legacy_target_exactly() {
    let legacy = binding("https://example.com/v1");
    assert_eq!(
        legacy.target,
        "PKMS:model-api:v1:bdfcdeca96ff23aaa372d3bf2d90380a47bdb8857aecb3ca747ea261c9667797"
    );
    let channel = CredentialBinding::new_channel(
        LOCAL,
        WORKSPACE,
        WORKSPACE,
        "openai_chat",
        "https://example.com/v1",
    )
    .unwrap();
    assert!(channel.target.starts_with("PKMS:channel-api:v1:"));
    assert_ne!(legacy.target, channel.target);
    assert!(!channel.target.contains("example.com") && !channel.target.contains(WORKSPACE));
}

#[test]
fn new_channel_credentials_isolate_channel_endpoint_protocol_and_workspace() {
    let endpoint = "https://example.com/v1";
    let baseline =
        CredentialBinding::new_channel(LOCAL, WORKSPACE, LOCAL, "openai_chat", endpoint).unwrap();
    for (local, workspace, channel, protocol, endpoint) in [
        (WORKSPACE, WORKSPACE, LOCAL, "openai_chat", endpoint),
        (LOCAL, LOCAL, LOCAL, "openai_chat", endpoint),
        (LOCAL, WORKSPACE, WORKSPACE, "openai_chat", endpoint),
        (LOCAL, WORKSPACE, LOCAL, "openai_responses", endpoint),
        (LOCAL, WORKSPACE, LOCAL, "gemini", endpoint),
        (
            LOCAL,
            WORKSPACE,
            LOCAL,
            "openai_chat",
            "https://different.test/v1",
        ),
        (
            LOCAL,
            WORKSPACE,
            LOCAL,
            "openai_chat",
            "https://example.com/v2",
        ),
    ] {
        assert_ne!(
            baseline.target,
            CredentialBinding::new_channel(local, workspace, channel, protocol, endpoint)
                .unwrap()
                .target
        );
    }
    assert_eq!(
        baseline.target,
        CredentialBinding::new_channel(
            LOCAL,
            WORKSPACE,
            LOCAL,
            "openai_chat",
            "https://EXAMPLE.COM:443/v1/"
        )
        .unwrap()
        .target
    );
}

#[test]
fn new_channel_credentials_reject_invalid_ids_and_embedding_binding() {
    for channel in ["invalid", "00000000-0000-0000-0000-000000000000"] {
        assert!(
            CredentialBinding::new_channel(
                LOCAL,
                WORKSPACE,
                channel,
                "gemini",
                "https://example.com/v1beta"
            )
            .is_err()
        );
    }
    assert!(
        CredentialBinding::new_channel(
            LOCAL,
            WORKSPACE,
            LOCAL,
            "openai_embeddings",
            "https://example.com/v1"
        )
        .is_err()
    );
    assert!(
        CredentialBinding::new_channel(
            LOCAL,
            WORKSPACE,
            LOCAL,
            "gemini",
            "https://user:secret@example.com/v1beta"
        )
        .is_err()
    );
}
