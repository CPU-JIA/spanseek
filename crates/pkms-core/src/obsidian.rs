//! Bounded Obsidian/Markdown vault interoperability.
//!
//! Export renders a note through the existing rich-Markdown renderer, rewrites
//! internal note references to Obsidian `[[Title]]` syntax and emits exactly
//! one YAML frontmatter block carrying the title, the spanseek identity, the
//! note tags and any foreign keys the note already held.
//!
//! Import parses Markdown into spanseek's document model (headings, lists,
//! quotes, code, tables, math, images and wiki links) instead of flattening it
//! into escaped plain text, so an export/import round trip keeps the body
//! readable and re-exportable. Anything the importer cannot represent is
//! listed in the report instead of being dropped silently.
use crate::{
    AssignTag, CreateNote, DOCUMENT_SCHEMA, DocumentMark, DocumentNode, DocumentType, Error,
    Library, MarkAttributes, MarkType, MetadataTarget, NodeAttributes, NodeType, NoteDocument,
    Result, TableAlignment, TagSelector,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
use ts_rs::TS;

const MAX_VAULT_FILES: usize = 5_000;
const MAX_VAULT_DEPTH: usize = 32;
const MAX_MARKDOWN_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TITLE_CHARS: usize = 120;
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VaultReport {
    pub notes: u32,
    pub markdown_files: u32,
    pub attachments: u32,
    pub links: u32,
    pub tags: u32,
    pub skipped_existing: u32,
    pub unresolved_links: u32,
    pub losses: Vec<String>,
    pub normalized: Vec<String>,
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_owned(), 0usize)];
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_VAULT_DEPTH {
            return Err(Error::InvalidInput("vault_depth_limit"));
        }
        let mut entries = fs::read_dir(&directory)
            .map_err(|_| Error::InvalidInput("vault_unreadable"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::InvalidInput("vault_unreadable"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|_| Error::InvalidInput("vault_unreadable"))?;
            if metadata.is_dir() {
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push((path, depth + 1));
                }
            } else if metadata.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            {
                files.push(path);
                if files.len() > MAX_VAULT_FILES {
                    return Err(Error::InvalidInput("vault_file_limit"));
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Splits a leading YAML frontmatter block from the Markdown body. The
/// delimiters must be on their own line and the block must be closed, matching
/// how Obsidian behaves: an unterminated block is plain body text.
fn split_frontmatter(text: &str) -> (&str, Option<&str>) {
    let rest = match text.strip_prefix("---\n") {
        Some(rest) => rest,
        None => match text.strip_prefix("---\r\n") {
            Some(rest) => rest,
            None => return (text, None),
        },
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return (&rest[offset + line.len()..], Some(&rest[..offset]));
        }
        offset += line.len();
    }
    (text, None)
}

/// Groups YAML lines by top-level key so list items and nested values stay
/// attached to the key that owns them.
fn yaml_groups(yaml: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let list_item = trimmed.trim_start().starts_with("- ");
        if indented || list_item {
            if let Some(last) = groups.last_mut() {
                last.1.push(trimmed.to_owned());
            }
            continue;
        }
        let Some((key, _)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.chars().count() > 80 {
            continue;
        }
        groups.push((key.to_owned(), vec![trimmed.to_owned()]));
    }
    groups
}

fn yaml_value(group: &(String, Vec<String>)) -> &str {
    group
        .1
        .first()
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim())
        .unwrap_or_default()
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches(['"', '\''])
}

fn yaml_list(value: &str) -> Vec<String> {
    let value = value.trim().trim_start_matches('[').trim_end_matches(']');
    let separator = if value.contains(',') { ',' } else { ' ' };
    value
        .split(separator)
        .map(|item| item.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|item| !item.is_empty() && item.chars().count() <= 100)
        .collect()
}

fn tags_from_groups(groups: &[(String, Vec<String>)]) -> Vec<String> {
    let Some(group) = groups.iter().find(|(key, _)| key == "tags") else {
        return Vec::new();
    };
    let mut tags = yaml_list(yaml_value(group));
    for line in group.1.iter().skip(1) {
        let item = line
            .trim()
            .trim_start_matches('-')
            .trim()
            .trim_matches(['"', '\'']);
        if !item.is_empty() && item.chars().count() <= 100 {
            tags.push(item.to_owned());
        }
    }
    tags
}

fn title_from_groups(groups: &[(String, Vec<String>)], body: &str, path: &Path) -> String {
    if let Some(group) = groups.iter().find(|(key, _)| key == "title") {
        let title = unquote(yaml_value(group));
        if !title.is_empty() {
            return title.chars().take(1024).collect();
        }
    }
    for line in body.lines() {
        let line = line.trim();
        if let Some(title) = line
            .strip_prefix("# ")
            .filter(|title| !title.trim().is_empty())
        {
            return title.trim().chars().take(1024).collect();
        }
        if !line.is_empty() {
            break;
        }
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("未命名")
        .chars()
        .take(1024)
        .collect()
}

fn render_groups(groups: &[(String, Vec<String>)]) -> String {
    groups
        .iter()
        .flat_map(|(_, lines)| lines.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keys that spanseek owns and re-emits itself; everything else is carried
/// through so unknown YAML never disappears on a round trip.
fn preserved_groups(source: &str) -> Vec<(String, Vec<String>)> {
    yaml_groups(source)
        .into_iter()
        .filter(|(key, _)| !matches!(key.as_str(), "title" | "spanseek_id" | "tags"))
        .collect()
}

fn frontmatter_block(
    title: &str,
    id: &str,
    tags: &[String],
    preserved: &[(String, Vec<String>)],
) -> String {
    let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let tag_list = tags
        .iter()
        .map(|tag| format!("\"{}\"", escape(tag)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![
        "---".to_owned(),
        format!("title: \"{}\"", escape(title)),
        format!("spanseek_id: \"{}\"", escape(id)),
        format!("tags: [{tag_list}]"),
    ];
    for (_, group) in preserved {
        lines.extend(group.iter().cloned());
    }
    lines.extend(["---".to_owned(), String::new()]);
    lines.join("\n")
}
fn sanitize_filename(title: &str) -> String {
    let mut name = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take(MAX_TITLE_CHARS)
        .collect::<String>();
    name = name.trim().trim_end_matches('.').trim().to_owned();
    if name.is_empty() {
        "未命名".into()
    } else {
        name
    }
}

/// Every regular file in the vault indexed by lower-case file name so both
/// `![[image.png]]` and `![](../attachments/image.png)` resolve.
fn attachment_index(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut index = BTreeMap::new();
    let mut stack = vec![(root.to_owned(), 0usize)];
    let mut visited = 0usize;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_VAULT_DEPTH {
            continue;
        }
        let entries =
            fs::read_dir(&directory).map_err(|_| Error::InvalidInput("vault_unreadable"))?;
        for entry in entries {
            let entry = entry.map_err(|_| Error::InvalidInput("vault_unreadable"))?;
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|_| Error::InvalidInput("vault_unreadable"))?;
            if metadata.is_dir() {
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push((path, depth + 1));
                }
            } else if metadata.is_file() {
                visited += 1;
                if visited > MAX_VAULT_FILES {
                    return Err(Error::InvalidInput("vault_file_limit"));
                }
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    index.entry(name.to_lowercase()).or_insert(path);
                }
            }
        }
    }
    Ok(index)
}

fn wikilinks(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        if start > 0 && rest.as_bytes()[start - 1] == b'!' {
            rest = &rest[start + 2..];
            continue;
        }
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let target = after[..end]
            .split(['|', '#'])
            .next()
            .unwrap_or_default()
            .trim();
        if !target.is_empty() && !target.contains('\n') {
            links.push(target.to_owned());
        }
        rest = &after[end + 2..];
    }
    links
}

fn rewrite_links(markdown: &str, titles: &BTreeMap<String, String>) -> (String, u32) {
    let mut output = String::with_capacity(markdown.len());
    let mut rest = markdown;
    let mut rewritten = 0;
    while let Some(position) = rest.find("](pkms://object/") {
        output.push_str(&rest[..position]);
        let after = &rest[position + "](pkms://object/".len()..];
        let Some(end) = after.find(')') else {
            output.push_str(rest);
            return (output, rewritten);
        };
        let id = after[..end].split(['?', '#']).next().unwrap_or_default();
        if let Some(title) = titles.get(id) {
            output.push_str(&format!("[[{}]]", title.replace('|', "\\|")));
            rewritten += 1;
        } else {
            output.push_str(&rest[position..position + "](pkms://object/".len() + end + 1]);
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    (output, rewritten)
}

/// Tracks the node that a Markdown event is currently filling.
enum Open {
    Node,
    Mark,
    Image,
    Ignore,
}

struct ImageCapture {
    dest: String,
    title: String,
    alt: String,
}

const TOKEN_MARKER: char = '\u{E001}';

/// pulldown-cmark splits `[[link]]` into several text events, so wiki links are
/// replaced by plain-text markers before parsing and restored afterwards.
fn protect_wikilinks(text: &str) -> (String, Vec<(bool, String)>) {
    let mut tokens: Vec<(bool, String)> = Vec::new();
    let mut output = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let fence_line = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if fence_line {
            let mark = trimmed[..3].to_owned();
            match &fence {
                Some(open) if *open == mark => fence = None,
                None => fence = Some(mark),
                _ => {}
            }
            output.push_str(line);
            continue;
        }
        if fence.is_some() {
            output.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find("[[") {
            let embed = start > 0 && rest.as_bytes()[start - 1] == b'!';
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else {
                break;
            };
            let target = after[..end].trim();
            if target.is_empty() {
                break;
            }
            let prefix = if embed {
                &rest[..start - 1]
            } else {
                &rest[..start]
            };
            output.push_str(prefix);
            output.push(TOKEN_MARKER);
            output.push_str(&tokens.len().to_string());
            output.push(TOKEN_MARKER);
            tokens.push((embed, target.to_owned()));
            rest = &after[end + 2..];
        }
        output.push_str(rest);
    }
    (output, tokens)
}

struct Builder<'a> {
    library: &'a mut Library,
    root: &'a Path,
    base_dir: &'a Path,
    attachments: &'a BTreeMap<String, PathBuf>,
    tokens: &'a [(bool, String)],
    report: &'a mut VaultReport,
    root_nodes: Vec<DocumentNode>,
    stack: Vec<DocumentNode>,
    open: Vec<Open>,
    marks: Vec<DocumentMark>,
    image: Option<ImageCapture>,
    table_aligns: Vec<Alignment>,
    cell_index: usize,
    in_head: bool,
    pending_blocks: Vec<DocumentNode>,
}

impl<'a> Builder<'a> {
    fn new(
        library: &'a mut Library,
        root: &'a Path,
        base_dir: &'a Path,
        attachments: &'a BTreeMap<String, PathBuf>,
        tokens: &'a [(bool, String)],
        report: &'a mut VaultReport,
    ) -> Self {
        Self {
            library,
            root,
            base_dir,
            attachments,
            tokens,
            report,
            root_nodes: Vec::new(),
            stack: Vec::new(),
            open: Vec::new(),
            marks: Vec::new(),
            image: None,
            table_aligns: Vec::new(),
            cell_index: 0,
            in_head: false,
            pending_blocks: Vec::new(),
        }
    }

    fn empty(kind: NodeType) -> DocumentNode {
        DocumentNode {
            kind,
            attrs: NodeAttributes {
                id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            },
            content: Vec::new(),
            text: None,
            marks: Vec::new(),
        }
    }

    fn inline(kind: NodeType) -> DocumentNode {
        DocumentNode {
            kind,
            attrs: NodeAttributes::default(),
            content: Vec::new(),
            text: None,
            marks: Vec::new(),
        }
    }

    fn text_node(value: &str, marks: &[DocumentMark]) -> DocumentNode {
        DocumentNode {
            kind: NodeType::Text,
            attrs: NodeAttributes::default(),
            content: Vec::new(),
            text: Some(value.to_owned()),
            marks: marks.to_vec(),
        }
    }

    fn push(&mut self, node: DocumentNode) {
        match self.stack.last_mut() {
            Some(parent) => parent.content.push(node),
            None => self.root_nodes.push(node),
        }
    }

    fn open_node(&mut self, kind: NodeType) {
        self.stack.push(Self::empty(kind));
        self.open.push(Open::Node);
    }

    fn close_node(&mut self) {
        if matches!(self.open.pop(), Some(Open::Node))
            && let Some(node) = self.stack.pop()
        {
            self.push(node);
        }
    }
    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.open_node(NodeType::Paragraph),
            Tag::Heading { level, .. } => {
                let mut node = Self::empty(NodeType::Heading);
                node.attrs.level = Some(heading_level(level));
                self.stack.push(node);
                self.open.push(Open::Node);
            }
            Tag::BlockQuote(_) => self.open_node(NodeType::Blockquote),
            Tag::CodeBlock(kind) => {
                let mut node = Self::empty(NodeType::CodeBlock);
                if let CodeBlockKind::Fenced(info) = kind
                    && let Some(language) = info
                        .split_whitespace()
                        .next()
                        .filter(|language| !language.is_empty())
                {
                    node.attrs.language = Some(language.to_owned());
                }
                self.stack.push(node);
                self.open.push(Open::Node);
            }
            Tag::List(start) => {
                let mut node = Self::empty(if start.is_some() {
                    NodeType::OrderedList
                } else {
                    NodeType::BulletList
                });
                node.attrs.start = start.map(|value| value as u32);
                self.stack.push(node);
                self.open.push(Open::Node);
            }
            Tag::Item => self.open_node(NodeType::ListItem),
            Tag::FootnoteDefinition(name) => {
                let mut node = Self::empty(NodeType::FootnoteDefinition);
                node.attrs.label = Some(name.into_string());
                self.stack.push(node);
                self.open.push(Open::Node);
            }
            Tag::Table(aligns) => {
                self.table_aligns = aligns;
                self.cell_index = 0;
                self.open_node(NodeType::Table);
            }
            Tag::TableHead => {
                self.in_head = true;
                self.cell_index = 0;
                self.open_node(NodeType::TableRow);
            }
            Tag::TableRow => {
                self.cell_index = 0;
                self.open_node(NodeType::TableRow);
            }
            Tag::TableCell => {
                let kind = if self.in_head {
                    NodeType::TableHeader
                } else {
                    NodeType::TableCell
                };
                let mut node = Self::empty(kind);
                node.attrs.align =
                    self.table_aligns
                        .get(self.cell_index)
                        .and_then(|align| match align {
                            Alignment::Left => Some(TableAlignment::Left),
                            Alignment::Center => Some(TableAlignment::Center),
                            Alignment::Right => Some(TableAlignment::Right),
                            Alignment::None => None,
                        });
                self.stack.push(node);
                self.open.push(Open::Node);
            }
            Tag::Emphasis => self.start_mark(MarkType::Italic, MarkAttributes::default()),
            Tag::Strong => self.start_mark(MarkType::Bold, MarkAttributes::default()),
            Tag::Strikethrough => self.start_mark(MarkType::Strike, MarkAttributes::default()),
            Tag::Link { dest_url, .. } => self.start_mark(
                MarkType::Link,
                MarkAttributes {
                    href: Some(dest_url.into_string()),
                    ..Default::default()
                },
            ),
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some(ImageCapture {
                    dest: dest_url.into_string(),
                    title: title.into_string(),
                    alt: String::new(),
                });
                self.open.push(Open::Image);
            }
            _ => self.open.push(Open::Ignore),
        }
    }

    fn start_mark(&mut self, kind: MarkType, attrs: MarkAttributes) {
        self.marks.push(DocumentMark { kind, attrs });
        self.open.push(Open::Mark);
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                if matches!(self.open.pop(), Some(Open::Mark)) {
                    self.marks.pop();
                }
            }
            TagEnd::Image => {
                self.open.pop();
                if let Some(capture) = self.image.take() {
                    self.emit_image(&capture, false);
                }
            }
            TagEnd::TableHead => {
                self.in_head = false;
                self.close_node();
            }
            TagEnd::TableCell => {
                self.close_node();
                self.cell_index += 1;
            }
            _ => self.close_node(),
        }
    }

    fn text(&mut self, value: &str) {
        if let Some(capture) = self.image.as_mut() {
            capture.alt.push_str(value);
            return;
        }
        if self
            .stack
            .last()
            .is_some_and(|node| node.kind == NodeType::CodeBlock)
        {
            if let Some(node) = self.stack.last_mut() {
                node.text.get_or_insert_with(String::new).push_str(value);
            }
            return;
        }
        self.emit_text(value);
    }

    fn code(&mut self, value: &str) {
        let mut marks = self.marks.clone();
        marks.push(DocumentMark {
            kind: MarkType::Code,
            attrs: MarkAttributes::default(),
        });
        self.push(Self::text_node(value, &marks));
    }

    fn soft_break(&mut self) {
        if self.image.is_some() {
            return;
        }
        self.emit_text("\n");
    }

    fn hard_break(&mut self) {
        self.push(Self::empty(NodeType::HardBreak));
    }

    fn rule(&mut self) {
        self.push(Self::empty(NodeType::HorizontalRule));
    }

    fn task_marker(&mut self, checked: bool) {
        self.emit_text(if checked { "[x] " } else { "[ ] " });
    }

    fn raw_html(&mut self, html: &str) {
        self.report
            .losses
            .push("原始 HTML 按纯文本保留，导出时会被转义。".to_owned());
        self.emit_text(html);
    }
    /// Emits text, restoring the wiki-link and embed markers that were
    /// protected before parsing.
    fn emit_text(&mut self, value: &str) {
        let marks = self.marks.clone();
        let mut buffer = String::new();
        let mut rest = value;
        while let Some(start) = rest.find(TOKEN_MARKER) {
            let after = &rest[start + TOKEN_MARKER.len_utf8()..];
            let Some(end) = after.find(TOKEN_MARKER) else {
                break;
            };
            let index = after[..end].parse::<usize>().ok();
            buffer.push_str(&rest[..start]);
            if !buffer.is_empty() {
                self.push(Self::text_node(&buffer, &marks));
                buffer.clear();
            }
            if let Some((embed, target)) = index.and_then(|index| self.tokens.get(index)) {
                if *embed {
                    self.emit_image(
                        &ImageCapture {
                            dest: target.clone(),
                            title: String::new(),
                            alt: String::new(),
                        },
                        true,
                    );
                } else {
                    self.emit_wikilink(target);
                }
            }
            rest = &after[end + TOKEN_MARKER.len_utf8()..];
        }
        buffer.push_str(rest);
        if !buffer.is_empty() {
            self.push(Self::text_node(&buffer, &marks));
        }
    }
    fn emit_wikilink(&mut self, target: &str) {
        let mut parts = target.splitn(2, '|');
        let name = parts.next().unwrap_or_default().trim();
        let alias = parts
            .next()
            .map(str::trim)
            .filter(|alias| !alias.is_empty());
        if name.is_empty() {
            return;
        }
        let mut node = Self::inline(NodeType::WikiLink);
        node.attrs.wiki_target = Some(name.to_owned());
        node.attrs.occurrence_id = Some(uuid::Uuid::new_v4().to_string());
        if let Some(alias) = alias {
            node.attrs.label = Some(alias.to_owned());
        }
        self.push(node);
    }

    fn emit_image(&mut self, capture: &ImageCapture, embed: bool) {
        let marks = self.marks.clone();
        if capture.dest.starts_with("http://") || capture.dest.starts_with("https://") {
            self.report
                .losses
                .push(format!("远程图片未下载：{}", capture.dest));
            self.place_block(Self::wiki_embed(&capture.dest), &marks);
            return;
        }
        let Some(path) = self.resolve(&capture.dest) else {
            self.report
                .losses
                .push(format!("未找到附件：{}", capture.dest));
            if embed {
                self.place_block(Self::wiki_embed(&capture.dest), &marks);
            } else {
                self.push(Self::text_node(&literal_image(capture, false), &marks));
            }
            return;
        };
        let is_image = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                IMAGE_EXTENSIONS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
            });
        if !is_image {
            self.report.losses.push(format!(
                "附件不是受支持的图片类型，仅保留引用：{}",
                capture.dest
            ));
            self.place_block(Self::wiki_embed(&capture.dest), &marks);
            return;
        }
        let Some(((hash, name), _)) = self.import_image(&path) else {
            self.report
                .losses
                .push(format!("附件导入失败，仅保留引用：{}", capture.dest));
            self.place_block(Self::wiki_embed(&capture.dest), &marks);
            return;
        };
        let mut node = Self::empty(NodeType::Image);
        node.attrs.blob_hash = Some(hash);
        node.attrs.original_name = Some(name);
        if !capture.alt.trim().is_empty() {
            node.attrs.alt = Some(capture.alt.trim().to_owned());
        }
        if !capture.title.trim().is_empty() {
            node.attrs.title = Some(capture.title.trim().to_owned());
        }
        self.place_block(node, &marks);
    }

    fn wiki_embed(target: &str) -> DocumentNode {
        let mut node = Self::empty(NodeType::WikiEmbed);
        node.attrs.wiki_target = Some(target.to_owned());
        node.attrs.occurrence_id = Some(uuid::Uuid::new_v4().to_string());
        node
    }

    /// Images and wiki embeds are block nodes. They are parked while the
    /// paragraph is still open and replaced by a marker run that is hoisted
    /// back out into sibling blocks once the tree is closed.
    fn place_block(&mut self, node: DocumentNode, marks: &[DocumentMark]) {
        let index = self.pending_blocks.len();
        self.pending_blocks.push(node);
        self.push(Self::text_node(&block_marker(index), marks));
    }
    fn import_image(&mut self, path: &Path) -> Option<((String, String), ())> {
        let name = path.file_name()?.to_str()?.to_owned();
        let bytes = fs::read(path).ok()?;
        let command = uuid::Uuid::new_v4().to_string();
        let import = self
            .library
            .import_attachment(&command, &name, &bytes)
            .ok()?;
        self.report.attachments += 1;
        Some(((import.blob.hash, name), ()))
    }

    fn resolve(&self, target: &str) -> Option<PathBuf> {
        let target = target.trim();
        if target.is_empty() {
            return None;
        }
        let cleaned = target.trim_start_matches("./").replace('\\', "/");
        let relative = Path::new(&cleaned);
        if !relative
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            let candidate = self.base_dir.join(relative);
            if candidate.is_file() {
                let root = self.root.canonicalize().ok();
                let resolved = candidate.canonicalize().ok();
                if let (Some(root), Some(resolved)) = (root, resolved)
                    && resolved.starts_with(root)
                {
                    return Some(resolved);
                }
            }
        }
        let name = cleaned
            .rsplit('/')
            .next()
            .unwrap_or(&cleaned)
            .to_lowercase();
        self.attachments.get(&name).cloned()
    }

    fn finish(mut self) -> Vec<DocumentNode> {
        while !self.open.is_empty() {
            let open = self.open.pop();
            if matches!(open, Some(Open::Node))
                && let Some(node) = self.stack.pop()
            {
                match self.stack.last_mut() {
                    Some(parent) => parent.content.push(node),
                    None => self.root_nodes.push(node),
                }
            }
        }
        let root = hoist_blocks(std::mem::take(&mut self.root_nodes), &self.pending_blocks);
        normalize_tree(root, false)
    }
}

