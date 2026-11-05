use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn setup(db: &mut Library) -> ChatConversation {
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(GenerationProfile {
                channel_id: None,
                capabilities: None,
                provider: "openai_chat".into(),
                endpoint: "https://example.test".into(),
                model: "synthetic".into(),
                model_digest: String::new(),
                config_revision: 1,
                temperature: 0.0,
                context_tokens: 32768,
                chat_token_limit: None,
            }),
        },
    )
    .unwrap();
    db.create_chat_conversation(
        &id(),
        CreateConversation {
            title: "Free conversation".into(),
        },
    )
    .unwrap()
    .conversation
}
fn request(c: &ChatConversation) -> StartChatRun {
    StartChatRun {
        conversation_id: c.id.clone(),
        base_row_version: c.row_version,
        text: "Hello\nPlease question my assumptions.".into(),
        purpose: ChatPurpose::Review,
        materials: vec![],
        scope: SearchScope::default(),
        use_knowledge_tools: false,
        options: None,
    }
}
fn complete(db: &mut Library, c: &ChatConversation) -> ChatRun {
    let a = db.accept_chat_run_from_settings(&id(), request(c)).unwrap();
    let l = db.claim_chat_run(&a.run.id).unwrap().unwrap();
    let r = db
        .append_chat_delta(
            &l.run.id,
            &l.lease_token,
            l.run.last_event_seq + 1,
            "Original **model** output\nWithout required citations.",
        )
        .unwrap();
    db.finish_chat_run(
        &r.id,
        &l.lease_token,
        ChatRunStatus::Completed,
        None,
        ChatUsage {
            input_tokens: Some(10),
            output_tokens: Some(20),
        },
        Some("stop"),
    )
    .unwrap()
}
#[test]
fn free_text_multiturn_and_stream_replay_preserve_exact_content() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let command = id();
    let r = db
        .accept_chat_run_from_settings(&command, request(&c))
        .unwrap();
    assert_eq!(r.run.request.options.as_ref().unwrap().max_model_calls, 8);
    assert!(
        db.accept_chat_run_from_settings(&command, request(&c))
            .unwrap()
            .receipt
            .replayed
    );
    assert!(
        db.accept_chat_run_from_settings(&id(), request(&r.conversation))
            .is_err()
    );
    let lease = db.claim_chat_run(&r.run.id).unwrap().unwrap();
    let seq = lease.run.last_event_seq + 1;
    let first = db
        .append_chat_delta(&r.run.id, &lease.lease_token, seq, "你好\n")
        .unwrap();
    assert_eq!(
        db.append_chat_delta(&r.run.id, &lease.lease_token, seq, "你好\n")
            .unwrap()
            .last_event_seq,
        first.last_event_seq
    );
    assert!(
        db.append_chat_delta(&r.run.id, &lease.lease_token, seq, "changed")
            .is_err()
    );
    db.append_chat_delta(&r.run.id, &lease.lease_token, seq + 1, "**世界**")
        .unwrap();
    db.finish_chat_run(
        &r.run.id,
        &lease.lease_token,
        ChatRunStatus::Completed,
        None,
        ChatUsage::default(),
        Some("stop"),
    )
    .unwrap();
    let messages = db.chat_messages(&c.id, 0, 100).unwrap();
    assert_eq!(messages[1].content, "你好\n**世界**");
    let c = db.chat_conversation(&c.id).unwrap();
    complete(&mut db, &c);
    assert_eq!(db.chat_messages(&c.id, 2, 100).unwrap().len(), 2);
}
#[test]
fn cancel_and_recovery_keep_partial_and_reject_late_output() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let a = db
        .accept_chat_run_from_settings(&id(), request(&c))
        .unwrap();
    let l = db.claim_chat_run(&a.run.id).unwrap().unwrap();
    let r = db
        .append_chat_delta(
            &l.run.id,
            &l.lease_token,
            l.run.last_event_seq + 1,
            "partial",
        )
        .unwrap();
    db.cancel_chat_run(&id(), &r.id).unwrap();
    assert!(
        db.append_chat_delta(&r.id, &l.lease_token, r.last_event_seq + 1, "late")
            .is_err()
    );
    assert_eq!(
        db.chat_messages(&c.id, 0, 100).unwrap()[1].content,
        "partial"
    );
    let c = db.chat_conversation(&c.id).unwrap();
    let a = db
        .accept_chat_run_from_settings(&id(), request(&c))
        .unwrap();
    let l = db.claim_chat_run(&a.run.id).unwrap().unwrap();
    db.append_chat_delta(
        &l.run.id,
        &l.lease_token,
        l.run.last_event_seq + 1,
        "interrupted partial",
    )
    .unwrap();
    assert_eq!(db.recover_chat_runs_exclusive().unwrap(), 1);
    assert_eq!(
        db.chat_run(&l.run.id).unwrap().status,
        ChatRunStatus::Interrupted
    );
    assert!(db.pending_chat_run_ids(100).unwrap().is_empty());
}
#[test]
fn free_chat_proposal_and_backup_restore_keep_origins_and_usage() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let r = complete(&mut db, &c);
    let p = db
        .create_proposal(
            &id(),
            CreateProposal {
                answer_id: None,
                chat_run_id: Some(r.id.clone()),
                external_request_id: None,
                note_id: None,
                base_revision_id: None,
                title: "Model draft".into(),
                document_schema: 2,
                document: NoteDocument::from_text(id(), "Draft without forced citations"),
            },
        )
        .unwrap();
    db.apply_proposal(&id(), &p.proposal.id).unwrap();
    let package = dir.path().join("chat.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let dest = dir.path().join("restore");
    prepare_restore(&package, &dest, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(dest).unwrap();
    assert_eq!(
        restored.chat_run(&r.id).unwrap().usage.output_tokens,
        Some(20)
    );
    assert_eq!(
        restored.chat_messages(&c.id, 0, 100).unwrap()[1].content,
        "Original **model** output\nWithout required citations."
    );
    assert_eq!(
        restored
            .proposal(&p.proposal.id)
            .unwrap()
            .request
            .chat_run_id,
        Some(r.id)
    );
}
#[test]
fn bounds_unconfigured_and_legacy_proposal_encoding() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "No model".into(),
            },
        )
        .unwrap()
        .conversation;
    let a = db
        .accept_chat_run_from_settings(&id(), request(&c))
        .unwrap();
    assert_eq!(a.run.status, ChatRunStatus::ModelUnconfigured);
    db.create_backup(dir.path().join("none.zip"), &mut |_| Ok(()))
        .unwrap();
    let bad = ChatRunOptions {
        max_output_tokens: 32769,
        ..Default::default()
    };
    assert!(validate_chat_run_options(&bad, None).is_err());
    let old = serde_json::json!({"answer_id":id(),"note_id":null,"base_revision_id":null,"title":"x","document_schema":2,"document":NoteDocument::from_text(id(),"x")});
    let parsed: CreateProposal = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
}
#[test]
fn truncated_output_is_kept_but_cannot_create_proposal() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let a = db
        .accept_chat_run_from_settings(&id(), request(&c))
        .unwrap();
    let l = db.claim_chat_run(&a.run.id).unwrap().unwrap();
    db.append_chat_delta(
        &l.run.id,
        &l.lease_token,
        l.run.last_event_seq + 1,
        "unfinished",
    )
    .unwrap();
    db.finish_chat_run(
        &l.run.id,
        &l.lease_token,
        ChatRunStatus::Truncated,
        None,
        ChatUsage::default(),
        Some("length"),
    )
    .unwrap();
    assert!(
        db.create_proposal(
            &id(),
            CreateProposal {
                answer_id: None,
                chat_run_id: Some(l.run.id),
                external_request_id: None,
                note_id: None,
                base_revision_id: None,
                title: "x".into(),
                document_schema: 2,
                document: NoteDocument::from_text(id(), "x")
            }
        )
        .is_err()
    );
    db.create_backup(dir.path().join("truncated.zip"), &mut |_| Ok(()))
        .unwrap();
}

