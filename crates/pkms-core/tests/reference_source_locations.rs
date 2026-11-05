//! Incremental reference navigation checks; fixtures do not test the external PDF parser.
use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn captured(db: &mut Library, format: SourceFormat) -> (SourceVersion, SourceJobLease) {
    let original: &[u8] = if format == SourceFormat::Pdf {
        b"%PDF-1.7 controlled immutable original"
    } else {
        b"controlled immutable original"
    };
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "reference navigation fixture".into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(original),
            &mut |_| Ok(()),
        )
        .unwrap();
    (version, lease)
}

fn resolve(db: &Library, version: &SourceVersion, block: &str) -> ResolvedLink {
    db.resolve_link(ResolveLink {
        target_id: version.source_id.clone(),
        target_revision_id: Some(version.id.clone()),
        target_block_id: Some(block.into()),
    })
    .unwrap()
}

#[test]
fn pdf_block_reference_locates_the_original_page_without_fabricated_highlights() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, lease) = captured(&mut db, SourceFormat::Pdf);
    let block = id();
    let no_geometry = id();
    db.publish_source_parse(
        &lease.job.id,
        &lease.lease_token,
        ParsedSource {
            title: None,
            source_version_id: version.id.clone(),
            parser_version: "controlled-reference-pages".into(),
            author: None,
            page_count: Some(100),
            needs_ocr: false,
            blocks: vec![
                SourceBlock {
                    id: block.clone(),
                    text: "第九十九页".into(),
                    page_index: Some(98),
                    pdf_geometry: Some(PdfBlockGeometry {
                        page_label: Some("xcix".into()),
                        crop_box: [10., 20., 610., 820.],
                        rotation: 90,
                        coordinate_space: "pdf_user_space".into(),
                        chars: (0..5)
                            .map(|index| PdfCharacterBox {
                                start_codepoint: index,
                                end_codepoint: index + 1,
                                rect: None,
                            })
                            .collect(),
                    }),
                },
                SourceBlock {
                    id: no_geometry.clone(),
                    text: "Legacy page without geometry".into(),
                    page_index: Some(99),
                    pdf_geometry: None,
                },
            ],
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
    )
    .unwrap();
    let resolved = resolve(&db, &version, &block);
    assert_eq!(resolved.state, LinkState::Available);
    assert_eq!(resolved.block_text.as_deref(), Some("第九十九页"));
    let target = resolved.source_target.unwrap();
    assert_eq!(target.source_version_id, version.id);
    assert_eq!(target.block_offset, Some(0));
    assert!(target.parse_id.is_some());
    assert!(target.annotation_id.is_none());
    assert!(matches!(
        target.locator_status,
        Some(LocatorStatus::Approximate)
    ));
    match target.locator.unwrap() {
        SourceLocator::Pdf {
            page_index,
            page_label,
            crop_box,
            rotation,
            rects,
            quote,
            ..
        } => {
            assert_eq!(page_index, 98);
            assert_eq!(page_label.as_deref(), Some("xcix"));
            assert_eq!(crop_box, [10., 20., 610., 820.]);
            assert_eq!(rotation, 90);
            assert!(rects.is_empty());
            assert!(quote.is_empty());
        }
        _ => panic!("A PDF reference must never create a text locator"),
    }
    let unavailable = resolve(&db, &version, &no_geometry).source_target.unwrap();
    assert!(unavailable.locator.is_none());
    assert!(matches!(
        unavailable.locator_status,
        Some(LocatorStatus::Unavailable)
    ));
    assert_eq!(unavailable.block_offset, Some(1));
    assert_eq!(resolve(&db, &version, &id()).state, LinkState::BlockMissing);
    assert_eq!(
        db.annotations_page(&version.id, None, 10, false)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn source_paragraph_reference_preserves_unicode_and_the_exact_block() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, lease) = captured(&mut db, SourceFormat::Text);
    let parsed = parse_text_source(
        &version.id,
        SourceFormat::Text,
        "同名正文\n目标😀中段".as_bytes(),
    )
    .unwrap();
    let block = parsed.blocks[1].id.clone();
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    let resolved = resolve(&db, &version, &block).source_target.unwrap();
    assert!(matches!(
        resolved.locator_status,
        Some(LocatorStatus::Exact)
    ));
    assert_eq!(resolved.block_offset, Some(1));
    match resolved.locator.unwrap() {
        SourceLocator::Text {
            block_id,
            start_codepoint,
            end_codepoint,
            quote,
            ..
        } => {
            assert_eq!(block_id, block);
            assert_eq!(start_codepoint, 0);
            assert_eq!(end_codepoint, 5);
            assert_eq!(quote, "目标😀中段");
        }
        _ => panic!("A text source must keep its exact paragraph identity"),
    }
    let other = Library::open(root.path().join("other-workspace")).unwrap();
    assert_eq!(
        resolve(&other, &version, &block).state,
        LinkState::TargetMissing
    );
}
