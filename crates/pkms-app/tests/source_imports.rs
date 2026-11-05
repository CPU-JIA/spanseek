use pkms_app::{
    AppError, AppService, GrantPurpose, OperationKind, ParseComponentIssue, SourceWorkerConfig,
    WorkspaceIdentity,
};
use pkms_core::{
    CreateAnnotation, ExportFormat, ImportSource, JobStatus, SourceFormat, SourceLocator,
};
use std::path::{Path, PathBuf};
mod common;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn workspace(app: &mut AppService, path: &Path, create: bool) -> WorkspaceIdentity {
    let grant = app
        .grant_path(
            if create {
                GrantPurpose::CreateWorkspace
            } else {
                GrantPurpose::OpenWorkspace
            },
            path,
        )
        .unwrap();
    let operation = app
        .accept_workspace_operation(
            &id(),
            if create {
                OperationKind::CreateWorkspace
            } else {
                OperationKind::OpenWorkspace
            },
            &grant.token,
        )
        .unwrap();
    app.execute_operation(&operation.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity
}
fn config() -> SourceWorkerConfig {
    let executable = common::worker_executable();
    SourceWorkerConfig {
        executable,
        pdfium_library_path: None,
        ocr: None,
        shutdown: Default::default(),
    }
}
fn input(app: &mut AppService, path: &Path, format: SourceFormat) -> ImportSource {
    let grant = app.grant_path(GrantPurpose::ImportSource, path).unwrap();
    ImportSource {
        input_token: grant.token,
        original_name: grant.display_name,
        format,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}
#[test]
fn selected_text_and_markdown_use_real_child_and_leave_originals_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    for (name, text, format) in [
        ("中文.txt", "你好，世界。\n第二段", SourceFormat::Text),
        (
            "reference.md",
            "\u{feff}# A heading\n\nMarkdown **source**",
            SourceFormat::Markdown,
        ),
    ] {
        let path = root.path().join(name);
        std::fs::write(&path, text).unwrap();
        let request = input(&mut app, &path, format);
        let command = id();
        let accepted = app
            .import_selected_source(&identity, &command, request.clone())
            .unwrap();
        let worker = app.source_jobs_worker().unwrap().unwrap();
        assert!(app.source_jobs_worker().unwrap().is_none());
        worker.run(config()).unwrap();
        let job = app
            .library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap();
        assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
        let version = job.source_version_id.unwrap();
        let parsed = app
            .library(&identity)
            .unwrap()
            .source_text_page(&version, None, 0, 20)
            .unwrap();
        assert!(!parsed.blocks.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(
            app.import_selected_source(&identity, &command, request)
                .unwrap()
                .job_id,
            accepted.job_id
        );
    }
}
#[test]
fn source_grants_reject_wrong_purpose_and_detect_same_file_modification() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.txt");
    std::fs::write(&file, "first").unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    assert!(app.grant_path(GrantPurpose::ImportSource, &file).is_err());
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let mut request = input(&mut app, &file, SourceFormat::Text);
    let image_grant = app.grant_path(GrantPurpose::ImportFile, &file).unwrap();
    let token = request.input_token.clone();
    request.input_token = image_grant.token;
    assert!(matches!(
        app.import_selected_source(&identity, &id(), request.clone()),
        Err(AppError::InputUnavailable)
    ));
    request.input_token = token;
    let previous = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::write(&file, "other").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(previous + std::time::Duration::from_secs(1)),
        )
        .unwrap();
    assert!(matches!(
        app.import_selected_source(&identity, &id(), request),
        Err(AppError::InputUnavailable)
    ));
    assert!(
        app.library(&identity)
            .unwrap()
            .source_jobs(100)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn file_replacement_with_identical_content_does_not_reuse_the_old_grant() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.txt");
    std::fs::write(&file, "same content").unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    std::fs::rename(&file, root.path().join("previous.txt")).unwrap();
    std::fs::write(&file, "same content").unwrap();
    assert!(matches!(
        app.import_selected_source(&identity, &id(), request),
        Err(AppError::InputUnavailable)
    ));
}

#[test]
fn running_capture_can_be_canceled_from_the_independent_service_connection() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("large.txt");
    std::fs::write(&file, vec![b'a'; 12 * 1024 * 1024]).unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    let worker = app.source_jobs_worker().unwrap().unwrap();
    let running = std::thread::spawn(move || worker.run(config()));
    let started = std::time::Instant::now();
    loop {
        let job = app
            .library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap();
        if job.status == JobStatus::Running {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let canceled = app
        .library_mut(&identity)
        .unwrap()
        .cancel_source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(canceled.status, JobStatus::Canceled);
    running.join().unwrap().unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap()
            .status,
        JobStatus::Canceled
    );
}

#[test]
fn shutdown_retains_interrupted_job_and_next_supervisor_resumes_it() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("large.txt");
    std::fs::write(&file, vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    let worker_config = config();
    let shutdown = worker_config.shutdown.clone();
    let worker = app.source_jobs_worker().unwrap().unwrap();
    let running = std::thread::spawn(move || worker.run(worker_config));
    let started = std::time::Instant::now();
    while app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap()
        .status
        != JobStatus::Running
    {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    shutdown.store(true, std::sync::atomic::Ordering::Release);
    running.join().unwrap().unwrap();
    let interrupted = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(interrupted.status, JobStatus::Running);
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let resumed = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(resumed.status, JobStatus::Succeeded);
    assert_eq!(resumed.attempt, 2);
}
#[test]
fn changed_after_acceptance_fails_without_registering_an_original() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.txt");
    std::fs::write(&file, "first").unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    std::fs::write(&file, "changed after acceptance").unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error_code.as_deref(), Some("input_unavailable"));
    assert!(job.source_version_id.is_none());
}
#[test]
fn a_missing_parse_component_fails_before_the_original_is_captured() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.txt");
    std::fs::write(&file, "原文件保持可读").unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    let mut unavailable = config();
    unavailable.executable = root.path().join("spanseek-worker.exe");
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(unavailable)
        .unwrap();
    let failed = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    // The record names the environment: this is the code the interface maps to
    // "the import component is unavailable", not a parse failure of the file.
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("worker_unavailable"));
    // Nothing was captured, so the user has no half-finished job to clean up.
    assert!(failed.source_version_id.is_none());
    // Once the component is in place the retry captures the file and parses it.
    let retry = app
        .retry_selected_source_job(&identity, &id(), &accepted.job_id)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let result = app
        .library(&identity)
        .unwrap()
        .source_job(&retry.job_id)
        .unwrap();
    assert_eq!(result.status, JobStatus::Succeeded, "{result:?}");
}
#[test]
fn a_pdf_without_its_engine_fails_before_the_original_is_captured() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("scan.pdf");
    std::fs::write(&file, b"%PDF-1.4 not a real document").unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Pdf);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    // A real parser without the PDF engine is what a partial installation looks
    // like, and the report names the component instead of leaving an empty path.
    let unavailable = config();
    assert_eq!(
        unavailable.parse_component_issues(),
        vec![ParseComponentIssue {
            code: "pdfium_unavailable".into(),
            path: None,
        }]
    );
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(unavailable)
        .unwrap();
    let failed = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("pdfium_unavailable"));
    assert!(failed.source_version_id.is_none());
}
#[test]
fn captured_original_survives_restart_expired_grant_and_missing_external_file() {
    let root = tempfile::tempdir().unwrap();
    let app_path = root.path().join("app");
    let knowledge = root.path().join("knowledge");
    let file = root.path().join("input.txt");
    std::fs::write(&file, "keep this original").unwrap();
    let mut app = AppService::open(&app_path).unwrap();
    let identity = workspace(&mut app, &knowledge, true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let command = id();
    let accepted = app
        .import_selected_source(&identity, &command, request.clone())
        .unwrap();
    // The helper is there but unusable, so this job stops when the parser cannot be
    // started. What the test is about is the captured original, and it has to outlive
    // the broken installation the retry later replaces.
    let mut broken = config();
    broken.executable = root.path().join("unusable-worker.exe");
    std::fs::write(&broken.executable, b"not an executable image").unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(broken)
        .unwrap();
    let failed = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(failed.error_code.as_deref(), Some("worker_unavailable"));
    assert!(failed.source_version_id.is_some());
    std::fs::remove_file(&file).unwrap();
    drop(app);
    let db = rusqlite::Connection::open(app_path.join("app-state.sqlite3")).unwrap();
    db.execute(
        "UPDATE local_resource_grants SET expires_at=0 WHERE purpose='import_source'",
        [],
    )
    .unwrap();
    drop(db);
    let mut app = AppService::open(&app_path).unwrap();
    let reopened = workspace(&mut app, &knowledge, false);
    assert_eq!(identity, reopened);
    assert_eq!(
        app.import_selected_source(&identity, &command, request)
            .unwrap()
            .job_id,
        accepted.job_id
    );
    let retry = app
        .retry_selected_source_job(&identity, &id(), &accepted.job_id)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let result = app
        .library(&identity)
        .unwrap()
        .source_job(&retry.job_id)
        .unwrap();
    assert_eq!(result.status, JobStatus::Succeeded);
    assert_eq!(result.source_version_id, failed.source_version_id);
}
#[test]
fn invalid_utf8_is_a_real_failure_with_the_original_preserved() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("invalid.txt");
    std::fs::write(&file, [0xff, 0xfe, 0x41]).unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let request = input(&mut app, &file, SourceFormat::Text);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error_code.as_deref(), Some("invalid_utf8"));
    assert!(job.source_version_id.is_some());
}

