//! PPTX extraction: slides in presentation order with paragraphs, tables,
//! images, chart caches, SmartArt text and speaker notes. The original
//! presentation container stays immutable; unsupported layout/animation data is
//! intentionally not converted into fabricated text.
use crate::{Result, WorkerRequest, error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pkms_core::{ParsedSource, ParsedSourceImage, SourceBlock, SourceTocItem};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;
use std::io::{Cursor, Read};

const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SLIDES: usize = 5_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;
const MAX_METADATA_CHARS: usize = 300;
const MAX_TOC_ITEMS: usize = 5_000;
const MAX_IMAGES: usize = 128;
const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_IMAGE_TOTAL_BYTES: usize = 24 * 1024 * 1024;
const MAX_TABLE_ROWS: usize = 2_000;
const MAX_TABLE_COLUMNS: usize = 200;
const MAX_TABLE_CELLS: usize = 100_000;
const MAX_CHART_SERIES: usize = 50;
const MAX_CHART_VALUES: usize = 2_000;
const MAX_RELATIONSHIPS: usize = 20_000;

#[derive(Debug, Clone)]
struct Relationship {
    id: String,
    target: String,
    kind: String,
    external: bool,
}

#[derive(Debug, Clone)]
struct ImageDraft {
    block_position: u32,
    path: String,
    alt: String,
}

#[derive(Debug, Default)]
struct ChartSeries {
    name: String,
    values: Vec<String>,
}

#[derive(Debug, Default)]
struct ChartData {
    title: String,
    categories: Vec<String>,
    series: Vec<ChartSeries>,
}

fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("pptx_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > MAX_PART_BYTES {
        return Err(error("pptx_input_limit"));
    }
    Ok(bytes)
}

fn read_limited(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("pptx_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > limit {
        return Err(error("pptx_input_limit"));
    }
    Ok(bytes)
}

