use pkms_core::*;
use serde_json::json;
use std::collections::BTreeMap;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

/// One contract-shaped relation entry, indented for a YAML list.
fn entry(note_id: &str, title: &str) -> String {
    format!("  - kind: note\n    id: {note_id}\n    title: {title}\n")
}

fn document_with_frontmatter(source: &str) -> NoteDocument {
    serde_json::from_value(json!({
        "type": "doc",
        "content": [
            { "type": "frontmatter", "attrs": { "id": id(), "source": source } },
            { "type": "paragraph", "attrs": { "id": id() } }
        ]
    }))
    .unwrap()
}

fn create_source(db: &mut Library, title: &str, source: &str) -> Note {
    db.create_document(
        &id(),
        CreateNote {
            collection_id: None,
            base_collection_row_version: None,
            title: title.into(),
            document_schema: 3,
            document: document_with_frontmatter(source),
        },
    )
    .unwrap()
    .note
}

fn save_source(db: &mut Library, note: &Note, title: &str, source: &str) -> Note {
    db.save_document(
        &id(),
        SaveNote {
            note_id: note.id.clone(),
            base_revision_id: note.revision_id.clone(),
            title: title.into(),
            document_schema: 3,
            document: document_with_frontmatter(source),
        },
    )
    .unwrap()
    .note
}

#[test]
fn only_contract_shaped_entries_for_the_target_are_reported() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let other = db.create_note("Other", "Other body").unwrap();

    let long_title = "长".repeat(301);
    let long_key = "k".repeat(257);
    let source = format!(
        "type: relation-archive\nrelated:\n{related}also:\n{duplicate}kindred:\n{kinds}blank:\n{blank}tooLong:\n{too_long}scalars:\n  - {target_id}\nscalarField: {target_id}\nmiss:\n{miss}{long_key}:\n{long_key_entry}upper:\n{upper}",
        related = entry(&target.id, "Related target"),
        duplicate = entry(&target.id, "Related target"),
        kinds = entry(&other.id, "Other target"),
        blank = entry(&target.id, ""),
        too_long = entry(&target.id, &long_title),
        target_id = target.id,
        miss = entry(&other.id, "Not the target"),
        long_key = long_key,
        long_key_entry = entry(&target.id, "Long key target"),
        upper = entry(&target.id.to_uppercase(), "Uppercase id"),
    );
    let note = create_source(&mut db, "Relation source", &source);

    let items = db.relation_backlinks(&target.id).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| (item.field_key.as_str(), item.relation_title.as_str()))
            .collect::<Vec<_>>(),
        [("related", "Related target"), ("also", "Related target")]
    );
    for item in &items {
        assert_eq!(item.source_id, note.id);
        assert_eq!(item.source_revision_id, note.revision_id);
        assert_eq!(item.source_title, note.title);
        assert!(!item.source_trashed);
        assert_eq!(
            item.context,
            format!("{}: {}", item.field_key, item.relation_title)
        );
        assert!(item.context.chars().count() <= 280);
    }

    // The same shapes pointing at another note only report that note.
    let other_items = db.relation_backlinks(&other.id).unwrap();
    assert_eq!(
        other_items
            .iter()
            .map(|item| (item.field_key.as_str(), item.source_id.as_str()))
            .collect::<Vec<_>>(),
        [("kindred", note.id.as_str()), ("miss", note.id.as_str())]
    );
}

#[test]
fn duplicate_targets_across_and_inside_fields_keep_document_order() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let source = format!(
        "first:\n{one}{two}second:\n{three}",
        one = entry(&target.id, "One"),
        two = entry(&target.id, "Two"),
        three = entry(&target.id, "Three"),
    );
    let note = create_source(&mut db, "Duplicates", &source);
    let items = db.relation_backlinks(&target.id).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| (item.field_key.as_str(), item.relation_title.as_str()))
            .collect::<Vec<_>>(),
        [("first", "One"), ("first", "Two"), ("second", "Three")]
    );
    assert!(items.iter().all(|item| item.source_id == note.id));
}

#[test]
fn malformed_yaml_unrelated_shapes_and_body_text_stay_inert() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();

    // Invalid YAML that still contains the target UUID as a substring.
    let broken = create_source(
        &mut db,
        "Broken",
        &format!("related:\n  - kind: note\n   id: {}\n", target.id),
    );
    // A mapping list that is not the relation contract.
    let shapes = create_source(
        &mut db,
        "Shapes",
        &format!("related:\n  - id: {}\n    title: No kind\n", target.id),
    );
    // The UUID only appears in body text, never in frontmatter.
    let body: NoteDocument = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "id": id() },
            "content": [{ "type": "text", "text": target.id.clone() }]
        }]
    }))
    .unwrap();
    let body_note = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Body only".into(),
                document_schema: 3,
                document: body,
            },
        )
        .unwrap()
        .note;

    let page = db.relation_backlinks_page(&target.id, 100, None).unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    assert!(!page.truncated);
    assert_eq!(page.scan_limit, 10_000);
    for note in [&broken, &shapes, &body_note] {
        assert!(
            db.relation_backlinks(&target.id)
                .unwrap()
                .iter()
                .all(|item| item.source_id != note.id)
        );
    }
}

