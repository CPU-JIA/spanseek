//! V1.9.5 controlled automation: proposals, per-item application, audit,
//! research tasks, evidence snapshots and explainable review.
use chrono::{Duration, Utc};
use pkms_core::*;
use std::io::Cursor;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn assign(db: &mut Library, note: &str, name: &str) {
    let metadata = db.object_metadata(note).unwrap();
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: note.into(),
                base_row_version: metadata.row_version,
            }],
            tag: TagSelector::Name { name: name.into() },
        },
    )
    .unwrap();
}

fn proposal(db: &mut Library, kind: ChangeSetKind, limit: u32) -> ChangeSet {
    db.create_change_set(
        &id(),
        CreateChangeSet {
            kind,
            collection_id: None,
            limit,
            stale_after_days: Some(365),
        },
    )
    .unwrap()
    .change
}

fn apply(db: &mut Library, change: &ChangeSet, item_id: &str, confirm: bool) -> ChangeItemCommit {
    db.apply_change_item(
        &id(),
        ApplyChangeItem {
            change_id: change.id.clone(),
            item_id: item_id.into(),
            confirm,
        },
    )
    .unwrap()
}

fn tagged_note(db: &mut Library, title: &str, body: &str, tags: &[&str]) -> Note {
    let note = db.create_note(title, body).unwrap();
    for tag in tags {
        assign(db, &note.id, tag);
    }
    note
}

fn import_source(db: &mut Library, origin_url: Option<&str>) -> SourceVersion {
    let job = db
        .accept_source_import(
            &id(),
            ImportSource {
                input_token: id(),
                original_name: "资料.txt".into(),
                format: SourceFormat::Text,
                origin_url: origin_url.map(Into::into),
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
        )
        .unwrap();
    let lease = db.claim_source_job(&job.job_id).unwrap().unwrap();
    db.capture_source_original(
        &job.job_id,
        &lease.lease_token,
        &mut Cursor::new(b"stale source body"),
        &mut |_| Ok(()),
    )
    .unwrap()
}

#[test]
fn current_schema_and_explainable_review_is_off_by_default() {
    let root = tempdir().unwrap();
    let db = Library::open(root.path()).unwrap();
    assert_eq!(
        db.info().unwrap().database_schema,
        pkms_core::CURRENT_SCHEMA_VERSION as u32
    );
    let settings = db.review_insight_settings().unwrap();
    assert!(!settings.enabled);
    let insights = db.review_insights(20).unwrap();
    assert!(!insights.enabled);
    assert_eq!(
        insights.empty_reason.as_deref(),
        Some("review_insights_disabled")
    );
    assert!(insights.disclaimer.contains("不等于掌握程度"));
}

#[test]
fn tag_proposal_previews_applies_and_undoes() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let a = tagged_note(&mut db, "验收记录", "这里讨论验收标准", &["验收"]);
    let b = db.create_note("发布清单", "验收之后才能发布").unwrap();
    let change = proposal(&mut db, ChangeSetKind::TagSuggestions, 20);
    let item = change
        .items
        .iter()
        .find(|item| {
            item.target_object_id == b.id && matches!(item.payload, ChangePayload::AssignTag { .. })
        })
        .expect("tag suggestion for the second note");
    assert_eq!(item.status, ChangeItemStatus::Pending);
    assert!(!item.requires_confirmation);
    assert!(
        item.evidence
            .iter()
            .any(|evidence| evidence.excerpt.contains("验收"))
    );
    let revision_before = db.note(&b.id).unwrap().revision_id;
    let result = apply(&mut db, &change, &item.item_id, false);
    assert_eq!(result.item.status, ChangeItemStatus::Applied);
    assert!(
        db.object_metadata(&b.id)
            .unwrap()
            .tags
            .iter()
            .any(|tag| tag.display_name == "验收")
    );
    assert_eq!(db.note(&b.id).unwrap().revision_id, revision_before);
    let audit = result.audit.expect("audit event");
    assert!(audit.undoable);
    assert_eq!(audit.subject_kind, AuditSubjectKind::Tag);
    let undone = db
        .undo_audit_event(
            &id(),
            UndoAuditEvent {
                audit_id: audit.id.clone(),
                confirm: true,
            },
        )
        .unwrap();
    assert_eq!(undone.reversed.kind, "audit.undo");
    assert!(
        !db.object_metadata(&b.id)
            .unwrap()
            .tags
            .iter()
            .any(|tag| tag.display_name == "验收")
    );
    let page = db
        .audit_page(AuditPageRequest {
            limit: 20,
            before_seq: None,
            subject_kind: None,
            subject_id: None,
        })
        .unwrap();
    assert!(
        page.items
            .iter()
            .any(|event| event.id == audit.id && event.undone)
    );
    let _ = a;
}