fn attribute(event: &BytesStart<'_>, name: &str) -> Option<String> {
    let value = event.try_get_attribute(name).ok().flatten()?.value;
    Some(value.to_string())
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn count_text(total: &mut usize, text: &str) -> Result<()> {
    *total = total
        .checked_add(text.chars().count())
        .ok_or_else(|| error("pptx_text_limit"))?;
    if *total > MAX_TEXT_CHARS {
        return Err(error("pptx_text_limit"));
    }
    Ok(())
}

fn push(blocks: &mut Vec<SourceBlock>, text: String) {
    if text.is_empty() {
        return;
    }
    let mut chunk = String::new();
    for character in text.chars() {
        if chunk.len() + character.len_utf8() > MAX_BLOCK_CHARS {
            blocks.push(SourceBlock {
                id: uuid::Uuid::new_v4().to_string(),
                text: std::mem::take(&mut chunk),
                page_index: None,
                pdf_geometry: None,
            });
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        blocks.push(SourceBlock {
            id: uuid::Uuid::new_v4().to_string(),
            text: chunk,
            page_index: None,
            pdf_geometry: None,
        });
    }
}

fn slide_ids(xml: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut ids = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().into_inner() == "sldId" =>
            {
                if let Some(id) = attribute(&event, "r:id") {
                    ids.push(id);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(ids)
}

fn relationships(xml: &[u8]) -> Result<Vec<Relationship>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut links = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().into_inner() == "Relationship" =>
            {
                if let (Some(id), Some(target)) =
                    (attribute(&event, "Id"), attribute(&event, "Target"))
                    && links.len() < MAX_RELATIONSHIPS
                {
                    links.push(Relationship {
                        id,
                        target,
                        kind: attribute(&event, "Type").unwrap_or_default(),
                        external: attribute(&event, "TargetMode")
                            .is_some_and(|mode| mode.eq_ignore_ascii_case("External")),
                    });
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(links)
}

/// Resolves a relationship target against the folder that owns `base_part`.
fn resolve_target(base_part: &str, target: &str) -> Option<String> {
    let target = target.split('#').next()?.trim().replace('\\', "/");
    if target.is_empty() {
        return None;
    }
    if let Some(absolute) = target.strip_prefix('/') {
        return Some(absolute.to_owned());
    }
    let mut parts: Vec<&str> = base_part.split('/').collect();
    parts.pop();
    for segment in target.split('/') {
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

fn rels_part(base_part: &str) -> Option<String> {
    let (folder, file) = base_part.rsplit_once('/')?;
    Some(format!("{folder}/_rels/{file}.rels"))
}

fn related_parts(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    base_part: &str,
) -> Result<Vec<Relationship>> {
    let Some(path) = rels_part(base_part) else {
        return Ok(Vec::new());
    };
    let Ok(xml) = part(archive, &path) else {
        return Ok(Vec::new());
    };
    relationships(&xml)
}

fn relations_by_id(links: &[Relationship]) -> HashMap<&str, &Relationship> {
    links.iter().map(|link| (link.id.as_str(), link)).collect()
}

/// Every non-empty `<a:p>` becomes one paragraph. Table paragraphs are skipped
/// here because `slide_tables` reconstructs their rows without duplication.
/// Notes slides carry generated slide-number, date, header and footer
/// placeholders; those are chrome rather than authored notes and are omitted.
fn paragraphs(
    xml: &[u8],
    total: &mut usize,
    skip_tables: bool,
    skip_notes_placeholders: bool,
) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = Vec::new();
    let mut paragraph = String::new();
    let mut in_text = false;
    let mut table_depth = 0usize;
    let mut skip_shape = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "sp" if skip_notes_placeholders => skip_shape = false,
                "ph" if skip_notes_placeholders => {
                    skip_shape = attribute(&event, "type").is_some_and(|kind| {
                        ["sldNum", "ftr", "hdr", "dt", "sldImg"]
                            .iter()
                            .any(|generated| kind.eq_ignore_ascii_case(generated))
                    });
                }
                "tbl" if skip_tables => table_depth += 1,
                "p" if table_depth == 0 => paragraph.clear(),
                "t" if table_depth == 0 => in_text = !skip_shape,
                _ => {}
            },
            Ok(Event::Empty(event)) if skip_notes_placeholders => {
                if event.local_name().into_inner() == "ph" {
                    skip_shape = attribute(&event, "type").is_some_and(|kind| {
                        ["sldNum", "ftr", "hdr", "dt", "sldImg"]
                            .iter()
                            .any(|generated| kind.eq_ignore_ascii_case(generated))
                    });
                }
            }
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "t" if table_depth == 0 => in_text = false,
                "p" if table_depth == 0 && !skip_shape => {
                    let text = normalized(&paragraph);
                    if !text.is_empty() {
                        count_text(total, &text)?;
                        output.push(text);
                    }
                }
                "sp" if skip_notes_placeholders => skip_shape = false,
                "tbl" if skip_tables => table_depth = table_depth.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Text(event)) if in_text && table_depth == 0 && !skip_shape => {
                let raw = event.xml10_content();
                let value =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("pptx_invalid_xml"))?;
                paragraph.push_str(&value);
            }
            Ok(Event::GeneralRef(event)) if in_text && table_depth == 0 && !skip_shape => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("pptx_invalid_xml"))?;
                paragraph.push_str(&value);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(output)
}

fn slide_tables(xml: &[u8], total: &mut usize) -> Result<Vec<Vec<Vec<String>>>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut tables = Vec::new();
    let mut in_table = false;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_cell = false;
    let mut in_text = false;
    let mut cell_count = 0usize;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "tbl" if !in_table => {
                    in_table = true;
                    rows.clear();
                }
                "tr" if in_table => row.clear(),
                "tc" if in_table => {
                    if row.len() >= MAX_TABLE_COLUMNS {
                        return Err(error("pptx_structure_limit"));
                    }
                    in_cell = true;
                    cell.clear();
                    cell_count += 1;
                    if cell_count > MAX_TABLE_CELLS {
                        return Err(error("pptx_structure_limit"));
                    }
                }
                "p" if in_cell && !cell.is_empty() => cell.push(' '),
                "t" if in_cell => in_text = true,
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "tc" if in_table => {
                    if row.len() >= MAX_TABLE_COLUMNS {
                        return Err(error("pptx_structure_limit"));
                    }
                    cell_count += 1;
                    if cell_count > MAX_TABLE_CELLS {
                        return Err(error("pptx_structure_limit"));
                    }
                    row.push(String::new());
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "t" => in_text = false,
                "tc" if in_cell => {
                    in_cell = false;
                    in_text = false;
                    let value = normalized(&cell);
                    count_text(total, &value)?;
                    if row.len() < MAX_TABLE_COLUMNS {
                        row.push(value);
                    }
                }
                "tr" if in_table => {
                    if !row.is_empty() {
                        if rows.len() >= MAX_TABLE_ROWS {
                            return Err(error("pptx_structure_limit"));
                        }
                        rows.push(std::mem::take(&mut row));
                    }
                }
                "tbl" if in_table => {
                    in_table = false;
                    if !rows.is_empty() {
                        tables.push(std::mem::take(&mut rows));
                    }
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_cell && in_text => {
                let raw = event.xml10_content();
                let value =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("pptx_invalid_xml"))?;
                cell.push_str(&value);
            }
            Ok(Event::GeneralRef(event)) if in_cell && in_text => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("pptx_invalid_xml"))?;
                cell.push_str(&value);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(tables)
}

fn image_blocks(
    xml: &[u8],
    slide_part: &str,
    links: &[Relationship],
    blocks: &mut Vec<SourceBlock>,
    drafts: &mut Vec<ImageDraft>,
    total: &mut usize,
) -> Result<()> {
    let by_id = relations_by_id(links);
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut picture_depth = 0usize;
    let mut alt = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "pic" => {
                    picture_depth += 1;
                    alt.clear();
                }
                "cNvPr" if picture_depth > 0 => {
                    alt = attribute(&event, "descr")
                        .or_else(|| attribute(&event, "name"))
                        .unwrap_or_default();
                }
                "blip" => add_image_block(&event, slide_part, &by_id, blocks, drafts, total, &alt)?,
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "cNvPr" if picture_depth > 0 => {
                    alt = attribute(&event, "descr")
                        .or_else(|| attribute(&event, "name"))
                        .unwrap_or_default();
                }
                "blip" => add_image_block(&event, slide_part, &by_id, blocks, drafts, total, &alt)?,
                _ => {}
            },
            Ok(Event::End(event)) => {
                if event.local_name().into_inner() == "pic" {
                    picture_depth = picture_depth.saturating_sub(1);
                    alt.clear();
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(())
}

fn add_image_block(
    event: &BytesStart<'_>,
    slide_part: &str,
    links: &HashMap<&str, &Relationship>,
    blocks: &mut Vec<SourceBlock>,
    drafts: &mut Vec<ImageDraft>,
    total: &mut usize,
    alt: &str,
) -> Result<()> {
    let Some(id) = attribute(event, "r:embed") else {
        return Ok(());
    };
    let Some(link) = links.get(id.as_str()) else {
        return Ok(());
    };
    if link.external {
        return Ok(());
    }
    let Some(path) = resolve_target(slide_part, &link.target) else {
        return Ok(());
    };
    let alt = normalized(alt)
        .chars()
        .take(MAX_METADATA_CHARS)
        .collect::<String>();
    let label = if alt.is_empty() {
        "Image".to_owned()
    } else {
        alt.clone()
    };
    count_text(total, &label)?;
    let before = blocks.len();
    push(blocks, label);
    if blocks.len() > before && drafts.len() < MAX_IMAGES && before <= u32::MAX as usize {
        drafts.push(ImageDraft {
            block_position: before as u32,
            path,
            alt,
        });
    }
    Ok(())
}

fn chart_data(xml: &[u8], total: &mut usize) -> Result<ChartData> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut data = ChartData::default();
    let mut title_depth = 0usize;
    let mut category_depth = 0usize;
    let mut value_depth = 0usize;
    let mut text_depth = 0usize;
    let mut in_series = false;
    let mut series = ChartSeries::default();
    let mut current_categories: Vec<String> = Vec::new();
    let mut value = String::new();
    let mut capture = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "title" => title_depth += 1,
                "ser" => {
                    in_series = true;
                    series = ChartSeries::default();
                    current_categories.clear();
                }
                "cat" => category_depth += 1,
                "val" => value_depth += 1,
                "tx" => text_depth += 1,
                "t" | "v" => capture = true,
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "t" => {
                    if capture && title_depth > 0 {
                        let fragment = normalized(&value);
                        if !fragment.is_empty() {
                            if !data.title.is_empty() {
                                data.title.push(' ');
                            }
                            data.title.push_str(&fragment);
                        }
                    }
                    value.clear();
                    capture = false;
                }
                "v" => {
                    if capture {
                        let fragment = normalized(&value);
                        if title_depth > 0 {
                            if !fragment.is_empty() {
                                if !data.title.is_empty() {
                                    data.title.push(' ');
                                }
                                data.title.push_str(&fragment);
                            }
                        } else if category_depth > 0 {
                            if current_categories.len() < MAX_CHART_VALUES && !fragment.is_empty() {
                                current_categories.push(fragment);
                            }
                        } else if value_depth > 0 {
                            if series.values.len() < MAX_CHART_VALUES && !fragment.is_empty() {
                                series.values.push(fragment);
                            }
                        } else if text_depth > 0 && series.name.is_empty() {
                            series.name = fragment;
                        }
                    }
                    value.clear();
                    capture = false;
                }
                "tx" => text_depth = text_depth.saturating_sub(1),
                "val" => value_depth = value_depth.saturating_sub(1),
                "cat" => category_depth = category_depth.saturating_sub(1),
                "ser" => {
                    if in_series {
                        if data.categories.is_empty() && !current_categories.is_empty() {
                            data.categories = std::mem::take(&mut current_categories);
                        }
                        if data.series.len() < MAX_CHART_SERIES {
                            data.series.push(std::mem::take(&mut series));
                        }
                        in_series = false;
                    }
                }
                "title" => title_depth = title_depth.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Text(event)) if capture => {
                let raw = event.xml10_content();
                let value_part =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("pptx_invalid_xml"))?;
                value.push_str(&value_part);
            }
            Ok(Event::GeneralRef(event)) if capture => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value_part = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("pptx_invalid_xml"))?;
                value.push_str(&value_part);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("pptx_invalid_xml")),
        }
        buffer.clear();
    }
    data.title = normalized(&data.title);
    count_text(total, &data.title)?;
    for category in &data.categories {
        count_text(total, category)?;
    }
    for series in &data.series {
        count_text(total, &series.name)?;
        for value in &series.values {
            count_text(total, value)?;
        }
    }
    Ok(data)
}

