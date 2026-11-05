use crate::{AppError, AppService, GrantPurpose, Result, WorkspaceIdentity, physical_identity};
use pkms_core::{ImportSource, JobAccepted, SourceFormat};
use std::{fs::Metadata, path::Path};

/// Every extension this build can import, in the order the native file dialogs
/// offer them. The dialogs, the directory scan, the temporary "open documents"
/// reader and the import command all read this one list, so adding a parser can
/// never leave an entry point behind or let one of them accept a file the
/// importer then rejects.
pub const IMPORTABLE_EXTENSIONS: &[&str] = &[
    "pdf", "md", "markdown", "txt", "docx", "doc", "epub", "mobi", "csv", "xlsx", "xls", "pptx",
];

/// The one place that decides which file extensions are importable. The
/// directory scan and the file dialog must agree, or a scanned file could be
/// offered and then rejected on import.
pub fn format_for_extension(extension: &str) -> Option<SourceFormat> {
    match extension {
        "txt" => Some(SourceFormat::Text),
        "md" | "markdown" => Some(SourceFormat::Markdown),
        "pdf" => Some(SourceFormat::Pdf),
        "docx" => Some(SourceFormat::Docx),
        "doc" => Some(SourceFormat::Doc),
        "epub" => Some(SourceFormat::Epub),
        "mobi" => Some(SourceFormat::Mobi),
        "csv" => Some(SourceFormat::Csv),
        "xlsx" => Some(SourceFormat::Xlsx),
        "xls" => Some(SourceFormat::Xls),
        "pptx" => Some(SourceFormat::Pptx),
        _ => None,
    }
}

pub(crate) fn metadata_signature(metadata: &Metadata) -> Result<(u64, u128)> {
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 256 * 1024 * 1024 {
        return Err(AppError::InvalidInput(
            "source must be a nonempty file no larger than 256 MiB",
        ));
    }
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AppError::InputUnavailable)?
        .as_nanos();
    Ok((metadata.len(), modified))
}
pub(crate) fn input_identity(path: &Path) -> Result<String> {
    let physical = physical_identity(path)?;
    let (size, modified) = metadata_signature(&std::fs::metadata(path)?)?;
    Ok(serde_json::to_string(&(
        physical,
        size,
        modified.to_string(),
    ))?)
}
impl AppService {
    pub fn import_webpage(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        request: pkms_core::ImportWebpage,
    ) -> Result<JobAccepted> {
        self.import_webpage_with_tags(identity, command_id, request, Vec::new())
    }
    pub fn import_webpage_with_tags(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        request: pkms_core::ImportWebpage,
        tag_names: Vec<String>,
    ) -> Result<JobAccepted> {
        self.library(identity)?;
        crate::webpage_capture::validate_url(&request.url)?;
        Ok(self
            .library_mut(identity)?
            .accept_webpage_import_with_tags(command_id, request, tag_names)?)
    }
    pub fn import_selected_source(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        input: ImportSource,
    ) -> Result<JobAccepted> {
        self.import_selected_source_with_tags(identity, command_id, input, Vec::new())
    }
    pub fn import_selected_source_with_tags(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        input: ImportSource,
        tag_names: Vec<String>,
    ) -> Result<JobAccepted> {
        let library = self.library(identity)?;
        if let Some(accepted) =
            library.replay_source_import_with_tags(command_id, &input, &tag_names)?
        {
            return Ok(accepted);
        }
        let path = self.resolve_grant(&input.input_token, &GrantPurpose::ImportSource)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(AppError::InputUnavailable)?;
        if input.original_name != name || input.origin_url.is_some() {
            return Err(AppError::InvalidInput(
                "file import must preserve the selected filename and cannot supply a webpage URL",
            ));
        }
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let format = format_for_extension(&extension).ok_or(AppError::InvalidInput(
            "select a supported TXT, Markdown, PDF, DOCX, EPUB, MOBI, spreadsheet, or presentation file",
        ))?;
        if input.format != format {
            return Err(AppError::InvalidInput(
                "source format does not match the selected file",
            ));
        }
        Ok(self
            .library_mut(identity)?
            .accept_source_import_with_tags(command_id, input, tag_names)?)
    }
    pub fn retry_selected_source_job(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        id: &str,
    ) -> Result<JobAccepted> {
        // Core replays the accepted retry before inspecting the original status.
        Ok(self
            .library_mut(identity)?
            .retry_source_job(command_id, id)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_extension_is_importable_and_listed_once() {
        for extension in IMPORTABLE_EXTENSIONS {
            assert!(
                format_for_extension(extension).is_some(),
                "the file dialogs offer .{extension} but the importer rejects it"
            );
        }
        let mut unique = IMPORTABLE_EXTENSIONS.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            IMPORTABLE_EXTENSIONS.len(),
            "the offered extension list repeats an entry"
        );
    }
}
