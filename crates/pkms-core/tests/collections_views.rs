use pkms_core::*;
use rusqlite::Connection;
use std::sync::{Arc, Barrier};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn collection(name: &str) -> CreateCollection {
    CreateCollection {
        name: name.into(),
        overview_note_id: None,
        is_pinned: false,
        pin_position: None,
    }
}
fn members(library: &Library, collection_id: &str, ids: Vec<String>) -> ChangeCollectionMembers {
    ChangeCollectionMembers {
        collection_id: collection_id.into(),
        base_row_version: library
            .collection_get(collection_id)
            .unwrap()
            .metadata
            .row_version,
        member_ids: ids,
    }
}
fn member_page(collection_id: &str) -> CollectionMembersRequest {
    CollectionMembersRequest {
        collection_id: collection_id.into(),
        limit: 100,
        cursor: None,
        include_trashed: false,
        kind: None,
    }
}
fn collections() -> CollectionPageRequest {
    CollectionPageRequest {
        limit: 100,
        cursor: None,
        trashed: false,
        archived: false,
        pinned_only: false,
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
fn flag(library: &Library, object_id: &str, value: bool) -> SetObjectFlag {
    SetObjectFlag {
        object_id: object_id.into(),
        base_row_version: library.object_metadata(object_id).unwrap().row_version,
        value,
    }
}
fn view(name: &str) -> SaveView {
    SaveView {
        view_id: None,
        base_row_version: None,
        name: name.into(),
        filter_schema: 1,
        filter: NotesFilter::default(),
        sort: NoteSort::TitleAsc,
        layout: ViewLayout::Table,
        config_schema: 1,
        config: ViewConfig::default(),
        is_pinned: false,
        pin_position: None,
    }
}
fn edit_view(view: &SavedView) -> SaveView {
    SaveView {
        view_id: Some(view.id.clone()),
        base_row_version: Some(view.row_version),
        name: view.name.clone(),
        filter_schema: view.filter_schema,
        filter: view.filter.clone(),
        sort: view.sort,
        layout: view.layout,
        config_schema: view.config_schema,
        config: view.config.clone(),
        is_pinned: view.is_pinned,
        pin_position: view.pin_position,
    }
}
fn views() -> ViewPageRequest {
    ViewPageRequest {
        limit: 100,
        cursor: None,
        pinned_only: false,
        query: None,
    }
}
fn scalar(raw: &Connection, sql: &str) -> i64 {
    raw.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn collection_order_overview_and_multiple_memberships_preserve_note_identity_and_revisions() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let a = library.create_note("甲", "独立正文").unwrap();
    let b = library.create_note("乙", "另一正文").unwrap();
    let intro = library.create_note("主题介绍", "普通笔记").unwrap();
    let create_id = id();
    let request = CreateCollection {
        overview_note_id: Some(intro.id.clone()),
        is_pinned: true,
        pin_position: Some(4),
        ..collection("研究")
    };
    let original = library
        .collection_create(&create_id, request.clone())
        .unwrap();
    let first = original.collection;
    let second = library
        .collection_create(&id(), collection("写作"))
        .unwrap()
        .collection;
    let added = library
        .collection_add_members(
            &id(),
            members(&library, &first.id, vec![a.id.clone(), b.id.clone()]),
        )
        .unwrap();
    assert_eq!(added.collection.metadata.row_version, 2);
    assert_eq!(added.collection.member_count, 2);
    assert_eq!(added.collection.overview_note_id, Some(intro.id.clone()));
    library
        .collection_add_members(&id(), members(&library, &second.id, vec![a.id.clone()]))
        .unwrap();
    let noop = library
        .collection_add_members(&id(), members(&library, &first.id, vec![a.id.clone()]))
        .unwrap();
    assert_eq!(noop.collection.metadata.row_version, 2);
    let reorder_id = id();
    let reorder = members(&library, &first.id, vec![b.id.clone(), a.id.clone()]);
    let result = library
        .collection_reorder(&reorder_id, reorder.clone())
        .unwrap();
    assert_eq!(result.collection.metadata.row_version, 3);
    let page = library
        .collection_members_page(member_page(&first.id))
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|m| m.object.id.clone())
            .collect::<Vec<_>>(),
        [b.id.clone(), a.id.clone()]
    );
    assert_eq!(
        page.items.iter().map(|m| m.position).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(page.items.iter().all(|m| m.note.is_some()));
    assert!(matches!(
        library.collection_reorder(&id(), members(&library, &first.id, vec![a.id.clone()])),
        Err(Error::InvalidInput(_))
    ));
    let changed = library
        .collection_update(
            &id(),
            UpdateCollection {
                collection_id: first.id.clone(),
                base_row_version: 3,
                name: "新主题名".into(),
                overview_note_id: Some(intro.id.clone()),
                is_pinned: true,
                pin_position: Some(1),
            },
        )
        .unwrap();
    assert_eq!(changed.collection.id, first.id);
    assert_eq!(changed.collection.metadata.row_version, 4);
    let replay = library.collection_reorder(&reorder_id, reorder).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.collection.name, "研究");
    assert_eq!(replay.collection.metadata.row_version, 3);
    assert_eq!(
        library
            .collection_create(&create_id, request)
            .unwrap()
            .collection,
        first
    );
    library
        .collection_remove_members(&id(), members(&library, &first.id, vec![a.id.clone()]))
        .unwrap();
    assert_eq!(library.collection_get(&first.id).unwrap().member_count, 1);
    assert_eq!(library.collection_get(&second.id).unwrap().member_count, 1);
    assert_eq!(library.note(&a.id).unwrap(), a);
    assert_eq!(library.note(&b.id).unwrap(), b);
    assert_eq!(library.note(&intro.id).unwrap(), intro);
    assert_eq!(library.object_metadata(&a.id).unwrap().row_version, 1);
    library
        .collection_add_members(&id(), members(&library, &second.id, vec![b.id.clone()]))
        .unwrap();
    library
        .collection_remove_members(&id(), members(&library, &second.id, vec![a.id.clone()]))
        .unwrap();
    assert_eq!(
        library
            .collection_members_page(member_page(&second.id))
            .unwrap()
            .items[0]
            .position,
        1
    );
    // Reordering the same sequence also compacts gaps left by removed members.
    library
        .collection_reorder(&id(), members(&library, &second.id, vec![b.id.clone()]))
        .unwrap();
    assert_eq!(
        library
            .collection_members_page(member_page(&second.id))
            .unwrap()
            .items[0]
            .position,
        0
    );
}

