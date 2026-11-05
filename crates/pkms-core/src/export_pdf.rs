//! Self-contained PDF rendering through the locally installed Edge engine.
//!
//! The HTML is written beside the destination, embeds every image as a data
//! URI, disables scripts and network access through CSP, and is removed after
//! the renderer finishes. No document content leaves the machine.
use super::html;
use crate::{BibliographyReport, Error, Note, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn render(
    note: &Note,
    assets: &BTreeMap<String, (String, Vec<u8>)>,
    citations: Option<&BibliographyReport>,
    parent: &Path,
) -> Result<Vec<u8>> {
    let inline_assets = assets
        .iter()
        .map(|(hash, (media_type, bytes))| {
            (
                hash.clone(),
                format!("data:{media_type};base64,{}", STANDARD.encode(bytes)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let no_paths = BTreeMap::new();
    let html = html::document(note, &no_paths, Some(&inline_assets), false, citations);
    let temporary = tempfile::Builder::new()
        .prefix(".pkms-pdf-")
        .tempdir_in(parent)?;
    let source = temporary.path().join("document.html");
    let output = temporary.path().join("document.pdf");
    let profile = temporary.path().join("edge-profile");
    fs::write(&source, html.as_bytes())?;
    let edge = find_edge().ok_or(Error::InvalidInput("pdf_renderer_unavailable"))?;
    let url = file_url(&source)?;
    let mut child = Command::new(edge)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--disable-sync")
        .arg("--disable-extensions")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--no-pdf-header-footer")
        .arg("--run-all-compositor-stages-before-draw")
        .arg("--virtual-time-budget=2000")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--print-to-pdf={}", output.display()))
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::InvalidInput("pdf_render_failed"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(Error::InvalidInput("pdf_render_failed"));
            }
            break;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidInput("pdf_render_timeout"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut stable = None;
    loop {
        if Instant::now() > deadline {
            return Err(Error::InvalidInput("pdf_render_failed"));
        }
        if let Ok(metadata) = fs::metadata(&output)
            && metadata.len() >= 5
        {
            if stable == Some(metadata.len()) {
                break;
            }
            stable = Some(metadata.len());
        }
        thread::sleep(Duration::from_millis(250));
    }
    let mut bytes = Vec::new();
    fs::File::open(&output)?.read_to_end(&mut bytes)?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(Error::InvalidInput("pdf_render_failed"));
    }
    Ok(bytes)
}

fn find_edge() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SPANSEEK_EDGE_PATH").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|base| base.join("Microsoft/Edge/Application/msedge.exe"))
        .find(|path| path.is_file())
}

fn file_url(path: &Path) -> Result<String> {
    let canonical = path.canonicalize()?;
    let raw = canonical.to_string_lossy();
    let without_verbatim = raw.strip_prefix("\\\\?\\").unwrap_or(&raw);
    let normalized = without_verbatim
        .strip_prefix("UNC\\")
        .map(|value| format!("//{value}"))
        .unwrap_or_else(|| without_verbatim.to_owned());
    let text = normalized.replace('\\', "/");
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(format!("file:///{}", encoded.trim_start_matches('/')))
}
