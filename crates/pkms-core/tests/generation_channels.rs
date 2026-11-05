use pkms_core::*;
mod common;

use rusqlite::{Connection, params, types::Value};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
use tempfile::tempdir;
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn model(name: &str) -> GenerationChannelModel {
    GenerationChannelModel {
        model: name.into(),
        context_tokens: 32768,
        temperature: 0.2,
        chat_token_limit: None,
    }
}

fn spec() -> GenerationChannelSpec {
    GenerationChannelSpec {
        name: "Custom relay".into(),
        provider: "openai_chat".into(),
        endpoint: "https://relay.example.test/v1".into(),
        models: vec![model("model-a"), model("model-b")],
        capabilities: GenerationCapabilities {
            text_transport: GenerationTransport::Stream,
            structured_transport: Some(GenerationTransport::Json),
        },
        preset: None,
    }
}

fn save_request(channel: Option<&GenerationChannel>, settings: u32) -> SaveGenerationChannel {
    SaveGenerationChannel {
        channel_id: channel.map_or_else(id, |channel| channel.id.clone()),
        base_row_version: channel.map_or(0, |channel| channel.row_version),
        base_generation_row_version: settings,
        spec: channel.map_or_else(spec, |channel| channel.spec.clone()),
    }
}

fn select(
    library: &mut Library,
    channel: &GenerationChannel,
    settings: u32,
    model: &str,
) -> GenerationSettingsMutation {
    library
        .select_generation_channel_model(
            &id(),
            SelectGenerationChannelModel {
                channel_id: channel.id.clone(),
                model: model.into(),
                base_row_version: channel.row_version,
                base_generation_row_version: settings,
            },
        )
        .unwrap()
}

fn search() -> SearchQuery {
    SearchQuery {
        query: "synthetic evidence".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    }
}

fn chat(library: &mut Library) -> ChatRun {
    let conversation = library
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "Synthetic channel snapshot".into(),
            },
        )
        .unwrap()
        .conversation;
    library
        .accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: conversation.id,
                base_row_version: conversation.row_version,
                text: "Synthetic input, never sent to a model".into(),
                purpose: ChatPurpose::Review,
                materials: vec![],
                scope: SearchScope::default(),
                use_knowledge_tools: false,
                options: None,
            },
        )
        .unwrap()
        .run
}

fn rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut query = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = query.column_count();
    query
        .query_map([], |row| {
            (0..columns).map(|column| row.get(column)).collect()
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
/// Schema 38 adds an indexed type key to notes. Historical package tests compare
/// the legacy row shape, so read only the columns that existed at K15.
fn comparison_rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    if table == "notes" {
        let mut query = db
            .prepare("SELECT id,title,current_revision,anchor_id,trashed,updated_at,current_revision_id FROM notes ORDER BY rowid")
            .unwrap();
        let columns = query.column_count();
        return query
            .query_map([], |row| {
                (0..columns).map(|column| row.get(column)).collect()
            })
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
    }
    rows(db, table)
}

// K18 adds retrieval-parameter columns to search_settings by design, so row-shape
// comparisons read its legacy columns explicitly.
fn legacy_settings(db: &Connection) -> Vec<Vec<Value>> {
    let mut query = db
        .prepare("SELECT singleton,active_profile_id,generation_profile_json,generation_row_version FROM search_settings ORDER BY rowid")
        .unwrap();
    query
        .query_map([], |row| (0..4).map(|column| row.get(column)).collect())
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

#[test]
fn empty_channel_can_be_saved_before_credentials_and_explicit_model_selection() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let mut create = save_request(None, 0);
    create.spec.models.clear();
    let empty = library.save_generation_channel(&id(), create).unwrap();
    assert_eq!(empty.channel.row_version, 1);
    assert_eq!(empty.settings.row_version, 0);
    assert!(empty.settings.profile.is_none());
    assert_eq!(library.generation_channels().unwrap().channels.len(), 1);
    assert!(
        library
            .select_generation_channel_model(
                &id(),
                SelectGenerationChannelModel {
                    channel_id: empty.channel.id.clone(),
                    model: "missing".into(),
                    base_row_version: 1,
                    base_generation_row_version: 0,
                },
            )
            .is_err()
    );
    let mut add = save_request(Some(&empty.channel), 0);
    add.spec.models.push(model("discovered-model"));
    let added = library.save_generation_channel(&id(), add).unwrap();
    let selected = select(&mut library, &added.channel, 0, "discovered-model");
    let profile = selected.settings.profile.as_ref().unwrap();
    assert_eq!(
        profile.channel_id.as_deref(),
        Some(added.channel.id.as_str())
    );
    assert_eq!(profile.config_revision, selected.settings.row_version);
    assert_eq!(profile.capabilities, Some(added.channel.spec.capabilities));
    assert!(profile.model_digest.is_empty());
    assert!(matches!(
        library.set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: selected.settings.row_version,
                profile: Some(profile.clone()),
            },
        ),
        Err(Error::InvalidInput("generation_channel_selection_required"))
    ));
    assert_eq!(library.generation_settings().unwrap().row_version, 1);
}

