use pkms_core::*;
use std::{fs, io::Read};
use tempfile::tempdir;

const BIB: &str = "\
@article{vaswani2017,
  title = {Attention is All You Need},
  author = {Vaswani, Ashish and Shazeer, Noam},
  journal = {Advances in Neural Information Processing Systems},
  year = {2017},
  doi = {10.48550/arXiv.1706.03762}
}

@inproceedings{devlin2019,
  title = {BERT: Pre-training of Deep Bidirectional Transformers},
  author = {Devlin, Jacob and Chang, Ming-Wei},
  booktitle = {NAACL-HLT},
  year = {2019},
  url = {https://aclanthology.org/N19-1423/}
}
";

#[test]
fn bibtex_import_resolves_citations_and_appends_markers() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();

    let report = library.import_bibliography(&bib).unwrap();
    assert_eq!(report.entries, 2, "{report:?}");
    assert_eq!(report.imported, 2, "{report:?}");
    assert_eq!(report.skipped_existing, 0, "{report:?}");

    let again = library.import_bibliography(&bib).unwrap();
    assert_eq!(again.imported, 0, "{again:?}");
    assert_eq!(again.skipped_existing, 2, "{again:?}");

    let csl_dir = tempdir().unwrap();
    let csl = csl_dir.path().join("extra.json");
    fs::write(
        &csl,
        r#"[{"id":"devlin2019","type":"paper-conference","title":"duplicate"},{"id":"cslonly","type":"webpage","title":"CSL only","URL":"https://example.test"}]"#,
    )
    .unwrap();
    let csl_report = library.import_bibliography(&csl).unwrap();
    assert_eq!(csl_report.entries, 2, "{csl_report:?}");
    assert_eq!(csl_report.imported, 1, "{csl_report:?}");
    assert_eq!(csl_report.skipped_existing, 1, "{csl_report:?}");

    let catalog = library.bibliography_catalog().unwrap();
    assert_eq!(
        catalog
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>(),
        ["cslonly", "devlin2019", "vaswani2017"]
    );
    assert!(catalog.iter().all(|entry| !entry.note_id.is_empty()));
    assert!(catalog.iter().all(|entry| !entry.formatted.is_empty()));
    assert!(catalog.iter().all(|entry| !entry.title.is_empty()));
    let empty_root = tempdir().unwrap();
    let empty = Library::open(empty_root.path()).unwrap();
    assert!(empty.bibliography_catalog().unwrap().is_empty());
    let draft = library
        .create_note(
            "论文草稿",
            "方法部分参考两篇工作[@vaswani2017; @devlin2019]。",
        )
        .unwrap();
    let bibliography = library.bibliography(&draft.id).unwrap();
    assert_eq!(bibliography.keys, ["vaswani2017", "devlin2019"]);
    assert_eq!(bibliography.entries.len(), 2);
    assert!(bibliography.missing.is_empty());
    assert_eq!(bibliography.entries[0].key, "vaswani2017");
    assert!(!bibliography.entries[0].note_id.is_empty());
    assert!(
        bibliography.markdown.contains("[J]"),
        "{}",
        bibliography.markdown
    );
    assert!(
        bibliography.markdown.contains("[C]"),
        "{}",
        bibliography.markdown
    );

    let gap = library.create_note("缺口", "待补[@nokey]。").unwrap();
    let missing = library.bibliography(&gap.id).unwrap();
    assert_eq!(missing.missing, ["nokey"]);
    assert!(missing.markdown.is_empty());

    let before = library.note(&draft.id).unwrap().revision;
    let updated = library.citation_insert(&draft.id, "vaswani2017").unwrap();
    assert_eq!(updated.revision, before + 1);
    assert!(updated.document.plain_text().contains("[@vaswani2017]"));
    assert!(matches!(
        library.citation_insert(&draft.id, "nokey"),
        Err(Error::NotFound)
    ));
}
#[test]
fn a_switched_style_rerenders_the_same_entries() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library.create_note("样式", "参考[@vaswani2017]。").unwrap();

    let plain = library.bibliography(&draft.id).unwrap();
    assert_eq!(plain.style, CitationStyle::Plain);
    // The default plain format already follows GB/T 7714 ordering, so the
    // styled variants are the incremental ones.
    assert!(plain.markdown.contains("[J]"), "{}", plain.markdown);

    let apa = library
        .bibliography_styled(&draft.id, CitationStyle::Apa)
        .unwrap();
    assert_ne!(plain.entries[0].formatted, apa.entries[0].formatted);
    assert!(apa.entries[0].formatted.contains("(2017)."));
    assert!(
        apa.entries[0]
            .formatted
            .contains("https://doi.org/10.48550/arXiv.1706.03762"),
        "{}",
        apa.entries[0].formatted
    );
    let chicago = library
        .bibliography_styled(&draft.id, CitationStyle::ChicagoAuthorDate)
        .unwrap();
    assert!(
        chicago.entries[0].formatted.contains("2017."),
        "{}",
        chicago.entries[0].formatted
    );
    assert!(
        chicago.entries[0]
            .formatted
            .contains("Attention Is All You Need"),
        "{}",
        chicago.entries[0].formatted
    );
    let gbt = library
        .bibliography_styled(&draft.id, CitationStyle::Gbt7714)
        .unwrap();
    assert_eq!(gbt.style, CitationStyle::Gbt7714);
    assert!(
        gbt.entries[0].formatted.contains("[J"),
        "{}",
        gbt.entries[0].formatted
    );
    assert!(
        gbt.entries[0].formatted.contains("VASWANI A"),
        "{}",
        gbt.entries[0].formatted
    );
    assert!(!gbt.entries[0].formatted.contains('\u{1b}'));
    assert!(!apa.entries[0].formatted.contains('\u{1b}'));
}

