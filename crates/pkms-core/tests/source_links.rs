use pkms_core::*;
use serde_json::json;
use std::io::{Cursor, Read};
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn target(id: &str, revision: Option<String>, block: Option<String>) -> ResolveLink {
    ResolveLink {
        target_id: id.into(),
        target_revision_id: revision,
        target_block_id: block,
    }
}
fn import(
    db: &mut Library,
    bytes: &[u8],
    previous: Option<&SourceSummary>,
) -> (SourceVersion, SourceBlock) {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: previous.map(|s| s.id.clone()),
                base_row_version: previous.map(|s| s.row_version),
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap();
    let parsed = parse_text_source(&version.id, SourceFormat::Text, bytes).unwrap();
    let block = parsed.blocks[0].clone();
    db.publish_source_parse(&job.job_id, &lease.lease_token, parsed)
        .unwrap();
    (version, block)
}

#[test]
fn staged_source_blocks_are_hidden_while_published_historical_blocks_resolve() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (version, _) = import(&mut db, b"published original", None);
    let active_parse = db
        .source(&version.source_id)
        .unwrap()
        .active_parse_id
        .unwrap();
    let staged_parse = id();
    let staged_block = id();
    let sql = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    sql.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) SELECT ?1,source_version_id,'test-staged',page_count,needs_ocr,created_at FROM source_parses WHERE id=?2", rusqlite::params![staged_parse,active_parse]).unwrap();
    sql.execute("INSERT INTO source_blocks(id,parse_id,position,text) VALUES(?1,?2,0,'unpublished private text')",rusqlite::params![staged_block,staged_parse]).unwrap();
    sql.execute("INSERT INTO source_parse_staging(parse_id,job_id,lease_token) SELECT ?1,id,?2 FROM source_jobs WHERE source_version_id=?3 LIMIT 1",rusqlite::params![staged_parse,id(),version.id]).unwrap();
    let request = target(
        &version.source_id,
        Some(version.id.clone()),
        Some(staged_block),
    );
    let hidden = db.resolve_link(request.clone()).unwrap();
    assert_eq!(hidden.state, LinkState::BlockMissing);
    assert!(hidden.block_text.is_none());
    let jump = hidden.source_target.unwrap();
    assert_ne!(jump.parse_id.as_deref(), Some(staged_parse.as_str()));
    assert!(jump.block_offset.is_none());
    // Removing the publication barrier makes this an inactive but published
    // historical generation; its immutable block remains a valid target.
    sql.execute(
        "DELETE FROM source_parse_staging WHERE parse_id=?1",
        [&staged_parse],
    )
    .unwrap();
    let published = db.resolve_link(request).unwrap();
    assert_eq!(published.state, LinkState::Available);
    assert_eq!(
        published.block_text.as_deref(),
        Some("unpublished private text")
    );
    assert_eq!(
        published.source_target.unwrap().parse_id,
        Some(staged_parse)
    );
    assert_eq!(
        db.source(&version.source_id).unwrap().active_parse_id,
        Some(active_parse)
    );
}

