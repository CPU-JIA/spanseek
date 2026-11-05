//! Controlled generations exercise queue persistence, not embedding quality.
use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn spec(rev: &str) -> EmbeddingProfileSpec {
    EmbeddingProfileSpec {
        endpoint: None,
        model: "fixture".into(),
        revision: rev.into(),
        dimensions: 3,
        normalized: true,
        query_prefix: "query: ".into(),
        passage_prefix: "passage: ".into(),
        tokenizer: "fixture".into(),
        chunker: "fixture".into(),
        max_tokens: 512,
        overlap_tokens: 0,
        provider: None,
        output_dimensions: None,
    }
}
fn ready(db: &mut Library, object: &str, profile: &EmbeddingProfile) -> IndexGeneration {
    let input = db.index_input(object).unwrap();
    let plan = IndexPlan {
        profile_id: profile.id.clone(),
        object_id: object.into(),
        content_revision: input.content_revision,
        metadata_revision: input.metadata_revision,
        parser_revision: input.parser_revision,
        chunks: input
            .blocks
            .into_iter()
            .map(|b| IndexChunkInput {
                chunk_id: id(),
                block_id: b.target.block_id,
                start_codepoint: 0,
                end_codepoint: b.text.chars().count() as u32,
                text: b.text,
                token_count: 10,
            })
            .collect(),
    };
    let l = db.begin_index_generation(&id(), plan).unwrap();
    db.publish_index_generation(IndexPublish {
        generation_id: l.generation.id,
        lease_token: l.lease_token,
        profile_id: profile.id.clone(),
        chunk_count: l.generation.chunk_count,
    })
    .unwrap()
}
#[test]
fn generation_settings_cas_canonical_revision_and_receipt_rollback() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    assert_eq!(db.generation_settings().unwrap().row_version, 0);
    let request = SetGenerationProfile {
        base_row_version: 0,
        profile: Some(GenerationProfile {
            channel_id: None,
            capabilities: None,
            chat_token_limit: None,
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "fixture".into(),
            model_digest: "fixture".into(),
            config_revision: 999,
            temperature: 0.0,
            context_tokens: 4096,
        }),
    };
    let command = id();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_settings BEFORE INSERT ON command_receipts WHEN NEW.command_kind='generation.configure' BEGIN SELECT RAISE(ABORT,'controlled'); END;").unwrap();
    assert!(
        db.set_generation_profile(&command, request.clone())
            .is_err()
    );
    assert_eq!(db.generation_settings().unwrap().row_version, 0);
    sql.execute_batch("DROP TRIGGER fail_settings;").unwrap();
    let saved = db
        .set_generation_profile(&command, request.clone())
        .unwrap();
    assert_eq!(saved.settings.profile.unwrap().config_revision, 1);
    assert!(
        db.set_generation_profile(&command, request.clone())
            .unwrap()
            .receipt
            .replayed
    );
    assert!(matches!(
        db.set_generation_profile(&id(), request),
        Err(Error::MetadataConflict { .. })
    ));
    let question = SearchQuery {
        query: "evidence".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    };
    let answer_command = id();
    let original = db
        .accept_answer_from_settings(&answer_command, question.clone())
        .unwrap();
    assert_eq!(
        original
            .answer
            .request
            .model
            .as_ref()
            .unwrap()
            .config_revision,
        1
    );
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 1,
            profile: None,
        },
    )
    .unwrap();
    let replay = db
        .accept_answer_from_settings(&answer_command, question.clone())
        .unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.answer.id, original.answer.id);
    assert_eq!(
        replay
            .answer
            .request
            .model
            .as_ref()
            .unwrap()
            .config_revision,
        1
    );
    assert!(
        db.accept_answer_from_settings(&id(), question)
            .unwrap()
            .answer
            .request
            .model
            .is_none()
    );
    drop(db);
    let db = Library::open(dir.path()).unwrap();
    assert_eq!(db.generation_settings().unwrap().row_version, 2);
    assert!(db.generation_settings().unwrap().profile.is_none());
}
#[test]
fn rebuild_manifest_order_ready_identity_and_explicit_trash_skip() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("first", "evidence one").unwrap();
    db.create_note("second", "evidence two").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let command = id();
    let req = RebuildRequest {
        profile_id: profile.id.clone(),
        scope: SearchScope::default(),
    };
    let job = db.accept_index_rebuild(&command, req.clone()).unwrap().job;
    assert!(
        db.accept_index_rebuild(&command, req)
            .unwrap()
            .receipt
            .replayed
    );
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let objects = db.index_rebuild_next_objects(&job.id, 100).unwrap();
    assert_eq!(objects.len(), 2);
    assert!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &objects[1], None, Some("skipped"))
            .is_err()
    );
    assert!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &objects[0], None, None)
            .is_err()
    );
    let g = ready(&mut db, &objects[0], &profile);
    let done = db
        .checkpoint_index_rebuild(&job.id, &l.lease_token, &objects[0], Some(&g.id), None)
        .unwrap();
    assert_eq!(done.processed, 1);
    assert_eq!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &objects[0], Some(&g.id), None)
            .unwrap()
            .processed,
        1
    );
    db.set_trashed(&objects[1], true).unwrap();
    assert_eq!(
        db.index_rebuild_next_objects(&job.id, 100).unwrap(),
        vec![objects[1].clone()]
    );
    assert!(db.finish_index_rebuild(&job.id, &l.lease_token).is_err());
    db.checkpoint_index_rebuild(
        &job.id,
        &l.lease_token,
        &objects[1],
        None,
        Some("object_trashed"),
    )
    .unwrap();
    let done = db.finish_index_rebuild(&job.id, &l.lease_token).unwrap();
    assert_eq!(done.status, RebuildStatus::Completed);
    assert_eq!(done.processed, 2);
    assert_eq!(done.skipped, 1);
    assert!(db.heartbeat_index_rebuild(&job.id, &l.lease_token).is_err());
    drop(db);
    let db = Library::open(dir.path()).unwrap();
    assert_eq!(db.index_rebuild_job(&job.id).unwrap().skipped, 1);
}
#[test]
fn rebuild_new_eligible_object_and_stale_generation_are_not_silent_success() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("first", "evidence one").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let g = ready(&mut db, &n.id, &profile);
    db.save_note(&n.id, n.revision, "edited", "different")
        .unwrap();
    assert!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &n.id, Some(&g.id), None)
            .is_err()
    );
    db.checkpoint_index_rebuild(&job.id, &l.lease_token, &n.id, None, Some("input_changed"))
        .unwrap();
    db.create_note("new arrival", "new evidence").unwrap();
    let done = db.finish_index_rebuild(&job.id, &l.lease_token).unwrap();
    assert_eq!(done.status, RebuildStatus::Failed);
    assert_eq!(done.error_code.as_deref(), Some("scope_changed"));
}
#[test]
fn canceled_profile_changed_and_expired_rebuild_leases_cannot_advance() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("first", "evidence one").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let req = RebuildRequest {
        profile_id: profile.id.clone(),
        scope: SearchScope::default(),
    };
    let job = db.accept_index_rebuild(&id(), req.clone()).unwrap().job;
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let cmd = id();
    db.cancel_index_rebuild(&cmd, &job.id).unwrap();
    assert!(
        db.cancel_index_rebuild(&cmd, &job.id)
            .unwrap()
            .receipt
            .replayed
    );
    assert!(db.heartbeat_index_rebuild(&job.id, &l.lease_token).is_err());
    assert!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &n.id, None, Some("skip"))
            .is_err()
    );
    let job = db.accept_index_rebuild(&id(), req.clone()).unwrap().job;
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute(
        "UPDATE index_rebuild_jobs SET lease_expires_at='2000-01-01T00:00:00+00:00' WHERE id=?1",
        [&job.id],
    )
    .unwrap();
    assert_eq!(db.recover_expired_index_rebuilds().unwrap(), 1);
    assert!(db.heartbeat_index_rebuild(&job.id, &l.lease_token).is_err());
    assert_eq!(
        db.index_rebuild_job(&job.id).unwrap().error_code.as_deref(),
        Some("interrupted")
    );
    let queued_old = db.accept_index_rebuild(&id(), req.clone()).unwrap().job;
    let job = db.accept_index_rebuild(&id(), req).unwrap().job;
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let newer = db
        .register_embedding_profile(&id(), spec("2"))
        .unwrap()
        .profile;
    assert!(db.heartbeat_index_rebuild(&job.id, &l.lease_token).is_err());
    assert!(
        db.checkpoint_index_rebuild(&job.id, &l.lease_token, &n.id, None, Some("skip"))
            .is_err()
    );
    assert_eq!(
        db.fail_index_rebuild(&job.id, &l.lease_token, "profile_changed")
            .unwrap()
            .status,
        RebuildStatus::Failed
    );
    assert!(db.claim_index_rebuild(&queued_old.id).unwrap().is_none());
    assert_eq!(
        db.index_rebuild_job(&queued_old.id)
            .unwrap()
            .error_code
            .as_deref(),
        Some("profile_changed")
    );
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: newer.id,
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    db.claim_index_rebuild(&job.id).unwrap().unwrap();
    assert_eq!(db.recover_index_rebuilds_exclusive().unwrap(), 1);
    assert_eq!(
        db.index_rebuild_job(&job.id).unwrap().error_code.as_deref(),
        Some("interrupted")
    );
}

