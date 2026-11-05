//! Bounded research tasks, revision-bound evidence and explainable review.
//!
//! Core never performs network I/O. The desktop host executes one external GET
//! and records the bounded result here; the recorded endpoint/query pair must
//! match the task declaration, so a private excerpt cannot be smuggled into an
//! outbound request through this API.
use crate::{
    AdvanceResearch, AuditSubjectKind, CancelResearch, CreateResearchTask, Error,
    EvidenceRevisionState, Library, MAX_EVIDENCE_EXCERPT, MAX_RESEARCH_QUERIES,
    MAX_RESEARCH_SOURCES, MAX_RESEARCH_STEPS, RecordExternalStep, RecordFailedExternalStep,
    ResearchBudget, ResearchCommit, ResearchEvidence, ResearchOrigin, ResearchPage,
    ResearchPageRequest, ResearchSource, ResearchStatus, ResearchStep, ResearchStepInput,
    ResearchStepKind, ResearchStepResult, ResearchStepStatus, ResearchTask, ResearchTaskSummary,
    Result, ReviewInsight, ReviewInsightBasis, ReviewInsightKind, ReviewInsightSettings,
    ReviewInsights, ReviewSettingsCommit, SetReviewInsightSettings,
    automation::{NewAudit, insert_audit},
    document::valid_uuid,
    mutations::{check_version, next_receipt, replay, request_hash, store_receipt},
    notes::revision as note_revision,
};
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const RECORD_EXTERNAL_KIND: &str = "research.record_external";
const RECORD_EXTERNAL_FAILURE_KIND: &str = "research.record_external_failure";

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn validate_goal(goal: &str) -> Result<String> {
    let goal = goal.trim();
    if !(1..=2000).contains(&goal.chars().count()) || goal.contains('\0') {
        return Err(Error::InvalidInput(
            "research goal must contain 1-2000 characters",
        ));
    }
    Ok(goal.to_owned())
}

