mod common;

use pkms_core::*;
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn document(alias: &str) -> NoteDocument {
    serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {"type":"heading","attrs":{"id":id(),"level":2},"content":[{"type":"text","text":"Block section"}]},
            {"type":"paragraph","attrs":{"id":id(),"block_id":alias},"content":[{"type":"text","text":"Block database alpha"}]},
            {"type":"bulletList","attrs":{"id":id()},"content":[{"type":"listItem","attrs":{"id":id()},"content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"Nested beta"}]}]}]}
        ]
    })).unwrap()
}

fn create(library: &mut Library, title: &str, document: NoteDocument) -> Note {
    library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: title.into(),
                document_schema: 3,
                document,
            },
        )
        .unwrap()
        .note
}

fn page(
    library: &Library,
    note_id: Option<String>,
    query: Option<&str>,
    cursor: Option<String>,
) -> BlockPage {
    library
        .blocks_page(BlocksPageRequest {
            limit: 20,
            cursor,
            query: query.map(str::to_owned),
            note_id,
        })
        .unwrap()
}

#[test]
fn block_projection_preserves_identity_parent_order_alias_and_reference_counts() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = create(&mut library, "Alpha note", document("acceptance-block"));
    let blocks = page(&library, Some(first.id.clone()), None, None);
    assert_eq!(blocks.items.len(), 5);
    assert_eq!(blocks.items[0].kind, "heading");
    assert_eq!(blocks.items[1].alias.as_deref(), Some("acceptance-block"));
    assert_eq!(blocks.items[2].kind, "bulletList");
    assert_eq!(blocks.items[3].kind, "listItem");
    assert_eq!(blocks.items[3].depth, 1);
    assert_eq!(
        blocks.items[3].parent_block_id.as_deref(),
        Some(blocks.items[2].id.as_str())
    );
    assert_eq!(blocks.items[4].depth, 2);
    assert_eq!(
        blocks.items[4].parent_block_id.as_deref(),
        Some(blocks.items[3].id.as_str())
    );
    let alpha = page(&library, None, Some("alpha acceptance-block"), None);
    assert_eq!(alpha.items.len(), 1);
    assert_eq!(alpha.items[0].note_id, first.id);

    let target = alpha.items[0].clone();
    let referencing = serde_json::from_value(json!({
        "type":"doc",
        "content":[{"type":"paragraph","attrs":{"id":id()},"content":[
            {"type":"noteReference","attrs":{"target_id":first.id,"target_block_id":target.id,"occurrence_id":id(),"label":"Alpha"}},
            {"type":"text","text":" links this block"}
        ]}]
    })).unwrap();
    create(&mut library, "Referencing note", referencing);
    let after = page(
        &library,
        Some(first.id.clone()),
        Some("Block database alpha"),
        None,
    );
    assert_eq!(after.items[0].reference_count, 1);
}

#[test]
fn block_projection_deactivates_removed_blocks_and_reuses_alias_only_after_removal() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = create(&mut library, "Mutable", document("stable-alias"));
    let replacement = serde_json::from_value(json!({
        "type":"doc",
        "content":[{"type":"paragraph","attrs":{"id":id(),"block_id":"stable-alias"},"content":[{"type":"text","text":"Replacement gamma"}]}]
    })).unwrap();
    library
        .save_document(
            &id(),
            SaveNote {
                note_id: note.id.clone(),
                base_revision_id: note.revision_id,
                title: note.title,
                document_schema: 3,
                document: replacement,
            },
        )
        .unwrap();
    assert!(
        page(&library, Some(note.id.clone()), Some("alpha"), None)
            .items
            .is_empty()
    );
    assert!(
        page(&library, Some(note.id.clone()), Some("nested"), None)
            .items
            .is_empty()
    );
    let current = page(&library, Some(note.id), Some("gamma"), None);
    assert_eq!(current.items.len(), 1);
    assert_eq!(current.items[0].alias.as_deref(), Some("stable-alias"));
}

#[test]
fn block_query_rejects_foreign_cursors_and_upgrades_metadata_when_history_rolls_to_32() {
    let root = tempdir().unwrap();
    let note = {
        let mut library = Library::open(root.path()).unwrap();
        create(&mut library, "Backfill", document("backfill-alias"))
    };
    let path = root.path().join("library.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(&common::block_backfill_schema32())
        .unwrap();
    drop(db);
    let library = Library::open_existing(root.path()).unwrap();
    let all = page(&library, Some(note.id.clone()), None, None);
    let restored = page(&library, Some(note.id), Some("backfill-alias alpha"), None);
    assert_eq!(
        restored.items.len(),
        1,
        "all blocks after backfill: {all:?}"
    );
    let bad = library.blocks_page(BlocksPageRequest {
        limit: 20,
        cursor: Some("{\"filter_hash\":\"wrong\",\"title\":\"a\",\"position\":0,\"id\":\"00000000-0000-4000-8000-000000000000\"}".into()),
        query: Some("alpha".into()),
        note_id: None,
    });
    assert!(matches!(bad, Err(Error::InvalidInput(_))));
}

#[test]
fn block_history_skips_unrelated_revisions_and_pages_without_losing_deletion() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let target = id();
    let initial: NoteDocument = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": {"id": target, "block_id": "history-anchor"},
            "content": [{"type": "text", "text": "First text"}]
        }]
    }))
    .unwrap();
    let first = create(&mut library, "History note", initial.clone());
    let second = library
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: first.revision_id.clone(),
                title: "Renamed only".into(),
                document_schema: 3,
                document: initial,
            },
        )
        .unwrap()
        .note;
    let third_document = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": {"id": target, "block_id": "history-anchor"},
            "content": [{"type": "text", "text": "Second text"}]
        }]
    }))
    .unwrap();
    let third = library
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: second.revision_id,
                title: "Renamed only".into(),
                document_schema: 3,
                document: third_document,
            },
        )
        .unwrap()
        .note;
    let removal = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": {"id": id()},
            "content": [{"type": "text", "text": "Replacement"}]
        }]
    }))
    .unwrap();
    library
        .save_document(
            &id(),
            SaveNote {
                note_id: first.id.clone(),
                base_revision_id: third.revision_id,
                title: "Renamed only".into(),
                document_schema: 3,
                document: removal,
            },
        )
        .unwrap();

    let page = library
        .block_history(BlockHistoryRequest {
            note_id: first.id.clone(),
            block_id: target.clone(),
            before: None,
            limit: 2,
        })
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].revision, 4);
    assert!(!page.items[0].present);
    assert_eq!(page.items[1].revision, 3);
    assert_eq!(page.items[1].text.as_deref(), Some("Second text"));
    assert_eq!(page.next_before, Some(3));

    let older = library
        .block_history(BlockHistoryRequest {
            note_id: first.id.clone(),
            block_id: target.clone(),
            before: page.next_before,
            limit: 2,
        })
        .unwrap();
    assert_eq!(older.items.len(), 1);
    assert_eq!(older.items[0].revision, 1);
    assert_eq!(older.items[0].text.as_deref(), Some("First text"));
    assert_eq!(older.items[0].alias.as_deref(), Some("history-anchor"));
    assert_eq!(older.next_before, None);

    assert!(matches!(
        library.block_history(BlockHistoryRequest {
            note_id: first.id,
            block_id: id(),
            before: None,
            limit: 2,
        }),
        Err(Error::NotFound)
    ));
}
