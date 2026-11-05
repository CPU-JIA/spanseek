//! Public webpage snapshots: one bounded request chain, no browser cookies or scripts.
use crate::{AppError, Result};
use pkms_core::WebCaptureInput;
use std::{
    future::Future,
    io::{Seek, SeekFrom, Write},
    time::Duration,
};

pub(crate) const MAX_WEB_BYTES: u64 = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) struct CapturedWebpage {
    pub file: tempfile::NamedTempFile,
    pub capture: WebCaptureInput,
}

pub(crate) fn validate_url(value: &str) -> Result<reqwest::Url> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(AppError::InvalidInput("webpage_invalid_url"));
    }
    let url =
        reqwest::Url::parse(value).map_err(|_| AppError::InvalidInput("webpage_invalid_url"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::InvalidInput("webpage_invalid_url"));
    }
    Ok(url)
}
async fn checked<T>(
    future: impl Future<Output = std::result::Result<T, reqwest::Error>>,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<T> {
    let mut future = std::pin::pin!(future);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut future => return result.map_err(|error| AppError::InvalidInput(if error.is_timeout() { "webpage_timeout" } else { "webpage_network_failed" })),
            _ = interval.tick() => check()?,
        }
    }
}
pub(crate) fn capture(url: &str, check: &mut dyn FnMut() -> Result<()>) -> Result<CapturedWebpage> {
    let url = validate_url(url)?;
    check()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        tokio::time::timeout(TOTAL_TIMEOUT, fetch(url, check))
            .await
            .map_err(|_| AppError::InvalidInput("webpage_timeout"))?
    });
    // An OS DNS lookup may be a non-cancellable blocking task. Do not let runtime
    // teardown make user cancellation wait indefinitely for the resolver.
    runtime.shutdown_timeout(Duration::from_millis(250));
    result
}
async fn fetch(
    mut url: reqwest::Url,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<CapturedWebpage> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent("spanseek/0.1 (local webpage capture)")
        .connect_timeout(Duration::from_secs(8))
        .read_timeout(Duration::from_secs(8))
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|_| AppError::InvalidInput("webpage_network_failed"))?;
    let mut seen = std::collections::HashSet::new();
    for redirects in 0..=MAX_REDIRECTS {
        check()?;
        if !seen.insert(url.as_str().to_owned()) {
            return Err(AppError::InvalidInput("webpage_redirect_loop"));
        }
        let mut response = checked(
            client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "text/html, application/xhtml+xml")
                .send(),
            check,
        )
        .await?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if redirects == MAX_REDIRECTS {
                return Err(AppError::InvalidInput("webpage_redirect_limit"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(AppError::InvalidInput("webpage_invalid_redirect"))?;
            let next = url
                .join(location)
                .map_err(|_| AppError::InvalidInput("webpage_invalid_redirect"))?;
            url = validate_url(next.as_str())?;
            continue;
        }
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(AppError::InvalidInput("webpage_login_required"));
        }
        if !response.status().is_success() {
            return Err(AppError::InvalidInput("webpage_http_error"));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let media = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(media.as_str(), "text/html" | "application/xhtml+xml") {
            return Err(AppError::InvalidInput("webpage_unsupported_content_type"));
        }
        if content_type.len() > 512 {
            return Err(AppError::InvalidInput("webpage_unsupported_content_type"));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_WEB_BYTES)
        {
            return Err(AppError::InvalidInput("webpage_size_limit"));
        }
        let capture = WebCaptureInput {
            final_url: url.as_str().to_owned(),
            http_status: response.status().as_u16(),
            content_type,
        };
        let mut file = tempfile::Builder::new()
            .prefix("pkms-web-capture-")
            .suffix(".html")
            .tempfile()?;
        let mut size = 0;
        while let Some(chunk) = checked(response.chunk(), check).await? {
            size += chunk.len() as u64;
            if size > MAX_WEB_BYTES {
                return Err(AppError::InvalidInput("webpage_size_limit"));
            }
            file.write_all(&chunk)?;
        }
        check()?;
        if size == 0 {
            return Err(AppError::InvalidInput("webpage_empty"));
        }
        file.as_file().sync_all()?;
        file.seek(SeekFrom::Start(0))?;
        return Ok(CapturedWebpage { file, capture });
    }
    Err(AppError::InvalidInput("webpage_redirect_limit"))
}
