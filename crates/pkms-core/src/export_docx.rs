//! Minimal, self-contained OOXML export for a note revision.
//!
//! The package keeps the document editable in Word and embeds the same image
//! blobs as the HTML archive. Unsupported interactive behavior is represented
//! by stable text rather than being silently discarded.
use crate::portable::archive_error;
use crate::{
    BibliographyEntry, BibliographyReport, DocumentMark, DocumentNode, Error, InTextCitation,
    MarkType, NodeType, Note, Result, TableAlignment,
};
use image::GenericImageView;
use std::{
    cell::Cell,
    collections::BTreeMap,
    io::{Cursor, Write},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const EMU_PER_PIXEL: u64 = 9_525;
const MAX_IMAGE_WIDTH_EMU: u64 = 5_943_600;

struct CitationDoc<'a> {
    in_text: &'a [InTextCitation],
    next: Cell<usize>,
    styled: bool,
    entries: &'a [BibliographyEntry],
}

impl<'a> CitationDoc<'a> {
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

struct Media {
    hash: String,
    media_type: String,
    extension: &'static str,
    bytes: Vec<u8>,
    relationship: String,
}

struct Docx<'a> {
    assets: &'a BTreeMap<String, (String, Vec<u8>)>,
    citations: Option<CitationDoc<'a>>,
    media: Vec<Media>,
    media_index: BTreeMap<String, usize>,
    drawing_id: u64,
}

pub(super) fn document(
    note: &Note,
    assets: &BTreeMap<String, (String, Vec<u8>)>,
    citations: Option<&BibliographyReport>,
) -> Result<Vec<u8>> {
    let citation_doc = citations.map(|report| CitationDoc {
        in_text: &report.in_text,
        next: Cell::new(0),
        styled: report.style_id != "plain",
        entries: &report.entries,
    });
    let mut docx = Docx {
        assets,
        citations: citation_doc,
        media: Vec::new(),
        media_index: BTreeMap::new(),
        drawing_id: 1,
    };
    for node in note.document.nodes() {
        if node.kind == NodeType::Image {
            docx.register_media(node)?;
        }
    }
    let mut body = String::new();
    body.push_str(&docx.heading(0, &run(&note.title, &[], false)));
    for node in &note.document.content {
        body.push_str(&docx.block(node, 0, None));
    }
    body.push_str(&docx.references());
    body.push_str("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr>");
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}</w:body></w:document>"
    );
    package(note, &document, &docx.media)
}

