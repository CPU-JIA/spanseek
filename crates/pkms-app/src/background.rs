//! Detached storage connections share process/workspace ownership, never the UI
//! service mutex. Only short acceptance and terminal state writes serialize.
use crate::{
    ActiveWorkspace, AppError, AppOperation, AppService, OperationKind, Result, path_text,
    storage_identity,
};
use pkms_core::{Library, WorkspaceLease};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::Path,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(crate) struct WorkerSchedule {
    running: bool,
    requested: bool,
}
pub(crate) struct WorkspaceOwner {
    storage_identity: String,
    lease: Weak<WorkspaceLease>,
    schedule: Arc<Mutex<WorkerSchedule>>,
}
type Owner = (Arc<WorkspaceLease>, Arc<Mutex<WorkerSchedule>>);

pub struct PortableJobsWorker {
    service: AppService,
    guard: WorkerGuard,
}
struct WorkerGuard {
    schedule: Arc<Mutex<WorkerSchedule>>,
    active: bool,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.active {
            let mut state = self.schedule.lock().unwrap_or_else(|p| p.into_inner());
            state.running = false;
        }
    }
}
pub struct BackupOperationWorker {
    service: AppService,
    operation_id: String,
}

impl AppService {
    fn background_copy(&self, active: Option<ActiveWorkspace>) -> Result<Self> {
        let connection = Connection::open_with_flags(
            self.connection
                .path()
                .ok_or(AppError::UnsupportedDatabase)?,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self {
            connection,
            app_directory: self.app_directory.clone(),
            active,
            _lock: self._lock.clone(),
            workspace_owners: Default::default(),
        })
    }

    /// Returns one worker per physical workspace. Calls during an active worker
    /// request another drain pass, so a newly accepted job cannot lose its wakeup.
    pub fn portable_jobs_worker(&self) -> Result<Option<PortableJobsWorker>> {
        let Some(active) = &self.active else {
            return Ok(None);
        };
        let schedule = active.portable_schedule.clone();
        {
            let mut state = schedule.lock().map_err(|_| AppError::AppBusy)?;
            state.requested = true;
            if state.running {
                return Ok(None);
            }
            state.running = true;
        }
        let guard = WorkerGuard {
            schedule,
            active: true,
        };
        if storage_identity(Path::new(&active.entry.path))? != active.storage_identity {
            return Err(AppError::InputUnavailable);
        }
        let library = Library::open_existing(&active.entry.path)?;
        if library.info()?.workspace_id != active.entry.identity.workspace_id {
            return Err(AppError::InstanceMismatch);
        }
        let copied = ActiveWorkspace {
            library,
            entry: active.entry.clone(),
            storage_identity: active.storage_identity.clone(),
            _lease: active._lease.clone(),
            portable_schedule: active.portable_schedule.clone(),
        };
        Ok(Some(PortableJobsWorker {
            service: self.background_copy(Some(copied))?,
            guard,
        }))
    }

    /// Only verify/restore can detach. Opening/creating a workspace changes the
    /// live handle and remains in the application's serialized operation queue.
    pub fn backup_operation_worker(&self, id: &str) -> Result<BackupOperationWorker> {
        let operation = self.operation(id)?;
        if !matches!(
            operation.kind,
            OperationKind::VerifyBackup | OperationKind::RestoreBackup
        ) {
            return Err(AppError::InvalidInput(
                "operation needs the live workspace service",
            ));
        }
        Ok(BackupOperationWorker {
            service: self.background_copy(None)?,
            operation_id: id.into(),
        })
    }