#[test]
fn duplicate_proposal_requires_confirmation_and_undo_restores() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let first = db.create_note("重复样本", "同样的正文内容").unwrap();
    let second = db.create_note("重复样本", "同样的正文内容").unwrap();
    let change = proposal(&mut db, ChangeSetKind::DuplicateCandidates, 20);
    assert_eq!(change.items.len(), 1);
    let item = &change.items[0];
    assert!(item.requires_confirmation);
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: change.id.clone(),
                item_id: item.item_id.clone(),
                confirm: false,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("automation_confirmation_required")
    ));
    assert!(!db.object_metadata(&second.id).unwrap().trashed);
    let result = apply(&mut db, &change, &item.item_id, true);
    let trashed_id = match &result.item.payload {
        ChangePayload::TrashDuplicate { duplicate_id, .. } => duplicate_id.clone(),
        _ => panic!("unexpected payload"),
    };
    assert!(db.object_metadata(&trashed_id).unwrap().trashed);
    let kept = if trashed_id == first.id {
        &second
    } else {
        &first
    };
    assert!(!db.object_metadata(&kept.id).unwrap().trashed);
    let audit = result.audit.expect("audit");
    db.undo_audit_event(
        &id(),
        UndoAuditEvent {
            audit_id: audit.id,
            confirm: true,
        },
    )
    .unwrap();
    assert!(!db.object_metadata(&trashed_id).unwrap().trashed);
}

#[test]
fn relation_proposal_appends_link_and_undo_conflict_is_explicit() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let a = tagged_note(&mut db, "研究方法甲", "访谈和编码分析", &["研究", "方法"]);
    let b = tagged_note(&mut db, "研究方法乙", "访谈资料编码步骤", &["研究", "方法"]);
    let change = proposal(&mut db, ChangeSetKind::RelationSuggestions, 20);
    let item = change
        .items
        .iter()
        .find(|item| matches!(item.payload, ChangePayload::AppendRelation { .. }))
        .expect("relation suggestion");
    let (source_id, target_id) = match &item.payload {
        ChangePayload::AppendRelation { target_id, .. } => {
            (item.target_object_id.clone(), target_id.clone())
        }
        _ => unreachable!(),
    };
    assert!(item.requires_confirmation);
    let result = apply(&mut db, &change, &item.item_id, true);
    let source = db.note(&source_id).unwrap();
    assert!(source.document.plain_text().contains("相关："));
    assert_eq!(source.author_kind, "ai_proposal");
    assert!(
        db.note_references(&source_id, &source.revision_id)
            .unwrap()
            .iter()
            .any(|r| r.target_id == target_id)
    );
    let audit = result.audit.expect("audit");
    db.undo_audit_event(
        &id(),
        UndoAuditEvent {
            audit_id: audit.id.clone(),
            confirm: true,
        },
    )
    .unwrap();
    let restored = db.note(&source_id).unwrap();
    assert!(!restored.document.plain_text().contains("相关："));
    assert_eq!(restored.author_kind, "restore");
    // Re-apply through a new proposal, edit afterwards, then refuse a silent undo.
    let change = proposal(&mut db, ChangeSetKind::RelationSuggestions, 20);
    let item = change
        .items
        .iter()
        .find(|item| matches!(item.payload, ChangePayload::AppendRelation { .. }))
        .expect("second relation suggestion");
    let result = apply(&mut db, &change, &item.item_id, true);
    let audit = result.audit.expect("audit");
    let source = db.note(&source_id).unwrap();
    db.save_document(
        &id(),
        SaveNote {
            note_id: source_id.clone(),
            base_revision_id: source.revision_id.clone(),
            title: source.title.clone(),
            document_schema: 3,
            document: NoteDocument::from_text(id(), "人工改写后的正文"),
        },
    )
    .unwrap();
    let error = db
        .undo_audit_event(
            &id(),
            UndoAuditEvent {
                audit_id: audit.id,
                confirm: true,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("automation_undo_conflict")
    ));
    assert_eq!(
        db.note(&source_id).unwrap().document.plain_text(),
        "人工改写后的正文"
    );
    let _ = (a, b);
}
#[test]
fn apply_after_source_revision_changed_is_blocked() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    tagged_note(&mut db, "来源甲", "共享 研究 方法 访谈", &["研究", "方法"]);
    tagged_note(&mut db, "来源乙", "共享 研究 方法 编码", &["研究", "方法"]);
    let change = proposal(&mut db, ChangeSetKind::RelationSuggestions, 20);
    let item = change
        .items
        .iter()
        .find(|item| matches!(item.payload, ChangePayload::AppendRelation { .. }))
        .unwrap();
    let source = db.note(&item.target_object_id).unwrap();
    db.save_note(&source.id, source.revision, &source.title, "人工先改了正文")
        .unwrap();
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: change.id.clone(),
                item_id: item.item_id.clone(),
                confirm: true,
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::RevisionConflict { .. }));
    assert_eq!(
        db.note(&source.id).unwrap().document.plain_text(),
        "人工先改了正文"
    );
}