fn wrap_inlines(children: Vec<DocumentNode>) -> Vec<DocumentNode> {
    let mut out = Vec::new();
    let mut run = Vec::new();
    for child in children {
        if child.is_inline() {
            run.push(child);
            continue;
        }
        if !run.is_empty() {
            let mut paragraph = Builder::empty(NodeType::Paragraph);
            paragraph.content = std::mem::take(&mut run);
            out.push(paragraph);
        }
        out.push(child);
    }
    if !run.is_empty() {
        let mut paragraph = Builder::empty(NodeType::Paragraph);
        paragraph.content = run;
        out.push(paragraph);
    }
    out
}

/// Tight lists emit inline content straight inside the item, and images are
/// hoisted into blocks; both leave containers that expect block children.
fn normalize_tree(nodes: Vec<DocumentNode>, wrap: bool) -> Vec<DocumentNode> {
    let mut out = Vec::new();
    for mut node in nodes {
        let container = matches!(
            node.kind,
            NodeType::BulletList
                | NodeType::OrderedList
                | NodeType::TaskList
                | NodeType::ListItem
                | NodeType::TaskItem
                | NodeType::Blockquote
                | NodeType::Callout
                | NodeType::FootnoteDefinition
                | NodeType::Table
                | NodeType::TableRow
                | NodeType::TableCell
                | NodeType::TableHeader
        );
        let children = std::mem::take(&mut node.content);
        let children = normalize_tree(children, container);
        node.content = if container {
            wrap_inlines(children)
        } else {
            children
        };
        out.push(node);
    }
    if wrap { wrap_inlines(out) } else { out }
}

