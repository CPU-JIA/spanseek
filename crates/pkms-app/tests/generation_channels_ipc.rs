use pkms_app::*;
use pkms_core::{GenerationChannelMutation, GenerationSettingsMutation};
use serde_json::{Value, json};
use std::path::Path;
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

/// Remote embeddings and non-local channels are refused until the workspace has
/// acknowledged, once, that enabled text may leave this device.
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
    let reply = app
        .dispatch(request(
            identity,
            json!({"kind": "retrieval_settings_set", "payload": {
                "command_id": id(),
                "request": {"base_row_version": current.row_version, "params": params},
            }}),
        ))
        .unwrap();
    let DesktopData::RetrievalSettingsMutation(mutation) = reply.data else {
        panic!("retrieval settings mutation reply")
    };
    assert!(mutation.settings.params.remote_consent);
}
fn request(identity: &WorkspaceIdentity, command: Value) -> DesktopRequest {
    DesktopRequest::parse(json!({
        "api_version": 1, "request_id": id(), "identity": identity, "command": command,
    }))
    .unwrap()
}

fn spec(provider: &str) -> Value {
    json!({
        "name": format!("Synthetic {provider}"), "provider": provider,
        "endpoint": "https://example.invalid/v1", "preset": null,
        "capabilities": {"text_transport": "stream", "structured_transport": "json"},
        "models": [{"model": "offline-contract-model", "context_tokens": 65536,
                    "temperature": 0.0, "chat_token_limit": null}],
    })
}

fn save(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    channel_id: &str,
    channel_version: u32,
    settings_version: u32,
    spec: Value,
) -> GenerationChannelMutation {
    let command = request(
        identity,
        json!({"kind": "generation_channel_save", "payload": {
            "command_id": id(), "request": {"channel_id": channel_id,
            "base_row_version": channel_version, "base_generation_row_version": settings_version, "spec": spec},
        }}),
    );
    let first = app.dispatch(command.clone()).unwrap();
    let second = app.dispatch(command).unwrap();
    assert_eq!(
        serde_json::to_value(&first.data).unwrap()["kind"],
        "generation_channel_mutation"
    );
    let DesktopData::GenerationChannelMutation(first) = first.data else {
        panic!("channel mutation reply")
    };
    let DesktopData::GenerationChannelMutation(second) = second.data else {
        panic!("channel replay reply")
    };
    assert!(!first.receipt.replayed);
    assert!(second.receipt.replayed);
    assert_eq!(first.channel, second.channel);
    first
}

fn select(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    channel: &GenerationChannelMutation,
) -> GenerationSettingsMutation {
    let command = request(
        identity,
        json!({"kind": "generation_channel_select", "payload": {
            "command_id": id(), "request": {"channel_id": channel.channel.id,
            "model": "offline-contract-model", "base_row_version": channel.channel.row_version,
            "base_generation_row_version": channel.settings.row_version},
        }}),
    );
    let reply = app.dispatch(command).unwrap();
    assert_eq!(
        serde_json::to_value(&reply.data).unwrap()["kind"],
        "generation_settings_mutation"
    );
    let DesktopData::GenerationSettingsMutation(selected) = reply.data else {
        panic!("settings mutation reply")
    };
    selected
}

#[test]
fn new_channel_ipc_wire_preserves_all_four_protocols_and_replays_without_duplicate_channels() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    allow_remote_data(&mut app, &identity);
    let mut settings_version = 0;
    for provider in ["openai_chat", "openai_responses", "anthropic", "gemini"] {
        let channel_id = id();
        let saved = save(
            &mut app,
            &identity,
            &channel_id,
            0,
            settings_version,
            spec(provider),
        );
        assert_eq!(saved.channel.row_version, 1);
        let selected = select(&mut app, &identity, &saved);
        settings_version = selected.settings.row_version;
        let profile = selected.settings.profile.unwrap();
        assert_eq!(profile.provider, provider);
        assert_eq!(profile.channel_id.as_deref(), Some(channel_id.as_str()));
        assert_eq!(profile.capabilities, Some(saved.channel.spec.capabilities));
    }
    let reply = app
        .dispatch(request(&identity, json!({"kind": "generation_channels"})))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&reply.data).unwrap()["kind"],
        "generation_channels"
    );
    let DesktopData::GenerationChannels(channels) = reply.data else {
        panic!("channel list")
    };
    assert_eq!(channels.channels.len(), 4);
    assert_eq!(channels.settings.row_version, settings_version);
}

