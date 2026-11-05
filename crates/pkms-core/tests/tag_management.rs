use pkms_core::{
    AssignTag, CreateTag, DeleteTag, Error, Library, MetadataTarget, NoteSort, NotesFilter,
    RemoveView, SaveView, TagSelector, TagsBrowseRequest, UpdateTag, ViewConfig, ViewLayout,
};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn create(library: &mut Library, name: &str) -> pkms_core::TagDetails {
    library
        .tag_create(
            &id(),
            CreateTag {
                name: name.into(),
                description: "说明".into(),
            },
        )
        .unwrap()
        .tag
}
fn assign(library: &mut Library, object: &str, tag: &str) {
    let version = library.object_metadata(object).unwrap().row_version;
    library
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: object.into(),
                    base_row_version: version,
                }],
                tag: TagSelector::Id { id: tag.into() },
            },
        )
        .unwrap();
}
fn delete_request(tag: &pkms_core::TagDetails) -> DeleteTag {
    DeleteTag {
        id: tag.tag.id.clone(),
        base_row_version: tag.row_version,
        impact_token: tag.impact_token.clone(),
    }
}
fn browse() -> TagsBrowseRequest {
    TagsBrowseRequest {
        page: 1,
        limit: 10,
        query: None,
        collection_id: None,
    }
}

fn suggestions(library: &mut Library) -> pkms_core::ChangeSet {
    library
        .create_change_set(
            &id(),
            pkms_core::CreateChangeSet {
                kind: pkms_core::ChangeSetKind::TagSuggestions,
                collection_id: None,
                limit: 20,
                stale_after_days: None,
            },
        )
        .unwrap()
        .change
}

#[test]
fn dictionary_rename_skips_old_suggestions_but_description_edits_keep_them_actionable() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library
        .create_note("Rust Sqlite", "学习 Rust 与 Sqlite")
        .unwrap();
    let tag = create(&mut library, "Rust");
    create(&mut library, "Sqlite");
    let change = suggestions(&mut library);
    assert_eq!(change.items.len(), 2);
    let updated = library
        .tag_update(
            &id(),
            UpdateTag {
                id: tag.tag.id.clone(),
                base_row_version: tag.row_version,
                name: "Rust".into(),
                description: "新说明".into(),
            },
        )
        .unwrap()
        .tag;
    assert!(
        library
            .change_set(&change.id)
            .unwrap()
            .items
            .iter()
            .all(|i| i.status == pkms_core::ChangeItemStatus::Pending)
    );
    library
        .tag_update(
            &id(),
            UpdateTag {
                id: tag.tag.id.clone(),
                base_row_version: updated.row_version,
                name: "Rust 语言".into(),
                description: updated.description,
            },
        )
        .unwrap();
    let refreshed = library.change_set(&change.id).unwrap();
    let old = refreshed.items.iter().find(|i| matches!(&i.payload, pkms_core::ChangePayload::AssignTag { tag, .. } if tag == "Rust")).unwrap();
    assert_eq!(old.status, pkms_core::ChangeItemStatus::Skipped);
    assert_eq!(old.blocked_reason.as_deref(), Some("tag_changed"));
    assert_eq!(refreshed.status, pkms_core::ChangeSetStatus::Pending);
    assert_eq!(refreshed.applied_count, 0);
    assert!(
        library
            .apply_change_item(
                &id(),
                pkms_core::ApplyChangeItem {
                    change_id: change.id.clone(),
                    item_id: old.item_id.clone(),
                    confirm: true
                }
            )
            .is_err()
    );
    let other = refreshed
        .items
        .iter()
        .find(|i| i.status == pkms_core::ChangeItemStatus::Pending)
        .unwrap();
    library
        .apply_change_item(
            &id(),
            pkms_core::ApplyChangeItem {
                change_id: change.id,
                item_id: other.item_id.clone(),
                confirm: true,
            },
        )
        .unwrap();
    let metadata = library.object_metadata(&note.id).unwrap();
    assert_eq!(metadata.tags.len(), 1);
    assert_eq!(metadata.tags[0].display_name, "Sqlite");
    assert_eq!(library.tags_browse(browse()).unwrap().total, 2);
}

