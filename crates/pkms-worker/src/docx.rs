//! DOCX text extraction: the body's paragraphs and table cells become the same
//! text blocks the other parsers produce. Styling, revision marks and embedded
//! objects are not interpreted, and the original container is stored unchanged.
use crate::{Result, WorkerRequest, error};
use pkms_core::{ParsedSource, SourceBlock};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::io::{Cursor, Read};

/// The body XML is capped before parsing; only its text survives into blocks.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BLOCK_CHARS: usize = 16 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_TEXT_CHARS: usize = 16 * 1024 * 1024;
const MAX_METADATA_CHARS: usize = 300;

/// Reads one part without trusting its declared size, so a crafted container
/// cannot expand past the limit while being decompressed.
fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| error("docx_part_missing"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("io_error"))?;
    if bytes.len() as u64 > MAX_PART_BYTES {
        return Err(error("docx_input_limit"));
    }
    Ok(bytes)
}

fn push(blocks: &mut Vec<SourceBlock>, text: String) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    if blocks.len() >= MAX_BLOCKS {
        return Err(error("docx_structure_limit"));
    }
    blocks.push(SourceBlock {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        page_index: None,
        pdf_geometry: None,
    });
    Ok(())
}

/// A paragraph or table cell ends one block; an explicit line break inside it
/// ends one too, so breaks are not flattened away.
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

fn body_blocks(xml: &[u8]) -> Result<Vec<SourceBlock>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut in_text = false;
    // Field instructions and deleted revisions are markup, not document text.
    let mut skipped = 0usize;
    let mut total = 0usize;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.local_name().into_inner() {
                "t" => in_text = true,
                "delText" | "instrText" => skipped += 1,
                "tab" => paragraph.push(' '),
                "br" | "cr" => paragraph.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().into_inner() {
                "tab" => paragraph.push(' '),
                "br" | "cr" => paragraph.push('\n'),
                _ => {}
            },
            Ok(Event::End(event)) => match event.local_name().into_inner() {
                "t" => in_text = false,
                "delText" | "instrText" => skipped = skipped.saturating_sub(1),
                "p" | "tc" => flush(&mut blocks, &mut paragraph)?,
                _ => {}
            },
            Ok(Event::Text(event)) if in_text && skipped == 0 => {
                let raw = event.xml10_content();
                let text =
                    quick_xml::escape::unescape(&raw).map_err(|_| error("docx_invalid_xml"))?;
                total += text.chars().count();
                if total > MAX_TEXT_CHARS {
                    return Err(error("docx_text_limit"));
                }
                paragraph.push_str(&text);
            }
            // The reader reports `&amp;` and `&#x4E2D;` as their own events.
            Ok(Event::GeneralRef(event)) if in_text && skipped == 0 => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                let resolved = quick_xml::escape::unescape(&reference)
                    .map_err(|_| error("docx_invalid_xml"))?;
                total += resolved.chars().count();
                if total > MAX_TEXT_CHARS {
                    return Err(error("docx_text_limit"));
                }
                paragraph.push_str(&resolved);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(error("docx_invalid_xml")),
        }
        buffer.clear();
    }
    flush(&mut blocks, &mut paragraph)?;
    Ok(blocks)
}

/// One text value from the package's core properties, when it has one.
fn core_value(xml: &[u8], name: &str) -> Option<String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_value = false;
    let mut value = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) if event.local_name().into_inner() == name => in_value = true,
            Ok(Event::End(event)) if event.local_name().into_inner() == name => in_value = false,
            Ok(Event::Text(event)) if in_value => {
                let raw = event.xml10_content();
                if let Ok(text) = quick_xml::escape::unescape(&raw) {
                    value.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(event)) if in_value => {
                let raw = event.xml10_content();
                let reference = format!("&{raw};");
                if let Ok(text) = quick_xml::escape::unescape(&reference) {
                    value.push_str(&text);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buffer.clear();
    }
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!value.is_empty()).then(|| value.chars().take(MAX_METADATA_CHARS).collect())
}

pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| error("docx_not_container"))?;
    let document = part(&mut archive, "word/document.xml")?;
    let blocks = body_blocks(&document)?;
    if blocks.is_empty() {
        return Err(error("docx_no_content"));
    }
    let core = part(&mut archive, "docProps/core.xml").ok();
    let title = core.as_deref().and_then(|core| core_value(core, "title"));
    // The author comes from the file itself; a package without one stays None.
    let author = core.as_deref().and_then(|core| core_value(core, "creator"));
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version: "ooxml-wordprocessing-v1".into(),
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
