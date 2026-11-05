//! Core fixtures simulate worker completion; real model verification belongs to the Tauri flow.
use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn material(db: &mut Library) -> SearchEvidence {
    let note = db
        .create_note(
            "知识应用材料",
            "知识应用材料记录了两个发现和一个待验证的问题。",
        )
        .unwrap();
    db.keyword_search(&SearchQuery {
        query: "知识应用".into(),
        mode: SearchMode::Lexical,
        scope: SearchScope {
            object_ids: vec![note.id],
            ..Default::default()
        },
        limit: 1,
        minimum_cosine: None,
    })
    .unwrap()
    .items
    .remove(0)
}

#[test]
fn selected_materials_are_version_bound_and_report_partial_or_unavailable_content() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let short = db.create_note("短材料", "中文材料🙂完整正文").unwrap();
    let long = db.create_note("长材料", &"资料🙂".repeat(50_000)).unwrap();
    let targets = vec![
        KnowledgeTaskMaterialTarget {
            object_id: short.id.clone(),
            content_revision: Some(short.revision_id.clone()),
        },
        KnowledgeTaskMaterialTarget {
            object_id: long.id.clone(),
            content_revision: Some(long.revision_id.clone()),
        },
    ];
    let loaded = db
        .knowledge_task_materials(KnowledgeTaskMaterialsRequest { targets })
        .unwrap();
    assert_eq!(
        loaded.coverage[0].status,
        KnowledgeTaskMaterialStatus::Complete
    );
    assert_eq!(loaded.materials[0].text, "中文材料🙂完整正文");
    assert_eq!(
        loaded.coverage[1].status,
        KnowledgeTaskMaterialStatus::Limited
    );
    assert!(loaded.materials.iter().map(|e| e.text.len()).sum::<usize>() <= 256 * 1024);
    for evidence in &loaded.materials {
        assert!(
            db.validate_search_evidence(
                evidence,
                &SearchScope {
                    object_ids: vec![evidence.target.object_id.clone()],
                    ..Default::default()
                }
            )
            .unwrap()
        );
    }
    let changed = db
        .knowledge_task_materials(KnowledgeTaskMaterialsRequest {
            targets: vec![KnowledgeTaskMaterialTarget {
                object_id: short.id.clone(),
                content_revision: Some(id()),
            }],
        })
        .unwrap();
    assert_eq!(
        changed.coverage[0].status,
        KnowledgeTaskMaterialStatus::Changed
    );
    assert!(changed.materials.is_empty());
    let metadata = db.object_metadata(&short.id).unwrap();
    db.set_object_search_enabled(
        &id(),
        SetObjectFlag {
            object_id: short.id.clone(),
            base_row_version: metadata.row_version,
            value: false,
        },
    )
    .unwrap();
    let disabled = db
        .knowledge_task_materials(KnowledgeTaskMaterialsRequest {
            targets: vec![
                KnowledgeTaskMaterialTarget {
                    object_id: short.id,
                    content_revision: None,
                },
                KnowledgeTaskMaterialTarget {
                    object_id: id(),
                    content_revision: None,
                },
            ],
        })
        .unwrap();
    assert_eq!(
        disabled.coverage[0].status,
        KnowledgeTaskMaterialStatus::SearchDisabled
    );
    assert_eq!(
        disabled.coverage[1].status,
        KnowledgeTaskMaterialStatus::Missing
    );
    assert!(disabled.materials.is_empty());
}

