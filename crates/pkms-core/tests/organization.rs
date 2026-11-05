use pkms_core::{
    AssignTag, Error, InboxState, Library, MetadataTarget, NotesFilter, NotesPageRequest,
    RemoveTag, SetInboxState, TagSelector, TagsPageRequest,
};
use rusqlite::Connection;
use std::sync::{Arc, Barrier};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn target(library: &Library, object_id: &str) -> MetadataTarget {
    MetadataTarget {
        object_id: object_id.into(),
        base_row_version: library.object_metadata(object_id).unwrap().row_version,
    }
}
fn named(targets: Vec<MetadataTarget>, name: &str) -> AssignTag {
    AssignTag {
        targets,
        tag: TagSelector::Name { name: name.into() },
    }
}
fn tags() -> TagsPageRequest {
    TagsPageRequest {
        limit: 100,
        cursor: None,
        query: None,
    }
}
fn notes() -> NotesPageRequest {
    NotesPageRequest {
        limit: 100,
        cursor: None,
        trashed: false,
        filter: None,
        sort: None,
        query: None,
    }
}
fn count(raw: &Connection, table: &str) -> i64 {
    raw.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn organization_is_independent_of_content_and_preserves_shared_tags_on_removal() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("第一篇", "正文不变").unwrap();
    let second = library.create_note("第二篇", "另一篇正文").unwrap();
    let collection = library.create_collection("主题").unwrap();
    library
        .add_to_collection(&collection.id, &first.id)
        .unwrap();
    assert!(library.tags_page(tags()).unwrap().items.is_empty());
    let batch = vec![
        target(&library, &first.id),
        target(&library, &second.id),
        target(&library, &collection.id),
    ];
    let result = library.assign_tag(&id(), named(batch, " Rust ")).unwrap();
    assert!(result.tag_created);
    let tag = result.tag.unwrap();
    assert_eq!(tag.display_name, "Rust");
    assert_eq!(tag.normalized_name, "rust");
    assert!(
        result
            .objects
            .iter()
            .all(|object| object.tags == [tag.clone()])
    );
    assert_eq!(result.objects[0].row_version, 2);
    assert_eq!(result.objects[1].row_version, 2);
    assert_eq!(result.objects[2].row_version, 3);
    assert_eq!(result.objects[0].inbox_state, Some(InboxState::Pending));
    assert_eq!(result.objects[2].inbox_state, None);
    let noop = library
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![target(&library, &first.id)],
                tag: TagSelector::Id { id: tag.id.clone() },
            },
        )
        .unwrap();
    assert!(!noop.tag_created);
    assert_eq!(noop.objects[0].row_version, 2);
    let processed = library
        .set_inbox_state(
            &id(),
            SetInboxState {
                object_id: first.id.clone(),
                base_row_version: 2,
                target: InboxState::Processed,
            },
        )
        .unwrap();
    assert_eq!(processed.objects[0].row_version, 3);
    assert_eq!(
        processed.objects[0].tags.as_slice(),
        std::slice::from_ref(&tag)
    );
    let removed = library
        .remove_tag(
            &id(),
            RemoveTag {
                targets: vec![target(&library, &first.id)],
                tag_id: tag.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(removed.objects[0].row_version, 4);
    assert!(removed.objects[0].tags.is_empty());
    assert_eq!(removed.objects[0].inbox_state, Some(InboxState::Processed));
    assert_eq!(
        library.object_metadata(&second.id).unwrap().tags.as_slice(),
        std::slice::from_ref(&tag)
    );
    assert_eq!(library.tags_page(tags()).unwrap().items, [tag]);
    assert_eq!(library.note(&first.id).unwrap(), first);
    assert_eq!(library.note(&second.id).unwrap(), second);
    assert_eq!(library.revisions(&first.id).unwrap().len(), 1);
    assert_eq!(
        library.collection_notes(&collection.id).unwrap()[0].id,
        first.id
    );
    assert!(matches!(
        library.set_inbox_state(
            &id(),
            SetInboxState {
                object_id: collection.id,
                base_row_version: 3,
                target: InboxState::Processed,
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    assert_eq!(count(&raw, "note_revisions"), 2);
    assert_eq!(raw.query_row("SELECT count(*) FROM command_receipts WHERE command_kind LIKE 'tags.%' AND revision_id IS NOT NULL", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
}

#[test]
fn invalid_batch_and_late_sql_failure_roll_back_tags_versions_receipts_and_cursors() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("甲", "正文").unwrap();
    let second = library.create_note("乙", "正文").unwrap();
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let original_receipts = count(&raw, "command_receipts");
    let generation: i64 = raw
        .query_row("SELECT generation FROM note_list_state", [], |r| r.get(0))
        .unwrap();
    let bad = named(
        vec![
            target(&library, &first.id),
            MetadataTarget {
                object_id: id(),
                base_row_version: 1,
            },
        ],
        "不应留下空标签",
    );
    assert!(matches!(
        library.assign_tag(&id(), bad),
        Err(Error::NotFound)
    ));
    assert_eq!(count(&raw, "tags"), 0);
    raw.execute_batch("CREATE TRIGGER fail_second_tag BEFORE INSERT ON object_tags WHEN (SELECT count(*) FROM object_tags)>0 BEGIN SELECT RAISE(ABORT,'test failure after first insert'); END;").unwrap();
    let command_id = id();
    let request = named(
        vec![target(&library, &first.id), target(&library, &second.id)],
        "原子标签",
    );
    assert!(library.assign_tag(&command_id, request.clone()).is_err());
    assert_eq!(count(&raw, "tags"), 0);
    assert_eq!(count(&raw, "object_tags"), 0);
    assert_eq!(count(&raw, "command_receipts"), original_receipts);
    assert_eq!(library.object_metadata(&first.id).unwrap().row_version, 1);
    assert_eq!(
        raw.query_row("SELECT generation FROM note_list_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    raw.execute_batch("DROP TRIGGER fail_second_tag").unwrap();
    let result = library.assign_tag(&command_id, request.clone()).unwrap();
    assert!(!result.receipt.replayed);
    assert_eq!(count(&raw, "object_tags"), 2);
    assert!(matches!(
        library.assign_tag(&id(), request),
        Err(Error::MetadataConflict {
            expected: 1,
            actual: 2,
            ..
        })
    ));
    let tag_id = result.tag.unwrap().id;
    let batch = RemoveTag {
        targets: vec![
            target(&library, &first.id),
            MetadataTarget {
                object_id: second.id.clone(),
                base_row_version: 1,
            },
        ],
        tag_id,
    };
    assert!(matches!(
        library.remove_tag(&id(), batch),
        Err(Error::MetadataConflict { .. })
    ));
    assert_eq!(count(&raw, "object_tags"), 2);
    assert_eq!(library.object_metadata(&first.id).unwrap().row_version, 2);
}

#[test]
fn tag_names_use_nfc_ascii_case_and_codepoint_limits_without_hidden_hierarchy() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("规范化", "正文").unwrap();
    let first = library
        .assign_tag(
            &id(),
            named(vec![target(&library, &note.id)], "  Cafe\u{301}/RUST  "),
        )
        .unwrap();
    let tag = first.tag.unwrap();
    assert_eq!(tag.display_name, "Café/RUST");
    assert_eq!(tag.normalized_name, "café/rust");
    let again = library
        .assign_tag(&id(), named(vec![target(&library, &note.id)], "café/rust"))
        .unwrap();
    assert_eq!(again.tag.unwrap(), tag);
    assert!(!again.tag_created);
    assert_eq!(again.objects[0].row_version, 2);
    for name in [
        "CAFÉ/RUST".to_owned(),
        "a  b".to_owned(),
        "a b".to_owned(),
        "🙂".repeat(100),
    ] {
        assert!(
            library
                .assign_tag(&id(), named(vec![target(&library, &note.id)], &name))
                .unwrap()
                .tag_created
        );
    }
    for name in ["   ".to_owned(), "🙂".repeat(101), "a\0b".to_owned()] {
        assert!(matches!(
            library.assign_tag(&id(), named(vec![target(&library, &note.id)], &name)),
            Err(Error::InvalidInput(_))
        ));
    }
    let same = target(&library, &note.id);
    for targets in [
        vec![],
        vec![same.clone(), same.clone()],
        vec![MetadataTarget {
            base_row_version: 0,
            ..same.clone()
        }],
        vec![same; 101],
    ] {
        assert!(matches!(
            library.assign_tag(&id(), named(targets, "不得创建")),
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(library.tags_page(tags()).unwrap().items.len(), 5);
    assert_eq!(
        library.object_metadata(&note.id).unwrap().inbox_state,
        Some(InboxState::Pending)
    );
}

#[test]
fn command_replay_survives_restart_and_returns_original_metadata_even_after_trashing() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("可重试", "旧正文").unwrap();
    let command_id = id();
    let request = named(vec![target(&library, &note.id)], "标签");
    let original = library.assign_tag(&command_id, request.clone()).unwrap();
    let inbox_request = SetInboxState {
        object_id: note.id.clone(),
        base_row_version: 2,
        target: InboxState::Snoozed,
    };
    let inbox_command = id();
    let inbox_result = library
        .set_inbox_state(&inbox_command, inbox_request.clone())
        .unwrap();
    assert!(matches!(
        library.set_inbox_state(&command_id, inbox_request.clone()),
        Err(Error::IdempotencyMismatch)
    ));
    let remove_command = id();
    let remove_request = RemoveTag {
        targets: vec![target(&library, &note.id)],
        tag_id: original.tag.as_ref().unwrap().id.clone(),
    };
    let remove_result = library
        .remove_tag(&remove_command, remove_request.clone())
        .unwrap();
    library
        .save_note(&note.id, note.revision, "新标题", "新正文")
        .unwrap();
    library.set_trashed(&note.id, true).unwrap();
    drop(library);
    let mut reopened = Library::open_existing(root.path()).unwrap();
    let replay = reopened.assign_tag(&command_id, request.clone()).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.objects, original.objects);
    assert_eq!(replay.tag, original.tag);
    assert!(replay.tag_created);
    assert_eq!(replay.receipt.change_seq, original.receipt.change_seq);
    assert_eq!(
        reopened
            .set_inbox_state(&inbox_command, inbox_request)
            .unwrap()
            .objects,
        inbox_result.objects
    );
    assert_eq!(
        reopened
            .remove_tag(&remove_command, remove_request)
            .unwrap()
            .objects,
        remove_result.objects
    );
    assert!(reopened.object_metadata(&note.id).unwrap().trashed);
    assert!(matches!(
        reopened.assign_tag(&id(), request.clone()),
        Err(Error::Trashed)
    ));
    assert!(matches!(
        reopened.assign_tag(
            &command_id,
            AssignTag {
                tag: TagSelector::Name {
                    name: "不同内容".into()
                },
                ..request
            }
        ),
        Err(Error::IdempotencyMismatch)
    ));
}

#[test]
fn two_connections_share_one_normalized_tag_and_reject_stale_metadata_writers() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("甲", "正文").unwrap();
    let second = library.create_note("乙", "正文").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles = [
        (first.id.clone(), "Cafe\u{301}/RUST"),
        (second.id, "Café/rust"),
    ]
    .map(|(object_id, name)| {
        let barrier = barrier.clone();
        let path = root.path().to_path_buf();
        std::thread::spawn(move || {
            let mut library = Library::open_existing(path).unwrap();
            let request = named(vec![target(&library, &object_id)], name);
            barrier.wait();
            library.assign_tag(&id(), request).unwrap()
        })
    });
    let [a, b] = handles.map(|thread| thread.join().unwrap());
    assert_eq!(a.tag, b.tag);
    assert_ne!(a.tag_created, b.tag_created);
    assert_eq!(library.tags_page(tags()).unwrap().items.len(), 1);

    let handles = ["竞争A", "竞争B"].map(|name| {
        let barrier = barrier.clone();
        let path = root.path().to_path_buf();
        let object_id = first.id.clone();
        std::thread::spawn(move || {
            let mut library = Library::open_existing(path).unwrap();
            let request = named(vec![target(&library, &object_id)], name);
            barrier.wait();
            library.assign_tag(&id(), request)
        })
    });
    let results = handles.map(|thread| thread.join().unwrap());
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(
                r,
                Err(Error::MetadataConflict {
                    expected: 2,
                    actual: 3,
                    ..
                })
            ))
            .count(),
        1
    );
    assert_eq!(library.tags_page(tags()).unwrap().items.len(), 2);
}

#[test]
fn filtered_note_pages_bind_cursors_and_invalidate_after_organization_changes() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("甲", "正文甲").unwrap();
    let second = library.create_note("乙", "正文乙").unwrap();
    let third = library.create_note("丙", "正文丙").unwrap();
    let a = library
        .assign_tag(
            &id(),
            named(
                vec![target(&library, &first.id), target(&library, &second.id)],
                "A",
            ),
        )
        .unwrap()
        .tag
        .unwrap();
    let b = library
        .assign_tag(
            &id(),
            named(
                vec![target(&library, &second.id), target(&library, &third.id)],
                "B",
            ),
        )
        .unwrap()
        .tag
        .unwrap();
    let both = library
        .notes_page(NotesPageRequest {
            filter: Some(NotesFilter {
                inbox_state: None,
                tag_ids: vec![a.id.clone(), b.id.clone()],
                ..Default::default()
            }),
            ..notes()
        })
        .unwrap();
    assert_eq!(both.items.len(), 1);
    assert_eq!(both.items[0].id, second.id);
    assert_eq!(both.items[0].row_version, 3);
    assert!(
        !serde_json::to_string(&both)
            .unwrap()
            .contains("document_json")
    );
    let filtered = NotesPageRequest {
        limit: 1,
        filter: Some(NotesFilter {
            inbox_state: Some(InboxState::Pending),
            tag_ids: vec![a.id.clone()],
            ..Default::default()
        }),
        ..notes()
    };
    let page = library.notes_page(filtered.clone()).unwrap();
    let cursor = page.next_cursor.unwrap();
    assert!(matches!(
        library.notes_page(NotesPageRequest {
            cursor: Some(cursor.clone()),
            filter: Some(NotesFilter {
                inbox_state: None,
                tag_ids: vec![b.id],
                ..Default::default()
            }),
            ..filtered.clone()
        }),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        library
            .notes_page(NotesPageRequest {
                cursor: Some(cursor.clone()),
                ..filtered.clone()
            })
            .unwrap()
            .items
            .len(),
        1
    );
    library
        .set_inbox_state(
            &id(),
            SetInboxState {
                object_id: second.id.clone(),
                base_row_version: 3,
                target: InboxState::Processed,
            },
        )
        .unwrap();
    assert!(matches!(
        library.notes_page(NotesPageRequest {
            cursor: Some(cursor),
            ..filtered.clone()
        }),
        Err(Error::QueryExpired)
    ));
    assert_eq!(library.notes_page(filtered).unwrap().items[0].id, first.id);
    let processed = library
        .notes_page(NotesPageRequest {
            filter: Some(NotesFilter {
                inbox_state: Some(InboxState::Processed),
                tag_ids: vec![a.id],
                ..Default::default()
            }),
            ..notes()
        })
        .unwrap();
    assert_eq!(processed.items[0].id, second.id);
    let old_cursor = library
        .notes_page(NotesPageRequest {
            limit: 1,
            ..notes()
        })
        .unwrap()
        .next_cursor;
    library
        .assign_tag(&id(), named(vec![target(&library, &third.id)], "新标签"))
        .unwrap();
    assert!(matches!(
        library.notes_page(NotesPageRequest {
            cursor: old_cursor,
            ..notes()
        }),
        Err(Error::QueryExpired)
    ));
    let missing_tag = library
        .notes_page(NotesPageRequest {
            filter: Some(NotesFilter {
                inbox_state: None,
                tag_ids: vec![id()],
                ..Default::default()
            }),
            ..notes()
        })
        .unwrap();
    assert!(missing_tag.items.is_empty());
}

#[test]
fn tag_browser_is_bounded_searches_literal_text_and_invalidates_only_on_tag_changes() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("标签浏览", "正文").unwrap();
    for name in ["Rust", "Rust 100%", "Rust_更多", "a' OR 1=1 --"] {
        library
            .assign_tag(&id(), named(vec![target(&library, &note.id)], name))
            .unwrap();
    }
    let request = TagsPageRequest {
        limit: 1,
        query: Some("RUST".into()),
        ..tags()
    };
    let first = library.tags_page(request.clone()).unwrap();
    assert_eq!(first.items.len(), 1);
    let cursor = first.next_cursor.unwrap();
    assert!(matches!(
        library.tags_page(TagsPageRequest {
            cursor: Some(cursor.clone()),
            query: Some("other".into()),
            ..request.clone()
        }),
        Err(Error::InvalidInput(_))
    ));
    library
        .save_note(&note.id, note.revision, "更新正文", "更新")
        .unwrap();
    assert!(
        library
            .tags_page(TagsPageRequest {
                cursor: Some(cursor.clone()),
                ..request.clone()
            })
            .is_ok()
    );
    library
        .assign_tag(&id(), named(vec![target(&library, &note.id)], "新增"))
        .unwrap();
    assert!(matches!(
        library.tags_page(TagsPageRequest {
            cursor: Some(cursor),
            ..request
        }),
        Err(Error::QueryExpired)
    ));
    for query in ["%", "_", "' OR 1=1"] {
        assert_eq!(
            library
                .tags_page(TagsPageRequest {
                    query: Some(query.into()),
                    ..tags()
                })
                .unwrap()
                .items
                .len(),
            1
        );
    }
    assert!(
        library
            .tags_page(TagsPageRequest {
                limit: 101,
                ..tags()
            })
            .is_err()
    );
}
