mod common;

use pkms_core::*;
const DROP_REVIEW: &str = "DROP TABLE chat_run_events; DROP TABLE chat_messages; DROP TABLE chat_runs; DROP TABLE chat_conversations; DELETE FROM migration_history WHERE version=14; DROP TABLE review_events; DROP TABLE review_entries; DELETE FROM migration_history WHERE version=13;";
const DROP_ASSISTANT_INDEX: &str = "DROP TABLE chat_run_events; DROP TABLE chat_messages; DROP TABLE chat_runs; DROP TABLE chat_conversations; DELETE FROM migration_history WHERE version=14; DROP TABLE review_events; DROP TABLE review_entries; DELETE FROM migration_history WHERE version=13; DROP TRIGGER index_dirty_note_insert; DROP TRIGGER index_dirty_note_update; DROP TRIGGER index_dirty_source_insert; DROP TRIGGER index_dirty_source_update; DROP TRIGGER index_dirty_object_update; DROP TRIGGER index_dirty_parse_insert; DROP TRIGGER index_dirty_parse_update; DROP TABLE index_dirty_objects; ALTER TABLE body_links DROP COLUMN target_revision_id; DROP TABLE index_rebuild_objects; DROP TABLE index_rebuild_jobs; DROP TABLE proposal_blobs; DROP TABLE proposals; DROP TABLE answer_feedback; DROP TABLE answer_events; DROP TABLE answers; DROP TABLE active_index_generations; DROP TABLE index_chunks; DROP TABLE index_generations; DROP TABLE search_settings; DROP TABLE embedding_profiles; ALTER TABLE source_job_events DROP COLUMN attempt; DELETE FROM migration_history WHERE version=12;";
const DROP_SOURCE_TITLES: &str = "DROP TABLE source_title_fts; DROP TRIGGER source_reading_insert; DROP TRIGGER source_reading_update; DROP TRIGGER source_reading_delete; DROP TRIGGER source_tag_insert; DROP TRIGGER source_tag_update; DROP TRIGGER source_tag_delete; DROP TRIGGER source_membership_insert; DROP TRIGGER source_membership_update; DROP TRIGGER source_membership_delete; DELETE FROM migration_history WHERE version=11;";
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::Path,
};
use tempfile::tempdir;
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn ok(_: &str) -> Result<()> {
    Ok(())
}
fn png() -> Vec<u8> {
    let mut b = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(5, 7)
        .write_to(&mut b, image::ImageFormat::Png)
        .unwrap();
    b.into_inner()
}
fn image_doc(hash: &str) -> NoteDocument {
    serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"历史图片"}]},{"type":"image","attrs":{"id":id(),"blob_hash":hash,"original_name":"图.png","alt":"图片 <&>"}}]})).unwrap()
}
fn unpack(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut z = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut bytes = vec![];
            f.read_to_end(&mut bytes).unwrap();
            (f.name().to_owned(), bytes)
        })
        .collect()
}
fn pack(path: &Path, files: &BTreeMap<String, Vec<u8>>) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in files {
        z.start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap();
}
fn update_manifest(
    files: &mut BTreeMap<String, Vec<u8>>,
    change: impl FnOnce(&mut BackupManifest),
) {
    let mut m: BackupManifest = serde_json::from_slice(&files["manifest.json"]).unwrap();
    change(&mut m);
    let b = serde_json::to_vec(&m).unwrap();
    files.insert(
        "COMPLETE".into(),
        format!("{:x}", Sha256::digest(&b)).into_bytes(),
    );
    files.insert("manifest.json".into(), b);
}
fn change_database(files: &mut BTreeMap<String, Vec<u8>>, sql: &str) {
    let t = tempdir().unwrap();
    let p = t.path().join("db");
    fs::write(&p, &files["library.sqlite3"]).unwrap();
    {
        let db = rusqlite::Connection::open(&p).unwrap();
        db.execute_batch(sql).unwrap();
        // A backed-up database can still be in WAL mode. Flush the fixture
        // rewrite before reading the main file, otherwise user_version can
        // appear stale to the exact-schema package verifier.
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    }
    let b = fs::read(p).unwrap();
    update_manifest(files, |m| {
        m.database.sha256 = format!("{:x}", Sha256::digest(&b));
        m.database.size = b.len() as u64;
    });
    files.insert("library.sqlite3".into(), b);
}

// These fixtures model published pre-D3 packages, not a supported user downgrade.
// Reuse frozen DDL verbatim so the exact DDL validator still checks real old schemas.
fn pre_d3_fixture(files: &mut BTreeMap<String, Vec<u8>>) {
    let table = include_str!("../src/migrations/0003_rich_notes.sql")
        .split_once("CREATE TABLE note_revisions")
        .unwrap()
        .1
        .split_once(';')
        .unwrap()
        .0;
    let trigger = include_str!("../src/migrations/0005_note_queries.sql")
        .split_once("CREATE TRIGGER note_preview_changed")
        .unwrap()
        .1
        .split_once("END;")
        .unwrap()
        .0;
    change_database(
        files,
        &format!(
            "PRAGMA foreign_keys=OFF; DROP TABLE object_tombstones; ALTER TABLE reading_states DROP COLUMN pdf_view_json; DELETE FROM migration_history WHERE version=17; ALTER TABLE search_settings DROP COLUMN retrieval_params_json; ALTER TABLE search_settings DROP COLUMN retrieval_row_version; ALTER TABLE search_settings DROP COLUMN cleaning_params_json; ALTER TABLE search_settings DROP COLUMN cleaning_row_version; ALTER TABLE source_parses DROP COLUMN cleaning_json; DELETE FROM migration_history WHERE version=22; ALTER TABLE search_settings DROP COLUMN chunk_params_json; ALTER TABLE search_settings DROP COLUMN chunk_row_version; ALTER TABLE search_settings DROP COLUMN previous_profile_id; DELETE FROM migration_history WHERE version=23; DELETE FROM migration_history WHERE version=24; DELETE FROM migration_history WHERE version=18; DROP TABLE retrieval_events; DROP TABLE model_usage_events; DELETE FROM migration_history WHERE version=19; DELETE FROM migration_history WHERE version=20; ALTER TABLE sources DROP COLUMN author; DELETE FROM migration_history WHERE version=21; DROP TABLE generation_channels; DELETE FROM migration_history WHERE version=16; {versions} CREATE TEMP TABLE pkms_fixture_revisions AS SELECT * FROM note_revisions; DROP TRIGGER note_preview_changed; DROP TABLE note_revisions; CREATE TABLE note_revisions{table}; INSERT INTO note_revisions SELECT * FROM pkms_fixture_revisions; DROP TABLE pkms_fixture_revisions; CREATE TRIGGER note_preview_changed{trigger}END; DELETE FROM migration_history WHERE version=15; {views} {automation}",
            versions = common::pre_docx_source_versions(),
            views = common::pre_views_canvas_versions(),
            automation = common::pre_automation_versions()
        ),
    );
    update_manifest(files, |manifest| manifest.document_schemas = vec![1, 2]);
}

