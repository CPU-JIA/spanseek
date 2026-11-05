use crate::{
    Error, IntegrityCheckState, Library, Result, SourceFormat, StatisticCount, WorkspaceStatistics,
};
use rusqlite::Connection;

fn unsigned(value: i64) -> Result<u64> {
    let value = u64::try_from(value).map_err(|_| Error::IntegrityFailure)?;
    // DTOs use JavaScript numbers. Never silently round an otherwise valid SQL sum.
    if value > (1_u64 << 53) - 1 {
        return Err(Error::InvalidInput(
            "statistics value exceeds exact JSON number range",
        ));
    }
    Ok(value)
}

fn count(db: &Connection, sql: &str) -> Result<u64> {
    unsigned(db.query_row(sql, [], |row| row.get(0))?)
}

fn grouped(db: &Connection, sql: &str, keys: &[&str]) -> Result<Vec<StatisticCount>> {
    let mut result = keys
        .iter()
        .map(|key| StatisticCount {
            key: (*key).into(),
            count: 0,
        })
        .collect::<Vec<_>>();
    let mut statement = db.prepare(sql)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let entry = result
            .iter_mut()
            .find(|entry| entry.key == key)
            .ok_or(Error::IntegrityFailure)?;
        entry.count = unsigned(row.get(1)?)?;
    }
    Ok(result)
}

