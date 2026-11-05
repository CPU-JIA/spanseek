use pkms_core::{
    CopyNote, CreateNote, DOCUMENT_SCHEMA, Error, Library, NoteDocument, RestoreNote, SaveNote,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn document(mut content: Value) -> NoteDocument {
    fn anchors(node: &mut Value) {
        if !matches!(
            node["type"].as_str().unwrap(),
            "text" | "hardBreak" | "noteReference"
        ) {
            if node.get("attrs").is_none() {
                node["attrs"] = json!({});
            }
            node["attrs"]["id"] = json!(id());
        }
        if let Some(children) = node.get_mut("content").and_then(Value::as_array_mut) {
            for child in children {
                anchors(child);
            }
        }
    }
    for node in content.as_array_mut().unwrap() {
        anchors(node);
    }
    serde_json::from_value(json!({"type":"doc","content":content})).unwrap()
}
fn rich() -> NoteDocument {
    document(json!([
        {"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"自己的理解"}]},
        {"type":"paragraph","content":[
            {"type":"text","text":"知识🙂","marks":[{"type":"bold"},{"type":"italic"},{"type":"strike"}]},
            {"type":"hardBreak"},
            {"type":"text","text":"代码","marks":[{"type":"code"}]},
            {"type":"text","text":"来源","marks":[{"type":"link","attrs":{"href":"https://example.org/path?q=中文","target":"_blank","rel":"noopener noreferrer"}}]}
        ]},
        {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"记录"}]}]}]},
        {"type":"orderedList","attrs":{"start":3},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"复用"}]}]}]},
        {"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":true},"content":[{"type":"paragraph","content":[{"type":"text","text":"完成"}]}]}]},
        {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"引用原话"}]}]},
        {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"fn main() {\n  println!(\"你好\");\n}"}]},
        {"type":"horizontalRule"},
        {"type":"table","content":[
            {"type":"tableRow","content":[{"type":"tableHeader","attrs":{"colspan":1,"rowspan":1,"colwidth":[120]},"content":[{"type":"paragraph","content":[{"type":"text","text":"对照"}]}]}]},
            {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"结论"}]}]}]}
        ]}
    ]))
}
fn create(doc: NoteDocument) -> CreateNote {
    CreateNote {
        collection_id: None,
        base_collection_row_version: None,
        title: "富文本研究".into(),
        document_schema: DOCUMENT_SCHEMA,
        document: doc,
    }
}

#[test]
fn rich_history_is_lossless_and_replay_returns_original_revision_after_further_edits() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let cmd = id();
    let request = create(rich());
    let first = library.create_document(&cmd, request.clone()).unwrap();
    assert_eq!(first.note.document, request.document);
    assert_eq!(library.search("自己的理解", 10).unwrap().len(), 1);
    let save = SaveNote {
        note_id: first.note.id.clone(),
        base_revision_id: first.note.revision_id.clone(),
        title: "第二版".into(),
        document_schema: 2,
        document: NoteDocument::from_text(
            first.note.document.content[0].attrs.id.clone(),
            "新的文字",
        ),
    };
    let second = library.save_document(&id(), save).unwrap();
    assert!(library.search("自己的理解", 10).unwrap().is_empty());
    assert_eq!(library.search("新的文字", 10).unwrap().len(), 1);
    assert_eq!(
        second.note.parent_revision_id,
        Some(first.note.revision_id.clone())
    );
    let replay = library.create_document(&cmd, request.clone()).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.note, first.note);
    assert_eq!(replay.receipt.change_seq, first.receipt.change_seq);
    let mut changed = request;
    changed.title = "different".into();
    assert!(matches!(
        library.create_document(&cmd, changed),
        Err(Error::IdempotencyMismatch)
    ));
    let restore_cmd = id();
    let restore = RestoreNote {
        note_id: first.note.id.clone(),
        base_revision_id: second.note.revision_id.clone(),
        target_revision_id: first.note.revision_id.clone(),
    };
    let restored = library
        .restore_document(&restore_cmd, restore.clone())
        .unwrap();
    assert_eq!(restored.note.document, first.note.document);
    assert_eq!(restored.note.content_hash, first.note.content_hash);
    assert_eq!(
        restored.note.parent_revision_id,
        Some(second.note.revision_id)
    );
    assert_ne!(restored.note.revision_id, first.note.revision_id);
    assert!(library.search("新的文字", 10).unwrap().is_empty());
    assert_eq!(library.search("自己的理解", 10).unwrap().len(), 1);
    assert!(matches!(
        library.save_note(&first.note.id, 3, "损失", "平面文字"),
        Err(Error::InvalidDocument(_))
    ));
    drop(library);
    let mut reopened = Library::open_existing(dir.path()).unwrap();
    assert_eq!(reopened.note(&first.note.id).unwrap(), restored.note);
    assert!(
        reopened
            .restore_document(&restore_cmd, restore)
            .unwrap()
            .receipt
            .replayed
    );
    assert_eq!(reopened.revisions(&first.note.id).unwrap().len(), 3);
    assert_eq!(
        reopened.search("自己的理解", 10).unwrap()[0].note.id,
        first.note.id
    );
}

