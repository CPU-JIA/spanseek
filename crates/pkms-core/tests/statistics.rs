use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn count(entries: &[StatisticCount], key: &str) -> u64 {
    entries.iter().find(|entry| entry.key == key).unwrap().count
}

#[test]
fn statistics_follow_real_inventory_without_disclosing_private_fields() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let empty = db.workspace_statistics().unwrap();
    assert_eq!(empty.blobs, 0);
    assert_eq!(empty.blob_bytes, 0);
    assert_eq!(empty.note_revisions, 0);
    assert!(empty.objects.iter().all(|entry| entry.count == 0));
    assert_eq!(empty.integrity_check, IntegrityCheckState::NotRun);
    let marker = "PRIVATE_7f6242_user_material";
    let note = db.create_note(marker, marker).unwrap();
    db.save_note(
        &note.id,
        note.revision,
        marker,
        &format!("{marker} updated"),
    )
    .unwrap();
    db.create_collection(marker).unwrap();
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: format!("{marker}.txt"),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let before_capture = db.workspace_statistics().unwrap();
    assert_eq!(count(&before_capture.source_jobs, "queued"), 1);
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(marker.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, marker.as_bytes()).unwrap(),
    )
    .unwrap();
    db.accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
        .unwrap();
    db.accept_answer(
        &id(),
        StartAnswer {
            max_output_tokens: None,
            citation_count: None,
            agent: None,
            search: SearchQuery {
                query: marker.into(),
                mode: SearchMode::Lexical,
                scope: SearchScope::default(),
                limit: 10,
                minimum_cosine: None,
            },
            model: None,
        },
    )
    .unwrap();
    let stats = db.workspace_statistics().unwrap();
    assert_eq!(count(&stats.objects, "note"), 1);
    assert_eq!(count(&stats.objects, "collection"), 1);
    assert_eq!(count(&stats.objects, "source"), 1);
    assert_eq!(count(&stats.sources_by_format, "text"), 1);
    assert_eq!(count(&stats.source_jobs, "succeeded"), 1);
    assert_eq!(count(&stats.source_jobs, "running"), 0);
    assert_eq!(count(&stats.portable_jobs, "queued"), 1);
    assert_eq!(
        stats.answers.iter().map(|entry| entry.count).sum::<u64>(),
        1
    );
    assert_eq!(stats.note_revisions, 2);
    assert_eq!(stats.blobs, 1);
    assert_eq!(stats.blob_bytes, marker.len() as u64);
    assert_eq!(stats.integrity_check, IntegrityCheckState::NotRun);
    let encoded = serde_json::to_string(&stats).unwrap();
    assert!(!encoded.contains(marker));
    assert!(!encoded.contains(&note.id));
    assert!(!encoded.contains(&version.id));
    assert!(!encoded.contains(root.path().to_string_lossy().as_ref()));
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
    let trashed = db.workspace_statistics().unwrap();
    assert_eq!(count(&trashed.objects, "source"), 1);
    assert_eq!(count(&trashed.sources_by_format, "text"), 1);
    assert_eq!(trashed.blob_bytes, stats.blob_bytes);
}

#[test]
fn statistics_report_overflow_instead_of_wrapping_or_rounding() {
    let root = tempdir().unwrap();
    let db = Library::open(root.path()).unwrap();
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    // Controlled malformed inventory fixture: statistics must not silently round
    // sizes or pretend that it has checked these absent original files.
    raw.execute("INSERT INTO blobs(hash,size,media_type,created_at) VALUES(?1,?2,'text/plain','2026-10-03T00:00:00Z')",
        rusqlite::params!["a".repeat(64), i64::MAX]).unwrap();
    assert!(matches!(
        db.workspace_statistics(),
        Err(Error::InvalidInput(_))
    ));
    raw.execute("INSERT INTO blobs(hash,size,media_type,created_at) VALUES(?1,?2,'text/plain','2026-10-03T00:00:00Z')",
        rusqlite::params!["b".repeat(64), i64::MAX]).unwrap();
    assert!(db.workspace_statistics().is_err());
}

#[test]
fn unexpected_status_is_not_returned_as_a_statistics_key() {
    let root = tempdir().unwrap();
    let db = Library::open(root.path()).unwrap();
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute("INSERT INTO answers(id,request_json,status,created_at,updated_at) VALUES(?1,'{}','PRIVATE_STATUS_MARKER','2026-10-03T00:00:00Z','2026-10-03T00:00:00Z')", [id()]).unwrap();
    assert!(matches!(
        db.workspace_statistics(),
        Err(Error::IntegrityFailure)
    ));
}

