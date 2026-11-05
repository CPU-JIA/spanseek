use pkms_core::{Error, Library, ReportTemplate, SearchMode, SearchQuery, SearchScope};

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        query: text.into(),
        mode: SearchMode::Lexical,
        scope: SearchScope::default(),
        limit: 10,
        minimum_cosine: None,
    }
}

#[test]
fn evidence_report_is_cited_and_becomes_invalid_after_revision_change() {
    let directory = tempfile::tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let note = library
        .create_note("报告证据", "每个关键判断都必须能回到具体修订与文本块。")
        .unwrap();
    let evidence = library.keyword_search(&query("关键判断")).unwrap().items;
    let template = ReportTemplate::built_in().unwrap();
    let report = library
        .build_evidence_report("交付样本", "如何核对引用？", evidence, &template)
        .unwrap();
    assert_eq!(report.generated_by, "evidence_extractive");
    assert!(report.markdown.contains("[E1]"));
    assert_eq!(report.sources[0].object_id, note.id);
    assert!(library.verify_report(&report).unwrap().all_valid);

    library
        .save_note(&note.id, 1, "报告证据（修订）", "正文已经变化。")
        .unwrap();
    let verification = library.verify_report(&report).unwrap();
    assert!(!verification.all_valid);
    assert_eq!(verification.invalid_citation_ids, vec!["E1"]);
}

#[test]
fn no_evidence_and_uncited_model_text_fail_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let library = Library::open(directory.path()).unwrap();
    let template = ReportTemplate::built_in().unwrap();
    assert!(matches!(
        library.build_evidence_report("空报告", "", vec![], &template),
        Err(Error::InvalidInput("no_evidence"))
    ));

    let mut library = Library::open(directory.path()).unwrap();
    let _note = library.create_note("引用", "有据正文").unwrap();
    let evidence = library.keyword_search(&query("有据")).unwrap().items;
    assert!(matches!(
        library.build_model_report("模型报告", "", "没有引用的段落。", evidence, &template),
        Err(Error::InvalidInput("report_paragraph_without_citation"))
    ));
}
