use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn profile(provider: &str) -> GenerationProfile {
    GenerationProfile {
        channel_id: None,
        capabilities: None,
        chat_token_limit: None,
        provider: provider.into(),
        endpoint: "https://relay.example.test/custom/v1".into(),
        model: "gpt-oss-120b".into(),
        model_digest: String::new(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 131072,
    }
}
fn search() -> SearchQuery {
    SearchQuery {
        query: "evidence".into(),
        mode: SearchMode::Lexical,
        scope: Default::default(),
        limit: 8,
        minimum_cosine: None,
    }
}

#[test]
fn remote_protocol_profiles_freeze_without_fabricated_digest_and_roundtrip_backup() {
    for provider in ["openai_chat", "openai_responses", "anthropic"] {
        let root = tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let command = id();
        let request = SetGenerationProfile {
            base_row_version: 0,
            profile: Some(profile(provider)),
        };
        let configured = db
            .set_generation_profile(&command, request.clone())
            .unwrap();
        let original = configured.settings.profile.unwrap();
        assert!(original.model_digest.is_empty());
        let start = id();
        let accepted = db.accept_answer_from_settings(&start, search()).unwrap();
        let first = accepted.answer;
        assert_eq!(first.request.model.as_ref().unwrap().provider, provider);
        let mut changed = profile(provider);
        changed.model = "another-user-model".into();
        db.set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: 1,
                profile: Some(changed),
            },
        )
        .unwrap();
        assert!(
            db.set_generation_profile(&command, request)
                .unwrap()
                .receipt
                .replayed
        );
        let replayed = db.accept_answer_from_settings(&start, search()).unwrap();
        assert_eq!(replayed.answer.id, first.id);
        assert_eq!(
            replayed.answer.request.model.as_ref().unwrap().model,
            "gpt-oss-120b"
        );
        let lease = db.claim_answer(&first.id).unwrap().unwrap();
        assert_eq!(
            db.prepare_answer_evidence(&first.id, &lease.lease_token, vec![])
                .unwrap()
                .status,
            AnswerStatus::NoEvidence
        );
        let archive = root.path().join("remote.zip");
        db.create_backup(&archive, &mut |_| Ok(())).unwrap();
        let destination = root.path().join("restored");
        prepare_restore(&archive, &destination, &mut |_| Ok(()))
            .unwrap()
            .publish()
            .unwrap();
        let restored = Library::open_existing(destination).unwrap();
        let frozen = restored.answer(&first.id).unwrap().request.model.unwrap();
        assert_eq!(
            serde_json::to_value(&frozen).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(
            restored
                .generation_settings()
                .unwrap()
                .profile
                .unwrap()
                .model,
            "another-user-model"
        );
    }
}

#[test]
fn remote_endpoint_and_model_bounds_fail_before_settings_are_written() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for endpoint in [
        "http://relay.example.test/v1",
        "https://user:password@relay.example.test/v1",
        "https://@relay.example.test/v1",
        "https://:@relay.example.test/v1",
        "https:relay.example.test/v1",
        "https:/relay.example.test/v1",
        "https:////relay.example.test/v1",
        "https://relay.example.test/v1?key=private",
        "https://relay.example.test/v1#secret",
        "https://relay.example.test:0/v1",
        "https://relay.example.test:99999/v1",
        "https://",
        " https://relay.example.test/v1",
        "https://relay.example.test/\npath",
        "https://relay.example.test\\v1",
        "file:///relay",
        "https://relay.example.test/v1?",
    ] {
        let mut invalid = profile("openai_chat");
        invalid.endpoint = endpoint.into();
        assert!(
            db.set_generation_profile(
                &id(),
                SetGenerationProfile {
                    base_row_version: 0,
                    profile: Some(invalid)
                }
            )
            .is_err(),
            "{endpoint:?}"
        );
        assert_eq!(db.generation_settings().unwrap().row_version, 0);
    }
    let mut variants = Vec::new();
    let mut invalid = profile("openai_chat");
    invalid.model_digest = "invented-remote-digest".into();
    variants.push(invalid);
    let mut invalid = profile("openai_chat");
    invalid.model.clear();
    variants.push(invalid);
    let mut invalid = profile("openai_chat");
    invalid.temperature = f64::NAN;
    variants.push(invalid);
    let mut invalid = profile("openai_chat");
    invalid.context_tokens = 2_097_153;
    variants.push(invalid);
    variants.push(profile("unknown"));
    for invalid in variants {
        assert!(
            db.accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: search(),
                    model: Some(invalid)
                }
            )
            .is_err()
        );
    }
    for endpoint in [
        "https://relay.example.test:8443/v1/",
        "https://[::1]/custom",
        "https://relay.example.test",
    ] {
        let mut valid = profile("anthropic");
        valid.endpoint = endpoint.into();
        assert!(
            db.accept_answer(
                &id(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: search(),
                    model: Some(valid)
                }
            )
            .is_ok()
        );
    }
}

