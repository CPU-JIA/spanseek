use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
#[test]
fn older_export_note_json_keeps_working_without_a_citation_style() {
    let input: PortableJobInput = serde_json::from_value(serde_json::json!({
        "kind": "export_note",
        "note_id": id(),
        "revision_id": id(),
        "format": "markdown"
    }))
    .unwrap();
    let PortableJobInput::ExportNote {
        citation_style_id, ..
    } = input
    else {
        panic!("expected export note input");
    };
    assert_eq!(citation_style_id, None);
}
#[test]
fn acceptance_is_atomic_idempotent_and_export_input_keeps_the_selected_revision() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("first", "first content").unwrap();
    let input = PortableJobInput::ExportNote {
        note_id: note.id.clone(),
        revision_id: note.revision_id.clone(),
        format: ExportFormat::Markdown,
        citation_style_id: None,
    };
    let command = id();
    let token = id();
    let accepted = library
        .accept_portable_job(&command, input.clone(), &token)
        .unwrap();
    library
        .save_note(&note.id, 1, "second", "second content")
        .unwrap();
    drop(library);
    let mut library = Library::open_existing(root.path()).unwrap();
    let replay = library
        .accept_portable_job(&command, input.clone(), &token)
        .unwrap();
    assert_eq!(replay.job_id, accepted.job_id);
    assert!(replay.receipt.replayed);
    assert_eq!(library.portable_job(&accepted.job_id).unwrap().input, input);
    assert!(matches!(
        library.accept_portable_job(&command, input, &id()),
        Err(Error::IdempotencyMismatch)
    ));
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_job_receipt BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT,'test'); END").unwrap();
    assert!(
        library
            .accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
            .is_err()
    );
    assert_eq!(library.portable_jobs(100).unwrap().len(), 1);
}

#[test]
fn cancellation_and_recovery_replace_leases_and_reject_late_worker_results() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let job = library
        .accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
        .unwrap();
    let first = library.claim_portable_job(&job.job_id).unwrap().unwrap();
    assert!(library.claim_portable_job(&job.job_id).unwrap().is_none());
    library.recover_portable_jobs().unwrap();
    let second = library.claim_portable_job(&job.job_id).unwrap().unwrap();
    assert_ne!(first, second);
    assert!(
        library
            .finish_portable_job(&job.job_id, &first, None, Some("old_result"))
            .is_err()
    );
    let other = Library::open_existing(root.path()).unwrap();
    other.cancel_portable_job(&job.job_id).unwrap();
    assert!(
        library
            .portable_job_checkpoint(&job.job_id, &second, "copy")
            .is_err()
    );
    assert!(
        library
            .begin_portable_publication(&job.job_id, &second)
            .is_err()
    );
    library
        .finish_portable_job(&job.job_id, &second, None, Some("canceled"))
        .unwrap();
    assert_eq!(
        library.portable_job(&job.job_id).unwrap().status,
        JobStatus::Canceled
    );
    let next = library
        .accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
        .unwrap();
    let lease = library.claim_portable_job(&next.job_id).unwrap().unwrap();
    library
        .begin_portable_publication(&next.job_id, &lease)
        .unwrap();
    assert!(
        !other
            .cancel_portable_job(&next.job_id)
            .unwrap()
            .cancel_requested
    );
}

#[test]
fn job_pages_are_ordered_and_a_cursor_cannot_be_replayed_into_another_library() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    for _ in 0..3 {
        library
            .accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
            .unwrap();
    }
    let first = library.portable_jobs_page(2, None).unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());
    let cursor = first.next_cursor.clone().unwrap();
    let second = library.portable_jobs_page(2, Some(cursor.clone())).unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none(), "the last page has no cursor");
    let mut seen = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|job| job.id.clone())
        .collect::<Vec<_>>();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3, "pages must not repeat or skip a job");
    assert!(
        library
            .portable_jobs_page(2, Some("not-a-cursor".into()))
            .is_err(),
        "a malformed cursor is refused"
    );
    // Replaying a page into another physical database must fail instead of
    // silently continuing into that library's list.
    let other = tempdir().unwrap();
    let copy = Library::open(other.path()).unwrap();
    assert!(copy.portable_jobs_page(2, Some(cursor)).is_err());
}
