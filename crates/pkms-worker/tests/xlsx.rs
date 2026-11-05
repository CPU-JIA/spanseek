//! XLSX extraction: shared strings, inline strings, sheet order and core metadata.
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
        input_path: root.join("input.xlsx"),
        format: SourceFormat::Xlsx,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

fn package(files: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

const WORKBOOK: &str = r#"<?xml version="1.0"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="明细" sheetId="1" r:id="rId1"/><sheet name="汇总" sheetId="2" r:id="rId2"/></sheets>
</workbook>"#;

const RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;

const SHARED: &str = r#"<?xml version="1.0"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">
  <si><t>名称</t></si><si><t>数量</t></si><si><t>甲</t></si><si><t>乙</t></si>
</sst>"#;

const STYLES: &str = r#"<?xml version="1.0"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="10"/></cellXfs>
</styleSheet>"#;

const SHEET1: &str = r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
  <row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><v>3</v></c></row>
  <row r="3"><c r="A3" t="s"><v>3</v></c></row>
  <row r="4"><c r="A4" s="1"><v>45292</v></c><c r="B4" s="2"><v>0.25</v></c><c r="C4"><f>SUM(B2:B4)</f><v>3.25</v></c></row>
  <row r="5"><c r="A5" t="inlineStr"><is><t>合并标题</t></is></c></row>
  <row r="6"><c r="A6"><f>1+1</f></c></row>
</sheetData><mergeCells count="1"><mergeCell ref="A5:B5"/></mergeCells></worksheet>"#;

const SHEET2: &str = r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>合计</t></is></c><c r="B1"><v>42</v></c></row>
</sheetData></worksheet>"#;

const CORE: &str = r#"<?xml version="1.0"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>销售台账</dc:title><dc:creator>张三</dc:creator>
</cp:coreProperties>"#;

#[test]
fn sheets_keep_their_order_and_render_shared_inline_and_numeric_cells() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(
        &request.input_path,
        package(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", RELS),
            ("xl/sharedStrings.xml", SHARED),
            ("xl/styles.xml", STYLES),
            ("xl/worksheets/sheet1.xml", SHEET1),
            ("xl/worksheets/sheet2.xml", SHEET2),
            ("docProps/core.xml", CORE),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.parser_version, "xlsx-v2");
    assert_eq!(parsed.title.as_deref(), Some("销售台账"));
    assert_eq!(parsed.author.as_deref(), Some("张三"));
    assert!(parsed.page_count.is_none());
    assert!(!parsed.needs_ocr);
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Sheet: 明细",
            "名称 | 数量",
            "甲 | 3",
            "乙",
            "2024-01-01 | 25% | 3.25",
            "合并标题",
            "=1+1",
            "Merged: A5:B5",
            "Formulas: C4 = SUM(B2:B4), A6 = 1+1",
            "Sheet: 汇总",
            "合计 | 42"
        ]
    );
}

#[test]
fn broken_containers_missing_workbooks_and_empty_sheets_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input_path, b"not a zip container").unwrap();
    assert_eq!(failure_code(&request), "xlsx_not_container");

    std::fs::write(
        &request.input_path,
        package(&[("xl/sharedStrings.xml", SHARED)]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "xlsx_part_missing");

    let empty = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/></workbook>"#;
    std::fs::write(
        &request.input_path,
        package(&[
            ("xl/workbook.xml", empty),
            ("xl/_rels/workbook.xml.rels", RELS),
        ]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "xlsx_no_content");
}
