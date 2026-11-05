//! Controlled source geometry exercises persistence, not PDF parser/model quality.
use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn capture(db: &mut Library, text: &[u8], format: SourceFormat) -> (SourceVersion, SourceJobLease) {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "private original".into(),
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
            &mut Cursor::new(text),
            &mut |_| Ok(()),
        )
        .unwrap();
    (version, lease)
}
fn query(annotation: &Annotation, text: &str) -> SearchQuery {
    SearchQuery {
        query: text.into(),
        mode: SearchMode::Lexical,
        scope: SearchScope {
            object_ids: vec![annotation.id.clone()],
            ..Default::default()
        },
        limit: 10,
        minimum_cosine: None,
    }
}

#[test]
fn distant_fact_in_long_quote_uses_absolute_nonzero_unicode_offsets() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let prefix = "PRIVATE😀OUTSIDE|";
    let quote = format!(
        "{}the zephyr capacity is 1536 MiB.{}",
        "中😀e\u{301} filler ".repeat(260),
        " padding".repeat(300)
    );
    let suffix = "|HIDDEN_OUTSIDE";
    let full = format!("{prefix}{quote}{suffix}");
    let (version, lease) = capture(&mut db, full.as_bytes(), SourceFormat::Text);
    let parsed = parse_text_source(&version.id, SourceFormat::Text, full.as_bytes()).unwrap();
    assert_eq!(parsed.blocks.len(), 1);
    let block = parsed.blocks[0].id.clone();
    db.publish_source_parse(&lease.job.id, &lease.lease_token, parsed)
        .unwrap();
    let start = prefix.chars().count() as u32;
    let end = start + quote.chars().count() as u32;
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block.clone(),
                    start_codepoint: start,
                    end_codepoint: end,
                    quote: quote.clone(),
                    prefix: prefix.into(),
                    suffix: suffix.into(),
                },
                comment: "comment excluded".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let results = db
        .keyword_search(&query(&annotation, "zephyr capacity 1536"))
        .unwrap();
    assert_eq!(results.items.len(), 1);
    let evidence = &results.items[0];
    assert!(evidence.text.contains("zephyr capacity is 1536 MiB"));
    assert_eq!(evidence.target.block_id, block);
    assert_eq!(evidence.target.content_revision, version.id);
    assert!(evidence.target.start_codepoint > start + 2048);
    assert!(evidence.target.start_codepoint >= start && evidence.target.end_codepoint <= end);
    assert!(evidence.text.chars().count() <= 2048);
    assert_eq!(
        evidence.text,
        full.chars()
            .skip(evidence.target.start_codepoint as usize)
            .take((evidence.target.end_codepoint - evidence.target.start_codepoint) as usize)
            .collect::<String>()
    );
    assert!(!evidence.text.contains("PRIVATE") && !evidence.text.contains("HIDDEN_OUTSIDE"));
    assert_eq!(
        evidence.annotation.as_ref().unwrap().annotation_id,
        annotation.id
    );
}

fn geometry(text: &str) -> PdfBlockGeometry {
    PdfBlockGeometry {
        page_label: Some("2".into()),
        crop_box: [0., 0., 100., 100.],
        rotation: 0,
        coordinate_space: "pdf_user_space".into(),
        chars: text
            .chars()
            .enumerate()
            .map(|(i, _)| PdfCharacterBox {
                start_codepoint: i as u32,
                end_codepoint: i as u32 + 1,
                rect: Some([i as f64 * 10., 0., i as f64 * 10. + 10., 10.]),
            })
            .collect(),
    }
}

fn pdf_frozen(db: &mut Library) -> (SourceVersion, String, String) {
    let (version, lease) = capture(
        db,
        b"%PDF-1.7 controlled two-page geometry",
        SourceFormat::Pdf,
    );
    db.publish_source_parse(
        &lease.job.id,
        &lease.lease_token,
        ParsedSource {
            title: None,
            source_version_id: version.id.clone(),
            parser_version: "controlled-original".into(),
            author: None,
            page_count: Some(2),
            needs_ocr: false,
            blocks: vec![SourceBlock {
                id: id(),
                text: "ABCD".into(),
                page_index: Some(1),
                pdf_geometry: Some(geometry("ABCD")),
            }],
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
    )
    .unwrap();
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Pdf {
                    source_version_id: version.id.clone(),
                    page_index: 1,
                    page_label: Some("2".into()),
                    crop_box: [0., 0., 100., 100.],
                    rotation: 0,
                    coordinate_space: "pdf_user_space".into(),
                    rects: vec![[10., 0., 30., 10.]],
                    quote: "BC".into(),
                    prefix: "A".into(),
                    suffix: "D".into(),
                },
                comment: "".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    db.set_reading_state(
        &id(),
        SetReadingState {
            pdf_view: None,
            source_id: version.source_id.clone(),
            source_version_id: version.id.clone(),
            locator: Some(annotation.locator.clone()),
            status: ReadingStatus::Reading,
            base_row_version: 0,
        },
    )
    .unwrap();
    let accepted = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                agent: None,
                search: query(&annotation, "BC"),
                model: None,
            },
        )
        .unwrap();
    let answer_lease = db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    let frozen = db
        .prepare_answer_evidence(&accepted.answer.id, &answer_lease.lease_token, vec![])
        .unwrap();
    assert_eq!(frozen.status, AnswerStatus::ModelUnconfigured);
    assert_eq!(frozen.evidence.len(), 1);
    assert_eq!(frozen.evidence[0].evidence.text, "BC");
    assert!(frozen.evidence_current);
    let parse = frozen.evidence[0].evidence.target.parse_id.clone().unwrap();
    (version, frozen.id, parse)
}

