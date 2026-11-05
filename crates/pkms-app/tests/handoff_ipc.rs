use pkms_app::{
    AppService, DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, OperationKind,
};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn create(app: &mut AppService, path: &std::path::Path) -> pkms_app::WorkspaceIdentity {
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

fn dispatch(
    app: &mut AppService,
    identity: &pkms_app::WorkspaceIdentity,
    command: DesktopCommand,
) -> Result<DesktopData, pkms_app::ApiError> {
    app.dispatch(DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    })
    .map(|reply| reply.data)
}

#[test]
fn handoff_export_and_import_use_grants_and_preserve_identity() {
    let root = tempdir().unwrap();
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let identity = create(&mut app, &root.path().join("knowledge"));
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("桌面续读源", "桌面正文\n第二行")
        .unwrap();
    let destination = root.path().join("resume.json");
    let output = app
        .grant_path(GrantPurpose::HandoffOutput, &destination)
        .unwrap();
    let exported = dispatch(
        &mut app,
        &identity,
        DesktopCommand::HandoffExport {
            token: output.token,
        },
    )
    .unwrap();
    let DesktopData::HandoffExport(summary) = exported else {
        panic!("unexpected export data");
    };
    assert_eq!(summary.workspace_id, identity.workspace_id);
    assert_eq!(summary.objects, 1);
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
    assert_eq!(bundle["format"], "spanseek-resume-handoff-v1");
    assert_eq!(bundle["objects"][0]["object_id"], note.id);
    assert_eq!(bundle["objects"][0]["body"], "桌面正文\n第二行");

    let imported_id = Uuid::new_v4().to_string();
    bundle["objects"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "object_id": imported_id,
            "title": "手机导入",
            "body": "手机正文",
            "source_url": "",
            "base_version": 0,
            "updated_at": "2026-10-18T20:00:00Z",
            "reading_position": {"kind": "scroll", "position": 320}
        }));
    let input_path = root.path().join("resume-input.json");
    std::fs::write(&input_path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let input = app
        .grant_path(GrantPurpose::HandoffInput, &input_path)
        .unwrap();
    let first = dispatch(
        &mut app,
        &identity,
        DesktopCommand::HandoffImport {
            token: input.token.clone(),
        },
    )
    .unwrap();
    let DesktopData::HandoffImport(first) = first else {
        panic!("unexpected import data");
    };
    assert_eq!(first.imported, vec![imported_id.clone()]);
    assert_eq!(first.skipped_existing, vec![note.id.clone()]);
    assert_eq!(
        app.library(&identity)
            .unwrap()
            .note(&imported_id)
            .unwrap()
            .document
            .plain_text(),
        "手机正文"
    );

    let second = dispatch(
        &mut app,
        &identity,
        DesktopCommand::HandoffImport { token: input.token },
    )
    .unwrap();
    let DesktopData::HandoffImport(second) = second else {
        panic!("unexpected import data");
    };
    assert!(second.imported.is_empty());
    assert_eq!(second.skipped_existing.len(), 2);

    bundle["workspace_id"] = serde_json::json!("workspace-foreign");
    let foreign_path = root.path().join("foreign.json");
    std::fs::write(&foreign_path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let foreign = app
        .grant_path(GrantPurpose::HandoffInput, &foreign_path)
        .unwrap();
    assert!(
        dispatch(
            &mut app,
            &identity,
            DesktopCommand::HandoffImport {
                token: foreign.token
            }
        )
        .is_err()
    );
}
