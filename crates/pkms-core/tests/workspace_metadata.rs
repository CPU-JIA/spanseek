use pkms_core::*;
use tempfile::tempdir;

fn registry(workspace: &str, value: &str) -> String {
    serde_json::json!({
        "schema": 1,
        "workspaces": { workspace: [{ "id": value }] }
    })
    .to_string()
}

#[test]
fn workspace_metadata_round_trips_with_optimistic_versioning_and_validation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let initial = db.workspace_metadata().unwrap();
    assert_eq!(initial.row_version, 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&initial.object_type_registry).unwrap()["schema"],
        1
    );

    let next = db
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: initial.row_version,
            object_type_registry: registry("workspace-a", "project"),
            note_template_registry: registry("workspace-a", "brief"),
        })
        .unwrap();
    assert_eq!(next.row_version, 2);
    assert!(next.object_type_registry.contains("project"));
    assert!(next.note_template_registry.contains("brief"));

    let conflict = db
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: initial.row_version,
            object_type_registry: registry("workspace-a", "stale"),
            note_template_registry: registry("workspace-a", "stale"),
        })
        .unwrap_err();
    assert!(matches!(
        conflict,
        Error::Conflict {
            expected: 1,
            actual: 2
        }
    ));

    let invalid = db
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: 2,
            object_type_registry: "[]".into(),
            note_template_registry: next.note_template_registry.clone(),
        })
        .unwrap_err();
    assert!(matches!(invalid, Error::InvalidInput(_)));
    assert_eq!(db.workspace_metadata().unwrap(), next);
}

#[test]
fn workspace_metadata_remote_application_is_validated() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let remote = WorkspaceMetadata {
        object_type_registry: registry("workspace-a", "project"),
        note_template_registry: registry("workspace-a", "brief"),
        row_version: 9,
        updated_at: "2026-10-25T12:00:00Z".into(),
    };
    db.workspace_metadata_apply_remote(remote.clone()).unwrap();
    assert_eq!(db.workspace_metadata().unwrap(), remote);

    let before = db.workspace_metadata().unwrap();
    let invalid = WorkspaceMetadata {
        row_version: 0,
        updated_at: String::new(),
        ..remote
    };
    assert!(matches!(
        db.workspace_metadata_apply_remote(invalid),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(db.workspace_metadata().unwrap(), before);
}

#[test]
fn workspace_metadata_survives_full_backup_and_restore() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let initial = db.workspace_metadata().unwrap();
    let saved = db
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: initial.row_version,
            object_type_registry: registry("workspace-a", "project"),
            note_template_registry: registry("workspace-a", "brief"),
        })
        .unwrap();

    let package = root.path().join("metadata.pkmsbackup");
    let mut ok = |_: &str| Ok(());
    db.create_backup(&package, &mut ok).unwrap();
    verify_backup(&package, &mut ok).unwrap();

    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut ok)
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    assert_eq!(restored.workspace_metadata().unwrap(), saved);
}
