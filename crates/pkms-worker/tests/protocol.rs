use pkms_core::SourceFormat;
use pkms_worker::*;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn request(dir: &std::path::Path, format: SourceFormat) -> WorkerRequest {
    WorkerRequest {
        protocol_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        attempt: 1,
        lease_token: uuid::Uuid::new_v4().to_string(),
        source_version_id: uuid::Uuid::new_v4().to_string(),
        input_path: dir.join("input"),
        format,
        output_dir: dir.to_path_buf(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}
fn child(r: &WorkerRequest) -> WorkerResponse {
    let mut process = Command::new(env!("CARGO_BIN_EXE_pkms-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(r).unwrap())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.len() as u64 <= MAX_RESPONSE_BYTES);
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn real_process_unicode_and_tamper_checks() {
    let dir = tempfile::tempdir().unwrap();
    let r = request(dir.path(), SourceFormat::Markdown);
    fs::write(&r.input_path, "# 中文\r\n\r\n重复😀 e\u{301}\n").unwrap();
    let response = child(&r);
    let parsed = read_validated_result(&r, &response).unwrap();
    assert_eq!(parsed.blocks[1].text, "重复😀 e\u{301}");
    let mut stale = response.clone();
    stale.attempt += 1;
    assert_eq!(
        read_validated_result(&r, &stale).unwrap_err().code,
        "worker_identity_mismatch"
    );
    fs::write(dir.path().join(RESULT_FILE), "{}").unwrap();
    assert_eq!(
        read_validated_result(&r, &response).unwrap_err().code,
        "result_hash_mismatch"
    );
}
#[test]
fn invalid_input_protocol_and_existing_output_fail() {
    let dir = tempfile::tempdir().unwrap();
    let r = request(dir.path(), SourceFormat::Text);
    fs::write(&r.input_path, [0xff, 0xfe]).unwrap();
    assert!(matches!(child(&r).result,WorkerResult::Failure{code} if code=="invalid_utf8"));
    fs::write(&r.input_path, "hello").unwrap();
    fs::write(dir.path().join(RESULT_FILE), "keep").unwrap();
    assert!(matches!(child(&r).result,WorkerResult::Failure{code} if code=="output_unavailable"));
    assert_eq!(
        fs::read_to_string(dir.path().join(RESULT_FILE)).unwrap(),
        "keep"
    );
    assert_eq!(
        read_request(vec![b' '; MAX_REQUEST_BYTES as usize + 1].as_slice())
            .unwrap_err()
            .code,
        "size_limit"
    );
    let mut future = r;
    future.protocol_version = 2;
    assert_eq!(
        read_request(serde_json::to_vec(&future).unwrap().as_slice())
            .unwrap_err()
            .code,
        "invalid_request"
    );
}

#[test]
fn input_limits_and_engine_identity_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(dir.path(), SourceFormat::Pdf);
    let file = fs::File::create(&r.input_path).unwrap();
    file.set_len(MAX_INPUT_BYTES + 1).unwrap();
    assert!(matches!(child(&r).result, WorkerResult::Failure { code } if code == "input_limit"));
    drop(file);
    fs::write(&r.input_path, b"%PDF-1.7").unwrap();
    assert!(
        matches!(child(&r).result, WorkerResult::Failure { code } if code == "pdfium_unavailable")
    );
    let dll = dir.path().join("fake.dll");
    fs::write(&dll, b"not the approved binary").unwrap();
    r.pdfium_library_path = Some(dll);
    assert!(
        matches!(child(&r).result, WorkerResult::Failure { code } if code == "pdfium_hash_mismatch")
    );
    assert!(!dir.path().join(RESULT_FILE).exists());
}

// Self-authored PDF bytes with a genuine xref table, font, nonzero crop origin and page rotation.
fn pdf(text: bool, pages: usize) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Count {pages} /Kids [{}] >>",
            (0..pages)
                .map(|i| format!("{} 0 R", 5 + i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let stream = if text {
        "BT /F1 12 Tf 40 100 Td (Repeat Repeat) Tj ET"
    } else {
        "10 10 50 50 re f"
    };
    objects.push(format!(
        "<< /Length {} >>\nstream\n{stream}\nendstream",
        stream.len()
    ));
    for _ in 0..pages {
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /CropBox [10 20 290 180] /Rotate 90 /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".into());
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}
#[test]
#[ignore = "requires pinned native PDFium; set PKMS_TEST_PDFIUM"]
fn native_pdf_text_geometry_scan_corrupt_and_page_limit() {
    let dll = std::env::var_os("PKMS_TEST_PDFIUM").expect("set PKMS_TEST_PDFIUM");
    for (data, expected) in [
        (pdf(true, 1), "text"),
        (pdf(false, 1), "scan"),
        (b"%PDF-corrupt".to_vec(), "pdf_invalid"),
        (pdf(true, MAX_PAGES + 1), "pdf_page_limit"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(dir.path(), SourceFormat::Pdf);
        r.pdfium_library_path = Some(dll.clone().into());
        fs::write(&r.input_path, data).unwrap();
        let response = child(&r);
        if expected == "text" || expected == "scan" {
            let parsed = read_validated_result(&r, &response).unwrap();
            assert_eq!(parsed.page_count, Some(1));
            assert_eq!(parsed.needs_ocr, expected == "scan");
            if expected == "text" {
                let block = &parsed.blocks[0];
                assert_eq!(block.text, "Repeat Repeat");
                let g = block.pdf_geometry.as_ref().unwrap();
                assert_eq!(g.rotation, 90);
                assert_eq!(g.crop_box, [10., 20., 290., 180.]);
                assert_eq!(g.chars.len(), block.text.chars().count());
                assert!(g.chars[0].rect.unwrap()[0] >= 40.);
            }
        } else {
            assert!(
                matches!(response.result,WorkerResult::Failure{ref code} if code==expected),
                "{response:?}"
            );
        }
    }
}
