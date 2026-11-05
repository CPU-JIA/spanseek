use pkms_core::*;
use rusqlite::Connection;
use serde_json::json;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn create(text: &str) -> CreateNote {
    CreateNote {
        title: text.into(),
        document_schema: DOCUMENT_SCHEMA,
        document: NoteDocument::from_text(id(), text),
        collection_id: None,
        base_collection_row_version: None,
    }
}
fn trash(db: &mut Library, object_id: &str) {
    let base_row_version = db.object_metadata(object_id).unwrap().row_version;
    db.set_object_trashed(
        &id(),
        SetObjectFlag {
            object_id: object_id.into(),
            base_row_version,
            value: true,
        },
    )
    .unwrap();
}
fn preview(db: &Library, object_ids: &[String]) -> TrashPreview {
    db.preview_trash_delete(TrashPreviewRequest {
        selection: TrashSelection::Objects {
            object_ids: object_ids.to_vec(),
        },
    })
    .unwrap()
}
fn deletion(snapshot: &TrashPreview) -> DeleteTrash {
    DeleteTrash {
        selection: snapshot.selection.clone(),
        snapshot_token: snapshot.snapshot_token.clone(),
    }
}
fn scalar(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn target(object_id: &str) -> ResolveLink {
    ResolveLink {
        target_id: object_id.into(),
        target_block_id: None,
        target_revision_id: None,
    }
}
fn input() -> ImportSource {
    ImportSource {
        input_token: id(),
        original_name: "synthetic.txt".into(),
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}
fn capture(db: &mut Library, command: &str, input: ImportSource, text: &[u8]) -> SourceVersion {
    let job = db.accept_source_import(command, input).unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let v = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut Cursor::new(text),
            &mut |_| Ok(()),
        )
        .unwrap();
    if db.source_job(&job.job_id).unwrap().status == JobStatus::Running {
        db.publish_source_parse(
            &job.job_id,
            &lease.lease_token,
            parse_text_source(&v.id, SourceFormat::Text, text).unwrap(),
        )
        .unwrap();
    }
    v
}
fn annotate(db: &mut Library, version: &SourceVersion) -> Annotation {
    let b = db
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    db.create_annotation(
        &id(),
        CreateAnnotation {
            locator: SourceLocator::Text {
                source_version_id: version.id.clone(),
                block_id: b.id,
                start_codepoint: 0,
                end_codepoint: b.text.chars().count() as u32,
                quote: b.text,
                prefix: String::new(),
                suffix: String::new(),
            },
            comment: "独立旁注".into(),
        },
    )
    .unwrap()
    .annotation
    .unwrap()
}

// Controlled core worker completion only; this fixture performs no model call.
fn completed_chat(db: &mut Library) -> String {
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: 0,
            profile: Some(GenerationProfile {
                channel_id: None,
                capabilities: None,
                chat_token_limit: None,
                provider: "openai_chat".into(),
                endpoint: "https://example.test".into(),
                model: "synthetic".into(),
                model_digest: String::new(),
                config_revision: 1,
                temperature: 0.0,
                context_tokens: 32768,
            }),
        },
    )
    .unwrap();
    let c = db
        .create_chat_conversation(
            &id(),
            CreateConversation {
                title: "独立对话快照".into(),
            },
        )
        .unwrap()
        .conversation;
    let run = db
        .accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: c.id,
                base_row_version: c.row_version,
                text: "synthetic request".into(),
                purpose: ChatPurpose::Review,
                materials: vec![],
                scope: SearchScope::default(),
                use_knowledge_tools: false,
                options: None,
            },
        )
        .unwrap()
        .run;
    let lease = db.claim_chat_run(&run.id).unwrap().unwrap();
    db.append_chat_delta(
        &run.id,
        &lease.lease_token,
        lease.run.last_event_seq + 1,
        "独立保存的对话文字",
    )
    .unwrap();
    db.finish_chat_run(
        &run.id,
        &lease.lease_token,
        ChatRunStatus::Completed,
        None,
        ChatUsage {
            input_tokens: None,
            output_tokens: None,
        },
        Some("stop"),
    )
    .unwrap();
    run.id
}

