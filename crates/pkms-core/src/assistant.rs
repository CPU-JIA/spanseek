use crate::*;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;
use uuid::Uuid;

const MAX_TEXT: usize = 256 * 1024;
fn validate_agent_evidence(
    db: &Connection,
    id: &str,
    token: &str,
    evidence: &[SearchEvidence],
) -> Result<()> {
    lease(db, id, token)?;
    let answer = read(db, id)?;
    let agent = answer
        .request
        .agent
        .as_ref()
        .ok_or(Error::InvalidInput("agent_request_required"))?;
    validate_agent_options(agent)?;
    if evidence.len() > agent.max_evidence as usize
        || evidence
            .iter()
            .try_fold(0usize, |total, item| total.checked_add(item.text.len()))
            .is_none_or(|total| total > MAX_TEXT)
    {
        return Err(Error::InvalidInput("agent_evidence_limit"));
    }
    let mut identities = BTreeSet::new();
    for item in evidence {
        let identity = serde_json::to_string(&(&item.target, &item.annotation))?;
        if !identities.insert(identity)
            || item.text.contains('\0')
            || item
                .cosine_similarity
                .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
            || item.fusion_rank.is_some_and(|v| !v.is_finite() || v < 0.0)
            || !retrieval::validate_evidence(db, &answer.request.search.scope, item)?
        {
            return Err(Error::InvalidInput("agent_evidence_invalid"));
        }
    }
    Ok(())
}
pub(crate) fn generation_settings(db: &Connection) -> Result<GenerationSettings> {
    let (json,row_version):(Option<String>,u32)=db.query_row("SELECT generation_profile_json,generation_row_version FROM search_settings WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    Ok(GenerationSettings {
        profile: json.map(|s| serde_json::from_str(&s)).transpose()?,
        row_version,
    })
}
pub(crate) fn validate_generation_settings_backup(db: &Connection) -> Result<()> {
    let schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema < 16
        && db
            .prepare("SELECT 1 FROM search_settings WHERE json_type(generation_profile_json,'$.channel_id') IS NOT NULL OR json_type(generation_profile_json,'$.capabilities') IS NOT NULL")?
            .exists([])?
    {
        return Err(Error::IntegrityFailure);
    }
    let settings = generation_settings(db)?;
    if let Some(p) = &settings.profile {
        validate_generation_profile_for_backup(db, p, schema)?;
        if p.config_revision != settings.row_version {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}
fn citations_valid(text: &str, ids: &[String], evidence: &[AnswerEvidence]) -> bool {
    let allowed = evidence
        .iter()
        .map(|e| e.citation_id.as_str())
        .collect::<BTreeSet<_>>();
    let cited = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut markers = BTreeSet::new();
    for tail in text.split("[E").skip(1) {
        let Some((n, _)) = tail.split_once(']') else {
            return false;
        };
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        markers.insert(format!("E{n}"));
    }
    !text.trim().is_empty()
        && !cited.is_empty()
        && cited.len() == ids.len()
        && cited.is_subset(&allowed)
        && markers.iter().map(String::as_str).collect::<BTreeSet<_>>() == cited
}

fn status_text(status: AnswerStatus) -> Result<String> {
    Ok(serde_json::to_value(status)?
        .as_str()
        .ok_or(Error::IntegrityFailure)?
        .into())
}
fn event(db: &Connection, id: &str, status: AnswerStatus, delta: Option<&str>) -> Result<()> {
    db.execute("INSERT INTO answer_events(answer_id,seq,status,text_delta) SELECT ?1,COALESCE(MAX(seq),0)+1,?2,?3 FROM answer_events WHERE answer_id=?1", params![id,status_text(status)?,delta])?;
    Ok(())
}
fn current_evidence(db: &Connection, a: &AnswerRecord) -> Result<bool> {
    for item in &a.evidence {
        if !crate::retrieval::validate_evidence(db, &a.request.search.scope, &item.evidence)? {
            return Ok(false);
        }
    }
    Ok(!a.evidence.is_empty())
}
fn read(db: &Connection, id: &str) -> Result<AnswerRecord> {
    crate::document::valid_uuid(id)?;
    let row = db.query_row("SELECT request_json,status,text,evidence_json,cited_ids_json,error_code,created_at,updated_at,(SELECT COALESCE(MAX(seq),0) FROM answer_events WHERE answer_id=answers.id) FROM answers WHERE id=?1",[id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,u32>(8)?))).optional()?.ok_or(Error::NotFound)?;
    let mut a = AnswerRecord {
        id: id.into(),
        request: serde_json::from_str(&row.0)?,
        status: serde_json::from_value(serde_json::Value::String(row.1))?,
        text: row.2,
        evidence: serde_json::from_str(&row.3)?,
        cited_ids: serde_json::from_str(&row.4)?,
        error_code: row.5,
        created_at: row.6,
        updated_at: row.7,
        last_event_seq: row.8,
        evidence_current: false,
        feedback: db
            .query_row(
                "SELECT rating,comment FROM answer_feedback WHERE answer_id=?1",
                [id],
                |r| {
                    Ok(AnswerFeedback {
                        answer_id: id.into(),
                        rating: r.get(0)?,
                        comment: r.get(1)?,
                        updated_at: None,
                    })
                },
            )
            .optional()?
            .map(|mut feedback| {
                // Every rating change is recorded immutably as a command receipt,
                // so the change time comes from there instead of a second column.
                feedback.updated_at = db
                    .query_row(
                        "SELECT committed_at FROM command_receipts WHERE command_kind='answers.feedback' AND object_id=?1 ORDER BY change_seq DESC LIMIT 1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten();
                feedback
            }),
    };
    a.evidence_current = current_evidence(db, &a)?;
    Ok(a)
}
fn lease(db: &Connection, id: &str, token: &str) -> Result<()> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM answers WHERE id=?1 AND status='running' AND lease_token=?2 AND lease_expires_at>?3)",params![id,token,Utc::now().to_rfc3339()],|r|r.get(0))?;
    if !valid {
        return Err(Error::InvalidInput("answer lease expired or inactive"));
    }
    Ok(())
}
fn finish(db: &Connection, id: &str, status: AnswerStatus, code: Option<&str>) -> Result<()> {
    db.execute("UPDATE answers SET status=?2,error_code=?3,lease_token=NULL,lease_expires_at=NULL,updated_at=?4 WHERE id=?1",params![id,status_text(status)?,code,Utc::now().to_rfc3339()])?;
    event(db, id, status, None)
}
fn answer_receipt(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<AnswerMutation> {
    let result = AnswerMutation {
        answer: read(db, id)?,
        receipt: crate::mutations::next_receipt(db, command)?,
    };
    crate::mutations::store_receipt(db, hash, kind, id, &result.receipt, &result)?;
    Ok(result)
}
/// The feedback receipt is built before it is stored, so the answer it returns
/// already carries the change time that the receipt records.
fn feedback_receipt(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<AnswerMutation> {
    let receipt = crate::mutations::next_receipt(db, command)?;
    let mut answer = read(db, id)?;
    if let Some(feedback) = answer.feedback.as_mut() {
        feedback.updated_at = Some(receipt.committed_at.clone());
    }
    let result = AnswerMutation { answer, receipt };
    crate::mutations::store_receipt(db, hash, kind, id, &result.receipt, &result)?;
    Ok(result)
}
pub(crate) fn validate_remote_generation_endpoint(value: &str) -> Result<()> {
    let endpoint = url::Url::parse(value)
        .map_err(|_| Error::InvalidInput("invalid remote generation endpoint"))?;
    // URL parsing normalizes empty userinfo and missing authority slashes.
    // Require the configured endpoint itself to have an unambiguous authority.
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default());
    let loopback_http = endpoint.scheme() == "http"
        && endpoint
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
    if value.len() > 2048
        || authority.is_none_or(|value| value.is_empty() || value.contains('@'))
        || value.contains(|c: char| c.is_whitespace() || c.is_control() || c == '\\')
        || (endpoint.scheme() != "https" && !loopback_http)
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.port() == Some(0)
    {
        return Err(Error::InvalidInput("invalid remote generation endpoint"));
    }
    Ok(())
}

pub(crate) fn validate_generation_profile_reference(
    db: &Connection,
    profile: &GenerationProfile,
) -> Result<()> {
    if let Some(id) = &profile.channel_id
        && !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM generation_channels WHERE id=?1)",
            [id],
            |row| row.get::<_, bool>(0),
        )?
    {
        return Err(Error::InvalidInput("generation_channel_missing"));
    }
    Ok(())
}