#[test]
fn channel_and_settings_versions_conflict_without_partial_writes_and_replay_is_exact() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let command = id();
    let request = save_request(None, 0);
    let first = library
        .save_generation_channel(&command, request.clone())
        .unwrap();
    let other = library
        .save_generation_channel(&id(), save_request(None, 0))
        .unwrap();
    select(&mut library, &first.channel, 0, "model-a");
    let mut edit_other = save_request(Some(&other.channel), 1);
    edit_other.spec.name = "Other channel revision two".into();
    library.save_generation_channel(&id(), edit_other).unwrap();
    let db = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let tables = [
        "generation_channels",
        "search_settings",
        "command_receipts",
        "workspace_meta",
    ];
    let before: Vec<_> = tables
        .iter()
        .map(|table| comparison_rows(&db, table))
        .collect();
    let mut wrong_payload = request.clone();
    wrong_payload.spec.name = "Changed payload".into();
    assert!(matches!(
        library.save_generation_channel(&command, wrong_payload),
        Err(Error::IdempotencyMismatch)
    ));
    let mut stale_settings = save_request(Some(&other.channel), 0);
    stale_settings.spec.name = "Must not persist".into();
    assert!(matches!(
        library.save_generation_channel(&id(), stale_settings),
        Err(Error::MetadataConflict { object_id, .. }) if object_id == "generation_settings"
    ));
    let stale_channel = save_request(Some(&other.channel), 1);
    assert!(matches!(
        library.save_generation_channel(&id(), stale_channel),
        Err(Error::MetadataConflict { object_id, .. }) if object_id == other.channel.id
    ));
    let replay = library.save_generation_channel(&command, request).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.channel, first.channel);
    assert_eq!(replay.settings.row_version, 0);
    assert_eq!(replay.receipt.change_seq, first.receipt.change_seq);
    assert_eq!(
        tables
            .iter()
            .map(|table| comparison_rows(&db, table))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn editing_active_defaults_is_atomic_and_preserves_answer_and_chat_snapshots() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let saved = library
        .save_generation_channel(&id(), save_request(None, 0))
        .unwrap();
    let selected = select(&mut library, &saved.channel, 0, "model-a");
    let answer = library
        .accept_answer_from_settings(&id(), search())
        .unwrap()
        .answer;
    let run = chat(&mut library);
    let old_profile = serde_json::to_value(selected.settings.profile).unwrap();
    let mut update = save_request(Some(&saved.channel), 1);
    update.spec.endpoint = "https://other.example.test/v1beta".into();
    update.spec.provider = "gemini".into();
    update.spec.capabilities.text_transport = GenerationTransport::Json;
    update.spec.capabilities.structured_transport = None;
    update.spec.models[0].context_tokens = 2_097_152;
    update.spec.models[0].temperature = 0.7;
    let changed = library.save_generation_channel(&id(), update).unwrap();
    assert_eq!(changed.settings.row_version, 2);
    let active = changed.settings.profile.as_ref().unwrap();
    assert_eq!(active.provider, "gemini");
    assert_eq!(active.context_tokens, 2_097_152);
    assert_eq!(active.temperature, 0.7);
    assert_eq!(
        active.capabilities.as_ref().unwrap().structured_transport,
        None
    );
    assert_eq!(
        serde_json::to_value(library.answer(&answer.id).unwrap().request.model).unwrap(),
        old_profile
    );
    assert_eq!(
        serde_json::to_value(library.chat_run(&run.id).unwrap().model).unwrap(),
        old_profile
    );
    let mut remove_current = save_request(Some(&changed.channel), 2);
    remove_current
        .spec
        .models
        .retain(|model| model.model != "model-a");
    assert!(matches!(
        library.save_generation_channel(&id(), remove_current),
        Err(Error::InvalidInput("generation_channel_model_missing"))
    ));
    assert_eq!(
        library.generation_channel(&saved.channel.id).unwrap(),
        changed.channel
    );
    assert_eq!(library.generation_settings().unwrap().row_version, 2);
}

