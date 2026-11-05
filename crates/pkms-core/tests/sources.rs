use base64::{Engine as _, engine::general_purpose::STANDARD};
use pkms_core::*;
use std::io::{Cursor, Read};
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn input() -> ImportSource {
    ImportSource {
        input_token: id(),
        original_name: "中文资料.txt".into(),
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}
fn capture_format(
    db: &mut Library,
    bytes: &[u8],
    original_name: &str,
    format: SourceFormat,
) -> (SourceJobLease, SourceVersion) {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                original_name: original_name.into(),
                format,
                ..input()
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    (lease, version)
}
fn capture(db: &mut Library, bytes: &[u8]) -> (SourceJobLease, SourceVersion) {
    capture_format(db, bytes, "中文资料.txt", SourceFormat::Text)
}

#[test]
fn pptx_parse_keeps_slide_count_without_treating_slides_as_pdf_pages() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let (lease, version) =
        capture_format(&mut db, b"controlled pptx", "deck.pptx", SourceFormat::Pptx);
    let parsed = ParsedSource {
        title: Some("季度汇报".into()),
        author: None,
        source_version_id: version.id.clone(),
        parser_version: "pptx-v2".into(),
        blocks: vec![
            SourceBlock {
                id: id(),
                text: "Slide: 1".into(),
                page_index: None,
                pdf_geometry: None,
            },
            SourceBlock {
                id: id(),
                text: "季度目标".into(),
                page_index: None,
                pdf_geometry: None,
            },
        ],
        page_count: Some(3),
        needs_ocr: false,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    };
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    let page = db.source_text_page(&version.id, None, 0, 10).unwrap();
    assert_eq!(page.page_count, Some(3));
    assert_eq!(page.blocks[1].text, "季度目标");
}

#[test]
fn first_epub_and_mobi_parse_backfill_metadata_title_and_author() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    for (format, original_name, bytes, title, author) in [
        (
            SourceFormat::Epub,
            "book.epub",
            b"epub body".as_slice(),
            "EPUB 元数据标题",
            "EPUB 元数据作者",
        ),
        (
            SourceFormat::Mobi,
            "book.mobi",
            b"mobi body".as_slice(),
            "MOBI 元数据标题",
            "MOBI 元数据作者",
        ),
    ] {
        let (job, version) = capture_format(&mut db, bytes, original_name, format);
        let parsed = ParsedSource {
            title: Some(title.into()),
            author: Some(author.into()),
            source_version_id: version.id.clone(),
            parser_version: "test-v1".into(),
            blocks: vec![SourceBlock {
                id: id(),
                text: String::from_utf8(bytes.to_vec()).unwrap(),
                page_index: None,
                pdf_geometry: None,
            }],
            page_count: None,
            needs_ocr: false,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        };
        db.publish_source_parse(&job.job.id, &job.lease_token, parsed)
            .unwrap();
        let source = db.source(&version.source_id).unwrap();
        assert_eq!(source.display_title, title);
        assert_eq!(source.author.as_deref(), Some(author));
    }
}

