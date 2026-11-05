use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn import(
    db: &mut Library,
    title: &str,
    text: &str,
    format: SourceFormat,
    prior: Option<&SourceSummary>,
) -> SourceVersion {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: title.into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: prior.map(|source| source.id.clone()),
                base_row_version: prior.map(|source| source.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &accepted.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, format, text.as_bytes()).unwrap(),
    )
    .unwrap();
    version
}

fn annotate(db: &mut Library, source: &SourceVersion, comment: &str) -> Annotation {
    let block = db
        .source_text_page(&source.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    db.create_annotation(
        &id(),
        CreateAnnotation {
            locator: SourceLocator::Text {
                source_version_id: source.id.clone(),
                block_id: block.id,
                start_codepoint: 0,
                end_codepoint: block.text.chars().count() as u32,
                quote: block.text,
                prefix: "".into(),
                suffix: "".into(),
            },
            comment: comment.into(),
        },
    )
    .unwrap()
    .annotation
    .unwrap()
}

fn request(query: Option<&str>, limit: u32, cursor: Option<String>) -> AnnotationsQueryRequest {
    AnnotationsQueryRequest {
        query: query.map(str::to_owned),
        limit,
        cursor,
    }
}

#[test]
fn empty_annotation_comments_remain_empty_in_picker_and_collection_summaries() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let quote = "记录保留期限为45天。";
    let source = import(&mut db, "合成摘录资料", quote, SourceFormat::Text, None);
    let annotation = annotate(&mut db, &source, "");

    for query in [None, Some("45天")] {
        let result = db.annotations_query(request(query, 50, None)).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].id, annotation.id);
        assert_eq!(result.items[0].quote_preview, quote);
        assert_eq!(result.items[0].comment_preview, "");
    }

    let collection = db.create_collection("仅摘录主题").unwrap();
    db.collection_add_members(
        &id(),
        ChangeCollectionMembers {
            collection_id: collection.id.clone(),
            base_row_version: db
                .collection_get(&collection.id)
                .unwrap()
                .metadata
                .row_version,
            member_ids: vec![annotation.id.clone()],
        },
    )
    .unwrap();
    let members = db
        .collection_members_page(CollectionMembersRequest {
            collection_id: collection.id,
            limit: 50,
            cursor: None,
            include_trashed: false,
            kind: Some(ObjectKind::Annotation),
        })
        .unwrap();
    assert_eq!(members.items.len(), 1);
    let summary = members.items[0].annotation.as_ref().unwrap();
    assert_eq!(summary.id, annotation.id);
    assert_eq!(summary.quote_preview, quote);
    assert_eq!(summary.comment_preview, "");
}

#[test]
fn global_annotation_query_filters_before_paging_and_preserves_unicode_previews() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let first = import(
        &mut db,
        "Alpha资料",
        "first quotation",
        SourceFormat::Text,
        None,
    );
    let second = import(
        &mut db,
        "Beta资料",
        "second quotation",
        SourceFormat::Text,
        None,
    );
    annotate(&mut db, &first, "first comment");
    annotate(&mut db, &second, "second comment");
    let quote = format!("a{}", "😀".repeat(400));
    let comment = format!("A\0{} MATCHNEEDLE", "中😀e\u{301}".repeat(100));
    let third = import(&mut db, "Gamma资料", &quote, SourceFormat::Text, None);
    let long = annotate(&mut db, &third, &comment);
    let mut ids = vec![];
    let mut cursor = None;
    loop {
        let page = db.annotations_query(request(None, 1, cursor)).unwrap();
        assert_eq!(page.items.len(), 1);
        ids.push(page.items[0].id.clone());
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|ids| ids[0] < ids[1]));
    let title = db
        .annotations_query(request(Some("  ALPHA资料  "), 1, None))
        .unwrap();
    assert_eq!(title.items[0].source_id, first.source_id);
    let quotation = db
        .annotations_query(request(Some("SECOND QUOTATION"), 1, None))
        .unwrap();
    assert_eq!(quotation.items[0].source_id, second.source_id);
    let found = db
        .annotations_query(request(Some("matchneedle"), 1, None))
        .unwrap();
    assert_eq!(found.items[0].id, long.id);
    assert_eq!(
        found.items[0].quote_preview,
        quote.chars().take(280).collect::<String>()
    );
    assert_eq!(
        found.items[0].comment_preview,
        comment.chars().take(280).collect::<String>()
    );
    assert_eq!(found.items[0].quote_preview.chars().count(), 280);
    assert_eq!(found.items[0].comment_preview.chars().count(), 280);
    assert!(found.next_cursor.is_none());
    assert!(db.annotations_query(request(None, 0, None)).is_err());
    assert!(
        db.annotations_query(request(Some(&"x".repeat(301)), 1, None))
            .is_err()
    );
}

