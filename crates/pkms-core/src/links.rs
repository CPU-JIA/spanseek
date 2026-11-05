//! Read-only resolution of stable identities and per-occurrence body backlinks.
use crate::{
    DocumentNode, Error, Library, LocatorStatus, NodeType, Note, ObjectKind, Result, SourceLocator,
    document::valid_uuid,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolveLink {
    pub target_id: String,
    pub target_block_id: Option<String>,
    pub target_revision_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Available,
    TargetMissing,
    TargetTrashed,
    BlockMissing,
    RevisionMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ResolvedLink {
    pub target: ResolveLink,
    pub state: LinkState,
    pub title: Option<String>,
    pub current_revision_id: Option<String>,
    pub resolved_revision_id: Option<String>,
    pub block_text: Option<String>,
    pub last_seen_revision_id: Option<String>,
    pub target_kind: Option<ObjectKind>,
    pub source_target: Option<ResolvedSourceTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ResolvedSourceTarget {
    pub source_id: String,
    pub source_version_id: String,
    pub annotation_id: Option<String>,
    pub locator: Option<SourceLocator>,
    pub locator_status: Option<LocatorStatus>,
    pub parse_id: Option<String>,
    pub block_offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BodyReference {
    pub source_id: String,
    pub source_revision_id: String,
    pub source_title: String,
    pub source_trashed: bool,
    pub source_block_id: String,
    pub occurrence_id: String,
    pub target_id: String,
    pub target_block_id: Option<String>,
    pub target_revision_id: Option<String>,
    pub label: String,
    /// Display excerpt, at most 280 Unicode scalar values. The source revision
    /// and source block/occurrence IDs identify the complete original content.
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BodyReferencePage {
    pub items: Vec<BodyReference>,
    /// Bound to this workspace, query and note-list generation; restart from
    /// the first page if a mutation expires it.
    pub next_cursor: Option<String>,
}

/// One frontmatter relation entry whose target is the queried note.
///
/// Relation fields are extensible YAML; only contract-shaped
/// `{kind: note, id, title}` records are reported. The source revision together
/// with the field key identifies the complete original entry, so the excerpt is
/// only a display aid.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RelationBacklink {
    pub source_id: String,
    pub source_revision_id: String,
    pub source_title: String,
    pub source_trashed: bool,
    /// Frontmatter key that carries the relation list.
    pub field_key: String,
    /// Title the source records for this entry, before any display bounding.
    pub relation_title: String,
    /// Display excerpt, at most 280 Unicode scalar values.
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RelationBacklinkPage {
    pub items: Vec<RelationBacklink>,
    /// Bound to this workspace, query and note-list generation; restart from
    /// the first page if a mutation expires it.
    pub next_cursor: Option<String>,
    /// True when the bounded candidate scan stopped before covering every note,
    /// so later pages can be missing matches. Callers state the list may be
    /// incomplete instead of presenting it as complete.
    pub truncated: bool,
    /// Candidate notes one page is allowed to parse.
    pub scan_limit: u32,
}

/// One frontmatter relation entry owned by the queried note.
///
/// Directions stay explicit: relation backlinks answer "who points at this
/// note", while this type answers "which live notes does this note point at".
/// The field key and title come from the same YAML contract used by graph,
/// canvas and knowledge-index related counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OutgoingRelation {
    pub target_id: String,
    pub field_key: String,
    pub relation_title: String,
    /// Display excerpt, at most 280 Unicode scalar values.
    pub context: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceCursor {
    scope: String,
    generation: i64,
    position: ReferencePosition,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReferencePosition {
    Backlink { rowid: i64 },
    Outgoing { offset: usize },
    RelationBacklink { offset: usize },
}

const CONTEXT_CHARS: usize = 280;
/// Candidate notes one relation-backlink page may parse. The target-UUID
/// pre-filter already keeps unrelated notes out of the scan; this bound stops a
/// library where most notes mention the target from making one page unbounded.
const MAX_RELATION_SCAN_ROWS: usize = 10_000;
/// A relation field key is a user-authored YAML key that is echoed back per row.
/// An over-long or control-bearing key is treated as malformed.
const MAX_RELATION_FIELD_CHARS: usize = 256;

impl Library {
    pub fn resolve_link(&self, target: ResolveLink) -> Result<ResolvedLink> {
        valid_uuid(&target.target_id)?;
        for value in [&target.target_block_id, &target.target_revision_id]
            .into_iter()
            .flatten()
        {
            valid_uuid(value)?;
        }
        // Resolve lifecycle, current version and immutable locator against one
        // read snapshot, including when another connection publishes a version.
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let mut result = ResolvedLink {
            target,
            state: LinkState::TargetMissing,
            title: None,
            current_revision_id: None,
            resolved_revision_id: None,
            block_text: None,
            last_seen_revision_id: None,
            target_kind: None,
            source_target: None,
        };
        let kind: Option<String> = self
            .connection
            .query_row(
                "SELECT kind FROM objects WHERE id=?1",
                [&result.target.target_id],
                |r| r.get(0),
            )
            .optional()?;
        match kind.as_deref() {
            Some("source") => return self.resolve_source_link(result, false),
            Some("annotation") => return self.resolve_source_link(result, true),
            Some("note") => result.target_kind = Some(ObjectKind::Note),
            _ => return Ok(result),
        }
        let head = match self.note(&result.target.target_id) {
            Ok(note) => note,
            Err(crate::Error::NotFound) => return Ok(result),
            Err(error) => return Err(error),
        };
        result.title = Some(head.title.clone());
        result.current_revision_id = Some(head.revision_id.clone());
        let note = if let Some(revision) = &result.target.target_revision_id {
            match self.revision(&head.id, revision) {
                Ok(note) => note,
                Err(crate::Error::NotFound) => {
                    result.state = LinkState::RevisionMissing;
                    return Ok(result);
                }
                Err(error) => return Err(error),
            }
        } else {
            head.clone()
        };
        result.title = Some(note.title.clone());
        result.resolved_revision_id = Some(note.revision_id.clone());
        result.state = if head.trashed {
            LinkState::TargetTrashed
        } else {
            LinkState::Available
        };
        if let Some(block) = &result.target.target_block_id {
            result.last_seen_revision_id = self.connection.query_row(
                "SELECT r.revision_id FROM note_revisions r JOIN note_revision_blocks b ON b.revision_id=r.revision_id WHERE r.note_id=?1 AND b.block_id=?2 ORDER BY r.revision DESC LIMIT 1",
                params![head.id, block], |r| r.get(0),
            ).optional()?;
            result.block_text = note
                .document
                .nodes()
                .into_iter()
                .find(|n| &n.attrs.id == block)
                .map(DocumentNode::plain_text);
            if result.block_text.is_none() && !head.trashed {
                result.state = LinkState::BlockMissing;
            }
        }
        Ok(result)
    }

    fn resolve_source_link(
        &self,
        mut result: ResolvedLink,
        is_annotation: bool,
    ) -> Result<ResolvedLink> {
        result.target_kind = Some(if is_annotation {
            ObjectKind::Annotation
        } else {
            ObjectKind::Source
        });
        let resolved = if is_annotation {
            Some(self.resolve_annotation(&result.target.target_id)?)
        } else {
            None
        };
        let source_id = match &resolved {
            Some(annotation) => {
                self.source_version(&annotation.annotation.source_version_id)?
                    .source_id
            }
            None => result.target.target_id.clone(),
        };
        let source = self.source(&source_id)?;
        result.title = Some(source.display_title.clone());
        result.current_revision_id = Some(source.current_version_id.clone());
        let version_id = resolved
            .as_ref()
            .map(|a| a.annotation.source_version_id.clone())
            .or_else(|| result.target.target_revision_id.clone())
            .unwrap_or(source.current_version_id);
        if resolved.is_some()
            && result
                .target
                .target_revision_id
                .as_ref()
                .is_some_and(|id| id != &version_id)
        {
            result.state = LinkState::RevisionMissing;
            return Ok(result);
        }
        let version = match self.source_version(&version_id) {
            Ok(version) if version.source_id == source_id => version,
            Ok(_) | Err(Error::NotFound) => {
                result.state = LinkState::RevisionMissing;
                return Ok(result);
            }
            Err(error) => return Err(error),
        };
        result.resolved_revision_id = Some(version_id.clone());
        let trashed = source.trashed || resolved.as_ref().is_some_and(|a| a.annotation.trashed);
        result.state = if trashed {
            LinkState::TargetTrashed
        } else {
            LinkState::Available
        };
        let mut jump = ResolvedSourceTarget {
            source_id,
            source_version_id: version_id.clone(),
            annotation_id: None,
            locator: None,
            locator_status: None,
            parse_id: None,
            block_offset: None,
        };
        if let Some(annotation) = resolved {
            result.block_text = Some(annotation.annotation.locator.quote().to_owned());
            if let Some(block_id) = &result.target.target_block_id {
                let matches = matches!(&annotation.annotation.locator, SourceLocator::Text { block_id: expected, .. } if expected == block_id);
                if !matches && !trashed {
                    result.state = LinkState::BlockMissing;
                }
            }
            jump.annotation_id = Some(annotation.annotation.id);
            jump.locator = Some(annotation.annotation.locator);
            jump.locator_status = Some(annotation.status);
            jump.parse_id = annotation.parse_id;
            jump.block_offset = annotation.block_offset;
        } else {
            jump.parse_id = self
                .connection
                .query_row(
                    "SELECT parse_id FROM source_active_parses WHERE source_version_id=?1",
                    [&version_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(block_id) = &result.target.target_block_id {
                let block: Option<(crate::SourceBlock, u32, String)> = self
                    .connection
                    .query_row(
                        "SELECT b.id,b.text,b.page_index,b.geometry_json,b.position,b.parse_id FROM source_blocks b JOIN source_parses p ON p.id=b.parse_id WHERE b.id=?1 AND p.source_version_id=?2 AND NOT EXISTS(SELECT 1 FROM source_parse_staging s WHERE s.parse_id=p.id)",
                        params![block_id, version_id],
                        |r| Ok((crate::sources::source_block_from_row(r)?, r.get(4)?, r.get(5)?)),
                    )
                    .optional()?;
                if let Some((block, offset, parse_id)) = block {
                    result.block_text = Some(block.text.clone());
                    jump.block_offset = Some(offset);
                    jump.parse_id = Some(parse_id);
                    if version.format == crate::SourceFormat::Pdf {
                        if let (Some(page_index), Some(geometry)) =
                            (block.page_index, block.pdf_geometry)
                        {
                            // A page reference is deliberately not an exact text highlight.
                            // Empty rectangles navigate to the real page without inventing
                            // text coordinates or adding an unrelated annotation object.
                            jump.locator = Some(SourceLocator::Pdf {
                                source_version_id: version_id.clone(),
                                page_index,
                                page_label: geometry.page_label,
                                crop_box: geometry.crop_box,
                                rotation: geometry.rotation,
                                coordinate_space: geometry.coordinate_space,
                                rects: Vec::new(),
                                quote: String::new(),
                                prefix: String::new(),
                                suffix: String::new(),
                            });
                            jump.locator_status = Some(LocatorStatus::Approximate);
                        } else {
                            // Legacy parses can lack geometry. Do not navigate to page one
                            // or make up a crop box while claiming to have located this block.
                            jump.locator_status = Some(LocatorStatus::Unavailable);
                        }
                    } else {
                        jump.locator = Some(SourceLocator::Text {
                            source_version_id: version_id.clone(),
                            block_id: block.id,
                            start_codepoint: 0,
                            end_codepoint: block.text.chars().count() as u32,
                            quote: block.text,
                            prefix: String::new(),
                            suffix: String::new(),
                        });
                        jump.locator_status = Some(LocatorStatus::Exact);
                    }
                } else if !trashed {
                    result.state = LinkState::BlockMissing;
                }
            }
        }
        result.source_target = Some(jump);
        Ok(result)
    }

    /// Complete occurrence manifest for export; contexts are bounded display excerpts.
    pub fn note_references(&self, note_id: &str, revision_id: &str) -> Result<Vec<BodyReference>> {
        valid_uuid(note_id)?;
        valid_uuid(revision_id)?;
        let note = self.revision(note_id, revision_id)?;
        collect_references(&note, 0, usize::MAX, None)
    }

    /// Compatibility helper for core callers. Desktop consumers use the bounded page API.
    pub fn body_backlinks(&self, target_id: &str) -> Result<Vec<BodyReference>> {
        let mut output = vec![];
        let mut cursor = None;
        loop {
            let page = self.body_backlinks_page(target_id, 100, cursor)?;
            output.extend(page.items);
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(output);
            }
        }
    }

    /// Read only source revisions represented on this page. The extra SQL row
    /// determines continuation without loading another document. Trashed sources
    /// remain explicit, and all reads share the same SQLite snapshot/generation.
    pub fn body_backlinks_page(
        &self,
        target_id: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<BodyReferencePage> {
        valid_uuid(target_id)?;
        crate::collections::validate_limit(limit)?;
        let tx = self.connection.unchecked_transaction()?;
        let (scope, generation, cursor) =
            self.reference_cursor(&tx, "backlinks", target_id, None, cursor.as_deref())?;
        let after = match cursor.map(|cursor| cursor.position) {
            Some(ReferencePosition::Backlink { rowid }) if rowid > 0 => Some(rowid),
            None => None,
            _ => return Err(Error::InvalidInput("invalid backlink cursor position")),
        };
        let mut statement = tx.prepare(
            "SELECT source_id,source_revision_id,occurrence_id,rowid,target_revision_id FROM body_links INDEXED BY body_links_target
             WHERE target_id=?1 AND rowid>?2 ORDER BY rowid LIMIT ?3",
        )?;
        let mut rows = statement
            .query_map(params![target_id, after.unwrap_or(0), limit + 1], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let mut items = Vec::with_capacity(rows.len());
        let mut start = 0;
        while start < rows.len() {
            let end = start
                + rows[start..]
                    .iter()
                    .take_while(|row| row.0 == rows[start].0)
                    .count();
            let source_rows = &rows[start..end];
            let (source, revision, _, _, _) = &source_rows[0];
            if source_rows.iter().any(|row| row.1 != *revision) {
                return Err(Error::IntegrityFailure);
            }
            let wanted = source_rows
                .iter()
                .map(|row| row.2.clone())
                .collect::<BTreeSet<_>>();
            let note = crate::notes::revision(&tx, source, revision)?;
            let mut found = collect_references(&note, 0, wanted.len(), Some(&wanted))?
                .into_iter()
                .map(|reference| (reference.occurrence_id.clone(), reference))
                .collect::<BTreeMap<_, _>>();
            for (_, _, occurrence, _, target_revision) in source_rows {
                let reference = found.remove(occurrence).ok_or(Error::IntegrityFailure)?;
                if reference.target_id != target_id
                    || reference.target_revision_id != *target_revision
                {
                    return Err(Error::IntegrityFailure);
                }
                items.push(reference);
            }
            start = end;
        }
        let next_cursor = if more {
            let rowid = rows.last().ok_or(Error::IntegrityFailure)?.3;
            Some(serde_json::to_string(&ReferenceCursor {
                scope,
                generation,
                position: ReferencePosition::Backlink { rowid },
            })?)
        } else {
            None
        };
        Ok(BodyReferencePage { items, next_cursor })
    }

    /// Immutable outgoing occurrences stay in document order. Only this bounded
    /// document is decoded; at most limit + 1 reference summaries are constructed.
    pub fn note_references_page(
        &self,
        note_id: &str,
        revision_id: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<BodyReferencePage> {
        valid_uuid(note_id)?;
        valid_uuid(revision_id)?;
        crate::collections::validate_limit(limit)?;
        let tx = self.connection.unchecked_transaction()?;
        let (scope, generation, cursor) = self.reference_cursor(
            &tx,
            "outgoing",
            note_id,
            Some(revision_id),
            cursor.as_deref(),
        )?;
        let offset = match cursor.map(|cursor| cursor.position) {
            Some(ReferencePosition::Outgoing { offset }) if offset <= 50_000 => offset,
            None => 0,
            _ => {
                return Err(Error::InvalidInput(
                    "invalid outgoing reference cursor position",
                ));
            }
        };
        let note = crate::notes::revision(&tx, note_id, revision_id)?;
        let mut items = collect_references(&note, offset, limit as usize + 1, None)?;
        let more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = if more {
            Some(serde_json::to_string(&ReferenceCursor {
                scope,
                generation,
                position: ReferencePosition::Outgoing {
                    offset: offset + items.len(),
                },
            })?)
        } else {
            None
        };
        Ok(BodyReferencePage { items, next_cursor })
    }

    /// Frontmatter relations recorded by one note, in document order.
    ///
    /// Missing and trashed targets stay out so the result uses the same live
    /// note set as `knowledge_index::related_count`. Malformed YAML and
    /// non-contract entries remain inert; they never make this read fail.
    pub fn outgoing_relations(&self, source_id: &str) -> Result<Vec<OutgoingRelation>> {
        valid_uuid(source_id)?;
        let tx = self.connection.unchecked_transaction()?;
        let source = tx
            .query_row(
                "SELECT json_extract(r.document_json,'$.content[0].attrs.source')
                 FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
                 WHERE n.id=?1 AND n.trashed=0
                   AND json_extract(r.document_json,'$.content[0].type')='frontmatter'",
                [source_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or_default();
        if source.is_empty() {
            return Ok(Vec::new());
        }
        let mut live_target =
            tx.prepare("SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1 AND trashed=0)")?;
        let mut items = Vec::new();
        for (target_id, field_key, relation_title) in crate::graph::relation_targets(&source) {
            if target_id == source_id {
                continue;
            }
            let live: bool = live_target.query_row([&target_id], |row| row.get(0))?;
            if !live {
                continue;
            }
            let context = bounded_relation_context(&format!("{field_key}: {relation_title}"));
            items.push(OutgoingRelation {
                target_id,
                field_key,
                relation_title,
                context,
            });
        }
        Ok(items)
    }

    /// Compatibility helper for core callers. Desktop consumers use the bounded page API.
    pub fn relation_backlinks(&self, target_id: &str) -> Result<Vec<RelationBacklink>> {
        let mut output = vec![];
        let mut cursor = None;
        loop {
            let page = self.relation_backlinks_page(target_id, 100, cursor)?;
            output.extend(page.items);
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(output);
            }
        }
    }

    /// Live and trashed notes whose current frontmatter carries a contract-shaped
    /// relation entry for `target_id`.
    ///
    /// Candidates are pre-filtered by the target UUID inside the frontmatter
    /// source and then parsed strictly, so a note whose YAML is malformed, whose
    /// entry is not `{kind: note, id, title}`, or that only mentions the UUID in
    /// body text stays inert instead of failing the page. Relation ids are
    /// compared as the canonical lowercase UUID, which is what the note graph
    /// matches as well. The candidate scan is bounded and reports `truncated`
    /// when the bound was reached.
    pub fn relation_backlinks_page(
        &self,
        target_id: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<RelationBacklinkPage> {
        valid_uuid(target_id)?;
        crate::collections::validate_limit(limit)?;
        let tx = self.connection.unchecked_transaction()?;
        let (scope, generation, cursor) = self.reference_cursor(
            &tx,
            "relation_backlinks",
            target_id,
            None,
            cursor.as_deref(),
        )?;
        let offset = match cursor.map(|cursor| cursor.position) {
            Some(ReferencePosition::RelationBacklink { offset })
                if offset <= MAX_RELATION_SCAN_ROWS =>
            {
                offset
            }
            None => 0,
            _ => {
                return Err(Error::InvalidInput(
                    "invalid relation backlink cursor position",
                ));
            }
        };
        // One row past the bound detects a truncated candidate scan without
        // decoding its frontmatter.
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT n.id,n.current_revision_id,n.title,n.trashed,
                        json_extract(r.document_json,'$.content[0].attrs.source')
                 FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
                 WHERE json_extract(r.document_json,'$.content[0].type')='frontmatter'
                   AND instr(json_extract(r.document_json,'$.content[0].attrs.source'),?1)>0
                 ORDER BY n.id LIMIT ?2",
            )?;
            statement
                .query_map(
                    params![target_id, (MAX_RELATION_SCAN_ROWS + 1) as i64],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        ))
                    },
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let truncated = candidates.len() > MAX_RELATION_SCAN_ROWS;
        let wanted = offset + limit as usize;
        let mut matched = Vec::new();
        for (source_id, source_revision_id, source_title, trashed, source) in
            candidates.into_iter().take(MAX_RELATION_SCAN_ROWS)
        {
            // Stop parsing once this page and its lookahead row are complete; the
            // candidates that remain stay reachable through `next_cursor`.
            if matched.len() > wanted {
                break;
            }
            for (field_key, relation_title) in relation_targets(&source, target_id) {
                let context = bounded_relation_context(&format!("{field_key}: {relation_title}"));
                matched.push(RelationBacklink {
                    source_id: source_id.clone(),
                    source_revision_id: source_revision_id.clone(),
                    source_title: source_title.clone(),
                    source_trashed: trashed != 0,
                    field_key,
                    relation_title,
                    context,
                });
            }
        }
        let more = matched.len() > wanted;
        let items = matched
            .into_iter()
            .skip(offset)
            .take(limit as usize)
            .collect::<Vec<_>>();
        let next_cursor = if more {
            Some(serde_json::to_string(&ReferenceCursor {
                scope,
                generation,
                position: ReferencePosition::RelationBacklink {
                    offset: offset + items.len(),
                },
            })?)
        } else {
            None
        };
        Ok(RelationBacklinkPage {
            items,
            next_cursor,
            truncated,
            scan_limit: MAX_RELATION_SCAN_ROWS as u32,
        })
    }

    fn reference_cursor(
        &self,
        connection: &Connection,
        kind: &str,
        note_id: &str,
        revision_id: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<(String, i64, Option<ReferenceCursor>)> {
        let workspace: String = connection.query_row(
            "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let scope = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                1,
                self.database_path.to_string_lossy(),
                workspace,
                kind,
                note_id,
                revision_id,
            ))?)
        );
        let generation: i64 = connection.query_row(
            "SELECT generation FROM note_list_state WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let cursor: Option<ReferenceCursor> = crate::collections::decode_cursor(cursor)?;
        if let Some(cursor) = &cursor {
            if cursor.scope != scope {
                return Err(Error::InvalidInput(
                    "reference cursor belongs to a different query or workspace",
                ));
            }
            if cursor.generation != generation {
                return Err(Error::QueryExpired);
            }
        }
        Ok((scope, generation, cursor))
    }
}

fn collect_references(
    note: &Note,
    skip: usize,
    take: usize,
    wanted: Option<&BTreeSet<String>>,
) -> Result<Vec<BodyReference>> {
    let mut output = Vec::new();
    let mut contexts = BTreeMap::new();
    let mut seen = 0;
    visit_references(&note.document.content, None, &mut |node, parent| {
        let occurrence_id = node
            .attrs
            .occurrence_id
            .as_ref()
            .ok_or(Error::IntegrityFailure)?;
        if wanted.is_some_and(|ids| !ids.contains(occurrence_id)) {
            return Ok(true);
        }
        seen += 1;
        if seen <= skip {
            return Ok(true);
        }
        let context = contexts
            .entry(parent.attrs.id.as_str())
            .or_insert_with(|| context_excerpt(parent));
        output.push(BodyReference {
            source_id: note.id.clone(),
            source_revision_id: note.revision_id.clone(),
            source_title: note.title.clone(),
            source_trashed: note.trashed,
            source_block_id: parent.attrs.id.clone(),
            occurrence_id: occurrence_id.clone(),
            target_id: node
                .attrs
                .target_id
                .clone()
                .ok_or(Error::IntegrityFailure)?,
            target_block_id: node.attrs.target_block_id.clone(),
            target_revision_id: node.attrs.target_revision_id.clone(),
            label: node
                .attrs
                .label
                .clone()
                .or_else(|| node.attrs.wiki_target.clone())
                .unwrap_or_default(),
            context: context.clone(),
        });
        Ok(output.len() < take)
    })?;
    Ok(output)
}

fn visit_references<'a>(
    nodes: &'a [DocumentNode],
    parent: Option<&'a DocumentNode>,
    visit: &mut impl FnMut(&'a DocumentNode, &'a DocumentNode) -> Result<bool>,
) -> Result<bool> {
    for node in nodes {
        let source_block = if node.attrs.id.is_empty() {
            parent.unwrap_or(node)
        } else {
            node
        };
        if node.is_body_reference() && !visit(node, source_block)? {
            return Ok(false);
        }
        if !visit_references(&node.content, Some(node), visit)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Stop traversal after 281 Unicode scalar values; never materialize a whole
/// paragraph just to truncate it. Cached once per source block by the collector.
fn context_excerpt(node: &DocumentNode) -> String {
    fn append(node: &DocumentNode, output: &mut Vec<char>) {
        if matches!(
            node.kind,
            NodeType::Comment | NodeType::CommentBlock | NodeType::Frontmatter
        ) {
            return;
        }
        if output.len() > CONTEXT_CHARS {
            return;
        }
        let text = node.text.as_deref().or(match node.kind {
            NodeType::HardBreak => Some("\n"),
            NodeType::NoteReference => node.attrs.label.as_deref(),
            NodeType::Image => node.attrs.alt.as_deref(),
            NodeType::WikiLink | NodeType::WikiEmbed => node
                .attrs
                .label
                .as_deref()
                .or(node.attrs.wiki_target.as_deref()),
            NodeType::InlineMath | NodeType::BlockMath => node.attrs.source.as_deref(),
            _ => None,
        });
        if let Some(text) = text {
            output.extend(text.chars().take(CONTEXT_CHARS + 1 - output.len()));
        } else {
            for child in &node.content {
                append(child, output);
                if output.len() > CONTEXT_CHARS {
                    break;
                }
            }
        }
    }
    let mut output = Vec::with_capacity(CONTEXT_CHARS + 1);
    append(node, &mut output);
    if output.len() > CONTEXT_CHARS {
        output.truncate(CONTEXT_CHARS - 1);
        output.push('…');
    }
    output.into_iter().collect()
}

/// Contract-shaped relation entries in one frontmatter source that point at
/// `target_id`, as `(field key, recorded title)` pairs in document order.
///
/// Extensible YAML stays inert: malformed YAML, non-list fields, unrelated
/// records and over-long field keys are skipped instead of failing the query.
fn relation_targets(source: &str, target_id: &str) -> Vec<(String, String)> {
    let Ok(serde_yaml_ng::Value::Mapping(mapping)) =
        serde_yaml_ng::from_str::<serde_yaml_ng::Value>(source)
    else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for (key, value) in mapping.iter() {
        let serde_yaml_ng::Value::String(key) = key else {
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || key.chars().count() > MAX_RELATION_FIELD_CHARS
            || key.chars().any(char::is_control)
        {
            continue;
        }
        let serde_yaml_ng::Value::Sequence(items) = value else {
            continue;
        };
        for item in items {
            let serde_yaml_ng::Value::Mapping(entry) = item else {
                continue;
            };
            let Some(serde_yaml_ng::Value::String(kind)) = yaml_field(entry, "kind") else {
                continue;
            };
            let Some(serde_yaml_ng::Value::String(note_id)) = yaml_field(entry, "id") else {
                continue;
            };
            let Some(serde_yaml_ng::Value::String(title)) = yaml_field(entry, "title") else {
                continue;
            };
            let title = title.trim();
            if kind != "note"
                || note_id != target_id
                || title.is_empty()
                || title.chars().count() > 300
            {
                continue;
            }
            targets.push((key.to_owned(), title.to_owned()));
        }
    }
    targets
}

fn yaml_field<'a>(
    mapping: &'a serde_yaml_ng::Mapping,
    key: &str,
) -> Option<&'a serde_yaml_ng::Value> {
    mapping.iter().find_map(|(candidate, value)| {
        matches!(candidate, serde_yaml_ng::Value::String(candidate) if candidate == key)
            .then_some(value)
    })
}

/// Bound one relation display excerpt to 281 Unicode scalar values of work and
/// at most 280 stored, matching the body-reference context contract.
fn bounded_relation_context(value: &str) -> String {
    let mut chars = value.chars();
    let mut output: String = chars.by_ref().take(CONTEXT_CHARS).collect();
    if chars.next().is_some() {
        output.pop();
        output.push('…');
    }
    output
}