#[test]
fn collection_failures_roll_back_entire_batch_and_never_leave_unreceipted_objects() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let a = library.create_note("甲", "正文").unwrap();
    let b = library.create_note("乙", "正文").unwrap();
    let c = library
        .collection_create(&id(), collection("主题"))
        .unwrap()
        .collection;
    assert!(matches!(
        library.collection_add_members(&id(), members(&library, &c.id, vec![a.id.clone(), id()])),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        library.collection_add_members(&id(), members(&library, &c.id, vec![c.id.clone()])),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        library.collection_add_members(
            &id(),
            members(&library, &c.id, vec![a.id.clone(), a.id.clone()])
        ),
        Err(Error::InvalidInput(_))
    ));
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let original_receipts = scalar(&raw, "SELECT count(*) FROM command_receipts");
    let original_generation = scalar(&raw, "SELECT generation FROM collection_list_state");
    raw.execute_batch("CREATE TRIGGER fail_second_member BEFORE INSERT ON memberships WHEN (SELECT count(*) FROM memberships)>0 BEGIN SELECT RAISE(ABORT,'test second member failure'); END;").unwrap();
    let command = id();
    let batch = members(&library, &c.id, vec![a.id, b.id]);
    assert!(
        library
            .collection_add_members(&command, batch.clone())
            .is_err()
    );
    assert_eq!(library.collection_get(&c.id).unwrap(), c);
    assert_eq!(scalar(&raw, "SELECT count(*) FROM memberships"), 0);
    assert_eq!(
        scalar(&raw, "SELECT count(*) FROM command_receipts"),
        original_receipts
    );
    assert_eq!(
        scalar(&raw, "SELECT generation FROM collection_list_state"),
        original_generation
    );
    raw.execute_batch("DROP TRIGGER fail_second_member")
        .unwrap();
    assert!(
        !library
            .collection_add_members(&command, batch)
            .unwrap()
            .receipt
            .replayed
    );
    let count = scalar(&raw, "SELECT count(*) FROM objects");
    raw.execute_batch("CREATE TRIGGER fail_collection_receipt BEFORE INSERT ON command_receipts WHEN NEW.command_kind='collections.create' BEGIN SELECT RAISE(ABORT,'test receipt failure'); END;").unwrap();
    let command = id();
    assert!(
        library
            .collection_create(&command, collection("失败不留空主题"))
            .is_err()
    );
    assert_eq!(scalar(&raw, "SELECT count(*) FROM objects"), count);
    raw.execute_batch("DROP TRIGGER fail_collection_receipt")
        .unwrap();
    assert!(
        !library
            .collection_create(&command, collection("失败不留空主题"))
            .unwrap()
            .receipt
            .replayed
    );
}