fn chart_blocks(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
    blocks: &mut Vec<SourceBlock>,
    total: &mut usize,
) -> Result<()> {
    let Ok(xml) = part(archive, path) else {
        return Ok(());
    };
    let data = chart_data(&xml, total)?;
    if data.title.is_empty() && data.categories.is_empty() && data.series.is_empty() {
        return Ok(());
    }
    push(
        blocks,
        format!(
            "Chart: {}",
            if data.title.is_empty() {
                "Untitled"
            } else {
                &data.title
            }
        ),
    );
    if !data.categories.is_empty() {
        push(
            blocks,
            format!("Categories: {}", data.categories.join(" | ")),
        );
    }
    for (index, series) in data.series.into_iter().enumerate() {
        let name = if series.name.is_empty() {
            format!("Series {}", index + 1)
        } else {
            series.name
        };
        push(
            blocks,
            format!("Series: {name} = {}", series.values.join(" | ")),
        );
    }
    Ok(())
}

fn smartart_blocks(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
    blocks: &mut Vec<SourceBlock>,
    total: &mut usize,
) -> Result<()> {
    let Ok(xml) = part(archive, path) else {
        return Ok(());
    };
    let values = paragraphs(&xml, total, false, false)?;
    if values.is_empty() {
        return Ok(());
    }
    push(blocks, "SmartArt:".to_owned());
    for value in values {
        push(blocks, value);
    }
    Ok(())
}