#[test]
fn an_explicit_annotation_grants_only_its_quote_and_can_start_a_knowledge_task() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let text = "允许使用的摘录。没有选择的其他正文。";
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "任务来源.txt".into(),
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    let version = db
        .capture_source_original(
            &job.job_id,
            &lease.lease_token,
            &mut std::io::Cursor::new(text.as_bytes()),
            &mut |_| Ok(()),
        )
        .unwrap();
    db.publish_source_parse(
        &job.job_id,
        &lease.lease_token,
        parse_text_source(&version.id, SourceFormat::Text, text.as_bytes()).unwrap(),
    )
    .unwrap();
    let block = db
        .source_text_page(&version.id, None, 0, 1)
        .unwrap()
        .blocks
        .remove(0);
    let quote = "允许使用的摘录。";
    let annotation = db
        .create_annotation(
            &id(),
            CreateAnnotation {
                locator: SourceLocator::Text {
                    source_version_id: version.id.clone(),
                    block_id: block.id,
                    start_codepoint: 0,
                    end_codepoint: quote.chars().count() as u32,
                    quote: quote.into(),
                    prefix: String::new(),
                    suffix: "没有选择的其他正文。".into(),
                },
                comment: String::new(),
            },
        )
        .unwrap()
        .annotation
        .unwrap();
    let loaded = db
        .knowledge_task_materials(KnowledgeTaskMaterialsRequest {
            targets: vec![KnowledgeTaskMaterialTarget {
                object_id: annotation.id.clone(),
                content_revision: Some(version.id),
            }],
        })
        .unwrap();
    assert_eq!(
        loaded.coverage[0].status,
        KnowledgeTaskMaterialStatus::Complete
    );
    assert_eq!(loaded.materials.len(), 1);
    assert_eq!(loaded.materials[0].text, quote);
    let request = input(&db, loaded.materials[0].clone());
    let task = db.start_knowledge_task(&id(), request).unwrap().task;
    assert_eq!(task.run.request.scope.object_ids, vec![annotation.id]);
    assert!(task.materials_current);
}

#[test]
fn a_complete_selection_that_fits_is_not_cut_to_an_artificial_per_object_share() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let body = "文".repeat(60_000);
    let large = db.create_note("较长材料", &body).unwrap();
    let small = db.create_note("短材料", "小段正文").unwrap();
    let loaded = db
        .knowledge_task_materials(KnowledgeTaskMaterialsRequest {
            targets: vec![
                KnowledgeTaskMaterialTarget {
                    object_id: large.id,
                    content_revision: None,
                },
                KnowledgeTaskMaterialTarget {
                    object_id: small.id,
                    content_revision: None,
                },
            ],
        })
        .unwrap();
    assert!(
        loaded
            .coverage
            .iter()
            .all(|item| item.status == KnowledgeTaskMaterialStatus::Complete)
    );
    assert_eq!(loaded.materials[0].text, body);
    assert_eq!(loaded.materials[1].text, "小段正文");
}
fn input(db: &Library, evidence: SearchEvidence) -> StartKnowledgeTask {
    StartKnowledgeTask {
        title: "阅读材料后的下一步".into(),
        instructions: "根据材料形成有来源的输出，并明确待核实项。".into(),
        template: None,
        period: Some(KnowledgeTaskPeriod {
            start: "2026-11-01".into(),
            end: "2026-11-07".into(),
        }),
        materials: vec![evidence],
        generation_settings_version: db.generation_settings().unwrap().row_version,
        options: None,
        language: "zh-CN".into(),
        retry_of: None,
    }
}
fn page() -> KnowledgeTasksRequest {
    KnowledgeTasksRequest {
        template_id: None,
        page: 1,
        limit: 10,
        query: None,
    }
}
fn template(db: &mut Library, name: &str, instructions: &str) -> KnowledgeTaskTemplate {
    db.save_knowledge_task_template(
        &id(),
        SaveKnowledgeTaskTemplate {
            id: None,
            base_row_version: None,
            name: name.into(),
            description: "用户自定义场景".into(),
            instructions: instructions.into(),
        },
    )
    .unwrap()
    .template
}
fn model(db: &mut Library) {
    let version = db.generation_settings().unwrap().row_version;
    db.set_generation_profile(
        &id(),
        SetGenerationProfile {
            base_row_version: version,
            profile: Some(GenerationProfile {
                channel_id: None,
                capabilities: None,
                provider: "openai_chat".into(),
                endpoint: "https://example.test".into(),
                model: "unit-fixture".into(),
                model_digest: String::new(),
                config_revision: 1,
                temperature: 0.0,
                context_tokens: 32768,
                chat_token_limit: None,
            }),
        },
    )
    .unwrap();
}
fn finish(db: &mut Library, id: &str) {
    let lease = db.claim_chat_run(id).unwrap().unwrap();
    db.append_chat_delta(
        id,
        &lease.lease_token,
        lease.run.last_event_seq + 1,
        "# 结果\n材料记载了两个发现。[Material 1]",
    )
    .unwrap();
    db.finish_chat_run(
        id,
        &lease.lease_token,
        ChatRunStatus::Completed,
        None,
        ChatUsage {
            input_tokens: Some(12),
            output_tokens: Some(24),
        },
        Some("stop"),
    )
    .unwrap();
}
fn proposal(db: &mut Library, run: &str) -> ProposalRecord {
    db.create_proposal(&id(),CreateProposal { answer_id:None,chat_run_id:Some(run.into()),external_request_id:None,note_id:None,base_revision_id:None,title:"审阅后的结果".into(),document_schema:3,
        document:serde_json::from_value(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"id":id()},"content":[{"type":"text","text":"经人工核对的输出。"}]}]})).unwrap() }).unwrap().proposal
}

