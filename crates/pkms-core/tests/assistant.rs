//! Controlled model output tests verify state and persistence, not model quality.
use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request(model: bool) -> StartAnswer {
    StartAnswer {
        max_output_tokens: None,
        citation_count: None,
        agent: None,
        search: SearchQuery {
            query: "evidence".into(),
            mode: SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 8,
            minimum_cosine: None,
        },
        model: model.then(|| GenerationProfile {
            channel_id: None,
            capabilities: None,
            chat_token_limit: None,
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "test-fixture".into(),
            model_digest: "fixture-digest".into(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 4096,
        }),
    }
}
fn running(db: &mut Library) -> AnswerLease {
    let a = db.accept_answer(&id(), request(true)).unwrap();
    let lease = db.claim_answer(&a.answer.id).unwrap().unwrap();
    let prepared = db
        .prepare_answer_evidence(&a.answer.id, &lease.lease_token, vec![])
        .unwrap();
    assert_eq!(prepared.status, AnswerStatus::Running);
    AnswerLease {
        answer: prepared,
        ..lease
    }
}
fn completed(db: &mut Library) -> AnswerRecord {
    let a = running(db);
    db.complete_answer(CompleteAnswer {
        answer_id: a.answer.id,
        lease_token: a.lease_token,
        text: "Controlled claim [E1]".into(),
        cited_ids: vec!["E1".into()],
    })
    .unwrap()
}
#[test]
fn the_refusal_marker_is_a_stable_cross_language_contract() {
    // The desktop refusal copy pins the same literal; changing it would break
    // machine detection of refusals that are already stored in answers.
    assert_eq!(
        INSUFFICIENT_EVIDENCE_MARKER,
        "[[PKMS_INSUFFICIENT_EVIDENCE]]"
    );
}

#[test]
fn no_evidence_and_no_model_never_accept_generated_output() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let mut oversized = request(true);
    oversized.search.query = "x".repeat(4097);
    assert!(db.accept_answer(&id(), oversized).is_err());
    assert!(db.pending_answer_ids(100).unwrap().is_empty());
    let a = db.accept_answer(&id(), request(true)).unwrap();
    let l = db.claim_answer(&a.answer.id).unwrap().unwrap();
    let result = db
        .prepare_answer_evidence(&a.answer.id, &l.lease_token, vec![])
        .unwrap();
    assert_eq!(result.status, AnswerStatus::NoEvidence);
    assert!(
        db.append_answer_delta(
            &a.answer.id,
            &l.lease_token,
            result.last_event_seq + 1,
            "fake"
        )
        .is_err()
    );
    db.create_note("evidence", "evidence text").unwrap();
    let a = db.accept_answer(&id(), request(false)).unwrap();
    let l = db.claim_answer(&a.answer.id).unwrap().unwrap();
    assert_eq!(
        db.prepare_answer_evidence(&a.answer.id, &l.lease_token, vec![])
            .unwrap()
            .status,
        AnswerStatus::ModelUnconfigured
    );
}
#[test]
fn citation_whitelist_delta_replay_and_terminal_cancel_are_enforced() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let a = running(&mut db);
    let seq = a.answer.last_event_seq + 1;
    let one = db
        .append_answer_delta(&a.answer.id, &a.lease_token, seq, "claim ")
        .unwrap();
    let replay = db
        .append_answer_delta(&a.answer.id, &a.lease_token, seq, "claim ")
        .unwrap();
    assert_eq!(one.text, replay.text);
    assert!(
        db.append_answer_delta(&a.answer.id, &a.lease_token, seq, "different")
            .is_err()
    );
    assert!(
        db.prepare_answer_evidence(&a.answer.id, &a.lease_token, vec![])
            .is_err()
    );
    let bad = db
        .complete_answer(CompleteAnswer {
            answer_id: a.answer.id.clone(),
            lease_token: a.lease_token,
            text: "invented [E99]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
    assert_eq!(bad.status, AnswerStatus::InvalidCitation);
    let a = running(&mut db);
    let command = id();
    assert_eq!(
        db.cancel_answer(&command, &a.answer.id)
            .unwrap()
            .answer
            .status,
        AnswerStatus::Canceled
    );
    assert!(
        db.cancel_answer(&command, &a.answer.id)
            .unwrap()
            .receipt
            .replayed
    );
    assert!(
        db.complete_answer(CompleteAnswer {
            answer_id: a.answer.id,
            lease_token: a.lease_token,
            text: "claim [E1]".into(),
            cited_ids: vec!["E1".into()]
        })
        .is_err()
    );
}
#[test]
fn feedback_stays_attached_to_its_question_evidence_and_retrieval_parameters() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("反馈关联", "副本保留七天。").unwrap();
    let mut ask = request(true);
    ask.search.query = "副本保留几天？".into();
    ask.citation_count = Some(5);
    let a = db.accept_answer(&id(), ask).unwrap();
    let lease = db.claim_answer(&a.answer.id).unwrap().unwrap();
    let prepared = db
        .prepare_answer_evidence(&a.answer.id, &lease.lease_token, vec![])
        .unwrap();
    assert_eq!(prepared.evidence.len(), 1);
    db.complete_answer(CompleteAnswer {
        answer_id: a.answer.id.clone(),
        lease_token: lease.lease_token,
        text: "副本保留七天 [E1]".into(),
        cited_ids: vec!["E1".into()],
    })
    .unwrap();
    db.feedback_answer(
        &id(),
        AnswerFeedback {
            answer_id: a.answer.id.clone(),
            rating: -1,
            comment: "片段不全".into(),
            updated_at: None,
        },
    )
    .unwrap();

    // Everything a later retrieval or prompt review needs is reachable from the
    // feedback row through the answer it is attached to.
    let read_back = db.answer(&a.answer.id).unwrap();
    assert_eq!(read_back.request.search.query, "副本保留几天？");
    assert_eq!(read_back.request.citation_count, Some(5));
    assert_eq!(
        read_back.request.model.as_ref().unwrap().model,
        "test-fixture"
    );
    assert_eq!(read_back.cited_ids, vec!["E1".to_string()]);
    assert_eq!(read_back.evidence.len(), 1);
    assert_eq!(read_back.evidence[0].citation_id, "E1");
    assert_eq!(read_back.evidence[0].evidence.target.object_id, note.id);
    assert_eq!(
        read_back.evidence[0].evidence.text, "副本保留七天。",
        "the fragment the feedback refers to is the frozen one"
    );
    let feedback = read_back.feedback.unwrap();
    assert_eq!(feedback.rating, -1);
    assert_eq!(feedback.comment, "片段不全");
    // The change time is read back from the immutable command receipt.
    let saved_at = feedback
        .updated_at
        .clone()
        .expect("the feedback change time comes from its receipt");
    assert!(saved_at.len() >= 20 && saved_at.contains('T'), "{saved_at}");
    // A caller cannot set that timestamp itself.
    assert!(
        db.feedback_answer(
            &id(),
            AnswerFeedback {
                answer_id: a.answer.id.clone(),
                rating: 0,
                comment: String::new(),
                updated_at: Some(saved_at),
            },
        )
        .is_err()
    );

    // The feedback survives a reopen and keeps pointing at the same answer.
    drop(db);
    let db = Library::open(dir.path()).unwrap();
    let reopened = db.answer(&a.answer.id).unwrap();
    assert_eq!(reopened.feedback.unwrap().comment, "片段不全");
    assert_eq!(reopened.cited_ids, vec!["E1".to_string()]);
}

