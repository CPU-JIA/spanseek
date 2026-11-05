//! Append-only audit records for external requests. The table intentionally has
//! no query, excerpt, document, path, or bearer-token column.
use crate::{
    EXTERNAL_MAX_PAGE_SIZE, Error, ExternalAuditDraft, ExternalAuditEvent, ExternalAuditOutcome,
    ExternalAuditPage, ExternalAuditPageParams, ExternalErrorCode, ExternalScope, Library, Result,
};
use chrono::Utc;
use rusqlite::params;

impl Library {
    pub fn record_external_audit(&self, event: ExternalAuditDraft) -> Result<()> {
        if event.request_id.is_empty()
            || event.request_id.len() > crate::EXTERNAL_MAX_REQUEST_ID
            || !matches!(event.transport.as_str(), "mcp" | "http")
            || event.principal_id.is_empty()
            || event.principal_id.len() > 128
            || event.method.is_empty()
            || event.method.len() > 64
            || event
                .subject_id
                .as_ref()
                .is_some_and(|value| value.len() > 128)
        {
            return Err(Error::InvalidInput("invalid external audit event"));
        }
        let scopes = serde_json::to_string(&event.granted_scopes)?;
        let error_code = event
            .error_code
            .map(|code| serde_json::to_string(&code))
            .transpose()?
            .map(|value| value.trim_matches('"').to_owned());
        self.connection.execute(
            "INSERT INTO external_audit_events(request_id,occurred_at,transport,principal_id,method,required_scope,granted_scopes_json,outcome,error_code,subject_id,duration_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                event.request_id,
                Utc::now().to_rfc3339(),
                event.transport,
                event.principal_id,
                event.method,
                scope_str(event.required_scope),
                scopes,
                outcome_str(event.outcome),
                error_code,
                event.subject_id,
                event.duration_ms.min(i64::MAX as u64) as i64,
            ],
        )?;
        Ok(())
    }

    pub fn external_audit_page(
        &self,
        request: ExternalAuditPageParams,
    ) -> Result<ExternalAuditPage> {
        if !(1..=EXTERNAL_MAX_PAGE_SIZE).contains(&request.limit) {
            return Err(Error::InvalidInput(
                "external audit page limit is out of range",
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT seq,request_id,occurred_at,transport,principal_id,method,required_scope,
                    granted_scopes_json,outcome,error_code,subject_id,duration_ms
             FROM external_audit_events
             WHERE (?1 IS NULL OR seq<?1)
               AND (?2 IS NULL OR request_id=?2)
               AND (?3 IS NULL OR principal_id=?3)
               AND (?4 IS NULL OR subject_id=?4)
             ORDER BY seq DESC LIMIT ?5",
        )?;
        let rows = statement
            .query_map(
                params![
                    request.before_seq,
                    request.request_id,
                    request.principal_id,
                    request.subject_id,
                    request.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, i64>(11)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > request.limit as usize;
        let items = rows
            .into_iter()
            .take(request.limit as usize)
            .map(|row| {
                Ok(ExternalAuditEvent {
                    seq: row.0,
                    request_id: row.1,
                    occurred_at: row.2,
                    transport: row.3,
                    principal_id: row.4,
                    method: row.5,
                    required_scope: parse_scope(&row.6)?,
                    granted_scopes: serde_json::from_str(&row.7)?,
                    outcome: parse_outcome(&row.8)?,
                    error_code: row.9.as_deref().map(parse_error_code).transpose()?,
                    subject_id: row.10,
                    duration_ms: row.11.max(0) as u64,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ExternalAuditPage {
            next_before_seq: if more {
                items.last().map(|event| event.seq)
            } else {
                None
            },
            items,
        })
    }
}

fn scope_str(scope: ExternalScope) -> &'static str {
    match scope {
        ExternalScope::Read => "read",
        ExternalScope::Search => "search",
        ExternalScope::Propose => "propose",
        ExternalScope::Audit => "audit",
    }
}

fn parse_scope(value: &str) -> Result<ExternalScope> {
    match value {
        "read" => Ok(ExternalScope::Read),
        "search" => Ok(ExternalScope::Search),
        "propose" => Ok(ExternalScope::Propose),
        "audit" => Ok(ExternalScope::Audit),
        _ => Err(Error::IntegrityFailure),
    }
}

fn outcome_str(outcome: ExternalAuditOutcome) -> &'static str {
    match outcome {
        ExternalAuditOutcome::Allowed => "allowed",
        ExternalAuditOutcome::Denied => "denied",
        ExternalAuditOutcome::Failed => "failed",
    }
}

fn parse_outcome(value: &str) -> Result<ExternalAuditOutcome> {
    match value {
        "allowed" => Ok(ExternalAuditOutcome::Allowed),
        "denied" => Ok(ExternalAuditOutcome::Denied),
        "failed" => Ok(ExternalAuditOutcome::Failed),
        _ => Err(Error::IntegrityFailure),
    }
}

fn parse_error_code(value: &str) -> Result<ExternalErrorCode> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| Error::IntegrityFailure)
}