    pub(crate) fn workspace_owner(&mut self, path: &Path) -> Result<Owner> {
        self.workspace_owners
            .retain(|_, owner| owner.lease.strong_count() > 0);
        let key = path_text(path)?;
        if let Some(owner) = self.workspace_owners.get(&key)
            && let Some(lease) = owner.lease.upgrade()
        {
            if storage_identity(path)? != owner.storage_identity {
                return Err(AppError::InputUnavailable);
            }
            return Ok((lease, owner.schedule.clone()));
        }
        Ok((
            Arc::new(WorkspaceLease::acquire(path)?),
            Arc::new(Mutex::new(WorkerSchedule::default())),
        ))
    }
    pub(crate) fn remember_workspace_owner(
        &mut self,
        path: &Path,
        physical: &str,
        lease: &Arc<WorkspaceLease>,
        schedule: &Arc<Mutex<WorkerSchedule>>,
    ) -> Result<()> {
        self.workspace_owners.insert(
            path_text(path)?,
            WorkspaceOwner {
                storage_identity: physical.into(),
                lease: Arc::downgrade(lease),
                schedule: schedule.clone(),
            },
        );
        Ok(())
    }
}

impl PortableJobsWorker {
    pub fn run(mut self) -> Result<()> {
        loop {
            self.guard
                .schedule
                .lock()
                .map_err(|_| AppError::AppBusy)?
                .requested = false;
            self.service.run_pending_portable_jobs()?;
            let mut state = self.guard.schedule.lock().map_err(|_| AppError::AppBusy)?;
            if state.requested {
                continue;
            }
            state.running = false;
            self.guard.active = false;
            return Ok(());
        }
    }
}
impl BackupOperationWorker {
    pub fn run(mut self) -> Result<AppOperation> {
        self.service.execute_operation(&self.operation_id)
    }
}