#[test]
fn a_purged_object_keeps_a_historical_answer_that_still_backs_up() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("备份关联", "副本保留七天。").unwrap();
    let mut ask = request(true);
    ask.search.query = "副本保留几天？".into();
    let a = db.accept_answer(&id(), ask).unwrap();
    let lease = db.claim_answer(&a.answer.id).unwrap().unwrap();
    db.prepare_answer_evidence(&a.answer.id, &lease.lease_token, vec![])
        .unwrap();
    db.complete_answer(CompleteAnswer {
        answer_id: a.answer.id.clone(),
        lease_token: lease.lease_token,
        text: "副本保留七天 [E1]".into(),
        cited_ids: vec!["E1".into()],
    })
    .unwrap();
    db.feedback_answer(
        &id(),
        AnswerFeedback {
            answer_id: a.answer.id.clone(),
            rating: 1,
            comment: String::new(),
            updated_at: None,
        },
    )
    .unwrap();

    // The user permanently deletes the note the answer was built from.
    db.set_trashed(&note.id, true).unwrap();
    let snapshot = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::Objects {
                object_ids: vec![note.id.clone()],
            },
        })
        .unwrap();
    db.delete_trash(
        &id(),
        DeleteTrash {
            selection: snapshot.selection.clone(),
            snapshot_token: snapshot.snapshot_token.clone(),
        },
    )
    .unwrap();

    // The answer keeps its frozen evidence and reports it as no longer current,
    // so a full backup must still succeed: the deletion is a legitimate history,
    // not an integrity failure.
    let answer = db.answer(&a.answer.id).unwrap();
    assert_eq!(answer.status, AnswerStatus::Completed);
    assert!(!answer.evidence_current);
    assert_eq!(answer.evidence[0].evidence.text, "副本保留七天。");
    assert_eq!(answer.feedback.unwrap().rating, 1);
    db.create_backup(dir.path().join("backup.zip"), &mut |_| Ok(()))
        .unwrap();
}

