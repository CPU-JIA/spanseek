use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;
fn id() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn pinned_self_reference_survives_copy_export_and_backup() {
    use std::io::Read;
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("library")).unwrap();
    let original = db.create_note("历史标题", "历史正文").unwrap();
    let block = original.document.content[0].attrs.id.clone();
    let doc = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{"target_id":original.id,"target_block_id":block,"target_revision_id":original.revision_id,"occurrence_id":id(),"label":"固定历史"}}]}]})).unwrap();
    let current = db
        .save_document(
            &id(),
            SaveNote {
                note_id: original.id.clone(),
                base_revision_id: original.revision_id.clone(),
                title: "当前标题".into(),
                document_schema: 2,
                document: doc,
            },
        )
        .unwrap()
        .note;
    let copied = db
        .copy_document(
            &id(),
            CopyNote {
                note_id: current.id,
                source_revision_id: current.revision_id,
                title: "副本".into(),
            },
        )
        .unwrap()
        .note;
    let reference = db
        .note_references(&copied.id, &copied.revision_id)
        .unwrap()
        .remove(0);
    assert_eq!(reference.target_id, original.id);
    assert_eq!(
        reference.target_revision_id,
        Some(original.revision_id.clone())
    );
    assert_eq!(reference.target_block_id, Some(block));
    let resolved = db
        .resolve_link(ResolveLink {
            target_id: reference.target_id.clone(),
            target_block_id: reference.target_block_id.clone(),
            target_revision_id: reference.target_revision_id.clone(),
        })
        .unwrap();
    assert_eq!(resolved.state, LinkState::Available);
    assert_eq!(resolved.block_text.as_deref(), Some("历史正文"));
    let sql = rusqlite::Connection::open(root.path().join("library/library.sqlite3")).unwrap();
    let projected: String = sql
        .query_row(
            "SELECT target_revision_id FROM body_links WHERE source_id=?1",
            [&copied.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(projected, original.revision_id);
    let archive = root.path().join("pinned.zip");
    let report = db
        .export_note_archive(&copied.id, &archive, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut html = String::new();
    zip.by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert!(html.contains(&format!("?revision={}#", original.revision_id)));
    let manifest: Vec<BodyReference> =
        serde_json::from_reader(zip.by_name("references.json").unwrap()).unwrap();
    assert_eq!(manifest[0].target_revision_id, reference.target_revision_id);
    let backup = root.path().join("pinned.pkmsbackup");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&backup, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&restored_path).unwrap();
    assert!(
        restored
            .body_backlinks(&original.id)
            .unwrap()
            .iter()
            .all(|r| r.target_revision_id == Some(original.revision_id.clone()))
    );
    let projected: String = rusqlite::Connection::open(restored_path.join("library.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT target_revision_id FROM body_links WHERE source_id=?1",
            [&copied.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(projected, original.revision_id);
}

#[test]
fn malformed_reference_revision_is_rejected_and_absent_field_stays_compatible() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let document=serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{"target_id":id(),"target_revision_id":"bad","occurrence_id":id(),"label":"坏引用"}}]}]})).unwrap();
    assert!(
        db.create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "拒绝".into(),
                document_schema: 2,
                document
            }
        )
        .is_err()
    );
    let note = db.create_note("兼容", "旧正文").unwrap();
    assert!(
        !serde_json::to_string(&note.document)
            .unwrap()
            .contains("target_revision_id")
    );
}
fn request(target: &Note, block: Option<String>, revision: Option<String>) -> ResolveLink {
    ResolveLink {
        target_id: target.id.clone(),
        target_block_id: block,
        target_revision_id: revision,
    }
}
#[test]
fn body_occurrences_resolve_stable_targets_and_historical_deleted_blocks() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("原名", "段落原文").unwrap();
    let anchor = target.document.content[0].attrs.id.clone();
    let source_doc: NoteDocument = serde_json::from_value(json!({"type":"doc","content":[
        {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"第一处上下文 "},{"type":"noteReference","attrs":{"target_id":target.id,"target_block_id":anchor,"occurrence_id":id(),"label":"原名"}}]},
        {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"第二处上下文 "},{"type":"noteReference","attrs":{"target_id":target.id,"occurrence_id":id(),"label":"原名"}}]}
    ]})).unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "来源".into(),
                document_schema: 2,
                document: source_doc,
            },
        )
        .unwrap()
        .note;
    let links = db.body_backlinks(&target.id).unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].context, "第一处上下文 原名");
    assert_ne!(links[0].occurrence_id, links[1].occurrence_id);
    let renamed = db
        .save_document(
            &id(),
            SaveNote {
                note_id: target.id.clone(),
                base_revision_id: target.revision_id.clone(),
                title: "新名".into(),
                document_schema: 2,
                document: target.document.clone(),
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        db.resolve_link(request(&target, Some(anchor.clone()), None))
            .unwrap()
            .title
            .as_deref(),
        Some("新名")
    );
    db.save_document(
        &id(),
        SaveNote {
            note_id: target.id.clone(),
            base_revision_id: renamed.revision_id.clone(),
            title: "新名".into(),
            document_schema: 2,
            document: NoteDocument::from_text(id(), "替换段落"),
        },
    )
    .unwrap();
    let missing = db
        .resolve_link(request(&target, Some(anchor.clone()), None))
        .unwrap();
    assert_eq!(missing.state, LinkState::BlockMissing);
    assert_eq!(missing.last_seen_revision_id, Some(renamed.revision_id));
    let historical = db
        .resolve_link(request(
            &target,
            Some(anchor),
            Some(target.revision_id.clone()),
        ))
        .unwrap();
    assert_eq!(historical.state, LinkState::Available);
    assert_eq!(historical.block_text.as_deref(), Some("段落原文"));
    assert_eq!(
        db.resolve_link(request(&target, None, Some(id())))
            .unwrap()
            .state,
        LinkState::RevisionMissing
    );
    db.set_trashed(&target.id, true).unwrap();
    assert_eq!(
        db.resolve_link(request(&target, None, None)).unwrap().state,
        LinkState::TargetTrashed
    );
    db.set_trashed(&source.id, true).unwrap();
    assert!(
        db.body_backlinks(&target.id)
            .unwrap()
            .iter()
            .all(|r| r.source_trashed)
    );
    db.set_trashed(&source.id, false).unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: source.id.clone(),
            base_revision_id: source.revision_id.clone(),
            title: source.title.clone(),
            document_schema: 2,
            document: NoteDocument::from_text(id(), "删去引用"),
        },
    )
    .unwrap();
    assert!(db.body_backlinks(&target.id).unwrap().is_empty());
    assert_eq!(
        db.note_references(&source.id, &source.revision_id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        db.resolve_link(ResolveLink {
            target_id: id(),
            target_block_id: None,
            target_revision_id: None
        })
        .unwrap()
        .state,
        LinkState::TargetMissing
    );
}