#[test]
fn frontmatter_rebuilds_the_structured_entry() {
    let source = "citation_key: \"k1\"\ncitation_type: \"article\"\nyear: \"2020\"\nauthors: \"A, B; C, D\"\ncontainer: \"Journal\"\ndoi: \"10.1/x\"";
    let entry = bib_entry_from_frontmatter(source, "A title").unwrap();
    assert_eq!(entry.key, "k1");
    assert_eq!(entry.entry_type, "article");
    assert_eq!(entry.title, "A title");
    assert_eq!(entry.authors, ["A, B", "C, D"]);
    assert_eq!(entry.year.as_deref(), Some("2020"));
    assert_eq!(entry.container.as_deref(), Some("Journal"));
    assert_eq!(entry.doi.as_deref(), Some("10.1/x"));
    assert!(bib_entry_from_frontmatter("title: \"no key\"", "t").is_none());
    assert_eq!(
        format_reference_styled(&entry, CitationStyle::Plain),
        format_reference(&entry)
    );
}
const CUSTOM_CSL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="in-text" version="1.0" default-locale="en-US">
  <info>
    <title>Spanseek Test Style</title>
    <id>https://example.test/styles/spanseek-test</id>
    <updated>2026-01-01T00:00:00+00:00</updated>
  </info>
  <citation>
    <layout prefix="[" suffix="]" delimiter="; ">
      <text variable="title"/>
    </layout>
  </citation>
  <bibliography>
    <layout suffix=".">
      <text variable="title"/>
    </layout>
  </bibliography>
</style>"#;

const DEPENDENT_CSL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="in-text" version="1.0" default-locale="en-US">
  <info>
    <title>Spanseek Dependent Style</title>
    <id>https://example.test/styles/spanseek-dependent</id>
    <link rel="independent-parent" href="https://example.test/styles/spanseek-test"/>
    <updated>2026-01-01T00:00:00+00:00</updated>
  </info>
</style>"#;

