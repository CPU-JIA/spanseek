use pkms_core::*;
use rusqlite::params;
use std::io::Cursor;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request(filter: SourcesFilter) -> SourcesPageRequest {
    SourcesPageRequest {
        limit: 1,
        cursor: None,
        trashed: false,
        query: None,
        filter: Some(filter),
    }
}
fn source(db: &mut Library, number: u32) -> SourceVersion {
    let accepted = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: format!("source{number}.txt"),
                format: if number == 3 {
                    SourceFormat::Markdown
                } else {
                    SourceFormat::Text
                },
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    db.capture_source_original(
        &accepted.job_id,
        &lease.lease_token,
        &mut Cursor::new(format!("body{number}")),
        &mut |_| Ok(()),
    )
    .unwrap()
}

#[test]
fn source_filters_apply_before_pagination_with_tag_intersection_and_normalized_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let versions = (0..4).map(|n| source(&mut db, n)).collect::<Vec<_>>();
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let tags = [id(), id()];
    for (n, tag) in tags.iter().enumerate() {
        raw.execute(
            "INSERT INTO tags(id,display_name,normalized_name) VALUES(?1,?2,?2)",
            params![tag, format!("tag{n}")],
        )
        .unwrap();
    }
    let collection = id();
    raw.execute(
        "INSERT INTO objects(id,kind) VALUES(?1,'collection')",
        [&collection],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO collections(id,name) VALUES(?1,'group')",
        [&collection],
    )
    .unwrap();
    for (n, v) in versions.iter().enumerate() {
        raw.execute(
            "UPDATE sources SET updated_at=?2 WHERE id=?1",
            params![v.source_id, format!("2026-10-03T00:00:0{n}Z")],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2)",
            params![v.source_id, tags[0]],
        )
        .unwrap();
        if n < 2 {
            raw.execute(
                "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2)",
                params![v.source_id, tags[1]],
            )
            .unwrap();
        }
        raw.execute(
            "INSERT INTO memberships(collection_id,member_id,position) VALUES(?1,?2,?3)",
            params![collection, v.source_id, n as u32],
        )
        .unwrap();
    }
    let filter = SourcesFilter {
        format: Some(SourceFormat::Text),
        reading_status: Some(ReadingStatus::Unread),
        tag_ids: tags.to_vec(),
        inbox_state: Some(InboxState::Pending),
        collection_id: Some(collection.clone()),
    };
    let first = db.sources_page(request(filter.clone())).unwrap();
    let direct = db
        .sources_browse(SourcesBrowseRequest {
            page: 2,
            limit: 1,
            trashed: false,
            query: None,
            filter: Some(filter.clone()),
        })
        .unwrap();
    assert_eq!(direct.total, 2);
    assert_eq!(direct.page, 2);
    assert_eq!(direct.items[0].id, versions[0].source_id);
    assert_eq!(first.items[0].id, versions[1].source_id);
    let cursor = first.next_cursor.unwrap();
    let mut normalized = filter.clone();
    normalized.tag_ids = vec![tags[1].clone(), tags[0].clone(), tags[0].clone()];
    let second = db
        .sources_page(SourcesPageRequest {
            cursor: Some(cursor.clone()),
            ..request(normalized)
        })
        .unwrap();
    assert_eq!(second.items[0].id, versions[0].source_id);
    assert!(second.next_cursor.is_none());
    for changed in [
        SourcesFilter {
            format: Some(SourceFormat::Markdown),
            ..filter.clone()
        },
        SourcesFilter {
            reading_status: Some(ReadingStatus::Reading),
            ..filter.clone()
        },
        SourcesFilter {
            tag_ids: vec![tags[0].clone()],
            ..filter.clone()
        },
        SourcesFilter {
            inbox_state: Some(InboxState::Processed),
            ..filter.clone()
        },
        SourcesFilter {
            collection_id: Some(id()),
            ..filter.clone()
        },
    ] {
        assert!(matches!(
            db.sources_page(SourcesPageRequest {
                cursor: Some(cursor.clone()),
                ..request(changed)
            }),
            Err(Error::InvalidInput(_))
        ));
    }
    let markdown = db
        .sources_page(request(SourcesFilter {
            format: Some(SourceFormat::Markdown),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(markdown.items[0].id, versions[3].source_id);
    assert!(
        db.sources_page(request(SourcesFilter {
            collection_id: Some(id()),
            ..Default::default()
        }))
        .unwrap()
        .items
        .is_empty()
    );
    raw.execute(
        "UPDATE objects SET inbox_state='processed' WHERE id=?1",
        [&versions[3].source_id],
    )
    .unwrap();
    assert_eq!(
        db.sources_page(request(SourcesFilter {
            inbox_state: Some(InboxState::Processed),
            ..Default::default()
        }))
        .unwrap()
        .items[0]
            .id,
        versions[3].source_id
    );
    raw.execute("INSERT INTO reading_states(source_id,source_version_id,status,row_version) VALUES(?1,?2,'finished',1)",params![versions[1].source_id,versions[1].id]).unwrap();
    let narrowed = db
        .sources_browse(SourcesBrowseRequest {
            page: 2,
            limit: 1,
            trashed: false,
            query: None,
            filter: Some(filter.clone()),
        })
        .unwrap();
    assert_eq!(narrowed.total, 1);
    assert_eq!(narrowed.page, 1);
    assert_eq!(narrowed.items[0].id, versions[0].source_id);
    assert!(matches!(
        db.sources_page(SourcesPageRequest {
            cursor: Some(cursor),
            ..request(filter.clone())
        }),
        Err(Error::QueryExpired)
    ));
    assert_eq!(
        db.sources_page(request(filter)).unwrap().items[0].id,
        versions[0].source_id
    );
    assert_eq!(
        db.sources_page(request(SourcesFilter {
            reading_status: Some(ReadingStatus::Finished),
            ..Default::default()
        }))
        .unwrap()
        .items[0]
            .id,
        versions[1].source_id
    );
}

#[test]
fn direct_source_pages_match_cursor_order_and_shrink_after_trash_without_walking_pages() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    for number in 0..25 {
        source(&mut db, number);
    }
    let request = SourcesBrowseRequest {
        page: 3,
        limit: 10,
        trashed: false,
        query: None,
        filter: None,
    };
    let direct = db.sources_browse(request.clone()).unwrap();
    assert_eq!((direct.page, direct.total, direct.items.len()), (3, 25, 5));
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let result = db
            .sources_page(SourcesPageRequest {
                limit: 10,
                cursor,
                trashed: false,
                query: None,
                filter: None,
            })
            .unwrap();
        seen.extend(result.items.into_iter().map(|s| s.id));
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        direct.items.iter().map(|s| &s.id).collect::<Vec<_>>(),
        seen[20..].iter().collect::<Vec<_>>()
    );
    for item in &direct.items {
        let metadata = db.object_metadata(&item.id).unwrap();
        db.set_object_trashed(
            &id(),
            SetObjectFlag {
                object_id: item.id.clone(),
                base_row_version: metadata.row_version,
                value: true,
            },
        )
        .unwrap();
    }
    let shorter = db.sources_browse(request.clone()).unwrap();
    assert_eq!(
        (shorter.page, shorter.total, shorter.items.len()),
        (2, 20, 10)
    );
    let trash = db
        .sources_browse(SourcesBrowseRequest {
            trashed: true,
            ..request.clone()
        })
        .unwrap();
    assert_eq!((trash.page, trash.total, trash.items.len()), (1, 5, 5));
    let empty = db
        .sources_browse(SourcesBrowseRequest {
            query: Some("missing title".into()),
            ..request.clone()
        })
        .unwrap();
    assert_eq!((empty.page, empty.total, empty.items.len()), (1, 0, 0));
    assert!(
        db.sources_browse(SourcesBrowseRequest {
            page: 0,
            ..request.clone()
        })
        .is_err()
    );
    assert!(
        db.sources_browse(SourcesBrowseRequest {
            limit: 0,
            ..request.clone()
        })
        .is_err()
    );
    assert!(
        db.sources_browse(SourcesBrowseRequest {
            query: Some("bad\0query".into()),
            ..request.clone()
        })
        .is_err()
    );
    let clamped = db
        .sources_browse(SourcesBrowseRequest {
            page: u32::MAX,
            ..request
        })
        .unwrap();
    assert_eq!(clamped.page, 2);
}

