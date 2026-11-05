//! EPUB text extraction: the package's spine order defines the reading order
//! and each XHTML document's block elements become text blocks. Images, styles
//! and scripts are not interpreted; the original container is stored unchanged.
use crate::{Result, WorkerRequest, error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pkms_core::{ParsedCover, ParsedSource, ParsedSourceImage, SourceBlock, SourceTocItem};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::io::{Cursor, Read};

const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 5_000;
const MAX_METADATA_CHARS: usize = 300;
const MAX_TOC_ITEMS: usize = 5_000;
const MAX_COVER_BYTES: u64 = 4 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_IMAGE_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const CONTAINER: &str = "META-INF/container.xml";

fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("epub_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > MAX_PART_BYTES {
        return Err(error("epub_input_limit"));
    }
    Ok(bytes)
}

fn attribute(event: &BytesStart<'_>, name: &str) -> Option<String> {
    let value = event.try_get_attribute(name).ok().flatten()?.value;
    Some(value.to_string())
}

#[derive(Debug, Clone)]
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties: String,
}

#[derive(Default)]
struct BookManifest {
    items: Vec<ManifestItem>,
    cover_id: Option<String>,
}

fn manifest_item(event: &BytesStart<'_>) -> Option<ManifestItem> {
    Some(ManifestItem {
        id: attribute(event, "id")?,
        href: attribute(event, "href")?,
        media_type: attribute(event, "media-type").unwrap_or_default(),
        properties: attribute(event, "properties").unwrap_or_default(),
    })
}

fn book_manifest(xml: &[u8]) -> Result<BookManifest> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut manifest = BookManifest::default();
    let mut in_manifest = false;
    let mut in_metadata = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "metadata" => in_metadata = true,
                "manifest" => in_manifest = true,
                "item" if in_manifest => {
                    if let Some(item) = manifest_item(&event) {
                        manifest.items.push(item);
                    }
                }
                "meta" if in_metadata && attribute(&event, "name").as_deref() == Some("cover") => {
                    manifest.cover_id = attribute(&event, "content");
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "item" if in_manifest => {
                    if let Some(item) = manifest_item(&event) {
                        manifest.items.push(item);
                    }
                }
                "meta" if in_metadata && attribute(&event, "name").as_deref() == Some("cover") => {
                    manifest.cover_id = attribute(&event, "content");
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "metadata" => in_metadata = false,
                "manifest" => in_manifest = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("epub_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(manifest)
}

fn has_property(item: &ManifestItem, name: &str) -> bool {
    item.properties
        .split_whitespace()
        .any(|value| value.eq_ignore_ascii_case(name))
}

fn nav_item(manifest: &BookManifest) -> Option<&ManifestItem> {
    manifest
        .items
        .iter()
        .find(|item| has_property(item, "nav"))
        .or_else(|| {
            manifest
                .items
                .iter()
                .find(|item| item.media_type == "application/x-dtbncx+xml")
        })
}

fn cover_item(manifest: &BookManifest) -> Option<&ManifestItem> {
    manifest
        .items
        .iter()
        .find(|item| has_property(item, "cover-image"))
        .or_else(|| {
            manifest
                .cover_id
                .as_deref()
                .and_then(|id| manifest.items.iter().find(|item| item.id == id))
        })
        .or_else(|| {
            manifest.items.iter().find(|item| {
                item.media_type.starts_with("image/")
                    && item
                        .href
                        .rsplit('/')
                        .next()
                        .is_some_and(|name| name.to_ascii_lowercase().contains("cover"))
            })
        })
}

#[derive(Debug)]
struct TocDraft {
    title: String,
    level: u32,
    href: String,
    base: String,
}

fn toc_title(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.chars().take(300).collect())
    }
}

