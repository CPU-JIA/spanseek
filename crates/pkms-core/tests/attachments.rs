use image::{DynamicImage, ImageFormat};
use pkms_core::{CopyNote, CreateNote, Error, Library, NoteDocument, RestoreNote, SaveNote};
use rusqlite::Connection;
use serde_json::json;
use std::{io::Cursor, path::Path};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn picture(format: ImageFormat) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(8, 6)
        .write_to(&mut output, format)
        .unwrap();
    output.into_inner()
}
fn image_document(hash: &str, name: &str) -> NoteDocument {
    serde_json::from_value(json!({"type":"doc","content":[
        {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"图片的历史"}]},
        {"type":"image","attrs":{"id":id(),"blob_hash":hash,"original_name":name,"alt":"真实本地图片"}}
    ]})).unwrap()
}
fn blob_path(root: &Path, hash: &str) -> std::path::PathBuf {
    root.join("blobs")
        .join("sha256")
        .join(&hash[..2])
        .join(hash)
}

#[test]
fn supported_image_bytes_are_captured_by_content_and_preview_grants_are_scoped() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    for (format, mime) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = picture(format);
        let command = id();
        let capture = library
            .import_attachment(&command, "名称不可信.txt", &bytes)
            .unwrap();
        assert_eq!(capture.blob.media_type, mime);
        assert_eq!(capture.blob.width, 8);
        assert_eq!(capture.blob.height, 6);
        assert_eq!(
            library.attachment_preview(&capture.preview_token).unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read(blob_path(dir.path(), &capture.blob.hash)).unwrap(),
            bytes
        );
        let retry = library
            .import_attachment(&command, "名称不可信.txt", &bytes)
            .unwrap();
        assert!(retry.replayed);
        assert_eq!(retry.change_seq, capture.change_seq);
        assert_eq!(retry.blob, capture.blob);
        assert!(matches!(
            library.import_attachment(&command, "改名.txt", &bytes),
            Err(Error::IdempotencyMismatch)
        ));
        assert!(matches!(
            library.attachment(&id(), &capture.blob.hash),
            Err(Error::NotFound)
        ));
        let other = tempdir().unwrap();
        let isolated = Library::open(other.path()).unwrap();
        assert!(isolated.attachment_preview(&capture.preview_token).is_err());
    }
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM blobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn removing_current_image_preserves_history_and_copy_reuses_immutable_bytes() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let bytes = picture(ImageFormat::Png);
    let capture = library.import_attachment(&id(), "图.png", &bytes).unwrap();
    let command = id();
    let request = CreateNote {
        collection_id: None,
        base_collection_row_version: None,
        title: "历史图片".into(),
        document_schema: 2,
        document: image_document(&capture.blob.hash, "图.png"),
    };
    let first = library
        .create_document(&command, request.clone())
        .unwrap()
        .note;
    let copy = library
        .copy_document(
            &id(),
            CopyNote {
                note_id: first.id.clone(),
                source_revision_id: first.revision_id.clone(),
                title: "副本".into(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        library
            .attachment(&copy.revision_id, &capture.blob.hash)
            .unwrap()
            .1,
        bytes
    );
    assert_ne!(
        copy.document.content[1].attrs.id,
        first.document.content[1].attrs.id
    );
    let removed = library
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: first.revision_id.clone(),
                title: first.title.clone(),
                document_schema: 2,
                document: NoteDocument::from_text(
                    first.document.content[0].attrs.id.clone(),
                    "当前移除图片",
                ),
            },
        )
        .unwrap()
        .note;
    assert!(matches!(
        library.attachment(&removed.revision_id, &capture.blob.hash),
        Err(Error::NotFound)
    ));
    assert_eq!(
        library
            .attachment(&first.revision_id, &capture.blob.hash)
            .unwrap()
            .1,
        bytes
    );
    let restored = library
        .restore_document(
            &id(),
            RestoreNote {
                note_id: first.id.clone(),
                base_revision_id: removed.revision_id,
                target_revision_id: first.revision_id.clone(),
            },
        )
        .unwrap()
        .note;
    library.set_trashed(&first.id, true).unwrap();
    assert_eq!(
        library
            .attachment(&restored.revision_id, &capture.blob.hash)
            .unwrap()
            .1,
        bytes
    );
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM blobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM note_revision_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(library);
    let mut reopened = Library::open_existing(dir.path()).unwrap();
    assert!(reopened.attachment_preview(&capture.preview_token).is_err());
    assert_eq!(
        reopened
            .attachment(&first.revision_id, &capture.blob.hash)
            .unwrap()
            .1,
        bytes
    );
    assert!(
        reopened
            .create_document(&command, request)
            .unwrap()
            .receipt
            .replayed
    );
}