#[test]
fn complete_package_preserves_rebuild_cursor_skip_and_generation_settings() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    db.create_note("first", "text one").unwrap();
    db.create_note("second", "text two").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(GenerationProfile {
                channel_id: None,
                capabilities: None,
                chat_token_limit: None,
                provider: "ollama".into(),
                endpoint: "http://127.0.0.1:11434".into(),
                model: "fixture".into(),
                model_digest: "frozen-digest".into(),
                config_revision: 0,
                temperature: 0.0,
                context_tokens: 4096,
            }),
        },
    )
    .unwrap();
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id,
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let l = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let objects = db.index_rebuild_next_objects(&job.id, 100).unwrap();
    db.checkpoint_index_rebuild(
        &job.id,
        &l.lease_token,
        &objects[0],
        None,
        Some("empty_after_parse"),
    )
    .unwrap();
    let package = root.path().join("queue.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    let recovered = restored.index_rebuild_job(&job.id).unwrap();
    assert_eq!(recovered.status, RebuildStatus::Failed);
    assert_eq!(recovered.error_code.as_deref(), Some("interrupted"));
    assert_eq!(recovered.processed, 1);
    assert_eq!(recovered.skipped, 1);
    assert_eq!(
        recovered.after_object_id.as_deref(),
        Some(objects[0].as_str())
    );
    assert_eq!(
        restored.index_rebuild_next_objects(&job.id, 100).unwrap(),
        vec![objects[1].clone()]
    );
    let settings = restored.generation_settings().unwrap();
    assert_eq!(settings.row_version, 1);
    assert_eq!(settings.profile.unwrap().model_digest, "frozen-digest");
}