const NOTE_CSL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="note" version="1.0" default-locale="en-US">
  <info>
    <title>Spanseek Note Test Style</title>
    <id>https://example.test/styles/spanseek-note-test</id>
    <updated>2026-01-01T00:00:00+00:00</updated>
  </info>
  <citation>
    <layout delimiter="; ">
      <group delimiter=" ">
        <text variable="title"/>
        <label variable="locator" form="short"/>
        <text variable="locator"/>
      </group>
    </layout>
  </citation>
  <bibliography>
    <layout suffix=".">
      <text variable="title"/>
    </layout>
  </bibliography>
</style>"#;

const NOTE_IBID_CSL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="note" version="1.0" default-locale="en-US">
  <info>
    <title>Spanseek Ibid Test Style</title>
    <id>https://example.test/styles/spanseek-ibid-test</id>
    <updated>2026-01-01T00:00:00+00:00</updated>
  </info>
  <citation>
    <layout delimiter="; ">
      <choose>
        <if position="first"><text value="first-note"/></if>
        <else-if position="ibid"><text value="ibid-marker"/></else-if>
        <else-if position="subsequent"><text value="subsequent-marker"/></else-if>
        <else><text value="other-marker"/></else>
      </choose>
    </layout>
  </citation>
  <bibliography>
    <layout suffix=".">
      <text variable="title"/>
    </layout>
  </bibliography>
</style>"#;

#[test]
fn custom_and_dependent_csl_styles_persist_and_render_in_text_groups() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "CSL",
            "一组[@vaswani2017; @devlin2019]，再引 [@vaswani2017]。",
        )
        .unwrap();
    let style_dir = tempdir().unwrap();
    let custom_path = style_dir.path().join("custom.csl");
    fs::write(&custom_path, CUSTOM_CSL).unwrap();
    let imported = library.import_citation_style(&custom_path).unwrap();
    assert_eq!(imported.id, "https://example.test/styles/spanseek-test");
    assert_eq!(imported.kind, CitationStyleKind::Independent);
    assert_eq!(library.citation_style_catalog().unwrap().len(), 5);

    let report = library
        .bibliography_with_style(&draft.id, "https://example.test/styles/spanseek-test")
        .unwrap();
    assert_eq!(report.style, CitationStyle::Custom);
    assert_eq!(report.style_id, "https://example.test/styles/spanseek-test");
    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.in_text.len(), 2);
    assert_eq!(report.in_text[0].keys, ["vaswani2017", "devlin2019"]);
    assert!(
        report.in_text[0]
            .rendered
            .contains("Attention is All You Need")
    );
    assert!(report.in_text[0].rendered.contains("BERT"));
    assert!(
        report.in_text[1]
            .rendered
            .contains("Attention is All You Need")
    );
    assert_eq!(report.entries[0].formatted, "Attention is All You Need.");

    let dependent_path = style_dir.path().join("dependent.csl");
    fs::write(&dependent_path, DEPENDENT_CSL).unwrap();
    let dependent = library.import_citation_style(&dependent_path).unwrap();
    assert_eq!(dependent.kind, CitationStyleKind::Dependent);
    let inherited = library
        .bibliography_with_style(&draft.id, &dependent.id)
        .unwrap();
    assert_eq!(inherited.entries[0].formatted, report.entries[0].formatted);
    assert!(matches!(
        library.delete_citation_style(&report.style_id),
        Err(Error::InvalidInput("csl_style_in_use"))
    ));
    drop(library);
    let reopened = Library::open(root.path()).unwrap();
    let reopened_catalog = reopened.citation_style_catalog().unwrap();
    assert!(reopened_catalog.iter().any(|style| {
        style.id == "https://example.test/styles/spanseek-test"
            && style.kind == CitationStyleKind::Independent
    }));
    assert!(reopened_catalog.iter().any(|style| {
        style.id == "https://example.test/styles/spanseek-dependent"
            && style.kind == CitationStyleKind::Dependent
    }));
    let reopened_report = reopened
        .bibliography_with_style(&draft.id, &dependent.id)
        .unwrap();
    assert_eq!(reopened_report.entries.len(), 2);
    assert_eq!(reopened_report.in_text.len(), 2);

    let package = root.path().join("citation-styles.pkmsbackup");
    reopened.create_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    let restored_catalog = restored.citation_style_catalog().unwrap();
    assert!(restored_catalog.iter().any(|style| {
        style.id == "https://example.test/styles/spanseek-dependent"
            && style.kind == CitationStyleKind::Dependent
    }));
    let restored_report = restored
        .bibliography_with_style(&draft.id, &dependent.id)
        .unwrap();
    assert_eq!(restored_report.entries.len(), 2);
    assert_eq!(restored_report.in_text.len(), 2);
    let mut reopened = reopened;
    assert!(matches!(
        reopened.delete_citation_style(&report.style_id),
        Err(Error::InvalidInput("csl_style_in_use"))
    ));
    reopened.delete_citation_style(&dependent.id).unwrap();
    reopened.delete_citation_style(&report.style_id).unwrap();
    assert_eq!(reopened.citation_style_catalog().unwrap().len(), 4);
}