#[test]
fn backup_rejects_transcript_or_event_tampering() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let r = complete(&mut db, &c);
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE chat_messages SET content='forged' WHERE id=?1",
        [&r.assistant_message_id],
    )
    .unwrap();
    assert!(
        db.create_backup(dir.path().join("forged.zip"), &mut |_| Ok(()))
            .is_err()
    );
}
#[test]
fn selected_materials_are_snapshots_not_required_truth_and_cannot_be_forged() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let c = setup(&mut db);
    let note = db
        .create_note("Possibly wrong", "The moon is made of cheese.")
        .unwrap();
    let query = SearchQuery {
        query: "cheese".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 8,
        minimum_cosine: None,
    };
    let evidence = db.keyword_search(&query).unwrap().items.remove(0);
    let mut req = request(&c);
    req.materials = vec![evidence.clone()];
    let mut bad = req.clone();
    bad.materials[0].text = "forged text".into();
    assert!(db.accept_chat_run_from_settings(&id(), bad).is_err());
    let a = db.accept_chat_run_from_settings(&id(), req).unwrap();
    let l = db.claim_chat_run(&a.run.id).unwrap().unwrap();
    db.append_chat_delta(
        &l.run.id,
        &l.lease_token,
        l.run.last_event_seq + 1,
        "This source is mistaken; the Moon is rock.",
    )
    .unwrap();
    db.finish_chat_run(
        &l.run.id,
        &l.lease_token,
        ChatRunStatus::Completed,
        None,
        ChatUsage::default(),
        Some("stop"),
    )
    .unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: note.id,
            base_revision_id: note.revision_id,
            title: "Corrected by human".into(),
            document_schema: note.schema_version,
            document: NoteDocument::from_text(id(), "The Moon is rock."),
        },
    )
    .unwrap();
    db.create_backup(dir.path().join("historical-material.zip"), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(
        db.chat_run(&l.run.id).unwrap().request.materials[0].text,
        evidence.text
    );
}

