use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn spec(revision: &str) -> EmbeddingProfileSpec {
    EmbeddingProfileSpec {
        endpoint: None,
        model: "controlled-vector-test".into(),
        revision: revision.into(),
        dimensions: 3,
        normalized: true,
        query_prefix: "query: ".into(),
        passage_prefix: "passage: ".into(),
        tokenizer: "test-tokenizer".into(),
        chunker: "test-complete-block".into(),
        max_tokens: 512,
        overlap_tokens: 0,
        provider: None,
        output_dimensions: None,
    }
}
fn plan(input: &IndexInput, profile: &EmbeddingProfile) -> IndexPlan {
    IndexPlan {
        profile_id: profile.id.clone(),
        object_id: input.object_id.clone(),
        content_revision: input.content_revision.clone(),
        metadata_revision: input.metadata_revision,
        parser_revision: input.parser_revision.clone(),
        chunks: input
            .blocks
            .iter()
            .map(|b| IndexChunkInput {
                chunk_id: id(),
                block_id: b.target.block_id.clone(),
                start_codepoint: 0,
                end_codepoint: b.text.chars().count() as u32,
                text: b.text.clone(),
                token_count: 10,
            })
            .collect(),
    }
}
fn publication(lease: &IndexLease) -> IndexPublish {
    IndexPublish {
        generation_id: lease.generation.id.clone(),
        lease_token: lease.lease_token.clone(),
        profile_id: lease.generation.profile_id.clone(),
        chunk_count: lease.generation.chunk_count,
    }
}
fn query(mode: SearchMode) -> SearchQuery {
    SearchQuery {
        query: "中文知识".into(),
        mode,
        scope: SearchScope::default(),
        limit: 10,
        minimum_cosine: Some(0.5),
    }
}

#[test]
fn dirty_queue_coalesces_persists_backoff_and_clears_only_current_input() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut note = db.create_note("dirty", "initial").unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
    let profile = db
        .register_embedding_profile(&id(), spec("dirty-first"))
        .unwrap()
        .profile;
    let stale = db.index_input(&note.id).unwrap();
    for i in 0..10 {
        note = db
            .save_note(&note.id, note.revision, "dirty", &format!("edit {i}"))
            .unwrap();
    }
    assert_eq!(
        db.dirty_index_object_ids(32).unwrap(),
        vec![note.id.clone()]
    );
    assert!(!db.clear_dirty_index_input(&profile.id, &stale).unwrap());
    db.defer_dirty_index_objects(&[note.id.clone()], "embedding_failed")
        .unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
    drop(db);
    let mut db = Library::open_existing(root.path()).unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
    let next = db
        .register_embedding_profile(&id(), spec("dirty-second"))
        .unwrap()
        .profile;
    assert_eq!(
        db.dirty_index_object_ids(32).unwrap(),
        vec![note.id.clone()]
    );
    let input = db.index_input(&note.id).unwrap();
    assert!(!db.clear_dirty_index_input(&profile.id, &input).unwrap());
    assert!(db.clear_dirty_index_input(&next.id, &input).unwrap());
    note = db
        .save_note(&note.id, note.revision, "dirty", "publication")
        .unwrap();
    assert_eq!(
        db.dirty_index_object_ids(32).unwrap(),
        vec![note.id.clone()]
    );
    let lease = db
        .begin_index_generation(&id(), plan(&db.index_input(&note.id).unwrap(), &next))
        .unwrap();
    db.publish_index_generation(publication(&lease)).unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
}

