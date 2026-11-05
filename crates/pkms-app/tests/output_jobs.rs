use pkms_app::*;
use pkms_core::{ExportFormat, JobStatus, PortableJobInput, PortableJobResult};
use tempfile::tempdir;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn open(app: &mut AppService, path: &std::path::Path, create: bool) -> WorkspaceIdentity {
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
    let accepted = app
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
    app.execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity
}
#[test]
fn output_jobs_execute_after_restart_keep_original_revision_and_replay_without_rewriting_files() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = open(&mut app, &workspace, true);
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("original", "原始正文")
        .unwrap();
    let destination = root.path().join("export.zip");
    let grant = app
        .grant_path(GrantPurpose::ExportFile, &destination)
        .unwrap();
    let command = id();
    let input = PortableJobInput::ExportNote {
        note_id: note.id.clone(),
        revision_id: note.revision_id.clone(),
        format: ExportFormat::Markdown,
        citation_style_id: None,
    };
    let accepted = app
        .accept_output_job(&identity, &command, &grant.token, input.clone())
        .unwrap();
    assert!(!destination.exists());
    app.library_mut(&identity)
        .unwrap()
        .save_note(&note.id, 1, "new", "新正文")
        .unwrap();
    drop(app);
    let mut app = AppService::open(&app_path).unwrap();
    assert_eq!(open(&mut app, &workspace, false), identity);
    app.run_pending_portable_jobs().unwrap();
    let result = app
        .library(&identity)
        .unwrap()
        .portable_job(&accepted.job_id)
        .unwrap();
    assert_eq!(result.status, JobStatus::Succeeded);
    let Some(PortableJobResult::Export(report)) = result.result else {
        panic!("missing export")
    };
    assert_eq!(report.revision_id, note.revision_id);
    let before = std::fs::read(&destination).unwrap();
    let replay = app
        .accept_output_job(&identity, &command, &grant.token, input)
        .unwrap();
    assert!(replay.receipt.replayed);
    app.run_pending_portable_jobs().unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), before);
    let backup = root.path().join("backup.zip");
    let grant = app.grant_path(GrantPurpose::ExportFile, &backup).unwrap();
    let accepted = app
        .accept_output_job(
            &identity,
            &id(),
            &grant.token,
            PortableJobInput::BackupCreate,
        )
        .unwrap();
    app.run_pending_portable_jobs().unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .portable_job(&accepted.job_id)
            .unwrap()
            .status,
        JobStatus::Succeeded
    );
    pkms_core::verify_backup(&backup, &mut |_| Ok(())).unwrap();
}
#[test]
fn independent_job_control_enforces_active_identity_and_cancel_prevents_publication() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = open(&mut app, &root.path().join("knowledge"), true);
    let output = root.path().join("never.zip");
    let token = app
        .grant_path(GrantPurpose::ExportFile, &output)
        .unwrap()
        .token;
    let accepted = app
        .accept_output_job(&identity, &id(), &token, PortableJobInput::BackupCreate)
        .unwrap();
    let mut control = app.operation_control().unwrap();
    let request = DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command: DesktopCommand::JobCancel {
            job_id: accepted.job_id.clone(),
        },
    };
    assert!(control.dispatch(request.clone(), None).is_err());
    let response = control.dispatch(request, Some(&identity)).unwrap();
    let DesktopData::Job(job) = response.data else {
        panic!("wrong reply")
    };
    assert_eq!(job.status, JobStatus::Canceled);
    app.run_pending_portable_jobs().unwrap();
    assert!(!output.exists());
}

#[test]
fn output_directory_requires_completed_local_publication_and_original_file_identity() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = open(&mut app, &workspace, true);
    let destination = root.path().join("中文导出.zip");
    let grant = app
        .grant_path(GrantPurpose::ExportFile, &destination)
        .unwrap();
    let accepted = app
        .accept_output_job(
            &identity,
            &id(),
            &grant.token,
            PortableJobInput::BackupCreate,
        )
        .unwrap();
    assert!(matches!(
        app.published_output_directory(&identity, &accepted.job_id),
        Err(AppError::InputUnavailable)
    ));
    app.run_pending_portable_jobs().unwrap();
    assert_eq!(
        app.published_output_directory(&identity, &accepted.job_id)
            .unwrap(),
        root.path().canonicalize().unwrap()
    );
    drop(app);
    let mut app = AppService::open(&app_path).unwrap();
    assert_eq!(open(&mut app, &workspace, false), identity);
    assert!(
        app.published_output_directory(&identity, &accepted.job_id)
            .is_ok()
    );
    let other = open(&mut app, &root.path().join("other"), true);
    assert!(
        app.published_output_directory(&other, &accepted.job_id)
            .is_err()
    );
    assert_eq!(open(&mut app, &workspace, false), identity);
    // Keep the original alive at another path, so a replacement cannot reuse
    // its filesystem identity. A same-name replacement is not the granted output.
    std::fs::rename(&destination, root.path().join("original.zip")).unwrap();
    std::fs::write(&destination, b"unrelated replacement").unwrap();
    assert!(matches!(
        app.published_output_directory(&identity, &accepted.job_id),
        Err(AppError::InputUnavailable)
    ));
}

