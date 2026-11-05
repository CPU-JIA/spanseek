//! CSV extraction: RFC 4180 quoting, one text block per row, cells joined with
//! ` | `. No type inference happens here; the original bytes stay unchanged.
use crate::{Result, WorkerRequest, error};
use pkms_core::{ParsedSource, SourceBlock};

const MAX_ROWS: usize = 100_000;
const MAX_COLUMNS: usize = 1_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;

fn push(blocks: &mut Vec<SourceBlock>, text: String) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    if blocks.len() >= MAX_ROWS {
        return Err(error("csv_structure_limit"));
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
    Ok(())
}

/// Splits text into records. A quoted field may contain commas, doubled quotes
/// and line breaks; a record ends on an unquoted CR, LF or CRLF.
fn records(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut total = 0usize;
    while let Some(character) = chars.next() {
        total += character.len_utf8();
        if total > MAX_TEXT_CHARS {
            return Err(error("csv_text_limit"));
        }
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                row.push(std::mem::take(&mut field));
                if row.len() > MAX_COLUMNS {
                    return Err(error("csv_structure_limit"));
                }
            }
            '\r' | '\n' if !quoted => {
                if character == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if rows.len() > MAX_ROWS {
                    return Err(error("csv_structure_limit"));
                }
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn row_text(fields: &[String]) -> String {
    let cells: Vec<String> = fields
        .iter()
        .map(|cell| cell.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let last = cells.iter().rposition(|cell| !cell.is_empty());
    let Some(last) = last else {
        return String::new();
    };
    cells[..=last].join(" | ")
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let text = std::str::from_utf8(bytes).map_err(|_| error("csv_not_utf8"))?;
    let text = text.trim_start_matches('\u{feff}');
    if text.contains('\0') {
        return Err(error("csv_invalid_text"));
    }
    let mut blocks = Vec::new();
    for fields in records(text)? {
        push(&mut blocks, row_text(&fields))?;
    }
    if blocks.is_empty() {
        return Err(error("csv_no_content"));
    }
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "csv-v1".into(),
        blocks,
        page_count: None,
        needs_ocr: false,
        title: None,
        author: None,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    })
}
