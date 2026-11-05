use pkms_core::{
    CancelResearch, CreateResearchTask, Error, Library, RecordExternalStep,
    RecordFailedExternalStep, ResearchBudget, ResearchCommit, ResearchStatus,
    ResearchStopConditions, ResearchTask,
};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn task(db: &mut Library) -> (String, ResearchTask) {
    let command = id();
    let result = db
        .create_research_task(
            &command,
            CreateResearchTask {
                goal: "Receipt recovery".into(),
                collection_id: None,
                queries: vec!["first public query".into(), "second public query".into()],
                budget: ResearchBudget {
                    max_steps: 3,
                    max_sources: 5,
                },
                stop: ResearchStopConditions {
                    deadline_seconds: None,
                    missing_after_step: None,
                },
                external_allowed: true,
                external_search_endpoint: Some("https://example.test/search?q={query}".into()),
            },
        )
        .unwrap();
    (command, result.task)
}

fn success(task: &ResearchTask) -> RecordExternalStep {
    RecordExternalStep {
        task_id: task.id.clone(),
        base_row_version: task.row_version,
        query: task.queries[0].clone(),
        endpoint: task.external_search_endpoint.clone().unwrap(),
        http_status: 200,
        final_url: "https://example.test/search?q=first%20public%20query".into(),
        title: "  original response title  ".into(),
        excerpt: "  public evidence from the original response  ".into(),
    }
}

fn counters(path: &Path) -> (i64, i64, i64, i64) {
    let db = Connection::open_with_flags(
        path.join("library.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT (SELECT count(*) FROM command_receipts),(SELECT count(*) FROM research_steps),(SELECT count(*) FROM research_evidence),(SELECT next_change_seq FROM workspace_meta WHERE singleton=1)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap()
}

fn assert_replayed(original: &ResearchCommit, replayed: &ResearchCommit) {
    assert_eq!(replayed.task, original.task);
    assert!(replayed.receipt.replayed);
    assert_eq!(replayed.receipt.command_id, original.receipt.command_id);
    assert_eq!(replayed.receipt.change_seq, original.receipt.change_seq);
    assert_eq!(replayed.receipt.committed_at, original.receipt.committed_at);
}

#[test]
fn successful_external_receipt_replays_its_snapshot_after_cancel_and_reopen() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (_, task) = task(&mut db);
    let command = id();
    let first = db.record_external_step(&command, success(&task)).unwrap();
    let canceled = db
        .cancel_research(
            &id(),
            CancelResearch {
                task_id: task.id.clone(),
                base_row_version: first.task.row_version,
                reason: "later cancel".into(),
            },
        )
        .unwrap()
        .task;
    drop(db);
    let db = Library::open_existing(root.path()).unwrap();
    let before = counters(root.path());
    let replayed = db
        .replay_external_research(&command, &task.id, task.row_version)
        .unwrap()
        .unwrap();
    assert_replayed(&first, &replayed);
    assert_eq!(db.research_task(&task.id).unwrap(), canceled);
    assert_eq!(
        counters(root.path()),
        before,
        "receipt lookup must not write anything"
    );
}

#[test]
fn failed_external_receipt_replays_but_changed_task_base_and_occupied_id_are_rejected() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (creation, task) = task(&mut db);
    let command = id();
    let first = db
        .record_failed_external_step(
            &command,
            RecordFailedExternalStep {
                task_id: task.id.clone(),
                base_row_version: task.row_version,
                query: task.queries[0].clone(),
                endpoint: task.external_search_endpoint.clone().unwrap(),
                error_code: "webpage_network_failed".into(),
            },
        )
        .unwrap();
    assert_eq!(first.task.status, ResearchStatus::Failed);
    let before = counters(root.path());
    let replayed = db
        .replay_external_research(&command, &task.id, task.row_version)
        .unwrap()
        .unwrap();
    assert_replayed(&first, &replayed);
    for base in [0, task.row_version + 1, u32::MAX] {
        assert!(matches!(
            db.replay_external_research(&command, &task.id, base),
            Err(Error::IdempotencyMismatch)
        ));
    }
    assert!(matches!(
        db.replay_external_research(&command, &id(), task.row_version),
        Err(Error::IdempotencyMismatch)
    ));
    assert!(matches!(
        db.replay_external_research(&creation, &task.id, task.row_version),
        Err(Error::IdempotencyMismatch)
    ));
    assert_eq!(counters(root.path()), before);
}

#[test]
fn host_replay_does_not_weaken_the_full_external_record_hash() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (_, task) = task(&mut db);
    assert!(
        db.replay_external_research(&id(), &task.id, task.row_version)
            .unwrap()
            .is_none()
    );
    let command = id();
    let request = success(&task);
    let first = db.record_external_step(&command, request.clone()).unwrap();
    assert_replayed(
        &first,
        &db.record_external_step(&command, request.clone()).unwrap(),
    );
    let changed = RecordExternalStep {
        title: "a different HTTP response".into(),
        ..request
    };
    assert!(matches!(
        db.record_external_step(&command, changed),
        Err(Error::IdempotencyMismatch)
    ));
    assert_replayed(
        &first,
        &db.replay_external_research(&command, &task.id, task.row_version)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(db.research_task(&task.id).unwrap().steps.len(), 1);
    assert_eq!(db.research_task(&task.id).unwrap().evidence.len(), 1);
}

#[test]
fn malformed_external_receipt_metadata_or_step_is_an_integrity_failure() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (_, task) = task(&mut db);
    let command = id();
    let first = db.record_external_step(&command, success(&task)).unwrap();
    let connection = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let original = serde_json::to_value(&first).unwrap();
    for pointer in ["/task/id", "/task/steps/0/kind", "/receipt/command_id"] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() =
            serde_json::Value::String(if pointer.ends_with("kind") {
                "private_retrieval".into()
            } else {
                id()
            });
        connection
            .execute(
                "UPDATE command_receipts SET result_json=?2 WHERE command_id=?1",
                [&command, &serde_json::to_string(&changed).unwrap()],
            )
            .unwrap();
        assert!(
            matches!(
                db.replay_external_research(&command, &task.id, task.row_version),
                Err(Error::IntegrityFailure)
            ),
            "{pointer}"
        );
    }
}
