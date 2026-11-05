//! Legacy Word 97-2003 binary extraction. `undoc` reads the OLE word stream,
//! piece table, paragraph styles, tables, notes and embedded images without
//! executing macros. The original `.doc` stays immutable; this module produces
//! only the bounded reading/search projection.
use crate::{Result, WorkerError, WorkerRequest, error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pkms_core::{ParsedSource, ParsedSourceImage, SourceBlock, SourceTocItem};
use std::collections::HashSet;
use undoc::model::RevisionType;
use undoc::{Block, Document, ListType, Paragraph, Resource, ResourceRole, ResourceType, Table};

const MAX_BLOCKS: usize = 100_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_BLOCK_BYTES: usize = 16 * 1024;
const MAX_ASSETS: usize = 256;
const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
const MAX_SINGLE_ASSET_BYTES: usize = 12 * 1024 * 1024;
const MAX_METADATA_CHARS: usize = 300;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

struct Builder<'a> {
    document: &'a Document,
    blocks: Vec<SourceBlock>,
    toc: Vec<SourceTocItem>,
    images: Vec<ParsedSourceImage>,
    text_chars: usize,
    asset_bytes: usize,
    seen_assets: HashSet<(String, u32)>,
}

impl<'a> Builder<'a> {
    fn new(document: &'a Document) -> Self {
        Self {
            document,
            blocks: Vec::new(),
            toc: Vec::new(),
            images: Vec::new(),
            text_chars: 0,
            asset_bytes: 0,
            seen_assets: HashSet::new(),
        }
    }

    fn add_text(&mut self, text: &str) -> Result<Option<(u32, String)>> {
        let start = self.blocks.len();
        for line in text.lines() {
            let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if normalized.is_empty() {
                continue;
            }
            self.text_chars = self
                .text_chars
                .checked_add(normalized.chars().count())
                .ok_or_else(|| error("doc_text_limit"))?;
            if self.text_chars > MAX_TEXT_CHARS {
                return Err(error("doc_text_limit"));
            }
            let mut chunk = String::new();
            for character in normalized.chars() {
                if chunk.len() + character.len_utf8() > MAX_BLOCK_BYTES {
                    self.push_block(std::mem::take(&mut chunk))?;
                }
                chunk.push(character);
            }
            self.push_block(chunk)?;
        }
        Ok(self
            .blocks
            .get(start)
            .map(|block| (start as u32, block.id.clone())))
    }