const BLOCK_MARKER: char = '\u{E000}';

fn block_marker(index: usize) -> String {
    format!("{BLOCK_MARKER}{index}{BLOCK_MARKER}")
}

fn block_marker_index(node: &DocumentNode) -> Option<usize> {
    if node.kind != NodeType::Text {
        return None;
    }
    let text = node.text.as_deref()?;
    text.strip_prefix(BLOCK_MARKER)?
        .strip_suffix(BLOCK_MARKER)?
        .parse()
        .ok()
}

fn push_segment(
    segment: &mut Vec<DocumentNode>,
    out: &mut Vec<DocumentNode>,
    kind: NodeType,
    level: Option<u8>,
) {
    if segment.is_empty() {
        return;
    }
    let mut part = Builder::empty(kind);
    part.attrs.level = level;
    part.content = std::mem::take(segment);
    out.push(part);
}

/// Replaces image/wiki-embed markers with the block nodes they stand for,
/// splitting the surrounding paragraph so a block never becomes an inline
/// child of text.
fn hoist_blocks(nodes: Vec<DocumentNode>, blocks: &[DocumentNode]) -> Vec<DocumentNode> {
    let mut out = Vec::new();
    for mut node in nodes {
        if !matches!(node.kind, NodeType::Paragraph | NodeType::Heading) {
            node.content = hoist_blocks(std::mem::take(&mut node.content), blocks);
            out.push(node);
            continue;
        }
        if !node
            .content
            .iter()
            .any(|child| block_marker_index(child).is_some())
        {
            out.push(node);
            continue;
        }
        let first_kind = node.kind.clone();
        let level = node.attrs.level;
        let mut first = true;
        let mut segment = Vec::new();
        for child in std::mem::take(&mut node.content) {
            if let Some(index) = block_marker_index(&child) {
                if !segment.is_empty() {
                    let kind = if first {
                        first_kind.clone()
                    } else {
                        NodeType::Paragraph
                    };
                    let level = if first { level } else { None };
                    push_segment(&mut segment, &mut out, kind, level);
                }
                if let Some(block) = blocks.get(index) {
                    out.push(block.clone());
                }
                first = false;
                continue;
            }
            segment.push(child);
        }
        if !segment.is_empty() {
            let kind = if first {
                first_kind.clone()
            } else {
                NodeType::Paragraph
            };
            let level = if first { level } else { None };
            push_segment(&mut segment, &mut out, kind, level);
        }
    }
    out
}