#[test]
fn channel_storage_failure_rolls_back_active_profile_and_receipt_before_retry() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let saved = library
        .save_generation_channel(&id(), save_request(None, 0))
        .unwrap();
    select(&mut library, &saved.channel, 0, "model-a");
    let db = Connection::open(root.path().join("library.sqlite3")).unwrap();
    // This trigger exists only in a disposable test database. It fails the
    // channel write after the active profile UPDATE has already executed.
    db.execute_batch("CREATE TRIGGER controlled_channel_failure BEFORE UPDATE ON generation_channels BEGIN SELECT RAISE(ABORT,'controlled channel storage failure'); END;").unwrap();
    let tables = [
        "generation_channels",
        "search_settings",
        "command_receipts",
        "workspace_meta",
    ];
    let before: Vec<_> = tables
        .iter()
        .map(|table| comparison_rows(&db, table))
        .collect();
    let command = id();
    let mut update = save_request(Some(&saved.channel), 1);
    update.spec.models[0].temperature = 0.9;
    assert!(matches!(
        library.save_generation_channel(&command, update.clone()),
        Err(Error::Database(_))
    ));
    assert_eq!(
        tables
            .iter()
            .map(|table| comparison_rows(&db, table))
            .collect::<Vec<_>>(),
        before
    );
    db.execute_batch("DROP TRIGGER controlled_channel_failure;")
        .unwrap();
    let applied = library
        .save_generation_channel(&command, update.clone())
        .unwrap();
    assert_eq!(applied.channel.row_version, 2);
    assert_eq!(applied.settings.row_version, 2);
    assert_eq!(applied.settings.profile.unwrap().temperature, 0.9);
    assert!(
        library
            .save_generation_channel(&command, update)
            .unwrap()
            .receipt
            .replayed
    );
}

#[test]
fn structured_channels_reject_answers_and_agents_before_queue_but_accept_plain_chat() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let mut create = save_request(None, 0);
    create.spec.capabilities.structured_transport = None;
    let saved = library.save_generation_channel(&id(), create).unwrap();
    let selected = select(&mut library, &saved.channel, 0, "model-a");
    let db = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let tables = [
        "answers",
        "answer_events",
        "command_receipts",
        "workspace_meta",
    ];
    let before: Vec<_> = tables
        .iter()
        .map(|table| comparison_rows(&db, table))
        .collect();
    let rejected_command = id();
    let mut results = vec![
        library.accept_answer_from_settings(&rejected_command, search()),
        library.accept_answer_from_settings_with_budget(&id(), search(), Some(2048)),
        library.accept_agent_answer_from_settings(&id(), search(), AgentOptions::default()),
        library.accept_agent_answer_from_settings_with_budget(
            &id(),
            search(),
            AgentOptions::default(),
            Some(2048),
        ),
    ];
    for agent in [None, Some(AgentOptions::default())] {
        results.push(library.accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                search: search(),
                model: selected.settings.profile.clone(),
                agent,
            },
        ));
    }
    for result in results {
        assert!(matches!(
            result,
            Err(Error::InvalidInput("model_unsupported_output"))
        ));
    }
    assert_eq!(
        tables
            .iter()
            .map(|table| comparison_rows(&db, table))
            .collect::<Vec<_>>(),
        before
    );
    let plain = chat(&mut library);
    assert_eq!(plain.status, ChatRunStatus::Queued);
    assert_eq!(
        plain
            .model
            .unwrap()
            .capabilities
            .unwrap()
            .structured_transport,
        None
    );
    let mut enable = save_request(Some(&saved.channel), 1);
    enable.spec.capabilities.structured_transport = Some(GenerationTransport::Json);
    library.save_generation_channel(&id(), enable).unwrap();
    let accepted = library
        .accept_answer_from_settings(&rejected_command, search())
        .unwrap();
    assert!(
        !accepted.receipt.replayed,
        "rejected attempts must not record success receipts"
    );
    assert_eq!(accepted.answer.status, AnswerStatus::Queued);
}