#[test]
fn trashed_sources_are_reported_explicitly() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let live = create_source(
        &mut db,
        "Live source",
        &format!("related:\n{}", entry(&target.id, "Live")),
    );
    let trashed = create_source(
        &mut db,
        "Trashed source",
        &format!("related:\n{}", entry(&target.id, "Trashed")),
    );
    db.set_trashed(&trashed.id, true).unwrap();

    let items = db.relation_backlinks(&target.id).unwrap();
    let by_source = items
        .iter()
        .map(|item| (item.source_id.clone(), item.source_trashed))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(by_source.len(), 2);
    assert_eq!(by_source.get(&live.id), Some(&false));
    assert_eq!(by_source.get(&trashed.id), Some(&true));
    assert!(
        items
            .iter()
            .any(|item| item.source_id == trashed.id && item.source_title == "Trashed source")
    );
}

#[test]
fn pages_cover_every_row_once_without_duplicates() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let mut expected = vec![];
    for index in 0..5 {
        let note = create_source(
            &mut db,
            &format!("Source {index}"),
            &format!("related:\n{}", entry(&target.id, &format!("Title {index}"))),
        );
        expected.push(note.id);
    }
    expected.sort();

    let mut cursor = None;
    let mut seen = vec![];
    let mut sizes = vec![];
    loop {
        let page = db.relation_backlinks_page(&target.id, 2, cursor).unwrap();
        assert!(!page.truncated);
        sizes.push(page.items.len());
        seen.extend(page.items.into_iter().map(|item| item.source_id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(sizes, [2, 2, 1]);
    assert_eq!(seen, expected);

    for limit in [0, 101, u32::MAX] {
        assert!(matches!(
            db.relation_backlinks_page(&target.id, limit, None),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn cursors_are_bound_to_the_query_workspace_and_generation() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path().join("a")).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let source = create_source(
        &mut db,
        "Source",
        &format!("related:\n{}", entry(&target.id, "Title")),
    );
    for index in 0..2 {
        create_source(
            &mut db,
            &format!("Filler {index}"),
            &format!(
                "related:\n{}",
                entry(&target.id, &format!("Filler {index}"))
            ),
        );
    }
    let relation = db
        .relation_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    assert!(relation.is_some());

    // A malformed, foreign, or differently scoped cursor is refused.
    assert!(matches!(
        db.relation_backlinks_page(&target.id, 1, Some("not-json".into())),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        db.relation_backlinks_page(&id(), 1, relation.clone()),
        Err(Error::InvalidInput(_))
    ));
    let other = Library::open(dir.path().join("b")).unwrap();
    assert!(matches!(
        other.relation_backlinks_page(&target.id, 1, relation.clone()),
        Err(Error::InvalidInput(_))
    ));

    // Every notes mutation advances the list generation and expires the cursor.
    db.set_trashed(&source.id, true).unwrap();
    assert!(matches!(
        db.relation_backlinks_page(&target.id, 1, relation),
        Err(Error::QueryExpired)
    ));
    let cursor = db
        .relation_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    assert!(
        db.relation_backlinks_page(&target.id, 100, None)
            .unwrap()
            .items
            .iter()
            .any(|item| item.source_id == source.id && item.source_trashed)
    );
    db.set_trashed(&source.id, false).unwrap();
    assert!(matches!(
        db.relation_backlinks_page(&target.id, 1, cursor),
        Err(Error::QueryExpired)
    ));
    let cursor = db
        .relation_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    save_source(&mut db, &source, "Renamed", "related: []\n");
    assert!(matches!(
        db.relation_backlinks_page(&target.id, 1, cursor),
        Err(Error::QueryExpired)
    ));
}

#[test]
fn unknown_target_and_self_relation_are_reported_without_error() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();

    // A valid UUID without any note is an empty list, not a failure.
    let missing = db.relation_backlinks_page(&id(), 100, None).unwrap();
    assert!(missing.items.is_empty());
    assert!(missing.next_cursor.is_none());
    assert!(!missing.truncated);
    assert!(matches!(
        db.relation_backlinks_page("not-a-uuid", 100, None),
        Err(Error::InvalidInput(_))
    ));

    // A note whose own frontmatter lists itself is a real, reported relation.
    let self_link = create_source(&mut db, "Self", "related: []\n");
    let self_link = save_source(
        &mut db,
        &self_link,
        "Self",
        &format!("related:\n{}", entry(&self_link.id, "Self")),
    );
    let items = db.relation_backlinks(&self_link.id).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_id, self_link.id);
    assert_eq!(items[0].source_revision_id, self_link.revision_id);
    assert_eq!(items[0].field_key, "related");
    assert_eq!(items[0].relation_title, "Self");
    assert!(!items[0].source_trashed);

    // Nothing points at the untouched target.
    assert!(db.relation_backlinks(&target.id).unwrap().is_empty());
}

#[test]
fn long_relation_titles_are_bounded_to_the_context_contract() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let title = "知🦀".repeat(150);
    create_source(
        &mut db,
        "Long",
        &format!("related:\n{}", entry(&target.id, &title)),
    );
    let items = db.relation_backlinks(&target.id).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].relation_title, title);
    let context = &items[0].context;
    assert!(context.chars().count() <= 280);
    assert!(context.starts_with("related: 知🦀"));
    assert!(context.ends_with('…'));
}