#[test]
fn legacy_ollama_json_and_validation_are_unchanged() {
    let json = r#"{"provider":"ollama","endpoint":"http://127.0.0.1:11434","model":"existing","model_digest":"original-digest","config_revision":1,"temperature":0.0,"context_tokens":4096}"#;
    let original: GenerationProfile = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_string(&original).unwrap(), json);
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let command = id();
    let request = StartAnswer {
        max_output_tokens: None,
        citation_count: None,
        agent: None,
        search: search(),
        model: Some(original.clone()),
    };
    let first = db.accept_answer(&command, request.clone()).unwrap();
    assert_eq!(
        db.accept_answer(&command, request).unwrap().answer.id,
        first.answer.id
    );
    let mut empty = original.clone();
    empty.model_digest.clear();
    assert!(
        db.accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: search(),
                model: Some(empty)
            }
        )
        .is_err()
    );
    let mut remote = original;
    remote.endpoint = "https://relay.example.test".into();
    assert!(
        db.accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: search(),
                model: Some(remote)
            }
        )
        .is_err()
    );
}

#[test]
fn chat_budget_field_defaults_without_rewriting_old_json_and_rejects_other_protocols() {
    let legacy = r#"{"provider":"openai_chat","endpoint":"https://relay.example.test/v1","model":"custom","model_digest":"","config_revision":1,"temperature":0.0,"context_tokens":4096}"#;
    let parsed: GenerationProfile = serde_json::from_str(legacy).unwrap();
    assert!(parsed.chat_token_limit.is_none());
    assert_eq!(
        parsed.chat_token_limit.unwrap_or_default(),
        ChatTokenLimit::MaxCompletionTokens
    );
    assert_eq!(serde_json::to_string(&parsed).unwrap(), legacy);
    let mut value: serde_json::Value = serde_json::from_str(legacy).unwrap();
    value["chat_token_limit"] = serde_json::Value::Null;
    let explicit_null: GenerationProfile = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_string(&explicit_null).unwrap(), legacy);
    value["chat_token_limit"] = "max_output_tokens".into();
    assert!(serde_json::from_value::<GenerationProfile>(value).is_err());
    for provider in ["ollama", "openai_responses", "anthropic"] {
        let mut invalid = profile(provider);
        if provider == "ollama" {
            invalid.endpoint = "http://127.0.0.1:11434".into();
            invalid.model_digest = "local-weight".into();
        }
        validate_generation_profile(&invalid).unwrap();
        invalid.chat_token_limit = Some(ChatTokenLimit::MaxTokens);
        assert!(validate_generation_profile(&invalid).is_err());
    }
}

#[test]
fn chat_budget_choice_is_frozen_replayable_and_preserved_in_complete_backup() {
    for choice in [
        ChatTokenLimit::MaxTokens,
        ChatTokenLimit::MaxCompletionTokens,
    ] {
        let root = tempdir().unwrap();
        let mut db = Library::open(root.path()).unwrap();
        let mut selected = profile("openai_chat");
        selected.chat_token_limit = Some(choice);
        let command = id();
        let request = SetGenerationProfile {
            base_row_version: 0,
            profile: Some(selected),
        };
        db.set_generation_profile(&command, request.clone())
            .unwrap();
        let start = id();
        let accepted = db
            .accept_answer_from_settings(&start, search())
            .unwrap()
            .answer;
        assert_eq!(
            accepted.request.model.as_ref().unwrap().chat_token_limit,
            Some(choice)
        );
        // Changing the live setting must not alter accepted request bytes or replay semantics.
        db.set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: 1,
                profile: Some(profile("openai_chat")),
            },
        )
        .unwrap();
        assert!(
            db.set_generation_profile(&command, request)
                .unwrap()
                .receipt
                .replayed
        );
        let replay = db.accept_answer_from_settings(&start, search()).unwrap();
        assert!(replay.receipt.replayed);
        assert_eq!(replay.answer.id, accepted.id);
        assert_eq!(
            replay.answer.request.model.unwrap().chat_token_limit,
            Some(choice)
        );
        let lease = db.claim_answer(&accepted.id).unwrap().unwrap();
        db.prepare_answer_evidence(&accepted.id, &lease.lease_token, vec![])
            .unwrap();
        let package = root.path().join("budget.zip");
        db.create_backup(&package, &mut |_| Ok(())).unwrap();
        let destination = root.path().join("restored");
        prepare_restore(&package, &destination, &mut |_| Ok(()))
            .unwrap()
            .publish()
            .unwrap();
        let restored = Library::open_existing(destination).unwrap();
        let frozen = restored
            .answer(&accepted.id)
            .unwrap()
            .request
            .model
            .unwrap();
        assert_eq!(frozen.chat_token_limit, Some(choice));
        assert!(
            restored
                .generation_settings()
                .unwrap()
                .profile
                .unwrap()
                .chat_token_limit
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(frozen).unwrap()["chat_token_limit"],
            match choice {
                ChatTokenLimit::MaxTokens => "max_tokens",
                ChatTokenLimit::MaxCompletionTokens => "max_completion_tokens",
            }
        );
    }
}