#[test]
fn collection_trash_archive_and_search_switch_are_independent_and_do_not_cascade() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let a = library.create_note("甲", "knowledge").unwrap();
    let b = library.create_note("乙", "knowledge").unwrap();
    let c = library
        .collection_create(
            &id(),
            CreateCollection {
                overview_note_id: Some(a.id.clone()),
                ..collection("主题")
            },
        )
        .unwrap()
        .collection;
    library
        .collection_add_members(
            &id(),
            members(&library, &c.id, vec![a.id.clone(), b.id.clone()]),
        )
        .unwrap();
    let trash_request = flag(&library, &c.id, true);
    let command = id();
    library
        .set_object_trashed(&command, trash_request.clone())
        .unwrap();
    assert!(
        library
            .collections_page(collections())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        library
            .collections_page(CollectionPageRequest {
                trashed: true,
                ..collections()
            })
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(library.search("knowledge", 10).unwrap().len(), 2);
    assert!(!library.object_metadata(&a.id).unwrap().trashed);
    library
        .set_object_trashed(&id(), flag(&library, &a.id, true))
        .unwrap();
    library
        .set_object_trashed(&id(), flag(&library, &c.id, false))
        .unwrap();
    assert!(library.object_metadata(&a.id).unwrap().trashed);
    assert_eq!(
        library.collection_get(&c.id).unwrap().overview_note_id,
        Some(a.id.clone())
    );
    assert_eq!(library.collection_get(&c.id).unwrap().member_count, 2);
    assert_eq!(
        library
            .collection_members_page(member_page(&c.id))
            .unwrap()
            .items[0]
            .object
            .id,
        b.id
    );
    assert_eq!(
        library
            .collection_members_page(CollectionMembersRequest {
                include_trashed: true,
                ..member_page(&c.id)
            })
            .unwrap()
            .items
            .len(),
        2
    );
    assert!(
        library
            .set_object_trashed(&command, trash_request)
            .unwrap()
            .receipt
            .replayed
    );
    assert!(!library.object_metadata(&c.id).unwrap().trashed);
    library
        .set_object_archived(&id(), flag(&library, &b.id, true))
        .unwrap();
    assert!(library.notes_page(notes()).unwrap().items.is_empty());
    assert!(library.list_notes().unwrap().is_empty());
    assert_eq!(library.search("knowledge", 10).unwrap().len(), 1);
    let archived = library
        .notes_page(NotesPageRequest {
            filter: Some(NotesFilter {
                archived: Some(true),
                ..Default::default()
            }),
            ..notes()
        })
        .unwrap();
    assert_eq!(archived.items[0].id, b.id);
    library
        .set_object_search_enabled(&id(), flag(&library, &b.id, false))
        .unwrap();
    assert!(library.search("knowledge", 10).unwrap().is_empty());
    assert!(!library.object_metadata(&b.id).unwrap().trashed);
    library
        .set_object_trashed(&id(), flag(&library, &b.id, true))
        .unwrap();
    assert_eq!(
        library
            .notes_page(NotesPageRequest {
                trashed: true,
                ..notes()
            })
            .unwrap()
            .items
            .len(),
        2
    );
    assert_eq!(library.revisions(&a.id).unwrap().len(), 1);
    assert_eq!(library.revisions(&b.id).unwrap().len(), 1);
}