#[test]
fn structured_guard_preserves_legacy_profiles_and_capable_answer_replay_after_channel_edit() {
    let legacy: GenerationProfile = serde_json::from_str(LEGACY_PROFILE).unwrap();
    validate_structured_generation_profile(&legacy).unwrap();
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let saved = library
        .save_generation_channel(&id(), save_request(None, 0))
        .unwrap();
    let selected = select(&mut library, &saved.channel, 0, "model-a");
    let mut supported = selected.settings.profile.clone().unwrap();
    for transport in [GenerationTransport::Json, GenerationTransport::Stream] {
        supported
            .capabilities
            .as_mut()
            .unwrap()
            .structured_transport = Some(transport);
        validate_structured_generation_profile(&supported).unwrap();
    }
    let answer_command = id();
    let answer = library
        .accept_answer_from_settings(&answer_command, search())
        .unwrap();
    let agent_command = id();
    let agent = library
        .accept_agent_answer_from_settings(&agent_command, search(), AgentOptions::default())
        .unwrap();
    let mut disable = save_request(Some(&saved.channel), 1);
    disable.spec.capabilities.structured_transport = None;
    library.save_generation_channel(&id(), disable).unwrap();
    let replay = library
        .accept_answer_from_settings(&answer_command, search())
        .unwrap();
    let agent_replay = library
        .accept_agent_answer_from_settings(&agent_command, search(), AgentOptions::default())
        .unwrap();
    assert!(replay.receipt.replayed && agent_replay.receipt.replayed);
    assert_eq!(replay.answer.id, answer.answer.id);
    assert_eq!(agent_replay.answer.id, agent.answer.id);
    assert_eq!(
        serde_json::to_value(replay.answer.request).unwrap(),
        serde_json::to_value(answer.answer.request).unwrap()
    );
    assert_eq!(
        serde_json::to_value(agent_replay.answer.request).unwrap(),
        serde_json::to_value(agent.answer.request).unwrap()
    );
}

#[test]
fn deleting_active_channel_requires_clear_keeps_tombstone_and_never_reuses_its_id() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let create = save_request(None, 0);
    let saved = library
        .save_generation_channel(&id(), create.clone())
        .unwrap();
    let selected = select(&mut library, &saved.channel, 0, "model-a");
    let mut delete = DeleteGenerationChannel {
        channel_id: saved.channel.id.clone(),
        base_row_version: saved.channel.row_version,
        base_generation_row_version: selected.settings.row_version,
        clear_active: false,
    };
    assert!(matches!(
        library.delete_generation_channel(&id(), delete.clone()),
        Err(Error::InvalidInput("generation_channel_is_active"))
    ));
    delete.clear_active = true;
    let command = id();
    let removed = library
        .delete_generation_channel(&command, delete.clone())
        .unwrap();
    assert!(removed.channel.deleted);
    assert_eq!(removed.channel.row_version, 2);
    assert!(removed.settings.profile.is_none());
    assert_eq!(removed.settings.row_version, 2);
    assert!(library.generation_channels().unwrap().channels.is_empty());
    assert_eq!(
        library.generation_channel(&saved.channel.id).unwrap(),
        removed.channel
    );
    let mut recreate = create;
    recreate.base_generation_row_version = 2;
    assert!(library.save_generation_channel(&id(), recreate).is_err());
    let replay = library.delete_generation_channel(&command, delete).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.channel, removed.channel);
    assert_eq!(library.generation_settings().unwrap().row_version, 2);
}