#[test]
fn invalid_csl_is_rejected_without_leaving_a_style_row() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let style_dir = tempdir().unwrap();
    let path = style_dir.path().join("broken.csl");
    fs::write(&path, "<style>broken").unwrap();
    assert!(matches!(
        library.import_citation_style(&path),
        Err(Error::InvalidInput("csl_invalid_style"))
    ));
    assert_eq!(library.citation_style_catalog().unwrap().len(), 4);
}
#[test]
fn numeric_csl_keeps_one_sequence_across_citation_groups() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "编号连续",
            "第一组[@vaswani2017; @devlin2019]，第二组[@vaswani2017]。",
        )
        .unwrap();
    let gbt = library
        .bibliography_styled(&draft.id, CitationStyle::Gbt7714)
        .unwrap();
    assert_eq!(gbt.entries.len(), 2);
    assert!(
        gbt.entries
            .iter()
            .any(|entry| entry.formatted.contains("[1"))
    );
    assert!(
        gbt.entries
            .iter()
            .any(|entry| entry.formatted.contains("[2"))
    );
    assert_eq!(gbt.in_text.len(), 2);
    assert!(gbt.in_text[0].rendered.contains('1'));
    assert!(gbt.in_text[0].rendered.contains('2'));
    assert!(gbt.in_text[1].rendered.contains('1'));
}

#[test]
fn csl_locators_render_and_note_styles_receive_note_numbers() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "Locator coverage",
            "甲[@vaswani2017, pp. 3-4]，乙[@devlin2019, ch. 2]。",
        )
        .unwrap();

    let apa = library
        .bibliography_styled(&draft.id, CitationStyle::Apa)
        .unwrap();
    assert_eq!(apa.in_text.len(), 2);
    assert_eq!(apa.in_text[0].marker, "[@vaswani2017, pp. 3-4]");
    assert_eq!(apa.in_text[1].marker, "[@devlin2019, ch. 2]");
    assert!(
        apa.in_text[0].rendered.contains("3-4") || apa.in_text[0].rendered.contains("3–4"),
        "{}",
        apa.in_text[0].rendered
    );
    assert!(
        apa.in_text[1].rendered.contains("2"),
        "{}",
        apa.in_text[1].rendered
    );
    assert!(
        apa.in_text
            .iter()
            .all(|citation| citation.note_number.is_none())
    );

    let style_dir = tempdir().unwrap();
    let style_path = style_dir.path().join("note.csl");
    fs::write(&style_path, NOTE_CSL).unwrap();
    let imported = library.import_citation_style(&style_path).unwrap();
    let note = library
        .bibliography_with_style(&draft.id, &imported.id)
        .unwrap();
    assert_eq!(note.in_text.len(), 2);
    assert_eq!(note.in_text[0].note_number, Some(1));
    assert_eq!(note.in_text[1].note_number, Some(2));
    assert!(
        note.in_text[0].rendered.contains("3-4") || note.in_text[0].rendered.contains("3–4"),
        "{}",
        note.in_text[0].rendered
    );
    assert!(
        note.in_text[1].rendered.contains("chap.") || note.in_text[1].rendered.contains("Chapter"),
        "{}",
        note.in_text[1].rendered
    );
}