#[test]
fn annotation_references_preserve_version_and_backlinks_through_rename_and_trash() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let (first, block) = import(&mut db, "原始引文".as_bytes(), None);
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: first.id.clone(),
                    block_id: block.id.clone(),
                    start_codepoint: 0,
                    end_codepoint: 4,
                    quote: "原始引文".into(),
                    prefix: "".into(),
                    suffix: "".into(),
                },
                comment: "理解".into(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let doc = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{"target_id":annotation.id,"occurrence_id":id(),"label":"原始引文"}}]}]})).unwrap();
    let note = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "引用笔记".into(),
                document_schema: 2,
                document: doc,
            },
        )
        .unwrap()
        .note;
    assert_eq!(
        db.body_backlinks(&annotation.id).unwrap()[0].source_id,
        note.id
    );
    let archive = root.path().join("reference.zip");
    let report = db
        .export_note_archive(&note.id, &archive, ExportFormat::Html, &mut |_| Ok(()))
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut html = String::new();
    zip.by_name(&report.document_path)
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert!(html.contains(&format!("pkms://object/{}", annotation.id)));
    let references: Vec<BodyReference> =
        serde_json::from_reader(zip.by_name("references.json").unwrap()).unwrap();
    assert_eq!(references[0].target_id, annotation.id);
    let source = db.source(&first.source_id).unwrap();
    db.update_source_metadata(
        &id(),
        UpdateSourceMetadata {
            source_id: source.id.clone(),
            base_row_version: source.row_version,
            display_title: "重命名资料".into(),
        },
    )
    .unwrap();
    let source = db.source(&first.source_id).unwrap();
    let (second, _) = import(&mut db, "更新资料".as_bytes(), Some(&source));
    let pinned_doc = serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{"target_id":source.id,"target_revision_id":first.id,"target_block_id":block.id,"occurrence_id":id(),"label":"固定资料版本"}}]}]})).unwrap();
    let pinned_note = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "版本证据".into(),
                document_schema: 2,
                document: pinned_doc,
            },
        )
        .unwrap()
        .note;
    let pinned = db
        .note_references(&pinned_note.id, &pinned_note.revision_id)
        .unwrap()
        .remove(0);
    assert_eq!(pinned.target_revision_id, Some(first.id.clone()));
    let pinned_target = db
        .resolve_link(ResolveLink {
            target_id: pinned.target_id,
            target_block_id: pinned.target_block_id,
            target_revision_id: pinned.target_revision_id,
        })
        .unwrap();
    assert_eq!(pinned_target.state, LinkState::Available);
    assert_eq!(pinned_target.resolved_revision_id, Some(first.id.clone()));
    assert_eq!(pinned_target.block_text.as_deref(), Some("原始引文"));
    // Simulate a later parser generation for the same immutable original.
    // A frozen block reference must still select its owning historical parse.
    let old_parse = pinned_target.source_target.unwrap().parse_id.unwrap();
    let new_parse = id();
    let sql = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    sql.execute("INSERT INTO source_parses(id,source_version_id,parser_version,page_count,needs_ocr,created_at) SELECT ?1,source_version_id,'test-next',page_count,needs_ocr,created_at FROM source_parses WHERE id=?2", rusqlite::params![new_parse, old_parse]).unwrap();
    sql.execute(
        "INSERT INTO source_blocks(id,parse_id,position,text) VALUES(?1,?2,0,'new parse text')",
        rusqlite::params![id(), new_parse],
    )
    .unwrap();
    sql.execute(
        "UPDATE source_active_parses SET parse_id=?1 WHERE source_version_id=?2",
        rusqlite::params![new_parse, first.id],
    )
    .unwrap();
    let historical = db
        .resolve_link(target(
            &source.id,
            Some(first.id.clone()),
            Some(block.id.clone()),
        ))
        .unwrap();
    assert_eq!(historical.source_target.unwrap().parse_id, Some(old_parse));
    assert_eq!(historical.block_text.as_deref(), Some("原始引文"));
    let resolved = db.resolve_link(target(&annotation.id, None, None)).unwrap();
    assert_eq!(resolved.target_kind, Some(ObjectKind::Annotation));
    assert_eq!(resolved.state, LinkState::Available);
    assert_eq!(resolved.title.as_deref(), Some("重命名资料"));
    assert_eq!(
        resolved.current_revision_id.as_deref(),
        Some(second.id.as_str())
    );
    let jump = resolved.source_target.unwrap();
    assert_eq!(jump.source_version_id, first.id);
    assert!(matches!(jump.locator_status, Some(LocatorStatus::Exact)));
    assert_eq!(
        db.source_text_page(
            &first.id,
            jump.parse_id.as_deref(),
            jump.block_offset.unwrap(),
            1
        )
        .unwrap()
        .blocks[0]
            .id,
        block.id
    );
    assert_eq!(
        db.resolve_link(target(&annotation.id, Some(second.id.clone()), None))
            .unwrap()
            .state,
        LinkState::RevisionMissing
    );
    assert_eq!(
        db.resolve_link(target(&annotation.id, None, Some(id())))
            .unwrap()
            .state,
        LinkState::BlockMissing
    );
    let current = db.resolve_link(target(&source.id, None, None)).unwrap();
    assert_eq!(current.resolved_revision_id, Some(second.id));
    let (foreign, _) = import(&mut db, b"another source", None);
    assert_eq!(
        db.resolve_link(target(&source.id, Some(foreign.id), None))
            .unwrap()
            .state,
        LinkState::RevisionMissing
    );
    let old = db
        .resolve_link(target(&source.id, Some(first.id.clone()), Some(block.id)))
        .unwrap();
    assert_eq!(old.state, LinkState::Available);
    assert_eq!(old.block_text.as_deref(), Some("原始引文"));
    assert_eq!(
        db.resolve_link(target(&source.id, Some(id()), None))
            .unwrap()
            .state,
        LinkState::RevisionMissing
    );
    assert_eq!(
        db.resolve_link(target(&source.id, None, Some(id())))
            .unwrap()
            .state,
        LinkState::BlockMissing
    );
    db.set_annotation_trashed(
        &id(),
        SetObjectFlag {
            object_id: annotation.id.clone(),
            base_row_version: annotation.row_version,
            value: true,
        },
    )
    .unwrap();
    assert_eq!(
        db.resolve_link(target(&annotation.id, None, None))
            .unwrap()
            .state,
        LinkState::TargetTrashed
    );
    let source = db.source(&source.id).unwrap();
    db.set_source_trashed(
        &id(),
        SetObjectFlag {
            object_id: source.id.clone(),
            base_row_version: source.row_version,
            value: true,
        },
    )
    .unwrap();
    assert_eq!(
        db.resolve_link(target(&source.id, Some(first.id), None))
            .unwrap()
            .state,
        LinkState::TargetTrashed
    );
    assert_eq!(db.body_backlinks(&annotation.id).unwrap().len(), 1);
}
