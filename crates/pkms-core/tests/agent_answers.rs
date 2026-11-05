use pkms_core::*;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn query(text: &str) -> SearchQuery {
    SearchQuery {
        query: text.into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    }
}
fn profile() -> GenerationProfile {
    GenerationProfile {
        channel_id: None,
        capabilities: None,
        provider: "openai_chat".into(),
        endpoint: "https://example.test/v1".into(),
        model: "synthetic".into(),
        model_digest: String::new(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 4096,
        chat_token_limit: None,
    }
}
fn start(db: &mut Library, scope: SearchScope) -> AnswerLease {
    let mut search = query("original-unmatched-question");
    search.scope = scope;
    let a = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                search,
                model: Some(profile()),
                agent: Some(AgentOptions::default()),
            },
        )
        .unwrap();
    db.claim_answer(&a.answer.id).unwrap().unwrap()
}

#[test]
fn multi_query_union_is_frozen_without_repeating_original_query() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("fruit", "apple orchard").unwrap();
    db.create_note("sky", "telescope nebula").unwrap();
    let a = start(&mut db, SearchScope::default());
    let mut evidence = db.keyword_search(&query("apple")).unwrap().items;
    evidence.extend(db.keyword_search(&query("nebula")).unwrap().items);
    assert_eq!(evidence.len(), 2);
    db.validate_agent_answer_evidence(&a.answer.id, &a.lease_token, &evidence)
        .unwrap();
    let frozen = db
        .prepare_agent_answer_evidence(&a.answer.id, &a.lease_token, evidence)
        .unwrap();
    assert_eq!(frozen.status, AnswerStatus::Running);
    assert_eq!(frozen.evidence.len(), 2);
    assert_eq!(frozen.evidence[0].citation_id, "E1");
    assert_eq!(frozen.evidence[1].citation_id, "E2");
    assert!(
        db.prepare_agent_answer_evidence(&a.answer.id, &a.lease_token, vec![])
            .is_err()
    );
}

#[test]
fn forged_out_of_scope_duplicate_and_stale_evidence_never_freezes() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("fruit", "apple orchard").unwrap();
    let other = db.create_note("other", "nebula").unwrap();
    let scope = SearchScope {
        object_ids: vec![note.id.clone()],
        ..Default::default()
    };
    let a = start(&mut db, scope);
    let valid = db.keyword_search(&query("apple")).unwrap().items.remove(0);
    let outside = db.keyword_search(&query("nebula")).unwrap().items.remove(0);
    assert_eq!(outside.target.object_id, other.id);
    let mut forged = valid.clone();
    forged.text = "made up".into();
    for evidence in [
        vec![outside],
        vec![forged],
        vec![valid.clone(), valid.clone()],
    ] {
        assert!(
            db.prepare_agent_answer_evidence(&a.answer.id, &a.lease_token, evidence)
                .is_err()
        );
        assert!(db.answer(&a.answer.id).unwrap().evidence.is_empty());
    }
    db.save_document(
        &id(),
        SaveNote {
            note_id: note.id,
            base_revision_id: note.revision_id,
            title: "changed".into(),
            document_schema: note.schema_version,
            document: note.document,
        },
    )
    .unwrap();
    assert!(
        db.validate_agent_answer_evidence(
            &a.answer.id,
            &a.lease_token,
            std::slice::from_ref(&valid)
        )
        .is_err()
    );
    assert!(
        db.prepare_agent_answer_evidence(&a.answer.id, &a.lease_token, vec![valid])
            .is_err()
    );
}

#[test]
fn legacy_json_and_setting_hash_replay_are_unchanged() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let search = query("apple");
    let legacy = serde_json::json!({"search":search,"model":null});
    let parsed: StartAnswer = serde_json::from_value(legacy.clone()).unwrap();
    assert!(parsed.agent.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), legacy);
    let command = id();
    let accepted = db
        .accept_answer_from_settings(&command, search.clone())
        .unwrap();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let stored: String = raw
        .query_row(
            "SELECT payload_hash FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get(0),
        )
        .unwrap();
    let legacy_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(1, "answers.start_from_settings", &search)).unwrap())
    );
    assert_eq!(stored, legacy_hash);
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(profile()),
        },
    )
    .unwrap();
    let replay = db
        .accept_answer_from_settings(&command, search.clone())
        .unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(accepted.answer.id, replay.answer.id);
    assert!(replay.answer.request.model.is_none());
    assert!(
        db.accept_agent_answer_from_settings(&command, search, AgentOptions::default())
            .is_err()
    );
}

#[test]
fn agent_replay_freezes_options_and_settings_and_survives_backup() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(profile()),
        },
    )
    .unwrap();
    let command = id();
    let accepted = db
        .accept_agent_answer_from_settings(&command, query("apple"), AgentOptions::default())
        .unwrap();
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 1,
            profile: None,
        },
    )
    .unwrap();
    let replay = db
        .accept_agent_answer_from_settings(&command, query("apple"), AgentOptions::default())
        .unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(accepted.answer.id, replay.answer.id);
    assert!(replay.answer.request.model.is_some());
    let changed = AgentOptions {
        max_tool_calls: 1,
        ..Default::default()
    };
    assert!(
        db.accept_agent_answer_from_settings(&command, query("apple"), changed)
            .is_err()
    );
    let package = dir.path().join("agent.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = dir.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    assert_eq!(
        restored.answer(&accepted.answer.id).unwrap().request.agent,
        Some(AgentOptions::default())
    );
}