impl Docx<'_> {
    fn register_media(&mut self, node: &DocumentNode) -> Result<()> {
        let Some(hash) = node.attrs.blob_hash.as_deref() else {
            return Ok(());
        };
        if self.media_index.contains_key(hash) {
            return Ok(());
        }
        let Some((media_type, bytes)) = self.assets.get(hash) else {
            return Err(Error::IntegrityFailure);
        };
        let extension = match media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => return Err(Error::IntegrityFailure),
        };
        let relationship = format!("rId{}", self.media.len() + 2);
        self.media_index.insert(hash.to_owned(), self.media.len());
        self.media.push(Media {
            hash: hash.to_owned(),
            media_type: media_type.clone(),
            extension,
            bytes: bytes.clone(),
            relationship,
        });
        Ok(())
    }

    fn block(&mut self, node: &DocumentNode, depth: usize, marker: Option<&str>) -> String {
        match node.kind {
            NodeType::Paragraph => self.paragraph("Normal", depth, marker, &node.content, None),
            NodeType::Heading => self.heading(node.attrs.level.unwrap_or(1), &self.runs(&node.content)),
            NodeType::BulletList => self.list(node, depth, None),
            NodeType::OrderedList => self.list(node, depth, Some(node.attrs.start.unwrap_or(1))),
            NodeType::TaskList => self.list(node, depth, None),
            NodeType::ListItem => self.list_item(node, depth, marker.unwrap_or("•")),
            NodeType::TaskItem => {
                let marker = if node.attrs.checked == Some(true) { "☑" } else { "☐" };
                self.list_item(node, depth, marker)
            }
            NodeType::Blockquote => self.paragraph("Quote", depth, None, &node.content, None),
            NodeType::CodeBlock => self.code_block(node),
            NodeType::HorizontalRule => {
                "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"A0A0A0\"/></w:pBdr></w:pPr></w:p>".into()
            }
            NodeType::Image => self.image(node),
            NodeType::Table => self.table(node),
            NodeType::Callout => {
                let label = node.attrs.callout_type.as_deref().unwrap_or("Note");
                self.paragraph(
                    "Quote",
                    depth,
                    None,
                    &node.content,
                    Some(&format!("{label}: ")),
                )
            }
            NodeType::InlineMath | NodeType::BlockMath => {
                self.paragraph("Code", depth, None, &node.content, None)
            }
            NodeType::FootnoteDefinition => self.paragraph(
                "Normal",
                depth,
                None,
                &node.content,
                Some(&format!("{}: ", node.attrs.label.as_deref().unwrap_or("Footnote"))),
            ),
            NodeType::Frontmatter => self.paragraph(
                "Code",
                depth,
                None,
                &[],
                node.attrs.source.as_deref(),
            ),
            NodeType::Comment | NodeType::CommentBlock => String::new(),
            NodeType::Text
            | NodeType::HardBreak
            | NodeType::NoteReference
            | NodeType::WikiLink
            | NodeType::WikiEmbed
            | NodeType::FootnoteReference => {
                self.paragraph("Normal", depth, None, std::slice::from_ref(node), None)
            }
            NodeType::TableRow | NodeType::TableCell | NodeType::TableHeader => String::new(),
        }
    }

    fn heading(&self, level: u8, content: &str) -> String {
        let style = format!("Heading{}", level.clamp(1, 6));
        format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{content}</w:p>")
    }

    fn paragraph(
        &self,
        style: &str,
        depth: usize,
        marker: Option<&str>,
        nodes: &[DocumentNode],
        prefix: Option<&str>,
    ) -> String {
        let indent = if depth == 0 {
            String::new()
        } else {
            format!(
                "<w:ind w:left=\"{}\" w:hanging=\"360\"/>",
                720 + depth * 360
            )
        };
        let mut runs = String::new();
        if let Some(prefix) = prefix {
            runs.push_str(&run(prefix, &[], false));
        }
        if let Some(marker) = marker {
            runs.push_str(&run(&format!("{marker} "), &[], false));
        }
        runs.push_str(&self.runs(nodes));
        format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{indent}</w:pPr>{runs}</w:p>")
    }

    fn list(&mut self, node: &DocumentNode, depth: usize, ordered: Option<u32>) -> String {
        let mut body = String::new();
        for (offset, child) in node.content.iter().enumerate() {
            let marker = if let Some(start) = ordered {
                format!("{}.", start + offset as u32)
            } else if child.kind == NodeType::TaskItem {
                if child.attrs.checked == Some(true) {
                    "☑".into()
                } else {
                    "☐".into()
                }
            } else {
                "•".into()
            };
            body.push_str(&self.list_item(child, depth, &marker));
        }
        body
    }

    fn list_item(&mut self, node: &DocumentNode, depth: usize, marker: &str) -> String {
        let mut body = String::new();
        let mut used_marker = false;
        for child in &node.content {
            if matches!(
                child.kind,
                NodeType::BulletList | NodeType::OrderedList | NodeType::TaskList
            ) {
                body.push_str(&self.block(child, depth + 1, None));
            } else if child.kind == NodeType::Paragraph && !used_marker {
                body.push_str(&self.paragraph(
                    "ListParagraph",
                    depth + 1,
                    Some(marker),
                    &child.content,
                    None,
                ));
                used_marker = true;
            } else {
                let prefix = (!used_marker).then_some(marker);
                body.push_str(&self.block_with_prefix(child, depth + 1, prefix));
                used_marker = true;
            }
        }
        if !used_marker {
            body.push_str(&self.paragraph("ListParagraph", depth + 1, Some(marker), &[], None));
        }
        body
    }

    fn block_with_prefix(
        &mut self,
        node: &DocumentNode,
        depth: usize,
        marker: Option<&str>,
    ) -> String {
        match node.kind {
            NodeType::Paragraph | NodeType::Blockquote | NodeType::Callout => {
                let style = if node.kind == NodeType::Paragraph {
                    "ListParagraph"
                } else {
                    "Quote"
                };
                self.paragraph(style, depth, marker, &node.content, None)
            }
            _ => self.block(node, depth, marker),
        }
    }

    fn code_block(&self, node: &DocumentNode) -> String {
        // Code blocks normally keep their text in a child text node.
        let text = node.text.as_deref().unwrap_or("");
        let value = if text.is_empty() {
            node.content
                .iter()
                .map(|child| child.text.as_deref().unwrap_or_default())
                .collect::<String>()
        } else {
            text.to_owned()
        };
        let mut runs = String::new();
        for (index, line) in value.split('\n').enumerate() {
            if index > 0 {
                runs.push_str("<w:r><w:br/></w:r>");
            }
            if !line.is_empty() {
                runs.push_str(&run(
                    line,
                    &[DocumentMark {
                        kind: MarkType::Code,
                        attrs: Default::default(),
                    }],
                    false,
                ));
            }
        }
        format!("<w:p><w:pPr><w:pStyle w:val=\"Code\"/></w:pPr>{runs}</w:p>")
    }

    fn image(&mut self, node: &DocumentNode) -> String {
        let Some(hash) = node.attrs.blob_hash.as_deref() else {
            return String::new();
        };
        let Some(index) = self.media_index.get(hash).copied() else {
            return String::new();
        };
        let media = &self.media[index];
        let dimensions = image::load_from_memory(&media.bytes)
            .ok()
            .map(|image| image.dimensions())
            .unwrap_or((640, 360));
        let mut width = u64::from(node.attrs.embed_width.unwrap_or(dimensions.0)) * EMU_PER_PIXEL;
        let mut height = u64::from(node.attrs.embed_height.unwrap_or(dimensions.1)) * EMU_PER_PIXEL;
        if width > MAX_IMAGE_WIDTH_EMU {
            height = height.saturating_mul(MAX_IMAGE_WIDTH_EMU) / width.max(1);
            width = MAX_IMAGE_WIDTH_EMU;
        }
        let id = self.drawing_id;
        self.drawing_id += 1;
        let alt = escape(node.attrs.alt.as_deref().unwrap_or(""));
        let relationship = &media.relationship;
        format!(
            "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\" descr=\"{alt}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"image-{id}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{relationship}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
        )
    }

    fn table(&mut self, node: &DocumentNode) -> String {
        let mut rows = String::new();
        for row in &node.content {
            let mut cells = String::new();
            for cell in &row.content {
                let width = if let Some(widths) = &cell.attrs.colwidth {
                    format!(
                        "<w:tcW w:w=\"{}\" w:type=\"dxa\"/>",
                        widths.first().copied().unwrap_or(0)
                    )
                } else {
                    "<w:tcW w:w=\"0\" w:type=\"auto\"/>".into()
                };
                let span = cell.attrs.colspan.unwrap_or(1);
                let span = if span > 1 {
                    format!("<w:gridSpan w:val=\"{span}\"/>")
                } else {
                    String::new()
                };
                let shading = if cell.kind == NodeType::TableHeader {
                    "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"E8EEF5\"/>"
                } else {
                    ""
                };
                let mut content = String::new();
                if cell.content.is_empty() {
                    content.push_str("<w:p/>");
                } else {
                    for child in &cell.content {
                        content.push_str(&align_cell_paragraph(
                            self.block(child, 0, None),
                            cell.attrs.align,
                        ));
                    }
                }
                cells.push_str(&format!(
                    "<w:tc><w:tcPr>{width}{span}{shading}</w:tcPr>{content}</w:tc>"
                ));
            }
            rows.push_str(&format!("<w:tr>{cells}</w:tr>"));
        }
        format!(
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/></w:tblPr>{rows}</w:tbl>"
        )
    }

    fn runs(&self, nodes: &[DocumentNode]) -> String {
        let mut output = String::new();
        for node in nodes {
            match node.kind {
                NodeType::Text => output.push_str(
                    &self.text_runs(node.text.as_deref().unwrap_or_default(), &node.marks),
                ),
                NodeType::HardBreak => output.push_str("<w:r><w:br/></w:r>"),
                NodeType::Image => {
                    output.push_str(&escape(node.attrs.alt.as_deref().unwrap_or("[image]")))
                }
                NodeType::InlineMath => output.push_str(&run(
                    &format!("${}$", node.text.as_deref().unwrap_or_default()),
                    &node.marks,
                    false,
                )),
                NodeType::WikiLink | NodeType::WikiEmbed | NodeType::NoteReference => {
                    let label = node
                        .attrs
                        .label
                        .as_deref()
                        .or(node.attrs.wiki_target.as_deref())
                        .or(node.attrs.target_id.as_deref())
                        .unwrap_or_default();
                    output.push_str(&run(label, &node.marks, false));
                }
                NodeType::FootnoteReference => output.push_str(&run(
                    &format!("[{}]", node.attrs.label.as_deref().unwrap_or("footnote")),
                    &node.marks,
                    false,
                )),
                NodeType::Comment | NodeType::CommentBlock => {}
                _ => {
                    if !node.content.is_empty() {
                        output.push_str(&self.runs(&node.content));
                    } else if let Some(text) = node.text.as_deref() {
                        output.push_str(&run(text, &node.marks, false));
                    }
                }
            }
        }
        output
    }

    fn text_runs(&self, text: &str, marks: &[DocumentMark]) -> String {
        let groups = crate::citation::citation_group_ranges(text);
        if groups.is_empty() {
            return run(text, marks, false);
        }
        let mut output = String::new();
        let mut cursor = 0usize;
        for (range, keys) in groups {
            output.push_str(&run(&text[cursor..range.start], marks, false));
            if let Some(rendered) = self
                .citations
                .as_ref()
                .and_then(|citations| citations.rendered(&text[range.clone()], &keys))
            {
                output.push_str(&run(rendered, marks, false));
            } else if self
                .citations
                .as_ref()
                .is_some_and(|citations| citations.styled)
            {
                output.push_str(&run(&text[range.clone()], marks, true));
            } else {
                output.push_str(&run(&text[range.clone()], marks, false));
            }
            cursor = range.end;
        }
        output.push_str(&run(&text[cursor..], marks, false));
        output
    }

    fn references(&self) -> String {
        let Some(citations) = &self.citations else {
            return String::new();
        };
        if citations.entries.is_empty() {
            return String::new();
        }
        let mut body = self.heading(1, &run("参考文献 / References", &[], false));
        for entry in citations.entries {
            body.push_str(&self.paragraph("Bibliography", 0, None, &[], Some(&entry.formatted)));
        }
        body
    }
}