#[test]
fn concurrent_identical_commands_commit_once_and_stale_other_commands_conflict() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_document(&id(), create(rich())).unwrap().note;
    let command = id();
    let request = SaveNote {
        note_id: note.id.clone(),
        base_revision_id: note.revision_id.clone(),
        title: "提交一次".into(),
        document_schema: 2,
        document: note.document.clone(),
    };
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = dir.path().to_owned();
            let barrier = barrier.clone();
            let command = command.clone();
            let request = request.clone();
            std::thread::spawn(move || {
                let mut lib = Library::open_existing(path).unwrap();
                barrier.wait();
                lib.save_document(&command, request).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.receipt.replayed).count(), 1);
    assert_eq!(results[0].note, results[1].note);
    assert_eq!(library.revisions(&note.id).unwrap().len(), 2);
    assert!(matches!(
        library.save_document(&id(), request),
        Err(Error::RevisionConflict { .. })
    ));
}

#[test]
fn receipt_failure_rolls_back_revision_blocks_projection_and_sequence_then_same_command_can_retry()
{
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let first = library.create_document(&id(), create(rich())).unwrap();
    let raw = Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let before: i64 = raw
        .query_row("SELECT next_change_seq FROM workspace_meta", [], |r| {
            r.get(0)
        })
        .unwrap();
    raw.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT,'test receipt failure'); END;").unwrap();
    let command = id();
    let request = SaveNote {
        note_id: first.note.id.clone(),
        base_revision_id: first.note.revision_id.clone(),
        title: "失败不应出现".into(),
        document_schema: 2,
        document: NoteDocument::from_text(id(), "苹果失败关键字"),
    };
    assert!(library.save_document(&command, request.clone()).is_err());
    assert_eq!(library.note(&first.note.id).unwrap(), first.note);
    assert!(library.search("苹果失败关键字", 10).unwrap().is_empty());
    assert_eq!(
        raw.query_row("SELECT next_change_seq FROM workspace_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM blocks WHERE id=?1",
            [&request.document.content[0].attrs.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        raw.query_row(
            "SELECT revision_id FROM note_index_queue WHERE note_id=?1",
            [&first.note.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        first.note.revision_id
    );
    raw.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    let saved = library.save_document(&command, request).unwrap();
    assert_eq!(saved.note.revision, 2);
    assert!(!saved.receipt.replayed);
    assert_eq!(saved.receipt.change_seq, before.to_string());
}

#[test]
fn copy_regenerates_blocks_and_self_links_and_retains_external_references() {
    let dir = tempdir().unwrap();
    let mut library = Library::open(dir.path()).unwrap();
    let note = library.create_note("自己", "正文").unwrap();
    let external = library.create_note("外部", "外部正文").unwrap();
    let occurrence = id();
    let block = note.document.content[0].attrs.id.clone();
    let mut doc = note.document.clone();
    doc.content[0].content.extend(serde_json::from_value::<Vec<_>>(json!([
        {"type":"noteReference","attrs":{"target_id":note.id,"target_block_id":block,"occurrence_id":occurrence,"label":"自引用"}},
        {"type":"noteReference","attrs":{"target_id":external.id,"occurrence_id":id(),"label":"外部"}}
    ])).unwrap());
    let saved = library
        .save_document(
            &id(),
            SaveNote {
                note_id: note.id.clone(),
                base_revision_id: note.revision_id,
                title: note.title,
                document_schema: 2,
                document: doc,
            },
        )
        .unwrap()
        .note;
    let command = id();
    let request = CopyNote {
        note_id: saved.id.clone(),
        source_revision_id: saved.revision_id.clone(),
        title: "副本".into(),
    };
    let copy = library.copy_document(&command, request.clone()).unwrap();
    assert_ne!(copy.note.id, saved.id);
    let attrs = &copy.note.document.content[0].content[1].attrs;
    assert_eq!(attrs.target_id.as_deref(), Some(copy.note.id.as_str()));
    assert_eq!(
        attrs.target_block_id.as_deref(),
        Some(copy.note.document.content[0].attrs.id.as_str())
    );
    assert_ne!(attrs.occurrence_id.as_deref(), Some(occurrence.as_str()));
    assert_eq!(
        copy.note.document.content[0].content[2]
            .attrs
            .target_id
            .as_deref(),
        Some(external.id.as_str())
    );
    assert!(copy.note.parent_revision_id.is_none());
    assert_eq!(
        library.copy_document(&command, request).unwrap().note,
        copy.note
    );
    assert_eq!(library.list_notes().unwrap().len(), 3);
    assert!(matches!(
        library.create_document(&id(), create(saved.document)),
        Err(Error::InvalidDocument(_))
    ));
}

#[test]
fn unknown_or_invalid_structures_never_become_silent_plain_text() {
    for unknown in [
        json!({"type":"doc","surprise":true,"content":[]}),
        json!({"type":"doc","content":[{"type":"video"}]}),
        json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id(),"script":"bad"}}]}),
        json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"x","marks":[{"type":"color"}]}]}]}),
    ] {
        assert!(serde_json::from_value::<NoteDocument>(unknown).is_err());
    }
    let mut duplicate = rich();
    duplicate.content[1].attrs.id = duplicate.content[0].attrs.id.clone();
    assert!(duplicate.validate(2).is_err());
    let mut heading = rich();
    heading.content[0].attrs.level = Some(4);
    assert!(heading.validate(2).is_err());
    let mut javascript = rich();
    javascript.content[1].content[3].marks[0].attrs.href = Some("javascript:alert(1)".into());
    assert!(javascript.validate(2).is_err());
    let mut merged = rich();
    merged.content[8].content[0].content[0].attrs.colspan = Some(2);
    assert!(merged.validate(2).is_err());
    let mut deep = NoteDocument::from_text(id(), "deep");
    for _ in 0..33 {
        deep.content = vec![
            serde_json::from_value(
                json!({"type":"blockquote","attrs":{"id":id()},"content":deep.content}),
            )
            .unwrap(),
        ];
    }
    assert!(deep.validate(2).is_err());
    assert!(rich().validate(1).is_err());
    assert!(rich().validate(99).is_err());
    let huge = NoteDocument::from_text(id(), &"大".repeat(1_500_000));
    assert!(huge.validate(2).is_err());
}

