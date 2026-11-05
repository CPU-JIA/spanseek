use pkms_core::{
    BoardGroupBy, CanvasNode, DeleteTrash, Library, NoteSort, NotesFilter, SaveCanvas, SaveView,
    SetObjectFlag, TrashPreviewRequest, TrashSelection, ViewConfig, ViewLayout, ViewProperty,
};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn view_config() -> ViewConfig {
    ViewConfig {
        visible_properties: vec![
            ViewProperty::InboxState,
            ViewProperty::Tags,
            ViewProperty::UpdatedAt,
        ],
        board_group_by: BoardGroupBy::InboxState,
        calendar_month: Some("2026-10".into()),
        ..Default::default()
    }
}

#[test]
fn database_style_layouts_persist_and_view_results_carry_editable_metadata() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("研究主题", "正文").unwrap();
    let tag = library
        .assign_tag(
            &id(),
            pkms_core::AssignTag {
                targets: vec![pkms_core::MetadataTarget {
                    object_id: note.id.clone(),
                    base_row_version: 1,
                }],
                tag: pkms_core::TagSelector::Name {
                    name: "研究".into(),
                },
            },
        )
        .unwrap()
        .tag
        .unwrap();

    for (layout, group) in [
        (ViewLayout::Board, BoardGroupBy::InboxState),
        (ViewLayout::Calendar, BoardGroupBy::Tag),
        (ViewLayout::Timeline, BoardGroupBy::InboxState),
    ] {
        let saved = library
            .view_save(
                &id(),
                SaveView {
                    view_id: None,
                    base_row_version: None,
                    name: format!("{layout:?}"),
                    filter_schema: 1,
                    filter: NotesFilter::default(),
                    sort: NoteSort::UpdatedDesc,
                    layout,
                    config_schema: 1,
                    config: ViewConfig {
                        board_group_by: group,
                        ..view_config()
                    },
                    is_pinned: false,
                    pin_position: None,
                },
            )
            .unwrap()
            .view;
        let reopened = library.view_get(&saved.id).unwrap();
        assert_eq!(reopened.layout, layout);
        assert_eq!(reopened.config_schema, 1);
        assert_eq!(reopened.config.calendar_month.as_deref(), Some("2026-10"));
        let run = library.view_run(&saved.id, 100, None).unwrap();
        assert_eq!(run.notes.items.len(), 1);
        assert_eq!(run.notes.items[0].note.id, note.id);
        assert_eq!(run.notes.items[0].metadata.tags[0].id, tag.id);
        assert_eq!(
            run.notes.items[0].metadata.inbox_state,
            Some(pkms_core::InboxState::Pending)
        );
    }
}

#[test]
fn canvas_keeps_stable_note_references_without_copying_the_body() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let first = library.create_note("第一节点", "第一份正文").unwrap();
    let second = library.create_note("第二节点", "第二份正文").unwrap();
    let canvas = library
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "研究画布".into(),
                config_schema: 1,
                nodes: vec![
                    CanvasNode {
                        note_id: first.id.clone(),
                        x: 40.0,
                        y: 60.0,
                        width: 250.0,
                        height: 148.0,
                        z_index: 0,
                    },
                    CanvasNode {
                        note_id: second.id.clone(),
                        x: 340.0,
                        y: 60.0,
                        width: 250.0,
                        height: 148.0,
                        z_index: 1,
                    },
                ],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    let loaded = library.canvas_get(&canvas.id).unwrap();
    assert_eq!(loaded.notes.len(), 2);
    assert!(loaded.missing_note_ids.is_empty());
    assert_eq!(loaded.notes[0].note.title, "第一节点");
    assert!(!loaded.canvas.nodes[0].note_id.is_empty());

    let current = library.note(&first.id).unwrap();
    library
        .save_document(
            &id(),
            pkms_core::SaveNote {
                note_id: first.id.clone(),
                base_revision_id: current.revision_id,
                title: "重命名后的第一节点".into(),
                document_schema: current.schema_version,
                document: current.document,
            },
        )
        .unwrap();
    let renamed = library.canvas_get(&canvas.id).unwrap();
    assert_eq!(renamed.notes[0].note.id, first.id);
    assert_eq!(renamed.notes[0].note.title, "重命名后的第一节点");

    library.set_trashed(&second.id, true).unwrap();
    let trashed = library.canvas_get(&canvas.id).unwrap();
    assert!(
        trashed
            .notes
            .iter()
            .any(|note| note.note.id == second.id && note.note.trashed)
    );
    library.set_trashed(&second.id, false).unwrap();
    let restored = library.canvas_get(&canvas.id).unwrap();
    assert!(restored.notes.iter().all(|note| !note.note.trashed));
    assert_eq!(restored.notes.len(), 2);

    let updated = library
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: Some(canvas.id.clone()),
                base_row_version: Some(canvas.row_version),
                name: canvas.name.clone(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: first.id.clone(),
                    x: 80.0,
                    y: 80.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    assert_eq!(updated.nodes.len(), 1);
    assert_eq!(library.canvas_get(&canvas.id).unwrap().notes.len(), 1);
}