#[test]
fn new_channel_ipc_rejects_stale_channel_settings_and_deleted_or_changed_workspace_snapshots() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    allow_remote_data(&mut app, &identity);
    let channel_id = id();
    let saved = save(&mut app, &identity, &channel_id, 0, 0, spec("openai_chat"));
    let selected = select(&mut app, &identity, &saved);
    let settings_version = selected.settings.row_version;
    let mut changed_spec = spec("openai_chat");
    changed_spec["name"] = json!("Renamed synthetic channel");
    let updated = save(
        &mut app,
        &identity,
        &channel_id,
        1,
        settings_version,
        changed_spec,
    );
    assert!(matches!(
        app.generation_channel_at_version(&identity, &channel_id, 1),
        Err(AppError::Core(pkms_core::Error::MetadataConflict {
            expected: 1,
            actual: 2,
            ..
        }))
    ));
    for (channel_version, generation_version, conflict_id) in [
        (1, updated.settings.row_version, channel_id.as_str()),
        (2, settings_version, "generation_settings"),
    ] {
        let err = app
            .dispatch(request(
                &identity,
                json!({"kind": "generation_channel_select", "payload": {
                    "command_id": id(), "request": {"channel_id": channel_id,
                    "model": "offline-contract-model", "base_row_version": channel_version,
                    "base_generation_row_version": generation_version},
                }}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ApiErrorCode::MetadataConflict);
        assert!(
            matches!(err.details, Some(ApiErrorDetails::MetadataConflict { object_id, .. }) if object_id == conflict_id)
        );
    }
    let deleted = app
        .dispatch(request(
            &identity,
            json!({"kind": "generation_channel_delete", "payload": {
                "command_id": id(), "request": {"channel_id": channel_id, "base_row_version": 2,
                "base_generation_row_version": updated.settings.row_version, "clear_active": true},
            }}),
        ))
        .unwrap();
    let DesktopData::GenerationChannelMutation(deleted) = deleted.data else {
        panic!("deleted mutation")
    };
    assert!(deleted.channel.deleted);
    assert!(deleted.settings.profile.is_none());
    assert!(matches!(
        app.generation_channel_at_version(&identity, &channel_id, deleted.channel.row_version),
        Err(AppError::InvalidInput("generation_channel_deleted"))
    ));
    workspace(&mut app, &root.path().join("other"));
    assert!(matches!(
        app.generation_channel_at_version(&identity, &channel_id, deleted.channel.row_version),
        Err(AppError::InstanceMismatch)
    ));
}

#[test]
fn new_channel_ipc_credentials_reject_stale_versions_before_touching_the_vault() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    allow_remote_data(&mut app, &identity);
    let channel_id = id();
    save(&mut app, &identity, &channel_id, 0, 0, spec("gemini"));
    for kind in [
        "generation_channel_credential_set",
        "generation_channel_credential_status",
        "generation_channel_credential_delete",
    ] {
        let mut payload = json!({"channel_id": channel_id, "row_version": 0});
        if kind.ends_with("_set") {
            payload["api_key"] = json!("synthetic-secret-must-never-reach-vault");
        }
        let error = app
            .dispatch(request(
                &identity,
                json!({"kind": kind, "payload": payload}),
            ))
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::MetadataConflict);
        assert!(!format!("{error:?}").contains("synthetic-secret"));
    }
}