fn replace_parse(root: &std::path::Path, version: &SourceVersion, pages: u32) {
    // Simulates publication by a later parser for the same immutable original;
    // earlier immutable generations remain intact for frozen evidence.
    let sql = rusqlite::Connection::open(root.join("library.sqlite3")).unwrap();
    let parse = id();
    sql.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) VALUES(?1,?2,'controlled-next',?3,0,'2026-10-03T00:00:00Z')",
        rusqlite::params![parse, version.id, pages]).unwrap();
    let page = pages - 1;
    sql.execute("INSERT INTO source_blocks(id,parse_id,position,text,page_index,geometry_json) VALUES(?1,?2,0,'AXYD',?3,?4)",
        rusqlite::params![id(), parse, page, serde_json::to_string(&geometry("AXYD")).unwrap()]).unwrap();
    sql.execute(
        "UPDATE source_active_parses SET parse_id=?1 WHERE source_version_id=?2",
        rusqlite::params![parse, version.id],
    )
    .unwrap();
}

fn assert_frozen_roundtrip(
    db: &Library,
    root: &std::path::Path,
    answer_id: &str,
    original_parse: &str,
) {
    let old = db.answer(answer_id).unwrap();
    let reading_before = db
        .reading_state(&old.evidence[0].evidence.target.object_id)
        .unwrap();
    assert!(!old.evidence_current);
    assert_eq!(old.evidence[0].evidence.text, "BC");
    assert_eq!(
        old.evidence[0].evidence.target.parse_id.as_deref(),
        Some(original_parse)
    );
    let backup = root.join("history.zip");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let destination = root.join("restored");
    prepare_restore(&backup, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open(destination).unwrap();
    let result = restored.answer(answer_id).unwrap();
    let reading_after = restored
        .reading_state(&result.evidence[0].evidence.target.object_id)
        .unwrap();
    assert_eq!(
        reading_after.source_version_id,
        reading_before.source_version_id
    );
    assert_eq!(
        serde_json::to_value(&reading_after.locator).unwrap(),
        serde_json::to_value(&reading_before.locator).unwrap()
    );
    assert!(matches!(
        reading_after.locator,
        Some(SourceLocator::Pdf { page_index: 1, .. })
    ));
    assert!(!result.evidence_current);
    assert_eq!(result.evidence[0].evidence.text, "BC");
    assert_eq!(
        result.evidence[0].evidence.target.parse_id.as_deref(),
        Some(original_parse)
    );
}

#[test]
fn pdf_frozen_evidence_keeps_historical_parse_after_active_text_changes() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, answer, parse) = pdf_frozen(&mut db);
    replace_parse(root.path(), &version, 2);
    assert_frozen_roundtrip(&db, root.path(), &answer, &parse);
}

#[test]
fn pdf_historical_page_remains_backupable_when_later_parse_reports_fewer_pages() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, answer, parse) = pdf_frozen(&mut db);
    replace_parse(root.path(), &version, 1);
    let locator = db
        .reading_state(&version.source_id)
        .unwrap()
        .locator
        .unwrap();
    assert!(matches!(
        db.create_annotation(
            &id(),
            CreateAnnotation {
                locator: locator.clone(),
                comment: "".into()
            }
        ),
        Err(Error::InvalidInput("PDF page is unavailable"))
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
                base_row_version: 1,
            }
        ),
        Err(Error::InvalidInput("PDF page is unavailable"))
    ));
    assert_frozen_roundtrip(&db, root.path(), &answer, &parse);
}

#[test]
fn pdf_backup_rejects_pages_not_in_any_published_parse_even_if_staging_claims_them() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, _, _) = pdf_frozen(&mut db);
    replace_parse(root.path(), &version, 1);
    let sql = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    // Reading has its own historical validation path; a missing page must fail
    // before any frozen answer checks, even with a valid annotation left intact.
    sql.execute("UPDATE reading_states SET locator_json=json_set(locator_json,'$.page_index',2) WHERE source_id=?1", [&version.source_id]).unwrap();
    assert!(matches!(
        db.prepare_backup(root.path().join("invalid-reading.zip"), &mut |_| Ok(())),
        Err(Error::InvalidInput("PDF page is unavailable"))
    ));
    sql.execute("UPDATE reading_states SET locator_json=json_set(locator_json,'$.page_index',1) WHERE source_id=?1", [&version.source_id]).unwrap();
    sql.execute("UPDATE annotations SET locator_json=json_set(locator_json,'$.page_index',2) WHERE source_version_id=?1", [&version.id]).unwrap();
    assert!(matches!(
        db.prepare_backup(root.path().join("invalid-annotation.zip"), &mut |_| Ok(())),
        Err(Error::InvalidInput("PDF page is unavailable"))
    ));
    let stage = id();
    sql.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) VALUES(?1,?2,'unfinished-parser',3,0,'2026-10-03T00:00:00Z')", rusqlite::params![stage,version.id]).unwrap();
    sql.execute("INSERT INTO source_parse_staging(parse_id,job_id,lease_token) SELECT ?1,id,?2 FROM source_jobs WHERE source_version_id=?3 LIMIT 1", rusqlite::params![stage,id(),version.id]).unwrap();
    assert!(matches!(
        db.prepare_backup(root.path().join("staging-page.zip"), &mut |_| Ok(())),
        Err(Error::InvalidInput("PDF page is unavailable"))
    ));
}