#[test]
fn reading_tags_and_memberships_invalidate_cursors_for_insert_update_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let versions = [source(&mut db, 0), source(&mut db, 1)];
    let raw = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    let tag = id();
    let collection = id();
    raw.execute(
        "INSERT INTO tags(id,display_name,normalized_name) VALUES(?1,'tag','tag')",
        [&tag],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO objects(id,kind) VALUES(?1,'collection')",
        [&collection],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO collections(id,name) VALUES(?1,'group')",
        [&collection],
    )
    .unwrap();
    for sql in [
        format!(
            "INSERT INTO reading_states(source_id,source_version_id,status,row_version) VALUES('{}','{}','reading',1)",
            versions[0].source_id, versions[0].id
        ),
        "UPDATE reading_states SET status='finished'".into(),
        "DELETE FROM reading_states".into(),
        format!(
            "INSERT INTO object_tags(object_id,tag_id) VALUES('{}','{tag}')",
            versions[0].source_id
        ),
        format!(
            "UPDATE object_tags SET object_id='{}'",
            versions[1].source_id
        ),
        "DELETE FROM object_tags".into(),
        format!(
            "INSERT INTO memberships(collection_id,member_id,position) VALUES('{collection}','{}',0)",
            versions[0].source_id
        ),
        "UPDATE memberships SET position=1".into(),
        "DELETE FROM memberships".into(),
    ] {
        let cursor = db
            .sources_page(request(Default::default()))
            .unwrap()
            .next_cursor
            .unwrap();
        raw.execute_batch(&sql).unwrap();
        assert!(
            matches!(
                db.sources_page(SourcesPageRequest {
                    cursor: Some(cursor),
                    ..request(Default::default())
                }),
                Err(Error::QueryExpired)
            ),
            "{sql}"
        );
    }
}