#[test]
fn new_channel_ipc_sensitive_debug_and_parse_errors_are_redacted_and_capability_reason_is_preserved()
 {
    let identity = WorkspaceIdentity {
        local_instance_id: id(),
        workspace_id: id(),
    };
    let secret = "synthetic-sensitive-input-not-a-real-credential";
    let command = json!({"kind": "generation_channel_credential_set", "payload": {
        "channel_id": id(), "row_version": 1, "api_key": secret,
    }});
    let parsed = request(&identity, command.clone());
    let debug = format!("{parsed:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(secret));
    let mut malformed =
        json!({"api_version": 1, "request_id": id(), "identity": identity, "command": command});
    malformed["command"]["payload"]["row_version"] = json!(secret);
    let error = DesktopRequest::parse(malformed).unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidInput);
    assert!(!format!("{error:?}").contains(secret));
    let error = ApiError::from_app(
        pkms_core::Error::InvalidInput("model_unsupported_output").into(),
        &id(),
    );
    assert_eq!(error.code, ApiErrorCode::InvalidInput);
    assert!(
        matches!(error.details, Some(ApiErrorDetails::InferenceFailure { reason }) if reason == "model_unsupported_output")
    );
}

#[test]
fn new_channel_ipc_legacy_credential_commands_cannot_bypass_channel_version_checks() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    allow_remote_data(&mut app, &identity);
    let saved = save(&mut app, &identity, &id(), 0, 0, spec("openai_chat"));
    let selected = select(&mut app, &identity, &saved);
    let profile = selected.settings.profile.unwrap();
    for kind in [
        "generation_credential_set",
        "generation_credential_status",
        "generation_credential_delete",
    ] {
        let mut payload = json!({"profile": profile});
        if kind.ends_with("_set") {
            payload["api_key"] = json!("synthetic-key-must-never-reach-vault");
        }
        let error = app
            .dispatch(request(
                &identity,
                json!({"kind": kind, "payload": payload}),
            ))
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::InvalidInput);
        assert!(!format!("{error:?}").contains("synthetic-key"));
    }
}
#[test]
fn new_channel_ipc_requires_acknowledged_remote_data_before_remote_configuration() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let reason = |error: ApiError| {
        assert_eq!(error.code, ApiErrorCode::InvalidInput);
        match error.details {
            Some(ApiErrorDetails::InferenceFailure { reason }) => reason,
            other => panic!("unexpected details: {other:?}"),
        }
    };
    let channel_id = id();
    let refused = app
        .dispatch(request(
            &identity,
            json!({"kind": "generation_channel_save", "payload": {
                "command_id": id(), "request": {"channel_id": channel_id,
                "base_row_version": 0, "base_generation_row_version": 0,
                "spec": spec("openai_chat")},
            }}),
        ))
        .unwrap_err();
    assert_eq!(reason(refused), "remote_consent_required");
    let embedding = app
        .dispatch(request(
            &identity,
            json!({"kind": "embedding_remote_configure", "payload": {
                "command_id": id(), "provider": "cloudflare",
                "endpoint": "https://example.invalid/v1", "model": "@cf/baai/bge-m3",
                "output_dimensions": null,
            }}),
        ))
        .unwrap_err();
    assert_eq!(reason(embedding), "remote_consent_required");
    // The legacy single-model profile is the other way to reach a remote provider.
    let legacy = |provider: &str, endpoint: &str, digest: &str| {
        json!({"kind": "generation_settings_set", "payload": {
            "command_id": id(), "request": {"base_row_version": 0, "profile": {
                "provider": provider, "endpoint": endpoint, "model": "offline-contract-model",
                "model_digest": digest, "config_revision": 1, "temperature": 0.0,
                "context_tokens": 4096, "chat_token_limit": null,
                "channel_id": null, "capabilities": null},
            }},
        })
    };
    let refused = app
        .dispatch(request(
            &identity,
            legacy("openai_chat", "https://example.invalid/v1", ""),
        ))
        .unwrap_err();
    assert_eq!(reason(refused), "remote_consent_required");
    // A loopback Ollama profile never sends text away, so it needs no acknowledgement.
    let reply = app
        .dispatch(request(
            &identity,
            legacy("ollama", "http://127.0.0.1:11434", "synthetic-local-digest"),
        ))
        .unwrap();
    let DesktopData::GenerationSettingsMutation(local) = reply.data else {
        panic!("settings mutation reply")
    };
    assert_eq!(local.settings.profile.unwrap().provider, "ollama");
    let settings_version = local.settings.row_version;
    allow_remote_data(&mut app, &identity);
    let saved = save(
        &mut app,
        &identity,
        &channel_id,
        0,
        settings_version,
        spec("openai_chat"),
    );
    assert_eq!(saved.channel.row_version, 1);
}