fn run(value: &str, marks: &[DocumentMark], missing_citation: bool) -> String {
    let mut bold = false;
    let mut italic = false;
    let mut strike = false;
    let mut underline = false;
    let mut code = false;
    let mut superscript = false;
    let mut subscript = false;
    let mut color: Option<String> = None;
    let mut highlight: Option<String> = None;
    for mark in marks {
        match mark.kind {
            MarkType::Bold => bold = true,
            MarkType::Italic => italic = true,
            MarkType::Strike => strike = true,
            MarkType::Underline | MarkType::Link => underline = true,
            MarkType::Code => code = true,
            MarkType::Superscript => superscript = true,
            MarkType::Subscript => subscript = true,
            MarkType::Highlight => highlight = mark.attrs.color.as_deref().and_then(hex_color),
            MarkType::TextStyle => color = mark.attrs.color.as_deref().and_then(hex_color),
        }
    }
    if missing_citation {
        color = Some("A34C18".into());
    }
    let mut props = String::new();
    if bold {
        props.push_str("<w:b/>");
    }
    if italic {
        props.push_str("<w:i/>");
    }
    if strike {
        props.push_str("<w:strike/>");
    }
    if underline {
        props.push_str("<w:u w:val=\"single\"/>");
    }
    if code {
        props.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/>");
    }
    if superscript {
        props.push_str("<w:vertAlign w:val=\"superscript\"/>");
    }
    if subscript {
        props.push_str("<w:vertAlign w:val=\"subscript\"/>");
    }
    if let Some(color) = color {
        props.push_str(&format!("<w:color w:val=\"{color}\"/>"));
    }
    if let Some(highlight) = highlight {
        props.push_str(&format!(
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{highlight}\"/>"
        ));
    }
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{props}</w:rPr>")
    };
    format!(
        "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        escape(value)
    )
}