fn legacy_note(library: &mut Library, title: &str, text: &str) -> Note {
    library
        .create_document(
            &id(),
            CreateNote {
                title: title.into(),
                document_schema: 2,
                document: NoteDocument::from_text(id(), text),
                collection_id: None,
                base_collection_row_version: None,
            },
        )
        .unwrap()
        .note
}

#[test]
fn rich_format_review_legacy_backup_rejects_d3_pending_proposals_outside_manifest_schemas() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let evidence = legacy_note(&mut library, "evidence", "fixtureevidence");
    let answer = library
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: SearchQuery {
                    query: "fixtureevidence".into(),
                    mode: SearchMode::Lexical,
                    scope: SearchScope {
                        object_ids: vec![evidence.id],
                        ..Default::default()
                    },
                    limit: 1,
                    minimum_cosine: None,
                },
                model: Some(GenerationProfile {
                    channel_id: None,
                    capabilities: None,
                    chat_token_limit: None,
                    provider: "ollama".into(),
                    endpoint: "http://127.0.0.1:11434".into(),
                    model: "controlled".into(),
                    model_digest: "controlled".into(),
                    config_revision: 1,
                    temperature: 0.0,
                    context_tokens: 4096,
                }),
            },
        )
        .unwrap()
        .answer;
    let lease = library.claim_answer(&answer.id).unwrap().unwrap();
    library
        .prepare_answer_evidence(&answer.id, &lease.lease_token, vec![])
        .unwrap();
    library
        .complete_answer(CompleteAnswer {
            answer_id: answer.id.clone(),
            lease_token: lease.lease_token,
            text: "Controlled fixture [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
    let legacy_request = CreateProposal {
        answer_id: Some(answer.id),
        chat_run_id: None,
        external_request_id: None,
        note_id: None,
        base_revision_id: None,
        title: "旧待审提案".into(),
        document_schema: 2,
        document: NoteDocument::from_text(id(), "旧提案正文"),
    };
    let legacy = library
        .create_proposal(&id(), legacy_request.clone())
        .unwrap()
        .proposal;
    let downgrade_fixture = |source: &Path, destination: &Path| {
        let mut files = unpack(source);
        pre_d3_fixture(&mut files);
        change_database(
            &mut files,
            "UPDATE workspace_meta SET schema_version=14; PRAGMA user_version=14;",
        );
        update_manifest(&mut files, |manifest| manifest.database_schema = 14);
        pack(destination, &files);
    };
    let plain = root.path().join("plain-current.pkmsbackup");
    library.create_backup(&plain, &mut ok).unwrap();
    let old = root.path().join("valid-old.pkmsbackup");
    downgrade_fixture(&plain, &old);
    let checked = verify_backup(&old, &mut ok).unwrap();
    assert_eq!(checked.manifest.database_schema, 14);
    assert_eq!(checked.manifest.document_schemas, [1, 2]);
    let restored_path = root.path().join("valid-restored");
    prepare_restore(&old, &restored_path, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    assert_eq!(
        serde_json::to_value(restored.proposal(&legacy.id).unwrap()).unwrap(),
        serde_json::to_value(&legacy).unwrap()
    );

    let rich_document: NoteDocument = serde_json::from_value(json!({
        "type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"新格式提案","marks":[{"type":"underline"}]}]}]
    })).unwrap();
    let rich = library
        .create_proposal(
            &id(),
            CreateProposal {
                document_schema: 3,
                document: rich_document,
                ..legacy_request
            },
        )
        .unwrap()
        .proposal;
    assert_eq!(rich.status, ProposalStatus::Pending);
    let current = root.path().join("valid-current.pkmsbackup");
    let report = library.create_backup(&current, &mut ok).unwrap();
    assert_eq!(
        report.manifest.database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(report.manifest.document_schemas, [1, 2, 3]);
    let forged = root.path().join("invalid-old-d3.pkmsbackup");
    downgrade_fixture(&current, &forged);
    assert!(matches!(
        verify_backup(&forged, &mut ok),
        Err(Error::IntegrityFailure)
    ));
    let rejected_path = root.path().join("must-not-publish");
    assert!(matches!(
        prepare_restore(&forged, &rejected_path, &mut ok),
        Err(Error::IntegrityFailure)
    ));
    assert!(!rejected_path.exists());
}

#[test]
fn full_package_restores_historical_images_trash_and_organization_without_orphan_imports() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("source");
    let mut library = Library::open(&workspace).unwrap();
    let bytes = png();
    let imported = library.import_attachment(&id(), "图.png", &bytes).unwrap();
    let first = library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "历史".into(),
                document_schema: 2,
                document: image_doc(&imported.blob.hash),
            },
        )
        .unwrap()
        .note;
    let latest = library
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: first.revision_id.clone(),
                title: "图片已删除".into(),
                document_schema: 2,
                document: NoteDocument::from_text(id(), "保留历史"),
            },
        )
        .unwrap()
        .note;
    let mut other = Cursor::new(vec![]);
    image::DynamicImage::new_rgb8(9, 8)
        .write_to(&mut other, image::ImageFormat::Png)
        .unwrap();
    let orphan = library
        .import_attachment(&id(), "未提交.png", &other.into_inner())
        .unwrap();
    fs::remove_file(workspace.join(format!(
        "blobs/sha256/{}/{}",
        &orphan.blob.hash[..2],
        orphan.blob.hash
    )))
    .unwrap();
    let c = library.create_collection("主题").unwrap();
    library.add_to_collection(&c.id, &first.id).unwrap();
    library
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: first.id.clone(),
                    base_row_version: 2,
                }],
                tag: TagSelector::Name {
                    name: "资料".into(),
                },
            },
        )
        .unwrap();
    library.set_trashed(&first.id, true).unwrap();
    let package = root.path().join("full.pkmsbackup");
    let report = library.create_backup(&package, &mut ok).unwrap();
    assert_eq!(report.verified_revisions, 2);
    assert_eq!(report.verified_blobs, 1);
    assert_eq!(report.manifest.counts.memberships, 1);
    assert!(!inspect_backup(&package).unwrap().fully_verified);
    assert_eq!(verify_backup(&package, &mut ok).unwrap(), report);
    let files = unpack(&package);
    assert_eq!(files.len(), 5);
    assert!(
        !files
            .keys()
            .any(|p| p.contains("app-state") || p.contains("local_instance"))
    );
    let destination = root.path().join("restored");
    let ready = prepare_restore(&package, &destination, &mut ok).unwrap();
    assert!(!destination.exists());
    ready.publish().unwrap();
    let mut restored = Library::open_existing(&destination).unwrap();
    assert_eq!(
        restored.info().unwrap().workspace_id,
        library.info().unwrap().workspace_id
    );
    assert_eq!(restored.note(&latest.id).unwrap().document, latest.document);
    assert!(restored.note(&latest.id).unwrap().trashed);
    assert_eq!(
        restored
            .attachment(&first.revision_id, &imported.blob.hash)
            .unwrap()
            .1,
        bytes
    );
    assert_eq!(restored.object_metadata(&first.id).unwrap().tags.len(), 1);
    assert_eq!(
        restored.collection_get(&c.id).unwrap(),
        library.collection_get(&c.id).unwrap()
    );
    assert!(restored.search("保留历史", 10).unwrap().is_empty());
    restored.set_trashed(&first.id, false).unwrap();
    let hits = restored.search("保留历史", 10).unwrap();
    assert_eq!(hits.len(), 1, "rebuilt FTS has one current entry per note");
    assert_eq!(hits[0].note.id, first.id);
}

