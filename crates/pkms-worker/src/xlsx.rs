//! XLSX extraction: every sheet becomes a heading block and one text block per
//! non-empty row, cells joined with ` | `.
//! The reader shows cached formula results, preserves formula sources and
//! merged ranges, and formats date/time cells. Formulas are never evaluated.
use crate::{Result, WorkerRequest, error};
use pkms_core::{ParsedSource, SourceBlock};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::io::{Cursor, Read};

const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SHEETS: usize = 500;
const MAX_ROWS: usize = 200_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;
const MAX_METADATA_CHARS: usize = 300;

fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("xlsx_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > MAX_PART_BYTES {
        return Err(error("xlsx_input_limit"));
    }
    Ok(bytes)
}

fn attribute(event: &BytesStart<'_>, name: &str) -> Option<String> {
    let value = event.try_get_attribute(name).ok().flatten()?.value;
    Some(value.to_string())
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

/// `sharedStrings.xml`: every `<si>` is one string, rich runs concatenated.
fn shared_strings(xml: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "si" => current = Some(String::new()),
                "t" => in_text = true,
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "t" => in_text = false,
                "si" => {
                    if let Some(value) = current.take() {
                        strings.push(value);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_text => {
                let raw = event.xml10_content();
                let value =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("xlsx_invalid_xml"))?;
                if let Some(current) = current.as_mut() {
                    current.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(event)) if in_text => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("xlsx_invalid_xml"))?;
                if let Some(current) = current.as_mut() {
                    current.push_str(&value);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("xlsx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(strings)
}

fn workbook_sheets(xml: &[u8]) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut sheets = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().into_inner() == "sheet" =>
            {
                if let (Some(name), Some(id)) =
                    (attribute(&event, "name"), attribute(&event, "r:id"))
                {
                    sheets.push((name, id));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("xlsx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(sheets)
}

fn relationships(xml: &[u8]) -> Result<Vec<(String, String)>> {
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
                {
                    links.push((id, target));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("xlsx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(links)
}

/// Resolves a relationship target against the `xl/` package folder.
fn resolve_target(target: &str) -> Option<String> {
    let target = target.split('#').next()?.trim();
    if target.is_empty() {
        return None;
    }
    if let Some(absolute) = target.strip_prefix('/') {
        return Some(absolute.to_owned());
    }
    let mut parts = vec!["xl"];
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

#[derive(Debug, Clone, Copy, Default)]
struct CellFormat {
    date: bool,
    time: bool,
    percent: bool,
}

impl CellFormat {
    fn from_code(code: &str) -> Self {
        let lower = code.to_ascii_lowercase();
        let date = lower.contains('y') || lower.contains('d');
        let time = lower.contains('h')
            || lower.contains('s')
            || lower.contains("am/pm")
            || (lower.contains(':') && lower.contains('m'));
        Self {
            date,
            time,
            percent: lower.contains('%'),
        }
    }
}

fn builtin_format(id: u32) -> Option<&'static str> {
    match id {
        14 => Some("mm-dd-yy"),
        15 => Some("d-mmm-yy"),
        16 => Some("d-mmm"),
        17 => Some("mmm-yy"),
        18 => Some("h:mm AM/PM"),
        19 => Some("h:mm:ss AM/PM"),
        20 => Some("h:mm"),
        21 => Some("h:mm:ss"),
        22 => Some("m/d/yy h:mm"),
        45 => Some("mm:ss"),
        46 => Some("[h]:mm:ss"),
        47 => Some("mmss.0"),
        9 | 10 => Some("0.00%"),
        _ => None,
    }
}

fn style_formats(xml: &[u8]) -> Result<Vec<CellFormat>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut custom = std::collections::HashMap::new();
    let mut styles = Vec::new();
    let mut in_cell_xfs = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        attribute(&event, "numFmtId"),
                        attribute(&event, "formatCode"),
                    ) && let Ok(id) = id.parse::<u32>()
                    {
                        custom.insert(id, code);
                    }
                }
                "cellXfs" => in_cell_xfs = true,
                "xf" if in_cell_xfs => {
                    let id = attribute(&event, "numFmtId")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(0);
                    let code = custom
                        .get(&id)
                        .map(String::as_str)
                        .or_else(|| builtin_format(id));
                    styles.push(code.map(CellFormat::from_code).unwrap_or_default());
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        attribute(&event, "numFmtId"),
                        attribute(&event, "formatCode"),
                    ) && let Ok(id) = id.parse::<u32>()
                    {
                        custom.insert(id, code);
                    }
                }
                "xf" if in_cell_xfs => {
                    let id = attribute(&event, "numFmtId")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(0);
                    let code = custom
                        .get(&id)
                        .map(String::as_str)
                        .or_else(|| builtin_format(id));
                    styles.push(code.map(CellFormat::from_code).unwrap_or_default());
                }
                _ => {}
            },
            Ok(Event::End(event)) if event.local_name().into_inner() == "cellXfs" => {
                in_cell_xfs = false;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("xlsx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(styles)
}

fn workbook_uses_1904(xml: &[u8]) -> bool {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().into_inner() == "workbookPr" =>
            {
                return attribute(&event, "date1904")
                    .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE"));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buffer.clear();
    }
    false
}

fn excel_date(value: f64, date1904: bool) -> Option<chrono::NaiveDateTime> {
    if !value.is_finite() {
        return None;
    }
    let mut days = value.trunc() as i64;
    let fraction = value - value.trunc();
    let base = if date1904 {
        chrono::NaiveDate::from_ymd_opt(1904, 1, 1)?
    } else {
        if days >= 60 {
            days -= 1;
        }
        chrono::NaiveDate::from_ymd_opt(1899, 12, 31)?
    };
    let date = base.checked_add_days(chrono::Days::new(days.max(0) as u64))?;
    let mut seconds = (fraction.abs() * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        seconds %= 86_400;
    }
    date.and_hms_opt(0, 0, 0)?
        .checked_add_signed(chrono::Duration::seconds(seconds))
}

fn format_numeric(raw: &str, format: CellFormat, date1904: bool) -> String {
    let Ok(value) = raw.trim().parse::<f64>() else {
        return raw.trim().to_owned();
    };
    if (format.date || format.time)
        && let Some(date) = excel_date(value, date1904)
    {
        return match (format.date, format.time) {
            (true, true) => date.format("%Y-%m-%d %H:%M:%S").to_string(),
            (true, false) => date.format("%Y-%m-%d").to_string(),
            (false, true) => date.format("%H:%M:%S").to_string(),
            (false, false) => unreachable!(),
        };
    }
    if format.percent {
        let scaled = value * 100.0;
        if (scaled - scaled.round()).abs() < 1e-9 {
            return format!("{}%", scaled.round());
        }
        let text = format!("{scaled:.2}");
        return format!("{}%", text.trim_end_matches('0').trim_end_matches('.'));
    }
    raw.trim().to_owned()
}

#[derive(Debug, Default)]
struct SheetData {
    rows: Vec<String>,
    merged: Vec<String>,
    formulas: Vec<String>,
}

/// One sheet: cached cell values, formula sources and merged ranges. Formula
/// evaluation stays outside this parser; the workbook's cached value is shown.
fn sheet_rows(
    xml: &[u8],
    shared: &[String],
    styles: &[CellFormat],
    date1904: bool,
    total: &mut usize,
    rows_budget: &mut usize,
) -> Result<SheetData> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut data = SheetData::default();
    let mut row: Vec<String> = Vec::new();
    let mut cell_ref = String::new();
    let mut cell_type = String::new();
    let mut cell_style = 0usize;
    let mut cell_value = String::new();
    let mut cell_formula = String::new();
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_text = false;
    let mut in_inline = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "row" => row.clear(),
                "c" => {
                    cell_ref = attribute(&event, "r").unwrap_or_default();
                    cell_type = attribute(&event, "t").unwrap_or_default();
                    cell_style = attribute(&event, "s")
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    cell_value.clear();
                    cell_formula.clear();
                }
                "v" => in_value = true,
                "f" => in_formula = true,
                "is" => in_inline = true,
                "t" if in_inline => in_text = true,
                "mergeCell" => {
                    if let Some(range) = attribute(&event, "ref") {
                        data.merged.push(range);
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => {
                if event.local_name().into_inner() == "mergeCell"
                    && let Some(range) = attribute(&event, "ref")
                {
                    data.merged.push(range);
                }
            }
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "v" => in_value = false,
                "f" => in_formula = false,
                "t" if in_inline => in_text = false,
                "is" => in_inline = false,
                "c" => {
                    let raw = cell_value.trim();
                    let formula = cell_formula.trim();
                    let format = styles.get(cell_style).copied().unwrap_or_default();
                    let mut value = match cell_type.as_str() {
                        "s" => raw
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index).cloned())
                            .unwrap_or_default(),
                        "b" => match raw {
                            "1" => "TRUE".to_owned(),
                            "0" => "FALSE".to_owned(),
                            other => other.to_owned(),
                        },
                        "d" | "str" | "e" => raw.to_owned(),
                        _ => format_numeric(raw, format, date1904),
                    };
                    if value.is_empty() && !formula.is_empty() {
                        value = format!("={formula}");
                    }
                    if !value.is_empty() {
                        *total += value.chars().count();
                        if *total > MAX_TEXT_CHARS {
                            return Err(error("xlsx_text_limit"));
                        }
                        row.push(value);
                    }
                    if !formula.is_empty() {
                        let reference = if cell_ref.is_empty() { "?" } else { &cell_ref };
                        data.formulas.push(format!("{reference} = {formula}"));
                    }
                }
                "row" if !row.is_empty() => {
                    if *rows_budget == 0 {
                        return Err(error("xlsx_structure_limit"));
                    }
                    *rows_budget -= 1;
                    data.rows.push(row.join(" | "));
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_value || in_formula || in_text => {
                let raw = event.xml10_content();
                let value =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("xlsx_invalid_xml"))?;
                if in_formula {
                    cell_formula.push_str(&value);
                } else {
                    cell_value.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(event)) if in_value || in_formula || in_text => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let value = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("xlsx_invalid_xml"))?;
                if in_formula {
                    cell_formula.push_str(&value);
                } else {
                    cell_value.push_str(&value);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("xlsx_invalid_xml")),
        }
        buffer.clear();
    }
    Ok(data)
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
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| error("xlsx_not_container"))?;
    let workbook = part(&mut archive, "xl/workbook.xml")?;
    let sheets = workbook_sheets(&workbook)?;
    if sheets.is_empty() {
        return Err(error("xlsx_no_content"));
    }
    let date1904 = workbook_uses_1904(&workbook);
    let rels = part(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let links = relationships(&rels)?;
    let shared = part(&mut archive, "xl/sharedStrings.xml")
        .ok()
        .map(|xml| shared_strings(&xml))
        .transpose()?
        .unwrap_or_default();
    let styles = part(&mut archive, "xl/styles.xml")
        .ok()
        .map(|xml| style_formats(&xml))
        .transpose()?
        .unwrap_or_default();
    let mut blocks = Vec::new();
    let mut total = 0usize;
    let mut rows_budget = MAX_ROWS;
    for (name, id) in sheets.iter().take(MAX_SHEETS) {
        let Some(target) = links
            .iter()
            .find(|(link, _)| link == id)
            .and_then(|(_, target)| resolve_target(target))
        else {
            continue;
        };
        let Ok(xml) = part(&mut archive, &target) else {
            continue;
        };
        let sheet = sheet_rows(
            &xml,
            &shared,
            &styles,
            date1904,
            &mut total,
            &mut rows_budget,
        )?;
        if sheet.rows.is_empty() && sheet.merged.is_empty() && sheet.formulas.is_empty() {
            continue;
        }
        push(&mut blocks, format!("Sheet: {name}"));
        for row in sheet.rows {
            push(&mut blocks, row);
        }
        if !sheet.merged.is_empty() {
            let text = format!("Merged: {}", sheet.merged.join(", "));
            total += text.chars().count();
            if total > MAX_TEXT_CHARS {
                return Err(error("xlsx_text_limit"));
            }
            push(&mut blocks, text);
        }
        if !sheet.formulas.is_empty() {
            let text = format!("Formulas: {}", sheet.formulas.join(", "));
            total += text.chars().count();
            if total > MAX_TEXT_CHARS {
                return Err(error("xlsx_text_limit"));
            }
            push(&mut blocks, text);
        }
    }
    if blocks.is_empty() {
        return Err(error("xlsx_no_content"));
    }
    let core = part(&mut archive, "docProps/core.xml").ok();
    let title = core.as_deref().and_then(|core| core_value(core, "title"));
    let author = core.as_deref().and_then(|core| core_value(core, "creator"));
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "xlsx-v2".into(),
        blocks,
        page_count: None,
        needs_ocr: false,
        title,
        author,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    })
}
