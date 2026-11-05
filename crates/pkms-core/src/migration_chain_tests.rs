//! The chain every published schema version still has to survive. The installed
//! V1.6 candidate writes K17 libraries, so K17 -> current must run unattended.
use super::*;

#[test]
fn tag_management_upgrade_preserves_legacy_identity_and_unknown_timestamps() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("library.sqlite3");
    let mut connection = connect(&path, true).unwrap();
    initialize_at(&mut connection, 41).unwrap();
    let id = Uuid::new_v4().to_string();
    connection
        .execute(
            "INSERT INTO tags(id,display_name,normalized_name) VALUES(?1,'旧标签','旧标签')",
            [&id],
        )
        .unwrap();
    drop(connection);
    let mut connection = connect(&path, false).unwrap();
    upgrade(&mut connection, &path).unwrap();
    let tag = connection.query_row(
        "SELECT display_name,description,row_version,created_at,updated_at FROM tags WHERE id=?1", [&id],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,u32>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?)),
    ).unwrap();
    assert_eq!(tag, ("旧标签".into(), "".into(), 1, None, None));
}

#[test]
fn every_published_version_upgrades_to_the_current_schema_in_one_run() {
    for version in 7..CURRENT_SCHEMA_VERSION {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("library.sqlite3");
        let mut connection = connect(&path, true).unwrap();
        initialize_at(&mut connection, version).unwrap();
        let before: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(before, version);
        drop(connection);

        let mut connection = connect(&path, false).unwrap();
        upgrade(&mut connection, &path).unwrap();
        let after: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(after, CURRENT_SCHEMA_VERSION, "upgrading from {version}");
        let applied: Vec<i64> = connection
            .prepare("SELECT version FROM migration_history ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            applied,
            (1..=CURRENT_SCHEMA_VERSION).collect::<Vec<_>>(),
            "history after upgrading from {version}"
        );
    }
}

/// Upgrading must never silently drop the migration snapshots it promises.
#[test]
fn an_upgrade_writes_a_recoverable_snapshot_beside_the_library() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("library.sqlite3");
    let mut connection = connect(&path, true).unwrap();
    initialize_at(&mut connection, 17).unwrap();
    drop(connection);
    let mut connection = connect(&path, false).unwrap();
    upgrade(&mut connection, &path).unwrap();
    let snapshots: Vec<_> = std::fs::read_dir(temporary.path().join("migration-backups"))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(snapshots.len(), 1);
    assert!(
        snapshots[0].file_name().to_string_lossy().contains(".v17-"),
        "snapshot must name the version it replaced"
    );
}
#[test]
fn newer_library_and_foreign_application_are_rejected_without_rewriting_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("library.sqlite3");
    let mut connection = connect(&path, true).unwrap();
    initialize_at(&mut connection, CURRENT_SCHEMA_VERSION).unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
        .unwrap();
    assert!(matches!(
        database_state(&connection, false),
        Err(Error::UnsupportedDatabase)
    ));
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION + 1);

    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
        .unwrap();
    connection
        .pragma_update(None, "application_id", 99)
        .unwrap();
    assert!(matches!(
        database_state(&connection, false),
        Err(Error::UnsupportedDatabase)
    ));
    let app: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .unwrap();
    assert_eq!(app, 99);
}
