//! MOBI extraction: a self-built PDB/PalmDOC container keeps the test free of
//! third-party ebook fixtures while exercising the real worker protocol.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pkms_core::SourceFormat;
use pkms_worker::{WorkerRequest, WorkerResult, execute, read_validated_result};

fn request(root: &std::path::Path) -> WorkerRequest {
    WorkerRequest {
        protocol_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        attempt: 1,
        lease_token: uuid::Uuid::new_v4().to_string(),
        source_version_id: uuid::Uuid::new_v4().to_string(),
        input_path: root.join("input.mobi"),
        format: SourceFormat::Mobi,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

fn put16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
fn exth_record(kind: u32, value: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(8 + value.len());
    record.extend_from_slice(&kind.to_be_bytes());
    record.extend_from_slice(&(8 + value.len() as u32).to_be_bytes());
    record.extend_from_slice(value);
    record
}
fn exth(title: &str, author: &str, cover_offset: Option<u32>) -> Vec<u8> {
    let mut records = vec![
        exth_record(100, author.as_bytes()),
        exth_record(503, title.as_bytes()),
    ];
    if let Some(offset) = cover_offset {
        records.push(exth_record(201, &offset.to_be_bytes()));
    }
    let length = 12 + records.iter().map(Vec::len).sum::<usize>() as u32;
    let mut body = Vec::with_capacity(length as usize);
    body.extend_from_slice(b"EXTH");
    body.extend_from_slice(&length.to_be_bytes());
    body.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for record in records {
        body.extend_from_slice(&record);
    }
    body
}
const DEFAULT_HTML: &str = concat!(
    "<html><head><title>HTML 标题</title><meta name=\"author\" content=\"HTML 作者\"/></head><body>",
    "<h1>第一章</h1><p>合成 MOBI 正文 &amp; 更多</p><p>Second paragraph.</p>",
    "</body></html>"
);

fn mobi_bytes(title: &str, author: &str, encrypted: bool) -> Vec<u8> {
    mobi_bytes_with(title, author, encrypted, DEFAULT_HTML, &[], None)
}

fn mobi_bytes_with(
    title: &str,
    author: &str,
    encrypted: bool,
    html: &str,
    images: &[&[u8]],
    cover_offset: Option<u32>,
) -> Vec<u8> {
    let exth = exth(title, author, cover_offset);
    let name_offset = 16 + 264 + exth.len();
    let mut record0 = vec![0u8; 4096];
    put16(&mut record0, 0, 1); // no compression
    put16(&mut record0, 2, 0);
    put32(&mut record0, 4, html.len() as u32);
    put16(&mut record0, 8, 1);
    put16(&mut record0, 10, 4096);
    put16(&mut record0, 12, u16::from(encrypted)); // PalmDOC encryption field
    let mobi = 16;
    record0[mobi..mobi + 4].copy_from_slice(b"MOBI");
    put32(&mut record0, mobi + 4, 264);
    put32(&mut record0, mobi + 8, 2);
    put32(&mut record0, mobi + 12, 65001);
    put32(&mut record0, mobi + 16, 7);
    put32(&mut record0, mobi + 20, 6);
    for offset in [mobi + 24, mobi + 28, mobi + 32, mobi + 36] {
        put32(&mut record0, offset, 0xffff_ffff);
    }
    for offset in (mobi + 40..mobi + 64).step_by(4) {
        put32(&mut record0, offset, 0xffff_ffff);
    }
    put32(&mut record0, mobi + 64, 2); // first non-book record
    put32(&mut record0, mobi + 68, name_offset as u32);
    put32(&mut record0, mobi + 72, title.len() as u32);
    put32(&mut record0, mobi + 76, 9); // English UTF-8 locale
    put32(&mut record0, mobi + 88, 6);
    put32(
        &mut record0,
        mobi + 92,
        if images.is_empty() { 0xffff_ffff } else { 2 },
    ); // first image record
    put32(&mut record0, mobi + 96, 0);
    put32(&mut record0, mobi + 100, 0);
    put32(&mut record0, mobi + 112, 0x50);
    put32(&mut record0, mobi + 148, 0xffff_ffff);
    put32(&mut record0, mobi + 152, 0xffff_ffff);
    put32(&mut record0, mobi + 176, (1 << 16) | 1);
    put32(&mut record0, mobi + 180, 1);
    put32(&mut record0, mobi + 184, 0xffff_ffff);
    put32(&mut record0, mobi + 188, 0);
    put32(&mut record0, mobi + 192, 0xffff_ffff);
    put32(&mut record0, mobi + 196, 0);
    put32(&mut record0, mobi + 224, 0);
    put32(&mut record0, mobi + 228, 0xffff_ffff);
    put32(&mut record0, mobi + 232, 0xffff_ffff);
    put32(&mut record0, mobi + 236, 0xffff_ffff);
    put32(&mut record0, mobi + 240, 0xffff_ffff);
    record0.splice(16 + 264..16 + 264, exth);
    let name_start = 16 + 264 + (name_offset - (16 + 264));
    record0.splice(name_start..name_start, title.as_bytes().iter().copied());

    let num_records = 2 + images.len();
    let header_size = 78 + num_records * 8 + 2;
    let mut file = vec![0u8; header_size];
    let name = title.as_bytes();
    file[..name.len().min(31)].copy_from_slice(&name[..name.len().min(31)]);
    file[60..64].copy_from_slice(b"BOOK");
    file[64..68].copy_from_slice(b"MOBI");
    put32(&mut file, 68, 1);
    put16(&mut file, 76, num_records as u16);
    let mut offset = header_size + record0.len();
    let mut offsets = vec![header_size];
    offsets.push(offset);
    offset += html.len();
    for image in images {
        offsets.push(offset);
        offset += image.len();
    }
    for (index, offset) in offsets.into_iter().enumerate() {
        put32(&mut file, 78 + index * 8, offset as u32);
        put32(&mut file, 82 + index * 8, index as u32 + 1);
    }
    file.extend_from_slice(&record0);
    file.extend_from_slice(html.as_bytes());
    for image in images {
        file.extend_from_slice(image);
    }
    file
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

#[test]
fn uncompressed_mobi_metadata_and_blocks_are_read() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(
        &request.input_path,
        mobi_bytes("MOBI 合成书", "跨索自测", false),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("MOBI 合成书"));
    assert_eq!(parsed.author.as_deref(), Some("跨索自测"));
    assert_eq!(parsed.parser_version, "mobi-v3");
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["第一章", "合成 MOBI 正文 & 更多", "Second paragraph."]
    );
    assert!(parsed.page_count.is_none());
    assert!(!parsed.needs_ocr);
    assert!(parsed.cover.is_none());
    assert!(parsed.images.is_empty());
    assert_eq!(
        parsed
            .toc
            .iter()
            .map(|item| (item.title.as_str(), item.level, item.block_position))
            .collect::<Vec<_>>(),
        [("第一章", 1, Some(0))]
    );
}

#[test]
fn recindex_images_and_exth_cover_keep_their_block_positions() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let cover_image = b"\xff\xd8\xffcover-image";
    let body = b"\x89PNG\r\n\x1a\nbody-image";
    let html = concat!(
        "<html><head><title>图片书</title></head><body>",
        "<p>图片之前<img recindex=\"00002\" alt=\"正文插图\"/>图片之后</p>",
        "<p>末尾</p></body></html>"
    );
    std::fs::write(
        &request.input_path,
        mobi_bytes_with(
            "图片 MOBI",
            "跨索自测",
            false,
            html,
            &[cover_image, body],
            Some(0),
        ),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["图片之前 正文插图 图片之后", "末尾"]
    );
    let cover = parsed.cover.unwrap();
    assert_eq!(cover.media_type, "image/jpeg");
    assert_eq!(STANDARD.decode(cover.data_base64).unwrap(), cover_image);
    assert_eq!(parsed.images.len(), 1);
    assert_eq!(parsed.images[0].block_position, 0);
    assert_eq!(parsed.images[0].media_type, "image/png");
    assert_eq!(parsed.images[0].alt, "正文插图");
    assert_eq!(
        STANDARD.decode(&parsed.images[0].data_base64).unwrap(),
        body
    );
}

