//! Deterministic text cleaning for derived source text. Originals stay byte for
//! byte: cleaning only shapes the blocks that feed reading, search and indexing,
//! and every parse keeps a report of what was removed. A step that would leave
//! the document with no text at all is skipped and reported instead.
use crate::*;
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

/// A paged document needs at least this many pages before repeated page edges or
/// repeated short text can be recognised as boilerplate.
const MIN_PAGED_PAGES: u32 = 3;
/// The same text has to repeat on at least this many pages ...
const MIN_REPEAT_PAGES: u32 = 3;
/// ... and on at least half of them.
const REPEAT_SHARE: u32 = 2;
/// Page edges longer than this are treated as content, not as a running header.
const MAX_EDGE_CHARS: usize = 120;
/// Repeated short text is treated as a watermark up to this length.
const MAX_WATERMARK_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleaningParams {
    /// Repeated first or last block of a page in a paged document.
    pub headers_footers: bool,
    /// Short text repeated on most pages of a paged document.
    pub watermark: bool,
    /// Control and format characters that carry no text of their own.
    pub control_chars: bool,
}
impl Default for CleaningParams {
    fn default() -> Self {
        Self {
            headers_footers: true,
            watermark: true,
            control_chars: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleaningReport {
    pub params: CleaningParams,
    /// Blocks removed as running headers or footers.
    pub headers_footers: u32,
    /// Blocks removed as repeated short text.
    pub watermark: u32,
    /// Control or format characters removed from block text.
    pub control_chars: u32,
    pub removed_blocks: u32,
    pub removed_chars: u32,
    /// Replacement characters kept in the text: they mark a decoding problem and
    /// are counted instead of being deleted.
    pub replacement_chars: u32,
    /// True when a step was skipped because it would have emptied the document.
    pub kept_everything: bool,
}

impl CleaningReport {
    pub fn changed(&self) -> bool {
        self.removed_blocks > 0 || self.control_chars > 0
    }
}

#[derive(Clone, Copy)]
enum Step {
    HeaderFooter,
    Watermark,
}

/// Format characters that carry no text of their own (Unicode category Cf).
fn is_strippable(ch: char) -> bool {
    ch != '\n'
        && ch != '\t'
        && (ch.is_control()
            || matches!(
                ch,
                '\u{00AD}'
                    | '\u{061C}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
            ))
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pages are only available when every block belongs to one, so a paged format
/// that failed to record pages keeps its text instead of being guessed at.
fn page_blocks(blocks: &[SourceBlock]) -> Option<BTreeMap<u32, Vec<usize>>> {
    let mut pages: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        pages.entry(block.page_index?).or_default().push(index);
    }
    (pages.len() as u32 >= MIN_PAGED_PAGES).then_some(pages)
}

fn remove_repeats(
    blocks: &mut Vec<SourceBlock>,
    keys: &BTreeSet<String>,
    report: &mut CleaningReport,
    step: Step,
) {
    if keys.is_empty() {
        return;
    }
    let keep = blocks
        .iter()
        .map(|block| !keys.contains(&normalized(&block.text)))
        .collect::<Vec<_>>();
    if !keep.iter().any(|keep| *keep) {
        report.kept_everything = true;
        return;
    }
    let mut removed_blocks = 0u32;
    let mut removed_chars = 0u32;
    let mut kept = Vec::with_capacity(blocks.len());
    for (block, keep) in std::mem::take(blocks).into_iter().zip(keep) {
        if keep {
            kept.push(block);
        } else {
            removed_blocks += 1;
            removed_chars += block.text.chars().count() as u32;
        }
    }
    *blocks = kept;
    report.removed_blocks += removed_blocks;
    report.removed_chars += removed_chars;
    match step {
        Step::HeaderFooter => report.headers_footers += removed_blocks,
        Step::Watermark => report.watermark += removed_blocks,
    }
}

/// Removes control and format characters while keeping PDF character boxes in
/// step with the text they describe.
fn strip_control_characters(blocks: &mut [SourceBlock], report: &mut CleaningReport) {
    for block in blocks.iter_mut() {
        let geometry = block.pdf_geometry.take();
        let mut kept_boxes = geometry.as_ref().map(|_| Vec::new());
        let mut text = String::with_capacity(block.text.len());
        let mut removed = 0u32;
        for (index, ch) in block.text.chars().enumerate() {
            if ch == '\u{FFFD}' {
                report.replacement_chars += 1;
            } else if is_strippable(ch) {
                report.control_chars += 1;
                removed += 1;
                continue;
            }
            text.push(ch);
            if let Some(boxes) = kept_boxes.as_mut()
                && let Some(cell) = geometry.as_ref().and_then(|value| value.chars.get(index))
            {
                // Removing a control character also removes its box; the
                // remaining boxes must be renumbered or every later PDF
                // selection/table lookup would point one character ahead.
                let mut cell = cell.clone();
                let index = boxes.len() as u32;
                cell.start_codepoint = index;
                cell.end_codepoint = index + 1;
                boxes.push(cell);
            }
        }
        if removed > 0 {
            block.text = text;
            if let (Some(mut value), Some(boxes)) = (geometry, kept_boxes) {
                value.chars = boxes;
                block.pdf_geometry = Some(value);
            }
        } else {
            block.pdf_geometry = geometry;
        }
    }
}

pub fn clean_blocks(blocks: &mut Vec<SourceBlock>, params: &CleaningParams) -> CleaningReport {
    let mut report = CleaningReport {
        params: params.clone(),
        ..CleaningReport::default()
    };
    if params.control_chars {
        strip_control_characters(blocks, &mut report);
    }
    if params.headers_footers
        && let Some(pages) = page_blocks(blocks)
    {
        let total = pages.len() as u32;
        let mut edges: BTreeMap<String, u32> = BTreeMap::new();
        for indices in pages.values() {
            let mut seen = BTreeSet::new();
            for index in [indices.first(), indices.last()].into_iter().flatten() {
                let key = normalized(&blocks[*index].text);
                if !key.is_empty() && seen.insert(key.clone()) {
                    *edges.entry(key).or_default() += 1;
                }
            }
        }
        let boilerplate = edges
            .into_iter()
            .filter(|(key, count)| {
                *count >= MIN_REPEAT_PAGES
                    && *count * REPEAT_SHARE >= total
                    && key.chars().count() <= MAX_EDGE_CHARS
            })
            .map(|(key, _)| key)
            .collect();
        remove_repeats(blocks, &boilerplate, &mut report, Step::HeaderFooter);
    }
    if params.watermark
        && let Some(pages) = page_blocks(blocks)
    {
        let total = pages.len() as u32;
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for indices in pages.values() {
            let mut seen = BTreeSet::new();
            for index in indices {
                let key = normalized(&blocks[*index].text);
                if !key.is_empty() && seen.insert(key.clone()) {
                    *counts.entry(key).or_default() += 1;
                }
            }
        }
        let marks = counts
            .into_iter()
            .filter(|(key, count)| {
                *count >= MIN_REPEAT_PAGES
                    && *count * REPEAT_SHARE >= total
                    && key.chars().count() <= MAX_WATERMARK_CHARS
            })
            .map(|(key, _)| key)
            .collect();
        remove_repeats(blocks, &marks, &mut report, Step::Watermark);
    }
    report
}

pub(crate) fn cleaning_settings(db: &Connection) -> Result<CleaningSettings> {
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT cleaning_params_json,cleaning_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let params = match json {
        Some(json) => serde_json::from_str(&json)?,
        None => CleaningParams::default(),
    };
    Ok(CleaningSettings {
        params,
        row_version,
    })
}

/// Backups from before schema 22 have no column to check; a stored value must
/// always be internally consistent with its row version.
pub(crate) fn validate_cleaning_settings_backup(db: &Connection) -> Result<()> {
    let schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema < 22 {
        return Ok(());
    }
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT cleaning_params_json,cleaning_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match json {
        Some(json) => {
            serde_json::from_str::<CleaningParams>(&json).map_err(|_| Error::IntegrityFailure)?;
            if row_version == 0 {
                return Err(Error::IntegrityFailure);
            }
        }
        None if row_version != 0 => return Err(Error::IntegrityFailure),
        None => {}
    }
    Ok(())
}

impl Library {
    pub fn cleaning_settings(&self) -> Result<CleaningSettings> {
        cleaning_settings(&self.connection)
    }

    pub fn set_cleaning_params(
        &mut self,
        command: &str,
        request: SetCleaningParams,
    ) -> Result<CleaningSettingsMutation> {
        let kind = "cleaning.configure";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let old = cleaning_settings(&tx)?;
        if old.row_version != request.base_row_version {
            return Err(Error::MetadataConflict {
                object_id: "cleaning_settings".into(),
                expected: request.base_row_version,
                actual: old.row_version,
            });
        }
        let row_version = old
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput("cleaning settings version overflow"))?;
        tx.execute(
            "UPDATE search_settings SET cleaning_params_json=?1,cleaning_row_version=?2 WHERE singleton=1",
            params![serde_json::to_string(&request.params)?, row_version],
        )?;
        let result = CleaningSettingsMutation {
            settings: CleaningSettings {
                params: request.params,
                row_version,
            },
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            kind,
            "cleaning_settings",
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleaningSettings {
    pub params: CleaningParams,
    pub row_version: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SetCleaningParams {
    pub base_row_version: u32,
    pub params: CleaningParams,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleaningSettingsMutation {
    pub settings: CleaningSettings,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for CleaningSettingsMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::{PdfBlockGeometry, PdfCharacterBox, SourceBlock};

    #[test]
    fn stripping_control_characters_renumbers_pdf_boxes() {
        let mut blocks = vec![SourceBlock {
            id: "block".into(),
            text: "项\r\n目".into(),
            page_index: Some(0),
            pdf_geometry: Some(PdfBlockGeometry {
                page_label: None,
                crop_box: [0.0, 0.0, 100.0, 100.0],
                rotation: 0,
                coordinate_space: "pdf_user_space".into(),
                chars: vec![
                    PdfCharacterBox {
                        start_codepoint: 0,
                        end_codepoint: 1,
                        rect: Some([1.0, 1.0, 2.0, 2.0]),
                    },
                    PdfCharacterBox {
                        start_codepoint: 1,
                        end_codepoint: 2,
                        rect: None,
                    },
                    PdfCharacterBox {
                        start_codepoint: 2,
                        end_codepoint: 3,
                        rect: None,
                    },
                    PdfCharacterBox {
                        start_codepoint: 3,
                        end_codepoint: 4,
                        rect: Some([3.0, 3.0, 4.0, 4.0]),
                    },
                ],
            }),
        }];
        let report = clean_blocks(&mut blocks, &CleaningParams::default());
        assert_eq!(report.control_chars, 1);
        assert_eq!(blocks[0].text, "项\n目");
        let chars = &blocks[0].pdf_geometry.as_ref().unwrap().chars;
        assert_eq!(chars[0].start_codepoint, 0);
        assert_eq!(chars[1].start_codepoint, 1);
        assert_eq!(chars[1].rect, None);
        assert_eq!(chars[2].start_codepoint, 2);
        assert_eq!(chars[2].rect, Some([3.0, 3.0, 4.0, 4.0]));
    }
}