#[test]
fn reject_writes_nothing_and_expiry_has_a_terminal_state() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    tagged_note(&mut db, "标签种子", "已有标签", &["验收"]);
    let note = db.create_note("待整理", "验收 相关").unwrap();
    let change = proposal(&mut db, ChangeSetKind::TagSuggestions, 20);
    let rejected = db
        .reject_change_set(
            &id(),
            RejectChangeSet {
                change_id: change.id.clone(),
            },
        )
        .unwrap()
        .change;
    assert_eq!(rejected.status, ChangeSetStatus::Rejected);
    assert!(
        rejected
            .items
            .iter()
            .all(|item| item.status == ChangeItemStatus::Skipped)
    );
    assert_eq!(db.object_metadata(&note.id).unwrap().tags.len(), 0);
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: change.id.clone(),
                item_id: change.items[0].item_id.clone(),
                confirm: true,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("automation_change_not_pending")
    ));

    let change = proposal(&mut db, ChangeSetKind::TagSuggestions, 20);
    let expired = db
        .expire_change_sets_before(Utc::now() + Duration::days(1))
        .unwrap();
    assert_eq!(expired, 1);
    let expired = db.change_set(&change.id).unwrap();
    assert_eq!(expired.status, ChangeSetStatus::Expired);
    assert!(
        expired
            .items
            .iter()
            .all(|item| item.status == ChangeItemStatus::Skipped)
    );
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: expired.id,
                item_id: expired.items[0].item_id.clone(),
                confirm: true,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("automation_change_expired")
    ));
}

#[test]
fn stale_report_is_read_only_and_separates_reasons() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let empty = db.stale_source_report(365, 20).unwrap();
    assert_eq!(empty.checked, 0);
    assert_eq!(
        empty.empty_reason.as_deref(),
        Some("workspace_has_no_sources")
    );

    let no_origin = import_source(&mut db, None);
    let report = db
        .stale_source_report_at(365, 20, Utc::now() + Duration::days(60))
        .unwrap();
    assert_eq!(report.checked, 1);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].reason, StaleReason::NoOrigin);
    assert_eq!(report.findings[0].age_days, Some(60));
    assert!(!db.source(&no_origin.source_id).unwrap().trashed);

    let with_origin = import_source(&mut db, Some("https://example.com/article"));
    let report = db
        .stale_source_report_at(365, 20, Utc::now() + Duration::days(400))
        .unwrap();
    assert_eq!(report.checked, 2);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.source_id == with_origin.source_id
                && finding.reason == StaleReason::OlderThanWindow)
    );
    assert!(!db.source(&with_origin.source_id).unwrap().trashed);
    let audit = db
        .audit_page(AuditPageRequest {
            limit: 20,
            before_seq: None,
            subject_kind: None,
            subject_id: None,
        })
        .unwrap();
    assert!(
        audit.items.is_empty(),
        "read-only report must not write audit rows"
    );
}

fn research_task(
    db: &mut Library,
    queries: &[&str],
    max_steps: u32,
    max_sources: u32,
) -> ResearchTask {
    db.create_research_task(
        &id(),
        CreateResearchTask {
            goal: "整理访谈研究方法".into(),
            collection_id: None,
            queries: queries.iter().map(|query| query.to_string()).collect(),
            budget: ResearchBudget {
                max_steps,
                max_sources,
            },
            stop: ResearchStopConditions {
                deadline_seconds: None,
                missing_after_step: None,
            },
            external_allowed: false,
            external_search_endpoint: None,
        },
    )
    .unwrap()
    .task
}