pub(crate) fn validate_generation_profile_for_backup(
    db: &Connection,
    profile: &GenerationProfile,
    schema: u32,
) -> Result<()> {
    validate_generation_profile(profile).map_err(|_| Error::IntegrityFailure)?;
    if schema < 16
        && (profile.channel_id.is_some()
            || profile.capabilities.is_some()
            || profile.provider == "gemini"
            || profile.context_tokens > 131072)
    {
        return Err(Error::IntegrityFailure);
    }
    // Historical requests only need a stable reference. Channel edits and deletion
    // must never rewrite their endpoint, model, parameters or capabilities.
    validate_generation_profile_reference(db, profile).map_err(|_| Error::IntegrityFailure)
}

pub fn validate_generation_profile(p: &GenerationProfile) -> Result<()> {
    if p.channel_id.is_some() != p.capabilities.is_some()
        || (p.provider == "ollama" && p.channel_id.is_some())
    {
        return Err(Error::InvalidInput("invalid_generation_channel_profile"));
    }
    if let Some(id) = &p.channel_id {
        document::valid_uuid(id)?;
    }
    if p.provider != "openai_chat" && p.chat_token_limit.is_some() {
        return Err(Error::InvalidInput(
            "chat token limit requires OpenAI Chat provider",
        ));
    }
    if matches!(
        p.provider.as_str(),
        "openai_chat" | "openai_responses" | "anthropic" | "gemini"
    ) {
        validate_remote_generation_endpoint(&p.endpoint)?;
        if p.model.trim().is_empty()
            || p.model.len() > 256
            || p.model.contains(char::is_control)
            || !p.model_digest.is_empty()
            || p.config_revision == 0
            || !p.temperature.is_finite()
            || !(0.0..=2.0).contains(&p.temperature)
            || !(256..=2_097_152).contains(&p.context_tokens)
        {
            return Err(Error::InvalidInput("invalid remote generation profile"));
        }
        return Ok(());
    }
    let authority = p
        .endpoint
        .strip_prefix("http://")
        .map(|s| s.strip_suffix('/').unwrap_or(s))
        .ok_or(Error::InvalidInput(
            "generation endpoint must be loopback HTTP",
        ))?;
    let host = authority.split(':').next().unwrap_or_default();
    if p.provider != "ollama"
        || !matches!(host, "127.0.0.1" | "localhost")
        || authority.contains(['/', '?', '#', '@'])
        || authority
            .split(':')
            .nth(1)
            .is_some_and(|v| v.parse::<u16>().is_err())
        || authority.matches(':').count() > 1
        || p.model.trim().is_empty()
        || p.model.len() > 256
        || p.model_digest.trim().is_empty()
        || p.model_digest.len() > 256
        || p.model.contains(char::is_control)
        || p.model_digest.contains(char::is_control)
        || p.config_revision == 0
        || !p.temperature.is_finite()
        || !(0.0..=2.0).contains(&p.temperature)
        || !(256..=131072).contains(&p.context_tokens)
    {
        return Err(Error::InvalidInput("invalid generation profile"));
    }
    Ok(())
}

/// Evidence answers and agents require structured output before any planning or
/// remote work begins. Legacy profiles retain their historical capabilities.
pub fn validate_structured_generation_profile(profile: &GenerationProfile) -> Result<()> {
    validate_generation_profile(profile)?;
    if profile
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.structured_transport.is_none())
    {
        return Err(Error::InvalidInput("model_unsupported_output"));
    }
    Ok(())
}

/// Bounds shared by every answer acceptance path. Top-K is capped at the
/// documented maximum; the similarity floor stays optional.
fn validate_answer_search(search: &SearchQuery) -> Result<()> {
    if search.query.trim().is_empty()
        || search.query.len() > 4096
        || search.query.contains('\0')
        || !(1..=MAX_TOP_K).contains(&search.limit)
        || search
            .minimum_cosine
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
    {
        return Err(Error::InvalidInput("invalid answer search"));
    }
    Ok(())
}

fn validate_citation_count(count: Option<u32>) -> Result<()> {
    if count.is_some_and(|n| !(1..=MAX_CITATION_COUNT).contains(&n)) {
        return Err(Error::InvalidInput("invalid_answer_citation_count"));
    }
    Ok(())
}

/// Frozen citation cap. Missing values keep the historical behavior of citing
/// every candidate, still bounded by the evidence ceiling.
fn citation_cap(count: Option<u32>) -> usize {
    count
        .map(|n| (n as usize).min(LEGACY_EVIDENCE_LIMIT))
        .unwrap_or(LEGACY_EVIDENCE_LIMIT)
}
/// Per-citation ceiling for the frozen parent context handed to the model.
const MAX_CONTEXT_CHARS: usize = 4000;
/// Total ceiling across all frozen citations, applied in citation order.
const MAX_CONTEXT_TOTAL_CHARS: usize = 16_000;

/// Small-to-big expansion. A hit may be a narrow chunk or an excerpt of a much
/// larger indexed block; freeze the surrounding parent text so the model reads
/// the full context while the citation still resolves to the precise hit range.
fn freeze_evidence(db: &Connection, items: Vec<SearchEvidence>) -> Vec<AnswerEvidence> {
    let mut budget = MAX_CONTEXT_TOTAL_CHARS;
    items
        .into_iter()
        .enumerate()
        .map(|(index, evidence)| {
            let context = parent_context(db, &evidence, budget);
            if let Some(text) = &context {
                budget = budget.saturating_sub(text.chars().count());
            }
            AnswerEvidence {
                citation_id: format!("E{}", index + 1),
                evidence,
                context,
            }
        })
        .collect()
}

fn parent_context(db: &Connection, evidence: &SearchEvidence, budget: usize) -> Option<String> {
    // Annotation hits carry their own body; only indexed blocks expand.
    if evidence.annotation.is_some() || budget == 0 {
        return None;
    }
    let input = crate::retrieval::read_index_input(db, &evidence.target.object_id).ok()?;
    let block = input
        .blocks
        .into_iter()
        .find(|b| b.target.block_id == evidence.target.block_id)?;
    // A hit that already is the whole block needs no expansion.
    if block.text == evidence.text {
        return None;
    }
    let limit = budget.min(MAX_CONTEXT_CHARS);
    let chars: Vec<char> = block.text.chars().collect();
    let text: String = if chars.len() <= limit {
        block.text
    } else {
        // Keep a window centred on the hit so the parent stays bounded.
        let start = (evidence.target.start_codepoint as usize).min(chars.len());
        let end = (evidence.target.end_codepoint as usize).min(chars.len());
        let span = end.saturating_sub(start).min(limit);
        let from = start
            .saturating_sub((limit - span) / 2)
            .min(chars.len() - limit);
        chars[from..from + limit].iter().collect()
    };
    (text != evidence.text).then_some(text)
}

impl Library {
    /// Agent acceptance has its own command kind; legacy search-only hashes stay unchanged.
    pub fn accept_agent_answer_from_settings(
        &mut self,
        command: &str,
        search: SearchQuery,
        agent: AgentOptions,
    ) -> Result<AnswerMutation> {
        self.accept_agent_answer_from_settings_with_budget(command, search, agent, None)
    }

    pub fn accept_agent_answer_from_settings_with_budget(
        &mut self,
        command: &str,
        search: SearchQuery,
        agent: AgentOptions,
        max_output_tokens: Option<u32>,
    ) -> Result<AnswerMutation> {
        let kind = "answers.agent_start_from_settings";
        let hash = if let Some(tokens) = max_output_tokens {
            mutations::request_hash(command, kind, &(&search, &agent, tokens))?
        } else {
            mutations::request_hash(command, kind, &(&search, &agent))?
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(result);
        }
        validate_agent_options(&agent)?;
        validate_answer_search(&search)?;
        retrieval::eligible_ids(&tx, &search.scope)?;
        let model = generation_settings(&tx)?.profile;
        if let Some(p) = &model {
            validate_structured_generation_profile(p)?;
        }
        if model.as_ref().is_some_and(|p| p.provider == "ollama") {
            return Err(Error::InvalidInput("agent_remote_model_required"));
        }
        validate_answer_output_tokens(max_output_tokens, model.as_ref())?;
        let request = StartAnswer {
            max_output_tokens,
            citation_count: None,
            search,
            model,
            agent: Some(agent),
        };
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO answers(id,request_json,status,created_at,updated_at) VALUES(?1,?2,'queued',?3,?3)",params![id,serde_json::to_string(&request)?,now])?;
        event(&tx, &id, AnswerStatus::Queued, None)?;
        let result = answer_receipt(&tx, command, &hash, kind, &id)?;
        tx.commit()?;
        Ok(result)
    }