#[test]
fn restore_drill_small_medium_large_preserve_objects_relations_and_attachments() {
    fn counts(path: &Path) -> Vec<(String, i64)> {
        let connection = Connection::open(path).unwrap();
        [
            "notes",
            "note_revisions",
            "collections",
            "memberships",
            "body_links",
            "blobs",
        ]
        .into_iter()
        .map(|table| {
            let count = connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            (table.to_owned(), count)
        })
        .collect()
    }

    for (name, note_count, collection_count, link_count, attachment_count) in [
        ("small", 3usize, 1usize, 1usize, 1usize),
        ("medium", 40, 5, 10, 5),
        ("large", 300, 15, 30, 15),
    ] {
        let root = tempdir().unwrap();
        let workspace = root.path().join(format!("{name}-source"));
        let mut library = Library::open(&workspace).unwrap();
        let mut notes = Vec::new();
        for index in 0..note_count {
            notes.push(
                library
                    .create_note(&format!("笔记 {index}"), &format!("正文 {index}"))
                    .unwrap(),
            );
        }
        let mut images = Vec::new();
        for index in 0..attachment_count {
            let mut image_bytes = Vec::new();
            image::DynamicImage::new_rgb8(5 + u32::try_from(index).unwrap(), 7)
                .write_to(&mut Cursor::new(&mut image_bytes), image::ImageFormat::Png)
                .unwrap();
            let imported = library
                .import_attachment(&id(), "图.png", &image_bytes)
                .unwrap();
            let note = library
                .create_document(
                    &id(),
                    CreateNote {
                        collection_id: None,
                        base_collection_row_version: None,
                        title: "图片".into(),
                        document_schema: 2,
                        document: image_doc(&imported.blob.hash),
                    },
                )
                .unwrap()
                .note;
            images.push((note, imported.blob.hash, image_bytes));
        }
        let mut collections = Vec::new();
        for index in 0..collection_count {
            let collection = library.create_collection(&format!("集合 {index}")).unwrap();
            for note in notes.iter().skip(index).step_by(collection_count.max(1)) {
                library.add_to_collection(&collection.id, &note.id).unwrap();
            }
            collections.push(collection);
        }
        for index in 1..=link_count.min(notes.len().saturating_sub(1)) {
            let target = &notes[index - 1];
            let source = &notes[index];
            let document: NoteDocument = serde_json::from_value(json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "attrs": {"id": id()},
                    "content": [{
                        "type": "noteReference",
                        "attrs": {
                            "target_id": target.id,
                            "target_block_id": target.document.content[0].attrs.id,
                            "target_revision_id": target.revision_id,
                            "occurrence_id": id(),
                            "label": "drill"
                        }
                    }]
                }]
            }))
            .unwrap();
            let saved = library
                .save_document(
                    &id(),
                    SaveNote {
                        note_id: source.id.clone(),
                        base_revision_id: source.revision_id.clone(),
                        title: source.title.clone(),
                        document_schema: 2,
                        document,
                    },
                )
                .unwrap()
                .note;
            notes[index] = saved;
        }
        let package = root.path().join(format!("{name}.pkmsbackup"));
        let report = library.create_backup(&package, &mut ok).unwrap();
        assert_eq!(
            report.verified_blobs,
            u32::try_from(attachment_count).unwrap()
        );
        println!(
            "restore-drill {name}: notes={note_count} collections={collection_count} links={link_count} attachments={attachment_count} package_bytes={}",
            fs::metadata(&package).unwrap().len()
        );
        let destination = root.path().join(format!("{name}-restored"));
        let ready = prepare_restore(&package, &destination, &mut ok).unwrap();
        assert!(!destination.exists());
        ready.publish().unwrap();
        let restored = Library::open_existing(&destination).unwrap();
        assert_eq!(
            restored.info().unwrap().workspace_id,
            library.info().unwrap().workspace_id
        );
        assert_eq!(
            counts(&workspace.join("library.sqlite3")),
            counts(&destination.join("library.sqlite3"))
        );
        for note in &notes {
            assert_eq!(restored.note(&note.id).unwrap().document, note.document);
        }
        for (note, hash, bytes) in &images {
            assert_eq!(
                &restored.attachment(&note.revision_id, hash).unwrap().1,
                bytes
            );
        }
        for collection in &collections {
            assert_eq!(
                restored.collection_get(&collection.id).unwrap(),
                library.collection_get(&collection.id).unwrap()
            );
        }
        for index in 1..=link_count.min(notes.len().saturating_sub(1)) {
            assert_eq!(
                restored.body_backlinks(&notes[index - 1].id).unwrap().len(),
                1
            );
        }
        assert!(!restored.search("正文", 10).unwrap().is_empty());
    }
}
#[test]
fn cancellation_corrupt_blobs_and_existing_outputs_preserve_original_data() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let imported = library.import_attachment(&id(), "图.png", &png()).unwrap();
    let note = library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "保存".into(),
                document_schema: 2,
                document: image_doc(&imported.blob.hash),
            },
        )
        .unwrap()
        .note;
    let path = root.path().join("canceled.zip");
    let mut cancel = |stage: &str| {
        if stage == "copy_blobs" {
            Err(Error::Io(std::io::ErrorKind::Interrupted.into()))
        } else {
            Ok(())
        }
    };
    assert!(library.create_backup(&path, &mut cancel).is_err());
    assert!(!path.exists());
    fs::write(&path, b"existing").unwrap();
    assert!(library.create_backup(&path, &mut ok).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing");
    let blob = root.path().join("source").join(format!(
        "blobs/sha256/{}/{}",
        &imported.blob.hash[..2],
        imported.blob.hash
    ));
    fs::write(blob, b"corrupt").unwrap();
    assert!(
        library
            .create_backup(root.path().join("bad.zip"), &mut ok)
            .is_err()
    );
    assert_eq!(library.note(&note.id).unwrap(), note);
}

