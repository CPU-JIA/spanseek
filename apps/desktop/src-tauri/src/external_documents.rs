//! Temporary local readers. Paths enter only through native selection, drop or argv.
use crate::runtime::lock;
use pkms_app::{
    ApiError, ApiErrorCode, GrantPurpose, LocalDocument, LocalDocumentFailure, LocalDocuments,
    ResourceGrant, WorkspaceIdentity,
};
use pkms_core::SourceFormat;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
use tauri::{
    AppHandle, Emitter, WebviewWindow,
    http::{Request, Response, StatusCode, header},
};
use tauri_plugin_dialog::DialogExt;

const MAX_DOCUMENTS: usize = 100;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
struct OpenFile {
    path: PathBuf,
    physical_file: same_file::Handle,
    file: Arc<Mutex<File>>,
    modified: Option<SystemTime>,
    document: LocalDocument,
}
static FILES: OnceLock<Mutex<HashMap<String, OpenFile>>> = OnceLock::new();
static PENDING: OnceLock<Mutex<LocalDocuments>> = OnceLock::new();
fn empty() -> LocalDocuments {
    LocalDocuments {
        documents: Vec::new(),
        failures: Vec::new(),
    }
}
fn pending() -> &'static Mutex<LocalDocuments> {
    PENDING.get_or_init(|| Mutex::new(empty()))
}
fn unavailable() -> ApiError {
    ApiError::new(ApiErrorCode::InputUnavailable, "")
}
fn checked_import_path(entry: &OpenFile) -> Result<PathBuf, ApiError> {
    let original = lock(&entry.file, "")?
        .metadata()
        .map_err(|_| unavailable())?;
    let current = std::fs::metadata(&entry.path).map_err(|_| unavailable())?;
    if same_file::Handle::from_path(&entry.path).map_err(|_| unavailable())? != entry.physical_file
        || original.len() != entry.document.size
        || current.len() != entry.document.size
        || original.modified().ok() != entry.modified
        || current.modified().ok() != entry.modified
    {
        return Err(unavailable());
    }
    Ok(entry.path.clone())
}
fn main_window(window: &WebviewWindow) -> Result<(), ApiError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(ApiError::new(ApiErrorCode::InvalidInput, ""))
    }
}

fn open_file(path: &Path) -> Result<LocalDocument, ApiError> {
    let path = path.canonicalize().map_err(|_| unavailable())?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // The importer and this temporary reader must accept exactly the same files:
    // a document that opens here but fails on import (or the reverse) would show
    // a confident "opened" state and then refuse to save.
    let format = pkms_app::format_for_extension(&extension)
        .ok_or_else(|| ApiError::new(ApiErrorCode::InvalidInput, ""))?;
    let mut file = File::open(&path).map_err(|_| unavailable())?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    let physical_file = same_file::Handle::from_file(file.try_clone().map_err(|_| unavailable())?)
        .map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BYTES {
        return Err(unavailable());
    }
    if format == SourceFormat::Pdf {
        let mut header = [0u8; 1024];
        let count = file.read(&mut header).map_err(|_| unavailable())?;
        if !header[..count].windows(5).any(|value| value == b"%PDF-") {
            return Err(unavailable());
        }
        file.rewind().map_err(|_| unavailable())?;
    }
    let mut files = lock(FILES.get_or_init(Default::default), "")?;
    if let Some(existing) = files.values().find(|entry| {
        entry.path == path
            && entry.physical_file == physical_file
            && entry.document.size == metadata.len()
            && entry.modified == metadata.modified().ok()
    }) {
        return Ok(existing.document.clone());
    }
    if files.len() >= MAX_DOCUMENTS {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, ""));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let resource_url = (format == SourceFormat::Pdf).then(|| {
        if cfg!(target_os = "windows") {
            format!("http://pkms-source.localhost/local/{token}")
        } else {
            format!("pkms-source://localhost/local/{token}")
        }
    });
    let document = LocalDocument {
        token: token.clone(),
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        location_hint: path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default(),
        format,
        size: metadata.len(),
        resource_url,
    };
    files.insert(
        token,
        OpenFile {
            path,
            physical_file,
            file: Arc::new(Mutex::new(file)),
            modified: metadata.modified().ok(),
            document: document.clone(),
        },
    );
    Ok(document)
}

fn open_paths(paths: Vec<PathBuf>) -> LocalDocuments {
    let mut result = empty();
    for path in paths {
        let display_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let opened = if result.documents.len() + result.failures.len() >= MAX_DOCUMENTS {
            Err(ApiError::new(ApiErrorCode::InvalidInput, ""))
        } else {
            open_file(&path)
        };
        match opened {
            Ok(document) => {
                if !result
                    .documents
                    .iter()
                    .any(|item| item.token == document.token)
                {
                    result.documents.push(document);
                }
            }
            Err(error) => result.failures.push(LocalDocumentFailure {
                display_name,
                error,
            }),
        }
    }
    result
}