#[test]
fn lifecycle_change_invalidates_frozen_evidence() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("evidence", "evidence text").unwrap();
    let a = running(&mut db);
    db.set_trashed(&note.id, true).unwrap();
    let end = db
        .complete_answer(CompleteAnswer {
            answer_id: a.answer.id,
            lease_token: a.lease_token,
            text: "claim [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
    assert_eq!(end.status, AnswerStatus::EvidenceChanged);
    assert!(!end.evidence_current);
}

#[test]
fn answer_receipts_are_immutable_and_feedback_events_survive_reopen() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let command = id();
    let accepted = db.accept_answer(&command, request(true)).unwrap();
    let l = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    let a = db
        .prepare_answer_evidence(&accepted.answer.id, &l.lease_token, vec![])
        .unwrap();
    db.append_answer_delta(&a.id, &l.lease_token, a.last_event_seq + 1, "claim [E1]")
        .unwrap();
    let final_answer = db
        .complete_answer(CompleteAnswer {
            answer_id: a.id.clone(),
            lease_token: l.lease_token,
            text: "claim [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
    let replay = db.accept_answer(&command, request(true)).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.answer.status, AnswerStatus::Queued);
    assert!(db.accept_answer(&command, request(false)).is_err());
    let feedback_id = id();
    let feedback = AnswerFeedback {
        answer_id: a.id.clone(),
        rating: 1,
        comment: "Evidence checked".into(),
        updated_at: None,
    };
    db.feedback_answer(&feedback_id, feedback.clone()).unwrap();
    assert!(
        db.feedback_answer(&feedback_id, feedback)
            .unwrap()
            .receipt
            .replayed
    );
    drop(db);
    let db = Library::open(dir.path()).unwrap();
    assert_eq!(db.answer(&a.id).unwrap().status, AnswerStatus::Completed);
    let restored_feedback = db.answer(&a.id).unwrap().feedback.unwrap();
    assert_eq!(restored_feedback.rating, 1);
    assert_eq!(restored_feedback.comment, "Evidence checked");
    assert_eq!(restored_feedback.answer_id, a.id);
    let first = db.answer_events(&a.id, 0, 2).unwrap();
    let rest = db
        .answer_events(&a.id, first.last().unwrap().seq, 100)
        .unwrap();
    assert_eq!(rest.last().unwrap().seq, final_answer.last_event_seq);
    assert_eq!(rest.last().unwrap().status, AnswerStatus::Completed);
}
fn proposal_request(answer: &AnswerRecord, note: &Note) -> CreateProposal {
    CreateProposal {
        chat_run_id: None,
        external_request_id: None,
        answer_id: Some(answer.id.clone()),
        note_id: Some(note.id.clone()),
        base_revision_id: Some(note.revision_id.clone()),
        title: "AI proposed title".into(),
        document_schema: note.schema_version,
        document: note.document.clone(),
    }
}
#[test]
fn proposal_apply_rolls_back_revision_status_and_receipt_on_database_failure() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let target = db.create_note("target", "original rich document").unwrap();
    let a = completed(&mut db);
    assert_eq!(a.status, AnswerStatus::Completed);
    let p = db
        .create_proposal(&id(), proposal_request(&a, &target))
        .unwrap();
    let command = id();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_proposal BEFORE UPDATE ON proposals WHEN NEW.status='applied' BEGIN SELECT RAISE(ABORT,'controlled failure'); END;").unwrap();
    assert!(db.apply_proposal(&command, &p.proposal.id).is_err());
    assert_eq!(db.note(&target.id).unwrap().revision_id, target.revision_id);
    assert_eq!(
        db.proposal(&p.proposal.id).unwrap().status,
        ProposalStatus::Pending
    );
    let receipts: u32 = sql
        .query_row(
            "SELECT count(*) FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 0);
    sql.execute_batch("DROP TRIGGER fail_proposal;").unwrap();
    let applied = db.apply_proposal(&command, &p.proposal.id).unwrap();
    assert_eq!(applied.note.document, target.document);
    assert_eq!(applied.note.author_kind, "ai_proposal");
    assert_eq!(
        db.proposal(&p.proposal.id).unwrap().status,
        ProposalStatus::Applied
    );
    assert!(
        db.apply_proposal(&command, &p.proposal.id)
            .unwrap()
            .receipt
            .replayed
    );
}
#[test]
fn proposal_conflict_and_expired_generation_do_not_write() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let target = db.create_note("target", "original").unwrap();
    let a = completed(&mut db);
    let p = db
        .create_proposal(&id(), proposal_request(&a, &target))
        .unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: target.id.clone(),
            base_revision_id: target.revision_id,
            title: "human edit".into(),
            document_schema: target.schema_version,
            document: target.document,
        },
    )
    .unwrap();
    assert!(matches!(
        db.apply_proposal(&id(), &p.proposal.id),
        Err(Error::RevisionConflict { .. })
    ));
    assert_eq!(
        db.proposal(&p.proposal.id).unwrap().status,
        ProposalStatus::Pending
    );
    let a = running(&mut db);
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute(
        "UPDATE answers SET lease_expires_at='2000-01-01T00:00:00+00:00' WHERE id=?1",
        [&a.answer.id],
    )
    .unwrap();
    assert_eq!(db.recover_expired_answers().unwrap(), 1);
    let failed = db.answer(&a.answer.id).unwrap();
    assert_eq!(failed.status, AnswerStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("interrupted"));
    assert!(db.pending_answer_ids(100).unwrap().is_empty());
}