fn nav_toc(xml: &[u8], base: &str) -> Vec<TocDraft> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut drafts = Vec::new();
    let mut nav_seen = false;
    let mut inside_toc = false;
    let mut in_anchor = false;
    let mut href = String::new();
    let mut text = String::new();
    let mut depth = 0u32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "nav" if !nav_seen => {
                    nav_seen = true;
                    inside_toc = true;
                }
                "ol" if inside_toc => depth = depth.saturating_add(1),
                "a" if inside_toc => {
                    in_anchor = true;
                    href = attribute(&event, "href").unwrap_or_default();
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "nav" => inside_toc = false,
                "ol" if inside_toc => depth = depth.saturating_sub(1),
                "a" if in_anchor => {
                    in_anchor = false;
                    if let Some(title) = toc_title(&text) {
                        drafts.push(TocDraft {
                            title,
                            level: depth.max(1),
                            href: std::mem::take(&mut href),
                            base: base.to_owned(),
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_anchor => {
                if let Ok(value) = quick_xml::escape::unescape(&event.xml10_content()) {
                    text.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(event)) if in_anchor => {
                let reference = format!("&{};", event.xml10_content());
                if let Ok(value) = quick_xml::escape::unescape(&reference) {
                    text.push_str(&value);
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        if drafts.len() >= MAX_TOC_ITEMS {
            break;
        }
        buffer.clear();
    }
    drafts
}

fn ncx_toc(xml: &[u8], base: &str) -> Vec<TocDraft> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut drafts = Vec::new();
    let mut depth = 0u32;
    let mut in_label = false;
    let mut label = String::new();
    let mut href = String::new();
    let mut nav_title = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "navPoint" => {
                    depth = depth.saturating_add(1);
                    nav_title.clear();
                    href.clear();
                }
                "navLabel" => in_label = true,
                "text" if in_label => label.clear(),
                "content" => href = attribute(&event, "src").unwrap_or_default(),
                _ => {}
            },
            Ok(Event::Empty(event)) => {
                if event.local_name().into_inner() == "content" {
                    href = attribute(&event, "src").unwrap_or_default();
                }
            }
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "navLabel" => in_label = false,
                "text" if in_label => {
                    if let Some(title) = toc_title(&label) {
                        nav_title = title;
                    }
                }
                "navPoint" => {
                    if !nav_title.is_empty() && !href.is_empty() {
                        drafts.push(TocDraft {
                            title: std::mem::take(&mut nav_title),
                            level: depth.max(1),
                            href: href.clone(),
                            base: base.to_owned(),
                        });
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_label => {
                if let Ok(value) = quick_xml::escape::unescape(&event.xml10_content()) {
                    label.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(event)) if in_label => {
                let reference = format!("&{};", event.xml10_content());
                if let Ok(value) = quick_xml::escape::unescape(&reference) {
                    label.push_str(&value);
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        if drafts.len() >= MAX_TOC_ITEMS {
            break;
        }
        buffer.clear();
    }
    drafts
}

fn mapped_toc(
    drafts: Vec<TocDraft>,
    spine_starts: &[(String, String, u32, String)],
) -> Vec<SourceTocItem> {
    let mut seen = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    for draft in drafts {
        let Some(path) = resolve(&draft.base, &draft.href) else {
            continue;
        };
        let Some((_, block_id, position, _)) = spine_starts
            .iter()
            .find(|(candidate, ..)| candidate == &path)
        else {
            continue;
        };
        if seen.insert((block_id.clone(), draft.title.clone())) {
            items.push(SourceTocItem {
                title: draft.title,
                level: draft.level.min(16),
                block_id: Some(block_id.clone()),
                block_position: Some(*position),
            });
        }
        if items.len() >= MAX_TOC_ITEMS {
            break;
        }
    }
    if items.is_empty() {
        for (_, block_id, position, title) in spine_starts {
            items.push(SourceTocItem {
                title: title.clone(),
                level: 1,
                block_id: Some(block_id.clone()),
                block_position: Some(*position),
            });
        }
    }
    items
}

pub(crate) fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn read_limited(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("epub_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > limit {
        return Err(error("epub_input_limit"));
    }
    Ok(bytes)
}

fn book_cover(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    opf_path: &str,
    manifest: &BookManifest,
) -> Option<ParsedCover> {
    let item = cover_item(manifest)?;
    let path = resolve(opf_path, &item.href)?;
    let bytes = read_limited(archive, &path, MAX_COVER_BYTES).ok()?;
    let media_type = image_media_type(&bytes)?;
    Some(ParsedCover {
        media_type: media_type.to_owned(),
        data_base64: STANDARD.encode(bytes),
    })
}

fn push(blocks: &mut Vec<SourceBlock>, text: String) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    if blocks.len() >= MAX_BLOCKS {
        return Err(error("epub_structure_limit"));
    }
    blocks.push(SourceBlock {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        page_index: None,
        pdf_geometry: None,
    });
    Ok(())
}

fn flush(blocks: &mut Vec<SourceBlock>, text: &mut String) -> Result<()> {
    for line in std::mem::take(text).split('\n') {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut chunk = String::new();
        for character in normalized.chars() {
            if chunk.len() + character.len_utf8() > MAX_BLOCK_CHARS {
                push(blocks, std::mem::take(&mut chunk))?;
            }
            chunk.push(character);
        }
        push(blocks, chunk)?;
    }
    Ok(())
}

/// `META-INF/container.xml` names the package document; the first rootfile wins.
fn rootfile_path(xml: &[u8]) -> Option<String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().into_inner() == "rootfile" =>
            {
                if let Some(path) = attribute(&event, "full-path") {
                    return Some(path.replace('\\', "/"));
                }
            }
            Ok(Event::Eof) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        buffer.clear();
    }
}

#[derive(Default)]
struct Package {
    title: Option<String>,
    author: Option<String>,
    spine: Vec<String>,
}

/// Resolves a spine href against the package document's directory. Paths that
/// climb above the package root are rejected.
fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.split(['#', '?']).next()?.trim();
    if href.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    parts.pop();
    for segment in href.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn package_document(xml: &[u8]) -> Result<Package> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut package = Package::default();
    let mut manifest: Vec<(String, String)> = Vec::new();
    let mut spine_ids: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut field: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "metadata" | "manifest" | "spine" => {
                    section = event.local_name().into_inner().to_owned();
                }
                "title" if section == "metadata" => field = Some("title"),
                "creator" if section == "metadata" => field = Some("author"),
                "item" if section == "manifest" => {
                    if let (Some(id), Some(href)) =
                        (attribute(&event, "id"), attribute(&event, "href"))
                    {
                        manifest.push((id, href));
                    }
                }
                "itemref" if section == "spine" => {
                    if let Some(idref) = attribute(&event, "idref") {
                        spine_ids.push(idref);
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "item" if section == "manifest" => {
                    if let (Some(id), Some(href)) =
                        (attribute(&event, "id"), attribute(&event, "href"))
                    {
                        manifest.push((id, href));
                    }
                }
                "itemref" if section == "spine" => {
                    if let Some(idref) = attribute(&event, "idref") {
                        spine_ids.push(idref);
                    }
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "metadata" | "manifest" | "spine" => section.clear(),
                "title" | "creator" => field = None,
                _ => {}
            },
            Ok(Event::Text(event)) => {
                let Some(target) = field else { continue };
                let raw = event.xml10_content();
                let text = quick_xml::escape::unescape(&raw)
                    .map_err(|_| error("epub_invalid_xml"))?
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    continue;
                }
                let slot = if target == "title" {
                    &mut package.title
                } else {
                    &mut package.author
                };
                let value = slot.get_or_insert_with(String::new);
                if !value.is_empty() {
                    value.push_str(", ");
                }
                value.push_str(&text);
                if value.chars().count() > MAX_METADATA_CHARS {
                    value.truncate(
                        value
                            .char_indices()
                            .nth(MAX_METADATA_CHARS)
                            .map(|(index, _)| index)
                            .unwrap_or(value.len()),
                    );
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("epub_invalid_xml")),
        }
        buffer.clear();
    }
    for idref in spine_ids {
        if let Some((_, href)) = manifest.iter().find(|(id, _)| *id == idref) {
            package.spine.push(href.clone());
        }
    }
    Ok(package)
}

/// Block-level XHTML elements end one text block; head, script and style text
/// never reaches the reader.
#[derive(Debug)]
struct ImageDraft {
    block_position: u32,
    path: String,
    alt: String,
}

fn image_source(event: &BytesStart<'_>) -> Option<String> {
    attribute(event, "src")
        .or_else(|| attribute(event, "href"))
        .or_else(|| attribute(event, "xlink:href"))
}

fn image_block(
    event: &BytesStart<'_>,
    document_path: &str,
    blocks: &mut Vec<SourceBlock>,
    images: &mut Vec<ImageDraft>,
    text: &mut String,
) -> Result<Option<String>> {
    let Some(href) = image_source(event) else {
        return Ok(None);
    };
    let Some(path) = resolve(document_path, &href) else {
        return Ok(None);
    };
    let alt = attribute(event, "alt")
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect::<String>();
    flush(blocks, text)?;
    let label = if alt.is_empty() {
        "Image".to_owned()
    } else {
        alt.clone()
    };
    let before = blocks.len();
    push(blocks, label)?;
    if blocks.len() > before && images.len() < MAX_IMAGES && before <= u32::MAX as usize {
        images.push(ImageDraft {
            block_position: before as u32,
            path,
            alt: alt.clone(),
        });
    }
    Ok((!alt.is_empty()).then_some(alt))
}

fn document_blocks(
    xml: &[u8],
    document_path: &str,
    blocks: &mut Vec<SourceBlock>,
    images: &mut Vec<ImageDraft>,
    total: &mut usize,
) -> Result<()> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut skipped = 0usize;
    let mut image_alt: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "head" | "script" | "style" => skipped += 1,
                "br" => text.push('\n'),
                "img" | "image" if skipped == 0 => {
                    if let Some(alt) =
                        image_block(&event, document_path, blocks, images, &mut text)?
                    {
                        image_alt = Some(alt);
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "br" => text.push('\n'),
                "img" | "image" if skipped == 0 => {
                    if let Some(alt) =
                        image_block(&event, document_path, blocks, images, &mut text)?
                    {
                        image_alt = Some(alt);
                    }
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "head" | "script" | "style" => skipped = skipped.saturating_sub(1),
                "figcaption" => {
                    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if image_alt.as_deref() == Some(normalized.as_str()) {
                        text.clear();
                    } else {
                        flush(blocks, &mut text)?;
                    }
                    image_alt = None;
                }
                "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote"
                | "tr" | "td" | "th" | "section" | "article" | "dt" | "dd" | "pre" => {
                    flush(blocks, &mut text)?
                }
                _ => {}
            },
            Ok(Event::Text(event)) if skipped == 0 => {
                let raw = event.xml10_content();
                let value =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("epub_invalid_xml"))?;
                *total += value.chars().count();
                if *total > MAX_TEXT_CHARS {
                    return Err(error("epub_text_limit"));
                }
                text.push_str(&value);
            }
            Ok(Event::GeneralRef(event)) if skipped == 0 => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("epub_invalid_xml"))?;
                *total += value.chars().count();
                if *total > MAX_TEXT_CHARS {
                    return Err(error("epub_text_limit"));
                }
                text.push_str(&value);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("epub_invalid_xml")),
        }
        buffer.clear();
    }
    flush(blocks, &mut text)
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| error("epub_not_container"))?;
    let container = part(&mut archive, CONTAINER)?;
    let opf_path = rootfile_path(&container).ok_or_else(|| error("epub_container_missing"))?;
    let opf = part(&mut archive, &opf_path)?;
    let package = package_document(&opf)?;
    if package.spine.is_empty() {
        return Err(error("epub_no_content"));
    }
    let manifest = book_manifest(&opf)?;
    let cover = book_cover(&mut archive, &opf_path, &manifest);
    let mut blocks = Vec::new();
    let mut total = 0usize;
    let mut image_drafts = Vec::new();
    let mut spine_starts = Vec::new();
    for href in package.spine.iter().take(MAX_DOCUMENTS) {
        let Some(path) = resolve(&opf_path, href) else {
            continue;
        };
        let Ok(document) = part(&mut archive, &path) else {
            continue;
        };
        let before = blocks.len();
        document_blocks(&document, &path, &mut blocks, &mut image_drafts, &mut total)?;
        if blocks.len() > before && before <= u32::MAX as usize {
            let first = &blocks[before];
            let fallback = first
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect::<String>();
            spine_starts.push((path, first.id.clone(), before as u32, fallback));
        }
    }
    if blocks.is_empty() {
        return Err(error("epub_no_content"));
    }
    let mut images = Vec::new();
    let mut image_total = 0usize;
    for draft in image_drafts {
        let Ok(bytes) = read_limited(&mut archive, &draft.path, MAX_IMAGE_BYTES) else {
            continue;
        };
        let Some(media_type) = image_media_type(&bytes) else {
            continue;
        };
        image_total = image_total
            .checked_add(bytes.len())
            .ok_or_else(|| error("epub_input_limit"))?;
        if image_total > MAX_IMAGE_TOTAL_BYTES {
            return Err(error("epub_input_limit"));
        }
        images.push(ParsedSourceImage {
            block_position: draft.block_position,
            media_type: media_type.to_owned(),
            data_base64: STANDARD.encode(bytes),
            alt: draft.alt,
        });
    }
    let drafts = nav_item(&manifest)
        .and_then(|item| {
            let path = resolve(&opf_path, &item.href)?;
            let data = part(&mut archive, &path).ok()?;
            if item.media_type == "application/x-dtbncx+xml" {
                Some(ncx_toc(&data, &path))
            } else {
                Some(nav_toc(&data, &path))
            }
        })
        .unwrap_or_default();
    let toc = mapped_toc(drafts, &spine_starts);
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "epub-v2".into(),
        blocks,
        page_count: None,
        needs_ocr: false,
        toc,
        cover,
        images,
        title: package.title,
        author: package.author,
    })
}