fn literal_image(capture: &ImageCapture, embed: bool) -> String {
    if embed {
        format!("![[{}]]", capture.dest)
    } else {
        format!("![{}]({})", capture.alt, capture.dest)
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn markdown_to_nodes(
    text: &str,
    library: &mut Library,
    root: &Path,
    base_dir: &Path,
    attachments: &BTreeMap<String, PathBuf>,
    report: &mut VaultReport,
) -> Vec<DocumentNode> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_MATH);

    let (protected, tokens) = protect_wikilinks(text);
    let mut builder = Builder::new(library, root, base_dir, attachments, &tokens, report);
    for event in Parser::new_ext(&protected, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(end) => builder.end(end),
            Event::Text(value) => builder.text(&value),
            Event::Code(value) => builder.code(&value),
            Event::InlineMath(value) => {
                let mut node = Builder::inline(NodeType::InlineMath);
                node.attrs.source = Some(value.into_string());
                builder.push(node);
            }
            Event::DisplayMath(value) => {
                let mut node = Builder::empty(NodeType::BlockMath);
                node.attrs.source = Some(value.into_string());
                builder.push(node);
            }
            Event::SoftBreak => builder.soft_break(),
            Event::HardBreak => builder.hard_break(),
            Event::Rule => builder.rule(),
            Event::TaskListMarker(checked) => builder.task_marker(checked),
            Event::Html(value) | Event::InlineHtml(value) => builder.raw_html(&value),
            Event::FootnoteReference(name) => {
                let mut node = Builder::inline(NodeType::FootnoteReference);
                node.attrs.label = Some(name.into_string());
                builder.push(node);
            }
        }
    }
    builder.finish()
}
impl Library {
    pub fn export_obsidian_vault(&self, destination: &Path) -> Result<VaultReport> {
        if destination.exists() {
            let empty = fs::read_dir(destination)
                .map_err(|_| Error::InvalidInput("vault_unreadable"))?
                .next()
                .is_none();
            if !empty {
                return Err(Error::InvalidInput("vault_destination_not_empty"));
            }
        }
        fs::create_dir_all(destination.join("notes"))
            .map_err(|_| Error::InvalidInput("vault_unwritable"))?;
        fs::create_dir_all(destination.join("attachments"))
            .map_err(|_| Error::InvalidInput("vault_unwritable"))?;
        let notes = self.list_notes()?;
        let titles: BTreeMap<String, String> = notes
            .iter()
            .map(|note| (note.id.clone(), note.title.clone()))
            .collect();
        let mut used = BTreeSet::new();
        let mut report = VaultReport {
            notes: 0,
            markdown_files: 0,
            attachments: 0,
            links: 0,
            tags: 0,
            skipped_existing: 0,
            unresolved_links: 0,
            losses: vec![
                "富文本块 ID 与评论不会写入 Markdown，需要它们时请使用完整备份。".into(),
                "只打包 png/jpeg/webp 图片；其他附件不随 vault 导出。".into(),
            ],
            normalized: Vec::new(),
        };
        let mut attachment_hashes = BTreeSet::new();
        for note in notes {
            let metadata = self.object_metadata(&note.id)?;
            let tags = metadata
                .tags
                .iter()
                .map(|tag| tag.display_name.clone())
                .collect::<Vec<_>>();
            report.tags += tags.len() as u32;
            let mut assets = BTreeMap::new();
            for node in note.document.nodes() {
                if node.kind != NodeType::Image {
                    continue;
                }
                let Some(hash) = node.attrs.blob_hash.as_deref() else {
                    continue;
                };
                if attachment_hashes.insert(hash.to_owned()) {
                    let (info, bytes) = self.attachment(&note.revision_id, hash)?;
                    let extension = match info.media_type.as_str() {
                        "image/png" => "png",
                        "image/jpeg" => "jpg",
                        "image/webp" => "webp",
                        _ => {
                            report
                                .losses
                                .push(format!("未导出不支持的图片类型：{}", info.media_type));
                            continue;
                        }
                    };
                    let path = format!("attachments/{hash}.{extension}");
                    fs::write(destination.join(&path), bytes)
                        .map_err(|_| Error::InvalidInput("vault_unwritable"))?;
                    report.attachments += 1;
                    assets.insert(hash.to_owned(), format!("../{path}"));
                }
            }
            let markdown = crate::export::markdown_document(&note, &assets, false)?;
            let (markdown, links) = rewrite_links(&markdown, &titles);
            report.links += links;
            report.links += note
                .document
                .nodes()
                .iter()
                .filter(|node| matches!(node.kind, NodeType::WikiLink | NodeType::WikiEmbed))
                .count() as u32;
            let existing = note.document.nodes().iter().find_map(|node| {
                (node.kind == NodeType::Frontmatter)
                    .then(|| node.attrs.source.as_deref().unwrap_or_default())
                    .map(str::to_owned)
            });
            let preserved = existing
                .as_deref()
                .map(preserved_groups)
                .unwrap_or_default();
            let body = format!(
                "{}{}",
                frontmatter_block(&note.title, &note.id, &tags, &preserved),
                markdown
            );
            let mut filename = sanitize_filename(&note.title);
            let mut candidate = filename.clone();
            let mut ordinal = 2;
            while !used.insert(candidate.clone()) {
                candidate = format!("{filename} ({ordinal})");
                ordinal += 1;
            }
            filename = candidate;
            fs::write(
                destination.join("notes").join(format!("{filename}.md")),
                body,
            )
            .map_err(|_| Error::InvalidInput("vault_unwritable"))?;
            report.notes += 1;
            report.markdown_files += 1;
        }
        let bytes = serde_json::to_vec_pretty(&report)?;
        fs::write(destination.join("spanseek-vault-report.json"), bytes)
            .map_err(|_| Error::InvalidInput("vault_unwritable"))?;
        Ok(report)
    }
    pub fn import_obsidian_vault(
        &mut self,
        source: &Path,
        _command_prefix: &str,
    ) -> Result<VaultReport> {
        let files = markdown_files(source)?;
        let existing = self
            .list_notes()?
            .into_iter()
            .map(|note| note.title)
            .collect::<BTreeSet<_>>();
        let mut report = VaultReport {
            notes: 0,
            markdown_files: 0,
            attachments: 0,
            links: 0,
            tags: 0,
            skipped_existing: 0,
            unresolved_links: 0,
            losses: vec![
                "Markdown 内联语法只映射 spanseek 支持的加粗/斜体/删除线/代码/链接；其余语法按文本保留。"
                    .into(),
                "非图片附件只保留引用文本，不写入附件库。".into(),
            ],
            normalized: Vec::new(),
        };
        let attachments = attachment_index(source)?;
        let mut created = BTreeMap::<String, CreatedNote>::new();
        let mut seen = BTreeSet::<String>::new();
        for path in files {
            let bytes = fs::read(&path).map_err(|_| Error::InvalidInput("vault_unreadable"))?;
            if bytes.len() as u64 > MAX_MARKDOWN_BYTES {
                report
                    .losses
                    .push(format!("跳过超过 2 MiB 的 Markdown：{}", path.display()));
                continue;
            }
            let text = String::from_utf8(bytes)
                .map_err(|_| Error::InvalidInput("vault_markdown_not_utf8"))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let (body, yaml) = split_frontmatter(text);
            let groups = yaml.map(yaml_groups).unwrap_or_default();
            let title = title_from_groups(&groups, body, &path);
            report.markdown_files += 1;
            if existing.contains(&title) || !seen.insert(title.clone()) {
                report.skipped_existing += 1;
                continue;
            }
            let tags = tags_from_groups(&groups);
            let preserved = groups
                .into_iter()
                .filter(|(key, _)| !matches!(key.as_str(), "title" | "tags"))
                .collect::<Vec<_>>();
            let body = fold_title_heading(body, &title, &mut report);
            let base_dir = path.parent().unwrap_or(source);
            let nodes = markdown_to_nodes(body, self, source, base_dir, &attachments, &mut report);
            let mut content = Vec::new();
            if !preserved.is_empty() {
                let mut frontmatter = Builder::empty(NodeType::Frontmatter);
                frontmatter.attrs.source = Some(render_groups(&preserved));
                content.push(frontmatter);
            }
            content.extend(nodes);
            if content
                .iter()
                .all(|node| node.kind == NodeType::Frontmatter)
            {
                content.push(Builder::empty(NodeType::Paragraph));
            }
            let document = NoteDocument {
                kind: DocumentType::Doc,
                content,
            };

            let note = self
                .create_document(
                    &uuid::Uuid::new_v4().to_string(),
                    CreateNote {
                        collection_id: None,
                        base_collection_row_version: None,
                        title: title.clone(),
                        document_schema: DOCUMENT_SCHEMA,
                        document,
                    },
                )?
                .note;
            created.insert(
                title.clone(),
                CreatedNote {
                    id: note.id,
                    tags,
                    body: body.to_owned(),
                },
            );
            report.notes += 1;
        }
        let title_index: BTreeMap<String, String> = created
            .keys()
            .map(|title| (title.to_lowercase(), title.clone()))
            .collect();
        for note in created.values() {
            for tag in &note.tags {
                let metadata = self.object_metadata(&note.id)?;
                self.assign_tag(
                    &uuid::Uuid::new_v4().to_string(),
                    AssignTag {
                        targets: vec![MetadataTarget {
                            object_id: note.id.clone(),
                            base_row_version: metadata.row_version,
                        }],
                        tag: TagSelector::Name { name: tag.clone() },
                    },
                )?;
                report.tags += 1;
            }
            for target in wikilinks(&note.body) {
                let key = target
                    .rsplit('/')
                    .next()
                    .unwrap_or(&target)
                    .trim_end_matches(".md")
                    .to_lowercase();
                let Some(target_title) = title_index.get(&key) else {
                    report.unresolved_links += 1;
                    continue;
                };
                let target_id = &created[target_title].id;
                self.link_notes(&note.id, target_id, "Obsidian wikilink import")?;
                report.links += 1;
            }
        }
        Ok(report)
    }
}

