//! The two current portable-output jobs; no speculative worker framework.
use crate::{
    BackupReport, Error, ExportFormat, Library, MetadataReceipt, NoteExportReport, Result,
    mutations,
};
use crate::{collections, document::valid_uuid};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PortableJobInput {
    BackupCreate,
    ExportDiagnostics {
        categories: Vec<crate::DiagnosticCategory>,
        snapshot: Box<crate::DiagnosticSnapshot>,
    },
    ExportNote {
        note_id: String,
        revision_id: String,
        format: ExportFormat,
        #[serde(default)]
        citation_style_id: Option<String>,
    },
    /// Exports one immutable original with the document that owns it, so a
    /// stored source can leave the library without a full backup.
    ExportSource {
        source_id: String,
        version_id: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", content = "report", rename_all = "snake_case")]
pub enum PortableJobResult {
    Backup(Box<BackupReport>),
    Export(NoteExportReport),
    SourceExport(crate::SourceExportReport),
    Diagnostics(crate::DiagnosticExportReport),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct PortableJob {
    pub id: String,
    pub input: PortableJobInput,
    pub status: JobStatus,
    pub stage: String,
    pub cancel_requested: bool,
    pub result: Option<PortableJobResult>,
    pub error_code: Option<String>,
    pub updated_at: String,
}
/// One page of the portable job list. The cursor is opaque and bound to the
/// workspace *and* the physical database that produced it, so a page cannot be
/// replayed into a different library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct PortableJobPage {
    pub items: Vec<PortableJob>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct JobAccepted {
    pub job_id: String,
    pub receipt: MetadataReceipt,
}
impl mutations::RecordedResult for JobAccepted {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}

impl Library {
    /// output_binding is an opaque app-issued token. Only its payload hash is
    /// persisted in the workspace; paths and the token itself remain app-local.
    pub fn accept_portable_job(
        &mut self,
        command_id: &str,
        input: PortableJobInput,
        output_binding: &str,
    ) -> Result<JobAccepted> {
        crate::document::valid_uuid(output_binding)?;
        let hash = mutations::request_hash(command_id, "jobs.portable", &(&input, output_binding))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(replay) = mutations::replay(&tx, command_id, &hash, "jobs.portable")? {
            return Ok(replay);
        }
        if let PortableJobInput::ExportDiagnostics {
            categories,
            snapshot,
        } = &input
        {
            if categories.is_empty()
                || categories.len() > 3
                || categories
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != categories.len()
            {
                return Err(Error::InvalidInput("invalid diagnostic categories"));
            }
            if categories.contains(&crate::DiagnosticCategory::Versions)
                != snapshot.versions.is_some()
                || categories.contains(&crate::DiagnosticCategory::Statistics)
                    != snapshot.statistics.is_some()
                || categories.contains(&crate::DiagnosticCategory::Errors)
                    != snapshot.errors.is_some()
                || serde_json::to_vec(snapshot)?.len() > 3 * 1024 * 1024
            {
                return Err(Error::InvalidInput(
                    "diagnostic snapshot categories mismatch",
                ));
            }
        }
        if let PortableJobInput::ExportNote {
            note_id,
            revision_id,
            ..
        } = &input
        {
            crate::document::valid_uuid(note_id)?;
            crate::document::valid_uuid(revision_id)?;
            crate::notes::revision(&tx, note_id, revision_id)?;
        }
        if let PortableJobInput::ExportSource {
            source_id,
            version_id,
        } = &input
        {
            crate::document::valid_uuid(source_id)?;
            crate::document::valid_uuid(version_id)?;
            if crate::sources::read_version(&tx, version_id)?.source_id != *source_id {
                return Err(Error::InvalidInput(
                    "exported version does not belong to the source",
                ));
            }
        }
        let job_id = uuid::Uuid::new_v4().to_string();
        let receipt = mutations::next_receipt(&tx, command_id)?;
        let accepted = JobAccepted {
            job_id: job_id.clone(),
            receipt,
        };
        tx.execute("INSERT INTO portable_jobs(id,input_json,status,stage,created_at,updated_at) VALUES(?1,?2,'queued','queued',?3,?3)",params![job_id,serde_json::to_string(&input)?,accepted.receipt.committed_at])?;
        mutations::store_receipt(
            &tx,
            &hash,
            "jobs.portable",
            &job_id,
            &accepted.receipt,
            &accepted,
        )?;
        tx.commit()?;
        Ok(accepted)
    }
    pub fn portable_job(&self, id: &str) -> Result<PortableJob> {
        read_job(&self.connection, id)
    }
    pub fn portable_jobs(&self, limit: u32) -> Result<Vec<PortableJob>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("job limit must be 1-100"));
        }
        self.connection
            .prepare("SELECT id FROM portable_jobs ORDER BY created_at DESC,id LIMIT ?1")?
            .query_map([limit], |r| r.get::<_, String>(0))?
            .map(|id| self.portable_job(&id?))
            .collect()
    }
    /// One page of jobs, newest first. Ordering is `created_at DESC, id DESC`, so
    /// the cursor is a total order and pages cannot repeat or skip a row while
    /// jobs keep arriving at the head of the list.
    pub fn portable_jobs_page(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<PortableJobPage> {
        collections::validate_limit(limit)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let workspace: String =
            snapshot.query_row("SELECT workspace_id FROM workspace_meta", [], |r| r.get(0))?;
        // Tag 3 keeps this namespace apart from collections (1) and source jobs
        // (2); the physical path keeps two libraries with equal workspace ids
        // apart, the same rule `collections::decode_cursor` was fixed to carry.
        let scope = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                3,
                self.database_path.to_string_lossy(),
                workspace
            ))?)
        );
        let cursor: Option<(String, String, String)> =
            collections::decode_cursor(cursor.as_deref())?;
        if let Some((seen_scope, time, job_id)) = &cursor {
            if seen_scope != &scope || chrono::DateTime::parse_from_rfc3339(time).is_err() {
                return Err(Error::InvalidInput(
                    "portable jobs cursor does not match workspace",
                ));
            }
            valid_uuid(job_id)?;
        }
        let mut rows = snapshot
            .prepare(
                "SELECT id,created_at FROM portable_jobs \
                 WHERE (?1 IS NULL OR created_at<?1 OR (created_at=?1 AND id<?2)) \
                 ORDER BY created_at DESC,id DESC LIMIT ?3",
            )?
            .query_map(
                params![
                    cursor.as_ref().map(|value| &value.1),
                    cursor.as_ref().map(|value| &value.2),
                    limit + 1
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            let last = rows.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&(&scope, &last.1, &last.0))?)
        } else {
            None
        };
        let items = rows
            .iter()
            .map(|(job_id, _)| read_job(&snapshot, job_id))
            .collect::<Result<Vec<_>>>()?;
        Ok(PortableJobPage { items, next_cursor })
    }
    pub fn pending_portable_jobs(&self) -> Result<Vec<String>> {
        Ok(self
            .connection
            .prepare("SELECT id FROM portable_jobs WHERE status='queued' ORDER BY created_at,id")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    /// Only the holder of the application's exclusive WorkspaceLease calls this
    /// after a worker/process has gone away. A fresh lease rejects late results.
    pub fn recover_portable_jobs(&self) -> Result<()> {
        self.connection.execute("UPDATE portable_jobs SET status=CASE WHEN cancel_requested=1 THEN 'canceled' ELSE 'queued' END,stage=CASE WHEN stage='publishing' THEN stage ELSE 'resume_pending' END,lease_id=NULL WHERE status='running'",[])?;
        Ok(())
    }
    pub fn claim_portable_job(&self, id: &str) -> Result<Option<String>> {
        let lease = uuid::Uuid::new_v4().to_string();
        let changed=self.connection.execute("UPDATE portable_jobs SET status='running',stage='preparing',lease_id=?2,updated_at=?3 WHERE id=?1 AND status='queued' AND cancel_requested=0",params![id,lease,chrono::Utc::now().to_rfc3339()])?;
        Ok((changed == 1).then_some(lease))
    }
    pub fn portable_job_checkpoint(&self, id: &str, lease: &str, stage: &str) -> Result<()> {
        let (current, cancel, status): (Option<String>, bool, String) = self.connection.query_row(
            "SELECT lease_id,cancel_requested,status FROM portable_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if current.as_deref() != Some(lease) || status != "running" {
            return Err(Error::InvalidInput("job lease expired"));
        }
        if cancel {
            return Err(Error::Io(std::io::ErrorKind::Interrupted.into()));
        }
        self.connection.execute(
            "UPDATE portable_jobs SET stage=?3,updated_at=?4 WHERE id=?1 AND lease_id=?2",
            params![id, lease, stage, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
    pub fn begin_portable_publication(&mut self, id: &str, lease: &str) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE portable_jobs SET stage='publishing' WHERE id=?1 AND lease_id=?2 AND status='running' AND cancel_requested=0",params![id,lease])?;
        if changed != 1 {
            return Err(Error::Io(std::io::ErrorKind::Interrupted.into()));
        }
        tx.commit()?;
        Ok(())
    }
    pub fn finish_portable_job(
        &self,
        id: &str,
        lease: &str,
        result: Option<&PortableJobResult>,
        error_code: Option<&str>,
    ) -> Result<PortableJob> {
        let status = if result.is_some() {
            "succeeded"
        } else if error_code == Some("canceled") {
            "canceled"
        } else {
            "failed"
        };
        let changed=self.connection.execute("UPDATE portable_jobs SET status=?3,stage=?3,result_json=?4,error_code=?5,updated_at=?6 WHERE id=?1 AND lease_id=?2 AND status='running'",params![id,lease,status,result.map(serde_json::to_string).transpose()?,error_code,chrono::Utc::now().to_rfc3339()])?;
        if changed != 1 {
            return Err(Error::InvalidInput("job lease expired"));
        }
        self.portable_job(id)
    }
    pub fn cancel_portable_job(&self, id: &str) -> Result<PortableJob> {
        self.portable_job(id)?;
        self.connection.execute("UPDATE portable_jobs SET cancel_requested=1,status=CASE WHEN status='queued' THEN 'canceled' ELSE status END,stage=CASE WHEN status='queued' THEN 'canceled' ELSE stage END,updated_at=?2 WHERE id=?1 AND status IN ('queued','running') AND stage!='publishing'",params![id,chrono::Utc::now().to_rfc3339()])?;
        self.portable_job(id)
    }
}
pub(crate) fn read_job(db: &Connection, id: &str) -> Result<PortableJob> {
    crate::document::valid_uuid(id)?;
    let row=db.query_row("SELECT input_json,status,stage,cancel_requested,result_json,error_code,updated_at FROM portable_jobs WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(PortableJob {
        id: id.into(),
        input: serde_json::from_str(&row.0)?,
        status: serde_json::from_value(serde_json::Value::String(row.1))?,
        stage: row.2,
        cancel_requested: row.3,
        result: row.4.map(|json| serde_json::from_str(&json)).transpose()?,
        error_code: row.5,
        updated_at: row.6,
    })
}
