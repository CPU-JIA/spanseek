use pkms_core::{
    HandoffObject, HandoffReadingPosition, Library, RESUME_HANDOFF_FORMAT, ResumeHandoff,
};
use std::collections::HashMap;
use uuid::Uuid;

fn handoff(workspace_id: &str, object: HandoffObject) -> ResumeHandoff {
    ResumeHandoff {
        format: RESUME_HANDOFF_FORMAT.into(),
        workspace_id: workspace_id.into(),
        exported_at: "2026-10-18T00:00:00Z".into(),
        objects: vec![object],
        excerpts: serde_json::Value::Null,
    }
}

fn object(id: &str, title: &str, body: &str) -> HandoffObject {
    HandoffObject {
        object_id: id.into(),
        title: title.into(),
        body: body.into(),
        source_url: String::new(),
        base_version: 1,
        base_revision_id: None,
        updated_at: "2026-10-18T00:00:00Z".into(),
        reading_position: HandoffReadingPosition::default(),
        document_schema: None,
        document: None,
    }
}

#[test]
fn export_contains_stable_identity_body_and_position() {
    let directory = tempfile::tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let note = library.create_note("桌面续读", "第一段\n第二段").unwrap();
    let mut positions = HashMap::new();
    positions.insert(note.id.clone(), 400u64);
    let exported = library
        .export_resume_handoff(std::slice::from_ref(&note.id), &positions)
        .unwrap();
    assert_eq!(exported.format, RESUME_HANDOFF_FORMAT);
    assert_eq!(exported.workspace_id, library.info().unwrap().workspace_id);
    assert_eq!(exported.objects.len(), 1);
    assert_eq!(exported.objects[0].object_id, note.id);
    assert_eq!(exported.objects[0].body, "第一段\n第二段");
    assert_eq!(exported.objects[0].reading_position.position, 400);
    assert!(exported.objects[0].document.is_some());
    assert_eq!(
        exported.objects[0].document_schema,
        Some(note.schema_version)
    );
    let json = serde_json::to_value(&exported).unwrap();
    assert_eq!(json["format"], RESUME_HANDOFF_FORMAT);
    assert_eq!(json["objects"][0]["reading_position"]["kind"], "scroll");
}

#[test]
fn import_preserves_object_id_is_idempotent_and_fails_closed_on_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let mut library = Library::open(directory.path()).unwrap();
    let workspace = library.info().unwrap().workspace_id;
    let new_id = Uuid::new_v4().to_string();
    let input = handoff(&workspace, object(&new_id, "手机续读", "手机正文\n第二行"));
    let imported = library.import_resume_handoff(&input).unwrap();
    assert_eq!(imported.imported.len(), 1);
    assert_eq!(imported.imported[0].id, new_id);
    assert_eq!(imported.imported[0].title, "手机续读");
    assert_eq!(
        imported.imported[0].document.plain_text(),
        "手机正文\n第二行"
    );
    assert!(imported.skipped_existing.is_empty());

    let again = library.import_resume_handoff(&input).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(again.skipped_existing, vec![new_id.clone()]);

    let conflict = handoff(&workspace, object(&new_id, "手机续读", "不同正文"));
    assert!(library.import_resume_handoff(&conflict).is_err());

    let foreign = handoff(
        "workspace-foreign",
        object(&Uuid::new_v4().to_string(), "外部", "正文"),
    );
    assert!(library.import_resume_handoff(&foreign).is_err());

    let future = ResumeHandoff {
        format: "spanseek-resume-handoff-v99".into(),
        ..handoff(
            &workspace,
            object(&Uuid::new_v4().to_string(), "未来", "正文"),
        )
    };
    assert!(library.import_resume_handoff(&future).is_err());
}
