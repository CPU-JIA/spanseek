//! Controlled tidy-up proposals, per-item application and the audit log.
//!
//! A change set is generated from existing local data only. Reading a proposal
//! never writes; each row is applied through `apply_change_item`, which checks
//! the original revision/target again before writing and records an audit event
//! in the same transaction as the change.
use crate::{
    ApplyChangeItem, AuditEvent, AuditPage, AuditPageRequest, AuditSubjectKind, AuditUndoCommit,
    ChangeEvidence, ChangeItem, ChangeItemCommit, ChangeItemKind, ChangeItemStatus, ChangePayload,
    ChangeScope, ChangeSet, ChangeSetCommit, ChangeSetKind, ChangeSetPage, ChangeSetPageRequest,
    ChangeSetStatus, ChangeSetSummary, CreateChangeSet, Error, Library, MAX_AUDIT_PAGE,
    MAX_CHANGESET_ITEMS, MAX_CHANGESET_PAGE, NodeAttributes, NodeType, Note, NoteDocument,
    RejectChangeSet, Result, StaleReason, StaleSourceFinding, StaleSourceReport, UndoAuditEvent,
    document::{DocumentNode, valid_uuid},
    mutations::{next_receipt, replay, request_hash, store_receipt},
    notes::{Prepared, checked_head, commit_revision, revision as note_revision},
    organization::{bump_version, normalize_name, read_metadata},
};
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use uuid::Uuid;

const CHANGE_TTL_DAYS: i64 = 7;
const DEFAULT_STALE_DAYS: u32 = 365;
const MAX_RELATION_NOTES: usize = 200;
const MAX_TAG_MATCHES_PER_NOTE: usize = 5;
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "from", "are", "was", "not", "you", "的", "了",
    "和", "是", "在", "有", "与", "及", "或", "对", "为", "一个", "我们", "可以",
];

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn excerpt_around(text: &str, needle: &str, radius: usize) -> String {
    let lower = text.to_lowercase();
    let needle = needle.to_lowercase();
    let Some(byte) = lower.find(&needle) else {
        return text.chars().take(radius * 2).collect();
    };
    let position = lower[..byte].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = position.saturating_sub(radius);
    let end = (position + needle.chars().count() + radius).min(chars.len());
    chars[start..end]
        .iter()
        .collect::<String>()
        .trim()
        .to_owned()
}

