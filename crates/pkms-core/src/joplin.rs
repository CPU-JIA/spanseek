//! Joplin JEX import.
//!
//! A JEX export is a tar archive of `<note-id>.md` files plus a `resources/`
//! directory. Each note begins with a `key: value` header terminated by a blank
//! line. This module converts that header into YAML frontmatter, rewrites
//! Joplin's `:/<resource>` links to relative paths and then reuses the Obsidian
//! vault importer, so links, tags and attachments follow the same rules.

use crate::{Error, Library, Result, VaultReport};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use tempfile::tempdir;

const MAX_ENTRIES: usize = 20_000;
const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoplinImportReport {
    pub notes: u32,
    pub skipped_deleted: u32,
    pub skipped_encrypted: u32,
    pub resources: u32,
    pub vault: VaultReport,
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Splits a JEX note header from its body. Only files whose first line starts
/// with `title:` are treated as JEX notes, so ordinary Markdown is untouched.
fn split_header(text: &str) -> Option<(BTreeMap<String, String>, &str)> {
    if !text.starts_with("title:") {
        return None;
    }
    let mut fields = BTreeMap::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim().is_empty() {
            return Some((fields, &text[offset + line.len()..]));
        }
        let (key, value) = trimmed.split_once(':')?;
        fields.insert(key.trim().to_owned(), value.trim().to_owned());
        offset += line.len();
    }
    Some((fields, ""))
}

/// Joplin link syntax: `:/<resource-id>.<ext>` relative to the archive root.
fn rewrite_resources(body: &str) -> String {
    body.replace("](:/", "](resources/")
}

impl Library {
    pub fn import_joplin_jex(&mut self, source: &Path) -> Result<JoplinImportReport> {
        let file = fs::File::open(source).map_err(|_| Error::InvalidInput("jex_unreadable"))?;
        let mut archive = tar::Archive::new(file);
        let temp = tempdir().map_err(|_| Error::InvalidInput("jex_unwritable"))?;
        let notes_dir = temp.path().join("notes");
        let resources_dir = temp.path().join("resources");
        fs::create_dir_all(&notes_dir).map_err(|_| Error::InvalidInput("jex_unwritable"))?;
        fs::create_dir_all(&resources_dir).map_err(|_| Error::InvalidInput("jex_unwritable"))?;

        let mut report = JoplinImportReport {
            notes: 0,
            skipped_deleted: 0,
            skipped_encrypted: 0,
            resources: 0,
            vault: VaultReport {
                notes: 0,
                markdown_files: 0,
                attachments: 0,
                links: 0,
                tags: 0,
                skipped_existing: 0,
                unresolved_links: 0,
                losses: Vec::new(),
                normalized: Vec::new(),
            },
        };
        let entries = archive
            .entries()
            .map_err(|_| Error::InvalidInput("jex_unreadable"))?;
        let mut total = 0u64;
        let mut seen = 0usize;
        for entry in entries {
            let mut entry = entry.map_err(|_| Error::InvalidInput("jex_unreadable"))?;
            seen += 1;
            if seen > MAX_ENTRIES {
                report
                    .vault
                    .losses
                    .push("JEX 条目超过 20000，其余未导入。".to_owned());
                break;
            }
            let path = entry
                .path()
                .map_err(|_| Error::InvalidInput("jex_unreadable"))?
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.header().size().unwrap_or(0);
            total = total.saturating_add(size);
            if total > MAX_TOTAL_BYTES {
                return Err(Error::InvalidInput("jex_too_large"));
            }
            if path == "manifest.json" || path.ends_with('/') {
                continue;
            }
            if let Some(name) = path.strip_prefix("resources/") {
                if name.contains('/') {
                    continue;
                }
                if size > MAX_RESOURCE_BYTES {
                    report
                        .vault
                        .losses
                        .push(format!("跳过超过 20 MiB 的附件：{name}"));
                    continue;
                }
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error::InvalidInput("jex_unreadable"))?;
                fs::write(resources_dir.join(name), bytes)
                    .map_err(|_| Error::InvalidInput("jex_unwritable"))?;
                report.resources += 1;
                continue;
            }
            if !path.ends_with(".md") {
                continue;
            }
            if size > MAX_NOTE_BYTES {
                report
                    .vault
                    .losses
                    .push(format!("跳过超过 2 MiB 的笔记：{path}"));
                continue;
            }
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_err() {
                report
                    .vault
                    .losses
                    .push(format!("跳过非 UTF-8 的笔记：{path}"));
                continue;
            }
            let Some((fields, body)) = split_header(&text) else {
                continue;
            };
            if fields
                .get("deleted_time")
                .and_then(|value| value.parse::<i64>().ok())
                .is_some_and(|deleted| deleted > 0)
            {
                report.skipped_deleted += 1;
                continue;
            }
            if fields
                .get("encryption_applied")
                .is_some_and(|value| value == "1")
            {
                report.skipped_encrypted += 1;
                continue;
            }
            let title = fields
                .get("title")
                .map(String::as_str)
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("未命名");
            let stem = Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| {
                    !stem.is_empty()
                        && stem
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
                })
                .unwrap_or("note");
            let content = format!(
                "---\ntitle: \"{}\"\njoplin_id: \"{}\"\n---\n{}",
                escape(title),
                escape(stem),
                rewrite_resources(body)
            );
            fs::write(notes_dir.join(format!("{stem}.md")), content)
                .map_err(|_| Error::InvalidInput("jex_unwritable"))?;
            report.notes += 1;
        }

        report.vault = self.import_obsidian_vault(temp.path(), "joplin")?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_jex_header_at_the_first_blank_line() {
        let text = "title: 笔记\nupdated_time: 2024-01-01T00:00:00Z\n\n正文第一行\n\n正文第二段";
        let (fields, body) = split_header(text).unwrap();
        assert_eq!(fields.get("title").map(String::as_str), Some("笔记"));
        assert_eq!(body, "正文第一行\n\n正文第二段");
    }

    #[test]
    fn plain_markdown_is_not_treated_as_a_jex_note() {
        assert!(split_header("# 标题\n\n正文").is_none());
    }

    #[test]
    fn resource_links_become_relative_paths() {
        assert_eq!(
            rewrite_resources("看这个 ![图](:/abc123.png) 还有 [文档](:/def.pdf)"),
            "看这个 ![图](resources/abc123.png) 还有 [文档](resources/def.pdf)"
        );
    }
}