fn notes_blocks(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
    blocks: &mut Vec<SourceBlock>,
    total: &mut usize,
) -> Result<()> {
    let Ok(xml) = part(archive, path) else {
        return Ok(());
    };
    let values = paragraphs(&xml, total, false, true)?;
    if values.is_empty() {
        return Ok(());
    }
    push(blocks, "Notes:".to_owned());
    for value in values {
        push(blocks, value);
    }
    Ok(())
}

fn core_value(xml: &[u8], name: &str) -> Option<String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut capture = false;
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) if event.local_name().into_inner() == name => capture = true,
            Ok(Event::End(event)) if event.local_name().into_inner() == name => {
                let value = text.trim().to_owned();
                return (!value.is_empty())
                    .then(|| value.chars().take(MAX_METADATA_CHARS).collect::<String>());
            }
            Ok(Event::Text(event)) if capture => {
                let raw = event.xml10_content();
                if let Ok(value) = quick_xml::escape::unescape(&raw) {
                    text.push_str(&value);
                }
            }
            Ok(Event::Eof) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        buffer.clear();
    }
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| error("pptx_not_container"))?;
    let presentation = part(&mut archive, "ppt/presentation.xml")?;
    let ids = slide_ids(&presentation)?;
    if ids.is_empty() {
        return Err(error("pptx_no_content"));
    }
    let rels = part(&mut archive, "ppt/_rels/presentation.xml.rels")?;
    let links = relationships(&rels)?;
    let mut blocks = Vec::new();
    let mut drafts = Vec::new();
    let mut toc = Vec::new();
    let mut total = 0usize;
    let mut number = 0usize;
    for id in ids.iter().take(MAX_SLIDES) {
        let Some(target) = links
            .iter()
            .find(|link| link.id == *id)
            .and_then(|link| resolve_target("ppt/presentation.xml", &link.target))
        else {
            continue;
        };
        let Ok(xml) = part(&mut archive, &target) else {
            continue;
        };
        number += 1;
        let slide_number = number;
        let slide_before = blocks.len();
        push(&mut blocks, format!("Slide: {slide_number}"));
        let slide_rels = related_parts(&mut archive, &target)?;
        let values = paragraphs(&xml, &mut total, true, false)?;
        for value in &values {
            push(&mut blocks, value.clone());
        }
        for table in slide_tables(&xml, &mut total)? {
            let columns = table.iter().map(Vec::len).max().unwrap_or(0);
            push(&mut blocks, format!("Table: {} x {columns}", table.len()));
            for row in table {
                push(&mut blocks, row.join(" | "));
            }
        }
        image_blocks(
            &xml,
            &target,
            &slide_rels,
            &mut blocks,
            &mut drafts,
            &mut total,
        )?;
        for link in &slide_rels {
            if link.external {
                continue;
            }
            let Some(path) = resolve_target(&target, &link.target) else {
                continue;
            };
            if link.kind.ends_with("/chart") {
                chart_blocks(&mut archive, &path, &mut blocks, &mut total)?;
            } else if link.kind.ends_with("/diagramData") {
                smartart_blocks(&mut archive, &path, &mut blocks, &mut total)?;
            } else if link.kind.ends_with("/notesSlide") {
                notes_blocks(&mut archive, &path, &mut blocks, &mut total)?;
            }
        }
        if blocks.len() > slide_before && slide_before <= u32::MAX as usize {
            let title = values
                .first()
                .cloned()
                .unwrap_or_else(|| format!("Slide {slide_number}"));
            if toc.len() < MAX_TOC_ITEMS {
                toc.push(SourceTocItem {
                    title: title.chars().take(120).collect(),
                    level: 1,
                    block_id: Some(blocks[slide_before].id.clone()),
                    block_position: Some(slide_before as u32),
                });
            }
        }
    }
    if blocks.is_empty() {
        return Err(error("pptx_no_content"));
    }
    let mut images = Vec::new();
    let mut image_total = 0usize;
    for draft in drafts {
        let Ok(bytes) = read_limited(&mut archive, &draft.path, MAX_IMAGE_BYTES) else {
            continue;
        };
        let Some(media_type) = crate::epub::image_media_type(&bytes) else {
            continue;
        };
        image_total = image_total
            .checked_add(bytes.len())
            .ok_or_else(|| error("pptx_input_limit"))?;
        if image_total > MAX_IMAGE_TOTAL_BYTES {
            return Err(error("pptx_input_limit"));
        }
        images.push(ParsedSourceImage {
            block_position: draft.block_position,
            media_type: media_type.to_owned(),
            data_base64: STANDARD.encode(bytes),
            alt: draft.alt,
        });
    }
    let core = part(&mut archive, "docProps/core.xml").ok();
    let title = core.as_deref().and_then(|core| core_value(core, "title"));
    let author = core.as_deref().and_then(|core| core_value(core, "creator"));
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "pptx-v2".into(),
        blocks,
        page_count: Some(number.min(u32::MAX as usize) as u32),
        needs_ocr: false,
        title,
        author,
        toc,
        cover: None,
        images,
    })
}