#[test]
fn deleted_and_recreated_tag_cannot_be_assigned_by_an_old_proposal() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library
        .create_note("项目归档", "需要整理项目归档记录")
        .unwrap();
    let tag = create(&mut library, "项目归档");
    let change = suggestions(&mut library);
    assert_eq!(change.items.len(), 1);
    library.tag_delete(&id(), delete_request(&tag)).unwrap();
    let recreated = create(&mut library, "项目归档");
    assert_ne!(recreated.tag.id, tag.tag.id);
    let old = library.change_set(&change.id).unwrap();
    assert_eq!(old.items[0].status, pkms_core::ChangeItemStatus::Skipped);
    assert_eq!(old.applied_count, 0);
    assert!(
        library
            .apply_change_item(
                &id(),
                pkms_core::ApplyChangeItem {
                    change_id: change.id,
                    item_id: change.items[0].item_id.clone(),
                    confirm: true
                }
            )
            .is_err()
    );
    assert!(library.object_metadata(&note.id).unwrap().tags.is_empty());
    assert_eq!(
        library.note(&note.id).unwrap().revision_id,
        note.revision_id
    );
}

#[test]
fn creates_unused_tags_replays_receipts_and_rejects_canonical_duplicates() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let command = id();
    let request = CreateTag {
        name: "  Rust  ".into(),
        description: "  技术学习  ".into(),
    };
    let created = library.tag_create(&command, request.clone()).unwrap();
    assert_eq!(created.tag.tag.display_name, "Rust");
    assert_eq!(created.tag.description, "技术学习");
    assert!(created.tag.created_at.is_some());
    assert_eq!(created.tag.active_object_count, 0);
    let replay = library.tag_create(&command, request).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.tag, created.tag);
    assert!(matches!(
        library.tag_create(
            &id(),
            CreateTag {
                name: "rust".into(),
                description: String::new()
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(library.tags_browse(browse()).unwrap().total, 1);
}

#[test]
fn rename_updates_all_object_names_and_versions_without_rewriting_note_content() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("笔记", "正文与历史不变").unwrap();
    let collection = library.create_collection("分类").unwrap();
    let tag = create(&mut library, "旧名");
    assign(&mut library, &note.id, &tag.tag.id);
    assign(&mut library, &collection.id, &tag.tag.id);
    let before = library.object_metadata(&note.id).unwrap();
    let changed = library
        .tag_update(
            &id(),
            UpdateTag {
                id: tag.tag.id.clone(),
                base_row_version: tag.row_version,
                name: "新名".into(),
                description: tag.description.clone(),
            },
        )
        .unwrap();
    assert_eq!(changed.affected_objects, 2);
    assert_eq!(changed.tag.tag.id, tag.tag.id);
    let after = library.object_metadata(&note.id).unwrap();
    assert_eq!(after.row_version, before.row_version + 1);
    assert_eq!(after.tags[0].display_name, "新名");
    assert_eq!(library.note(&note.id).unwrap().document, note.document);
    assert_eq!(
        library.note(&note.id).unwrap().revision_id,
        note.revision_id
    );
    assert!(matches!(
        library.tag_update(
            &id(),
            UpdateTag {
                id: tag.tag.id,
                base_row_version: tag.row_version,
                name: "迟到修改".into(),
                description: String::new()
            }
        ),
        Err(Error::MetadataConflict { .. })
    ));
}

#[test]
fn description_only_change_does_not_conflict_with_unrelated_object_metadata_edits() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("笔记", "正文").unwrap();
    let tag = create(&mut library, "标签");
    assign(&mut library, &note.id, &tag.tag.id);
    let version = library.object_metadata(&note.id).unwrap().row_version;
    let result = library
        .tag_update(
            &id(),
            UpdateTag {
                id: tag.tag.id.clone(),
                base_row_version: tag.row_version,
                name: tag.tag.display_name,
                description: "新的说明".into(),
            },
        )
        .unwrap();
    assert_eq!(result.affected_objects, 0);
    assert_eq!(
        library.object_metadata(&note.id).unwrap().row_version,
        version
    );
    assert_eq!(result.tag.row_version, tag.row_version + 1);
}

#[test]
fn deletion_requires_current_impact_preserves_notes_and_replays_after_tag_is_gone() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("第一篇", "保留").unwrap();
    let second = library.create_note("第二篇", "保留").unwrap();
    let tag = create(&mut library, "共享");
    assign(&mut library, &first.id, &tag.tag.id);
    let old = library.tag_details(&tag.tag.id).unwrap();
    assign(&mut library, &second.id, &tag.tag.id);
    assert!(matches!(
        library.tag_delete(&id(), delete_request(&old)),
        Err(Error::QueryExpired)
    ));
    let current = library.tag_details(&tag.tag.id).unwrap();
    let command = id();
    let request = delete_request(&current);
    let removed = library.tag_delete(&command, request.clone()).unwrap();
    assert!(removed.deleted);
    assert_eq!(removed.affected_objects, 2);
    assert!(library.object_metadata(&first.id).unwrap().tags.is_empty());
    assert_eq!(library.note(&first.id).unwrap().document, first.document);
    assert!(matches!(
        library.tag_details(&tag.tag.id),
        Err(Error::NotFound)
    ));
    assert!(
        library
            .tag_delete(&command, request)
            .unwrap()
            .receipt
            .replayed
    );
}

