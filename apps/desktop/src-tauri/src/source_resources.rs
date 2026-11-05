//! Short-lived, version-scoped original resources; business calls never accept file paths.
use crate::runtime::{DesktopRuntime, lock};
use pkms_app::{ApiError, ApiErrorCode, WorkspaceIdentity};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, WebviewWindow};

#[derive(Clone)]
enum TicketSource {
    Original {
        identity: WorkspaceIdentity,
        version: String,
    },
    /// A derived file in the application cache, never an arbitrary user path.
    File(PathBuf),
}

struct Ticket {
    source: TicketSource,
    size: u64,
    media_type: String,
    expires: Instant,
}
static TICKETS: OnceLock<Mutex<HashMap<String, Ticket>>> = OnceLock::new();
const MAX_RANGE: u64 = 16 * 1024 * 1024;
const MAX_TICKETS: usize = 256;

#[tauri::command]
pub fn release_source_resource(
    window: WebviewWindow,
    resource_url: String,
) -> Result<(), ApiError> {
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let token = resource_url
        .rsplit('/')
        .next()
        .ok_or_else(|| ApiError::new(ApiErrorCode::InvalidInput, ""))?;
    uuid::Uuid::parse_str(token).map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, ""))?;
    lock(TICKETS.get_or_init(Default::default), "")?.remove(token);
    Ok(())
}

fn register(
    source: TicketSource,
    size: u64,
    media_type: &str,
    ttl: Duration,
    request_id: &str,
) -> Result<pkms_core::SourceResourceTicket, ApiError> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut tickets = lock(TICKETS.get_or_init(Default::default), request_id)?;
    tickets.retain(|_, ticket| ticket.expires > Instant::now());
    if tickets.len() >= MAX_TICKETS {
        return Err(ApiError::new(ApiErrorCode::StorageBusy, request_id));
    }
    tickets.insert(
        token.clone(),
        Ticket {
            source,
            size,
            media_type: media_type.into(),
            expires: Instant::now() + ttl,
        },
    );
    let url = if cfg!(target_os = "windows") {
        format!("http://pkms-source.localhost/{token}")
    } else {
        format!("pkms-source://localhost/{token}")
    };
    Ok(pkms_core::SourceResourceTicket {
        url,
        size,
        media_type: media_type.into(),
        expires_at: (chrono::Utc::now() + chrono::Duration::seconds(ttl.as_secs() as i64))
            .to_rfc3339(),
    })
}

/// Registers a derived, application-owned file for PDF.js/range requests.
pub(crate) fn issue_file_ticket(
    path: PathBuf,
    media_type: &str,
    ttl: Duration,
    request_id: &str,
) -> Result<pkms_core::SourceResourceTicket, ApiError> {
    let size = std::fs::metadata(&path)
        .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, request_id))?
        .len();
    if size == 0 {
        return Err(ApiError::new(ApiErrorCode::Internal, request_id));
    }
    register(TicketSource::File(path), size, media_type, ttl, request_id)
}

