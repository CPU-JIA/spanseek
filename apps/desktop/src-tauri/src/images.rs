use crate::{
    runtime::{DesktopRuntime, lock},
    thumbnail,
};
use pkms_app::{ApiError, ApiErrorCode, WorkspaceIdentity};
use tauri::http::{Request, Response, StatusCode, header};

pub fn read_image(runtime: &DesktopRuntime, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let result = (|| {
        if request.method() != "GET"
            || request.uri().query().is_some()
            || request.uri().path().len() > 512
        {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
        }
        let parts: Vec<_> = request
            .uri()
            .path()
            .trim_start_matches('/')
            .split('/')
            .collect();
        let [instance, workspace, kind, rest @ ..] = parts.as_slice() else {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
        };
        let identity = WorkspaceIdentity {
            local_instance_id: (*instance).into(),
            workspace_id: (*workspace).into(),
        };
        if let ("thumbnail", [revision, hash]) = (*kind, rest) {
            if request.uri().path().starts_with("//") {
                return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
            }
            let bytes = thumbnail::read(&identity, revision, hash, || {
                let service = lock(&runtime.service, "")?;
                let library = service
                    .library(&identity)
                    .map_err(|error| ApiError::from_app(error, ""))?;
                library
                    .attachment(revision, hash)
                    .map_err(|error| ApiError::from_app(error.into(), ""))
            })?;
            return Ok(("image/png".to_owned(), bytes));
        }
        let service = lock(&runtime.service, "")?;
        let library = service
            .library(&identity)
            .map_err(|error| ApiError::from_app(error, ""))?;
        let result = match (*kind, rest) {
            ("preview", [token]) => library.attachment_preview_resource(token),
            ("revision", [revision, hash]) => library.attachment(revision, hash),
            _ => return Err(ApiError::new(ApiErrorCode::InvalidInput, "")),
        };
        result
            .map(|(info, bytes)| (info.media_type, bytes))
            .map_err(|error| ApiError::from_app(error.into(), ""))
    })();
    let (status, mime, bytes) = match result {
        Ok((mime, bytes)) => (StatusCode::OK, mime, bytes),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "text/plain".into(),
            b"Image unavailable".to_vec(),
        ),
    };
    let mut response = Response::new(bytes);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    // MIME comes only from the verified core decoder, never a filename or request.
    if let Ok(value) = mime.parse() {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static header"),
    );
    response
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