#[test]
fn optional_and_custom_templates_share_one_durable_task_flow_without_fixed_scenarios() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let evidence = material(&mut db);
    let example = db
        .knowledge_task_templates(KnowledgeTaskTemplatesRequest {
            page: 1,
            limit: 10,
            query: None,
        })
        .unwrap()
        .items
        .remove(0);
    let custom = template(
        &mut db,
        "比较两种备选方案",
        "比较材料中两个方案的依据、局限和未确定项，帮助我做决定。",
    );
    for (index, template) in [None, Some(example), Some(custom)].into_iter().enumerate() {
        let mut request = input(&db, evidence.clone());
        request.title = format!("自定义知识任务 {index}");
        if let Some(template) = template {
            request.instructions = template.instructions;
            request.template = Some(KnowledgeTaskTemplateRef {
                id: template.id,
                row_version: template.row_version,
            });
        }
        let command = id();
        let result = db.start_knowledge_task(&command, request.clone()).unwrap();
        assert_eq!(result.task.run.status, ChatRunStatus::ModelUnconfigured);
        assert!(result.task.materials_current);
        assert!(result.task.template_current);
        assert_eq!(result.task.input.materials[0], evidence);
        assert!(!result.task.run.request.use_knowledge_tools);
        let replay = db.start_knowledge_task(&command, request.clone()).unwrap();
        assert!(replay.receipt.replayed);
        assert_eq!(replay.task.id, result.task.id);
        assert_eq!(db.knowledge_tasks(page()).unwrap().total, index as u32 + 1);
        if let Some(template) = request.template {
            let history = db
                .knowledge_tasks(KnowledgeTasksRequest {
                    template_id: Some(template.id),
                    ..page()
                })
                .unwrap();
            assert_eq!(history.total, 1);
            assert_eq!(history.items[0].id, result.task.id);
        }
    }
    assert!(db.chat_conversations(None, 100).unwrap().is_empty());
    assert!(
        db.chat_conversations_page(None, 100)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn task_and_identity_rollback_together_and_invalid_materials_never_start_a_run() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let evidence = material(&mut db);
    let request = input(&db, evidence.clone());
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_app BEFORE INSERT ON knowledge_tasks BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(db.start_knowledge_task(&id(), request.clone()).is_err());
    for table in [
        "knowledge_tasks",
        "chat_runs",
        "chat_messages",
        "chat_conversations",
    ] {
        assert_eq!(
            raw.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    raw.execute_batch("DROP TRIGGER fail_app;").unwrap();
    let mut invalid = request.clone();
    invalid.materials.clear();
    assert!(matches!(
        db.start_knowledge_task(&id(), invalid),
        Err(Error::InvalidInput("knowledge_task_materials_required"))
    ));
    let mut invalid = request.clone();
    invalid.materials[0].text = "编造的正文".into();
    assert!(db.start_knowledge_task(&id(), invalid).is_err());
    let mut invalid = request.clone();
    invalid.period = Some(KnowledgeTaskPeriod {
        start: "2026-11-08".into(),
        end: "2026-11-01".into(),
    });
    assert!(matches!(
        db.start_knowledge_task(&id(), invalid),
        Err(Error::InvalidInput("knowledge_task_period_invalid"))
    ));
    model(&mut db);
    assert!(matches!(
        db.start_knowledge_task(&id(), request),
        Err(Error::MetadataConflict { .. })
    ));
    assert_eq!(db.knowledge_tasks(page()).unwrap().total, 0);
}

#[test]
fn completed_output_requires_review_and_retains_saved_identity_after_proposal_removal() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let evidence = material(&mut db);
    model(&mut db);
    let request = input(&db, evidence);
    let run = db.start_knowledge_task(&id(), request).unwrap().task;
    assert_eq!(run.run.status, ChatRunStatus::Queued);
    assert_eq!(run.output_count, 0);
    finish(&mut db, &run.id);
    let complete = db.knowledge_task(&run.id).unwrap();
    assert!(complete.output_text.contains("[Material 1]"));
    let proposal = proposal(&mut db, &run.id);
    let pending = db.knowledge_task(&run.id).unwrap();
    assert_eq!(pending.output_count, 1);
    assert_eq!(
        pending.latest_output.as_ref().unwrap().status,
        ProposalStatus::Pending
    );
    assert!(
        pending
            .latest_output
            .as_ref()
            .unwrap()
            .applied_note_id
            .is_none()
    );
    let note = db.apply_proposal(&id(), &proposal.id).unwrap().note;
    let saved = db.knowledge_task(&run.id).unwrap().latest_output.unwrap();
    assert_eq!(saved.applied_note_id.as_deref(), Some(note.id.as_str()));
    assert_eq!(
        saved.applied_revision_id.as_deref(),
        Some(note.revision_id.as_str())
    );
    let origins = db
        .knowledge_task_origins(KnowledgeTaskOriginsRequest {
            note_id: note.id.clone(),
            page: 100,
            limit: 10,
        })
        .unwrap();
    assert_eq!(origins.total, 1);
    assert_eq!(origins.page, 1);
    assert_eq!(origins.items[0].task_id, run.id);
    assert_eq!(origins.items[0].applied_revision_id, note.revision_id);
    assert!(saved.note_available);
    assert!(!saved.note_trashed);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite3")).unwrap();
    raw.pragma_update(None, "foreign_keys", true).unwrap();
    raw.execute("DELETE FROM proposals WHERE id=?1", [&proposal.id])
        .unwrap();
    let kept = db.knowledge_task(&run.id).unwrap().latest_output.unwrap();
    assert!(kept.proposal_id.is_none());
    assert_eq!(kept.status, ProposalStatus::Applied);
    assert_eq!(kept.applied_note_id, Some(note.id));
    assert_eq!(db.knowledge_tasks(page()).unwrap().items[0].output_count, 1);
}

#[test]
fn cancel_retry_and_material_changes_are_explicit_and_history_cannot_be_used_as_free_chat() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let evidence = material(&mut db);
    model(&mut db);
    let request = input(&db, evidence.clone());
    let run = db
        .start_knowledge_task(&id(), request.clone())
        .unwrap()
        .task;
    let mut retry = request.clone();
    retry.retry_of = Some(run.id.clone());
    assert!(db.start_knowledge_task(&id(), retry.clone()).is_err());
    let canceled = db.cancel_chat_run(&id(), &run.id).unwrap();
    assert_eq!(canceled.run.status, ChatRunStatus::Canceled);
    let retried = db.start_knowledge_task(&id(), retry).unwrap().task;
    assert_eq!(retried.input.retry_of, Some(run.id.clone()));
    assert!(
        db.accept_chat_run_from_settings(
            &id(),
            StartChatRun {
                conversation_id: canceled.conversation.id,
                base_row_version: canceled.conversation.row_version,
                text: "another message".into(),
                purpose: ChatPurpose::Chat,
                materials: vec![],
                scope: SearchScope::default(),
                use_knowledge_tools: false,
                options: None
            }
        )
        .is_err()
    );
    let metadata = db.object_metadata(&evidence.target.object_id).unwrap();
    db.set_object_archived(
        &id(),
        SetObjectFlag {
            object_id: metadata.id,
            base_row_version: metadata.row_version,
            value: true,
        },
    )
    .unwrap();
    assert!(!db.knowledge_task(&run.id).unwrap().materials_current);
    assert!(db.start_knowledge_task(&id(), request).is_err());
}