#[test]
fn invalid_input_and_storage_failures_never_report_success_or_overwrite_blobs() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let bytes = picture(ImageFormat::Png);
    let mut wide = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(16_385, 1)
        .write_to(&mut wide, ImageFormat::Png)
        .unwrap();
    assert!(
        library
            .import_attachment(&id(), "过宽.png", wide.get_ref())
            .is_err()
    );
    assert!(
        library
            .import_attachment(&id(), "假.png", b"not an image")
            .is_err()
    );
    assert!(
        library
            .import_attachment(&id(), "断.png", &bytes[..bytes.len() / 2])
            .is_err()
    );
    assert!(
        library
            .import_attachment(&id(), "../out.png", &bytes)
            .is_err()
    );
    assert!(
        library
            .import_attachment(&id(), "大.png", &vec![0; 20 * 1024 * 1024 + 1])
            .is_err()
    );
    // A real filesystem conflict blocks capture. It is not a simulated success path.
    std::fs::write(dir.path().join("blobs"), b"occupied").unwrap();
    let command = id();
    assert!(
        library
            .import_attachment(&command, "图.png", &bytes)
            .is_err()
    );
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM command_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    std::fs::rename(dir.path().join("blobs"), dir.path().join("occupied-file")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT,'test import commit failure'); END;").unwrap();
    assert!(
        library
            .import_attachment(&command, "图.png", &bytes)
            .is_err()
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM blobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    raw.execute_batch("DROP TRIGGER fail_import;").unwrap();
    let capture = library
        .import_attachment(&command, "图.png", &bytes)
        .unwrap();
    assert!(!capture.replayed);
    assert_eq!(capture.change_seq, "1");
    let path = blob_path(dir.path(), &capture.blob.hash);
    std::fs::write(&path, b"corrupted").unwrap();
    assert!(library.attachment_preview(&capture.preview_token).is_err());
    assert!(library.import_attachment(&id(), "图.png", &bytes).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupted");
    assert!(
        library
            .create_document(
                &id(),
                CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: "坏图".into(),
                    document_schema: 2,
                    document: image_document(&capture.blob.hash, "图.png")
                }
            )
            .is_err()
    );
    assert!(library.list_notes().unwrap().is_empty());
}

#[test]
fn simultaneous_capture_of_one_command_publishes_one_blob_and_one_receipt() {
    let dir = tempdir().unwrap();
    let _library = Library::open(dir.path()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let command = id();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let command = command.clone();
            let path = dir.path().to_owned();
            std::thread::spawn(move || {
                let mut library = Library::open_existing(path).unwrap();
                let bytes = picture(ImageFormat::Png);
                barrier.wait();
                library
                    .import_attachment(&command, "同时.png", &bytes)
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results[0].blob, results[1].blob);
    assert_eq!(results[0].change_seq, results[1].change_seq);
    assert_eq!(results.iter().filter(|result| result.replayed).count(), 1);
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM blobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM command_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        std::fs::read_dir(
            blob_path(dir.path(), &results[0].blob.hash)
                .parent()
                .unwrap()
        )
        .unwrap()
        .count(),
        1
    );
}

#[test]
fn shared_command_namespace_and_replay_survive_attachment_loss_without_new_note_commits() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let bytes = picture(ImageFormat::WebP);
    let command = id();
    let capture = library
        .import_attachment(&command, "图.webp", &bytes)
        .unwrap();
    let create = CreateNote {
        collection_id: None,
        base_collection_row_version: None,
        title: "图笔记".into(),
        document_schema: 2,
        document: image_document(&capture.blob.hash, "图.webp"),
    };
    assert!(matches!(
        library.create_document(&command, create.clone()),
        Err(Error::IdempotencyMismatch)
    ));
    let note_command = id();
    let note = library
        .create_document(&note_command, create.clone())
        .unwrap();
    assert!(matches!(
        library.import_attachment(&note_command, "图.webp", &bytes),
        Err(Error::IdempotencyMismatch)
    ));
    let path = blob_path(dir.path(), &capture.blob.hash);
    std::fs::rename(&path, path.with_extension("displaced")).unwrap();
    let retry = library.create_document(&note_command, create).unwrap();
    assert!(retry.receipt.replayed);
    assert_eq!(retry.note, note.note);
    assert!(
        library
            .attachment(&note.note.revision_id, &capture.blob.hash)
            .is_err()
    );
    assert_eq!(library.revisions(&note.note.id).unwrap().len(), 1);
}

#[test]
fn failed_note_receipt_rolls_back_revision_blob_references() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let capture = library
        .import_attachment(&id(), "图.png", &picture(ImageFormat::Png))
        .unwrap();
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_note BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT,'test note receipt failure'); END;").unwrap();
    let command = id();
    let create = CreateNote {
        collection_id: None,
        base_collection_row_version: None,
        title: "图笔记".into(),
        document_schema: 2,
        document: image_document(&capture.blob.hash, "图.png"),
    };
    assert!(library.create_document(&command, create.clone()).is_err());
    assert_eq!(
        raw.query_row("SELECT count(*) FROM note_revision_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(library.list_notes().unwrap().is_empty());
    raw.execute_batch("DROP TRIGGER fail_note;").unwrap();
    let note = library.create_document(&command, create).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM note_revision_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        library
            .attachment(&note.note.revision_id, &capture.blob.hash)
            .is_ok()
    );
}
