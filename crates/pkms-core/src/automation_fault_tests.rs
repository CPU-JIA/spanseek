//! V1.9.5 G10: a storage failure while applying an item must roll back the
//! knowledge write, the audit row, and the proposal status together.
use crate::*;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn storage_failure_rolls_back_all_three_writes() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let seed = db.create_note("标签种子", "验收 相关").unwrap();
    let metadata = db.object_metadata(&seed.id).unwrap();
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: seed.id.clone(),
                base_row_version: metadata.row_version,
            }],
            tag: TagSelector::Name {
                name: "验收".into(),
            },
        },
    )
    .unwrap();
    let target = db.create_note("待打标签", "验收 流程").unwrap();
    let change = db
        .create_change_set(
            &id(),
            CreateChangeSet {
                kind: ChangeSetKind::TagSuggestions,
                collection_id: None,
                limit: 20,
                stale_after_days: None,
            },
        )
        .unwrap()
        .change;
    let item = change
        .items
        .iter()
        .find(|item| item.target_object_id == target.id)
        .unwrap()
        .item_id
        .clone();
    db.connection
        .execute_batch(
            "CREATE TRIGGER fail_audit BEFORE INSERT ON audit_events BEGIN
               SELECT RAISE(ABORT, 'injected storage failure');
             END;",
        )
        .unwrap();
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: change.id.clone(),
                item_id: item.clone(),
                confirm: false,
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::Database(_)));
    db.connection
        .execute_batch("DROP TRIGGER fail_audit;")
        .unwrap();
    assert!(
        db.object_metadata(&target.id)
            .unwrap()
            .tags
            .iter()
            .all(|tag| tag.display_name != "验收")
    );
    let reread = db.change_set(&change.id).unwrap();
    assert_eq!(reread.status, ChangeSetStatus::Pending);
    assert_eq!(reread.applied_count, 0);
    assert!(
        reread
            .items
            .iter()
            .all(|row| row.item_id != item || row.status == ChangeItemStatus::Pending)
    );
    let audit = db
        .audit_page(AuditPageRequest {
            limit: 20,
            before_seq: None,
            subject_kind: None,
            subject_id: None,
        })
        .unwrap();
    assert!(audit.items.is_empty());
}

#[test]
fn repeated_apply_and_budget_exhaustion_are_terminal() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("标签种子", "验收").unwrap();
    let seed_meta = db.object_metadata(&db.list_notes().unwrap()[0].id).unwrap();
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: seed_meta.id.clone(),
                base_row_version: seed_meta.row_version,
            }],
            tag: TagSelector::Name {
                name: "验收".into(),
            },
        },
    )
    .unwrap();
    let target = db.create_note("待打标签", "验收").unwrap();
    let change = db
        .create_change_set(
            &id(),
            CreateChangeSet {
                kind: ChangeSetKind::TagSuggestions,
                collection_id: None,
                limit: 20,
                stale_after_days: None,
            },
        )
        .unwrap()
        .change;
    let item = change
        .items
        .iter()
        .find(|item| item.target_object_id == target.id)
        .unwrap()
        .item_id
        .clone();
    db.apply_change_item(
        &id(),
        ApplyChangeItem {
            change_id: change.id.clone(),
            item_id: item.clone(),
            confirm: false,
        },
    )
    .unwrap();
    let error = db
        .apply_change_item(
            &id(),
            ApplyChangeItem {
                change_id: change.id,
                item_id: item,
                confirm: false,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput("automation_change_not_pending")
    ));

    let error = db
        .create_research_task(
            &id(),
            CreateResearchTask {
                goal: "预算边界".into(),
                collection_id: None,
                queries: vec!["验收".into(), "验收".into()],
                budget: ResearchBudget {
                    max_steps: 1,
                    max_sources: 10,
                },
                stop: ResearchStopConditions {
                    deadline_seconds: None,
                    missing_after_step: None,
                },
                external_allowed: false,
                external_search_endpoint: None,
            },
        )
        .unwrap_err();
    // Duplicate queries are rejected before a task exists.
    assert!(matches!(
        error,
        Error::InvalidInput("research queries must be unique")
    ));
    let task = db
        .create_research_task(
            &id(),
            CreateResearchTask {
                goal: "预算边界".into(),
                collection_id: None,
                queries: vec!["验收".into(), "流程".into()],
                budget: ResearchBudget {
                    max_steps: 1,
                    max_sources: 10,
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
        .task;
    let finished = db.run_research_steps(&task.id, 5).unwrap();
    assert_eq!(finished.status, ResearchStatus::Completed);
    assert_eq!(finished.stop_reason.as_deref(), Some("budget_steps"));
    assert_eq!(finished.step_count, 1);
}
#[test]
fn missing_evidence_revision_is_reported_without_replacement() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    db.create_note("证据样本", "证据正文 研究").unwrap();
    let task = db
        .create_research_task(
            &id(),
            CreateResearchTask {
                goal: "证据失效呈现".into(),
                collection_id: None,
                queries: vec!["证据".into()],
                budget: ResearchBudget {
                    max_steps: 2,
                    max_sources: 10,
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
        .task;
    let finished = db.run_research_steps(&task.id, 3).unwrap();
    assert!(
        finished
            .evidence
            .iter()
            .all(|evidence| evidence.revision_state == EvidenceRevisionState::Current)
    );
    let excerpt = finished.evidence[0].excerpt.clone();
    db.connection
        .execute(
            "UPDATE research_evidence SET object_revision_id=?2 WHERE task_id=?1",
            rusqlite::params![task.id, Uuid::new_v4().to_string()],
        )
        .unwrap();
    let reread = db.research_task(&task.id).unwrap();
    assert!(
        reread
            .evidence
            .iter()
            .all(|evidence| evidence.revision_state == EvidenceRevisionState::Missing)
    );
    assert_eq!(reread.evidence[0].excerpt, excerpt);
}
