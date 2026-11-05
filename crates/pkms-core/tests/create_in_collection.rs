use pkms_core::*;
use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request(collection: Option<&str>, base: Option<u32>) -> CreateNote {
    CreateNote {
        title: "主题中的记录".into(),
        document_schema: DOCUMENT_SCHEMA,
        document: NoteDocument::from_text(id(), "真实正文"),
        collection_id: collection.map(str::to_owned),
        base_collection_row_version: base,
    }
}
fn counts(sql: &Connection) -> Vec<i64> {
    [
        "objects",
        "notes",
        "note_revisions",
        "memberships",
        "command_receipts",
    ]
    .into_iter()
    .map(|table| {
        sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
    .collect()
}
fn state(sql: &Connection, collection: &str) -> (Vec<i64>, i64, u32, Option<String>) {
    let (version, updated) = sql.query_row("SELECT o.row_version,c.updated_at FROM collections c JOIN objects o ON o.id=c.id WHERE c.id=?1", [collection], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    let seq = sql
        .query_row("SELECT next_change_seq FROM workspace_meta", [], |r| {
            r.get(0)
        })
        .unwrap();
    (counts(sql), seq, version, updated)
}

#[test]
fn accepted_create_replays_after_reopen_even_when_collection_changed() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("主题").unwrap();
    let base = db
        .collection_get(&collection.id)
        .unwrap()
        .metadata
        .row_version;
    let req = request(Some(&collection.id), Some(base));
    let command = id();
    let accepted = db.create_document(&command, req.clone()).unwrap();
    assert!(!accepted.receipt.replayed);
    assert_eq!(
        db.collection_get(&collection.id)
            .unwrap()
            .metadata
            .row_version,
        base + 1
    );
    let sql = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let member: (String, u32) = sql
        .query_row(
            "SELECT member_id,position FROM memberships WHERE collection_id=?1",
            [&collection.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(member, (accepted.note.id.clone(), 0));
    let after = state(&sql, &collection.id);
    let retry = db.create_document(&command, req.clone()).unwrap();
    assert!(retry.receipt.replayed);
    assert_eq!(retry.note.revision_id, accepted.note.revision_id);
    assert_eq!(state(&sql, &collection.id), after);
    // A lost reply must still replay after reopening and later lifecycle changes.
    sql.execute(
        "UPDATE objects SET trashed=1,row_version=row_version+1 WHERE id=?1",
        [&collection.id],
    )
    .unwrap();
    let after_trash = state(&sql, &collection.id);
    drop(db);
    let mut db = Library::open(root.path()).unwrap();
    let retry = db.create_document(&command, req).unwrap();
    assert!(retry.receipt.replayed);
    assert_eq!(retry.note.id, accepted.note.id);
    assert_eq!(state(&sql, &collection.id), after_trash);
}

#[test]
fn old_request_serialization_and_receipt_hash_remain_compatible() {
    #[derive(Serialize)]
    struct OldCreate<'a> {
        title: &'a str,
        document_schema: u32,
        document: &'a NoteDocument,
    }
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let req = request(None, None);
    let old = OldCreate {
        title: &req.title,
        document_schema: req.document_schema,
        document: &req.document,
    };
    let old_json = serde_json::to_string(&old).unwrap();
    assert_eq!(serde_json::to_string(&req).unwrap(), old_json);
    let decoded: CreateNote = serde_json::from_str(&old_json).unwrap();
    assert_eq!(decoded, req);
    let command = id();
    let accepted = db.create_document(&command, decoded.clone()).unwrap();
    let old_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(1, "notes.create", &old)).unwrap())
    );
    let sql = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let stored: String = sql
        .query_row(
            "SELECT payload_hash FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, old_hash);
    // Install the independently computed pre-extension hash as a legacy receipt.
    sql.execute(
        "UPDATE command_receipts SET payload_hash=?2 WHERE command_id=?1",
        params![command, old_hash],
    )
    .unwrap();
    drop(db);
    let mut db = Library::open(root.path()).unwrap();
    let replayed = db.create_document(&command, decoded).unwrap();
    assert!(replayed.receipt.replayed);
    assert_eq!(replayed.note.revision_id, accepted.note.revision_id);
}

#[test]
fn invalid_membership_requests_leave_no_note_or_receipt() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("主题").unwrap();
    let base = db
        .collection_get(&collection.id)
        .unwrap()
        .metadata
        .row_version;
    let sql = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let before = state(&sql, &collection.id);
    for req in [
        request(Some(&collection.id), None),
        request(None, Some(base)),
        request(Some("invalid"), Some(base)),
        request(Some(&collection.id), Some(0)),
    ] {
        assert!(matches!(
            db.create_document(&id(), req),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(state(&sql, &collection.id), before);
    }
    assert!(matches!(
        db.create_document(&id(), request(Some(&collection.id), Some(base + 1))),
        Err(Error::MetadataConflict { .. })
    ));
    assert!(matches!(
        db.create_document(&id(), request(Some(&id()), Some(base))),
        Err(Error::NotFound)
    ));
    assert_eq!(state(&sql, &collection.id), before);
    sql.execute("UPDATE objects SET trashed=1 WHERE id=?1", [&collection.id])
        .unwrap();
    let trashed = state(&sql, &collection.id);
    assert!(matches!(
        db.create_document(&id(), request(Some(&collection.id), Some(base))),
        Err(Error::Trashed)
    ));
    assert_eq!(state(&sql, &collection.id), trashed);
}

#[test]
fn full_collection_position_overflow_and_receipt_failure_roll_back_everything() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("主题").unwrap();
    let mut sql = Connection::open(root.path().join("library.sqlite3")).unwrap();
    // Build a bounded capacity fixture without creating 10,000 unrelated revisions.
    let tx = sql.transaction().unwrap();
    for position in 0..10_000 {
        let member = id();
        tx.execute("INSERT INTO objects(id,kind) VALUES(?1,'note')", [&member])
            .unwrap();
        tx.execute(
            "INSERT INTO memberships(collection_id,member_id,position) VALUES(?1,?2,?3)",
            params![collection.id, member, position],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let base = db
        .collection_get(&collection.id)
        .unwrap()
        .metadata
        .row_version;
    let before = state(&sql, &collection.id);
    assert!(matches!(
        db.create_document(&id(), request(Some(&collection.id), Some(base))),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(state(&sql, &collection.id), before);
    sql.execute(
        "DELETE FROM memberships WHERE collection_id=?1 AND position<>0",
        [&collection.id],
    )
    .unwrap();
    sql.execute(
        "UPDATE memberships SET position=4294967295 WHERE collection_id=?1",
        [&collection.id],
    )
    .unwrap();
    let before = state(&sql, &collection.id);
    assert!(matches!(
        db.create_document(&id(), request(Some(&collection.id), Some(base))),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(state(&sql, &collection.id), before);
    sql.execute(
        "UPDATE memberships SET position=0 WHERE collection_id=?1",
        [&collection.id],
    )
    .unwrap();
    sql.execute_batch("CREATE TRIGGER controlled_receipt_failure BEFORE INSERT ON command_receipts WHEN NEW.command_kind='notes.create' BEGIN SELECT RAISE(ABORT,'controlled receipt failure'); END;").unwrap();
    let before = state(&sql, &collection.id);
    let command = id();
    let req = request(Some(&collection.id), Some(base));
    assert!(db.create_document(&command, req.clone()).is_err());
    assert_eq!(state(&sql, &collection.id), before);
    sql.execute_batch("DROP TRIGGER controlled_receipt_failure;")
        .unwrap();
    let accepted = db.create_document(&command, req).unwrap();
    assert!(!accepted.receipt.replayed);
    let position: u32 = sql
        .query_row(
            "SELECT position FROM memberships WHERE member_id=?1",
            [&accepted.note.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(position, 1);
}