fn envelope(
    identity: &WorkspaceIdentity,
    command: pkms_app::DesktopCommand,
) -> pkms_app::DesktopRequest {
    pkms_app::DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    }
}
#[test]
fn source_ipc_pages_real_versions_annotations_and_rejects_foreign_identity() {
    use pkms_app::{DesktopCommand as Cmd, DesktopData as Data};
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.txt");
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let mut source_id: Option<String> = None;
    let mut version_ids = Vec::new();
    for number in 0..3 {
        std::fs::write(&file, format!("body {number}\nsecond block\nthird block")).unwrap();
        let mut request = input(&mut app, &file, SourceFormat::Text);
        if let Some(source_id) = &source_id {
            let source = app.library(&identity).unwrap().source(source_id).unwrap();
            request.add_version_to = Some(source.id);
            request.base_row_version = Some(source.row_version);
        }
        let reply = app
            .dispatch(envelope(
                &identity,
                Cmd::SourcesImport {
                    command_id: id(),
                    request,
                    tag_names: Vec::new(),
                },
            ))
            .unwrap();
        let Data::JobAccepted(accepted) = reply.data else {
            panic!("expected accepted source job")
        };
        app.source_jobs_worker()
            .unwrap()
            .unwrap()
            .run(config())
            .unwrap();
        let job = app
            .library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap();
        source_id = job.source_id;
        version_ids.push(job.source_version_id.unwrap());
    }
    let source_id = source_id.unwrap();
    let mut after_id = None;
    let mut listed = Vec::new();
    loop {
        let reply = app
            .dispatch(envelope(
                &identity,
                Cmd::SourcesVersions {
                    source_id: source_id.clone(),
                    after_id: after_id.clone(),
                    limit: 1,
                },
            ))
            .unwrap();
        let Data::SourceVersions(items) = reply.data else {
            panic!("expected versions")
        };
        if items.is_empty() {
            break;
        }
        assert_eq!(items.len(), 1);
        after_id = Some(items[0].id.clone());
        listed.push(items[0].id.clone());
    }
    listed.sort();
    version_ids.sort();
    assert_eq!(listed, version_ids);
    let version = version_ids[0].clone();
    let reply = app
        .dispatch(envelope(
            &identity,
            Cmd::SourcesText {
                source_version_id: version.clone(),
                parse_id: None,
                offset: 0,
                limit: 1,
            },
        ))
        .unwrap();
    let Data::SourceText(text) = reply.data else {
        panic!("expected text")
    };
    assert_eq!(text.blocks.len(), 1);
    assert_eq!(text.next_offset, Some(1));
    for comment in ["one", "two", "three"] {
        app.dispatch(envelope(
            &identity,
            Cmd::AnnotationsCreate {
                command_id: id(),
                request: pkms_core::CreateAnnotation {
                    locator: pkms_core::SourceLocator::Text {
                        source_version_id: version.clone(),
                        block_id: text.blocks[0].id.clone(),
                        start_codepoint: 0,
                        end_codepoint: 4,
                        quote: "body".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                    comment: comment.into(),
                },
            },
        ))
        .unwrap();
    }
    let mut after_id = None;
    let mut annotation_ids = Vec::new();
    loop {
        let reply = app
            .dispatch(envelope(
                &identity,
                Cmd::AnnotationsList {
                    source_version_id: version.clone(),
                    after_id: after_id.clone(),
                    limit: 1,
                    trashed: false,
                },
            ))
            .unwrap();
        let Data::Annotations(items) = reply.data else {
            panic!("expected annotations")
        };
        if items.is_empty() {
            break;
        }
        assert_eq!(items.len(), 1);
        after_id = Some(items[0].id.clone());
        annotation_ids.push(items[0].id.clone());
    }
    annotation_ids.sort();
    annotation_ids.dedup();
    assert_eq!(annotation_ids.len(), 3);
    let foreign = WorkspaceIdentity {
        local_instance_id: id(),
        workspace_id: identity.workspace_id.clone(),
    };
    assert_eq!(
        app.dispatch(envelope(&foreign, Cmd::SourcesGet { source_id }))
            .unwrap_err()
            .code,
        pkms_app::ApiErrorCode::InstanceMismatch
    );
    let mut control = app.operation_control().unwrap();
    assert_eq!(
        control
            .dispatch(
                envelope(&foreign, Cmd::SourceJobsList { limit: 100 }),
                Some(&identity)
            )
            .unwrap_err()
            .code,
        pkms_app::ApiErrorCode::InstanceMismatch
    );
    assert_eq!(
        control
            .dispatch(
                envelope(&identity, Cmd::SourceJobsList { limit: 100 }),
                None
            )
            .unwrap_err()
            .code,
        pkms_app::ApiErrorCode::InstanceMismatch
    );
}
/// A minimal but real OOXML container: the body part plus core properties.
fn docx_bytes(paragraphs: &[&str], title: &str) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let body = paragraphs
        .iter()
        .map(|text| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"))
        .collect::<String>();
    writer.start_file("word/document.xml", options).unwrap();
    writer
        .write_all(
            format!(
                "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
            )
            .as_bytes(),
        )
        .unwrap();
    writer.start_file("docProps/core.xml", options).unwrap();
    writer
        .write_all(
            format!(
                "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{title}</dc:title></cp:coreProperties>"
            )
            .as_bytes(),
        )
        .unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn selected_docx_becomes_readable_text_and_exports_its_original_container() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let path = root.path().join("会议纪要.docx");
    let bytes = docx_bytes(&["第一段正文", "第二段正文"], "合成标题");
    std::fs::write(&path, &bytes).unwrap();
    let request = input(&mut app, &path, SourceFormat::Docx);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let version_id = job.source_version_id.unwrap();
    let version = app
        .library(&identity)
        .unwrap()
        .source_version(&version_id)
        .unwrap();
    assert_eq!(version.format, SourceFormat::Docx);
    let page = app
        .library(&identity)
        .unwrap()
        .source_text_page(&version_id, None, 0, 20)
        .unwrap();
    let text: Vec<&str> = page
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(text.contains(&"第一段正文"), "{text:?}");
    assert!(text.contains(&"第二段正文"), "{text:?}");
    // The stored original is byte-identical and exports with its own extension.
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .source_original_file(&version_id)
            .unwrap()
            .0
            .size,
        bytes.len() as u64
    );
    let exported = root.path().join("docx-export.zip");
    app.library(&identity)
        .unwrap()
        .prepare_source_export(&version_id, &exported, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&exported).unwrap()).unwrap();
    let mut exported_bytes = Vec::new();
    use std::io::Read;
    archive
        .by_name(&format!("original/{version_id}.docx"))
        .unwrap()
        .read_to_end(&mut exported_bytes)
        .unwrap();
    assert_eq!(exported_bytes, bytes);
}

