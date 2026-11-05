use pkms_sync::{
    CURRENT_AUTHORITY_SCHEMA, MIN_SUPPORTED_AUTHORITY_SCHEMA, RejectReason, SYNC_PROTOCOL_VERSION,
    SchemaDecision, SyncAuthority, SyncError, classify_authority_database,
};

fn open_error(path: &std::path::Path) -> SyncError {
    match SyncAuthority::open(path) {
        Ok(_) => panic!("authority unexpectedly opened"),
        Err(error) => error,
    }
}
fn create_at(path: &std::path::Path, version: i64) {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    for (number, sql) in [
        (2, include_str!("../src/migrations/0002_oplog.sql")),
        (3, include_str!("../src/migrations/0003_conflicts.sql")),
        (4, include_str!("../src/migrations/0004_trash.sql")),
        (5, include_str!("../src/migrations/0005_blobs.sql")),
        (6, include_str!("../src/migrations/0006_keys.sql")),
        (7, include_str!("../src/migrations/0007_resume.sql")),
        (8, include_str!("../src/migrations/0008_member_roles.sql")),
        (
            9,
            include_str!("../src/migrations/0009_member_identity.sql"),
        ),
    ] {
        if number <= version {
            connection.execute_batch(sql).unwrap();
        }
    }
}

#[test]
fn policy_matrix_is_explicit() {
    assert_eq!(MIN_SUPPORTED_AUTHORITY_SCHEMA, 1);
    assert_eq!(CURRENT_AUTHORITY_SCHEMA, 9);
    assert_eq!(SYNC_PROTOCOL_VERSION, 3);
    assert!(matches!(
        classify_authority_database(0, 0, true),
        SchemaDecision::Create
    ));
    for version in 1..CURRENT_AUTHORITY_SCHEMA {
        assert_eq!(
            classify_authority_database(1_347_112_258, version, false),
            SchemaDecision::Migrate { from: version }
        );
    }
    assert_eq!(
        classify_authority_database(1_347_112_258, CURRENT_AUTHORITY_SCHEMA, false),
        SchemaDecision::Current
    );
    assert_eq!(
        classify_authority_database(1_347_112_258, CURRENT_AUTHORITY_SCHEMA + 1, false),
        SchemaDecision::Reject(RejectReason::FutureSchema)
    );
    assert_eq!(
        classify_authority_database(99, 7, false),
        SchemaDecision::Reject(RejectReason::ForeignApplication)
    );
    assert_eq!(
        classify_authority_database(1_347_112_258, 0, false),
        SchemaDecision::Reject(RejectReason::InvalidVersion)
    );
}

#[test]
fn every_supported_old_authority_migrates_to_current() {
    for version in 1..CURRENT_AUTHORITY_SCHEMA {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sync.sqlite3");
        create_at(&path, version);
        let authority = SyncAuthority::open(&path).unwrap();
        assert_eq!(
            authority.schema_version().unwrap(),
            CURRENT_AUTHORITY_SCHEMA
        );
    }
}

#[test]
fn future_authority_is_rejected_without_modification() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sync.sqlite3");
    create_at(&path, CURRENT_AUTHORITY_SCHEMA);
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", CURRENT_AUTHORITY_SCHEMA + 1)
            .unwrap();
    }
    let error = open_error(&path);
    assert!(matches!(error, SyncError::UnsupportedDatabase(_)));
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_AUTHORITY_SCHEMA + 1);
}

#[test]
fn foreign_identity_and_nonempty_unidentified_database_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let foreign = directory.path().join("foreign.sqlite3");
    let connection = rusqlite::Connection::open(&foreign).unwrap();
    connection
        .pragma_update(None, "application_id", 99)
        .unwrap();
    connection.pragma_update(None, "user_version", 7).unwrap();
    assert!(matches!(
        open_error(&foreign),
        SyncError::UnsupportedDatabase(_)
    ));

    let stray = directory.path().join("stray.sqlite3");
    let connection = rusqlite::Connection::open(&stray).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated(value TEXT);")
        .unwrap();
    assert!(matches!(
        open_error(&stray),
        SyncError::UnsupportedDatabase(_)
    ));
}

#[test]
fn v7_database_backfills_earliest_device_as_owner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sync.sqlite3");
    create_at(&path, 7);
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO sync_devices(device_id,display_name,public_key,fingerprint,status,paired_at) VALUES(?1,?2,?3,?4,'active',?5)",
                rusqlite::params!["device-new", "new phone", "key-new", "fp-new", "2026-02-01T00:00:00.000Z"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sync_devices(device_id,display_name,public_key,fingerprint,status,paired_at) VALUES(?1,?2,?3,?4,'active',?5)",
                rusqlite::params!["device-old", "old desktop", "key-old", "fp-old", "2026-01-01T00:00:00.000Z"],
            )
            .unwrap();
    }
    let authority = SyncAuthority::open(&path).unwrap();
    assert_eq!(authority.schema_version().unwrap(), 9);
    assert_eq!(
        authority.device("device-old").unwrap().role,
        pkms_sync::MemberRole::Owner
    );
    assert_eq!(
        authority.device("device-new").unwrap().role,
        pkms_sync::MemberRole::Editor
    );
    let old = authority.device("device-old").unwrap();
    assert_eq!(old.member_name, "old desktop");
    assert_eq!(authority.member(&old.member_id).unwrap().device_count, 1);
    assert_eq!(authority.list_members().unwrap().len(), 2);
}
