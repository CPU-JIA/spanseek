//! Bind unambiguous wiki names once, before a revision enters its write transaction.
//! The original spelling stays in the document; existing stable IDs are never rebound.
use crate::{DocumentNode, Library, NodeType, NoteDocument, Result};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

const MAX_ANCHOR_DOCUMENTS: usize = 32;
const MAX_ANCHOR_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct Target {
    id: String,
    note: bool,
}

#[derive(Clone)]
enum Match {
    Unique(Target),
    Ambiguous,
}

fn record(matches: &mut BTreeMap<String, Match>, key: String, target: Target) {
    matches
        .entry(key)
        .and_modify(|found| {
            if !matches!(found, Match::Unique(previous) if previous.id == target.id) {
                *found = Match::Ambiguous;
            }
        })
        .or_insert(Match::Unique(target));
}

fn title_key(title: &str, note: bool) -> String {
    let key = title.nfc().collect::<String>().to_lowercase();
    if note {
        key.strip_suffix(".md").unwrap_or(&key).to_owned()
    } else {
        key
    }
}

fn percent_decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

pub(crate) fn has_fragment(value: &str) -> bool {
    percent_decode(value.trim()).is_some_and(|decoded| {
        decoded
            .split_once('#')
            .is_some_and(|(_, fragment)| !fragment.is_empty())
    })
}

fn wiki_parts(value: &str, title: &str) -> Option<(String, String)> {
    let decoded = percent_decode(value.trim())?;
    if decoded.chars().any(char::is_control) || decoded.starts_with(['/', '\\']) {
        return None;
    }
    if decoded.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    }) {
        return None;
    }
    let (name, fragment) = decoded.split_once('#').unwrap_or((&decoded, ""));
    let name = name.replace('\\', "/");
    let mut segments = if name.starts_with("./") || name.starts_with("../") {
        title.split('/').collect::<Vec<_>>()
    } else {
        vec![]
    };
    if !segments.is_empty() {
        segments.pop();
    }
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(part),
        }
    }
    Some((segments.join("/"), fragment.to_owned()))
}

fn note_like(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    !leaf.contains('.') || leaf.to_ascii_lowercase().ends_with(".md") || leaf.ends_with('.')
}

type Anchors = BTreeMap<String, Option<String>>;

fn anchors(document: &NoteDocument) -> Anchors {
    fn add(result: &mut Anchors, key: String, id: &str) {
        result
            .entry(key)
            .and_modify(|found| {
                if found.as_deref() != Some(id) {
                    *found = None;
                }
            })
            .or_insert_with(|| Some(id.to_owned()));
    }
    fn visit(nodes: &[DocumentNode], headings: &mut Vec<String>, result: &mut Anchors) {
        for node in nodes {
            if let Some(alias) = &node.attrs.block_id {
                add(result, format!("^{alias}"), &node.attrs.id);
            }
            if node.kind == NodeType::Heading {
                let level = usize::from(node.attrs.level.unwrap_or(1));
                headings.resize(level, String::new());
                headings[level - 1] = node.plain_text().trim().to_owned();
                let meaningful = headings
                    .iter()
                    .filter(|heading| !heading.is_empty())
                    .collect::<Vec<_>>();
                for start in 0..meaningful.len() {
                    add(
                        result,
                        meaningful[start..]
                            .iter()
                            .map(|value| value.as_str())
                            .collect::<Vec<_>>()
                            .join("#"),
                        &node.attrs.id,
                    );
                }
            }
            visit(&node.content, headings, result);
        }
    }
    let mut result = Anchors::new();
    visit(&document.content, &mut vec![], &mut result);
    result
}

fn anchor_key(fragment: &str) -> String {
    if fragment.starts_with('^') {
        fragment.to_owned()
    } else {
        fragment
            .split('#')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("#")
    }
}