// This is deliberately a closed vocabulary, not a slug/pattern validator: a
// syntactically valid slug can still contain a user's document or machine name.
// It must stay a superset of every code the source, portable, index and answer
// paths can store, otherwise a real failure is exported as `unknown_error`.
fn diagnostic_code(code: &str) -> &str {
    match code {
        "input_unavailable"
        | "worker_unavailable"
        | "worker_timeout"
        | "worker_protocol_mismatch"
        | "model_refusal_protocol_invalid"
        | "worker_interrupted"
        | "worker_failed"
        | "parse_failed"
        | "output_limit_or_io"
        | "integrity_failed"
        | "io_failed"
        | "canceled"
        | "interrupted"
        | "interrupted_unknown_outcome"
        | "model_unconfigured"
        | "model_profile_unsupported"
        | "model_output_truncated"
        | "inference_unavailable"
        | "inference_timeout"
        | "evidence_changed"
        | "insufficient_evidence"
        | "invalid_citation"
        | "storage_full"
        | "embedding_batch_limit"
        | "embedding_chunk_limit"
        | "embedding_chunk_size_limit"
        | "embedding_inference_failed"
        | "embedding_input_limit"
        | "embedding_invalid_input"
        | "embedding_invalid_plan"
        | "embedding_invalid_vector"
        | "embedding_invalid_vector_count"
        | "embedding_model_hash_mismatch"
        | "embedding_model_invalid"
        | "embedding_model_load_failed"
        | "embedding_model_unavailable"
        | "embedding_profile_mismatch"
        | "embedding_title_token_limit"
        | "embedding_token_budget"
        | "embedding_tokenization_failed"
        | "embedding_tokenizer_invalid"
        | "inference_event_limit"
        | "inference_invalid_frame"
        | "inference_invalid_request"
        | "inference_output_unavailable"
        | "inference_protocol_error"
        | "inference_result_invalid"
        | "inference_result_limit"
        | "inference_result_unavailable"
        | "inference_runtime_unavailable"
        | "model_api_error"
        | "model_canceled"
        | "model_context_limit"
        | "model_first_token_timeout"
        | "model_idle_timeout"
        | "model_incomplete"
        | "model_invalid_endpoint"
        | "model_invalid_request"
        | "model_invalid_response"
        | "model_not_found"
        | "model_response_too_large"
        | "model_total_timeout"
        | "model_unavailable"
        | "ollama_model_unavailable"
        | "ollama_timeout"
        | "ollama_unavailable"
        | "projection_incomplete"
        | "vector_commit_failed"
        | "vector_open_failed"
        | "vector_query_failed"
        // Source worker, import and webpage-capture failures. These are the codes
        // `crates/pkms-worker` and the app's import path really store, so a
        // password-protected PDF, a broken DOCX or a failed capture keeps its own
        // code instead of collapsing into `unknown_error`.
        | "worker_pipe_failed"
        | "worker_request_limit"
        | "worker_identity_mismatch"
        | "invalid_worker_response"
        | "invalid_result"
        | "result_hash_mismatch"
        | "invalid_source"
        | "source_import_failed"
        | "attempts_exhausted"
        | "unsupported_format"
        | "input_limit"
        | "size_limit"
        | "output_unavailable"
        | "io_error"
        | "invalid_utf8"
        | "invalid_text"
        | "pdfium_unavailable"
        | "pdfium_hash_mismatch"
        | "pdf_password_required"
        | "pdf_invalid"
        | "pdf_parse_failed"
        | "pdf_page_limit"
        | "pdf_text_limit"
        | "pdf_invalid_unicode"
        | "pdf_limit"
        | "html_input_limit"
        | "html_structure_limit"
        | "html_invalid_encoding"
        | "html_text_limit"
        | "html_no_content"
        | "docx_not_container"
        | "docx_part_missing"
        | "docx_invalid_xml"
        | "docx_no_content"
        | "docx_input_limit"
        | "docx_text_limit"
        | "docx_structure_limit"
        | "webpage_invalid_url"
        | "webpage_invalid_redirect"
        | "webpage_redirect_loop"
        | "webpage_redirect_limit"
        | "webpage_http_error"
        | "webpage_network_failed"
        | "webpage_timeout"
        | "webpage_size_limit"
        | "webpage_empty"
        | "webpage_unsupported_content_type"
        | "webpage_login_required"
        | "webpage_dynamic_content"
        | "job_failed"
        | "agent_evidence_freeze_required"
        | "agent_evidence_handle_invalid"
        | "agent_evidence_invalid"
        | "agent_evidence_limit"
        | "agent_options_invalid"
        | "agent_options_missing"
        | "agent_remote_model_required"
        | "agent_request_invalid"
        | "agent_request_required"
        | "agent_tool_budget_exhausted"
        | "agent_tool_invalid"
        | "agent_tool_unknown"
        | "agent_total_timeout"
        | "answer_output_budget_invalid"
        | "answer_output_limit"
        | "chat_context_limit"
        | "chat_conversation_conflict"
        | "chat_delta_sequence"
        | "chat_empty_completion"
        | "chat_empty_response"
        | "chat_evidence_handle_invalid"
        | "chat_evidence_invalid"
        | "chat_evidence_limit"
        | "chat_history_invalid"
        | "chat_lease_lost"
        | "chat_material_limit"
        | "chat_model_budget_exhausted"
        | "chat_output_limit"
        | "chat_remote_model_required"
        | "chat_run_active"
        | "chat_scope_limit"
        | "chat_tool_budget_exhausted"
        | "chat_tool_invalid"
        | "chat_tool_unknown"
        | "chat_total_timeout"
        | "chat_unexpected_tool_calls"
        | "cloudflare_transport_unsupported"
        | "duplicate_or_invalid_channel_model"
        | "embedding_dimension_mismatch"
        | "embedding_profile_changed"
        | "embedding_remote_required"
        | "embedding_unconfigured"
        | "generation_channel_deleted"
        | "generation_channel_is_active"
        | "generation_channel_limit"
        | "generation_channel_missing"
        | "generation_channel_model_missing"
        | "generation_channel_selection_required"
        | "generation_settings_version_overflow"
        | "inference_failed"
        | "inference_identity_mismatch"
        | "inference_incomplete"
        | "inference_invalid_quote"
        | "inference_manifest_mismatch"
        | "inference_pipe_failed"
        | "inference_request_limit"
        | "inference_result_hash"
        | "inference_result_kind"
        | "inference_result_path"
        | "invalid_agent_options"
        | "invalid_answer_citation_count"
        | "invalid_chat_completion"
        | "invalid_chat_delta"
        | "invalid_chat_finish"
        | "invalid_chat_options"
        | "invalid_chat_request"
        | "invalid_chat_title"
        | "invalid_generation_channel"
        | "invalid_generation_channel_profile"
        | "invalid_model_endpoint"
        | "invalid_proposal_origin"
        | "model_authentication_failed"
        | "model_discovery_unsupported"
        | "model_invalid_key"
        | "model_key_missing"
        | "model_rate_limited"
        | "model_response_invalid"
        | "model_response_limit"
        | "model_unsupported_output"
        | "proposal_requires_completed_chat"
        | "remote_consent_required"
        | "unknown_error" => code,
        _ => "unknown_error",
    }
}