#[test]
fn note_numbers_follow_rendered_groups_only() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "Note number gaps",
            "缺口[@nokey]，一[@vaswani2017]，二[@devlin2019]。",
        )
        .unwrap();
    let style_dir = tempdir().unwrap();
    let style_path = style_dir.path().join("note.csl");
    fs::write(&style_path, NOTE_CSL).unwrap();
    let imported = library.import_citation_style(&style_path).unwrap();
    let note = library
        .bibliography_with_style(&draft.id, &imported.id)
        .unwrap();

    assert_eq!(note.missing, ["nokey"]);
    assert_eq!(note.in_text.len(), 2);
    assert_eq!(note.in_text[0].note_number, Some(1));
    assert_eq!(note.in_text[1].note_number, Some(2));
}

#[test]
fn repeated_identical_markers_keep_each_note_occurrence_in_exports() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note("Repeated marker", "甲[@vaswani2017]，乙[@vaswani2017]。")
        .unwrap();
    let style_dir = tempdir().unwrap();
    let style_path = style_dir.path().join("ibid.csl");
    fs::write(&style_path, NOTE_IBID_CSL).unwrap();
    let imported = library.import_citation_style(&style_path).unwrap();
    let note = library
        .bibliography_with_style(&draft.id, &imported.id)
        .unwrap();

    assert_eq!(note.in_text.len(), 2);
    assert_eq!(note.in_text[0].rendered, "first-note");
    assert_eq!(note.in_text[1].rendered, "ibid-marker");
    assert_eq!(note.in_text[0].note_number, Some(1));
    assert_eq!(note.in_text[1].note_number, Some(2));

    let html_path = root.path().join("ibid.zip");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &html_path,
            ExportFormat::Html,
            Some(imported.id.as_str()),
            &mut |_| Ok(()),
        )
        .unwrap();
    let report = ready.publish().unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&html_path).unwrap()).unwrap();
    let mut html = String::new();
    archive
        .by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert_eq!(html.matches("class=\"pkms-citation\"").count(), 2, "{html}");
    assert!(html.contains(">first-note</a>"), "{html}");
    assert!(html.contains(">ibid-marker</a>"), "{html}");
    assert!(!html.contains("[@vaswani2017]"), "{html}");

    let docx_path = root.path().join("ibid.docx");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &docx_path,
            ExportFormat::Docx,
            Some(imported.id.as_str()),
            &mut |_| Ok(()),
        )
        .unwrap();
    ready.publish().unwrap();
    let mut package = zip::ZipArchive::new(fs::File::open(&docx_path).unwrap()).unwrap();
    let mut document = String::new();
    package
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert_eq!(document.matches("first-note").count(), 1, "{document}");
    assert_eq!(document.matches("ibid-marker").count(), 1, "{document}");
}

#[test]
fn html_export_renders_csl_groups_and_references_for_the_selected_revision() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "导出引用",
            "已知引用[@vaswani2017; @devlin2019]，缺口[@nokey]。",
        )
        .unwrap();
    let archive = root.path().join("cited-html.zip");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &archive,
            ExportFormat::Html,
            Some("apa"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let report = ready.publish().unwrap();
    assert_eq!(report.format, ExportFormat::Html);

    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut html = String::new();
    zip.by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert!(html.contains("class=\"pkms-citation\""), "{html}");
    assert!(
        html.contains("data-citation-keys=\"vaswani2017;devlin2019\""),
        "{html}"
    );
    assert!(!html.contains("[@vaswani2017]"), "{html}");
    assert!(
        html.contains("class=\"pkms-citation pkms-citation-missing\""),
        "{html}"
    );
    assert!(html.contains("[@nokey]"), "{html}");
    assert!(html.contains("role=\"doc-bibliography\""), "{html}");
    assert!(html.contains("data-citation-key=\"vaswani2017\""), "{html}");
    assert!(html.contains("Attention is All You Need"), "{html}");
    assert!(
        html.contains("BERT: Pre-training of Deep Bidirectional Transformers"),
        "{html}"
    );
}

