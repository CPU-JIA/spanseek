//! Escaped standalone HTML, with locally bundled optional math/diagram rendering.
use super::escape;
use crate::{
    BibliographyEntry, BibliographyReport, DocumentMark, DocumentNode, InTextCitation, MarkType,
    NodeType, Note,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

fn footnote_key(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

struct Html<'a> {
    note: &'a Note,
    assets: &'a BTreeMap<String, String>,
    inline_assets: Option<&'a BTreeMap<String, String>>,
    citations: Option<&'a CitationHtml<'a>>,
    numbers: BTreeMap<String, usize>,
    definitions: BTreeSet<String>,
    block_ids: BTreeSet<String>,
}

struct CitationHtml<'a> {
    in_text: &'a [InTextCitation],
    next: Cell<usize>,
    entries: &'a [BibliographyEntry],
    styled: bool,
}

impl<'a> CitationHtml<'a> {
    fn rendered(&self, marker: &str, keys: &[String]) -> Option<&'a str> {
        let start = self.next.get();
        let offset = self
            .in_text
            .get(start..)?
            .iter()
            .position(|citation| citation.marker == marker && citation.keys == keys)?;
        let index = start + offset;
        self.next.set(index + 1);
        Some(self.in_text[index].rendered.as_str())
    }
}

pub(super) fn needs_renderer(note: &Note) -> bool {
    note.document.nodes().iter().any(|node| {
        matches!(node.kind, NodeType::InlineMath | NodeType::BlockMath)
            || (node.kind == NodeType::CodeBlock
                && node.attrs.language.as_deref() == Some("mermaid"))
    })
}

pub(super) fn document(
    note: &Note,
    assets: &BTreeMap<String, String>,
    inline_assets: Option<&BTreeMap<String, String>>,
    renderer: bool,
    citations: Option<&BibliographyReport>,
) -> String {
    let mut numbers = BTreeMap::new();
    let definitions = note
        .document
        .nodes()
        .iter()
        .filter(|node| node.kind == NodeType::FootnoteDefinition)
        .filter_map(|node| node.attrs.label.as_deref())
        .map(footnote_key)
        .collect();
    for kind in [NodeType::FootnoteReference, NodeType::FootnoteDefinition] {
        for node in note
            .document
            .nodes()
            .iter()
            .filter(|node| node.kind == kind)
        {
            let key = footnote_key(node.attrs.label.as_deref().unwrap_or_default());
            let next = numbers.len() + 1;
            numbers.entry(key).or_insert(next);
        }
    }
    let citation_context = citations.map(|report| CitationHtml {
        in_text: &report.in_text,
        next: Cell::new(0),
        entries: &report.entries,
        styled: report.style_id != "plain",
    });
    let context = Html {
        note,
        assets,
        inline_assets,
        citations: citation_context.as_ref(),
        numbers,
        definitions,
        block_ids: note
            .document
            .nodes()
            .iter()
            .filter(|node| !node.attrs.id.is_empty())
            .map(|node| node.attrs.id.clone())
            .collect(),
    };
    let resources = if renderer {
        "<link rel=\"stylesheet\" href=\"assets/rich-export/style.css\"><script defer src=\"assets/rich-export/renderer.js\"></script>"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' data:; script-src 'self'; font-src 'self' data:; style-src 'self' 'unsafe-inline'\"><title>{}</title><style>body{{max-width:900px;margin:32px auto;padding:0 20px;font:17px/1.65 system-ui,sans-serif;color:#242424}}.pkms-tasks{{list-style:none;padding-left:1.75em}}.pkms-tasks li>input[type=checkbox]{{margin-right:.45em}}p,h1,h2,h3,h4,h5,h6{{overflow-wrap:anywhere}}img,svg{{max-width:100%}}mark{{color:inherit}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#f5f5f5;padding:12px;border-radius:6px}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccc;padding:6px 10px}}aside,details[data-callout]{{border-left:4px solid #6896c7;background:#f2f6fa;padding:10px 16px;margin:16px 0}}figure{{margin:16px 0}}.pkms-wiki-unresolved,.pkms-footnote-missing,.pkms-citation-missing{{color:#a34c18}}.pkms-citation{{white-space:nowrap}}.pkms-references{{margin-top:48px;border-top:1px solid #d8d8d8;padding-top:20px}}.pkms-references li{{margin:8px 0;overflow-wrap:anywhere}}[hidden]{{display:none}}@page{{size:A4;margin:18mm}}@media print{{body{{max-width:none;margin:0;padding:0}}a{{color:inherit;text-decoration:none}}.pkms-references{{break-inside:avoid}}}}</style>{resources}</head><body><h1 id=\"pkms-note-title\">{}</h1>{}{}</body></html>\n",
        escape(&note.title),
        escape(&note.title),
        context.nodes(&note.document.content),
        context.references()
    )
}

