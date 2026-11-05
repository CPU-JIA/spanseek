//! CSV extraction: quoting, empty-row skipping and whitespace normalization.
use pkms_core::SourceFormat;
use pkms_worker::{WorkerRequest, WorkerResult, execute, read_validated_result};

fn request(root: &std::path::Path) -> WorkerRequest {
    WorkerRequest {
        protocol_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        attempt: 1,
        lease_token: uuid::Uuid::new_v4().to_string(),
        source_version_id: uuid::Uuid::new_v4().to_string(),
        input_path: root.join("input.csv"),
        format: SourceFormat::Csv,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: None,
        ocr: None,
    }
}

fn failure_code(request: &WorkerRequest) -> String {
    match execute(request).result {
        WorkerResult::Failure { code } => code,
        other => panic!("expected a failure result, got {other:?}"),
    }
}

#[test]
fn quoted_commas_newlines_and_escaped_quotes_survive_as_row_blocks() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let body =
        "名称,说明\r\n\"甲, 乙\",\"第一行\n第二行\"\r\n\"引号\"\"示例\",3\r\n,,\r\n合计,2\r\n";
    std::fs::write(&request.input_path, body.as_bytes()).unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.parser_version, "csv-v1");
    assert!(parsed.page_count.is_none());
    assert!(!parsed.needs_ocr);
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        [
            "名称 | 说明",
            "甲, 乙 | 第一行 第二行",
            "引号\"示例 | 3",
            "合计 | 2"
        ]
    );
}

#[test]
fn empty_and_invalid_csv_files_are_explicit_failures() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input_path, "\n,,\n  \n").unwrap();
    assert_eq!(failure_code(&request), "csv_no_content");
    std::fs::write(&request.input_path, [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(failure_code(&request), "csv_not_utf8");
}