#[test]
fn vector_admission_rejects_wrong_profile_stale_metadata_and_forged_candidates() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("中文知识", "中文知识需要真实证据").unwrap();
    let lexical = db
        .search_with_vectors(&query(SearchMode::Hybrid), vec![])
        .unwrap();
    assert!(!lexical.items.is_empty());
    assert!(!lexical.semantic_available);
    let profile = db
        .register_embedding_profile(&id(), spec("first"))
        .unwrap()
        .profile;
    let input = db.index_input(&note.id).unwrap();
    let p = plan(&input, &profile);
    let chunk = p.chunks[0].chunk_id.clone();
    let command = id();
    let lease = db.begin_index_generation(&command, p.clone()).unwrap();
    assert_eq!(
        db.begin_index_generation(&command, p)
            .unwrap()
            .generation
            .id,
        lease.generation.id
    );
    let candidate = VectorCandidate {
        chunk_id: chunk,
        generation_id: lease.generation.id.clone(),
        object_id: note.id.clone(),
        content_revision: input.content_revision,
        profile_id: profile.id.clone(),
        cosine_similarity: 0.9,
    };
    assert!(
        db.search_with_vectors(&query(SearchMode::Semantic), vec![candidate.clone()])
            .unwrap()
            .items
            .is_empty()
    );
    let publish = publication(&lease);
    db.publish_index_generation(publish.clone()).unwrap();
    assert_eq!(
        db.publish_index_generation(publish).unwrap().status,
        IndexGenerationStatus::Ready
    );
    let hit = db
        .search_with_vectors(&query(SearchMode::Hybrid), vec![candidate.clone()])
        .unwrap();
    assert!(hit.semantic_available);
    assert!(hit.items.iter().any(|i| i.cosine_similarity == Some(0.9)));
    let mut wrong = candidate.clone();
    wrong.content_revision = id();
    assert!(
        db.search_with_vectors(&query(SearchMode::Semantic), vec![wrong])
            .unwrap()
            .items
            .is_empty()
    );
    let mut invalid = candidate.clone();
    invalid.cosine_similarity = f64::NAN;
    assert!(
        db.search_with_vectors(&query(SearchMode::Semantic), vec![invalid])
            .is_err()
    );
    db.register_embedding_profile(&id(), spec("second"))
        .unwrap();
    assert!(
        db.search_with_vectors(&query(SearchMode::Semantic), vec![candidate.clone()])
            .unwrap()
            .items
            .is_empty()
    );
    db.register_embedding_profile(&id(), spec("first")).unwrap();
    db.save_note(&note.id, note.revision, "中文知识", "中文知识已经变更")
        .unwrap();
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    assert!(
        db.search_with_vectors(&query(SearchMode::Semantic), vec![candidate])
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn publication_failure_rolls_back_pointer_and_cancel_rejects_late_worker() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("中文知识", "中文知识原文").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("first"))
        .unwrap()
        .profile;
    let input = db.index_input(&note.id).unwrap();
    let lease = db
        .begin_index_generation(&id(), plan(&input, &profile))
        .unwrap();
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_index_pointer BEFORE INSERT ON active_index_generations BEGIN SELECT RAISE(ABORT,'controlled publication failure'); END;").unwrap();
    assert!(db.publish_index_generation(publication(&lease)).is_err());
    assert_eq!(
        db.index_generation(&lease.generation.id).unwrap().status,
        IndexGenerationStatus::Running
    );
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    raw.execute_batch("DROP TRIGGER fail_index_pointer;")
        .unwrap();
    db.publish_index_generation(publication(&lease)).unwrap();
    let later = db
        .begin_index_generation(&id(), plan(&input, &profile))
        .unwrap();
    db.cancel_index_generation(&later.generation.id).unwrap();
    assert!(db.publish_index_generation(publication(&later)).is_err());
    assert_eq!(
        db.active_index_generation_ids().unwrap(),
        vec![lease.generation.id]
    );
}

