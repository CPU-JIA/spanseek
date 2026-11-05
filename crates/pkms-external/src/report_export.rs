//! Offline report exporters for Markdown, DOCX and PDF.
//!
//! PDF uses a locally installed Microsoft Edge in headless mode with background
//! networking disabled. The generated HTML is self-contained. If the renderer
//! is missing the export fails explicitly instead of producing a fake PDF.
use pkms_core::{Library, ReportDraft, ReportVerification};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Markdown,
    Docx,
    Pdf,
}

impl ReportFormat {
    pub fn parse(value: &str) -> Result<Self, ExportError> {
        match value {
            "md" | "markdown" => Ok(Self::Markdown),
            "docx" => Ok(Self::Docx),
            "pdf" => Ok(Self::Pdf),
            _ => Err(ExportError::InvalidFormat(value.into())),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("report encoding error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("report is invalid: {0}")]
    Core(#[from] pkms_core::Error),
    #[error("invalid export format: {0}")]
    InvalidFormat(String),
    #[error("output already exists: {0}")]
    OutputExists(String),
    #[error("duplicate output path for report: {0}")]
    DuplicateOutput(String),
    #[error("local PDF renderer is unavailable")]
    RendererUnavailable,
    #[error("local PDF renderer failed: {0}")]
    RendererFailed(String),
    #[error("DOCX package error: {0}")]
    Zip(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFile {
    pub report: String,
    pub format: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportManifest {
    pub schema_version: u32,
    pub created_at: String,
    pub files: Vec<ExportFile>,
    pub warnings: Vec<String>,
    pub citations: Vec<ReportVerification>,
}

pub struct PreparedReport {
    pub source: PathBuf,
    pub report: ReportDraft,
}

pub fn export_reports(
    library: &Library,
    reports: Vec<PreparedReport>,
    formats: &[ReportFormat],
    output_dir: &Path,
) -> Result<ExportManifest, ExportError> {
    if reports.is_empty() {
        return Err(ExportError::InvalidFormat("no input reports".into()));
    }
    if formats.is_empty() {
        return Err(ExportError::InvalidFormat("no output formats".into()));
    }
    fs::create_dir_all(output_dir)?;
    let mut unique_formats = Vec::new();
    for format in formats {
        if !unique_formats.contains(format) {
            unique_formats.push(*format);
        }
    }
    let mut verifications = Vec::new();
    let mut plans = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for prepared in &reports {
        let verification = library.verify_report(&prepared.report)?;
        if !verification.all_valid {
            return Err(ExportError::Core(pkms_core::Error::InvalidInput(
                "report_citation_verification_failed",
            )));
        }
        verifications.push(verification);
        let stem = output_stem(&prepared.source)?;
        for format in &unique_formats {
            let path = output_dir.join(format!("{stem}.{}", format.extension()));
            let key = path.to_string_lossy().to_ascii_lowercase();
            if !seen.insert(key) {
                return Err(ExportError::DuplicateOutput(stem.clone()));
            }
            if path.exists() {
                return Err(ExportError::OutputExists(path.display().to_string()));
            }
            plans.push((prepared, *format, path));
        }
    }
    let manifest_path = output_dir.join("export-manifest.json");
    if manifest_path.exists() {
        return Err(ExportError::OutputExists(
            manifest_path.display().to_string(),
        ));
    }

    let mut files = Vec::new();
    let mut created = Vec::new();
    let result = (|| {
        for (prepared, format, path) in &plans {
            match format {
                ReportFormat::Markdown => write_new(path, prepared.report.markdown.as_bytes())?,
                ReportFormat::Docx => export_docx(&prepared.report, path)?,
                ReportFormat::Pdf => export_pdf(&prepared.report, path)?,
            }
            created.push(path.clone());
            let bytes = fs::read(path)?;
            files.push(ExportFile {
                report: prepared.report.title.clone(),
                format: format.label().into(),
                path: path.display().to_string(),
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        }
        let mut warnings = vec![
            "导出前已逐条回查引用；任一引用失效会阻止整批输出。".into(),
            "Markdown 可编辑并保留引用编号；DOCX 保留标题层级、段落、列表文本和引用来源；PDF 为固定版式。".into(),
            "DOCX/PDF 不保留 spanseek 块 UUID、交互编辑能力或应用内主题；交付后请以来源清单核对原文。".into(),
        ];
        if unique_formats.contains(&ReportFormat::Pdf) {
            warnings.push(
                "PDF 使用本机 Microsoft Edge 无头离线打印；若本机无 Edge 会明确失败，不生成替代文件。"
                    .into(),
            );
        }
        let manifest = ExportManifest {
            schema_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            files: files.clone(),
            warnings,
            citations: verifications.clone(),
        };
        let bytes = format!("{}\n", serde_json::to_string_pretty(&manifest)?);
        write_new(&manifest_path, bytes.as_bytes())?;
        created.push(manifest_path.clone());
        Ok(manifest)
    })();
    if result.is_err() {
        for path in created.iter().rev() {
            let _ = fs::remove_file(path);
        }
    }
    result
}

fn output_stem(path: &Path) -> Result<String, ExportError> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("report");
    let cleaned = stem
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.') {
                value
            } else {
                '_'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        Err(ExportError::InvalidFormat("empty report file name".into()))
    } else {
        Ok(cleaned)
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn export_docx(report: &ReportDraft, path: &Path) -> Result<(), ExportError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    {
        let mut zip = ZipWriter::new(&mut file);
        let stored = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o600);
        for (name, contents) in docx_parts(report) {
            zip.start_file(name, stored)
                .map_err(|error| ExportError::Zip(error.to_string()))?;
            zip.write_all(contents.as_bytes())?;
        }
        zip.finish()
            .map_err(|error| ExportError::Zip(error.to_string()))?;
    }
    file.sync_all()?;
    Ok(())
}

fn docx_parts(report: &ReportDraft) -> Vec<(&'static str, String)> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#.to_owned();
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#.to_owned();
    let document_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#.to_owned();
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos" w:eastAsia="Microsoft YaHei"/><w:sz w:val="22"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="360" w:lineRule="auto"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="36"/></w:rPr><w:pPr><w:spacing w:before="240" w:after="160"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="30"/></w:rPr><w:pPr><w:spacing w:before="200" w:after="120"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="26"/></w:rPr><w:pPr><w:spacing w:before="160" w:after="100"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:rPr><w:i/></w:rPr><w:pPr><w:ind w:left="720"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/></w:pPr></w:style>
</w:styles>"#.to_owned();
    let document = docx_document(report);
    vec![
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("word/_rels/document.xml.rels", document_rels),
        ("word/styles.xml", styles),
        ("word/document.xml", document),
    ]
}

fn docx_document(report: &ReportDraft) -> String {
    let mut body = String::new();
    for line in report.markdown.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            body.push_str("<w:p/>");
            continue;
        }
        let (style, text): (&str, String) = if let Some(value) = trimmed.strip_prefix("### ") {
            ("Heading3", value.to_owned())
        } else if let Some(value) = trimmed.strip_prefix("## ") {
            ("Heading2", value.to_owned())
        } else if let Some(value) = trimmed.strip_prefix("# ") {
            ("Heading1", value.to_owned())
        } else if let Some(value) = trimmed.strip_prefix("> ") {
            ("Quote", value.to_owned())
        } else if let Some(value) = trimmed.strip_prefix("- ") {
            ("ListParagraph", format!("• {value}"))
        } else {
            ("Normal", trimmed.to_owned())
        };
        body.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            xml_escape(&text)
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr></w:body></w:document>"
    )
}

fn export_pdf(report: &ReportDraft, path: &Path) -> Result<(), ExportError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let token = uuid::Uuid::new_v4().to_string();
    let html_path = parent.join(format!(".report-{token}.html"));
    let pdf_temp = parent.join(format!(".report-{token}.pdf"));
    let profile_dir = parent.join(format!(".edge-{token}"));
    let result = (|| {
        let html = html_document(report);
        write_new(&html_path, html.as_bytes())?;
        let edge = find_edge().ok_or(ExportError::RendererUnavailable)?;
        let url = file_url(&html_path)?;
        let mut command = Command::new(edge);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-sync")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--no-pdf-header-footer")
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg(format!("--print-to-pdf={}", pdf_temp.display()))
            .arg(&url)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| ExportError::RendererFailed(error.to_string()))?;
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(ExportError::RendererFailed(format!(
                        "Edge exited with {status}"
                    )));
                }
                break;
            }
            if started.elapsed() > Duration::from_secs(60) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExportError::RendererFailed("Edge timed out".into()));
            }
            thread::sleep(Duration::from_millis(100));
        }
        // Edge can hand off to a child writer and exit before the PDF is fully
        // flushed. Wait for a non-empty, size-stable file before validating it.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut stable_size = None;
        loop {
            if Instant::now() > deadline {
                return Err(ExportError::RendererFailed(
                    "renderer did not produce a PDF".into(),
                ));
            }
            if let Ok(metadata) = fs::metadata(&pdf_temp)
                && metadata.len() >= 5
            {
                if stable_size == Some(metadata.len()) {
                    break;
                }
                stable_size = Some(metadata.len());
            }
            thread::sleep(Duration::from_millis(300));
        }
        let mut header = [0_u8; 5];
        File::open(&pdf_temp)?.read_exact(&mut header)?;
        if &header != b"%PDF-" {
            return Err(ExportError::RendererFailed(
                "renderer produced a non-PDF file".into(),
            ));
        }
        fs::rename(&pdf_temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&html_path);
    let _ = fs::remove_file(&pdf_temp);
    let _ = fs::remove_dir_all(&profile_dir);
    result
}

