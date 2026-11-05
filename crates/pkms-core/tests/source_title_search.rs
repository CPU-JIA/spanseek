use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn import(
    db: &mut Library,
    title: &str,
    text: &str,
    source: Option<&SourceSummary>,
) -> SourceVersion {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: title.into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: source.map(|s| s.id.clone()),
                base_row_version: source.map(|s| s.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &accepted.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}

#[test]
fn source_title_is_independent_and_mixed_search_preserves_and_semantics() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let version = import(
        &mut db,
        "旧标题 amber",
        "正文 cobalt\nsecond tungsten",
        None,
    );
    assert_eq!(db.search_sources("amber", 20).unwrap().len(), 1);
    assert_eq!(db.search_sources("amber cobalt", 20).unwrap().len(), 1);
    assert!(db.search_sources("cobalt tungsten", 20).unwrap().is_empty());
    let source = db.source(&version.source_id).unwrap();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let original_body: Vec<(i64, String, String)> = raw
        .prepare("SELECT rowid,title_tokens,body_tokens FROM source_fts ORDER BY rowid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    db.update_source_metadata(
        &id(),
        UpdateSourceMetadata {
            source_id: source.id,
            base_row_version: source.row_version,
            display_title: "新标题 zircon".into(),
        },
    )
    .unwrap();
    assert!(db.search_sources("amber", 20).unwrap().is_empty());
    assert_eq!(db.search_sources("zircon", 20).unwrap().len(), 1);
    assert_eq!(db.search_sources("zircon cobalt", 20).unwrap().len(), 1);
    assert_eq!(db.search_sources("新标题 正文", 20).unwrap().len(), 1);
    assert_eq!(db.search_sources("cobalt", 20).unwrap().len(), 1);
    let body: Vec<(i64, String, String)> = raw
        .prepare("SELECT rowid,title_tokens,body_tokens FROM source_fts ORDER BY rowid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        body, original_body,
        "rename must not rewrite any body FTS row"
    );
    let source = db.source(&version.source_id).unwrap();
    import(
        &mut db,
        "replacement.txt",
        "current palladium",
        Some(&source),
    );
    assert!(db.search_sources("cobalt", 20).unwrap().is_empty());
    assert!(db.search_sources("zircon cobalt", 20).unwrap().is_empty());
    assert_eq!(db.search_sources("zircon palladium", 20).unwrap().len(), 1);
    let source = db.source(&version.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id,
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(db.search_sources("zircon", 20).unwrap().is_empty());
    assert!(db.search_sources("palladium", 20).unwrap().is_empty());
}

#[test]
fn title_index_rolls_back_when_receipt_write_fails() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let version = import(&mut db, "amber", "cobalt", None);
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER reject_title_receipt BEFORE INSERT ON command_receipts
        WHEN NEW.command_kind='sources.update_metadata' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    let source = db.source(&version.source_id).unwrap();
    assert!(
        db.update_source_metadata(
            &id(),
            UpdateSourceMetadata {
                source_id: source.id.clone(),
                base_row_version: source.row_version,
                display_title: "zircon".into(),
            }
        )
        .is_err()
    );
    assert_eq!(
        db.source(&source.id).unwrap().row_version,
        source.row_version
    );
    assert_eq!(db.search_sources("amber cobalt", 20).unwrap().len(), 1);
    assert!(db.search_sources("zircon", 20).unwrap().is_empty());
}

#[test]
fn mixed_title_groups_do_not_leak_terms_between_sources() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let a = import(&mut db, "amber", "cobalt platinum", None);
    let b = import(&mut db, "cobalt", "amber titanium", None);
    import(&mut db, "unrelated", "amber cobalt uranium", None);
    import(&mut db, "amber", "copper", None);
    import(&mut db, "amber cobalt", "title only", None);
    let hits = db.search_sources("amber cobalt", 20).unwrap();
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].text, "title only");
    assert!(hits.iter().any(|h| h.source_id == a.source_id));
    assert!(hits.iter().any(|h| h.source_id == b.source_id));
    assert_eq!(
        db.search_sources("amber cobalt platinum", 20)
            .unwrap()
            .len(),
        1
    );
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE objects SET search_enabled=0 WHERE kind='source'",
        [],
    )
    .unwrap();
    assert!(db.search_sources("amber cobalt", 20).unwrap().is_empty());
}

#[test]
fn source_title_does_not_expose_unpublished_parse() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "amber".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let text = (0..350).map(|_| "cobalt\n").collect::<String>();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER cancel_title_stage AFTER INSERT ON source_blocks
        WHEN NEW.position=100 BEGIN UPDATE source_jobs SET cancel_requested=1; END;",
    )
    .unwrap();
    assert!(
        db.publish_source_parse(
            &accepted.job_id,
            &lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap()
        )
        .is_err()
    );
    assert!(db.search_sources("amber", 20).unwrap().is_empty());
    assert!(db.search_sources("cobalt", 20).unwrap().is_empty());
    assert!(db.search_sources("amber cobalt", 20).unwrap().is_empty());
}