#[test]
fn book_toc_and_cover_survive_publish_pagination_and_backup() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (job, version) = capture_format(
        &mut db,
        b"book placeholder",
        "book.epub",
        SourceFormat::Epub,
    );
    let blocks = (0..3)
        .map(|index| SourceBlock {
            id: id(),
            text: format!("第{}段", index + 1),
            page_index: None,
            pdf_geometry: None,
        })
        .collect::<Vec<_>>();
    let toc = vec![
        SourceTocItem {
            title: "第一章".into(),
            level: 1,
            block_id: Some(blocks[0].id.clone()),
            block_position: Some(0),
        },
        SourceTocItem {
            title: "第二节".into(),
            level: 2,
            block_id: Some(blocks[1].id.clone()),
            block_position: Some(1),
        },
    ];
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let mut body_png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 2)
        .write_to(&mut body_png, image::ImageFormat::Png)
        .unwrap();
    let body_png = body_png.into_inner();
    db.publish_source_parse(
        &job.job.id,
        &job.lease_token,
        ParsedSource {
            title: Some("目录测试".into()),
            author: Some("跨索自测".into()),
            source_version_id: version.id.clone(),
            parser_version: "epub-v2".into(),
            blocks,
            page_count: None,
            needs_ocr: false,
            toc: toc.clone(),
            cover: Some(ParsedCover {
                media_type: "image/png".into(),
                data_base64: STANDARD.encode(&png),
            }),
            images: vec![ParsedSourceImage {
                block_position: 1,
                media_type: "image/png".into(),
                data_base64: STANDARD.encode(&body_png),
                alt: "正文插图".into(),
            }],
        },
    )
    .unwrap();

    let first = db.source_text_page(&version.id, None, 0, 1).unwrap();
    assert_eq!(first.toc, toc);
    assert!(first.has_cover);
    let second = db.source_text_page(&version.id, None, 1, 1).unwrap();
    assert_eq!(second.images.len(), 1);
    assert_eq!(second.images[0].block_id, second.blocks[0].id);
    assert_eq!(second.images[0].alt, "正文插图");
    let body_image = db
        .source_asset(&version.id, &first.parse_id, &second.images[0].blob_hash)
        .unwrap();
    assert_eq!(body_image.media_type, "image/png");
    assert_eq!(STANDARD.decode(body_image.data_base64).unwrap(), body_png);
    assert_eq!(second.toc, toc);
    let cover = db.source_cover(&version.id, None).unwrap();
    assert_eq!(cover.source_version_id, version.id);
    assert_eq!(cover.media_type, "image/png");
    assert_eq!(STANDARD.decode(cover.data_base64).unwrap(), png);

    let package = root.path().join("book-navigation.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let target = root.path().join("restored-book-navigation");
    prepare_restore(package, &target, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(target).unwrap();
    assert_eq!(
        restored
            .source_text_page(&version.id, None, 0, 1)
            .unwrap()
            .toc,
        toc
    );
    assert_eq!(
        STANDARD
            .decode(
                restored
                    .source_cover(&version.id, None)
                    .unwrap()
                    .data_base64
            )
            .unwrap(),
        png
    );
    let restored_page = restored.source_text_page(&version.id, None, 1, 1).unwrap();
    assert_eq!(restored_page.images.len(), 1);
    assert_eq!(
        STANDARD
            .decode(
                restored
                    .source_asset(
                        &version.id,
                        &first.parse_id,
                        &restored_page.images[0].blob_hash
                    )
                    .unwrap()
                    .data_base64
            )
            .unwrap(),
        body_png
    );
}
#[test]
fn concurrent_duplicate_parse_preserves_annotations_and_reading_receipts() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let bytes = b"first line\nsecond line";
    let (a, version) = capture(&mut db, bytes);
    let (b, duplicate) = capture(&mut db, bytes);
    assert_eq!(version.id, duplicate.id);
    let before_parse_version = db.source(&version.source_id).unwrap().row_version;
    let parsed = parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap();
    let block_id = parsed.blocks[1].id.clone();
    db.publish_source_parse(&a.job.id, &a.lease_token, parsed)
        .unwrap();
    assert_eq!(
        db.source(&version.source_id).unwrap().row_version,
        before_parse_version + 1
    );
    let locator = SourceLocator::Text {
        source_version_id: version.id.clone(),
        block_id: block_id.clone(),
        start_codepoint: 0,
        end_codepoint: 6,
        quote: "second".into(),
        prefix: "".into(),
        suffix: " line".into(),
    };
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: locator.clone(),
                comment: "".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let reading_command = id();
    let request = SetReadingState {
        pdf_view: None,
        source_id: version.source_id.clone(),
        source_version_id: version.id.clone(),
        locator: Some(locator),
        status: ReadingStatus::Reading,
        base_row_version: 0,
    };
    let saved = db
        .set_reading_state(&reading_command, request.clone())
        .unwrap();
    let before = db.resolve_annotation(&annotation.id).unwrap();
    db.publish_source_parse(
        &b.job.id,
        &b.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap(),
    )
    .unwrap();
    let after = db.resolve_annotation(&annotation.id).unwrap();
    assert_eq!(before.parse_id, after.parse_id);
    assert_eq!(after.block_offset, Some(1));
    assert_eq!(
        db.source_text_page(&version.id, None, 1, 1).unwrap().blocks[0].id,
        block_id
    );
    assert_eq!(
        db.reading_state(&version.source_id).unwrap().parse_id,
        after.parse_id
    );
    let source = db.source(&version.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id,
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert_eq!(
        db.set_reading_state(&reading_command, request.clone())
            .unwrap()
            .reading
            .unwrap()
            .row_version,
        saved.reading.unwrap().row_version
    );
    assert!(matches!(
        db.set_reading_state(
            &id(),
            SetReadingState {
                pdf_view: None,
                base_row_version: 1,
                ..request
            }
        ),
        Err(Error::Trashed)
    ));
}

#[test]
fn source_cursor_rejects_other_local_copy_and_malformed_positions_before_generation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("original")).unwrap();
    capture(&mut db, b"one");
    capture(&mut db, b"two");
    let request = SourcesPageRequest {
        filter: None,
        limit: 1,
        cursor: None,
        trashed: false,
        query: None,
    };
    let cursor = db
        .sources_page(request.clone())
        .unwrap()
        .next_cursor
        .unwrap();
    let package = root.path().join("copy.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    prepare_restore(&package, root.path().join("copy"), &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let copy = Library::open_existing(root.path().join("copy")).unwrap();
    assert!(matches!(
        copy.sources_page(SourcesPageRequest {
            cursor: Some(cursor.clone()),
            ..request.clone()
        }),
        Err(Error::InvalidInput(_))
    ));
    for (position, value) in [(0, "bad-time"), (1, "not-a-uuid")] {
        let mut corrupt: serde_json::Value = serde_json::from_str(&cursor).unwrap();
        corrupt[position] = value.into();
        corrupt[2] = (-99).into();
        assert!(matches!(
            db.sources_page(SourcesPageRequest {
                cursor: Some(corrupt.to_string()),
                ..request.clone()
            }),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn canceled_staging_never_exposes_partial_blocks_or_replaces_active_parse() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let bytes = (0..350)
        .map(|n| format!("staging {n}\n"))
        .collect::<String>();
    let (job, version) = capture(&mut db, bytes.as_bytes());
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER cancel_parse_batch AFTER INSERT ON source_blocks WHEN NEW.position=100 BEGIN UPDATE source_jobs SET cancel_requested=1; END;").unwrap();
    let parsed = parse_text_source(&version.id, SourceFormat::Text, bytes.as_bytes()).unwrap();
    let first_id = parsed.blocks[0].id.clone();
    assert!(
        db.publish_source_parse(&job.job.id, &job.lease_token, parsed)
            .is_err()
    );
    let parse_id: String = raw
        .query_row("SELECT parse_id FROM source_parse_staging", [], |r| {
            r.get(0)
        })
        .unwrap();
    let count: u32 = raw
        .query_row("SELECT count(*) FROM source_blocks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 200);
    assert!(!db.source(&version.source_id).unwrap().text_ready);
    assert!(
        db.source_text_page(&version.id, Some(&parse_id), 0, 10)
            .is_err()
    );
    assert!(db.search_sources("staging", 10).unwrap().is_empty());
    let locator = SourceLocator::Text {
        source_version_id: version.id.clone(),
        block_id: first_id,
        start_codepoint: 0,
        end_codepoint: 7,
        quote: "staging".into(),
        prefix: "".into(),
        suffix: " 0".into(),
    };
    assert!(matches!(
        db.create_annotation(
            &id(),
            CreateAnnotation {
                locator: locator.clone(),
                comment: "".into()
            }
        ),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.set_reading_state(
            &id(),
            SetReadingState {
                pdf_view: None,
                source_id: version.source_id.clone(),
                source_version_id: version.id.clone(),
                locator: Some(locator),
                status: ReadingStatus::Reading,
                base_row_version: 0
            }
        ),
        Err(Error::NotFound)
    ));
    raw.execute_batch("DROP TRIGGER cancel_parse_batch;")
        .unwrap();
    let (next, _) = capture(&mut db, bytes.as_bytes());
    db.publish_source_parse(
        &next.job.id,
        &next.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, bytes.as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        db.source_text_page(&version.id, None, 0, 1).unwrap().blocks[0].text,
        "staging 0"
    );
    assert!(!db.search_sources("staging", 10).unwrap().is_empty());
}
#[test]
fn immutable_text_unicode_annotations_reading_and_full_package_round_trip() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("original");
    let mut db = Library::open(&workspace).unwrap();
    let bytes = "开头😀中文结尾\n第二段\n第三段".as_bytes();
    let (job, version) = capture(&mut db, bytes);
    assert!(!db.source(&version.source_id).unwrap().text_ready);
    let parsed = parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap();
    let block = parsed.blocks[0].clone();
    db.publish_source_parse(&job.job.id, &job.lease_token, parsed)
        .unwrap();
    let source = db.source(&version.source_id).unwrap();
    assert!(source.lexical_ready);
    assert_eq!(
        db.search_sources("中文", 10).unwrap()[0].source_id,
        source.id
    );
    let first = db.source_text_page(&version.id, None, 0, 1).unwrap();
    assert_eq!(first.next_offset, Some(1));
    assert_eq!(
        db.source_text_page(&version.id, Some(&first.parse_id), 1, 1)
            .unwrap()
            .blocks[0]
            .text,
        "第二段"
    );
    let locator = SourceLocator::Text {
        source_version_id: version.id.clone(),
        block_id: block.id,
        start_codepoint: 2,
        end_codepoint: 5,
        quote: "😀中文".into(),
        prefix: "开头".into(),
        suffix: "结尾".into(),
    };
    let command = id();
    let request = CreateAnnotation {
        locator: locator.clone(),
        comment: "自己的理解".into(),
    };
    let annotation = db
        .create_annotation(&command, request.clone())
        .unwrap()
        .annotation
        .unwrap();
    assert_eq!(
        db.create_annotation(&command, request)
            .unwrap()
            .annotation
            .unwrap()
            .id,
        annotation.id
    );
    assert!(matches!(
        db.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
    let reading = db
        .set_reading_state(
            &id(),
            SetReadingState {
                pdf_view: None,
                source_id: source.id.clone(),
                source_version_id: version.id.clone(),
                locator: Some(locator),
                status: ReadingStatus::Reading,
                base_row_version: 0,
            },
        )
        .unwrap()
        .reading
        .unwrap();
    assert_eq!(reading.row_version, 1);
    db.update_source_metadata(
        &id(),
        UpdateSourceMetadata {
            source_id: source.id.clone(),
            base_row_version: source.row_version,
            display_title: "改名仍可定位".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        db.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
    let mut original = vec![];
    db.source_original_file(&version.id)
        .unwrap()
        .1
        .read_to_end(&mut original)
        .unwrap();
    assert_eq!(original, bytes);
    let exported = root.path().join("original-export.zip");
    db.prepare_source_export(&version.id, &exported, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(exported).unwrap()).unwrap();
    let mut exported_bytes = vec![];
    zip.by_name(&format!("original/{}.txt", version.id))
        .unwrap()
        .read_to_end(&mut exported_bytes)
        .unwrap();
    assert_eq!(exported_bytes, bytes);
    let package = root.path().join("full.pkmsbackup");
    let report = db.create_backup(&package, &mut |_| Ok(())).unwrap();
    assert_eq!(report.manifest.counts.sources, 1);
    assert_eq!(report.verified_blobs, 1);
    verify_backup(&package, &mut |_| Ok(())).unwrap();
    let restored = root.path().join("restored");
    prepare_restore(&package, &restored, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored).unwrap();
    assert_eq!(restored.reading_state(&source.id).unwrap().row_version, 1);
    assert!(matches!(
        restored.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
    assert_eq!(
        restored.source(&source.id).unwrap().display_title,
        "改名仍可定位"
    );
}
#[test]
fn capture_dedup_replays_same_job_cancel_and_recovery_reject_late_worker() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let request = input();
    let command = id();
    let accepted = db.accept_source_import(&command, request.clone()).unwrap();
    assert_eq!(
        db.accept_source_import(&command, request).unwrap().job_id,
        accepted.job_id
    );
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let first = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"same"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let (_, second) = capture(&mut db, b"same");
    assert_eq!(first.source_id, second.source_id);
    assert_eq!(first.id, second.id);
    db.recover_source_jobs().unwrap();
    let new = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    assert_ne!(new.lease_token, lease.lease_token);
    assert!(
        db.publish_source_parse(
            &accepted.job_id,
            &lease.lease_token,
            parse_text_source(&first.id, SourceFormat::Text, b"same").unwrap()
        )
        .is_err()
    );
    db.cancel_source_job(&accepted.job_id).unwrap();
    assert!(
        db.publish_source_parse(
            &accepted.job_id,
            &new.lease_token,
            parse_text_source(&first.id, SourceFormat::Text, b"same").unwrap()
        )
        .is_err()
    );
    assert!(matches!(
        db.source_job(&accepted.job_id).unwrap().status,
        JobStatus::Canceled
    ));
    let events = db.source_job_events(&accepted.job_id, 0, 100).unwrap();
    assert!(events.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
    assert_eq!(
        db.sources_page(SourcesPageRequest {
            limit: 10,
            filter: None,
            cursor: None,
            trashed: false,
            query: None
        })
        .unwrap()
        .items
        .len(),
        1
    );
}
#[test]
fn original_versions_and_historical_annotations_remain_when_current_changes() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (lease, first) = capture(&mut db, b"old original");
    let parsed = parse_text_source(&first.id, SourceFormat::Text, b"old original").unwrap();
    let block = parsed.blocks[0].id.clone();
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: first.id.clone(),
                    block_id: block,
                    start_codepoint: 0,
                    end_codepoint: 3,
                    quote: "old".into(),
                    prefix: "".into(),
                    suffix: " original".into(),
                },
                comment: "".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let source = db.source(&first.source_id).unwrap();
    let mut request = input();
    request.add_version_to = Some(source.id.clone());
    request.base_row_version = Some(source.row_version);
    let accepted = db.accept_source_import(&id(), request).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let second = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"new original"),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        db.source_versions(&first.source_id, None, 1).unwrap()[0].id,
        second.id
    );
    assert_eq!(
        db.source_versions(&first.source_id, Some(&second.id), 1)
            .unwrap()[0]
            .id,
        first.id
    );
    assert_eq!(first.source_id, second.source_id);
    assert!(matches!(
        db.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
    let source = db.source(&source.id).unwrap();
    assert!(!source.text_ready);
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id.clone(),
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(matches!(
        db.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Unavailable
    ));
    let package = root.path().join("history.zip");
    assert_eq!(
        db.create_backup(package, &mut |_| Ok(()))
            .unwrap()
            .verified_blobs,
        2
    );
}
#[test]
fn malformed_input_and_late_database_failures_do_not_commit_half_sources() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let accepted = db.accept_source_import(&id(), input()).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER source_failure BEFORE INSERT ON source_versions BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(
        db.capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"saved blob, rolled back metadata"),
            &mut |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        raw.query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert!(db.source_job(&accepted.job_id).unwrap().source_id.is_none());
    raw.execute_batch("DROP TRIGGER source_failure;").unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"saved blob, rolled back metadata"),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(parse_text_source(&version.id, SourceFormat::Text, &[255]).is_err());
    let mut parsed = parse_text_source(&version.id, SourceFormat::Text, b"safe").unwrap();
    parsed.source_version_id = id();
    assert!(
        db.publish_source_parse(&accepted.job_id, &lease.lease_token, parsed)
            .is_err()
    );
    assert!(!db.source(&version.source_id).unwrap().text_ready);
}

#[test]
fn expired_leases_and_restored_capture_authority_have_explicit_retry_paths() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let queued = db.accept_source_import(&id(), input()).unwrap();
    let (lease, version) = capture(&mut db, b"recoverable original");
    let raw = rusqlite::Connection::open(root.path().join("source/library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE source_jobs SET lease_expires_at='2000-01-01T00:00:00+00:00' WHERE id=?1",
        [&lease.job.id],
    )
    .unwrap();
    db.recover_expired_source_jobs().unwrap();
    let next = db.claim_source_job(&lease.job.id).unwrap().unwrap();
    assert_eq!(next.job.attempt, 2);
    assert!(
        db.publish_source_parse(
            &lease.job.id,
            &lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, b"recoverable original").unwrap()
        )
        .is_err()
    );
    let package = root.path().join("pending.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let target = root.path().join("copy");
    prepare_restore(package, &target, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(&target).unwrap();
    assert_eq!(
        restored
            .source_job(&queued.job_id)
            .unwrap()
            .error_code
            .as_deref(),
        Some("input_unavailable")
    );
    let resumed = restored.claim_source_job(&lease.job.id).unwrap().unwrap();
    assert_eq!(resumed.job.attempt, 3);
    restored
        .fail_source_job(&lease.job.id, &resumed.lease_token, "parser_failed", true)
        .unwrap();
    assert!(matches!(
        restored.source_job(&lease.job.id).unwrap().status,
        JobStatus::Failed
    ));
    let command = id();
    let retry = restored.retry_source_job(&command, &lease.job.id).unwrap();
    assert_eq!(
        restored
            .retry_source_job(&command, &lease.job.id)
            .unwrap()
            .job_id,
        retry.job_id
    );
    let retry_lease = restored.claim_source_job(&retry.job_id).unwrap().unwrap();
    assert_eq!(retry_lease.job.attempt, 1);
    assert_eq!(
        retry_lease.job.source_version_id.as_deref(),
        Some(version.id.as_str())
    );
    restored
        .publish_source_parse(
            &retry.job_id,
            &retry_lease.lease_token,
            parse_text_source(&version.id, SourceFormat::Text, b"recoverable original").unwrap(),
        )
        .unwrap();
    assert!(restored.source(&version.source_id).unwrap().text_ready);
}

#[test]
fn controlled_pdf_geometry_requires_matching_quote_context_and_survives_backup() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let mut input = input();
    input.format = SourceFormat::Pdf;
    input.original_name = "geometry-contract.pdf".into();
    let accepted = db.accept_source_import(&id(), input).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    // Capture header validation only; actual PDF rendering is covered by the worker suite.
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"%PDF-1.7\ngeometry contract input"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let geometry = PdfBlockGeometry {
        page_label: Some("1".into()),
        crop_box: [0., 0., 100., 100.],
        rotation: 0,
        coordinate_space: "pdf_user_space".into(),
        chars: (0..5)
            .map(|i| PdfCharacterBox {
                start_codepoint: i,
                end_codepoint: i + 1,
                rect: if i == 2 {
                    None
                } else {
                    Some([i as f64 * 10., 10., i as f64 * 10. + 10., 20.])
                },
            })
            .collect(),
    };
    let parsed = ParsedSource {
        title: None,
        source_version_id: version.id.clone(),
        parser_version: "controlled-geometry-v1".into(),
        author: None,
        blocks: vec![SourceBlock {
            id: id(),
            text: "AA AA".into(),
            page_index: Some(0),
            pdf_geometry: Some(geometry),
        }],
        page_count: Some(1),
        needs_ocr: false,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    };
    db.publish_source_parse(&accepted.job_id, &lease.lease_token, parsed)
        .unwrap();
    let pdf_block = db.source_text_page(&version.id, None, 0, 1).unwrap().blocks[0]
        .id
        .clone();
    assert!(matches!(
        db.create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: pdf_block,
                    start_codepoint: 0,
                    end_codepoint: 2,
                    quote: "AA".into(),
                    prefix: "".into(),
                    suffix: " AA".into()
                },
                comment: "invalid PDF geometry bypass".into()
            }
        ),
        Err(Error::InvalidInput(_))
    ));
    let locator = SourceLocator::Pdf {
        source_version_id: version.id.clone(),
        page_index: 0,
        page_label: Some("1".into()),
        crop_box: [0., 0., 100., 100.],
        rotation: 0,
        coordinate_space: "pdf_user_space".into(),
        rects: vec![[30., 10., 50., 20.]],
        quote: "AA".into(),
        prefix: "AA ".into(),
        suffix: "".into(),
    };
    let mut reading_locator = locator.clone();
    if let SourceLocator::Pdf {
        quote,
        rects,
        prefix,
        ..
    } = &mut reading_locator
    {
        quote.clear();
        rects.clear();
        prefix.clear();
    }
    db.set_reading_state(
        &id(),
        SetReadingState {
            pdf_view: None,
            source_id: version.source_id.clone(),
            source_version_id: version.id.clone(),
            locator: Some(reading_locator),
            status: ReadingStatus::Reading,
            base_row_version: 0,
        },
    )
    .unwrap();
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: locator.clone(),
                comment: "second occurrence".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    assert!(matches!(
        db.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
    let mut wrong = locator;
    if let SourceLocator::Pdf { rects, .. } = &mut wrong {
        *rects = vec![[0., 10., 20., 20.]];
    }
    let wrong = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: wrong,
                comment: "wrong rectangle for context".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    assert!(matches!(
        db.resolve_annotation(&wrong.id).unwrap().status,
        LocatorStatus::Approximate
    ));
    let package = root.path().join("geometry.zip");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let target = root.path().join("copy");
    prepare_restore(package, &target, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(target).unwrap();
    assert!(matches!(
        restored.resolve_annotation(&annotation.id).unwrap().status,
        LocatorStatus::Exact
    ));
}

#[test]
fn dedup_preserves_lifecycle_collection_versions_and_job_history_is_paginated() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let collection = db.create_collection("主题").unwrap();
    let mut request = input();
    request.collection_id = Some(collection.id.clone());
    let command = id();
    let accepted = db.accept_source_import(&command, request.clone()).unwrap();
    assert_eq!(
        db.replay_source_import(&command, &request)
            .unwrap()
            .unwrap()
            .job_id,
        accepted.job_id
    );
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"indexed text"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let collection_version = db
        .collection_get(&collection.id)
        .unwrap()
        .metadata
        .row_version;
    assert!(collection_version > 1);
    db.publish_source_parse(
        &accepted.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, b"indexed text").unwrap(),
    )
    .unwrap();
    let duplicate = db.accept_source_import(&id(), request.clone()).unwrap();
    let lease = db.claim_source_job(&duplicate.job_id).unwrap().unwrap();
    let duplicated = db
        .capture_source_original(
            &duplicate.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"indexed text"),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(duplicated.id, version.id);
    assert_eq!(
        db.source_job(&duplicate.job_id).unwrap().stage,
        "existing_source"
    );
    assert_eq!(
        db.collection_get(&collection.id)
            .unwrap()
            .metadata
            .row_version,
        collection_version
    );
    let first = db.source_jobs_page(1, None).unwrap();
    assert_eq!(first.items[0].id, duplicate.job_id);
    let second = db.source_jobs_page(1, first.next_cursor).unwrap();
    assert_eq!(second.items[0].id, accepted.job_id);
    assert!(second.next_cursor.is_none());
    let source = db.source(&version.source_id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id.clone(),
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    let duplicate = db.accept_source_import(&id(), request).unwrap();
    let lease = db.claim_source_job(&duplicate.job_id).unwrap().unwrap();
    db.capture_source_original(
        &duplicate.job_id,
        &lease.lease_token,
        &mut Cursor::new(b"indexed text"),
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        db.source_job(&duplicate.job_id).unwrap().stage,
        "existing_trashed_source"
    );
    assert!(db.source(&source.id).unwrap().trashed);
    let invalid = db.accept_source_import(&id(), input()).unwrap();
    let lease = db.claim_source_job(&invalid.job_id).unwrap().unwrap();
    let failed = db
        .fail_source_job(&invalid.job_id, &lease.lease_token, "invalid_utf8", false)
        .unwrap();
    assert_eq!(failed.error_code.as_deref(), Some("invalid_utf8"));
}

#[test]
fn parsed_author_is_filled_once_from_the_document_and_never_overwritten() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (lease, first) = capture(&mut db, b"first original");
    // A format that carries no author of its own leaves the field unset.
    let mut parsed = parse_text_source(&first.id, SourceFormat::Text, b"first original").unwrap();
    assert_eq!(parsed.author, None);
    parsed.author = Some("  张三  ".into());
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    assert_eq!(
        db.source(&first.source_id).unwrap().author.as_deref(),
        Some("张三")
    );
    // The list carries the same value the reader shows.
    let page = db
        .sources_page(SourcesPageRequest {
            filter: None,
            limit: 20,
            cursor: None,
            trashed: false,
            query: None,
        })
        .unwrap();
    assert_eq!(page.items[0].author.as_deref(), Some("张三"));
    // A later version of the same source never replaces what is already shown.
    let source = db.source(&first.source_id).unwrap();
    let mut request = input();
    request.add_version_to = Some(source.id.clone());
    request.base_row_version = Some(source.row_version);
    let accepted = db.accept_source_import(&id(), request).unwrap();
    let lease = db.claim_source_job(&accepted.job_id).unwrap().unwrap();
    let second = db
        .capture_source_original(
            &accepted.job_id,
            &lease.lease_token,
            &mut Cursor::new(b"second original"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let mut parsed = parse_text_source(&second.id, SourceFormat::Text, b"second original").unwrap();
    parsed.author = Some("李四".into());
    db.publish_source_parse(&accepted.job_id, &lease.lease_token, parsed)
        .unwrap();
    assert_eq!(
        db.source(&first.source_id).unwrap().author.as_deref(),
        Some("张三")
    );
}