#[test]
fn restored_chat_never_replays_queued_or_running_requests_and_keeps_partial() {
    let root = tempdir().unwrap();
    let mut source = Library::open(root.path().join("source")).unwrap();
    let running_conversation = setup(&mut source);
    let accepted = source
        .accept_chat_run_from_settings(&id(), request(&running_conversation))
        .unwrap();
    let lease = source.claim_chat_run(&accepted.run.id).unwrap().unwrap();
    let running = source
        .append_chat_delta(
            &lease.run.id,
            &lease.lease_token,
            lease.run.last_event_seq + 1,
            "Partial **retained**\n",
        )
        .unwrap();
    let queued_conversation = source
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "Queued before backup".into(),
            },
        )
        .unwrap()
        .conversation;
    let queued = source
        .accept_chat_run_from_settings(&id(), request(&queued_conversation))
        .unwrap()
        .run;
    let package = root.path().join("chat.zip");
    source.create_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(&restored_path).unwrap();
    for original in [&running, &queued] {
        let run = restored.chat_run(&original.id).unwrap();
        assert_eq!(run.status, ChatRunStatus::Interrupted);
        assert_eq!(run.error_code.as_deref(), Some("chat_restored_copy"));
        assert_eq!(run.last_event_seq, original.last_event_seq + 1);
        assert!(restored.claim_chat_run(&run.id).unwrap().is_none());
    }
    assert!(restored.pending_chat_run_ids(100).unwrap().is_empty());
    assert_eq!(restored.recover_chat_runs_exclusive().unwrap(), 0);
    assert_eq!(
        restored
            .chat_messages(&running_conversation.id, 0, 100)
            .unwrap()[1]
            .content,
        "Partial **retained**\n"
    );
    assert!(
        restored
            .append_chat_delta(
                &running.id,
                &lease.lease_token,
                running.last_event_seq + 2,
                "late"
            )
            .is_err()
    );
    assert_eq!(
        source.chat_run(&running.id).unwrap().status,
        ChatRunStatus::Running
    );
    assert_eq!(
        source.chat_run(&queued.id).unwrap().status,
        ChatRunStatus::Queued
    );
    // The normalized copy is itself a valid, restorable backup.
    restored
        .create_backup(root.path().join("copy.zip"), &mut |_| Ok(()))
        .unwrap();
    verify_backup(root.path().join("copy.zip"), &mut |_| Ok(())).unwrap();
}

