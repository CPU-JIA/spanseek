//! MOBI/PalmDOC extraction. The crate decodes no-compression, PalmDOC and Huff
//! records; the original container stays immutable and only readable text is
//! handed to the existing HTML block extractor.
use crate::{Result, WorkerRequest, error, html};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mobi::{
    Mobi,
    headers::{Encryption, ExthRecord},
};
use pkms_core::{ParsedCover, ParsedSource, ParsedSourceImage, SourceBlock, SourceTocItem};
use scraper::node::Element;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Keep the decompressed book compatible with the worker's bounded result and
/// the HTML reader's text cap. Checking the PalmDOC declaration avoids asking
/// the decompressor to allocate an unbounded string first.
const MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;

fn metadata(value: String) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then(|| normalized.chars().take(300).collect())
}

fn html_error(failure: crate::WorkerError) -> crate::WorkerError {
    match failure.code {
        "html_no_content" | "webpage_dynamic_content" | "webpage_login_required" => {
            error("mobi_no_content")
        }
        "html_text_limit" => error("mobi_text_limit"),
        "html_structure_limit" => error("mobi_structure_limit"),
        "html_invalid_encoding" => error("mobi_invalid_text"),
        _ => failure,
    }
}

const MAX_TOC_ITEMS: usize = 5_000;
const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const MAX_IMAGE_TOTAL_BYTES: usize = 16 * 1024 * 1024;

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
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

fn clean_alt(element: &Element) -> String {
    element
        .attr("alt")
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

fn base32_image_number(value: &str) -> Option<usize> {
    if value.is_empty() || value.len() > 16 {
        return None;
    }
    let mut number = 0usize;
    for byte in value.bytes() {
        let digit = match byte {
            b'0'..=b'9' => usize::from(byte - b'0'),
            b'A'..=b'V' => usize::from(byte - b'A') + 10,
            b'a'..=b'v' => usize::from(byte - b'a') + 10,
            _ => return None,
        };
        number = number.checked_mul(32)?.checked_add(digit)?;
    }
    Some(number)
}

fn kindle_embed_number(value: &str) -> Option<usize> {
    let lower = value.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("kindle:embed:")?;
    base32_image_number(rest.split(['?', '#']).next().unwrap_or_default())
}

fn image_record_number(element: &Element) -> Option<usize> {
    ["lowrecindex", "recindex", "hirecindex"]
        .iter()
        .find_map(|attribute| {
            element
                .attr(attribute)?
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|number| *number > 0)
        })
        .or_else(|| {
            element
                .attr("src")
                .or_else(|| element.attr("href"))
                .or_else(|| element.attr("xlink:href"))
                .and_then(kindle_embed_number)
        })
}

fn data_image(value: &str) -> Option<(String, Vec<u8>)> {
    let (metadata, payload) = value.strip_prefix("data:")?.split_once(',')?;
    let media_type = metadata.strip_suffix(";base64")?.to_ascii_lowercase();
    if !matches!(
        media_type.as_str(),
        "image/png" | "image/jpeg" | "image/webp"
    ) {
        return None;
    }
    let bytes = STANDARD.decode(payload.trim()).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    (image_media_type(&bytes) == Some(media_type.as_str())).then_some((media_type, bytes))
}

enum ImageSource {
    Record(usize),
    Data(Vec<u8>),
}

struct ImageDraft {
    source: ImageSource,
    alt: String,
}

fn image_bytes(book: &Mobi, number: usize) -> Option<(&'static str, Vec<u8>)> {
    let first = book.metadata.mobi.first_image_index as usize;
    if first == u32::MAX as usize || number == 0 {
        return None;
    }
    let index = first.checked_add(number - 1)?;
    let bytes = book.raw_records().records().get(index)?.content;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let media_type = image_media_type(bytes)?;
    Some((media_type, bytes.to_vec()))
}

fn cover(book: &Mobi) -> Option<ParsedCover> {
    let raw = book
        .metadata
        .exth
        .get_record(ExthRecord::CoverOffset)?
        .first()?;
    if raw.len() != 4 {
        return None;
    }
    let offset = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let index = (book.metadata.mobi.first_image_index as usize).checked_add(offset)?;
    let records = book.raw_records();
    let bytes = records.records().get(index)?.content;
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    let media_type = image_media_type(bytes)?;
    Some(ParsedCover {
        media_type: media_type.to_owned(),
        data_base64: STANDARD.encode(bytes),
    })
}

