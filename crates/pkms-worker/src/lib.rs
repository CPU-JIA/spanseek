//! One trusted-app request per process. No database connection or network access.
use pkms_core::{ParsedSource, SourceFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

mod csv;
mod doc;
mod docx;
mod epub;
mod html;
mod mobi;
mod ocr;
mod pdf;
mod pptx;
mod xls;
mod xlsx;
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: u64 = 64 * 1024;
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_RESULT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PAGES: usize = 10_000;
pub const TIMEOUT_SECONDS: u64 = 120;
pub const RESULT_FILE: &str = "parsed.json";
pub const PDFIUM_DLL_SHA256: &str =
    "79d4676b656cfb1abcea88f9ade3b4b0826c5200382db5f4ec72a636c598c118";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRequest {
    pub protocol_version: u32,
    pub job_id: String,
    pub attempt: u32,
    pub lease_token: String,
    pub source_version_id: String,
    pub input_path: PathBuf,
    pub format: SourceFormat,
    pub output_dir: PathBuf,
    pub pdfium_library_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Optional local OCR engine. Its absence is not a parse failure: pages
    /// without a text layer stay flagged as `needs_ocr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<OcrConfig>,
}

/// A pinned local OCR engine invocation. Paths are supplied by the trusted app,
/// never by a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrConfig {
    pub executable: PathBuf,
    pub tessdata_dir: PathBuf,
    /// Tesseract language list, for example `chi_sim+eng`.
    pub languages: String,
    /// Rasterization resolution used only for pages without a text layer.
    pub dpi: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResponse {
    pub protocol_version: u32,
    pub job_id: String,
    pub attempt: u32,
    pub lease_token: String,
    pub source_version_id: String,
    pub result: WorkerResult,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerResult {
    Success {
        result_file: String,
        sha256: String,
        size: u64,
    },
    Failure {
        code: String,
    },
}
#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct WorkerError {
    pub code: &'static str,
}
pub(crate) fn error(code: &'static str) -> WorkerError {
    WorkerError { code }
}
type Result<T> = std::result::Result<T, WorkerError>;

pub fn read_bounded(mut reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > limit {
        return Err(error("size_limit"));
    }
    Ok(bytes)
}
pub fn read_request(reader: impl Read) -> Result<WorkerRequest> {
    let request: WorkerRequest = serde_json::from_slice(&read_bounded(reader, MAX_REQUEST_BYTES)?)
        .map_err(|_| error("invalid_request"))?;
    validate_request(&request)?;
    Ok(request)
}
fn validate_request(r: &WorkerRequest) -> Result<()> {
    if r.protocol_version != PROTOCOL_VERSION
        || r.attempt == 0
        || [&r.job_id, &r.lease_token, &r.source_version_id]
            .iter()
            .any(|v| uuid::Uuid::parse_str(v).is_err())
        || !r.input_path.is_absolute()
        || !r.output_dir.is_absolute()
        || r.pdfium_library_path
            .as_ref()
            .is_some_and(|p| !p.is_absolute())
    {
        return Err(error("invalid_request"));
    }
    if let Some(ocr) = &r.ocr
        && (!ocr.executable.is_absolute()
            || !ocr.tessdata_dir.is_absolute()
            || ocr.languages.is_empty()
            || ocr.languages.len() > 64
            || !ocr
                .languages
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
            || !(150..=400).contains(&ocr.dpi))
    {
        return Err(error("invalid_request"));
    }
    Ok(())
}
fn regular_file(path: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error("input_unavailable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(error("invalid_path"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(error("invalid_path"));
        }
    }
    File::open(path).map_err(|_| error("input_unavailable"))
}
pub fn execute(request: &WorkerRequest) -> WorkerResponse {
    let result = execute_inner(request).unwrap_or_else(|e| WorkerResult::Failure {
        code: e.code.into(),
    });
    WorkerResponse {
        protocol_version: PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        attempt: request.attempt,
        lease_token: request.lease_token.clone(),
        source_version_id: request.source_version_id.clone(),
        result,
    }
}
fn execute_inner(r: &WorkerRequest) -> Result<WorkerResult> {
    validate_request(r)?;
    let input = regular_file(&r.input_path)?;
    // Container formats are bounded by the input cap; text formats by the parse cap.
    let limit = if matches!(
        r.format,
        SourceFormat::Pdf
            | SourceFormat::Docx
            | SourceFormat::Doc
            | SourceFormat::Epub
            | SourceFormat::Mobi
            | SourceFormat::Xlsx
            | SourceFormat::Xls
            | SourceFormat::Pptx
    ) {
        MAX_INPUT_BYTES
    } else {
        MAX_TEXT_BYTES as u64
    };
    if input.metadata().map_err(|_| error("io_error"))?.len() > limit {
        return Err(error("input_limit"));
    }
    let bytes = read_bounded(input, limit)?;
    let parsed = match r.format {
        SourceFormat::Pdf => pdf::parse(r, &bytes)?,
        SourceFormat::Text | SourceFormat::Markdown => {
            if std::str::from_utf8(&bytes).is_err() {
                return Err(error("invalid_utf8"));
            }
            pkms_core::parse_text_source(&r.source_version_id, r.format, &bytes)
                .map_err(|_| error("invalid_text"))?
        }
        SourceFormat::Html => html::parse(r, &bytes)?,
        SourceFormat::Docx => docx::parse(r, &bytes)?,
        SourceFormat::Doc => doc::parse(r, &bytes)?,
        SourceFormat::Epub => epub::parse(r, &bytes)?,
        SourceFormat::Mobi => mobi::parse(r, &bytes)?,
        SourceFormat::Csv => csv::parse(r, &bytes)?,
        SourceFormat::Xlsx => xlsx::parse(r, &bytes)?,
        SourceFormat::Xls => xls::parse(r, &bytes)?,
        SourceFormat::Pptx => pptx::parse(r, &bytes)?,
    };
    // A capped writer stops serialization before a large output can be allocated or published.
    let path = r.output_dir.join(RESULT_FILE);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| error("output_unavailable"))?;
    let mut writer = ResultWriter {
        file,
        size: 0,
        hash: Sha256::new(),
    };
    serde_json::to_writer(&mut writer, &parsed).map_err(|_| error("output_limit_or_io"))?;
    writer.file.sync_all().map_err(|_| error("io_error"))?;
    Ok(WorkerResult::Success {
        result_file: RESULT_FILE.into(),
        sha256: format!("{:x}", writer.hash.finalize()),
        size: writer.size,
    })
}
struct ResultWriter {
    file: File,
    size: u64,
    hash: Sha256,
}
impl Write for ResultWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.size + buf.len() as u64 > MAX_RESULT_BYTES {
            return Err(std::io::Error::other("output limit"));
        }
        let count = self.file.write(buf)?;
        self.hash.update(&buf[..count]);
        self.size += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
