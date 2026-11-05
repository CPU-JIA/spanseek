//! Text cleaning: the synthetic before/after comparison, the per-library
//! settings contract, and the report a published parse carries.
use pkms_core::*;
use std::io::{Cursor, Read};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn block(text: &str, page: Option<u32>) -> SourceBlock {
    SourceBlock {
        id: id(),
        text: text.into(),
        page_index: page,
        pdf_geometry: None,
    }
}

/// Four synthetic pages: a running header, a repeated watermark, one real body
/// block with invisible characters, and a constant footer.
fn synthetic_pages() -> Vec<SourceBlock> {
    let mut blocks = Vec::new();
    for page in 0..4u32 {
        blocks.push(block("会议纪要 · 内部", Some(page)));
        blocks.push(block("绝密", Some(page)));
        blocks.push(block(
            &format!("第 {page} 页正文，含零宽\u{200b}字符与\u{FEFF}标记。"),
            Some(page),
        ));
        blocks.push(block("第 X 页 共 4 页", Some(page)));
    }
    blocks
}

#[test]
fn repeated_page_edges_watermark_and_control_characters_are_removed_with_a_report() {
    let mut blocks = synthetic_pages();
    let before = blocks.iter().map(|b| b.text.clone()).collect::<Vec<_>>();
    assert_eq!(before.len(), 16);

    let report = clean_blocks(&mut blocks, &CleaningParams::default());

    // Only the four body blocks survive, and each one lost its two invisible
    // characters while keeping its text.
    assert_eq!(blocks.len(), 4);
    assert_eq!(
        blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(),
        [
            "第 0 页正文，含零宽字符与标记。",
            "第 1 页正文，含零宽字符与标记。",
            "第 2 页正文，含零宽字符与标记。",
            "第 3 页正文，含零宽字符与标记。",
        ]
    );
    assert!(blocks.iter().all(|b| b.page_index.is_some()));
    assert_eq!(report.headers_footers, 8);
    assert_eq!(report.watermark, 4);
    assert_eq!(report.control_chars, 8);
    assert_eq!(report.removed_blocks, 12);
    assert_eq!(report.replacement_chars, 0);
    assert!(!report.kept_everything);
    assert!(report.changed());
    // The report records the options that produced it.
    assert_eq!(report.params, CleaningParams::default());
}

#[test]
fn replacement_characters_are_counted_but_never_silently_deleted() {
    let mut blocks = vec![block("编码损坏\u{FFFD}的位置", None)];
    let report = clean_blocks(&mut blocks, &CleaningParams::default());
    assert_eq!(blocks[0].text, "编码损坏\u{FFFD}的位置");
    assert_eq!(report.replacement_chars, 1);
    assert_eq!(report.control_chars, 0);
    assert!(!report.changed());
}

#[test]
fn disabled_steps_leave_the_text_alone() {
    let mut blocks = synthetic_pages();
    let report = clean_blocks(
        &mut blocks,
        &CleaningParams {
            headers_footers: false,
            watermark: false,
            control_chars: false,
        },
    );
    assert_eq!(blocks.len(), 16);
    assert!(blocks[2].text.contains('\u{200b}'));
    assert_eq!(report.removed_blocks, 0);
    assert_eq!(report.control_chars, 0);
    assert!(!report.changed());
}

#[test]
fn formats_without_pages_only_lose_control_characters() {
    let mut blocks = vec![
        block("同一行\u{200b}", None),
        block("同一行\u{200b}", None),
        block("同一行\u{200b}", None),
    ];
    let report = clean_blocks(&mut blocks, &CleaningParams::default());
    assert_eq!(blocks.len(), 3);
    assert_eq!(report.removed_blocks, 0);
    assert_eq!(report.headers_footers, 0);
    assert_eq!(report.watermark, 0);
    assert_eq!(report.control_chars, 3);
    assert!(blocks.iter().all(|b| b.text == "同一行"));
}

#[test]
fn cleaning_never_empties_a_document() {
    let mut blocks = (0..3).map(|page| block("扫描件", Some(page))).collect();
    let report = clean_blocks(&mut blocks, &CleaningParams::default());
    assert_eq!(blocks.len(), 3);
    assert_eq!(report.removed_blocks, 0);
    assert!(report.kept_everything);
}

#[test]
fn cleaning_settings_roundtrip_replay_and_conflict() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let initial = db.cleaning_settings().unwrap();
    assert_eq!(initial.params, CleaningParams::default());
    assert!(
        initial.params.headers_footers && initial.params.watermark && initial.params.control_chars
    );
    assert_eq!(initial.row_version, 0);

    let command = id();
    let request = SetCleaningParams {
        base_row_version: 0,
        params: CleaningParams {
            headers_footers: true,
            watermark: false,
            control_chars: true,
        },
    };
    let saved = db.set_cleaning_params(&command, request.clone()).unwrap();
    assert_eq!(saved.settings.row_version, 1);
    assert!(!saved.receipt.replayed);
    assert_eq!(db.cleaning_settings().unwrap(), saved.settings);
    let replayed = db.set_cleaning_params(&command, request).unwrap();
    assert!(replayed.receipt.replayed);
    assert_eq!(replayed.settings, saved.settings);
    assert!(matches!(
        db.set_cleaning_params(
            &id(),
            SetCleaningParams {
                base_row_version: 0,
                params: CleaningParams::default(),
            }
        ),
        Err(Error::MetadataConflict { .. })
    ));
    drop(db);
    let reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(reopened.cleaning_settings().unwrap(), saved.settings);
}

#[test]
fn a_published_parse_carries_its_cleaning_report() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "带不可见字符.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let bytes = "第一行\u{200b}正文\n第二行".as_bytes();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    let mut parsed = parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap();
    assert!(parsed.blocks[0].text.contains('\u{200b}'));
    parsed.author = Some("测试作者".into());
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
        .unwrap();
    let page = db.source_text_page(&version.id, None, 0, 10).unwrap();
    let report = page.cleaning.expect("report stored with the parse");
    assert_eq!(report.control_chars, 1);
    assert_eq!(report.removed_blocks, 0);
    assert_eq!(page.blocks[0].text, "第一行正文");
    // The original bytes are untouched by cleaning.
    let (_, mut original) = db.source_original_file(&version.id).unwrap();
    let mut stored = Vec::new();
    original.read_to_end(&mut stored).unwrap();
    assert_eq!(stored, bytes);
}
