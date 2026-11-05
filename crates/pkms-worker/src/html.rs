//! Readable text extraction only: never runs scripts, styles, frames, or external requests.
use crate::{WorkerRequest, error};
use pkms_core::{ParsedSource, SourceBlock};
use scraper::{ElementRef, Html, Node, Selector};

#[derive(Debug)]
pub(crate) struct InlineImageAnchor {
    pub block_position: u32,
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static selector")
}
fn charset(value: &str) -> Option<String> {
    value.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['\'', '"']).to_owned())
    })
}
fn decode(bytes: &[u8], content_type: Option<&str>) -> Result<String, crate::WorkerError> {
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(error("html_input_limit"));
    }
    let mut encoding = content_type
        .and_then(charset)
        .and_then(|name| encoding_rs::Encoding::for_label(name.as_bytes()));
    if encoding.is_none() {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let fragment = Html::parse_document(&head);
        for meta in fragment.select(&selector("meta")) {
            let name = meta.value().attr("charset").map(str::to_owned).or_else(|| {
                meta.value()
                    .attr("http-equiv")
                    .filter(|value| value.eq_ignore_ascii_case("content-type"))?;
                charset(meta.value().attr("content")?)
            });
            if let Some(found) =
                name.and_then(|name| encoding_rs::Encoding::for_label(name.as_bytes()))
            {
                encoding = Some(found);
                break;
            }
        }
    }
    let (encoding, skip) = encoding_rs::Encoding::for_bom(bytes)
        .unwrap_or((encoding.unwrap_or(encoding_rs::UTF_8), 0));
    let (text, _, errors) = encoding.decode(&bytes[skip..]);
    if errors || text.contains('\0') {
        return Err(error("html_invalid_encoding"));
    }
    Ok(text.into_owned())
}
fn hidden(element: &scraper::node::Element) -> bool {
    matches!(
        element.name(),
        "script"
            | "style"
            | "nav"
            | "header"
            | "footer"
            | "aside"
            | "form"
            | "button"
            | "input"
            | "textarea"
            | "select"
            | "noscript"
            | "template"
            | "svg"
            | "canvas"
            | "iframe"
            | "object"
            | "embed"
    ) || element.attr("hidden").is_some()
        || element
            .attr("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || element.attr("style").is_some_and(|value| {
            let style = value.to_ascii_lowercase().replace(' ', "");
            style.contains("display:none") || style.contains("visibility:hidden")
        })
}
fn block_name(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "pre"
            | "blockquote"
            | "div"
            | "section"
            | "article"
            | "main"
            | "td"
            | "th"
            | "figcaption"
    )
}
fn append_block(
    blocks: &mut Vec<SourceBlock>,
    text: &mut String,
) -> Result<(), crate::WorkerError> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.clear();
    let mut chunk = String::new();
    for character in normalized.chars() {
        if chunk.len() + character.len_utf8() > 16 * 1024 {
            push(blocks, std::mem::take(&mut chunk))?;
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        push(blocks, chunk)?;
    }
    Ok(())
}
fn push(blocks: &mut Vec<SourceBlock>, text: String) -> Result<(), crate::WorkerError> {
    if blocks.len() >= 100_000 {
        return Err(error("html_structure_limit"));
    }
    blocks.push(SourceBlock {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        page_index: None,
        pdf_geometry: None,
    });
    Ok(())
}
pub(crate) fn parse(
    request: &WorkerRequest,
    bytes: &[u8],
) -> Result<ParsedSource, crate::WorkerError> {
    let text = decode(bytes, request.content_type.as_deref())?;
    parse_text(request, &text)
}

/// Parses text that a container parser has already decoded. The caller owns the
/// container's encoding and size checks; block and text caps still apply here.
pub(crate) fn parse_text(
    request: &WorkerRequest,
    text: &str,
) -> Result<ParsedSource, crate::WorkerError> {
    parse_text_with_images(request, text, |_| None).map(|(source, _)| source)
}

