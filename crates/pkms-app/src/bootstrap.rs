use crate::{
    ActiveWorkspace, AppError, AppOperation, AppOperationPage, AppService, BootstrapAccepted,
    GrantPurpose, OperationKind, OperationStatus, Result, WorkspaceAvailability, WorkspaceEntry,
    WorkspaceIdentity, path_text, physical_identity, storage_identity, uuid,
};
use chrono::Utc;
use pkms_core::Library;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

#[cfg(test)]
thread_local! { static CRASH_BEFORE_CATALOG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Test-only pause point: lets a race test request cancellation while a
/// workspace operation is already running on the storage worker.
#[cfg(test)]
pub(crate) mod pause_hooks {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    static ARMED: AtomicBool = AtomicBool::new(false);
    static REACHED: AtomicBool = AtomicBool::new(false);
    static RELEASE: AtomicBool = AtomicBool::new(false);

    pub(crate) fn arm() {
        REACHED.store(false, Ordering::SeqCst);
        RELEASE.store(false, Ordering::SeqCst);
        ARMED.store(true, Ordering::SeqCst);
    }

    pub(crate) fn wait_until_reached(timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !REACHED.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "operation never reached the cancellation pause point"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn release() {
        RELEASE.store(true, Ordering::SeqCst);
    }

    pub(crate) fn pause() {
        if !ARMED.swap(false, Ordering::SeqCst) {
            return;
        }
        REACHED.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !RELEASE.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CreateCheckpoint {
    operation_id: String,
    planned_instance_id: String,
    directory_identity: String,
}

pub(crate) fn kind_key(kind: &OperationKind) -> &'static str {
    match kind {
        OperationKind::CreateWorkspace => "create_workspace",
        OperationKind::OpenWorkspace => "open_workspace",
        OperationKind::VerifyBackup => "verify_backup",
        OperationKind::RestoreBackup => "restore_backup",
    }
}
fn purpose(kind: &OperationKind) -> GrantPurpose {
    match kind {
        OperationKind::CreateWorkspace => GrantPurpose::CreateWorkspace,
        OperationKind::OpenWorkspace => GrantPurpose::OpenWorkspace,
        OperationKind::VerifyBackup | OperationKind::RestoreBackup => GrantPurpose::BackupInput,
    }
}
fn parse_kind(value: &str) -> Result<OperationKind> {
    match value {
        "create_workspace" => Ok(OperationKind::CreateWorkspace),
        "open_workspace" => Ok(OperationKind::OpenWorkspace),
        "verify_backup" => Ok(OperationKind::VerifyBackup),
        "restore_backup" => Ok(OperationKind::RestoreBackup),
        _ => Err(AppError::UnsupportedDatabase),
    }
}
fn parse_status(value: &str) -> Result<OperationStatus> {
    match value {
        "queued" => Ok(OperationStatus::Queued),
        "running" => Ok(OperationStatus::Running),
        "succeeded" => Ok(OperationStatus::Succeeded),
        "failed" => Ok(OperationStatus::Failed),
        "canceled" => Ok(OperationStatus::Canceled),
        _ => Err(AppError::UnsupportedDatabase),
    }
}

impl AppService {
    /// Includes all unfinished operations, even behind a long terminal history.
    /// Acceptance order is the supervisor order; mutex scheduling is not FIFO.
    pub fn pending_operation_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .connection
            .prepare("SELECT id FROM app_operations WHERE status='queued' ORDER BY rowid")?
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn accept_workspace_operation(
        &mut self,
        command_id: &str,
        kind: OperationKind,
        token: &str,
    ) -> Result<BootstrapAccepted> {
        if !matches!(
            kind,
            OperationKind::CreateWorkspace | OperationKind::OpenWorkspace
        ) {
            return Err(AppError::InvalidInput("use the backup operation endpoint"));
        }
        uuid(command_id)?;
        uuid(token)?;
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(1, kind_key(&kind), token))?)
        );
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                "SELECT payload_hash,operation_id FROM app_command_receipts WHERE command_id=?1",
                [command_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((expected, operation_id)) = existing {
            if expected != hash {
                return Err(AppError::IdempotencyMismatch);
            }
            return Ok(BootstrapAccepted {
                command_id: command_id.into(),
                operation_id,
                replayed: true,
            });
        }
        let grant = tx
            .query_row(
                "SELECT purpose,expires_at FROM local_resource_grants WHERE token=?1",
                [token],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
            .ok_or(AppError::InputUnavailable)?;
        if grant.0 != crate::grants::purpose_key(&purpose(&kind))
            || grant.1 <= Utc::now().timestamp()
        {
            return Err(AppError::InputUnavailable);
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        let planned_instance = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO app_operations(id,kind,token,status,stage,planned_instance_id,created_at,updated_at) VALUES(?1,?2,?3,'queued','queued',?4,?5,?5)",params![operation_id,kind_key(&kind),token,planned_instance,now])?;
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

    pub fn operation(&self, id: &str) -> Result<AppOperation> {
        read_operation(
            &self.connection,
            id,
            self.active.as_ref().map(|active| &active.entry.identity),
        )
    }

    pub fn operations(&self, limit: u32) -> Result<Vec<AppOperation>> {
        if !(1..=100).contains(&limit) {
            return Err(AppError::InvalidInput("operation limit must be 1-100"));
        }
        let ids = self
            .connection
            .prepare("SELECT id FROM app_operations ORDER BY updated_at DESC,id LIMIT ?1")?
            .query_map([limit], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.operation(id)).collect()
    }

    /// One page of bootstrap operations, newest first. `updated_at DESC, id DESC`
    /// is a total order, so the cursor cannot repeat or skip an operation while
    /// new ones keep arriving at the head of the list.
    pub fn operations_page(&self, limit: u32, cursor: Option<String>) -> Result<AppOperationPage> {
        if !(1..=100).contains(&limit) {
            return Err(AppError::InvalidInput("operation limit must be 1-100"));
        }
        let (scope, after) = AppService::decode_app_cursor(&self.connection, 4, cursor.as_deref())?;
        let mut rows = self
            .connection
            .prepare(
                "SELECT id,updated_at FROM app_operations \
                 WHERE (?1 IS NULL OR updated_at<?1 OR (updated_at=?1 AND id<?2)) \
                 ORDER BY updated_at DESC,id DESC LIMIT ?3",
            )?
            .query_map(
                params![
                    after.as_ref().map(|value| &value.0),
                    after.as_ref().map(|value| &value.1),
                    limit + 1
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last()
                .map(|last| serde_json::to_string(&(&scope, &last.1, &last.0)))
                .transpose()?
        } else {
            None
        };
        let items = rows
            .iter()
            .map(|(id, _)| self.operation(id))
            .collect::<Result<Vec<_>>>()?;
        Ok(AppOperationPage { items, next_cursor })
    }

    pub fn cancel_operation(&mut self, id: &str) -> Result<AppOperation> {
        self.operation(id)?;
        self.connection.execute("UPDATE app_operations SET cancel_requested=1,status=CASE WHEN status='queued' THEN 'canceled' ELSE status END,stage=CASE WHEN status='queued' THEN 'canceled' ELSE stage END,updated_at=?2 WHERE id=?1 AND status IN ('queued','running') AND NOT(kind='restore_backup' AND checkpoint_json IS NOT NULL)",params![id,Utc::now().to_rfc3339()])?;
        self.operation(id)
    }

    /// Called by the adapter's storage worker after returning BootstrapAccepted.
    /// Terminal operations are observed, never silently executed a second time.
    pub fn execute_operation(&mut self, id: &str) -> Result<AppOperation> {
        let operation = self.operation(id)?;
        if operation.status != OperationStatus::Queued {
            return Ok(operation);
        }
        let claimed=self.connection.execute("UPDATE app_operations SET status='running',stage='validating',updated_at=?2 WHERE id=?1 AND status='queued'",params![id,Utc::now().to_rfc3339()])?;
        if claimed != 1 {
            return self.operation(id);
        }
        let result = match operation.kind {
            OperationKind::VerifyBackup | OperationKind::RestoreBackup => {
                self.execute_backup_operation(id, &operation.kind)
            }
            _ => self.execute_workspace(id, &operation.kind),
        };
        if let Err(error) = result {
            let code = error_code(&error);
            let interrupted = matches!(&error, AppError::Core(pkms_core::Error::Io(e)) if e.kind()==std::io::ErrorKind::Interrupted);
            let status = if interrupted {
                "canceled"
            } else if operation.kind == OperationKind::RestoreBackup
                && self.published_restore_exists(id)?
            {
                "queued"
            } else {
                "failed"
            };
            self.connection.execute("UPDATE app_operations SET status=?2,stage=?2,error_code=?3,updated_at=?4 WHERE id=?1",params![id,status,if interrupted {None}else{Some(code)},Utc::now().to_rfc3339()])?;
        }
        self.operation(id)
    }

    /// Debug-only: hold a workspace operation open so a real WebView run can
    /// exercise the cancel path. Cancellation is re-checked every 100 ms;
    /// release builds never include this seam.
    #[cfg(debug_assertions)]
    fn debug_hold_workspace_operation(&self, id: &str) -> Result<bool> {
        let Ok(value) = std::env::var("PKMS_TEST_BOOTSTRAP_DELAY_MS") else {
            return Ok(false);
        };
        let Ok(ms) = value.parse::<u64>() else {
            return Ok(false);
        };
        if ms == 0 {
            return Ok(false);
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(ms.min(120_000));
        while std::time::Instant::now() < deadline {
            if self.cancel_workspace_operation_if_requested(id)? {
                return Ok(true);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(false)
    }

    fn execute_workspace(&mut self, id: &str, kind: &OperationKind) -> Result<()> {
        let (token, planned_instance, checkpoint) = self.connection.query_row(
            "SELECT token,planned_instance_id,checkpoint_json FROM app_operations WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        let target = self.resolve_grant(&token, &purpose(kind))?;
        if self.cancel_workspace_operation_if_requested(id)? {
            return Ok(());
        }
        if *kind == OperationKind::CreateWorkspace {
            self.prepare_create(id, &planned_instance, &target, checkpoint.as_deref())?;
        }
        if self.cancel_workspace_operation_if_requested(id)? {
            return Ok(());
        }
        let path = path_text(&target)?;
        if let Some(active) = &self.active
            && active.entry.path == path
        {
            let actual = storage_identity(&target)?;
            if actual != active.storage_identity {
                return Err(AppError::InputUnavailable);
            }
            let workspace = active.library.info()?.workspace_id;
            let entry =
                self.complete_operation(id, &planned_instance, &target, &workspace, &actual)?;
            self.active.as_mut().unwrap().entry = entry;
            return Ok(());
        }
        let (lease, portable_schedule) = self.workspace_owner(&target)?;
        #[cfg(debug_assertions)]
        if self.debug_hold_workspace_operation(id)? {
            return Ok(());
        }
        #[cfg(test)]
        pause_hooks::pause();
        if self.cancel_workspace_operation_if_requested(id)? {
            return Ok(());
        }
        self.connection.execute(
            "UPDATE app_operations SET stage='opening',updated_at=?2 WHERE id=?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        let library = if *kind == OperationKind::CreateWorkspace
            && !target.join("library.sqlite3").exists()
        {
            Library::open(&target)?
        } else {
            Library::open_existing(&target)?
        };
        if self.cancel_workspace_operation_if_requested(id)? {
            return Ok(());
        }
        let workspace = library.info()?.workspace_id;
        #[cfg(test)]
        if CRASH_BEFORE_CATALOG.get() {
            std::process::exit(86);
        }
        let physical = storage_identity(&target)?;
        if self.cancel_workspace_operation_if_requested(id)? {
            return Ok(());
        }
        self.connection.execute(
            "UPDATE app_operations SET stage='registering',updated_at=?2 WHERE id=?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        let entry =
            self.complete_operation(id, &planned_instance, &target, &workspace, &physical)?;
        self.remember_workspace_owner(&target, &physical, &lease, &portable_schedule)?;
        // Keep the old workspace usable until both new handle and catalog commit succeed.
        self.active = Some(ActiveWorkspace {
            library,
            entry,
            storage_identity: physical,
            _lease: lease,
            portable_schedule,
        });
        Ok(())
    }

    /// A workspace operation runs on the storage worker. Check cancellation
    /// between expensive stages so a request has a terminal result instead of
    /// being ignored until the whole operation finishes.
    fn cancel_workspace_operation_if_requested(&self, id: &str) -> Result<bool> {
        let cancel: bool = self.connection.query_row(
            "SELECT cancel_requested FROM app_operations WHERE id=?1",
            [id],
            |row| row.get(0),
        )?;
        if cancel {
            self.connection.execute(
                "UPDATE app_operations SET status='canceled',stage='canceled',updated_at=?2 WHERE id=?1",
                params![id, Utc::now().to_rfc3339()],
            )?;
        }
        Ok(cancel)
    }

    fn prepare_create(
        &mut self,
        id: &str,
        planned_instance: &str,
        target: &Path,
        checkpoint: Option<&str>,
    ) -> Result<()> {
        let marker = target.join(".pkms-bootstrap.json");
        let expected = if let Some(checkpoint) = checkpoint {
            Some(serde_json::from_str::<CreateCheckpoint>(checkpoint)?)
        } else {
            None
        };
        let actual = if target.exists() {
            if !target.is_dir() {
                return Err(AppError::InputUnavailable);
            }
            let file = fs::File::open(&marker).map_err(|_| AppError::InputUnavailable)?;
            if file.metadata()?.len() > 16 * 1024 {
                return Err(AppError::InputUnavailable);
            }
            let mut data = String::new();
            file.take(16 * 1024).read_to_string(&mut data)?;
            let saved: CreateCheckpoint =
                serde_json::from_str(&data).map_err(|_| AppError::InputUnavailable)?;
            if saved.operation_id != id
                || saved.planned_instance_id != planned_instance
                || saved.directory_identity != physical_identity(target)?
                || expected.as_ref().is_some_and(|expected| expected != &saved)
            {
                return Err(AppError::InputUnavailable);
            }
            saved
        } else {
            if expected.is_some() {
                return Err(AppError::InputUnavailable);
            }
            fs::create_dir(target)?;
            let saved = CreateCheckpoint {
                operation_id: id.into(),
                planned_instance_id: planned_instance.into(),
                directory_identity: physical_identity(target)?,
            };
            let mut file = fs::File::create_new(&marker)?;
            file.write_all(&serde_json::to_vec(&saved)?)?;
            file.sync_all()?;
            saved
        };
        self.connection.execute("UPDATE app_operations SET checkpoint_json=?2,stage='directory_ready',updated_at=?3 WHERE id=?1",params![id,serde_json::to_string(&actual)?,Utc::now().to_rfc3339()])?;
        Ok(())
    }

    fn complete_operation(
        &mut self,
        id: &str,
        planned_instance: &str,
        target: &Path,
        workspace: &str,
        physical: &str,
    ) -> Result<WorkspaceEntry> {
        let path = path_text(target)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let instance:Option<String>=tx.query_row("SELECT local_instance_id FROM workspace_catalog WHERE canonical_path=?1 AND storage_identity=?2 AND workspace_id=?3",params![path,physical,workspace],|r|r.get(0)).optional()?;
        let instance = instance.unwrap_or_else(|| planned_instance.into());
        let now = Utc::now().to_rfc3339();
        let entry = WorkspaceEntry {
            identity: WorkspaceIdentity {
                local_instance_id: instance.clone(),
                workspace_id: workspace.into(),
            },
            path: path.clone(),
            last_opened_at: now.clone(),
            availability: WorkspaceAvailability::Available,
        };
        tx.execute("INSERT INTO workspace_catalog(local_instance_id,workspace_id,canonical_path,storage_identity,last_opened_at,show_in_recents) VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(local_instance_id) DO UPDATE SET last_opened_at=excluded.last_opened_at,show_in_recents=1",params![instance,workspace,path,physical,now])?;
        tx.execute("UPDATE app_operations SET status='succeeded',stage='complete',result_json=?2,error_code=NULL,updated_at=?3 WHERE id=?1",params![id,serde_json::to_string(&entry)?,now])?;
        tx.commit()?;
        Ok(entry)
    }
}

pub(crate) fn read_operation(
    connection: &rusqlite::Connection,
    id: &str,
    active_identity: Option<&WorkspaceIdentity>,
) -> Result<AppOperation> {
    uuid(id)?;
    let row=connection.query_row("SELECT kind,status,stage,result_json,error_code,cancel_requested,updated_at FROM app_operations WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?))).optional()?.ok_or(AppError::NotFound)?;
    // A history row outlives the build that wrote it: read every shape this
    // build knows and keep the row when it knows none of them.
    let result = row
        .3
        .as_deref()
        .and_then(|json| workspace_entry_from_history(connection, json));
    let active = result
        .as_ref()
        .is_some_and(|result| active_identity.is_some_and(|active| active == &result.identity));
    let report_json: Option<String> = connection.query_row(
        "SELECT report_json FROM app_operations WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(AppOperation {
        id: id.into(),
        kind: parse_kind(&row.0)?,
        status: parse_status(&row.1)?,
        stage: row.2,
        result,
        report: report_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        error_code: row.4,
        cancel_requested: row.5,
        active_in_current_process: active,
        updated_at: row.6,
    })
}

/// The result column keeps the workspace entry as the *writing* build knew it,
/// so a row can outlive the shape it was written in: V1.8 replaced the two-state
/// `available` flag with the four-way `availability` (V18-G10). Reading only the
/// current shape made every page that contained an older row fail as invalid
/// input, which hid the whole operation history behind one banner. Both shapes are
/// read here, and a row this build cannot read at all keeps its place in the list
/// without a result instead of taking the list down with it.
fn workspace_entry_from_history(
    connection: &rusqlite::Connection,
    json: &str,
) -> Option<WorkspaceEntry> {
    if let Ok(entry) = serde_json::from_str::<WorkspaceEntry>(json) {
        return Some(entry);
    }
    let legacy: LegacyWorkspaceEntry = serde_json::from_str(json).ok()?;
    let availability = if legacy.available {
        WorkspaceAvailability::Available
    } else {
        // The old flag only said "not usable". Ask the rule the recent list
        // uses, against the identity the catalog still keeps for this path; an
        // identity the catalog no longer holds cannot be opened from here.
        match catalog_storage_identity(connection, &legacy.identity) {
            Some(stored) => crate::workspace_availability(Path::new(&legacy.path), &stored),
            None => WorkspaceAvailability::Missing,
        }
    };
    Some(WorkspaceEntry {
        identity: legacy.identity,
        path: legacy.path,
        last_opened_at: legacy.last_opened_at,
        availability,
    })
}

/// The entry as builds before V18-G10 wrote it: one bool instead of a reason.
#[derive(Deserialize)]
struct LegacyWorkspaceEntry {
    identity: WorkspaceIdentity,
    path: String,
    last_opened_at: String,
    available: bool,
}

fn catalog_storage_identity(
    connection: &rusqlite::Connection,
    identity: &WorkspaceIdentity,
) -> Option<String> {
    connection
        .query_row(
            "SELECT storage_identity FROM workspace_catalog WHERE local_instance_id=?1 AND workspace_id=?2",
            params![identity.local_instance_id, identity.workspace_id],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

/// The workspace entry point stores this code on the operation row. The
/// classification is shared with every other entry point so that, for example,
/// a corrupt library never reads as `operation_failed` here and
/// `integrity_failed` elsewhere.
fn error_code(error: &AppError) -> &'static str {
    crate::error_codes::error_code(error, crate::error_codes::ErrorDomain::Workspace)
}
