//! Legacy BIFF XLS extraction through a pure-Rust binary reader. Each sheet is
//! emitted in workbook order as a heading plus one text row per non-empty range
//! row. Cached values and formula sources are kept separately; macros and VBA
//! are never executed.
use crate::{Result, WorkerRequest, error};
use calamine::{Data, Reader, Xls};
use pkms_core::{ParsedSource, SourceBlock};
use std::io::Cursor;

const MAX_SHEETS: usize = 500;
const MAX_ROWS: usize = 200_000;
const MAX_FORMULAS: usize = 100_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;
const MAX_FORMULA_LIST_CHARS: usize = 1024 * 1024;

fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn count_text(total: &mut usize, text: &str) -> Result<()> {
    *total = total
        .checked_add(text.chars().count())
        .ok_or_else(|| error("xls_text_limit"))?;
    if *total > MAX_TEXT_CHARS {
        return Err(error("xls_text_limit"));
    }
    Ok(())
}

fn push(blocks: &mut Vec<SourceBlock>, text: String) {
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

fn cell_text(value: &Data) -> String {
    match value {
        Data::Empty => String::new(),
        Data::Int(number) => number.to_string(),
        Data::Float(number) if number.is_finite() => number.to_string(),
        Data::Float(number) => number.to_string(),
        Data::String(text) => normalized(text),
        Data::Bool(boolean) => (if *boolean { "TRUE" } else { "FALSE" }).to_owned(),
        Data::DateTime(datetime) => datetime
            .as_datetime()
            .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("{datetime:?}")),
        Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
        Data::Error(kind) => kind.to_string(),
    }
}

fn row_text(row: &[Data]) -> String {
    let last = row
        .iter()
        .rposition(|value| !matches!(value, Data::Empty))
        .map_or(0, |index| index + 1);
    row[..last]
        .iter()
        .map(cell_text)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn column_label(mut column: u32) -> String {
    let mut label = Vec::new();
    loop {
        label.push(b'A' + (column % 26) as u8);
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    label.reverse();
    String::from_utf8(label).unwrap_or_default()
}

fn formula_list(range: &calamine::Range<String>) -> Result<String> {
    let start = range.start().unwrap_or((0, 0));
    let mut formulas = Vec::new();
    for (row, column, formula) in range.cells() {
        let formula = formula.trim().trim_start_matches('=').trim();
        if formula.is_empty() {
            continue;
        }
        if formulas.len() >= MAX_FORMULAS {
            return Err(error("xls_structure_limit"));
        }
        let reference = format!(
            "{}{}",
            column_label(start.1 + column as u32),
            start.0 + row as u32 + 1
        );
        formulas.push(format!("{reference} = {formula}"));
    }
    let text = formulas.join(", ");
    if text.chars().count() > MAX_FORMULA_LIST_CHARS {
        return Err(error("xls_structure_limit"));
    }
    Ok(text)
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let mut workbook = Xls::new(Cursor::new(bytes)).map_err(|_| error("xls_not_container"))?;
    let names = workbook.sheet_names();
    if names.is_empty() {
        return Err(error("xls_no_content"));
    }
    if names.len() > MAX_SHEETS {
        return Err(error("xls_structure_limit"));
    }
    let mut blocks = Vec::new();
    let mut total = 0usize;
    let mut rows_budget = MAX_ROWS;
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|_| error("xls_invalid_workbook"))?;
        let formulas = workbook
            .worksheet_formula(&name)
            .map_err(|_| error("xls_invalid_workbook"))?;
        let mut rows = Vec::new();
        for row in range.rows() {
            if row.is_empty() {
                continue;
            }
            if rows_budget == 0 {
                return Err(error("xls_structure_limit"));
            }
            rows_budget -= 1;
            let text = row_text(row);
            if !text.is_empty() {
                rows.push(text);
            }
        }
        let formula_text = formula_list(&formulas)?;
        if rows.is_empty() && formula_text.is_empty() {
            continue;
        }
        push(&mut blocks, format!("Sheet: {name}"));
        for row in rows {
            count_text(&mut total, &row)?;
            push(&mut blocks, row);
        }
        if !formula_text.is_empty() {
            let text = format!("Formulas: {formula_text}");
            count_text(&mut total, &text)?;
            push(&mut blocks, text);
        }
    }
    if blocks.is_empty() {
        return Err(error("xls_no_content"));
    }
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "xls-v1".into(),
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
