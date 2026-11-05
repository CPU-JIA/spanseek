use pkms_app::{
    AppError, AppService, DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, OperationKind,
    ParseComponentIssue, SourceWorkerConfig, WorkspaceIdentity,
};
use pkms_core::*;
use sha2::{Digest, Sha256};
use std::path::Path;
mod common;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn setup(root: &Path) -> (AppService, WorkspaceIdentity) {
    let mut app = AppService::open(root.join("app-private-path-marker")).unwrap();
    let grant = app
        .grant_path(
            GrantPurpose::CreateWorkspace,
            root.join("workspace-private-path-marker"),
        )
        .unwrap();
    let operation = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&operation.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    (app, identity)
}
fn selected() -> Vec<DiagnosticCategory> {
    vec![
        DiagnosticCategory::Versions,
        DiagnosticCategory::Statistics,
        DiagnosticCategory::Errors,
    ]
}
#[test]
fn preview_and_export_are_sanitized_frozen_and_replayable() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    app.library_mut(&identity)
        .unwrap()
        .create_note("private_title_marker", "private_body_marker")
        .unwrap();
    let answer = app
        .library_mut(&identity)
        .unwrap()
        .accept_answer_from_settings(
            &id(),
            SearchQuery {
                query: "private_query_marker".into(),
                mode: SearchMode::Lexical,
                scope: Default::default(),
                limit: 8,
                minimum_cosine: None,
            },
        )
        .unwrap()
        .answer;
    let lease = app
        .library_mut(&identity)
        .unwrap()
        .claim_answer(&answer.id)
        .unwrap()
        .unwrap();
    app.library_mut(&identity)
        .unwrap()
        .fail_answer(
            &answer.id,
            &lease.lease_token,
            AnswerStatus::Failed,
            "private_error_marker",
        )
        .unwrap();
    let preview = app.diagnostics_preview(&identity, selected()).unwrap();
    let json = serde_json::to_string(&preview).unwrap();
    for forbidden in [
        "private_title_marker",
        "private_body_marker",
        "private_query_marker",
        "private_error_marker",
        "private-path-marker",
    ] {
        assert!(
            !json.contains(forbidden),
            "private field escaped: {forbidden}"
        );
    }
    assert!(json.contains("unknown_error"));
    let output = root.path().join("diagnostics.zip");
    let grant = app.grant_path(GrantPurpose::ExportFile, &output).unwrap();
    let command = id();
    let accepted = app
        .export_diagnostics(
            &identity,
            &command,
            &grant.token,
            selected(),
            &preview.fingerprint,
        )
        .unwrap();
    app.library_mut(&identity)
        .unwrap()
        .create_note("changed after preview", "changed")
        .unwrap();
    let replay = app
        .export_diagnostics(
            &identity,
            &command,
            &grant.token,
            selected(),
            &preview.fingerprint,
        )
        .unwrap();
    assert_eq!(accepted.job_id, replay.job_id);
    let PortableJobInput::ExportDiagnostics { snapshot, .. } = app
        .library(&identity)
        .unwrap()
        .portable_job(&accepted.job_id)
        .unwrap()
        .input
    else {
        panic!("diagnostic input");
    };
    assert_eq!(*snapshot, preview.snapshot);
    app.run_pending_portable_jobs().unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .portable_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{:?}", job.error_code);
    let PortableJobResult::Diagnostics(report) = job.result.unwrap() else {
        panic!("diagnostic result");
    };
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&output).unwrap())),
        report.sha256
    );
    assert!(
        report
            .files
            .iter()
            .any(|file| file.name == "statistics.json")
    );
    let bytes = std::fs::read(&output).unwrap();
    app.run_pending_portable_jobs().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}
#[test]
fn changed_preview_is_rejected_before_job_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let preview = app.diagnostics_preview(&identity, selected()).unwrap();
    let output = root.path().join("changed.zip");
    let grant = app.grant_path(GrantPurpose::ExportFile, &output).unwrap();
    app.library_mut(&identity)
        .unwrap()
        .create_note("new", "data changed")
        .unwrap();
    assert!(matches!(
        app.export_diagnostics(
            &identity,
            &id(),
            &grant.token,
            selected(),
            &preview.fingerprint
        ),
        Err(AppError::StateConflict)
    ));
    assert!(
        app.library(&identity)
            .unwrap()
            .portable_jobs(100)
            .unwrap()
            .is_empty()
    );
    assert!(!output.exists());
}