#[test]
fn restore_never_replaces_existing_directory_even_when_created_after_preparation() {
    let root = tempdir().unwrap();
    let library = Library::open(root.path().join("source")).unwrap();
    let package = root.path().join("p.zip");
    library.create_backup(&package, &mut ok).unwrap();
    let destination = root.path().join("restored");
    let ready = prepare_restore(&package, &destination, &mut ok).unwrap();
    fs::create_dir(&destination).unwrap();
    assert!(ready.publish().is_err());
    assert!(destination.is_dir());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert!(prepare_restore(&package, &destination, &mut ok).is_err());
}

#[test]
fn manifest_future_versions_hash_corruption_and_missing_members_are_rejected() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    library.create_note("测试", "正文").unwrap();
    let original = root.path().join("original.zip");
    library.create_backup(&original, &mut ok).unwrap();
    let baseline = unpack(&original);
    for field in ["format", "schema", "document"] {
        let mut files = baseline.clone();
        update_manifest(&mut files, |m| match field {
            "format" => m.format_version += 1,
            "schema" => m.database_schema += 1,
            _ => m.document_schemas = vec![99],
        });
        let path = root.path().join(format!("{field}.zip"));
        pack(&path, &files);
        assert!(inspect_backup(&path).is_err());
    }
    let mut files = baseline.clone();
    files.get_mut("library.sqlite3").unwrap()[100] ^= 1;
    let path = root.path().join("hash.zip");
    pack(&path, &files);
    assert!(inspect_backup(&path).is_ok());
    assert!(verify_backup(&path, &mut ok).is_err());
    let mut files = baseline.clone();
    files.remove("RESTORE.txt");
    let path = root.path().join("missing.zip");
    pack(&path, &files);
    assert!(inspect_backup(&path).is_err());
    let mut files = baseline;
    update_manifest(&mut files, |m| m.counts.notes += 1);
    let path = root.path().join("counts.zip");
    pack(&path, &files);
    assert!(verify_backup(&path, &mut ok).is_err());
}

