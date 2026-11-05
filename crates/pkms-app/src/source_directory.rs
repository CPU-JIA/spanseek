//! Recursive directory import. One user-granted root is walked once; every
//! importable file then gets its own per-file grant, so the existing
//! `sources_import` pipeline keeps its per-item receipts, retry and cancel
//! instead of a second import path growing beside it.
use crate::{AppService, GrantPurpose, ResourceGrant, Result, WorkspaceIdentity};
use pkms_core::SourceFormat;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// Bounds so one scan cannot become an unbounded walk of a whole drive.
const MAX_VISITED_FILES: usize = 5_000;
const MAX_GRANTS: usize = 500;
const MAX_DEPTH: usize = 32;
/// A file at or below this size is hashed during the scan so duplicates are
/// reported up front. Larger files stay importable and are compared on import.
const MAX_HASH_BYTES: u64 = 64 * 1024 * 1024;
/// Mirrors the import limit in the core; larger files are reported, not read.
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryScanStatus {
    Ready,
    /// Same content as a document already in this library.
    Duplicate,
    Unsupported,
    Empty,
    TooLarge,
    Unreadable,
    /// Past the per-scan grant limit; scan again to reach the rest.
    LimitReached,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DirectoryScanEntry {
    /// Path below the granted root, always with `/` separators.
    pub relative_path: String,
    pub name: String,
    pub size: u64,
    pub format: Option<SourceFormat>,
    pub status: DirectoryScanStatus,
    /// Machine code for the UI to word: a skip reason, or `not_hashed` on a
    /// large ready file whose duplicate check happens at import time.
    pub note: Option<String>,
    /// Title of the existing document a duplicate points at.
    pub existing_title: Option<String>,
    pub existing_trashed: bool,
    /// Present exactly when the entry can be handed to `sources_import` as-is.
    pub grant: Option<ResourceGrant>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DirectoryScan {
    pub root_name: String,
    pub entries: Vec<DirectoryScanEntry>,
    /// Files inspected before the visited limit stopped the walk.
    pub visited: u32,
    pub truncated: bool,
    pub limited: u32,
    /// Directories left unscanned because they sit below the depth cap. The
    /// interface has to state this instead of dropping them silently.
    pub deep_skipped: u32,
}

struct Walked {
    path: PathBuf,
    relative_path: String,
}

fn relative_text(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Depth-first walk that never follows a link, so a scan cannot leave the root
/// the user granted. Entries are sorted afterwards to keep results stable.
fn walk(root: &Path) -> (Vec<Walked>, u32, bool, u32) {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0u32;
    let mut truncated = false;
    let mut deep_skipped = 0u32;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            // Counted rather than dropped: the caller reports how many
            // directories stayed out of reach of the depth cap.
            deep_skipped += 1;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            visited += 1;
            if files.len() >= MAX_VISITED_FILES {
                truncated = true;
                break;
            }
            let Some(relative_path) = relative_text(root, &path) else {
                continue;
            };
            files.push(Walked {
                path,
                relative_path,
            });
        }
        if truncated {
            break;
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    (files, visited, truncated, deep_skipped)
}

fn content_hash(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

impl AppService {
    /// Scans a user-granted directory and returns one grant per importable file.
    /// Every grant is re-derived from the granted root, so a crafted entry can
    /// never widen the original selection.
    pub fn scan_source_directory(
        &mut self,
        identity: &WorkspaceIdentity,
        token: &str,
    ) -> Result<DirectoryScan> {
        self.library(identity)?;
        let root = self.resolve_grant(token, &GrantPurpose::ImportDirectory)?;
        let root_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let (walked, visited, truncated, deep_skipped) = walk(&root);
        let mut entries = Vec::with_capacity(walked.len());
        let mut limited = 0u32;
        let mut granted = 0usize;
        for item in walked {
            let name = item
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let format = item
                .path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .and_then(|extension| crate::source_imports::format_for_extension(&extension));
            let mut entry = DirectoryScanEntry {
                relative_path: item.relative_path,
                name,
                size: 0,
                format,
                status: DirectoryScanStatus::Ready,
                note: None,
                existing_title: None,
                existing_trashed: false,
                grant: None,
            };
            match std::fs::symlink_metadata(&item.path) {
                Ok(metadata) if metadata.is_file() => entry.size = metadata.len(),
                _ => {
                    entry.status = DirectoryScanStatus::Unreadable;
                    entry.note = Some("unreadable".into());
                    entries.push(entry);
                    continue;
                }
            }
            if entry.format.is_none() {
                entry.status = DirectoryScanStatus::Unsupported;
                entry.note = Some("unsupported_format".into());
            } else if entry.size == 0 {
                entry.status = DirectoryScanStatus::Empty;
                entry.note = Some("empty_file".into());
            } else if entry.size > MAX_SOURCE_BYTES {
                entry.status = DirectoryScanStatus::TooLarge;
                entry.note = Some("too_large".into());
            } else if entry.size > MAX_HASH_BYTES {
                entry.note = Some("not_hashed".into());
            } else {
                match content_hash(&item.path) {
                    Ok(hash) => {
                        if let Some(source) =
                            self.library(identity)?.source_for_content_hash(&hash)?
                        {
                            entry.status = DirectoryScanStatus::Duplicate;
                            entry.existing_title = Some(source.display_title);
                            entry.existing_trashed = source.trashed;
                        }
                    }
                    Err(_) => {
                        entry.status = DirectoryScanStatus::Unreadable;
                        entry.note = Some("unreadable".into());
                    }
                }
            }
            if matches!(
                entry.status,
                DirectoryScanStatus::Ready | DirectoryScanStatus::Duplicate
            ) {
                if granted >= MAX_GRANTS {
                    entry.status = DirectoryScanStatus::LimitReached;
                    entry.note = Some("limit_reached".into());
                    limited += 1;
                } else {
                    match self.grant_path(GrantPurpose::ImportSource, &item.path) {
                        Ok(grant) => {
                            entry.grant = Some(grant);
                            granted += 1;
                        }
                        Err(_) => {
                            entry.status = DirectoryScanStatus::Unreadable;
                            entry.note = Some("unreadable".into());
                        }
                    }
                }
            }
            entries.push(entry);
        }
        Ok(DirectoryScan {
            root_name,
            entries,
            visited,
            truncated,
            limited,
            deep_skipped,
        })
    }
}