/// Raw fixture rows keep a 10k-candidate library a unit test instead of a minute
/// of document writes. Notes are inserted before their revisions so foreign keys
/// hold, then the current revision is published.
fn seed_relation_candidates(path: &std::path::Path, target_id: &str, start: usize, end: usize) {
    let connection = rusqlite::Connection::open(path).unwrap();
    let range = format!(
        "WITH RECURSIVE seq(n) AS (SELECT {start} UNION ALL SELECT n+1 FROM seq WHERE n < {end})"
    );
    // Objects first: the note insert trigger marks the note dirty through a
    // foreign key onto objects(id).
    connection
        .execute(
            &format!(
                "{range} INSERT INTO objects(id,kind)
                 SELECT printf('00000000-0000-4000-9000-%012d',n),'note' FROM seq"
            ),
            [],
        )
        .unwrap();
    connection
        .execute(
            &format!(
                "{range} INSERT INTO notes(id,title,current_revision,anchor_id,trashed,updated_at,current_revision_id)
                 SELECT printf('00000000-0000-4000-9000-%012d',n),'Candidate '||n,1,printf('anchor-%012d',n),0,'1970-01-01T00:00:00Z',NULL FROM seq"
            ),
            [],
        )
        .unwrap();
    connection
        .execute(
            &format!(
                "{range} INSERT INTO note_revisions(revision_id,note_id,revision,parent_revision_id,title,schema_version,document_json,content_hash,author_kind,created_at)
                 SELECT printf('00000000-0000-4000-8000-%012d',n),printf('00000000-0000-4000-9000-%012d',n),1,NULL,'Candidate '||n,3,
                   '{{\"type\":\"doc\",\"content\":[{{\"type\":\"frontmatter\",\"attrs\":{{\"id\":\"00000000-0000-4000-a000-%012d\",\"source\":\"related:\\n  - kind: note\\n    id: ' || ?1 || '\\n    title: Candidate\\n\"}}}},{{\"type\":\"paragraph\",\"attrs\":{{\"id\":\"00000000-0000-4000-b000-%012d\"}}}}]}}',
                   printf('%064d',0),'human','1970-01-01T00:00:00Z' FROM seq"
            ),
            rusqlite::params![target_id],
        )
        .unwrap();
    // SQLite does not expose an UPDATE's CTE inside its WHERE clause, so the
    // ordinal is read back from the generated id instead.
    connection
        .execute(
            "UPDATE notes SET current_revision_id=printf('00000000-0000-4000-8000-%012d',CAST(substr(id,-12) AS INTEGER))
             WHERE id LIKE '00000000-0000-4000-9000-%'",
            [],
        )
        .unwrap();
}

/// The candidate scan is bounded, and a caller is told when the bound was
/// reached instead of being handed an incomplete list as if it were complete.
#[test]
fn candidate_scan_stops_at_the_bound_and_reports_truncation() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let path = dir.path().join("library.sqlite3");

    seed_relation_candidates(&path, &target.id, 1, 10_000);
    let page = db.relation_backlinks_page(&target.id, 100, None).unwrap();
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.scan_limit, 10_000);
    assert!(!page.truncated);
    assert!(page.next_cursor.is_some());

    seed_relation_candidates(&path, &target.id, 10_001, 10_001);
    let page = db.relation_backlinks_page(&target.id, 100, None).unwrap();
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.scan_limit, 10_000);
    assert!(page.truncated);
    assert!(page.next_cursor.is_some());
}

#[test]
fn outgoing_relations_return_live_contract_targets_in_document_order() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let first = db.create_note("First target", "First body").unwrap();
    let second = db.create_note("Second target", "Second body").unwrap();
    let trashed = db.create_note("Trashed target", "Trashed body").unwrap();
    db.set_trashed(&trashed.id, true).unwrap();
    let missing = id();
    let source = format!(
        "related:\n{first}{second}{trashed}{missing}also:\n{first_again}",
        first = entry(&first.id, "First snapshot"),
        second = entry(&second.id, "Second snapshot"),
        trashed = entry(&trashed.id, "Trashed snapshot"),
        missing = entry(&missing, "Missing snapshot"),
        first_again = entry(&first.id, "First again"),
    );
    let note = create_source(&mut db, "Outgoing source", &source);

    let items = db.outgoing_relations(&note.id).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| {
                (
                    item.field_key.as_str(),
                    item.target_id.as_str(),
                    item.relation_title.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        [
            ("related", first.id.as_str(), "First snapshot"),
            ("related", second.id.as_str(), "Second snapshot"),
            ("also", first.id.as_str(), "First again"),
        ]
    );
    assert!(items.iter().all(|item| item.context.chars().count() <= 280));
    assert!(db.outgoing_relations(&id()).unwrap().is_empty());
}
