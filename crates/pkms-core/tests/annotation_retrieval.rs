use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn import(
    db: &mut Library,
    bytes: &[u8],
    format: SourceFormat,
    previous: Option<SourceSummary>,
) -> (SourceJobLease, SourceVersion) {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "private document".into(),
                format,
                origin_url: None,
                collection_id: None,
                add_version_to: previous.as_ref().map(|s| s.id.clone()),
                base_row_version: previous.map(|s| s.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    (lease, version)
}
fn text_fixture(db: &mut Library) -> (SourceVersion, Annotation) {
    let bytes = b"SECRET|ALLOWED|PRIVATE";
    let (lease, v) = import(db, bytes, SourceFormat::Text, None);
    let parsed = parse_text_source(&v.id, SourceFormat::Text, bytes).unwrap();
    let block = parsed.blocks[0].id.clone();
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    let a = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: v.id.clone(),
                    block_id: block,
                    start_codepoint: 7,
                    end_codepoint: 14,
                    quote: "ALLOWED".into(),
                    prefix: "SECRET|".into(),
                    suffix: "|PRIVATE".into(),
                },
                comment: "COMMENT SECRET".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    (v, a)
}
fn scope(a: &Annotation) -> SearchScope {
    SearchScope {
        object_ids: vec![a.id.clone()],
        ..Default::default()
    }
}
fn flag(db: &mut Library, id: &str, value: bool) {
    let base = db.object_metadata(id).unwrap().row_version;
    db.set_object_search_enabled(
        &Uuid::new_v4().to_string(),
        SetObjectFlag {
            object_id: id.into(),
            base_row_version: base,
            value,
        },
    )
    .unwrap();
}
#[test]
fn exact_membership_exposes_only_quote_not_parent_comment_or_context() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (v, a) = text_fixture(&mut db);
    assert!(
        db.annotation_search_inputs(&SearchScope::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        db.annotation_search_inputs(&SearchScope {
            object_ids: vec![v.source_id.clone()],
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );
    let collection = db.create_collection("theme").unwrap();
    db.add_to_collection(&collection.id, &v.source_id).unwrap();
    let q = SearchScope {
        collection_id: Some(collection.id.clone()),
        ..Default::default()
    };
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
    db.add_to_collection(&collection.id, &a.id).unwrap();
    let hits = db.annotation_search_inputs(&q).unwrap();
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.text, "ALLOWED");
    assert_eq!(hit.title, "摘录");
    assert_eq!(hit.target.object_id, v.source_id);
    assert_eq!(hit.target.start_codepoint, 7);
    assert_eq!(hit.target.end_codepoint, 14);
    assert_eq!(hit.annotation.as_ref().unwrap().annotation_id, a.id);
    assert_eq!(db.annotation_search_inputs(&q).unwrap()[0].id, hit.id);
    assert!(
        db.annotation_search_inputs(&SearchScope {
            kinds: vec![EvidenceKind::Note],
            ..q.clone()
        })
        .unwrap()
        .is_empty()
    );
    let mut both = q.clone();
    both.object_ids = vec![v.source_id];
    assert!(db.annotation_search_inputs(&both).unwrap().is_empty());
}
#[test]
fn tags_archive_and_both_lifecycle_gates_apply_to_actual_annotation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (v, a) = text_fixture(&mut db);
    let mut q = scope(&a);
    let tag = db
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: v.source_id.clone(),
                    base_row_version: db.object_metadata(&v.source_id).unwrap().row_version,
                }],
                tag: TagSelector::Name {
                    name: "parent only".into(),
                },
            },
        )
        .unwrap()
        .tag
        .unwrap();
    q.tag_ids = vec![tag.id.clone()];
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: a.id.clone(),
                base_row_version: 1,
            }],
            tag: TagSelector::Id { id: tag.id },
        },
    )
    .unwrap();
    assert_eq!(db.annotation_search_inputs(&q).unwrap().len(), 1);
    flag(&mut db, &v.source_id, false);
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
    flag(&mut db, &v.source_id, true);
    flag(&mut db, &a.id, false);
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
    flag(&mut db, &a.id, true);
    q.archived = Some(true);
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
    q.archived = None;
    let base = db.object_metadata(&a.id).unwrap().row_version;
    db.set_annotation_trashed(
        &id(),
        SetObjectFlag {
            object_id: a.id.clone(),
            base_row_version: base,
            value: true,
        },
    )
    .unwrap();
    assert!(db.annotation_search_inputs(&q).unwrap().is_empty());
}
#[test]
fn fixed_text_version_remains_retrievable_after_source_update() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (old, a) = text_fixture(&mut db);
    let previous = db.source(&old.source_id).unwrap();
    let bytes = b"new private content without the old quote";
    let (lease, new) = import(&mut db, bytes, SourceFormat::Text, Some(previous));
    db.publish_source_parse(
        &lease.job.id,
        &lease.lease_token,
        parse_text_source(&new.id, SourceFormat::Text, bytes).unwrap(),
    )
    .unwrap();
    let hit = db.annotation_search_inputs(&scope(&a)).unwrap().remove(0);
    assert_eq!(hit.target.content_revision, old.id);
    assert_eq!(hit.text, "ALLOWED");
    assert_ne!(hit.target.content_revision, new.id);
}
#[test]
fn exact_pdf_selection_splits_real_blocks_and_approximate_never_yields_page_text() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (lease, v) = import(
        &mut db,
        b"%PDF-1.7\ncontrolled geometry fixture",
        SourceFormat::Pdf,
        None,
    );
    let mut blocks = vec![];
    for (line, text) in ["AB", "CD"].iter().enumerate() {
        blocks.push(SourceBlock {
            id: id(),
            text: text.to_string(),
            page_index: Some(0),
            pdf_geometry: Some(PdfBlockGeometry {
                page_label: Some("1".into()),
                crop_box: [0., 0., 100., 100.],
                rotation: 0,
                coordinate_space: "pdf_user_space".into(),
                chars: (0..2)
                    .map(|i| PdfCharacterBox {
                        start_codepoint: i,
                        end_codepoint: i + 1,
                        rect: Some([
                            i as f64 * 10.,
                            line as f64 * 20.,
                            i as f64 * 10. + 10.,
                            line as f64 * 20. + 10.,
                        ]),
                    })
                    .collect(),
            }),
        });
    }
    db.publish_source_parse(
        &lease.job.id,
        &lease.lease_token,
        ParsedSource {
            source_version_id: v.id.clone(),
            parser_version: "controlled-pdf".into(),
            author: None,
            blocks,
            title: None,
            page_count: Some(1),
            needs_ocr: false,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
    )
    .unwrap();
    let locator = SourceLocator::Pdf {
        source_version_id: v.id,
        page_index: 0,
        page_label: Some("1".into()),
        crop_box: [0., 0., 100., 100.],
        rotation: 0,
        coordinate_space: "pdf_user_space".into(),
        rects: vec![[10., 0., 20., 10.], [0., 20., 10., 30.]],
        quote: "BC".into(),
        prefix: "A".into(),
        suffix: "D".into(),
    };
    let a = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: locator.clone(),
                comment: "private".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let hits = db.annotation_search_inputs(&scope(&a)).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        vec!["B", "C"]
    );
    assert_ne!(hits[0].target.block_id, hits[1].target.block_id);
    assert_eq!(
        (hits[0].target.start_codepoint, hits[0].target.end_codepoint),
        (1, 2)
    );
    assert_eq!(
        (hits[1].target.start_codepoint, hits[1].target.end_codepoint),
        (0, 1)
    );
    let mut wrong = locator;
    if let SourceLocator::Pdf { quote, .. } = &mut wrong {
        *quote = "NOT THE SELECTED TEXT".into();
    }
    let bad = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: wrong,
                comment: String::new(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    assert!(
        db.annotation_search_inputs(&scope(&bad))
            .unwrap()
            .is_empty()
    );
}