#[test]
fn annotation_cursor_binds_query_workspace_local_copy_and_mutation_generation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("original")).unwrap();
    let source = import(&mut db, "source", "quote", SourceFormat::Text, None);
    let first = annotate(&mut db, &source, "first");
    annotate(&mut db, &source, "second");
    let cursor = db
        .annotations_query(request(None, 1, None))
        .unwrap()
        .next_cursor
        .unwrap();
    assert!(matches!(
        db.annotations_query(request(Some("changed"), 1, Some(cursor.clone()))),
        Err(Error::InvalidInput(_))
    ));
    let other = Library::open(root.path().join("other")).unwrap();
    assert!(matches!(
        other.annotations_query(request(None, 1, Some(cursor.clone()))),
        Err(Error::InvalidInput(_))
    ));
    let backup = root.path().join("backup.zip");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("copy");
    prepare_restore(&backup, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let copy = Library::open(&destination).unwrap();
    assert!(matches!(
        copy.annotations_query(request(None, 1, Some(cursor.clone()))),
        Err(Error::InvalidInput(_))
    ));
    db.update_annotation(
        &id(),
        UpdateAnnotation {
            id: first.id,
            base_row_version: first.row_version,
            comment: "updated".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        db.annotations_query(request(None, 1, Some(cursor))),
        Err(Error::QueryExpired)
    ));
    let cursor = db
        .annotations_query(request(None, 1, None))
        .unwrap()
        .next_cursor
        .unwrap();
    let source = db.source(&source.source_id).unwrap();
    db.update_source_metadata(
        &id(),
        UpdateSourceMetadata {
            source_id: source.id,
            base_row_version: source.row_version,
            display_title: "renamed".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        db.annotations_query(request(None, 1, Some(cursor))),
        Err(Error::QueryExpired)
    ));
    let cursor = db
        .annotations_query(request(None, 1, None))
        .unwrap()
        .next_cursor
        .unwrap();
    db.create_note("legacy API mutation", "body").unwrap();
    assert!(matches!(
        db.annotations_query(request(None, 1, Some(cursor))),
        Err(Error::QueryExpired)
    ));
}

#[test]
fn annotation_summaries_keep_fixed_version_and_hide_trashed_children_or_parents() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let original = import(
        &mut db,
        "Original title",
        "immutable original quote",
        SourceFormat::Markdown,
        None,
    );
    let annotation = annotate(&mut db, &original, "comment");
    let source = db.source(&original.source_id).unwrap();
    let latest = import(
        &mut db,
        "replacement.txt",
        "new text",
        SourceFormat::Text,
        Some(&source),
    );
    let summary = db
        .annotations_query(request(None, 100, None))
        .unwrap()
        .items
        .remove(0);
    assert_eq!(summary.id, annotation.id);
    assert_eq!(summary.source_id, original.source_id);
    assert_eq!(summary.source_version_id, original.id);
    assert_ne!(summary.source_version_id, latest.id);
    assert_eq!(summary.source_format, SourceFormat::Markdown);
    assert_eq!(summary.quote_preview, "immutable original quote");
    assert_eq!(summary.row_version, annotation.row_version);
    assert_eq!(summary.page_index, None);
    assert!(!summary.trashed && !summary.source_trashed && summary.source_search_enabled);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE objects SET search_enabled=0 WHERE id=?1",
        [&original.source_id],
    )
    .unwrap();
    assert!(
        !db.annotations_query(request(None, 100, None))
            .unwrap()
            .items[0]
            .source_search_enabled
    );
    db.set_annotation_trashed(
        &id(),
        SetObjectFlag {
            object_id: annotation.id.clone(),
            base_row_version: annotation.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(
        db.annotations_query(request(None, 100, None))
            .unwrap()
            .items
            .is_empty()
    );
    let current = db.annotation(&annotation.id).unwrap();
    db.set_annotation_trashed(
        &id(),
        SetObjectFlag {
            object_id: current.id,
            base_row_version: current.row_version,
            value: false,
        },
    )
    .unwrap();
    let source = db.source(&original.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id,
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(
        db.annotations_query(request(None, 100, None))
            .unwrap()
            .items
            .is_empty()
    );
}