#[test]
fn deletion_removes_note_history_dependencies_but_keeps_independent_snapshots_and_shared_blobs() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("library")).unwrap();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let blob = db
        .import_attachment(&id(), "shared.png", png.get_ref())
        .unwrap()
        .blob;
    let command = id();
    let mut request = create("删除专属正文");
    request.document = serde_json::from_value(json!({"type":"doc","content":[
        {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"删除专属正文"}]},
        {"type":"image","attrs":{"id":id(),"blob_hash":blob.hash,"original_name":"shared.png","alt":"共享图"}}
    ]})).unwrap();
    let removed = db.create_document(&command, request.clone()).unwrap().note;
    let kept = db
        .copy_document(
            &id(),
            CopyNote {
                note_id: removed.id.clone(),
                source_revision_id: removed.revision_id.clone(),
                title: "独立副本".into(),
            },
        )
        .unwrap()
        .note;
    let linked = db.create_document(&id(), CreateNote { document: serde_json::from_value(json!({"type":"doc","content":[
        {"type":"paragraph","attrs":{"id":id()},"content":[{"type":"noteReference","attrs":{"target_id":removed.id,"target_revision_id":removed.revision_id,"target_block_id":removed.document.content[0].attrs.id,"occurrence_id":id(),"label":"独立保存的引文快照"}}]}
    ]})).unwrap(), ..create("引用者") }).unwrap().note;
    let collection = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "保持集合".into(),
                overview_note_id: Some(removed.id.clone()),
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .collection;
    db.add_to_collection(&collection.id, &removed.id).unwrap();
    db.add_to_collection(&collection.id, &kept.id).unwrap();
    let discarded_collection = db.create_collection("仅删除集合").unwrap();
    db.add_to_collection(&discarded_collection.id, &kept.id)
        .unwrap();
    let tags_command = id();
    let tags_request = AssignTag {
        targets: [&removed.id, &kept.id]
            .into_iter()
            .map(|o| MetadataTarget {
                object_id: o.clone(),
                base_row_version: db.object_metadata(o).unwrap().row_version,
            })
            .collect(),
        tag: TagSelector::Name {
            name: "共享标签".into(),
        },
    };
    db.assign_tag(&tags_command, tags_request.clone()).unwrap();
    let review_command = id();
    let review_request = AddReview {
        target: target(&removed.id),
        base_row_version: None,
    };
    db.add_review(&review_command, review_request.clone())
        .unwrap();
    let proposal_command = id();
    let chat_run = completed_chat(&mut db);
    let proposal_request = CreateProposal {
        answer_id: None,
        chat_run_id: Some(chat_run.clone()),
        external_request_id: None,
        note_id: Some(removed.id.clone()),
        base_revision_id: Some(removed.revision_id.clone()),
        title: "待应用建议".into(),
        document_schema: DOCUMENT_SCHEMA,
        document: NoteDocument::from_text(id(), "提案正文"),
    };
    let proposal = db
        .create_proposal(&proposal_command, proposal_request.clone())
        .unwrap()
        .proposal;
    db.apply_proposal(&id(), &proposal.id).unwrap();
    trash(&mut db, &removed.id);
    trash(&mut db, &discarded_collection.id);
    let snapshot = preview(&db, &[removed.id.clone(), discarded_collection.id.clone()]);
    let collection_version = db.object_metadata(&collection.id).unwrap().row_version;
    let result = db.delete_trash(&id(), deletion(&snapshot)).unwrap();
    assert_eq!(result.deleted_objects.len(), 2);
    assert_eq!(db.note(&kept.id).unwrap().document, kept.document);
    assert_eq!(db.note(&linked.id).unwrap().document, linked.document);
    assert!(matches!(db.note(&removed.id), Err(Error::NotFound)));
    assert!(matches!(
        db.revision(&removed.id, &removed.revision_id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.create_document(&command, request),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.assign_tag(&tags_command, tags_request),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.add_review(&review_command, review_request),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.create_proposal(&proposal_command, proposal_request),
        Err(Error::NotFound)
    ));
    assert_eq!(
        db.resolve_link(target(&removed.id)).unwrap().state,
        LinkState::TargetMissing
    );
    assert!(
        db.collection_get(&collection.id)
            .unwrap()
            .overview_note_id
            .is_none()
    );
    assert_eq!(
        db.object_metadata(&collection.id).unwrap().row_version,
        collection_version + 1
    );
    assert_eq!(db.object_metadata(&kept.id).unwrap().tags.len(), 1);
    assert!(db.attachment(&kept.revision_id, &blob.hash).is_ok());
    let raw = Connection::open(root.path().join("library/library.sqlite3")).unwrap();
    for table in [
        "proposals",
        "proposal_blobs",
        "review_entries",
        "review_events",
    ] {
        assert_eq!(scalar(&raw, &format!("SELECT count(*) FROM {table}")), 0);
    }
    for (table, column) in [
        ("notes", "id"),
        ("note_revisions", "note_id"),
        ("blocks", "note_id"),
        ("note_previews", "note_id"),
        ("note_index_queue", "note_id"),
        ("note_fts", "note_id"),
        ("index_dirty_objects", "object_id"),
    ] {
        assert_eq!(
            raw.query_row(
                &format!("SELECT count(*) FROM {table} WHERE {column}=?1"),
                [&removed.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    assert_eq!(scalar(&raw, "SELECT count(*) FROM body_links"), 1);
    assert_eq!(
        db.chat_run(&chat_run).unwrap().status,
        ChatRunStatus::Completed
    );
    assert_eq!(scalar(&raw, "SELECT count(*) FROM blobs"), 1);
    assert_eq!(
        scalar(&raw, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
    let backup = root.path().join("after-deletion.pkmsbackup");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let restored_path = root.path().join("restored");
    prepare_restore(&backup, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(restored_path).unwrap();
    assert!(matches!(restored.note(&removed.id), Err(Error::NotFound)));
    assert_eq!(restored.note(&kept.id).unwrap().document, kept.document);
    assert_eq!(
        restored
            .permanently_deleted_ids(std::slice::from_ref(&removed.id))
            .unwrap(),
        [removed.id]
    );
}

#[test]
fn deletion_rolls_back_every_object_on_failure_and_replays_after_lost_reply_and_restart() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let a = db.create_note("删除甲", "甲正文").unwrap();
    let b = db.create_note("删除乙", "乙正文").unwrap();
    trash(&mut db, &a.id);
    trash(&mut db, &b.id);
    let snapshot = preview(&db, &[a.id.clone(), b.id.clone()]);
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER synthetic_purge_failure BEFORE DELETE ON objects BEGIN SELECT RAISE(ABORT,'synthetic deletion failure'); END;").unwrap();
    let command = id();
    assert!(matches!(
        db.delete_trash(&command, deletion(&snapshot)),
        Err(Error::Database(_))
    ));
    assert!(db.note(&a.id).unwrap().trashed && db.note(&b.id).unwrap().trashed);
    assert_eq!(scalar(&raw, "SELECT count(*) FROM object_tombstones"), 0);
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM command_receipts WHERE command_id=?1",
            [&command],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    raw.execute_batch("DROP TRIGGER synthetic_purge_failure;")
        .unwrap();
    let committed = db.delete_trash(&command, deletion(&snapshot)).unwrap();
    drop(db);
    drop(raw);
    let mut reopened = Library::open_existing(root.path()).unwrap();
    let replayed = reopened
        .delete_trash(&command, deletion(&snapshot))
        .unwrap();
    assert_eq!(committed.deleted_objects, replayed.deleted_objects);
    assert_eq!(committed.receipt.change_seq, replayed.receipt.change_seq);
    assert!(replayed.receipt.replayed);
    assert!(matches!(
        reopened.delete_trash(
            &command,
            DeleteTrash {
                selection: TrashSelection::All,
                ..deletion(&snapshot)
            }
        ),
        Err(Error::IdempotencyMismatch)
    ));
}

#[test]
fn source_deletion_cascades_annotations_but_preserves_shared_original_and_dedup_identity() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("library")).unwrap();
    let import_command = id();
    let original_input = input();
    let source = capture(
        &mut db,
        &import_command,
        original_input.clone(),
        b"shared original",
    );
    let other = capture(&mut db, &id(), input(), b"second original");
    let mut add_version = input();
    add_version.add_version_to = Some(other.source_id.clone());
    add_version.base_row_version = Some(db.source(&other.source_id).unwrap().row_version);
    let shared = capture(&mut db, &id(), add_version, b"shared original");
    let first_annotation = annotate(&mut db, &source);
    let second_annotation = annotate(&mut db, &shared);
    let external = root.path().join("user-original.txt");
    std::fs::write(&external, b"shared original").unwrap();
    let backup = root.path().join("before-delete.pkmsbackup");
    db.create_backup(&backup, &mut |_| Ok(())).unwrap();
    let backup_bytes = std::fs::read(&backup).unwrap();
    trash(&mut db, &source.source_id);
    let snapshot = preview(&db, std::slice::from_ref(&source.source_id));
    assert_eq!(
        (
            snapshot.selected_count,
            snapshot.dependent_count,
            snapshot.total_count
        ),
        (1, 1, 2)
    );
    db.delete_trash(&id(), deletion(&snapshot)).unwrap();
    assert!(matches!(
        db.resolve_annotation(&first_annotation.id),
        Err(Error::NotFound)
    ));
    assert!(db.resolve_annotation(&second_annotation.id).is_ok());
    assert!(matches!(
        db.accept_source_import(&import_command, original_input),
        Err(Error::NotFound)
    ));
    assert_eq!(
        db.source(&other.source_id).unwrap().current_version_id,
        shared.id
    );
    let duplicate = capture(&mut db, &id(), input(), b"shared original");
    assert_eq!(duplicate.source_id, other.source_id);
    assert_eq!(std::fs::read(&external).unwrap(), b"shared original");
    assert_eq!(std::fs::read(&backup).unwrap(), backup_bytes);
    let blob_path = root
        .path()
        .join("library/blobs/sha256")
        .join(&source.blob_hash[..2])
        .join(&source.blob_hash);
    assert_eq!(std::fs::read(blob_path).unwrap(), b"shared original");
    let raw = Connection::open(root.path().join("library/library.sqlite3")).unwrap();
    for sql in [
        "SELECT count(*) FROM source_versions WHERE source_id=?1",
        "SELECT count(*) FROM source_jobs WHERE source_id=?1",
        "SELECT count(*) FROM source_fts WHERE source_id=?1",
        "SELECT count(*) FROM source_title_fts WHERE source_id=?1",
    ] {
        assert_eq!(
            raw.query_row(sql, [&source.source_id], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        scalar(&raw, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
    trash(&mut db, &second_annotation.id);
    db.delete_trash(
        &id(),
        deletion(&preview(&db, std::slice::from_ref(&second_annotation.id))),
    )
    .unwrap();
    assert!(db.source(&other.source_id).is_ok());
    db.create_backup(root.path().join("after.pkmsbackup"), &mut |_| Ok(()))
        .unwrap();
}

#[test]
fn all_is_frozen_and_never_deletes_newly_trashed_or_restored_items_without_confirmation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let a = db.create_note("甲", "a").unwrap();
    let b = db.create_note("乙", "b").unwrap();
    trash(&mut db, &a.id);
    let snapshot = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::All,
        })
        .unwrap();
    trash(&mut db, &b.id);
    assert!(matches!(
        db.delete_trash(&id(), deletion(&snapshot)),
        Err(Error::QueryExpired)
    ));
    let fresh = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::All,
        })
        .unwrap();
    let base = db.object_metadata(&a.id).unwrap().row_version;
    db.set_object_trashed(
        &id(),
        SetObjectFlag {
            object_id: a.id.clone(),
            base_row_version: base,
            value: false,
        },
    )
    .unwrap();
    assert!(matches!(
        db.delete_trash(&id(), deletion(&fresh)),
        Err(Error::QueryExpired)
    ));
    assert!(db.contains_object(&a.id).unwrap() && db.contains_object(&b.id).unwrap());
    let page = db
        .trash_page(TrashPageRequest {
            limit: 1,
            cursor: None,
            query: None,
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].object_id, b.id);
    let fresh = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::All,
        })
        .unwrap();
    db.delete_trash(&id(), deletion(&fresh)).unwrap();
    assert!(!db.note(&a.id).unwrap().trashed);
    assert_eq!(
        db.trash_page(TrashPageRequest {
            limit: 10,
            cursor: None,
            query: None
        })
        .unwrap()
        .total,
        0
    );
    let empty = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::All,
        })
        .unwrap();
    assert!(
        db.delete_trash(&id(), deletion(&empty))
            .unwrap()
            .deleted_objects
            .is_empty()
    );
    assert!(matches!(
        db.preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::Objects {
                object_ids: vec![a.id]
            }
        }),
        Err(Error::QueryExpired)
    ));
}

