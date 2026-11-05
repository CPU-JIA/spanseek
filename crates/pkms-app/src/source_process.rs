use crate::worker_process::WorkerChild;
use crate::{AppError, Result};
use pkms_core::{ParsedSource, SourceFormat, SourceJobLease, SourceVersion};
use pkms_worker::{WorkerRequest, WorkerResponse, WorkerResult};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use ts_rs::TS;

#[derive(Debug, Clone)]
pub struct SourceWorkerConfig {
    pub executable: PathBuf,
    pub pdfium_library_path: Option<PathBuf>,
    /// Optional local OCR engine; absent means scanned pages stay flagged.
    pub ocr: Option<pkms_worker::OcrConfig>,
    pub shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// A parse component this build needs and cannot use, with the path it loads from.
///
/// `code` is the stable `source_jobs.error_code` vocabulary the interface already
/// maps to an actionable sentence. "The component is unavailable" has to stay
/// distinguishable from "the parser started and then failed": the second case is a
/// worker failure, while a component the installation does not carry is an
/// environment problem that no retry can change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ParseComponentIssue {
    /// `worker_unavailable` or `pdfium_unavailable`.
    pub code: String,
    /// The exact location the application looked for, or `None` when this build was
    /// given no path at all for the component.
    pub path: Option<String>,
}

/// True when a configured component path cannot be loaded as it stands.
fn unusable_component(path: &Path) -> bool {
    !path.is_absolute() || !path.is_file()
}