fn normalized_text(note: &Note) -> String {
    let text = format!("{}\n{}", note.title, note.document.plain_text());
    crate::chinese::fold(&text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn content_fingerprint(note: &Note) -> String {
    format!("{:x}", Sha256::digest(normalized_text(note).as_bytes()))
}

fn significant_terms(note: &Note) -> BTreeSet<String> {
    crate::tokens(&format!("{}\n{}", note.title, note.document.plain_text()))
        .into_iter()
        .filter(|term| term.chars().count() >= 2 && !STOPWORDS.contains(&term.as_str()))
        .collect()
}

fn active_note_ids(
    connection: &Connection,
    collection_id: Option<&str>,
    limit: u32,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT n.id FROM notes n JOIN objects o ON o.id=n.id
         WHERE n.trashed=0 AND o.archived=0
           AND (?1 IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.collection_id=?1 AND m.member_id=n.id))
         ORDER BY n.updated_at DESC,n.id LIMIT ?2",
    )?;
    Ok(statement
        .query_map(params![collection_id, limit], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn assigned_tags(connection: &Connection, object_id: &str) -> Result<BTreeSet<String>> {
    let mut statement = connection.prepare(
        "SELECT t.normalized_name FROM tags t JOIN object_tags ot ON ot.tag_id=t.id WHERE ot.object_id=?1",
    )?;
    Ok(statement
        .query_map([object_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?)
}

fn linked(connection: &Connection, a: &str, b: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM body_links WHERE (source_id=?1 AND target_id=?2) OR (source_id=?2 AND target_id=?1))",
        params![a, b],
        |row| row.get::<_, bool>(0),
    )?)
}

struct NewItem {
    kind: ChangeItemKind,
    target_object_id: String,
    target_title: String,
    target_revision_id: Option<String>,
    payload: ChangePayload,
    evidence: Vec<ChangeEvidence>,
}

fn read_tag_rows(connection: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement = connection
        .prepare("SELECT id,display_name,normalized_name FROM tags ORDER BY normalized_name")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn note_revision_by_id(connection: &Connection, id: &str) -> Result<Note> {
    let revision_id: String = connection
        .query_row(
            "SELECT current_revision_id FROM notes WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    note_revision(connection, id, &revision_id)
}

fn generate_tag_items(connection: &Connection, request: &CreateChangeSet) -> Result<Vec<NewItem>> {
    let tags = read_tag_rows(connection)?;
    if tags.is_empty() {
        return Ok(vec![]);
    }
    let notes = active_note_ids(
        connection,
        request.collection_id.as_deref(),
        MAX_CHANGESET_ITEMS,
    )?;
    let mut items = Vec::new();
    for note_id in notes {
        let note = note_revision_by_id(connection, &note_id)?;
        let text = format!("{}\n{}", note.title, note.document.plain_text());
        let folded = text.to_lowercase();
        let assigned = assigned_tags(connection, &note_id)?;
        let mut matches = 0usize;
        for (_, display_name, normalized_name) in &tags {
            if matches >= MAX_TAG_MATCHES_PER_NOTE || assigned.contains(normalized_name) {
                continue;
            }
            if normalized_name.chars().count() < 2 || !folded.contains(normalized_name.as_str()) {
                continue;
            }
            let excerpt = excerpt_around(&text, display_name, 40);
            if excerpt.is_empty() {
                continue;
            }
            items.push(NewItem {
                kind: ChangeItemKind::AssignTag,
                target_object_id: note_id.clone(),
                target_title: note.title.clone(),
                target_revision_id: Some(note.revision_id.clone()),
                payload: ChangePayload::AssignTag {
                    tag: display_name.clone(),
                    matched_text: normalized_name.clone(),
                    field: if note.title.to_lowercase().contains(normalized_name.as_str()) {
                        "title".into()
                    } else {
                        "body".into()
                    },
                },
                evidence: vec![ChangeEvidence {
                    object_id: note_id.clone(),
                    revision_id: Some(note.revision_id.clone()),
                    field: "text".into(),
                    excerpt,
                }],
            });
            matches += 1;
        }
        if items.len() >= request.limit as usize {
            break;
        }
    }
    items.truncate(request.limit as usize);
    Ok(items)
}

fn generate_duplicate_items(
    connection: &Connection,
    request: &CreateChangeSet,
) -> Result<Vec<NewItem>> {
    let ids = active_note_ids(
        connection,
        request.collection_id.as_deref(),
        MAX_CHANGESET_ITEMS,
    )?;
    let mut by_hash: BTreeMap<String, Vec<Note>> = BTreeMap::new();
    let mut by_fingerprint: BTreeMap<String, Vec<Note>> = BTreeMap::new();
    for id in ids {
        let note = note_revision_by_id(connection, &id)?;
        by_hash
            .entry(note.content_hash.clone())
            .or_default()
            .push(note.clone());
        by_fingerprint
            .entry(content_fingerprint(&note))
            .or_default()
            .push(note);
    }
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let groups = by_hash
        .values()
        .chain(by_fingerprint.values())
        .collect::<Vec<_>>();
    for group in groups {
        if group.len() < 2 {
            continue;
        }
        let mut sorted = group.clone();
        sorted.sort_by(|a, b| (&a.updated_at, &a.id).cmp(&(&b.updated_at, &b.id)));
        let keep = &sorted[0];
        for duplicate in sorted.iter().skip(1) {
            if !seen.insert((keep.id.clone(), duplicate.id.clone())) {
                continue;
            }
            let similarity = if keep.content_hash == duplicate.content_hash {
                10_000
            } else {
                9_800
            };
            items.push(NewItem {
                kind: ChangeItemKind::TrashDuplicate,
                target_object_id: keep.id.clone(),
                target_title: keep.title.clone(),
                target_revision_id: Some(keep.revision_id.clone()),
                payload: ChangePayload::TrashDuplicate {
                    duplicate_id: duplicate.id.clone(),
                    duplicate_title: duplicate.title.clone(),
                    similarity_basis_points: similarity,
                    reason: if similarity == 10_000 {
                        "identical_content_hash".into()
                    } else {
                        "identical_normalized_content".into()
                    },
                },
                evidence: vec![
                    ChangeEvidence {
                        object_id: keep.id.clone(),
                        revision_id: Some(keep.revision_id.clone()),
                        field: "content_hash".into(),
                        excerpt: keep.content_hash.clone(),
                    },
                    ChangeEvidence {
                        object_id: duplicate.id.clone(),
                        revision_id: Some(duplicate.revision_id.clone()),
                        field: "content_hash".into(),
                        excerpt: duplicate.content_hash.clone(),
                    },
                ],
            });
            if items.len() >= request.limit as usize {
                return Ok(items);
            }
        }
    }
    Ok(items)
}

fn generate_relation_items(
    connection: &Connection,
    request: &CreateChangeSet,
) -> Result<Vec<NewItem>> {
    let ids = active_note_ids(
        connection,
        request.collection_id.as_deref(),
        MAX_RELATION_NOTES as u32,
    )?;
    let mut notes = Vec::new();
    for id in ids {
        let note = note_revision_by_id(connection, &id)?;
        let tags = assigned_tags(connection, &id)?;
        let terms = significant_terms(&note);
        notes.push((note, tags, terms));
    }
    let mut candidates = Vec::new();
    for i in 0..notes.len() {
        for j in (i + 1)..notes.len() {
            let (left, left_tags, left_terms) = &notes[i];
            let (right, right_tags, right_terms) = &notes[j];
            if linked(connection, &left.id, &right.id)? {
                continue;
            }
            let shared_tags: Vec<String> = left_tags.intersection(right_tags).cloned().collect();
            let shared_terms: Vec<String> = left_terms.intersection(right_terms).cloned().collect();
            let score = shared_tags.len() as u32 * 4 + shared_terms.len().min(10) as u32;
            if shared_tags.len() < 2 && shared_terms.len() < 3 {
                continue;
            }
            candidates.push((score, left, right, shared_tags, shared_terms));
        }
    }
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| (&a.1.id, &a.2.id).cmp(&(&b.1.id, &b.2.id)))
    });
    let mut items = Vec::new();
    for (_, source, target, shared_tags, shared_terms) in candidates {
        items.push(NewItem {
            kind: ChangeItemKind::AppendRelation,
            target_object_id: source.id.clone(),
            target_title: source.title.clone(),
            target_revision_id: Some(source.revision_id.clone()),
            payload: ChangePayload::AppendRelation {
                target_id: target.id.clone(),
                target_title: target.title.clone(),
                shared_tags: shared_tags.clone(),
                shared_terms: shared_terms.clone(),
            },
            evidence: vec![
                ChangeEvidence {
                    object_id: source.id.clone(),
                    revision_id: Some(source.revision_id.clone()),
                    field: "shared_tags".into(),
                    excerpt: if shared_tags.is_empty() {
                        "—".into()
                    } else {
                        shared_tags.join(", ")
                    },
                },
                ChangeEvidence {
                    object_id: target.id.clone(),
                    revision_id: Some(target.revision_id.clone()),
                    field: "shared_terms".into(),
                    excerpt: shared_terms
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                },
            ],
        });
        if items.len() >= request.limit as usize {
            break;
        }
    }
    Ok(items)
}

fn generate_stale_items(
    connection: &Connection,
    request: &CreateChangeSet,
    as_of: chrono::DateTime<Utc>,
) -> Result<Vec<NewItem>> {
    let window = request
        .stale_after_days
        .unwrap_or(DEFAULT_STALE_DAYS)
        .clamp(1, 3650);
    let mut statement = connection.prepare(
        "SELECT s.id,s.display_title,v.captured_at,v.origin_url
         FROM sources s JOIN objects o ON o.id=s.id JOIN source_versions v ON v.id=s.current_version_id
         WHERE o.trashed=0 ORDER BY s.updated_at ASC,s.id LIMIT ?1",
    )?;
    let rows = statement
        .query_map([MAX_CHANGESET_ITEMS], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let current = as_of;
    let mut items = Vec::new();
    for (id, title, captured_at, origin_url) in rows {
        let age_days = chrono::DateTime::parse_from_rfc3339(&captured_at)
            .ok()
            .map(|at| (current - at.with_timezone(&Utc)).num_days().max(0) as u32)
            .unwrap_or(u32::MAX);
        let reason = if age_days == u32::MAX {
            Some(StaleReason::MissingVersion)
        } else if origin_url.as_ref().is_none_or(|url| url.trim().is_empty()) {
            (age_days > window.min(30)).then_some(StaleReason::NoOrigin)
        } else if age_days > window {
            Some(StaleReason::OlderThanWindow)
        } else {
            None
        };
        let Some(reason) = reason else {
            continue;
        };
        let reason_text = match reason {
            StaleReason::OlderThanWindow => format!("捕获时间已超过 {window} 天"),
            StaleReason::NoOrigin => "来源没有可回查的原始地址".to_owned(),
            StaleReason::MissingVersion => "当前版本捕获时间无法解析".to_owned(),
        };
        items.push(NewItem {
            kind: ChangeItemKind::AcknowledgeStale,
            target_object_id: id.clone(),
            target_title: title,
            target_revision_id: None,
            payload: ChangePayload::StaleSource {
                age_days: if age_days == u32::MAX { 0 } else { age_days },
                captured_at: captured_at.clone(),
                origin_url: origin_url.clone(),
                reason: reason_text,
            },
            evidence: vec![ChangeEvidence {
                object_id: id,
                revision_id: None,
                field: "source_version".into(),
                excerpt: format!(
                    "captured_at={captured_at}; origin={}",
                    origin_url.unwrap_or_else(|| "—".into())
                ),
            }],
        });
        if items.len() >= request.limit as usize {
            break;
        }
    }
    Ok(items)
}
fn insert_change_set(
    connection: &Connection,
    id: &str,
    request: &CreateChangeSet,
    items: &[NewItem],
    created_at: &str,
) -> Result<()> {
    let scope = ChangeScope {
        collection_id: request.collection_id.clone(),
        limit: request.limit,
        stale_after_days: request.stale_after_days,
    };
    connection.execute(
        "INSERT INTO automation_changesets(id,kind,status,scope_json,item_count,applied_count,row_version,created_at,updated_at) VALUES(?1,?2,'pending',?3,?4,0,1,?5,?5)",
        params![id, request.kind.as_str(), serde_json::to_string(&scope)?, items.len() as u32, created_at],
    )?;
    for (ordinal, item) in items.iter().enumerate() {
        connection.execute(
            "INSERT INTO automation_items(change_id,item_id,ordinal,kind,target_object_id,target_title,target_revision_id,payload_json,evidence_json,requires_confirmation,status,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending',?11)",
            params![
                id,
                Uuid::new_v4().to_string(),
                ordinal as u32,
                item.kind.as_str(),
                item.target_object_id,
                item.target_title,
                item.target_revision_id,
                serde_json::to_string(&item.payload)?,
                serde_json::to_string(&item.evidence)?,
                item.kind.requires_confirmation(),
                created_at
            ],
        )?;
    }
    Ok(())
}

fn read_change_set(connection: &Connection, id: &str) -> Result<ChangeSet> {
    valid_uuid(id)?;
    let row = connection
        .query_row(
            "SELECT kind,status,scope_json,item_count,applied_count,row_version,created_at,updated_at FROM automation_changesets WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(ChangeSet {
        id: id.into(),
        kind: parse_change_kind(&row.0)?,
        status: parse_change_status(&row.1)?,
        scope: serde_json::from_str(&row.2)?,
        item_count: row.3,
        applied_count: row.4,
        row_version: row.5,
        created_at: row.6,
        updated_at: row.7,
        items: read_change_items(connection, id)?,
    })
}

fn parse_change_kind(value: &str) -> Result<ChangeSetKind> {
    Ok(match value {
        "tag_suggestions" => ChangeSetKind::TagSuggestions,
        "duplicate_candidates" => ChangeSetKind::DuplicateCandidates,
        "relation_suggestions" => ChangeSetKind::RelationSuggestions,
        "stale_sources" => ChangeSetKind::StaleSources,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn parse_change_status(value: &str) -> Result<ChangeSetStatus> {
    Ok(match value {
        "pending" => ChangeSetStatus::Pending,
        "applied" => ChangeSetStatus::Applied,
        "rejected" => ChangeSetStatus::Rejected,
        "expired" => ChangeSetStatus::Expired,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn parse_item_kind(value: &str) -> Result<ChangeItemKind> {
    Ok(match value {
        "assign_tag" => ChangeItemKind::AssignTag,
        "trash_duplicate" => ChangeItemKind::TrashDuplicate,
        "append_relation" => ChangeItemKind::AppendRelation,
        "acknowledge_stale" => ChangeItemKind::AcknowledgeStale,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn parse_item_status(value: &str) -> Result<ChangeItemStatus> {
    Ok(match value {
        "pending" => ChangeItemStatus::Pending,
        "applied" => ChangeItemStatus::Applied,
        "skipped" => ChangeItemStatus::Skipped,
        "failed" => ChangeItemStatus::Failed,
        _ => return Err(Error::IntegrityFailure),
    })
}

fn item_status_str(status: ChangeItemStatus) -> &'static str {
    match status {
        ChangeItemStatus::Pending => "pending",
        ChangeItemStatus::Applied => "applied",
        ChangeItemStatus::Skipped => "skipped",
        ChangeItemStatus::Failed => "failed",
    }
}

fn read_change_items(connection: &Connection, change_id: &str) -> Result<Vec<ChangeItem>> {
    let mut statement = connection.prepare(
        "SELECT item_id,ordinal,kind,target_object_id,target_title,target_revision_id,payload_json,evidence_json,requires_confirmation,status,blocked_reason,audit_id,updated_at
         FROM automation_items WHERE change_id=?1 ORDER BY ordinal,item_id",
    )?;
    let rows = statement
        .query_map([change_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, bool>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, String>(12)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|row| {
            Ok(ChangeItem {
                item_id: row.0,
                ordinal: row.1,
                kind: parse_item_kind(&row.2)?,
                target_object_id: row.3,
                target_title: row.4,
                target_revision_id: row.5,
                payload: serde_json::from_str(&row.6)?,
                evidence: serde_json::from_str(&row.7)?,
                requires_confirmation: row.8,
                status: parse_item_status(&row.9)?,
                blocked_reason: row.10,
                audit_id: row.11,
                updated_at: row.12,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn item_by_id(connection: &Connection, change_id: &str, item_id: &str) -> Result<ChangeItem> {
    read_change_items(connection, change_id)?
        .into_iter()
        .find(|item| item.item_id == item_id)
        .ok_or(Error::NotFound)
}

fn subject_kind_str(value: AuditSubjectKind) -> &'static str {
    match value {
        AuditSubjectKind::Note => "note",
        AuditSubjectKind::Source => "source",
        AuditSubjectKind::ChangeSet => "change_set",
        AuditSubjectKind::ResearchTask => "research_task",
        AuditSubjectKind::ReviewSetting => "review_setting",
        AuditSubjectKind::Tag => "tag",
    }
}

fn parse_subject_kind(value: &str) -> Result<AuditSubjectKind> {
    Ok(match value {
        "note" => AuditSubjectKind::Note,
        "source" => AuditSubjectKind::Source,
        "change_set" => AuditSubjectKind::ChangeSet,
        "research_task" => AuditSubjectKind::ResearchTask,
        "review_setting" => AuditSubjectKind::ReviewSetting,
        "tag" => AuditSubjectKind::Tag,
        _ => return Err(Error::IntegrityFailure),
    })
}

pub(crate) struct NewAudit<'a> {
    pub kind: &'a str,
    pub subject_kind: AuditSubjectKind,
    pub subject_id: &'a str,
    pub summary: String,
    pub origin: &'a str,
    pub undo: Option<Value>,
    pub change_id: Option<&'a str>,
    pub item_id: Option<&'a str>,
}

pub(crate) fn insert_audit(connection: &Connection, event: NewAudit<'_>) -> Result<AuditEvent> {
    let id = Uuid::new_v4().to_string();
    let occurred_at = now();
    let undo_json = event.undo.as_ref().map(serde_json::to_string).transpose()?;
    connection.execute(
        "INSERT INTO audit_events(id,occurred_at,kind,subject_kind,subject_id,summary,origin,undoable,undo_json,change_id,item_id)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            id,
            occurred_at,
            event.kind,
            subject_kind_str(event.subject_kind),
            event.subject_id,
            &event.summary,
            event.origin,
            event.undo.is_some(),
            undo_json,
            event.change_id,
            event.item_id
        ],
    )?;
    let seq = connection.last_insert_rowid();
    Ok(AuditEvent {
        seq,
        id,
        occurred_at,
        kind: event.kind.into(),
        subject_kind: event.subject_kind,
        subject_id: event.subject_id.into(),
        summary: event.summary,
        origin: event.origin.into(),
        undoable: event.undo.is_some(),
        undone: false,
        undone_at: None,
        change_id: event.change_id.map(Into::into),
        item_id: event.item_id.map(Into::into),
    })
}

pub(crate) fn read_audit_by_id(
    connection: &Connection,
    id: &str,
) -> Result<(AuditEvent, Option<Value>)> {
    let row = connection
        .query_row(
            "SELECT seq,id,occurred_at,kind,subject_kind,subject_id,summary,origin,undoable,undone_at,change_id,item_id,undo_json FROM audit_events WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let undo = row.12.as_deref().map(serde_json::from_str).transpose()?;
    Ok((
        AuditEvent {
            seq: row.0,
            id: row.1,
            occurred_at: row.2,
            kind: row.3,
            subject_kind: parse_subject_kind(&row.4)?,
            subject_id: row.5,
            summary: row.6,
            origin: row.7,
            undoable: row.8,
            undone: row.9.is_some(),
            undone_at: row.9,
            change_id: row.10,
            item_id: row.11,
        },
        undo,
    ))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error::IntegrityFailure)
}

fn read_active_note(connection: &Connection, id: &str) -> Result<Note> {
    let note = note_revision_by_id(connection, id)?;
    if note.trashed {
        return Err(Error::Trashed);
    }
    Ok(note)
}

fn change_is_expired(change: &ChangeSet) -> bool {
    chrono::DateTime::parse_from_rfc3339(&change.created_at)
        .ok()
        .is_some_and(|created| {
            Utc::now() - created.with_timezone(&Utc) > Duration::days(CHANGE_TTL_DAYS)
        })
}

fn mark_item(
    connection: &Connection,
    change_id: &str,
    item_id: &str,
    status: ChangeItemStatus,
    blocked_reason: Option<&str>,
    audit_id: Option<&str>,
) -> Result<()> {
    connection.execute(
        "UPDATE automation_items SET status=?3,blocked_reason=?4,audit_id=?5,updated_at=?6 WHERE change_id=?1 AND item_id=?2",
        params![change_id, item_id, item_status_str(status), blocked_reason, audit_id, now()],
    )?;
    Ok(())
}

fn refresh_change_status(connection: &Connection, change_id: &str) -> Result<()> {
    let pending: u32 = connection.query_row(
        "SELECT COUNT(*) FROM automation_items WHERE change_id=?1 AND status='pending'",
        [change_id],
        |row| row.get(0),
    )?;
    let applied: u32 = connection.query_row(
        "SELECT COUNT(*) FROM automation_items WHERE change_id=?1 AND status='applied'",
        [change_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "UPDATE automation_changesets SET status=?2,applied_count=?3,row_version=row_version+1,updated_at=?4 WHERE id=?1",
        params![
            change_id,
            if pending == 0 { "applied" } else { "pending" },
            applied,
            now()
        ],
    )?;
    Ok(())
}
/// A dictionary edit invalidates suggestions based on its previous name, including
/// legacy payloads that predate tag IDs. Keep the original evidence in history.
pub(crate) fn invalidate_tag_suggestions(
    connection: &Connection,
    normalized_name: &str,
) -> Result<()> {
    let candidates = connection
        .prepare(
            "SELECT i.change_id,i.item_id,i.payload_json FROM automation_items i
         JOIN automation_changesets c ON c.id=i.change_id
         WHERE i.kind='assign_tag' AND i.status='pending' AND c.status='pending'",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut changed = BTreeSet::new();
    for (change_id, item_id, payload) in candidates {
        let ChangePayload::AssignTag { tag, .. } = serde_json::from_str(&payload)? else {
            return Err(Error::IntegrityFailure);
        };
        if normalize_name(&tag)?.1 == normalized_name {
            mark_item(
                connection,
                &change_id,
                &item_id,
                ChangeItemStatus::Skipped,
                Some("tag_changed"),
                None,
            )?;
            changed.insert(change_id);
        }
    }
    for change_id in changed {
        refresh_change_status(connection, &change_id)?;
    }
    Ok(())
}

fn apply_assign_tag(connection: &Connection, item: &ChangeItem) -> Result<Option<Value>> {
    let ChangePayload::AssignTag { tag, .. } = &item.payload else {
        return Err(Error::IntegrityFailure);
    };
    let (_, normalized) = normalize_name(tag)?;
    let object = read_metadata(connection, &item.target_object_id)?;
    if object.trashed {
        return Err(Error::Trashed);
    }
    // Suggestions only use an existing dictionary entry. Never recreate an old
    // name after the user removed or renamed it while a proposal was open.
    let tag_id: String = connection
        .query_row(
            "SELECT id FROM tags WHERE normalized_name=?1",
            [&normalized],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::InvalidInput("automation_tag_changed"))?;
    let inserted = connection.execute(
        "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2) ON CONFLICT(object_id,tag_id) DO NOTHING",
        params![item.target_object_id, tag_id],
    )? == 1;
    if !inserted {
        return Ok(None);
    }
    bump_version(connection, &object)?;
    Ok(Some(json!({
        "kind": "assign_tag",
        "object_id": item.target_object_id,
        "tag_id": tag_id,
    })))
}

fn apply_trash_duplicate(connection: &Connection, item: &ChangeItem) -> Result<Option<Value>> {
    let ChangePayload::TrashDuplicate { duplicate_id, .. } = &item.payload else {
        return Err(Error::IntegrityFailure);
    };
    let keep = read_active_note(connection, &item.target_object_id)?;
    let duplicate = read_active_note(connection, duplicate_id)?;
    if content_fingerprint(&keep) != content_fingerprint(&duplicate) {
        return Err(Error::InvalidInput("automation_target_changed"));
    }
    read_metadata(connection, duplicate_id)?;
    connection.execute(
        "UPDATE objects SET trashed=1,row_version=row_version+1 WHERE id=?1 AND trashed=0",
        [duplicate_id],
    )?;
    connection.execute("UPDATE notes SET trashed=1 WHERE id=?1", [duplicate_id])?;
    Ok(Some(json!({
        "kind": "trash_duplicate",
        "object_id": duplicate_id,
        "keep_object_id": item.target_object_id,
    })))
}

fn append_relation(note: &Note, target_id: &str, target_title: &str) -> Result<NoteDocument> {
    if note.schema_version < 3 {
        return Err(Error::InvalidInput("relation_requires_schema_3"));
    }
    let mut document = note.document.clone();
    document.content.push(DocumentNode {
        kind: NodeType::Paragraph,
        attrs: NodeAttributes {
            id: Uuid::new_v4().to_string(),
            ..Default::default()
        },
        content: vec![
            DocumentNode {
                kind: NodeType::Text,
                attrs: Default::default(),
                content: vec![],
                text: Some("相关：".into()),
                marks: vec![],
            },
            DocumentNode {
                kind: NodeType::WikiLink,
                attrs: NodeAttributes {
                    wiki_target: Some(target_title.to_owned()),
                    label: Some(target_title.to_owned()),
                    occurrence_id: Some(Uuid::new_v4().to_string()),
                    target_id: Some(target_id.to_owned()),
                    ..Default::default()
                },
                content: vec![],
                text: None,
                marks: vec![],
            },
        ],
        text: None,
        marks: vec![],
    });
    document.validate(3)?;
    Ok(document)
}

fn apply_append_relation(connection: &Connection, item: &ChangeItem) -> Result<Option<Value>> {
    let ChangePayload::AppendRelation {
        target_id,
        target_title,
        ..
    } = &item.payload
    else {
        return Err(Error::IntegrityFailure);
    };
    let expected = item
        .target_revision_id
        .as_deref()
        .ok_or(Error::IntegrityFailure)?;
    let note = checked_head(connection, &item.target_object_id, expected)?;
    if note.trashed {
        return Err(Error::Trashed);
    }
    if linked(connection, &note.id, target_id)? {
        return Err(Error::InvalidInput("automation_relation_already_linked"));
    }
    read_active_note(connection, target_id)?;
    let document = append_relation(&note, target_id, target_title)?;
    let prepared = Prepared::new(&note.title, 3, document, None)?;
    let updated = commit_revision(connection, &note.id, Some(&note), prepared, "ai_proposal")?;
    Ok(Some(json!({
        "kind": "append_relation",
        "object_id": note.id,
        "previous_revision_id": note.revision_id,
        "resulting_revision_id": updated.revision_id,
        "target_id": target_id,
    })))
}
impl Library {
    /// Builds a reviewable proposal. No knowledge object is modified here.
    pub fn create_change_set(
        &mut self,
        command_id: &str,
        request: CreateChangeSet,
    ) -> Result<ChangeSetCommit> {
        let kind = "automation.create_change_set";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        if !(1..=MAX_CHANGESET_ITEMS).contains(&request.limit) {
            return Err(Error::InvalidInput("change set limit is out of range"));
        }
        if request.collection_id.is_some() {
            valid_uuid(request.collection_id.as_deref().unwrap_or_default())?;
        }
        let items = match request.kind {
            ChangeSetKind::TagSuggestions => generate_tag_items(&tx, &request)?,
            ChangeSetKind::DuplicateCandidates => generate_duplicate_items(&tx, &request)?,
            ChangeSetKind::RelationSuggestions => generate_relation_items(&tx, &request)?,
            ChangeSetKind::StaleSources => generate_stale_items(&tx, &request, Utc::now())?,
        };
        let id = Uuid::new_v4().to_string();
        let created_at = now();
        insert_change_set(&tx, &id, &request, &items, &created_at)?;
        let change = read_change_set(&tx, &id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ChangeSetCommit { change, receipt };
        store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn change_set(&self, id: &str) -> Result<ChangeSet> {
        read_change_set(&self.connection, id)
    }

    pub fn change_sets_page(&self, request: ChangeSetPageRequest) -> Result<ChangeSetPage> {
        if !(1..=MAX_CHANGESET_PAGE).contains(&request.limit) {
            return Err(Error::InvalidInput("change set page limit is out of range"));
        }
        let kind = request.kind.map(|kind| kind.as_str());
        let status = request.status.map(|status| match status {
            ChangeSetStatus::Pending => "pending",
            ChangeSetStatus::Applied => "applied",
            ChangeSetStatus::Rejected => "rejected",
            ChangeSetStatus::Expired => "expired",
        });
        let before = request
            .before_id
            .as_deref()
            .map(|id| -> Result<(String, String)> {
                let created: String = self.connection.query_row(
                    "SELECT created_at FROM automation_changesets WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                Ok((created, id.to_owned()))
            })
            .transpose()?;
        let mut statement = self.connection.prepare(
            "SELECT id,kind,status,scope_json,item_count,applied_count,row_version,created_at,updated_at
             FROM automation_changesets
             WHERE (?1 IS NULL OR kind=?1) AND (?2 IS NULL OR status=?2)
               AND (?3 IS NULL OR created_at<?3 OR (created_at=?3 AND id<?4))
             ORDER BY created_at DESC,id DESC LIMIT ?5",
        )?;
        let rows = statement
            .query_map(
                params![
                    kind,
                    status,
                    before.as_ref().map(|(created, _)| created),
                    before.as_ref().map(|(_, id)| id),
                    request.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, u32>(5)?,
                        row.get::<_, u32>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > request.limit as usize;
        let items = rows
            .into_iter()
            .take(request.limit as usize)
            .map(|row| {
                Ok(ChangeSetSummary {
                    id: row.0,
                    kind: parse_change_kind(&row.1)?,
                    status: parse_change_status(&row.2)?,
                    scope: serde_json::from_str(&row.3)?,
                    item_count: row.4,
                    applied_count: row.5,
                    row_version: row.6,
                    created_at: row.7,
                    updated_at: row.8,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ChangeSetPage {
            next_before_id: if more {
                items.last().map(|item| item.id.clone())
            } else {
                None
            },
            items,
        })
    }

    /// Marks stale pending proposals as expired without touching their targets.
    /// The desktop host calls this when opening the automation page.
    pub fn expire_change_sets_before(&mut self, before: chrono::DateTime<Utc>) -> Result<u32> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut statement = tx.prepare(
            "SELECT id FROM automation_changesets WHERE status='pending' AND created_at<?1",
        )?;
        let ids = statement
            .query_map([before.to_rfc3339()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let at = now();
        for id in &ids {
            tx.execute(
                "UPDATE automation_items SET status='skipped',blocked_reason='change_set_expired',updated_at=?2 WHERE change_id=?1 AND status='pending'",
                params![id, at],
            )?;
            tx.execute(
                "UPDATE automation_changesets SET status='expired',row_version=row_version+1,updated_at=?2 WHERE id=?1",
                params![id, at],
            )?;
        }
        tx.commit()?;
        Ok(ids.len() as u32)
    }

    pub fn reject_change_set(
        &mut self,
        command_id: &str,
        request: RejectChangeSet,
    ) -> Result<ChangeSetCommit> {
        let kind = "automation.reject_change_set";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let change = read_change_set(&tx, &request.change_id)?;
        if change.status != ChangeSetStatus::Pending {
            return Err(Error::InvalidInput("automation_change_not_pending"));
        }
        tx.execute(
            "UPDATE automation_items SET status='skipped',blocked_reason='change_set_rejected',updated_at=?2 WHERE change_id=?1 AND status='pending'",
            params![change.id, now()],
        )?;
        tx.execute(
            "UPDATE automation_changesets SET status='rejected',row_version=row_version+1,updated_at=?2 WHERE id=?1",
            params![change.id, now()],
        )?;
        insert_audit(
            &tx,
            NewAudit {
                kind: "automation.change_set_rejected",
                subject_kind: AuditSubjectKind::ChangeSet,
                subject_id: &change.id,
                summary: format!("拒绝整理提案（{} 项）", change.item_count),
                origin: "user",
                undo: None,
                change_id: Some(&change.id),
                item_id: None,
            },
        )?;
        let change = read_change_set(&tx, &change.id)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ChangeSetCommit { change, receipt };
        store_receipt(
            &tx,
            &hash,
            kind,
            &request.change_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn apply_change_item(
        &mut self,
        command_id: &str,
        request: ApplyChangeItem,
    ) -> Result<ChangeItemCommit> {
        let kind = "automation.apply_change_item";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let change = read_change_set(&tx, &request.change_id)?;
        if change.status == ChangeSetStatus::Expired || change_is_expired(&change) {
            if change.status == ChangeSetStatus::Pending {
                tx.execute(
                    "UPDATE automation_changesets SET status='expired',row_version=row_version+1,updated_at=?2 WHERE id=?1",
                    params![change.id, now()],
                )?;
            }
            return Err(Error::InvalidInput("automation_change_expired"));
        }
        if change.status != ChangeSetStatus::Pending {
            return Err(Error::InvalidInput("automation_change_not_pending"));
        }
        let item = item_by_id(&tx, &request.change_id, &request.item_id)?;
        if item.status != ChangeItemStatus::Pending {
            return Err(Error::InvalidInput("automation_item_not_pending"));
        }
        if item.requires_confirmation && !request.confirm {
            return Err(Error::InvalidInput("automation_confirmation_required"));
        }
        let (undo, audit_kind) = match item.kind {
            ChangeItemKind::AssignTag => (apply_assign_tag(&tx, &item)?, "automation.assign_tag"),
            ChangeItemKind::TrashDuplicate => (
                apply_trash_duplicate(&tx, &item)?,
                "automation.trash_duplicate",
            ),
            ChangeItemKind::AppendRelation => (
                apply_append_relation(&tx, &item)?,
                "automation.append_relation",
            ),
            ChangeItemKind::AcknowledgeStale => (None, "automation.acknowledge_stale"),
        };
        let audit = insert_audit(
            &tx,
            NewAudit {
                kind: audit_kind,
                subject_kind: match item.kind {
                    ChangeItemKind::AssignTag => AuditSubjectKind::Tag,
                    ChangeItemKind::TrashDuplicate | ChangeItemKind::AppendRelation => {
                        AuditSubjectKind::Note
                    }
                    ChangeItemKind::AcknowledgeStale => AuditSubjectKind::Source,
                },
                subject_id: &item.target_object_id,
                summary: format!("逐项应用整理提案：{}", item.target_title),
                origin: "automation",
                undo,
                change_id: Some(&change.id),
                item_id: Some(&item.item_id),
            },
        )?;
        mark_item(
            &tx,
            &change.id,
            &item.item_id,
            ChangeItemStatus::Applied,
            None,
            Some(&audit.id),
        )?;
        refresh_change_status(&tx, &change.id)?;
        let change = read_change_set(&tx, &change.id)?;
        let item = change
            .items
            .iter()
            .find(|row| row.item_id == request.item_id)
            .cloned()
            .ok_or(Error::IntegrityFailure)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ChangeItemCommit {
            change,
            item,
            audit: Some(audit),
            receipt,
        };
        store_receipt(
            &tx,
            &hash,
            kind,
            &request.change_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn skip_change_item(
        &mut self,
        command_id: &str,
        request: ApplyChangeItem,
    ) -> Result<ChangeItemCommit> {
        let kind = "automation.skip_change_item";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let change = read_change_set(&tx, &request.change_id)?;
        if change.status != ChangeSetStatus::Pending {
            return Err(Error::InvalidInput("automation_change_not_pending"));
        }
        let item = item_by_id(&tx, &change.id, &request.item_id)?;
        if item.status != ChangeItemStatus::Pending {
            return Err(Error::InvalidInput("automation_item_not_pending"));
        }
        mark_item(
            &tx,
            &change.id,
            &item.item_id,
            ChangeItemStatus::Skipped,
            Some("user_skipped"),
            None,
        )?;
        refresh_change_status(&tx, &change.id)?;
        let change = read_change_set(&tx, &change.id)?;
        let item = change
            .items
            .iter()
            .find(|row| row.item_id == request.item_id)
            .cloned()
            .ok_or(Error::IntegrityFailure)?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = ChangeItemCommit {
            change,
            item,
            audit: None,
            receipt,
        };
        store_receipt(
            &tx,
            &hash,
            kind,
            &request.change_id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    /// Read-only stale-source report; the same data is also available as a
    /// change set whose items can only be acknowledged.
    pub fn stale_source_report(&self, window_days: u32, limit: u32) -> Result<StaleSourceReport> {
        self.stale_source_report_at(window_days, limit, Utc::now())
    }

    /// Deterministic variant used by reports and tests; it evaluates ages as of
    /// the supplied instant without changing any stored timestamp.
    pub fn stale_source_report_at(
        &self,
        window_days: u32,
        limit: u32,
        as_of: chrono::DateTime<Utc>,
    ) -> Result<StaleSourceReport> {
        if !(1..=MAX_CHANGESET_ITEMS).contains(&limit) {
            return Err(Error::InvalidInput("stale report limit is out of range"));
        }
        let window = window_days.clamp(1, 3650);
        let request = CreateChangeSet {
            kind: ChangeSetKind::StaleSources,
            collection_id: None,
            limit,
            stale_after_days: Some(window),
        };
        let items = generate_stale_items(&self.connection, &request, as_of)?;
        let checked: u32 = self.connection.query_row(
            "SELECT COUNT(*) FROM sources s JOIN objects o ON o.id=s.id WHERE o.trashed=0",
            [],
            |row| row.get(0),
        )?;
        let findings = items
            .into_iter()
            .map(|item| {
                let ChangePayload::StaleSource {
                    age_days,
                    captured_at,
                    origin_url,
                    reason,
                } = item.payload
                else {
                    unreachable!()
                };
                let reason = if reason.contains("捕获时间") {
                    StaleReason::OlderThanWindow
                } else if reason.contains("原始地址") {
                    StaleReason::NoOrigin
                } else {
                    StaleReason::MissingVersion
                };
                StaleSourceFinding {
                    source_id: item.target_object_id,
                    title: item.target_title,
                    captured_at: Some(captured_at),
                    age_days: Some(age_days),
                    origin_url,
                    reason,
                    evidence: item.evidence,
                }
            })
            .collect::<Vec<_>>();
        let empty_reason = if checked == 0 {
            Some("workspace_has_no_sources".into())
        } else if findings.is_empty() {
            Some("no_stale_sources".into())
        } else {
            None
        };
        Ok(StaleSourceReport {
            checked,
            window_days: window,
            scanned_at: now(),
            findings,
            empty_reason,
        })
    }

    pub fn audit_page(&self, request: AuditPageRequest) -> Result<AuditPage> {
        if !(1..=MAX_AUDIT_PAGE).contains(&request.limit) {
            return Err(Error::InvalidInput("audit page limit is out of range"));
        }
        let subject_kind = request.subject_kind.map(subject_kind_str);
        let mut statement = self.connection.prepare(
            "SELECT seq,id,occurred_at,kind,subject_kind,subject_id,summary,origin,undoable,undone_at,change_id,item_id
             FROM audit_events
             WHERE (?1 IS NULL OR seq<?1)
               AND (?2 IS NULL OR subject_kind=?2)
               AND (?3 IS NULL OR subject_id=?3)
             ORDER BY seq DESC LIMIT ?4",
        )?;
        let rows = statement
            .query_map(
                params![
                    request.before_seq,
                    subject_kind,
                    request.subject_id,
                    request.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, bool>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > request.limit as usize;
        let items = rows
            .into_iter()
            .take(request.limit as usize)
            .map(|row| {
                Ok(AuditEvent {
                    seq: row.0,
                    id: row.1,
                    occurred_at: row.2,
                    kind: row.3,
                    subject_kind: parse_subject_kind(&row.4)?,
                    subject_id: row.5,
                    summary: row.6,
                    origin: row.7,
                    undoable: row.8,
                    undone: row.9.is_some(),
                    undone_at: row.9,
                    change_id: row.10,
                    item_id: row.11,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(AuditPage {
            next_before_seq: if more {
                items.last().map(|event| event.seq)
            } else {
                None
            },
            items,
        })
    }

    pub fn undo_audit_event(
        &mut self,
        command_id: &str,
        request: UndoAuditEvent,
    ) -> Result<AuditUndoCommit> {
        let kind = "automation.undo_audit_event";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        if !request.confirm {
            return Err(Error::InvalidInput("automation_confirmation_required"));
        }
        let (event, undo) = read_audit_by_id(&tx, &request.audit_id)?;
        if !event.undoable {
            return Err(Error::InvalidInput("automation_audit_not_undoable"));
        }
        if event.undone {
            return Err(Error::InvalidInput("automation_audit_already_undone"));
        }
        let undo = undo.ok_or(Error::IntegrityFailure)?;
        match undo.get("kind").and_then(Value::as_str) {
            Some("assign_tag") => {
                let object_id = str_field(&undo, "object_id")?;
                let tag_id = str_field(&undo, "tag_id")?;
                let object = read_metadata(&tx, object_id)?;
                if tx.execute(
                    "DELETE FROM object_tags WHERE object_id=?1 AND tag_id=?2",
                    params![object_id, tag_id],
                )? == 1
                {
                    bump_version(&tx, &object)?;
                }
            }
            Some("trash_duplicate") => {
                let object_id = str_field(&undo, "object_id")?;
                tx.execute(
                    "UPDATE objects SET trashed=0,row_version=row_version+1 WHERE id=?1 AND trashed=1",
                    [object_id],
                )?;
                tx.execute("UPDATE notes SET trashed=0 WHERE id=?1", [object_id])?;
            }
            Some("review_settings") => {
                let enabled = undo
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or(Error::IntegrityFailure)?;
                let window_days = undo
                    .get("window_days")
                    .and_then(Value::as_u64)
                    .ok_or(Error::IntegrityFailure)? as u32;
                let include_completed = undo
                    .get("include_completed")
                    .and_then(Value::as_bool)
                    .ok_or(Error::IntegrityFailure)?;
                let include_stale = undo
                    .get("include_stale")
                    .and_then(Value::as_bool)
                    .ok_or(Error::IntegrityFailure)?;
                tx.execute(
                    "UPDATE review_insight_settings SET enabled=?1,window_days=?2,include_completed=?3,include_stale=?4,row_version=row_version+1 WHERE singleton=1",
                    params![enabled, window_days, include_completed, include_stale],
                )?;
            }
            Some("append_relation") => {
                let object_id = str_field(&undo, "object_id")?;
                let previous = str_field(&undo, "previous_revision_id")?;
                let resulting = str_field(&undo, "resulting_revision_id")?;
                let head: String = tx.query_row(
                    "SELECT current_revision_id FROM notes WHERE id=?1",
                    [object_id],
                    |row| row.get(0),
                )?;
                if head != resulting {
                    return Err(Error::InvalidInput("automation_undo_conflict"));
                }
                let head_note = note_revision(&tx, object_id, &head)?;
                let old = note_revision(&tx, object_id, previous)?;
                let json: String = tx.query_row(
                    "SELECT document_json FROM note_revisions WHERE revision_id=?1",
                    [previous],
                    |row| row.get(0),
                )?;
                let prepared = Prepared::new(
                    &old.title,
                    old.schema_version,
                    old.document.clone(),
                    Some(json),
                )?;
                commit_revision(&tx, object_id, Some(&head_note), prepared, "restore")?;
            }
            _ => return Err(Error::IntegrityFailure),
        }
        tx.execute(
            "UPDATE audit_events SET undone_by=?2,undone_at=?3 WHERE id=?1",
            params![event.id, command_id, now()],
        )?;
        let reversed = insert_audit(
            &tx,
            NewAudit {
                kind: "audit.undo",
                subject_kind: event.subject_kind,
                subject_id: &event.subject_id,
                summary: format!("撤销审计事件：{}", event.summary),
                origin: "user",
                undo: None,
                change_id: None,
                item_id: None,
            },
        )?;
        let receipt = next_receipt(&tx, command_id)?;
        let result = AuditUndoCommit { reversed, receipt };
        store_receipt(&tx, &hash, kind, &event.id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
}
