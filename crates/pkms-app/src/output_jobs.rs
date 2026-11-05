//! Workspace jobs own durable state; local output grants stay in app-state.
use crate::{
    AppError, AppService, GrantPurpose, Result, WorkspaceIdentity, backup::file_hash, path_text,
    physical_identity, uuid,
};
use pkms_core::{JobAccepted, JobStatus, PortableJobInput, PortableJobResult};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputCheckpoint {
    job_id: String,
    destination: String,
    parent_identity: String,
    file_identity: String,
    sha256: String,
    result: PortableJobResult,
}

impl AppService {
    /// Reveal only the directory of a completed output authorized by this local
    /// workspace instance. Callers never provide a filesystem path. The durable
    /// publication checkpoint remains usable after the original grant expires.
    pub fn published_output_directory(
        &self,
        identity: &WorkspaceIdentity,
        job_id: &str,
    ) -> Result<PathBuf> {
        uuid(job_id)?;
        if self.library(identity)?.portable_job(job_id)?.status != JobStatus::Succeeded {
            return Err(AppError::InputUnavailable);
        }
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT checkpoint_json FROM workspace_job_outputs
                 WHERE local_instance_id=?1 AND workspace_id=?2 AND job_id=?3",
                params![identity.local_instance_id, identity.workspace_id, job_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let saved: OutputCheckpoint =
            serde_json::from_str(&json.ok_or(AppError::InputUnavailable)?)?;
        let path = Path::new(&saved.destination);
        let parent = path.parent().ok_or(AppError::InputUnavailable)?;
        let metadata = std::fs::symlink_metadata(path)?;
        if saved.job_id != job_id
            || !path.is_absolute()
            || !metadata.is_file()
            || metadata.file_type().is_symlink()
            || physical_identity(path)? != saved.file_identity
            || physical_identity(parent)? != saved.parent_identity
        {
            return Err(AppError::InputUnavailable);
        }
        // Showing a folder does not execute or verify the output. In-place user
        // edits do not require hashing an arbitrarily large completed backup.
        Ok(parent.to_path_buf())
    }

    pub fn accept_output_job(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        token: &str,
        input: PortableJobInput,
    ) -> Result<JobAccepted> {
        self.library(identity)?;
        uuid(command_id)?;
        uuid(token)?;
        let input_json = serde_json::to_string(&input)?;
        let binding:Option<(String,String)>=self.connection.query_row("SELECT token,input_json FROM workspace_job_outputs WHERE local_instance_id=?1 AND workspace_id=?2 AND command_id=?3",params![identity.local_instance_id,identity.workspace_id,command_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((expected_token, expected_json)) = binding {
            if expected_token != token || expected_json != input_json {
                return Err(AppError::IdempotencyMismatch);
            }
        } else {
            self.resolve_grant(token, &GrantPurpose::ExportFile)?;
            self.connection.execute("INSERT INTO workspace_job_outputs(local_instance_id,workspace_id,command_id,token,input_json) VALUES(?1,?2,?3,?4,?5)",params![identity.local_instance_id,identity.workspace_id,command_id,token,input_json])?;
        }
        let accepted = self
            .library_mut(identity)?
            .accept_portable_job(command_id, input, token)?;
        self.connection.execute("UPDATE workspace_job_outputs SET job_id=?4 WHERE local_instance_id=?1 AND workspace_id=?2 AND command_id=?3",params![identity.local_instance_id,identity.workspace_id,command_id,accepted.job_id])?;
        Ok(accepted)
    }

    /// Runs under the adapter's storage worker and exclusive workspace lease.
    /// Poll/cancel commands use AppOperationControl's independent connection.
    pub fn run_pending_portable_jobs(&mut self) -> Result<()> {
        let Some(active) = self.active_workspace() else {
            return Ok(());
        };
        let identity = active.identity;
        self.library(&identity)?.recover_portable_jobs()?;
        // Finish acceptance if the process exited between the core commit and
        // app-local binding update. Replaying uses the original request/token.
        let missing=self.connection.prepare("SELECT command_id,token,input_json FROM workspace_job_outputs WHERE local_instance_id=?1 AND workspace_id=?2 AND job_id IS NULL")?.query_map(params![identity.local_instance_id,identity.workspace_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (command, token, input) in missing {
            let input = serde_json::from_str(&input)?;
            // A binding can precede a deterministic command rejection. Such a
            // rejection must not prevent unrelated durable jobs from running.
            match self.accept_output_job(&identity, &command, &token, input) {
                Ok(_) => {}
                Err(AppError::Core(
                    pkms_core::Error::IdempotencyMismatch | pkms_core::Error::NotFound,
                )) => {}
                Err(error) => return Err(error),
            }
        }
        let jobs = self.library(&identity)?.pending_portable_jobs()?;
        for id in jobs {
            let Some(lease) = self.library(&identity)?.claim_portable_job(&id)? else {
                continue;
            };
            if let Err(error) = self.execute_output_job(&identity, &id, &lease) {
                // Publication can precede a lost result commit. Leave the job
                // running for lease recovery; never overwrite or publish twice.
                if self.output_was_published(&identity, &id)? {
                    return Err(error);
                }
                // The task keeps its own generic `job_failed` code, but every
                // classification above it is shared with the other entry points.
                let code = crate::error_codes::error_code(
                    &error,
                    crate::error_codes::ErrorDomain::PortableJob,
                );
                self.library(&identity)?
                    .finish_portable_job(&id, &lease, None, Some(code))?;
            }
        }
        Ok(())
    }

    fn execute_output_job(
        &mut self,
        identity: &WorkspaceIdentity,
        id: &str,
        lease: &str,
    ) -> Result<()> {
        let (token,json):(String,Option<String>)=self.connection.query_row("SELECT token,checkpoint_json FROM workspace_job_outputs WHERE local_instance_id=?1 AND workspace_id=?2 AND job_id=?3",params![identity.local_instance_id,identity.workspace_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(AppError::InputUnavailable)?;
        if let Some(json) = json {
            let saved: OutputCheckpoint = serde_json::from_str(&json)?;
            if Path::new(&saved.destination).exists() {
                validate_output(&saved, id)?;
                self.library(identity)?.finish_portable_job(
                    id,
                    lease,
                    Some(&saved.result),
                    None,
                )?;
                return Ok(());
            }
        }
        let destination = self.resolve_grant(&token, &GrantPurpose::ExportFile)?;
        let library = self.library(identity)?;
        let job = library.portable_job(id)?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut previous = String::new();
        let mut check = |stage: &str| {
            #[cfg(test)]
            crate::background::observe_stage(stage);
            if stage == "copy_bytes" {
                if last.elapsed() > Duration::from_millis(100) {
                    library.portable_job_checkpoint(id, lease, &previous)?;
                    last = Instant::now();
                }
                return Ok(());
            }
            if last.elapsed() > Duration::from_millis(100) || stage != previous {
                library.portable_job_checkpoint(id, lease, stage)?;
                last = Instant::now();
                previous = stage.into();
            }
            Ok(())
        };
        let ready = match job.input {
            PortableJobInput::ExportDiagnostics { snapshot, .. } => {
                pkms_core::prepare_diagnostics_archive(
                    &destination,
                    crate::diagnostics::snapshot_files(&snapshot)?,
                    &mut check,
                )?
                .map_report(PortableJobResult::Diagnostics)
            }
            PortableJobInput::BackupCreate => library
                .prepare_backup(&destination, &mut check)?
                .map_report(|report| PortableJobResult::Backup(Box::new(report))),
            PortableJobInput::ExportNote {
                note_id,
                revision_id,
                format,
                citation_style_id,
            } => library
                .prepare_note_export_with_citations(
                    &note_id,
                    &revision_id,
                    &destination,
                    format,
                    citation_style_id.as_deref(),
                    &mut check,
                )?
                .map_report(PortableJobResult::Export),
            PortableJobInput::ExportSource { version_id, .. } => library
                .prepare_source_export(&version_id, &destination, &mut check)?
                .map_report(PortableJobResult::SourceExport),
        };
        self.resolve_grant(&token, &GrantPurpose::ExportFile)?;
        let saved = OutputCheckpoint {
            job_id: id.into(),
            destination: path_text(&destination)?,
            parent_identity: physical_identity(
                destination.parent().ok_or(AppError::InputUnavailable)?,
            )?,
            file_identity: physical_identity(ready.artifact_path())?,
            sha256: file_hash(ready.artifact_path())?,
            result: ready.report.clone(),
        };
        self.connection.execute("UPDATE workspace_job_outputs SET checkpoint_json=?4 WHERE local_instance_id=?1 AND workspace_id=?2 AND job_id=?3",params![identity.local_instance_id,identity.workspace_id,id,serde_json::to_string(&saved)?])?;
        self.library_mut(identity)?
            .begin_portable_publication(id, lease)?;
        let result = ready.publish()?;
        self.library(identity)?
            .finish_portable_job(id, lease, Some(&result), None)?;
        Ok(())
    }

    fn output_was_published(&self, identity: &WorkspaceIdentity, id: &str) -> Result<bool> {
        let json:Option<Option<String>>=self.connection.query_row("SELECT checkpoint_json FROM workspace_job_outputs WHERE local_instance_id=?1 AND workspace_id=?2 AND job_id=?3",params![identity.local_instance_id,identity.workspace_id,id],|r|r.get(0)).optional()?;
        let Some(Some(json)) = json else {
            return Ok(false);
        };
        let saved: OutputCheckpoint = serde_json::from_str(&json)?;
        Ok(validate_output(&saved, id).is_ok())
    }
}

fn validate_output(saved: &OutputCheckpoint, id: &str) -> Result<()> {
    let path = Path::new(&saved.destination);
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || saved.job_id != id
        || physical_identity(path)? != saved.file_identity
        || physical_identity(path.parent().ok_or(AppError::InputUnavailable)?)?
            != saved.parent_identity
        || file_hash(path)? != saved.sha256
    {
        return Err(AppError::InputUnavailable);
    }
    Ok(())
}