#[test]
fn collection_queries_keep_pin_order_and_expire_members_after_note_changes() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let a = library.create_note("甲", "正文").unwrap();
    let b = library.create_note("乙", "正文").unwrap();
    let first = library
        .collection_create(
            &id(),
            CreateCollection {
                is_pinned: true,
                pin_position: Some(3),
                ..collection("A")
            },
        )
        .unwrap()
        .collection;
    let second = library
        .collection_create(
            &id(),
            CreateCollection {
                is_pinned: true,
                pin_position: Some(1),
                ..collection("Z")
            },
        )
        .unwrap()
        .collection;
    let other = library
        .collection_create(&id(), collection("AA"))
        .unwrap()
        .collection;
    let page = library
        .collections_page(CollectionPageRequest {
            limit: 1,
            ..collections()
        })
        .unwrap();
    assert_eq!(page.items[0].id, second.id);
    let next = library
        .collections_page(CollectionPageRequest {
            limit: 1,
            cursor: page.next_cursor,
            ..collections()
        })
        .unwrap();
    assert_eq!(next.items[0].id, first.id);
    assert_eq!(
        library
            .collections_page(CollectionPageRequest {
                cursor: next.next_cursor,
                ..collections()
            })
            .unwrap()
            .items[0]
            .id,
        other.id
    );
    library
        .collection_add_members(
            &id(),
            members(&library, &first.id, vec![a.id.clone(), b.id]),
        )
        .unwrap();
    let page = library
        .collection_members_page(CollectionMembersRequest {
            limit: 1,
            ..member_page(&first.id)
        })
        .unwrap();
    library
        .save_note(&a.id, a.revision, "改名", "正文更新")
        .unwrap();
    assert!(matches!(
        library.collection_members_page(CollectionMembersRequest {
            cursor: page.next_cursor,
            ..member_page(&first.id)
        }),
        Err(Error::QueryExpired)
    ));
    assert_eq!(
        library
            .collections_page(CollectionPageRequest {
                pinned_only: true,
                ..collections()
            })
            .unwrap()
            .items
            .len(),
        2
    );
    assert!(
        library
            .collections_page(CollectionPageRequest {
                query: Some("%' OR 1=1".into()),
                ..collections()
            })
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn saved_views_execute_real_conjunctive_filters_and_all_sort_orders_with_stable_ties() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let mut records = vec![];
    for title in ["Beta", "Alpha", "Alpha", "Gamma"] {
        records.push(library.create_note(title, "真实正文").unwrap());
    }
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute("UPDATE notes SET updated_at='2026-10-02T00:00:00Z'", [])
        .unwrap();
    for sort in [
        NoteSort::TitleAsc,
        NoteSort::TitleDesc,
        NoteSort::UpdatedAsc,
        NoteSort::UpdatedDesc,
    ] {
        let saved = library
            .view_save(
                &id(),
                SaveView {
                    sort,
                    ..view("排序视图")
                },
            )
            .unwrap()
            .view;
        let mut actual = vec![];
        let mut cursor = None;
        loop {
            let page = library.view_run(&saved.id, 1, cursor).unwrap();
            actual.extend(page.notes.items.iter().map(|n| n.note.id.clone()));
            cursor = page.notes.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let mut expected = records.clone();
        expected.sort_by(|a, b| match sort {
            NoteSort::TitleAsc => a.title.cmp(&b.title).then(a.id.cmp(&b.id)),
            NoteSort::TitleDesc => b.title.cmp(&a.title).then(a.id.cmp(&b.id)),
            _ => a.id.cmp(&b.id),
        });
        assert_eq!(
            actual,
            expected.iter().map(|n| n.id.clone()).collect::<Vec<_>>()
        );
    }
    let c = library
        .collection_create(&id(), collection("筛选主题"))
        .unwrap()
        .collection;
    library
        .collection_add_members(
            &id(),
            members(
                &library,
                &c.id,
                vec![records[0].id.clone(), records[1].id.clone()],
            ),
        )
        .unwrap();
    let targets = records
        .iter()
        .skip(1)
        .map(|n| MetadataTarget {
            object_id: n.id.clone(),
            base_row_version: 1,
        })
        .collect();
    let tag = library
        .assign_tag(
            &id(),
            AssignTag {
                targets,
                tag: TagSelector::Name {
                    name: "Rust".into(),
                },
            },
        )
        .unwrap()
        .tag
        .unwrap();
    let saved = library
        .view_save(
            &id(),
            SaveView {
                filter: NotesFilter {
                    tag_ids: vec![tag.id],
                    collection_id: Some(c.id),
                    inbox_state: Some(InboxState::Pending),
                    ..Default::default()
                },
                ..view("交集")
            },
        )
        .unwrap()
        .view;
    let result = library.view_run(&saved.id, 100, None).unwrap();
    assert_eq!(result.notes.items.len(), 1);
    assert_eq!(result.notes.items[0].note.id, records[1].id);
    assert_eq!(result.view.layout, ViewLayout::Table);
    library
        .set_inbox_state(
            &id(),
            SetInboxState {
                object_id: records[1].id.clone(),
                base_row_version: 2,
                target: InboxState::Processed,
            },
        )
        .unwrap();
    assert!(
        library
            .view_run(&saved.id, 100, None)
            .unwrap()
            .notes
            .items
            .is_empty()
    );
}

#[test]
fn saved_view_versions_scope_cursors_and_retries_preserve_deleted_view_snapshots() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    library.create_note("一", "正文").unwrap();
    library.create_note("二", "正文").unwrap();
    let create_id = id();
    let request = SaveView {
        is_pinned: true,
        pin_position: Some(5),
        ..view("常用视图")
    };
    let first = library.view_save(&create_id, request.clone()).unwrap().view;
    let cursor = library
        .view_run(&first.id, 1, None)
        .unwrap()
        .notes
        .next_cursor;
    let another = library.view_save(&id(), view("其他")).unwrap().view;
    assert!(matches!(
        library.view_run(&another.id, 1, cursor.clone()),
        Err(Error::InvalidInput(_))
    ));
    let update = SaveView {
        name: "已更新".into(),
        pin_position: Some(1),
        ..edit_view(&first)
    };
    let updated = library.view_save(&id(), update.clone()).unwrap().view;
    assert!(matches!(
        library.view_run(&first.id, 1, cursor),
        Err(Error::QueryExpired)
    ));
    assert!(matches!(
        library.view_save(&id(), update),
        Err(Error::MetadataConflict { .. })
    ));
    let removal_id = id();
    let removal = RemoveView {
        view_id: first.id.clone(),
        base_row_version: updated.row_version,
    };
    let deleted = library.view_remove(&removal_id, removal.clone()).unwrap();
    assert!(deleted.removed);
    assert!(matches!(library.view_get(&first.id), Err(Error::NotFound)));
    drop(library);
    let mut reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(reopened.view_save(&create_id, request).unwrap().view, first);
    let replay = reopened.view_remove(&removal_id, removal).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.view, updated);
    assert_eq!(reopened.views_page(views()).unwrap().items.len(), 1);
}