#[test]
fn exports_distinguish_same_key_with_different_locators() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "Locator export",
            "先[@vaswani2017, pp. 3-4]，后[@vaswani2017, p. 9]。",
        )
        .unwrap();

    let html_path = root.path().join("locators.zip");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &html_path,
            ExportFormat::Html,
            Some("apa"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let report = ready.publish().unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&html_path).unwrap()).unwrap();
    let mut html = String::new();
    archive
        .by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert_eq!(html.matches("class=\"pkms-citation\"").count(), 2, "{html}");
    assert!(
        html.contains("3-4") || html.contains("3–4"),
        "first locator missing: {html}"
    );
    assert!(
        html.contains("p. 9") || html.contains("p.&nbsp;9"),
        "{html}"
    );
    assert!(!html.contains("[@vaswani2017"), "{html}");

    let docx_path = root.path().join("locators.docx");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &docx_path,
            ExportFormat::Docx,
            Some("apa"),
            &mut |_| Ok(()),
        )
        .unwrap();
    ready.publish().unwrap();
    let mut package = zip::ZipArchive::new(fs::File::open(&docx_path).unwrap()).unwrap();
    let mut document = String::new();
    package
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert!(
        document.contains("3-4") || document.contains("3–4"),
        "first locator missing: {document}"
    );
    assert!(document.contains("9"), "{document}");
    assert!(!document.contains("[@vaswani2017"), "{document}");
}
#[test]
fn docx_export_renders_csl_in_text_and_bibliography() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note(
            "DOCX 引用",
            "方法[@vaswani2017; @devlin2019]，缺口[@nokey]。",
        )
        .unwrap();
    let path = root.path().join("cited.docx");
    let ready = library
        .prepare_note_export_with_citations(
            &draft.id,
            &draft.revision_id,
            &path,
            ExportFormat::Docx,
            Some("apa"),
            &mut |_| Ok(()),
        )
        .unwrap();
    let report = ready.publish().unwrap();
    assert_eq!(report.format, ExportFormat::Docx);
    assert_eq!(report.document_path, "cited.docx");
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    let mut package = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut document = String::new();
    package
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert!(document.contains("Vaswani"), "{document}");
    assert!(
        document.contains(
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">DOCX 引用</w:t></w:r></w:p>"
        ),
        "the note title must be a real run Word can display: {document}"
    );
    assert!(
        document.contains(
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">参考文献 / References</w:t></w:r></w:p>"
        ),
        "the bibliography heading must be a real run Word can display: {document}"
    );
    assert!(document.contains("Attention is All You Need"), "{document}");
    assert!(document.contains("[@nokey]"), "{document}");
    assert!(!document.contains("&lt;script&gt;"), "{document}");
}

#[test]
fn pdf_export_renders_a_self_contained_document_when_edge_is_available() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let bib_dir = tempdir().unwrap();
    let bib = bib_dir.path().join("refs.bib");
    fs::write(&bib, BIB).unwrap();
    library.import_bibliography(&bib).unwrap();
    let draft = library
        .create_note("PDF 引用", "方法[@vaswani2017] 与缺口[@nokey]。")
        .unwrap();
    let path = root.path().join("cited.pdf");
    let ready = match library.prepare_note_export_with_citations(
        &draft.id,
        &draft.revision_id,
        &path,
        ExportFormat::Pdf,
        Some("apa"),
        &mut |_| Ok(()),
    ) {
        Ok(ready) => ready,
        Err(Error::InvalidInput("pdf_renderer_unavailable")) => return,
        Err(error) => panic!("PDF export failed: {error:?}"),
    };
    let report = ready.publish().unwrap();
    assert_eq!(report.format, ExportFormat::Pdf);
    assert_eq!(report.document_path, "cited.pdf");
    let bytes = fs::read(path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.len() > 10_000);
}
