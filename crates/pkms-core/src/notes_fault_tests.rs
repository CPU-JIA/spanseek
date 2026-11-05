//! Test-only checkpoints let the parent terminate exactly its own save process.
use super::*;
use std::{
    cell::RefCell,
    fs::File,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

thread_local! {
    static CHECKPOINT: RefCell<Option<(String, PathBuf)>> = const { RefCell::new(None) };
}

pub(super) fn checkpoint(stage: &str, connection: &Connection) {
    CHECKPOINT.with_borrow(|configured| {
        let Some((expected, marker)) = configured else {
            return;
        };
        if expected != stage {
            return;
        }
        if stage == "before_receipt" {
            // Flush dirty pages into the WAL while the save remains uncommitted.
            connection.cache_flush().unwrap();
        }
        File::create_new(marker).unwrap().sync_all().unwrap();
        // No unwind or graceful close is allowed across the selected checkpoint.
        // The parent records readiness, kills this specific Child and waits for it.
        std::thread::sleep(Duration::from_secs(30));
        panic!("parent did not terminate the save test process");
    });
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn save_process_child() {
    let Some(root) = std::env::var_os("PKMS_NOTE_SAVE_FAULT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let stage = std::env::var("PKMS_NOTE_SAVE_FAULT_STAGE").unwrap();
    let (command, request): (String, SaveNote) =
        serde_json::from_slice(&std::fs::read(root.join("request.json")).unwrap()).unwrap();
    let mut library = Library::open_existing(root.join("knowledge")).unwrap();
    CHECKPOINT.set(Some((stage, root.join("checkpoint"))));
    library.save_document(&command, request).unwrap();
    panic!("save did not reach the selected checkpoint");
}

fn snapshot(db: &Connection) -> Vec<(String, String)> {
    [
        "objects",
        "notes",
        "note_revisions",
        "blocks",
        "note_revision_blocks",
        "body_links",
        "note_revision_blobs",
        "note_index_queue",
        "index_dirty_objects",
        "command_receipts",
        "workspace_meta",
        "note_fts",
    ]
    .into_iter()
    .map(|table| {
        let columns = db
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap()
            .column_count();
        let order = (1..=columns)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        let mut hash = Sha256::new();
        while let Some(row) = rows.next().unwrap() {
            let values = (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i).unwrap())
                .collect::<Vec<_>>();
            hash.update(format!("{values:?}\n"));
        }
        (table.to_owned(), format!("{:x}", hash.finalize()))
    })
    .collect()
}

fn verify_crash(stage: &str) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("knowledge");
    let mut library = Library::open(&workspace).unwrap();
    let target = library.create_note("引用目标", "targetoriginal").unwrap();
    let original = library.create_note("原稿", "olduniqueword").unwrap();
    let command = Uuid::new_v4().to_string();
    let request = SaveNote {
        note_id: original.id.clone(),
        base_revision_id: original.revision_id.clone(),
        title: "新修订".into(),
        document_schema: DOCUMENT_SCHEMA,
        document: serde_json::from_value(serde_json::json!({
            "type":"doc", "content":[
                {"type":"heading", "attrs":{"id":Uuid::new_v4().to_string(),"level":2},
                 "content":[{"type":"text","text":"newuniqueword 中文😀","marks":[{"type":"bold"}]}]},
                {"type":"paragraph", "attrs":{"id":Uuid::new_v4().to_string()},
                 "content":[{"type":"noteReference","attrs":{"target_id":target.id,
                    "target_block_id":target.document.content[0].attrs.id,
                    "target_revision_id":target.revision_id,
                    "occurrence_id":Uuid::new_v4().to_string(),"label":"固定引用"}}]}
            ]
        })).unwrap(),
    };
    library.connection.execute(
        "UPDATE index_dirty_objects SET attempts=2,next_retry_at='2099-01-01T00:00:00Z',last_error='qa_retry' WHERE object_id=?1",
        [&original.id],
    ).unwrap();
    let before = snapshot(&library.connection);
    drop(library);
    std::fs::write(
        root.path().join("request.json"),
        serde_json::to_vec(&(&command, &request)).unwrap(),
    )
    .unwrap();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "notes::fault_tests::save_process_child",
                "--nocapture",
            ])
            .env("PKMS_NOTE_SAVE_FAULT_ROOT", root.path())
            .env("PKMS_NOTE_SAVE_FAULT_STAGE", stage)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.path().join("checkpoint").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {stage}"
        );
        assert!(Instant::now() < deadline, "checkpoint timed out: {stage}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let child_id = child.0.id();
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let mut reopened = Library::open_existing(&workspace).unwrap();
    let committed = stage == "after_commit";
    let observed = reopened.note(&original.id).unwrap();
    assert_eq!(reopened.note(&target.id).unwrap(), target);
    if committed {
        assert_eq!(observed.document, request.document);
        assert_eq!(observed.revision, 2);
    } else {
        assert_eq!(observed, original);
        assert_eq!(
            snapshot(&reopened.connection),
            before,
            "partial transaction survived {stage}"
        );
    }
    let settled = snapshot(&reopened.connection);
    let result = reopened.save_document(&command, request.clone()).unwrap();
    assert_eq!(result.receipt.replayed, committed);
    assert_eq!(result.note.document, request.document);
    assert_eq!(
        result.note.parent_revision_id,
        Some(original.revision_id.clone())
    );
    if committed {
        assert_eq!(snapshot(&reopened.connection), settled);
    }
    let replay = reopened.save_document(&command, request.clone()).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.note, result.note);
    assert_eq!(replay.receipt.change_seq, result.receipt.change_seq);
    assert_eq!(reopened.revisions(&original.id).unwrap().len(), 2);
    assert!(reopened.search("olduniqueword", 10).unwrap().is_empty());
    assert_eq!(reopened.search("newuniqueword", 10).unwrap().len(), 1);
    assert_eq!(
        reopened
            .note_references(&original.id, &result.note.revision_id)
            .unwrap()
            .len(),
        1
    );
    let queued: String = reopened
        .connection
        .query_row(
            "SELECT revision_id FROM note_index_queue WHERE note_id=?1",
            [&original.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(queued, result.note.revision_id);
    let dirty: (u32, Option<String>, Option<String>) = reopened
        .connection
        .query_row(
            "SELECT attempts,next_retry_at,last_error FROM index_dirty_objects WHERE object_id=?1",
            [&original.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(dirty, (0, None, None));
    let integrity: String = reopened
        .connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        reopened
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        reopened.save_document(&Uuid::new_v4().to_string(), request),
        Err(Error::RevisionConflict { .. })
    ));
    eprintln!(
        "verified save crash stage={stage} child_pid={child_id} replayed={committed} revisions=2 integrity=ok"
    );
}

#[test]
fn killed_before_save_transaction_preserves_all_original_rows() {
    verify_crash("before_transaction");
}
#[test]
fn killed_after_revision_before_receipt_rolls_back_every_projection() {
    verify_crash("before_receipt");
}
#[test]
fn killed_after_commit_before_result_replays_same_rich_revision() {
    verify_crash("after_commit");
}
