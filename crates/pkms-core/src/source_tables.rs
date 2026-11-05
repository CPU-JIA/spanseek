//! Geometry-only table extraction for parsed PDF pages.
//!
//! This is deliberately not a layout model. It groups stored character boxes
//! into lines, splits lines at wide horizontal gaps, and reports runs of rows
//! with at least three cells. Merged cells, spanning headers, vertical text and
//! scanned cells the OCR engine did not recognize stay outside the support
//! level and are reported as such by the interface.
use crate::{SourceBlock, SourceTable};
use std::collections::BTreeMap;

const MAX_TABLES: usize = 32;
const MAX_ROWS: usize = 200;
const MAX_COLUMNS: usize = 64;
const MAX_CELL_CHARS: usize = 4096;

#[derive(Debug, Clone)]
struct Glyph {
    ch: char,
    rect: [f64; 4],
}

impl Glyph {
    fn center_y(&self) -> f64 {
        (self.rect[1] + self.rect[3]) / 2.0
    }
    fn height(&self) -> f64 {
        (self.rect[3] - self.rect[1]).abs().max(0.1)
    }
    fn width(&self) -> f64 {
        (self.rect[2] - self.rect[0]).abs().max(0.1)
    }
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn glyphs(block: &SourceBlock) -> Vec<Glyph> {
    let Some(geometry) = block.pdf_geometry.as_ref() else {
        return Vec::new();
    };
    let text: Vec<char> = block.text.chars().collect();
    geometry
        .chars
        .iter()
        .filter_map(|box_| {
            let rect = box_.rect?;
            if !rect.iter().all(|value| value.is_finite())
                || rect[2] <= rect[0]
                || rect[3] <= rect[1]
            {
                return None;
            }
            let ch = *text.get(box_.start_codepoint as usize)?;
            (!ch.is_whitespace()).then_some(Glyph { ch, rect })
        })
        .collect()
}

fn is_ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn line_text(cells: &[Vec<Glyph>], median_width: f64) -> String {
    let mut text = String::new();
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            text.push('\t');
        }
        let mut previous: Option<&Glyph> = None;
        for glyph in cell {
            if let Some(previous) = previous {
                let gap = glyph.rect[0] - previous.rect[2];
                if gap > median_width * 0.55
                    && is_ascii_word(previous.ch)
                    && is_ascii_word(glyph.ch)
                {
                    text.push(' ');
                }
            }
            text.push(glyph.ch);
            previous = Some(glyph);
        }
    }
    text
}

