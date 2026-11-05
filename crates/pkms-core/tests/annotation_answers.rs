//! Controlled answer text tests persistence/authorization, not model quality.
use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn agent_scope_preflight_preserves_annotation_only_authorization() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let (mut search, candidate) = fixture(&mut db);
    search.query = "not-the-original-query".into();
    let accepted = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                search,
                agent: Some(AgentOptions::default()),
                model: Some(GenerationProfile {
                    channel_id: None,
                    capabilities: None,
                    provider: "openai_chat".into(),
                    endpoint: "https://example.test".into(),
                    model: "synthetic".into(),
                    model_digest: String::new(),
                    config_revision: 1,
                    temperature: 0.0,
                    context_tokens: 4096,
                    chat_token_limit: None,
                }),
            },
        )
        .unwrap();
    let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    assert!(
        db.agent_scope_has_evidence(&lease.answer.id, &lease.lease_token)
            .unwrap()
    );
    let annotation = candidate.evidence.annotation.unwrap().annotation_id;
    db.set_object_search_enabled(
        &id(),
        SetObjectFlag {
            object_id: annotation.clone(),
            base_row_version: db.object_metadata(&annotation).unwrap().row_version,
            value: false,
        },
    )
    .unwrap();
    assert!(
        !db.agent_scope_has_evidence(&lease.answer.id, &lease.lease_token)
            .unwrap()
    );
}
fn fixture(db: &mut Library) -> (SearchQuery, AnnotationVectorCandidate) {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "原始资料".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let text = "授权证据，外部正文";
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    let block = db
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id,
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: 4,
                    quote: "授权证据".into(),
                    prefix: String::new(),
                    suffix: "，外部正文".into(),
                },
                comment: "人工批注".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let collection = db.create_collection("研究主题").unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![annotation.id],
        },
    )
    .unwrap();
    let profile = db
        .register_embedding_profile(
            &id(),
            EmbeddingProfileSpec {
                endpoint: None,
                model: "annotation-answer-fixture".into(),
                revision: "one".into(),
                dimensions: 3,
                normalized: true,
                query_prefix: "query: ".into(),
                passage_prefix: "passage: ".into(),
                tokenizer: "fixture".into(),
                chunker: "exact-annotation".into(),
                max_tokens: 512,
                overlap_tokens: 0,
                provider: None,
                output_dimensions: None,
            },
        )
        .unwrap()
        .profile;
    let query = SearchQuery {
        query: "授权证据".into(),
        mode: SearchMode::Semantic,
        scope: SearchScope {
            collection_id: Some(collection.id),
            ..Default::default()
        },
        limit: 8,
        minimum_cosine: Some(0.5),
    };
    let evidence = db.annotation_search_inputs(&query.scope).unwrap().remove(0);
    (
        query,
        AnnotationVectorCandidate {
            evidence,
            profile_id: profile.id,
            cosine_similarity: 0.9,
        },
    )
}
fn prepare(
    db: &mut Library,
    query: SearchQuery,
    candidate: AnnotationVectorCandidate,
) -> AnswerLease {
    let accepted = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: query,
                model: Some(GenerationProfile {
                    channel_id: None,
                    capabilities: None,
                    chat_token_limit: None,
                    provider: "ollama".into(),
                    endpoint: "http://127.0.0.1:11434".into(),
                    model: "controlled-fixture".into(),
                    model_digest: "fixture-digest".into(),
                    config_revision: 1,
                    temperature: 0.0,
                    context_tokens: 4096,
                }),
            },
        )
        .unwrap();
    let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    let answer = db
        .prepare_answer_evidence_with_annotations(
            &accepted.answer.id,
            &lease.lease_token,
            vec![],
            vec![candidate],
        )
        .unwrap();
    assert_eq!(answer.status, AnswerStatus::Running);
    assert_eq!(answer.evidence.len(), 1);
    assert!(answer.evidence_current);
    assert_eq!(answer.evidence[0].evidence.text, "授权证据");
    AnswerLease { answer, ..lease }
}
fn remove(db: &mut Library, query: &SearchQuery, candidate: &AnnotationVectorCandidate) {
    let collection = query.scope.collection_id.as_ref().unwrap();
    db.collection_remove_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.clone(),
            base_row_version: db.collection_get(collection).unwrap().metadata.row_version,
            member_ids: vec![
                candidate
                    .evidence
                    .annotation
                    .as_ref()
                    .unwrap()
                    .annotation_id
                    .clone(),
            ],
        },
    )
    .unwrap();
}
fn complete(db: &mut Library, lease: AnswerLease) -> AnswerRecord {
    db.complete_answer(CompleteAnswer {
        answer_id: lease.answer.id,
        lease_token: lease.lease_token,
        text: "授权内容 [E1]".into(),
        cited_ids: vec!["E1".into()],
    })
    .unwrap()
}

