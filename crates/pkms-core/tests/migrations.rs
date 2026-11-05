use pkms_core::{Error, Library};
use rusqlite::{Connection, types::Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use tempfile::tempdir;

const FIXTURE: &str = include_str!("fixtures/schema-v1.sql");
const NOTE: &str = "11111111-1111-4111-8111-111111111111";
const TRASHED: &str = "33333333-3333-4333-8333-333333333333";
const COLLECTION: &str = "44444444-4444-4444-8444-444444444444";

fn create_v1(path: &Path) {
    Connection::open(path)
        .unwrap()
        .execute_batch(FIXTURE)
        .unwrap();
}

fn content(connection: &Connection) -> Vec<Vec<Value>> {
    let modern_members: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('memberships') WHERE name='member_id')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut rows = Vec::new();
    for (table, order) in [
        ("notes", "id"),
        ("note_revisions", "note_id,revision"),
        ("collections", "id"),
        ("memberships", "collection_id,note_id"),
        ("note_links", "source_id,target_id"),
        ("note_fts", "note_id"),
    ] {
        let columns = match table {
            "notes" => "id,title,current_revision,anchor_id,trashed,updated_at",
            "note_revisions" => "note_id,revision,title,schema_version,document_json,created_at",
            "collections" => "id,name",
            "memberships" if modern_members => "collection_id,member_id AS note_id",
            _ => "*",
        };
        let mut statement = connection
            .prepare(&format!("SELECT {columns} FROM {table} ORDER BY {order}"))
            .unwrap();
        let count = statement.column_count();
        let values = statement
            .query_map([], |row| (0..count).map(|column| row.get(column)).collect())
            .unwrap();
        rows.extend(values.map(|row| row.unwrap()));
    }
    rows
}

fn version(connection: &Connection) -> i64 {
    connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn backups(workspace: &Path) -> Vec<std::path::PathBuf> {
    let path = workspace.join("migration-backups");
    if !path.exists() {
        return vec![];
    }
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .collect()
}

#[test]
fn historical_fixture_is_frozen_and_upgrade_preserves_all_original_rows() {
    let checksum = format!(
        "{:x}",
        Sha256::digest(FIXTURE.replace("\r\n", "\n").as_bytes())
    );
    assert_eq!(
        checksum,
        "d4471365ad940bfab0e245539c50b49da2e5933effc74923730df894d11f7395"
    );
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    let before = content(&Connection::open(&path).unwrap());
    let library = Library::open_existing(dir.path()).unwrap();
    let info = library.info().unwrap();
    assert_eq!(
        info.database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(info.supported_document_schemas, vec![1, 2, 3]);
    assert_eq!(
        info.migrations
            .iter()
            .map(|r| r.version)
            .collect::<Vec<_>>(),
        (1..=pkms_core::CURRENT_SCHEMA_VERSION as u32).collect::<Vec<_>>()
    );
    assert_eq!(
        info.migrations[0].checksum,
        "c23976de7a7e9d48a0904d0425d14e5f3d5af5d56c29a084ad59d64ae2791f0f"
    );
    assert_eq!(content(&Connection::open(&path).unwrap()), before);
    assert_eq!(library.revisions(NOTE).unwrap().len(), 2);
    assert_eq!(
        library.note(NOTE).unwrap().document.content[0].attrs.id,
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    );
    assert!(library.note(TRASHED).unwrap().trashed);
    assert_eq!(library.collection_notes(COLLECTION).unwrap()[0].id, NOTE);
    assert_eq!(
        library.backlinks(NOTE).unwrap()[0].context,
        "原始引用上下文🙂"
    );
    assert_eq!(library.search("Rust", 10).unwrap()[0].note.id, NOTE);
    let snapshots = backups(dir.path());
    assert_eq!(snapshots.len(), 1);
    let snapshot = Connection::open(&snapshots[0]).unwrap();
    assert_eq!(version(&snapshot), 1);
    assert_eq!(content(&snapshot), before);
    let snapshot_bytes = fs::read(&snapshots[0]).unwrap();
    drop(snapshot);
    drop(library);
    let reopened = Library::open_existing(dir.path()).unwrap();
    assert_eq!(reopened.info().unwrap(), info);
    assert_eq!(backups(dir.path()).len(), 1);
    assert_eq!(fs::read(&snapshots[0]).unwrap(), snapshot_bytes);
}

#[test]
fn online_migration_snapshot_includes_committed_wal_frames() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    // Keep this legacy connection open, so its committed data remains in WAL.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO collections VALUES('wal-collection','仅存在 WAL 中的提交');").unwrap();
    assert!(
        fs::metadata(path.with_extension("sqlite3-wal"))
            .unwrap()
            .len()
            > 0
    );
    let before = content(&raw);
    let library = Library::open_existing(dir.path()).unwrap();
    assert_eq!(content(&raw), before);
    let snapshots = backups(dir.path());
    let snapshot = Connection::open(&snapshots[0]).unwrap();
    assert_eq!(version(&snapshot), 1);
    assert_eq!(content(&snapshot), before);
    drop(library);
}

#[test]
fn snapshot_destination_failure_preserves_v1_and_retry_upgrades() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    let original = fs::read(&path).unwrap();
    // A real filesystem failure, without filling the user's disk or changing ACLs.
    fs::write(dir.path().join("migration-backups"), b"keep this file").unwrap();
    assert!(matches!(
        Library::open_existing(dir.path()),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        fs::read(dir.path().join("migration-backups")).unwrap(),
        b"keep this file"
    );
    assert_eq!(version(&Connection::open(&path).unwrap()), 1);
    fs::remove_file(dir.path().join("migration-backups")).unwrap();
    assert!(Library::open_existing(dir.path()).is_ok());
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn published_migration_checksum_mismatch_is_rejected_without_repair() {
    for version in [1, 2] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        drop(Library::open(dir.path()).unwrap());
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute(
                "UPDATE migration_history SET checksum=?1 WHERE version=?2",
                ("0".repeat(64), version),
            )
            .unwrap();
        }
        let before = fs::read(&path).unwrap();
        assert!(
            matches!(Library::open_existing(dir.path()), Err(Error::MigrationHistoryMismatch {version: actual}) if actual == version)
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(backups(dir.path()).is_empty());
    }
}

#[test]
fn future_and_unrelated_databases_never_enter_migration() {
    for sql in [
        "PRAGMA application_id=1347112275; PRAGMA user_version=999; CREATE TABLE future_data(value TEXT); INSERT INTO future_data VALUES('keep');",
        "CREATE TABLE personal_data(value TEXT); INSERT INTO personal_data VALUES('keep');",
    ] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            Library::open_existing(dir.path()),
            Err(Error::UnsupportedDatabase)
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(backups(dir.path()).is_empty());
    }
}