#[test]
fn hostile_paths_duplicates_symlinks_and_oversized_entries_fail_before_extraction() {
    let root = tempdir().unwrap();
    let library = Library::open(root.path().join("source")).unwrap();
    let original = root.path().join("original.zip");
    library.create_backup(&original, &mut ok).unwrap();
    let baseline = unpack(&original);
    for (index, path) in [
        "../escape",
        "/absolute",
        "C:/absolute",
        "blobs\\escape",
        "blobs/sha256/../escape",
        "manifest.json:stream",
    ]
    .iter()
    .enumerate()
    {
        let mut files = baseline.clone();
        files.insert((*path).into(), vec![1]);
        let archive = root.path().join(format!("path-{index}.zip"));
        pack(&archive, &files);
        assert!(inspect_backup(&archive).is_err());
    }
    let mut files = baseline.clone();
    files.insert("manifest.jsox".into(), b"duplicate".to_vec());
    let duplicate = root.path().join("duplicate.zip");
    pack(&duplicate, &files);
    let mut raw = fs::read(&duplicate).unwrap();
    let from = b"manifest.jsox";
    for index in 0..=raw.len() - from.len() {
        if &raw[index..index + from.len()] == from {
            raw[index + from.len() - 1] = b'n';
        }
    }
    fs::write(&duplicate, raw).unwrap();
    assert!(inspect_backup(&duplicate).is_err());
    let raw = fs::read(&original).unwrap();
    let central = raw
        .windows(4)
        .position(|w| w == [0x50, 0x4b, 0x01, 0x02])
        .unwrap();
    let mut link = raw.clone();
    link[central + 38..central + 42].copy_from_slice(&(0o120777_u32 << 16).to_le_bytes());
    let path = root.path().join("link.zip");
    fs::write(&path, link).unwrap();
    assert!(inspect_backup(&path).is_err());
    let mut huge = raw;
    huge[central + 24..central + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    let path = root.path().join("huge.zip");
    fs::write(&path, huge).unwrap();
    assert!(inspect_backup(&path).is_err());
    assert!(!root.path().join("escape").exists());
}

#[test]
fn valid_container_and_database_hash_cannot_hide_bad_history_or_attacker_sql() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let note = library.create_note("old", "old body").unwrap();
    library.save_note(&note.id, 1, "new", "new body").unwrap();
    let original = root.path().join("original.zip");
    library.create_backup(&original, &mut ok).unwrap();
    let baseline = unpack(&original);
    for (i,sql) in ["UPDATE note_revisions SET title='tampered' WHERE revision=1", "DELETE FROM note_revision_blocks WHERE revision_id=(SELECT revision_id FROM note_revisions WHERE revision=1)","UPDATE note_revisions SET parent_revision_id=revision_id WHERE revision=2", "CREATE TRIGGER hostile AFTER UPDATE ON notes BEGIN DELETE FROM collections; END", "PRAGMA user_version=999"].iter().enumerate() {let mut files=baseline.clone();change_database(&mut files,sql);let path=root.path().join(format!("invalid-{i}.zip"));pack(&path,&files);assert!(verify_backup(&path,&mut ok).is_err(),"{sql}");let destination=root.path().join(format!("restored-{i}"));assert!(prepare_restore(&path,&destination,&mut ok).is_err());assert!(!destination.exists());}
}

#[test]
fn exports_render_real_structure_escape_text_and_include_referenced_image_bytes() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let picture = png();
    let blob = library
        .import_attachment(&id(), "image.png", &picture)
        .unwrap();
    let mut document = image_doc(&blob.blob.hash);
    document.content[0].content[0].text = Some("<script>alert('x')</script> *text*".into());
    document.content.push(serde_json::from_value(json!({"type":"codeBlock","attrs":{"id":id()},"content":[{"type":"text","text":"```\ncode\n```"}]})).unwrap());
    let note = library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "../bad <title>".into(),
                document_schema: 2,
                document,
            },
        )
        .unwrap()
        .note;
    for format in [ExportFormat::Markdown, ExportFormat::Html] {
        let path = root.path().join(format!("{format:?}.zip"));
        let report = library
            .export_note_archive(&note.id, &path, format, &mut ok)
            .unwrap();
        let files = unpack(&path);
        let rendered = String::from_utf8(files[&report.document_path].clone()).unwrap();
        assert!(!rendered.contains("<script>"));
        assert!(report.document_path.starts_with(&note.id));
        assert_eq!(report.attachment_count, 1);
        assert_eq!(files[&format!("assets/{}.png", blob.blob.hash)], picture);
        let saved: Note = serde_json::from_slice(&files["document.json"]).unwrap();
        assert_eq!(saved, note);
        if format == ExportFormat::Markdown {
            assert!(rendered.contains("````\n```\ncode"));
        } else {
            assert!(rendered.contains("&lt;script&gt;"));
        }
    }
}