#[test]
fn frozen_annotation_evidence_loses_live_authorization_when_removed() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let (query, candidate) = fixture(&mut db);
    let lease = prepare(&mut db, query.clone(), candidate.clone());
    assert!(
        db.prepare_answer_evidence_with_annotations(
            &lease.answer.id,
            &lease.lease_token,
            vec![],
            vec![candidate.clone()]
        )
        .is_err()
    );
    remove(&mut db, &query, &candidate);
    let result = complete(&mut db, lease);
    assert_eq!(result.status, AnswerStatus::EvidenceChanged);
    assert!(!result.evidence_current);
    assert_eq!(result.evidence[0].evidence.text, "授权证据");
}

#[test]
fn annotation_history_survives_membership_and_lifecycle_changes_but_not_forged_quote() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let (query, candidate) = fixture(&mut db);
    let lease = prepare(&mut db, query.clone(), candidate.clone());
    let completed = complete(&mut db, lease);
    assert_eq!(completed.status, AnswerStatus::Completed);
    remove(&mut db, &query, &candidate);
    let annotation = candidate.evidence.annotation.as_ref().unwrap();
    db.update_annotation(
        &id(),
        UpdateAnnotation {
            id: annotation.annotation_id.clone(),
            base_row_version: annotation.annotation_revision,
            comment: "后来的评论".into(),
        },
    )
    .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute(
        "UPDATE objects SET trashed=1,row_version=row_version+1 WHERE id IN (?1,?2)",
        rusqlite::params![
            candidate.evidence.target.object_id,
            annotation.annotation_id
        ],
    )
    .unwrap();
    let package = dir.path().join("annotation-history.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = dir.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    let answer = restored.answer(&completed.id).unwrap();
    assert_eq!(answer.status, AnswerStatus::Completed);
    assert!(!answer.evidence_current);
    assert_eq!(
        serde_json::to_value(&answer.evidence).unwrap(),
        serde_json::to_value(&completed.evidence).unwrap()
    );
    let mut forged = completed.evidence.clone();
    forged[0].evidence.text = "伪造证据".into();
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![completed.id, serde_json::to_string(&forged).unwrap()],
    )
    .unwrap();
    assert!(
        db.create_backup(
            dir.path().join("forged-history.pkmsbackup"),
            &mut |_| Ok(())
        )
        .is_err()
    );
}

#[test]
fn historical_annotation_cannot_be_downgraded_to_ordinary_source_evidence() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let (mut query, candidate) = fixture(&mut db);
    let annotation_id = candidate
        .evidence
        .annotation
        .as_ref()
        .unwrap()
        .annotation_id
        .clone();
    query.scope.object_ids = vec![annotation_id];
    let lease = prepare(&mut db, query, candidate.clone());
    let completed = complete(&mut db, lease);
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    // Even an otherwise correct immutable quote cannot shed its annotation grant.
    let mut forged = completed.evidence.clone();
    forged[0].evidence.annotation = None;
    forged[0].evidence.cosine_similarity = None;
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![completed.id, serde_json::to_string(&forged).unwrap()],
    )
    .unwrap();
    assert!(
        db.create_backup(dir.path().join("missing-grant.pkmsbackup"), &mut |_| Ok(()))
            .is_err()
    );
    // Renaming the evidence ID must not evade the explicit annotation-only scope.
    forged[0].evidence.id = "lexical:forged-source".into();
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![completed.id, serde_json::to_string(&forged).unwrap()],
    )
    .unwrap();
    assert!(
        db.create_backup(dir.path().join("renamed-grant.pkmsbackup"), &mut |_| Ok(()))
            .is_err()
    );
    // A genuine note fact is still unauthorized by the explicit annotation-only scope.
    let note = db.create_note("unrelated", "真实笔记内容").unwrap();
    let note_evidence = db
        .keyword_search(&SearchQuery {
            query: "真实笔记内容".into(),
            mode: SearchMode::Lexical,
            scope: SearchScope {
                object_ids: vec![note.id],
                ..Default::default()
            },
            limit: 1,
            minimum_cosine: None,
        })
        .unwrap()
        .items
        .remove(0);
    forged[0].evidence = note_evidence;
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![completed.id, serde_json::to_string(&forged).unwrap()],
    )
    .unwrap();
    assert!(
        db.create_backup(
            dir.path().join("note-substitution.pkmsbackup"),
            &mut |_| Ok(())
        )
        .is_err()
    );
    // A collection-only query still rejects a retained annotation identity lacking provenance.
    let lease = prepare(&mut db, fixture_query(&completed.request.search), candidate);
    let collection_answer = complete(&mut db, lease);
    let mut forged = collection_answer.evidence.clone();
    forged[0].evidence.annotation = None;
    forged[0].evidence.cosine_similarity = None;
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![
            completed.id,
            serde_json::to_string(&completed.evidence).unwrap()
        ],
    )
    .unwrap();
    sql.execute(
        "UPDATE answers SET evidence_json=?2 WHERE id=?1",
        rusqlite::params![
            collection_answer.id,
            serde_json::to_string(&forged).unwrap()
        ],
    )
    .unwrap();
    assert!(
        db.create_backup(
            dir.path().join("collection-missing-grant.pkmsbackup"),
            &mut |_| Ok(())
        )
        .is_err()
    );
}