fn find_edge() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SPANSEEK_EDGE_PATH").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let mut candidates = Vec::new();
    for root in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(root) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn file_url(path: &Path) -> Result<String, ExportError> {
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

fn html_document(report: &ReportDraft) -> String {
    let mut body = String::new();
    let mut in_list = false;
    for line in report.markdown.lines() {
        let trimmed = line.trim_end();
        let is_list = trimmed.starts_with("- ");
        if in_list && !is_list {
            body.push_str("</ul>");
            in_list = false;
        }
        if trimmed.is_empty() {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("### ") {
            body.push_str(&format!("<h3>{}</h3>", html_escape(value)));
        } else if let Some(value) = trimmed.strip_prefix("## ") {
            body.push_str(&format!("<h2>{}</h2>", html_escape(value)));
        } else if let Some(value) = trimmed.strip_prefix("# ") {
            body.push_str(&format!("<h1>{}</h1>", html_escape(value)));
        } else if let Some(value) = trimmed.strip_prefix("> ") {
            body.push_str(&format!("<blockquote>{}</blockquote>", html_escape(value)));
        } else if let Some(value) = trimmed.strip_prefix("- ") {
            if !in_list {
                body.push_str("<ul>");
                in_list = true;
            }
            body.push_str(&format!("<li>{}</li>", html_escape(value)));
        } else {
            body.push_str(&format!("<p>{}</p>", html_escape(trimmed)));
        }
    }
    if in_list {
        body.push_str("</ul>");
    }
    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>{}</title><style>@page{{size:A4;margin:18mm}}body{{font-family:'Microsoft YaHei','SimSun',sans-serif;font-size:11pt;line-height:1.65;color:#111}}h1{{font-size:22pt}}h2{{font-size:16pt;margin-top:1.2em}}h3{{font-size:13pt;margin-top:1em}}blockquote{{margin:0.6em 0;padding:0.2em 0.8em;border-left:3px solid #888;color:#333}}li{{margin:0.25em 0}}</style></head><body>{body}</body></html>",
        html_escape(&report.title)
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn html_escape(value: &str) -> String {
    xml_escape(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkms_core::{ReportCitation, ReportSource};

    fn report() -> ReportDraft {
        ReportDraft {
            schema_version: 1,
            title: "测试 <报告>".into(),
            question: "问题".into(),
            generated_by: "evidence_extractive".into(),
            template_name: "test".into(),
            markdown: "# 测试 <报告>\n\n> 问题\n\n## 章节\n\n证据 [E1]\n\n## 来源\n- [E1] 证据"
                .into(),
            citations: Vec::<ReportCitation>::new(),
            sources: Vec::<ReportSource>::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn docx_parts_escape_text_and_include_required_office_parts() {
        let parts = docx_parts(&report());
        let names = parts.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert!(names.contains(&"[Content_Types].xml"));
        assert!(names.contains(&"word/document.xml"));
        assert!(names.contains(&"word/styles.xml"));
        let document = &parts[4].1;
        assert!(document.contains("测试 &lt;报告&gt;"));
        assert!(document.contains("Heading2"));
    }

    #[test]
    fn pdf_html_is_self_contained() {
        let html = html_document(&report());
        assert!(html.contains("charset=\"utf-8\""));
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
        assert!(html.contains("Microsoft YaHei"));
    }

    #[test]
    fn output_stem_and_file_url_are_bounded_and_encoded() {
        assert_eq!(
            output_stem(Path::new("report name?.json")).unwrap(),
            "report_name_"
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a b.html");
        fs::write(&path, b"fixture").unwrap();
        let encoded = file_url(&path).unwrap();
        assert!(encoded.starts_with("file:///"));
        assert!(!encoded.contains("?/"));
        assert!(encoded.contains("%20"));
    }
}