#[test]
fn complete_backup_restores_task_inputs_output_history_and_cancels_inflight_generation() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let evidence = material(&mut db);
    model(&mut db);
    let request = input(&db, evidence);
    let completed = db
        .start_knowledge_task(&id(), request.clone())
        .unwrap()
        .task;
    finish(&mut db, &completed.id);
    let draft = proposal(&mut db, &completed.id);
    let saved = db.apply_proposal(&id(), &draft.id).unwrap().note;
    let queued = db.start_knowledge_task(&id(), request).unwrap().task;
    let package = root.path().join("applications.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    verify_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(&destination).unwrap();
    let completed_copy = restored.knowledge_task(&completed.id).unwrap();
    assert_eq!(completed_copy.input.materials, completed.input.materials);
    assert_eq!(
        completed_copy.latest_output.unwrap().applied_note_id,
        Some(saved.id)
    );
    assert_eq!(
        restored.knowledge_task(&queued.id).unwrap().run.status,
        ChatRunStatus::Interrupted
    );
    assert_eq!(restored.knowledge_tasks(page()).unwrap().total, 2);
}

#[test]
fn backup_rejects_an_output_reassigned_to_a_different_knowledge_task() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path().join("source")).unwrap();
    let evidence = material(&mut db);
    model(&mut db);
    let request = input(&db, evidence);
    let first = db
        .start_knowledge_task(&id(), request.clone())
        .unwrap()
        .task;
    finish(&mut db, &first.id);
    let output = proposal(&mut db, &first.id);
    let second = db.start_knowledge_task(&id(), request).unwrap().task;
    let raw = rusqlite::Connection::open(root.path().join("source/library.sqlite3")).unwrap();
    raw.execute(
        "UPDATE knowledge_task_outputs SET task_id=?2 WHERE id=?1",
        rusqlite::params![output.id, second.id],
    )
    .unwrap();
    assert!(
        db.create_backup(root.path().join("tampered.pkmsbackup"), &mut |_| Ok(()))
            .is_err()
    );
}