#[test]
fn research_steps_trace_input_result_sources_and_revision_state() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db
        .create_note("研究方法笔记", "访谈资料需要编码与归类")
        .unwrap();
    let task = research_task(&mut db, &["研究方法", "访谈编码"], 5, 10);
    let finished = db.run_research_steps(&task.id, 10).unwrap();
    assert_eq!(finished.status, ResearchStatus::Completed);
    assert_eq!(finished.stop_reason.as_deref(), Some("queries_exhausted"));
    assert_eq!(finished.steps.len(), 2);
    for (index, step) in finished.steps.iter().enumerate() {
        match &step.input {
            ResearchStepInput::PrivateRetrieval { query, limit } => {
                assert_eq!(query, &["研究方法", "访谈编码"][index]);
                assert!((1..=10).contains(limit));
            }
            _ => panic!("expected private retrieval"),
        }
        match &step.result {
            ResearchStepResult::PrivateRetrieval { matched, returned } => {
                assert!(*matched >= 1);
                assert_eq!(*returned, step.sources.len() as u32);
            }
            _ => panic!("expected private retrieval result"),
        }
        assert!(!step.sources.is_empty());
        assert!(
            step.sources
                .iter()
                .all(|source| source.origin == ResearchOrigin::PrivateLibrary)
        );
    }
    assert!(
        finished
            .evidence
            .iter()
            .all(|evidence| evidence.revision_state == EvidenceRevisionState::Current)
    );
    assert!(
        finished
            .evidence
            .iter()
            .any(|evidence| evidence.object_id == note.id)
    );
    let head = db.note(&note.id).unwrap();
    db.save_note(&note.id, head.revision, &head.title, "访谈资料更新后的正文")
        .unwrap();
    let reread = db.research_task(&task.id).unwrap();
    assert!(
        reread
            .evidence
            .iter()
            .filter(|evidence| evidence.object_id == note.id)
            .all(|evidence| evidence.revision_state == EvidenceRevisionState::Superseded)
    );
}

#[test]
fn cancel_prevents_any_further_step() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("研究方法", "访谈编码与研究记录").unwrap();
    let task = research_task(
        &mut db,
        &["研究方法", "访谈", "编码", "记录", "研究"],
        5,
        10,
    );
    let advanced = db
        .advance_research(
            &id(),
            AdvanceResearch {
                task_id: task.id.clone(),
                base_row_version: task.row_version,
            },
        )
        .unwrap()
        .task;
    assert_eq!(advanced.step_count, 1);
    assert_eq!(advanced.status, ResearchStatus::Running);
    let canceled = db
        .cancel_research(
            &id(),
            CancelResearch {
                task_id: task.id.clone(),
                base_row_version: advanced.row_version,
                reason: "用户停止".into(),
            },
        )
        .unwrap()
        .task;
    assert_eq!(canceled.status, ResearchStatus::Canceled);
    assert_eq!(canceled.stop_reason.as_deref(), Some("用户停止"));
    let error = db
        .advance_research(
            &id(),
            AdvanceResearch {
                task_id: task.id.clone(),
                base_row_version: canceled.row_version,
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::InvalidInput("research_canceled")));
    let final_task = db.run_research_steps(&task.id, 10).unwrap();
    assert_eq!(final_task.step_count, 1);
    assert_eq!(final_task.steps.len(), 1);
}