#[test]
fn deletion_removes_active_and_inflight_index_generations_and_invalidates_worker_retries() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("唯一检索针", "删除之后不得命中").unwrap();
    let profile = db
        .register_embedding_profile(
            &id(),
            EmbeddingProfileSpec {
                endpoint: None,
                model: "controlled-no-model-call".into(),
                revision: "fixture".into(),
                dimensions: 3,
                normalized: true,
                query_prefix: String::new(),
                passage_prefix: String::new(),
                tokenizer: "synthetic".into(),
                chunker: "complete-block".into(),
                max_tokens: 512,
                overlap_tokens: 0,
                provider: None,
                output_dimensions: None,
            },
        )
        .unwrap()
        .profile;
    let input = db.index_input(&note.id).unwrap();
    let plan = IndexPlan {
        profile_id: profile.id,
        object_id: note.id.clone(),
        content_revision: input.content_revision,
        metadata_revision: input.metadata_revision,
        parser_revision: input.parser_revision,
        chunks: input
            .blocks
            .iter()
            .map(|b| IndexChunkInput {
                chunk_id: id(),
                block_id: b.target.block_id.clone(),
                start_codepoint: 0,
                end_codepoint: b.text.chars().count() as u32,
                text: b.text.clone(),
                token_count: 10,
            })
            .collect(),
    };
    let index_command = id();
    let lease = db
        .begin_index_generation(&index_command, plan.clone())
        .unwrap();
    db.publish_index_generation(IndexPublish {
        generation_id: lease.generation.id.clone(),
        lease_token: lease.lease_token.clone(),
        profile_id: lease.generation.profile_id.clone(),
        chunk_count: lease.generation.chunk_count,
    })
    .unwrap();
    let completed = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: lease.generation.profile_id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let complete_lease = db.claim_index_rebuild(&completed.id).unwrap().unwrap();
    db.checkpoint_index_rebuild(
        &completed.id,
        &complete_lease.lease_token,
        &note.id,
        Some(&lease.generation.id),
        None,
    )
    .unwrap();
    db.finish_index_rebuild(&completed.id, &complete_lease.lease_token)
        .unwrap();
    let running = db
        .accept_index_rebuild(
            &id(),
            RebuildRequest {
                profile_id: lease.generation.profile_id.clone(),
                scope: SearchScope::default(),
            },
        )
        .unwrap()
        .job;
    let running_lease = db.claim_index_rebuild(&running.id).unwrap().unwrap();
    db.checkpoint_index_rebuild(
        &running.id,
        &running_lease.lease_token,
        &note.id,
        Some(&lease.generation.id),
        None,
    )
    .unwrap();
    let inflight = db.begin_index_generation(&id(), plan.clone()).unwrap();
    trash(&mut db, &note.id);
    db.delete_trash(
        &id(),
        deletion(&preview(&db, std::slice::from_ref(&note.id))),
    )
    .unwrap();
    assert!(db.search("唯一检索针", 10).unwrap().is_empty());
    assert!(db.active_index_generation_ids().unwrap().is_empty());
    assert!(matches!(
        db.index_generation(&lease.generation.id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        db.begin_index_generation(&index_command, plan),
        Err(Error::NotFound)
    ));
    assert!(
        db.publish_index_generation(IndexPublish {
            generation_id: inflight.generation.id,
            lease_token: inflight.lease_token,
            profile_id: inflight.generation.profile_id,
            chunk_count: inflight.generation.chunk_count
        })
        .is_err()
    );
    let raw = Connection::open(root.path().join("library.sqlite3")).unwrap();
    for table in [
        "index_generations",
        "index_chunks",
        "active_index_generations",
        "index_dirty_objects",
    ] {
        assert_eq!(scalar(&raw, &format!("SELECT count(*) FROM {table}")), 0);
    }
    assert_eq!(db.index_rebuild_job(&completed.id).unwrap().processed, 0);
    assert!(
        db.index_rebuild_job(&completed.id)
            .unwrap()
            .after_object_id
            .is_none()
    );
    assert_eq!(
        db.index_rebuild_job(&running.id).unwrap().status,
        RebuildStatus::Canceled
    );
    assert!(
        db.heartbeat_index_rebuild(&running.id, &running_lease.lease_token)
            .is_err()
    );
    db.create_backup(
        root.path().join("after-index-purge.pkmsbackup"),
        &mut |_| Ok(()),
    )
    .unwrap();
}