#[test]
fn templates_are_editable_removable_and_do_not_rewrite_task_history() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let evidence = material(&mut db);
    let created = template(&mut db, "候选方案对照", "按材料比较两个候选方案。");
    let mut request = input(&db, evidence.clone());
    request.instructions = created.instructions.clone();
    request.template = Some(KnowledgeTaskTemplateRef {
        id: created.id.clone(),
        row_version: created.row_version,
    });
    let accepted = db
        .start_knowledge_task(&id(), request.clone())
        .unwrap()
        .task;
    let updated = db
        .save_knowledge_task_template(
            &id(),
            SaveKnowledgeTaskTemplate {
                id: Some(created.id.clone()),
                base_row_version: Some(created.row_version),
                name: "方案对照".into(),
                description: created.description.clone(),
                instructions: "补充反例与不确定性。".into(),
            },
        )
        .unwrap()
        .template;
    assert!(matches!(
        db.start_knowledge_task(&id(), request),
        Err(Error::MetadataConflict { .. })
    ));
    let historical = db.knowledge_task(&accepted.id).unwrap();
    assert!(!historical.template_current);
    assert_eq!(
        historical.template_snapshot.as_ref().unwrap().name,
        created.name
    );
    assert_eq!(historical.input.instructions, created.instructions);
    assert!(
        db.save_knowledge_task_template(
            &id(),
            SaveKnowledgeTaskTemplate {
                id: None,
                base_row_version: None,
                name: updated.name.clone(),
                description: String::new(),
                instructions: updated.instructions.clone()
            }
        )
        .is_err()
    );
    let copied = template(&mut db, "方案对照 副本", &updated.instructions);
    assert_ne!(copied.id, created.id);
    let command = id();
    let removal = DeleteKnowledgeTaskTemplate {
        id: created.id.clone(),
        base_row_version: updated.row_version,
    };
    db.delete_knowledge_task_template(&command, removal.clone())
        .unwrap();
    assert!(
        db.delete_knowledge_task_template(&command, removal)
            .unwrap()
            .receipt
            .replayed
    );
    assert_eq!(
        db.knowledge_task(&accepted.id)
            .unwrap()
            .template_snapshot
            .unwrap()
            .id,
        created.id
    );
    assert_eq!(
        db.knowledge_tasks(KnowledgeTasksRequest {
            template_id: Some(created.id.clone()),
            ..page()
        })
        .unwrap()
        .total,
        1
    );
    let all = db
        .knowledge_task_templates(KnowledgeTaskTemplatesRequest {
            page: 1,
            limit: 100,
            query: None,
        })
        .unwrap();
    for item in all.items {
        db.delete_knowledge_task_template(
            &id(),
            DeleteKnowledgeTaskTemplate {
                id: item.id,
                base_row_version: item.row_version,
            },
        )
        .unwrap();
    }
    db.start_knowledge_task(&id(), input(&db, evidence))
        .unwrap();
    let package = root.path().join("templates-deleted.pkmsbackup");
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.path().join("restored");
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let restored = Library::open_existing(destination).unwrap();
    assert_eq!(
        restored
            .knowledge_task_templates(KnowledgeTaskTemplatesRequest {
                page: 1,
                limit: 10,
                query: None
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        restored
            .knowledge_task(&accepted.id)
            .unwrap()
            .template_snapshot
            .unwrap()
            .name,
        created.name
    );
}