    fn push_block(&mut self, text: String) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if self.blocks.len() >= MAX_BLOCKS {
            return Err(error("doc_structure_limit"));
        }
        self.blocks.push(SourceBlock {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            page_index: None,
            pdf_geometry: None,
        });
        Ok(())
    }

    fn add_paragraph(&mut self, paragraph: &Paragraph) -> Result<()> {
        let text = paragraph_text(paragraph);
        let first = self.add_text(&text)?;
        if let Some((position, id)) = &first
            && paragraph.is_heading()
        {
            let title = text.lines().next().unwrap_or_default().trim();
            if !title.is_empty() {
                self.toc.push(SourceTocItem {
                    title: title.to_string(),
                    level: paragraph.heading.level().into(),
                    block_id: Some(id.clone()),
                    block_position: Some(*position),
                });
            }
        }
        self.attach_images(
            &paragraph.images,
            first.as_ref().map(|(position, _)| *position),
        )?;
        Ok(())
    }

    fn attach_images(
        &mut self,
        images: &[undoc::model::InlineImage],
        mut anchor: Option<u32>,
    ) -> Result<()> {
        for image in images {
            if anchor.is_none() {
                let label = image.alt_text.as_deref().unwrap_or("Image");
                anchor = self.add_text(label)?.map(|(position, _)| position);
            }
            let Some(position) = anchor.take() else {
                continue;
            };
            self.add_resource(&image.resource_id, image.alt_text.as_deref(), position)?;
        }
        Ok(())
    }

    fn add_table(&mut self, table: &Table) -> Result<()> {
        if table.rows.is_empty() {
            return Ok(());
        }
        self.add_text(&format!(
            "Table: {} x {}",
            table.rows.len(),
            table.column_count()
        ))?;
        if let Some(caption) = table.caption.as_deref() {
            self.add_text(caption)?;
        }
        for row in &table.rows {
            let mut cells = Vec::with_capacity(row.cells.len());
            for cell in &row.cells {
                cells.push(
                    cell.content
                        .iter()
                        .map(paragraph_text)
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" / "),
                );
            }
            let first = self.add_text(&cells.join(" | "))?;
            let position = first
                .as_ref()
                .map(|(position, _)| *position)
                .unwrap_or(self.blocks.len() as u32);
            let mut image_anchor = Some(position);
            for cell in &row.cells {
                for paragraph in &cell.content {
                    self.attach_images(&paragraph.images, image_anchor)?;
                    image_anchor = None;
                }
                for nested in &cell.nested_tables {
                    self.add_table(nested)?;
                }
            }
        }
        Ok(())
    }

    fn add_image_block(&mut self, resource_id: &str, alt: &Option<String>) -> Result<()> {
        let label = alt.as_deref().filter(|value| !value.trim().is_empty());
        let text = label.unwrap_or("Image");
        let Some((position, _)) = self.add_text(text)? else {
            return Ok(());
        };
        self.add_resource(resource_id, alt.as_deref(), position)
    }

    fn add_resource(
        &mut self,
        resource_id: &str,
        inline_alt: Option<&str>,
        block_position: u32,
    ) -> Result<()> {
        let Some(resource) = self.document.resources.get(resource_id) else {
            return Ok(());
        };
        if resource.resource_type != ResourceType::Image || resource.role != ResourceRole::Primary {
            return Ok(());
        }
        if resource.data.is_empty() || resource.data.len() > MAX_SINGLE_ASSET_BYTES {
            return Err(error("doc_asset_limit"));
        }
        let Some(media_type) = supported_image_type(resource) else {
            let note = format!(
                "Image omitted: {}",
                resource
                    .mime_type
                    .as_deref()
                    .unwrap_or("unsupported format")
            );
            self.add_text(&note)?;
            return Ok(());
        };
        let key = (resource_id.to_string(), block_position);
        if !self.seen_assets.insert(key) {
            return Ok(());
        }
        if self.images.len() >= MAX_ASSETS {
            return Err(error("doc_asset_limit"));
        }
        self.asset_bytes = self
            .asset_bytes
            .checked_add(resource.data.len())
            .ok_or_else(|| error("doc_asset_limit"))?;
        if self.asset_bytes > MAX_ASSET_BYTES {
            return Err(error("doc_asset_limit"));
        }
        self.images.push(ParsedSourceImage {
            block_position,
            media_type: media_type.to_string(),
            data_base64: STANDARD.encode(&resource.data),
            alt: safe_text(inline_alt.or(resource.alt_text.as_deref()), 300),
        });
        Ok(())
    }

    fn consume(&mut self, sections: &[undoc::Section]) -> Result<()> {
        for section in sections {
            for block in &section.content {
                match block {
                    Block::Paragraph(paragraph) => self.add_paragraph(paragraph)?,
                    Block::Table(table) => self.add_table(table)?,
                    Block::Note { label, content } => {
                        self.add_text(&format!("Note: {label}"))?;
                        for paragraph in content {
                            self.add_paragraph(paragraph)?;
                        }
                    }
                    Block::Image {
                        resource_id,
                        alt_text,
                        ..
                    } => self.add_image_block(resource_id, alt_text)?,
                    Block::PageBreak | Block::SectionBreak => {}
                }
            }
        }
        Ok(())
    }
}

fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut text = String::new();
    if let Some(list) = &paragraph.list_info {
        text.push_str(&"  ".repeat(list.level as usize));
        match list.list_type {
            ListType::Bullet => text.push_str("• "),
            ListType::Numbered => {
                text.push_str(&format!("{}. ", list.number.unwrap_or(1)));
            }
            ListType::None => {}
        }
    }
    for run in &paragraph.runs {
        if run.revision == RevisionType::Deleted {
            continue;
        }
        text.push_str(&run.text);
        if let Some(label) = &run.note {
            text.push_str(&format!("[^{label}]"));
        }
        if run.line_break {
            text.push('\n');
        }
        if run.page_break {
            text.push('\n');
        }
    }
    text.trim().to_string()
}

fn supported_image_type(resource: &Resource) -> Option<&'static str> {
    match resource.mime_type.as_deref()?.to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn safe_text(value: Option<&str>, limit: usize) -> String {
    value
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn metadata(value: Option<&str>) -> Option<String> {
    let value = safe_text(value, MAX_METADATA_CHARS);
    (!value.is_empty()).then_some(value)
}

fn parser_failure(failure: undoc::Error) -> WorkerError {
    match failure.kind() {
        undoc::ErrorKind::Encrypted => error("doc_encrypted"),
        undoc::ErrorKind::Io => error("io_error"),
        _ => error("doc_invalid"),
    }
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    if !bytes.starts_with(&OLE_MAGIC) {
        return Err(error("doc_not_container"));
    }
    let document = undoc::doc::DocParser::from_bytes(bytes.to_vec())
        .and_then(|mut parser| parser.parse())
        .map_err(parser_failure)?;
    let mut builder = Builder::new(&document);
    builder.consume(&document.sections)?;
    if builder.blocks.is_empty() {
        return Err(error("doc_no_content"));
    }
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "doc-undoc-v1".into(),
        blocks: builder.blocks,
        page_count: None,
        needs_ocr: false,
        title: metadata(document.metadata.title.as_deref()),
        author: metadata(document.metadata.author.as_deref()),
        toc: builder.toc,
        cover: None,
        images: builder.images,
    })
}