fn validate_queries(queries: &[String]) -> Result<Vec<String>> {
    if queries.is_empty() || queries.len() > MAX_RESEARCH_QUERIES {
        return Err(Error::InvalidInput("research must declare 1-20 queries"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(queries.len());
    for query in queries {
        let query = query.trim();
        if !(1..=500).contains(&query.chars().count()) || query.contains('\0') {
            return Err(Error::InvalidInput(
                "research query must contain 1-500 characters",
            ));
        }
        if !seen.insert(query.to_owned()) {
            return Err(Error::InvalidInput("research queries must be unique"));
        }
        result.push(query.to_owned());
    }
    Ok(result)
}

fn validate_budget(budget: &ResearchBudget) -> Result<()> {
    if !(1..=MAX_RESEARCH_STEPS).contains(&budget.max_steps) {
        return Err(Error::InvalidInput("research max_steps is out of range"));
    }
    if !(1..=MAX_RESEARCH_SOURCES).contains(&budget.max_sources) {
        return Err(Error::InvalidInput("research max_sources is out of range"));
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim();
    if endpoint.len() > 2048
        || endpoint.contains('\0')
        || !(endpoint.starts_with("https://") || endpoint.starts_with("http://"))
        || !endpoint.contains("{query}")
    {
        return Err(Error::InvalidInput("research_external_endpoint_invalid"));
    }
    Ok(endpoint.to_owned())
}

fn read_task(connection: &Connection, id: &str) -> Result<ResearchTask> {
    valid_uuid(id)?;
    let row = connection
        .query_row(
            "SELECT goal,collection_id,queries_json,budget_json,stop_json,external_allowed,external_search_endpoint,status,stop_reason,step_count,row_version,created_at,updated_at FROM research_tasks WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, u32>(9)?,
                    row.get::<_, u32>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let status = match row.7.as_str() {
        "pending" => ResearchStatus::Pending,
        "running" => ResearchStatus::Running,
        "completed" => ResearchStatus::Completed,
        "canceled" => ResearchStatus::Canceled,
        "failed" => ResearchStatus::Failed,
        _ => return Err(Error::IntegrityFailure),
    };
    let budget: ResearchBudget = serde_json::from_str(&row.3)?;
    let mut task = ResearchTask {
        id: id.into(),
        goal: row.0,
        collection_id: row.1,
        queries: serde_json::from_str(&row.2)?,
        budget,
        stop: serde_json::from_str(&row.4)?,
        external_allowed: row.5,
        external_search_endpoint: row.6,
        status,
        stop_reason: row.8,
        step_count: row.9,
        row_version: row.10,
        created_at: row.11,
        updated_at: row.12,
        steps: vec![],
        evidence: vec![],
    };
    task.steps = read_steps(connection, &task.id)?;
    task.evidence = read_evidence(connection, &task.id)?;
    Ok(task)
}

fn parse_step_kind(value: &str) -> Result<ResearchStepKind> {
    Ok(match value {
        "private_retrieval" => ResearchStepKind::PrivateRetrieval,
        "external_search" => ResearchStepKind::ExternalSearch,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn parse_step_status(value: &str) -> Result<ResearchStepStatus> {
    Ok(match value {
        "succeeded" => ResearchStepStatus::Succeeded,
        "failed" => ResearchStepStatus::Failed,
        "canceled" => ResearchStepStatus::Canceled,
        "skipped" => ResearchStepStatus::Skipped,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn parse_origin(value: &str) -> Result<ResearchOrigin> {
    Ok(match value {
        "private_library" => ResearchOrigin::PrivateLibrary,
        "external" => ResearchOrigin::External,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn origin_str(value: ResearchOrigin) -> &'static str {
    match value {
        ResearchOrigin::PrivateLibrary => "private_library",
        ResearchOrigin::External => "external",
    }
}

fn read_steps(connection: &Connection, task_id: &str) -> Result<Vec<ResearchStep>> {
    let mut statement = connection.prepare(
        "SELECT seq,kind,status,origin,input_json,result_json,sources_json,created_at FROM research_steps WHERE task_id=?1 ORDER BY seq",
    )?;
    let rows = statement
        .query_map([task_id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|row| {
            Ok(ResearchStep {
                task_id: task_id.into(),
                seq: row.0,
                kind: parse_step_kind(&row.1)?,
                status: parse_step_status(&row.2)?,
                origin: parse_origin(&row.3)?,
                input: serde_json::from_str(&row.4)?,
                result: serde_json::from_str(&row.5)?,
                sources: serde_json::from_str(&row.6)?,
                created_at: row.7,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn read_evidence(connection: &Connection, task_id: &str) -> Result<Vec<ResearchEvidence>> {
    let mut statement = connection.prepare(
        "SELECT id,step_seq,object_id,object_revision_id,origin,title,excerpt,excerpt_hash,source_url,captured_at FROM research_evidence WHERE task_id=?1 ORDER BY step_seq,id",
    )?;
    let rows = statement
        .query_map([task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|row| {
            let origin = parse_origin(&row.4)?;
            let revision_state = match (origin, row.3.as_deref()) {
                (ResearchOrigin::External, _) => EvidenceRevisionState::External,
                (_, None) => EvidenceRevisionState::Missing,
                (_, Some(revision)) => {
                    let exists: Option<String> = connection
                        .query_row(
                            "SELECT note_id FROM note_revisions WHERE revision_id=?1",
                            [revision],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if exists.is_none() {
                        EvidenceRevisionState::Missing
                    } else {
                        let current: Option<String> = connection
                            .query_row(
                                "SELECT current_revision_id FROM notes WHERE id=?1",
                                [&row.2],
                                |row| row.get(0),
                            )
                            .optional()?;
                        if current.as_deref() == Some(revision) {
                            EvidenceRevisionState::Current
                        } else {
                            EvidenceRevisionState::Superseded
                        }
                    }
                }
            };
            Ok(ResearchEvidence {
                id: row.0,
                task_id: task_id.into(),
                step_seq: row.1,
                object_id: row.2,
                object_revision_id: row.3,
                origin,
                title: row.5,
                excerpt: row.6,
                excerpt_hash: row.7,
                source_url: row.8,
                captured_at: row.9,
                revision_state,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn fts_query(query: &str) -> Option<String> {
    let terms = crate::tokens(query);
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .into_iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn excerpt(text: &str) -> String {
    let excerpt: String = text.chars().take(280).collect();
    excerpt
}

fn hash_excerpt(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

struct NewStep<'a> {
    task_id: &'a str,
    seq: u32,
    kind: ResearchStepKind,
    status: ResearchStepStatus,
    origin: ResearchOrigin,
    input: &'a ResearchStepInput,
    result: &'a ResearchStepResult,
    sources: &'a [ResearchSource],
    at: &'a str,
}

fn insert_step(connection: &Connection, step: NewStep<'_>) -> Result<()> {
    connection.execute(
        "INSERT INTO research_steps(task_id,seq,kind,status,origin,input_json,result_json,sources_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            step.task_id,
            step.seq,
            match step.kind {
                ResearchStepKind::PrivateRetrieval => "private_retrieval",
                ResearchStepKind::ExternalSearch => "external_search",
            },
            match step.status {
                ResearchStepStatus::Succeeded => "succeeded",
                ResearchStepStatus::Failed => "failed",
                ResearchStepStatus::Canceled => "canceled",
                ResearchStepStatus::Skipped => "skipped",
            },
            origin_str(step.origin),
            serde_json::to_string(step.input)?,
            serde_json::to_string(step.result)?,
            serde_json::to_string(step.sources)?,
            step.at
        ],
    )?;
    Ok(())
}
fn insert_evidence(
    connection: &Connection,
    task_id: &str,
    step_seq: u32,
    source: &ResearchSource,
    at: &str,
) -> Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO research_evidence(id,task_id,step_seq,object_id,object_revision_id,origin,title,excerpt,excerpt_hash,source_url,captured_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            Uuid::new_v4().to_string(),
            task_id,
            step_seq,
            source.object_id,
            source.revision_id,
            origin_str(source.origin),
            source.title,
            source.excerpt,
            source.excerpt_hash,
            source.url,
            at
        ],
    )?;
    Ok(())
}

fn total_evidence(connection: &Connection, task_id: &str) -> Result<u32> {
    Ok(connection.query_row(
        "SELECT COUNT(*) FROM research_evidence WHERE task_id=?1",
        [task_id],
        |row| row.get(0),
    )?)
}

fn update_task(
    connection: &Connection,
    task_id: &str,
    status: ResearchStatus,
    stop_reason: Option<&str>,
    step_count: u32,
) -> Result<()> {
    connection.execute(
        "UPDATE research_tasks SET status=?2,stop_reason=?3,step_count=?4,row_version=row_version+1,updated_at=?5 WHERE id=?1",
        params![
            task_id,
            match status {
                ResearchStatus::Pending => "pending",
                ResearchStatus::Running => "running",
                ResearchStatus::Completed => "completed",
                ResearchStatus::Canceled => "canceled",
                ResearchStatus::Failed => "failed",
            },
            stop_reason,
            step_count,
            now()
        ],
    )?;
    Ok(())
}

fn deadline_passed(task: &ResearchTask) -> bool {
    let Some(seconds) = task.stop.deadline_seconds else {
        return false;
    };
    chrono::DateTime::parse_from_rfc3339(&task.created_at)
        .ok()
        .is_some_and(|created| {
            Utc::now() - created.with_timezone(&Utc) >= Duration::seconds(i64::from(seconds))
        })
}

impl Library {
    pub fn create_research_task(
        &mut self,
        command_id: &str,
        request: CreateResearchTask,
    ) -> Result<ResearchCommit> {
        let kind = "research.create";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let goal = validate_goal(&request.goal)?;
        let queries = validate_queries(&request.queries)?;
        validate_budget(&request.budget)?;
        if request
            .stop
            .missing_after_step
            .is_some_and(|value| value == 0 || value > 50)
        {
            return Err(Error::InvalidInput(
                "research missing_after_step is out of range",
            ));
        }
        if request
            .stop
            .deadline_seconds
            .is_some_and(|value| value == 0 || value > 86_400)
        {
            return Err(Error::InvalidInput("research deadline is out of range"));
        }
        let endpoint = match (&request.external_search_endpoint, request.external_allowed) {
            (Some(endpoint), true) => Some(validate_endpoint(endpoint)?),
            (None, true) => None,
            (Some(_), false) => return Err(Error::InvalidInput("research_external_not_allowed")),
            (None, false) => None,
        };
        if let Some(collection_id) = request.collection_id.as_deref() {
            valid_uuid(collection_id)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1 AND kind='collection' AND trashed=0)",
                [collection_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        let id = Uuid::new_v4().to_string();
        let at = now();
        tx.execute(
            "INSERT INTO research_tasks(id,goal,collection_id,queries_json,budget_json,stop_json,external_allowed,external_search_endpoint,status,stop_reason,step_count,row_version,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',NULL,0,1,?9,?9)",
            params![
                id,
                goal,
                request.collection_id,
                serde_json::to_string(&queries)?,
                serde_json::to_string(&request.budget)?,
                serde_json::to_string(&request.stop)?,
                request.external_allowed,
                endpoint,
                at
            ],
        )?;
        insert_audit(
            &tx,
            NewAudit {
                kind: "research.created",
                subject_kind: AuditSubjectKind::ResearchTask,
                subject_id: &id,
                summary: format!("创建研究任务：{}", request.goal.trim()),
                origin: "user",
                undo: None,
                change_id: None,
                item_id: None,
            },
        )?;
        let task = read_task(&tx, &id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ResearchCommit { task, receipt };
        store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn research_task(&self, id: &str) -> Result<ResearchTask> {
        read_task(&self.connection, id)
    }

    pub fn research_page(&self, request: ResearchPageRequest) -> Result<ResearchPage> {
        if !(1..=50).contains(&request.limit) {
            return Err(Error::InvalidInput("research page limit is out of range"));
        }
        let before = request
            .before_id
            .as_deref()
            .map(|id| -> Result<(String, String)> {
                let created: String = self.connection.query_row(
                    "SELECT created_at FROM research_tasks WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                Ok((created, id.to_owned()))
            })
            .transpose()?;
        let mut statement = self.connection.prepare(
            "SELECT id,goal,status,stop_reason,step_count,row_version,created_at,updated_at FROM research_tasks
             WHERE (?1 IS NULL OR created_at<?1 OR (created_at=?1 AND id<?2))
             ORDER BY created_at DESC,id DESC LIMIT ?3",
        )?;
        let rows = statement
            .query_map(
                params![
                    before.as_ref().map(|(created, _)| created),
                    before.as_ref().map(|(_, id)| id),
                    request.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, u32>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > request.limit as usize;
        let items = rows
            .into_iter()
            .take(request.limit as usize)
            .map(|row| {
                Ok(ResearchTaskSummary {
                    id: row.0,
                    goal: row.1,
                    status: match row.2.as_str() {
                        "pending" => ResearchStatus::Pending,
                        "running" => ResearchStatus::Running,
                        "completed" => ResearchStatus::Completed,
                        "canceled" => ResearchStatus::Canceled,
                        "failed" => ResearchStatus::Failed,
                        _ => return Err(Error::IntegrityFailure),
                    },
                    stop_reason: row.3,
                    step_count: row.4,
                    row_version: row.5,
                    created_at: row.6,
                    updated_at: row.7,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ResearchPage {
            next_before_id: if more {
                items.last().map(|item| item.id.clone())
            } else {
                None
            },
            items,
        })
    }

    /// Executes exactly one bounded private-library step. The caller decides
    /// when to advance again, so a cancel is observed before the next step.
    pub fn advance_research(
        &mut self,
        command_id: &str,
        request: AdvanceResearch,
    ) -> Result<ResearchCommit> {
        let kind = "research.advance";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let task = read_task(&tx, &request.task_id)?;
        check_version(&task.id, request.base_row_version, task.row_version)?;
        if task.status == ResearchStatus::Canceled {
            return Err(Error::InvalidInput("research_canceled"));
        }
        if !matches!(
            task.status,
            ResearchStatus::Pending | ResearchStatus::Running
        ) {
            return Err(Error::InvalidInput("research_not_running"));
        }
        let mut stop_reason = None;
        if deadline_passed(&task) {
            stop_reason = Some("deadline");
        } else if task.step_count >= task.budget.max_steps {
            stop_reason = Some("budget_steps");
        }
        if let Some(reason) = stop_reason {
            update_task(
                &tx,
                &task.id,
                ResearchStatus::Completed,
                Some(reason),
                task.step_count,
            )?;
            let task = read_task(&tx, &task.id)?;
            let receipt = next_receipt(&tx, command_id)?;
            let result = ResearchCommit { task, receipt };
            store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
            tx.commit()?;
            return Ok(result);
        }
        let Some(query) = task.queries.get(task.step_count as usize).cloned() else {
            update_task(
                &tx,
                &task.id,
                ResearchStatus::Completed,
                Some("queries_exhausted"),
                task.step_count,
            )?;
            let task = read_task(&tx, &task.id)?;
            let receipt = next_receipt(&tx, command_id)?;
            let result = ResearchCommit { task, receipt };
            store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
            tx.commit()?;
            return Ok(result);
        };
        let used = total_evidence(&tx, &task.id)?;
        let remaining = task.budget.max_sources.saturating_sub(used);
        if remaining == 0 {
            update_task(
                &tx,
                &task.id,
                ResearchStatus::Completed,
                Some("budget_sources"),
                task.step_count,
            )?;
            let task = read_task(&tx, &task.id)?;
            let receipt = next_receipt(&tx, command_id)?;
            let result = ResearchCommit { task, receipt };
            store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
            tx.commit()?;
            return Ok(result);
        }
        let limit = remaining.min(20);
        let (matched, sources) = match fts_query(&query) {
            None => (0, Vec::new()),
            Some(match_query) => {
                let matched: u32 = tx.query_row(
                    "SELECT COUNT(*) FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=n.id
                     WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.search_enabled=1
                       AND (?2 IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.collection_id=?2 AND m.member_id=n.id))",
                    params![match_query, task.collection_id],
                    |row| row.get(0),
                )?;
                let mut statement = tx.prepare(
                    "SELECT f.note_id FROM note_fts f JOIN notes n ON n.id=f.note_id JOIN objects o ON o.id=n.id
                     WHERE note_fts MATCH ?1 AND n.trashed=0 AND o.search_enabled=1
                       AND (?2 IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.collection_id=?2 AND m.member_id=n.id))
                     ORDER BY bm25(note_fts,0.0,3.0,1.0),f.note_id LIMIT ?3",
                )?;
                let ids = statement
                    .query_map(params![match_query, task.collection_id, limit], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let mut sources = Vec::new();
                for id in ids {
                    let note = read_note_head(&tx, &id)?;
                    let body = excerpt(&note.document.plain_text());
                    sources.push(ResearchSource {
                        object_id: note.id,
                        revision_id: Some(note.revision_id),
                        title: note.title,
                        origin: ResearchOrigin::PrivateLibrary,
                        url: None,
                        excerpt_hash: hash_excerpt(&body),
                        excerpt: body,
                    });
                }
                (matched, sources)
            }
        };
        let seq = task.step_count + 1;
        let at = now();
        let input = ResearchStepInput::PrivateRetrieval {
            query: query.clone(),
            limit,
        };
        let result = ResearchStepResult::PrivateRetrieval {
            matched,
            returned: sources.len() as u32,
        };
        insert_step(
            &tx,
            NewStep {
                task_id: &task.id,
                seq,
                kind: ResearchStepKind::PrivateRetrieval,
                status: ResearchStepStatus::Succeeded,
                origin: ResearchOrigin::PrivateLibrary,
                input: &input,
                result: &result,
                sources: &sources,
                at: &at,
            },
        )?;
        for source in &sources {
            insert_evidence(&tx, &task.id, seq, source, &at)?;
        }
        let step_count = seq;
        let mut stop_reason = None;
        if total_evidence(&tx, &task.id)? >= task.budget.max_sources {
            stop_reason = Some("budget_sources");
        } else if step_count >= task.budget.max_steps {
            stop_reason = Some("budget_steps");
        } else if step_count as usize >= task.queries.len() {
            stop_reason = Some("queries_exhausted");
        } else if let Some(missing) = task.stop.missing_after_step {
            let tail = read_steps(&tx, &task.id)?;
            let empty_tail = tail
                .iter()
                .rev()
                .take(missing as usize)
                .filter(|step| step.sources.is_empty())
                .count();
            if empty_tail == missing as usize {
                stop_reason = Some("missing_sources");
            }
        }
        update_task(
            &tx,
            &task.id,
            if stop_reason.is_some() {
                ResearchStatus::Completed
            } else {
                ResearchStatus::Running
            },
            stop_reason,
            step_count,
        )?;
        if let Some(reason) = stop_reason {
            insert_audit(
                &tx,
                NewAudit {
                    kind: "research.completed",
                    subject_kind: AuditSubjectKind::ResearchTask,
                    subject_id: &task.id,
                    summary: format!("研究任务停止：{reason}"),
                    origin: "automation",
                    undo: None,
                    change_id: None,
                    item_id: None,
                },
            )?;
        }
        let task = read_task(&tx, &task.id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ResearchCommit { task, receipt };
        store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn cancel_research(
        &mut self,
        command_id: &str,
        request: CancelResearch,
    ) -> Result<ResearchCommit> {
        let kind = "research.cancel";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let task = read_task(&tx, &request.task_id)?;
        check_version(&task.id, request.base_row_version, task.row_version)?;
        if !matches!(
            task.status,
            ResearchStatus::Pending | ResearchStatus::Running
        ) {
            return Err(Error::InvalidInput("research_not_running"));
        }
        let reason = request.reason.trim();
        if reason.chars().count() > 500 {
            return Err(Error::InvalidInput("research cancel reason is too long"));
        }
        update_task(
            &tx,
            &task.id,
            ResearchStatus::Canceled,
            Some(if reason.is_empty() {
                "user_canceled"
            } else {
                reason
            }),
            task.step_count,
        )?;
        insert_audit(
            &tx,
            NewAudit {
                kind: "research.canceled",
                subject_kind: AuditSubjectKind::ResearchTask,
                subject_id: &task.id,
                summary: format!("取消研究任务：{}", task.goal),
                origin: "user",
                undo: None,
                change_id: None,
                item_id: None,
            },
        )?;
        let task = read_task(&tx, &task.id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ResearchCommit { task, receipt };
        store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    /// Replays the host command before it plans or repeats an external GET.
    ///
    /// The user request contains only the command ID, task ID and base version;
    /// the full recording hash also contains the response, which a caller that
    /// lost its reply does not have. Both external recorders update the task
    /// exactly once, so their immutable receipt snapshot identifies the original
    /// base as `row_version - 1`. Never compare with the task's current version:
    /// later steps or a cancel must not change a committed command's receipt.
    pub fn replay_external_research(
        &self,
        command_id: &str,
        task_id: &str,
        base_row_version: u32,
    ) -> Result<Option<ResearchCommit>> {
        valid_uuid(command_id)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let saved = snapshot
            .query_row(
                "SELECT payload_hash,command_kind,object_id FROM command_receipts WHERE command_id=?1",
                [command_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?;
        let Some((hash, kind, object_id)) = saved else {
            return Ok(None);
        };
        if object_id != task_id
            || !matches!(
                kind.as_str(),
                RECORD_EXTERNAL_KIND | RECORD_EXTERNAL_FAILURE_KIND
            )
        {
            return Err(Error::IdempotencyMismatch);
        }
        // The stored full-response hash is authoritative only for this narrower
        // host replay. Direct record_* calls still compare the caller's full
        // payload hash using the ordinary mutation replay path.
        let result: ResearchCommit =
            replay(&snapshot, command_id, &hash, &kind)?.ok_or(Error::IntegrityFailure)?;
        let task = &result.task;
        if task.id != object_id || task.row_version <= 1 {
            return Err(Error::IntegrityFailure);
        }
        if task.row_version - 1 != base_row_version {
            return Err(Error::IdempotencyMismatch);
        }
        let step = task.steps.last().ok_or(Error::IntegrityFailure)?;
        let ResearchStepInput::ExternalSearch { query, endpoint } = &step.input else {
            return Err(Error::IntegrityFailure);
        };
        let valid_result = if kind == RECORD_EXTERNAL_KIND {
            step.status == ResearchStepStatus::Succeeded
                && matches!(step.result, ResearchStepResult::ExternalSearch { .. })
                && matches!(
                    task.status,
                    ResearchStatus::Running | ResearchStatus::Completed
                )
        } else {
            step.status == ResearchStepStatus::Failed
                && matches!(step.result, ResearchStepResult::Failed { .. })
                && task.status == ResearchStatus::Failed
        };
        if !valid_result
            || step.task_id != task.id
            || step.seq != task.step_count
            || task.steps.len() != task.step_count as usize
            || step.kind != ResearchStepKind::ExternalSearch
            || step.origin != ResearchOrigin::External
            || !task.external_allowed
            || task.external_search_endpoint.as_deref() != Some(endpoint.as_str())
            || step
                .seq
                .checked_sub(1)
                .and_then(|index| task.queries.get(index as usize))
                != Some(query)
        {
            return Err(Error::IntegrityFailure);
        }
        Ok(Some(result))
    }

    /// Records the result of one external GET performed by the desktop host.
    /// The query and endpoint must match the declaration; a canceled task
    /// rejects the late result without adding a step.
    pub fn record_external_step(
        &mut self,
        command_id: &str,
        request: RecordExternalStep,
    ) -> Result<ResearchCommit> {
        let kind = RECORD_EXTERNAL_KIND;
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let task = prepare_external(
            &tx,
            &request.task_id,
            request.base_row_version,
            &request.query,
            &request.endpoint,
        )?;
        if request.excerpt.chars().count() > MAX_EVIDENCE_EXCERPT {
            return Err(Error::InvalidInput("research_external_excerpt_too_long"));
        }
        let seq = task.step_count + 1;
        let at = now();
        let input = ResearchStepInput::ExternalSearch {
            query: request.query.clone(),
            endpoint: request.endpoint.clone(),
        };
        let result = ResearchStepResult::ExternalSearch {
            http_status: request.http_status,
            body_bytes: request.excerpt.len() as u32,
        };
        let body = request
            .excerpt
            .trim()
            .chars()
            .take(MAX_EVIDENCE_EXCERPT)
            .collect::<String>();
        let source = ResearchSource {
            object_id: request.final_url.clone(),
            revision_id: None,
            title: request.title.trim().chars().take(300).collect(),
            origin: ResearchOrigin::External,
            url: Some(request.final_url.clone()),
            excerpt_hash: hash_excerpt(&body),
            excerpt: body,
        };
        insert_step(
            &tx,
            NewStep {
                task_id: &task.id,
                seq,
                kind: ResearchStepKind::ExternalSearch,
                status: ResearchStepStatus::Succeeded,
                origin: ResearchOrigin::External,
                input: &input,
                result: &result,
                sources: std::slice::from_ref(&source),
                at: &at,
            },
        )?;
        insert_evidence(&tx, &task.id, seq, &source, &at)?;
        let mut stop_reason = None;
        if total_evidence(&tx, &task.id)? >= task.budget.max_sources {
            stop_reason = Some("budget_sources");
        } else if seq >= task.budget.max_steps {
            stop_reason = Some("budget_steps");
        } else if seq as usize >= task.queries.len() {
            stop_reason = Some("queries_exhausted");
        }
        update_task(
            &tx,
            &task.id,
            if stop_reason.is_some() {
                ResearchStatus::Completed
            } else {
                ResearchStatus::Running
            },
            stop_reason,
            seq,
        )?;
        let task = read_task(&tx, &task.id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ResearchCommit { task, receipt };
        store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn record_failed_external_step(
        &mut self,
        command_id: &str,
        request: RecordFailedExternalStep,
    ) -> Result<ResearchCommit> {
        let kind = RECORD_EXTERNAL_FAILURE_KIND;
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let task = prepare_external(
            &tx,
            &request.task_id,
            request.base_row_version,
            &request.query,
            &request.endpoint,
        )?;
        let seq = task.step_count + 1;
        let at = now();
        let input = ResearchStepInput::ExternalSearch {
            query: request.query.clone(),
            endpoint: request.endpoint.clone(),
        };
        let result = ResearchStepResult::Failed {
            code: request.error_code.trim().chars().take(120).collect(),
        };
        insert_step(
            &tx,
            NewStep {
                task_id: &task.id,
                seq,
                kind: ResearchStepKind::ExternalSearch,
                status: ResearchStepStatus::Failed,
                origin: ResearchOrigin::External,
                input: &input,
                result: &result,
                sources: &[],
                at: &at,
            },
        )?;
        update_task(
            &tx,
            &task.id,
            ResearchStatus::Failed,
            Some("external_failed"),
            seq,
        )?;
        insert_audit(
            &tx,
            NewAudit {
                kind: "research.external_failed",
                subject_kind: AuditSubjectKind::ResearchTask,
                subject_id: &task.id,
                summary: format!("外部检索失败：{}", request.error_code),
                origin: "automation",
                undo: None,
                change_id: None,
                item_id: None,
            },
        )?;
        let task = read_task(&tx, &task.id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ResearchCommit { task, receipt };
        store_receipt(&tx, &hash, kind, &request.task_id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    /// Runs private steps until the task stops. Each iteration reloads the
    /// task, so a cancel committed by another connection ends the loop.
    pub fn run_research_steps(
        &mut self,
        task_id: &str,
        max_iterations: u32,
    ) -> Result<ResearchTask> {
        for _ in 0..max_iterations.min(MAX_RESEARCH_STEPS) {
            let task = read_task(&self.connection, task_id)?;
            if !matches!(
                task.status,
                ResearchStatus::Pending | ResearchStatus::Running
            ) {
                return Ok(task);
            }
            self.advance_research(
                &Uuid::new_v4().to_string(),
                AdvanceResearch {
                    task_id: task.id.clone(),
                    base_row_version: task.row_version,
                },
            )?;
        }
        read_task(&self.connection, task_id)
    }
}

fn prepare_external(
    connection: &Connection,
    task_id: &str,
    base_row_version: u32,
    query: &str,
    endpoint: &str,
) -> Result<ResearchTask> {
    let task = read_task(connection, task_id)?;
    if task.status == ResearchStatus::Canceled {
        return Err(Error::InvalidInput("research_canceled"));
    }
    if !matches!(
        task.status,
        ResearchStatus::Pending | ResearchStatus::Running
    ) {
        return Err(Error::InvalidInput("research_not_running"));
    }
    check_version(&task.id, base_row_version, task.row_version)?;
    if !task.external_allowed {
        return Err(Error::InvalidInput("research_external_not_allowed"));
    }
    if task.external_search_endpoint.as_deref() != Some(endpoint) {
        return Err(Error::InvalidInput("research_external_endpoint_mismatch"));
    }
    if task
        .queries
        .get(task.step_count as usize)
        .map(String::as_str)
        != Some(query)
    {
        return Err(Error::InvalidInput("research_external_query_mismatch"));
    }
    Ok(task)
}
impl Library {
    pub fn review_insight_settings(&self) -> Result<ReviewInsightSettings> {
        read_review_settings(&self.connection)
    }

    pub fn set_review_insight_settings(
        &mut self,
        command_id: &str,
        request: SetReviewInsightSettings,
    ) -> Result<ReviewSettingsCommit> {
        let kind = "review.insight_settings";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let previous = read_review_settings(&tx)?;
        check_version(
            "review_insight_settings",
            request.base_row_version,
            previous.row_version,
        )?;
        if !(1..=365).contains(&request.window_days) {
            return Err(Error::InvalidInput("review insight window is out of range"));
        }
        tx.execute(
            "UPDATE review_insight_settings SET enabled=?1,window_days=?2,include_completed=?3,include_stale=?4,row_version=row_version+1 WHERE singleton=1",
            params![
                request.enabled,
                request.window_days,
                request.include_completed,
                request.include_stale
            ],
        )?;
        insert_audit(
            &tx,
            NewAudit {
                kind: "review.insight_settings",
                subject_kind: AuditSubjectKind::ReviewSetting,
                subject_id: "review_insight_settings",
                summary: format!(
                    "可解释回顾{}（窗口 {} 天）",
                    if request.enabled {
                        "已开启"
                    } else {
                        "已关闭"
                    },
                    request.window_days
                ),
                origin: "user",
                undo: Some(json!({
                    "kind": "review_settings",
                    "enabled": previous.enabled,
                    "window_days": previous.window_days,
                    "include_completed": previous.include_completed,
                    "include_stale": previous.include_stale,
                })),
                change_id: None,
                item_id: None,
            },
        )?;
        let settings = read_review_settings(&tx)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ReviewSettingsCommit { settings, receipt };
        store_receipt(
            &tx,
            &hash,
            kind,
            "review_insight_settings",
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn review_insights(&self, limit: u32) -> Result<ReviewInsights> {
        self.review_insights_at(limit, Utc::now())
    }

    /// Deterministic variant: evaluates windows as of the supplied instant and
    /// never rewrites stored timestamps.
    pub fn review_insights_at(
        &self,
        limit: u32,
        as_of: chrono::DateTime<Utc>,
    ) -> Result<ReviewInsights> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("review insight limit is out of range"));
        }
        let settings = read_review_settings(&self.connection)?;
        let disclaimer = "点击、完成标记与模型分数都不等于掌握程度；这里只依据你选择的使用记录给出可核对的提示。".to_owned();
        if !settings.enabled {
            return Ok(ReviewInsights {
                enabled: false,
                settings,
                items: vec![],
                empty_reason: Some("review_insights_disabled".into()),
                disclaimer,
            });
        }
        let cutoff = (as_of - Duration::days(i64::from(settings.window_days))).to_rfc3339();
        let caveat = "仅基于回顾记录与笔记更新时间，不评价你的理解程度。".to_owned();
        let mut items = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut statement = self.connection.prepare(
            "SELECT r.object_id,n.title,r.due_at FROM review_entries r JOIN notes n ON n.id=r.object_id
             WHERE r.state='pending' AND r.due_at<?1 AND n.trashed=0 ORDER BY r.due_at LIMIT ?2",
        )?;
        let overdue = statement
            .query_map(params![as_of.to_rfc3339(), limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (object_id, title, due_at) in overdue {
            if !seen.insert((object_id.clone(), "gap")) {
                continue;
            }
            items.push(ReviewInsight {
                object_id,
                title,
                kind: ReviewInsightKind::Gap,
                reason: "回顾已过期仍未完成".into(),
                basis: vec![ReviewInsightBasis {
                    kind: "review_due".into(),
                    occurred_at: Some(due_at.clone()),
                    detail: format!("到期时间 {due_at}"),
                }],
                caveat: caveat.clone(),
            });
        }
        if settings.include_completed {
            let mut statement = self.connection.prepare(
                "SELECT r.object_id,n.title,r.updated_at FROM review_entries r JOIN notes n ON n.id=r.object_id
                 WHERE r.state='completed' AND r.updated_at>=?1 AND n.trashed=0 AND n.updated_at<=r.updated_at
                 ORDER BY r.updated_at DESC LIMIT ?2",
            )?;
            let completed = statement
                .query_map(params![cutoff, limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (object_id, title, completed_at) in completed {
                if !seen.insert((object_id.clone(), "revisit")) {
                    continue;
                }
                items.push(ReviewInsight {
                    object_id,
                    title,
                    kind: ReviewInsightKind::Revisit,
                    reason: "已完成回顾但笔记之后没有更新，可能值得重访".into(),
                    basis: vec![ReviewInsightBasis {
                        kind: "review_completed".into(),
                        occurred_at: Some(completed_at.clone()),
                        detail: format!("完成于 {completed_at}，此后正文无更新"),
                    }],
                    caveat: caveat.clone(),
                });
            }
        }
        if settings.include_stale {
            let mut statement = self.connection.prepare(
                "SELECT n.id,n.title,n.updated_at FROM notes n JOIN objects o ON o.id=n.id
                 WHERE n.trashed=0 AND o.archived=0 AND n.updated_at<?1
                   AND NOT EXISTS(SELECT 1 FROM review_entries r WHERE r.object_id=n.id)
                   AND EXISTS(SELECT 1 FROM object_tags t WHERE t.object_id=n.id)
                 ORDER BY n.updated_at ASC LIMIT ?2",
            )?;
            let stale = statement
                .query_map(params![cutoff, limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (object_id, title, updated_at) in stale {
                if !seen.insert((object_id.clone(), "stale")) {
                    continue;
                }
                items.push(ReviewInsight {
                    object_id,
                    title,
                    kind: ReviewInsightKind::Stale,
                    reason: "有标签但从未安排回顾".into(),
                    basis: vec![ReviewInsightBasis {
                        kind: "note_updated".into(),
                        occurred_at: Some(updated_at.clone()),
                        detail: format!("最近更新于 {updated_at}，没有回顾记录"),
                    }],
                    caveat: caveat.clone(),
                });
            }
        }
        let empty_reason = if items.is_empty() {
            Some("no_review_insights".into())
        } else {
            None
        };
        Ok(ReviewInsights {
            enabled: true,
            settings,
            items,
            empty_reason,
            disclaimer,
        })
    }
}

fn read_review_settings(connection: &Connection) -> Result<ReviewInsightSettings> {
    connection
        .query_row(
            "SELECT enabled,window_days,include_completed,include_stale,row_version FROM review_insight_settings WHERE singleton=1",
            [],
            |row| {
                Ok(ReviewInsightSettings {
                    enabled: row.get(0)?,
                    window_days: row.get(1)?,
                    include_completed: row.get(2)?,
                    include_stale: row.get(3)?,
                    row_version: row.get(4)?,
                })
            },
        )
        .optional()?
        .ok_or(Error::IntegrityFailure)
}

fn read_note_head(connection: &Connection, id: &str) -> Result<crate::Note> {
    let revision_id: String = connection
        .query_row(
            "SELECT current_revision_id FROM notes WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    note_revision(connection, id, &revision_id)
}