fn ocr_config() -> SourceWorkerConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    SourceWorkerConfig {
        executable: common::worker_executable(),
        pdfium_library_path: Some(
            root.join(".local-archive/dependencies/pdfium-7881/bin/pdfium.dll"),
        ),
        ocr: Some(pkms_app::OcrConfig {
            executable: PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"),
            tessdata_dir: root.join(".local-archive/dependencies/tesseract/tessdata"),
            languages: "chi_sim+eng".into(),
            dpi: 300,
        }),
        shutdown: Default::default(),
    }
}

#[test]
#[ignore = "requires pinned PDFium, Tesseract 5 and the generated scan fixture"]
fn scanned_pdf_ocr_flows_through_reading_annotation_and_markdown_export() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local-archive/verification/v194/scan-cn-20261019.pdf");
    std::fs::create_dir_all(sample.parent().unwrap()).unwrap();
    if !sample.is_file() {
        panic!("missing scan fixture: {}", sample.display());
    }
    let request = input(&mut app, &sample, SourceFormat::Pdf);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(ocr_config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let version_id = job.source_version_id.unwrap();
    let text = app
        .library(&identity)
        .unwrap()
        .source_text_page(&version_id, None, 0, 50)
        .unwrap();
    assert!(text.ocr_applied);
    assert!(!text.needs_ocr);
    assert_eq!(text.ocr_engine.as_deref(), Some("tesseract5"));
    let block = text
        .blocks
        .iter()
        .find(|block| block.text.contains("本地 OCR"))
        .expect("OCR text should contain the paragraph");
    let byte_start = block.text.find("本地 OCR").unwrap();
    let start = block.text[..byte_start].chars().count() as u32;
    let quote = "本地 OCR";
    let end = start + quote.chars().count() as u32;
    let geometry = block.pdf_geometry.as_ref().unwrap();
    let rects = geometry
        .chars
        .iter()
        .filter(|character| character.start_codepoint >= start && character.end_codepoint <= end)
        .filter_map(|character| character.rect)
        .collect::<Vec<_>>();
    assert!(!rects.is_empty(), "OCR word boxes should produce a locator");
    let locator = SourceLocator::Pdf {
        source_version_id: version_id.clone(),
        page_index: block.page_index.unwrap(),
        page_label: geometry.page_label.clone(),
        crop_box: geometry.crop_box,
        rotation: geometry.rotation,
        coordinate_space: geometry.coordinate_space.clone(),
        rects,
        quote: quote.into(),
        prefix: block.text.chars().take(start as usize).collect(),
        suffix: block.text.chars().skip(end as usize).take(64).collect(),
    };
    let annotation = app
        .library_mut(&identity)
        .unwrap()
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator,
                comment: "OCR 摘录".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    assert_eq!(annotation.locator.quote(), quote);

    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note(
            "扫描件摘录",
            &format!("{}（{}）", quote, annotation.comment),
        )
        .unwrap();
    let archive = root.path().join("note.zip");
    app.library(&identity)
        .unwrap()
        .export_note_archive(&note.id, &archive, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut markdown = String::new();
    {
        use std::io::Read;
        zip.by_name(&format!("{}.md", note.id))
            .unwrap()
            .read_to_string(&mut markdown)
            .unwrap();
    }
    assert!(markdown.contains(quote));
    assert!(markdown.contains("OCR 摘录"));
}

#[test]
#[ignore = "requires pinned PDFium and the generated text-layer table fixture"]
fn text_layer_table_geometry_is_extracted_without_replacing_source_text() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local-archive/verification/v194/table-cn-20261019.pdf");
    if !sample.is_file() {
        panic!("missing table fixture: {}", sample.display());
    }
    let request = input(&mut app, &sample, SourceFormat::Pdf);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(ocr_config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let text = app
        .library(&identity)
        .unwrap()
        .source_text_page(&job.source_version_id.unwrap(), None, 0, 50)
        .unwrap();
    eprintln!("tables={:#?}", text.tables);
    eprintln!("rows={:#?}", text.tables);
    eprintln!("rows={:#?}", text.tables);
    assert_eq!(text.tables.len(), 1, "{:?}", text.tables);
    assert_eq!(text.tables[0].rows.len(), 4);
    assert_eq!(text.tables[0].rows[0], ["项目", "结果", "备注"]);
    assert_eq!(text.tables[0].rows[3], ["引用", "通过", "可回查"]);
    let all: String = text
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(all.contains("复杂排版与表格抽取样本"));
    assert!(all.contains("表后正文"));
}
fn epub_bytes() -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>合成 EPUB</dc:title></metadata><manifest><item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
    let c1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>一</title></head><body><h1>第一章 起点</h1><p>EPUB 正文第一段</p></body></html>"#;
    let c2 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>二</title></head><body><h1>第二章 深入</h1><p>第二个章节段落</p></body></html>"#;
    for (name, body) in [
        ("mimetype", "application/epub+zip"),
        ("META-INF/container.xml", container),
        ("OEBPS/content.opf", opf),
        ("OEBPS/text/c1.xhtml", c1),
        ("OEBPS/text/c2.xhtml", c2),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn selected_epub_follows_the_spine_order_and_becomes_readable_text() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let path = root.path().join("书.epub");
    std::fs::write(&path, epub_bytes()).unwrap();
    let request = input(&mut app, &path, SourceFormat::Epub);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let source = app
        .library(&identity)
        .unwrap()
        .source(job.source_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(source.display_title, "合成 EPUB");
    let version_id = job.source_version_id.unwrap();
    let page = app
        .library(&identity)
        .unwrap()
        .source_text_page(&version_id, None, 0, 20)
        .unwrap();
    let text: Vec<&str> = page
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(
        text,
        [
            "第一章 起点",
            "EPUB 正文第一段",
            "第二章 深入",
            "第二个章节段落"
        ]
    );
}
#[test]
fn selected_csv_and_xlsx_become_readable_text_rows() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);

    let csv = root.path().join("台账.csv");
    std::fs::write(&csv, "名称,数量\n甲,3\n乙,4\n").unwrap();
    let request = input(&mut app, &csv, SourceFormat::Csv);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let page = app
        .library(&identity)
        .unwrap()
        .source_text_page(&job.source_version_id.unwrap(), None, 0, 20)
        .unwrap();
    let text: Vec<&str> = page
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(text, ["名称 | 数量", "甲 | 3", "乙 | 4"]);

    let xlsx = root.path().join("台账.xlsx");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let workbook = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="明细" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>合计</t></is></c><c r="B1"><v>7</v></c></row></sheetData></worksheet>"#;
    for (name, body) in [
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/worksheets/sheet1.xml", sheet),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    std::fs::write(&xlsx, writer.finish().unwrap().into_inner()).unwrap();
    let request = input(&mut app, &xlsx, SourceFormat::Xlsx);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let page = app
        .library(&identity)
        .unwrap()
        .source_text_page(&job.source_version_id.unwrap(), None, 0, 20)
        .unwrap();
    let text: Vec<&str> = page
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(text, ["Sheet: 明细", "合计 | 7"]);
}
#[test]
fn selected_pptx_becomes_readable_slide_paragraphs() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"), true);
    let path = root.path().join("汇报.pptx");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let presentation = r#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;
    let slide = r#"<?xml version="1.0"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>季度目标</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
    for (name, body) in [
        ("ppt/presentation.xml", presentation),
        ("ppt/_rels/presentation.xml.rels", rels),
        ("ppt/slides/slide1.xml", slide),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    std::fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    let request = input(&mut app, &path, SourceFormat::Pptx);
    let accepted = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{job:?}");
    let page = app
        .library(&identity)
        .unwrap()
        .source_text_page(&job.source_version_id.unwrap(), None, 0, 20)
        .unwrap();
    let text: Vec<&str> = page
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(text, ["Slide: 1", "季度目标"]);
    assert_eq!(page.page_count, Some(1));
    assert_eq!(page.toc[0].title, "季度目标");
}