#[test]
fn diagnostic_errors_are_bounded_sorted_and_use_closed_vocabularies() {
    let root = tempdir().unwrap();
    let db = Library::open(root.path()).unwrap();
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    let private = "private_customer_document_marker";
    for (sql, code, time) in [
        (
            "INSERT INTO source_jobs(id,input_json,status,stage,error_code,created_at,updated_at) VALUES(?1,?2,'failed',?3,?4,?5,?5)",
            "worker_timeout",
            "2026-10-03T12:00:00+08:00",
        ),
        (
            "INSERT INTO portable_jobs(id,input_json,status,stage,error_code,created_at,updated_at) VALUES(?1,?2,'failed',?3,?4,?5,?5)",
            private,
            "2026-10-03T03:00:00Z",
        ),
        (
            "INSERT INTO index_rebuild_jobs(id,request_json,status,error_code,created_at,updated_at) VALUES(?1,?2,'failed',?4,?5,?5)",
            "embedding_model_unavailable",
            "2026-10-03T05:00:00Z",
        ),
        (
            "INSERT INTO answers(id,request_json,status,text,error_code,created_at,updated_at) VALUES(?1,?2,'failed',?3,?4,?5,?5)",
            "model_unavailable",
            "2026-10-03T02:00:00Z",
        ),
    ] {
        raw.execute(
            sql,
            rusqlite::params![
                id(),
                serde_json::json!({"private":private}).to_string(),
                private,
                code,
                time
            ],
        )
        .unwrap();
    }
    let errors = db.diagnostic_errors(100).unwrap();
    assert_eq!(errors.len(), 4);
    assert_eq!(
        errors
            .iter()
            .map(|error| error.component.as_str())
            .collect::<Vec<_>>(),
        vec!["index", "source", "portable", "answer"]
    );
    assert_eq!(errors[1].occurred_at, "2026-10-03T04:00:00+00:00");
    assert_eq!(errors[1].code, "worker_timeout");
    assert_eq!(errors[2].code, "unknown_error");
    assert!(errors.iter().all(|error| error.stage == "failed"));
    assert!(!serde_json::to_string(&errors).unwrap().contains(private));
    assert_eq!(db.diagnostic_errors(1).unwrap().len(), 1);
    assert!(db.diagnostic_errors(0).is_err());
    assert!(db.diagnostic_errors(101).is_err());

    raw.execute("UPDATE answers SET status=?1, error_code=?1", [private])
        .unwrap();
    let errors = db.diagnostic_errors(100).unwrap();
    let answer = errors
        .iter()
        .find(|error| error.component == "answer")
        .unwrap();
    assert_eq!(answer.stage, "unknown_state");
    assert_eq!(answer.code, "unknown_error");
    raw.execute("UPDATE answers SET id=?1", [private]).unwrap();
    raw.execute("UPDATE portable_jobs SET updated_at=?1", [private])
        .unwrap();
    assert_eq!(db.diagnostic_errors(100).unwrap().len(), 2);
    for _ in 0..105 {
        raw.execute("INSERT INTO answers(id,request_json,status,error_code,created_at,updated_at) VALUES(?1,'{}','failed','model_unavailable','2026-10-04T00:00:00Z','2026-10-04T00:00:00Z')", [id()]).unwrap();
    }
    let errors = db.diagnostic_errors(100).unwrap();
    assert_eq!(errors.len(), 100);
    assert!(errors.windows(2).all(|items| items[0].id < items[1].id));
}
/// A Word document is a first-class source format. Its stored value has to be
/// part of the statistics vocabulary: the inventory of a library that holds a
/// DOCX must not fail as an integrity error.
#[test]
fn a_word_document_is_counted_instead_of_failing_the_inventory() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "report.docx".into(),
                format: SourceFormat::Docx,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"PK\x03\x04wordprocessingml".as_slice()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        ParsedSource {
            title: Some("report".into()),
            author: None,
            source_version_id: version.id.clone(),
            parser_version: "ooxml-wordprocessing-v1".into(),
            blocks: vec![SourceBlock {
                id: id(),
                text: "Word body".into(),
                page_index: None,
                pdf_geometry: None,
            }],
            page_count: None,
            needs_ocr: false,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
    )
    .unwrap();
    let stats = db.workspace_statistics().unwrap();
    assert_eq!(count(&stats.sources_by_format, "docx"), 1);
    assert_eq!(count(&stats.objects, "source"), 1);
}