#[test]
fn channel_validation_enforces_protocol_capabilities_and_explicit_bounds() {
    for provider in ["openai_chat", "openai_responses", "anthropic", "gemini"] {
        let mut valid = spec();
        valid.provider = provider.into();
        valid.models[0].context_tokens = 2_097_152;
        validate_generation_channel_spec(&valid).unwrap();
        valid.models.clear();
        validate_generation_channel_spec(&valid).unwrap();
    }
    let mut invalids = vec![];
    for endpoint in [
        "http://relay.test",
        "https://@relay.test",
        "https:relay.test",
        "https://relay.test?key=secret",
        "https://relay.test/#token",
        "https://relay.test:0",
        "https://relay.test\\path",
    ] {
        let mut invalid = spec();
        invalid.endpoint = endpoint.into();
        invalids.push(invalid);
    }
    for name in ["", " trailing ", "control\n"] {
        let mut invalid = spec();
        invalid.name = name.into();
        invalids.push(invalid);
    }
    for context in [255, 2_097_153] {
        let mut invalid = spec();
        invalid.models[0].context_tokens = context;
        invalids.push(invalid);
    }
    let mut invalid = spec();
    invalid.models.push(invalid.models[0].clone());
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.models[0].model = " padded ".into();
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.models[0].temperature = f64::NAN;
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.models[0].temperature = 2.01;
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.provider = "ollama".into();
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.models = (0..129).map(|n| model(&format!("model-{n}"))).collect();
    invalids.push(invalid);
    let mut invalid = spec();
    invalid.provider = "gemini".into();
    invalid.models[0].chat_token_limit = Some(ChatTokenLimit::MaxTokens);
    invalids.push(invalid);
    for invalid in invalids {
        assert!(
            validate_generation_channel_spec(&invalid).is_err(),
            "{invalid:?}"
        );
    }
    let mut cf = spec();
    cf.preset = Some(GenerationChannelPreset::CloudflareWorkersAi);
    validate_generation_channel_spec(&cf).unwrap();
    cf.capabilities.structured_transport = Some(GenerationTransport::Stream);
    assert!(validate_generation_channel_spec(&cf).is_err());
    cf.capabilities.structured_transport = Some(GenerationTransport::Json);
    cf.provider = "openai_responses".into();
    assert!(validate_generation_channel_spec(&cf).is_err());
    cf.capabilities.text_transport = GenerationTransport::Json;
    validate_generation_channel_spec(&cf).unwrap();
    cf.provider = "anthropic".into();
    assert!(validate_generation_channel_spec(&cf).is_err());
    let mut unknown = serde_json::to_value(spec()).unwrap();
    unknown["api_key"] = "must-never-be-stored".into();
    assert!(serde_json::from_value::<GenerationChannelSpec>(unknown).is_err());
}

fn unpack(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|index| {
            let mut file = zip.by_index(index).unwrap();
            let mut bytes = vec![];
            file.read_to_end(&mut bytes).unwrap();
            (file.name().to_string(), bytes)
        })
        .collect()
}

