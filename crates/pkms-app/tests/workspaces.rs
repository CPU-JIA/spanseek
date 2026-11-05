use pkms_app::{
    AppError, AppPreferences, AppService, GrantPurpose, Navigation, OpenTab, OperationKind,
    OperationStatus, PaneState, SessionLayout, TabKind, Theme, WorkspaceAvailability,
};
use rusqlite::Connection;
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn create(app: &mut AppService, path: &Path) -> pkms_app::AppOperation {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    assert_eq!(
        app.operation(&accepted.operation_id).unwrap().status,
        OperationStatus::Queued
    );
    assert!(!path.exists());
    let result = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Succeeded, "{result:?}");
    result
}
fn open(app: &mut AppService, path: &Path) -> pkms_app::AppOperation {
    let grant = app.grant_path(GrantPurpose::OpenWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::OpenWorkspace, &grant.token)
        .unwrap();
    app.execute_operation(&accepted.operation_id).unwrap()
}

#[test]
fn preferences_work_without_workspace_and_persist_with_conflict_detection() {
    let dir = tempdir().unwrap();
    let mut app = AppService::open(dir.path()).unwrap();
    assert!(app.active_workspace().is_none());
    assert!(app.workspaces().unwrap().is_empty());
    assert!(matches!(
        AppService::open(dir.path()),
        Err(AppError::AppBusy)
    ));
    let old = app.preferences().unwrap();
    let mut preferences = old.preferences.clone();
    preferences.theme = Theme::Dark;
    preferences.zoom_percent = 150;
    let saved = app
        .update_preferences(old.state_version, preferences.clone())
        .unwrap();
    assert!(matches!(
        app.update_preferences(old.state_version, old.preferences),
        Err(AppError::StateConflict)
    ));
    preferences.search_shortcut = preferences.new_note_shortcut.clone();
    assert!(
        app.update_preferences(saved.state_version, preferences)
            .is_err()
    );
    assert_eq!(app.preferences().unwrap(), saved);
    drop(app);
    let reopened = AppService::open(dir.path()).unwrap();
    assert_eq!(reopened.preferences().unwrap(), saved);
    assert!(serde_json::from_str::<AppPreferences>(r#"{"theme":"system","unknown":1}"#).is_err());
}

#[test]
fn stable_identity_sessions_recents_and_forget_survive_restart_without_fake_active_handle() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let created = create(&mut app, &workspace);
    let identity = created.result.unwrap().identity;
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("自己的知识", "真实正文")
        .unwrap();
    app.touch_recent_object(&identity, &note.id).unwrap();
    assert_eq!(
        app.library(&identity).unwrap().note(&note.id).unwrap(),
        note
    );
    let pane = PaneState {
        scroll_offsets: vec![pkms_app::ScrollOffset {
            object_id: note.id.clone(),
            x: 0,
            y: 412,
        }],
        tabs: vec![OpenTab {
            object_id: note.id.clone(),
            kind: TabKind::Note,
        }],
        active_object_id: Some(note.id.clone()),
    };
    let layout = SessionLayout {
        window: Some(pkms_app::WindowGeometry {
            width: 1440,
            height: 960,
            left: 20,
            top: 40,
            maximized: false,
        }),
        panes: vec![pane.clone(), pane],
        active_pane: 1,
        navigation: Navigation::Notes,
        sidebar_visible: true,
        sidebar_width: Some(236),
        note_list_width: Some(340),
    };
    let saved = app.save_session(&identity, 0, layout.clone()).unwrap();
    assert!(matches!(
        app.save_session(&identity, 0, layout),
        Err(AppError::StateConflict)
    ));
    drop(app);
    let mut app = AppService::open(&app_path).unwrap();
    assert_eq!(app.session(&identity).unwrap(), saved);
    assert_eq!(app.recent_objects(&identity).unwrap()[0].object_id, note.id);
    let historical = app.operation(&created.id).unwrap();
    assert_eq!(historical.status, OperationStatus::Succeeded);
    assert!(!historical.active_in_current_process);
    assert!(
        !app.execute_operation(&created.id)
            .unwrap()
            .active_in_current_process
    );
    assert!(matches!(
        app.library(&identity),
        Err(AppError::InstanceMismatch)
    ));
    let reopened = open(&mut app, &workspace);
    assert!(reopened.active_in_current_process);
    assert_eq!(reopened.result.unwrap().identity, identity);
    app.forget_workspace(&identity).unwrap();
    assert!(app.workspaces().unwrap().is_empty());
    assert_eq!(app.session(&identity).unwrap(), saved);
    assert_eq!(
        open(&mut app, &workspace).result.unwrap().identity,
        identity
    );
    assert_eq!(app.workspaces().unwrap().len(), 1);
}

#[test]
fn legacy_session_widths_default_to_unset_without_rewriting_or_schema_upgrade() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = create(&mut app, &root.path().join("knowledge"))
        .result
        .unwrap()
        .identity;
    let original = app
        .save_session(&identity, 0, SessionLayout::default())
        .unwrap();
    let mut legacy = serde_json::to_value(&original.layout).unwrap();
    let fields = legacy.as_object_mut().unwrap();
    fields.remove("sidebar_width");
    fields.remove("note_list_width");
    let legacy = legacy.to_string();
    let raw = Connection::open(app_path.join("app-state.sqlite3")).unwrap();
    let schema: i64 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    raw.execute("UPDATE local_sessions SET session_json=?1", [&legacy])
        .unwrap();
    drop(raw);
    drop(app);

    let reopened = AppService::open(&app_path).unwrap();
    let session = reopened.session(&identity).unwrap();
    assert_eq!(session, original);
    assert_eq!(session.layout.sidebar_width, None);
    assert_eq!(session.layout.note_list_width, None);
    let raw = Connection::open(app_path.join("app-state.sqlite3")).unwrap();
    assert_eq!(
        raw.query_row("SELECT session_json FROM local_sessions", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        legacy
    );
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        schema
    );
}