#[test]
fn broken_v1_references_are_rejected_before_snapshot_or_schema_changes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM note_revisions WHERE revision=2;")
            .unwrap();
    }
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        Library::open_existing(dir.path()),
        Err(Error::IntegrityFailure)
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn fresh_database_has_stable_identity_and_needs_no_migration_backup() {
    let dir = tempdir().unwrap();
    let first = Library::open(dir.path()).unwrap();
    let info = first.info().unwrap();
    let second = Library::open_existing(dir.path()).unwrap();
    assert_eq!(second.info().unwrap(), info);
    assert!(backups(dir.path()).is_empty());
    drop(first);
    drop(second);
    assert_eq!(
        Library::open_existing(dir.path()).unwrap().info().unwrap(),
        info
    );
}

#[test]
fn schema_two_upgrade_keeps_workspace_identity_and_backs_up_before_revision_conversion() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    let raw = Connection::open(&path).unwrap();
    let workspace_id = "77777777-7777-4777-8777-777777777777";
    let m2 = include_str!("../src/migrations/0002_workspace.sql");
    raw.execute_batch(m2).unwrap();
    raw.execute(
        "INSERT INTO workspace_meta VALUES(1,?1,2,17)",
        [workspace_id],
    )
    .unwrap();
    for (version, sql) in [(1, include_str!("../src/schema.sql")), (2, m2)] {
        let hash = format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()));
        raw.execute(
            "INSERT INTO migration_history VALUES(?1,?2,'2026-10-01T00:00:00Z')",
            rusqlite::params![version, hash],
        )
        .unwrap();
    }
    raw.pragma_update(None, "user_version", 2).unwrap();
    let before = content(&raw);
    drop(raw);
    let library = Library::open_existing(dir.path()).unwrap();
    assert_eq!(library.info().unwrap().workspace_id, workspace_id);
    let raw = Connection::open(&path).unwrap();
    assert_eq!(content(&raw), before);
    assert_eq!(
        raw.query_row("SELECT next_change_seq FROM workspace_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        17
    );
    let history = library.revisions(NOTE).unwrap();
    assert_eq!(
        history[1].parent_revision_id,
        Some(history[0].revision_id.clone())
    );
    let snapshots = backups(dir.path());
    assert_eq!(snapshots.len(), 1);
    let snapshot = Connection::open(&snapshots[0]).unwrap();
    assert_eq!(version(&snapshot), 2);
    assert_eq!(content(&snapshot), before);
    assert_eq!(
        snapshot
            .query_row("SELECT workspace_id FROM workspace_meta", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        workspace_id
    );
}

#[test]
fn legacy_sqlite_writer_finishes_before_snapshot_and_migration() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    create_v1(&path);
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "BEGIN IMMEDIATE; INSERT INTO collections VALUES('legacy-writer','迁移前的并发提交');",
    )
    .unwrap();
    let workspace = dir.path().to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let result = Library::open_existing(workspace).map(|library| library.info().unwrap());
        sender.send(result).unwrap();
    });
    assert!(
        receiver
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_err()
    );
    raw.execute_batch("COMMIT;").unwrap();
    assert!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .is_ok()
    );
    handle.join().unwrap();
    let snapshot = Connection::open(&backups(dir.path())[0]).unwrap();
    let count: i64 = snapshot
        .query_row(
            "SELECT count(*) FROM collections WHERE id='legacy-writer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