#[test]
fn legacy_revision_mapping_and_raw_json_survive_migration_and_restore() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("fixtures/schema-v1.sql"))
        .unwrap();
    let note_id = "11111111-1111-4111-8111-111111111111";
    let original: String = raw
        .query_row(
            "SELECT document_json FROM note_revisions WHERE note_id=?1 AND revision=1",
            [note_id],
            |r| r.get(0),
        )
        .unwrap();
    drop(raw);
    let mut library = Library::open_existing(dir.path()).unwrap();
    let history = library.revisions(note_id).unwrap();
    assert_eq!(
        history[1].parent_revision_id,
        Some(history[0].revision_id.clone())
    );
    assert!(history[0].parent_revision_id.is_none());
    let restored = library
        .restore_document(
            &id(),
            RestoreNote {
                note_id: note_id.into(),
                base_revision_id: history[1].revision_id.clone(),
                target_revision_id: history[0].revision_id.clone(),
            },
        )
        .unwrap();
    let raw = Connection::open(&path).unwrap();
    let restored_json: String = raw
        .query_row(
            "SELECT document_json FROM note_revisions WHERE revision_id=?1",
            [&restored.note.revision_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, restored_json);
    assert_eq!(restored.note.schema_version, 1);
    drop(library);
    let reopened = Library::open_existing(dir.path()).unwrap();
    assert_eq!(
        reopened.revisions(note_id).unwrap()[0].revision_id,
        history[0].revision_id
    );
}