pub(crate) fn parse_text_with_images<F>(
    request: &WorkerRequest,
    text: &str,
    mut image_label: F,
) -> Result<(ParsedSource, Vec<InlineImageAnchor>), crate::WorkerError>
where
    F: FnMut(&scraper::node::Element) -> Option<String>,
{
    let document = Html::parse_document(text);
    let title = document
        .select(&selector("title"))
        .next()
        .map(|node| {
            node.text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(300)
                .collect::<String>()
        })
        .filter(|title| !title.is_empty());
    // The page's own author, when it declares one through a meta tag.
    let author = document
        .select(&selector("meta"))
        .filter_map(|meta| {
            let key = meta
                .value()
                .attr("name")
                .or_else(|| meta.value().attr("property"))?;
            matches!(
                key.to_ascii_lowercase().as_str(),
                "author" | "dc.creator" | "article:author" | "og:article:author"
            )
            .then(|| meta.value().attr("content"))?
        })
        .map(|content| content.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|content| !content.is_empty())
        .map(|content| content.chars().take(300).collect::<String>());
    let main = document
        .select(&selector("article, main, [role=main]"))
        .take(100)
        .max_by_key(|element| element.text().map(str::len).sum::<usize>());
    let has_main = main.is_some();
    let root = main
        .or_else(|| document.select(&selector("body")).next())
        .ok_or_else(|| error("html_no_content"))?;
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut last_block = None;
    let mut total = 0usize;
    let mut pending_images = Vec::new();
    for (index, node) in root.descendants().enumerate() {
        if index >= 200_000 {
            return Err(error("html_structure_limit"));
        }
        let replacement = match node.value() {
            Node::Text(value) if !value.trim().is_empty() => Some(value.to_string()),
            Node::Element(element) if element.name() == "img" => image_label(element),
            _ => None,
        };
        let Some(replacement) = replacement else {
            if matches!(node.value(), Node::Text(_)) && !text.is_empty() {
                text.push(' ');
            }
            continue;
        };
        let mut skip = false;
        let mut block = None;
        for (depth, ancestor) in node.ancestors().enumerate() {
            if depth >= 256 {
                return Err(error("html_structure_limit"));
            }
            if let Some(element) = ElementRef::wrap(ancestor) {
                if hidden(element.value()) {
                    skip = true;
                    break;
                }
                if block.is_none() && block_name(element.value().name()) {
                    block = Some(ancestor.id());
                }
            }
            if ancestor.id() == root.id() {
                break;
            }
        }
        if skip {
            continue;
        }
        let is_image = matches!(node.value(), Node::Element(element) if element.name() == "img");
        if is_image && block.is_none() {
            // An image that is not inside a text block still needs a stable
            // block anchor of its own.
            block = Some(node.id());
        }
        if last_block != block {
            append_block(&mut blocks, &mut text)?;
            last_block = block;
        }
        if is_image {
            let marker = format!("SPANSEEKIMAGE{}X", uuid::Uuid::new_v4().simple());
            text.push_str(&marker);
            pending_images.push((marker, replacement));
        } else {
            total += replacement.len();
            if total > 16 * 1024 * 1024 {
                return Err(error("html_text_limit"));
            }
            text.push_str(&replacement);
        }
    }
    append_block(&mut blocks, &mut text)?;
    let mut anchors = Vec::with_capacity(pending_images.len());
    let mut next_image = 0usize;
    for (position, block) in blocks.iter_mut().enumerate() {
        while next_image < pending_images.len()
            && block.text.contains(&pending_images[next_image].0)
        {
            let (marker, label) = &pending_images[next_image];
            let label = if label.trim().is_empty() {
                "Image"
            } else {
                label
            };
            block.text = block
                .text
                .replace(marker, &format!(" {label} "))
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            anchors.push(InlineImageAnchor {
                block_position: position as u32,
            });
            next_image += 1;
        }
        if next_image == pending_images.len() {
            break;
        }
    }
    let length: usize = blocks.iter().map(|block| block.text.chars().count()).sum();
    if length > 16 * 1024 * 1024 {
        return Err(error("html_text_limit"));
    }
    if document
        .select(&selector("input[type=password]"))
        .next()
        .is_some()
        && (!has_main || length < 200)
    {
        return Err(error("webpage_login_required"));
    }
    if length == 0 {
        return Err(error("html_no_content"));
    }
    if !has_main && length < 80 && document.select(&selector("script")).next().is_some() {
        return Err(error("webpage_dynamic_content"));
    }
    Ok((
        ParsedSource {
            source_version_id: request.source_version_id.clone(),
            parser_version: "html5ever-readable-v1".into(),
            blocks,
            page_count: None,
            needs_ocr: false,
            title,
            author,
            toc: Vec::new(),
            cover: None,
            images: Vec::new(),
        },
        anchors,
    ))
}