#[test]
fn trash_cursor_and_confirmation_are_bound_to_the_physical_library_copy() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("original")).unwrap();
    let a = db.create_note("甲", "a").unwrap();
    let b = db.create_note("乙", "b").unwrap();
    trash(&mut db, &a.id);
    trash(&mut db, &b.id);
    let first_page = db
        .trash_page(TrashPageRequest {
            limit: 1,
            cursor: None,
            query: None,
        })
        .unwrap();
    assert_eq!(first_page.total, 2);
    assert_eq!(first_page.items.len(), 1);
    let cursor = first_page.next_cursor.unwrap();
    let second_page = db
        .trash_page(TrashPageRequest {
            limit: 1,
            cursor: Some(cursor.clone()),
            query: None,
        })
        .unwrap();
    assert_eq!(second_page.total, 2);
    assert_eq!(second_page.items.len(), 1);
    assert!(second_page.next_cursor.is_none());
    let mut expected = [a.id.as_str(), b.id.as_str()];
    expected.sort_unstable();
    assert_eq!(first_page.items[0].object_id, expected[0]);
    assert_eq!(second_page.items[0].object_id, expected[1]);
    let snapshot = db
        .preview_trash_delete(TrashPreviewRequest {
            selection: TrashSelection::All,
        })
        .unwrap();
    let package = root.path().join("copy.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let copied = root.path().join("copy");
    prepare_restore(&package, &copied, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut other = Library::open_existing(copied).unwrap();
    assert!(matches!(
        other.trash_page(TrashPageRequest {
            limit: 1,
            cursor: Some(cursor),
            query: None
        }),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        other.delete_trash(&id(), deletion(&snapshot)),
        Err(Error::QueryExpired)
    ));
    assert!(other.contains_object(&a.id).unwrap());
}