#[test]
fn saved_view_filters_must_be_explicitly_edited_before_deleting_their_tag() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let tag = create(&mut library, "视图标签");
    let view = library
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "带标签的视图".into(),
                filter_schema: 1,
                filter: NotesFilter {
                    tag_ids: vec![tag.tag.id.clone()],
                    ..Default::default()
                },
                sort: NoteSort::UpdatedDesc,
                layout: ViewLayout::List,
                config_schema: 1,
                config: ViewConfig::default(),
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .view;
    let current = library.tag_details(&tag.tag.id).unwrap();
    assert_eq!(current.saved_views[0].id, view.id);
    assert!(matches!(
        library.tag_delete(&id(), delete_request(&current)),
        Err(Error::InvalidInput(_))
    ));
    library
        .view_remove(
            &id(),
            RemoveView {
                view_id: view.id,
                base_row_version: view.row_version,
            },
        )
        .unwrap();
    let current = library.tag_details(&tag.tag.id).unwrap();
    library.tag_delete(&id(), delete_request(&current)).unwrap();
}

#[test]
fn numbered_browsing_counts_actual_collection_members_and_clamps_a_removed_last_page() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let inside = library.create_note("分类内", "正文").unwrap();
    let outside = library.create_note("分类外", "正文").unwrap();
    let collection = library.create_collection("目标分类").unwrap();
    library
        .add_to_collection(&collection.id, &inside.id)
        .unwrap();
    for index in 0..11 {
        let tag = create(&mut library, &format!("标签{index:02}"));
        assign(&mut library, &outside.id, &tag.tag.id);
        if index == 0 {
            assign(&mut library, &inside.id, &tag.tag.id);
        }
    }
    let all = library
        .tags_browse(TagsBrowseRequest {
            page: 2,
            ..browse()
        })
        .unwrap();
    assert_eq!(all.total, 11);
    assert_eq!(all.items.len(), 1);
    let scoped = library
        .tags_browse(TagsBrowseRequest {
            collection_id: Some(collection.id),
            ..browse()
        })
        .unwrap();
    assert_eq!(scoped.total, 1);
    assert_eq!(scoped.items[0].object_count, 1);
    let last = library.tag_details(&all.items[0].tag.id).unwrap();
    library.tag_delete(&id(), delete_request(&last)).unwrap();
    let page = library
        .tags_browse(TagsBrowseRequest {
            page: 2,
            ..browse()
        })
        .unwrap();
    assert_eq!(page.total, 10);
    assert_eq!(page.page, 1);
}

#[test]
fn a_late_sql_failure_rolls_back_assignment_removal_and_metadata_versions() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("笔记", "保留").unwrap();
    let tag = create(&mut library, "保留标签");
    assign(&mut library, &note.id, &tag.tag.id);
    let before = library.object_metadata(&note.id).unwrap();
    let current = library.tag_details(&tag.tag.id).unwrap();
    let db = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_tag_delete BEFORE DELETE ON tags BEGIN SELECT RAISE(ABORT,'controlled failure'); END;").unwrap();
    assert!(library.tag_delete(&id(), delete_request(&current)).is_err());
    assert_eq!(library.object_metadata(&note.id).unwrap(), before);
    assert_eq!(library.tag_details(&tag.tag.id).unwrap(), current);
}

