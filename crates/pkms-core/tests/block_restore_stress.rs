//! Explicit 10k-block backup/restore stress test for the block database.
//!
//! Run with: cargo test -p pkms-core --test block_restore_stress -- --ignored

mod common;

use pkms_core::*;
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

const NOTES: usize = 40;
const BLOCKS_PER_NOTE: usize = 250;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn document(note: usize) -> NoteDocument {
    let content: Vec<serde_json::Value> = (0..BLOCKS_PER_NOTE)
        .map(|offset| {
            let ordinal = note * BLOCKS_PER_NOTE + offset;
            json!({
                "type": "paragraph",
                "attrs": {"id": id(), "block_id": format!("stress-block-{ordinal:05}")},
                "content": [{"type": "text", "text": format!("Stress block {ordinal:05} shared-token")}]
            })
        })
        .collect();
    serde_json::from_value(json!({"type": "doc", "content": content})).unwrap()
}

fn create(library: &mut Library, note: usize) -> Note {
    library
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: format!("Stress note {note:02}"),
                document_schema: 3,
                document: document(note),
            },
        )
        .unwrap()
        .note
}

fn all_block_ids(library: &Library, query: Option<&str>) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = library
            .blocks_page(BlocksPageRequest {
                limit: 100,
                cursor,
                query: query.map(str::to_owned),
                note_id: None,
            })
            .unwrap();
        ids.extend(page.items.into_iter().map(|item| item.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    ids
}

#[test]
#[ignore = "10k-block backup/restore stress; run explicitly in the acceptance run"]
fn backup_restore_preserves_ten_thousand_blocks_and_queries() {
    let root = tempdir().unwrap();
    let source_dir = root.path().join("source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let mut library = Library::open(&source_dir).unwrap();
    for note in 0..NOTES {
        create(&mut library, note);
    }

    let expected = all_block_ids(&library, None);
    assert_eq!(expected.len(), NOTES * BLOCKS_PER_NOTE);
    let queried = all_block_ids(&library, Some("shared-token"));
    assert_eq!(queried.len(), expected.len());

    let backup = root.path().join("block-stress.sqlite3");
    library.backup_to(&backup).unwrap();
    drop(library);
    let connection = Connection::open(&backup).unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let active: i64 = connection
        .query_row("SELECT COUNT(*) FROM blocks WHERE active=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(active as usize, expected.len());
    drop(connection);

    let restored = Library::open_database(&backup).unwrap();
    let mut restored_ids = all_block_ids(&restored, None);
    restored_ids.sort();
    let mut source_ids = expected;
    source_ids.sort();
    assert_eq!(restored_ids, source_ids);
    assert_eq!(
        all_block_ids(&restored, Some("shared-token")).len(),
        NOTES * BLOCKS_PER_NOTE
    );

    let located = restored
        .blocks_page(BlocksPageRequest {
            limit: 20,
            cursor: None,
            query: Some("Stress block 00000".to_owned()),
            note_id: None,
        })
        .unwrap();
    assert_eq!(located.items.len(), 1);
    assert!(restored_ids.contains(&located.items[0].id));
    drop(restored);

    let source_db = source_dir.join("library.sqlite3");
    let connection = Connection::open(&source_db).unwrap();
    connection
        .execute_batch(&common::block_backfill_schema32())
        .unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    drop(connection);

    let upgraded = Library::open(&source_dir).unwrap();
    let mut upgraded_ids = all_block_ids(&upgraded, None);
    upgraded_ids.sort();
    assert_eq!(upgraded_ids, source_ids);
    assert_eq!(
        all_block_ids(&upgraded, Some("shared-token")).len(),
        NOTES * BLOCKS_PER_NOTE
    );
}