#[test]
fn output_directory_rejects_replaced_parent_even_when_the_file_identity_is_preserved() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = open(&mut app, &root.path().join("knowledge"), true);
    let directory = root.path().join("授权输出");
    std::fs::create_dir(&directory).unwrap();
    let destination = directory.join("backup.zip");
    let grant = app
        .grant_path(GrantPurpose::ExportFile, &destination)
        .unwrap();
    let accepted = app
        .accept_output_job(
            &identity,
            &id(),
            &grant.token,
            PortableJobInput::BackupCreate,
        )
        .unwrap();
    app.run_pending_portable_jobs().unwrap();
    assert!(
        app.published_output_directory(&identity, &accepted.job_id)
            .is_ok()
    );
    let previous = root.path().join("original-output");
    std::fs::rename(&directory, &previous).unwrap();
    std::fs::create_dir(&directory).unwrap();
    std::fs::hard_link(previous.join("backup.zip"), &destination).unwrap();
    assert!(matches!(
        app.published_output_directory(&identity, &accepted.job_id),
        Err(AppError::InputUnavailable)
    ));
}

#[test]
fn published_output_survives_result_commit_failure_and_recovery_does_not_republish() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = open(&mut app, &workspace, true);
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("原文", "写出后回执失败")
        .unwrap();
    let destination = root.path().join("export.zip");
    let token = app
        .grant_path(GrantPurpose::ExportFile, &destination)
        .unwrap()
        .token;
    let accepted = app
        .accept_output_job(
            &identity,
            &id(),
            &token,
            PortableJobInput::ExportNote {
                note_id: note.id,
                revision_id: note.revision_id,
                format: ExportFormat::Html,
                citation_style_id: None,
            },
        )
        .unwrap();
    let raw = rusqlite::Connection::open(workspace.join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_output_receipt BEFORE UPDATE OF status ON portable_jobs WHEN NEW.status='succeeded' BEGIN SELECT RAISE(ABORT,'injected result loss'); END").unwrap();
    assert!(app.run_pending_portable_jobs().is_err());
    assert!(destination.is_file());
    let bytes = std::fs::read(&destination).unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .portable_job(&accepted.job_id)
            .unwrap()
            .status,
        JobStatus::Running
    );
    raw.execute_batch("DROP TRIGGER fail_output_receipt")
        .unwrap();
    drop(raw);
    drop(app);
    let mut app = AppService::open(&app_path).unwrap();
    open(&mut app, &workspace, false);
    app.run_pending_portable_jobs().unwrap();
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .portable_job(&accepted.job_id)
            .unwrap()
            .status,
        JobStatus::Succeeded
    );
    assert_eq!(std::fs::read(destination).unwrap(), bytes);
}
#[test]
fn operation_and_job_pages_walk_newest_first_and_refuse_a_foreign_cursor() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    // Three accepted-but-not-executed operations: paging only reads the queue.
    for _ in 0..3 {
        let grant = app
            .grant_path(GrantPurpose::CreateWorkspace, root.path().join(id()))
            .unwrap();
        app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
            .unwrap();
    }
    let page = |app: &mut AppService, limit: u32, cursor: Option<String>| {
        let DesktopData::OperationsPage(page) = app
            .dispatch(DesktopRequest {
                api_version: 1,
                request_id: id(),
                identity: None,
                command: DesktopCommand::OperationsPage { limit, cursor },
            })
            .unwrap()
            .data
        else {
            panic!("expected an operation page")
        };
        page
    };
    let first = page(&mut app, 2, None);
    assert_eq!(first.items.len(), 2);
    let cursor = first.next_cursor.clone().unwrap();
    let second = page(&mut app, 2, Some(cursor.clone()));
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none(), "the last page has no cursor");
    let mut seen = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|operation| operation.id.clone())
        .collect::<Vec<_>>();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3, "pages must not repeat or skip an operation");

    // Another app state must refuse the page instead of continuing its own list.
    let other = AppService::open(root.path().join("other-app")).unwrap();
    assert!(other.operations_page(2, Some(cursor)).is_err());

    let identity = open(&mut app, &root.path().join("knowledge"), true);
    for _ in 0..3 {
        let destination = root.path().join(format!("{}.zip", id()));
        let grant = app
            .grant_path(GrantPurpose::ExportFile, &destination)
            .unwrap();
        app.accept_output_job(
            &identity,
            &id(),
            &grant.token,
            PortableJobInput::BackupCreate,
        )
        .unwrap();
    }
    let jobs = |app: &mut AppService, limit: u32, cursor: Option<String>| {
        let DesktopData::JobsPage(page) = app
            .dispatch(DesktopRequest {
                api_version: 1,
                request_id: id(),
                identity: Some(identity.clone()),
                command: DesktopCommand::JobsPage { limit, cursor },
            })
            .unwrap()
            .data
        else {
            panic!("expected a job page")
        };
        page
    };
    let first = jobs(&mut app, 2, None);
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());
    let second = jobs(&mut app, 2, first.next_cursor.clone());
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
}
/// A row is written by the build that ran the operation, so history can carry a
/// shape this build no longer produces. V18-G10 replaced the two-state
/// `available` flag with the four-way `availability`; reading only the new shape
/// made every page that contained an older row fail as invalid input, which hid
/// the whole operation history behind one banner.
#[test]
fn a_history_row_written_before_the_availability_field_keeps_its_page_readable() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = open(&mut app, &workspace, true);
    let recorded = app.operations_page(30, None).unwrap().items;
    assert_eq!(
        recorded.len(),
        1,
        "the create operation is the one current row"
    );
    let recorded_path = recorded[0].result.as_ref().unwrap().path.clone();
    drop(app);

    let legacy_true = id();
    let unknown_shape = id();
    let legacy_false_now_openable = id();
    let legacy_false_forgotten = id();
    let legacy = |available: bool, known: bool| {
        serde_json::json!({
            "identity": {
                "local_instance_id": if known { identity.local_instance_id.clone() } else { id() },
                "workspace_id": if known { identity.workspace_id.clone() } else { id() },
            },
            "path": recorded_path,
            "last_opened_at": "2026-10-11T03:14:53+00:00",
            "available": available,
        })
        .to_string()
    };
    let rows = [
        (
            legacy_true.as_str(),
            "2026-10-12T00:00:00+00:00",
            legacy(true, true),
        ),
        (
            legacy_false_now_openable.as_str(),
            "2026-10-10T00:00:00+00:00",
            legacy(false, true),
        ),
        (
            legacy_false_forgotten.as_str(),
            "2026-10-09T00:00:00+00:00",
            legacy(false, false),
        ),
        (
            unknown_shape.as_str(),
            "2026-10-08T00:00:00+00:00",
            serde_json::json!({ "shape_this_build_does_not_know": 1 }).to_string(),
        ),
    ];
    let connection = rusqlite::Connection::open(app_path.join("app-state.sqlite3")).unwrap();
    // The operation row references the grant it was accepted with, so the
    // fixture keeps one: the point of the test is the result column, not the grant.
    connection
        .execute(
            "INSERT INTO local_resource_grants(token,purpose,canonical_path,path_identity,expires_at) VALUES('legacy-grant','open_workspace',?1,'legacy-identity',0)",
            rusqlite::params![recorded_path],
        )
        .unwrap();
    for (row_id, updated_at, result_json) in rows {
        connection
            .execute(
                "INSERT INTO app_operations(id,kind,token,status,stage,planned_instance_id,result_json,created_at,updated_at) VALUES(?1,'open_workspace','legacy-grant','succeeded','complete',?2,?3,?4,?4)",
                rusqlite::params![row_id, identity.local_instance_id, result_json, updated_at],
            )
            .unwrap();
    }
    drop(connection);

    let mut app = AppService::open(&app_path).unwrap();
    let DesktopData::OperationsPage(page) = app
        .dispatch(DesktopRequest {
            api_version: 1,
            request_id: id(),
            identity: None,
            command: DesktopCommand::OperationsPage {
                limit: 30,
                cursor: None,
            },
        })
        .expect("one history row this build cannot read must not fail the page")
        .data
    else {
        panic!("expected an operation page")
    };
    assert_eq!(page.items.len(), 5, "every row keeps its place in the list");
    let mapped = page
        .items
        .iter()
        .find(|item| item.id == legacy_true)
        .unwrap();
    let entry = mapped
        .result
        .as_ref()
        .expect("the pre-V18-G10 shape is read, not rejected");
    assert_eq!(
        entry.availability,
        WorkspaceAvailability::Available,
        "available: true is the same statement the old build made"
    );
    assert_eq!(entry.identity.workspace_id, identity.workspace_id);
    assert_eq!(
        mapped.updated_at, "2026-10-12T00:00:00+00:00",
        "the row keeps its own recorded position"
    );
    let measured = page
        .items
        .iter()
        .find(|item| item.id == legacy_false_now_openable)
        .and_then(|item| item.result.as_ref())
        .expect("an old row is still an entry");
    assert_eq!(
        measured.availability,
        WorkspaceAvailability::Available,
        "the old flag only said \"not usable\", so the reason is measured now"
    );
    let forgotten = page
        .items
        .iter()
        .find(|item| item.id == legacy_false_forgotten)
        .and_then(|item| item.result.as_ref())
        .expect("an old row is still an entry");
    assert_eq!(
        forgotten.availability,
        WorkspaceAvailability::Missing,
        "an identity the catalog no longer holds cannot be opened from here"
    );
    let unknown = page
        .items
        .iter()
        .find(|item| item.id == unknown_shape)
        .unwrap();
    assert!(
        unknown.result.is_none(),
        "a shape this build cannot read keeps the row without a result"
    );
}
