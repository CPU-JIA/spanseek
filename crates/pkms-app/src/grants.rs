use crate::{
    AppError, AppService, GrantPurpose, ResourceGrant, Result, path_text, physical_identity,
    storage_identity, uuid,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use std::path::{Path, PathBuf};

pub(crate) fn purpose_key(purpose: &GrantPurpose) -> &'static str {
    match purpose {
        GrantPurpose::OpenWorkspace => "open_workspace",
        GrantPurpose::CreateWorkspace => "create_workspace",
        GrantPurpose::ImportFile => "import_file",
        GrantPurpose::ImportSource => "import_source",
        GrantPurpose::ImportBibliography => "import_bibliography",
        GrantPurpose::ImportDirectory => "import_directory",
        GrantPurpose::ExportFile => "export_file",
        GrantPurpose::BackupInput => "backup_input",
        GrantPurpose::HandoffInput => "handoff_input",
        GrantPurpose::HandoffOutput => "handoff_output",
        GrantPurpose::RestoreWorkspace => "restore_workspace",
    }
}

impl AppService {
    /// Imports a user-selected BibTeX/CSL-JSON file. Import is idempotent by
    /// citation key, so a retry after a crash re-runs safely and skips existing
    /// entries instead of duplicating reference notes.
    pub fn import_bibliography_file(
        &mut self,
        identity: &crate::WorkspaceIdentity,
        token: &str,
    ) -> Result<pkms_core::BibImportReport> {
        let path = self.resolve_grant(token, &GrantPurpose::ImportBibliography)?;
        // The grant is resolved again after the import attempt so a file swapped
        // between grant and read is still rejected by the physical identity check.
        let report = self.library_mut(identity)?.import_bibliography(&path)?;
        self.resolve_grant(token, &GrantPurpose::ImportBibliography)?;
        Ok(report)
    }
    /// Imports a user-selected `.csl` style under the same one-shot path grant
    /// used for bibliography references. The file is revalidated after the write.
    pub fn import_citation_style_file(
        &mut self,
        identity: &crate::WorkspaceIdentity,
        token: &str,
    ) -> Result<pkms_core::CitationStyleImportReport> {
        let path = self.resolve_grant(token, &GrantPurpose::ImportBibliography)?;
        let style = self.library_mut(identity)?.import_citation_style(&path)?;
        self.resolve_grant(token, &GrantPurpose::ImportBibliography)?;
        Ok(pkms_core::CitationStyleImportReport { style })
    }
    pub fn import_selected_attachment(
        &mut self,
        identity: &crate::WorkspaceIdentity,
        command_id: &str,
        token: &str,
    ) -> Result<pkms_core::AttachmentImport> {
        self.library(identity)?;
        uuid(command_id)?;
        uuid(token)?;
        let existing: Option<(String, String, String)> = self.connection.query_row(
            "SELECT token,captured_hash,original_name FROM attachment_import_commands WHERE local_instance_id=?1 AND workspace_id=?2 AND command_id=?3",
            params![identity.local_instance_id, identity.workspace_id, command_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if let Some((expected, captured_hash, original_name)) = &existing {
            if expected != token {
                return Err(AppError::IdempotencyMismatch);
            }
            match self
                .library_mut(identity)?
                .resume_attachment_import(command_id)
            {
                Ok(result) => {
                    if &result.blob.hash != captured_hash || &result.original_name != original_name
                    {
                        return Err(AppError::IdempotencyMismatch);
                    }
                    return Ok(result);
                }
                Err(pkms_core::Error::NotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let path = self.resolve_grant(token, &GrantPurpose::ImportFile)?;
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(20 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        self.resolve_grant(token, &GrantPurpose::ImportFile)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(AppError::InputUnavailable)?;
        use sha2::{Digest, Sha256};
        let captured_hash = format!("{:x}", Sha256::digest(&bytes));
        if let Some((_, expected_hash, expected_name)) = existing {
            if expected_hash != captured_hash || expected_name != name {
                return Err(AppError::InputUnavailable);
            }
        } else {
            self.connection.execute(
                "INSERT INTO attachment_import_commands VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    identity.local_instance_id,
                    identity.workspace_id,
                    command_id,
                    token,
                    captured_hash,
                    name
                ],
            )?;
        }
        Ok(self
            .library_mut(identity)?
            .import_attachment(command_id, name, &bytes)?)
    }

    /// Native dialog/clipboard issuer only. Business IPC accepts the returned token,
    /// not caller-provided filesystem paths.
    pub fn grant_path(
        &mut self,
        purpose: GrantPurpose,
        path: impl AsRef<Path>,
    ) -> Result<ResourceGrant> {
        let scope = if matches!(
            purpose,
            GrantPurpose::ImportFile
                | GrantPurpose::ImportSource
                | GrantPurpose::ImportDirectory
                | GrantPurpose::ExportFile
                | GrantPurpose::HandoffInput
                | GrantPurpose::HandoffOutput
        ) {
            Some(
                self.active_workspace()
                    .ok_or(AppError::InstanceMismatch)?
                    .identity,
            )
        } else {
            None
        };
        let path = path.as_ref();
        let output = matches!(
            purpose,
            GrantPurpose::CreateWorkspace
                | GrantPurpose::ExportFile
                | GrantPurpose::HandoffOutput
                | GrantPurpose::RestoreWorkspace
        );
        let canonical = if output {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(AppError::OutputExists);
            }
            let name = path
                .file_name()
                .filter(|name| *name != "." && *name != "..")
                .ok_or(AppError::InvalidInput("destination must have a filename"))?;
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()?
                .join(name)
        } else {
            path.canonicalize()?
        };
        let identity = match purpose {
            GrantPurpose::OpenWorkspace => {
                if !canonical.is_dir() {
                    return Err(AppError::InvalidInput("workspace must be a directory"));
                }
                storage_identity(&canonical)?
            }
            GrantPurpose::ImportFile
            | GrantPurpose::ImportBibliography
            | GrantPurpose::BackupInput
            | GrantPurpose::HandoffInput => {
                if !canonical.is_file() {
                    return Err(AppError::InvalidInput("input must be a file"));
                }
                physical_identity(&canonical)?
            }
            GrantPurpose::ImportSource => crate::source_imports::input_identity(&canonical)?,
            GrantPurpose::ImportDirectory => {
                if !canonical.is_dir() {
                    return Err(AppError::InvalidInput("input must be a directory"));
                }
                physical_identity(&canonical)?
            }
            _ => physical_identity(
                canonical
                    .parent()
                    .ok_or(AppError::InvalidInput("missing output parent"))?,
            )?,
        };
        let token = uuid::Uuid::new_v4().to_string();
        let expiry = Utc::now() + chrono::Duration::hours(1);
        self.connection.execute("INSERT INTO local_resource_grants(token,purpose,canonical_path,path_identity,expires_at,local_instance_id,workspace_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![token,purpose_key(&purpose),path_text(&canonical)?,identity,expiry.timestamp(),scope.as_ref().map(|id|&id.local_instance_id),scope.as_ref().map(|id|&id.workspace_id)])?;
        Ok(ResourceGrant {
            identity: scope,
            token,
            purpose,
            display_name: canonical
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            expires_at: expiry.to_rfc3339(),
        })
    }

    pub fn resolve_grant(&self, token: &str, purpose: &GrantPurpose) -> Result<PathBuf> {
        uuid(token)?;
        let row=self.connection.query_row("SELECT purpose,canonical_path,path_identity,expires_at,local_instance_id,workspace_id FROM local_resource_grants WHERE token=?1",[token],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?))).optional()?.ok_or(AppError::InputUnavailable)?;
        if row.0 != purpose_key(purpose) || row.3 <= Utc::now().timestamp() {
            return Err(AppError::InputUnavailable);
        }
        if let (Some(instance), Some(workspace)) = (&row.4, &row.5) {
            let active = self.active_workspace().ok_or(AppError::InstanceMismatch)?;
            if &active.identity.local_instance_id != instance
                || &active.identity.workspace_id != workspace
            {
                return Err(AppError::InstanceMismatch);
            }
        }
        let path = PathBuf::from(row.1);
        let actual = match purpose {
            GrantPurpose::OpenWorkspace => storage_identity(&path),
            GrantPurpose::ImportFile
            | GrantPurpose::ImportBibliography
            | GrantPurpose::BackupInput
            | GrantPurpose::HandoffInput => physical_identity(&path),
            GrantPurpose::ImportSource => crate::source_imports::input_identity(&path),
            GrantPurpose::ImportDirectory => {
                if !path.is_dir() {
                    return Err(AppError::InputUnavailable);
                }
                physical_identity(&path)
            }
            _ => physical_identity(path.parent().ok_or(AppError::InputUnavailable)?),
        }
        .map_err(|_| AppError::InputUnavailable)?;
        if actual != row.2 {
            return Err(AppError::InputUnavailable);
        }
        Ok(path)
    }
}