impl Html<'_> {
    fn nodes(&self, nodes: &[DocumentNode]) -> String {
        nodes.iter().map(|node| self.node(node)).collect()
    }

    fn text(&self, value: &str) -> String {
        let groups = crate::citation::citation_group_ranges(value);
        if groups.is_empty() {
            return escape(value);
        }
        let mut html = String::new();
        let mut cursor = 0usize;
        for (range, keys) in groups {
            html.push_str(&escape(&value[cursor..range.start]));
            let raw = &value[range.clone()];
            let marker = &value[range.clone()];
            if let Some(rendered) = self
                .citations
                .and_then(|citations| citations.rendered(marker, &keys))
            {
                let target = keys
                    .first()
                    .map(|key| format!("#pkms-ref-{}", escape(key)))
                    .unwrap_or_default();
                html.push_str(&format!(
                    "<span class=\"pkms-citation\" data-citation-keys=\"{}\"><a href=\"{target}\">{}</a></span>",
                    escape(&keys.join(";")),
                    escape(rendered)
                ));
            } else if self.citations.is_some_and(|citations| citations.styled) {
                html.push_str(&format!(
                    "<span class=\"pkms-citation pkms-citation-missing\" title=\"引用目标未解析\">{}</span>",
                    escape(raw)
                ));
            } else {
                html.push_str(&escape(raw));
            }
            cursor = range.end;
        }
        html.push_str(&escape(&value[cursor..]));
        html
    }

    fn references(&self) -> String {
        let Some(citations) = self.citations else {
            return String::new();
        };
        if citations.entries.is_empty() {
            return String::new();
        }
        let mut items = String::new();
        for entry in citations.entries {
            items.push_str(&format!(
                "<li id=\"pkms-ref-{}\" data-citation-key=\"{}\">{}</li>",
                escape(&entry.key),
                escape(&entry.key),
                escape(&entry.formatted)
            ));
        }
        format!(
            "<section class=\"pkms-references\" role=\"doc-bibliography\"><h2>参考文献 / References</h2><ol>{items}</ol></section>"
        )
    }

    fn local_target(&self, node: &DocumentNode) -> bool {
        node.attrs.target_id.as_deref() == Some(self.note.id.as_str())
            && node
                .attrs
                .target_revision_id
                .as_deref()
                .is_none_or(|id| id == self.note.revision_id)
    }

    fn wiki_unresolved(&self, node: &DocumentNode) -> Option<(&'static str, &'static str)> {
        if node.attrs.target_id.is_none() {
            return Some(("target-unresolved", "引用目标尚未解析"));
        }
        if node.attrs.target_block_id.is_none()
            && crate::wiki::has_fragment(node.attrs.wiki_target.as_deref().unwrap_or_default())
        {
            return Some(("anchor-unresolved", "引用锚点尚未定位"));
        }
        if self.local_target(node)
            && node
                .attrs
                .target_block_id
                .as_ref()
                .is_some_and(|block| !self.block_ids.contains(block))
        {
            return Some(("anchor-missing", "引用锚点已失效"));
        }
        None
    }

    fn reference(&self, node: &DocumentNode) -> String {
        if self.local_target(node) {
            format!(
                "#{}",
                node.attrs
                    .target_block_id
                    .as_deref()
                    .unwrap_or("pkms-note-title")
            )
        } else {
            super::reference_url(node)
        }
    }

    fn node(&self, node: &DocumentNode) -> String {
        let body = self.nodes(&node.content);
        let id = if node.attrs.id.is_empty() {
            String::new()
        } else {
            format!(" id=\"{}\"", escape(&node.attrs.id))
        };
        let tag = |name: &str| format!("<{name}{id}>{body}</{name}>");
        let value = match node.kind {
            NodeType::Text => self.text(node.text.as_deref().unwrap_or_default()),
            NodeType::Paragraph => tag("p"),
            NodeType::Heading => tag(&format!("h{}", node.attrs.level.unwrap_or(1))),
            NodeType::HardBreak => "<br>".into(),
            NodeType::BulletList => tag("ul"),
            NodeType::TaskList => format!("<ul{id} class=\"pkms-tasks\">{body}</ul>"),
            NodeType::OrderedList => format!(
                "<ol{id} start=\"{}\">{body}</ol>",
                node.attrs.start.unwrap_or(1)
            ),
            NodeType::ListItem => tag("li"),
            NodeType::TaskItem => {
                let marker = node
                    .attrs
                    .task_marker
                    .as_ref()
                    .map(|marker| format!(" data-task-marker=\"{}\"", escape(marker)))
                    .unwrap_or_default();
                let custom = node
                    .attrs
                    .task_marker
                    .as_ref()
                    .filter(|marker| !matches!(marker.as_str(), " " | "x" | "X"))
                    .map(|marker| {
                        format!(
                            "<span class=\"pkms-task-marker\">[{}]</span> ",
                            escape(marker)
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "<li{id}{marker}><input type=\"checkbox\" disabled{}>{custom}{body}</li>",
                    if node.attrs.checked.unwrap_or(false) {
                        " checked"
                    } else {
                        ""
                    }
                )
            }
            NodeType::Blockquote => tag("blockquote"),
            NodeType::CodeBlock if node.attrs.language.as_deref() == Some("mermaid") => format!(
                "<pre{id} data-pkms-mermaid>{}</pre>",
                escape(&node.plain_text())
            ),
            NodeType::CodeBlock => {
                let language = node
                    .attrs
                    .language
                    .as_ref()
                    .map(|language| format!(" data-language=\"{}\"", escape(language)))
                    .unwrap_or_default();
                format!(
                    "<pre{id}><code{language}>{}</code></pre>",
                    escape(&node.plain_text())
                )
            }
            NodeType::HorizontalRule => format!("<hr{id}>"),
            NodeType::Image => format!(
                "<img{id} src=\"{}\" alt=\"{}\" title=\"{}\">",
                escape(
                    self.inline_assets
                        .and_then(|assets| {
                            assets.get(node.attrs.blob_hash.as_deref().unwrap_or_default())
                        })
                        .or_else(|| {
                            self.assets
                                .get(node.attrs.blob_hash.as_deref().unwrap_or_default())
                        })
                        .map(String::as_str)
                        .unwrap_or_default()
                ),
                escape(node.attrs.alt.as_deref().unwrap_or_default()),
                escape(node.attrs.title.as_deref().unwrap_or_default())
            ),
            NodeType::NoteReference => format!(
                "<a href=\"{}\">{}</a>",
                escape(&self.reference(node)),
                escape(node.attrs.label.as_deref().unwrap_or("笔记引用"))
            ),
            NodeType::Table => tag("table"),
            NodeType::TableRow => tag("tr"),
            NodeType::TableCell | NodeType::TableHeader => {
                let name = if node.kind == NodeType::TableHeader {
                    "th"
                } else {
                    "td"
                };
                let alignment = node
                    .attrs
                    .align
                    .map(|align| format!(" style=\"text-align:{}\"", align.as_str()))
                    .unwrap_or_default();
                format!(
                    "<{name}{id}{alignment} colspan=\"{}\" rowspan=\"{}\">{body}</{name}>",
                    node.attrs.colspan.unwrap_or(1),
                    node.attrs.rowspan.unwrap_or(1)
                )
            }
            NodeType::Callout => {
                let kind = escape(node.attrs.callout_type.as_deref().unwrap_or("note"));
                let title = escape(
                    node.attrs
                        .title
                        .as_deref()
                        .unwrap_or(node.attrs.callout_type.as_deref().unwrap_or("note")),
                );
                if let Some(collapsed) = node.attrs.collapsed {
                    format!(
                        "<details{id} data-callout=\"{kind}\"{}><summary>{title}</summary>{body}</details>",
                        if collapsed { "" } else { " open" }
                    )
                } else {
                    format!(
                        "<aside{id} data-callout=\"{kind}\"><p><strong>{title}</strong></p>{body}</aside>"
                    )
                }
            }
            NodeType::InlineMath => format!(
                "<span data-pkms-math=\"inline\">{}</span>",
                escape(node.attrs.source.as_deref().unwrap_or_default())
            ),
            NodeType::BlockMath => format!(
                "<div{id} data-pkms-math=\"block\">{}</div>",
                escape(node.attrs.source.as_deref().unwrap_or_default())
            ),
            NodeType::WikiLink | NodeType::WikiEmbed => {
                let target = escape(node.attrs.wiki_target.as_deref().unwrap_or_default());
                let label = escape(
                    node.attrs
                        .label
                        .as_deref()
                        .or(node.attrs.wiki_target.as_deref())
                        .unwrap_or_default(),
                );
                let content = if let Some((state, reason)) = self.wiki_unresolved(node) {
                    format!(
                        "<span class=\"pkms-wiki-unresolved\" data-unresolved=\"true\" data-wiki-state=\"{state}\" title=\"{reason}\">{label} <small>（{reason}）</small></span>"
                    )
                } else {
                    format!("<a href=\"{}\">{label}</a>", escape(&self.reference(node)))
                };
                if node.kind == NodeType::WikiEmbed {
                    let width = node
                        .attrs
                        .embed_width
                        .map(|value| format!(" data-width=\"{value}\""))
                        .unwrap_or_default();
                    let height = node
                        .attrs
                        .embed_height
                        .map(|value| format!(" data-height=\"{value}\""))
                        .unwrap_or_default();
                    format!(
                        "<figure{id} data-wiki-target=\"{target}\"{width}{height}><figcaption>{content}</figcaption></figure>"
                    )
                } else {
                    format!("<span data-wiki-target=\"{target}\">{content}</span>")
                }
            }
            NodeType::FootnoteReference => {
                let label = node.attrs.label.as_deref().unwrap_or_default();
                let key = footnote_key(label);
                let number = self.numbers.get(&key).copied().unwrap_or(1);
                if self.definitions.contains(&key) {
                    format!(
                        "<sup><a role=\"doc-noteref\" href=\"#pkms-footnote-{number}\">{number}</a></sup>"
                    )
                } else {
                    format!(
                        "<sup class=\"pkms-footnote-missing\" title=\"脚注未定义：{}\">[{number}?]</sup>",
                        escape(label)
                    )
                }
            }
            NodeType::FootnoteDefinition => {
                let label = node.attrs.label.as_deref().unwrap_or_default();
                let number = self.numbers.get(&footnote_key(label)).copied().unwrap_or(1);
                format!(
                    "<section{id} role=\"doc-footnote\" data-footnote-label=\"{}\"><span id=\"pkms-footnote-{number}\">{number}.</span>{body}</section>",
                    escape(label)
                )
            }
            NodeType::Comment | NodeType::CommentBlock => format!(
                "<span{id} hidden data-pkms-comment=\"true\">{}</span>",
                escape(node.attrs.source.as_deref().unwrap_or_default())
            ),
            NodeType::Frontmatter => format!(
                "<details{id} class=\"pkms-properties\"><summary>属性 / Properties</summary><pre>{}</pre></details>",
                escape(node.attrs.source.as_deref().unwrap_or_default())
            ),
        };
        let value = if node.is_inline() {
            marked(
                value,
                &node.marks,
                matches!(
                    node.kind,
                    NodeType::NoteReference | NodeType::WikiLink | NodeType::FootnoteReference
                ),
            )
        } else {
            value
        };
        match &node.attrs.block_id {
            Some(alias) => format!(
                "<span id=\"pkms-block-alias-{}\" data-block-alias=\"{}\"></span>{value}",
                escape(alias),
                escape(alias)
            ),
            None => value,
        }
    }
}

fn marked(mut value: String, marks: &[DocumentMark], has_link: bool) -> String {
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
                    escape(color)
                ),
                None => format!("<mark>{value}</mark>"),
            },
            MarkType::TextStyle => format!(
                "<span style=\"color:{}\">{value}</span>",
                escape(mark.attrs.color.as_deref().unwrap_or_default())
            ),
            MarkType::Code => format!("<code>{value}</code>"),
            MarkType::Link => {
                let href = escape(mark.attrs.href.as_deref().unwrap_or_default());
                let title = escape(mark.attrs.title.as_deref().unwrap_or_default());
                // HTML cannot nest anchors. A separately visible link preserves
                // both the structured reference and its outer link mark.
                if has_link {
                    format!(
                        "<span>{value}<a href=\"{href}\" title=\"{title}\" rel=\"noreferrer noopener\" aria-label=\"附加链接\">↗</a></span>"
                    )
                } else {
                    format!(
                        "<a href=\"{href}\" title=\"{title}\" rel=\"noreferrer noopener\">{value}</a>"
                    )
                }
            }
        };
    }
    value
}