#[test]
fn chunks_cannot_fabricate_or_silently_truncate_original_text() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("中文知识", "真实😀中文原文").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("first"))
        .unwrap()
        .profile;
    let input = db.index_input(&note.id).unwrap();
    let mut p = plan(&input, &profile);
    p.chunks[0].text = "fabricated".into();
    assert!(db.begin_index_generation(&id(), p).is_err());
    let mut p = plan(&input, &profile);
    p.chunks[0].end_codepoint -= 1;
    p.chunks[0].text.pop();
    assert!(db.begin_index_generation(&id(), p).is_err());
    let mut p = plan(&input, &profile);
    p.chunks[0].token_count = 513;
    assert!(db.begin_index_generation(&id(), p).is_err());
    let lease = db
        .begin_index_generation(&id(), plan(&input, &profile))
        .unwrap();
    let page = db.index_chunks_page(&lease.generation.id, None, 1).unwrap();
    assert_eq!(page[0].text, "真实😀中文原文");
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE index_generations SET lease_expires_at='2000-01-01T00:00:00Z' WHERE id=?1",
        [&lease.generation.id],
    )
    .unwrap();
    assert_eq!(db.recover_expired_index_generations().unwrap(), 1);
    assert!(db.publish_index_generation(publication(&lease)).is_err());
}

#[test]
fn automatic_retry_stops_at_three_or_deterministic_error_and_edits_reset_it() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut note = db.create_note("retry", "body").unwrap();
    db.register_embedding_profile(&id(), spec("retry")).unwrap();
    let sql = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    for attempt in 1..=3 {
        db.defer_dirty_index_objects(&[note.id.clone()], "embedding_inference_failed")
            .unwrap();
        sql.execute(
            "UPDATE index_dirty_objects SET next_retry_at='2000-01-01T00:00:00+00:00'",
            [],
        )
        .unwrap();
        assert_eq!(
            db.dirty_index_object_ids(32).unwrap().is_empty(),
            attempt == 3
        );
    }
    drop(db);
    let mut db = Library::open_existing(root.path()).unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
    note = db
        .save_note(&note.id, note.revision, "retry", "new input")
        .unwrap();
    assert_eq!(
        db.dirty_index_object_ids(32).unwrap(),
        vec![note.id.clone()]
    );
    db.defer_dirty_index_objects(&[note.id.clone()], "embedding_model_hash_mismatch")
        .unwrap();
    sql.execute(
        "UPDATE index_dirty_objects SET next_retry_at='2000-01-01T00:00:00+00:00'",
        [],
    )
    .unwrap();
    assert!(db.dirty_index_object_ids(32).unwrap().is_empty());
    let attempts: u32 = sql
        .query_row(
            "SELECT attempts FROM index_dirty_objects WHERE object_id=?1",
            [&note.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 3);
    db.register_embedding_profile(&id(), spec("retry-new-config"))
        .unwrap();
    assert_eq!(db.dirty_index_object_ids(32).unwrap(), vec![note.id]);
}

