//! Word-exact DOCX/DOC preview: render the immutable original through a local
//! Microsoft Word instance and hand the resulting PDF to the existing reader.
//! A machine without Word returns `None`; the interface then uses its bundled
//! semantic renderer. The original blob is never opened for writing.

use crate::runtime::{AppDataDirectory, lock};
use crate::source_resources;
use crate::word_render::{WordRenderError, render_docx_to_pdf};
use pkms_app::{ApiError, ApiErrorCode, AppError, WorkspaceIdentity};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Manager, WebviewWindow};

const CACHE_VERSION: &str = "word-pdf-v2";
const MAX_CACHED_PDFS: usize = 32;
const CACHE_TICKET_TTL: Duration = Duration::from_secs(60 * 60);

#[tauri::command]
pub async fn issue_docx_word_preview(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
    source_version_id: String,
) -> Result<Option<pkms_core::SourceResourceTicket>, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    // Controlled QA hook: exercise the no-Word fallback on a machine that has
    // Word installed. Release builds never read this variable.
    #[cfg(debug_assertions)]
    if std::env::var_os("PKMS_DISABLE_WORD_PREVIEW").is_some() {
        return Ok(None);
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    let app_directory = app
        .try_state::<AppDataDirectory>()
        .map(|state| state.0.clone())
        .ok_or_else(|| ApiError::new(ApiErrorCode::BackendStarting, &request_id))?;
    tauri::async_runtime::spawn_blocking(move || {
        let (source, version_id, extension) = {
            let service = lock(&runtime.service, &request_id)?;
            let library = service
                .library(&identity)
                .map_err(|error| ApiError::from_app(error, &request_id))?;
            let (version, path) = library
                .source_original_path(&source_version_id)
                .map_err(|error| ApiError::from_app(error.into(), &request_id))?;
            if !matches!(
                version.format,
                pkms_core::SourceFormat::Docx | pkms_core::SourceFormat::Doc
            ) {
                return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
            }
            (path, version.id, version.format.stored())
        };
        let cache = app_directory.join("word-pdf").join(CACHE_VERSION);
        fs::create_dir_all(&cache).map_err(|error| io_error(error, &request_id))?;
        cleanup_temporary_files(&cache);
        let output = cache.join(format!("{version_id}.pdf"));
        if !valid_pdf(&output) {
            let _ = fs::remove_file(&output);
            let input = cache.join(format!("input-{}.{}", uuid::Uuid::new_v4(), extension));
            let rendered = cache.join(format!("output-{}.pdf", uuid::Uuid::new_v4()));
            fs::copy(&source, &input).map_err(|error| io_error(error, &request_id))?;
            let result = render_docx_to_pdf(&input, &rendered);
            let _ = fs::remove_file(&input);
            match result {
                Ok(()) if valid_pdf(&rendered) => {
                    if let Err(error) = fs::rename(&rendered, &output) {
                        let _ = fs::remove_file(&rendered);
                        if !valid_pdf(&output) {
                            return Err(io_error(error, &request_id));
                        }
                    }
                }
                Ok(()) => {
                    let _ = fs::remove_file(&rendered);
                    return Err(ApiError::new(ApiErrorCode::Internal, &request_id));
                }
                Err(WordRenderError::NotInstalled) => {
                    let _ = fs::remove_file(&rendered);
                    return Ok(None);
                }
                Err(WordRenderError::Failed(message)) => {
                    let _ = fs::remove_file(&rendered);
                    // Keep the cause in diagnostics but never a document path.
                    eprintln!("Word layout failed: {message}");
                    return Err(ApiError::new(ApiErrorCode::Internal, &request_id));
                }
            }
        }
        prune_cache(&cache, &output);
        source_resources::issue_file_ticket(
            output,
            "application/pdf",
            CACHE_TICKET_TTL,
            &request_id,
        )
        .map(Some)
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

fn io_error(error: std::io::Error, request_id: &str) -> ApiError {
    ApiError::from_app(AppError::Io(error), request_id)
}

fn valid_pdf(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() < 8 {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut header = [0u8; 5];
    file.read_exact(&mut header).is_ok() && &header == b"%PDF-"
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Interrupted conversions can leave a copied input or a partial Word output.
/// These files live only in our own cache directory and are safe to remove.
fn cleanup_temporary_files(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if (name.starts_with("input-") || name.starts_with("output-") || name.starts_with("~$"))
            && modified(&path) < cutoff
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn prune_cache(cache: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let mut pdfs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("pdf"))
        .collect();
    pdfs.sort_by_key(|path| std::cmp::Reverse(modified(path)));
    for path in pdfs.into_iter().skip(MAX_CACHED_PDFS) {
        if path != current {
            let _ = fs::remove_file(path);
        }
    }
}