#[test]
fn backup_restores_attachment_only_referenced_by_pending_proposal() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let target = db.create_note("target", "original").unwrap();
    let a = completed(&mut db);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 3)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let bytes = bytes.into_inner();
    let captured = db.import_attachment(&id(), "proposal.png", &bytes).unwrap();
    let mut request = proposal_request(&a, &target);
    request.document=serde_json::from_value(serde_json::json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"Proposed illustration"}]},{"type":"image","attrs":{"id":id(),"blob_hash":captured.blob.hash,"original_name":"proposal.png","alt":"Pending illustration"}}]})).unwrap();
    let p = db.create_proposal(&id(), request).unwrap();
    let package = root.path().join("proposal.pkmsbackup");
    let report = db.create_backup(&package, &mut |_| Ok(())).unwrap();
    assert_eq!(report.verified_blobs, 1);
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(&destination).unwrap();
    assert_eq!(
        restored.proposal(&p.proposal.id).unwrap().status,
        ProposalStatus::Pending
    );
    let applied = restored.apply_proposal(&id(), &p.proposal.id).unwrap();
    assert_eq!(
        restored
            .attachment(&applied.note.revision_id, &captured.blob.hash)
            .unwrap()
            .1,
        bytes
    );
}

#[test]
fn exclusive_answer_recovery_stops_only_running_and_heartbeat_cannot_revive_it() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let _owner = WorkspaceLease::acquire(dir.path()).unwrap();
    let queued = db.accept_answer(&id(), request(true)).unwrap().answer;
    let active = db.accept_answer(&id(), request(true)).unwrap().answer;
    let l = db.claim_answer(&active.id).unwrap().unwrap();
    db.heartbeat_answer(&active.id, &l.lease_token).unwrap();
    db.check_answer_lease(&active.id, &l.lease_token).unwrap();
    assert_eq!(db.recover_answers_exclusive().unwrap(), 1);
    assert_eq!(db.answer(&queued.id).unwrap().status, AnswerStatus::Queued);
    let failed = db.answer(&active.id).unwrap();
    assert_eq!(failed.status, AnswerStatus::Failed);
    assert_eq!(
        failed.error_code.as_deref(),
        Some("interrupted_unknown_outcome")
    );
    assert!(db.heartbeat_answer(&active.id, &l.lease_token).is_err());
    assert!(db.check_answer_lease(&active.id, &l.lease_token).is_err());
    assert_eq!(
        db.answer_events(&active.id, 0, 100)
            .unwrap()
            .last()
            .unwrap()
            .status,
        AnswerStatus::Failed
    );
}