#[test]
fn active_manifest_revalidates_each_lifecycle_field_and_scope() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let first = db.create_note("first", "current evidence").unwrap();
    let second = db.create_note("second", "other evidence").unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("manifest"))
        .unwrap()
        .profile;
    let mut leases = Vec::new();
    for note in [&first, &second] {
        let input = db.index_input(&note.id).unwrap();
        let lease = db
            .begin_index_generation(&id(), plan(&input, &profile))
            .unwrap();
        db.publish_index_generation(publication(&lease)).unwrap();
        leases.push(lease);
    }
    let visible = |db: &Library| {
        let rows = db.active_index_generations().unwrap();
        let ids = rows.iter().map(|g| g.id.clone()).collect::<Vec<_>>();
        assert_eq!(db.active_index_generation_ids().unwrap(), ids);
        assert_eq!(db.index_status().unwrap().ready_objects as usize, ids.len());
        assert_eq!(
            db.search_with_vectors(&query(SearchMode::Semantic), vec![])
                .unwrap()
                .semantic_available,
            !ids.is_empty()
        );
        ids
    };
    let mut expected = leases
        .iter()
        .map(|l| l.generation.id.clone())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(visible(&db), expected);
    let scoped = SearchScope {
        object_ids: vec![first.id.clone()],
        ..Default::default()
    };
    assert_eq!(
        db.active_index_generations_in_scope(&scoped).unwrap()[0].id,
        leases[0].generation.id
    );
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE objects SET search_enabled=0 WHERE id=?1",
        [&second.id],
    )
    .unwrap();
    for (invalidate, restore) in [
        (
            "UPDATE objects SET trashed=1 WHERE id=?1",
            "UPDATE objects SET trashed=0 WHERE id=?1",
        ),
        (
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            "UPDATE objects SET row_version=row_version-1 WHERE id=?1",
        ),
        (
            "UPDATE objects SET search_enabled=0 WHERE id=?1",
            "UPDATE objects SET search_enabled=1 WHERE id=?1",
        ),
    ] {
        raw.execute(invalidate, [&first.id]).unwrap();
        assert!(visible(&db).is_empty());
        raw.execute(restore, [&first.id]).unwrap();
        assert_eq!(visible(&db), vec![leases[0].generation.id.clone()]);
    }
    raw.execute(
        "UPDATE objects SET search_enabled=1 WHERE id=?1",
        [&second.id],
    )
    .unwrap();
    // Controlled stale state: leave the published pointer in place so the
    // enumeration itself must reject every field, rather than relying on cleanup.
    for (invalidate, restore) in [
        (
            "UPDATE objects SET trashed=1 WHERE id=?1",
            "UPDATE objects SET trashed=0 WHERE id=?1",
        ),
        (
            "UPDATE objects SET search_enabled=0 WHERE id=?1",
            "UPDATE objects SET search_enabled=1 WHERE id=?1",
        ),
        (
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            "UPDATE objects SET row_version=row_version-1 WHERE id=?1",
        ),
    ] {
        raw.execute(invalidate, [&first.id]).unwrap();
        assert_eq!(visible(&db), vec![leases[1].generation.id.clone()]);
        assert!(
            db.active_index_generations_in_scope(&scoped)
                .unwrap()
                .is_empty()
        );
        raw.execute(restore, [&first.id]).unwrap();
        assert_eq!(visible(&db), expected);
    }
    for column in ["content_revision", "parser_revision"] {
        let original: String = raw
            .query_row(
                &format!("SELECT {column} FROM index_generations WHERE id=?1"),
                [&leases[0].generation.id],
                |r| r.get(0),
            )
            .unwrap();
        raw.execute(
            &format!("UPDATE index_generations SET {column}='obsolete' WHERE id=?1"),
            [&leases[0].generation.id],
        )
        .unwrap();
        assert_eq!(visible(&db), vec![leases[1].generation.id.clone()]);
        raw.execute(
            &format!("UPDATE index_generations SET {column}=?2 WHERE id=?1"),
            rusqlite::params![leases[0].generation.id, original],
        )
        .unwrap();
    }
    raw.execute(
        "UPDATE index_generations SET status='failed' WHERE id=?1",
        [&leases[0].generation.id],
    )
    .unwrap();
    assert_eq!(visible(&db), vec![leases[1].generation.id.clone()]);
    raw.execute(
        "UPDATE index_generations SET status='ready' WHERE id=?1",
        [&leases[0].generation.id],
    )
    .unwrap();
    assert_eq!(visible(&db), expected);
    db.register_embedding_profile(&id(), spec("changed"))
        .unwrap();
    assert!(visible(&db).is_empty());
}

