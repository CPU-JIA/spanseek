//! DOCX extraction: a real OOXML container in, plain text blocks out.
use pkms_core::SourceFormat;
use pkms_worker::{WorkerRequest, WorkerResult, execute, read_validated_result};
use std::io::Write;

fn request(root: &std::path::Path) -> WorkerRequest {
    WorkerRequest {
        protocol_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        attempt: 1,
        lease_token: uuid::Uuid::new_v4().to_string(),
        source_version_id: uuid::Uuid::new_v4().to_string(),
        input_path: root.join("input.docx"),
        format: SourceFormat::Docx,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

/// Builds a real container with deflated entries, exactly like a Word file.
fn docx(body: &str, core: Option<&str>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("[Content_Types].xml", options).unwrap();
    writer.write_all(b"<Types/>").unwrap();
    writer.start_file("word/document.xml", options).unwrap();
    writer.write_all(body.as_bytes()).unwrap();
    if let Some(core) = core {
        writer.start_file("docProps/core.xml", options).unwrap();
        writer.write_all(core.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

#[test]
fn paragraphs_table_cells_and_breaks_become_text_blocks_with_the_core_title() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let body = concat!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        r#"<w:p><w:r><w:t>第一段正文</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">A &amp; B </w:t></w:r><w:r><w:tab/><w:t>tabbed</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>line one</w:t><w:br/><w:t>line two</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>单元格甲</w:t></w:r></w:p></w:tc>"#,
        r#"<w:tc><w:p><w:r><w:t>单元格乙</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:delText>deleted</w:delText></w:r>"#,
        r#"<w:r><w:t>Kept</w:t></w:r></w:p><w:p/></w:body></w:document>"#,
    );
    let core = concat!(
        r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" "#,
        r#"xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>合成 DOCX 标题</dc:title>"#,
        r#"<dc:creator>张三 &amp; 李四</dc:creator></cp:coreProperties>"#,
    );
    std::fs::write(&request.input_path, docx(body, Some(core))).unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("合成 DOCX 标题"));
    assert_eq!(parsed.author.as_deref(), Some("张三 & 李四"));
    assert_eq!(parsed.parser_version, "ooxml-wordprocessing-v1");
    assert!(parsed.page_count.is_none());
    assert!(!parsed.needs_ocr);
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        [
            "第一段正文",
            "A & B tabbed",
            "line one",
            "line two",
            "单元格甲",
            "单元格乙",
            "Kept"
        ]
    );
    assert!(
        parsed
            .blocks
            .iter()
            .all(|block| block.page_index.is_none() && block.pdf_geometry.is_none())
    );
}

#[test]
fn a_container_without_core_properties_has_no_title() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let body = concat!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        r#"<w:body><w:p><w:r><w:t>Only body text</w:t></w:r></w:p></w:body></w:document>"#,
    );
    std::fs::write(&request.input_path, docx(body, None)).unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title, None);
    assert_eq!(parsed.author, None);
    assert_eq!(parsed.blocks[0].text, "Only body text");
}

#[test]
fn missing_body_broken_container_and_empty_text_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("word/styles.xml", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"<styles/>").unwrap();
    std::fs::write(&request.input_path, writer.finish().unwrap().into_inner()).unwrap();
    assert_eq!(failure_code(&request), "docx_part_missing");

    std::fs::write(&request.input_path, b"not a zip container").unwrap();
    assert_eq!(failure_code(&request), "docx_not_container");

    let body = concat!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        r#"<w:body><w:p><w:r><w:t>   </w:t></w:r></w:p></w:body></w:document>"#,
    );
    std::fs::write(&request.input_path, docx(body, None)).unwrap();
    assert_eq!(failure_code(&request), "docx_no_content");
}
