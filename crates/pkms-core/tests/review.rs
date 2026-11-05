use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn target(id: &str) -> ResolveLink {
    ResolveLink {
        target_id: id.into(),
        target_block_id: None,
        target_revision_id: None,
    }
}
fn add(db: &mut Library, note: &str) -> ReviewEntry {
    db.add_review(
        &id(),
        AddReview {
            target: target(note),
            base_row_version: None,
        },
    )
    .unwrap()
    .entry
}
fn change(e: &ReviewEntry) -> ReviewChange {
    ReviewChange {
        entry_id: e.id.clone(),
        base_row_version: e.row_version,
    }
}
fn list(bucket: ReviewBucket) -> ReviewListRequest {
    ReviewListRequest {
        bucket,
        collection_id: None,
        limit: 100,
        cursor: None,
    }
}
#[test]
fn add_freezes_revision_and_duplicate_targets_do_not_add_events() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("Original", "old content").unwrap();
    let command = id();
    let input = AddReview {
        target: target(&n.id),
        base_row_version: None,
    };
    let e = db.add_review(&command, input.clone()).unwrap().entry;
    assert_eq!(
        e.target.target_revision_id.as_deref(),
        Some(n.revision_id.as_str())
    );
    assert!(db.add_review(&command, input).unwrap().receipt.replayed);
    let duplicate = db
        .add_review(
            &id(),
            AddReview {
                target: e.target.clone(),
                base_row_version: None,
            },
        )
        .unwrap()
        .entry;
    assert_eq!(duplicate.id, e.id);
    assert_eq!(duplicate.row_version, 1);
    db.save_note(&n.id, n.revision, "Changed", "new content")
        .unwrap();
    let resolved = db.review_entry(&e.id).unwrap().resolved;
    assert_eq!(resolved.title.as_deref(), Some("Original"));
    assert_eq!(
        resolved.resolved_revision_id.as_deref(),
        Some(n.revision_id.as_str())
    );
    let done = db.complete_review(&id(), change(&e)).unwrap().entry;
    assert_eq!(done.state, ReviewState::Completed);
    // A completed entry has left the queue: the same add is now a request for its
    // current version instead of a silent no-op success.
    assert!(matches!(
        db.add_review(
            &id(),
            AddReview {
                target: e.target.clone(),
                base_row_version: None
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_ne!(add(&mut db, &n.id).id, e.id);
    let history = db
        .review_history(&ReviewHistoryRequest {
            entry_id: e.id,
            limit: 100,
            cursor: None,
        })
        .unwrap();
    assert_eq!(history.items.len(), 2);
    assert_eq!(history.items[1].action, ReviewAction::Completed);
}
#[test]
fn completed_review_requeues_on_an_explicit_add_with_its_current_version() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "content").unwrap();
    let e = add(&mut db, &n.id);
    let snoozed = db
        .snooze_review(
            &id(),
            SnoozeReview {
                entry_id: e.id.clone(),
                base_row_version: e.row_version,
                due_at: "2099-01-01T09:00:00+08:00".into(),
            },
        )
        .unwrap()
        .entry;
    let done = db.complete_review(&id(), change(&snoozed)).unwrap().entry;
    assert_eq!(done.state, ReviewState::Completed);
    assert_eq!(done.due_at, "2099-01-01T01:00:00.000Z");
    assert_eq!(
        db.review_list(&list(ReviewBucket::Completed))
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        db.review_list(&list(ReviewBucket::Due))
            .unwrap()
            .items
            .is_empty()
    );
    // Leaving the queue is not a duplicate add: the caller must bring the version
    // it read, and a wrong one stays the existing conflict.
    assert!(matches!(
        db.add_review(
            &id(),
            AddReview {
                target: done.target.clone(),
                base_row_version: None
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        db.add_review(
            &id(),
            AddReview {
                target: done.target.clone(),
                base_row_version: Some(done.row_version + 1)
            }
        ),
        Err(Error::MetadataConflict { .. })
    ));
    let untouched = db.review_entry(&e.id).unwrap().entry;
    assert_eq!(untouched.state, ReviewState::Completed);
    assert_eq!(untouched.row_version, done.row_version);
    let requeued = db
        .add_review(
            &id(),
            AddReview {
                target: done.target.clone(),
                base_row_version: Some(done.row_version),
            },
        )
        .unwrap()
        .entry;
    assert_eq!(requeued.id, e.id);
    assert_eq!(requeued.state, ReviewState::Pending);
    assert_eq!(requeued.row_version, done.row_version + 1);
    assert_ne!(requeued.due_at, done.due_at);
    assert!(
        db.review_list(&list(ReviewBucket::Completed))
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        db.review_list(&list(ReviewBucket::Due))
            .unwrap()
            .items
            .len(),
        1
    );
    let history = db
        .review_history(&ReviewHistoryRequest {
            entry_id: e.id.clone(),
            limit: 100,
            cursor: None,
        })
        .unwrap();
    assert_eq!(
        history
            .items
            .iter()
            .map(|event| (event.seq, event.action))
            .collect::<Vec<_>>(),
        vec![
            (1, ReviewAction::Added),
            (2, ReviewAction::Snoozed),
            (3, ReviewAction::Completed),
            (4, ReviewAction::Added)
        ]
    );
    assert_eq!(history.items[3].state, ReviewState::Pending);
    assert_eq!(history.items[3].due_at, requeued.due_at);
    // Content the user can no longer open does not re-enter the queue.
    let completed_again = db.complete_review(&id(), change(&requeued)).unwrap().entry;
    db.set_trashed(&n.id, true).unwrap();
    assert!(matches!(
        db.add_review(
            &id(),
            AddReview {
                target: completed_again.target.clone(),
                base_row_version: Some(completed_again.row_version)
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        db.review_entry(&e.id).unwrap().entry.row_version,
        completed_again.row_version
    );
    // Added after completed is a valid history: backup and restore keep it.
    let package = dir.path().join("requeued.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = dir.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    let item = restored.review_entry(&e.id).unwrap();
    assert_eq!(item.entry.state, ReviewState::Completed);
    assert_eq!(item.entry.row_version, completed_again.row_version);
    assert_eq!(
        restored
            .review_history(&ReviewHistoryRequest {
                entry_id: e.id,
                limit: 100,
                cursor: None
            })
            .unwrap()
            .items
            .len(),
        completed_again.row_version as usize
    );
}
#[test]
fn review_state_machine_cas_snooze_canonical_time_and_removed_revival() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "content").unwrap();
    let e = add(&mut db, &n.id);
    let done = db.complete_review(&id(), change(&e)).unwrap().entry;
    assert!(matches!(
        db.remove_review(&id(), change(&e)),
        Err(Error::MetadataConflict { .. })
    ));
    let snoozed = db
        .snooze_review(
            &id(),
            SnoozeReview {
                entry_id: e.id.clone(),
                base_row_version: done.row_version,
                due_at: "2099-01-01T09:00:00+08:00".into(),
            },
        )
        .unwrap()
        .entry;
    assert_eq!(snoozed.due_at, "2099-01-01T01:00:00.000Z");
    assert!(
        db.review_list(&list(ReviewBucket::Due))
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        db.review_list(&list(ReviewBucket::Later))
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        db.snooze_review(
            &id(),
            SnoozeReview {
                entry_id: e.id.clone(),
                base_row_version: snoozed.row_version,
                due_at: "2000-01-01T00:00:00Z".into()
            }
        )
        .is_err()
    );
    let removed = db.remove_review(&id(), change(&snoozed)).unwrap().entry;
    assert!(db.complete_review(&id(), change(&removed)).is_err());
    assert!(
        db.add_review(
            &id(),
            AddReview {
                target: removed.target.clone(),
                base_row_version: None
            }
        )
        .is_err()
    );
    let revived = db
        .add_review(
            &id(),
            AddReview {
                target: removed.target.clone(),
                base_row_version: Some(removed.row_version),
            },
        )
        .unwrap()
        .entry;
    assert_eq!(revived.id, e.id);
    assert_eq!(revived.row_version, 5);
    assert_eq!(revived.state, ReviewState::Pending);
    let history = db
        .review_history(&ReviewHistoryRequest {
            entry_id: e.id,
            limit: 100,
            cursor: None,
        })
        .unwrap();
    assert_eq!(
        history.items.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
}
#[test]
fn trashed_target_cannot_advance_but_can_be_removed_and_backed_up() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let n = db.create_note("note", "content").unwrap();
    let e = add(&mut db, &n.id);
    db.set_trashed(&n.id, true).unwrap();
    assert_eq!(
        db.review_entry(&e.id).unwrap().resolved.state,
        LinkState::TargetTrashed
    );
    assert!(db.complete_review(&id(), change(&e)).is_err());
    assert!(
        db.snooze_review(
            &id(),
            SnoozeReview {
                entry_id: e.id.clone(),
                base_row_version: 1,
                due_at: "2099-01-01T00:00:00Z".into()
            }
        )
        .is_err()
    );
    assert_eq!(
        db.add_review(
            &id(),
            AddReview {
                target: e.target.clone(),
                base_row_version: None
            }
        )
        .unwrap()
        .entry
        .row_version,
        1
    );
    let removed = db.remove_review(&id(), change(&e)).unwrap().entry;
    assert_eq!(removed.row_version, 2);
    let package = root.path().join("review.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    let item = restored.review_entry(&e.id).unwrap();
    assert_eq!(item.entry.state, ReviewState::Removed);
    assert_eq!(item.resolved.state, LinkState::TargetTrashed);
    assert_eq!(
        restored
            .review_history(&ReviewHistoryRequest {
                entry_id: e.id,
                limit: 100,
                cursor: None
            })
            .unwrap()
            .items
            .len(),
        2
    );
}
#[test]
fn event_failure_rolls_back_state_version_and_receipt() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let n = db.create_note("note", "content").unwrap();
    let e = add(&mut db, &n.id);
    let sql = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_review BEFORE INSERT ON review_events WHEN NEW.action='completed' BEGIN SELECT RAISE(ABORT,'controlled review failure'); END;").unwrap();
    let command = id();
    assert!(db.complete_review(&command, change(&e)).is_err());
    assert_eq!(db.review_entry(&e.id).unwrap().entry.row_version, 1);
    let receipts: u32 = sql
        .query_row(
            "SELECT count(*) FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 0);
    sql.execute_batch("DROP TRIGGER fail_review;").unwrap();
    db.complete_review(&command, change(&e)).unwrap();
    assert!(
        db.complete_review(&command, change(&e))
            .unwrap()
            .receipt
            .replayed
    );
}
#[test]
fn list_cursor_binds_local_library_and_expires_on_target_lifecycle_change() {
    let first = tempdir().unwrap();
    let other = tempdir().unwrap();
    let mut db = Library::open(first.path()).unwrap();
    let mut other_db = Library::open(other.path()).unwrap();
    let mut notes = Vec::new();
    for _ in 0..3 {
        let n = db.create_note("note", "content").unwrap();
        add(&mut db, &n.id);
        notes.push(n);
        let n = other_db.create_note("note", "content").unwrap();
        add(&mut other_db, &n.id);
    }
    let mut request = list(ReviewBucket::Due);
    request.limit = 1;
    let page = db.review_list(&request).unwrap();
    request.cursor = page.next_cursor.clone();
    assert_eq!(db.review_list(&request).unwrap().items.len(), 1);
    assert!(matches!(
        other_db.review_list(&request),
        Err(Error::InvalidInput(_))
    ));
    db.set_trashed(&notes[2].id, true).unwrap();
    assert!(matches!(db.review_list(&request), Err(Error::QueryExpired)));
    let collection = db.create_collection("collection").unwrap();
    db.add_to_collection(&collection.id, &notes[0].id).unwrap();
    let mut scoped = list(ReviewBucket::Due);
    scoped.collection_id = Some(collection.id);
    let page = db.review_list(&scoped).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].entry.target.target_id, notes[0].id);
}
#[test]
fn source_and_annotation_reviews_freeze_the_original_version() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let command = id();
    let job = db
        .accept_source_import(
            &command,
            ImportSource {
                input_token: command.clone(),
                original_name: "source.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let bytes = b"source quote\nsecond quote";
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    let parsed = parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap();
    let block = parsed.blocks[0].id.clone();
    let other_block = parsed.blocks[1].id.clone();
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
        .unwrap();
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block,
                    start_codepoint: 0,
                    end_codepoint: 12,
                    quote: "source quote".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                comment: "remember".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let source = add(&mut db, &version.source_id);
    let excerpt = add(&mut db, &annotation.id);
    assert_eq!(source.kind, ObjectKind::Source);
    assert_eq!(excerpt.kind, ObjectKind::Annotation);
    assert_eq!(
        excerpt.target.target_revision_id.as_deref(),
        Some(version.id.as_str())
    );
    let jump = db
        .review_entry(&excerpt.id)
        .unwrap()
        .resolved
        .source_target
        .unwrap();
    assert_eq!(jump.source_version_id, version.id);
    assert_eq!(jump.annotation_id, Some(annotation.id));
    db.prepare_backup(dir.path().join("valid.zip"), &mut |_| Ok(()))
        .unwrap();
    let mut forged = excerpt.target.clone();
    forged.target_block_id = Some(other_block);
    let json = serde_json::to_string(&forged).unwrap();
    use sha2::{Digest, Sha256};
    let key = format!("{:x}", Sha256::digest(serde_json::to_vec(&forged).unwrap()));
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE review_entries SET target_json=?2,target_key=?3 WHERE id=?1",
        rusqlite::params![excerpt.id, json, key],
    )
    .unwrap();
    assert!(matches!(
        db.prepare_backup(dir.path().join("forged.zip"), &mut |_| Ok(())),
        Err(Error::IntegrityFailure)
    ));
}

#[test]
fn counts_agree_with_each_list_and_reject_a_bad_collection() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let due_note = db.create_note("Due", "due content").unwrap();
    let later_note = db.create_note("Later", "later content").unwrap();
    let done_note = db.create_note("Done", "done content").unwrap();
    let removed_note = db.create_note("Removed", "removed content").unwrap();
    let due = add(&mut db, &due_note.id);
    let later = add(&mut db, &later_note.id);
    let done = add(&mut db, &done_note.id);
    let removed = add(&mut db, &removed_note.id);
    // A fresh entry is due now; only a due date behind the boundary moves it
    // into 稍后, which is exactly the split the lists filter with.
    let later = db
        .snooze_review(
            &id(),
            SnoozeReview {
                entry_id: later.id.clone(),
                base_row_version: later.row_version,
                due_at: "2999-01-01T00:00:00.000Z".into(),
            },
        )
        .unwrap()
        .entry;
    db.complete_review(&id(), change(&done)).unwrap();
    db.remove_review(&id(), change(&removed)).unwrap();
    let _ = (due, later);
    let counts = db.review_counts(None).unwrap();
    assert_eq!(
        (counts.due, counts.later, counts.completed, counts.removed),
        (1, 1, 1, 1)
    );
    assert!(!counts.as_of.is_empty(), "the split instant is reported");
    // Same buckets, same boundary: every badge equals the length of its list.
    for (bucket, expected) in [
        (ReviewBucket::Due, counts.due),
        (ReviewBucket::Later, counts.later),
        (ReviewBucket::Completed, counts.completed),
        (ReviewBucket::Removed, counts.removed),
    ] {
        assert_eq!(
            db.review_list(&list(bucket)).unwrap().items.len() as u32,
            expected
        );
    }
    // The collection filter is the filter the list uses, and a bad id is refused.
    let collection = id();
    let scoped = db.review_counts(Some(&collection)).unwrap();
    assert_eq!(
        (scoped.due, scoped.later, scoped.completed, scoped.removed),
        (0, 0, 0, 0)
    );
    assert!(db.review_counts(Some("not-a-uuid")).is_err());
}