#[test]
fn source_manifest_requires_current_parse_and_uses_source_scope() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let import_lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let bytes = "资料的真实内容".as_bytes();
    let version = db
        .capture_source_original(
            &job.job_id,
            &import_lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &import_lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap(),
    )
    .unwrap();
    let profile = db
        .register_embedding_profile(&id(), spec("source-manifest"))
        .unwrap()
        .profile;
    let input = db.index_input(&version.source_id).unwrap();
    let lease = db
        .begin_index_generation(&id(), plan(&input, &profile))
        .unwrap();
    db.publish_index_generation(publication(&lease)).unwrap();
    let scope = SearchScope {
        kinds: vec![EvidenceKind::Source],
        source_format: Some(SourceFormat::Text),
        ..Default::default()
    };
    assert_eq!(
        db.active_index_generations_in_scope(&scope).unwrap()[0].id,
        lease.generation.id
    );
    assert!(
        db.active_index_generations_in_scope(&SearchScope {
            kinds: vec![EvidenceKind::Note],
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE source_parses SET parser_version='changed-parser' WHERE source_version_id=?1",
        [&version.id],
    )
    .unwrap();
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    assert!(
        !db.search_with_vectors(&query(SearchMode::Semantic), vec![])
            .unwrap()
            .semantic_available
    );
    raw.execute(
        "UPDATE source_parses SET parser_version=?2 WHERE source_version_id=?1",
        rusqlite::params![version.id, input.parser_revision],
    )
    .unwrap();
    assert_eq!(
        db.active_index_generation_ids().unwrap(),
        vec![lease.generation.id]
    );
    raw.execute(
        "DELETE FROM source_active_parses WHERE source_version_id=?1",
        [&version.id],
    )
    .unwrap();
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    assert!(
        !db.search_with_vectors(&query(SearchMode::Semantic), vec![])
            .unwrap()
            .semantic_available
    );
}

#[test]
fn remote_profile_identity_separates_endpoint_and_keeps_legacy_json_compatible() {
    let legacy = spec("legacy-json");
    let json = serde_json::to_value(&legacy).unwrap();
    assert!(json.get("endpoint").is_none());
    let read: EmbeddingProfileSpec = serde_json::from_value(json.clone()).unwrap();
    assert!(read.endpoint.is_none());
    assert_eq!(serde_json::to_value(read).unwrap(), json);
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut remote = legacy;
    remote.endpoint = Some("https://example.com/ai/v1".into());
    remote.model = "@cf/baai/bge-m3".into();
    remote.dimensions = 1024;
    let first = db
        .register_embedding_profile(&id(), remote.clone())
        .unwrap()
        .profile;
    remote.endpoint = Some("https://other.example/ai/v1".into());
    let second = db
        .register_embedding_profile(&id(), remote)
        .unwrap()
        .profile;
    assert_ne!(first.id, second.id);
    drop(db);
    let db = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        db.active_embedding_profile()
            .unwrap()
            .unwrap()
            .spec
            .endpoint
            .as_deref(),
        Some("https://other.example/ai/v1")
    );
}

#[test]
fn statistics_report_the_active_profile_index_totals() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("中文知识", "中文知识需要真实证据").unwrap();
    let empty = db.workspace_statistics().unwrap();
    assert_eq!(empty.indexed_objects, 0);
    assert_eq!(empty.index_chunks, 0);
    let profile = db
        .register_embedding_profile(&id(), spec("totals-first"))
        .unwrap()
        .profile;
    let index_plan = plan(&db.index_input(&note.id).unwrap(), &profile);
    let chunks = index_plan.chunks.len() as u64;
    assert!(chunks > 0);
    let lease = db.begin_index_generation(&id(), index_plan).unwrap();
    db.publish_index_generation(publication(&lease)).unwrap();
    let stats = db.workspace_statistics().unwrap();
    assert_eq!(stats.indexed_objects, 1);
    assert_eq!(stats.index_chunks, chunks);
    // Registering another profile changes the active index, so the totals must
    // follow the active profile instead of counting historical generations.
    let next = db
        .register_embedding_profile(&id(), spec("totals-second"))
        .unwrap()
        .profile;
    assert_ne!(next.id, profile.id);
    let stats = db.workspace_statistics().unwrap();
    assert_eq!(stats.indexed_objects, 0);
    assert_eq!(stats.index_chunks, 0);
}
