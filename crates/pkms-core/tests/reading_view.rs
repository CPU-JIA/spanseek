mod common;

use pkms_core::*;
use rusqlite::Connection;
use serde_json::json;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn source(db: &mut Library, format: SourceFormat) -> SourceVersion {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: if format == SourceFormat::Pdf {
                    "synthetic.pdf"
                } else {
                    "synthetic.txt"
                }
                .into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    // Geometry is a controlled worker result; native rendering is tested elsewhere.
    let bytes = if format == SourceFormat::Pdf {
        b"%PDF-1.4\n% synthetic core reading-state fixture\n%%EOF".as_slice()
    } else {
        b"text fixture".as_slice()
    };
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    let parsed = if format == SourceFormat::Pdf {
        ParsedSource {
            title: None,
            source_version_id: version.id.clone(),
            parser_version: "synthetic-three-pages".into(),
            author: None,
            blocks: vec![],
            page_count: Some(3),
            needs_ocr: true,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        }
    } else {
        parse_text_source(&version.id, format, bytes).unwrap()
    };
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
        .unwrap();
    version
}
fn view() -> PdfReadingView {
    PdfReadingView {
        page_index: 1,
        anchor_x: 0.42,
        anchor_y: 0.73,
        scale: 1.6,
        rotation: 90,
        zoom_mode: PdfZoomMode::Custom,
    }
}

#[test]
fn legacy_pdf_view_without_zoom_mode_defaults_to_responsive_width() {
    let legacy: PdfReadingView = serde_json::from_value(json!({
        "page_index": 1,
        "anchor_x": 0.42,
        "anchor_y": 0.73,
        "scale": 0.6,
        "rotation": 0
    }))
    .unwrap();
    assert_eq!(legacy.zoom_mode, PdfZoomMode::Width);
}
fn request(v: &SourceVersion) -> SetReadingState {
    SetReadingState {
        source_id: v.source_id.clone(),
        source_version_id: v.id.clone(),
        locator: None,
        status: ReadingStatus::Reading,
        base_row_version: 0,
        pdf_view: Some(view()),
    }
}

#[test]
fn pdf_view_roundtrips_replays_and_restores_without_rewriting_the_original() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("library")).unwrap();
    let v = source(&mut db, SourceFormat::Pdf);
    let command = id();
    let input = request(&v);
    db.set_reading_state(&command, input.clone()).unwrap();
    let package = root.path().join("reading.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    drop(db);
    let mut db = Library::open_existing(root.path().join("library")).unwrap();
    assert_eq!(
        db.reading_state(&v.source_id).unwrap().pdf_view,
        Some(view())
    );
    assert!(
        db.set_reading_state(&command, input.clone())
            .unwrap()
            .receipt
            .replayed
    );
    assert!(matches!(
        db.set_reading_state(&id(), input),
        Err(Error::MetadataConflict { .. })
    ));
    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&restored_path).unwrap();
    assert_eq!(
        restored.reading_state(&v.source_id).unwrap().pdf_view,
        Some(view())
    );
    assert_eq!(
        restored.source_version(&v.id).unwrap().blob_hash,
        v.blob_hash
    );
}