#[test]
fn session_column_width_bounds_reject_changes_without_losing_saved_layout() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = create(&mut app, &root.path().join("knowledge"))
        .result
        .unwrap()
        .identity;
    let mut saved = app.session(&identity).unwrap();
    for (sidebar_width, note_list_width) in [
        (Some(160), Some(200)),
        (Some(400), Some(600)),
        (None, None),
        (Some(240), None),
        (None, Some(360)),
    ] {
        let layout = SessionLayout {
            sidebar_width,
            note_list_width,
            ..saved.layout.clone()
        };
        saved = app
            .save_session(&identity, saved.state_version, layout)
            .unwrap();
        assert_eq!(app.session(&identity).unwrap(), saved);
    }
    for (sidebar_width, note_list_width) in [
        (Some(159), Some(200)),
        (Some(401), Some(200)),
        (Some(160), Some(199)),
        (Some(160), Some(601)),
        (Some(0), None),
        (None, Some(u32::MAX)),
    ] {
        let layout = SessionLayout {
            sidebar_width,
            note_list_width,
            ..saved.layout.clone()
        };
        assert!(matches!(
            app.save_session(&identity, saved.state_version, layout),
            Err(AppError::InvalidInput("column width exceeds bounds"))
        ));
        assert_eq!(app.session(&identity).unwrap(), saved);
    }
    drop(app);
    let reopened = AppService::open(&app_path).unwrap();
    assert_eq!(reopened.session(&identity).unwrap(), saved);
}

#[test]
fn restored_copy_gets_independent_local_namespace_but_retains_logical_workspace_identity() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let original = root.path().join("original");
    let identity = create(&mut app, &original).result.unwrap().identity;
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("同一知识", "原库正文")
        .unwrap();
    app.touch_recent_object(&identity, &note.id).unwrap();
    app.save_session(&identity, 0, SessionLayout::default())
        .unwrap();
    let restored = root.path().join("restored");
    std::fs::create_dir(&restored).unwrap();
    app.library(&identity)
        .unwrap()
        .backup_to(restored.join("library.sqlite3"))
        .unwrap();
    let copy = open(&mut app, &restored);
    assert_eq!(copy.status, OperationStatus::Succeeded);
    let copy_identity = copy.result.unwrap().identity;
    assert_eq!(copy_identity.workspace_id, identity.workspace_id);
    assert_ne!(copy_identity.local_instance_id, identity.local_instance_id);
    assert_eq!(app.session(&copy_identity).unwrap().state_version, 0);
    assert!(app.recent_objects(&copy_identity).unwrap().is_empty());
    assert!(matches!(
        app.library_mut(&identity),
        Err(AppError::InstanceMismatch)
    ));
    app.library_mut(&copy_identity)
        .unwrap()
        .save_note(&note.id, 1, "副本修改", "副本正文")
        .unwrap();
    assert_eq!(open(&mut app, &original).result.unwrap().identity, identity);
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .note(&note.id)
            .unwrap()
            .document
            .plain_text(),
        "原库正文"
    );
    // Replacing the database at the same path is a new physical instance too.
    let backup = root.path().join("replacement.sqlite3");
    app.library(&identity).unwrap().backup_to(&backup).unwrap();
    app.close_workspace();
    std::fs::rename(
        original.join("library.sqlite3"),
        original.join("original-displaced.sqlite3"),
    )
    .unwrap();
    std::fs::copy(backup, original.join("library.sqlite3")).unwrap();
    let replacement = open(&mut app, &original).result.unwrap().identity;
    assert_eq!(replacement.workspace_id, identity.workspace_id);
    assert_ne!(replacement.local_instance_id, identity.local_instance_id);
    assert_eq!(app.session(&replacement).unwrap().state_version, 0);
    assert_eq!(app.session(&identity).unwrap().state_version, 1);
}