#[test]
fn new_note_proposal_is_atomic_and_repeated_apply_never_creates_another_note() {
    let root = tempdir().unwrap();
    let path = root.path().join("source");
    let mut db = Library::open(&path).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let answer = completed(&mut db);
    let request=CreateProposal{chat_run_id:None,external_request_id:None,answer_id:Some(answer.id.clone()),note_id:None,base_revision_id:None,title:"Saved AI answer".into(),document_schema:2,document:serde_json::from_value(serde_json::json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"Reviewed answer [E1]"}]}]})).unwrap()};
    let proposal = db.create_proposal(&id(), request).unwrap().proposal;
    assert_eq!(db.list_notes().unwrap().len(), 1);
    assert!(proposal.applied_note_id.is_none());
    let sql = rusqlite::Connection::open(path.join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_new_proposal BEFORE UPDATE ON proposals WHEN NEW.status='applied' BEGIN SELECT RAISE(ABORT,'controlled failure'); END;").unwrap();
    let command = id();
    assert!(db.apply_proposal(&command, &proposal.id).is_err());
    assert_eq!(db.list_notes().unwrap().len(), 1);
    assert_eq!(
        db.proposal(&proposal.id).unwrap().status,
        ProposalStatus::Pending
    );
    let receipts: u32 = sql
        .query_row(
            "SELECT count(*) FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 0);
    sql.execute_batch("DROP TRIGGER fail_new_proposal;")
        .unwrap();
    let applied = db.apply_proposal(&command, &proposal.id).unwrap();
    assert_eq!(applied.note.author_kind, "ai_proposal");
    assert_eq!(applied.note.parent_revision_id, None);
    assert_eq!(db.list_notes().unwrap().len(), 2);
    let saved = db.proposal(&proposal.id).unwrap();
    assert_eq!(
        saved.applied_note_id.as_deref(),
        Some(applied.note.id.as_str())
    );
    assert_eq!(saved.request.answer_id, Some(answer.id));
    assert!(saved.request.note_id.is_none());
    assert!(
        db.apply_proposal(&command, &proposal.id)
            .unwrap()
            .receipt
            .replayed
    );
    let another = db.apply_proposal(&id(), &proposal.id).unwrap();
    assert_eq!(another.note.id, applied.note.id);
    assert_eq!(another.note.revision_id, applied.note.revision_id);
    assert_eq!(db.list_notes().unwrap().len(), 2);
    let package = root.path().join("new-note.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(destination).unwrap();
    assert_eq!(
        restored.proposal(&proposal.id).unwrap().applied_note_id,
        Some(applied.note.id.clone())
    );
    assert_eq!(
        restored
            .apply_proposal(&id(), &proposal.id)
            .unwrap()
            .note
            .id,
        applied.note.id
    );
    assert_eq!(restored.list_notes().unwrap().len(), 2);
}

#[test]
fn exact_insufficient_marker_keeps_evidence_stream_and_authenticated_replay_after_restore() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let note = db.create_note("evidence", "evidence text").unwrap();
    let l = running(&mut db);
    let split = INSUFFICIENT_EVIDENCE_MARKER.len() / 2;
    let partial = db
        .append_answer_delta(
            &l.answer.id,
            &l.lease_token,
            l.answer.last_event_seq + 1,
            &INSUFFICIENT_EVIDENCE_MARKER[..split],
        )
        .unwrap();
    let streamed = db
        .append_answer_delta(
            &l.answer.id,
            &l.lease_token,
            partial.last_event_seq + 1,
            &INSUFFICIENT_EVIDENCE_MARKER[split..],
        )
        .unwrap();
    let result = db
        .complete_answer_insufficient_evidence(
            &l.answer.id,
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER,
        )
        .unwrap();
    assert_eq!(result.status, AnswerStatus::NoEvidence);
    assert_eq!(result.error_code.as_deref(), Some("insufficient_evidence"));
    assert_eq!(result.text, INSUFFICIENT_EVIDENCE_MARKER);
    assert!(!result.evidence.is_empty());
    assert!(result.cited_ids.is_empty());
    assert!(
        db.create_proposal(
            &id(),
            CreateProposal {
                chat_run_id: None,
                external_request_id: None,
                answer_id: Some(result.id.clone()),
                note_id: None,
                base_revision_id: None,
                title: "Must not save refusal".into(),
                document_schema: note.schema_version,
                document: note.document.clone()
            }
        )
        .is_err()
    );
    assert_eq!(result.last_event_seq, streamed.last_event_seq + 1);
    let again = db
        .complete_answer_insufficient_evidence(
            &l.answer.id,
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER,
        )
        .unwrap();
    assert_eq!(again.last_event_seq, result.last_event_seq);
    assert!(
        db.complete_answer_insufficient_evidence(&l.answer.id, &id(), INSUFFICIENT_EVIDENCE_MARKER)
            .is_err()
    );
    assert!(
        db.complete_answer_insufficient_evidence(
            &id(),
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    assert!(db.check_answer_lease(&l.answer.id, &l.lease_token).is_err());
    let no_deltas = running(&mut db);
    assert_eq!(
        db.complete_answer_insufficient_evidence(
            &no_deltas.answer.id,
            &no_deltas.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .unwrap()
        .status,
        AnswerStatus::NoEvidence
    );
    db.set_trashed(&note.id, true).unwrap();
    let package = root.path().join("insufficient.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(destination).unwrap();
    let stored = restored.answer(&l.answer.id).unwrap();
    assert_eq!(stored.text, INSUFFICIENT_EVIDENCE_MARKER);
    assert!(!stored.evidence_current);
    assert_eq!(
        restored
            .complete_answer_insufficient_evidence(
                &l.answer.id,
                &l.lease_token,
                INSUFFICIENT_EVIDENCE_MARKER
            )
            .unwrap()
            .last_event_seq,
        result.last_event_seq
    );
    let sql = rusqlite::Connection::open(root.path().join("source/library.sqlite3")).unwrap();
    sql.execute(
        "UPDATE answers SET text=?2 WHERE id=?1",
        rusqlite::params![l.answer.id, format!("{INSUFFICIENT_EVIDENCE_MARKER}\n")],
    )
    .unwrap();
    assert!(
        db.create_backup(root.path().join("tampered-marker.pkmsbackup"), &mut |_| Ok(
            ()
        ))
        .is_err()
    );
}
#[test]
fn insufficient_marker_is_exact_and_cannot_overwrite_other_or_partial_streams() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let l = running(&mut db);
    for text in [
        format!(" {INSUFFICIENT_EVIDENCE_MARKER}"),
        format!("{INSUFFICIENT_EVIDENCE_MARKER}\n"),
        "insufficient evidence".into(),
    ] {
        assert!(
            db.complete_answer_insufficient_evidence(&l.answer.id, &l.lease_token, &text)
                .is_err()
        );
    }
    assert_eq!(
        db.answer(&l.answer.id).unwrap().status,
        AnswerStatus::Running
    );
    db.append_answer_delta(
        &l.answer.id,
        &l.lease_token,
        l.answer.last_event_seq + 1,
        "Other streamed prose",
    )
    .unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &l.answer.id,
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    assert_eq!(
        db.answer(&l.answer.id).unwrap().text,
        "Other streamed prose"
    );
    let partial = running(&mut db);
    db.append_answer_delta(
        &partial.answer.id,
        &partial.lease_token,
        partial.answer.last_event_seq + 1,
        &INSUFFICIENT_EVIDENCE_MARKER[..10],
    )
    .unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &partial.answer.id,
            &partial.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let accepted = db.accept_answer(&id(), request(true)).unwrap().answer;
    let unprepared = db.claim_answer(&accepted.id).unwrap().unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &accepted.id,
            &unprepared.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let normal = running(&mut db);
    assert_eq!(
        db.complete_answer(CompleteAnswer {
            answer_id: normal.answer.id,
            lease_token: normal.lease_token,
            text: INSUFFICIENT_EVIDENCE_MARKER.into(),
            cited_ids: vec![]
        })
        .unwrap()
        .status,
        AnswerStatus::InvalidCitation
    );
}
#[test]
fn insufficient_completion_cannot_reverse_cancel_failure_or_changed_evidence() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let note = db.create_note("evidence", "evidence text").unwrap();
    let canceled = running(&mut db);
    db.cancel_answer(&id(), &canceled.answer.id).unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &canceled.answer.id,
            &canceled.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let failed = running(&mut db);
    db.fail_answer(
        &failed.answer.id,
        &failed.lease_token,
        AnswerStatus::Failed,
        "interrupted",
    )
    .unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &failed.answer.id,
            &failed.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let changed = running(&mut db);
    db.set_trashed(&note.id, true).unwrap();
    assert_eq!(
        db.complete_answer_insufficient_evidence(
            &changed.answer.id,
            &changed.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .unwrap()
        .status,
        AnswerStatus::EvidenceChanged
    );
    assert!(
        db.complete_answer_insufficient_evidence(
            &changed.answer.id,
            &changed.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    for token in [
        &canceled.lease_token,
        &failed.lease_token,
        &changed.lease_token,
    ] {
        let count: u32 = sql
            .query_row(
                "SELECT count(*) FROM command_receipts WHERE command_id=?1",
                [token],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
#[test]
fn insufficient_receipt_failure_rolls_back_terminal_status_event_and_lease_cleanup() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    db.create_note("evidence", "evidence text").unwrap();
    let l = running(&mut db);
    let streamed = db
        .append_answer_delta(
            &l.answer.id,
            &l.lease_token,
            l.answer.last_event_seq + 1,
            INSUFFICIENT_EVIDENCE_MARKER,
        )
        .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_insufficient BEFORE INSERT ON command_receipts WHEN NEW.command_kind='answers.insufficient_evidence' BEGIN SELECT RAISE(ABORT,'controlled'); END;").unwrap();
    assert!(
        db.complete_answer_insufficient_evidence(
            &l.answer.id,
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .is_err()
    );
    let current = db.answer(&l.answer.id).unwrap();
    assert_eq!(current.status, AnswerStatus::Running);
    assert_eq!(current.last_event_seq, streamed.last_event_seq);
    db.check_answer_lease(&l.answer.id, &l.lease_token).unwrap();
    sql.execute_batch("DROP TRIGGER fail_insufficient;")
        .unwrap();
    assert_eq!(
        db.complete_answer_insufficient_evidence(
            &l.answer.id,
            &l.lease_token,
            INSUFFICIENT_EVIDENCE_MARKER
        )
        .unwrap()
        .status,
        AnswerStatus::NoEvidence
    );
}
