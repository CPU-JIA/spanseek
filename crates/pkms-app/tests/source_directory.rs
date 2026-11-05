//! Folder import: one granted root, one recursive scan, per-entry classification
//! and one per-file grant for everything that can actually be imported.
use pkms_app::{
    AppService, DirectoryScanStatus, GrantPurpose, OperationKind, SourceWorkerConfig,
    WorkspaceIdentity,
};
use pkms_core::{ImportSource, SourceFormat};
use std::path::Path;
mod common;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let operation = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
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

fn scan(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    root: &Path,
) -> pkms_app::DirectoryScan {
    let grant = app.grant_path(GrantPurpose::ImportDirectory, root).unwrap();
    app.scan_source_directory(identity, &grant.token).unwrap()
}

#[test]
fn directory_scan_classifies_every_entry_and_grants_importable_files() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let tree = root.path().join("tree");
    std::fs::create_dir_all(tree.join("nested/deeper")).unwrap();
    std::fs::write(tree.join("note.md"), "# note").unwrap();
    std::fs::write(tree.join("nested/readme.txt"), "hello").unwrap();
    std::fs::write(tree.join("nested/deeper/deep.txt"), "deep").unwrap();
    std::fs::write(tree.join("image.png"), [0x89, b'P', b'N', b'G']).unwrap();
    std::fs::write(tree.join("report.docx"), b"PK\x03\x04docx").unwrap();
    std::fs::write(tree.join("empty.txt"), "").unwrap();

    let scan = scan(&mut app, &identity, &tree);
    assert_eq!(scan.root_name, "tree");
    assert_eq!(scan.visited, 6);
    assert!(!scan.truncated);
    assert_eq!(scan.limited, 0);
    let seen: Vec<(&str, DirectoryScanStatus)> = scan
        .entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.status))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("empty.txt", DirectoryScanStatus::Empty),
            ("image.png", DirectoryScanStatus::Unsupported),
            ("nested/deeper/deep.txt", DirectoryScanStatus::Ready),
            ("nested/readme.txt", DirectoryScanStatus::Ready),
            ("note.md", DirectoryScanStatus::Ready),
            ("report.docx", DirectoryScanStatus::Ready),
        ]
    );
    assert_eq!(
        scan.entries
            .iter()
            .find(|entry| entry.relative_path == "report.docx")
            .unwrap()
            .format,
        Some(pkms_core::SourceFormat::Docx)
    );
    for entry in &scan.entries {
        if entry.status == DirectoryScanStatus::Ready {
            assert!(entry.grant.is_some(), "{}", entry.relative_path);
            assert!(entry.format.is_some(), "{}", entry.relative_path);
        } else {
            assert!(entry.grant.is_none(), "{}", entry.relative_path);
            assert!(entry.note.is_some(), "{}", entry.relative_path);
        }
    }
    // A granted file token still imports exactly the file it was issued for.
    let ready = scan
        .entries
        .iter()
        .find(|entry| entry.relative_path == "nested/readme.txt")
        .unwrap();
    let grant = ready.grant.clone().unwrap();
    let accepted = app
        .import_selected_source(
            &identity,
            &id(),
            ImportSource {
                input_token: grant.token,
                original_name: grant.display_name,
                format: SourceFormat::Text,
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
        .run(config())
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap()
            .status,
        pkms_core::JobStatus::Succeeded
    );
}

#[test]
fn directory_scan_reports_content_duplicates_against_the_library() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let original = root.path().join("original.md");
    std::fs::write(&original, "# identical content").unwrap();
    let grant = app
        .grant_path(GrantPurpose::ImportSource, &original)
        .unwrap();
    let accepted = app
        .import_selected_source(
            &identity,
            &id(),
            ImportSource {
                input_token: grant.token,
                original_name: grant.display_name,
                format: SourceFormat::Markdown,
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
        .run(config())
        .unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .source_job(&accepted.job_id)
            .unwrap()
            .status,
        pkms_core::JobStatus::Succeeded
    );

    let tree = root.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("copy.md"), "# identical content").unwrap();
    std::fs::write(tree.join("different.md"), "# other content").unwrap();
    let scan = scan(&mut app, &identity, &tree);
    let duplicate = scan
        .entries
        .iter()
        .find(|entry| entry.relative_path == "copy.md")
        .unwrap();
    assert_eq!(duplicate.status, DirectoryScanStatus::Duplicate);
    assert_eq!(duplicate.existing_title.as_deref(), Some("original.md"));
    assert!(!duplicate.existing_trashed);
    // A duplicate stays importable: importing it points at the existing document.
    assert!(duplicate.grant.is_some());
    let fresh = scan
        .entries
        .iter()
        .find(|entry| entry.relative_path == "different.md")
        .unwrap();
    assert_eq!(fresh.status, DirectoryScanStatus::Ready);
    assert_eq!(fresh.existing_title, None);
}

#[test]
fn directory_scan_enforces_the_per_scan_grant_limit() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let tree = root.path().join("many");
    std::fs::create_dir_all(&tree).unwrap();
    for index in 0..502 {
        std::fs::write(
            tree.join(format!("file-{index:04}.txt")),
            format!("body {index}"),
        )
        .unwrap();
    }
    let scan = scan(&mut app, &identity, &tree);
    assert_eq!(scan.entries.len(), 502);
    assert_eq!(scan.limited, 2);
    let granted = scan
        .entries
        .iter()
        .filter(|entry| entry.grant.is_some())
        .count();
    assert_eq!(granted, 500);
    assert_eq!(
        scan.entries
            .iter()
            .filter(|entry| entry.status == DirectoryScanStatus::LimitReached)
            .count(),
        2
    );
    // The limit is reported, never silently dropped.
    assert!(
        scan.entries
            .iter()
            .filter(|entry| entry.status == DirectoryScanStatus::LimitReached)
            .all(|entry| entry.note.as_deref() == Some("limit_reached"))
    );
}

#[test]
fn directory_grants_reject_files_and_other_purposes() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let file = root.path().join("note.txt");
    std::fs::write(&file, "body").unwrap();
    assert!(
        app.grant_path(GrantPurpose::ImportDirectory, &file)
            .is_err()
    );
    let tree = root.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let file_grant = app.grant_path(GrantPurpose::ImportSource, &file).unwrap();
    // A token issued for a file is not a directory grant.
    assert!(
        app.scan_source_directory(&identity, &file_grant.token)
            .is_err()
    );
    let directory_grant = app
        .grant_path(GrantPurpose::ImportDirectory, &tree)
        .unwrap();
    assert!(
        app.scan_source_directory(&identity, &directory_grant.token)
            .is_ok()
    );
}
#[test]
fn directories_below_the_depth_cap_are_reported_instead_of_dropped() {
    let root = tempfile::tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let tree = root.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("top.txt"), "top").unwrap();
    // One-letter directories keep the absolute path well inside the Windows
    // limit while still crossing MAX_DEPTH.
    let mut deep = tree.clone();
    for _ in 0..40 {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("deep.txt"), "deep").unwrap();

    let scan = scan(&mut app, &identity, &tree);
    assert_eq!(
        scan.deep_skipped, 1,
        "the unreachable directory is counted, not dropped"
    );
    assert!(scan.entries.iter().all(|entry| entry.name != "deep.txt"));
    assert!(scan.entries.iter().any(|entry| entry.name == "top.txt"));
}