#[test]
fn output_budget_freezes_per_request_replays_and_restores_without_changing_legacy_json() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(profile("openai_chat")),
        },
    )
    .unwrap();
    let legacy = db.accept_answer_from_settings(&id(), search()).unwrap();
    assert!(legacy.answer.request.max_output_tokens.is_none());
    assert_eq!(legacy.answer.request.output_tokens(), 2048);
    let json = serde_json::to_value(&legacy.answer.request).unwrap();
    assert!(json.get("max_output_tokens").is_none());
    let roundtrip: StartAnswer = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), json);
    let mut accepted = Vec::new();
    for agent in [false, true] {
        let command = id();
        let run = if agent {
            db.accept_agent_answer_from_settings_with_budget(
                &command,
                search(),
                AgentOptions::default(),
                Some(8192),
            )
            .unwrap()
        } else {
            db.accept_answer_from_settings_with_budget(&command, search(), Some(8192))
                .unwrap()
        };
        assert_eq!(run.answer.request.output_tokens(), 8192);
        let changed = if agent {
            db.accept_agent_answer_from_settings_with_budget(
                &command,
                search(),
                AgentOptions::default(),
                Some(16384),
            )
        } else {
            db.accept_answer_from_settings_with_budget(&command, search(), Some(16384))
        };
        assert!(
            changed.is_err(),
            "same command must not change its output budget"
        );
        accepted.push((agent, command, run.answer.id));
    }
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 1,
            profile: None,
        },
    )
    .unwrap();
    for (agent, command, answer_id) in &accepted {
        let replay = if *agent {
            db.accept_agent_answer_from_settings_with_budget(
                command,
                search(),
                AgentOptions::default(),
                Some(8192),
            )
            .unwrap()
        } else {
            db.accept_answer_from_settings_with_budget(command, search(), Some(8192))
                .unwrap()
        };
        assert!(replay.receipt.replayed);
        assert_eq!(&replay.answer.id, answer_id);
        assert_eq!(replay.answer.request.output_tokens(), 8192);
        assert!(replay.answer.request.model.is_some());
    }
    let package = root.path().join("budget.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    for (_, _, answer_id) in accepted {
        assert_eq!(
            restored
                .answer(&answer_id)
                .unwrap()
                .request
                .max_output_tokens,
            Some(8192)
        );
    }
}

#[test]
fn output_budget_rejects_invalid_values_before_creating_work() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut model = profile("anthropic");
    model.context_tokens = 8192;
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(model),
        },
    )
    .unwrap();
    for tokens in [0, 8192, 32769, u32::MAX] {
        assert!(
            db.accept_answer_from_settings_with_budget(&id(), search(), Some(tokens))
                .is_err()
        );
        assert!(
            db.accept_agent_answer_from_settings_with_budget(
                &id(),
                search(),
                AgentOptions::default(),
                Some(tokens)
            )
            .is_err()
        );
    }
    assert!(db.answers_page(None, 30).unwrap().is_empty());
    assert!(db.pending_answer_ids(30).unwrap().is_empty());
    let accepted = db
        .accept_answer_from_settings_with_budget(&id(), search(), Some(4096))
        .unwrap();
    assert_eq!(accepted.answer.request.output_tokens(), 4096);
}
