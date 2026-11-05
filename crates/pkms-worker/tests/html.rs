use pkms_core::SourceFormat;
use pkms_worker::{WorkerRequest, WorkerResult, execute, read_validated_result};
fn request(root: &std::path::Path) -> WorkerRequest {
    WorkerRequest {
        protocol_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        attempt: 1,
        lease_token: uuid::Uuid::new_v4().to_string(),
        source_version_id: uuid::Uuid::new_v4().to_string(),
        input_path: root.join("input.html"),
        format: SourceFormat::Html,
        output_dir: root.to_owned(),
        pdfium_library_path: None,
        content_type: Some("text/html; charset=utf-8".into()),
        ocr: None,
    }
}
#[test]
fn extracts_article_unicode_entities_inline_spacing_and_title_without_active_content() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input_path, "<!doctype html><title>阅读 &amp; 思考</title><nav>Navigation noise</nav><article><h1>标题</h1><p><span>Hello</span> <b>世界😀</b> &amp; welcome.</p><script>window.privateSecret = 'not text'</script><p hidden>Hidden</p><iframe src='https://example.invalid'>Frame</iframe></article><footer>Footer noise</footer>").unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("阅读 & 思考"));
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["标题", "Hello 世界😀 & welcome."]
    );
}
#[test]
fn honors_http_charset_and_retains_chinese_text() {
    let root = tempfile::tempdir().unwrap();
    let mut request = request(root.path());
    request.content_type = Some("text/html; charset=gbk".into());
    let (bytes, _, errors) =
        encoding_rs::GBK.encode("<title>中文标题</title><main><p>中文正文与标点。</p></main>");
    assert!(!errors);
    std::fs::write(&request.input_path, bytes).unwrap();
    let parsed = read_validated_result(&request, &execute(&request)).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("中文标题"));
    assert_eq!(parsed.blocks[0].text, "中文正文与标点。");
}
#[test]
fn login_form_and_javascript_shell_are_explicit_failures() {
    for (html, expected) in [
        (
            "<title>Login</title><form><input type=password></form><p>Sign in</p>",
            "webpage_login_required",
        ),
        (
            "<div id=app>Please enable JavaScript</div><script src=app.js></script>",
            "webpage_dynamic_content",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let request = request(root.path());
        std::fs::write(&request.input_path, html).unwrap();
        let WorkerResult::Failure { code } = execute(&request).result else {
            panic!("must not pretend to capture an article")
        };
        assert_eq!(code, expected);
    }
}
