use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn typed_document(kind: &str) -> NoteDocument {
    serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {
                "type": "frontmatter",
                "attrs": { "id": id(), "source": format!("type: {kind}") }
            },
            { "type": "paragraph", "attrs": { "id": id() } }
        ]
    }))
    .unwrap()
}

fn create_typed(db: &mut Library, title: &str, kind: &str) -> Note {
    db.create_document(
        &id(),
        CreateNote {
            collection_id: None,
            base_collection_row_version: None,
            title: title.into(),
            document_schema: 3,
            document: typed_document(kind),
        },
    )
    .unwrap()
    .note
}

fn page(db: &Library, object_types: Vec<&str>) -> Vec<String> {
    db.notes_page(NotesPageRequest {
        limit: 50,
        cursor: None,
        query: None,
        trashed: false,
        sort: Some(NoteSort::TitleAsc),
        filter: Some(NotesFilter {
            object_types: object_types.into_iter().map(str::to_owned).collect(),
            ..NotesFilter::default()
        }),
    })
    .unwrap()
    .items
    .into_iter()
    .map(|note| note.title)
    .collect()
}

#[test]
fn object_type_filter_is_indexed_case_normalized_and_keeps_untyped_notes_addressable() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let project = create_typed(&mut db, "A project", "projects");
    create_typed(&mut db, "A character", "character");
    let untyped = db.create_note("Untyped note", "Body").unwrap();

    assert_eq!(page(&db, vec!["projects"]), vec!["A project"]);
    assert_eq!(
        page(&db, vec!["projects", "character"]),
        vec!["A character", "A project"]
    );
    assert_eq!(page(&db, vec!["__untyped"]), vec!["Untyped note"]);

    create_typed(&mut db, "中文项目", "项目");
    assert_eq!(page(&db, vec!["项目"]), vec!["中文项目"]);
    assert_eq!(page(&db, vec!["PROJECTS"]), vec!["A project"]);

    let current = db.note(&project.id).unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: project.id.clone(),
            base_revision_id: current.revision_id,
            title: "A project".into(),
            document_schema: 3,
            document: typed_document("idea"),
        },
    )
    .unwrap();
    assert_eq!(page(&db, vec!["projects"]), Vec::<String>::new());
    assert_eq!(page(&db, vec!["idea"]), vec!["A project"]);
    assert_eq!(db.note(&untyped.id).unwrap().id, untyped.id);
}

#[test]
fn object_type_filter_rejects_invalid_tokens_before_querying() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("Note", "Body").unwrap();
    let error = db
        .notes_page(NotesPageRequest {
            limit: 10,
            cursor: None,
            query: None,
            trashed: false,
            sort: None,
            filter: Some(NotesFilter {
                object_types: vec!["../SQL".into()],
                ..NotesFilter::default()
            }),
        })
        .unwrap_err();
    assert!(matches!(error, Error::InvalidInput(_)));
}