struct CreatedNote {
    id: String,
    tags: Vec<String>,
    body: String,
}

/// Removes CommonMark backslash escapes so an escaped heading can still be
/// compared with the plain note title.
fn unescape_markdown(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(next) = chars.next()
        {
            output.push(next);
            continue;
        }
        output.push(ch);
    }
    output
}

/// Obsidian vaults usually start a note with `# Title`, which spanseek stores
/// as the note title. Folding it keeps export from emitting the heading twice;
/// the title (and therefore the heading) is reproduced on export.
fn fold_title_heading<'a>(body: &'a str, title: &str, report: &mut VaultReport) -> &'a str {
    let trimmed = body.trim_start_matches(['\n', '\r']);
    let Some(after) = trimmed.strip_prefix("# ") else {
        return body;
    };
    let Some((line, rest)) = after.split_once('\n') else {
        return body;
    };
    if unescape_markdown(line.trim()) != title {
        return body;
    }
    report
        .normalized
        .push("首行一级标题与笔记标题一致，已折叠为笔记标题。".to_owned());
    rest.trim_start_matches(['\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilink_target_ignores_alias_and_heading() {
        assert_eq!(
            wikilinks("a [[目标笔记|别名]] b [[另一篇#小节]]"),
            ["目标笔记", "另一篇"]
        );
    }

    #[test]
    fn frontmatter_tags_accept_inline_and_list_forms() {
        let groups = yaml_groups("tags: [\"甲\", '乙']\nother: 1");
        assert_eq!(tags_from_groups(&groups), ["甲", "乙"]);
        let groups = yaml_groups("tags:\n  - 甲\n  - 乙\naliases: [丙]");
        assert_eq!(tags_from_groups(&groups), ["甲", "乙"]);
    }

    #[test]
    fn yaml_groups_keep_list_items_with_their_key() {
        let groups = yaml_groups("title: \"方法\"\ntags:\n  - 研究\naliases: [证据]");
        let preserved = groups
            .into_iter()
            .filter(|(key, _)| !matches!(key.as_str(), "title" | "tags"))
            .collect::<Vec<_>>();
        assert_eq!(render_groups(&preserved), "aliases: [证据]");
    }

    #[test]
    fn unterminated_frontmatter_stays_body_text() {
        let (body, yaml) = split_frontmatter("---\ntitle: x\n\n# 正文");
        assert!(yaml.is_none());
        assert_eq!(body, "---\ntitle: x\n\n# 正文");
    }

    #[test]
    fn fold_title_heading_only_matches_the_note_title() {
        let mut report = VaultReport {
            notes: 0,
            markdown_files: 0,
            attachments: 0,
            links: 0,
            tags: 0,
            skipped_existing: 0,
            unresolved_links: 0,
            losses: Vec::new(),
            normalized: Vec::new(),
        };
        assert_eq!(
            fold_title_heading("# 方法\n\n正文", "方法", &mut report),
            "正文"
        );
        assert_eq!(
            fold_title_heading("# 别的\n\n正文", "方法", &mut report),
            "# 别的\n\n正文"
        );
        assert_eq!(
            fold_title_heading(
                "# Retrieval\\-Augmented Tasks\n\n正文",
                "Retrieval-Augmented Tasks",
                &mut report
            ),
            "正文"
        );
        assert_eq!(report.normalized.len(), 2);
    }
}
