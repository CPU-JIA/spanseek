use crate::ocr::{OcrLine, space_before};
use crate::{
    MAX_PAGES, MAX_TEXT_BYTES, OcrConfig, PDFIUM_DLL_SHA256, Result, WorkerRequest, error,
};
use pdfium_render::prelude::*;
use pkms_core::{ParsedSource, PdfBlockGeometry, PdfCharacterBox, SourceBlock};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_OCR_PAGES: usize = 50;
const MAX_OCR_PIXELS: i64 = 40_000_000;

fn pdf_error(e: PdfiumError) -> crate::WorkerError {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            error("pdf_password_required")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            error("pdf_invalid")
        }
        _ => error("pdf_parse_failed"),
    }
}
fn rect(r: PdfRect) -> [f64; 4] {
    [
        r.left().value as f64,
        r.bottom().value as f64,
        r.right().value as f64,
        r.top().value as f64,
    ]
}

/// Rasterizes one page and runs the configured OCR engine. The PNG exists only
/// for the duration of this call.
fn ocr_page(
    page: &PdfPage<'_>,
    config: &OcrConfig,
    page_index: usize,
    crop_box: [f64; 4],
    output_dir: &Path,
) -> Result<(Vec<OcrLine>, i32, i32)> {
    let width_points = (crop_box[2] - crop_box[0]).abs();
    if width_points < 1.0 {
        return Ok((Vec::new(), 0, 0));
    }
    let target_width =
        ((width_points * f64::from(config.dpi) / 72.0).round() as i32).clamp(200, 5000);
    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(target_width)
                .set_format(PdfBitmapFormat::BGRA),
        )
        .map_err(pdf_error)?;
    let (width, height) = (bitmap.width(), bitmap.height());
    if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > MAX_OCR_PIXELS {
        return Err(error("ocr_page_limit"));
    }
    let image = image::RgbaImage::from_raw(width as u32, height as u32, bitmap.as_rgba_bytes())
        .ok_or_else(|| error("ocr_render_failed"))?;
    let path = output_dir.join(format!("ocr-{page_index}.png"));
    image.save(&path).map_err(|_| error("ocr_render_failed"))?;
    let result = crate::ocr::recognize(config, &path);
    let _ = std::fs::remove_file(&path);
    let lines = result?;
    Ok((lines, width, height))
}