/// Names a component path so the user can act on it: a relative path is resolved
/// against the working directory instead of being reported as a bare fragment.
fn absolute_component_path(path: &Path) -> String {
    if path.is_absolute() {
        return path.display().to_string();
    }
    match std::env::current_dir() {
        Ok(directory) => directory.join(path).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

impl SourceWorkerConfig {
    /// The stable parse component code `format` needs and this installation cannot
    /// use, if any.
    ///
    /// Callers check this before they store an original or start the worker, so a
    /// broken installation fails under its own code instead of surfacing as a parse
    /// failure of the user's document. A component that failed *after* it started
    /// keeps its own codes (`worker_failed`, `worker_timeout`,
    /// `worker_protocol_mismatch`), because those describe the document attempt
    /// rather than the installation.
    pub(crate) fn missing_parse_component(&self, format: SourceFormat) -> Option<&'static str> {
        if unusable_component(&self.executable) {
            return Some("worker_unavailable");
        }
        if format == SourceFormat::Pdf
            && self
                .pdfium_library_path
                .as_deref()
                .is_none_or(unusable_component)
        {
            return Some("pdfium_unavailable");
        }
        None
    }

    /// The same facts for the local diagnostics surface. This report names local
    /// paths, so it stays inside the application; an exported diagnostics bundle is
    /// built from the sanitized snapshot instead.
    pub fn parse_component_issues(&self) -> Vec<ParseComponentIssue> {
        let mut issues = Vec::new();
        if unusable_component(&self.executable) {
            issues.push(ParseComponentIssue {
                code: "worker_unavailable".into(),
                path: Some(absolute_component_path(&self.executable)),
            });
        }
        match &self.pdfium_library_path {
            Some(path) if unusable_component(path) => issues.push(ParseComponentIssue {
                code: "pdfium_unavailable".into(),
                path: Some(absolute_component_path(path)),
            }),
            Some(_) => {}
            None => issues.push(ParseComponentIssue {
                code: "pdfium_unavailable".into(),
                path: None,
            }),
        }
        issues
    }
}
#[cfg(test)]
thread_local! { static LAST_CHILD_ID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) }; }
fn worker_failure_code(code: &str) -> &'static str {
    match code {
        "invalid_utf8" => "invalid_utf8",
        "invalid_text" => "invalid_text",
        "pdfium_unavailable" => "pdfium_unavailable",
        "pdf_password_required" => "pdf_password_required",
        "pdfium_hash_mismatch" => "pdfium_hash_mismatch",
        "pdf_invalid" => "pdf_invalid",
        "pdf_parse_failed" => "pdf_parse_failed",
        "pdf_page_limit" => "pdf_page_limit",
        "pdf_text_limit" => "pdf_text_limit",
        "pdf_invalid_unicode" => "pdf_invalid_unicode",
        "input_limit" => "input_limit",
        "pdf_limit" => "pdf_limit",
        "unsupported_format" => "unsupported_format",
        "output_limit_or_io" => "output_limit_or_io",
        "size_limit" => "size_limit",
        "input_unavailable" => "input_unavailable",
        "io_error" => "io_error",
        "output_unavailable" => "output_unavailable",
        "html_input_limit" => "html_input_limit",
        "html_structure_limit" => "html_structure_limit",
        "html_invalid_encoding" => "html_invalid_encoding",
        "html_text_limit" => "html_text_limit",
        "html_no_content" => "html_no_content",
        "webpage_login_required" => "webpage_login_required",
        "webpage_dynamic_content" => "webpage_dynamic_content",
        "docx_not_container" => "docx_not_container",
        "docx_part_missing" => "docx_part_missing",
        "docx_invalid_xml" => "docx_invalid_xml",
        "docx_no_content" => "docx_no_content",
        "docx_input_limit" => "docx_input_limit",
        "docx_text_limit" => "docx_text_limit",
        "docx_structure_limit" => "docx_structure_limit",
        "doc_not_container" => "doc_not_container",
        "doc_invalid" => "doc_invalid",
        "doc_encrypted" => "doc_encrypted",
        "doc_no_content" => "doc_no_content",
        "doc_text_limit" => "doc_text_limit",
        "doc_structure_limit" => "doc_structure_limit",
        "doc_asset_limit" => "doc_asset_limit",
        "epub_not_container" => "epub_not_container",
        "epub_container_missing" => "epub_container_missing",
        "epub_part_missing" => "epub_part_missing",
        "epub_invalid_xml" => "epub_invalid_xml",
        "epub_no_content" => "epub_no_content",
        "epub_input_limit" => "epub_input_limit",
        "epub_text_limit" => "epub_text_limit",
        "epub_structure_limit" => "epub_structure_limit",
        "csv_not_utf8" => "csv_not_utf8",
        "csv_invalid_text" => "csv_invalid_text",
        "csv_no_content" => "csv_no_content",
        "csv_text_limit" => "csv_text_limit",
        "csv_structure_limit" => "csv_structure_limit",
        "xlsx_not_container" => "xlsx_not_container",
        "xlsx_part_missing" => "xlsx_part_missing",
        "xlsx_invalid_xml" => "xlsx_invalid_xml",
        "xlsx_no_content" => "xlsx_no_content",
        "xlsx_input_limit" => "xlsx_input_limit",
        "xlsx_text_limit" => "xlsx_text_limit",
        "xlsx_structure_limit" => "xlsx_structure_limit",
        "xls_not_container" => "xls_not_container",
        "xls_invalid_workbook" => "xls_invalid_workbook",
        "xls_no_content" => "xls_no_content",
        "xls_text_limit" => "xls_text_limit",
        "xls_structure_limit" => "xls_structure_limit",
        "pptx_not_container" => "pptx_not_container",
        "pptx_part_missing" => "pptx_part_missing",
        "pptx_invalid_xml" => "pptx_invalid_xml",
        "pptx_no_content" => "pptx_no_content",
        "pptx_input_limit" => "pptx_input_limit",
        "pptx_text_limit" => "pptx_text_limit",
        "pptx_structure_limit" => "pptx_structure_limit",
        "mobi_invalid_container" => "mobi_invalid_container",
        "mobi_encrypted" => "mobi_encrypted",
        "mobi_invalid_text" => "mobi_invalid_text",
        "mobi_no_content" => "mobi_no_content",
        "mobi_text_limit" => "mobi_text_limit",
        "mobi_structure_limit" => "mobi_structure_limit",
        "mobi_asset_limit" => "mobi_asset_limit",
        _ => "parse_failed",
    }
}