#[test]
fn operation_receipts_cancellation_and_restart_have_explicit_terminal_states() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let target = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &target)
        .unwrap();
    let command = id();
    let accepted = app
        .accept_workspace_operation(&command, OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    assert_eq!(
        app.accept_workspace_operation(&command, OperationKind::CreateWorkspace, &grant.token)
            .unwrap()
            .operation_id,
        accepted.operation_id
    );
    assert!(matches!(
        app.accept_workspace_operation(&command, OperationKind::OpenWorkspace, &grant.token),
        Err(AppError::IdempotencyMismatch)
    ));
    let canceled = app.cancel_operation(&accepted.operation_id).unwrap();
    assert_eq!(canceled.status, OperationStatus::Canceled);
    assert_eq!(
        app.execute_operation(&accepted.operation_id)
            .unwrap()
            .status,
        OperationStatus::Canceled
    );
    assert!(!target.exists());
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    drop(app);
    let mut app = AppService::open(&app_path).unwrap();
    let complete = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(complete.status, OperationStatus::Succeeded);
    assert_eq!(
        app.cancel_operation(&complete.id).unwrap().status,
        OperationStatus::Succeeded
    );
    assert_eq!(app.operations(100).unwrap().len(), 2);
}

#[test]
fn invalid_or_expired_grants_and_other_workspace_owners_preserve_active_workspace() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let mut app = AppService::open(&app_path).unwrap();
    let original = root.path().join("original");
    let identity = create(&mut app, &original).result.unwrap().identity;
    assert!(
        app.grant_path(GrantPurpose::OpenWorkspace, root.path().join("typo"))
            .is_err()
    );
    assert!(!root.path().join("typo").exists());
    let target = root.path().join("expired");
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &target)
        .unwrap();
    assert!(matches!(
        app.accept_workspace_operation(&id(), OperationKind::OpenWorkspace, &grant.token),
        Err(AppError::InputUnavailable)
    ));
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let raw = Connection::open(app_path.join("app-state.sqlite3")).unwrap();
    raw.execute(
        "UPDATE local_resource_grants SET expires_at=0 WHERE token=?1",
        [&grant.token],
    )
    .unwrap();
    let failed = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("input_unavailable"));
    assert_eq!(app.active_workspace().unwrap().identity, identity);
    assert!(!target.exists());
    let mut other = AppService::open(root.path().join("other-app")).unwrap();
    let failed = open(&mut other, &original);
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("storage_busy"));
    assert!(other.workspaces().unwrap().is_empty());
    app.close_workspace();
    assert_eq!(
        open(&mut other, &original).status,
        OperationStatus::Succeeded
    );
}

#[test]
fn newer_application_state_is_rejected_without_resetting_existing_preferences() {
    let root = tempdir().unwrap();
    let app = AppService::open(root.path()).unwrap();
    drop(app);
    let path = root.path().join("app-state.sqlite3");
    let raw = Connection::open(&path).unwrap();
    let before: String = raw
        .query_row("SELECT preferences_json FROM app_preferences", [], |r| {
            r.get(0)
        })
        .unwrap();
    raw.pragma_update(None, "user_version", 999).unwrap();
    drop(raw);
    assert!(matches!(
        AppService::open(root.path()),
        Err(AppError::UnsupportedDatabase)
    ));
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("SELECT preferences_json FROM app_preferences", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        before
    );
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        999
    );
}