#[test]
fn backup_reports_count_chat_from_snapshot_without_changing_old_manifest() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let conversation = setup(&mut db);
    complete(&mut db, &conversation);
    let package = root.path().join("chat-report.zip");
    let written = db.create_backup(&package, &mut |_| Ok(())).unwrap();
    assert_eq!(
        written.chat,
        Some(BackupChatCounts {
            conversations: 1,
            messages: 2
        })
    );
    // New activity after the snapshot must not leak into its verification report.
    db.create_chat_conversation(
        &id(),
        CreateConversation {
            title: "After snapshot".into(),
        },
    )
    .unwrap();
    let verified = verify_backup(&package, &mut |_| Ok(())).unwrap();
    assert_eq!(written, verified);
    let inspection = inspect_backup(&package).unwrap();
    assert!(!inspection.fully_verified);
    assert_eq!(inspection.manifest, written.manifest);
    let manifest_json = serde_json::to_value(&written.manifest).unwrap();
    assert!(manifest_json.get("chat").is_none());
    assert!(manifest_json["counts"].get("chat").is_none());
    // Existing successful task records deserialize as unmeasured, never as zero.
    let mut old_json = serde_json::to_value(&written).unwrap();
    old_json.as_object_mut().unwrap().remove("chat");
    let old: BackupReport = serde_json::from_value(old_json).unwrap();
    assert!(old.chat.is_none());
    assert_eq!(old.manifest, written.manifest);
    let restored =
        prepare_restore(&package, root.path().join("restored"), &mut |_| Ok(())).unwrap();
    assert_eq!(restored.report.chat, written.chat);
}

#[test]
fn newest_message_pages_are_bounded_complete_and_seed_the_same_event_snapshot() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut conversation = setup(&mut db);
    for _ in 0..55 {
        complete(&mut db, &conversation);
        conversation = db.chat_conversation(&conversation.id).unwrap();
    }
    let accepted = db
        .accept_chat_run_from_settings(&id(), request(&conversation))
        .unwrap();
    let lease = db.claim_chat_run(&accepted.run.id).unwrap().unwrap();
    let partial = db
        .append_chat_delta(
            &lease.run.id,
            &lease.lease_token,
            lease.run.last_event_seq + 1,
            "**partial**",
        )
        .unwrap();
    let first = db.chat_messages_page(&conversation.id, None, 40).unwrap();
    assert_eq!(first.items.len(), 40);
    assert_eq!(first.items.first().unwrap().ordinal, 73);
    assert_eq!(first.next_before, Some(73));
    let latest = first.latest_run.unwrap();
    assert_eq!(latest.id, partial.id);
    assert_eq!(latest.last_event_seq, partial.last_event_seq);
    assert_eq!(first.items.last().unwrap().id, latest.assistant_message_id);
    assert_eq!(first.items.last().unwrap().content, "**partial**");
    db.append_chat_delta(
        &latest.id,
        &lease.lease_token,
        latest.last_event_seq + 1,
        " tail",
    )
    .unwrap();
    let events = db
        .chat_run_events(&latest.id, latest.last_event_seq, 100)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].text_delta.as_deref(), Some(" tail"));
    let second = db
        .chat_messages_page(&conversation.id, first.next_before, 40)
        .unwrap();
    assert!(second.latest_run.is_none());
    assert_eq!(second.next_before, Some(33));
    let third = db
        .chat_messages_page(&conversation.id, second.next_before, 40)
        .unwrap();
    assert!(third.next_before.is_none());
    assert_eq!(third.items.len(), 32);
    let ordinals = third
        .items
        .iter()
        .chain(&second.items)
        .chain(&first.items)
        .map(|m| m.ordinal)
        .collect::<Vec<_>>();
    assert_eq!(ordinals, (1..=112).collect::<Vec<_>>());
    assert!(
        db.chat_messages_page(&conversation.id, Some(1), 40)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        db.chat_messages_page(&conversation.id, Some(0), 40)
            .is_err()
    );
    assert!(db.chat_messages_page(&id(), None, 40).is_err());
    assert!(db.chat_messages_page(&conversation.id, None, 101).is_err());
}

