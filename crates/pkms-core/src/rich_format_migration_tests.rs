use super::*;
use crate::{CreateNote, Library, NoteDocument, RestoreNote, SaveNote};
use serde_json::json;

fn old_library(directory: &Path) -> Library {
    std::fs::create_dir_all(directory).unwrap();
    let path = directory.join("library.sqlite3");
    let mut connection = connect(&path, true).unwrap();
    initialize_at(&mut connection, 14).unwrap();
    connection
        .execute_batch("ALTER TABLE notes ADD COLUMN type_key TEXT;")
        .unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(&path))
        .unwrap();
    Library {
        connection,
        database_path: path.canonicalize().unwrap(),
        attachment_previews: Default::default(),
        _lock: lock,
    }
}

fn remove_legacy_write_type_key(library: &Library) {
    library
        .connection
        .execute_batch("ALTER TABLE notes DROP COLUMN type_key;")
        .unwrap();
}

fn request(title: &str) -> CreateNote {
    CreateNote {
        title: title.into(),
        document_schema: 2,
        document: NoteDocument::from_text(Uuid::new_v4().to_string(), "旧正文原样保留"),
        collection_id: None,
        base_collection_row_version: None,
    }
}

fn revisions(
    connection: &Connection,
) -> Vec<(String, String, u32, String, String, Option<String>)> {
    connection.prepare("SELECT revision_id,hex(CAST(document_json AS BLOB)),schema_version,content_hash,title,parent_revision_id FROM note_revisions ORDER BY revision_id").unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))).unwrap()
        .collect::<std::result::Result<_, _>>().unwrap()
}

#[test]
fn rich_format_v14_upgrade_keeps_raw_history_receipts_foreign_keys_and_old_package_restore() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("original");
    let mut old = old_library(&directory);
    let command = Uuid::new_v4().to_string();
    let original_request = request("旧笔记");
    let raw = format!(
        "  {}\n",
        serde_json::to_string_pretty(&original_request.document).unwrap()
    );
    let hash = crate::notes::content_hash(&original_request.title, 2, &raw);
    // A legitimate historical writer retained its original JSON spelling. Its
    // immutable revision and receipt are constructed together, never patched.
    let prepared = crate::notes::Prepared::new(
        &original_request.title,
        2,
        original_request.document.clone(),
        Some(raw.clone()),
    )
    .unwrap();
    let payload_hash =
        crate::notes::request_hash(&command, "notes.create", &original_request).unwrap();
    let tx = old.connection.transaction().unwrap();
    let note =
        crate::notes::commit_revision(&tx, &Uuid::new_v4().to_string(), None, prepared, "human")
            .unwrap();
    let created =
        crate::notes::receipt(&tx, &command, &payload_hash, "notes.create", note).unwrap();
    tx.commit().unwrap();
    let reference: NoteDocument = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":Uuid::new_v4().to_string()},"content":[{"type":"noteReference","attrs":{"target_id":created.note.id,"target_block_id":created.note.document.content[0].attrs.id,"occurrence_id":Uuid::new_v4().to_string(),"label":"稳定引用"}}]}]})).unwrap();
    old.create_document(
        &Uuid::new_v4().to_string(),
        CreateNote {
            document: reference,
            ..request("引用笔记")
        },
    )
    .unwrap();
    remove_legacy_write_type_key(&old);
    let before = revisions(&old.connection);
    let old_package = temporary.path().join("old.pkms");
    let report = old.create_backup(&old_package, &mut |_| Ok(())).unwrap();
    assert_eq!(report.manifest.database_schema, 14);
    assert_eq!(report.manifest.document_schemas, [1, 2]);
    drop(old);

    let mut upgraded = Library::open_existing(&directory).unwrap();
    assert_eq!(
        upgraded.info().unwrap().database_schema,
        CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(revisions(&upgraded.connection), before);
    assert_eq!(
        upgraded
            .connection
            .pragma_query_value::<u32, _>(None, "foreign_keys", |row| row.get(0))
            .unwrap(),
        1
    );
    assert!(
        upgraded
            .connection
            .execute(
                "INSERT INTO note_revision_blocks(revision_id,block_id) VALUES(?1,?2)",
                params![
                    Uuid::new_v4().to_string(),
                    created.note.document.content[0].attrs.id
                ]
            )
            .is_err()
    );
    let replay = upgraded
        .create_document(&command, original_request)
        .unwrap();
    assert_eq!(replay.note.revision_id, created.note.revision_id);
    assert_eq!(replay.note.content_hash, hash);
    assert!(replay.receipt.replayed);
    assert_eq!(revisions(&upgraded.connection), before);

    let document: NoteDocument = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":created.note.document.content[0].attrs.id},"content":[{"type":"text","text":"新格式正文","marks":[{"type":"underline"}]}]}]})).unwrap();
    let saved = upgraded
        .save_document(
            &Uuid::new_v4().to_string(),
            SaveNote {
                note_id: created.note.id.clone(),
                base_revision_id: created.note.revision_id.clone(),
                title: created.note.title.clone(),
                document_schema: 3,
                document,
            },
        )
        .unwrap();
    let preview: String = upgraded
        .connection
        .query_row(
            "SELECT preview FROM note_previews WHERE note_id=?1",
            [&created.note.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preview, "新格式正文");
    let restored = upgraded
        .restore_document(
            &Uuid::new_v4().to_string(),
            RestoreNote {
                note_id: created.note.id.clone(),
                base_revision_id: saved.note.revision_id,
                target_revision_id: created.note.revision_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(restored.note.schema_version, 2);
    assert_eq!(restored.note.content_hash, hash);
    let restored_json: String = upgraded
        .connection
        .query_row(
            "SELECT document_json FROM note_revisions WHERE revision_id=?1",
            [&restored.note.revision_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored_json, raw);
    let new_package = temporary.path().join("new.pkms");
    assert_eq!(
        upgraded
            .create_backup(&new_package, &mut |_| Ok(()))
            .unwrap()
            .manifest
            .document_schemas,
        [1, 2, 3]
    );

    crate::verify_backup(&old_package, &mut |_| Ok(())).unwrap();
    let destination = temporary.path().join("restored-old");
    crate::prepare_restore(&old_package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored_old = Library::open_existing(&destination).unwrap();
    assert_eq!(
        restored_old.info().unwrap().database_schema,
        CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(revisions(&restored_old.connection), before);
    assert_eq!(
        restored_old.note(&created.note.id).unwrap().content_hash,
        hash
    );
}

#[test]
fn rich_format_failed_v14_upgrade_restores_foreign_keys_and_all_revision_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let mut old = old_library(temporary.path());
    old.create_document(&Uuid::new_v4().to_string(), request("故障前正文"))
        .unwrap();
    remove_legacy_write_type_key(&old);
    let before = revisions(&old.connection);
    FAULT.with(|fault| fault.set(Some(Fault::Fail(Checkpoint::AfterMetadata))));
    let result = upgrade(&mut old.connection, &old.database_path);
    FAULT.with(|fault| fault.set(None));
    assert!(result.is_err());
    assert_eq!(
        old.connection
            .pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        14
    );
    assert_eq!(
        old.connection
            .pragma_query_value::<u32, _>(None, "foreign_keys", |row| row.get(0))
            .unwrap(),
        1
    );
    assert_eq!(revisions(&old.connection), before);
    validate_integrity(&old.connection).unwrap();
    upgrade(&mut old.connection, &old.database_path).unwrap();
    assert_eq!(revisions(&old.connection), before);
    validate_integrity(&old.connection).unwrap();
}