/// Tesseract exposes word boxes, not character boxes. Every character receives
/// its word rectangle; that is enough for a visible highlight but is not a
/// character-accurate glyph box. Rotated pages keep the text without geometry.
fn ocr_line_geometry(
    line: &OcrLine,
    width: i32,
    height: i32,
    crop_box: [f64; 4],
    rotation: u16,
) -> (String, Vec<PdfCharacterBox>) {
    let mut text = String::new();
    let mut chars = Vec::new();
    let mut codepoint = 0u32;
    let width = f64::from(width);
    let height = f64::from(height);
    let crop_width = crop_box[2] - crop_box[0];
    let crop_height = crop_box[3] - crop_box[1];
    for (index, word) in line.words.iter().enumerate() {
        if space_before(&line.words, word, index) {
            text.push(' ');
            chars.push(PdfCharacterBox {
                start_codepoint: codepoint,
                end_codepoint: codepoint + 1,
                rect: None,
            });
            codepoint += 1;
        }
        let rect = (rotation == 0).then(|| {
            let x0 = crop_box[0] + word.left / width * crop_width;
            let x1 = crop_box[0] + (word.left + word.width) / width * crop_width;
            let y1 = crop_box[3] - word.top / height * crop_height;
            let y0 = crop_box[3] - (word.top + word.height) / height * crop_height;
            [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
        });
        for c in word.text.chars() {
            text.push(c);
            chars.push(PdfCharacterBox {
                start_codepoint: codepoint,
                end_codepoint: codepoint + 1,
                rect,
            });
            codepoint += 1;
        }
    }
    (text, chars)
}
pub(crate) fn parse(request: &WorkerRequest, bytes: &[u8]) -> Result<ParsedSource> {
    let path = request
        .pdfium_library_path
        .as_ref()
        .ok_or_else(|| error("pdfium_unavailable"))?;
    let dll = crate::read_bounded(crate::regular_file(path)?, 32 * 1024 * 1024)?;
    if format!("{:x}", Sha256::digest(&dll)) != PDFIUM_DLL_SHA256 {
        return Err(error("pdfium_hash_mismatch"));
    }
    let bindings = Pdfium::bind_to_library(path).map_err(|_| error("pdfium_unavailable"))?;
    let pdfium = Pdfium::new(bindings);
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(pdf_error)?;
    let page_count = document.pages().len() as usize;
    if page_count > MAX_PAGES {
        return Err(error("pdf_page_limit"));
    }
    let mut blocks = Vec::new();
    let mut total = 0;
    let mut char_count = 0;
    let mut needs_ocr = false;
    let mut ocr_pages = Vec::new();
    let mut ocr_capped = false;
    for (page_index, page) in document.pages().iter().enumerate() {
        let media = page.boundaries().media().map_err(pdf_error)?.bounds;
        // PDF specifies CropBox defaults to MediaBox when absent.
        let crop = page.boundaries().crop().map(|b| b.bounds).unwrap_or(media);
        let crop_box = rect(crop);
        let rotation = page.rotation().map_err(pdf_error)?.as_degrees() as u16;
        let base = PdfBlockGeometry {
            page_label: page.label().map(str::to_owned),
            crop_box,
            rotation,
            coordinate_space: "pdf_user_space".into(),
            chars: Vec::new(),
        };
        let text = page.text().map_err(pdf_error)?;
        let mut block = SourceBlock {
            id: uuid::Uuid::new_v4().to_string(),
            text: String::new(),
            page_index: Some(page_index as u32),
            pdf_geometry: Some(base.clone()),
        };
        let mut has_text = false;
        for ch in text.chars().iter() {
            let Some(c) = ch.unicode_char() else {
                return Err(error("pdf_invalid_unicode"));
            };
            if c == '\0' {
                continue;
            }
            total += c.len_utf8();
            char_count += 1;
            // Geometry is considerably larger than text: bound memory before constructing JSON.
            if total > MAX_TEXT_BYTES || char_count > 200_000 {
                return Err(error("pdf_text_limit"));
            }
            if block.text.len() + c.len_utf8() > 16 * 1024 {
                blocks.push(block);
                block = SourceBlock {
                    id: uuid::Uuid::new_v4().to_string(),
                    text: String::new(),
                    page_index: Some(page_index as u32),
                    pdf_geometry: Some(base.clone()),
                };
            }
            let geometry = block.pdf_geometry.as_mut().unwrap();
            let index = geometry.chars.len() as u32;
            let bounds = if c.is_whitespace() {
                None
            } else {
                ch.tight_bounds()
                    .ok()
                    .map(rect)
                    .filter(|r| r.iter().all(|v| v.is_finite()) && r[2] > r[0] && r[3] > r[1])
            };
            geometry.chars.push(PdfCharacterBox {
                start_codepoint: index,
                end_codepoint: index + 1,
                rect: bounds,
            });
            block.text.push(c);
            has_text |= !c.is_whitespace();
        }
        if !has_text && let Some(config) = request.ocr.as_ref() {
            if ocr_pages.len() >= MAX_OCR_PAGES {
                ocr_capped = true;
            } else if config.executable.is_file() && config.tessdata_dir.is_dir() {
                match ocr_page(&page, config, page_index, crop_box, &request.output_dir) {
                    Ok((lines, width, height)) if !lines.is_empty() => {
                        for line in lines {
                            let (line_text, chars) =
                                ocr_line_geometry(&line, width, height, crop_box, rotation);
                            if line_text.trim().is_empty() {
                                continue;
                            }
                            total += line_text.len();
                            char_count += line_text.chars().count();
                            if total > MAX_TEXT_BYTES || char_count > 200_000 {
                                return Err(error("pdf_text_limit"));
                            }
                            let mut geometry = base.clone();
                            geometry.chars = chars;
                            blocks.push(SourceBlock {
                                id: uuid::Uuid::new_v4().to_string(),
                                text: line_text,
                                page_index: Some(page_index as u32),
                                pdf_geometry: Some(geometry),
                            });
                        }
                        has_text = true;
                        ocr_pages.push(page_index as u32);
                    }
                    // OCR is best effort. A failure leaves the page flagged for
                    // OCR instead of failing a document whose text layer is intact
                    // elsewhere.
                    Ok(_) | Err(_) => {}
                }
            }
        }
        needs_ocr |= !has_text;
        if !block.text.is_empty() {
            blocks.push(block);
        }
    }
    let parser_version = if ocr_pages.is_empty() {
        "pdfium-7881-render-0.9.4-char-v1".to_owned()
    } else if ocr_capped {
        "pdfium-7881-render-0.9.4-char-v1+ocr-tesseract5-v1+capped".to_owned()
    } else {
        "pdfium-7881-render-0.9.4-char-v1+ocr-tesseract5-v1".to_owned()
    }; // The author comes from the PDF's own document information dictionary.
    let author = document
        .metadata()
        .get(PdfDocumentMetadataTagType::Author)
        .map(|tag| tag.value().split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|author| !author.is_empty())
        .map(|author| author.chars().take(300).collect::<String>());
    Ok(ParsedSource {
        source_version_id: request.source_version_id.clone(),
        parser_version,
        blocks,
        page_count: Some(page_count as u32),
        needs_ocr,
        title: None,
        author,
        toc: Vec::new(),
        cover: None,
        images: Vec::new(),
    })
}