#[test]
fn view_validation_and_receipt_failure_preserve_existing_state() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    for request in [
        SaveView {
            filter_schema: 2,
            ..view("未来过滤器")
        },
        SaveView {
            filter: NotesFilter {
                collection_id: Some(id()),
                ..Default::default()
            },
            ..view("不存在主题")
        },
        SaveView {
            is_pinned: true,
            pin_position: None,
            ..view("缺失顺序")
        },
    ] {
        assert!(library.view_save(&id(), request).is_err());
    }
    let mut json = serde_json::to_value(view("原始SQL不得进入")).unwrap();
    json["filter"]["sql"] = serde_json::json!("SELECT * FROM notes");
    assert!(serde_json::from_value::<SaveView>(json).is_err());
    assert!(library.views_page(views()).unwrap().items.is_empty());
    let saved = library.view_save(&id(), view("保留")).unwrap().view;
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    let count = scalar(&raw, "SELECT count(*) FROM command_receipts");
    raw.execute_batch("CREATE TRIGGER fail_view_receipt BEFORE INSERT ON command_receipts WHEN NEW.command_kind='views.remove' BEGIN SELECT RAISE(ABORT,'test receipt failure'); END;").unwrap();
    let command = id();
    let request = RemoveView {
        view_id: saved.id.clone(),
        base_row_version: saved.row_version,
    };
    assert!(library.view_remove(&command, request.clone()).is_err());
    assert_eq!(library.view_get(&saved.id).unwrap(), saved);
    assert_eq!(scalar(&raw, "SELECT count(*) FROM command_receipts"), count);
    raw.execute_batch("DROP TRIGGER fail_view_receipt").unwrap();
    assert!(
        !library
            .view_remove(&command, request)
            .unwrap()
            .receipt
            .replayed
    );
}