#[test]
fn kindle_embed_and_data_uri_images_are_supported() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let body = b"\x89PNG\r\n\x1a\nbody-image";
    let encoded = STANDARD.encode(body);
    let html = format!(
        concat!(
            "<html><head><title>嵌入式图片</title></head><body>",
            "<p><img src=\"kindle:embed:0001?mime=image/png\" alt=\"记录图片\"/></p>",
            "<p><img src=\"data:image/png;base64,{}\" alt=\"数据图片\"/></p>",
            "</body></html>"
        ),
        encoded
    );
    std::fs::write(
        &request.input_path,
        mobi_bytes_with("嵌入式图片", "跨索自测", false, &html, &[body], None),
    )
    .unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["记录图片", "数据图片"]
    );
    assert_eq!(parsed.images.len(), 2);
    assert_eq!(parsed.images[0].block_position, 0);
    assert_eq!(parsed.images[0].media_type, "image/png");
    assert_eq!(parsed.images[0].alt, "记录图片");
    assert_eq!(parsed.images[1].block_position, 1);
    assert_eq!(parsed.images[1].media_type, "image/png");
    assert_eq!(parsed.images[1].alt, "数据图片");
    for image in parsed.images {
        assert_eq!(STANDARD.decode(image.data_base64).unwrap(), body);
    }
}

#[test]
fn oversized_mobi_image_sets_fail_with_an_asset_limit() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let mut first = vec![0u8; 8 * 1024 * 1024 + 1];
    first[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut second = vec![0u8; 8 * 1024 * 1024 + 1];
    second[..3].copy_from_slice(b"\xff\xd8\xff");
    let html = concat!(
        "<html><head><title>大图册</title></head><body>",
        "<p><img recindex=\"00001\" alt=\"第一图\"/></p>",
        "<p><img recindex=\"00002\" alt=\"第二图\"/></p>",
        "</body></html>"
    );
    std::fs::write(
        &request.input_path,
        mobi_bytes_with("大图册", "跨索自测", false, html, &[&first, &second], None),
    )
    .unwrap();
    assert_eq!(failure_code(&request), "mobi_asset_limit");
}

#[test]
fn encrypted_and_corrupt_mobi_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input_path, mobi_bytes("Encrypted", "Author", true)).unwrap();
    assert_eq!(failure_code(&request), "mobi_encrypted");
    std::fs::write(&request.input_path, b"not a mobi container").unwrap();
    assert_eq!(failure_code(&request), "mobi_invalid_container");
}
