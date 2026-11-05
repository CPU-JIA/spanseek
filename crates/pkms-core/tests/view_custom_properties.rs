use pkms_core::{
    BoardGroupBy, CreateNote, Library, NoteSort, NotesFilter, SaveView, ViewConfig, ViewLayout,
    ViewProperty, ViewPropertyKind,
};
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn create_with_properties(library: &mut Library) -> String {
    let document = serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {
                "type": "frontmatter",
                "attrs": {
                    "id": id(),
                    "source": "due: 2026-11-01\npriority: 3\ndone: false\nauthors:\n  - Alice\n  - Bob\ntags: [builtin]\nnested:\n  child: hidden\n"
                }
            },
            {
                "type": "paragraph",
                "attrs": { "id": id() },
                "content": [{ "type": "text", "text": "正文" }]
            }
        ]
    }))
    .unwrap();
    library
        .create_document(
            &id(),
            CreateNote {
                title: "带属性的笔记".into(),
                document_schema: 3,
                document,
                collection_id: None,
                base_collection_row_version: None,
            },
        )
        .unwrap()
        .note
        .id
}

#[test]
fn view_run_exposes_bounded_frontmatter_and_custom_date_property() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note_id = create_with_properties(&mut library);
    let saved = library
        .view_save(
            &id(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "截止日期视图".into(),
                filter_schema: 1,
                filter: NotesFilter::default(),
                sort: NoteSort::TitleAsc,
                layout: ViewLayout::Calendar,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: vec![ViewProperty::InboxState, ViewProperty::Tags],
                    board_group_by: BoardGroupBy::InboxState,
                    calendar_month: Some("2026-11".into()),
                    custom_properties: vec!["due".into(), "priority".into(), "done".into()],
                    date_property: Some("due".into()),
                },
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .view;
    let reopened = library.view_get(&saved.id).unwrap();
    assert_eq!(
        reopened.config.custom_properties,
        ["due", "priority", "done"]
    );
    assert_eq!(reopened.config.date_property.as_deref(), Some("due"));

    let run = library.view_run(&saved.id, 100, None).unwrap();
    let item = run
        .notes
        .items
        .iter()
        .find(|item| item.note.id == note_id)
        .unwrap();
    assert!(!item.properties_error);
    assert!(!item.properties_truncated);
    assert_eq!(
        item.properties
            .iter()
            .map(|property| (
                property.key.as_str(),
                property.kind,
                property.value.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("due", ViewPropertyKind::Date, "2026-11-01"),
            ("priority", ViewPropertyKind::Number, "3"),
            ("done", ViewPropertyKind::Boolean, "false"),
            ("authors", ViewPropertyKind::List, "Alice, Bob"),
            ("nested", ViewPropertyKind::Nested, ""),
        ]
    );
}

#[test]
fn custom_property_configuration_rejects_reserved_duplicate_and_missing_date_keys() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let base = SaveView {
        view_id: None,
        base_row_version: None,
        name: "属性校验".into(),
        filter_schema: 1,
        filter: NotesFilter::default(),
        sort: NoteSort::UpdatedDesc,
        layout: ViewLayout::Table,
        config_schema: 1,
        config: ViewConfig::default(),
        is_pinned: false,
        pin_position: None,
    };

    for custom_properties in [
        vec!["tags".to_string()],
        vec!["due".to_string(), "due".to_string()],
    ] {
        let mut request = base.clone();
        request.config.custom_properties = custom_properties;
        assert!(library.view_save(&id(), request).is_err());
    }

    let mut missing_date = base;
    missing_date.config.custom_properties = vec!["priority".into()];
    missing_date.config.date_property = Some("due".into());
    assert!(library.view_save(&id(), missing_date).is_err());
}