fn package(note: &Note, document: &str, media: &[Media]) -> Result<Vec<u8>> {
    let mut defaults = String::from(
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    let mut seen = std::collections::BTreeSet::new();
    for item in media {
        if seen.insert(item.extension) {
            defaults.push_str(&format!(
                "<Default Extension=\"{}\" ContentType=\"{}\"/>",
                item.extension,
                escape(&item.media_type)
            ));
        }
    }
    let content_types = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">{defaults}<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>"
    );
    let root_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>";
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
    );
    for item in media {
        rels.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{}.{}\"/>",
            item.relationship, item.hash, item.extension
        ));
    }
    rels.push_str("</Relationships>");
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos" w:eastAsia="Microsoft YaHei"/><w:sz w:val="22"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="360" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="360" w:after="180"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="280" w:after="140"/></w:pPr><w:rPr><w:b/><w:sz w:val="30"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="220" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="22"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:i/><w:sz w:val="22"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/><w:pBdr><w:left w:val="single" w:sz="12" w:space="8" w:color="B8C4D0"/></w:pBdr></w:pPr><w:rPr><w:i/><w:color w:val="4B5563"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="80"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="F5F5F5"/><w:spacing w:after="100"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Bibliography"><w:name w:val="Bibliography"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720" w:hanging="720"/><w:spacing w:after="100"/></w:pPr><w:rPr><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="B8C0C8"/><w:left w:val="single" w:sz="4" w:color="B8C0C8"/><w:bottom w:val="single" w:sz="4" w:color="B8C0C8"/><w:right w:val="single" w:sz="4" w:color="B8C0C8"/><w:insideH w:val="single" w:sz="4" w:color="D8DEE5"/><w:insideV w:val="single" w:sz="4" w:color="D8DEE5"/></w:tblBorders><w:tblCellMar><w:top w:w="80" w:type="dxa"/><w:left w:w="100" w:type="dxa"/><w:bottom w:w="80" w:type="dxa"/><w:right w:w="100" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style></w:styles>"#;
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>spanseek</dc:creator></cp:coreProperties>",
        escape(&note.title)
    );
    let app = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>spanseek</Application></Properties>";
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o600);
        for (name, bytes) in [
            ("[Content_Types].xml", content_types.as_bytes()),
            ("_rels/.rels", root_rels.as_bytes()),
            ("word/document.xml", document.as_bytes()),
            ("word/styles.xml", styles.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("docProps/core.xml", core.as_bytes()),
            ("docProps/app.xml", app.as_bytes()),
        ] {
            zip.start_file(name, options).map_err(archive_error)?;
            zip.write_all(bytes)?;
        }
        for item in media {
            zip.start_file(
                format!("word/media/{}.{}", item.hash, item.extension),
                options,
            )
            .map_err(archive_error)?;
            zip.write_all(&item.bytes)?;
        }
        zip.finish().map_err(archive_error)?;
    }
    Ok(cursor.into_inner())
}

fn escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            '\t' | '\n' | '\r' => output.push(character),
            character if character >= ' ' => output.push(character),
            _ => {}
        }
    }
    output
}

fn hex_color(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('#');
    (value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_uppercase())
}

fn align_cell_paragraph(markup: String, align: Option<TableAlignment>) -> String {
    let Some(align) = align else {
        return markup;
    };
    if markup.contains("<w:drawing>") {
        return markup;
    }
    let alignment = match align {
        TableAlignment::Left => "left",
        TableAlignment::Center => "center",
        TableAlignment::Right => "right",
    };
    let property = format!("<w:jc w:val=\"{alignment}\"/>");
    if let Some(index) = markup.find("<w:pPr>") {
        let insert = index + "<w:pPr>".len();
        let mut output = markup;
        output.insert_str(insert, &property);
        output
    } else if let Some(rest) = markup.strip_prefix("<w:p>") {
        format!("<w:p><w:pPr>{property}</w:pPr>{rest}")
    } else {
        markup
    }
}
