//! App-level backup operations remain usable before any workspace is opened.
use crate::{
    AppError, AppService, BootstrapAccepted, GrantPurpose, OperationKind, Result,
    WorkspaceAvailability, WorkspaceEntry, WorkspaceIdentity, bootstrap::kind_key, path_text,
    physical_identity, storage_identity, uuid,
};
use chrono::Utc;
use pkms_core::{BackupInspection, BackupReport, PreparedRestore, WorkspaceLease};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(test)]
thread_local! { static CRASH_AFTER_PUBLICATION:std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RestoreCheckpoint {
    operation_id: String,
    instance_id: String,
    destination: String,
    parent_identity: String,
    directory_identity: String,
    database_hash: String,
    report: BackupReport,
}

impl AppService {
    pub fn inspect_selected_backup(&self, token: &str) -> Result<BackupInspection> {
        let path = self.resolve_grant(token, &GrantPurpose::BackupInput)?;
        let inspected = pkms_core::inspect_backup(path)?;
        self.resolve_grant(token, &GrantPurpose::BackupInput)?;
        Ok(inspected)
    }

    pub fn accept_backup_operation(
        &mut self,
        command_id: &str,
        kind: OperationKind,
        input_token: &str,
        output_token: Option<&str>,
    ) -> Result<BootstrapAccepted> {
        uuid(command_id)?;
        uuid(input_token)?;
        if !matches!(
            kind,
            OperationKind::VerifyBackup | OperationKind::RestoreBackup
        ) || (kind == OperationKind::RestoreBackup) != output_token.is_some()
        {
            return Err(AppError::InvalidInput("invalid backup operation"));
        }
        if let Some(token) = output_token {
            uuid(token)?;
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                1,
                kind_key(&kind),
                input_token,
                output_token
            ))?)
        );
        // Replays are checked before expired grants: committed acceptance is durable.
        let previous = self
            .connection
            .query_row(
                "SELECT payload_hash,operation_id FROM app_command_receipts WHERE command_id=?1",
                [command_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((expected, operation_id)) = previous {
            if expected != hash {
                return Err(AppError::IdempotencyMismatch);
            }
            return Ok(BootstrapAccepted {
                command_id: command_id.into(),
                operation_id,
                replayed: true,
            });
        }
        self.resolve_grant(input_token, &GrantPurpose::BackupInput)?;
        if let Some(token) = output_token {
            self.resolve_grant(token, &GrantPurpose::RestoreWorkspace)?;
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        let instance = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO app_operations(id,kind,token,output_token,status,stage,planned_instance_id,created_at,updated_at) VALUES(?1,?2,?3,?4,'queued','queued',?5,?6,?6)",params![operation_id,kind_key(&kind),input_token,output_token,instance,now])?;
        tx.execute(
            "INSERT INTO app_command_receipts VALUES(?1,?2,?3)",
            params![command_id, hash, operation_id],
        )?;
        tx.commit()?;
        Ok(BootstrapAccepted {
            command_id: command_id.into(),
            operation_id,
            replayed: false,
        })
    }

    pub(crate) fn execute_backup_operation(
        &mut self,
        id: &str,
        kind: &OperationKind,
    ) -> Result<()> {
        let (input,output,instance,checkpoint):(String,Option<String>,String,Option<String>)=self.connection.query_row("SELECT token,output_token,planned_instance_id,checkpoint_json FROM app_operations WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if *kind == OperationKind::RestoreBackup
            && let Some(json) = checkpoint
        {
            let saved: RestoreCheckpoint = serde_json::from_str(&json)?;
            if Path::new(&saved.destination).exists() {
                validate_published(&saved, id, &instance)?;
                let _lease = WorkspaceLease::acquire(&saved.destination)?;
                return self.finish_restore(&saved, None);
            }
            // No publication happened. Release the old finalization barrier
            // before repeating cancellable staging work under the same ID.
            self.connection.execute(
                "UPDATE app_operations SET checkpoint_json=NULL WHERE id=?1",
                [id],
            )?;
        }
        let input_path = self.resolve_grant(&input, &GrantPurpose::BackupInput)?;
        if *kind == OperationKind::VerifyBackup {
            let mut check = progress(&self.connection, id);
            let report = pkms_core::verify_backup(input_path, &mut check)?;
            drop(check);
            self.resolve_grant(&input, &GrantPurpose::BackupInput)?;
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check_cancel(&tx, id)?;
            tx.execute("UPDATE app_operations SET status='succeeded',stage='complete',report_json=?2,error_code=NULL,updated_at=?3 WHERE id=?1",params![id,serde_json::to_string(&report)?,Utc::now().to_rfc3339()])?;
            tx.commit()?;
            return Ok(());
        }
        let output_token = output.ok_or(AppError::InvalidInput("missing restore output"))?;
        let destination = self.resolve_grant(&output_token, &GrantPurpose::RestoreWorkspace)?;
        let mut check = progress(&self.connection, id);
        let ready = pkms_core::prepare_restore(input_path, &destination, &mut check)?;
        drop(check);
        self.resolve_grant(&input, &GrantPurpose::BackupInput)?;
        self.resolve_grant(&output_token, &GrantPurpose::RestoreWorkspace)?;
        let saved = RestoreCheckpoint {
            operation_id: id.into(),
            instance_id: instance,
            destination: path_text(&destination)?,
            parent_identity: physical_identity(
                destination.parent().ok_or(AppError::InputUnavailable)?,
            )?,
            directory_identity: physical_identity(ready.staging_path())?,
            database_hash: file_hash(&ready.staging_path().join("library.sqlite3"))?,
            report: ready.report.clone(),
        };
        let json = serde_json::to_vec(&saved)?;
        let mut marker = File::create_new(ready.staging_path().join(".pkms-restore.json"))?;
        marker.write_all(&json)?;
        marker.sync_all()?;
        drop(marker);
        self.connection.execute("UPDATE app_operations SET checkpoint_json=?2,stage='restore_ready',updated_at=?3 WHERE id=?1",params![id,String::from_utf8(json).map_err(|_|AppError::InvalidInput("invalid checkpoint"))?,Utc::now().to_rfc3339()])?;
        self.finish_restore(&saved, Some(ready))
    }

    fn finish_restore(
        &mut self,
        saved: &RestoreCheckpoint,
        ready: Option<PreparedRestore>,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(ready) = ready {
            check_cancel(&tx, &saved.operation_id)?;
            // Cancellation uses a separate SQLite connection. Hold only this
            // short publication/registration transaction, never parsing or copy.
            ready.publish()?;
            #[cfg(test)]
            if CRASH_AFTER_PUBLICATION.get() {
                std::process::exit(87);
            }
        }
        let destination = PathBuf::from(&saved.destination);
        let entry = WorkspaceEntry {
            identity: WorkspaceIdentity {
                local_instance_id: saved.instance_id.clone(),
                workspace_id: saved.report.manifest.workspace_id.clone(),
            },
            path: saved.destination.clone(),
            last_opened_at: Utc::now().to_rfc3339(),
            availability: WorkspaceAvailability::Available,
        };
        tx.execute("INSERT INTO workspace_catalog(local_instance_id,workspace_id,canonical_path,storage_identity,last_opened_at,show_in_recents) VALUES(?1,?2,?3,?4,?5,1)",params![entry.identity.local_instance_id,entry.identity.workspace_id,entry.path,storage_identity(&destination)?,entry.last_opened_at])?;
        tx.execute("UPDATE app_operations SET status='succeeded',stage='complete',result_json=?2,report_json=?3,error_code=NULL,updated_at=?4 WHERE id=?1",params![saved.operation_id,serde_json::to_string(&entry)?,serde_json::to_string(&saved.report)?,entry.last_opened_at])?;
        tx.commit()?;
        // Deliberately no active-handle assignment, local session, or draft write.
        Ok(())
    }

    pub(crate) fn published_restore_exists(&self, id: &str) -> Result<bool> {
        let (json, instance): (Option<String>, String) = self.connection.query_row(
            "SELECT checkpoint_json,planned_instance_id FROM app_operations WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let Some(json) = json else {
            return Ok(false);
        };
        let saved: RestoreCheckpoint = serde_json::from_str(&json)?;
        Ok(validate_published(&saved, id, &instance).is_ok())
    }
}

pub(crate) fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > pkms_core::BACKUP_MAX_BYTES {
        return Err(AppError::InputUnavailable);
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn validate_published(saved: &RestoreCheckpoint, id: &str, instance: &str) -> Result<()> {
    let path = Path::new(&saved.destination);
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || saved.operation_id != id
        || saved.instance_id != instance
        || physical_identity(path)? != saved.directory_identity
        || physical_identity(path.parent().ok_or(AppError::InputUnavailable)?)?
            != saved.parent_identity
    {
        return Err(AppError::InputUnavailable);
    }
    let mut json = vec![];
    File::open(path.join(".pkms-restore.json"))?
        .take(pkms_core::BACKUP_MAX_MANIFEST_BYTES + 65536)
        .read_to_end(&mut json)?;
    let marker: RestoreCheckpoint = serde_json::from_slice(&json)?;
    if &marker != saved || file_hash(&path.join("library.sqlite3"))? != saved.database_hash {
        return Err(AppError::InputUnavailable);
    }
    Ok(())
}

fn check_cancel(db: &Connection, id: &str) -> pkms_core::Result<()> {
    if db.query_row(
        "SELECT cancel_requested FROM app_operations WHERE id=?1",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(pkms_core::Error::Io(std::io::ErrorKind::Interrupted.into()));
    }
    Ok(())
}
fn progress<'a>(db: &'a Connection, id: &'a str) -> impl FnMut(&str) -> pkms_core::Result<()> + 'a {
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut previous = String::new();
    move |stage| {
        #[cfg(test)]
        crate::background::observe_stage(stage);
        if stage == "copy_bytes" {
            if last.elapsed() >= Duration::from_millis(100) {
                check_cancel(db, id)?;
                last = Instant::now();
            }
            return Ok(());
        }
        if last.elapsed() >= Duration::from_millis(100) || previous != stage {
            check_cancel(db, id)?;
            db.execute(
                "UPDATE app_operations SET stage=?2,updated_at=?3 WHERE id=?1",
                params![id, stage, Utc::now().to_rfc3339()],
            )?;
            previous = stage.into();
            last = Instant::now();
        }
        Ok(())
    }
}