#[test]
fn canvas_and_view_validation_rejects_future_or_unsafe_configuration() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("节点", "正文").unwrap();
    let mut invalid_view = SaveView {
        view_id: None,
        base_row_version: None,
        name: "未来视图".into(),
        filter_schema: 1,
        filter: NotesFilter::default(),
        sort: NoteSort::UpdatedDesc,
        layout: ViewLayout::Board,
        config_schema: 2,
        config: view_config(),
        is_pinned: false,
        pin_position: None,
    };
    assert!(library.view_save(&id(), invalid_view.clone()).is_err());
    invalid_view.config_schema = 1;
    invalid_view.config.calendar_month = Some("2026-13".into());
    assert!(library.view_save(&id(), invalid_view).is_err());

    let invalid_node = CanvasNode {
        note_id: note.id,
        x: f64::NAN,
        y: 0.0,
        width: 250.0,
        height: 148.0,
        z_index: 0,
    };
    assert!(matches!(
        library.canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "非法画布".into(),
                config_schema: 1,
                nodes: vec![invalid_node],
                is_pinned: false,
                pin_position: None,
            },
        ),
        Err(pkms_core::Error::InvalidInput(_))
    ));
}

#[test]
fn hard_deleted_note_keeps_its_canvas_slot_as_a_missing_reference() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let keep = library.create_note("保留节点", "保留正文").unwrap();
    let doomed = library.create_note("待清除节点", "待清除正文").unwrap();
    let canvas = library
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "引用状态画布".into(),
                config_schema: 1,
                nodes: vec![
                    CanvasNode {
                        note_id: keep.id.clone(),
                        x: 40.0,
                        y: 60.0,
                        width: 250.0,
                        height: 148.0,
                        z_index: 0,
                    },
                    CanvasNode {
                        note_id: doomed.id.clone(),
                        x: 340.0,
                        y: 60.0,
                        width: 250.0,
                        height: 148.0,
                        z_index: 1,
                    },
                ],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    let base = library.object_metadata(&doomed.id).unwrap().row_version;
    library
        .set_object_trashed(
            &id(),
            SetObjectFlag {
                object_id: doomed.id.clone(),
                base_row_version: base,
                value: true,
            },
        )
        .unwrap();
    let snapshot = library
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::Objects {
                object_ids: vec![doomed.id.clone()],
            },
        })
        .unwrap();
    library
        .delete_trash(
            &id(),
            DeleteTrash {
                selection: snapshot.selection.clone(),
                snapshot_token: snapshot.snapshot_token.clone(),
            },
        )
        .unwrap();
    let view = library.canvas_get(&canvas.id).unwrap();
    assert_eq!(view.canvas.nodes.len(), 2);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].note.id, keep.id);
    assert_eq!(view.missing_note_ids, vec![doomed.id.clone()]);
}