#[test]
fn schema_seven_package_is_validated_before_migration_and_unfinished_jobs_do_not_resume_in_a_copy()
{
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let note = legacy_note(&mut library, "旧包", "正文");
    let job = library
        .accept_portable_job(&id(), PortableJobInput::BackupCreate, &id())
        .unwrap();
    let original = root.path().join("original.zip");
    library.create_backup(&original, &mut ok).unwrap();
    let destination = root.path().join("current");
    prepare_restore(&original, &destination, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    let restored_job = restored.portable_job(&job.job_id).unwrap();
    assert_eq!(restored_job.status, JobStatus::Failed);
    assert_eq!(restored_job.error_code.as_deref(), Some("restored_copy"));
    assert_eq!(
        library.portable_job(&job.job_id).unwrap().status,
        JobStatus::Queued
    );
    let mut files = unpack(&original);
    pre_d3_fixture(&mut files);
    change_database(&mut files, DROP_ASSISTANT_INDEX);
    change_database(&mut files, DROP_SOURCE_TITLES);
    change_database(
        &mut files,
        "DROP TABLE source_parse_staging; DROP TABLE source_web_captures; DROP TABLE source_job_events; DROP TABLE source_job_attempts; DROP TABLE source_jobs; DROP TABLE reading_states; DROP TABLE annotations; DROP TABLE source_fts; DROP TABLE source_blocks; DROP TABLE source_active_parses; DROP TABLE source_parses; DROP TABLE source_file_dedup; DROP TABLE source_versions; DROP TABLE sources; DROP TRIGGER source_metadata_update; DROP TABLE source_list_state; DROP TABLE portable_jobs; DELETE FROM command_receipts WHERE command_kind='jobs.portable'; DELETE FROM migration_history WHERE version>=8; UPDATE workspace_meta SET schema_version=7; PRAGMA user_version=7;",
    );
    update_manifest(&mut files, |m| {
        m.database_schema = 7;
        m.counts.portable_jobs = 0;
        m.locator_schemas = vec![];
    });
    let old = root.path().join("old.zip");
    pack(&old, &files);
    assert_eq!(
        verify_backup(&old, &mut ok)
            .unwrap()
            .manifest
            .database_schema,
        7
    );
    let destination = root.path().join("upgraded");
    prepare_restore(&old, &destination, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    assert_eq!(
        restored.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(restored.note(&note.id).unwrap(), note);
    assert!(destination.join("migration-backups").is_dir());
}

#[test]
fn schema_eight_through_twelve_packages_restore_through_frozen_migrations() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let note = legacy_note(&mut library, "旧版包", "原始正文");
    let original = root.path().join("current.zip");
    library.create_backup(&original, &mut ok).unwrap();
    for schema in [8, 9, 10, 11, 12] {
        let mut files = unpack(&original);
        pre_d3_fixture(&mut files);
        let mut sql = String::from(if schema == 12 {
            DROP_REVIEW
        } else {
            DROP_ASSISTANT_INDEX
        });
        if schema < 11 {
            sql.push_str(DROP_SOURCE_TITLES);
        }
        if schema < 10 {
            sql.push_str("DROP TABLE source_parse_staging; DROP TABLE source_web_captures; DELETE FROM migration_history WHERE version=10;");
        }
        if schema == 8 {
            sql.push_str("DROP TABLE source_job_events; DROP TABLE source_job_attempts; DROP TABLE source_jobs; DROP TABLE reading_states; DROP TABLE annotations; DROP TABLE source_fts; DROP TABLE source_blocks; DROP TABLE source_active_parses; DROP TABLE source_parses; DROP TABLE source_file_dedup; DROP TABLE source_versions; DROP TABLE sources; DROP TRIGGER source_metadata_update; DROP TABLE source_list_state; DELETE FROM migration_history WHERE version=9;");
        }
        sql.push_str(&format!(
            "UPDATE workspace_meta SET schema_version={schema}; PRAGMA user_version={schema};"
        ));
        change_database(&mut files, &sql);
        update_manifest(&mut files, |manifest| {
            manifest.database_schema = schema;
            if schema == 8 {
                manifest.locator_schemas.clear();
            }
        });
        // Published pre-W4 manifests do not contain the new count keys.
        let mut legacy_manifest: serde_json::Value =
            serde_json::from_slice(&files["manifest.json"]).unwrap();
        for key in [
            "embedding_profiles",
            "index_generations",
            "index_chunks",
            "answers",
            "answer_feedback",
            "proposals",
        ] {
            legacy_manifest["counts"]
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
        let manifest_bytes = serde_json::to_vec(&legacy_manifest).unwrap();
        files.insert(
            "COMPLETE".into(),
            format!("{:x}", Sha256::digest(&manifest_bytes)).into_bytes(),
        );
        files.insert("manifest.json".into(), manifest_bytes);
        let old = root.path().join(format!("v{schema}.zip"));
        pack(&old, &files);
        assert_eq!(
            verify_backup(&old, &mut ok)
                .unwrap()
                .manifest
                .database_schema,
            schema
        );
        let destination = root.path().join(format!("restored-{schema}"));
        prepare_restore(&old, &destination, &mut ok)
            .unwrap()
            .publish()
            .unwrap();
        let restored = Library::open_existing(&destination).unwrap();
        assert_eq!(
            restored.info().unwrap().database_schema,
            pkms_core::CURRENT_SCHEMA_VERSION as u32
        );
        assert_eq!(restored.note(&note.id).unwrap(), note);
    }
}

#[test]
fn schema_ten_source_titles_are_rebuilt_without_touching_original_body() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let command = uuid::Uuid::new_v4().to_string();
    let job = db
        .accept_source_import(
            &command,
            ImportSource {
                input_token: command.clone(),
                original_name: "迁移标题 amber".into(),
                format: SourceFormat::Text,
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
            &mut std::io::Cursor::new(b"cobalt body"),
            &mut ok,
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, b"cobalt body").unwrap(),
    )
    .unwrap();
    let original = root.path().join("current-source.zip");
    db.create_backup(&original, &mut ok).unwrap();
    let mut files = unpack(&original);
    pre_d3_fixture(&mut files);
    change_database(
        &mut files,
        &format!(
            "{DROP_ASSISTANT_INDEX} {DROP_SOURCE_TITLES} UPDATE workspace_meta SET schema_version=10; PRAGMA user_version=10;"
        ),
    );
    update_manifest(&mut files, |manifest| manifest.database_schema = 10);
    let old = root.path().join("source-v10.zip");
    pack(&old, &files);
    verify_backup(&old, &mut ok).unwrap();
    let restored_path = root.path().join("restored-source");
    prepare_restore(&old, &restored_path, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    assert_eq!(
        restored.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    assert_eq!(
        restored.search_sources("amber cobalt", 10).unwrap()[0].source_id,
        version.source_id
    );
    assert_eq!(
        restored
            .source_text_page(&version.id, None, 0, 1)
            .unwrap()
            .blocks[0]
            .text,
        "cobalt body"
    );
}

#[test]
fn assistant_history_proposal_only_images_and_index_metadata_survive_without_resuming_models() {
    // Controlled answer text verifies persistence only; no model or vector store is invoked.
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let note = db.create_note("evidence", "evidence body").unwrap();
    let other = db.create_note("second", "another block").unwrap();
    let request = StartAnswer {
        max_output_tokens: None,
        citation_count: None,
        agent: None,
        search: SearchQuery {
            query: "evidence".into(),
            mode: SearchMode::Lexical,
            scope: SearchScope::default(),
            limit: 8,
            minimum_cosine: None,
        },
        model: Some(GenerationProfile {
            channel_id: None,
            capabilities: None,
            chat_token_limit: None,
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "persistence-fixture".into(),
            model_digest: "fixture-digest".into(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 4096,
        }),
    };
    let queued = db.accept_answer(&id(), request.clone()).unwrap().answer;
    let running = db.accept_answer(&id(), request.clone()).unwrap().answer;
    let running_lease = db.claim_answer(&running.id).unwrap().unwrap();
    let running = db
        .prepare_answer_evidence(&running.id, &running_lease.lease_token, vec![])
        .unwrap();
    db.append_answer_delta(
        &running.id,
        &running_lease.lease_token,
        running.last_event_seq + 1,
        "partial controlled output",
    )
    .unwrap();
    let completed = db.accept_answer(&id(), request).unwrap().answer;
    let lease = db.claim_answer(&completed.id).unwrap().unwrap();
    db.prepare_answer_evidence(&completed.id, &lease.lease_token, vec![])
        .unwrap();
    let completed = db
        .complete_answer(CompleteAnswer {
            answer_id: completed.id,
            lease_token: lease.lease_token,
            text: "Controlled claim [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
    db.feedback_answer(
        &id(),
        AnswerFeedback {
            answer_id: completed.id.clone(),
            rating: 1,
            comment: "persistence test".into(),
            updated_at: None,
        },
    )
    .unwrap();
    let attachment = db
        .import_attachment(&id(), "proposal-only.png", &png())
        .unwrap();
    let proposal_request = CreateProposal {
        chat_run_id: None,
        external_request_id: None,
        answer_id: Some(completed.id.clone()),
        note_id: Some(note.id.clone()),
        base_revision_id: Some(note.revision_id.clone()),
        title: "proposed".into(),
        document_schema: 2,
        document: image_doc(&attachment.blob.hash),
    };
    let pending = db
        .create_proposal(&id(), proposal_request.clone())
        .unwrap()
        .proposal;
    let rejected = db
        .create_proposal(&id(), proposal_request)
        .unwrap()
        .proposal;
    db.reject_proposal(&id(), &rejected.id).unwrap();
    let profile = db
        .register_embedding_profile(
            &id(),
            EmbeddingProfileSpec {
                endpoint: None,
                model: "fixture".into(),
                revision: "sha256-fixture".into(),
                dimensions: 2,
                normalized: true,
                query_prefix: String::new(),
                passage_prefix: String::new(),
                tokenizer: "fixture-v1".into(),
                chunker: "whole-block-v1".into(),
                max_tokens: 512,
                overlap_tokens: 0,
                provider: None,
                output_dimensions: None,
            },
        )
        .unwrap()
        .profile;
    let mut generations = vec![];
    for (n, note) in [&note, &other].into_iter().enumerate() {
        let input = db.index_input(&note.id).unwrap();
        let block = &input.blocks[0];
        let lease = db
            .begin_index_generation(
                &id(),
                IndexPlan {
                    profile_id: profile.id.clone(),
                    object_id: input.object_id,
                    content_revision: input.content_revision,
                    metadata_revision: input.metadata_revision,
                    parser_revision: input.parser_revision,
                    chunks: vec![IndexChunkInput {
                        chunk_id: format!("chunk{n}"),
                        block_id: block.target.block_id.clone(),
                        start_codepoint: 0,
                        end_codepoint: block.text.chars().count() as u32,
                        text: block.text.clone(),
                        token_count: 3,
                    }],
                },
            )
            .unwrap();
        if n == 0 {
            db.publish_index_generation(IndexPublish {
                generation_id: lease.generation.id.clone(),
                lease_token: lease.lease_token,
                profile_id: profile.id.clone(),
                chunk_count: 1,
            })
            .unwrap();
        }
        generations.push(lease.generation.id);
    }
    let backup = root.path().join("assistant.zip");
    let report = db.create_backup(&backup, &mut ok).unwrap();
    let counts = &report.manifest.counts;
    assert_eq!(
        (
            counts.embedding_profiles,
            counts.index_generations,
            counts.index_chunks,
            counts.answers,
            counts.answer_feedback,
            counts.proposals
        ),
        (1, 2, 2, 3, 1, 2)
    );
    assert_eq!(report.verified_blobs, 1);
    let destination = root.path().join("restored");
    prepare_restore(&backup, &destination, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    assert!(restored.active_index_generations().unwrap().is_empty());
    assert!(!restored.dirty_index_object_ids(100).unwrap().is_empty());
    for generation in &generations {
        let state = restored.index_generation(generation).unwrap();
        assert_eq!(state.status, IndexGenerationStatus::Failed);
        assert_eq!(
            state.error_code.as_deref(),
            Some("projection_rebuild_required")
        );
    }
    for answer in [&queued, &running] {
        let state = restored.answer(&answer.id).unwrap();
        assert_eq!(state.status, AnswerStatus::Failed);
        assert_eq!(state.error_code.as_deref(), Some("interrupted"));
        assert_eq!(
            restored
                .answer_events(&answer.id, 0, 100)
                .unwrap()
                .last()
                .unwrap()
                .status,
            AnswerStatus::Failed
        );
    }
    assert_eq!(
        restored.answer(&running.id).unwrap().text,
        "partial controlled output"
    );
    assert_eq!(restored.answer(&completed.id).unwrap().text, completed.text);
    assert_eq!(
        restored.answer(&completed.id).unwrap().evidence.len(),
        completed.evidence.len()
    );
    assert_eq!(
        restored.proposal(&pending.id).unwrap().status,
        ProposalStatus::Pending
    );
    assert_eq!(
        restored.proposal(&rejected.id).unwrap().status,
        ProposalStatus::Rejected
    );
    assert!(restored.pending_answer_ids(20).unwrap().is_empty());
    let raw = rusqlite::Connection::open(destination.join("library.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM index_chunks", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM answer_feedback", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(raw.query_row("SELECT count(*) FROM index_generations WHERE lease_token IS NOT NULL OR lease_expires_at IS NOT NULL",[],|r|r.get::<_,u32>(0)).unwrap(),0);
    // Re-export revalidates the restored event sequence, retained evidence and proposal image.
    restored
        .create_backup(root.path().join("restored-again.zip"), &mut ok)
        .unwrap();
    assert_eq!(
        db.answer(&running.id).unwrap().status,
        AnswerStatus::Running
    );
    assert_eq!(
        db.index_generation(&generations[0]).unwrap().status,
        IndexGenerationStatus::Ready
    );
}

#[test]
fn partial_source_parse_backup_restores_without_publishing_and_rejects_visible_staging() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("source");
    let mut library = Library::open(&workspace).unwrap();
    let accepted = library
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "partial.pdf".into(),
                format: SourceFormat::Pdf,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = library.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = library
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"%PDF-partial-test"),
            &mut ok,
        )
        .unwrap();
    let parse = id();
    let block = id();
    let raw = rusqlite::Connection::open(workspace.join("library.sqlite3")).unwrap();
    raw.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) VALUES(?1,?2,'partial-test',2,0,?3)",rusqlite::params![parse,version.id,version.captured_at]).unwrap();
    raw.execute(
        "INSERT INTO source_parse_staging(parse_id,job_id,lease_token) VALUES(?1,?2,?3)",
        rusqlite::params![parse, accepted.job_id, lease.lease_token],
    )
    .unwrap();
    raw.execute("INSERT INTO source_blocks(id,parse_id,position,text,page_index) VALUES(?1,?2,0,'partial unsearchable',0)",rusqlite::params![block,parse]).unwrap();
    // The partial PDF has no geometry and is not valid complete parser output.
    let backup = root.path().join("partial.zip");
    library.create_backup(&backup, &mut ok).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&backup, &destination, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    assert!(
        restored
            .source(&version.source_id)
            .unwrap()
            .active_parse_id
            .is_none()
    );
    assert!(
        restored
            .source_text_page(&version.id, Some(&parse), 0, 20)
            .is_err()
    );
    assert!(
        restored
            .search_sources("unsearchable", 20)
            .unwrap()
            .is_empty()
    );
    let restored_raw = rusqlite::Connection::open(destination.join("library.sqlite3")).unwrap();
    assert_eq!(
        restored_raw
            .query_row("SELECT count(*) FROM source_parse_staging", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    raw.execute(
        "INSERT INTO source_active_parses(source_version_id,parse_id) VALUES(?1,?2)",
        rusqlite::params![version.id, parse],
    )
    .unwrap();
    assert!(
        library
            .create_backup(root.path().join("bad-active.zip"), &mut ok)
            .is_err()
    );
    raw.execute("DELETE FROM source_active_parses", []).unwrap();
    let annotation = id();
    let locator = SourceLocator::Text {
        source_version_id: version.id.clone(),
        block_id: block,
        start_codepoint: 0,
        end_codepoint: 7,
        quote: "partial".into(),
        prefix: String::new(),
        suffix: " unsearchable".into(),
    };
    raw.execute(
        "INSERT INTO objects(id,kind) VALUES(?1,'annotation')",
        [&annotation],
    )
    .unwrap();
    raw.execute("INSERT INTO annotations(id,source_version_id,locator_schema,locator_json,comment) VALUES(?1,?2,1,?3,'')",rusqlite::params![annotation,version.id,serde_json::to_string(&locator).unwrap()]).unwrap();
    assert!(
        library
            .create_backup(root.path().join("bad-annotation.zip"), &mut ok)
            .is_err()
    );
}

#[test]
fn concurrent_edits_do_not_mix_snapshot_counts_revisions_and_historical_blob_list() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("source");
    let mut library = Library::open(&workspace).unwrap();
    let image = library
        .import_attachment(&id(), "original.png", &png())
        .unwrap();
    let first = library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "快照前".into(),
                document_schema: 2,
                document: image_doc(&image.blob.hash),
            },
        )
        .unwrap()
        .note;
    let mut concurrent = Library::open_existing(&workspace).unwrap();
    let mut changed = false;
    let mut check = |stage: &str| {
        if stage == "copy_blobs" && !changed {
            concurrent.save_document(
                &id(),
                SaveNote {
                    note_id: first.id.clone(),
                    base_revision_id: first.revision_id.clone(),
                    title: "快照后".into(),
                    document_schema: 2,
                    document: NoteDocument::from_text(id(), "此时已经删图"),
                },
            )?;
            concurrent.create_note("新笔记", "不应进入此前快照")?;
            changed = true;
        }
        Ok(())
    };
    let package = root.path().join("snapshot.zip");
    let report = library.create_backup(&package, &mut check).unwrap();
    assert!(changed);
    assert_eq!(report.manifest.counts.notes, 1);
    assert_eq!(report.manifest.counts.revisions, 1);
    assert_eq!(report.verified_blobs, 1);
    assert_eq!(library.note(&first.id).unwrap().title, "快照后");
    let restored = root.path().join("restored");
    prepare_restore(&package, &restored, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored).unwrap();
    assert_eq!(restored.note(&first.id).unwrap(), first);
    assert_eq!(restored.list_notes().unwrap().len(), 1);
}
#[test]
fn full_backup_counts_and_restores_canvas_layout() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("source");
    let mut library = Library::open(&workspace).unwrap();
    let note = library
        .create_note("CAN 故障关系验证", "SPI 与 CAN 共享中断排查路径。")
        .unwrap();
    let canvas = library
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "CAN 故障关系验证".into(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: note.id.clone(),
                    x: 42.0,
                    y: 84.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    let package = root.path().join("canvas-backup.pkmsbackup");
    let report = library.create_backup(&package, &mut ok).unwrap();
    assert_eq!(report.manifest.counts.canvases, 1);
    assert_eq!(report.verified_revisions, 1);

    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    let restored_canvas = restored.canvas(&canvas.id).unwrap();
    assert_eq!(restored_canvas.name, "CAN 故障关系验证");
    assert_eq!(restored_canvas.nodes.len(), 1);
    assert_eq!(restored_canvas.nodes[0].note_id, note.id);
    assert_eq!(restored_canvas.nodes[0].x, 42.0);
    assert_eq!(restored_canvas.nodes[0].y, 84.0);

    let mut files = unpack(&package);
    change_database(
        &mut files,
        "UPDATE canvases SET nodes_json='[{\"note_id\":\"not-a-uuid\",\"x\":0,\"y\":0,\"width\":250,\"height\":148,\"z_index\":0}]'",
    );
    let broken = root.path().join("broken-canvas.pkmsbackup");
    pack(&broken, &files);
    assert!(matches!(
        verify_backup(&broken, &mut ok),
        Err(Error::IntegrityFailure)
    ));
}
