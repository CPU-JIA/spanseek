//! EPUB extraction: a real package container in, spine-ordered text blocks out.
use base64::{Engine as _, engine::general_purpose::STANDARD};
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
        input_path: root.join("input.epub"),
        format: SourceFormat::Epub,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

fn package_raw(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("mimetype", options).unwrap();
    writer.write_all(b"application/epub+zip").unwrap();
    for (name, body) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(body).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn package(files: &[(&str, &str)]) -> Vec<u8> {
    let owned = files
        .iter()
        .map(|(name, body)| (*name, body.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    package_raw(&owned)
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

const OPF: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>合成 EPUB</dc:title>
    <dc:creator>张三</dc:creator>
  </metadata>
  <manifest>
    <item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/c2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c2"/><itemref idref="c1"/></spine>
</package>"#;

const C1: &str = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>忽略的标题</title>
<style>p { color: red }</style><script>alert(1)</script></head>
<body><h1>第一章</h1><p>正文一 &amp; 更多</p><ul><li>项目甲</li></ul></body></html>"#;

const C2: &str = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Two</title></head>
<body><h2>Chapter Two</h2><p>Body two</p></body></html>"#;

#[test]
fn spine_order_wins_and_head_script_style_never_reach_the_reader() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(
        &request.input_path,
        package(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/text/c1.xhtml", C1),
            ("OEBPS/text/c2.xhtml", C2),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("合成 EPUB"));
    assert_eq!(parsed.author.as_deref(), Some("张三"));
    assert_eq!(parsed.parser_version, "epub-v2");
    assert!(parsed.page_count.is_none());
    assert!(!parsed.needs_ocr);
    assert!(parsed.cover.is_none());
    assert_eq!(
        parsed
            .toc
            .iter()
            .map(|item| (item.title.as_str(), item.block_position))
            .collect::<Vec<_>>(),
        [("Chapter Two", Some(0)), ("第一章", Some(2))]
    );
    for item in &parsed.toc {
        assert_eq!(
            parsed.blocks[item.block_position.unwrap() as usize].id,
            item.block_id.clone().unwrap()
        );
    }
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Chapter Two",
            "Body two",
            "第一章",
            "正文一 & 更多",
            "项目甲"
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
fn broken_containers_empty_spines_and_whitespace_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());

    std::fs::write(&request.input_path, b"not a zip container").unwrap();
    assert_eq!(failure_code(&request), "epub_not_container");

    std::fs::write(&request.input_path, package(&[("OEBPS/content.opf", OPF)])).unwrap();
    assert_eq!(failure_code(&request), "epub_part_missing");

    let empty_spine = OPF.replace(
        r#"<spine><itemref idref="c2"/><itemref idref="c1"/></spine>"#,
        "<spine/>",
    );
    std::fs::write(
        &request.input_path,
        package(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &empty_spine),
        ]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "epub_no_content");

    std::fs::write(
        &request.input_path,
        package(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            (
                "OEBPS/text/c1.xhtml",
                "<html><body><p>   </p></body></html>",
            ),
            ("OEBPS/text/c2.xhtml", "<html><body><p> </p></body></html>"),
        ]),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "epub_no_content");
}

#[test]
fn epub_nav_toc_and_cover_are_mapped_to_stable_blocks() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>带目录的书</dc:title></metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/c2.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover" href="images/cover.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine><itemref idref="c2"/><itemref idref="c1"/></spine>
</package>"#;
    let nav = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol>
  <li><a href="text/c2.xhtml">Two</a></li>
  <li><a href="text/c1.xhtml#start">One</a></li>
</ol></nav></body></html>"#;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(
        &request.input_path,
        package_raw(&[
            ("META-INF/container.xml", CONTAINER.as_bytes().to_vec()),
            ("OEBPS/content.opf", opf.as_bytes().to_vec()),
            ("OEBPS/nav.xhtml", nav.as_bytes().to_vec()),
            ("OEBPS/text/c1.xhtml", C1.as_bytes().to_vec()),
            ("OEBPS/text/c2.xhtml", C2.as_bytes().to_vec()),
            ("OEBPS/images/cover.png", png.into_inner()),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(
        parsed
            .toc
            .iter()
            .map(|item| (item.title.as_str(), item.level, item.block_position))
            .collect::<Vec<_>>(),
        [("Two", 1, Some(0)), ("One", 1, Some(2))]
    );
    assert_eq!(
        parsed.toc[0].block_id.as_deref(),
        Some(parsed.blocks[0].id.as_str())
    );
    assert_eq!(
        parsed.toc[1].block_id.as_deref(),
        Some(parsed.blocks[2].id.as_str())
    );
    let cover = parsed.cover.expect("cover should be extracted");
    assert_eq!(cover.media_type, "image/png");
    assert!(
        STANDARD
            .decode(cover.data_base64)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
}
#[test]
fn inline_images_become_stable_image_blocks_with_alt_text() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>图片书</dc:title></metadata>
  <manifest>
    <item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
    <item id="figure" href="images/body.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
    let chapter = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <p>图前</p><p><img src="../images/body.png" alt=" 图 1 "/></p><p>图后</p>
</body></html>"#;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(
        &request.input_path,
        package_raw(&[
            ("META-INF/container.xml", CONTAINER.as_bytes().to_vec()),
            ("OEBPS/content.opf", opf.as_bytes().to_vec()),
            ("OEBPS/text/c1.xhtml", chapter.as_bytes().to_vec()),
            ("OEBPS/images/body.png", png.into_inner()),
        ]),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["图前", "图 1", "图后"]
    );
    assert_eq!(parsed.images.len(), 1);
    let image = &parsed.images[0];
    assert_eq!(image.block_position, 1);
    assert_eq!(image.media_type, "image/png");
    assert_eq!(image.alt, "图 1");
    assert!(
        STANDARD
            .decode(&image.data_base64)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
}
