//! Knowledge reuse tasks with optional prompt templates, real generation, and reviewed outputs.
use crate::{
    ChatPurpose, ChatRunOptions, Error, KnowledgeTask, KnowledgeTaskMutation, KnowledgeTaskOutput,
    KnowledgeTaskOutputsPage, KnowledgeTaskOutputsRequest, KnowledgeTaskSummary,
    KnowledgeTaskTemplateSnapshot, KnowledgeTasksPage, KnowledgeTasksRequest, Library, Result,
    SearchScope, StartChatRun, StartKnowledgeTask, document::valid_uuid, mutations,
};
use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;
use uuid::Uuid;

fn validate_input(input: &StartKnowledgeTask) -> Result<()> {
    if mutations::validate_name(&input.title)? != input.title
        || input.instructions.trim().is_empty()
        || input.instructions.chars().count() > 8000
        || input.instructions.contains('\0')
        || !matches!(input.language.as_str(), "zh-CN" | "en")
    {
        return Err(Error::InvalidInput("knowledge_task_input_invalid"));
    }
    if input.materials.is_empty() {
        return Err(Error::InvalidInput("knowledge_task_materials_required"));
    }
    if input.materials.len() > 32
        || input
            .materials
            .iter()
            .map(|e| e.id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != input.materials.len()
        || input
            .materials
            .iter()
            .try_fold(0usize, |total, e| total.checked_add(e.text.len()))
            .is_none_or(|n| n > 256 * 1024)
    {
        return Err(Error::InvalidInput("knowledge_task_material_limit"));
    }
    if let Some(period) = &input.period {
        let date = |s: &str| -> Result<NaiveDate> {
            let value = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| Error::InvalidInput("knowledge_task_period_invalid"))?;
            if value.format("%Y-%m-%d").to_string() != s {
                return Err(Error::InvalidInput("knowledge_task_period_invalid"));
            }
            Ok(value)
        };
        if date(&period.start)? > date(&period.end)? {
            return Err(Error::InvalidInput("knowledge_task_period_invalid"));
        }
    }
    if let Some(template) = &input.template {
        valid_uuid(&template.id)?;
        if template.row_version == 0 {
            return Err(Error::InvalidInput("task_template_version_invalid"));
        }
    }
    if let Some(id) = &input.retry_of {
        valid_uuid(id)?;
    }
    Ok(())
}

