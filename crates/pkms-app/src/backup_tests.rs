use super::*;
use crate::OperationStatus;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[test]
fn restore_process_child() {
    let Some(root) = std::env::var_os("PKMS_RESTORE_TEST_APP") else {
        return;
    };
    let operation = std::env::var("PKMS_RESTORE_TEST_OPERATION").unwrap();
    let mut app = AppService::open(root).unwrap();
    CRASH_AFTER_PUBLICATION.set(true);
    app.execute_operation(&operation).unwrap();
    panic!("did not reach publication");
}

#[test]
fn publication_before_catalog_crash_resumes_same_instance_without_input_or_auto_open() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let mut library = pkms_core::Library::open(&source).unwrap();
    let note = library.create_note("可恢复", "真实内容").unwrap();
    let package = root.path().join("backup.zip");
    library.create_backup(&package, &mut |_| Ok(())).unwrap();
    drop(library);
    let app_path = root.path().join("app");
    let destination = root.path().join("restored");
    let mut app = AppService::open(&app_path).unwrap();
    let input = app.grant_path(GrantPurpose::BackupInput, &package).unwrap();
    let output = app
        .grant_path(GrantPurpose::RestoreWorkspace, &destination)
        .unwrap();
    let command = id();
    let accepted = app
        .accept_backup_operation(
            &command,
            OperationKind::RestoreBackup,
            &input.token,
            Some(&output.token),
        )
        .unwrap();
    let planned: String = app
        .connection
        .query_row(
            "SELECT planned_instance_id FROM app_operations WHERE id=?1",
            [&accepted.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    drop(app);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "backup::tests::restore_process_child",
            "--nocapture",
        ])
        .env("PKMS_RESTORE_TEST_APP", &app_path)
        .env("PKMS_RESTORE_TEST_OPERATION", &accepted.operation_id)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("restore process timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(87));
    assert!(destination.join("library.sqlite3").exists());
    fs::remove_file(package).unwrap();
    let mut app = AppService::open(&app_path).unwrap();
    assert!(app.workspaces().unwrap().is_empty());
    let mut control = app.operation_control().unwrap();
    let late_cancel = control
        .cancel_operation(&accepted.operation_id, None)
        .unwrap();
    assert_eq!(late_cancel.status, OperationStatus::Queued);
    assert!(!late_cancel.cancel_requested);
    drop(control);
    let operation = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(
        operation.status,
        OperationStatus::Succeeded,
        "{operation:?}"
    );
    let entry = operation.result.unwrap();
    assert_eq!(entry.identity.local_instance_id, planned);
    assert!(!operation.active_in_current_process);
    assert!(app.active_workspace().is_none());
    assert_eq!(app.workspaces().unwrap().len(), 1);
    assert!(
        app.accept_backup_operation(
            &command,
            OperationKind::RestoreBackup,
            &input.token,
            Some(&output.token)
        )
        .unwrap()
        .replayed
    );
    drop(app);
    let app = AppService::open(&app_path).unwrap();
    assert_eq!(app.workspaces().unwrap()[0].identity, entry.identity);
    assert_eq!(
        pkms_core::Library::open_existing(&destination)
            .unwrap()
            .note(&note.id)
            .unwrap(),
        note
    );
}

#[test]
fn verify_and_restore_without_active_workspace_are_durable_cancelable_and_do_not_share_instances() {
    let root = tempfile::tempdir().unwrap();
    let mut library = pkms_core::Library::open(root.path().join("source")).unwrap();
    library.create_note("测试", "离线数据").unwrap();
    let package = root.path().join("package.zip");
    let report = library.create_backup(&package, &mut |_| Ok(())).unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let input = app.grant_path(GrantPurpose::BackupInput, &package).unwrap();
    assert!(
        !app.inspect_selected_backup(&input.token)
            .unwrap()
            .fully_verified
    );
    let command = id();
    let accepted = app
        .accept_backup_operation(&command, OperationKind::VerifyBackup, &input.token, None)
        .unwrap();
    let done = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(done.status, OperationStatus::Succeeded);
    assert_eq!(done.report, Some(report));
    assert!(done.result.is_none());
    let canceled = app
        .accept_backup_operation(&id(), OperationKind::VerifyBackup, &input.token, None)
        .unwrap();
    let mut control = app.operation_control().unwrap();
    control
        .cancel_operation(&canceled.operation_id, None)
        .unwrap();
    assert_eq!(
        app.execute_operation(&canceled.operation_id)
            .unwrap()
            .status,
        OperationStatus::Canceled
    );
    let mut instances = vec![];
    for name in ["restore-one", "restore-two"] {
        let target = app
            .grant_path(GrantPurpose::RestoreWorkspace, root.path().join(name))
            .unwrap();
        let task = app
            .accept_backup_operation(
                &id(),
                OperationKind::RestoreBackup,
                &input.token,
                Some(&target.token),
            )
            .unwrap();
        let result = app.execute_operation(&task.operation_id).unwrap();
        assert_eq!(result.status, OperationStatus::Succeeded, "{result:?}");
        instances.push(result.result.unwrap().identity);
        assert!(app.active_workspace().is_none());
    }
    assert_eq!(instances[0].workspace_id, instances[1].workspace_id);
    assert_ne!(
        instances[0].local_instance_id,
        instances[1].local_instance_id
    );
    assert!(
        app.accept_backup_operation(&command, OperationKind::RestoreBackup, &input.token, None)
            .is_err()
    );
}