/// Call only after the supervised child exited successfully; the app still commits through core lease validation.
pub fn read_validated_result(r: &WorkerRequest, response: &WorkerResponse) -> Result<ParsedSource> {
    if response.protocol_version != PROTOCOL_VERSION
        || response.job_id != r.job_id
        || response.attempt != r.attempt
        || response.lease_token != r.lease_token
        || response.source_version_id != r.source_version_id
    {
        return Err(error("worker_identity_mismatch"));
    }
    let WorkerResult::Success {
        result_file,
        sha256,
        size,
    } = &response.result
    else {
        return Err(error("worker_failed"));
    };
    if result_file != RESULT_FILE || *size > MAX_RESULT_BYTES {
        return Err(error("invalid_result"));
    }
    let bytes = read_bounded(
        regular_file(&r.output_dir.join(RESULT_FILE))?,
        MAX_RESULT_BYTES,
    )?;
    if bytes.len() as u64 != *size || format!("{:x}", Sha256::digest(&bytes)) != *sha256 {
        return Err(error("result_hash_mismatch"));
    }
    let parsed: ParsedSource =
        serde_json::from_slice(&bytes).map_err(|_| error("invalid_result"))?;
    if parsed.source_version_id != r.source_version_id {
        return Err(error("worker_identity_mismatch"));
    }
    Ok(parsed)
}