#[cfg(test)]
type StageHook = Box<dyn FnMut(&str)>;
#[cfg(test)]
thread_local! {static TEST_STAGE:std::cell::RefCell<Option<StageHook>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) fn observe_stage(stage: &str) {
    TEST_STAGE.with_borrow_mut(|hook| {
        if let Some(hook) = hook {
            hook(stage);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, OperationStatus,
        WorkspaceIdentity,
    };
    use pkms_core::{ExportFormat, JobStatus, PortableJobInput};
    use std::{sync::mpsc, time::Duration};

    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn open(app: &mut AppService, path: &Path, create: bool) -> WorkspaceIdentity {
        let grant = app
            .grant_path(
                if create {
                    GrantPurpose::CreateWorkspace
                } else {
                    GrantPurpose::OpenWorkspace
                },
                path,
            )
            .unwrap();
        let operation = app
            .accept_workspace_operation(
                &id(),
                if create {
                    OperationKind::CreateWorkspace
                } else {
                    OperationKind::OpenWorkspace
                },
                &grant.token,
            )
            .unwrap();
        let done = app.execute_operation(&operation.operation_id).unwrap();
        assert_eq!(done.status, OperationStatus::Succeeded, "{done:?}");
        done.result.unwrap().identity
    }

    #[test]
    fn detached_backup_allows_saves_switch_and_reopen_and_does_not_lose_a_second_job() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("knowledge");
        let mut app = AppService::open(root.path().join("app")).unwrap();
        let identity = open(&mut app, &workspace, true);
        let note = app
            .library_mut(&identity)
            .unwrap()
            .create_note("快照前", "旧正文")
            .unwrap();
        let archive = root.path().join("backup.zip");
        let token = app
            .grant_path(GrantPurpose::ExportFile, &archive)
            .unwrap()
            .token;
        let first = app
            .accept_output_job(&identity, &id(), &token, PortableJobInput::BackupCreate)
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let worker = app.lock().unwrap().portable_jobs_worker().unwrap().unwrap();
        let switched_before_start = open(
            &mut app.lock().unwrap(),
            &root.path().join("before-worker-start"),
            true,
        );
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut paused = false;
            TEST_STAGE.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move |stage| {
                    if stage == "copy_database" && !paused {
                        paused = true;
                        ready_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
                    }
                }))
            });
            worker.run()
        });
        ready_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        let mut service = app
            .try_lock()
            .expect("worker must not retain UI service mutex");
        assert_eq!(
            service.active_workspace().unwrap().identity,
            switched_before_start
        );
        assert_eq!(open(&mut service, &workspace, false), identity);
        let saved = service
            .library_mut(&identity)
            .unwrap()
            .save_note(&note.id, 1, "保存仍可用", "备份期间编辑")
            .unwrap();
        let export = root.path().join("export.zip");
        let token = service
            .grant_path(GrantPurpose::ExportFile, &export)
            .unwrap()
            .token;
        let second = service
            .accept_output_job(
                &identity,
                &id(),
                &token,
                PortableJobInput::ExportNote {
                    note_id: note.id.clone(),
                    revision_id: saved.revision_id,
                    format: ExportFormat::Markdown,
                    citation_style_id: None,
                },
            )
            .unwrap();
        assert!(service.portable_jobs_worker().unwrap().is_none());
        let other = open(&mut service, &root.path().join("other"), true);
        assert_ne!(identity, other);
        assert!(
            WorkspaceLease::acquire(&workspace).is_err(),
            "background worker retains ownership"
        );
        assert_eq!(open(&mut service, &workspace, false), identity);
        assert!(
            service.portable_jobs_worker().unwrap().is_none(),
            "reopening must reuse the same worker schedule"
        );
        drop(service);
        release_tx.send(()).unwrap();
        thread.join().unwrap().unwrap();
        let service = app.lock().unwrap();
        assert_eq!(
            service
                .library(&identity)
                .unwrap()
                .portable_job(&first.job_id)
                .unwrap()
                .status,
            JobStatus::Succeeded
        );
        assert_eq!(
            service
                .library(&identity)
                .unwrap()
                .portable_job(&second.job_id)
                .unwrap()
                .status,
            JobStatus::Succeeded
        );
        assert!(export.exists());
        let report = pkms_core::verify_backup(&archive, &mut |_| Ok(())).unwrap();
        assert_eq!(report.manifest.counts.revisions, 1);
    }

    #[test]
    fn detached_verification_allows_saves_and_independent_control_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let mut app = AppService::open(root.path().join("app")).unwrap();
        let identity = open(&mut app, &root.path().join("knowledge"), true);
        let note = app
            .library_mut(&identity)
            .unwrap()
            .create_note("原文", "内容")
            .unwrap();
        let archive = root.path().join("backup.zip");
        app.library(&identity)
            .unwrap()
            .create_backup(&archive, &mut |_| Ok(()))
            .unwrap();
        let input = app.grant_path(GrantPurpose::BackupInput, &archive).unwrap();
        let accepted = app
            .accept_backup_operation(&id(), OperationKind::VerifyBackup, &input.token, None)
            .unwrap();
        let mut control = app.operation_control().unwrap();
        let app = Arc::new(Mutex::new(app));
        let worker = app
            .lock()
            .unwrap()
            .backup_operation_worker(&accepted.operation_id)
            .unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut paused = false;
            TEST_STAGE.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move |stage| {
                    if stage == "verify_files" && !paused {
                        paused = true;
                        ready_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
                    }
                }))
            });
            worker.run()
        });
        ready_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        let mut service = app
            .try_lock()
            .expect("verification must not retain UI service mutex");
        service
            .library_mut(&identity)
            .unwrap()
            .save_note(&note.id, 1, "保存成功", "校验时仍能写入")
            .unwrap();
        drop(service);
        let cancel = control
            .dispatch(
                DesktopRequest {
                    api_version: 1,
                    request_id: id(),
                    identity: Some(identity.clone()),
                    command: DesktopCommand::OperationCancel {
                        operation_id: accepted.operation_id,
                    },
                },
                Some(&identity),
            )
            .unwrap();
        assert!(
            matches!(cancel.data,DesktopData::Operation(operation) if operation.cancel_requested)
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            thread.join().unwrap().unwrap().status,
            OperationStatus::Canceled
        );
        assert_eq!(
            app.lock().unwrap().active_workspace().unwrap().identity,
            identity
        );
    }
}