/// A real OOXML container whose body part is not well-formed XML.
fn broken_docx() -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("word/document.xml", options).unwrap();
    writer
        .write_all(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>&not_an_entity;</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .unwrap();
    writer.finish().unwrap().into_inner()
}

fn source_worker_config() -> SourceWorkerConfig {
    let executable = common::worker_executable();
    SourceWorkerConfig {
        executable,
        pdfium_library_path: None,
        ocr: None,
        shutdown: Default::default(),
    }
}

fn zip_entry(path: &Path, name: &str) -> Vec<u8> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    bytes
}

/// A failed parse keeps its own code: the exported `errors.json` must name the
/// real worker failure instead of flattening it into `unknown_error`.
#[test]
fn real_parse_failure_keeps_its_code_in_the_exported_errors() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let file = root.path().join("broken.docx");
    std::fs::write(&file, broken_docx()).unwrap();
    let grant = app.grant_path(GrantPurpose::ImportSource, &file).unwrap();
    let accepted = app
        .import_selected_source(
            &identity,
            &id(),
            ImportSource {
                input_token: grant.token,
                original_name: grant.display_name,
                format: SourceFormat::Docx,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(source_worker_config())
        .unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .source_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    // The real child process reported a broken body part, not a generic failure.
    assert_eq!(job.error_code.as_deref(), Some("docx_invalid_xml"));
    let errors = vec![DiagnosticCategory::Errors];
    let preview = app.diagnostics_preview(&identity, errors.clone()).unwrap();
    let reported = preview.snapshot.errors.as_ref().unwrap();
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].component, "source");
    assert_eq!(reported[0].id, accepted.job_id);
    assert_eq!(reported[0].code, "docx_invalid_xml");
    let output = root.path().join("diagnostics.zip");
    let export = app.grant_path(GrantPurpose::ExportFile, &output).unwrap();
    let exported = app
        .export_diagnostics(
            &identity,
            &id(),
            &export.token,
            errors,
            &preview.fingerprint,
        )
        .unwrap();
    app.run_pending_portable_jobs().unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .portable_job(&exported.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{:?}", job.error_code);
    let errors_json = String::from_utf8(zip_entry(&output, "errors.json")).unwrap();
    assert!(errors_json.contains("docx_invalid_xml"), "{errors_json}");
    assert!(!errors_json.contains("unknown_error"), "{errors_json}");
}
/// A parse component this installation cannot use reaches the interface with the
/// exact path, while the exported bundle stays free of local paths.
#[test]
fn an_unusable_parse_component_is_named_locally_and_stays_out_of_the_export() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, identity) = setup(root.path());
    let missing = root
        .path()
        .join("component-path-marker")
        .join("spanseek-worker.exe");
    let report = || {
        vec![ParseComponentIssue {
            code: "worker_unavailable".into(),
            path: Some(missing.display().to_string()),
        }]
    };
    pkms_app::set_parse_component_issues(report());
    let request = DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: None,
        command: DesktopCommand::AppState,
    };
    let DesktopData::AppState(state) = app.dispatch(request).unwrap().data else {
        panic!("app state");
    };
    assert_eq!(state.parse_components, report());
    assert!(
        state.parse_components[0]
            .path
            .as_deref()
            .is_some_and(|path| path.contains("component-path-marker")),
        "{:?}",
        state.parse_components
    );
    // The report is local: the same installation still exports a path-free bundle.
    let preview = app.diagnostics_preview(&identity, selected()).unwrap();
    let output = root.path().join("component-diagnostics.zip");
    let grant = app.grant_path(GrantPurpose::ExportFile, &output).unwrap();
    let accepted = app
        .export_diagnostics(
            &identity,
            &id(),
            &grant.token,
            selected(),
            &preview.fingerprint,
        )
        .unwrap();
    app.run_pending_portable_jobs().unwrap();
    let job = app
        .library(&identity)
        .unwrap()
        .portable_job(&accepted.job_id)
        .unwrap();
    assert_eq!(job.status, JobStatus::Succeeded, "{:?}", job.error_code);
    for name in ["versions.json", "statistics.json", "errors.json"] {
        let entry = String::from_utf8(zip_entry(&output, name)).unwrap();
        assert!(!entry.contains("component-path-marker"), "{name}: {entry}");
    }
    // Leave no report behind for the tests that share this process.
    pkms_app::set_parse_component_issues(Vec::new());
}
