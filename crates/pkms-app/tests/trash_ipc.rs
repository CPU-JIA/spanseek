use pkms_app::*;
use pkms_core::{DeleteTrash, SetObjectFlag, TrashPreviewRequest, TrashSelection};
use rusqlite::Connection;
use std::path::Path;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
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
fn request(identity: &WorkspaceIdentity, command: DesktopCommand) -> DesktopRequest {
    DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    }
}

#[test]
fn trash_ipc_retries_after_app_cleanup_failure_and_removes_both_panes_and_recents() {
    let root = tempdir().unwrap();
    let app_dir = root.path().join("app");
    let library_path = root.path().join("library");
    let mut app = AppService::open(&app_dir).unwrap();
    let identity = workspace(&mut app, &library_path, true);
    let a = app
        .library_mut(&identity)
        .unwrap()
        .create_note("将被删除", "delete")
        .unwrap();
    let b = app
        .library_mut(&identity)
        .unwrap()
        .create_note("保留正文", "keep")
        .unwrap();
    let pane = PaneState {
        tabs: vec![
            OpenTab {
                object_id: a.id.clone(),
                kind: TabKind::Note,
            },
            OpenTab {
                object_id: b.id.clone(),
                kind: TabKind::Note,
            },
        ],
        active_object_id: Some(a.id.clone()),
        scroll_offsets: vec![ScrollOffset {
            object_id: a.id.clone(),
            x: 0,
            y: 60,
        }],
    };
    let layout = SessionLayout {
        panes: vec![pane.clone(), pane],
        ..SessionLayout::default()
    };
    let saved = app.save_session(&identity, 0, layout.clone()).unwrap();
    app.touch_recent_object(&identity, &a.id).unwrap();
    app.touch_recent_object(&identity, &b.id).unwrap();
    let base = app
        .library(&identity)
        .unwrap()
        .object_metadata(&a.id)
        .unwrap()
        .row_version;
    app.library_mut(&identity)
        .unwrap()
        .set_object_trashed(
            &id(),
            SetObjectFlag {
                object_id: a.id.clone(),
                base_row_version: base,
                value: true,
            },
        )
        .unwrap();
    let snapshot = app
        .dispatch(request(
            &identity,
            DesktopCommand::TrashPreview(TrashPreviewRequest {
                selection: TrashSelection::Objects {
                    object_ids: vec![a.id.clone()],
                },
            }),
        ))
        .unwrap();
    let DesktopData::TrashPreview(snapshot) = snapshot.data else {
        panic!("wrong reply")
    };
    let delete = request(
        &identity,
        DesktopCommand::TrashDelete {
            command_id: id(),
            request: DeleteTrash {
                selection: snapshot.selection,
                snapshot_token: snapshot.snapshot_token,
            },
        },
    );
    let mut wrong = delete.clone();
    wrong.identity.as_mut().unwrap().local_instance_id = id();
    assert_eq!(
        app.dispatch(wrong).unwrap_err().code,
        ApiErrorCode::InstanceMismatch
    );
    let raw = Connection::open(app_dir.join("app-state.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER synthetic_cleanup_failure BEFORE UPDATE ON local_sessions BEGIN SELECT RAISE(ABORT,'synthetic app cleanup failure'); END;").unwrap();
    assert!(app.dispatch(delete.clone()).is_err());
    assert!(
        !app.library(&identity)
            .unwrap()
            .contains_object(&a.id)
            .unwrap()
    );
    // Even before retry commits local cleanup, reads suppress the dead identities.
    assert!(
        app.session(&identity)
            .unwrap()
            .layout
            .panes
            .iter()
            .all(|p| p.tabs.len() == 1 && p.tabs[0].object_id == b.id)
    );
    assert_eq!(app.recent_objects(&identity).unwrap().len(), 1);
    let raw_before: String = raw
        .query_row("SELECT session_json FROM local_sessions", [], |r| r.get(0))
        .unwrap();
    assert!(raw_before.contains(&a.id));
    raw.execute_batch("DROP TRIGGER synthetic_cleanup_failure;")
        .unwrap();
    drop(raw);
    drop(app);
    let mut app = AppService::open(&app_dir).unwrap();
    let reopened = workspace(&mut app, &library_path, false);
    assert_eq!(reopened, identity);
    let reply = app.dispatch(delete).unwrap();
    let DesktopData::TrashCommit(commit) = reply.data else {
        panic!("wrong reply")
    };
    assert!(commit.receipt.replayed);
    let session = app.session(&identity).unwrap();
    assert_eq!(session.state_version, saved.state_version + 1);
    for pane in &session.layout.panes {
        assert_eq!(pane.tabs.len(), 1);
        assert_eq!(pane.active_object_id.as_deref(), Some(b.id.as_str()));
        assert!(pane.scroll_offsets.is_empty());
    }
    assert!(matches!(
        app.save_session(&identity, saved.state_version, layout.clone()),
        Err(AppError::StateConflict)
    ));
    // A late writer with the new base cannot reintroduce the purged tab either.
    let late = app
        .save_session(&identity, session.state_version, layout)
        .unwrap();
    assert!(late.layout.panes.iter().all(|p| p.tabs.len() == 1));
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .note(&b.id)
            .unwrap()
            .document,
        b.document
    );
}
