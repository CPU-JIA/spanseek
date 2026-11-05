use pkms_app::inference_credentials::{CredentialBinding, Secret, delete, store};
use pkms_app::{AppService, GrantPurpose, OperationKind, WorkspaceIdentity};
use pkms_core::{DiagnosticCategory, ExportFormat};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};
use tempfile::tempdir;
use uuid::Uuid;
use zip::ZipArchive;

const KEY: &str = "sk-g12-credential-sentinel-0123456789";
const BODY: &str = "G12BODYSENTINEL";

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    app.execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn assert_archive_has_no_key(path: &Path) {
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(
            !contains(&bytes, KEY),
            "credential leaked into archive entry {}",
            entry.name()
        );
    }
}

#[test]
fn credentials_and_body_stay_out_of_logs_exports_diagnostics_and_backups() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = workspace(&mut app, &root.path().join("knowledge"));
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("G12 sentinel", BODY)
        .unwrap();
    let binding = CredentialBinding::new(
        &identity.local_instance_id,
        &identity.workspace_id,
        "openai_embeddings",
        "https://example.invalid/v1",
        "g12-model",
    )
    .unwrap();
    store(&binding, &Secret::new(KEY.into()).unwrap()).unwrap();
    assert!(pkms_app::inference_credentials::exists(&binding).unwrap());
    // Diagnostics: only versions/statistics/error codes, never stored messages or paths.
    let preview = app
        .diagnostics_preview(
            &identity,
            vec![
                DiagnosticCategory::Versions,
                DiagnosticCategory::Statistics,
                DiagnosticCategory::Errors,
            ],
        )
        .unwrap();
    let diagnostics = serde_json::to_string(&preview.snapshot).unwrap();
    assert!(!diagnostics.contains(KEY));
    assert!(!diagnostics.contains(BODY));
    // Export: the note body is expected, the credential is not.
    let export = root.path().join("note.pkms.zip");
    let report = app
        .library(&identity)
        .unwrap()
        .export_note_archive(&note.id, &export, ExportFormat::Markdown, &mut |_| Ok(()))
        .unwrap();
    assert_archive_has_no_key(&export);
    let mut zip = ZipArchive::new(File::open(&export).unwrap()).unwrap();
    let mut document = String::new();
    zip.by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert!(document.contains(BODY), "document: {document}");
    assert!(!document.contains(KEY));
    // Backup: the body is expected, the credential is not.
    let backup = root.path().join("full.pkmsbackup");
    app.library_mut(&identity)
        .unwrap()
        .create_backup(&backup, &mut |_| Ok(()))
        .unwrap();
    assert_archive_has_no_key(&backup);
    // Local stores: the vault is separate; neither the library nor app-state carries the key.
    for path in [
        root.path().join("knowledge/library.sqlite3"),
        root.path().join("knowledge/library.sqlite3-wal"),
        root.path().join("app/app-state.sqlite3"),
        root.path().join("app/app-state.sqlite3-wal"),
    ] {
        if path.exists() {
            assert!(
                !contains(&fs::read(&path).unwrap(), KEY),
                "credential leaked into {}",
                path.display()
            );
        }
    }
    delete(&binding).unwrap();
}