    /// Recheck the original authorization and exact contents before exposing a tool result.
    pub fn validate_agent_answer_evidence(
        &self,
        id: &str,
        token: &str,
        evidence: &[SearchEvidence],
    ) -> Result<()> {
        validate_agent_evidence(&self.connection, id, token, evidence)
    }

    /// Query-independent preflight; callers with deadlines use the checked variant.
    pub fn agent_scope_has_evidence(&self, id: &str, token: &str) -> Result<bool> {
        self.agent_scope_has_evidence_with_check(id, token, &mut || Ok(()))
    }

    pub fn agent_scope_has_evidence_with_check(
        &self,
        id: &str,
        token: &str,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<bool> {
        check()?;
        validate_agent_evidence(&self.connection, id, token, &[])?;
        let scope = read(&self.connection, id)?.request.search.scope;
        let mut after = String::new();
        loop {
            check()?;
            lease(&self.connection, id, token)?;
            // Keyset pages bound allocation; no transaction pins old cancellation state.
            let page = self.connection.prepare("SELECT id,kind FROM objects WHERE id>?1 AND kind IN ('note','source','annotation') ORDER BY id LIMIT 16")?
                .query_map([&after], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
                .collect::<std::result::Result<Vec<_>,_>>()?;
            if page.is_empty() {
                check()?;
                return Ok(false);
            }
            for (object, kind) in page {
                check()?;
                lease(&self.connection, id, token)?;
                after = object.clone();
                if !scope.object_ids.is_empty() && !scope.object_ids.contains(&object) {
                    continue;
                }
                let mut narrowed = scope.clone();
                narrowed.object_ids = vec![object.clone()];
                if kind == "annotation" {
                    // An unscoped corpus does not grant access to historical annotation slices.
                    if scope.collection_id.is_none() && scope.object_ids.is_empty() {
                        continue;
                    }
                    let present = self
                        .annotation_search_inputs(&narrowed)?
                        .iter()
                        .any(|item| !item.text.trim().is_empty());
                    check()?;
                    if present {
                        return Ok(true);
                    }
                } else if !retrieval::eligible_ids(&self.connection, &narrowed)?.is_empty() {
                    let sql = if kind == "note" {
                        "SELECT j.value FROM notes n JOIN note_revisions v ON v.revision_id=n.current_revision_id JOIN json_tree(v.document_json) j WHERE n.id=?1 AND j.key='text' AND j.type='text'"
                    } else {
                        "SELECT b.text FROM sources s JOIN source_active_parses a ON a.source_version_id=s.current_version_id JOIN source_blocks b ON b.parse_id=a.parse_id WHERE s.id=?1"
                    };
                    // Stream text leaves/blocks rather than constructing a whole document input.
                    let mut statement = self.connection.prepare(sql)?;
                    let mut rows = statement.query([&object])?;
                    loop {
                        check()?;
                        let Some(row) = rows.next()? else {
                            break;
                        };
                        let text = row
                            .get_ref(0)?
                            .as_str()
                            .map_err(|_| Error::IntegrityFailure)?;
                        let present = !text.trim().is_empty();
                        check()?;
                        if present {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    /// Freeze the union of actual multi-query results without re-running the initial query.
    pub fn prepare_agent_answer_evidence(
        &mut self,
        id: &str,
        token: &str,
        evidence: Vec<SearchEvidence>,
    ) -> Result<AnswerRecord> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_agent_evidence(&tx, id, token, &evidence)?;
        let prepared: bool = tx.query_row(
            "SELECT evidence_prepared FROM answers WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        if prepared {
            return Err(Error::InvalidInput("answer evidence already frozen"));
        }
        let evidence = freeze_evidence(&tx, evidence);
        tx.execute(
            "UPDATE answers SET evidence_json=?2,evidence_prepared=1,updated_at=?3 WHERE id=?1",
            params![
                id,
                serde_json::to_string(&evidence)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        if read(&tx, id)?.request.model.is_none() {
            finish(&tx, id, AnswerStatus::ModelUnconfigured, None)?;
        } else if evidence.is_empty() {
            finish(&tx, id, AnswerStatus::NoEvidence, None)?;
        }
        let result = read(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }

    /// Freeze the configured model only for the first acceptance of this command.
    pub fn accept_answer_from_settings(
        &mut self,
        command: &str,
        search: SearchQuery,
    ) -> Result<AnswerMutation> {
        self.accept_answer_from_settings_with_budget(command, search, None)
    }

    pub fn accept_answer_from_settings_with_budget(
        &mut self,
        command: &str,
        search: SearchQuery,
        max_output_tokens: Option<u32>,
    ) -> Result<AnswerMutation> {
        let kind = "answers.start_from_settings";
        let hash = if let Some(tokens) = max_output_tokens {
            mutations::request_hash(command, kind, &(&search, tokens))?
        } else {
            mutations::request_hash(command, kind, &search)?
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(result);
        }
        validate_answer_search(&search)?;
        retrieval::eligible_ids(&tx, &search.scope)?;
        let model = generation_settings(&tx)?.profile;
        if let Some(p) = &model {
            validate_structured_generation_profile(p)?;
        }
        validate_answer_output_tokens(max_output_tokens, model.as_ref())?;
        // Freeze the configured citation cap so a running task keeps its snapshot.
        let citation_count = crate::retrieval_settings::retrieval_settings(&tx)?
            .params
            .citation_count;
        let request = StartAnswer {
            max_output_tokens,
            citation_count: Some(citation_count),
            search,
            model,
            agent: None,
        };
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO answers(id,request_json,status,created_at,updated_at) VALUES(?1,?2,'queued',?3,?3)",params![id,serde_json::to_string(&request)?,now])?;
        event(&tx, &id, AnswerStatus::Queued, None)?;
        let result = answer_receipt(&tx, command, &hash, kind, &id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn check_answer_lease(&self, id: &str, token: &str) -> Result<()> {
        lease(&self.connection, id, token)
    }
    pub fn heartbeat_answer(&mut self, id: &str, token: &str) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        tx.execute(
            "UPDATE answers SET lease_expires_at=?2 WHERE id=?1",
            params![id, (Utc::now() + Duration::minutes(5)).to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Caller must hold this workspace's exclusive WorkspaceLease and stop all old workers.
    /// Queued requests remain queued; an interrupted generation is never automatically retried.
    pub fn recover_answers_exclusive(&mut self) -> Result<u32> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = tx
            .prepare("SELECT id FROM answers WHERE status='running'")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in &ids {
            finish(
                &tx,
                id,
                AnswerStatus::Failed,
                Some("interrupted_unknown_outcome"),
            )?;
        }
        tx.commit()?;
        Ok(ids.len() as u32)
    }
    pub fn generation_settings(&self) -> Result<GenerationSettings> {
        generation_settings(&self.connection)
    }
    pub fn set_generation_profile(
        &mut self,
        command: &str,
        request: SetGenerationProfile,
    ) -> Result<GenerationSettingsMutation> {
        let kind = "generation.configure";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let old = generation_settings(&tx)?;
        if old.row_version != request.base_row_version {
            return Err(Error::MetadataConflict {
                object_id: "generation_settings".into(),
                expected: request.base_row_version,
                actual: old.row_version,
            });
        }
        let row_version = old
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput("generation settings version overflow"))?;
        let mut profile = request.profile;
        if let Some(p) = &mut profile {
            if p.channel_id.is_some() {
                return Err(Error::InvalidInput("generation_channel_selection_required"));
            }
            p.config_revision = row_version;
            validate_generation_profile(p)?;
        }
        let json = profile.as_ref().map(serde_json::to_string).transpose()?;
        tx.execute("UPDATE search_settings SET generation_profile_json=?1,generation_row_version=?2 WHERE singleton=1",params![json,row_version])?;
        let result = GenerationSettingsMutation {
            settings: GenerationSettings {
                profile,
                row_version,
            },
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            kind,
            "generation_settings",
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn accept_answer(&mut self, command: &str, request: StartAnswer) -> Result<AnswerMutation> {
        let kind = "answers.start";
        let hash = crate::mutations::request_hash(command, kind, &request)?;
        if let Some(r) = crate::mutations::replay(&self.connection, command, &hash, kind)? {
            return Ok(r);
        }
        validate_answer_search(&request.search)?;
        validate_citation_count(request.citation_count)?;
        validate_answer_output_tokens(request.max_output_tokens, request.model.as_ref())?;
        if let Some(agent) = &request.agent {
            validate_agent_options(agent)?;
            if request
                .model
                .as_ref()
                .is_some_and(|p| p.provider == "ollama")
            {
                return Err(Error::InvalidInput("agent_remote_model_required"));
            }
        }
        if let Some(p) = &request.model {
            validate_structured_generation_profile(p)?;
            validate_generation_profile_reference(&self.connection, p)?;
        }
        crate::retrieval::eligible_ids(&self.connection, &request.search.scope)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO answers(id,request_json,status,created_at,updated_at) VALUES(?1,?2,'queued',?3,?3)",params![id,serde_json::to_string(&request)?,now])?;
        event(&tx, &id, AnswerStatus::Queued, None)?;
        let r = answer_receipt(&tx, command, &hash, kind, &id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn answer(&self, id: &str) -> Result<AnswerRecord> {
        read(&self.connection, id)
    }
    pub fn answers_page(&self, after_id: Option<&str>, limit: u32) -> Result<Vec<AnswerRecord>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("invalid page size"));
        }
        let cursor = after_id.map(|id| read(&self.connection, id)).transpose()?;
        let mut stmt=self.connection.prepare("SELECT id FROM answers WHERE ?1 IS NULL OR (created_at,id)<(?1,?2) ORDER BY created_at DESC,id DESC LIMIT ?3")?;
        let ids = stmt
            .query_map(
                params![cursor.as_ref().map(|a| &a.created_at), after_id, limit],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| read(&self.connection, id)).collect()
    }
    /// One page of collected ratings, newest change first.
    ///
    /// The cursor is `changed_at|answer_id`, which is exactly this page's order:
    /// a rating's change time is not a column, it comes from the command receipt
    /// that recorded the change, the same way `read` resolves it.
    pub fn feedback_page(&self, cursor: Option<&str>, limit: u32) -> Result<FeedbackPage> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("invalid page size"));
        }
        let (after_at, after_id) = match cursor {
            Some(value) => {
                let (at, id) = feedback_cursor(value)?;
                (Some(at), Some(id))
            }
            None => (None, None),
        };
        let mut stmt = self.connection.prepare(
            "WITH rated AS (
                 SELECT f.answer_id,f.rating,f.comment,a.request_json,
                        COALESCE((SELECT r.committed_at FROM command_receipts r
                                  WHERE r.command_kind='answers.feedback'
                                    AND r.object_id=f.answer_id
                                  ORDER BY r.change_seq DESC LIMIT 1),a.updated_at) AS changed_at
                 FROM answer_feedback f JOIN answers a ON a.id=f.answer_id
             )
             SELECT answer_id,rating,comment,request_json,changed_at FROM rated
             WHERE ?1 IS NULL OR (changed_at,answer_id) < (?1,?2)
             ORDER BY changed_at DESC,answer_id DESC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![after_at, after_id, limit + 1], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        // One row past the page is how the page knows another one follows.
        let more = rows.len() > limit as usize;
        let items = rows
            .into_iter()
            .take(limit as usize)
            .map(
                |(answer_id, rating, comment, request_json, changed_at)| FeedbackRow {
                    question: feedback_question(&request_json),
                    answer_id,
                    rating,
                    comment,
                    changed_at,
                },
            )
            .collect::<Vec<_>>();
        let next_cursor = match (more, items.last()) {
            (true, Some(last)) => Some(format!("{}|{}", last.changed_at, last.answer_id)),
            _ => None,
        };
        Ok(FeedbackPage { items, next_cursor })
    }

    /// Counts over every collected rating, and when the newest one changed.
    pub fn feedback_summary(&self) -> Result<FeedbackSummary> {
        let (total, helpful, unhelpful, neutral, commented): (i64, i64, i64, i64, i64) =
            self.connection.query_row(
                "SELECT count(*),COALESCE(SUM(rating=1),0),COALESCE(SUM(rating=-1),0),
                        COALESCE(SUM(rating NOT IN (-1,1)),0),
                        COALESCE(SUM(length(trim(comment))>0),0)
                 FROM answer_feedback",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        let newest_at: Option<String> = self.connection.query_row(
            "SELECT MAX(COALESCE((SELECT r.committed_at FROM command_receipts r
                                  WHERE r.command_kind='answers.feedback'
                                    AND r.object_id=f.answer_id
                                  ORDER BY r.change_seq DESC LIMIT 1),a.updated_at))
             FROM answer_feedback f JOIN answers a ON a.id=f.answer_id",
            [],
            |r| r.get(0),
        )?;
        Ok(FeedbackSummary {
            total: total.max(0) as u32,
            helpful: helpful.max(0) as u32,
            unhelpful: unhelpful.max(0) as u32,
            neutral: neutral.max(0) as u32,
            commented: commented.max(0) as u32,
            newest_at,
        })
    }

    pub fn claim_answer(&mut self, id: &str) -> Result<Option<AnswerLease>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let a = read(&tx, id)?;
        if a.status != AnswerStatus::Queued {
            return Ok(None);
        }
        let token = Uuid::new_v4().to_string();
        tx.execute("UPDATE answers SET status='running',lease_token=?2,lease_expires_at=?3,updated_at=?4 WHERE id=?1",params![id,token,(Utc::now()+Duration::minutes(5)).to_rfc3339(),Utc::now().to_rfc3339()])?;
        event(&tx, id, AnswerStatus::Running, None)?;
        let answer = read(&tx, id)?;
        tx.commit()?;
        Ok(Some(AnswerLease {
            answer,
            lease_token: token,
        }))
    }
    pub fn prepare_answer_evidence(
        &mut self,
        id: &str,
        token: &str,
        candidates: Vec<VectorCandidate>,
    ) -> Result<AnswerRecord> {
        self.prepare_answer_evidence_with_annotations(id, token, candidates, vec![])
    }
    pub fn prepare_answer_evidence_with_annotations(
        &mut self,
        id: &str,
        token: &str,
        candidates: Vec<VectorCandidate>,
        annotations: Vec<AnnotationVectorCandidate>,
    ) -> Result<AnswerRecord> {
        lease(&self.connection, id, token)?;
        let a = read(&self.connection, id)?;
        if a.request.agent.is_some() {
            return Err(Error::InvalidInput("agent_evidence_freeze_required"));
        }
        let selected = self
            .search_with_annotation_vectors(&a.request.search, candidates, annotations)?
            .items
            .into_iter()
            .take(citation_cap(a.request.citation_count))
            .collect::<Vec<_>>();
        let evidence = freeze_evidence(&self.connection, selected);
        if evidence
            .iter()
            .map(|e| e.evidence.text.len())
            .sum::<usize>()
            > MAX_TEXT
        {
            return Err(Error::InvalidInput("answer evidence exceeds limit"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        let prepared: bool = tx.query_row(
            "SELECT evidence_prepared FROM answers WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        if prepared {
            return Err(Error::InvalidInput("answer evidence already frozen"));
        }
        tx.execute(
            "UPDATE answers SET evidence_json=?2,evidence_prepared=1,updated_at=?3 WHERE id=?1",
            params![
                id,
                serde_json::to_string(&evidence)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        let frozen = read(&tx, id)?;
        if evidence.is_empty() {
            finish(&tx, id, AnswerStatus::NoEvidence, None)?;
        } else if !frozen.evidence_current {
            finish(
                &tx,
                id,
                AnswerStatus::EvidenceChanged,
                Some("evidence_changed"),
            )?;
        } else if a.request.model.is_none() {
            finish(&tx, id, AnswerStatus::ModelUnconfigured, None)?;
        }
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn append_answer_delta(
        &mut self,
        id: &str,
        token: &str,
        sequence: u32,
        delta: &str,
    ) -> Result<AnswerRecord> {
        if delta.is_empty() || delta.len() > 16384 || delta.contains('\0') {
            return Err(Error::InvalidInput("invalid answer delta"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        let a = read(&tx, id)?;
        if sequence <= a.last_event_seq {
            let prior: Option<String> = tx
                .query_row(
                    "SELECT text_delta FROM answer_events WHERE answer_id=?1 AND seq=?2",
                    params![id, sequence],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if prior.as_deref() == Some(delta) {
                return Ok(a);
            }
            return Err(Error::IdempotencyMismatch);
        }
        if sequence != a.last_event_seq + 1 {
            return Err(Error::InvalidInput("answer event sequence gap"));
        }
        if a.evidence.is_empty()
            || a.request.model.is_none()
            || a.text.len() + delta.len() > MAX_TEXT
        {
            return Err(Error::InvalidInput("answer cannot accept delta"));
        }
        tx.execute(
            "UPDATE answers SET text=text||?2,updated_at=?3 WHERE id=?1",
            params![id, delta, Utc::now().to_rfc3339()],
        )?;
        event(&tx, id, AnswerStatus::Running, Some(delta))?;
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    /// Accept the exact refusal marker; the cleared lease is authenticated on replay
    /// by an immutable receipt whose payload binds the answer, lease and complete text.
    pub fn complete_answer_insufficient_evidence(
        &mut self,
        id: &str,
        token: &str,
        text: &str,
    ) -> Result<AnswerRecord> {
        if text != INSUFFICIENT_EVIDENCE_MARKER {
            return Err(Error::InvalidInput("invalid insufficient evidence marker"));
        }
        document::valid_uuid(id)?;
        let kind = "answers.insufficient_evidence";
        let digest = mutations::request_hash(token, kind, &(id, token, text))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay::<AnswerMutation>(&tx, token, &digest, kind)? {
            return Ok(result.answer);
        }
        lease(&tx, id, token)?;
        let answer = read(&tx, id)?;
        let streamed:String=tx.query_row("SELECT coalesce(group_concat(text_delta,''),'') FROM (SELECT text_delta FROM answer_events WHERE answer_id=?1 AND text_delta IS NOT NULL ORDER BY seq)",[id],|r|r.get(0))?;
        let prepared: bool = tx.query_row(
            "SELECT evidence_prepared FROM answers WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        if !prepared
            || answer.evidence.is_empty()
            || answer.request.model.is_none()
            || answer.text != streamed
            || !(streamed.is_empty() || streamed == INSUFFICIENT_EVIDENCE_MARKER)
        {
            return Err(Error::InvalidInput(
                "insufficient evidence completion requires prepared evidence and marker stream",
            ));
        }
        tx.execute(
            "UPDATE answers SET text=?2,cited_ids_json='[]' WHERE id=?1",
            params![id, text],
        )?;
        if answer.evidence_current {
            finish(
                &tx,
                id,
                AnswerStatus::NoEvidence,
                Some("insufficient_evidence"),
            )?;
        } else {
            finish(
                &tx,
                id,
                AnswerStatus::EvidenceChanged,
                Some("evidence_changed"),
            )?;
            let result = read(&tx, id)?;
            tx.commit()?;
            return Ok(result);
        }
        let result = answer_receipt(&tx, token, &digest, kind, id)?;
        tx.commit()?;
        Ok(result.answer)
    }
    pub fn complete_answer(&mut self, request: CompleteAnswer) -> Result<AnswerRecord> {
        if request.text.len() > MAX_TEXT
            || request.text.contains('\0')
            || request.cited_ids.len() > 32
        {
            return Err(Error::InvalidInput("answer exceeds limit"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, &request.answer_id, &request.lease_token)?;
        let a = read(&tx, &request.answer_id)?;
        if a.evidence.is_empty() || a.request.model.is_none() {
            return Err(Error::InvalidInput("evidence and model required"));
        }
        let valid = citations_valid(&request.text, &request.cited_ids, &a.evidence);
        let status = if !a.evidence_current {
            AnswerStatus::EvidenceChanged
        } else if !valid {
            AnswerStatus::InvalidCitation
        } else {
            AnswerStatus::Completed
        };
        tx.execute(
            "UPDATE answers SET text=?2,cited_ids_json=?3 WHERE id=?1",
            params![
                request.answer_id,
                request.text,
                serde_json::to_string(&request.cited_ids)?
            ],
        )?;
        finish(&tx, &request.answer_id, status, None)?;
        let r = read(&tx, &request.answer_id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn fail_answer(
        &mut self,
        id: &str,
        token: &str,
        status: AnswerStatus,
        code: &str,
    ) -> Result<AnswerRecord> {
        if !matches!(status, AnswerStatus::Failed | AnswerStatus::TimedOut)
            || code.is_empty()
            || code.len() > 80
            || !code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::InvalidInput("invalid answer failure"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lease(&tx, id, token)?;
        finish(&tx, id, status, Some(code))?;
        let r = read(&tx, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn cancel_answer(&mut self, command: &str, id: &str) -> Result<AnswerMutation> {
        let kind = "answers.cancel";
        let hash = crate::mutations::request_hash(command, kind, &id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let a = read(&tx, id)?;
        if matches!(a.status, AnswerStatus::Queued | AnswerStatus::Running) {
            finish(&tx, id, AnswerStatus::Canceled, None)?;
        }
        let r = answer_receipt(&tx, command, &hash, kind, id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn pending_answer_ids(&self, limit: u32) -> Result<Vec<String>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("invalid page size"));
        }
        Ok(self
            .connection
            .prepare(
                "SELECT id FROM answers WHERE status='queued' ORDER BY created_at,id LIMIT ?1",
            )?
            .query_map([limit], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn recover_expired_answers(&mut self) -> Result<u32> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = tx
            .prepare("SELECT id FROM answers WHERE status='running' AND lease_expires_at<=?1")?
            .query_map([Utc::now().to_rfc3339()], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in &ids {
            finish(&tx, id, AnswerStatus::Failed, Some("interrupted"))?;
        }
        tx.commit()?;
        Ok(ids.len() as u32)
    }
    pub fn answer_events(&self, id: &str, after_seq: u32, limit: u32) -> Result<Vec<AnswerEvent>> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::InvalidInput("invalid page size"));
        }
        read(&self.connection, id)?;
        let rows=self.connection.prepare("SELECT seq,status,text_delta FROM answer_events WHERE answer_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?.query_map(params![id,after_seq,limit],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(seq, s, text_delta)| {
                Ok(AnswerEvent {
                    seq,
                    status: serde_json::from_value(serde_json::Value::String(s))?,
                    text_delta,
                })
            })
            .collect()
    }
    pub fn feedback_answer(
        &mut self,
        command: &str,
        request: AnswerFeedback,
    ) -> Result<AnswerMutation> {
        let kind = "answers.feedback";
        let hash = crate::mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        if !(-1..=1).contains(&request.rating)
            || request.comment.len() > 8192
            || request.comment.contains('\0')
            || request.updated_at.is_some()
        {
            return Err(Error::InvalidInput("invalid feedback"));
        }
        let a = read(&tx, &request.answer_id)?;
        if matches!(a.status, AnswerStatus::Queued | AnswerStatus::Running) {
            return Err(Error::InvalidInput("answer is still running"));
        }
        tx.execute("INSERT INTO answer_feedback VALUES(?1,?2,?3) ON CONFLICT(answer_id) DO UPDATE SET rating=excluded.rating,comment=excluded.comment",params![request.answer_id,request.rating,request.comment])?;
        let r = feedback_receipt(&tx, command, &hash, kind, &request.answer_id)?;
        tx.commit()?;
        Ok(r)
    }
}

fn proposal_read(db: &Connection, id: &str) -> Result<ProposalRecord> {
    crate::document::valid_uuid(id)?;
    let row = db
        .query_row(
            "SELECT request_json,status,applied_revision_id,created_at FROM proposals WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(ProposalRecord {
        id: id.into(),
        request: serde_json::from_str(&row.0)?,
        status: serde_json::from_value(serde_json::Value::String(row.1))?,
        applied_note_id: row
            .2
            .as_ref()
            .map(|revision| {
                db.query_row(
                    "SELECT note_id FROM note_revisions WHERE revision_id=?1",
                    [revision],
                    |r| r.get(0),
                )
            })
            .transpose()?,
        applied_revision_id: row.2,
        created_at: row.3,
    })
}
fn proposal_receipt(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<ProposalMutation> {
    let r = ProposalMutation {
        proposal: proposal_read(db, id)?,
        receipt: crate::mutations::next_receipt(db, command)?,
    };
    crate::mutations::store_receipt(db, hash, kind, id, &r.receipt, &r)?;
    Ok(r)
}
/// A purged object is legitimate history rather than an integrity failure: the
/// answer keeps the evidence text it was frozen with and `evidence_current`
/// already reports it as stale. Only objects that still exist must match their
/// frozen evidence, because only then can the text be compared at all.
fn object_purged(db: &Connection, object_id: &str) -> Result<bool> {
    Ok(!db
        .prepare("SELECT 1 FROM objects WHERE id=?1")?
        .exists([object_id])?)
}
fn usable_answer(db: &Connection, id: &str) -> Result<()> {
    let a = read(db, id)?;
    if a.status != AnswerStatus::Completed || !a.evidence_current {
        return Err(Error::InvalidInput(
            "proposal requires completed current evidence",
        ));
    }
    Ok(())
}

/// Check immutable historical snapshots without requiring current search eligibility.
pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let database_schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if database_schema < 16
        && db
            .prepare("SELECT 1 FROM answers WHERE json_type(request_json,'$.model.channel_id') IS NOT NULL OR json_type(request_json,'$.model.capabilities') IS NOT NULL")?
            .exists([])?
    {
        return Err(Error::IntegrityFailure);
    }
    validate_generation_settings_backup(db)?;
    crate::retrieval_settings::validate_retrieval_settings_backup(db)?;
    crate::cleaning::validate_cleaning_settings_backup(db)?;
    crate::chunk_settings::validate_chunk_settings_backup(db)?;
    let ids = db
        .prepare("SELECT id FROM answers ORDER BY id")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let a = read(db, &id)?;
        let q = &a.request.search;
        if q.query.trim().is_empty()
            || q.query.len() > 4096
            || !(1..=MAX_TOP_K).contains(&q.limit)
            || a.request
                .citation_count
                .is_some_and(|n| !(1..=MAX_CITATION_COUNT).contains(&n))
            || q.minimum_cosine
                .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
            || a.text.len() > MAX_TEXT
            || a.text.contains('\0')
            || a.evidence.len() > 32
            || a.evidence
                .iter()
                .map(|e| e.evidence.text.len())
                .sum::<usize>()
                > MAX_TEXT
            || a.cited_ids.len() > 32
        {
            return Err(Error::IntegrityFailure);
        }
        validate_answer_output_tokens(a.request.max_output_tokens, a.request.model.as_ref())
            .map_err(|_| Error::IntegrityFailure)?;
        if let Some(agent) = &a.request.agent {
            validate_agent_options(agent).map_err(|_| Error::IntegrityFailure)?;
            if a.request
                .model
                .as_ref()
                .is_some_and(|p| p.provider == "ollama")
            {
                return Err(Error::IntegrityFailure);
            }
            if a.evidence.len() > agent.max_evidence as usize {
                return Err(Error::IntegrityFailure);
            }
        }
        if let Some(p) = &a.request.model {
            validate_generation_profile_for_backup(db, p, database_schema)?;
        }
        if q.scope.object_ids.len() > 100 || q.scope.tag_ids.len() > 100 || q.scope.kinds.len() > 2
        {
            return Err(Error::IntegrityFailure);
        }
        for id in q
            .scope
            .object_ids
            .iter()
            .chain(&q.scope.tag_ids)
            .chain(q.scope.collection_id.iter())
        {
            crate::document::valid_uuid(id)?;
        }
        chrono::DateTime::parse_from_rfc3339(&a.created_at).map_err(|_| Error::IntegrityFailure)?;
        chrono::DateTime::parse_from_rfc3339(&a.updated_at).map_err(|_| Error::IntegrityFailure)?;
        if a.error_code.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 80
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) {
            return Err(Error::IntegrityFailure);
        }
        let (prepared, token, expires): (bool, Option<String>, Option<String>) = db.query_row(
            "SELECT evidence_prepared,lease_token,lease_expires_at FROM answers WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if a.status == AnswerStatus::Running {
            crate::document::valid_uuid(token.as_deref().ok_or(Error::IntegrityFailure)?)?;
            chrono::DateTime::parse_from_rfc3339(
                expires.as_deref().ok_or(Error::IntegrityFailure)?,
            )
            .map_err(|_| Error::IntegrityFailure)?;
        } else if token.is_some() || expires.is_some() {
            return Err(Error::IntegrityFailure);
        }
        if !prepared && (!a.evidence.is_empty() || !a.text.is_empty()) {
            return Err(Error::IntegrityFailure);
        }
        if a.status == AnswerStatus::Queued && (prepared || !a.cited_ids.is_empty()) {
            return Err(Error::IntegrityFailure);
        }
        if a.status == AnswerStatus::Completed
            && (!prepared
                || a.request.model.is_none()
                || !citations_valid(&a.text, &a.cited_ids, &a.evidence))
        {
            return Err(Error::IntegrityFailure);
        }
        if a.status == AnswerStatus::NoEvidence {
            let empty_retrieval =
                a.evidence.is_empty() && a.text.is_empty() && a.error_code.is_none();
            let insufficient = !a.evidence.is_empty()
                && a.request.model.is_some()
                && a.text == INSUFFICIENT_EVIDENCE_MARKER
                && a.error_code.as_deref() == Some("insufficient_evidence");
            if !prepared || !a.cited_ids.is_empty() || !(empty_retrieval || insufficient) {
                return Err(Error::IntegrityFailure);
            }
        }
        if a.status == AnswerStatus::ModelUnconfigured
            && (!prepared
                || (a.evidence.is_empty() && a.request.agent.is_none())
                || a.request.model.is_some()
                || !a.text.is_empty())
        {
            return Err(Error::IntegrityFailure);
        }
        for (i, e) in a.evidence.iter().enumerate() {
            let t = &e.evidence.target;
            if e.citation_id != format!("E{}", i + 1)
                || e.evidence.title.len() > 4096
                || e.evidence.text.is_empty()
                || e.context
                    .as_ref()
                    .is_some_and(|c| c.is_empty() || c.chars().count() > MAX_CONTEXT_CHARS)
                || t.metadata_revision == 0
                || t.start_codepoint >= t.end_codepoint
                || e.evidence
                    .cosine_similarity
                    .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
                || e.evidence.fusion_rank.is_some_and(|v| !v.is_finite())
            {
                return Err(Error::IntegrityFailure);
            }
            crate::document::valid_uuid(&t.object_id)?;
            crate::document::valid_uuid(&t.content_revision)?;
            if object_purged(db, &t.object_id)? {
                continue;
            }
            if e.evidence.annotation.is_some() {
                if !crate::annotation_retrieval::validate_annotation_evidence(
                    db,
                    &q.scope,
                    &e.evidence,
                    true,
                )? {
                    return Err(Error::IntegrityFailure);
                }
                continue;
            }
            if e.evidence.id.starts_with("annotation:")
                || (!q.scope.object_ids.is_empty() && !q.scope.object_ids.contains(&t.object_id))
                || (!q.scope.kinds.is_empty() && !q.scope.kinds.contains(&t.kind))
                || (t.kind == EvidenceKind::Note && q.scope.source_format.is_some())
            {
                return Err(Error::IntegrityFailure);
            }
            if t.kind == EvidenceKind::Source
                && let Some(format) = q.scope.source_format
                && crate::sources::read_version(db, &t.content_revision)?.format != format
            {
                return Err(Error::IntegrityFailure);
            }
            if let Some(generation) = &e.evidence.generation_id {
                crate::document::valid_uuid(generation)?;
                let row:Option<(String,String,String,String,String,u32)>=db.query_row("SELECT c.target_json,c.title,c.text,g.object_id,g.content_revision,g.metadata_revision FROM index_chunks c JOIN index_generations g ON g.id=c.generation_id WHERE c.id=?1 AND g.id=?2",params![e.evidence.id,generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
                let (target, title, text, object, revision, metadata) =
                    row.ok_or(Error::IntegrityFailure)?;
                if serde_json::from_str::<serde_json::Value>(&target)? != serde_json::to_value(t)?
                    || title != e.evidence.title
                    || text != e.evidence.text
                    || object != t.object_id
                    || revision != t.content_revision
                    || metadata != t.metadata_revision
                {
                    return Err(Error::IntegrityFailure);
                }
            }
            let body = match t.kind {
                EvidenceKind::Note => {
                    if t.parse_id.is_some() || t.page_index.is_some() {
                        return Err(Error::IntegrityFailure);
                    }
                    let n = crate::notes::revision(db, &t.object_id, &t.content_revision)?;
                    if n.title != e.evidence.title {
                        return Err(Error::IntegrityFailure);
                    }
                    n.document
                        .nodes()
                        .into_iter()
                        .find(|b| b.attrs.id == t.block_id)
                        .ok_or(Error::IntegrityFailure)?
                        .plain_text()
                }
                EvidenceKind::Source => {
                    let (text,page):(String,Option<u32>)=db.query_row("SELECT b.text,b.page_index FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id JOIN source_versions v ON v.id=p.source_version_id WHERE b.id=?1 AND p.id=?2 AND v.id=?3 AND v.source_id=?4",params![t.block_id,t.parse_id,t.content_revision,t.object_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::IntegrityFailure)?;
                    if page != t.page_index {
                        return Err(Error::IntegrityFailure);
                    }
                    text
                }
            };
            if t.end_codepoint as usize > body.chars().count()
                || body
                    .chars()
                    .skip(t.start_codepoint as usize)
                    .take((t.end_codepoint - t.start_codepoint) as usize)
                    .collect::<String>()
                    != e.evidence.text
            {
                return Err(Error::IntegrityFailure);
            }
            if let Some(context) = e.context.as_deref()
                && (!body.contains(context) || !context.contains(e.evidence.text.as_str()))
            {
                return Err(Error::IntegrityFailure);
            }
        }
        if a.evidence
            .iter()
            .filter_map(|e| e.context.as_deref())
            .map(|context| context.chars().count())
            .sum::<usize>()
            > MAX_CONTEXT_TOTAL_CHARS
        {
            return Err(Error::IntegrityFailure);
        }
        let events = db
            .prepare(
                "SELECT seq,status,text_delta FROM answer_events WHERE answer_id=?1 ORDER BY seq",
            )?
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if events.is_empty() {
            return Err(Error::IntegrityFailure);
        }
        let mut last = AnswerStatus::Queued;
        let mut streamed = 0;
        for (i, (seq, status, delta)) in events.into_iter().enumerate() {
            let status: AnswerStatus = serde_json::from_value(serde_json::Value::String(status))?;
            if seq as usize != i + 1
                || (i == 0 && status != AnswerStatus::Queued)
                || (i > 0 && !matches!(last, AnswerStatus::Queued | AnswerStatus::Running))
                || (i > 0 && status == AnswerStatus::Queued)
                || (i > 0
                    && last == AnswerStatus::Queued
                    && !matches!(
                        status,
                        AnswerStatus::Running | AnswerStatus::Canceled | AnswerStatus::Failed
                    ))
            {
                return Err(Error::IntegrityFailure);
            }
            if let Some(delta) = delta {
                streamed += delta.len();
                if status != AnswerStatus::Running
                    || delta.is_empty()
                    || delta.len() > 16384
                    || delta.contains('\0')
                    || streamed > MAX_TEXT
                {
                    return Err(Error::IntegrityFailure);
                }
            }
            last = status;
        }
        if last != a.status {
            return Err(Error::IntegrityFailure);
        }
    }
    let ids = db
        .prepare("SELECT id FROM proposals")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let p = proposal_read(db, &id)?;
        if database_schema < 15 && p.request.document_schema > 2 {
            return Err(Error::IntegrityFailure);
        }
        validate_proposal_origin(db, &p.request, true)?;
        p.request.document.validate(p.request.document_schema)?;
        let expected = p
            .request
            .document
            .nodes()
            .into_iter()
            .filter(|n| n.kind == NodeType::Image)
            .filter_map(|n| n.attrs.blob_hash.clone())
            .collect::<BTreeSet<_>>();
        let actual = db
            .prepare("SELECT blob_hash FROM proposal_blobs WHERE proposal_id=?1")?
            .query_map([&id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if expected != actual {
            return Err(Error::IntegrityFailure);
        }
        crate::validate_text(&p.request.title, "")?;
        match (&p.request.note_id, &p.request.base_revision_id) {
            (Some(note), Some(base)) => {
                crate::notes::revision(db, note, base)?;
            }
            (None, None) => {}
            _ => return Err(Error::IntegrityFailure),
        }
        chrono::DateTime::parse_from_rfc3339(&p.created_at).map_err(|_| Error::IntegrityFailure)?;
        match (&p.status, &p.applied_revision_id) {
            (ProposalStatus::Applied, Some(id)) => {
                let note_id = p
                    .applied_note_id
                    .as_deref()
                    .ok_or(Error::IntegrityFailure)?;
                let n = crate::notes::revision(db, note_id, id)?;
                if n.author_kind != "ai_proposal"
                    || n.parent_revision_id != p.request.base_revision_id
                    || p.request.note_id.as_deref().is_some_and(|id| id != note_id)
                    || n.title != p.request.title.trim()
                    || n.document != p.request.document
                {
                    return Err(Error::IntegrityFailure);
                }
            }
            (ProposalStatus::Pending | ProposalStatus::Rejected, None) => {}
            _ => return Err(Error::IntegrityFailure),
        }
    }
    let invalid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM answer_feedback f JOIN answers a ON a.id=f.answer_id WHERE rating NOT BETWEEN -1 AND 1 OR length(CAST(comment AS BLOB))>8192 OR instr(comment,char(0))>0 OR a.status IN ('queued','running'))",[],|r|r.get(0))?;
    if invalid {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}
impl Library {
    pub fn proposal(&self, id: &str) -> Result<ProposalRecord> {
        proposal_read(&self.connection, id)
    }
    pub fn create_proposal(
        &mut self,
        command: &str,
        mut request: CreateProposal,
    ) -> Result<ProposalMutation> {
        let kind = "proposals.create";
        let hash = crate::mutations::request_hash(command, kind, &request)?;
        if let Some(r) = crate::mutations::replay(&self.connection, command, &hash, kind)? {
            return Ok(r);
        }
        request.document.validate(request.document_schema)?;
        self.resolve_wiki_document(
            request.note_id.as_deref(),
            &request.title,
            &mut request.document,
        )?;
        crate::notes::Prepared::new(
            &request.title,
            request.document_schema,
            request.document.clone(),
            None,
        )?;
        self.verify_document_attachments(&request.document)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        validate_proposal_origin(&tx, &request, false)?;
        match (&request.note_id, &request.base_revision_id) {
            (Some(note), Some(base)) => {
                if crate::notes::revision(&tx, note, base)?.trashed {
                    return Err(Error::Trashed);
                }
            }
            (None, None) => {}
            _ => {
                return Err(Error::InvalidInput(
                    "proposal note and base revision must both be present or absent",
                ));
            }
        }
        let id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO proposals(id,request_json,status,created_at) VALUES(?1,?2,'pending',?3)",
            params![
                id,
                serde_json::to_string(&request)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        for hash in request
            .document
            .nodes()
            .into_iter()
            .filter(|n| n.kind == NodeType::Image)
            .filter_map(|n| n.attrs.blob_hash.as_deref())
            .collect::<BTreeSet<_>>()
        {
            tx.execute(
                "INSERT INTO proposal_blobs(proposal_id,blob_hash) VALUES(?1,?2)",
                params![id, hash],
            )?;
        }
        let r = proposal_receipt(&tx, command, &hash, kind, &id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn apply_proposal(&mut self, command: &str, id: &str) -> Result<NoteCommit> {
        let kind = "proposals.apply";
        let hash = crate::notes::request_hash(command, kind, &id)?;
        if let Some(r) = crate::notes::replay(&self.connection, command, &hash)? {
            return Ok(r);
        }
        let p = proposal_read(&self.connection, id)?;
        let prepared = crate::notes::Prepared::new(
            &p.request.title,
            p.request.document_schema,
            p.request.document.clone(),
            None,
        )?;
        self.verify_document_attachments(&p.request.document)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::notes::replay(&tx, command, &hash)? {
            return Ok(r);
        }
        let p = proposal_read(&tx, id)?;
        if p.status == ProposalStatus::Applied {
            let note = crate::notes::revision(
                &tx,
                p.applied_note_id
                    .as_deref()
                    .ok_or(Error::IntegrityFailure)?,
                p.applied_revision_id
                    .as_deref()
                    .ok_or(Error::IntegrityFailure)?,
            )?;
            let result = crate::notes::receipt(&tx, command, &hash, kind, note)?;
            tx.commit()?;
            return Ok(result);
        }
        if p.status != ProposalStatus::Pending {
            return Err(Error::InvalidInput("proposal is not pending"));
        }
        validate_proposal_origin(&tx, &p.request, false)?;
        let note = match (&p.request.note_id, &p.request.base_revision_id) {
            (Some(note), Some(base)) => {
                let current = crate::notes::checked_head(&tx, note, base)?;
                crate::notes::commit_revision(&tx, note, Some(&current), prepared, "ai_proposal")?
            }
            (None, None) => crate::notes::commit_revision(
                &tx,
                &Uuid::new_v4().to_string(),
                None,
                prepared,
                "ai_proposal",
            )?,
            _ => {
                return Err(Error::InvalidInput(
                    "proposal note and base revision must both be present or absent",
                ));
            }
        };
        tx.execute(
            "UPDATE proposals SET status='applied',applied_revision_id=?2 WHERE id=?1",
            params![id, note.revision_id],
        )?;
        let r = crate::notes::receipt(&tx, command, &hash, kind, note)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn reject_proposal(&mut self, command: &str, id: &str) -> Result<ProposalMutation> {
        let kind = "proposals.reject";
        let hash = crate::mutations::request_hash(command, kind, &id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = crate::mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        if proposal_read(&tx, id)?.status != ProposalStatus::Pending {
            return Err(Error::InvalidInput("proposal is not pending"));
        }
        tx.execute("UPDATE proposals SET status='rejected' WHERE id=?1", [id])?;
        let r = proposal_receipt(&tx, command, &hash, kind, id)?;
        tx.commit()?;
        Ok(r)
    }
}

fn validate_proposal_origin(
    db: &Connection,
    request: &CreateProposal,
    history: bool,
) -> Result<()> {
    match (
        &request.answer_id,
        &request.chat_run_id,
        &request.external_request_id,
    ) {
        (Some(id), None, None) => {
            if history {
                if read(db, id)?.status != AnswerStatus::Completed {
                    return Err(Error::IntegrityFailure);
                }
            } else {
                usable_answer(db, id)?;
            }
        }
        (None, Some(id), None) => {
            document::valid_uuid(id)?;
            if crate::chat::run(db, id)?.status != ChatRunStatus::Completed {
                return Err(Error::InvalidInput("proposal_requires_completed_chat"));
            }
        }
        (None, None, Some(id))
            if !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control) => {}
        _ => return Err(Error::InvalidInput("invalid_proposal_origin")),
    }
    Ok(())
}

/// `changed_at|answer_id`, the order the page cursor was taken from.
fn feedback_cursor(value: &str) -> Result<(String, String)> {
    let (at, id) = value
        .split_once('|')
        .ok_or(Error::InvalidInput("invalid feedback cursor"))?;
    crate::document::valid_uuid(id)?;
    if at.is_empty() || at.len() > 64 {
        return Err(Error::InvalidInput("invalid feedback cursor"));
    }
    Ok((at.to_string(), id.to_string()))
}

/// The question a rating was given for. A report page reads historical payloads
/// defensively: one odd request must not fail the whole page.
fn feedback_question(request_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(request_json)
        .ok()
        .and_then(|value| {
            value
                .get("search")?
                .get("query")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod backup_tests {
    use super::*;
    #[test]
    fn history_backup_preserves_stale_evidence_but_rejects_forged_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        let n = db.create_note("evidence", "original evidence").unwrap();
        let a = db
            .accept_answer(
                &Uuid::new_v4().to_string(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: SearchQuery {
                        query: "evidence".into(),
                        mode: SearchMode::Lexical,
                        scope: SearchScope::default(),
                        limit: 8,
                        minimum_cosine: None,
                    },
                    model: Some(GenerationProfile {
                        channel_id: None,
                        capabilities: None,
                        chat_token_limit: None,
                        provider: "ollama".into(),
                        endpoint: "http://127.0.0.1:11434".into(),
                        model: "fixture".into(),
                        model_digest: "fixture".into(),
                        config_revision: 1,
                        temperature: 0.0,
                        context_tokens: 4096,
                    }),
                },
            )
            .unwrap();
        let l = db.claim_answer(&a.answer.id).unwrap().unwrap();
        db.prepare_answer_evidence(&a.answer.id, &l.lease_token, vec![])
            .unwrap();
        db.complete_answer(CompleteAnswer {
            answer_id: a.answer.id.clone(),
            lease_token: l.lease_token,
            text: "claim [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
        validate_backup(&db.connection).unwrap();
        db.set_trashed(&n.id, true).unwrap();
        validate_backup(&db.connection).unwrap();
        db.connection.execute("UPDATE answers SET evidence_json=json_set(evidence_json,'$[0].evidence.text','forged') WHERE id=?1",[&a.answer.id]).unwrap();
        assert!(validate_backup(&db.connection).is_err());
    }
    fn frozen_answer(db: &mut Library, query: &str) -> (String, AnswerRecord) {
        let a = db
            .accept_answer(
                &Uuid::new_v4().to_string(),
                StartAnswer {
                    max_output_tokens: None,
                    citation_count: None,
                    agent: None,
                    search: SearchQuery {
                        query: query.into(),
                        mode: SearchMode::Lexical,
                        scope: SearchScope::default(),
                        limit: 8,
                        minimum_cosine: None,
                    },
                    model: Some(GenerationProfile {
                        channel_id: None,
                        capabilities: None,
                        chat_token_limit: None,
                        provider: "ollama".into(),
                        endpoint: "http://127.0.0.1:11434".into(),
                        model: "fixture".into(),
                        model_digest: "fixture".into(),
                        config_revision: 1,
                        temperature: 0.0,
                        context_tokens: 4096,
                    }),
                },
            )
            .unwrap();
        let l = db.claim_answer(&a.answer.id).unwrap().unwrap();
        let frozen = db
            .prepare_answer_evidence(&a.answer.id, &l.lease_token, vec![])
            .unwrap();
        assert_eq!(frozen.status, AnswerStatus::Running);
        db.complete_answer(CompleteAnswer {
            answer_id: a.answer.id.clone(),
            lease_token: l.lease_token,
            text: "claim [E1]".into(),
            cited_ids: vec!["E1".into()],
        })
        .unwrap();
        (a.answer.id, frozen)
    }
    #[test]
    fn frozen_evidence_expands_the_parent_block_and_rejects_a_forged_context() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        let body = format!("{}目标事实。", "填充前文。".repeat(500));
        db.create_note("长块", &body).unwrap();
        let (id, answer) = frozen_answer(&mut db, "目标事实");
        assert_eq!(answer.evidence.len(), 1);
        let item = &answer.evidence[0];
        assert_ne!(item.evidence.text, body);
        assert_eq!(item.context.as_deref(), Some(body.as_str()));
        validate_backup(&db.connection).unwrap();
        db.connection
            .execute(
                "UPDATE answers SET evidence_json=json_set(evidence_json,'$[0].context','forged') WHERE id=?1",
                [&id],
            )
            .unwrap();
        assert!(validate_backup(&db.connection).is_err());
    }
    #[test]
    fn oversized_parent_blocks_keep_a_bounded_window_around_the_hit() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        let body = format!(
            "{}唯一标记{}",
            "填充段落。".repeat(1200),
            "填充段落。".repeat(1200)
        );
        db.create_note("超长块", &body).unwrap();
        let (_, answer) = frozen_answer(&mut db, "唯一标记");
        assert_eq!(answer.evidence.len(), 1);
        let item = &answer.evidence[0];
        let context = item.context.as_deref().unwrap();
        assert!(context.chars().count() <= MAX_CONTEXT_CHARS);
        assert!(context.contains(&item.evidence.text));
        assert!(body.contains(context));
        validate_backup(&db.connection).unwrap();
    }
    #[test]
    fn short_blocks_need_no_parent_context() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Library::open(dir.path()).unwrap();
        db.create_note("短块", "目标事实。").unwrap();
        let (_, answer) = frozen_answer(&mut db, "目标事实");
        assert_eq!(answer.evidence.len(), 1);
        assert!(answer.evidence[0].context.is_none());
        validate_backup(&db.connection).unwrap();
    }
}