#[test]
fn cancel_only_clears_frozen_inputs_and_replay_preserves_new_edits() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let same = db.create_note("same", "text").unwrap();
    let edited = db.create_note("edited", "text").unwrap();
    let outside = db.create_note("outside", "text").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id,
                scope: SearchScope {
                    object_ids: vec![same.id.clone(), edited.id.clone()],
                    ..Default::default()
                },
            },
        )
        .unwrap()
        .job;
    db.save_note(&edited.id, edited.revision, "edited later", "new text")
        .unwrap();
    let command = id();
    db.cancel_index_rebuild(&command, &job.id).unwrap();
    let dirty = db.dirty_index_object_ids(100).unwrap();
    assert!(!dirty.contains(&same.id));
    assert!(dirty.contains(&edited.id));
    assert!(dirty.contains(&outside.id));
    db.save_note(&same.id, same.revision, "changed after cancel", "fresh")
        .unwrap();
    assert!(
        db.cancel_index_rebuild(&command, &job.id)
            .unwrap()
            .receipt
            .replayed
    );
    assert!(db.dirty_index_object_ids(100).unwrap().contains(&same.id));
}
#[test]
fn failed_rebuild_backoff_cannot_delay_new_edit_or_new_profile() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let first = db.create_note("same", "text").unwrap();
    let second = db.create_note("edited", "text").unwrap();
    let (same, edited) = if first.id < second.id {
        (first, second)
    } else {
        (second, first)
    };
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id,
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let lease = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    db.save_note(&edited.id, edited.revision, "changed", "new text")
        .unwrap();
    db.fail_index_rebuild(&job.id, &lease.lease_token, "worker_failed")
        .unwrap();
    db.defer_dirty_index_rebuild(&job.id, "worker_failed")
        .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let state = |object: &str| {
        sql.query_row(
            "SELECT attempts,next_retry_at,last_error FROM index_dirty_objects WHERE object_id=?1",
            [object],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap()
    };
    assert_eq!(state(&same.id).0, 1);
    assert!(state(&same.id).1.is_some());
    assert_eq!(state(&edited.id), (0, None, None));
    db.register_embedding_profile(&id(), spec("2")).unwrap();
    db.defer_dirty_index_rebuild(&job.id, "old_profile_failure")
        .unwrap();
    assert_eq!(state(&same.id), (0, None, None));
    assert_eq!(state(&edited.id), (0, None, None));
}
#[test]
fn replacing_parse_with_same_parser_preserves_dirty_after_cancel_and_failure() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let input = id();
    let source_job = db
        .accept_source_import(
            &input,
            ImportSource {
                input_token: input.clone(),
                original_name: "source.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&source_job.job_id).unwrap().unwrap();
    let bytes = b"source text";
    let version = db
        .capture_source_original(
            &source_job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &source_job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap(),
    )
    .unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id,
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let parse: String = sql
        .query_row(
            "SELECT parse_id FROM source_active_parses WHERE source_version_id=?1",
            [&version.id],
            |r| r.get(0),
        )
        .unwrap();
    let replacement = id();
    sql.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) SELECT ?1,source_version_id,parser_version,page_count,needs_ocr,created_at FROM source_parses WHERE id=?2",rusqlite::params![replacement,parse]).unwrap();
    sql.execute(
        "UPDATE source_active_parses SET parse_id=?1 WHERE source_version_id=?2",
        rusqlite::params![replacement, version.id],
    )
    .unwrap();
    db.defer_dirty_index_rebuild(&job.id, "old_parse_failure")
        .unwrap();
    let attempts: u32 = sql
        .query_row(
            "SELECT attempts FROM index_dirty_objects WHERE object_id=?1",
            [&version.source_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 0);
    db.cancel_index_rebuild(&id(), &job.id).unwrap();
    assert!(
        db.dirty_index_object_ids(100)
            .unwrap()
            .contains(&version.source_id)
    );
}

fn pending_generation(db: &mut Library, object: &str, profile: &EmbeddingProfile) -> IndexLease {
    let input = db.index_input(object).unwrap();
    db.begin_index_generation(
        &id(),
        IndexPlan {
            profile_id: profile.id.clone(),
            object_id: object.into(),
            content_revision: input.content_revision,
            metadata_revision: input.metadata_revision,
            parser_revision: input.parser_revision,
            chunks: input
                .blocks
                .into_iter()
                .map(|b| IndexChunkInput {
                    chunk_id: id(),
                    block_id: b.target.block_id,
                    start_codepoint: 0,
                    end_codepoint: b.text.chars().count() as u32,
                    text: b.text,
                    token_count: 10,
                })
                .collect(),
        },
    )
    .unwrap()
}
fn publish_request(l: &IndexLease) -> IndexPublish {
    IndexPublish {
        generation_id: l.generation.id.clone(),
        lease_token: l.lease_token.clone(),
        profile_id: l.generation.profile_id.clone(),
        chunk_count: l.generation.chunk_count,
    }
}
#[test]
fn canceled_rebuild_cannot_publish_a_ready_worker_result() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "text").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let lease = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let generation = pending_generation(&mut db, &n.id, &profile);
    db.cancel_index_rebuild(&id(), &job.id).unwrap();
    assert!(
        db.publish_rebuild_generation(&job.id, &lease.lease_token, publish_request(&generation))
            .is_err()
    );
    assert_eq!(
        db.index_generation(&generation.generation.id)
            .unwrap()
            .status,
        IndexGenerationStatus::Running
    );
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    assert_eq!(db.index_rebuild_job(&job.id).unwrap().processed, 0);
}
#[test]
fn checkpoint_failure_rolls_back_publication_active_pointer_and_dirty_clear() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "text").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let previous = ready(&mut db, &n.id, &profile);
    db.save_note(&n.id, n.revision, "new title", "new text")
        .unwrap();
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let lease = db.claim_index_rebuild(&job.id).unwrap().unwrap();
    let generation = pending_generation(&mut db, &n.id, &profile);
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_checkpoint BEFORE UPDATE ON index_rebuild_objects WHEN NEW.generation_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'controlled checkpoint failure'); END;").unwrap();
    assert!(
        db.publish_rebuild_generation(&job.id, &lease.lease_token, publish_request(&generation))
            .is_err()
    );
    assert_eq!(
        db.index_generation(&generation.generation.id)
            .unwrap()
            .status,
        IndexGenerationStatus::Running
    );
    let active: String = sql
        .query_row(
            "SELECT generation_id FROM active_index_generations WHERE object_id=?1",
            [&n.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, previous.id);
    assert!(db.dirty_index_object_ids(100).unwrap().contains(&n.id));
    assert_eq!(db.index_rebuild_job(&job.id).unwrap().processed, 0);
    sql.execute_batch("DROP TRIGGER fail_checkpoint;").unwrap();
    let result = db
        .publish_rebuild_generation(&job.id, &lease.lease_token, publish_request(&generation))
        .unwrap();
    assert_eq!(result.processed, 1);
    assert!(!db.dirty_index_object_ids(100).unwrap().contains(&n.id));
    assert_eq!(
        db.active_index_generation_ids().unwrap(),
        vec![generation.generation.id.clone()]
    );
    assert_eq!(
        db.publish_rebuild_generation(&job.id, &lease.lease_token, publish_request(&generation))
            .unwrap()
            .processed,
        1
    );
}
#[test]
fn rebuild_backoff_caps_attempts_and_stops_deterministic_errors() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "text").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let request = RebuildRequest {
        profile_id: profile.id,
        scope: SearchScope::default(),
    };
    let job = db.accept_index_rebuild(&id(), request.clone()).unwrap().job;
    for _ in 0..5 {
        db.defer_dirty_index_rebuild(&job.id, "worker_failed")
            .unwrap();
    }
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let attempts = || {
        sql.query_row(
            "SELECT attempts FROM index_dirty_objects WHERE object_id=?1",
            [&n.id],
            |r| r.get::<_, u32>(0),
        )
        .unwrap()
    };
    assert_eq!(attempts(), 3);
    db.save_note(&n.id, n.revision, "new", "fresh input")
        .unwrap();
    assert_eq!(attempts(), 0);
    let new_job = db.accept_index_rebuild(&id(), request).unwrap().job;
    db.defer_dirty_index_rebuild(&new_job.id, "embedding_invalid_input")
        .unwrap();
    assert_eq!(attempts(), 3);
}

#[test]
fn deterministic_failure_only_stops_the_attempted_object_not_the_remaining_batch() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("first", "text").unwrap();
    db.create_note("second", "text").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("1"))
        .unwrap()
        .profile;
    let job = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: profile.id,
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let objects = db.index_rebuild_next_objects(&job.id, 100).unwrap();
    db.defer_dirty_index_rebuild(&job.id, "embedding_title_token_limit")
        .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let attempts = |object: &str| {
        sql.query_row(
            "SELECT attempts FROM index_dirty_objects WHERE object_id=?1",
            [object],
            |r| r.get::<_, u32>(0),
        )
        .unwrap()
    };
    assert_eq!(attempts(&objects[0]), 3);
    assert_eq!(attempts(&objects[1]), 0);
    let dirty = db.dirty_index_object_ids(100).unwrap();
    assert!(!dirty.contains(&objects[0]));
    assert!(dirty.contains(&objects[1]));
}