#[test]
fn conversation_pages_use_recent_order_with_stable_ties_and_exact_boundaries() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut ids = vec![];
    for i in 0..6 {
        ids.push(
            db.create_chat_conversation(
                &id(),
                CreateConversation {
                    title: format!("Conversation {i}"),
                },
            )
            .unwrap()
            .conversation
            .id,
        );
    }
    // A timestamp tie is a real cursor boundary; id must be the deterministic tiebreaker.
    let connection = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE chat_conversations SET updated_at='2000-01-01T00:00:00+00:00'",
            [],
        )
        .unwrap();
    ids.sort_by(|a, b| b.cmp(a));
    let one = db.chat_conversations_page(None, 3).unwrap();
    assert_eq!(
        one.items.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        ids[..3]
    );
    let two = db.chat_conversations_page(one.next_cursor, 3).unwrap();
    assert_eq!(
        two.items.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        ids[3..]
    );
    assert!(two.next_cursor.is_none());
    let empty = db.chat_messages_page(&ids[0], None, 40).unwrap();
    assert!(empty.items.is_empty() && empty.latest_run.is_none() && empty.next_before.is_none());
    let inserted = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "Newer".into(),
            },
        )
        .unwrap();
    assert_eq!(
        db.chat_conversations_page(None, 3).unwrap().items[0].id,
        inserted.conversation.id
    );
    assert!(db.chat_conversations_page(None, 0).is_err());
    assert!(
        db.chat_conversations_page(
            Some(ChatConversationCursor {
                id: "bad".into(),
                updated_at: "bad".into()
            }),
            3
        )
        .is_err()
    );
}

#[test]
fn paged_history_keeps_each_prior_terminal_status_and_error_after_newer_replies() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let mut c = setup(&mut db);
    let states = [
        ChatRunStatus::Canceled,
        ChatRunStatus::Interrupted,
        ChatRunStatus::TimedOut,
        ChatRunStatus::Failed,
        ChatRunStatus::Truncated,
        ChatRunStatus::Refused,
    ];
    for state in states {
        let accepted = db
            .accept_chat_run_from_settings(&id(), request(&c))
            .unwrap();
        let lease = db.claim_chat_run(&accepted.run.id).unwrap().unwrap();
        db.append_chat_delta(
            &lease.run.id,
            &lease.lease_token,
            lease.run.last_event_seq + 1,
            "partial retained",
        )
        .unwrap();
        db.finish_chat_run(
            &lease.run.id,
            &lease.lease_token,
            state,
            Some("synthetic_test_failure"),
            ChatUsage::default(),
            None,
        )
        .unwrap();
        c = db.chat_conversation(&c.id).unwrap();
    }
    complete(&mut db, &c);
    let first = db.chat_messages_page(&c.id, None, 4).unwrap();
    assert_eq!(
        first.items.last().unwrap().run_status,
        ChatRunStatus::Completed
    );
    assert!(first.items.last().unwrap().run_error_code.is_none());
    let mut messages = first.items;
    let mut before = first.next_before;
    while let Some(cursor) = before {
        let page = db.chat_messages_page(&c.id, Some(cursor), 4).unwrap();
        before = page.next_before;
        messages.splice(0..0, page.items);
    }
    assert_eq!(messages.len(), 14);
    for (i, state) in states.into_iter().enumerate() {
        let reply = &messages[i * 2 + 1];
        assert_eq!(reply.run_status, state);
        assert_eq!(
            reply.run_error_code.as_deref(),
            Some("synthetic_test_failure")
        );
        assert_eq!(reply.content, "partial retained");
    }
    let forward = db.chat_messages(&c.id, 0, 100).unwrap();
    assert_eq!(
        serde_json::to_value(messages).unwrap(),
        serde_json::to_value(forward).unwrap()
    );
}
