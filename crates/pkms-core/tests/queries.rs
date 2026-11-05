use pkms_core::{Error, Library, NotesPageRequest};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn title_query_filters_before_paging_and_scopes_cursors() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for index in 0..65 {
        db.create_note(&format!("普通 {index}"), "标题查询不搜索正文 专题")
            .unwrap();
    }
    let first = db.create_note("专题 Alpha %_", "").unwrap();
    let second = db.create_note("专题 Beta", "").unwrap();
    let query = NotesPageRequest {
        limit: 1,
        cursor: None,
        trashed: false,
        filter: None,
        sort: Some(pkms_core::NoteSort::TitleAsc),
        query: Some(" 专题 ".into()),
    };
    let page = db.notes_page(query.clone()).unwrap();
    assert_eq!(page.items[0].id, first.id);
    let next = page.next_cursor.unwrap();
    let page = db
        .notes_page(NotesPageRequest {
            cursor: Some(next.clone()),
            query: Some("专题".into()),
            ..query.clone()
        })
        .unwrap();
    assert_eq!(page.items[0].id, second.id);
    assert!(page.next_cursor.is_none());
    assert!(matches!(
        db.notes_page(NotesPageRequest {
            cursor: Some(next.clone()),
            query: Some("普通".into()),
            ..query.clone()
        }),
        Err(Error::InvalidInput(_))
    ));
    for text in ["alpha", "%_"] {
        assert_eq!(
            db.notes_page(NotesPageRequest {
                query: Some(text.into()),
                ..query.clone()
            })
            .unwrap()
            .items[0]
                .id,
            first.id
        );
    }
    assert!(
        db.notes_page(NotesPageRequest {
            query: Some("完全不存在".into()),
            ..query.clone()
        })
        .unwrap()
        .items
        .is_empty()
    );
    for text in ["a".repeat(513), "中\0文".into()] {
        assert!(matches!(
            db.notes_page(NotesPageRequest {
                query: Some(text),
                ..query.clone()
            }),
            Err(Error::InvalidInput(_))
        ));
    }
    let legacy: NotesPageRequest =
        serde_json::from_str(r#"{"limit":100,"cursor":null,"trashed":false}"#).unwrap();
    assert_eq!(db.notes_page(legacy.clone()).unwrap().items.len(), 67);
    assert_eq!(
        db.notes_page(NotesPageRequest {
            query: Some(" \t ".into()),
            ..legacy
        })
        .unwrap()
        .items
        .len(),
        67
    );
    db.set_trashed(&first.id, true).unwrap();
    assert!(matches!(
        db.notes_page(NotesPageRequest {
            cursor: Some(next),
            ..query.clone()
        }),
        Err(Error::QueryExpired)
    ));
    assert_eq!(db.notes_page(query.clone()).unwrap().items[0].id, second.id);
    assert_eq!(
        db.notes_page(NotesPageRequest {
            trashed: true,
            ..query
        })
        .unwrap()
        .items[0]
            .id,
        first.id
    );
}

#[test]
fn pagination_uses_stable_tie_order_and_rejects_changed_or_mismatched_cursors() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let mut ids = vec![];
    for index in 0..7 {
        ids.push(
            library
                .create_note(&format!("笔记{index}"), &"真实正文".repeat(90))
                .unwrap()
                .id,
        );
    }
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute("UPDATE notes SET updated_at='2026-10-02T00:00:00Z'", [])
        .unwrap();
    ids.sort();
    let mut collected = vec![];
    let mut cursor = None;
    loop {
        let page = library
            .notes_page(NotesPageRequest {
                limit: 2,
                cursor,
                trashed: false,
                filter: None,
                sort: None,
                query: None,
            })
            .unwrap();
        assert!(
            page.items
                .iter()
                .all(|note| note.preview.chars().count() == 180)
        );
        collected.extend(page.items.into_iter().map(|note| note.id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(collected, ids);
    let query = NotesPageRequest {
        limit: 2,
        cursor: None,
        trashed: false,
        filter: None,
        sort: None,
        query: None,
    };
    let cursor = library.notes_page(query.clone()).unwrap().next_cursor;
    assert!(
        library
            .notes_page(NotesPageRequest {
                trashed: true,
                cursor: cursor.clone(),
                ..query.clone()
            })
            .is_err()
    );
    library.set_trashed(&ids[0], true).unwrap();
    assert!(matches!(
        library.notes_page(NotesPageRequest {
            cursor,
            ..query.clone()
        }),
        Err(Error::QueryExpired)
    ));
    let trash = library
        .notes_page(NotesPageRequest {
            trashed: true,
            ..query.clone()
        })
        .unwrap();
    assert_eq!(trash.items.len(), 1);
    assert_eq!(trash.items[0].id, ids[0]);
    for limit in [0, 101, u32::MAX] {
        assert!(
            library
                .notes_page(NotesPageRequest {
                    limit,
                    ..query.clone()
                })
                .is_err()
        );
    }
    for cursor in ["invalid".into(), "x".repeat(257)] {
        assert!(
            library
                .notes_page(NotesPageRequest {
                    cursor: Some(cursor),
                    ..query.clone()
                })
                .is_err()
        );
    }
    let plan: Vec<String> = raw.prepare("EXPLAIN QUERY PLAN SELECT n.id FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id JOIN note_previews p ON p.note_id=n.id WHERE n.trashed=0 ORDER BY n.updated_at DESC,n.id LIMIT 20").unwrap()
        .query_map([], |row| row.get(3)).unwrap().collect::<Result<_, _>>().unwrap();
    assert!(
        plan.iter().any(|line| line.contains("notes_page_order")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|line| line.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

#[test]
fn revision_summaries_are_bounded_and_preview_changes_with_saved_revision() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let mut note = library.create_note("历史", "第一版").unwrap();
    for revision in 2..=5 {
        note = library
            .save_note(&note.id, note.revision, "历史", &format!("第{revision}版"))
            .unwrap();
    }
    let page = library.revision_page(&note.id, None, 2).unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.revision)
            .collect::<Vec<_>>(),
        vec![5, 4]
    );
    let next = library
        .revision_page(&note.id, page.next_before, 2)
        .unwrap();
    assert_eq!(
        next.items
            .iter()
            .map(|item| item.revision)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert_eq!(
        library
            .revision_page(&note.id, next.next_before, 2)
            .unwrap()
            .items[0]
            .revision,
        1
    );
    assert!(matches!(
        library.revision_page(&uuid::Uuid::new_v4().to_string(), None, 2),
        Err(Error::NotFound)
    ));
    let page = library
        .notes_page(NotesPageRequest {
            limit: 1,
            cursor: None,
            trashed: false,
            filter: None,
            sort: None,
            query: None,
        })
        .unwrap();
    assert_eq!(page.items[0].preview, "第5版");
}
