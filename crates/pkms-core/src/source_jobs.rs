use crate::{
    Error, ImportSource, JobAccepted, Library, Result, SourceJob, SourceJobEvent, SourceJobLease,
    document::valid_uuid, mutations,
};
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

pub(crate) fn validate_tag_names(names: &[String]) -> Result<()> {
    if names.len() > 100 {
        return Err(Error::InvalidInput("at most 100 tags may be assigned"));
    }
    for name in names {
        crate::organization::normalize_name(name)?;
    }
    Ok(())
}

fn input_json(input: &ImportSource, tag_names: &[String]) -> Result<String> {
    let mut value = serde_json::to_value(input)?;
    if !tag_names.is_empty() {
        value
            .as_object_mut()
            .ok_or(Error::IntegrityFailure)?
            .insert("tag_names".into(), serde_json::to_value(tag_names)?);
    }
    Ok(serde_json::to_string(&value)?)
}

fn read_input(raw: &str) -> Result<(ImportSource, Vec<String>)> {
    let mut value: serde_json::Value = serde_json::from_str(raw)?;
    let tag_names = match value
        .as_object_mut()
        .ok_or(Error::IntegrityFailure)?
        .remove("tag_names")
    {
        Some(value) => serde_json::from_value(value)?,
        None => Vec::new(),
    };
    let input = serde_json::from_value(value)?;
    Ok((input, tag_names))
}

fn import_hash(command_id: &str, input: &ImportSource, tag_names: &[String]) -> Result<String> {
    if tag_names.is_empty() {
        mutations::request_hash(command_id, "sources.import", input)
    } else {
        mutations::request_hash(command_id, "sources.import", &(input, tag_names))
    }
}

