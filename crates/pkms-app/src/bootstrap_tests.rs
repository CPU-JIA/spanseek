use super::*;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn process_child() {
    let Some(path) = std::env::var_os("PKMS_BOOTSTRAP_TEST_APP") else {
        return;
    };
    let operation = std::env::var("PKMS_BOOTSTRAP_TEST_OPERATION").unwrap();
    let mut app = AppService::open(path).unwrap();
    CRASH_BEFORE_CATALOG.set(true);
    app.execute_operation(&operation).unwrap();
    panic!("bootstrap did not reach database publication checkpoint");
}

#[test]
fn process_exit_between_database_commit_and_catalog_registration_resumes_same_planned_instance() {
    let root = tempfile::tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &workspace)
        .unwrap();
    let accepted = app
        .accept_workspace_operation(
            &uuid::Uuid::new_v4().to_string(),
            OperationKind::CreateWorkspace,
            &grant.token,
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
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bootstrap::tests::process_child", "--nocapture"])
        .env("PKMS_BOOTSTRAP_TEST_APP", &app_path)
        .env("PKMS_BOOTSTRAP_TEST_OPERATION", &accepted.operation_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "bootstrap child exceeded timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(86));
    let raw = rusqlite::Connection::open(workspace.join("library.sqlite3")).unwrap();
    let workspace_id: String = raw
        .query_row("SELECT workspace_id FROM workspace_meta", [], |r| r.get(0))
        .unwrap();
    drop(raw);
    let mut app = AppService::open(&app_path).unwrap();
    let recovering = app.operation(&accepted.operation_id).unwrap();
    assert_eq!(recovering.status, OperationStatus::Queued);
    assert_eq!(recovering.stage, "resume_pending");
    assert!(recovering.result.is_none());
    assert!(app.workspaces().unwrap().is_empty());
    let completed = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(
        completed.status,
        OperationStatus::Succeeded,
        "{completed:?}"
    );
    let entry = completed.result.unwrap();
    assert_eq!(entry.identity.local_instance_id, planned);
    assert_eq!(entry.identity.workspace_id, workspace_id);
    assert!(completed.active_in_current_process);
    assert_eq!(app.workspaces().unwrap().len(), 1);
}
#[test]
fn a_cancel_requested_mid_run_ends_in_a_terminal_canceled_state() {
    let root = tempfile::tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &workspace)
        .unwrap();
    let accepted = app
        .accept_workspace_operation(
            &uuid::Uuid::new_v4().to_string(),
            OperationKind::CreateWorkspace,
            &grant.token,
        )
        .unwrap();
    // The UI cancels through a second SQLite connection while the storage
    // worker owns AppService, exactly like the operation control endpoint.
    let mut control = app.operation_control().unwrap();
    pause_hooks::arm();
    let app = std::sync::Arc::new(std::sync::Mutex::new(app));
    let worker = {
        let app = std::sync::Arc::clone(&app);
        let id = accepted.operation_id.clone();
        std::thread::spawn(move || app.lock().unwrap().execute_operation(&id))
    };
    pause_hooks::wait_until_reached(Duration::from_secs(20));
    let requested = control
        .cancel_operation(&accepted.operation_id, None)
        .unwrap();
    assert!(requested.cancel_requested);
    pause_hooks::release();
    let finished = worker.join().expect("storage worker panicked").unwrap();
    assert_eq!(finished.status, OperationStatus::Canceled, "{finished:?}");
    assert_eq!(finished.stage, "canceled");
    assert!(finished.result.is_none());
    let app = app.lock().unwrap();
    assert!(app.active_workspace().is_none());
    assert!(app.workspaces().unwrap().is_empty());
}

#[test]
fn a_cancel_requested_before_the_worker_starts_never_creates_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &workspace)
        .unwrap();
    let accepted = app
        .accept_workspace_operation(
            &uuid::Uuid::new_v4().to_string(),
            OperationKind::CreateWorkspace,
            &grant.token,
        )
        .unwrap();
    let canceled = app.cancel_operation(&accepted.operation_id).unwrap();
    assert_eq!(canceled.status, OperationStatus::Canceled);
    let finished = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(finished.status, OperationStatus::Canceled);
    assert!(!workspace.exists());
    assert!(app.workspaces().unwrap().is_empty());
}