fn rewrite_package(
    source: &Path,
    destination: &Path,
    schema: u32,
    mutate: impl FnOnce(&Connection),
) {
    let mut files = unpack(source);
    let root = tempdir().unwrap();
    let database = root.path().join("library.sqlite3");
    fs::write(&database, &files["library.sqlite3"]).unwrap();
    {
        let db = Connection::open(&database).unwrap();
        mutate(&db);
        // Backups may retain WAL journaling. Checkpoint the fixture mutation so
        // the copied main file carries the intended user_version and schema.
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    }

    let bytes = fs::read(&database).unwrap();
    let mut manifest: BackupManifest = serde_json::from_slice(&files["manifest.json"]).unwrap();
    manifest.database.sha256 = format!("{:x}", Sha256::digest(&bytes));
    manifest.database.size = bytes.len() as u64;
    manifest.database_schema = schema;
    files.insert("library.sqlite3".into(), bytes);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    files.insert(
        "COMPLETE".into(),
        format!("{:x}", Sha256::digest(&bytes)).into_bytes(),
    );
    files.insert("manifest.json".into(), bytes);
    let mut zip = ZipWriter::new(File::create(destination).unwrap());
    for (name, bytes) in files {
        // Production packages are Stored; the verifier rejects anything else, and
        // the zip default follows the enabled features of the whole workspace build.
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn complete_package_preserves_deleted_channel_history_and_validates_active_profile_and_schema() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let first = library
        .save_generation_channel(&id(), save_request(None, 0))
        .unwrap();
    select(&mut library, &first.channel, 0, "model-a");
    let answer = library
        .accept_answer_from_settings(&id(), search())
        .unwrap()
        .answer;
    let run = chat(&mut library);
    library
        .delete_generation_channel(
            &id(),
            DeleteGenerationChannel {
                channel_id: first.channel.id.clone(),
                base_row_version: 1,
                base_generation_row_version: 1,
                clear_active: true,
            },
        )
        .unwrap();
    let mut create = save_request(None, 2);
    create.spec.provider = "gemini".into();
    create.spec.models[0].context_tokens = 2_097_152;
    let current = library.save_generation_channel(&id(), create).unwrap();
    let selected = select(&mut library, &current.channel, 2, "model-a");
    let archive = root.path().join("channels.pkmsbackup");
    let report = library.create_backup(&archive, &mut |_| Ok(())).unwrap();
    assert_eq!(
        report.manifest.database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    let restored_path = root.path().join("restored");
    prepare_restore(&archive, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    assert_eq!(
        restored.generation_channels().unwrap().channels,
        vec![current.channel]
    );
    assert!(
        restored
            .generation_channel(&first.channel.id)
            .unwrap()
            .deleted
    );
    assert_eq!(
        serde_json::to_value(restored.generation_settings().unwrap()).unwrap(),
        serde_json::to_value(selected.settings).unwrap()
    );
    assert_eq!(
        serde_json::to_value(restored.answer(&answer.id).unwrap().request.model).unwrap(),
        serde_json::to_value(answer.request.model).unwrap()
    );
    assert_eq!(
        serde_json::to_value(restored.chat_run(&run.id).unwrap().model).unwrap(),
        serde_json::to_value(run.model).unwrap()
    );
    for (case, sql) in [
        (
            "active",
            "UPDATE search_settings SET generation_profile_json=json_set(generation_profile_json,'$.temperature',1.5)",
        ),
        (
            "future-profile",
            "UPDATE search_settings SET generation_profile_json=json_set(generation_profile_json,'$.future_behavior',true)",
        ),
        (
            "future-spec",
            "UPDATE generation_channels SET spec_json=json_set(spec_json,'$.future_behavior',true)",
        ),
        (
            "missing-history-channel",
            "DELETE FROM generation_channels WHERE deleted=1",
        ),
        (
            "future-ddl",
            "ALTER TABLE generation_channels ADD COLUMN future_behavior TEXT",
        ),
    ] {
        let broken = root.path().join(format!("{case}.pkmsbackup"));
        rewrite_package(&archive, &broken, 16, |db| db.execute_batch(sql).unwrap());
        assert!(verify_backup(&broken, &mut |_| Ok(())).is_err(), "{case}");
    }
}

const LEGACY_PROFILE: &str = r#"{"provider":"openai_chat","endpoint":"https://legacy.example.test/v1","model":"existing","model_digest":"","config_revision":1,"temperature":0.0,"context_tokens":131072}"#;

fn legacy_fixture(root: &Path) -> (Library, String, String) {
    let mut library = Library::open(root).unwrap();
    library
        .set_generation_profile(
            &id(),
            SetGenerationProfile {
                base_row_version: 0,
                profile: Some(serde_json::from_str(LEGACY_PROFILE).unwrap()),
            },
        )
        .unwrap();
    library
        .create_document(
            &id(),
            CreateNote {
                title: "Existing D3".into(),
                document_schema: 3,
                document: NoteDocument::from_text(id(), "Existing body"),
                collection_id: None,
                base_collection_row_version: None,
            },
        )
        .unwrap();
    let answer = library
        .accept_answer_from_settings(&id(), search())
        .unwrap()
        .answer;
    let run = chat(&mut library);
    (library, answer.id, run.id)
}

#[test]
fn k15_to_k16_preserves_legacy_json_rows_receipts_and_frozen_migrations() {
    let root = tempdir().unwrap();
    let (library, _, _) = legacy_fixture(root.path());
    let profile: GenerationProfile = serde_json::from_str(LEGACY_PROFILE).unwrap();
    assert_eq!(serde_json::to_string(&profile).unwrap(), LEGACY_PROFILE);
    assert!(profile.channel_id.is_none() && profile.capabilities.is_none());
    drop(library);
    let path = root.path().join("library.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(&format!("DROP TABLE object_tombstones; ALTER TABLE reading_states DROP COLUMN pdf_view_json; DELETE FROM migration_history WHERE version=17; ALTER TABLE search_settings DROP COLUMN retrieval_params_json; ALTER TABLE search_settings DROP COLUMN retrieval_row_version; DELETE FROM migration_history WHERE version=18; DROP TABLE retrieval_events; DROP TABLE model_usage_events; DELETE FROM migration_history WHERE version=19; DELETE FROM migration_history WHERE version=20; ALTER TABLE sources DROP COLUMN author; DELETE FROM migration_history WHERE version=21; ALTER TABLE search_settings DROP COLUMN cleaning_params_json; ALTER TABLE search_settings DROP COLUMN cleaning_row_version; ALTER TABLE source_parses DROP COLUMN cleaning_json; DELETE FROM migration_history WHERE version=22; ALTER TABLE search_settings DROP COLUMN chunk_params_json; ALTER TABLE search_settings DROP COLUMN chunk_row_version; ALTER TABLE search_settings DROP COLUMN previous_profile_id; DELETE FROM migration_history WHERE version=23; DELETE FROM migration_history WHERE version=24; DROP TABLE generation_channels; DELETE FROM migration_history WHERE version=16; {versions} {views} {automation} UPDATE workspace_meta SET schema_version=15; PRAGMA user_version=15;", versions = common::pre_docx_source_versions(), views = common::pre_views_canvas_versions(), automation = common::pre_automation_versions())).unwrap();
    let tables = [
        "notes",
        "note_revisions",
        "answers",
        "answer_events",
        "chat_conversations",
        "chat_runs",
        "chat_messages",
        "chat_run_events",
        "search_settings",
        "command_receipts",
    ];
    let compared: Vec<&str> = tables
        .iter()
        .copied()
        .filter(|table| *table != "search_settings")
        .collect();
    let before: Vec<_> = compared
        .iter()
        .copied()
        .map(|table| comparison_rows(&db, table))
        .collect();
    let before_settings = legacy_settings(&db);
    let migrations = rows(&db, "migration_history");
    let workspace: (String, i64) = db
        .query_row(
            "SELECT workspace_id,next_change_seq FROM workspace_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    drop(db);
    let migrated = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        migrated.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert!(migrated.generation_channels().unwrap().channels.is_empty());
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        compared
            .iter()
            .copied()
            .map(|table| comparison_rows(&db, table))
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(legacy_settings(&db), before_settings);
    assert_eq!(&rows(&db, "migration_history")[..15], migrations);
    assert_eq!(
        db.query_row(
            "SELECT workspace_id,next_change_seq FROM workspace_meta",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        )
        .unwrap(),
        workspace
    );
    assert_eq!(
        db.query_row(
            "SELECT generation_profile_json FROM search_settings",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        LEGACY_PROFILE
    );
    let snapshots = fs::read_dir(root.path().join("migration-backups"))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(snapshots.len(), 1);
    let original = Connection::open(snapshots[0].path()).unwrap();
    assert_eq!(
        original
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        15
    );
    assert_eq!(
        compared
            .iter()
            .copied()
            .map(|table| comparison_rows(&original, table))
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(legacy_settings(&original), before_settings);
}

#[test]
fn old_k15_packages_restore_but_reject_new_profile_fields_protocol_and_context() {
    let root = tempdir().unwrap();
    let (library, answer_id, run_id) = legacy_fixture(&root.path().join("source"));
    let current = root.path().join("current.pkmsbackup");
    library.create_backup(&current, &mut |_| Ok(())).unwrap();
    let old = root.path().join("k15.pkmsbackup");
    rewrite_package(&current, &old, 15, |db| {
        db.execute_batch(&format!("DROP TABLE object_tombstones; ALTER TABLE reading_states DROP COLUMN pdf_view_json; DELETE FROM migration_history WHERE version=17; ALTER TABLE search_settings DROP COLUMN retrieval_params_json; ALTER TABLE search_settings DROP COLUMN retrieval_row_version; DELETE FROM migration_history WHERE version=18; DROP TABLE retrieval_events; DROP TABLE model_usage_events; DELETE FROM migration_history WHERE version=19; DELETE FROM migration_history WHERE version=20; ALTER TABLE sources DROP COLUMN author; DELETE FROM migration_history WHERE version=21; ALTER TABLE search_settings DROP COLUMN cleaning_params_json; ALTER TABLE search_settings DROP COLUMN cleaning_row_version; ALTER TABLE source_parses DROP COLUMN cleaning_json; DELETE FROM migration_history WHERE version=22; ALTER TABLE search_settings DROP COLUMN chunk_params_json; ALTER TABLE search_settings DROP COLUMN chunk_row_version; ALTER TABLE search_settings DROP COLUMN previous_profile_id; DELETE FROM migration_history WHERE version=23; DELETE FROM migration_history WHERE version=24; DROP TABLE generation_channels; DELETE FROM migration_history WHERE version=16; {versions} {views} {automation} UPDATE workspace_meta SET schema_version=15; PRAGMA user_version=15;", versions = common::pre_docx_source_versions(), views = common::pre_views_canvas_versions(), automation = common::pre_automation_versions())).unwrap();
    });
    assert_eq!(
        verify_backup(&old, &mut |_| Ok(()))
            .unwrap()
            .manifest
            .database_schema,
        15
    );
    let destination = root.path().join("restored");
    prepare_restore(&old, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    assert_eq!(
        restored.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert!(restored.generation_channels().unwrap().channels.is_empty());
    assert_eq!(
        serde_json::to_string(&restored.generation_settings().unwrap().profile.unwrap()).unwrap(),
        LEGACY_PROFILE
    );
    for (case, property, value) in [
        ("channel-null", "channel_id", serde_json::Value::Null),
        ("capabilities-null", "capabilities", serde_json::Value::Null),
        ("channel-ref", "channel_id", json!(id())),
        (
            "capabilities",
            "capabilities",
            json!({"text_transport":"json","structured_transport":null}),
        ),
        ("gemini", "provider", json!("gemini")),
        ("new-context", "context_tokens", json!(2_097_152)),
        ("future", "future_behavior", json!(true)),
    ] {
        for location in ["settings", "answer", "chat"] {
            let broken = root.path().join(format!("{case}-{location}.pkmsbackup"));
            rewrite_package(&old, &broken, 15, |db| {
                let mut profile: serde_json::Value = serde_json::from_str(LEGACY_PROFILE).unwrap();
                profile[property] = value.clone();
                let profile = serde_json::to_string(&profile).unwrap();
                match location {
                    "settings" => db.execute("UPDATE search_settings SET generation_profile_json=?1", [&profile]).unwrap(),
                    "answer" => db.execute("UPDATE answers SET request_json=json_set(request_json,'$.model',json(?1)) WHERE id=?2", params![profile,answer_id]).unwrap(),
                    "chat" => db.execute("UPDATE chat_runs SET model_json=?1 WHERE id=?2", params![profile,run_id]).unwrap(),
                    _ => unreachable!(),
                };
            });
            assert!(
                verify_backup(&broken, &mut |_| Ok(())).is_err(),
                "{case}-{location}"
            );
        }
    }
}
