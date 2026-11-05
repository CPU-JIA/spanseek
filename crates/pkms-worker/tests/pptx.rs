//! PPTX extraction: slide order, rich text, tables, images, chart caches,
//! SmartArt data and speaker notes.
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
        input_path: root.join("input.pptx"),
        format: SourceFormat::Pptx,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

fn package(files: &[(&str, &str)]) -> Vec<u8> {
    let files = files
        .iter()
        .map(|(name, body)| (*name, body.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    package_bytes(&files)
}

fn package_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(body).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

const PRESENTATION: &str = r#"<?xml version="1.0"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst>
</p:presentation>"#;

const RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"#;

const SLIDE1: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody>
    <a:p><a:r><a:t>标题一</a:t></a:r></a:p>
    <a:p><a:r><a:t>要点 A</a:t></a:r><a:r><a:t xml:space="preserve"> &amp; B</a:t></a:r></a:p>
  </p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#;

const SLIDE2: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>第二页</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#;

const CORE: &str = r#"<?xml version="1.0"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>季度汇报</dc:title><dc:creator>张三</dc:creator>
</cp:coreProperties>"#;

const DEEP_PRESENTATION: &str = r#"<?xml version="1.0"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
</p:presentation>"#;

const DEEP_RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

const DEEP_SLIDE_RELS: &str = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId10" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
  <Relationship Id="rId11" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
  <Relationship Id="rId12" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="../diagrams/data1.xml"/>
  <Relationship Id="rId13" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
</Relationships>"#;

const DEEP_SLIDE: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld><p:spTree>
    <p:sp><p:txBody><a:p><a:r><a:t>数据概览</a:t></a:r></a:p></p:txBody></p:sp>
    <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
      <a:tr><a:tc><a:txBody><a:p><a:r><a:t>指标</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>值</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
      <a:tr><a:tc><a:txBody><a:p><a:r><a:t>收入</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>42</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
    </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
    <p:pic><p:nvPicPr><p:cNvPr id="2" name="发布现场"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId10"/></p:blipFill></p:pic>
  </p:spTree></p:cSld>
</p:sld>"#;

const DEEP_CHART: &str = r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>营收趋势</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea><c:barChart><c:ser>
      <c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:pt idx="0"><c:v>收入</c:v></c:pt></c:strCache></c:strRef></c:tx>
      <c:cat><c:strRef><c:f>Sheet1!$A$2:$A$3</c:f><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>
      <c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>12</c:v></c:pt></c:numCache></c:numRef></c:val>
    </c:ser></c:barChart></c:plotArea>
  </c:chart>
</c:chartSpace>"#;

const DEEP_DIAGRAM: &str = r#"<?xml version="1.0"?>
<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <dgm:ptLst><dgm:pt modelId="1"><dgm:t><a:bodyPr/><a:p><a:r><a:t>规划</a:t></a:r></a:p><a:p><a:r><a:t>执行</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst>
</dgm:dataModel>"#;

const DEEP_NOTES: &str = r#"<?xml version="1.0"?>
<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree>
    <p:sp><p:nvSpPr><p:nvPr><p:ph type="sldNum" idx="10"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>7</a:t></a:r></a:p></p:txBody></p:sp>
    <p:sp><p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>讲稿第一段</a:t></a:r></a:p></p:txBody></p:sp>
    <p:sp><p:nvSpPr><p:nvPr><p:ph type="ftr" idx="11"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>机密页脚</a:t></a:r></a:p></p:txBody></p:sp>
  </p:spTree></p:cSld>
</p:notes>"#;

#[test]
fn slides_keep_presentation_order_and_concatenate_runs() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(
        &request.input_path,
        package(&[
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", RELS),
            ("ppt/slides/slide1.xml", SLIDE1),
            ("ppt/slides/slide2.xml", SLIDE2),
            ("docProps/core.xml", CORE),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.parser_version, "pptx-v2");
    assert_eq!(parsed.title.as_deref(), Some("季度汇报"));
    assert_eq!(parsed.author.as_deref(), Some("张三"));
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["Slide: 1", "标题一", "要点 A & B", "Slide: 2", "第二页"]
    );
    assert_eq!(parsed.page_count, Some(2));
    assert_eq!(
        parsed
            .toc
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>(),
        ["标题一", "第二页"]
    );
    assert!(!parsed.needs_ocr);
}

#[test]
fn deep_presentation_content_is_reconstructed_without_placeholder_chrome() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let png = b"\x89PNG\r\n\x1a\nspanseek-pptx-image".to_vec();
    std::fs::write(
        &request.input_path,
        package_bytes(&[
            (
                "ppt/presentation.xml",
                DEEP_PRESENTATION.as_bytes().to_vec(),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                DEEP_RELS.as_bytes().to_vec(),
            ),
            ("ppt/slides/slide1.xml", DEEP_SLIDE.as_bytes().to_vec()),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                DEEP_SLIDE_RELS.as_bytes().to_vec(),
            ),
            ("ppt/charts/chart1.xml", DEEP_CHART.as_bytes().to_vec()),
            ("ppt/diagrams/data1.xml", DEEP_DIAGRAM.as_bytes().to_vec()),
            (
                "ppt/notesSlides/notesSlide1.xml",
                DEEP_NOTES.as_bytes().to_vec(),
            ),
            ("ppt/media/image1.png", png),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.parser_version, "pptx-v2");
    assert_eq!(parsed.page_count, Some(1));
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Slide: 1",
            "数据概览",
            "Table: 2 x 2",
            "指标 | 值",
            "收入 | 42",
            "发布现场",
            "Chart: 营收趋势",
            "Categories: Q1 | Q2",
            "Series: 收入 = 10 | 12",
            "SmartArt:",
            "规划",
            "执行",
            "Notes:",
            "讲稿第一段",
        ]
    );
    assert_eq!(parsed.toc.len(), 1);
    assert_eq!(parsed.toc[0].title, "数据概览");
    assert_eq!(parsed.toc[0].block_position, Some(0));
    assert_eq!(parsed.images.len(), 1);
    assert_eq!(parsed.images[0].media_type, "image/png");
    assert_eq!(parsed.images[0].alt, "发布现场");
    assert_eq!(
        parsed.blocks[parsed.images[0].block_position as usize].text,
        "发布现场"
    );
    assert!(!parsed.blocks.iter().any(|block| block.text == "7"));
    assert!(!parsed.blocks.iter().any(|block| block.text == "机密页脚"));
}

#[test]
fn broken_presentations_and_empty_slides_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input_path, b"not a zip container").unwrap();
    assert_eq!(failure_code(&request), "pptx_not_container");

    std::fs::write(
        &request.input_path,
        package(&[("ppt/slides/slide1.xml", SLIDE1)]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "pptx_part_missing");

    let empty = r#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst/></p:presentation>"#;
    std::fs::write(
        &request.input_path,
        package(&[
            ("ppt/presentation.xml", empty),
            ("ppt/_rels/presentation.xml.rels", RELS),
        ]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "pptx_no_content");
}