#[test]
fn concurrent_collection_and_view_edits_have_one_winner_per_metadata_version() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let c = library
        .collection_create(&id(), collection("原主题"))
        .unwrap()
        .collection;
    let v = library.view_save(&id(), view("原视图")).unwrap().view;
    let barrier = Arc::new(Barrier::new(2));
    let handles = ["甲", "乙"].map(|name| {
        let path = root.path().to_path_buf();
        let barrier = barrier.clone();
        let c = c.clone();
        let v = v.clone();
        std::thread::spawn(move || {
            let mut library = Library::open_existing(path).unwrap();
            barrier.wait();
            let collection = library.collection_update(
                &id(),
                UpdateCollection {
                    collection_id: c.id,
                    base_row_version: 1,
                    name: name.into(),
                    overview_note_id: None,
                    is_pinned: false,
                    pin_position: None,
                },
            );
            let view = library.view_save(
                &id(),
                SaveView {
                    name: name.into(),
                    ..edit_view(&v)
                },
            );
            (collection, view)
        })
    });
    let results = handles.map(|h| h.join().unwrap());
    assert_eq!(results.iter().filter(|r| r.0.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r.0, Err(Error::MetadataConflict { .. })))
            .count(),
        1
    );
    assert_eq!(results.iter().filter(|r| r.1.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r.1, Err(Error::MetadataConflict { .. })))
            .count(),
        1
    );
}

#[test]
fn database_snapshot_restores_collection_pins_orders_tags_and_saved_view_behavior() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("可带走的组织", "正文").unwrap();
    let c = library
        .collection_create(
            &id(),
            CreateCollection {
                overview_note_id: Some(note.id.clone()),
                is_pinned: true,
                pin_position: Some(2),
                ..collection("主题")
            },
        )
        .unwrap()
        .collection;
    library
        .collection_add_members(&id(), members(&library, &c.id, vec![note.id.clone()]))
        .unwrap();
    let tag = library
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: note.id.clone(),
                    base_row_version: 1,
                }],
                tag: TagSelector::Name {
                    name: "备份".into(),
                },
            },
        )
        .unwrap()
        .tag
        .unwrap();
    let saved = library
        .view_save(
            &id(),
            SaveView {
                filter: NotesFilter {
                    collection_id: Some(c.id.clone()),
                    tag_ids: vec![tag.id],
                    ..Default::default()
                },
                is_pinned: true,
                pin_position: Some(4),
                ..view("固定视图")
            },
        )
        .unwrap()
        .view;
    let before_collection = library.collection_get(&c.id).unwrap();
    let backup = root.path().join("snapshot.sqlite3");
    library.backup_to(&backup).unwrap();
    let restored = tempdir().unwrap();
    std::fs::copy(backup, restored.path().join("library.sqlite3")).unwrap();
    let restored = Library::open_existing(restored.path()).unwrap();
    assert_eq!(restored.collection_get(&c.id).unwrap(), before_collection);
    assert_eq!(restored.view_get(&saved.id).unwrap(), saved);
    assert_eq!(
        restored.view_run(&saved.id, 100, None).unwrap().notes.items[0]
            .note
            .id,
        note.id
    );
    assert_eq!(
        restored
            .collection_members_page(member_page(&c.id))
            .unwrap()
            .items[0]
            .object
            .tags
            .len(),
        1
    );
}
/// Cursors are opaque but they are still library-scoped: two libraries can
/// share the same list generation and the same query fingerprint, so without a
/// physical binding the page from one library would be continued inside the
/// other and return a mixture of both.
#[test]
fn a_collection_cursor_from_another_library_is_rejected() {
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    let mut a = Library::open(first.path()).unwrap();
    let mut b = Library::open(second.path()).unwrap();
    for name in ["甲", "乙", "丙"] {
        a.collection_create(&id(), collection(name)).unwrap();
        b.collection_create(&id(), collection(name)).unwrap();
    }
    let page = a
        .collections_page(CollectionPageRequest {
            limit: 1,
            ..collections()
        })
        .unwrap();
    let cursor = page.next_cursor.expect("three items with limit 1 continue");

    assert_eq!(
        b.collections_page(CollectionPageRequest {
            limit: 1,
            ..collections()
        })
        .unwrap()
        .items
        .len(),
        1
    );
    assert!(matches!(
        b.collections_page(CollectionPageRequest {
            limit: 1,
            cursor: Some(cursor),
            ..collections()
        }),
        Err(Error::QueryExpired)
    ));
}