impl Library {
    /// Returns at most 100 structured error summaries, never arbitrary stage/error
    /// messages or request payloads. Invalid identifiers/timestamps are omitted.
    pub fn diagnostic_errors(&self, limit: u32) -> Result<Vec<crate::DiagnosticError>> {
        crate::collections::validate_limit(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT component,id,status,error_code,occurred_at FROM (
                SELECT 'source' AS component,id,status,error_code,updated_at AS occurred_at
                FROM source_jobs WHERE error_code IS NOT NULL
                UNION ALL SELECT 'portable',id,status,error_code,updated_at
                FROM portable_jobs WHERE error_code IS NOT NULL
                UNION ALL SELECT 'index',id,status,error_code,updated_at
                FROM index_rebuild_jobs WHERE error_code IS NOT NULL
                UNION ALL SELECT 'answer',id,status,error_code,updated_at
                FROM answers WHERE error_code IS NOT NULL
             ) ORDER BY julianday(occurred_at) DESC,component,id LIMIT ?1",
        )?;
        let mut rows = statement.query([limit])?;
        let mut errors = Vec::new();
        while let Some(row) = rows.next()? {
            let raw_id: String = row.get(1)?;
            let raw_time: String = row.get(4)?;
            let (Ok(id), Ok(time)) = (
                uuid::Uuid::parse_str(&raw_id),
                chrono::DateTime::parse_from_rfc3339(&raw_time),
            ) else {
                continue;
            };
            let status: String = row.get(2)?;
            let stage = match status.as_str() {
                "queued" | "running" | "succeeded" | "completed" | "failed" | "canceled"
                | "timed_out" | "no_evidence" | "invalid_citation" | "evidence_changed"
                | "model_unconfigured" => status,
                _ => "unknown_state".into(),
            };
            let code: String = row.get(3)?;
            errors.push(crate::DiagnosticError {
                component: row.get(0)?,
                id: id.to_string(),
                stage,
                code: diagnostic_code(&code).into(),
                occurred_at: time.with_timezone(&chrono::Utc).to_rfc3339(),
            });
        }
        Ok(errors)
    }

    /// Reads only fixed classification columns and numeric inventory aggregates.
    /// This does not run integrity_check, open blob files, or inspect user content.
    pub fn workspace_statistics(&self) -> Result<WorkspaceStatistics> {
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let db = &self.connection;
        let schema: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;

        let result = WorkspaceStatistics {
            schema_version: u32::try_from(schema).map_err(|_| Error::IntegrityFailure)?,
            objects: grouped(
                db,
                "SELECT kind,count(*) FROM objects GROUP BY kind",
                &["note", "collection", "source", "annotation"],
            )?,
            note_revisions: count(db, "SELECT count(*) FROM note_revisions")?,
            blobs: count(db, "SELECT count(*) FROM blobs")?,
            // SQLite SUM uses checked integer accumulation; overflow propagates as
            // an error rather than being coerced through floating-point TOTAL().
            blob_bytes: count(db, "SELECT coalesce(sum(size),0) FROM blobs")?,
            sources_by_format: {
                let formats = SourceFormat::ALL.map(SourceFormat::stored);
                grouped(
                    db,
                    "SELECT v.format,count(*) FROM sources s JOIN source_versions v ON v.id=s.current_version_id GROUP BY v.format",
                    &formats,
                )?
            },
            source_jobs: grouped(
                db,
                "SELECT status,count(*) FROM source_jobs GROUP BY status",
                &["queued", "running", "succeeded", "failed", "canceled"],
            )?,
            portable_jobs: grouped(
                db,
                "SELECT status,count(*) FROM portable_jobs GROUP BY status",
                &["queued", "running", "succeeded", "failed", "canceled"],
            )?,
            index_generations: grouped(
                db,
                "SELECT status,count(*) FROM index_generations GROUP BY status",
                &["running", "ready", "failed", "canceled"],
            )?,
            // Scoped to the active profile so the totals match what search reads.
            indexed_objects: count(
                db,
                "SELECT count(DISTINCT a.object_id) FROM active_index_generations a WHERE a.profile_id=(SELECT active_profile_id FROM search_settings WHERE singleton=1)",
            )?,
            index_chunks: count(
                db,
                "SELECT count(*) FROM index_chunks c WHERE EXISTS(SELECT 1 FROM active_index_generations a JOIN index_generations g ON g.id=a.generation_id WHERE a.generation_id=c.generation_id AND g.status='ready' AND a.profile_id=(SELECT active_profile_id FROM search_settings WHERE singleton=1))",
            )?,
            index_rebuild_jobs: grouped(
                db,
                "SELECT status,count(*) FROM index_rebuild_jobs GROUP BY status",
                &["queued", "running", "completed", "canceled", "failed"],
            )?,
            answers: grouped(
                db,
                "SELECT status,count(*) FROM answers GROUP BY status",
                &[
                    "queued",
                    "running",
                    "completed",
                    "canceled",
                    "timed_out",
                    "no_evidence",
                    "invalid_citation",
                    "evidence_changed",
                    "failed",
                    "model_unconfigured",
                ],
            )?,
            review_entries: grouped(
                db,
                "SELECT state,count(*) FROM review_entries GROUP BY state",
                &["pending", "completed", "removed"],
            )?,
            integrity_check: IntegrityCheckState::NotRun,
            retrieval_latency_24h: crate::metrics::retrieval_latency_stats(db)?,
            model_usage_30d: crate::metrics::model_usage_stats(db)?,
        };
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(result)
    }
}