fn fixture_query(query: &SearchQuery) -> SearchQuery {
    let mut result = query.clone();
    result.scope.object_ids.clear();
    result
}

#[test]
fn ordinary_history_enforces_static_kind_and_format_but_keeps_unscoped_legacy_answers() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let (query, candidate) = fixture(&mut db);
    let lease = prepare(&mut db, query, candidate.clone());
    let annotation_answer = complete(&mut db, lease);
    db.create_note("plain", "真实笔记内容").unwrap();
    let mut request = annotation_answer.request.clone();
    request.search = SearchQuery {
        query: "真实笔记内容".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 1,
        minimum_cosine: None,
    };
    let accepted = db.accept_answer(&id(), request.clone()).unwrap();
    let lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    db.prepare_answer_evidence(&lease.answer.id, &lease.lease_token, vec![])
        .unwrap();
    let note_answer = complete(&mut db, lease);
    assert_eq!(
        note_answer.evidence[0].evidence.target.kind,
        EvidenceKind::Note
    );
    // This real unscoped note answer retains the pre-annotation backup behavior.
    db.create_backup(dir.path().join("legacy-unscoped.pkmsbackup"), &mut |_| {
        Ok(())
    })
    .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    for (name, scope) in [
        (
            "note-wrong-kind",
            SearchScope {
                kinds: vec![EvidenceKind::Source],
                ..Default::default()
            },
        ),
        (
            "note-with-format",
            SearchScope {
                source_format: Some(SourceFormat::Text),
                ..Default::default()
            },
        ),
    ] {
        request.search.scope = scope;
        sql.execute(
            "UPDATE answers SET request_json=?2 WHERE id=?1",
            rusqlite::params![note_answer.id, serde_json::to_string(&request).unwrap()],
        )
        .unwrap();
        assert!(
            db.create_backup(dir.path().join(format!("{name}.pkmsbackup")), &mut |_| Ok(
                ()
            ))
            .is_err()
        );
    }
    sql.execute(
        "UPDATE answers SET request_json=?2 WHERE id=?1",
        rusqlite::params![
            note_answer.id,
            serde_json::to_string(&note_answer.request).unwrap()
        ],
    )
    .unwrap();
    let source_evidence = db
        .keyword_search(&SearchQuery {
            query: "授权证据".into(),
            mode: SearchMode::Lexical,
            scope: SearchScope {
                object_ids: vec![candidate.evidence.target.object_id],
                ..Default::default()
            },
            limit: 1,
            minimum_cosine: None,
        })
        .unwrap()
        .items
        .remove(0);
    let mut source_request = annotation_answer.request.clone();
    source_request.search.scope = SearchScope {
        source_format: Some(SourceFormat::Pdf),
        ..Default::default()
    };
    let mut evidence = annotation_answer.evidence.clone();
    evidence[0].evidence = source_evidence;
    sql.execute(
        "UPDATE answers SET request_json=?2,evidence_json=?3 WHERE id=?1",
        rusqlite::params![
            annotation_answer.id,
            serde_json::to_string(&source_request).unwrap(),
            serde_json::to_string(&evidence).unwrap()
        ],
    )
    .unwrap();
    assert!(
        db.create_backup(
            dir.path().join("source-wrong-format.pkmsbackup"),
            &mut |_| Ok(())
        )
        .is_err()
    );
}
