use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn import(db: &mut Library) -> SourceVersion {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "真实资料".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let text = "原文摘录😀";
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}
fn page(collection: &str, kind: Option<ObjectKind>, limit: u32) -> CollectionMembersRequest {
    CollectionMembersRequest {
        collection_id: collection.into(),
        limit,
        cursor: None,
        include_trashed: false,
        kind,
    }
}

#[test]
fn collection_lists_real_three_kind_summaries_and_filters_before_limit() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("研究主题").unwrap();
    let note = db.create_note("研究笔记", "内容").unwrap();
    let source = import(&mut db);
    let block = db
        .source_text_page(&source.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: source.id.clone(),
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: block.text.chars().count() as u32,
                    quote: block.text,
                    prefix: String::new(),
                    suffix: String::new(),
                },
                comment: "主题评论".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    assert_eq!(
        db.object_metadata(&annotation.id).unwrap().inbox_state,
        Some(InboxState::Pending)
    );
    // Keep the explicit pending state for annotations created by schema 9-13 builds.
    rusqlite::Connection::open(root.path().join("library.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE objects SET inbox_state='pending' WHERE id=?1",
            [&annotation.id],
        )
        .unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![
                note.id.clone(),
                source.source_id.clone(),
                annotation.id.clone(),
            ],
        },
    )
    .unwrap();
    let all = db
        .collection_members_page(page(&collection.id, None, 100))
        .unwrap();
    assert_eq!(all.items.len(), 3);
    assert!(all.items[0].note.is_some());
    assert!(all.items[0].source.is_none() && all.items[0].annotation.is_none());
    let sources = db
        .collection_members_page(page(&collection.id, Some(ObjectKind::Source), 1))
        .unwrap();
    assert_eq!(sources.items.len(), 1);
    assert_eq!(sources.items[0].position, 1);
    assert_eq!(
        sources.items[0].source.as_ref().unwrap().display_title,
        "真实资料"
    );
    assert_eq!(
        sources.items[0].source.as_ref().unwrap().current_version_id,
        source.id
    );
    assert!(sources.next_cursor.is_none());
    let annotations = db
        .collection_members_page(page(&collection.id, Some(ObjectKind::Annotation), 1))
        .unwrap();
    let summary = annotations.items[0].annotation.as_ref().unwrap();
    assert_eq!(summary.id, annotation.id);
    assert_eq!(summary.source_id, source.source_id);
    assert_eq!(summary.source_version_id, source.id);
    assert_eq!(summary.quote_preview, "原文摘录😀");
    assert_eq!(summary.comment_preview, "主题评论");
    assert!(annotations.items[0].note.is_none() && annotations.items[0].source.is_none());
    assert!(
        db.collection_members_page(page(&collection.id, Some(ObjectKind::Collection), 1))
            .is_err()
    );

    let cursor = db
        .collection_members_page(page(&collection.id, None, 1))
        .unwrap()
        .next_cursor;
    let mut changed_query = page(&collection.id, Some(ObjectKind::Source), 1);
    changed_query.cursor = cursor;
    assert!(db.collection_members_page(changed_query).is_err());
    let metadata = db.object_metadata(&source.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.source_id,
            base_row_version: metadata.row_version,
            value: true,
        },
    )
    .unwrap();
    let annotated = db
        .collection_members_page(page(&collection.id, Some(ObjectKind::Annotation), 1))
        .unwrap();
    assert!(
        annotated.items[0]
            .annotation
            .as_ref()
            .unwrap()
            .source_trashed
    );
    assert!(
        db.collection_members_page(page(&collection.id, Some(ObjectKind::Source), 1))
            .unwrap()
            .items
            .is_empty()
    );
    let mut with_trash = page(&collection.id, Some(ObjectKind::Source), 1);
    with_trash.include_trashed = true;
    assert!(
        db.collection_members_page(with_trash).unwrap().items[0]
            .object
            .trashed
    );
}

#[test]
fn member_cursor_rejects_restored_copy_and_missing_legacy_binding() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("original")).unwrap();
    let collection = db.create_collection("集合").unwrap();
    for title in ["甲", "乙"] {
        let note = db.create_note(title, "正文").unwrap();
        db.add_to_collection(&collection.id, &note.id).unwrap();
    }
    let mut next = page(&collection.id, None, 1);
    next.cursor = db
        .collection_members_page(next.clone())
        .unwrap()
        .next_cursor;
    let archive = root.path().join("backup.pkmsbackup");
    db.create_backup(&archive, &mut |_| Ok(())).unwrap();
    let restored = root.path().join("restored");
    prepare_restore(&archive, &restored, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let copy = Library::open(restored).unwrap();
    assert!(matches!(
        copy.collection_members_page(next.clone()),
        Err(Error::QueryExpired)
    ));
    let mut legacy: serde_json::Value =
        serde_json::from_str(next.cursor.as_ref().unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("scope");
    legacy.as_object_mut().unwrap().remove("source_generation");
    next.cursor = Some(legacy.to_string());
    assert!(matches!(
        db.collection_members_page(next),
        Err(Error::QueryExpired)
    ));
}