#[test]
fn external_step_rejects_undeclared_query_and_records_origin() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let task = db
        .create_research_task(
            &id(),
            CreateResearchTask {
                goal: "核对公开资料".into(),
                collection_id: None,
                queries: vec!["公开主题".into()],
                budget: ResearchBudget {
                    max_steps: 3,
                    max_sources: 5,
                },
                stop: ResearchStopConditions {
                    deadline_seconds: None,
                    missing_after_step: None,
                },
                external_allowed: true,
                external_search_endpoint: Some("http://127.0.0.1:9/search?q={query}".into()),
            },
        )
        .unwrap()
        .task;
    let error = db
        .record_external_step(
            &id(),
            RecordExternalStep {
                task_id: task.id.clone(),
                base_row_version: task.row_version,
                query: "私库正文片段".into(),
                endpoint: "http://127.0.0.1:9/search?q={query}".into(),
                http_status: 200,
                final_url: "http://127.0.0.1:9/search?q=private".into(),
                title: "外部结果".into(),
                excerpt: "公开摘要".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("research_external_query_mismatch")
    ));
    let recorded = db
        .record_external_step(
            &id(),
            RecordExternalStep {
                task_id: task.id.clone(),
                base_row_version: task.row_version,
                query: "公开主题".into(),
                endpoint: "http://127.0.0.1:9/search?q={query}".into(),
                http_status: 200,
                final_url: "http://127.0.0.1:9/search?q=%E5%85%AC%E5%BC%80%E4%B8%BB%E9%A2%98"
                    .into(),
                title: "外部结果".into(),
                excerpt: "公开摘要".into(),
            },
        )
        .unwrap()
        .task;
    assert_eq!(recorded.steps.len(), 1);
    assert_eq!(recorded.steps[0].origin, ResearchOrigin::External);
    assert_eq!(recorded.evidence[0].origin, ResearchOrigin::External);
    assert_eq!(
        recorded.evidence[0].revision_state,
        EvidenceRevisionState::External
    );
    assert_eq!(recorded.status, ResearchStatus::Completed);

    let failing = db
        .create_research_task(
            &id(),
            CreateResearchTask {
                goal: "失败记录".into(),
                collection_id: None,
                queries: vec!["公开主题".into()],
                budget: ResearchBudget {
                    max_steps: 3,
                    max_sources: 5,
                },
                stop: ResearchStopConditions {
                    deadline_seconds: None,
                    missing_after_step: None,
                },
                external_allowed: true,
                external_search_endpoint: Some("http://127.0.0.1:9/search?q={query}".into()),
            },
        )
        .unwrap()
        .task;
    let failed = db
        .record_failed_external_step(
            &id(),
            RecordFailedExternalStep {
                task_id: failing.id,
                base_row_version: failing.row_version,
                query: "公开主题".into(),
                endpoint: "http://127.0.0.1:9/search?q={query}".into(),
                error_code: "webpage_network_failed".into(),
            },
        )
        .unwrap()
        .task;
    assert_eq!(failed.status, ResearchStatus::Failed);
    assert_eq!(failed.steps[0].status, ResearchStepStatus::Failed);
    assert!(matches!(
        failed.steps[0].result,
        ResearchStepResult::Failed { .. }
    ));
}

#[test]
fn explainable_review_is_opt_in_and_never_claims_mastery() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let reviewed = tagged_note(&mut db, "已回顾", "回顾内容", &["主题"]);
    let untouched = tagged_note(&mut db, "未回顾", "另一篇内容", &["主题"]);
    let entry = db
        .add_review(
            &id(),
            AddReview {
                target: ResolveLink {
                    target_id: reviewed.id.clone(),
                    target_block_id: None,
                    target_revision_id: None,
                },
                base_row_version: None,
            },
        )
        .unwrap()
        .entry;
    db.complete_review(
        &id(),
        ReviewChange {
            entry_id: entry.id,
            base_row_version: entry.row_version,
        },
    )
    .unwrap();
    let enabled = db
        .set_review_insight_settings(
            &id(),
            SetReviewInsightSettings {
                base_row_version: 1,
                enabled: true,
                window_days: 1,
                include_completed: true,
                include_stale: true,
            },
        )
        .unwrap();
    assert!(enabled.settings.enabled);
    let revisit = db.review_insights_at(50, Utc::now()).unwrap();
    assert!(
        revisit
            .items
            .iter()
            .any(|insight| insight.object_id == reviewed.id
                && insight.kind == ReviewInsightKind::Revisit)
    );
    assert!(revisit.disclaimer.contains("不等于掌握程度"));
    assert!(
        revisit
            .items
            .iter()
            .all(|insight| !insight.caveat.is_empty())
    );
    let stale = db
        .review_insights_at(50, Utc::now() + Duration::days(10))
        .unwrap();
    assert!(stale.items.iter().any(
        |insight| insight.object_id == untouched.id && insight.kind == ReviewInsightKind::Stale
    ));
    let audit = db
        .audit_page(AuditPageRequest {
            limit: 20,
            before_seq: None,
            subject_kind: Some(AuditSubjectKind::ReviewSetting),
            subject_id: None,
        })
        .unwrap();
    assert_eq!(audit.items.len(), 1);
    db.undo_audit_event(
        &id(),
        UndoAuditEvent {
            audit_id: audit.items[0].id.clone(),
            confirm: true,
        },
    )
    .unwrap();
    assert!(!db.review_insight_settings().unwrap().enabled);
    let disabled = db.review_insights(50).unwrap();
    assert!(!disabled.enabled);
}