pub(crate) fn parse(
    config: &SourceWorkerConfig,
    lease: &SourceJobLease,
    version: &SourceVersion,
    input: &Path,
    output: &Path,
    checkpoint: &mut dyn FnMut() -> Result<()>,
) -> Result<ParsedSource> {
    let _permit = crate::inference_resources::heavy_resource_slot().acquire(checkpoint)?;
    if let Some(code) = config.missing_parse_component(version.format) {
        return Err(AppError::InvalidInput(code));
    }
    let request = WorkerRequest {
        protocol_version: pkms_worker::PROTOCOL_VERSION,
        job_id: lease.job.id.clone(),
        attempt: lease.job.attempt,
        lease_token: lease.lease_token.clone(),
        source_version_id: version.id.clone(),
        input_path: input.to_owned(),
        format: version.format,
        output_dir: output.to_owned(),
        pdfium_library_path: config.pdfium_library_path.clone(),
        ocr: config.ocr.clone(),
        content_type: version
            .web_capture
            .as_ref()
            .map(|capture| capture.content_type.clone()),
    };
    let bytes = serde_json::to_vec(&request)?;
    if bytes.len() as u64 > pkms_worker::MAX_REQUEST_BYTES {
        return Err(AppError::InvalidInput("worker_request_limit"));
    }
    checkpoint()?;
    let mut command = Command::new(&config.executable);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A component the system refuses to start is unavailable in the same way as one
    // that is not there: the file to repair is the parse component, not the user's
    // document, so it keeps `worker_unavailable` instead of the generic I/O code.
    // `worker_failed` stays reserved for a worker that started and then exited.
    let mut child = WorkerChild::spawn(&mut command)
        .map_err(|_| AppError::InvalidInput("worker_unavailable"))?;
    #[cfg(test)]
    LAST_CHILD_ID.set(Some(child.id()));
    let mut stdin = child
        .stdin
        .take()
        .ok_or(AppError::InvalidInput("worker_pipe_failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(AppError::InvalidInput("worker_pipe_failed"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(AppError::InvalidInput("worker_pipe_failed"))?;
    let input_writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let error_reader = std::thread::spawn(move || pkms_worker::read_bounded(stderr, 4096));
    let output_reader = std::thread::spawn(move || {
        pkms_worker::read_bounded(stdout, pkms_worker::MAX_RESPONSE_BYTES)
    });
    let start = Instant::now();
    let supervised = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(AppError::InvalidInput("worker_failed"));
                }
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(pkms_worker::TIMEOUT_SECONDS) {
                return Err(AppError::InvalidInput("worker_timeout"));
            }
            checkpoint()?;
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    // Always release the process tree and reap before joining pipe threads,
    // including successful completion, cancellation and timeout.
    let _ = child.kill();
    let _ = child.wait();
    let input_result = input_writer
        .join()
        .map_err(|_| AppError::InvalidInput("worker_pipe_failed"));
    let output_result = output_reader
        .join()
        .map_err(|_| AppError::InvalidInput("worker_pipe_failed"));
    let error_output = error_reader.join().ok().and_then(std::result::Result::ok);
    if matches!(supervised, Err(AppError::InvalidInput("worker_failed")))
        && error_output
            .as_deref()
            .is_some_and(|bytes| bytes == b"invalid_request\n" || bytes == b"invalid_request\r\n")
    {
        return Err(AppError::InvalidInput("worker_protocol_mismatch"));
    }
    supervised?;
    input_result??;
    let bytes = output_result?.map_err(|error| AppError::InvalidInput(error.code))?;
    let response: WorkerResponse = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::InvalidInput("invalid_worker_response"))?;
    if response.protocol_version != request.protocol_version
        || response.job_id != request.job_id
        || response.attempt != request.attempt
        || response.lease_token != request.lease_token
        || response.source_version_id != request.source_version_id
    {
        return Err(AppError::InvalidInput("worker_identity_mismatch"));
    }
    if let WorkerResult::Failure { code } = &response.result {
        return Err(AppError::InvalidInput(worker_failure_code(code)));
    }
    pkms_worker::read_validated_result(&request, &response)
        .map_err(|error| AppError::InvalidInput(error.code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_executable() -> PathBuf {
        let suffix = if cfg!(windows) {
            "pkms-worker.exe"
        } else {
            "pkms-worker"
        };
        let mut candidates = Vec::new();
        if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
            candidates.push(PathBuf::from(dir).join("debug"));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(debug) = exe.parent().and_then(|deps| deps.parent())
        {
            candidates.push(debug.to_path_buf());
        }
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug"));
        let searched = format!("{candidates:?}");
        candidates
            .into_iter()
            .map(|dir| dir.join(suffix))
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("build pkms-worker first: no {suffix} in {searched}"))
            .canonicalize()
            .expect("canonicalize pkms-worker")
    }

    #[test]
    fn a_component_this_installation_cannot_use_is_reported_with_its_exact_path() {
        let root = tempfile::tempdir().unwrap();
        let stub = |executable: PathBuf, pdfium: Option<PathBuf>| SourceWorkerConfig {
            executable,
            pdfium_library_path: pdfium,
            ocr: None,
            shutdown: Default::default(),
        };
        // A component that is not there is reported by path, and its code stays
        // distinct from a worker that started and failed (`worker_failed`).
        let absent = stub(root.path().join("spanseek-worker.exe"), None);
        assert_eq!(
            absent.missing_parse_component(SourceFormat::Text),
            Some("worker_unavailable")
        );
        assert_eq!(
            absent.parse_component_issues(),
            vec![
                ParseComponentIssue {
                    code: "worker_unavailable".into(),
                    path: Some(
                        root.path()
                            .join("spanseek-worker.exe")
                            .display()
                            .to_string()
                    ),
                },
                ParseComponentIssue {
                    code: "pdfium_unavailable".into(),
                    path: None,
                },
            ]
        );
        // A relative path would not be actionable, so the report resolves it.
        let relative = stub(PathBuf::from("spanseek-worker.exe"), None);
        assert!(
            relative.parse_component_issues()[0]
                .path
                .as_deref()
                .is_some_and(|path| Path::new(path).is_absolute())
        );
        // A PDF needs the PDF engine on top of the parser.
        let worker = root.path().join("spanseek-worker.exe");
        std::fs::write(&worker, b"stub").unwrap();
        let pdfium = root.path().join("pdfium.dll");
        let without_pdfium = stub(worker.clone(), Some(pdfium.clone()));
        assert!(
            without_pdfium
                .missing_parse_component(SourceFormat::Text)
                .is_none()
        );
        assert_eq!(
            without_pdfium.missing_parse_component(SourceFormat::Pdf),
            Some("pdfium_unavailable")
        );
        assert_eq!(
            without_pdfium.parse_component_issues(),
            vec![ParseComponentIssue {
                code: "pdfium_unavailable".into(),
                path: Some(pdfium.display().to_string()),
            }]
        );
        // With every component in place the job goes on to the worker itself.
        std::fs::write(&pdfium, b"stub").unwrap();
        let ready = stub(worker, Some(pdfium));
        assert!(ready.missing_parse_component(SourceFormat::Pdf).is_none());
        assert!(ready.parse_component_issues().is_empty());
    }
    #[test]
    fn structured_format_worker_failures_keep_their_specific_codes() {
        for code in [
            "epub_not_container",
            "epub_container_missing",
            "epub_part_missing",
            "epub_invalid_xml",
            "epub_no_content",
            "epub_input_limit",
            "epub_text_limit",
            "epub_structure_limit",
            "doc_not_container",
            "doc_invalid",
            "doc_encrypted",
            "doc_no_content",
            "doc_text_limit",
            "doc_structure_limit",
            "doc_asset_limit",
            "csv_not_utf8",
            "csv_invalid_text",
            "csv_no_content",
            "csv_text_limit",
            "csv_structure_limit",
            "xlsx_not_container",
            "xlsx_part_missing",
            "xlsx_invalid_xml",
            "xlsx_no_content",
            "xlsx_input_limit",
            "xlsx_text_limit",
            "xlsx_structure_limit",
            "xls_not_container",
            "xls_invalid_workbook",
            "xls_no_content",
            "xls_text_limit",
            "xls_structure_limit",
            "pptx_not_container",
            "pptx_part_missing",
            "pptx_invalid_xml",
            "pptx_no_content",
            "pptx_input_limit",
            "pptx_text_limit",
            "pptx_structure_limit",
            "mobi_invalid_container",
            "mobi_encrypted",
            "mobi_invalid_text",
            "mobi_no_content",
            "mobi_text_limit",
            "mobi_structure_limit",
            "mobi_asset_limit",
        ] {
            assert_eq!(worker_failure_code(code), code);
        }
        assert_eq!(
            worker_failure_code("future_unknown_failure"),
            "parse_failed"
        );
    }
    #[test]
    fn cancellation_after_real_child_spawn_kills_and_reaps_before_returning() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("original.txt");
        std::fs::write(&input, vec![b'a'; 16 * 1024 * 1024]).unwrap();
        let executable = worker_executable();
        let config = SourceWorkerConfig {
            executable,
            pdfium_library_path: None,
            ocr: None,
            shutdown: Default::default(),
        };
        let uuid = || uuid::Uuid::new_v4().to_string();
        let version = pkms_core::SourceVersion {
            id: uuid(),
            source_id: uuid(),
            blob_hash: "unused".into(),
            original_name: "original.txt".into(),
            origin_url: None,
            captured_at: String::new(),
            format: pkms_core::SourceFormat::Text,
            size: 16 * 1024 * 1024,
            web_capture: None,
        };
        let lease = pkms_core::SourceJobLease {
            job: pkms_core::SourceJob {
                id: uuid(),
                input: pkms_core::ImportSource {
                    input_token: uuid(),
                    original_name: "original.txt".into(),
                    format: pkms_core::SourceFormat::Text,
                    origin_url: None,
                    collection_id: None,
                    add_version_to: None,
                    base_row_version: None,
                },
                tag_names: Vec::new(),
                status: pkms_core::JobStatus::Running,
                stage: "parsing".into(),
                attempt: 1,
                config_revision: 1,
                cancel_requested: false,
                source_id: Some(version.source_id.clone()),
                source_version_id: Some(version.id.clone()),
                error_code: None,
                updated_at: String::new(),
            },
            lease_token: uuid(),
            expires_at: String::new(),
        };
        LAST_CHILD_ID.set(None);
        let mut checkpoint = || {
            if LAST_CHILD_ID.get().is_some() {
                Err(AppError::InvalidInput("worker_interrupted"))
            } else {
                Ok(())
            }
        };
        let started = Instant::now();
        assert!(matches!(
            parse(
                &config,
                &lease,
                &version,
                &input,
                root.path(),
                &mut checkpoint
            ),
            Err(AppError::InvalidInput("worker_interrupted"))
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid = LAST_CHILD_ID
            .get()
            .expect("a real worker child was spawned");
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            }
            // Independent OS observation after the supervisor returned, not a synthetic mock.
            unsafe {
                let process = OpenProcess(0x1000, 0, pid);
                if !process.is_null() {
                    let mut code = 259;
                    let read = GetExitCodeProcess(process, &mut code);
                    CloseHandle(process);
                    assert_ne!(read, 0);
                    assert_ne!(code, 259, "worker must no longer be STILL_ACTIVE");
                }
            }
        }
        #[cfg(target_os = "linux")]
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = pid;
    }
}