#[test]
fn pdf_view_rejects_wrong_source_invalid_geometry_page_and_version_without_a_write() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let pdf = source(&mut db, SourceFormat::Pdf);
    let text = source(&mut db, SourceFormat::Text);
    assert!(matches!(
        db.set_reading_state(&id(), request(&text)),
        Err(Error::InvalidInput(_))
    ));
    for value in [
        PdfReadingView {
            scale: 0.1,
            ..view()
        },
        PdfReadingView {
            anchor_x: 1.1,
            ..view()
        },
        PdfReadingView {
            anchor_y: -0.1,
            ..view()
        },
        PdfReadingView {
            rotation: 45,
            ..view()
        },
        PdfReadingView {
            page_index: 3,
            ..view()
        },
    ] {
        assert!(matches!(
            db.set_reading_state(
                &id(),
                SetReadingState {
                    pdf_view: Some(value),
                    ..request(&pdf)
                }
            ),
            Err(Error::InvalidInput(_))
        ));
    }
    let mut other = request(&pdf);
    other.source_id = text.source_id.clone();
    assert!(matches!(
        db.set_reading_state(&id(), other),
        Err(Error::InvalidInput(_))
    ));
    let locator = SourceLocator::Pdf {
        source_version_id: pdf.id.clone(),
        page_index: 0,
        page_label: None,
        crop_box: [0.0, 0.0, 600.0, 800.0],
        rotation: 0,
        coordinate_space: "pdf_user_space".into(),
        rects: vec![],
        quote: String::new(),
        prefix: String::new(),
        suffix: String::new(),
    };
    assert!(matches!(
        db.set_reading_state(
            &id(),
            SetReadingState {
                locator: Some(locator),
                ..request(&pdf)
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(db.reading_state(&pdf.source_id).unwrap().row_version, 0);
}

#[test]
fn pdf_view_can_save_before_text_parsing_without_claiming_an_exact_locator() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "pending.pdf".into(),
                format: SourceFormat::Pdf,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let v = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"%PDF-1.4\n% pending parser"),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(db.source(&v.source_id).unwrap().active_parse_id.is_none());
    let saved = db
        .set_reading_state(&id(), request(&v))
        .unwrap()
        .reading
        .unwrap();
    assert_eq!(saved.pdf_view, Some(view()));
    assert!(saved.locator.is_none());
    assert!(saved.parse_id.is_none());
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        ParsedSource {
            title: None,
            source_version_id: v.id,
            parser_version: "synthetic-three-pages".into(),
            author: None,
            blocks: vec![],
            page_count: Some(3),
            needs_ocr: true,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(
        db.reading_state(&v.source_id).unwrap().pdf_view,
        Some(view())
    );
}

#[test]
fn k16_upgrade_keeps_old_reading_receipt_hash_and_frozen_history_then_rejects_future_schema() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("迁移原文", "不得改写历史正文").unwrap();
    let v = source(&mut db, SourceFormat::Pdf);
    let command = id();
    let old_json = json!({"source_id":v.source_id,"source_version_id":v.id,"locator":null,"status":"reading","base_row_version":0});
    let old_request: SetReadingState = serde_json::from_value(old_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&old_request).unwrap(), old_json);
    db.set_reading_state(&command, old_request.clone()).unwrap();
    drop(db);
    let path = root.path().join("library.sqlite3");
    let raw = Connection::open(&path).unwrap();
    // Build the exact immediately preceding schema only inside this tempdir.
    raw.execute_batch(&format!("{versions} DROP TABLE object_tombstones; ALTER TABLE reading_states DROP COLUMN pdf_view_json; DELETE FROM migration_history WHERE version=17; ALTER TABLE search_settings DROP COLUMN retrieval_params_json; ALTER TABLE search_settings DROP COLUMN retrieval_row_version; DELETE FROM migration_history WHERE version=18; DROP TABLE retrieval_events; DROP TABLE model_usage_events; DELETE FROM migration_history WHERE version=19; DELETE FROM migration_history WHERE version=20; ALTER TABLE sources DROP COLUMN author; DELETE FROM migration_history WHERE version=21; ALTER TABLE search_settings DROP COLUMN cleaning_params_json; ALTER TABLE search_settings DROP COLUMN cleaning_row_version; ALTER TABLE source_parses DROP COLUMN cleaning_json; DELETE FROM migration_history WHERE version=22; ALTER TABLE search_settings DROP COLUMN chunk_params_json; ALTER TABLE search_settings DROP COLUMN chunk_row_version; ALTER TABLE search_settings DROP COLUMN previous_profile_id; DELETE FROM migration_history WHERE version=23; DELETE FROM migration_history WHERE version=24; {views} {automation} UPDATE workspace_meta SET schema_version=16; PRAGMA user_version=16;", versions = common::pre_docx_source_versions(), views = common::pre_views_canvas_versions(), automation = common::pre_automation_versions())).unwrap();
    let history = raw
        .prepare("SELECT version,checksum,applied_at FROM migration_history ORDER BY version")
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let receipt = raw
        .query_row(
            "SELECT payload_hash,result_json,change_seq FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    drop(raw);
    let mut upgraded = Library::open_existing(root.path()).unwrap();
    assert_eq!(
        upgraded.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(upgraded.note(&note.id).unwrap().document, note.document);
    assert!(
        upgraded
            .reading_state(&v.source_id)
            .unwrap()
            .pdf_view
            .is_none()
    );
    assert!(
        upgraded
            .set_reading_state(&command, old_request)
            .unwrap()
            .receipt
            .replayed
    );
    let raw = Connection::open(&path).unwrap();
    let actual=raw.prepare("SELECT version,checksum,applied_at FROM migration_history WHERE version<=16 ORDER BY version").unwrap().query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
    assert_eq!(actual, history);
    assert_eq!(
        raw.query_row(
            "SELECT payload_hash,result_json,change_seq FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        receipt
    );
    drop(upgraded);
    raw.execute_batch(&format!(
        "UPDATE workspace_meta SET schema_version={0}; PRAGMA user_version={0};",
        pkms_core::CURRENT_SCHEMA_VERSION + 1
    ))
    .unwrap();
    assert!(matches!(
        Library::open_existing(root.path()),
        Err(Error::UnsupportedDatabase)
    ));
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        pkms_core::CURRENT_SCHEMA_VERSION as u32 + 1
    );
}