#[test]
fn copy_remaps_source_anchors_and_occurrences_and_exports_reference_manifest() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path().join("library")).unwrap();
    let base = db.create_note("自己", "正文").unwrap();
    let anchor = base.document.content[0].attrs.id.clone();
    let document = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":anchor},"content":[{"type":"noteReference","attrs":{"target_id":base.id,"target_block_id":anchor,"occurrence_id":id(),"label":"自身"}}]}]})).unwrap();
    let source = db
        .save_document(
            &id(),
            SaveNote {
                note_id: base.id.clone(),
                base_revision_id: base.revision_id,
                title: base.title,
                document_schema: 2,
                document,
            },
        )
        .unwrap()
        .note;
    let copied = db
        .copy_document(
            &id(),
            CopyNote {
                note_id: source.id.clone(),
                source_revision_id: source.revision_id.clone(),
                title: "复制".into(),
            },
        )
        .unwrap()
        .note;
    let copied_link = &db.note_references(&copied.id, &copied.revision_id).unwrap()[0];
    assert_eq!(copied_link.target_id, copied.id);
    assert_eq!(
        copied_link.target_block_id.as_deref(),
        Some(copied.document.content[0].attrs.id.as_str())
    );
    assert_ne!(copied_link.source_block_id, anchor);
    assert_ne!(
        copied_link.occurrence_id,
        db.note_references(&source.id, &source.revision_id).unwrap()[0].occurrence_id
    );
    let path = dir.path().join("export.zip");
    db.export_note_archive(&copied.id, &path, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let manifest: Vec<BodyReference> =
        serde_json::from_reader(archive.by_name("references.json").unwrap()).unwrap();
    assert_eq!(manifest[0].target_id, copied.id);
    let package = dir.path().join("references.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    verify_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = dir.path().join("restored");
    prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&restored_path).unwrap();
    assert_eq!(
        restored.body_backlinks(&copied.id).unwrap()[0].occurrence_id,
        copied_link.occurrence_id
    );
    assert_eq!(
        restored
            .resolve_link(request(&copied, copied_link.target_block_id.clone(), None))
            .unwrap()
            .state,
        LinkState::Available
    );
}

