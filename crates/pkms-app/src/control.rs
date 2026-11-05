use crate::{
    AppError, AppOperation, AppOperationPage, AppService, Result, WorkspaceIdentity,
    bootstrap::read_operation,
};
use rusqlite::{Connection, OpenFlags, params};
use std::{fs::File, sync::Arc};

/// Separate SQLite connection so the UI can inspect/cancel bootstrap operations
/// while the storage worker owns AppService and performs migration or file I/O.
pub struct AppOperationControl {
    connection: Connection,
    _lease: Arc<File>,
}

impl AppService {
    pub fn operation_control(&self) -> Result<AppOperationControl> {
        let path = self
            .connection
            .path()
            .ok_or(AppError::UnsupportedDatabase)?;
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(AppOperationControl {
            connection,
            _lease: self._lock.clone(),
        })
    }
}

impl AppOperationControl {
    pub(crate) fn job_library(
        &self,
        requested: Option<&WorkspaceIdentity>,
        active: Option<&WorkspaceIdentity>,
    ) -> Result<pkms_core::Library> {
        use rusqlite::OptionalExtension;
        let identity = requested
            .filter(|identity| Some(*identity) == active)
            .ok_or(AppError::InstanceMismatch)?;
        let row:Option<(String,String)>=self.connection.query_row("SELECT canonical_path,storage_identity FROM workspace_catalog WHERE local_instance_id=?1 AND workspace_id=?2",params![identity.local_instance_id,identity.workspace_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (path, expected) = row.ok_or(AppError::InstanceMismatch)?;
        if crate::storage_identity(std::path::Path::new(&path))? != expected {
            return Err(AppError::InstanceMismatch);
        }
        let library = pkms_core::Library::open_existing(path)?;
        if library.info()?.workspace_id != identity.workspace_id {
            return Err(AppError::InstanceMismatch);
        }
        Ok(library)
    }
    /// active_identity comes from the adapter's live handle state, never a request field.
    pub fn operation(
        &self,
        id: &str,
        active_identity: Option<&WorkspaceIdentity>,
    ) -> Result<AppOperation> {
        read_operation(&self.connection, id, active_identity)
    }

    pub fn operations(
        &self,
        limit: u32,
        active_identity: Option<&WorkspaceIdentity>,
    ) -> Result<Vec<AppOperation>> {
        if !(1..=100).contains(&limit) {
            return Err(AppError::InvalidInput("operation limit must be 1-100"));
        }
        let ids = self
            .connection
            .prepare("SELECT id FROM app_operations ORDER BY updated_at DESC,id LIMIT ?1")?
            .query_map([limit], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| self.operation(id, active_identity))
            .collect()
    }
    /// The same page contract as `AppService::operations_page`, on the control
    /// connection, so the interface can page while the storage worker owns the
    /// service.
    pub fn operations_page(
        &self,
        limit: u32,
        cursor: Option<String>,
        active_identity: Option<&WorkspaceIdentity>,
    ) -> Result<AppOperationPage> {
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
            .map(|(id, _)| self.operation(id, active_identity))
            .collect::<Result<Vec<_>>>()?;
        Ok(AppOperationPage { items, next_cursor })
    }

    pub fn cancel_operation(
        &mut self,
        id: &str,
        active_identity: Option<&WorkspaceIdentity>,
    ) -> Result<AppOperation> {
        self.operation(id, active_identity)?;
        // A restore commit checkpoint can already have published a directory.
        // Cancellation must not strand that directory outside the catalog.
        self.connection.execute("UPDATE app_operations SET cancel_requested=1,status=CASE WHEN status='queued' THEN 'canceled' ELSE status END,stage=CASE WHEN status='queued' THEN 'canceled' ELSE stage END,updated_at=?2 WHERE id=?1 AND status IN ('queued','running') AND NOT(kind='restore_backup' AND checkpoint_json IS NOT NULL)",params![id,chrono::Utc::now().to_rfc3339()])?;
        self.operation(id, active_identity)
    }
}