/// Extract tables from the blocks supplied for one reading page. Blocks from
/// different pages never merge.
pub(crate) fn extract(blocks: &[SourceBlock]) -> Vec<SourceTable> {
    let mut by_page: BTreeMap<u32, Vec<Glyph>> = BTreeMap::new();
    for block in blocks {
        let Some(page) = block.page_index else {
            continue;
        };
        by_page.entry(page).or_default().extend(glyphs(block));
    }
    let mut tables = Vec::new();
    for (page_index, mut page_glyphs) in by_page {
        if page_glyphs.is_empty() {
            continue;
        }
        let heights: Vec<f64> = page_glyphs.iter().map(Glyph::height).collect();
        let widths: Vec<f64> = page_glyphs.iter().map(Glyph::width).collect();
        let median_height = median(&heights);
        let median_width = median(&widths);
        page_glyphs.sort_by(|a, b| b.center_y().total_cmp(&a.center_y()));
        let mut lines: Vec<Vec<Glyph>> = Vec::new();
        for glyph in page_glyphs {
            let join = lines.last().is_some_and(|line| {
                let center = line.iter().map(Glyph::center_y).sum::<f64>() / line.len() as f64;
                (glyph.center_y() - center).abs() <= median_height * 0.6
            });
            if join {
                lines.last_mut().unwrap().push(glyph);
            } else {
                lines.push(vec![glyph]);
            }
        }
        let mut rows: Vec<(Vec<String>, [f64; 4])> = Vec::new();
        for mut line in lines {
            line.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
            let mut cells: Vec<Vec<Glyph>> = Vec::new();
            for glyph in line {
                let split = cells.last().is_some_and(|cell| {
                    let previous = cell.last().unwrap();
                    glyph.rect[0] - previous.rect[2] > (median_width * 1.8).max(median_height * 0.9)
                });
                if split {
                    cells.push(vec![glyph]);
                } else if let Some(cell) = cells.last_mut() {
                    cell.push(glyph);
                } else {
                    cells.push(vec![glyph]);
                }
            }
            if cells.len() < 3 {
                continue;
            }
            let text = line_text(&cells, median_width);
            let cells_text: Vec<String> = text
                .split('\t')
                .map(|cell| -> String { cell.chars().take(MAX_CELL_CHARS).collect() })
                .filter(|cell| !cell.is_empty())
                .collect();
            if cells_text.len() < 3 || cells_text.len() > MAX_COLUMNS {
                continue;
            }
            let mut bbox = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for cell in &cells {
                for glyph in cell {
                    bbox[0] = bbox[0].min(glyph.rect[0]);
                    bbox[1] = bbox[1].min(glyph.rect[1]);
                    bbox[2] = bbox[2].max(glyph.rect[2]);
                    bbox[3] = bbox[3].max(glyph.rect[3]);
                }
            }
            rows.push((cells_text, bbox));
        }
        // Consecutive table-like rows become one table; a gap larger than 2.5
        // line heights ends it.
        let mut current: Vec<(Vec<String>, [f64; 4])> = Vec::new();
        let flush = |current: &mut Vec<(Vec<String>, [f64; 4])>, tables: &mut Vec<SourceTable>| {
            if current.len() < 3 || tables.len() >= MAX_TABLES {
                current.clear();
                return;
            }
            let mut bbox = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for (_, row_bbox) in current.iter() {
                bbox[0] = bbox[0].min(row_bbox[0]);
                bbox[1] = bbox[1].min(row_bbox[1]);
                bbox[2] = bbox[2].max(row_bbox[2]);
                bbox[3] = bbox[3].max(row_bbox[3]);
            }
            tables.push(SourceTable {
                page_index,
                rows: current
                    .iter()
                    .take(MAX_ROWS)
                    .map(|(row, _)| row.clone())
                    .collect(),
                bbox,
            });
            current.clear();
        };
        for (row, bbox) in rows {
            let continues = current.last().is_some_and(|(previous, previous_bbox)| {
                let column_delta = previous.len().abs_diff(row.len());
                let gap = (previous_bbox[1] - bbox[3]).abs();
                column_delta <= 1 && gap <= median_height * 4.5
            });
            if !continues {
                flush(&mut current, &mut tables);
            }
            current.push((row, bbox));
        }
        flush(&mut current, &mut tables);
    }
    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PdfBlockGeometry, PdfCharacterBox};

    fn block(rows: &[&[&str]]) -> SourceBlock {
        let mut text = String::new();
        let mut chars = Vec::new();
        let mut x = 20.0;
        let mut y = 300.0;
        for row in rows {
            for cell in *row {
                for ch in cell.chars() {
                    let index = text.chars().count() as u32;
                    text.push(ch);
                    chars.push(PdfCharacterBox {
                        start_codepoint: index,
                        end_codepoint: index + 1,
                        rect: Some([x, y, x + 8.0, y + 12.0]),
                    });
                    x += 8.0;
                }
                x += 40.0;
            }
            x = 20.0;
            y -= 20.0;
        }
        SourceBlock {
            id: "block".into(),
            text,
            page_index: Some(0),
            pdf_geometry: Some(PdfBlockGeometry {
                page_label: None,
                crop_box: [0.0, 0.0, 600.0, 800.0],
                rotation: 0,
                coordinate_space: "pdf_user_space".into(),
                chars,
            }),
        }
    }

    #[test]
    fn three_aligned_rows_with_wide_gaps_become_a_table() {
        let tables = extract(&[block(&[
            &["项目", "结果", "备注"],
            &["文字", "通过", "可检索"],
            &["表格", "待验", "跨页"],
        ])]);
        assert_eq!(tables.len(), 1, "{tables:?}");
        assert_eq!(tables[0].rows.len(), 3);
        assert_eq!(tables[0].rows[0], ["项目", "结果", "备注"]);
        assert_eq!(tables[0].rows[2], ["表格", "待验", "跨页"]);
    }

    #[test]
    fn ordinary_single_cell_lines_are_not_reported_as_tables() {
        assert!(
            extract(&[block(&[
                &["这是一段普通正文，没有多个单元格。"],
                &["第二行。"],
                &["第三行。"]
            ])])
            .is_empty()
        );
    }
}