fn reference_document(target: &Note, count: usize, prefix: &str) -> (NoteDocument, Vec<String>) {
    let occurrences = (0..count).map(|_| id()).collect::<Vec<_>>();
    let mut content = vec![json!({"type":"text","text":prefix})];
    content.extend(occurrences.iter().map(|occurrence| {
        json!({
            "type":"noteReference",
            "attrs":{"target_id":target.id,"occurrence_id":occurrence,"label":"引用"}
        })
    }));
    let document = serde_json::from_value(json!({
        "type":"doc",
        "content":[{"type":"paragraph","attrs":{"id":id()},"content":content}]
    }))
    .unwrap();
    (document, occurrences)
}

fn create_referencing_note(
    db: &mut Library,
    target: &Note,
    count: usize,
    prefix: &str,
) -> (Note, Vec<String>) {
    let (document, occurrences) = reference_document(target, count, prefix);
    let note = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "引用来源".into(),
                document_schema: 2,
                document,
            },
        )
        .unwrap()
        .note;
    (note, occurrences)
}

#[test]
fn reference_pages_cover_more_than_100_occurrences_once_and_preserve_document_order() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("目标", "正文").unwrap();
    let (source, expected) = create_referencing_note(&mut db, &target, 205, "上下文 ");
    let mut cursor = None;
    let mut outgoing = vec![];
    let mut sizes = vec![];
    loop {
        let page = db
            .note_references_page(&source.id, &source.revision_id, 73, cursor)
            .unwrap();
        sizes.push(page.items.len());
        outgoing.extend(page.items.into_iter().map(|item| item.occurrence_id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(sizes, [73, 73, 59]);
    assert_eq!(outgoing, expected);
    let mut cursor = None;
    let mut incoming = vec![];
    let mut sizes = vec![];
    loop {
        let page = db.body_backlinks_page(&target.id, 100, cursor).unwrap();
        sizes.push(page.items.len());
        assert!(page.items.iter().all(|item| item.source_id == source.id));
        incoming.extend(page.items.into_iter().map(|item| item.occurrence_id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(sizes, [100, 100, 5]);
    assert_eq!(incoming, expected);
    for limit in [0, 101, u32::MAX] {
        assert!(matches!(
            db.body_backlinks_page(&target.id, limit, None),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            db.note_references_page(&source.id, &source.revision_id, limit, None),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn reference_cursors_reject_other_queries_workspaces_and_changed_generations() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path().join("a")).unwrap();
    let target = db.create_note("目标", "正文").unwrap();
    let (source, _) = create_referencing_note(&mut db, &target, 3, "上下文 ");
    let incoming = db
        .body_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    let outgoing = db
        .note_references_page(&source.id, &source.revision_id, 1, None)
        .unwrap()
        .next_cursor;
    assert!(matches!(
        db.body_backlinks_page(&id(), 1, incoming.clone()),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        db.body_backlinks_page(&target.id, 1, outgoing.clone()),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        db.note_references_page(&source.id, &id(), 1, outgoing.clone()),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        db.body_backlinks_page(&target.id, 1, Some("not-json".into())),
        Err(Error::InvalidInput(_))
    ));
    let other = Library::open(dir.path().join("b")).unwrap();
    assert!(matches!(
        other.body_backlinks_page(&target.id, 1, incoming.clone()),
        Err(Error::InvalidInput(_))
    ));
    db.set_trashed(&source.id, true).unwrap();
    assert!(matches!(
        db.body_backlinks_page(&target.id, 1, incoming),
        Err(Error::QueryExpired)
    ));
    assert!(matches!(
        db.note_references_page(&source.id, &source.revision_id, 1, outgoing),
        Err(Error::QueryExpired)
    ));
    assert!(db.body_backlinks_page(&target.id, 1, None).unwrap().items[0].source_trashed);
    let cursor = db
        .body_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    db.set_trashed(&source.id, false).unwrap();
    assert!(matches!(
        db.body_backlinks_page(&target.id, 1, cursor),
        Err(Error::QueryExpired)
    ));
    let cursor = db
        .body_backlinks_page(&target.id, 1, None)
        .unwrap()
        .next_cursor;
    db.save_document(
        &id(),
        SaveNote {
            note_id: source.id.clone(),
            base_revision_id: source.revision_id.clone(),
            title: "改名".into(),
            document_schema: 2,
            document: source.document.clone(),
        },
    )
    .unwrap();
    assert!(matches!(
        db.body_backlinks_page(&target.id, 1, cursor),
        Err(Error::QueryExpired)
    ));
    let historical = db
        .note_references_page(&source.id, &source.revision_id, 100, None)
        .unwrap();
    assert!(
        historical
            .items
            .iter()
            .all(|item| item.source_revision_id == source.revision_id
                && item.source_title == source.title)
    );
}

#[test]
fn long_paragraph_contexts_are_unicode_bounded_while_export_preserves_every_occurrence_and_body() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path().join("library")).unwrap();
    let target = db.create_note("目标", "正文").unwrap();
    let prefix = "知🦀".repeat(10_000);
    let (source, expected) = create_referencing_note(&mut db, &target, 450, &prefix);
    let references = db.note_references(&source.id, &source.revision_id).unwrap();
    assert_eq!(references.len(), 450);
    assert!(
        references
            .iter()
            .all(|item| item.context.chars().count() == 280 && item.context.ends_with('…'))
    );
    assert!(
        references
            .windows(2)
            .all(|pair| pair[0].context == pair[1].context)
    );
    assert!(
        serde_json::to_vec(&references).unwrap().len() < 1_000_000,
        "summary response must not duplicate the full long paragraph per occurrence"
    );
    let path = dir.path().join("export.zip");
    db.export_note_archive(&source.id, &path, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let manifest: Vec<BodyReference> =
        serde_json::from_reader(archive.by_name("references.json").unwrap()).unwrap();
    assert_eq!(
        manifest
            .iter()
            .map(|item| item.occurrence_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    let document: serde_json::Value =
        serde_json::from_reader(archive.by_name("document.json").unwrap()).unwrap();
    assert_eq!(
        document["document"],
        serde_json::to_value(&source.document).unwrap()
    );
}

#[test]
fn backlink_page_does_not_read_unrequested_source_revisions_or_sort_all_target_rows() {
    let dir = tempdir().unwrap();
    let mut db = Library::open(dir.path()).unwrap();
    let target = db.create_note("目标", "正文").unwrap();
    let (first, expected) = create_referencing_note(&mut db, &target, 100, "第一篇 ");
    let (later, _) = create_referencing_note(&mut db, &target, 1, "尚未请求的来源 ");
    let connection = rusqlite::Connection::open(dir.path().join("library.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE note_revisions SET content_hash=?1 WHERE revision_id=?2",
            rusqlite::params!["0".repeat(64), later.revision_id],
        )
        .unwrap();
    let page = db.body_backlinks_page(&target.id, 100, None).unwrap();
    assert_eq!(page.items.len(), 100);
    assert!(page.items.iter().all(|item| item.source_id == first.id));
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.occurrence_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(matches!(
        db.body_backlinks_page(&target.id, 100, page.next_cursor),
        Err(Error::IntegrityFailure)
    ));
    let plans = connection.prepare("EXPLAIN QUERY PLAN SELECT source_id,source_revision_id,occurrence_id,rowid FROM body_links INDEXED BY body_links_target WHERE target_id=?1 AND rowid>?2 ORDER BY rowid LIMIT ?3").unwrap()
        .query_map(rusqlite::params![target.id, 0, 101], |row| row.get::<_, String>(3)).unwrap()
        .collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("body_links_target") && plan.contains("rowid>")),
        "{plans:?}"
    );
    assert!(
        !plans.iter().any(|plan| plan.contains("TEMP B-TREE")),
        "{plans:?}"
    );
}
