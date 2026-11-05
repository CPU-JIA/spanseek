//! Regression from independent orchestration review: preserve bounded-result failures.
use pkms_app::{AppService, GrantPurpose, OperationKind, SourceWorkerConfig};
use pkms_core::{ImportSource, JobStatus, SourceFormat};
use std::io::Read;
mod common;

#[test]
fn real_output_limit_remains_explicit_and_original_is_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let id = || uuid::Uuid::new_v4().to_string();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.path().join("knowledge"))
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;
    let original = root.path().join("limit.txt");
    // Valid UTF-8 at the input limit. Its JSON block envelope exceeds the output limit.
    std::fs::write(&original, vec![b'a'; 32 * 1024 * 1024]).unwrap();
    let grant = app
        .grant_path(GrantPurpose::ImportSource, &original)
        .unwrap();
    let request = ImportSource {
        input_token: grant.token,
        original_name: grant.display_name,
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    };
    let job = app
        .import_selected_source(&identity, &id(), request)
        .unwrap();
    let executable = common::worker_executable();
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(SourceWorkerConfig {
            executable,
            pdfium_library_path: None,
            ocr: None,
            shutdown: Default::default(),
        })
        .unwrap();
    let library = app.library(&identity).unwrap();
    let job = library.source_job(&job.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error_code.as_deref(), Some("output_limit_or_io"));
    assert_eq!(
        job.attempt, 1,
        "deterministic output limits must not auto-retry"
    );
    let source = library.source(job.source_id.as_ref().unwrap()).unwrap();
    assert!(!source.text_ready);
    let (version, original_file) = library
        .source_original_file(job.source_version_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(version.size, 32 * 1024 * 1024);
    let mut prefix = Vec::new();
    original_file.take(16).read_to_end(&mut prefix).unwrap();
    assert_eq!(prefix, vec![b'a'; 16]);
    assert_eq!(std::fs::metadata(&original).unwrap().len(), version.size);
}