#[test]
fn a_newer_workspace_is_rejected_without_writing_or_registering_it() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let workspace = root.path().join("newer-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    drop(pkms_core::Library::open(&workspace).unwrap());
    let path = workspace.join("library.sqlite3");
    let raw = Connection::open(&path).unwrap();
    raw.pragma_update(None, "user_version", 999).unwrap();
    drop(raw);
    let before = std::fs::read(&path).unwrap();
    let failed = open(&mut app, &workspace);
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("unsupported_database"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(app.workspaces().unwrap().is_empty());
    assert!(app.active_workspace().is_none());
    assert!(!workspace.join("migration-backups").exists());
}
#[test]
fn workspace_file_grants_cannot_follow_a_switch_to_another_instance() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    create(&mut app, &root.path().join("first"));
    let input = root.path().join("capture.txt");
    std::fs::write(&input, b"captured input").unwrap();
    let grant = app.grant_path(GrantPurpose::ImportFile, &input).unwrap();
    assert!(grant.identity.is_some());
    assert_eq!(
        app.resolve_grant(&grant.token, &GrantPurpose::ImportFile)
            .unwrap(),
        input.canonicalize().unwrap()
    );
    assert!(
        app.resolve_grant(&grant.token, &GrantPurpose::BackupInput)
            .is_err()
    );
    create(&mut app, &root.path().join("second"));
    assert!(matches!(
        app.resolve_grant(&grant.token, &GrantPurpose::ImportFile),
        Err(AppError::InstanceMismatch)
    ));
}

#[test]
fn a_new_directory_appearing_after_selection_is_never_overwritten() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let target = root.path().join("raced-destination");
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &target)
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("user-file.txt"), b"keep me").unwrap();
    let result = app.execute_operation(&accepted.operation_id).unwrap();
    assert_eq!(result.status, OperationStatus::Failed);
    assert_eq!(
        std::fs::read(target.join("user-file.txt")).unwrap(),
        b"keep me"
    );
    assert!(!target.join("library.sqlite3").exists());
    assert!(app.workspaces().unwrap().is_empty());
}

#[test]
fn operation_control_stays_available_without_the_storage_worker_mutex() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let mut app = AppService::open(&app_path).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.path().join("knowledge"))
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let mut control = app.operation_control().unwrap();
    let app = std::sync::Mutex::new(app);
    let worker = app.lock().unwrap();
    assert_eq!(
        control
            .operation(&accepted.operation_id, None)
            .unwrap()
            .status,
        OperationStatus::Queued
    );
    assert_eq!(
        control
            .cancel_operation(&accepted.operation_id, None)
            .unwrap()
            .status,
        OperationStatus::Canceled
    );
    drop(worker);
    assert_eq!(
        app.lock()
            .unwrap()
            .execute_operation(&accepted.operation_id)
            .unwrap()
            .status,
        OperationStatus::Canceled
    );
    drop(app);
    assert!(matches!(
        AppService::open(&app_path),
        Err(AppError::AppBusy)
    ));
    drop(control);
    assert!(AppService::open(&app_path).is_ok());
}

#[test]
fn a_recent_workspace_names_the_reason_it_cannot_be_opened() {
    let root = tempdir().unwrap();
    let app_path = root.path().join("app");
    let workspace = root.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let created = create(&mut app, &workspace);
    let identity = created.result.unwrap().identity;
    let availability = |app: &AppService| {
        app.workspaces()
            .unwrap()
            .into_iter()
            .find(|entry| entry.identity == identity)
            .expect("the recent list keeps the workspace")
            .availability
    };
    assert_eq!(availability(&app), WorkspaceAvailability::Available);

    // Release the handle first: Windows keeps an open library file locked, and a
    // user who deletes a library has no window open on it either.
    app.close_workspace();

    // The folder stays but its library disappears: the entry names the missing
    // file instead of only saying "unavailable".
    let library = workspace.join("library.sqlite3");
    let stash = root.path().join("library.sqlite3");
    std::fs::rename(&library, &stash).unwrap();
    assert_eq!(availability(&app), WorkspaceAvailability::Missing);
    std::fs::rename(&stash, &library).unwrap();
    assert_eq!(availability(&app), WorkspaceAvailability::Available);

    // The whole folder is gone: still a missing location, not a foreign library.
    let moved = root.path().join("knowledge-moved");
    std::fs::rename(&workspace, &moved).unwrap();
    assert_eq!(availability(&app), WorkspaceAvailability::Missing);

    // Another library now sits at that path - the user restored a different
    // workspace's file into this folder, for example. It is a real library, but
    // not the one the recent entry recorded, so the entry names that instead of
    // opening notes that belong elsewhere.
    std::fs::create_dir_all(&workspace).unwrap();
    drop(pkms_core::Library::open(&workspace).unwrap());
    assert_eq!(availability(&app), WorkspaceAvailability::ForeignLibrary);
}
