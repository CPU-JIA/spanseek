use pkms_core::*;
use std::io::Cursor;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request() -> ImportWebpage {
    ImportWebpage {
        url: "https://example.org/start".into(),
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}
fn facts() -> WebCaptureInput {
    WebCaptureInput {
        final_url: "https://example.org/final".into(),
        http_status: 200,
        content_type: "text/html; charset=utf-8".into(),
    }
}

#[test]
fn webpage_response_facts_are_immutable_and_survive_full_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path().join("original")).unwrap();
    let command = id();
    let accepted = db.accept_webpage_import(&command, request()).unwrap();
    assert!(
        db.replay_webpage_import(&command, &request())
            .unwrap()
            .unwrap()
            .receipt
            .replayed
    );
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_webpage_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"<title>Example</title>body"),
            facts(),
            &mut |_| Ok(()),
        )
        .unwrap();
    let capture = version.web_capture.as_ref().unwrap();
    assert_eq!(
        version.origin_url.as_deref(),
        Some("https://example.org/start")
    );
    assert_eq!(capture.final_url, "https://example.org/final");
    assert_eq!(capture.captured_at, version.captured_at);
    let mut changed = facts();
    changed.final_url = "https://example.org/changed".into();
    let replay = db
        .capture_webpage_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"changed"),
            changed,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&version).unwrap()
    );
    let backup = dir.path().join("web.pkmsbackup");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let restored = dir.path().join("restored");
    prepare_restore(&backup, &restored, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let db = Library::open_existing(restored).unwrap();
    assert_eq!(
        serde_json::to_value(db.source_version(&version.id).unwrap()).unwrap(),
        serde_json::to_value(version).unwrap()
    );
}

#[test]
fn invalid_response_is_rejected_before_registering_a_version() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let accepted = db.accept_webpage_import(&id(), request()).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    for capture in [
        WebCaptureInput {
            http_status: 404,
            ..facts()
        },
        WebCaptureInput {
            final_url: "file:///local".into(),
            ..facts()
        },
        WebCaptureInput {
            content_type: "text/html\nInjected".into(),
            ..facts()
        },
    ] {
        assert!(
            db.capture_webpage_original(
                &accepted.job_id,
                &lease.lease_token,
                &mut Cursor::new(b"html"),
                capture,
                &mut |_| Ok(())
            )
            .is_err()
        );
        assert!(
            db.source_job(&accepted.job_id)
                .unwrap()
                .source_version_id
                .is_none()
        );
    }
}

#[test]
fn response_fact_failure_rolls_back_source_version_and_job_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let accepted = db.accept_webpage_import(&id(), request()).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER capture_failure BEFORE INSERT ON source_web_captures BEGIN SELECT RAISE(ABORT,'injected capture failure'); END;").unwrap();
    assert!(
        db.capture_webpage_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"html"),
            facts(),
            &mut |_| Ok(())
        )
        .is_err()
    );
    assert!(
        db.source_job(&accepted.job_id)
            .unwrap()
            .source_version_id
            .is_none()
    );
    for table in ["sources", "source_versions", "source_web_captures"] {
        assert_eq!(
            raw.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    raw.execute_batch("DROP TRIGGER capture_failure").unwrap();
    assert!(
        db.capture_webpage_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"html"),
            facts(),
            &mut |_| Ok(())
        )
        .is_ok()
    );
}

#[test]
fn original_filename_preserves_spaces_while_display_title_is_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let request = ImportSource {
        input_token: id(),
        original_name: "  原始.txt ".into(),
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    };
    let command = id();
    let accepted = db.accept_source_import(&command, request.clone()).unwrap();
    assert!(
        db.replay_source_import(&command, &request)
            .unwrap()
            .unwrap()
            .receipt
            .replayed
    );
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"body"),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(version.original_name, request.original_name);
    assert_eq!(
        db.source(&version.source_id).unwrap().display_title,
        "原始.txt"
    );
}

#[test]
fn html_title_is_adopted_and_indexed_unless_user_renamed_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    for renamed in [false, true] {
        let accepted = db.accept_webpage_import(&id(), request()).unwrap();
        let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
        let version = db
            .capture_webpage_original(
                &accepted.job_id,
                &lease.lease_token,
                &mut Cursor::new(b"<title>extractedtitle</title>body"),
                facts(),
                &mut |_| Ok(()),
            )
            .unwrap();
        if renamed {
            db.update_source_metadata(
                &id(),
                UpdateSourceMetadata {
                    source_id: version.source_id.clone(),
                    base_row_version: 1,
                    display_title: "mytitle".into(),
                },
            )
            .unwrap();
        }
        let mut parsed = parse_text_source(&version.id, SourceFormat::Text, b"body").unwrap();
        parsed.title = Some(" extractedtitle ".into());
        db.publish_source_parse(&accepted.job_id, &lease.lease_token, parsed)
            .unwrap();
        let expected = if renamed { "mytitle" } else { "extractedtitle" };
        assert_eq!(
            db.source(&version.source_id).unwrap().display_title,
            expected
        );
        assert!(
            db.search_sources(expected, 10)
                .unwrap()
                .iter()
                .any(|hit| hit.source_id == version.source_id)
        );
        assert_eq!(
            db.source_version(&version.id).unwrap().original_name,
            "webpage.html"
        );
    }
}

#[test]
fn restored_workspace_cannot_reuse_original_source_job_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path().join("original")).unwrap();
    for _ in 0..2 {
        db.accept_webpage_import(&id(), request()).unwrap();
    }
    let cursor = db.source_jobs_page(1, None).unwrap().next_cursor.unwrap();
    assert_eq!(
        db.source_jobs_page(1, Some(cursor.clone()))
            .unwrap()
            .items
            .len(),
        1
    );
    let backup = dir.path().join("jobs.pkmsbackup");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let restored = dir.path().join("restored");
    prepare_restore(&backup, &restored, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let copy = Library::open_existing(restored).unwrap();
    assert_eq!(
        copy.info().unwrap().workspace_id,
        db.info().unwrap().workspace_id
    );
    assert!(copy.source_jobs_page(1, Some(cursor)).is_err());
}