pub fn receive_paths(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = open_paths(paths);
        match lock(pending(), "") {
            Ok(mut queue) => {
                for document in result.documents {
                    if !queue
                        .documents
                        .iter()
                        .any(|item| item.token == document.token)
                    {
                        queue.documents.push(document);
                    }
                }
                queue.failures.append(&mut result.failures);
                drop(queue);
                let _ = app.emit_to("main", "spanseek:open-files", ());
            }
            Err(_) => eprintln!("spanseek: file-open queue unavailable"),
        }
    });
}

pub fn receive_arguments(app: &AppHandle, arguments: Vec<String>, directory: PathBuf) {
    let paths = arguments
        .into_iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                directory.join(path)
            }
        })
        .collect();
    receive_paths(app, paths);
}

#[tauri::command]
pub fn take_open_documents(window: WebviewWindow) -> Result<LocalDocuments, ApiError> {
    main_window(&window)?;
    Ok(std::mem::replace(&mut *lock(pending(), "")?, empty()))
}

#[tauri::command]
pub async fn choose_open_documents(
    app: AppHandle,
    window: WebviewWindow,
    pdf_only: bool,
) -> Result<LocalDocuments, ApiError> {
    main_window(&window)?;
    let runtime = crate::runtime::current(&app, "")?;
    use std::sync::atomic::Ordering;
    if runtime
        .dialog_open
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, ""));
    }
    let lease = crate::commands::DialogLease(runtime.dialog_open.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let english = lock(&runtime.service, "")?
            .preferences()
            .map_err(|e| ApiError::from_app(e, ""))?
            .preferences
            .language
            == pkms_app::Language::English;
        let extensions: &[&str] = if pdf_only {
            &["pdf"]
        } else {
            pkms_app::IMPORTABLE_EXTENSIONS
        };
        let chosen = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(if english {
                "Open documents"
            } else {
                "打开资料"
            })
            .add_filter(if english { "Documents" } else { "文档" }, extensions)
            .blocking_pick_files()
            .unwrap_or_default();
        let paths = chosen
            .into_iter()
            .map(|file| file.into_path().map_err(|_| unavailable()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(open_paths(paths))
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

#[tauri::command]
pub fn release_open_document(window: WebviewWindow, token: String) -> Result<(), ApiError> {
    main_window(&window)?;
    uuid::Uuid::parse_str(&token).map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, ""))?;
    lock(FILES.get_or_init(Default::default), "")?.remove(&token);
    Ok(())
}

#[tauri::command]
pub async fn grant_open_documents(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
    tokens: Vec<String>,
) -> Result<Vec<ResourceGrant>, ApiError> {
    main_window(&window)?;
    if tokens.is_empty() || tokens.len() > MAX_DOCUMENTS {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let runtime = crate::runtime::current(&app, "")?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut service = lock(&runtime.service, "")?;
        service
            .library(&identity)
            .map_err(|e| ApiError::from_app(e, ""))?;
        let files = lock(FILES.get_or_init(Default::default), "")?;
        // Validate after acquiring the service lock, and again after issuing the
        // grant. Waiting for another command must not authorize a replacement.
        tokens
            .iter()
            .map(|token| {
                let entry = files.get(token).ok_or_else(unavailable)?;
                let path = checked_import_path(entry)?;
                let grant = service
                    .grant_path(GrantPurpose::ImportSource, &path)
                    .map_err(|e| ApiError::from_app(e, ""))?;
                checked_import_path(entry)?;
                service
                    .resolve_grant(&grant.token, &GrantPurpose::ImportSource)
                    .map_err(|e| ApiError::from_app(e, ""))?;
                Ok(grant)
            })
            .collect()
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

pub fn read_resource(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/pdf".parse().expect("static header"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    response.headers_mut().insert(
        header::ACCEPT_RANGES,
        "bytes".parse().expect("static header"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static header"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        "Accept-Ranges, Content-Range, Content-Length"
            .parse()
            .expect("static header"),
    );
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if !matches!(origin, "http://tauri.localhost" | "tauri://localhost")
            && !(cfg!(debug_assertions)
                && matches!(origin, "http://127.0.0.1:5173" | "http://localhost:5173"))
        {
            *response.status_mut() = StatusCode::FORBIDDEN;
            return response;
        }
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            origin.parse().expect("validated origin"),
        );
        response
            .headers_mut()
            .insert(header::VARY, "Origin".parse().expect("static header"));
    }
    let opened = (|| -> Result<_, ApiError> {
        if !matches!(request.method().as_str(), "GET" | "HEAD") || request.uri().query().is_some() {
            return Err(unavailable());
        }
        let token = request
            .uri()
            .path()
            .strip_prefix("/local/")
            .ok_or_else(unavailable)?;
        uuid::Uuid::parse_str(token).map_err(|_| unavailable())?;
        let files = lock(FILES.get_or_init(Default::default), "")?;
        let entry = files
            .get(token)
            .filter(|e| e.document.format == SourceFormat::Pdf)
            .ok_or_else(unavailable)?;
        let metadata = lock(&entry.file, "")?
            .metadata()
            .map_err(|_| unavailable())?;
        if metadata.len() != entry.document.size || metadata.modified().ok() != entry.modified {
            return Err(unavailable());
        }
        Ok((entry.file.clone(), entry.document.size))
    })();
    let Ok((file, size)) = opened else {
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    };
    if request.method() == "HEAD" {
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            size.to_string().parse().expect("numeric header"),
        );
        return response;
    }
    let Some((start, end, partial)) = crate::source_resources::range(
        request
            .headers()
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok()),
        size,
    ) else {
        *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes */{size}").parse().expect("numeric header"),
        );
        return response;
    };
    let count = end - start + 1;
    // A cloned OS handle shares its seek cursor. Serialize each handle's range
    // read so concurrent PDF.js requests cannot read one another's offsets.
    let Ok(mut file) = lock(&file, "") else {
        *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
        return response;
    };
    let mut bytes = Vec::with_capacity(count as usize);
    if file.seek(SeekFrom::Start(start)).is_err()
        || (&mut *file).take(count).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 != count
    {
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    }
    if partial {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{size}")
                .parse()
                .expect("numeric header"),
        );
    }
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        count.to_string().parse().expect("numeric header"),
    );
    *response.body_mut() = bytes;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_native_paths_are_one_pending_document() {
        let path =
            std::env::temp_dir().join(format!("spanseek-duplicate-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"%PDF-1.7\nfixture").unwrap();
        let documents = open_paths(vec![path.clone(), path.clone()]);
        assert_eq!(documents.documents.len(), 1);
        assert!(documents.failures.is_empty());
        lock(FILES.get_or_init(Default::default), "")
            .unwrap()
            .remove(&documents.documents[0].token);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replacing_a_path_with_same_size_and_timestamp_never_reuses_or_promotes_the_old_handle() {
        let path =
            std::env::temp_dir().join(format!("spanseek-identity-{}.pdf", uuid::Uuid::new_v4()));
        let previous = path.with_extension("original.pdf");
        std::fs::write(&path, b"%PDF-1.7\nOLD content").unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let first = open_file(&path).unwrap();
        std::fs::rename(&path, &previous).unwrap();
        std::fs::write(&path, b"%PDF-1.7\nNEW content").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let second = open_file(&path).unwrap();
        assert_ne!(first.token, second.token);
        let mut files = lock(FILES.get_or_init(Default::default), "").unwrap();
        assert!(checked_import_path(files.get(&first.token).unwrap()).is_err());
        assert!(checked_import_path(files.get(&second.token).unwrap()).is_ok());
        files.remove(&first.token);
        files.remove(&second.token);
        drop(files);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(previous).unwrap();
    }

    #[test]
    fn temporary_documents_accept_only_selected_supported_files_and_range_tokens() {
        let path =
            std::env::temp_dir().join(format!("spanseek-local-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"%PDF-1.7\nfixture content").unwrap();
        let doc = open_file(&path).unwrap();
        let uri = doc.resource_url.as_ref().unwrap();
        let response = read_resource(
            Request::builder()
                .uri(uri)
                .header(header::RANGE, "bytes=0-7")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), b"%PDF-1.7");
        let foreign = read_resource(
            Request::builder()
                .uri(uri)
                .header(header::ORIGIN, "https://example.com")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
        lock(FILES.get_or_init(Default::default), "")
            .unwrap()
            .remove(&doc.token);
        let closed = read_resource(Request::builder().uri(uri).body(Vec::new()).unwrap());
        assert_eq!(closed.status(), StatusCode::NOT_FOUND);
        std::fs::remove_file(path).unwrap();
    }

    /// "Open documents" is the entry point for the drop target, the Ctrl+O
    /// dialog and the shell's "open with" arguments. It must accept exactly the
    /// formats the importer accepts, or a dropped Word file reports "opened" and
    /// then fails when it is saved into a workspace.
    #[test]
    fn temporary_documents_open_every_importable_format_and_reject_the_rest() {
        for (name, expected) in [
            ("fixture.md", SourceFormat::Markdown),
            ("fixture.markdown", SourceFormat::Markdown),
            ("fixture.txt", SourceFormat::Text),
            ("fixture.docx", SourceFormat::Docx),
        ] {
            let path =
                std::env::temp_dir().join(format!("spanseek-{}-{name}", uuid::Uuid::new_v4()));
            std::fs::write(&path, b"fixture content").unwrap();
            let document = open_file(&path).unwrap();
            assert_eq!(document.format, expected, "{name}");
            assert!(document.resource_url.is_none(), "{name}");
            lock(FILES.get_or_init(Default::default), "")
                .unwrap()
                .remove(&document.token);
            std::fs::remove_file(path).unwrap();
        }
        let rejected = std::env::temp_dir().join(format!("spanseek-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&rejected, b"fixture content").unwrap();
        assert!(open_file(&rejected).is_err());
        std::fs::remove_file(rejected).unwrap();
    }
}
