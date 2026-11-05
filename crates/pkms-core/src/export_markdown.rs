//! CommonMark/GFM/OFM output. Reject unrepresentable content before publishing.
use crate::{
    DocumentMark, DocumentNode, Error, MarkType, NodeType, Note, NoteDocument, Result,
    TableAlignment,
};
use std::collections::BTreeMap;

fn unsupported(reason: &'static str) -> Error {
    Error::InvalidInput(reason)
}

pub(super) fn validate(document: &NoteDocument) -> Result<()> {
    for node in document.nodes() {
        if node.content.windows(2).any(|pair| {
            pair.iter()
                .all(|inline| inline.kind == NodeType::InlineMath && inline.marks.is_empty())
        }) {
            return Err(unsupported(
                "Markdown cannot preserve adjacent unformatted inline math boundaries; choose HTML or full backup",
            ));
        }
        if matches!(node.kind, NodeType::ListItem | NodeType::TaskItem)
            && node.attrs.block_id.is_some()
        {
            return Err(unsupported(
                "Markdown cannot preserve an item alias separately from its paragraph; choose HTML or full backup",
            ));
        }
        match node.kind {
            NodeType::InlineMath
                if node
                    .attrs
                    .source
                    .as_deref()
                    .unwrap_or_default()
                    .contains(['\r', '\n']) =>
            {
                return Err(unsupported(
                    "Markdown cannot preserve multiline inline math; choose HTML or full backup",
                ));
            }
            NodeType::Comment | NodeType::CommentBlock => {
                let source = node.attrs.source.as_deref().unwrap_or_default();
                if source.contains("%%")
                    || source.ends_with('%')
                    || source.bytes().rev().take_while(|c| *c == b'\\').count() % 2 != 0
                    || !comment_boundaries(source, node.kind == NodeType::CommentBlock)
                {
                    return Err(unsupported(
                        "Markdown cannot preserve this comment source and block boundary; choose HTML or full backup",
                    ));
                }
            }
            NodeType::Frontmatter => {
                if node
                    .attrs
                    .source
                    .as_deref()
                    .unwrap_or_default()
                    .lines()
                    .any(|line| matches!(line.trim_end_matches([' ', '\t', '\r']), "---" | "..."))
                {
                    return Err(unsupported(
                        "Markdown cannot preserve a YAML closing delimiter inside properties; choose HTML or full backup",
                    ));
                }
            }
            NodeType::FootnoteReference | NodeType::FootnoteDefinition => {
                if footnote_label(node.attrs.label.as_deref().unwrap_or_default())
                    .encode_utf16()
                    .count()
                    > 999
                {
                    return Err(unsupported(
                        "Markdown cannot preserve a footnote label beyond its syntax limit; choose HTML or full backup",
                    ));
                }
            }
            NodeType::CodeBlock => {
                if node.attrs.language.as_ref().is_some_and(|value| {
                    value
                        .chars()
                        .any(|c| c.is_whitespace() || matches!(c, '`' | '~'))
                }) {
                    return Err(unsupported(
                        "Markdown cannot preserve this code language name; choose HTML or full backup",
                    ));
                }
            }
            NodeType::WikiLink | NodeType::WikiEmbed => {
                if node
                    .attrs
                    .wiki_target
                    .as_deref()
                    .is_some_and(|target| target.contains('|'))
                {
                    return Err(unsupported(
                        "Markdown cannot preserve a pipe in a wiki target; choose HTML or full backup",
                    ));
                }
                if node.attrs.embed_height.is_some() && node.attrs.embed_width.is_none() {
                    return Err(unsupported(
                        "Markdown cannot preserve embed height without width; choose HTML or full backup",
                    ));
                }
                if node.kind == NodeType::WikiEmbed
                    && node.attrs.embed_width.is_none()
                    && node
                        .attrs
                        .label
                        .as_ref()
                        .is_some_and(|label| size_token(label.rsplit('|').next().unwrap_or(label)))
                {
                    return Err(unsupported(
                        "Markdown would interpret this embed alias as dimensions; choose HTML or full backup",
                    ));
                }
            }
            NodeType::TaskItem => {
                if node
                    .attrs
                    .task_marker
                    .as_ref()
                    .is_some_and(|marker| node.attrs.checked != Some(marker != " "))
                {
                    return Err(unsupported(
                        "Markdown cannot preserve conflicting task marker and checked state; choose HTML or full backup",
                    ));
                }
            }
            NodeType::Table => {
                for (row_index, row) in node.content.iter().enumerate() {
                    for (column, cell) in row.content.iter().enumerate() {
                        let expected = if row_index == 0 {
                            NodeType::TableHeader
                        } else {
                            NodeType::TableCell
                        };
                        if cell.kind != expected
                            || cell.content.len() != 1
                            || cell.content[0].kind != NodeType::Paragraph
                            || cell.attrs.align != node.content[0].content[column].attrs.align
                        {
                            return Err(unsupported(
                                "Markdown tables require one header row, one paragraph per cell and consistent column alignment; choose HTML or full backup",
                            ));
                        }
                        if row.attrs.block_id.is_some()
                            || cell.attrs.block_id.is_some()
                            || cell.content[0].attrs.block_id.is_some()
                        {
                            return Err(unsupported(
                                "Markdown cannot preserve separate row or cell block aliases; choose HTML or full backup",
                            ));
                        }
                        if cell.content[0].content.iter().any(|inline| {
                            inline.kind == NodeType::Comment
                                && inline
                                    .attrs
                                    .source
                                    .as_deref()
                                    .is_some_and(|source| source.contains(['\r', '\n']))
                        }) {
                            return Err(unsupported(
                                "Markdown cannot preserve multiline comment sources inside a table; choose HTML or full backup",
                            ));
                        }
                        if cell.content[0].content.iter().any(|inline| {
                            matches!(inline.kind, NodeType::Comment | NodeType::InlineMath)
                                && inline
                                    .attrs
                                    .source
                                    .as_deref()
                                    .is_some_and(|source| table_text(source) != source)
                        }) {
                            return Err(unsupported(
                                "Markdown cannot preserve literal sources requiring table escaping; choose HTML or full backup",
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
        if node.kind != NodeType::Text && node.marks.iter().any(|mark| mark.kind == MarkType::Code)
        {
            return Err(unsupported(
                "Markdown cannot preserve inline code on a structured reference or formula; choose HTML or full backup",
            ));
        }
    }
    Ok(())
}

// A comment inside a paragraph remains subject to Markdown's block tokenizer.
// Keep plain continuation lines, but reject boundaries that could expose hidden
// text or reinterpret it as a block. A complete standalone fence protects all
// its interior lines. Normalization here is only for checking; output retains
// the exact source, including its original line endings.
fn comment_boundaries(source: &str, standalone: bool) -> bool {
    if !source.contains(['\r', '\n']) {
        return true;
    }
    if standalone && source.starts_with(['\r', '\n']) && source.ends_with(['\r', '\n']) {
        return true;
    }
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized.split('\n').peekable();
    let first = lines.next().unwrap_or_default();
    if standalone && first.trim_matches([' ', '\t']).is_empty() {
        return false;
    }
    while let Some(line) = lines.next() {
        if line.is_empty() && lines.peek().is_none() {
            // The closing %% is on this final line, so it is not a blank line.
            continue;
        }
        if line.is_empty()
            || line.starts_with([
                ' ', '\t', '#', '*', '-', '+', '>', '`', '~', '<', '=', '$', '[', '|', ':',
            ])
        {
            return false;
        }
        let after_digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
        if after_digits.len() != line.len() && after_digits.starts_with(['.', ')']) {
            return false;
        }
    }
    true
}

fn size_token(value: &str) -> bool {
    let mut parts = value.split('x');
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit());
    digits(parts.next().unwrap_or_default())
        && parts.next().is_none_or(digits)
        && parts.next().is_none()
}

pub(super) fn document(note: &Note, assets: &BTreeMap<String, String>) -> String {
    render(note, assets, true)
}

/// Renders the note without its leading properties block. Vault export writes
/// its own Obsidian frontmatter and must not emit a second YAML header.
pub(super) fn document_without_properties(
    note: &Note,
    assets: &BTreeMap<String, String>,
) -> String {
    render(note, assets, false)
}

fn render(note: &Note, assets: &BTreeMap<String, String>, include_properties: bool) -> String {
    let mut nodes = note.document.content.iter().peekable();
    let properties = match nodes.peek() {
        Some(node) if node.kind == NodeType::Frontmatter => {
            let node = nodes.next().expect("peeked properties");
            if include_properties {
                format!("{}\n\n", block(node, assets))
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    format!(
        "{properties}# {}\n\n{}\n",
        text(&note.title),
        nodes
            .map(|node| block(node, assets))
            .collect::<Vec<_>>()
            .join("\n\n")
    )
}

fn text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            _ => {
                if ch.is_ascii_punctuation() {
                    output.push('\\');
                }
                output.push(ch);
            }
        }
    }
    output
}

fn html_attribute(value: &str) -> String {
    super::escape(value)
        .replace('|', "&#124;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
}

fn marks(mut value: String, marks: &[DocumentMark]) -> String {
    for mark in marks {
        value = match mark.kind {
            MarkType::Bold => format!("<strong>{value}</strong>"),
            MarkType::Italic => format!("<em>{value}</em>"),
            MarkType::Strike => format!("<s>{value}</s>"),
            MarkType::Underline => format!("<u>{value}</u>"),
            MarkType::Superscript => format!("<sup>{value}</sup>"),
            MarkType::Subscript => format!("<sub>{value}</sub>"),
            MarkType::Highlight => match &mark.attrs.color {
                Some(color) => format!(
                    "<mark style=\"background-color:{}\">{value}</mark>",
                    super::escape(color)
                ),
                None => format!("<mark>{value}</mark>"),
            },
            MarkType::TextStyle => format!(
                "<span style=\"color:{}\">{value}</span>",
                super::escape(mark.attrs.color.as_deref().unwrap_or_default())
            ),
            MarkType::Code => format!("<code>{value}</code>"),
            MarkType::Link => {
                let attributes = [
                    ("title", mark.attrs.title.as_deref()),
                    ("target", mark.attrs.target.as_deref()),
                    ("rel", mark.attrs.rel.as_deref()),
                ]
                .into_iter()
                .filter_map(|(name, value)| {
                    value.map(|value| format!(" {name}=\"{}\"", html_attribute(value)))
                })
                .collect::<String>();
                format!(
                    "<a href=\"{}\"{attributes}>{value}</a>",
                    html_attribute(mark.attrs.href.as_deref().unwrap_or_default())
                )
            }
        };
    }
    value
}

fn run(value: &str, ch: char) -> usize {
    value.split(|c| c != ch).map(str::len).max().unwrap_or(0)
}

fn span(source: &str, delimiter: char) -> String {
    let fence = delimiter.to_string().repeat(run(source, delimiter) + 1);
    let padding = if source.starts_with(delimiter)
        || source.ends_with(delimiter)
        || (source.starts_with(' ') && source.ends_with(' ') && !source.trim().is_empty())
    {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{source}{padding}{fence}")
}

fn inline(node: &DocumentNode) -> String {
    inline_in_context(node, false)
}

fn inline_in_context(node: &DocumentNode, table: bool) -> String {
    if node.kind == NodeType::Text && node.marks.len() == 1 {
        let source = node.text.as_deref().unwrap_or_default();
        match node.marks[0].kind {
            MarkType::Code => {
                return if source.contains(['\n', '\r']) || (table && source.contains('|')) {
                    format!(
                        "<code>{}</code>",
                        source
                            .chars()
                            .map(|c| format!("&#{};", u32::from(c)))
                            .collect::<String>()
                    )
                } else {
                    span(source, '`')
                };
            }
            MarkType::Highlight
                if node.marks[0].attrs.color.is_none()
                    && !source.starts_with(char::is_whitespace)
                    && !source.ends_with(char::is_whitespace) =>
            {
                return format!("=={}==", text(source));
            }
            MarkType::Bold | MarkType::Italic | MarkType::Strike
                if !source.starts_with(char::is_whitespace)
                    && !source.ends_with(char::is_whitespace) =>
            {
                let delimiter = match node.marks[0].kind {
                    MarkType::Bold => "**",
                    MarkType::Italic => "*",
                    _ => "~~",
                };
                return format!("{delimiter}{}{delimiter}", text(source));
            }
            _ => {}
        }
    }
    let value = match node.kind {
        NodeType::Text => text(node.text.as_deref().unwrap_or_default()),
        NodeType::HardBreak => "<br>".into(),
        NodeType::InlineMath => span(node.attrs.source.as_deref().unwrap_or_default(), '$'),
        NodeType::WikiLink => wiki(node, false),
        NodeType::FootnoteReference => format!(
            "[^{}]",
            footnote_label(node.attrs.label.as_deref().unwrap_or_default())
        ),
        NodeType::Comment => format!("%%{}%%", node.attrs.source.as_deref().unwrap_or_default()),
        NodeType::NoteReference => format!(
            "[{}]({})",
            text(node.attrs.label.as_deref().unwrap_or("笔记引用")),
            super::reference_url(node)
        ),
        _ => unreachable!("validated paragraph contains an unsupported block"),
    };
    marks(value, &node.marks)
}

fn prefix_lines(value: &str, prefix: &str) -> String {
    value
        .split('\n')
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn block(node: &DocumentNode, assets: &BTreeMap<String, String>) -> String {
    let inline_content = || node.content.iter().map(inline).collect::<String>();
    let blocks = || {
        node.content
            .iter()
            .map(|child| block(child, assets))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let value = match node.kind {
        NodeType::Paragraph => inline_content(),
        NodeType::Heading => format!(
            "{} {}",
            "#".repeat(usize::from(node.attrs.level.unwrap_or(1))),
            inline_content()
        ),
        NodeType::HorizontalRule => "---".into(),
        NodeType::CodeBlock => {
            let source = node.plain_text();
            let fence = "`".repeat(3.max(run(&source, '`') + 1));
            format!(
                "{fence}{}\n{source}\n{fence}",
                node.attrs.language.as_deref().unwrap_or_default()
            )
        }
        NodeType::BlockMath => {
            let source = node.attrs.source.as_deref().unwrap_or_default();
            let fence = "$".repeat(2.max(run(source, '$') + 1));
            format!("{fence}\n{source}\n{fence}")
        }
        NodeType::Frontmatter => format!(
            "---\n{}\n---",
            node.attrs.source.as_deref().unwrap_or_default()
        ),
        NodeType::CommentBlock => {
            format!("%%{}%%", node.attrs.source.as_deref().unwrap_or_default())
        }
        NodeType::Blockquote => prefix_lines(&blocks(), "> "),
        NodeType::Callout => {
            let fold = node
                .attrs
                .collapsed
                .map(|closed| if closed { "-" } else { "+" })
                .unwrap_or("");
            let title = node
                .attrs
                .title
                .as_ref()
                .map(|title| format!(" {}", text(title)))
                .unwrap_or_default();
            let body = if node.content.is_empty() {
                String::new()
            } else {
                format!("\n{}", blocks())
            };
            prefix_lines(
                &format!(
                    "[!{}]{fold}{title}{body}",
                    node.attrs.callout_type.as_deref().unwrap_or("note")
                ),
                "> ",
            )
        }
        NodeType::BulletList | NodeType::OrderedList | NodeType::TaskList => node
            .content
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if node.kind == NodeType::OrderedList {
                    format!(
                        "{}. ",
                        u64::from(node.attrs.start.unwrap_or(1)) + index as u64
                    )
                } else {
                    "- ".into()
                };
                let marker = if item.kind == NodeType::TaskItem {
                    format!(
                        "[{}] ",
                        item.attrs.task_marker.as_deref().unwrap_or(
                            if item.attrs.checked.unwrap_or(false) {
                                "x"
                            } else {
                                " "
                            }
                        )
                    )
                } else {
                    String::new()
                };
                let contents = item
                    .content
                    .iter()
                    .map(|child| block(child, assets))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let mut lines = contents.split('\n');
                let first = lines.next().unwrap_or_default();
                let rest = lines
                    .map(|line| format!("\n{}{line}", " ".repeat(prefix.len())))
                    .collect::<String>();
                format!("{prefix}{marker}{first}{rest}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        NodeType::FootnoteDefinition => {
            let contents = blocks();
            let mut lines = contents.split('\n');
            let first = lines.next().unwrap_or_default();
            let rest = lines
                .map(|line| format!("\n    {line}"))
                .collect::<String>();
            format!(
                "[^{}]: {first}{rest}",
                footnote_label(node.attrs.label.as_deref().unwrap_or_default())
            )
        }
        NodeType::Image => {
            let path = assets
                .get(node.attrs.blob_hash.as_deref().unwrap_or_default())
                .map(String::as_str)
                .unwrap_or_default();
            let title = node
                .attrs
                .title
                .as_ref()
                .map(|title| {
                    format!(
                        " \"{}\"",
                        title
                            .replace('\\', "\\\\")
                            .replace('"', "\\\"")
                            .replace('\n', "&#10;")
                            .replace('\r', "&#13;")
                    )
                })
                .unwrap_or_default();
            format!(
                "![{}]({path}{title})",
                text(node.attrs.alt.as_deref().unwrap_or_default())
            )
        }
        NodeType::WikiEmbed => wiki(node, true),
        NodeType::Table => {
            let mut rows = node
                .content
                .iter()
                .map(|row| {
                    let cells = row
                        .content
                        .iter()
                        .map(|cell| {
                            table_text(
                                &cell.content[0]
                                    .content
                                    .iter()
                                    .map(|node| inline_in_context(node, true))
                                    .collect::<String>(),
                            )
                        })
                        .collect::<Vec<_>>();
                    format!("| {} |", cells.join(" | "))
                })
                .collect::<Vec<_>>();
            let alignment = node.content[0]
                .content
                .iter()
                .map(|cell| match cell.attrs.align {
                    Some(TableAlignment::Left) => ":---",
                    Some(TableAlignment::Center) => ":---:",
                    Some(TableAlignment::Right) => "---:",
                    None => "---",
                })
                .collect::<Vec<_>>();
            rows.insert(1, format!("| {} |", alignment.join(" | ")));
            rows.join("\n")
        }
        _ => unreachable!("validated document contains an unsupported block"),
    };
    match &node.attrs.block_id {
        Some(alias) if matches!(node.kind, NodeType::Paragraph | NodeType::Heading) => {
            format!("{value} ^{alias}")
        }
        Some(alias) => format!("{value}\n\n^{alias}"),
        None => value,
    }
}

fn table_text(value: &str) -> String {
    let mut result = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if ch == '|' && !escaped {
            result.push('\\');
        }
        if ch == '\n' {
            result.push_str("<br>");
        } else {
            result.push(ch);
        }
        escaped = ch == '\\' && !escaped;
    }
    result
}

fn footnote_label(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if matches!(ch, '&' | '\\' | '[' | ']' | '|') || ch.is_whitespace() {
            output.push_str(&format!("&#{};", u32::from(ch)));
        } else {
            output.push(ch);
        }
    }
    output
}

fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn wiki(node: &DocumentNode, embed: bool) -> String {
    let target = label(node.attrs.wiki_target.as_deref().unwrap_or_default());
    let alias = node
        .attrs
        .label
        .as_ref()
        .map(|value| format!("|{}", label(value)))
        .unwrap_or_default();
    let size = node
        .attrs
        .embed_width
        .map(|width| match node.attrs.embed_height {
            Some(height) => format!("|{width}x{height}"),
            None => format!("|{width}"),
        })
        .unwrap_or_default();
    format!("{}[[{target}{alias}{size}]]", if embed { "!" } else { "" })
}