pub(crate) fn read_job(db: &Connection, id: &str) -> Result<SourceJob> {
    valid_uuid(id)?;
    let row=db.query_row("SELECT input_json,status,stage,attempt,cancel_requested,source_id,source_version_id,error_code,updated_at,config_revision FROM source_jobs WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional()?.ok_or(Error::NotFound)?;
    let (input, tag_names) = read_input(&row.0)?;
    Ok(SourceJob {
        id: id.into(),
        input,
        tag_names,
        status: serde_json::from_value(serde_json::Value::String(row.1))?,
        stage: row.2,
        attempt: row.3,
        config_revision: row.9,
        cancel_requested: row.4,
        source_id: row.5,
        source_version_id: row.6,
        error_code: row.7,
        updated_at: row.8,
    })
}
pub(crate) fn event(db: &Connection, id: &str) -> Result<()> {
    db.execute("INSERT INTO source_job_events(job_id,seq,status,stage,created_at,attempt) SELECT id,coalesce((SELECT max(seq) FROM source_job_events WHERE job_id=?1),0)+1,status,stage,updated_at,attempt FROM source_jobs WHERE id=?1",[id])?;
    Ok(())
}
pub(crate) fn check_lease(db: &Connection, id: &str, lease: &str) -> Result<SourceJob> {
    valid_uuid(lease)?;
    let job = read_job(db, id)?;
    let valid:bool=db.query_row("SELECT status='running' AND lease_token=?2 AND lease_expires_at>?3 AND cancel_requested=0 FROM source_jobs WHERE id=?1",params![id,lease,Utc::now().to_rfc3339()],|r|r.get(0))?;
    if !valid {
        return Err(Error::InvalidInput("source job lease expired or canceled"));
    }
    Ok(job)
}
pub(crate) fn validate_original_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 300 || name.chars().any(char::is_control) {
        return Err(Error::InvalidInput("invalid original filename"));
    }
    Ok(())
}
pub(crate) fn validate_web_url(url: &str) -> Result<()> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    if url.len() > 8192
        || url.chars().any(char::is_whitespace)
        || url.chars().any(char::is_control)
        || rest.is_none_or(|rest| {
            rest.is_empty()
                || rest.starts_with('/')
                || rest.starts_with('?')
                || rest.starts_with('#')
        })
    {
        return Err(Error::InvalidInput("source URL must be HTTP(S)"));
    }
    Ok(())
}
fn webpage_input(command_id: &str, request: &crate::ImportWebpage) -> Result<ImportSource> {
    validate_web_url(&request.url)?;
    Ok(ImportSource {
        input_token: command_id.into(),
        original_name: "webpage.html".into(),
        format: crate::SourceFormat::Html,
        origin_url: Some(request.url.clone()),
        collection_id: request.collection_id.clone(),
        add_version_to: request.add_version_to.clone(),
        base_row_version: request.base_row_version,
    })
}
impl Library {
    pub fn replay_webpage_import(
        &self,
        command_id: &str,
        request: &crate::ImportWebpage,
    ) -> Result<Option<JobAccepted>> {
        self.replay_webpage_import_with_tags(command_id, request, &[])
    }
    pub fn replay_webpage_import_with_tags(
        &self,
        command_id: &str,
        request: &crate::ImportWebpage,
        tag_names: &[String],
    ) -> Result<Option<JobAccepted>> {
        self.replay_source_import_with_tags(
            command_id,
            &webpage_input(command_id, request)?,
            tag_names,
        )
    }
    pub fn accept_webpage_import(
        &mut self,
        command_id: &str,
        request: crate::ImportWebpage,
    ) -> Result<JobAccepted> {
        self.accept_webpage_import_with_tags(command_id, request, Vec::new())
    }
    pub fn accept_webpage_import_with_tags(
        &mut self,
        command_id: &str,
        request: crate::ImportWebpage,
        tag_names: Vec<String>,
    ) -> Result<JobAccepted> {
        self.accept_source_import_with_tags(
            command_id,
            webpage_input(command_id, &request)?,
            tag_names,
        )
    }
    pub fn replay_source_import(
        &self,
        command_id: &str,
        input: &ImportSource,
    ) -> Result<Option<JobAccepted>> {
        self.replay_source_import_with_tags(command_id, input, &[])
    }
    pub fn replay_source_import_with_tags(
        &self,
        command_id: &str,
        input: &ImportSource,
        tag_names: &[String],
    ) -> Result<Option<JobAccepted>> {
        validate_original_name(&input.original_name)?;
        validate_tag_names(tag_names)?;
        let hash = import_hash(command_id, input, tag_names)?;
        mutations::replay(&self.connection, command_id, &hash, "sources.import")
    }
    pub fn accept_source_import(
        &mut self,
        command_id: &str,
        input: ImportSource,
    ) -> Result<JobAccepted> {
        self.accept_source_import_with_tags(command_id, input, Vec::new())
    }
    pub fn accept_source_import_with_tags(
        &mut self,
        command_id: &str,
        input: ImportSource,
        tag_names: Vec<String>,
    ) -> Result<JobAccepted> {
        valid_uuid(&input.input_token)?;
        validate_original_name(&input.original_name)?;
        validate_tag_names(&tag_names)?;
        if let Some(url) = &input.origin_url
            && (url.len() > 8192
                || url.chars().any(char::is_control)
                || !(url.starts_with("https://") || url.starts_with("http://")))
        {
            return Err(Error::InvalidInput("source URL must be HTTP(S)"));
        }
        if input.add_version_to.is_some() != input.base_row_version.is_some() {
            return Err(Error::InvalidInput(
                "adding a version requires a source base version",
            ));
        }
        let hash = import_hash(command_id, &input, &tag_names)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "sources.import")? {
            return Ok(old);
        }
        if let Some(id) = &input.collection_id {
            valid_uuid(id)?;
            let available:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM collections c JOIN objects o ON o.id=c.id WHERE c.id=?1 AND o.trashed=0)",[id],|r|r.get(0))?;
            if !available {
                return Err(Error::NotFound);
            }
        }
        if let Some(id) = &input.add_version_to {
            let source = crate::sources::read_source(&tx, id)?;
            mutations::check_version(id, input.base_row_version.unwrap_or(0), source.row_version)?;
            if source.trashed {
                return Err(Error::Trashed);
            }
        }
        let result = JobAccepted {
            job_id: uuid::Uuid::new_v4().to_string(),
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        tx.execute("INSERT INTO source_jobs(id,input_json,status,stage,created_at,updated_at) VALUES(?1,?2,'queued','capture_pending',?3,?3)",params![result.job_id,input_json(&input, &tag_names)?,result.receipt.committed_at])?;
        event(&tx, &result.job_id)?;
        mutations::store_receipt(
            &tx,
            &hash,
            "sources.import",
            &result.job_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn source_job(&self, id: &str) -> Result<SourceJob> {
        read_job(&self.connection, id)
    }
    pub fn retry_source_job(&mut self, command_id: &str, id: &str) -> Result<JobAccepted> {
        valid_uuid(id)?;
        let hash = mutations::request_hash(command_id, "sources.retry", &id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = mutations::replay(&tx, command_id, &hash, "sources.retry")? {
            return Ok(old);
        }
        let old = read_job(&tx, id)?;
        if !matches!(
            old.status,
            crate::JobStatus::Failed | crate::JobStatus::Canceled
        ) {
            return Err(Error::InvalidInput(
                "only failed or canceled source jobs may be retried",
            ));
        }
        let result = JobAccepted {
            job_id: uuid::Uuid::new_v4().to_string(),
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        tx.execute("INSERT INTO source_jobs(id,input_json,status,stage,source_id,source_version_id,created_at,updated_at) VALUES(?1,?2,'queued',?3,?4,?5,?6,?6)",params![result.job_id,input_json(&old.input, &old.tag_names)?,if old.source_version_id.is_some(){"parse_pending"}else{"capture_pending"},old.source_id,old.source_version_id,result.receipt.committed_at])?;
        event(&tx, &result.job_id)?;
        mutations::store_receipt(
            &tx,
            &hash,
            "sources.retry",
            &result.job_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn source_jobs(&self, limit: u32) -> Result<Vec<SourceJob>> {
        crate::collections::validate_limit(limit)?;
        self.connection
            .prepare("SELECT id FROM source_jobs ORDER BY created_at DESC,id LIMIT ?1")?
            .query_map([limit], |r| r.get::<_, String>(0))?
            .map(|id| self.source_job(&id?))
            .collect()
    }
    pub fn source_jobs_page(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<crate::SourceJobPage> {
        crate::collections::validate_limit(limit)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let workspace: String =
            snapshot.query_row("SELECT workspace_id FROM workspace_meta", [], |r| r.get(0))?;
        let workspace = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                1,
                self.database_path.to_string_lossy(),
                workspace
            ))?)
        );
        let cursor: Option<(String, String, String)> =
            crate::collections::decode_cursor(cursor.as_deref())?;
        if let Some((scope, time, id)) = &cursor {
            if scope != &workspace || chrono::DateTime::parse_from_rfc3339(time).is_err() {
                return Err(Error::InvalidInput(
                    "source jobs cursor does not match workspace",
                ));
            }
            valid_uuid(id)?;
        }
        let mut rows=snapshot.prepare("SELECT id,created_at FROM source_jobs WHERE (?1 IS NULL OR created_at<?1 OR (created_at=?1 AND id<?2)) ORDER BY created_at DESC,id DESC LIMIT ?3")?.query_map(params![cursor.as_ref().map(|v|&v.1),cursor.as_ref().map(|v|&v.2),limit+1],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            let last = rows.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&(&workspace, &last.1, &last.0))?)
        } else {
            None
        };
        let items = rows
            .iter()
            .map(|(id, _)| read_job(&snapshot, id))
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::SourceJobPage { items, next_cursor })
    }
    pub fn pending_source_jobs(&self) -> Result<Vec<String>> {
        Ok(self
            .connection
            .prepare(
                "SELECT id FROM source_jobs WHERE status='queued' ORDER BY created_at,id LIMIT 100",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn claim_source_job(&mut self, id: &str) -> Result<Option<SourceJobLease>> {
        valid_uuid(id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now();
        let lease = uuid::Uuid::new_v4().to_string();
        let expires = (now + Duration::minutes(5)).to_rfc3339();
        let changed=tx.execute("UPDATE source_jobs SET status='running',stage=CASE WHEN source_version_id IS NULL THEN 'capturing' ELSE 'parsing' END,attempt=attempt+1,lease_token=?2,lease_expires_at=?3,updated_at=?4 WHERE id=?1 AND status='queued' AND attempt<3 AND cancel_requested=0",params![id,lease,expires,now.to_rfc3339()])?;
        if changed == 0 {
            return Ok(None);
        }
        let job = read_job(&tx, id)?;
        tx.execute("INSERT INTO source_job_attempts(job_id,attempt,lease_token,started_at) VALUES(?1,?2,?3,?4)",params![id,job.attempt,lease,now.to_rfc3339()])?;
        event(&tx, id)?;
        tx.commit()?;
        Ok(Some(SourceJobLease {
            job,
            lease_token: lease,
            expires_at: expires,
        }))
    }
    pub fn source_job_checkpoint(&mut self, id: &str, lease: &str, stage: &str) -> Result<()> {
        if !matches!(stage, "capturing" | "parsing" | "indexing") {
            return Err(Error::InvalidInput("invalid source job stage"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_lease(&tx, id, lease)?;
        tx.execute("UPDATE source_jobs SET stage=?3,lease_expires_at=?4,updated_at=?5 WHERE id=?1 AND lease_token=?2",params![id,lease,stage,(Utc::now()+Duration::minutes(5)).to_rfc3339(),Utc::now().to_rfc3339()])?;
        event(&tx, id)?;
        tx.commit()?;
        Ok(())
    }
    pub fn cancel_source_job(&mut self, id: &str) -> Result<SourceJob> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        read_job(&tx, id)?;
        let changed=tx.execute("UPDATE source_jobs SET cancel_requested=1,status='canceled',stage='canceled',lease_token=NULL,lease_expires_at=NULL,error_code='canceled',updated_at=?2 WHERE id=?1 AND status IN ('queued','running')",params![id,Utc::now().to_rfc3339()])?;
        if changed > 0 {
            tx.execute("UPDATE source_job_attempts SET finished_at=?2,error_code='canceled' WHERE job_id=?1 AND finished_at IS NULL",params![id,Utc::now().to_rfc3339()])?;
            event(&tx, id)?;
        }
        let job = read_job(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn fail_source_job(
        &mut self,
        id: &str,
        lease: &str,
        error_code: &str,
        retryable: bool,
    ) -> Result<SourceJob> {
        if error_code.is_empty()
            || error_code.len() > 100
            || !error_code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::InvalidInput("invalid source error code"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, lease)?;
        let status = if retryable && job.attempt < 3 {
            "queued"
        } else {
            "failed"
        };
        tx.execute("UPDATE source_jobs SET status=?2,stage=?2,lease_token=NULL,lease_expires_at=NULL,error_code=?3,updated_at=?4 WHERE id=?1",params![id,status,error_code,Utc::now().to_rfc3339()])?;
        tx.execute("UPDATE source_job_attempts SET finished_at=?3,error_code=?4 WHERE job_id=?1 AND lease_token=?2",params![id,lease,Utc::now().to_rfc3339(),error_code])?;
        event(&tx, id)?;
        let job = read_job(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }
    /// Exclusive application supervisor only: invalidate leases after its workers exit.
    pub fn recover_source_jobs(&mut self) -> Result<()> {
        self.recover_source_jobs_in(false)
    }
    pub fn recover_expired_source_jobs(&mut self) -> Result<()> {
        self.recover_source_jobs_in(true)
    }
    fn recover_source_jobs_in(&mut self, expired_only: bool) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids=tx.prepare("SELECT id FROM source_jobs WHERE status='running' AND (?1=0 OR lease_expires_at<=?2)")?.query_map(params![expired_only,Utc::now().to_rfc3339()],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for id in ids {
            tx.execute("UPDATE source_job_attempts SET finished_at=?2,error_code='worker_interrupted' WHERE job_id=?1 AND finished_at IS NULL",params![id,Utc::now().to_rfc3339()])?;
            tx.execute("UPDATE source_jobs SET status=CASE WHEN attempt<3 THEN 'queued' ELSE 'failed' END,stage='resume_pending',lease_token=NULL,lease_expires_at=NULL,error_code='worker_interrupted',updated_at=?2 WHERE id=?1",params![id,Utc::now().to_rfc3339()])?;
            event(&tx, &id)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn source_job_events(
        &self,
        id: &str,
        after_seq: u32,
        limit: u32,
    ) -> Result<Vec<SourceJobEvent>> {
        self.source_job(id)?;
        crate::collections::validate_limit(limit)?;
        let rows=self.connection.prepare("SELECT seq,status,stage,created_at,attempt FROM source_job_events WHERE job_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?.query_map(params![id,after_seq,limit],|r|Ok((r.get(0)?,r.get::<_,String>(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(seq, status, stage, created_at, attempt)| {
                Ok(SourceJobEvent {
                    attempt,
                    seq,
                    status: serde_json::from_value(serde_json::Value::String(status))?,
                    stage,
                    created_at,
                })
            })
            .collect()
    }
}
