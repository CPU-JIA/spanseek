//! Free-form conversations have independent completion semantics from evidence answers.
use crate::*;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;
const MAX_CONTENT: usize = 4 * 1024 * 1024;
fn encode<T: Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}
fn value<T: DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_str(&s)?)
}
fn tag<T: Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .ok_or(Error::IntegrityFailure)?
        .into())
}
fn limit(n: u32) -> Result<()> {
    if !(1..=100).contains(&n) {
        return Err(Error::InvalidInput("invalid chat page size"));
    }
    Ok(())
}
pub fn validate_chat_run_options(
    o: &ChatRunOptions,
    model: Option<&GenerationProfile>,
) -> Result<()> {
    if !(1..=32).contains(&o.max_model_calls)
        || o.max_tool_calls > 64
        || !(1..=32768).contains(&o.max_output_tokens)
        || !(1..=3600).contains(&o.total_timeout_seconds)
        || model.is_some_and(|p| o.max_output_tokens >= p.context_tokens)
    {
        return Err(Error::InvalidInput("invalid_chat_options"));
    }
    Ok(())
}
pub(crate) fn conversation(db: &Connection, id: &str) -> Result<ChatConversation> {
    db.query_row(
        "SELECT id,title,row_version,created_at,updated_at FROM chat_conversations WHERE id=?1",
        [id],
        |r| {
            Ok(ChatConversation {
                id: r.get(0)?,
                title: r.get(1)?,
                row_version: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound)
}
pub(crate) fn run(db: &Connection, id: &str) -> Result<ChatRun> {
    let row=db.query_row("SELECT conversation_id,user_message_id,assistant_message_id,status,request_json,model_json,last_event_seq,error_code,created_at,updated_at,usage_json,finish_reason FROM chat_runs WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,u32>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,String>(10)?,r.get::<_,Option<String>>(11)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(ChatRun {
        id: id.into(),
        conversation_id: row.0,
        user_message_id: row.1,
        assistant_message_id: row.2,
        status: value(encode(&row.3)?)?,
        request: value(row.4)?,
        model: row.5.map(value).transpose()?,
        last_event_seq: row.6,
        error_code: row.7,
        created_at: row.8,
        updated_at: row.9,
        usage: value(row.10)?,
        finish_reason: row.11,
    })
}
fn lease(db: &Connection, id: &str, token: &str) -> Result<()> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND status='running' AND lease_token=?2 AND lease_expires_at>?3)",params![id,token,Utc::now().to_rfc3339()],|r|r.get(0))?;
    if !valid {
        return Err(Error::InvalidInput("chat_lease_lost"));
    }
    Ok(())
}
fn event(db: &Connection, id: &str, status: ChatRunStatus, delta: Option<&str>) -> Result<()> {
    let next = run(db, id)?
        .last_event_seq
        .checked_add(1)
        .ok_or(Error::IntegrityFailure)?;
    db.execute(
        "INSERT INTO chat_run_events(run_id,seq,kind,text_delta,status) VALUES(?1,?2,?3,?4,?5)",
        params![
            id,
            next,
            if delta.is_some() {
                "text_delta"
            } else {
                "status"
            },
            delta,
            tag(&status)?
        ],
    )?;
    db.execute(
        "UPDATE chat_runs SET last_event_seq=?2,updated_at=?3 WHERE id=?1",
        params![id, next, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
fn finish(
    db: &Connection,
    id: &str,
    status: ChatRunStatus,
    code: Option<&str>,
    usage: &ChatUsage,
    reason: Option<&str>,
) -> Result<()> {
    db.execute("UPDATE chat_runs SET status=?2,error_code=?3,usage_json=?4,finish_reason=?5,lease_token=NULL,lease_expires_at=NULL WHERE id=?1",params![id,tag(&status)?,code,encode(usage)?,reason])?;
    event(db, id, status, None)
}
fn mutation(
    db: &Connection,
    id: &str,
    command: &str,
    hash: &str,
    kind: &str,
) -> Result<ChatRunMutation> {
    let run = run(db, id)?;
    let result = ChatRunMutation {
        conversation: conversation(db, &run.conversation_id)?,
        run,
        receipt: mutations::next_receipt(db, command)?,
    };
    mutations::store_receipt(db, hash, kind, id, &result.receipt, &result)?;
    Ok(result)
}
fn validate_request(
    db: &Connection,
    r: &StartChatRun,
    model: Option<&GenerationProfile>,
) -> Result<()> {
    document::valid_uuid(&r.conversation_id)?;
    if r.text.trim().is_empty()
        || r.text.len() > 256 * 1024
        || r.text.contains('\0')
        || r.base_row_version == 0
    {
        return Err(Error::InvalidInput("invalid_chat_request"));
    }
    validate_chat_run_options(&r.options.clone().unwrap_or_default(), model)?;
    if let Some(p) = model {
        validate_generation_profile(p)?;
        if p.provider == "ollama" {
            return Err(Error::InvalidInput("chat_remote_model_required"));
        }
    }
    validate_scope(&r.scope)?;
    validate_materials(db, &r.scope, &r.materials)
}
fn validate_scope(scope: &SearchScope) -> Result<()> {
    if scope.object_ids.len() > 100 || scope.tag_ids.len() > 100 || scope.kinds.len() > 2 {
        return Err(Error::InvalidInput("chat_scope_limit"));
    }
    for id in scope
        .object_ids
        .iter()
        .chain(scope.tag_ids.iter())
        .chain(scope.collection_id.iter())
    {
        document::valid_uuid(id)?;
    }
    Ok(())
}
fn validate_materials(
    db: &Connection,
    scope: &SearchScope,
    items: &[SearchEvidence],
) -> Result<()> {
    if items.len() > 32
        || items
            .iter()
            .try_fold(0usize, |n, e| n.checked_add(e.text.len()))
            .is_none_or(|n| n > 256 * 1024)
    {
        return Err(Error::InvalidInput("chat_material_limit"));
    }
    for item in items {
        if !retrieval::validate_evidence(db, scope, item)? {
            return Err(Error::InvalidInput("chat_evidence_invalid"));
        }
    }
    Ok(())
}
/// Shared enqueue operation for a conversation or an application run. The caller
/// owns the transaction and its receipt, so app identity and the real task commit together.
pub(crate) fn queue_chat_run(db: &Connection, mut request: StartChatRun) -> Result<String> {
    let model: Option<String> = db.query_row(
        "SELECT generation_profile_json FROM search_settings WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let model: Option<GenerationProfile> = model.map(value).transpose()?;
    validate_request(db, &request, model.as_ref())?;
    // Hash the user's exact intent above, but persist the resolved budget so
    // future default changes cannot alter an already accepted run.
    request.options = Some(request.options.take().unwrap_or_default());
    let c = conversation(db, &request.conversation_id)?;
    if c.row_version != request.base_row_version {
        return Err(Error::InvalidInput("chat_conversation_conflict"));
    }
    let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM chat_runs WHERE conversation_id=?1 AND status IN ('queued','running'))",[&c.id],|r|r.get(0))?;
    if active {
        return Err(Error::InvalidInput("chat_run_active"));
    }
    let next: u32 = db.query_row(
        "SELECT coalesce(max(ordinal),0) FROM chat_messages WHERE conversation_id=?1",
        [&c.id],
        |r| r.get(0),
    )?;
    let assistant_ordinal = next.checked_add(2).ok_or(Error::IntegrityFailure)?;
    let id = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let assistant = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.execute("INSERT INTO chat_runs(id,conversation_id,user_message_id,assistant_message_id,status,request_json,model_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'queued',?5,?6,?7,?7)",params![id,c.id,user,assistant,encode(&request)?,model.as_ref().map(encode).transpose()?,now])?;
    db.execute(
        "INSERT INTO chat_messages VALUES(?1,?2,?3,'user',?4,?5,?6)",
        params![user, c.id, next + 1, request.text, now, id],
    )?;
    db.execute(
        "INSERT INTO chat_messages VALUES(?1,?2,?3,'assistant','',?4,?5)",
        params![assistant, c.id, assistant_ordinal, now, id],
    )?;
    let revision = c
        .row_version
        .checked_add(1)
        .ok_or(Error::IntegrityFailure)?;
    db.execute(
        "UPDATE chat_conversations SET row_version=?2,updated_at=?3 WHERE id=?1",
        params![c.id, revision, now],
    )?;
    event(db, &id, ChatRunStatus::Queued, None)?;
    if model.is_none() {
        finish(
            db,
            &id,
            ChatRunStatus::ModelUnconfigured,
            Some("model_unconfigured"),
            &ChatUsage::default(),
            None,
        )?;
    }
    Ok(id)
}

impl Library {
    pub fn create_chat_conversation(
        &mut self,
        command: &str,
        request: CreateConversation,
    ) -> Result<ChatConversationMutation> {
        let kind = "chat.conversations.create";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        if request.title.trim().is_empty()
            || request.title.len() > 1024
            || request.title.contains(char::is_control)
        {
            return Err(Error::InvalidInput("invalid_chat_title"));
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO chat_conversations VALUES(?1,?2,1,?3,?3)",
            params![id, request.title, now],
        )?;
        let result = ChatConversationMutation {
            conversation: conversation(&tx, &id)?,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn chat_conversation(&self, id: &str) -> Result<ChatConversation> {
        conversation(&self.connection, id)
    }
    pub fn chat_conversations(
        &self,
        after_id: Option<&str>,
        n: u32,
    ) -> Result<Vec<ChatConversation>> {
        limit(n)?;
        let ids=self.connection.prepare("SELECT id FROM chat_conversations WHERE NOT EXISTS(SELECT 1 FROM knowledge_tasks a JOIN chat_runs r ON r.id=a.id WHERE r.conversation_id=chat_conversations.id) AND (?1 IS NULL OR id>?1) ORDER BY id LIMIT ?2")?.query_map(params![after_id,n],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        ids.iter()
            .map(|id| conversation(&self.connection, id))
            .collect()
    }
    pub fn chat_messages(
        &self,
        conversation_id: &str,
        after_ordinal: u32,
        n: u32,
    ) -> Result<Vec<ChatMessage>> {
        limit(n)?;
        conversation(&self.connection, conversation_id)?;
        let rows=self.connection.prepare("SELECT m.id,m.ordinal,m.role,m.content,m.created_at,m.run_id,r.status,r.error_code FROM chat_messages m JOIN chat_runs r ON r.id=m.run_id WHERE m.conversation_id=?1 AND m.ordinal>?2 ORDER BY m.ordinal LIMIT ?3")?.query_map(params![conversation_id,after_ordinal,n],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,Option<String>>(7)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|r| {
                Ok(ChatMessage {
                    id: r.0,
                    conversation_id: conversation_id.into(),
                    ordinal: r.1,
                    role: value(encode(&r.2)?)?,
                    content: r.3,
                    created_at: r.4,
                    run_id: r.5,
                    run_status: value(encode(&r.6)?)?,
                    run_error_code: r.7,
                })
            })
            .collect()
    }
    pub fn chat_conversations_page(
        &self,
        before: Option<ChatConversationCursor>,
        n: u32,
    ) -> Result<ChatConversationPage> {
        limit(n)?;
        if let Some(cursor) = &before
            && (Uuid::parse_str(&cursor.id).is_err()
                || chrono::DateTime::parse_from_rfc3339(&cursor.updated_at).is_err())
        {
            return Err(Error::InvalidInput("invalid chat conversation cursor"));
        }
        let mut statement = self.connection.prepare(
            "SELECT id,title,row_version,created_at,updated_at FROM chat_conversations
             WHERE NOT EXISTS(SELECT 1 FROM knowledge_tasks a JOIN chat_runs r ON r.id=a.id WHERE r.conversation_id=chat_conversations.id)
             AND (?1 IS NULL OR (updated_at,id)<(?1,?2))
             ORDER BY updated_at DESC,id DESC LIMIT ?3",
        )?;
        let mut items = statement
            .query_map(
                params![
                    before.as_ref().map(|c| &c.updated_at),
                    before.as_ref().map(|c| &c.id),
                    n + 1
                ],
                |row| {
                    Ok(ChatConversation {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        row_version: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = items.len() > n as usize;
        items.truncate(n as usize);
        let next_cursor = if more {
            items.last().map(|c| ChatConversationCursor {
                updated_at: c.updated_at.clone(),
                id: c.id.clone(),
            })
        } else {
            None
        };
        Ok(ChatConversationPage { items, next_cursor })
    }

    pub fn chat_messages_page(
        &self,
        conversation_id: &str,
        before_ordinal: Option<u32>,
        n: u32,
    ) -> Result<ChatMessagePage> {
        limit(n)?;
        if before_ordinal == Some(0) {
            return Err(Error::InvalidInput("invalid chat message cursor"));
        }
        // One read snapshot pairs partial assistant content with its durable event cursor.
        let snapshot = self.connection.unchecked_transaction()?;
        let conversation = conversation(&snapshot, conversation_id)?;
        let mut statement = snapshot.prepare(
            "SELECT m.id,m.ordinal,m.role,m.content,m.created_at,m.run_id,r.status,r.error_code
             FROM chat_messages m JOIN chat_runs r ON r.id=m.run_id
             WHERE m.conversation_id=?1 AND m.ordinal<?2 ORDER BY m.ordinal DESC LIMIT ?3",
        )?;
        let mut items = statement
            .query_map(
                params![
                    conversation_id,
                    before_ordinal
                        .map(i64::from)
                        .unwrap_or(i64::from(u32::MAX) + 1),
                    n + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )?
            .map(|row| {
                let row = row?;
                Ok(ChatMessage {
                    id: row.0,
                    conversation_id: conversation_id.into(),
                    ordinal: row.1,
                    role: value(encode(&row.2)?)?,
                    content: row.3,
                    created_at: row.4,
                    run_id: row.5,
                    run_status: value(encode(&row.6)?)?,
                    run_error_code: row.7,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let more = items.len() > n as usize;
        items.truncate(n as usize);
        items.reverse();
        let next_before = if more {
            items.first().map(|m| m.ordinal)
        } else {
            None
        };
        let latest_run = if before_ordinal.is_none() {
            items
                .last()
                .map(|m| run(&snapshot, &m.run_id))
                .transpose()?
        } else {
            None
        };
        Ok(ChatMessagePage {
            conversation,
            items,
            next_before,
            latest_run,
        })
    }

    pub fn chat_run(&self, id: &str) -> Result<ChatRun> {
        run(&self.connection, id)
    }
    pub fn chat_run_events(&self, id: &str, after_seq: u32, n: u32) -> Result<Vec<ChatRunEvent>> {
        limit(n)?;
        run(&self.connection, id)?;
        let rows=self.connection.prepare("SELECT seq,kind,text_delta,status FROM chat_run_events WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?.query_map(params![id,after_seq,n],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|r| {
                Ok(ChatRunEvent {
                    seq: r.0,
                    kind: value(encode(&r.1)?)?,
                    text_delta: r.2,
                    status: value(encode(&r.3)?)?,
                })
            })
            .collect()
    }
    pub fn accept_chat_run_from_settings(
        &mut self,
        command: &str,
        request: StartChatRun,
    ) -> Result<ChatRunMutation> {
        let kind = "chat.runs.start";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let task_conversation: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM knowledge_tasks a JOIN chat_runs r ON r.id=a.id WHERE r.conversation_id=?1)",
            [&request.conversation_id], |r| r.get(0),
        )?;
        if task_conversation {
            return Err(Error::InvalidInput("knowledge_task_is_immutable"));
        }
        let id = queue_chat_run(&tx, request)?;
        let result = mutation(&tx, &id, command, &hash, kind)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn claim_chat_run(&mut self, id: &str) -> Result<Option<ChatRunLease>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let r = run(&tx, id)?;
        if r.status != ChatRunStatus::Queued {
            return Ok(None);
        }
        let token = Uuid::new_v4().to_string();
        tx.execute(
            "UPDATE chat_runs SET status='running',lease_token=?2,lease_expires_at=?3 WHERE id=?1",
            params![id, token, (Utc::now() + Duration::minutes(5)).to_rfc3339()],
        )?;
        event(&tx, id, ChatRunStatus::Running, None)?;
        let result = ChatRunLease {
            run: run(&tx, id)?,
            lease_token: token,
        };
        tx.commit()?;
        Ok(Some(result))
    }
    pub fn check_chat_run_lease(&self, id: &str, token: &str) -> Result<()> {
        lease(&self.connection, id, token)
    }
    pub fn heartbeat_chat_run(&mut self, id: &str, token: &str) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        tx.execute(
            "UPDATE chat_runs SET lease_expires_at=?2 WHERE id=?1",
            params![id, (Utc::now() + Duration::minutes(5)).to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn validate_chat_run_evidence(
        &self,
        id: &str,
        token: &str,
        evidence: &[SearchEvidence],
    ) -> Result<()> {
        lease(&self.connection, id, token)?;
        validate_materials(
            &self.connection,
            &run(&self.connection, id)?.request.scope,
            evidence,
        )
    }
    pub fn append_chat_delta(
        &mut self,
        id: &str,
        token: &str,
        seq: u32,
        delta: &str,
    ) -> Result<ChatRun> {
        if delta.is_empty() || delta.len() > 65536 || delta.contains('\0') {
            return Err(Error::InvalidInput("invalid_chat_delta"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        let current = run(&tx, id)?;
        if seq <= current.last_event_seq {
            let prior: Option<String> = tx
                .query_row(
                    "SELECT text_delta FROM chat_run_events WHERE run_id=?1 AND seq=?2",
                    params![id, seq],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if prior.as_deref() != Some(delta) {
                return Err(Error::IdempotencyMismatch);
            }
            return Ok(current);
        }
        if seq
            != current
                .last_event_seq
                .checked_add(1)
                .ok_or(Error::IntegrityFailure)?
        {
            return Err(Error::InvalidInput("chat_delta_sequence"));
        }
        let size: u32 = tx.query_row(
            "SELECT length(CAST(content AS BLOB)) FROM chat_messages WHERE id=?1",
            [&current.assistant_message_id],
            |r| r.get(0),
        )?;
        if (size as usize)
            .checked_add(delta.len())
            .is_none_or(|n| n > MAX_CONTENT)
        {
            return Err(Error::InvalidInput("chat_output_limit"));
        }
        tx.execute(
            "UPDATE chat_messages SET content=content||?2 WHERE id=?1",
            params![current.assistant_message_id, delta],
        )?;
        event(&tx, id, ChatRunStatus::Running, Some(delta))?;
        let result = run(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn finish_chat_run(
        &mut self,
        id: &str,
        token: &str,
        status: ChatRunStatus,
        code: Option<&str>,
        usage: ChatUsage,
        finish_reason: Option<&str>,
    ) -> Result<ChatRun> {
        if !status.is_terminal()
            || status == ChatRunStatus::ModelUnconfigured
            || code.is_some_and(|s| {
                s.len() > 128
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
            || finish_reason
                .is_some_and(|s| !matches!(s, "stop" | "tool_calls" | "length" | "refusal"))
        {
            return Err(Error::InvalidInput("invalid_chat_finish"));
        }
        if status == ChatRunStatus::Completed && (code.is_some() || finish_reason != Some("stop")) {
            return Err(Error::InvalidInput("invalid_chat_completion"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        if status == ChatRunStatus::Completed {
            let current = run(&tx, id)?;
            let text: String = tx.query_row(
                "SELECT content FROM chat_messages WHERE id=?1",
                [current.assistant_message_id],
                |r| r.get(0),
            )?;
            if text.is_empty() {
                return Err(Error::InvalidInput("chat_empty_completion"));
            }
        }
        finish(&tx, id, status, code, &usage, finish_reason)?;
        let result = run(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn cancel_chat_run(&mut self, command: &str, id: &str) -> Result<ChatRunMutation> {
        let kind = "chat.runs.cancel";
        let hash = mutations::request_hash(command, kind, &id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let r = run(&tx, id)?;
        if !r.status.is_terminal() {
            finish(
                &tx,
                id,
                ChatRunStatus::Canceled,
                None,
                &r.usage,
                r.finish_reason.as_deref(),
            )?;
        }
        let result = mutation(&tx, id, command, &hash, kind)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn pending_chat_run_ids(&self, n: u32) -> Result<Vec<String>> {
        limit(n)?;
        Ok(self
            .connection
            .prepare(
                "SELECT id FROM chat_runs WHERE status='queued' ORDER BY created_at,id LIMIT ?1",
            )?
            .query_map([n], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn recover_chat_runs_exclusive(&mut self) -> Result<u32> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = tx
            .prepare("SELECT id FROM chat_runs WHERE status='running'")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in &ids {
            let r = run(&tx, id)?;
            finish(
                &tx,
                id,
                ChatRunStatus::Interrupted,
                Some("worker_interrupted"),
                &r.usage,
                r.finish_reason.as_deref(),
            )?;
        }
        tx.commit()?;
        Ok(ids.len() as u32)
    }
}

/// Restoring a copy never grants permission to replay requests from the original instance.
/// Call inside the restore transaction so terminal events and lease removal commit together.
pub(crate) fn interrupt_restored_runs(db: &Connection) -> Result<()> {
    let ids = db
        .prepare("SELECT id FROM chat_runs WHERE status IN ('queued','running')")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let previous = run(db, &id)?;
        finish(
            db,
            &id,
            ChatRunStatus::Interrupted,
            Some("chat_restored_copy"),
            &previous.usage,
            previous.finish_reason.as_deref(),
        )?;
    }
    Ok(())
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let database_schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if database_schema < 16
        && db
            .prepare("SELECT 1 FROM chat_runs WHERE json_type(model_json,'$.channel_id') IS NOT NULL OR json_type(model_json,'$.capabilities') IS NOT NULL")?
            .exists([])?
    {
        return Err(Error::IntegrityFailure);
    }
    let ids = db
        .prepare("SELECT id FROM chat_conversations")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let c = conversation(db, &id)?;
        document::valid_uuid(&id)?;
        if c.row_version == 0
            || c.title.trim().is_empty()
            || c.title.len() > 1024
            || c.title.contains(char::is_control)
        {
            return Err(Error::IntegrityFailure);
        }
        for t in [&c.created_at, &c.updated_at] {
            chrono::DateTime::parse_from_rfc3339(t).map_err(|_| Error::IntegrityFailure)?;
        }
    }
    let ids = db
        .prepare("SELECT id FROM chat_runs")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let r = run(db, &id)?;
        document::valid_uuid(&id)?;
        document::valid_uuid(&r.conversation_id)?;
        if r.request.conversation_id != r.conversation_id
            || r.request.base_row_version == 0
            || r.request.text.trim().is_empty()
            || r.request.text.len() > 256 * 1024
            || r.request.text.contains('\0')
            || r.request.materials.len() > 32
        {
            return Err(Error::IntegrityFailure);
        }
        validate_scope(&r.request.scope)?;
        validate_chat_run_options(
            &r.request.options.clone().unwrap_or_default(),
            r.model.as_ref(),
        )?;
        if let Some(p) = &r.model {
            assistant::validate_generation_profile_for_backup(db, p, database_schema)?;
            if p.provider == "ollama" {
                return Err(Error::IntegrityFailure);
            }
        }
        for t in [&r.created_at, &r.updated_at] {
            chrono::DateTime::parse_from_rfc3339(t).map_err(|_| Error::IntegrityFailure)?;
        }
        if r.error_code.as_ref().is_some_and(|s| {
            s.len() > 128
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) || r
            .finish_reason
            .as_deref()
            .is_some_and(|s| !matches!(s, "stop" | "tool_calls" | "length" | "refusal"))
        {
            return Err(Error::IntegrityFailure);
        }
        if (r.status == ChatRunStatus::ModelUnconfigured) != (r.model.is_none()) {
            return Err(Error::IntegrityFailure);
        }
        let messages=db.prepare("SELECT id,conversation_id,ordinal,role,content FROM chat_messages WHERE run_id=?1 ORDER BY ordinal")?.query_map([&id],|q|Ok((q.get::<_,String>(0)?,q.get::<_,String>(1)?,q.get::<_,u32>(2)?,q.get::<_,String>(3)?,q.get::<_,String>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if messages.len() != 2 {
            return Err(Error::IntegrityFailure);
        }
        let u = &messages[0];
        let a = &messages[1];
        if u.0 != r.user_message_id
            || a.0 != r.assistant_message_id
            || u.1 != r.conversation_id
            || a.1 != r.conversation_id
            || u.3 != "user"
            || a.3 != "assistant"
            || u.4 != r.request.text
            || u.2.checked_add(1) != Some(a.2)
            || a.4.len() > MAX_CONTENT
            || a.4.contains('\0')
        {
            return Err(Error::IntegrityFailure);
        }
        document::valid_uuid(&u.0)?;
        document::valid_uuid(&a.0)?;
        let events=db.prepare("SELECT seq,kind,text_delta,status FROM chat_run_events WHERE run_id=?1 ORDER BY seq")?.query_map([&id],|q|Ok((q.get::<_,u32>(0)?,q.get::<_,String>(1)?,q.get::<_,Option<String>>(2)?,q.get::<_,String>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut text = String::new();
        let mut prior = None;
        for (index, e) in events.iter().enumerate() {
            let status: ChatRunStatus = value(encode(&e.3)?)?;
            if e.0 as usize != index + 1 || prior.is_some_and(ChatRunStatus::is_terminal) {
                return Err(Error::IntegrityFailure);
            }
            if index == 0 && (status != ChatRunStatus::Queued || e.1 != "status") {
                return Err(Error::IntegrityFailure);
            }
            match (e.1.as_str(), e.2.as_deref()) {
                ("text_delta", Some(delta))
                    if status == ChatRunStatus::Running
                        && prior == Some(ChatRunStatus::Running)
                        && !delta.is_empty()
                        && delta.len() <= 65536
                        && !delta.contains('\0') =>
                {
                    if text.len() + delta.len() > MAX_CONTENT {
                        return Err(Error::IntegrityFailure);
                    }
                    text.push_str(delta);
                }
                ("status", None) => {
                    if index > 0
                        && !(prior == Some(ChatRunStatus::Queued)
                            && (status == ChatRunStatus::Running
                                || matches!(
                                    status,
                                    ChatRunStatus::Canceled | ChatRunStatus::ModelUnconfigured
                                )
                                || status == ChatRunStatus::Interrupted
                                    && r.error_code.as_deref() == Some("chat_restored_copy"))
                            || prior == Some(ChatRunStatus::Running) && status.is_terminal())
                    {
                        return Err(Error::IntegrityFailure);
                    }
                }
                _ => return Err(Error::IntegrityFailure),
            }
            prior = Some(status);
        }
        if events.len() != r.last_event_seq as usize || prior != Some(r.status) || text != a.4 {
            return Err(Error::IntegrityFailure);
        }
        if r.status == ChatRunStatus::Completed
            && (a.4.is_empty()
                || r.error_code.is_some()
                || r.finish_reason.as_deref() != Some("stop"))
        {
            return Err(Error::IntegrityFailure);
        }
        let (token, expires): (Option<String>, Option<String>) = db.query_row(
            "SELECT lease_token,lease_expires_at FROM chat_runs WHERE id=?1",
            [&id],
            |q| Ok((q.get(0)?, q.get(1)?)),
        )?;
        if r.status == ChatRunStatus::Running {
            document::valid_uuid(token.as_deref().ok_or(Error::IntegrityFailure)?)?;
            chrono::DateTime::parse_from_rfc3339(
                expires.as_deref().ok_or(Error::IntegrityFailure)?,
            )
            .map_err(|_| Error::IntegrityFailure)?;
        } else if token.is_some() || expires.is_some() {
            return Err(Error::IntegrityFailure);
        }
        let mut total = 0usize;
        for evidence in &r.request.materials {
            total = total
                .checked_add(evidence.text.len())
                .ok_or(Error::IntegrityFailure)?;
            if total > 256 * 1024 {
                return Err(Error::IntegrityFailure);
            }
            if evidence.annotation.is_some() {
                if !crate::annotation_retrieval::validate_annotation_evidence(
                    db,
                    &r.request.scope,
                    evidence,
                    true,
                )? {
                    return Err(Error::IntegrityFailure);
                }
                continue;
            }
            let t = &evidence.target;
            document::valid_uuid(&t.object_id)?;
            document::valid_uuid(&t.content_revision)?;
            if !r.request.scope.object_ids.is_empty()
                && !r.request.scope.object_ids.contains(&t.object_id)
                || !r.request.scope.kinds.is_empty() && !r.request.scope.kinds.contains(&t.kind)
                || t.start_codepoint >= t.end_codepoint
            {
                return Err(Error::IntegrityFailure);
            }
            let body = if t.kind == EvidenceKind::Note {
                let note = crate::notes::revision(db, &t.object_id, &t.content_revision)?;
                note.document
                    .nodes()
                    .into_iter()
                    .find(|n| n.attrs.id == t.block_id)
                    .ok_or(Error::IntegrityFailure)?
                    .plain_text()
            } else {
                db.query_row("SELECT b.text FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id JOIN source_versions v ON v.id=p.source_version_id WHERE b.id=?1 AND p.id=?2 AND v.id=?3 AND v.source_id=?4",params![t.block_id,t.parse_id,t.content_revision,t.object_id],|q|q.get::<_,String>(0))?
            };
            if t.end_codepoint as usize > body.chars().count()
                || body
                    .chars()
                    .skip(t.start_codepoint as usize)
                    .take((t.end_codepoint - t.start_codepoint) as usize)
                    .collect::<String>()
                    != evidence.text
            {
                return Err(Error::IntegrityFailure);
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_thirteen_upgrades_without_rewriting_existing_answer_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        let mut raw = Connection::open(&path).unwrap();
        crate::migrations::initialize_at(&mut raw, 13).unwrap();
        let history: String = raw
            .query_row(
                "SELECT checksum FROM migration_history WHERE version=13",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(raw);
        let _db = Library::open_existing(dir.path()).unwrap();
        let raw = Connection::open(&path).unwrap();
        assert_eq!(
            raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::migrations::CURRENT_SCHEMA_VERSION
        );
        assert_eq!(
            raw.query_row(
                "SELECT checksum FROM migration_history WHERE version=13",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            history
        );
    }
}