#[tauri::command]
pub async fn issue_source_resource(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
    source_version_id: String,
) -> Result<pkms_core::SourceResourceTicket, ApiError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
    }
    let runtime = crate::runtime::current(&app, &request_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let service = lock(&runtime.service, &request_id)?;
        let (version, _) = service
            .library(&identity)
            .and_then(|library| {
                library
                    .source_original_file(&source_version_id)
                    .map_err(Into::into)
            })
            .map_err(|error| ApiError::from_app(error, &request_id))?;
        register(
            TicketSource::Original {
                identity,
                version: version.id,
            },
            version.size,
            version.format.media_type(),
            Duration::from_secs(300),
            &request_id,
        )
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}
pub(crate) fn range(value: Option<&str>, size: u64) -> Option<(u64, u64, bool)> {
    if size == 0 {
        return None;
    }
    let Some(value) = value else {
        return (size <= MAX_RANGE).then_some((0, size - 1, false));
    };
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if end.contains(',') {
        return None;
    }
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        (size.saturating_sub(suffix), size - 1)
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            size - 1
        } else {
            end.parse::<u64>().ok()?.min(size - 1)
        };
        (start, end)
    };
    (start <= end && start < size && end - start < MAX_RANGE).then_some((start, end, true))
}
pub fn read_source_resource(
    runtime: &DesktopRuntime,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static header"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; sandbox"
            .parse()
            .expect("static header"),
    );
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let trusted = matches!(origin, "http://tauri.localhost" | "tauri://localhost")
            || (cfg!(debug_assertions)
                && matches!(origin, "http://127.0.0.1:5173" | "http://localhost:5173"));
        if !trusted {
            *response.status_mut() = StatusCode::FORBIDDEN;
            return response;
        }
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            origin.parse().expect("validated origin"),
        );
        headers.insert(header::VARY, "Origin".parse().expect("static header"));
    }
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        "Accept-Ranges, Content-Range, Content-Length"
            .parse()
            .expect("static header"),
    );
    let opened = (|| -> Result<_, ()> {
        if !matches!(request.method().as_str(), "GET" | "HEAD") || request.uri().query().is_some() {
            return Err(());
        }
        let token = request.uri().path().strip_prefix('/').ok_or(())?;
        uuid::Uuid::parse_str(token).map_err(|_| ())?;
        let (source, size, media_type) = {
            let tickets = TICKETS
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| ())?;
            let ticket = tickets
                .get(token)
                .filter(|ticket| ticket.expires > Instant::now())
                .ok_or(())?;
            (
                ticket.source.clone(),
                ticket.size,
                ticket.media_type.clone(),
            )
        };
        let (file, size, media_type) = match source {
            TicketSource::Original { identity, version } => {
                let service = lock(&runtime.service, "").map_err(|_| ())?;
                let (version, file) = service
                    .library(&identity)
                    .and_then(|library| library.source_original_file(&version).map_err(Into::into))
                    .map_err(|_| ())?;
                let media_type = if version.format == pkms_core::SourceFormat::Html {
                    "text/plain".to_string()
                } else {
                    version.format.media_type().to_string()
                };
                (file, version.size, media_type)
            }
            TicketSource::File(path) => {
                let metadata = std::fs::metadata(&path).map_err(|_| ())?;
                if metadata.len() != size {
                    return Err(());
                }
                (File::open(path).map_err(|_| ())?, size, media_type)
            }
        };
        Ok((file, size, media_type))
    })();
    let Ok((mut file, size, media_type)) = opened else {
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    };
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        media_type
            .parse()
            .unwrap_or_else(|_| "application/octet-stream".parse().expect("static header")),
    );
    response.headers_mut().insert(
        header::ACCEPT_RANGES,
        "bytes".parse().expect("static header"),
    );
    if request.method() == "HEAD" {
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            size.to_string().parse().expect("numeric header"),
        );
        return response;
    }
    let requested = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let Some((start, end, partial)) = range(requested, size) else {
        *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes */{}", size).parse().expect("numeric header"),
        );
        return response;
    };
    let length = end - start + 1;
    let mut bytes = Vec::with_capacity(length as usize);
    if file.seek(SeekFrom::Start(start)).is_err()
        || file.take(length).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 != length
    {
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    }
    if partial {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", size)
                .parse()
                .expect("numeric header"),
        );
    }
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        length.to_string().parse().expect("numeric header"),
    );
    *response.body_mut() = bytes;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_file_tickets_are_registered_with_their_own_path_and_size() {
        let path =
            std::env::temp_dir().join(format!("spanseek-resource-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"%PDF-1.7\n").unwrap();
        let ticket = issue_file_ticket(
            path.clone(),
            "application/pdf",
            Duration::from_secs(30),
            "test",
        )
        .unwrap();
        let token = ticket.url.rsplit('/').next().unwrap().to_string();
        {
            let tickets = TICKETS
                .get_or_init(Default::default)
                .lock()
                .expect("ticket registry");
            let stored = tickets.get(&token).expect("registered ticket");
            assert_eq!(stored.size, 9);
            assert_eq!(stored.media_type, "application/pdf");
            match &stored.source {
                TicketSource::File(stored_path) => assert_eq!(stored_path, &path),
                TicketSource::Original { .. } => panic!("expected a generated file ticket"),
            }
        }
        lock(TICKETS.get_or_init(Default::default), "test")
            .unwrap()
            .remove(&token);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn ranges_are_single_bounded_and_never_read_outside_the_version() {
        assert_eq!(range(Some("bytes=2-5"), 10), Some((2, 5, true)));
        assert_eq!(range(Some("bytes=-3"), 10), Some((7, 9, true)));
        assert_eq!(range(Some("bytes=8-99"), 10), Some((8, 9, true)));
        assert_eq!(range(Some("bytes=10-"), 10), None);
        assert_eq!(range(Some("bytes=0-1,3-4"), 10), None);
        assert_eq!(range(None, MAX_RANGE + 1), None);
        assert_eq!(range(Some("bytes=0-16777216"), MAX_RANGE + 1), None);
    }
}
