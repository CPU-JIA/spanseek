use crate::{Error, Library, Result, UpdateWorkspaceMetadata, WorkspaceMetadata};
use chrono::Utc;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

const MAX_REGISTRY_BYTES: usize = 65_536;

impl Library {
    pub fn workspace_metadata(&self) -> Result<WorkspaceMetadata> {
        read_workspace_metadata(&self.connection)
    }

    pub fn update_workspace_metadata(
        &mut self,
        request: UpdateWorkspaceMetadata,
    ) -> Result<WorkspaceMetadata> {
        validate_registry(&request.object_type_registry, "object type")?;
        validate_registry(&request.note_template_registry, "note template")?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_workspace_metadata(&tx)?;
        if current.row_version != request.base_row_version {
            return Err(Error::Conflict {
                expected: request.base_row_version,
                actual: current.row_version,
            });
        }
        let next = current
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput(
                "workspace metadata row version overflow",
            ))?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE workspace_metadata SET object_type_registry=?1,note_template_registry=?2,row_version=?3,updated_at=?4 WHERE singleton=1",
            params![
                request.object_type_registry,
                request.note_template_registry,
                next,
                now
            ],
        )?;
        let updated = read_workspace_metadata(&tx)?;
        tx.commit()?;
        Ok(updated)
    }

    /// Applies a verified remote configuration. Conflict detection happens in
    /// the sync engine against its known fingerprint before this method runs.
    pub fn workspace_metadata_apply_remote(&mut self, metadata: WorkspaceMetadata) -> Result<()> {
        validate_registry(&metadata.object_type_registry, "object type")?;
        validate_registry(&metadata.note_template_registry, "note template")?;
        validate_state(metadata.row_version, &metadata.updated_at)?;
        self.connection.execute(
            "UPDATE workspace_metadata SET object_type_registry=?1,note_template_registry=?2,row_version=?3,updated_at=?4 WHERE singleton=1",
            params![
                metadata.object_type_registry,
                metadata.note_template_registry,
                metadata.row_version,
                metadata.updated_at
            ],
        )?;
        Ok(())
    }
}

pub(crate) fn validate_backup(connection: &rusqlite::Connection) -> Result<()> {
    let count: i64 =
        connection.query_row("SELECT count(*) FROM workspace_metadata", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        return Err(Error::IntegrityFailure);
    }
    let metadata = read_workspace_metadata(connection)?;
    validate_registry(&metadata.object_type_registry, "object type")?;
    validate_registry(&metadata.note_template_registry, "note template")?;
    if metadata.row_version == 0 || metadata.updated_at.trim().is_empty() {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}
fn validate_state(row_version: u32, updated_at: &str) -> Result<()> {
    if row_version == 0 || chrono::DateTime::parse_from_rfc3339(updated_at).is_err() {
        return Err(Error::InvalidInput(
            "workspace metadata version or timestamp is invalid",
        ));
    }
    Ok(())
}
fn read_workspace_metadata(connection: &rusqlite::Connection) -> Result<WorkspaceMetadata> {
    connection
        .query_row(
            "SELECT object_type_registry,note_template_registry,row_version,updated_at FROM workspace_metadata WHERE singleton=1",
            [],
            |row| {
                Ok(WorkspaceMetadata {
                    object_type_registry: row.get(0)?,
                    note_template_registry: row.get(1)?,
                    row_version: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()?
        .ok_or(Error::IntegrityFailure)
}

fn validate_registry(value: &str, label: &'static str) -> Result<()> {
    if value.len() > MAX_REGISTRY_BYTES {
        return Err(Error::InvalidInput(match label {
            "object type" => "object type registry is too large",
            _ => "note template registry is too large",
        }));
    }
    let registry: serde_json::Value = serde_json::from_str(value).map_err(|_| {
        Error::InvalidInput(match label {
            "object type" => "object type registry must be valid JSON",
            _ => "note template registry must be valid JSON",
        })
    })?;
    if registry.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || !registry
            .get("workspaces")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err(Error::InvalidInput(match label {
            "object type" => "object type registry must use schema 1 and a workspaces object",
            _ => "note template registry must use schema 1 and a workspaces object",
        }));
    }
    Ok(())
}