fn scope(input: &StartKnowledgeTask) -> SearchScope {
    SearchScope {
        object_ids: input
            .materials
            .iter()
            .map(|e| {
                e.annotation
                    .as_ref()
                    .map_or_else(|| e.target.object_id.clone(), |a| a.annotation_id.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        ..Default::default()
    }
}

fn prompt(input: &StartKnowledgeTask) -> String {
    let period = input
        .period
        .as_ref()
        .map(|p| format!("{} — {}", p.start, p.end))
        .unwrap_or_else(|| "unspecified".into());
    format!(
        "Complete the user's knowledge task from the explicitly supplied materials.\nOutput language: {}.\nTask title: {}\nReporting period: {period}. This is context, not a claim about when the inputs were created.\nUser requirements:\n{}\n\nUse the supplied material for factual claims. State missing information and uncertainty; do not invent people, dates, measurements or quotations. Cite supplied material with [1], [2], etc., matching its order. Separate source-supported statements from interpretation and proposed next steps. Return readable, editable Markdown.",
        input.language, input.title, input.instructions
    )
}

fn output_rows(
    db: &Connection,
    id: &str,
    page: u32,
    limit: u32,
) -> Result<KnowledgeTaskOutputsPage> {
    let total: u32 = db.query_row(
        "SELECT count(*) FROM knowledge_task_outputs WHERE task_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let page = page.min(total.max(1).div_ceil(limit));
    let items = db.prepare("SELECT id,proposal_id,title,status,target_note_id,applied_note_id,applied_revision_id,created_at,
        EXISTS(SELECT 1 FROM notes n WHERE n.id=a.applied_note_id),
        coalesce((SELECT trashed FROM notes n WHERE n.id=a.applied_note_id),0)
        FROM knowledge_task_outputs a WHERE task_id=?1 ORDER BY created_at DESC,id DESC LIMIT ?2 OFFSET ?3")?
        .query_map(params![id,limit,i64::from(page-1)*i64::from(limit)], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,bool>(8)?,r.get::<_,bool>(9)?)))?
        .map(|r| { let r=r?; Ok(KnowledgeTaskOutput { id:r.0,proposal_id:r.1,title:r.2,status:serde_json::from_value(serde_json::Value::String(r.3))?,target_note_id:r.4,applied_note_id:r.5,applied_revision_id:r.6,created_at:r.7,note_available:r.8,note_trashed:r.9 }) })
        .collect::<Result<Vec<_>>>()?;
    Ok(KnowledgeTaskOutputsPage {
        items,
        total,
        page,
        limit,
    })
}

fn read(db: &Connection, id: &str) -> Result<KnowledgeTask> {
    valid_uuid(id)?;
    let (schema,input,template_snapshot,channel_name): (u32,String,Option<String>,Option<String>) = db.query_row("SELECT input_schema,input_json,template_snapshot_json,channel_name FROM knowledge_tasks WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or(Error::NotFound)?;
    if schema != 1 {
        return Err(Error::UnsupportedDatabase);
    }
    let input: StartKnowledgeTask = serde_json::from_str(&input)?;
    let template_snapshot: Option<KnowledgeTaskTemplateSnapshot> = template_snapshot
        .map(|s| serde_json::from_str(&s))
        .transpose()?;
    let template_current = match &template_snapshot {
        Some(template) => {
            db.query_row(
                "SELECT row_version FROM knowledge_task_templates WHERE id=?1",
                [&template.id],
                |r| r.get::<_, u32>(0),
            )
            .optional()?
                == Some(template.row_version)
        }
        None => true,
    };
    let run = crate::chat::run(db, id)?;
    let output_text = db.query_row(
        "SELECT content FROM chat_messages WHERE id=?1 AND run_id=?2 AND role='assistant'",
        params![run.assistant_message_id, id],
        |r| r.get(0),
    )?;
    let outputs = output_rows(db, id, 1, 1)?;
    let mut materials_current = true;
    let scope = scope(&input);
    for material in &input.materials {
        materials_current &= crate::retrieval::validate_evidence(db, &scope, material)?;
    }
    Ok(KnowledgeTask {
        id: id.into(),
        input,
        template_snapshot,
        template_current,
        channel_name,
        run,
        output_text,
        latest_output: outputs.items.into_iter().next(),
        output_count: outputs.total,
        materials_current,
    })
}

impl Library {
    pub fn start_knowledge_task(
        &mut self,
        command: &str,
        input: StartKnowledgeTask,
    ) -> Result<KnowledgeTaskMutation> {
        let kind = "knowledge_tasks.start";
        let hash = mutations::request_hash(command, kind, &input)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(replayed) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(replayed);
        }
        validate_input(&input)?;
        let settings = crate::assistant::generation_settings(&tx)?;
        if input.generation_settings_version != settings.row_version {
            return Err(Error::MetadataConflict {
                object_id: "generation_settings".into(),
                expected: input.generation_settings_version,
                actual: settings.row_version,
            });
        }
        let template_snapshot = if let Some(reference) = &input.template {
            let template =
                crate::task_templates::read(&tx, &reference.id).map_err(|error| match error {
                    Error::NotFound => Error::InvalidInput("task_template_missing"),
                    other => other,
                })?;
            mutations::check_version(&reference.id, reference.row_version, template.row_version)?;
            Some(KnowledgeTaskTemplateSnapshot::from(&template))
        } else {
            None
        };
        if let Some(previous) = &input.retry_of
            && !read(&tx, previous)?.run.status.is_terminal()
        {
            return Err(Error::InvalidInput("knowledge_task_retry_invalid"));
        }
        let channel_name: Option<String> = match settings
            .profile
            .as_ref()
            .and_then(|p| p.channel_id.as_ref())
        {
            Some(id) => tx
                .query_row(
                    "SELECT json_extract(spec_json,'$.name') FROM generation_channels WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten(),
            None => None,
        };
        let conversation = Uuid::new_v4().to_string();
        let at = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO chat_conversations VALUES(?1,?2,1,?3,?3)",
            params![conversation, input.title, at],
        )?;
        let id = crate::chat::queue_chat_run(
            &tx,
            StartChatRun {
                conversation_id: conversation,
                base_row_version: 1,
                text: prompt(&input),
                purpose: ChatPurpose::Chat,
                materials: input.materials.clone(),
                scope: scope(&input),
                use_knowledge_tools: false,
                options: Some(
                    input.options.clone().unwrap_or(ChatRunOptions {
                        max_model_calls: 1,
                        max_tool_calls: 0,
                        max_output_tokens: settings
                            .profile
                            .as_ref()
                            .map_or(4096, |model| (model.context_tokens / 2).clamp(1, 4096)),
                        total_timeout_seconds: 600,
                    }),
                ),
            },
        )?;
        tx.execute("INSERT INTO knowledge_tasks(id,template_id,template_snapshot_json,title,input_schema,input_json,channel_name,retry_of,created_at) VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8)",params![id,template_snapshot.as_ref().map(|t|&t.id),template_snapshot.as_ref().map(serde_json::to_string).transpose()?,input.title,serde_json::to_string(&input)?,channel_name,input.retry_of,at])?;
        let result = KnowledgeTaskMutation {
            task: read(&tx, &id)?,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn knowledge_task(&self, id: &str) -> Result<KnowledgeTask> {
        let snapshot = self.connection.unchecked_transaction()?;
        read(&snapshot, id)
    }

    pub fn knowledge_task_outputs(
        &self,
        request: KnowledgeTaskOutputsRequest,
    ) -> Result<KnowledgeTaskOutputsPage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("knowledge_task_page_invalid"));
        }
        valid_uuid(&request.task_id)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let exists: bool = snapshot.query_row(
            "SELECT EXISTS(SELECT 1 FROM knowledge_tasks WHERE id=?1)",
            [&request.task_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        output_rows(&snapshot, &request.task_id, request.page, request.limit)
    }

    pub fn knowledge_tasks(&self, request: KnowledgeTasksRequest) -> Result<KnowledgeTasksPage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("knowledge_task_page_invalid"));
        }
        if let Some(id) = &request.template_id {
            valid_uuid(id)?;
        }
        let query = request
            .query
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if query.chars().count() > 512 || query.chars().any(char::is_control) {
            return Err(Error::InvalidInput("knowledge_task_query_invalid"));
        }
        let snapshot = self.connection.unchecked_transaction()?;
        let condition = "FROM knowledge_tasks t JOIN chat_runs r ON r.id=t.id WHERE (?1 IS NULL OR json_extract(t.template_snapshot_json,'$.id')=?1) AND (?2='' OR instr(lower(t.title),?2)>0)";
        let total: u32 = snapshot.query_row(
            &format!("SELECT count(*) {condition}"),
            params![request.template_id, query],
            |r| r.get(0),
        )?;
        let page = request.page.min(total.max(1).div_ceil(request.limit));
        let rows=snapshot.prepare(&format!("SELECT t.id,t.title,json_extract(t.template_snapshot_json,'$.name'),r.status,r.model_json,t.channel_name,t.created_at,r.updated_at,(SELECT count(*) FROM knowledge_task_outputs o WHERE o.task_id=t.id) {condition} ORDER BY t.created_at DESC,t.id DESC LIMIT ?3 OFFSET ?4"))?
            .query_map(params![request.template_id,query,request.limit,i64::from(page-1)*i64::from(request.limit)],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,u32>(8)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let items = rows
            .into_iter()
            .map(|r| {
                Ok(KnowledgeTaskSummary {
                    id: r.0,
                    title: r.1,
                    template_name: r.2,
                    status: serde_json::from_value(serde_json::Value::String(r.3))?,
                    model: r.4.map(|s| serde_json::from_str(&s)).transpose()?,
                    channel_name: r.5,
                    created_at: r.6,
                    updated_at: r.7,
                    output_count: r.8,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(KnowledgeTasksPage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    crate::task_templates::validate_backup(db)?;
    let rows=db.prepare("SELECT id,input_schema,input_json,retry_of,created_at,title,channel_name,template_snapshot_json,template_id FROM knowledge_tasks")?
        .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<String>>(8)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    for (id, schema, json, retry, created, title, channel, snapshot, template_id) in rows {
        valid_uuid(&id)?;
        if schema != 1 {
            return Err(Error::UnsupportedDatabase);
        }
        let input: StartKnowledgeTask = serde_json::from_str(&json)?;
        validate_input(&input)?;
        let run = crate::chat::run(db, &id)?;
        if input.title != title
            || input.retry_of != retry
            || retry.as_deref() == Some(id.as_str())
            || run.request.materials != input.materials
            || run.request.scope != scope(&input)
            || run.request.use_knowledge_tools
            || run.request.purpose != ChatPurpose::Chat
            || chrono::DateTime::parse_from_rfc3339(&created).is_err()
            || channel.is_some_and(|name| name.chars().count() > 300 || name.contains('\0'))
        {
            return Err(Error::IntegrityFailure);
        }
        let snapshot: Option<KnowledgeTaskTemplateSnapshot> =
            snapshot.map(|s| serde_json::from_str(&s)).transpose()?;
        match (&input.template, &snapshot) {
            (None, None) if template_id.is_none() => {}
            (Some(reference), Some(template))
                if reference.id == template.id && reference.row_version == template.row_version =>
            {
                valid_uuid(&template.id)?;
                crate::task_templates::fields(
                    &template.name,
                    &template.description,
                    &template.instructions,
                )?;
                if template.row_version == 0
                    || template_id.as_ref().is_some_and(|id| id != &template.id)
                {
                    return Err(Error::IntegrityFailure);
                }
            }
            _ => return Err(Error::IntegrityFailure),
        }
    }
    let inconsistent: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM knowledge_task_outputs a LEFT JOIN proposals p ON p.id=a.proposal_id
         WHERE (a.proposal_id IS NOT NULL AND (p.id IS NULL OR a.id!=p.id
          OR json_extract(p.request_json,'$.chat_run_id') IS NOT a.task_id
          OR json_extract(p.request_json,'$.title') IS NOT a.title OR p.status!=a.status
          OR json_extract(p.request_json,'$.note_id') IS NOT a.target_note_id
          OR p.applied_revision_id IS NOT a.applied_revision_id
          OR (SELECT note_id FROM note_revisions WHERE revision_id=p.applied_revision_id) IS NOT a.applied_note_id))
         OR (a.proposal_id IS NULL AND EXISTS(SELECT 1 FROM proposals q WHERE q.id=a.id)))",
        [], |r| r.get(0),
    )?;
    if inconsistent {
        return Err(Error::IntegrityFailure);
    }
    let outputs = db.prepare("SELECT id,proposal_id,target_note_id,applied_note_id,applied_revision_id,created_at FROM knowledge_task_outputs")?
        .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,String>(5)?)))?
        .collect::<std::result::Result<Vec<_>,_>>()?;
    for (id, proposal, target, note, revision, created) in outputs {
        for id in std::iter::once(&id)
            .chain(proposal.iter())
            .chain(target.iter())
            .chain(note.iter())
            .chain(revision.iter())
        {
            valid_uuid(id)?;
        }
        if note.is_some() != revision.is_some()
            || chrono::DateTime::parse_from_rfc3339(&created).is_err()
        {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}
