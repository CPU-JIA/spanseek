//! Structured note export. Rendering never treats user text as executable HTML.
use crate::{
    DocumentNode, Error, ExportFormat, Library, NodeType, Note, NoteExportReport, PreparedOutput,
    Result,
    portable::{archive_error, new_output, zip_bytes},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};
use zip::{ZipArchive, ZipWriter};

/// Renders one revision as Markdown for an external vault exporter. Validation
/// stays the same as the regular Markdown export, so unsupported rich nodes
/// fail loudly instead of being dropped.
pub(crate) fn markdown_document(
    note: &Note,
    assets: &BTreeMap<String, String>,
    include_properties: bool,
) -> Result<String> {
    markdown::validate(&note.document)?;
    Ok(if include_properties {
        markdown::document(note, assets)
    } else {
        markdown::document_without_properties(note, assets)
    })
}

impl Library {
    pub fn export_note_archive(
        &self,
        note_id: &str,
        destination: impl AsRef<Path>,
        format: ExportFormat,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<NoteExportReport> {
        let note = self.note(note_id)?;
        let ready =
            self.prepare_note_export(note_id, &note.revision_id, destination, format, check)?;
        check("publish_export")?;
        ready.publish()
    }

    pub fn prepare_note_export(
        &self,
        note_id: &str,
        revision_id: &str,
        destination: impl AsRef<Path>,
        format: ExportFormat,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<PreparedOutput<NoteExportReport>> {
        self.prepare_note_export_with_citations(
            note_id,
            revision_id,
            destination,
            format,
            None,
            check,
        )
    }

    /// Renders one historical revision with the requested citation style. The
    /// bibliography is read from the same revision, never from the current head.
    pub fn prepare_note_export_with_citations(
        &self,
        note_id: &str,
        revision_id: &str,
        destination: impl AsRef<Path>,
        format: ExportFormat,
        citation_style_id: Option<&str>,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<PreparedOutput<NoteExportReport>> {
        crate::document::valid_uuid(note_id)?;
        crate::document::valid_uuid(revision_id)?;
        let note = self.revision(note_id, revision_id)?;
        let citation_report = if matches!(
            format,
            ExportFormat::Html | ExportFormat::Docx | ExportFormat::Pdf
        ) {
            Some(self.bibliography_note_with_style(&note, citation_style_id.unwrap_or("plain"))?)
        } else {
            None
        };
        if format == ExportFormat::Markdown {
            markdown::validate(&note.document)?;
        }
        let destination = destination.as_ref();
        let mut output = new_output(destination)?;
        let assets = self.collect_export_assets(&note, check)?;
        let with_renderer = format == ExportFormat::Html && html::needs_renderer(&note);
        let mut warnings=vec!["仅导出所选修订正文和图片；修订历史、标签、集合、草稿与视图不在此导出范围，完整恢复请使用完整备份。".into()];
        if format == ExportFormat::Markdown {
            warnings.push("列表、任务、对齐表格及 OFM 使用对应 Markdown 语法；颜色/下划线等使用受约束 HTML，需要阅读器支持。内部块 UUID、列宽及已绑定的 wiki 身份另见 document.json/references.json。".into());
        }
        if format == ExportFormat::Html {
            warnings
                .push("HTML 保留正文语义和块锚点；编辑器主题及列宽以 document.json 为准。".into());
            if with_renderer {
                warnings.push("请先解压再打开 HTML；公式及 Mermaid 使用包内本地资源渲染，原始源码保留，渲染失败会显示原因。".into());
            }
        }
        if format == ExportFormat::Docx {
            warnings.push("DOCX 为可编辑 OOXML；图片、标题、列表、表格、引注和参考文献保留，应用内主题、评论和交互组件不随文件导出。".into());
        }
        if format == ExportFormat::Pdf {
            warnings.push("PDF 为固定版式，正文、图片、引注和参考文献保留；应用内主题、编辑能力与交互组件不随文件导出。".into());
        }
        if let Some(report) = &citation_report
            && !report.keys.is_empty()
        {
            warnings.push(format!(
                "正文引注与参考文献按 CSL 样式 {} 渲染；缺失引用保留原标记并醒目标记。",
                report.style_id
            ));
        }
        if note.document.nodes().iter().any(|n| {
            n.kind == NodeType::NoteReference
                || matches!(n.kind, NodeType::WikiLink | NodeType::WikiEmbed)
        }) {
            warnings.push(
                "正文引用保留 spanseek 对象/块 ID；未打包引用目标，外部阅读器可能无法打开这些引用。"
                    .into(),
            );
        }
        if format == ExportFormat::Docx {
            check("render_document")?;
            let bytes = docx::document(&note, &assets, citation_report.as_ref())?;
            output.as_file_mut().write_all(&bytes)?;
            let document_path = destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("note.docx")
                .to_owned();
            let report = NoteExportReport {
                note_id: note.id.clone(),
                revision_id: note.revision_id.clone(),
                format,
                document_path,
                attachment_count: assets.len() as u32,
                warnings,
            };
            return Ok(PreparedOutput {
                file: output,
                destination: destination.to_owned(),
                report,
            });
        }
        if format == ExportFormat::Pdf {
            check("render_document")?;
            let parent = destination.parent().unwrap_or(Path::new("."));
            let bytes = pdf::render(&note, &assets, citation_report.as_ref(), parent)?;
            output.as_file_mut().write_all(&bytes)?;
            let document_path = destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("note.pdf")
                .to_owned();
            let report = NoteExportReport {
                note_id: note.id.clone(),
                revision_id: note.revision_id.clone(),
                format,
                document_path,
                attachment_count: assets.len() as u32,
                warnings,
            };
            return Ok(PreparedOutput {
                file: output,
                destination: destination.to_owned(),
                report,
            });
        }
        check("render_document")?;
        let asset_paths = assets
            .iter()
            .map(|(hash, (media_type, _))| {
                let extension = match media_type.as_str() {
                    "image/png" => "png",
                    "image/jpeg" => "jpg",
                    "image/webp" => "webp",
                    _ => return Err(Error::IntegrityFailure),
                };
                Ok((hash.clone(), format!("assets/{hash}.{extension}")))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let body = match format {
            ExportFormat::Html => html::document(
                &note,
                &asset_paths,
                None,
                with_renderer,
                citation_report.as_ref(),
            ),
            ExportFormat::Markdown => markdown::document(&note, &asset_paths),
            ExportFormat::Docx | ExportFormat::Pdf => unreachable!(),
        };
        let extension = match format {
            ExportFormat::Html => "html",
            ExportFormat::Markdown => "md",
            ExportFormat::Docx | ExportFormat::Pdf => unreachable!(),
        };
        let report = NoteExportReport {
            note_id: note.id.clone(),
            revision_id: note.revision_id.clone(),
            format,
            document_path: format!("{}.{}", note.id, extension),
            attachment_count: assets.len() as u32,
            warnings,
        };
        let mut writer = ZipWriter::new(output.as_file_mut());
        for (hash, (media_type, bytes)) in &assets {
            let extension = match media_type.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/webp" => "webp",
                _ => return Err(Error::IntegrityFailure),
            };
            zip_bytes(&mut writer, &format!("assets/{hash}.{extension}"), bytes)?;
        }
        if with_renderer {
            copy_rich_resources(&mut writer, check)?;
        }
        zip_bytes(&mut writer, &report.document_path, body.as_bytes())?;
        zip_bytes(
            &mut writer,
            "references.json",
            &serde_json::to_vec_pretty(&self.note_references(note_id, revision_id)?)?,
        )?;
        zip_bytes(
            &mut writer,
            "document.json",
            &serde_json::to_vec_pretty(&note)?,
        )?;
        zip_bytes(
            &mut writer,
            "export-report.json",
            &serde_json::to_vec_pretty(&report)?,
        )?;
        writer.finish().map_err(archive_error)?;
        Ok(PreparedOutput {
            file: output,
            destination: destination.to_owned(),
            report,
        })
    }

    fn collect_export_assets(
        &self,
        note: &Note,
        check: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<BTreeMap<String, (String, Vec<u8>)>> {
        let mut assets = BTreeMap::new();
        let mut total = 0_usize;
        for node in note
            .document
            .nodes()
            .into_iter()
            .filter(|node| node.kind == NodeType::Image)
        {
            check("export_images")?;
            let hash = node
                .attrs
                .blob_hash
                .as_deref()
                .ok_or(Error::IntegrityFailure)?;
            if assets.contains_key(hash) {
                continue;
            }
            let (info, bytes) = self.attachment(&note.revision_id, hash)?;
            if !matches!(
                info.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            ) {
                return Err(Error::IntegrityFailure);
            }
            total = total
                .checked_add(bytes.len())
                .ok_or(Error::IntegrityFailure)?;
            if total > 1024 * 1024 * 1024 {
                return Err(Error::InvalidInput("note export exceeds 1 GiB"));
            }
            assets.insert(hash.to_owned(), (info.media_type, bytes));
        }
        Ok(assets)
    }
}

fn copy_rich_resources(
    writer: &mut ZipWriter<&mut File>,
    check: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    // A versioned offline artifact makes the Rust library independent of Node
    // and of the user's installation directory when exporting a document.
    let bytes = include_bytes!("../resources/rich-export.zip");
    let mut resources = ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(archive_error)?;
    if resources.len() > 256 {
        return Err(Error::IntegrityFailure);
    }
    let mut names = BTreeSet::new();
    let mut remaining = 64 * 1024 * 1024_u64;
    for index in 0..resources.len() {
        check("export_rich_resources")?;
        let mut entry = resources.by_index(index).map_err(archive_error)?;
        let name = entry.name().to_owned();
        if !name.starts_with("assets/rich-export/")
            || name.contains('\\')
            || entry.enclosed_name().is_none()
            || entry.is_dir()
            || !names.insert(name.clone())
            || entry.size() > remaining
        {
            return Err(Error::IntegrityFailure);
        }
        let mut data = Vec::new();
        let size = entry.size();
        entry.by_ref().take(size + 1).read_to_end(&mut data)?;
        if data.len() as u64 != size {
            return Err(Error::IntegrityFailure);
        }
        remaining -= size;
        zip_bytes(writer, &name, &data)?;
    }
    if !names.contains("assets/rich-export/renderer.js")
        || !names.contains("assets/rich-export/style.css")
        || !names.contains("assets/rich-export/manifest.json")
    {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn reference_url(node: &DocumentNode) -> String {
    let revision = node
        .attrs
        .target_revision_id
        .as_ref()
        .map(|value| format!("?revision={value}"))
        .unwrap_or_default();
    let block = node
        .attrs
        .target_block_id
        .as_ref()
        .map(|value| format!("#{value}"))
        .unwrap_or_default();
    format!(
        "pkms://object/{}{revision}{block}",
        node.attrs.target_id.as_deref().unwrap_or_default()
    )
}

#[path = "export_docx.rs"]
mod docx;
#[path = "export_html.rs"]
mod html;
#[path = "export_markdown.rs"]
mod markdown;
#[path = "export_pdf.rs"]
mod pdf;
