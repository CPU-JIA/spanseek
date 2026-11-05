use pkms_core::*;
use std::io::{Cursor, Read};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn capture(
    db: &mut Library,
    bytes: &[u8],
    format: SourceFormat,
    previous: Option<&SourceVersion>,
) -> SourceVersion {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: if format == SourceFormat::Markdown {
                    "中文.md"
                } else {
                    "中文.txt"
                }
                .into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: previous.map(|v| v.source_id.clone()),
                base_row_version: previous.map(|v| db.source(&v.source_id).unwrap().row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.cancel_source_job(&accepted.job_id).unwrap();
    version
}
fn request(version: &SourceVersion) -> CreateSourceNote {
    CreateSourceNote {
        source_version_id: version.id.clone(), source_blob_hash: version.blob_hash.clone(),
        title: "导入的结构".into(), document_schema: DOCUMENT_SCHEMA,
        document: serde_json::from_value(serde_json::json!({"type":"doc","content":[
            {"type":"heading","attrs":{"id":id(),"level":2},"content":[{"type":"text","text":"标题"}]},
            {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"保留粗体","marks":[{"type":"bold"}]}]},
            {"type":"codeBlock","attrs":{"id":id(),"language":"rs"},"content":[{"type":"text","text":"let n = 1;"}]}
        ]})).unwrap(),
    }
}

#[test]
fn source_note_preserves_raw_input_rich_revision_pinned_origin_and_restore() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let bytes = "\u{feff}## 标题\r\n\r\n**保留粗体**\r\n\r\n```rs\r\nlet n = 1;\r\n```".as_bytes();
    let version = capture(&mut db, bytes, SourceFormat::Markdown, None);
    let newer = capture(
        &mut db,
        b"newer original",
        SourceFormat::Markdown,
        Some(&version),
    );
    assert_ne!(version.id, newer.id);
    assert_eq!(
        db.source_note_input(&version.id).unwrap().text.as_bytes(),
        bytes
    );
    let input = request(&version);
    let command = id();
    let created = db.create_source_note(&command, input.clone()).unwrap();
    assert_eq!(created.note.document.content[..3], input.document.content);
    let origin = &created.note.document.content[3].content[0].attrs;
    assert_eq!(
        origin.target_id.as_deref(),
        Some(version.source_id.as_str())
    );
    assert_eq!(
        origin.target_revision_id.as_deref(),
        Some(version.id.as_str())
    );
    let resolved = db
        .resolve_link(ResolveLink {
            target_id: version.source_id.clone(),
            target_revision_id: Some(version.id.clone()),
            target_block_id: None,
        })
        .unwrap();
    assert_eq!(resolved.state, LinkState::Available);
    assert_eq!(
        resolved.source_target.unwrap().source_version_id,
        version.id
    );
    let mut original = vec![];
    db.source_original_file(&version.id)
        .unwrap()
        .1
        .read_to_end(&mut original)
        .unwrap();
    assert_eq!(original, bytes);
    let updated = db
        .save_document(
            &id(),
            SaveNote {
                note_id: created.note.id.clone(),
                base_revision_id: created.note.revision_id.clone(),
                title: "应用内修改".into(),
                document_schema: DOCUMENT_SCHEMA,
                document: NoteDocument::from_text(id(), "新正文"),
            },
        )
        .unwrap();
    let replay = db.create_source_note(&command, input.clone()).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.note.document, created.note.document);
    assert_eq!(
        db.note(&created.note.id).unwrap().revision_id,
        updated.note.revision_id
    );
    let archive = root.path().join("import.zip");
    db.create_backup(&archive, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&archive, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(&destination).unwrap();
    assert_eq!(
        restored
            .source_note_input(&version.id)
            .unwrap()
            .text
            .as_bytes(),
        bytes
    );
    assert_eq!(
        restored
            .create_source_note(&command, input)
            .unwrap()
            .note
            .document,
        created.note.document
    );
    assert_eq!(
        restored
            .revision(&created.note.id, &created.note.revision_id)
            .unwrap()
            .document,
        created.note.document
    );
}

#[test]
fn source_note_receipt_failure_rolls_back_note_projection_and_can_replay_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let version = capture(&mut db, b"source text", SourceFormat::Text, None);
    let input = request(&version);
    let command = id();
    let sql = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_source_note BEFORE INSERT ON command_receipts WHEN NEW.command_kind='sources.create_note' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    assert!(db.create_source_note(&command, input.clone()).is_err());
    for table in ["notes", "note_revisions", "body_links"] {
        let count: i64 = sql
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    sql.execute_batch("DROP TRIGGER reject_source_note")
        .unwrap();
    let created = db.create_source_note(&command, input.clone()).unwrap();
    drop(db);
    let mut reopened = Library::open_existing(root.path()).unwrap();
    let replay = reopened
        .create_source_note(&command, input.clone())
        .unwrap();
    assert_eq!(replay.note.id, created.note.id);
    assert!(replay.receipt.replayed);
    let mut changed = input;
    changed.title.push('改');
    assert!(matches!(
        reopened.create_source_note(&command, changed),
        Err(Error::IdempotencyMismatch)
    ));
    assert_eq!(
        sql.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn source_note_rejects_bad_encoding_size_hash_and_trashed_input_without_creating_notes() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let invalid = capture(&mut db, &[0xff, 0xfe, b'x'], SourceFormat::Text, None);
    assert!(matches!(
        db.source_note_input(&invalid.id),
        Err(Error::InvalidInput(_))
    ));
    let oversized = capture(
        &mut db,
        &vec![b'a'; 1024 * 1024 + 1],
        SourceFormat::Text,
        None,
    );
    assert!(matches!(
        db.source_note_input(&oversized.id),
        Err(Error::InvalidInput(_))
    ));
    let version = capture(&mut db, b"valid source", SourceFormat::Text, None);
    let mut input = request(&version);
    input.source_blob_hash = "0".repeat(64);
    assert!(matches!(
        db.create_source_note(&id(), input),
        Err(Error::InvalidInput(_))
    ));
    let input = request(&version);
    let command = id();
    let created = db.create_source_note(&command, input.clone()).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: version.source_id.clone(),
            base_row_version: db.source(&version.source_id).unwrap().row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(matches!(
        db.create_source_note(&id(), input.clone()),
        Err(Error::Trashed)
    ));
    assert_eq!(
        db.create_source_note(&command, input).unwrap().note.id,
        created.note.id
    );
    let blob = root
        .path()
        .join("blobs/sha256")
        .join(&invalid.blob_hash[..2])
        .join(&invalid.blob_hash);
    std::fs::write(blob, b"abc").unwrap();
    assert!(matches!(
        db.source_note_input(&invalid.id),
        Err(Error::IntegrityFailure)
    ));
}