#[test]
fn agent_options_and_unleased_freeze_are_rejected() {
    for bad in [
        AgentOptions {
            max_model_calls: 1,
            ..Default::default()
        },
        AgentOptions {
            max_tool_calls: 9,
            ..Default::default()
        },
        AgentOptions {
            max_evidence: 0,
            ..Default::default()
        },
        AgentOptions {
            total_timeout_seconds: 0,
            ..Default::default()
        },
        AgentOptions {
            total_timeout_seconds: 3601,
            ..Default::default()
        },
    ] {
        assert!(validate_agent_options(&bad).is_err());
    }
    // A local model may legitimately need the long end of the range; the bound
    // is 3600, not the historical 240.
    for accepted in [1, 240, 3600] {
        assert!(
            validate_agent_options(&AgentOptions {
                total_timeout_seconds: accepted,
                ..Default::default()
            })
            .is_ok()
        );
    }
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let a = start(&mut db, SearchScope::default());
    assert!(
        db.prepare_agent_answer_evidence(&a.answer.id, &id(), vec![])
            .is_err()
    );
    let ended = db
        .prepare_agent_answer_evidence(&a.answer.id, &a.lease_token, vec![])
        .unwrap();
    assert_eq!(ended.status, AnswerStatus::NoEvidence);
    assert!(
        db.validate_agent_answer_evidence(&a.answer.id, &a.lease_token, &[])
            .is_err()
    );
}

#[test]
fn backup_rejects_corrupt_agent_budget_without_schema_change() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let accepted = db
        .accept_agent_answer_from_settings(&id(), query("apple"), AgentOptions::default())
        .unwrap();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let schema: i64 = raw
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(schema, pkms_core::CURRENT_SCHEMA_VERSION);
    raw.execute("UPDATE answers SET request_json=json_set(request_json,'$.agent.max_tool_calls',999) WHERE id=?1",[&accepted.answer.id]).unwrap();
    let package = dir.path().join("invalid.zip");
    assert!(db.create_backup(&package, &mut |_| Ok(())).is_err());
    assert!(!package.exists());
}

#[test]
fn preflight_checks_scope_contents_not_query_hits_and_unconfigured_is_explicit() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let empty = db.create_note("empty", "   ").unwrap();
    let scope = SearchScope {
        object_ids: vec![empty.id],
        ..Default::default()
    };
    let a = start(&mut db, scope);
    assert!(
        !db.agent_scope_has_evidence(&a.answer.id, &a.lease_token)
            .unwrap()
    );
    let note = db.create_note("fruit", "apple orchard").unwrap();
    let a = start(
        &mut db,
        SearchScope {
            object_ids: vec![note.id.clone()],
            ..Default::default()
        },
    );
    assert!(
        db.keyword_search(&a.answer.request.search)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        db.agent_scope_has_evidence(&a.answer.id, &a.lease_token)
            .unwrap()
    );
    db.set_object_search_enabled(
        &id(),
        SetObjectFlag {
            object_id: note.id.clone(),
            base_row_version: db.object_metadata(&note.id).unwrap().row_version,
            value: false,
        },
    )
    .unwrap();
    assert!(
        !db.agent_scope_has_evidence(&a.answer.id, &a.lease_token)
            .unwrap()
    );
    db.set_object_search_enabled(
        &id(),
        SetObjectFlag {
            object_id: note.id.clone(),
            base_row_version: db.object_metadata(&note.id).unwrap().row_version,
            value: true,
        },
    )
    .unwrap();
    db.set_object_trashed(
        &id(),
        SetObjectFlag {
            object_id: note.id.clone(),
            base_row_version: db.object_metadata(&note.id).unwrap().row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(
        !db.agent_scope_has_evidence(&a.answer.id, &a.lease_token)
            .unwrap()
    );
    let collection = db.create_collection("empty topic").unwrap();
    let a = start(
        &mut db,
        SearchScope {
            collection_id: Some(collection.id),
            ..Default::default()
        },
    );
    assert!(
        !db.agent_scope_has_evidence(&a.answer.id, &a.lease_token)
            .unwrap()
    );
    let accepted = db
        .accept_agent_answer_from_settings(&id(), query("x"), AgentOptions::default())
        .unwrap();
    let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    let result = db
        .prepare_agent_answer_evidence(&lease.answer.id, &lease.lease_token, vec![])
        .unwrap();
    assert_eq!(result.status, AnswerStatus::ModelUnconfigured);
    db.create_backup(dir.path().join("unconfigured.zip"), &mut |_| Ok(()))
        .unwrap();
}

#[test]
fn preflight_interrupts_during_multiple_pages_without_pinning_cancellation_snapshot() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    for _ in 0..40 {
        db.create_note("empty", " ").unwrap();
    }
    let a = start(&mut db, SearchScope::default());
    let mut calls = 0;
    let result = db.agent_scope_has_evidence_with_check(&a.answer.id, &a.lease_token, &mut || {
        calls += 1;
        if calls == 20 {
            return Err(Error::InvalidInput("test_deadline"));
        }
        Ok(())
    });
    assert!(matches!(result, Err(Error::InvalidInput("test_deadline"))));
    assert_eq!(calls, 20);
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let mut calls = 0;
    let result=db.agent_scope_has_evidence_with_check(&a.answer.id,&a.lease_token,&mut || {
        calls+=1;
        if calls==8 {raw.execute("UPDATE answers SET status='canceled',lease_token=NULL,lease_expires_at=NULL WHERE id=?1",[&a.answer.id]).unwrap();}
        Ok(())
    });
    assert!(result.is_err());
    assert!(
        calls < 20,
        "next object must see cancellation from another connection"
    );
    let a = start(&mut db, SearchScope::default());
    let mut calls = 0;
    assert!(
        !db.agent_scope_has_evidence_with_check(&a.answer.id, &a.lease_token, &mut || {
            calls += 1;
            Ok(())
        })
        .unwrap()
    );
    assert!(calls > 40, "all pages must remain interruptible");
}