impl Library {
    pub(crate) fn resolve_wiki_document(
        &self,
        current_id: Option<&str>,
        title: &str,
        document: &mut NoteDocument,
    ) -> Result<()> {
        let requested: BTreeSet<String> = document
            .nodes()
            .into_iter()
            .filter(|node| {
                matches!(node.kind, NodeType::WikiLink | NodeType::WikiEmbed)
                    && node.attrs.target_id.is_none()
            })
            .filter_map(|node| node.attrs.wiki_target.clone())
            .collect();
        if requested.is_empty() {
            return Ok(());
        }
        let parts = requested
            .into_iter()
            .filter_map(|raw| wiki_parts(&raw, title).map(|parts| (raw, parts)))
            .collect::<BTreeMap<_, _>>();
        let note_keys = parts
            .values()
            .filter(|(path, _)| !path.is_empty() && note_like(path))
            .map(|(path, _)| title_key(path, true))
            .collect::<BTreeSet<_>>();
        let source_keys = parts
            .values()
            .filter(|(path, _)| !path.is_empty())
            .map(|(path, _)| title_key(path, false))
            .collect::<BTreeSet<_>>();
        let ids = parts
            .values()
            .filter_map(|(path, _)| Uuid::parse_str(path).ok().map(|id| id.to_string()))
            .collect::<BTreeSet<_>>();
        let mut notes = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let mut explicit = BTreeMap::new();
        // Scan narrow title columns only once. Large target documents are loaded
        // below under a separate, bounded anchor budget, never under a write lock.
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        if !note_keys.is_empty() || !ids.is_empty() {
            let mut statement = self
                .connection
                .prepare("SELECT id,title FROM notes WHERE trashed=0")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, name) = row?;
                let target = Target {
                    id: id.clone(),
                    note: true,
                };
                if ids.contains(&id) {
                    explicit.insert(id.clone(), target.clone());
                }
                if current_id == Some(id.as_str()) {
                    continue;
                }
                let key = title_key(&name, true);
                if note_keys.contains(&key) {
                    record(&mut notes, key, target);
                }
            }
        }
        if let Some(id) = current_id {
            let target = Target {
                id: id.to_owned(),
                note: true,
            };
            let key = title_key(title, true);
            if note_keys.contains(&key) {
                record(&mut notes, key, target.clone());
            }
            if ids.contains(id) {
                explicit.insert(id.to_owned(), target);
            }
        }
        if !source_keys.is_empty() || !ids.is_empty() {
            let mut statement = self.connection.prepare("SELECT s.id,s.display_title FROM sources s JOIN objects o ON o.id=s.id WHERE o.trashed=0")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, name) = row?;
                let target = Target {
                    id: id.clone(),
                    note: false,
                };
                if ids.contains(&id) {
                    explicit.insert(id, target.clone());
                }
                let key = title_key(&name, false);
                if source_keys.contains(&key) {
                    record(&mut sources, key, target);
                }
            }
        }
        let mut own_anchors = None;
        let mut cached = BTreeMap::<String, Anchors>::new();
        let mut remaining = MAX_ANCHOR_BYTES;
        let mut bindings = BTreeMap::<String, (String, Option<String>)>::new();
        for (raw, (path, fragment)) in parts {
            let target = if path.is_empty() {
                current_id.map(|id| Target {
                    id: id.into(),
                    note: true,
                })
            } else if let Ok(id) = Uuid::parse_str(&path) {
                explicit.get(&id.to_string()).cloned()
            } else {
                match notes
                    .get(&title_key(&path, true))
                    .filter(|_| note_like(&path))
                {
                    Some(Match::Unique(target)) => Some(target.clone()),
                    Some(Match::Ambiguous) => None,
                    None => match sources.get(&title_key(&path, false)) {
                        Some(Match::Unique(target)) => Some(target.clone()),
                        _ => None,
                    },
                }
            };
            let Some(target) = target else {
                continue;
            };
            let mut block = None;
            if target.note && !fragment.is_empty() {
                let key = anchor_key(&fragment);
                if current_id == Some(target.id.as_str()) {
                    block = own_anchors
                        .get_or_insert_with(|| anchors(document))
                        .get(&key)
                        .cloned()
                        .flatten();
                } else {
                    if !cached.contains_key(&target.id) && cached.len() < MAX_ANCHOR_DOCUMENTS {
                        let size: u32 = self.connection.query_row("SELECT length(CAST(r.document_json AS BLOB)) FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id WHERE n.id=?1", [&target.id], |row| row.get(0))?;
                        if size as usize <= remaining {
                            let note = self.note(&target.id)?;
                            remaining -= size as usize;
                            cached.insert(target.id.clone(), anchors(&note.document));
                        } else {
                            // Keep the named anchor unresolved for on-demand reading.
                            cached.insert(target.id.clone(), Anchors::new());
                        }
                    }
                    block = cached
                        .get(&target.id)
                        .and_then(|map| map.get(&key))
                        .cloned()
                        .flatten();
                }
            }
            bindings.insert(raw, (target.id, block));
        }
        fn bind(nodes: &mut [DocumentNode], bindings: &BTreeMap<String, (String, Option<String>)>) {
            for node in nodes {
                if matches!(node.kind, NodeType::WikiLink | NodeType::WikiEmbed)
                    && node.attrs.target_id.is_none()
                    && let Some((target, block)) = node
                        .attrs
                        .wiki_target
                        .as_ref()
                        .and_then(|raw| bindings.get(raw))
                {
                    node.attrs.target_id = Some(target.clone());
                    node.attrs.target_block_id.clone_from(block);
                }
                bind(&mut node.content, bindings);
            }
        }
        bind(&mut document.content, &bindings);
        Ok(())
    }
}
