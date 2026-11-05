//! The persisted editor schema. Unknown fields are rejected instead of discarded.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;
use uuid::Uuid;

pub const DOCUMENT_SCHEMA: u32 = 3;
pub const SUPPORTED_DOCUMENT_SCHEMAS: &[u32] = &[1, 2, 3];
const MAX_NODES: usize = 50_000;
const MAX_DEPTH: usize = 32;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub enum DocumentType {
    #[serde(rename = "doc")]
    Doc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct NoteDocument {
    #[serde(rename = "type")]
    pub kind: DocumentType,
    pub content: Vec<DocumentNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum NodeType {
    Paragraph,
    Heading,
    Text,
    HardBreak,
    BulletList,
    OrderedList,
    ListItem,
    TaskList,
    TaskItem,
    Blockquote,
    CodeBlock,
    HorizontalRule,
    Image,
    NoteReference,
    Table,
    TableRow,
    TableCell,
    TableHeader,
    Callout,
    InlineMath,
    BlockMath,
    WikiLink,
    WikiEmbed,
    FootnoteReference,
    FootnoteDefinition,
    Comment,
    CommentBlock,
    Frontmatter,
}

impl NodeType {
    /// Stable storage/search spelling without depending on Rust's Debug output.
    pub(crate) fn storage_name(&self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Heading => "heading",
            Self::Text => "text",
            Self::HardBreak => "hardBreak",
            Self::BulletList => "bulletList",
            Self::OrderedList => "orderedList",
            Self::ListItem => "listItem",
            Self::TaskList => "taskList",
            Self::TaskItem => "taskItem",
            Self::Blockquote => "blockquote",
            Self::CodeBlock => "codeBlock",
            Self::HorizontalRule => "horizontalRule",
            Self::Image => "image",
            Self::NoteReference => "noteReference",
            Self::Table => "table",
            Self::TableRow => "tableRow",
            Self::TableCell => "tableCell",
            Self::TableHeader => "tableHeader",
            Self::Callout => "callout",
            Self::InlineMath => "inlineMath",
            Self::BlockMath => "blockMath",
            Self::WikiLink => "wikiLink",
            Self::WikiEmbed => "wikiEmbed",
            Self::FootnoteReference => "footnoteReference",
            Self::FootnoteDefinition => "footnoteDefinition",
            Self::Comment => "comment",
            Self::CommentBlock => "commentBlock",
            Self::Frontmatter => "frontmatter",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum TableAlignment {
    Left,
    Center,
    Right,
}

impl TableAlignment {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct NodeAttributes {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colspan: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rowspan: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colwidth: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TableAlignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callout_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_height: Option<u32>,
    /// The CSL style used to render a generated bibliography section. This is
    /// optional metadata on the bibliography heading; ordinary headings leave
    /// both bibliography fields unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bibliography_style_id: Option<String>,
    /// Fingerprint of the generated bibliography content. A mismatch tells the
    /// editor that the section was edited by hand before replacing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bibliography_fingerprint: Option<String>,
}

impl NodeAttributes {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct DocumentNode {
    #[serde(rename = "type")]
    pub kind: NodeType,
    #[serde(default, skip_serializing_if = "NodeAttributes::is_empty")]
    pub attrs: NodeAttributes,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<DocumentNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<DocumentMark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum MarkType {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Underline,
    Superscript,
    Subscript,
    Highlight,
    TextStyle,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct MarkAttributes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl MarkAttributes {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct DocumentMark {
    #[serde(rename = "type")]
    pub kind: MarkType,
    #[serde(default, skip_serializing_if = "MarkAttributes::is_empty")]
    pub attrs: MarkAttributes,
}

pub(crate) fn valid_uuid(value: &str) -> Result<()> {
    Uuid::parse_str(value).map_err(|_| Error::InvalidInput("identity must be a UUID"))?;
    Ok(())
}

fn invalid(message: &'static str) -> Error {
    Error::InvalidDocument(message)
}

impl NoteDocument {
    pub fn from_text(anchor_id: String, text: &str) -> Self {
        Self {
            kind: DocumentType::Doc,
            content: vec![DocumentNode {
                kind: NodeType::Paragraph,
                attrs: NodeAttributes {
                    id: anchor_id,
                    ..Default::default()
                },
                text: None,
                marks: vec![],
                content: if text.is_empty() {
                    vec![]
                } else {
                    vec![DocumentNode {
                        kind: NodeType::Text,
                        attrs: Default::default(),
                        content: vec![],
                        text: Some(text.into()),
                        marks: vec![],
                    }]
                },
            }],
        }
    }

    pub fn validate(&self, schema: u32) -> Result<()> {
        if !SUPPORTED_DOCUMENT_SCHEMAS.contains(&schema) {
            return Err(invalid("unsupported document schema"));
        }
        if self.content.is_empty() {
            return Err(invalid("document must contain a block"));
        }
        let mut state = Validation {
            schema,
            ids: BTreeSet::new(),
            occurrences: BTreeSet::new(),
            block_aliases: BTreeSet::new(),
            footnotes: BTreeSet::new(),
            count: 0,
        };
        for (index, node) in self.content.iter().enumerate() {
            if !node.is_block() {
                return Err(invalid("document children must be blocks"));
            }
            if node.kind == NodeType::Frontmatter && index != 0 {
                return Err(invalid("frontmatter must be the first document block"));
            }
            node.validate(1, &mut state)?;
        }
        if serde_json::to_vec(self)?.len() > MAX_BYTES {
            return Err(invalid("document exceeds 4 MiB"));
        }
        if schema == 1
            && (self.content.iter().any(|n| {
                n.kind != NodeType::Paragraph
                    || n.content
                        .iter()
                        .any(|c| c.kind != NodeType::Text || !c.marks.is_empty())
            }))
        {
            return Err(invalid("schema 1 only supports text paragraphs"));
        }
        Ok(())
    }

    pub fn plain_text(&self) -> String {
        self.content
            .iter()
            .map(DocumentNode::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn nodes(&self) -> Vec<&DocumentNode> {
        fn visit<'a>(node: &'a DocumentNode, out: &mut Vec<&'a DocumentNode>) {
            out.push(node);
            for child in &node.content {
                visit(child, out);
            }
        }
        let mut out = vec![];
        for node in &self.content {
            visit(node, &mut out);
        }
        out
    }

    /// Creates an independent document identity tree for a new object while
    /// retaining text, marks and non-internal references. Used when a conflict
    /// is saved as a new note.
    pub fn copy_for_object(&self, source: &str, destination: &str) -> Self {
        self.copy_with_new_ids(source, destination)
    }
    pub(crate) fn copy_with_new_ids(&self, source: &str, destination: &str) -> Self {
        let mapping: BTreeMap<_, _> = self
            .nodes()
            .iter()
            .filter(|n| !n.attrs.id.is_empty())
            .map(|n| (n.attrs.id.clone(), Uuid::new_v4().to_string()))
            .collect();
        fn update(
            node: &mut DocumentNode,
            map: &BTreeMap<String, String>,
            source: &str,
            destination: &str,
        ) {
            if let Some(id) = map.get(&node.attrs.id) {
                node.attrs.id.clone_from(id);
            }
            if node.attrs.occurrence_id.is_some() {
                node.attrs.occurrence_id = Some(Uuid::new_v4().to_string());
            }
            // Pinned references retain their original object: its immutable
            // revision does not belong to the newly created copy.
            if node.attrs.target_id.as_deref() == Some(source)
                && node.attrs.target_revision_id.is_none()
            {
                node.attrs.target_id = Some(destination.into());
                if let Some(block) = &node.attrs.target_block_id {
                    node.attrs.target_block_id =
                        map.get(block).cloned().or_else(|| Some(block.clone()));
                }
            }
            for child in &mut node.content {
                update(child, map, source, destination);
            }
        }
        let mut result = self.clone();
        for node in &mut result.content {
            update(node, &mapping, source, destination);
        }
        result
    }
}

struct Validation {
    schema: u32,
    ids: BTreeSet<String>,
    occurrences: BTreeSet<String>,
    block_aliases: BTreeSet<String>,
    footnotes: BTreeSet<String>,
    count: usize,
}

impl DocumentNode {
    pub(crate) fn is_inline(&self) -> bool {
        matches!(
            self.kind,
            NodeType::Text
                | NodeType::HardBreak
                | NodeType::NoteReference
                | NodeType::InlineMath
                | NodeType::WikiLink
                | NodeType::FootnoteReference
                | NodeType::Comment
        )
    }

    pub(crate) fn is_body_reference(&self) -> bool {
        self.kind == NodeType::NoteReference
            || (matches!(self.kind, NodeType::WikiLink | NodeType::WikiEmbed)
                && self.attrs.target_id.is_some())
    }

    pub(crate) fn is_block(&self) -> bool {
        matches!(
            self.kind,
            NodeType::Paragraph
                | NodeType::Heading
                | NodeType::BulletList
                | NodeType::OrderedList
                | NodeType::TaskList
                | NodeType::Blockquote
                | NodeType::CodeBlock
                | NodeType::HorizontalRule
                | NodeType::Image
                | NodeType::Table
                | NodeType::Callout
                | NodeType::BlockMath
                | NodeType::WikiEmbed
                | NodeType::FootnoteDefinition
                | NodeType::CommentBlock
                | NodeType::Frontmatter
        )
    }

    pub(crate) fn plain_text(&self) -> String {
        if matches!(
            self.kind,
            NodeType::Comment | NodeType::CommentBlock | NodeType::Frontmatter
        ) {
            return String::new();
        }
        if matches!(self.kind, NodeType::InlineMath | NodeType::BlockMath) {
            return self.attrs.source.clone().unwrap_or_default();
        }
        if matches!(self.kind, NodeType::WikiLink | NodeType::WikiEmbed) {
            return self
                .attrs
                .label
                .clone()
                .or_else(|| self.attrs.wiki_target.clone())
                .unwrap_or_default();
        }
        if self.kind == NodeType::FootnoteReference {
            return String::new();
        }
        if let Some(text) = &self.text {
            return text.clone();
        }
        if self.kind == NodeType::HardBreak {
            return "\n".into();
        }
        if self.kind == NodeType::NoteReference {
            return self.attrs.label.clone().unwrap_or_default();
        }
        if self.kind == NodeType::Image {
            return self.attrs.alt.clone().unwrap_or_default();
        }
        let separator = if matches!(
            self.kind,
            NodeType::Paragraph | NodeType::Heading | NodeType::CodeBlock
        ) {
            ""
        } else {
            "\n"
        };
        let body = self
            .content
            .iter()
            .map(Self::plain_text)
            .collect::<Vec<_>>()
            .join(separator);
        if self.kind == NodeType::Callout
            && let Some(title) = &self.attrs.title
        {
            return format!("{title}\n{body}");
        }
        body
    }

    fn validate(&self, depth: usize, state: &mut Validation) -> Result<()> {
        state.count += 1;
        if depth > MAX_DEPTH || state.count > MAX_NODES {
            return Err(invalid("document nesting or node count exceeds bounds"));
        }
        if state.schema < 3
            && (matches!(
                self.kind,
                NodeType::Callout
                    | NodeType::InlineMath
                    | NodeType::BlockMath
                    | NodeType::WikiLink
                    | NodeType::WikiEmbed
                    | NodeType::FootnoteReference
                    | NodeType::FootnoteDefinition
                    | NodeType::Comment
                    | NodeType::CommentBlock
                    | NodeType::Frontmatter
            ) || self.attrs.block_id.is_some()
                || self.attrs.align.is_some()
                || self.attrs.task_marker.is_some()
                || self.attrs.callout_type.is_some()
                || self.attrs.collapsed.is_some()
                || self.attrs.source.is_some()
                || self.attrs.wiki_target.is_some()
                || self.attrs.embed_width.is_some()
                || self.attrs.embed_height.is_some()
                || self.attrs.bibliography_style_id.is_some()
                || self.attrs.bibliography_fingerprint.is_some()
                || self.marks.iter().any(|mark| {
                    matches!(
                        mark.kind,
                        MarkType::Underline
                            | MarkType::Superscript
                            | MarkType::Subscript
                            | MarkType::Highlight
                            | MarkType::TextStyle
                    ) || mark.attrs.color.is_some()
                }))
        {
            return Err(invalid("format requires document schema 3"));
        }
        let inline = self.is_inline();
        if !inline {
            valid_uuid(&self.attrs.id).map_err(|_| invalid("every block must have a UUID"))?;
            if !state.ids.insert(self.attrs.id.clone()) {
                return Err(invalid("duplicate block identity"));
            }
        } else if !self.attrs.id.is_empty() {
            return Err(invalid("inline nodes cannot own block identities"));
        }
        if let Some(alias) = &self.attrs.block_id
            && (inline
                || alias.is_empty()
                || alias.len() > 128
                || !alias
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || !state.block_aliases.insert(alias.clone()))
        {
            return Err(invalid(
                "block aliases must be unique Latin letters, digits or hyphens",
            ));
        }
        let allowed: &[&str] = match self.kind {
            NodeType::Heading => &[
                "id",
                "level",
                "bibliography_style_id",
                "bibliography_fingerprint",
            ],
            NodeType::OrderedList => &["id", "start"],
            NodeType::TaskItem => &["id", "checked", "task_marker"],
            NodeType::CodeBlock => &["id", "language"],
            NodeType::Image => &["id", "blob_hash", "original_name", "alt", "title"],
            NodeType::NoteReference => &[
                "target_id",
                "target_block_id",
                "target_revision_id",
                "occurrence_id",
                "label",
            ],
            NodeType::TableCell | NodeType::TableHeader => {
                &["id", "colspan", "rowspan", "colwidth", "align"]
            }
            NodeType::Callout => &["id", "callout_type", "title", "collapsed"],
            NodeType::InlineMath | NodeType::Comment => &["source"],
            NodeType::BlockMath | NodeType::CommentBlock | NodeType::Frontmatter => {
                &["id", "source"]
            }
            NodeType::WikiLink => &[
                "wiki_target",
                "label",
                "occurrence_id",
                "target_id",
                "target_block_id",
                "target_revision_id",
            ],
            NodeType::WikiEmbed => &[
                "id",
                "wiki_target",
                "label",
                "occurrence_id",
                "target_id",
                "target_block_id",
                "target_revision_id",
                "embed_width",
                "embed_height",
            ],
            NodeType::FootnoteReference => &["label"],
            NodeType::FootnoteDefinition => &["id", "label"],
            NodeType::Text | NodeType::HardBreak => &[],
            _ => &["id"],
        };
        let attrs = serde_json::to_value(&self.attrs)?;
        if attrs
            .as_object()
            .unwrap()
            .keys()
            .any(|key| !allowed.contains(&key.as_str()) && !(key == "block_id" && !inline))
        {
            return Err(invalid("attribute is not supported on this node"));
        }
        match self.kind {
            NodeType::Text => {
                if self.text.as_ref().is_none_or(String::is_empty) || !self.content.is_empty() {
                    return Err(invalid("text node must contain nonempty text only"));
                }
            }
            NodeType::Heading => {
                let maximum = if state.schema >= 3 { 6 } else { 3 };
                if !self.attrs.level.is_some_and(|n| (1..=maximum).contains(&n)) {
                    return Err(invalid(
                        "heading level is unsupported by this document schema",
                    ));
                }
                match (
                    self.attrs.bibliography_style_id.as_deref(),
                    self.attrs.bibliography_fingerprint.as_deref(),
                ) {
                    (None, None) => {}
                    (Some(style_id), Some(fingerprint)) => {
                        if style_id.is_empty()
                            || style_id.chars().count() > 1024
                            || style_id.chars().any(char::is_control)
                        {
                            return Err(invalid("bibliography style id is invalid"));
                        }
                        if fingerprint.len() != 16
                            || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
                        {
                            return Err(invalid("bibliography fingerprint is invalid"));
                        }
                    }
                    _ => {
                        return Err(invalid(
                            "bibliography style and fingerprint must both be present",
                        ));
                    }
                }
            }
            NodeType::OrderedList => {
                if self.attrs.start == Some(0) {
                    return Err(invalid("ordered list start must be positive"));
                }
            }
            NodeType::TaskItem => {
                if self.attrs.checked.is_none() {
                    return Err(invalid("task item requires checked state"));
                }
                if self.attrs.task_marker.as_ref().is_some_and(|marker| {
                    marker.chars().count() != 1
                        || marker
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '[' | ']'))
                }) {
                    return Err(invalid("task marker must be one printable character"));
                }
            }
            NodeType::Image => {
                let hash = self
                    .attrs
                    .blob_hash
                    .as_deref()
                    .ok_or(invalid("image requires a captured blob"))?;
                if hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(invalid("invalid image blob hash"));
                }
            }
            NodeType::NoteReference | NodeType::WikiLink | NodeType::WikiEmbed => {
                if self.kind == NodeType::NoteReference && self.attrs.target_id.is_none() {
                    return Err(invalid("missing reference target"));
                }
                if let Some(target) = &self.attrs.target_id {
                    valid_uuid(target)?;
                } else if self.attrs.target_block_id.is_some()
                    || self.attrs.target_revision_id.is_some()
                {
                    return Err(invalid("a reference fragment requires a resolved target"));
                }
                if self.kind != NodeType::NoteReference {
                    let target = self
                        .attrs
                        .wiki_target
                        .as_deref()
                        .ok_or(invalid("missing wiki target"))?;
                    if target.trim().is_empty()
                        || target.chars().count() > 2048
                        || target.chars().any(char::is_control)
                    {
                        return Err(invalid("wiki target is empty or exceeds bounds"));
                    }
                    if self.attrs.label.as_ref().is_some_and(|label| {
                        label.chars().count() > 512 || label.chars().any(char::is_control)
                    }) {
                        return Err(invalid("wiki label exceeds bounds"));
                    }
                    if [self.attrs.embed_width, self.attrs.embed_height]
                        .into_iter()
                        .flatten()
                        .any(|size| !(1..=4096).contains(&size))
                    {
                        return Err(invalid("embed dimensions must be 1 to 4096"));
                    }
                }
                let occurrence = self
                    .attrs
                    .occurrence_id
                    .as_deref()
                    .ok_or(invalid("missing reference occurrence"))?;
                valid_uuid(occurrence)?;
                if !state.occurrences.insert(occurrence.into()) {
                    return Err(invalid("duplicate reference occurrence"));
                }
                if let Some(block) = &self.attrs.target_block_id {
                    valid_uuid(block)?;
                }
                if let Some(revision) = &self.attrs.target_revision_id {
                    valid_uuid(revision)?;
                }
            }
            NodeType::Callout => {
                let kind = self
                    .attrs
                    .callout_type
                    .as_deref()
                    .ok_or(invalid("callout requires a type"))?;
                if kind.is_empty()
                    || kind.len() > 64
                    || !kind
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                {
                    return Err(invalid("callout type exceeds bounds"));
                }
                if self.attrs.title.as_ref().is_some_and(|title| {
                    title.chars().count() > 2048 || title.chars().any(char::is_control)
                }) {
                    return Err(invalid("callout title exceeds bounds"));
                }
            }
            NodeType::InlineMath
            | NodeType::BlockMath
            | NodeType::Comment
            | NodeType::CommentBlock
            | NodeType::Frontmatter => {
                let source = self
                    .attrs
                    .source
                    .as_deref()
                    .ok_or(invalid("source node requires original source"))?;
                let math = matches!(self.kind, NodeType::InlineMath | NodeType::BlockMath);
                if source.len() > 64 * 1024
                    || (math && source.trim().is_empty())
                    || source.contains('\0')
                {
                    return Err(invalid("source text is empty or exceeds bounds"));
                }
                if self.kind == NodeType::Frontmatter && depth != 1 {
                    return Err(invalid("frontmatter cannot be nested"));
                }
            }
            NodeType::FootnoteReference | NodeType::FootnoteDefinition => {
                let label = self
                    .attrs
                    .label
                    .as_deref()
                    .ok_or(invalid("footnote requires a label"))?;
                if label.trim().is_empty()
                    || label.chars().count() > 512
                    || label.chars().any(char::is_control)
                {
                    return Err(invalid("footnote label is empty or exceeds bounds"));
                }
                if self.kind == NodeType::FootnoteDefinition
                    && (!state.footnotes.insert(
                        label
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase(),
                    ) || state.footnotes.len() > 4096)
                {
                    return Err(invalid("footnote definitions must be unique and bounded"));
                }
            }
            NodeType::TableCell | NodeType::TableHeader
                if self.attrs.colspan.is_some_and(|n| n != 1)
                    || self.attrs.rowspan.is_some_and(|n| n != 1)
                    || self
                        .attrs
                        .colwidth
                        .as_ref()
                        .is_some_and(|v| v.len() != 1 || v[0] > 4096 || v[0] == 0) =>
            {
                return Err(invalid("merged or invalid table cells are not supported"));
            }
            _ => {}
        }
        if self.kind != NodeType::Text && self.text.is_some() {
            return Err(invalid("only text nodes may contain text"));
        }
        if !inline && !self.marks.is_empty() {
            return Err(invalid("marks apply only to inline nodes"));
        }
        if self.marks.len() > 1 && self.marks.iter().any(|mark| mark.kind == MarkType::Code) {
            return Err(invalid("inline code cannot combine with other marks"));
        }
        if self
            .marks
            .iter()
            .any(|mark| mark.kind == MarkType::Superscript)
            && self
                .marks
                .iter()
                .any(|mark| mark.kind == MarkType::Subscript)
        {
            return Err(invalid("superscript and subscript are mutually exclusive"));
        }
        let mut marks = BTreeSet::new();
        for mark in &self.marks {
            if !marks.insert(&mark.kind) {
                return Err(invalid("duplicate mark"));
            }
            if mark.kind == MarkType::Link {
                if mark.attrs.color.is_some() {
                    return Err(invalid("link does not accept color attributes"));
                }
                let href = mark
                    .attrs
                    .href
                    .as_deref()
                    .ok_or(invalid("link requires href"))?;
                let lower = href.to_ascii_lowercase();
                if href.len() > 8192
                    || href.chars().any(char::is_control)
                    || !(lower.starts_with("https://")
                        || lower.starts_with("http://")
                        || lower.starts_with("mailto:"))
                {
                    return Err(invalid("link scheme must be http, https or mailto"));
                }
                if mark
                    .attrs
                    .target
                    .as_deref()
                    .is_some_and(|t| !matches!(t, "_blank" | "_self"))
                    || mark.attrs.class.as_ref().is_some_and(|s| !s.is_empty())
                {
                    return Err(invalid("unsupported link presentation"));
                }
            } else if matches!(mark.kind, MarkType::Highlight | MarkType::TextStyle) {
                let mut expected = MarkAttributes::default();
                expected.color.clone_from(&mark.attrs.color);
                if mark.attrs != expected
                    || (mark.kind == MarkType::TextStyle && mark.attrs.color.is_none())
                    || mark.attrs.color.as_ref().is_some_and(|color| {
                        color.len() != 7
                            || !color.starts_with('#')
                            || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                    })
                {
                    return Err(invalid("color mark requires only a #RRGGBB color"));
                }
            } else if !mark.attrs.is_empty() {
                return Err(invalid("mark does not accept attributes"));
            }
        }
        let children_ok = self.content.iter().all(|child| match self.kind {
            NodeType::Paragraph | NodeType::Heading => child.is_inline(),
            NodeType::CodeBlock => child.kind == NodeType::Text && child.marks.is_empty(),
            NodeType::BulletList | NodeType::OrderedList => child.kind == NodeType::ListItem,
            NodeType::TaskList => child.kind == NodeType::TaskItem,
            NodeType::ListItem
            | NodeType::TaskItem
            | NodeType::Blockquote
            | NodeType::TableCell
            | NodeType::TableHeader
            | NodeType::Callout
            | NodeType::FootnoteDefinition => child.is_block(),
            NodeType::Table => child.kind == NodeType::TableRow,
            NodeType::TableRow => matches!(child.kind, NodeType::TableCell | NodeType::TableHeader),
            _ => false,
        });
        if !children_ok {
            return Err(invalid("invalid child structure"));
        }
        if matches!(
            self.kind,
            NodeType::BulletList
                | NodeType::OrderedList
                | NodeType::TaskList
                | NodeType::ListItem
                | NodeType::TaskItem
                | NodeType::Blockquote
                | NodeType::Table
                | NodeType::TableRow
                | NodeType::TableCell
                | NodeType::TableHeader
        ) && self.content.is_empty()
        {
            return Err(invalid("container must not be empty"));
        }
        if matches!(self.kind, NodeType::ListItem | NodeType::TaskItem)
            && self.content[0].kind != NodeType::Paragraph
        {
            return Err(invalid("list item must begin with a paragraph"));
        }
        if self.kind == NodeType::Table
            && self
                .content
                .iter()
                .any(|row| row.content.len() != self.content[0].content.len())
        {
            return Err(invalid("table rows must have equal width"));
        }
        for child in &self.content {
            child.validate(depth + 1, state)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn supported_document_schemas_are_exact_and_future_schema_is_rejected() {
        assert_eq!(SUPPORTED_DOCUMENT_SCHEMAS, &[1, 2, 3]);
        assert!(
            NoteDocument::from_text(uuid::Uuid::new_v4().to_string(), "正文")
                .validate(DOCUMENT_SCHEMA)
                .is_ok()
        );
        assert!(
            NoteDocument::from_text(uuid::Uuid::new_v4().to_string(), "正文")
                .validate(4)
                .is_err()
        );
    }

    #[test]
    fn bibliography_metadata_is_bounded_and_heading_only() {
        let heading = |style: Option<&str>, fingerprint: Option<&str>| NoteDocument {
            kind: DocumentType::Doc,
            content: vec![DocumentNode {
                kind: NodeType::Heading,
                attrs: NodeAttributes {
                    id: uuid::Uuid::new_v4().to_string(),
                    level: Some(2),
                    bibliography_style_id: style.map(str::to_owned),
                    bibliography_fingerprint: fingerprint.map(str::to_owned),
                    ..Default::default()
                },
                content: vec![DocumentNode {
                    kind: NodeType::Text,
                    attrs: NodeAttributes::default(),
                    content: vec![],
                    text: Some("References".into()),
                    marks: vec![],
                }],
                text: None,
                marks: vec![],
            }],
        };
        let style = Some("https://www.zotero.org/styles/apa");
        let fingerprint = Some("0123456789abcdef");
        assert!(
            heading(style, fingerprint)
                .validate(DOCUMENT_SCHEMA)
                .is_ok()
        );
        assert!(heading(style, None).validate(DOCUMENT_SCHEMA).is_err());
        assert!(
            heading(Some(""), fingerprint)
                .validate(DOCUMENT_SCHEMA)
                .is_err()
        );
        assert!(
            heading(style, Some("not-a-fingerprint"))
                .validate(DOCUMENT_SCHEMA)
                .is_err()
        );

        let mut paragraph = heading(style, fingerprint);
        paragraph.content[0].kind = NodeType::Paragraph;
        paragraph.content[0].attrs.level = None;
        assert!(paragraph.validate(DOCUMENT_SCHEMA).is_err());
    }
}
