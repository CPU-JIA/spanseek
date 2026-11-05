//! Bounded bibliography support: BibTeX parsing, CSL-JSON projection and
//! `[@key]` citation resolution.
//!
//! References live as ordinary notes whose frontmatter carries `citation_key`,
//! so a citation can be resolved back to a note (and therefore to its source)
//! and survives every existing export path. Depth is deliberately bounded:
//! spanseek inserts citations as text markers instead of owning a citation
//! editor, which keeps the writing surface unchanged.

use crate::{
    CreateNote, DOCUMENT_SCHEMA, DocumentNode, DocumentType, Error, Library, NodeAttributes,
    NodeType, Note, NoteDocument, Result, SaveNote,
};
use citationberg::IndependentStyle;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use ts_rs::TS;

const MAX_BIBTEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibEntry {
    pub key: String,
    pub entry_type: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<String>,
    pub container: Option<String>,
    pub doi: Option<String>,
    pub url: Option<String>,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BibImportReport {
    pub entries: u32,
    pub imported: u32,
    pub skipped_existing: u32,
    pub losses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BibliographyEntry {
    pub key: String,
    pub title: String,
    pub note_id: String,
    pub formatted: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct InTextCitation {
    pub keys: Vec<String>,
    pub rendered: String,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub note_number: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BibliographyReport {
    pub keys: Vec<String>,
    pub entries: Vec<BibliographyEntry>,
    pub missing: Vec<String>,
    pub markdown: String,
    pub style: CitationStyle,
    #[serde(default = "plain_style_id")]
    pub style_id: String,
    #[serde(default)]
    pub in_text: Vec<InTextCitation>,
}

fn plain_style_id() -> String {
    "plain".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CitationStyleKind {
    Builtin,
    Independent,
    Dependent,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CitationStyleSummary {
    pub id: String,
    pub title: String,
    pub kind: CitationStyleKind,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CitationStyleImportReport {
    pub style: CitationStyleSummary,
}

/// Citation rendering style. `Plain` keeps the historical one-line format;
/// the others cover the styles most used in Chinese and English writing. A
/// full CSL engine is tracked separately in the parity plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CitationStyle {
    /// The historical one-line format, which already follows GB/T 7714
    /// ordering (author. title[type]. container, year. DOI.). It is kept
    /// byte-for-byte so existing bibliographies do not change under the user.
    #[default]
    Plain,
    /// CSL "American Psychological Association 7th edition".
    Apa,
    /// CSL "Chicago Manual of Style 17th edition (author-date)".
    ChicagoAuthorDate,
    /// CSL "China National Standard GB/T 7714-2015 (numeric)".
    Gbt7714,
    /// A user-imported independent or dependent CSL style.
    Custom,
}

impl CitationStyle {
    pub fn id(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Apa => "apa",
            Self::ChicagoAuthorDate => "chicago_author_date",
            Self::Gbt7714 => "gbt7714",
            Self::Custom => "custom",
        }
    }
}

struct Scanner {
    chars: Vec<char>,
    index: usize,
}

impl Scanner {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            index: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let value = self.peek();
        if value.is_some() {
            self.index += 1;
        }
        value
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.index += 1;
        }
    }

    fn read_until(&mut self, stop: &[char]) -> String {
        let mut value = String::new();
        while let Some(next) = self.peek() {
            if stop.contains(&next) {
                break;
            }
            value.push(next);
            self.index += 1;
        }
        value
    }

    fn read_braced(&mut self) -> Option<String> {
        if self.peek() != Some('{') {
            return None;
        }
        self.index += 1;
        let mut depth = 1usize;
        let mut value = String::new();
        while let Some(next) = self.bump() {
            match next {
                '{' => {
                    depth += 1;
                    value.push(next);
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(value);
                    }
                    value.push(next);
                }
                _ => value.push(next),
            }
        }
        None
    }

    fn read_quoted(&mut self) -> Option<String> {
        if self.peek() != Some('"') {
            return None;
        }
        self.index += 1;
        let mut value = String::new();
        while let Some(next) = self.bump() {
            if next == '"' {
                return Some(value);
            }
            value.push(next);
        }
        None
    }
}

/// Parses a BibTeX file. Malformed entries are reported instead of aborting
/// the whole import, and `@string`/`@comment`/`@preamble` blocks are skipped.
pub fn parse_bibtex(text: &str) -> (Vec<BibEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut losses = Vec::new();
    let mut scanner = Scanner::new(text);
    while let Some(next) = scanner.bump() {
        if next != '@' {
            continue;
        }
        scanner.skip_whitespace();
        let entry_type = scanner.read_until(&['{', '(', ' ', '\t', '\n', '\r']);
        let entry_type = entry_type.to_lowercase();
        scanner.skip_whitespace();
        let Some(open) = scanner.peek() else { break };
        if open != '{' && open != '(' {
            continue;
        }
        let close = if open == '{' { '}' } else { ')' };
        scanner.index += 1;
        if matches!(entry_type.as_str(), "comment" | "preamble" | "string") {
            let mut depth = 1usize;
            while let Some(ch) = scanner.bump() {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            continue;
        }
        let key = scanner.read_until(&[',', close]).trim().to_owned();
        let mut fields = BTreeMap::new();
        loop {
            while scanner
                .peek()
                .is_some_and(|ch| ch == ',' || ch.is_whitespace())
            {
                scanner.index += 1;
            }
            match scanner.peek() {
                None => break,
                Some(ch) if ch == close => {
                    scanner.index += 1;
                    break;
                }
                _ => {}
            }
            let name = scanner.read_until(&['=', ',', close]).trim().to_lowercase();
            if name.is_empty() {
                break;
            }
            if scanner.peek() != Some('=') {
                continue;
            }
            scanner.index += 1;
            scanner.skip_whitespace();
            let value = match scanner.peek() {
                Some('{') => scanner.read_braced(),
                Some('"') => scanner.read_quoted(),
                _ => Some(scanner.read_until(&[',', close])),
            };
            let Some(value) = value else {
                losses.push(format!("条目 {key} 的字段 {name} 未闭合，已跳过该字段。"));
                break;
            };
            fields.insert(name, value.trim().to_owned());
        }
        if key.is_empty() {
            losses.push(format!("忽略缺少引用键的 @{entry_type} 条目。"));
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            losses.push("BibTeX 条目超过 10000 条，其余条目未导入。".to_owned());
            break;
        }
        entries.push(entry_from(key, entry_type, fields));
    }
    (entries, losses)
}

fn entry_type_from_csl(value: &str) -> String {
    match value {
        "article-journal" | "article" => "article",
        "book" => "book",
        "paper-conference" | "chapter" => "inproceedings",
        "thesis" => "phdthesis",
        "report" => "techreport",
        "webpage" => "misc",
        _ => "misc",
    }
    .to_owned()
}

/// Reads CSL-JSON so a bibliography coming from Zotero or another CSL tool can
/// enter the same reference-note pipeline as BibTeX.
pub fn parse_csl_json(text: &str) -> (Vec<BibEntry>, Vec<String>) {
    let mut losses = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return (Vec::new(), vec!["CSL-JSON 无法解析。".to_owned()]),
    };
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut object) if object.contains_key("items") => {
            match object.remove("items") {
                Some(serde_json::Value::Array(items)) => items,
                _ => return (Vec::new(), vec!["CSL-JSON 的 items 不是数组。".to_owned()]),
            }
        }
        _ => return (Vec::new(), vec!["CSL-JSON 需要是条目数组。".to_owned()]),
    };
    let mut entries = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            losses.push("忽略非对象的 CSL 条目。".to_owned());
            continue;
        };
        let key = object
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .unwrap_or_default();
        if key.is_empty() {
            losses.push("忽略缺少 id 的 CSL 条目。".to_owned());
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            losses.push("CSL 条目超过 10000 条，其余条目未导入。".to_owned());
            break;
        }
        let entry_type = object
            .get("type")
            .and_then(|value| value.as_str())
            .map(entry_type_from_csl)
            .unwrap_or_else(|| "misc".to_owned());
        let title = object
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let authors = object
            .get("author")
            .and_then(|value| value.as_array())
            .map(|authors| {
                authors
                    .iter()
                    .filter_map(|author| {
                        let family = author.get("family").and_then(|value| value.as_str());
                        let given = author.get("given").and_then(|value| value.as_str());
                        match (family, given) {
                            (Some(family), Some(given)) => Some(format!("{family}, {given}")),
                            (Some(family), None) => Some(family.to_owned()),
                            _ => author
                                .get("literal")
                                .and_then(|value| value.as_str())
                                .map(str::to_owned),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let year = object
            .get("issued")
            .and_then(|value| value.get("date-parts"))
            .and_then(|value| value.get(0))
            .and_then(|value| value.get(0))
            .and_then(|value| value.as_i64())
            .map(|year| year.to_string());
        let container = object
            .get("container-title")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let doi = object
            .get("DOI")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let url = object
            .get("URL")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let mut fields = BTreeMap::new();
        if let Some(container) = container.as_deref() {
            fields.insert("container-title".to_owned(), container.to_owned());
        }
        entries.push(BibEntry {
            key,
            entry_type,
            title,
            authors,
            year,
            container,
            doi,
            url,
            fields,
        });
    }
    (entries, losses)
}

fn clean(value: &str) -> String {
    let mut output = String::new();
    let mut last_space = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !last_space {
                output.push(' ');
            }
            last_space = true;
            continue;
        }
        output.push(ch);
        last_space = false;
    }
    output.trim().trim_matches(['{', '}']).to_owned()
}

fn entry_from(key: String, entry_type: String, fields: BTreeMap<String, String>) -> BibEntry {
    let title = fields
        .get("title")
        .map(|value| clean(value))
        .unwrap_or_default();
    let authors = fields
        .get("author")
        .map(|value| {
            value
                .split(" and ")
                .map(clean)
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let year = fields
        .get("year")
        .map(|value| clean(value))
        .or_else(|| {
            fields
                .get("date")
                .and_then(|value| value.get(..4))
                .map(str::to_owned)
        })
        .filter(|value| !value.is_empty());
    let container = ["journal", "booktitle", "publisher", "school", "institution"]
        .iter()
        .find_map(|field| fields.get(*field).map(|value| clean(value)))
        .filter(|value| !value.is_empty());
    let doi = fields
        .get("doi")
        .map(|value| clean(value))
        .filter(|value| !value.is_empty());
    let url = fields
        .get("url")
        .map(|value| clean(value))
        .filter(|value| !value.is_empty());
    BibEntry {
        key,
        entry_type,
        title,
        authors,
        year,
        container,
        doi,
        url,
        fields,
    }
}
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn csl_type(entry_type: &str) -> &'static str {
    match entry_type {
        "article" => "article-journal",
        "book" => "book",
        "inproceedings" | "conference" => "paper-conference",
        "phdthesis" | "mastersthesis" => "thesis",
        "techreport" => "report",
        "incollection" => "chapter",
        "online" | "misc" => "webpage",
        _ => "document",
    }
}

fn type_marker(entry_type: &str) -> &'static str {
    match entry_type {
        "article" => "[J]",
        "book" => "[M]",
        "inproceedings" | "conference" => "[C]",
        "phdthesis" | "mastersthesis" => "[D]",
        "techreport" => "[R]",
        "online" | "misc" => "[EB/OL]",
        _ => "[Z]",
    }
}

fn split_name(name: &str) -> (String, Option<String>) {
    match name.split_once(',') {
        Some((family, given)) => (
            family.trim().to_owned(),
            Some(given.trim().to_owned()).filter(|value| !value.is_empty()),
        ),
        None => match name.split_once(' ') {
            Some((given, family)) if !family.trim().is_empty() => (
                family.trim().to_owned(),
                Some(given.trim().to_owned()).filter(|value| !value.is_empty()),
            ),
            _ => (name.trim().to_owned(), None),
        },
    }
}

/// Projects the parsed entries into CSL-JSON so they can be consumed by
/// citation processors without spanseek owning a CSL stylesheet.
pub fn csl_json(entries: &[BibEntry]) -> serde_json::Value {
    let items = entries
        .iter()
        .map(|entry| {
            let mut item = serde_json::Map::new();
            item.insert("id".into(), entry.key.clone().into());
            item.insert("type".into(), csl_type(&entry.entry_type).into());
            item.insert("title".into(), entry.title.clone().into());
            if !entry.authors.is_empty() {
                let authors = entry
                    .authors
                    .iter()
                    .map(|name| {
                        let (family, given) = split_name(name);
                        let mut author = serde_json::Map::new();
                        author.insert("family".into(), family.into());
                        if let Some(given) = given {
                            author.insert("given".into(), given.into());
                        }
                        serde_json::Value::Object(author)
                    })
                    .collect::<Vec<_>>();
                item.insert("author".into(), authors.into());
            }
            if let Some(year) = entry
                .year
                .as_deref()
                .and_then(|year| year.parse::<i32>().ok())
            {
                item.insert(
                    "issued".into(),
                    serde_json::json!({ "date-parts": [[year]] }),
                );
            }
            if let Some(container) = &entry.container {
                item.insert("container-title".into(), container.clone().into());
            }
            if let Some(doi) = &entry.doi {
                item.insert("DOI".into(), doi.clone().into());
            }
            if let Some(url) = &entry.url {
                item.insert("URL".into(), url.clone().into());
            }
            serde_json::Value::Object(item)
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(items)
}

/// Renders a bounded bibliographic string. This is deliberately a plain,
/// deterministic format instead of a full CSL style engine.
pub fn format_reference(entry: &BibEntry) -> String {
    let mut parts = Vec::new();
    if !entry.authors.is_empty() {
        parts.push(format!("{}.", entry.authors.join(", ")));
    }
    let title = if entry.title.is_empty() {
        entry.key.clone()
    } else {
        entry.title.clone()
    };
    parts.push(format!("{}{}.", title, type_marker(&entry.entry_type)));
    if let Some(container) = &entry.container {
        parts.push(match &entry.year {
            Some(year) => format!("{container}, {year}."),
            None => format!("{container}."),
        });
    } else if let Some(year) = &entry.year {
        parts.push(format!("{year}."));
    }
    if let Some(doi) = &entry.doi {
        parts.push(format!("DOI: {doi}."));
    }
    if let Some(url) = &entry.url {
        parts.push(format!("URL: {url}"));
    }
    parts.join(" ")
}

/// Renders one entry through the bundled CSL styles. `None` means the style has
/// no bibliography output for this entry; the caller then falls back.
pub fn csl_render(entry: &BibEntry, style: CitationStyle) -> Option<String> {
    use hayagriva::archive::{ArchivedStyle, locales};
    use hayagriva::types::{Date, EntryType, Person};
    use hayagriva::{
        BibliographyDriver, BibliographyRequest, CitationItem, CitationRequest, Entry,
    };

    let archived = match style {
        CitationStyle::Apa => ArchivedStyle::AmericanPsychologicalAssociation,
        CitationStyle::ChicagoAuthorDate => ArchivedStyle::ChicagoAuthorDate,
        CitationStyle::Gbt7714 => ArchivedStyle::Gb77142015Numeric,
        CitationStyle::Plain | CitationStyle::Custom => return None,
    };
    let style = match archived.get() {
        citationberg::Style::Independent(style) => style,
        _ => return None,
    };
    let entry_type = match entry.entry_type.as_str() {
        "article" | "inproceedings" | "conference" | "paper-conference" | "incollection" => {
            EntryType::Article
        }
        "book" | "monograph" => EntryType::Book,
        "webpage" | "online" | "electronic" => EntryType::Web,
        _ => EntryType::Misc,
    };
    let mut item = Entry::new(&entry.key, entry_type);
    let title = if entry.title.is_empty() {
        entry.key.clone()
    } else {
        entry.title.clone()
    };
    item.set_title(title.into());
    let authors: Vec<Person> = entry
        .authors
        .iter()
        .filter_map(|author| {
            let parts: Vec<&str> = author.split(',').map(str::trim).collect();
            if parts.len() >= 2 {
                Person::from_strings(vec![parts[0], parts[1]]).ok()
            } else {
                Person::from_strings(vec![parts[0]]).ok()
            }
        })
        .collect();
    if !authors.is_empty() {
        item.set_authors(authors);
    }
    if let Some(year) = entry
        .year
        .as_deref()
        .and_then(|value| value.parse::<i32>().ok())
    {
        item.set_date(Date::from_year(year));
    }
    if let Some(container) = &entry.container {
        let mut parent = Entry::new("container", EntryType::Periodical);
        parent.set_title(container.clone().into());
        item.set_parents(vec![parent]);
    }
    if let Some(doi) = &entry.doi {
        item.set_doi(doi.clone());
    }
    if let Some(url) = entry
        .url
        .as_deref()
        .and_then(|value| value.parse::<hayagriva::types::QualifiedUrl>().ok())
    {
        item.set_url(url);
    }
    let locale_files = locales();
    let mut driver = BibliographyDriver::new();
    driver.citation(CitationRequest::from_items(
        vec![CitationItem::new(&item, None, None, false, None)],
        &style,
        &locale_files,
    ));
    let rendered = driver
        .finish(BibliographyRequest::new(&style, None, &locale_files))
        .bibliography?;
    let mut line = String::new();
    for item in &rendered.items {
        let _ = item
            .content
            .write_buf(&mut line, hayagriva::BufWriteFormat::Plain);
        line.push(' ');
    }
    let line = line.trim().to_owned();
    (!line.is_empty()).then_some(line)
}

/// Renders the same entry in one of the supported styles.
pub fn format_reference_styled(entry: &BibEntry, style: CitationStyle) -> String {
    if style == CitationStyle::Plain {
        return format_reference(entry);
    }
    if let Some(line) = csl_render(entry, style) {
        return line;
    }
    let title = if entry.title.is_empty() {
        entry.key.clone()
    } else {
        entry.title.clone()
    };
    let authors = entry.authors.join(", ");
    let year = entry.year.clone().unwrap_or_else(|| "n.d.".to_owned());
    let mut parts: Vec<String> = Vec::new();
    match style {
        CitationStyle::Apa => {
            if !authors.is_empty() {
                parts.push(format!("{authors} ({year})."));
            }
            parts.push(format!("{title}."));
            if let Some(container) = &entry.container {
                parts.push(format!("{container}."));
            }
        }

        CitationStyle::ChicagoAuthorDate => {
            if !authors.is_empty() {
                parts.push(format!("{authors}."));
            }
            parts.push(format!("\"{title}.\""));
            match &entry.container {
                Some(container) => parts.push(format!("{container} ({year}).")),
                None => parts.push(format!("({year}).")),
            }
        }
        CitationStyle::Plain | CitationStyle::Gbt7714 | CitationStyle::Custom => {
            return format_reference(entry);
        }
    }
    if style == CitationStyle::Apa {
        if let Some(doi) = &entry.doi {
            parts.push(format!("https://doi.org/{doi}"));
        } else if let Some(url) = &entry.url {
            parts.push(url.clone());
        }
    } else {
        if let Some(doi) = &entry.doi {
            parts.push(format!("DOI: {doi}."));
        }
        if let Some(url) = &entry.url {
            parts.push(format!("URL: {url}"));
        }
    }
    parts.join(" ")
}

const MAX_CSL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CITATION_GROUPS: usize = 5_000;
const MAX_CITATION_ITEMS: usize = 20_000;

struct CslReference {
    key: String,
    entry: BibEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CitationLocator {
    label: citationberg::taxonomy::Locator,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CitationSpec {
    key: String,
    locator: Option<CitationLocator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CitationGroup {
    marker: String,
    keys: Vec<String>,
    specs: Vec<CitationSpec>,
}

struct BatchCslRender {
    bibliography: BTreeMap<String, String>,
    citations: Vec<InTextCitation>,
}

fn built_in_citation_styles() -> Vec<CitationStyleSummary> {
    [
        ("plain", "Default (plain, GB/T order)"),
        ("apa", "APA 7th edition"),
        (
            "chicago_author_date",
            "Chicago Manual of Style 17th edition (author-date)",
        ),
        ("gbt7714", "GB/T 7714-2015 (numeric)"),
    ]
    .into_iter()
    .map(|(id, title)| CitationStyleSummary {
        id: id.to_owned(),
        title: title.to_owned(),
        kind: CitationStyleKind::Builtin,
        parent_id: None,
    })
    .collect()
}

fn citation_style_from_id(id: &str) -> Option<CitationStyle> {
    match id {
        "plain" => Some(CitationStyle::Plain),
        "apa" => Some(CitationStyle::Apa),
        "chicago_author_date" => Some(CitationStyle::ChicagoAuthorDate),
        "gbt7714" => Some(CitationStyle::Gbt7714),
        _ => None,
    }
}

fn archived_style_for_id(id: &str) -> Option<hayagriva::archive::ArchivedStyle> {
    use hayagriva::archive::ArchivedStyle;
    match id {
        "apa" => Some(ArchivedStyle::AmericanPsychologicalAssociation),
        "chicago_author_date" => Some(ArchivedStyle::ChicagoAuthorDate),
        "gbt7714" => Some(ArchivedStyle::Gb77142015Numeric),
        _ => ArchivedStyle::by_id(id).or_else(|| ArchivedStyle::by_name(id)),
    }
}

fn normalized_style_text(value: &str, max: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn parse_csl_style(xml: &str) -> Result<(String, String, CitationStyleKind, Option<String>)> {
    use citationberg::Style;
    let parsed = Style::from_xml(xml).map_err(|_| Error::InvalidInput("csl_invalid_style"))?;
    let id = parsed.info().id.trim().to_owned();
    let title = normalized_style_text(&parsed.info().title.value, 300);
    if id.is_empty() || id.chars().count() > 512 {
        return Err(Error::InvalidInput("csl_invalid_identity"));
    }
    let (kind, parent_id) = match parsed {
        Style::Independent(_) => (CitationStyleKind::Independent, None),
        Style::Dependent(style) => {
            let parent = style.parent_link.href.trim().to_owned();
            if parent.is_empty() || parent.chars().count() > 512 {
                return Err(Error::InvalidInput("csl_invalid_parent"));
            }
            (CitationStyleKind::Dependent, Some(parent))
        }
    };
    Ok((
        id,
        if title.is_empty() {
            "Imported CSL style".to_owned()
        } else {
            title
        },
        kind,
        parent_id,
    ))
}

fn resolved_archived_style(style_id: &str) -> Result<Option<IndependentStyle>> {
    use citationberg::Style;
    let Some(archived) = archived_style_for_id(style_id) else {
        return Ok(None);
    };
    let mut style = archived.get();
    let mut depth = 0usize;
    let mut seen = BTreeSet::new();
    loop {
        match style {
            Style::Independent(style) => return Ok(Some(style)),
            Style::Dependent(dependent) => {
                if depth >= 16 || !seen.insert(dependent.parent_link.href.clone()) {
                    return Err(Error::InvalidInput("csl_parent_cycle"));
                }
                let Some(parent) = archived_style_for_id(&dependent.parent_link.href) else {
                    return Err(Error::InvalidInput("csl_parent_missing"));
                };
                style = parent.get();
                depth += 1;
            }
        }
    }
}

fn resolve_citation_style(db: &Connection, style_id: &str) -> Result<IndependentStyle> {
    use citationberg::Style;
    if let Some(style) = resolved_archived_style(style_id)? {
        return Ok(style);
    }
    let mut current = style_id.to_owned();
    let mut seen = BTreeSet::new();
    for _ in 0..17 {
        if !seen.insert(current.clone()) {
            return Err(Error::InvalidInput("csl_parent_cycle"));
        }
        let xml: Option<String> = db
            .query_row(
                "SELECT xml FROM citation_styles WHERE id=?1",
                [&current],
                |row| row.get(0),
            )
            .optional()?;
        let Some(xml) = xml else {
            if let Some(style) = resolved_archived_style(&current)? {
                return Ok(style);
            }
            return Err(Error::InvalidInput("csl_parent_missing"));
        };
        let parsed = Style::from_xml(&xml).map_err(|_| Error::InvalidInput("csl_invalid_style"))?;
        match parsed {
            Style::Independent(style) => return Ok(style),
            Style::Dependent(style) => current = style.parent_link.href.trim().to_owned(),
        }
    }
    Err(Error::InvalidInput("csl_parent_limit"))
}

fn locator_label(value: &str) -> Option<(citationberg::taxonomy::Locator, usize)> {
    use citationberg::taxonomy::Locator as L;
    const LABELS: &[(&str, L)] = &[
        ("article-locator", L::ArticleLocator),
        ("title-locator", L::TitleLocator),
        ("sub verbo", L::SubVerbo),
        ("paragraphs", L::Paragraph),
        ("paragraph", L::Paragraph),
        ("sections", L::Section),
        ("section", L::Section),
        ("chapters", L::Chapter),
        ("chapter", L::Chapter),
        ("volumes", L::Volume),
        ("volume", L::Volume),
        ("figures", L::Figure),
        ("figure", L::Figure),
        ("columns", L::Column),
        ("column", L::Column),
        ("notes", L::Note),
        ("note", L::Note),
        ("lines", L::Line),
        ("line", L::Line),
        ("verses", L::Verse),
        ("verse", L::Verse),
        ("books", L::Book),
        ("book", L::Book),
        ("appendix", L::Appendix),
        ("supplement", L::Supplement),
        ("equation", L::Equation),
        ("timestamp", L::Timestamp),
        ("elocation", L::Elocation),
        ("table", L::Table),
        ("issue", L::Issue),
        ("title", L::TitleLocator),
        ("part", L::Part),
        ("act", L::Act),
        ("scene", L::Scene),
        ("canon", L::Canon),
        ("rule", L::Rule),
        ("opus", L::Opus),
        ("folio", L::Folio),
        ("pages", L::Page),
        ("page", L::Page),
        ("pp.", L::Page),
        ("p.", L::Page),
        ("chap.", L::Chapter),
        ("ch.", L::Chapter),
        ("sec.", L::Section),
        ("para.", L::Paragraph),
        ("vol.", L::Volume),
        ("fig.", L::Figure),
        ("n.", L::Note),
        ("l.", L::Line),
        ("bk.", L::Book),
        ("col.", L::Column),
        ("v.", L::Verse),
        ("s.v.", L::SubVerbo),
        ("tab.", L::Table),
        ("pt.", L::Part),
        ("sc.", L::Scene),
        ("can.", L::Canon),
        ("ts.", L::Timestamp),
        ("op.", L::Opus),
        ("fol.", L::Folio),
        ("eq.", L::Equation),
        ("eloc.", L::Elocation),
        ("supp.", L::Supplement),
        ("art.", L::ArticleLocator),
        ("app.", L::Appendix),
    ];
    let lower = value.to_ascii_lowercase();
    for (prefix, label) in LABELS {
        if !lower.starts_with(prefix) {
            continue;
        }
        let boundary = prefix.ends_with('.')
            || lower.len() == prefix.len()
            || lower.as_bytes()[prefix.len()].is_ascii_whitespace()
            || lower.as_bytes()[prefix.len()] == b':';
        if !boundary {
            continue;
        }
        let tail = &value[prefix.len()..];
        let trimmed = tail.trim_start_matches(|c: char| c.is_whitespace() || c == ':' || c == '.');
        if !trimmed.is_empty() {
            return Some((*label, value.len() - trimmed.len()));
        }
    }
    None
}

fn parse_locator(value: &str) -> Option<CitationLocator> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((label, consumed)) = locator_label(value) {
        let remainder = value[consumed..]
            .trim_start_matches(|c: char| c.is_whitespace() || c == ':' || c == '.')
            .trim();
        if !remainder.is_empty() {
            return Some(CitationLocator {
                label,
                value: remainder.to_owned(),
            });
        }
    }
    if value
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | '–' | '—' | ',' | '&' | ' '))
    {
        return Some(CitationLocator {
            label: citationberg::taxonomy::Locator::Page,
            value: value.to_owned(),
        });
    }
    Some(CitationLocator {
        label: citationberg::taxonomy::Locator::Custom,
        value: value.to_owned(),
    })
}

fn parse_citation_spec(part: &str) -> Option<CitationSpec> {
    let part = part.trim().strip_prefix('@')?;
    let split_at = part
        .find(|c: char| c.is_whitespace() || c == ',')
        .unwrap_or(part.len());
    let key = part[..split_at].trim();
    if key.is_empty() || key.chars().count() > 200 {
        return None;
    }
    let tail = part[split_at..].trim_start();
    let locator_text = tail.strip_prefix(',').map(str::trim).unwrap_or(tail);
    Some(CitationSpec {
        key: key.to_owned(),
        locator: parse_locator(locator_text),
    })
}

fn citation_occurrences(text: &str) -> Vec<(std::ops::Range<usize>, CitationGroup)> {
    let mut groups: Vec<(std::ops::Range<usize>, CitationGroup)> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('[') {
        let start = cursor + relative;
        let after = &text[start + 1..];
        let Some(end) = after.find(']') else {
            break;
        };
        let inner = &after[..end];
        if inner.contains('@') {
            let mut keys = Vec::new();
            let mut specs = Vec::new();
            for part in inner.split(';') {
                let Some(spec) = parse_citation_spec(part) else {
                    continue;
                };
                if keys.iter().any(|existing| existing == &spec.key) {
                    continue;
                }
                keys.push(spec.key.clone());
                specs.push(spec);
                if keys.len()
                    + groups
                        .iter()
                        .map(|(_, group)| group.keys.len())
                        .sum::<usize>()
                    > MAX_CITATION_ITEMS
                {
                    return groups;
                }
            }
            if !keys.is_empty() {
                let range = start..start + end + 2;
                groups.push((
                    range.clone(),
                    CitationGroup {
                        marker: text[range].to_owned(),
                        keys,
                        specs,
                    },
                ));
                if groups.len() >= MAX_CITATION_GROUPS {
                    return groups;
                }
            }
        }
        cursor = start + end + 2;
    }
    groups
}

pub(crate) fn citation_group_ranges(text: &str) -> Vec<(std::ops::Range<usize>, Vec<String>)> {
    citation_occurrences(text)
        .into_iter()
        .map(|(range, group)| (range, group.keys))
        .collect()
}
pub fn citation_groups(text: &str) -> Vec<Vec<String>> {
    citation_group_ranges(text)
        .into_iter()
        .map(|(_, keys)| keys)
        .collect()
}

fn hayagriva_entry(entry: &BibEntry) -> hayagriva::Entry {
    use hayagriva::types::{Date, EntryType, Person};
    let entry_type = match entry.entry_type.as_str() {
        "article" | "inproceedings" | "conference" | "paper-conference" | "incollection" => {
            EntryType::Article
        }
        "book" | "monograph" => EntryType::Book,
        "webpage" | "online" | "electronic" => EntryType::Web,
        _ => EntryType::Misc,
    };
    let mut item = hayagriva::Entry::new(&entry.key, entry_type);
    item.set_title(
        if entry.title.is_empty() {
            entry.key.clone()
        } else {
            entry.title.clone()
        }
        .into(),
    );
    let authors: Vec<Person> = entry
        .authors
        .iter()
        .filter_map(|author| {
            let parts: Vec<&str> = author.split(',').map(str::trim).collect();
            if parts.len() >= 2 {
                Person::from_strings(vec![parts[0], parts[1]]).ok()
            } else {
                Person::from_strings(vec![parts[0]]).ok()
            }
        })
        .collect();
    if !authors.is_empty() {
        item.set_authors(authors);
    }
    if let Some(year) = entry
        .year
        .as_deref()
        .and_then(|value| value.parse::<i32>().ok())
    {
        item.set_date(Date::from_year(year));
    }
    if let Some(container) = &entry.container {
        let mut parent = hayagriva::Entry::new("container", EntryType::Periodical);
        parent.set_title(container.clone().into());
        item.set_parents(vec![parent]);
    }
    if let Some(doi) = &entry.doi {
        item.set_doi(doi.clone());
    }
    if let Some(url) = entry
        .url
        .as_deref()
        .and_then(|value| value.parse::<hayagriva::types::QualifiedUrl>().ok())
    {
        item.set_url(url);
    }
    item
}

fn render_csl_batch(
    references: &[CslReference],
    groups: &[(std::ops::Range<usize>, CitationGroup)],
    style: &IndependentStyle,
) -> BatchCslRender {
    use citationberg::StyleClass;
    use hayagriva::archive::locales;
    use hayagriva::{
        BibliographyDriver, BibliographyRequest, BufWriteFormat, CitationItem, CitationRequest,
        LocatorPayload, SpecificLocator,
    };
    let entries: Vec<hayagriva::Entry> = references
        .iter()
        .map(|reference| hayagriva_entry(&reference.entry))
        .collect();
    let index: BTreeMap<&str, usize> = references
        .iter()
        .enumerate()
        .map(|(index, reference)| (reference.key.as_str(), index))
        .collect();
    let locale_files = locales();
    let note_style = style.settings.class == StyleClass::Note;
    let mut driver = BibliographyDriver::new();
    let mut called_groups = Vec::new();
    let mut rendered_note_count = 0usize;
    for (_, group) in groups {
        let items: Vec<CitationItem<'_, hayagriva::Entry>> = group
            .specs
            .iter()
            .filter_map(|spec| {
                let index = index.get(spec.key.as_str()).copied()?;
                let locator = spec.locator.as_ref().map(|locator| {
                    SpecificLocator(locator.label, LocatorPayload::Str(locator.value.as_str()))
                });
                Some(CitationItem::new(
                    &entries[index],
                    locator,
                    None,
                    false,
                    None,
                ))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        let note_number = if note_style {
            rendered_note_count += 1;
            Some(rendered_note_count)
        } else {
            None
        };
        driver.citation(CitationRequest::new(
            items,
            style,
            None,
            &locale_files,
            note_number,
        ));
        called_groups.push((
            group.marker.clone(),
            group.keys.clone(),
            note_number.map(|value| value as u32),
        ));
    }
    let rendered = driver.finish(BibliographyRequest::new(style, None, &locale_files));
    let mut bibliography = BTreeMap::new();
    if let Some(rendered) = rendered.bibliography {
        for item in rendered.items {
            let mut line = String::new();
            if let Some(first_field) = item.first_field {
                let mut prefix = String::new();
                let _ = first_field.write_buf(&mut prefix, BufWriteFormat::Plain);
                let prefix = prefix.trim();
                if !prefix.is_empty() {
                    line.push_str(prefix);
                    line.push(' ');
                }
            }
            let _ = item.content.write_buf(&mut line, BufWriteFormat::Plain);
            bibliography.insert(item.key, line.trim().to_owned());
        }
    }
    let citations = rendered
        .citations
        .into_iter()
        .zip(called_groups)
        .map(|(citation, (marker, keys, note_number))| {
            let mut line = String::new();
            let _ = citation
                .citation
                .write_buf(&mut line, BufWriteFormat::Plain);
            InTextCitation {
                keys,
                rendered: line.trim().to_owned(),
                marker,
                note_number,
            }
        })
        .collect();
    BatchCslRender {
        bibliography,
        citations,
    }
}
/// Rebuilds the structured entry stored in a reference note's frontmatter so a
/// different style renders without re-importing the source file.
pub fn bib_entry_from_frontmatter(source: &str, title: &str) -> Option<BibEntry> {
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for line in source.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(value);
        fields.insert(name.trim().to_owned(), value.to_owned());
    }
    let key = fields.get("citation_key")?.clone();
    if key.is_empty() {
        return None;
    }
    let authors = fields
        .get("authors")
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|author| !author.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(BibEntry {
        key,
        entry_type: fields
            .get("citation_type")
            .cloned()
            .unwrap_or_else(|| "misc".to_owned()),
        title: title.to_owned(),
        authors,
        year: fields.get("year").cloned().filter(|year| !year.is_empty()),
        container: fields
            .get("container")
            .cloned()
            .filter(|container| !container.is_empty()),
        doi: fields.get("doi").cloned().filter(|doi| !doi.is_empty()),
        url: fields.get("url").cloned().filter(|url| !url.is_empty()),
        fields: BTreeMap::new(),
    })
}

fn frontmatter_text(document: &NoteDocument) -> Option<&str> {
    document
        .content
        .iter()
        .find(|node| node.kind == NodeType::Frontmatter)
        .and_then(|node| node.attrs.source.as_deref())
}

/// Collects `[@key]`, `[@a; @b]` and `[@key, p. 3]` markers in document order.
pub fn citation_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for group in citation_groups(text) {
        for key in group {
            if !keys.iter().any(|existing| existing == &key) {
                keys.push(key);
            }
        }
    }
    keys
}

pub fn render_bibliography(entries: &[BibliographyEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| format!("{}. {}", index + 1, entry.formatted))
        .collect::<Vec<_>>()
        .join("\n")
}

fn frontmatter_source(entry: &BibEntry) -> String {
    let mut lines = vec![
        format!("citation_key: \"{}\"", escape(&entry.key)),
        format!("citation_type: \"{}\"", escape(&entry.entry_type)),
    ];
    if let Some(year) = &entry.year {
        lines.push(format!("year: \"{}\"", escape(year)));
    }
    if !entry.authors.is_empty() {
        lines.push(format!(
            "authors: \"{}\"",
            escape(&entry.authors.join("; "))
        ));
    }
    if let Some(container) = &entry.container {
        lines.push(format!("container: \"{}\"", escape(container)));
    }
    if let Some(doi) = &entry.doi {
        lines.push(format!("doi: \"{}\"", escape(doi)));
    }
    if let Some(url) = &entry.url {
        lines.push(format!("url: \"{}\"", escape(url)));
    }
    lines.join("\n")
}

fn annotation_node(kind: NodeType, source: Option<String>) -> DocumentNode {
    DocumentNode {
        kind,
        attrs: NodeAttributes {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            ..Default::default()
        },
        content: Vec::new(),
        text: None,
        marks: Vec::new(),
    }
}

fn paragraph_node(text: &str) -> DocumentNode {
    DocumentNode {
        kind: NodeType::Paragraph,
        attrs: NodeAttributes {
            id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        },
        content: vec![DocumentNode {
            kind: NodeType::Text,
            attrs: NodeAttributes::default(),
            content: Vec::new(),
            text: Some(text.to_owned()),
            marks: Vec::new(),
        }],
        text: None,
        marks: Vec::new(),
    }
}

fn citation_key(document: &NoteDocument) -> Option<String> {
    for node in &document.content {
        if node.kind != NodeType::Frontmatter {
            continue;
        }
        let source = node.attrs.source.as_deref()?;
        for line in source.lines() {
            if let Some(value) = line.trim().strip_prefix("citation_key:") {
                let value = value.trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

impl Library {
    fn citation_index(&self) -> Result<BTreeMap<String, String>> {
        let mut index = BTreeMap::new();
        for note in self.list_notes()? {
            if let Some(key) = citation_key(&note.document) {
                index.entry(key).or_insert(note.id);
            }
        }
        Ok(index)
    }

    /// Built-in and workspace-imported CSL styles available to the bibliography.
    pub fn citation_style_catalog(&self) -> Result<Vec<CitationStyleSummary>> {
        let mut styles = built_in_citation_styles();
        let mut statement = self.connection.prepare(
            "SELECT id,title,kind,parent_id FROM citation_styles ORDER BY lower(title),id",
        )?;
        let rows = statement.query_map([], |row| {
            let kind: String = row.get(2)?;
            Ok(CitationStyleSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                kind: match kind.as_str() {
                    "independent" => CitationStyleKind::Independent,
                    _ => CitationStyleKind::Dependent,
                },
                parent_id: row.get(3)?,
            })
        })?;
        for row in rows {
            styles.push(row?);
        }
        Ok(styles)
    }

    /// Imports a standalone `.csl` style. Dependent styles require an already
    /// available parent style, so a broken parent chain fails before commit.
    pub fn import_citation_style(&mut self, source: &Path) -> Result<CitationStyleSummary> {
        if !source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csl"))
        {
            return Err(Error::InvalidInput("csl_required"));
        }
        let metadata = fs::metadata(source).map_err(|_| Error::InvalidInput("csl_unreadable"))?;
        if metadata.len() > MAX_CSL_BYTES {
            return Err(Error::InvalidInput("csl_too_large"));
        }
        let bytes = fs::read(source).map_err(|_| Error::InvalidInput("csl_unreadable"))?;
        let xml = String::from_utf8(bytes).map_err(|_| Error::InvalidInput("csl_not_utf8"))?;
        let (id, title, kind, parent_id) = parse_csl_style(&xml)?;
        if citation_style_from_id(&id).is_some() || id == "plain" {
            return Err(Error::InvalidInput("csl_builtin_id_reserved"));
        }
        let kind_text = match kind {
            CitationStyleKind::Independent => "independent",
            CitationStyleKind::Dependent => "dependent",
            CitationStyleKind::Builtin => return Err(Error::InvalidInput("csl_invalid_style")),
        };
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO citation_styles(id,title,kind,parent_id,xml,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET title=excluded.title,kind=excluded.kind,parent_id=excluded.parent_id,xml=excluded.xml,created_at=excluded.created_at",
            params![id, title, kind_text, parent_id, xml, now],
        )?;
        resolve_citation_style(&tx, &id)?;
        tx.commit()?;
        Ok(CitationStyleSummary {
            id,
            title,
            kind,
            parent_id,
        })
    }

    /// Removes an imported style. Built-ins are immutable and a style still used
    /// as a dependent parent cannot be removed.
    pub fn delete_citation_style(&mut self, style_id: &str) -> Result<()> {
        if citation_style_from_id(style_id).is_some() || style_id == "plain" {
            return Err(Error::InvalidInput("csl_builtin_readonly"));
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM citation_styles WHERE id=?1)",
            [style_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        let used: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM citation_styles WHERE parent_id=?1)",
            [style_id],
            |row| row.get(0),
        )?;
        if used {
            return Err(Error::InvalidInput("csl_style_in_use"));
        }
        self.connection
            .execute("DELETE FROM citation_styles WHERE id=?1", [style_id])?;
        Ok(())
    }

    /// Resolves a note with a built-in alias or an imported CSL style id. The
    /// driver is shared across every entry and citation group so numeric styles
    /// keep one sequence instead of restarting at one for each reference.
    pub fn bibliography_with_style(
        &self,
        note_id: &str,
        style_id: &str,
    ) -> Result<BibliographyReport> {
        let note = self.note(note_id)?;
        self.bibliography_note_with_style(&note, style_id)
    }

    /// Renders citations from an already loaded revision. Export jobs use this
    /// so a historical revision is never replaced by the current head note.
    pub(crate) fn bibliography_note_with_style(
        &self,
        note: &Note,
        style_id: &str,
    ) -> Result<BibliographyReport> {
        if style_id == "plain" {
            return self.bibliography_note_plain(note);
        }
        let csl = resolve_citation_style(&self.connection, style_id)?;
        let text = note.document.plain_text();
        let groups = citation_occurrences(&text);
        let keys = citation_keys(&text);
        let index = self.citation_index()?;
        let mut references = Vec::new();
        let mut missing = Vec::new();
        let mut note_identity = BTreeMap::new();
        for key in &keys {
            let Some(target) = index.get(key) else {
                missing.push(key.clone());
                continue;
            };
            let reference = self.note(target)?;
            let entry = frontmatter_text(&reference.document)
                .and_then(|source| bib_entry_from_frontmatter(source, &reference.title))
                .unwrap_or_else(|| BibEntry {
                    key: key.clone(),
                    entry_type: "misc".to_owned(),
                    title: reference.title.clone(),
                    authors: Vec::new(),
                    year: None,
                    container: None,
                    doi: None,
                    url: None,
                    fields: BTreeMap::new(),
                });
            note_identity.insert(key.clone(), (target.clone(), reference.title.clone()));
            references.push(CslReference {
                key: key.clone(),
                entry,
            });
        }
        let rendered = render_csl_batch(&references, &groups, &csl);
        let entries = references
            .iter()
            .filter_map(|reference| {
                let (note_id, title) = note_identity.get(&reference.key)?;
                let formatted = rendered
                    .bibliography
                    .get(&reference.key)
                    .cloned()
                    .filter(|line| !line.trim().is_empty())
                    .unwrap_or_else(|| format_reference(&reference.entry));
                Some(BibliographyEntry {
                    key: reference.key.clone(),
                    title: title.clone(),
                    note_id: note_id.clone(),
                    formatted,
                })
            })
            .collect::<Vec<_>>();
        let markdown = render_bibliography(&entries);
        Ok(BibliographyReport {
            keys,
            entries,
            missing,
            markdown,
            style: citation_style_from_id(style_id).unwrap_or(CitationStyle::Custom),
            style_id: style_id.to_owned(),
            in_text: rendered.citations,
        })
    }
    /// Imports a BibTeX file as one reference note per entry. The citation key
    /// lives in the note frontmatter, so `[@key]` can resolve back to the note.
    pub fn import_bibliography(&mut self, source: &Path) -> Result<BibImportReport> {
        let metadata =
            fs::metadata(source).map_err(|_| Error::InvalidInput("bibtex_unreadable"))?;
        if metadata.len() > MAX_BIBTEX_BYTES {
            return Err(Error::InvalidInput("bibtex_too_large"));
        }
        let bytes = fs::read(source).map_err(|_| Error::InvalidInput("bibtex_unreadable"))?;
        let text = String::from_utf8(bytes).map_err(|_| Error::InvalidInput("bibtex_not_utf8"))?;
        let is_json = source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        let (entries, losses) = if is_json {
            parse_csl_json(&text)
        } else {
            parse_bibtex(&text)
        };
        let existing = self.citation_index()?;
        let mut report = BibImportReport {
            entries: entries.len() as u32,
            imported: 0,
            skipped_existing: 0,
            losses,
        };
        for entry in entries {
            if existing.contains_key(&entry.key) {
                report.skipped_existing += 1;
                continue;
            }
            let title = if entry.title.is_empty() {
                entry.key.clone()
            } else {
                entry.title.clone()
            };
            let document = NoteDocument {
                kind: DocumentType::Doc,
                content: vec![
                    annotation_node(NodeType::Frontmatter, Some(frontmatter_source(&entry))),
                    paragraph_node(&format_reference(&entry)),
                ],
            };
            self.create_document(
                &uuid::Uuid::new_v4().to_string(),
                CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title,
                    document_schema: DOCUMENT_SCHEMA,
                    document,
                },
            )?;
            report.imported += 1;
        }
        Ok(report)
    }

    /// Resolves the `[@key]` markers of one note with the historical plain style.
    pub fn bibliography(&self, note_id: &str) -> Result<BibliographyReport> {
        self.bibliography_styled(note_id, CitationStyle::Plain)
    }

    /// Resolves the `[@key]` markers of one note into a reference list.
    pub fn bibliography_styled(
        &self,
        note_id: &str,
        style: CitationStyle,
    ) -> Result<BibliographyReport> {
        if style != CitationStyle::Plain {
            return self.bibliography_with_style(note_id, style.id());
        }
        let note = self.note(note_id)?;
        self.bibliography_note_plain(&note)
    }

    fn bibliography_note_plain(&self, note: &Note) -> Result<BibliographyReport> {
        let keys = citation_keys(&note.document.plain_text());
        let index = self.citation_index()?;
        let mut entries = Vec::new();
        let mut missing = Vec::new();
        let mut seen = BTreeSet::new();
        for key in &keys {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some(target) = index.get(key) else {
                missing.push(key.clone());
                continue;
            };
            let reference = self.note(target)?;
            let formatted = reference
                .document
                .plain_text()
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned();
            entries.push(BibliographyEntry {
                key: key.clone(),
                title: reference.title.clone(),
                note_id: target.clone(),
                formatted,
            });
        }
        let markdown = render_bibliography(&entries);
        Ok(BibliographyReport {
            keys,
            entries,
            missing,
            markdown,
            style: CitationStyle::Plain,
            style_id: "plain".to_owned(),
            in_text: Vec::new(),
        })
    }

    /// Every citation-keyed note in the workspace, sorted by key, for the
    /// citation picker. Same frontmatter contract as bibliography import.
    pub fn bibliography_catalog(&self) -> Result<Vec<BibliographyEntry>> {
        let mut entries = Vec::new();
        for note in self.list_notes()? {
            let Some(key) = citation_key(&note.document) else {
                continue;
            };
            let formatted = note
                .document
                .plain_text()
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned();
            entries.push(BibliographyEntry {
                key,
                title: note.title.clone(),
                note_id: note.id,
                formatted,
            });
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(entries)
    }

    /// Appends a `[@key]` marker to a note without flattening its structure.
    pub fn citation_insert(&mut self, note_id: &str, key: &str) -> Result<crate::Note> {
        let index = self.citation_index()?;
        if !index.contains_key(key) {
            return Err(Error::NotFound);
        }
        let note = self.note(note_id)?;
        let mut document = note.document.clone();
        document.content.push(paragraph_node(&format!("[@{key}]")));
        let command_id = uuid::Uuid::new_v4().to_string();
        self.save_document(
            &command_id,
            SaveNote {
                note_id: note.id,
                base_revision_id: note.revision_id,
                title: note.title,
                document_schema: note.schema_version,
                document,
            },
        )
        .map(|result| result.note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
@comment{ignored}
@article{vaswani2017,
  title = {Attention is All You Need},
  author = {Vaswani, Ashish and Shazeer, Noam},
  journal = {NeurIPS},
  year = {2017},
  doi = {10.48550/arXiv.1706.03762}
}

@inproceedings(devlin2019,
  title = \"BERT: Pre-training {of} Deep Bidirectional Transformers\",
  author = {Devlin, Jacob},
  booktitle = {NAACL},
  year = {2019}
)
";

    #[test]
    fn parses_braced_and_parenthesised_entries_and_skips_comments() {
        let (entries, losses) = parse_bibtex(SAMPLE);
        assert!(losses.is_empty(), "{losses:?}");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "vaswani2017");
        assert_eq!(entries[0].title, "Attention is All You Need");
        assert_eq!(entries[0].authors, ["Vaswani, Ashish", "Shazeer, Noam"]);
        assert_eq!(entries[0].container.as_deref(), Some("NeurIPS"));
        assert_eq!(entries[1].key, "devlin2019");
        assert_eq!(
            entries[1].title,
            "BERT: Pre-training {of} Deep Bidirectional Transformers"
        );
        assert_eq!(entries[1].entry_type, "inproceedings");
    }

    #[test]
    fn string_and_comment_blocks_are_skipped() {
        let (entries, losses) =
            parse_bibtex("@string{x = {y}}\n@comment{hi}\n@book{b1, title={T}}");
        assert!(losses.is_empty(), "{losses:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "b1");
    }

    #[test]
    fn citation_keys_accept_multiple_and_locator_forms() {
        assert_eq!(
            citation_keys("甲[@vaswani2017; @devlin2019]。乙[@devlin2019, p. 3]。丙[@missing]"),
            ["vaswani2017", "devlin2019", "missing"]
        );
        assert!(citation_keys("没有引用").is_empty());
    }

    #[test]
    fn citation_locators_are_typed_and_keep_the_raw_marker() {
        use citationberg::taxonomy::Locator;
        let groups = citation_occurrences(
            "甲[@vaswani2017, pp. 3-4; @devlin2019, ch. 2; @other, 7; @raw, 图 3]。",
        );
        assert_eq!(groups.len(), 1);
        let group = &groups[0].1;
        assert_eq!(
            group.marker,
            "[@vaswani2017, pp. 3-4; @devlin2019, ch. 2; @other, 7; @raw, 图 3]"
        );
        assert_eq!(group.keys, ["vaswani2017", "devlin2019", "other", "raw"]);
        assert_eq!(
            group.specs[0].locator.as_ref().unwrap().label,
            Locator::Page
        );
        assert_eq!(group.specs[0].locator.as_ref().unwrap().value, "3-4");
        assert_eq!(
            group.specs[1].locator.as_ref().unwrap().label,
            Locator::Chapter
        );
        assert_eq!(group.specs[1].locator.as_ref().unwrap().value, "2");
        assert_eq!(
            group.specs[2].locator.as_ref().unwrap().label,
            Locator::Page
        );
        assert_eq!(group.specs[2].locator.as_ref().unwrap().value, "7");
        assert_eq!(
            group.specs[3].locator.as_ref().unwrap().label,
            Locator::Custom
        );
        assert_eq!(group.specs[3].locator.as_ref().unwrap().value, "图 3");
    }
    #[test]
    fn csl_json_projects_authors_and_issued_date() {
        let (entries, _) = parse_bibtex(SAMPLE);
        let value = csl_json(&entries);
        let first = &value[0];
        assert_eq!(first["id"], "vaswani2017");
        assert_eq!(first["type"], "article-journal");
        assert_eq!(first["author"][0]["family"], "Vaswani");
        assert_eq!(first["author"][0]["given"], "Ashish");
        assert_eq!(first["issued"]["date-parts"][0][0], 2017);
    }

    #[test]
    fn csl_json_items_become_entries() {
        let json = r#"[{"id":"z1","type":"article-journal","title":"A","author":[{"family":"Doe","given":"Jane"}],"issued":{"date-parts":[[2021]]},"DOI":"10.1/x","container-title":"J"}]"#;
        let (entries, losses) = parse_csl_json(json);
        assert!(losses.is_empty(), "{losses:?}");
        assert_eq!(entries[0].key, "z1");
        assert_eq!(entries[0].entry_type, "article");
        assert_eq!(entries[0].authors, ["Doe, Jane"]);
        assert_eq!(entries[0].year.as_deref(), Some("2021"));
        assert_eq!(entries[0].container.as_deref(), Some("J"));
        assert_eq!(entries[0].doi.as_deref(), Some("10.1/x"));
    }

    #[test]
    fn formatted_reference_carries_type_marker_and_doi() {
        let (entries, _) = parse_bibtex(SAMPLE);
        let text = format_reference(&entries[0]);
        assert!(text.contains("[J]"), "{text}");
        assert!(text.contains("DOI: 10.48550/arXiv.1706.03762"), "{text}");
        assert!(text.contains("NeurIPS, 2017."), "{text}");
    }
}