#[test]
fn a_complete_backup_restores_dictionary_metadata_usage_and_unused_tags() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path().join("source")).unwrap();
    let note = library.create_note("原始笔记", "原始正文").unwrap();
    let used = create(&mut library, "已使用标签");
    let unused = create(&mut library, "未使用标签");
    assign(&mut library, &note.id, &used.tag.id);
    let expected = library.tag_details(&used.tag.id).unwrap();
    let package = root.path().join("tags.pkmsbackup");
    let mut check = |_: &str| Ok(());
    library.create_backup(&package, &mut check).unwrap();
    pkms_core::verify_backup(&package, &mut check).unwrap();
    let destination = root.path().join("restored");
    pkms_core::prepare_restore(&package, &destination, &mut check)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    assert_eq!(restored.tag_details(&used.tag.id).unwrap(), expected);
    assert_eq!(restored.tag_details(&unused.tag.id).unwrap(), unused);
    assert_eq!(
        restored.object_metadata(&note.id).unwrap().tags[0].id,
        used.tag.id
    );
    assert_eq!(restored.note(&note.id).unwrap().document, note.document);
}

#[test]
fn related_objects_include_all_kinds_and_excerpt_source_scope_and_trash_stay_consistent() {
    use pkms_core::{
        CreateAnnotation, ImportSource, ObjectKind, SetObjectFlag, SourceFormat, SourceLocator,
        TaggedObjectsRequest, parse_text_source,
    };
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let collection = library.create_collection("资料分类").unwrap();
    let note = library.create_note("独立笔记", "内容").unwrap();
    let text = "需要定位的原文摘录。";
    let job = library
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "标签资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: Some(collection.id.clone()),
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = library.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = library
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    library
        .publish_source_parse(
            &job.job_id,
            &lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
        )
        .unwrap();
    let block = library
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let annotation = library
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: block.text.chars().count() as u32,
                    quote: block.text,
                    prefix: String::new(),
                    suffix: String::new(),
                },
                comment: "摘录评论".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let tag = create(&mut library, "混合资料");
    for object in [&note.id, &collection.id, &version.source_id, &annotation.id] {
        assign(&mut library, object, &tag.tag.id);
    }
    let request = |trashed, collection_id| TaggedObjectsRequest {
        tag_id: tag.tag.id.clone(),
        page: 1,
        limit: 10,
        trashed,
        collection_id,
    };
    let all = library.tagged_objects(request(false, None)).unwrap();
    assert_eq!(all.total, 4);
    assert!(
        all.items
            .iter()
            .any(|item| item.kind == ObjectKind::Collection)
    );
    let scoped = library
        .tagged_objects(request(false, Some(collection.id.clone())))
        .unwrap();
    assert_eq!(scoped.total, 2);
    let excerpt = scoped
        .items
        .iter()
        .find(|item| item.id == annotation.id)
        .unwrap();
    assert_eq!(excerpt.title, text);
    assert_eq!(
        excerpt.source_id.as_deref(),
        Some(version.source_id.as_str())
    );
    assert_eq!(
        excerpt.source_version_id.as_deref(),
        Some(version.id.as_str())
    );
    assert_eq!(
        library
            .tags_browse(TagsBrowseRequest {
                collection_id: Some(collection.id.clone()),
                ..browse()
            })
            .unwrap()
            .items[0]
            .object_count,
        2
    );
    library
        .set_object_trashed(
            &id(),
            SetObjectFlag {
                object_id: version.source_id.clone(),
                base_row_version: library
                    .object_metadata(&version.source_id)
                    .unwrap()
                    .row_version,
                value: true,
            },
        )
        .unwrap();
    let details = library.tag_details(&tag.tag.id).unwrap();
    assert_eq!(details.active_object_count, 2);
    assert_eq!(details.trashed_object_count, 2);
    assert_eq!(
        library.tagged_objects(request(false, None)).unwrap().total,
        2
    );
    let trashed = library.tagged_objects(request(true, None)).unwrap();
    assert_eq!(trashed.total, 2);
    assert!(
        trashed
            .items
            .iter()
            .find(|item| item.id == annotation.id)
            .unwrap()
            .source_trashed
    );
    assert_eq!(
        library
            .tags_browse(TagsBrowseRequest {
                collection_id: Some(collection.id),
                ..browse()
            })
            .unwrap()
            .total,
        0
    );
}