fn toc(content: &str, blocks: &[SourceBlock]) -> Vec<SourceTocItem> {
    let document = scraper::Html::parse_document(content);
    let Ok(selector) = scraper::Selector::parse("h1,h2,h3,h4,h5,h6") else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut cursor = 0usize;
    for heading in document.select(&selector) {
        let title = normalize(&heading.text().collect::<Vec<_>>().join(" "))
            .chars()
            .take(300)
            .collect::<String>();
        if title.is_empty() {
            continue;
        }
        let level = heading
            .value()
            .name()
            .trim_start_matches('h')
            .parse::<u32>()
            .unwrap_or(1)
            .min(16);
        let found = blocks[cursor..]
            .iter()
            .position(|block| normalize(&block.text).contains(&title))
            .map(|offset| cursor + offset);
        if let Some(position) = found {
            let block = &blocks[position];
            items.push(SourceTocItem {
                title,
                level,
                block_id: Some(block.id.clone()),
                block_position: Some(position as u32),
            });
            cursor = position.saturating_add(1);
        }
        if items.len() >= MAX_TOC_ITEMS {
            break;
        }
    }
    items
}
pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    catch_unwind(AssertUnwindSafe(|| parse_inner(request, bytes)))
        .map_err(|_| error("mobi_invalid_container"))?
}

fn parse_inner(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let book = Mobi::new(bytes.to_vec()).map_err(|_| error("mobi_invalid_container"))?;
    if book.encryption() != Encryption::No {
        return Err(error("mobi_encrypted"));
    }
    if book.metadata.palmdoc.text_length as usize > MAX_DECODED_BYTES {
        return Err(error("mobi_text_limit"));
    }
    let content = book
        .content_as_string()
        .map_err(|_| error("mobi_invalid_text"))?;
    if content.trim().is_empty() {
        return Err(error("mobi_no_content"));
    }
    let mut drafts = Vec::new();
    let (mut parsed, anchors) = html::parse_text_with_images(request, &content, |element| {
        let alt = clean_alt(element);
        let embedded = element.attr("src").and_then(data_image);
        let record = image_record_number(element);
        if embedded.is_none() && record.is_none() && alt.is_empty() {
            return None;
        }
        if drafts.len() < MAX_IMAGES {
            let source = embedded
                .map(|(_, bytes)| ImageSource::Data(bytes))
                .or_else(|| record.map(ImageSource::Record));
            if let Some(source) = source {
                drafts.push(ImageDraft {
                    source,
                    alt: alt.clone(),
                });
            }
        }
        Some(if alt.is_empty() {
            "Image".to_owned()
        } else {
            alt
        })
    })
    .map_err(html_error)?;
    parsed.parser_version = "mobi-v3".into();
    if let Some(title) = metadata(book.title()) {
        parsed.title = Some(title);
    }
    if let Some(author) = book.author().and_then(metadata) {
        parsed.author = Some(author);
    }
    parsed.toc = toc(&content, &parsed.blocks);
    parsed.cover = cover(&book);
    let mut image_total = 0usize;
    for (draft, anchor) in drafts.into_iter().zip(anchors) {
        let Some((media_type, bytes)) = (match draft.source {
            ImageSource::Record(number) => image_bytes(&book, number),
            ImageSource::Data(bytes) => {
                image_media_type(&bytes).map(|media_type| (media_type, bytes))
            }
        }) else {
            continue;
        };
        image_total = image_total
            .checked_add(bytes.len())
            .ok_or_else(|| error("mobi_asset_limit"))?;
        if image_total > MAX_IMAGE_TOTAL_BYTES {
            return Err(error("mobi_asset_limit"));
        }
        parsed.images.push(ParsedSourceImage {
            block_position: anchor.block_position,
            media_type: media_type.to_owned(),
            data_base64: STANDARD.encode(bytes),
            alt: draft.alt,
        });
    }
    Ok(parsed)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_normalized_and_bounded() {
        assert_eq!(metadata("  A\tB  ".into()).as_deref(), Some("A B"));
        assert_eq!(metadata(" \r\n ".into()), None);
        let long = "字".repeat(400);
        assert_eq!(metadata(long).unwrap().chars().count(), 300);
    }

    #[test]
    fn container_html_failures_keep_mobi_specific_codes() {
        assert_eq!(html_error(error("html_no_content")).code, "mobi_no_content");
        assert_eq!(html_error(error("html_text_limit")).code, "mobi_text_limit");
        assert_eq!(
            html_error(error("html_structure_limit")).code,
            "mobi_structure_limit"
        );
        assert_eq!(html_error(error("io_error")).code, "io_error");
    }
}
