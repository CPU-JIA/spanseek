//! Durable tag synchronization knowledge. Transport retries remain in the engine;
//! accepted dependencies, comparison baselines and dictionary tombstones live here.
use crate::{
    Error, Library, ObjectKind, Result, document::valid_uuid, organization::normalize_name,
    tag_sync_models::*,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
// UUID v8 uses a domain-separated SHA-256 digest; no optional UUID hash feature.
const TAG_NAMESPACE: Uuid = Uuid::from_u128(0x5193c5d4_3cf8_43b1_9a5d_d69703180d27);
const ASSIGNMENT_NAMESPACE: Uuid = Uuid::from_u128(0xd741b884_7f31_4737_957a_97f58102f52c);

pub fn tag_sync_object_id(kind: TagSyncKind, entity_id: &str) -> Result<String> {
    valid_uuid(entity_id)?;
    let mut digest = Sha256::new();
    digest.update(match kind {
        TagSyncKind::Tag => TAG_NAMESPACE.as_bytes(),
        TagSyncKind::TagAssignments => ASSIGNMENT_NAMESPACE.as_bytes(),
    });
    // Existing Core identities are opaque valid UUID strings. Hash the exact
    // stored text so legacy case/braces/URN forms are never silently merged.
    digest.update(entity_id.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let derived = Uuid::from_bytes(bytes).to_string();
    if derived == entity_id {
        return Err(Error::InvalidInput("tag sync identity collision"));
    }
    Ok(derived)
}

fn object_kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Note => "note",
        ObjectKind::Source => "source",
        ObjectKind::Annotation => "annotation",
        ObjectKind::Collection => "collection",
    }
}

fn object_kind(value: &str) -> Result<ObjectKind> {
    match value {
        "note" => Ok(ObjectKind::Note),
        "source" => Ok(ObjectKind::Source),
        "annotation" => Ok(ObjectKind::Annotation),
        "collection" => Ok(ObjectKind::Collection),
        _ => Err(Error::IntegrityFailure),
    }
}

fn parse_kind(value: &str) -> Result<TagSyncKind> {
    match value {
        "tag" => Ok(TagSyncKind::Tag),
        "tag_assignments" => Ok(TagSyncKind::TagAssignments),
        _ => Err(Error::IntegrityFailure),
    }
}

fn parse_reason(value: &str) -> Result<TagSyncPendingReason> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| Error::IntegrityFailure)
}

fn timestamp(value: &Option<String>) -> Result<()> {
    if let Some(value) = value
        && (value.len() > 64 || DateTime::parse_from_rfc3339(value).is_err())
    {
        return Err(Error::InvalidInput("invalid tag sync timestamp"));
    }
    Ok(())
}

impl TagSyncPayload {
    pub fn sync_object_id(&self) -> Result<String> {
        tag_sync_object_id(self.kind(), self.entity_id())
    }

    pub fn validate(&self) -> Result<()> {
        valid_uuid(self.entity_id())?;
        match self {
            Self::Tag { tag } => {
                let (display, _) = normalize_name(&tag.display_name)?;
                if display != tag.display_name
                    || tag.description.trim() != tag.description
                    || tag.description.chars().count() > 4000
                    || tag.description.contains('\0')
                {
                    return Err(Error::InvalidInput("invalid tag sync dictionary fields"));
                }
                timestamp(&tag.created_at)?;
                timestamp(&tag.updated_at)?;
                timestamp(&tag.deleted_at)?;
            }
            Self::TagAssignments { assignments } => {
                for id in &assignments.tag_ids {
                    valid_uuid(id)?;
                }
                if assignments
                    .tag_ids
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                {
                    return Err(Error::InvalidInput(
                        "tag sync IDs must be sorted and unique",
                    ));
                }
            }
        }
        if serde_json::to_vec(self)?.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::InvalidInput("tag sync payload exceeds 1 MiB"));
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
}

#[derive(Debug, Clone)]
struct State {
    kind: TagSyncKind,
    entity_id: String,
    object_kind: Option<ObjectKind>,
    generation: i64,
    remote_version: i64,
    base_version: i64,
    base: Option<String>,
    pending: Option<TagSyncPending>,
}

fn state(db: &Connection, kind: TagSyncKind, id: &str) -> Result<Option<State>> {
    let row = db
        .query_row(
            "SELECT object_kind,local_generation,remote_version,base_version,base_fingerprint,
         pending_payload_json,pending_baseline_fingerprint,pending_reason,pending_received_at
         FROM tag_sync_state WHERE kind=?1 AND entity_id=?2",
            params![kind.as_str(), id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        object,
        generation,
        remote_version,
        base_version,
        base,
        pending,
        baseline,
        reason,
        received_at,
    )) = row
    else {
        return Ok(None);
    };
    let pending = match (pending, reason, received_at) {
        (None, None, None) => None,
        (Some(json), Some(reason), Some(received_at)) => {
            if json.len() > MAX_PAYLOAD_BYTES {
                return Err(Error::IntegrityFailure);
            }
            let payload: TagSyncPayload = serde_json::from_str(&json)?;
            payload.validate()?;
            if payload.kind() != kind || payload.entity_id() != id {
                return Err(Error::IntegrityFailure);
            }
            Some(TagSyncPending {
                payload,
                reason: parse_reason(&reason)?,
                baseline_fingerprint: baseline,
                received_at,
            })
        }
        _ => return Err(Error::IntegrityFailure),
    };
    Ok(Some(State {
        kind,
        entity_id: id.into(),
        object_kind: object.as_deref().map(object_kind).transpose()?,
        generation,
        remote_version,
        base_version,
        base,
        pending,
    }))
}

fn ensure_state(db: &Connection, payload: &TagSyncPayload) -> Result<State> {
    let (object, actual) = match payload {
        TagSyncPayload::TagAssignments { assignments } => {
            let actual: Option<String> = db
                .query_row(
                    "SELECT kind FROM objects WHERE id=?1",
                    [&assignments.object_id],
                    |r| r.get(0),
                )
                .optional()?;
            (
                Some(
                    actual
                        .clone()
                        .unwrap_or_else(|| object_kind_name(assignments.object_kind).into()),
                ),
                actual,
            )
        }
        _ => (None, None),
    };
    db.execute("INSERT INTO tag_sync_state(kind,entity_id,object_kind) VALUES(?1,?2,?3) ON CONFLICT(kind,entity_id) DO NOTHING",
        params![payload.kind().as_str(), payload.entity_id(), object])?;
    if let Some(object) = actual {
        db.execute(
            "UPDATE tag_sync_state SET object_kind=?3 WHERE kind=?1 AND entity_id=?2",
            params![payload.kind().as_str(), payload.entity_id(), object],
        )?;
    }
    state(db, payload.kind(), payload.entity_id())?.ok_or(Error::IntegrityFailure)
}

struct Current {
    // An absent assignment target has an internal empty comparison baseline only.
    payload: Option<TagSyncPayload>,
    exists: bool,
    publishable: bool,
}

impl Current {
    fn fingerprint(&self) -> Result<Option<String>> {
        self.payload
            .as_ref()
            .map(TagSyncPayload::fingerprint)
            .transpose()
    }
    fn public_fingerprint(&self) -> Result<Option<String>> {
        if self.publishable {
            self.fingerprint()
        } else {
            Ok(None)
        }
    }
}

fn current(db: &Connection, state: &State) -> Result<Current> {
    match state.kind {
        TagSyncKind::Tag => {
            let tag = db.query_row("SELECT id,display_name,description,created_at,updated_at FROM tags WHERE id=?1", [&state.entity_id], |r| Ok(SyncedTag {
                id:r.get(0)?,display_name:r.get(1)?,description:r.get(2)?,created_at:r.get(3)?,updated_at:r.get(4)?,deleted_at:None
            })).optional()?;
            if let Some(tag) = tag {
                return Ok(Current {
                    payload: Some(TagSyncPayload::Tag { tag }),
                    exists: true,
                    publishable: true,
                });
            }
            let tombstone: Option<(String, String)> = db
                .query_row(
                    "SELECT definition_json,deleted_at FROM tag_sync_tombstones WHERE tag_id=?1",
                    [&state.entity_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let payload = tombstone
                .map(|(json, deleted_at)| -> Result<_> {
                    if json.len() > MAX_PAYLOAD_BYTES {
                        return Err(Error::IntegrityFailure);
                    }
                    let tag: SyncedTag = serde_json::from_str(&json)?;
                    if tag.id != state.entity_id || tag.deleted_at.as_deref() != Some(&deleted_at) {
                        return Err(Error::IntegrityFailure);
                    }
                    let payload = TagSyncPayload::Tag { tag };
                    payload.validate()?;
                    Ok(payload)
                })
                .transpose()?;
            Ok(Current {
                publishable: payload.is_some(),
                payload,
                exists: false,
            })
        }
        TagSyncKind::TagAssignments => {
            let object: Option<String> = db
                .query_row(
                    "SELECT kind FROM objects WHERE id=?1",
                    [&state.entity_id],
                    |r| r.get(0),
                )
                .optional()?;
            let kind = object
                .as_deref()
                .map(object_kind)
                .transpose()?
                .or(state.object_kind)
                .ok_or(Error::IntegrityFailure)?;
            let mut statement =
                db.prepare("SELECT tag_id FROM object_tags WHERE object_id=?1 ORDER BY tag_id")?;
            let tags = statement
                .query_map([&state.entity_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?;
            Ok(Current {
                payload: Some(TagSyncPayload::TagAssignments {
                    assignments: SyncedTagAssignments {
                        object_id: state.entity_id.clone(),
                        object_kind: kind,
                        tag_ids: tags,
                    },
                }),
                exists: object.is_some(),
                publishable: object.is_some() || state.generation > 0,
            })
        }
    }
}

fn labels(db: &Connection, payload: Option<&TagSyncPayload>) -> Result<Vec<TagSyncLabel>> {
    let Some(TagSyncPayload::TagAssignments { assignments }) = payload else {
        return Ok(Vec::new());
    };
    let mut statement = db.prepare(
        "SELECT j.value,COALESCE(t.display_name,json_extract(d.definition_json,'$.display_name')),d.tag_id IS NOT NULL
         FROM json_each(?1) j LEFT JOIN tags t ON t.id=j.value
         LEFT JOIN tag_sync_tombstones d ON d.tag_id=j.value ORDER BY j.key",
    )?;
    Ok(statement
        .query_map([serde_json::to_string(&assignments.tag_ids)?], |row| {
            Ok(TagSyncLabel {
                id: row.get(0)?,
                display_name: row.get(1)?,
                deleted: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn entity_title(db: &Connection, state: &State, local: &Current) -> Result<Option<String>> {
    if state.kind == TagSyncKind::Tag {
        return Ok(local
            .payload
            .as_ref()
            .or_else(|| state.pending.as_ref().map(|pending| &pending.payload))
            .and_then(|payload| match payload {
                TagSyncPayload::Tag { tag } => Some(tag.display_name.clone()),
                _ => None,
            }));
    }
    Ok(db
        .query_row(
            "SELECT CASE o.kind WHEN 'note' THEN n.title WHEN 'collection' THEN c.name
              WHEN 'source' THEN s.display_title ELSE substr(COALESCE(NULLIF(a.comment,''),
                NULLIF(json_extract(a.locator_json,'$.quote'),''),origin.display_title,''),1,300) END
             FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN collections c ON c.id=o.id
             LEFT JOIN sources s ON s.id=o.id LEFT JOIN annotations a ON a.id=o.id
             LEFT JOIN source_versions v ON v.id=a.source_version_id
             LEFT JOIN sources origin ON origin.id=v.source_id WHERE o.id=?1",
            [&state.entity_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn snapshot(db: &Connection, state: State) -> Result<TagSyncSnapshot> {
    let local = current(db, &state)?;
    let fingerprint = local.public_fingerprint()?;
    let delete_impact = local_delete_impact(db, &state, &local)?;
    let impact = delete_impact
        .as_ref()
        .map(|details| details.impact_token.clone());
    let entity_title = entity_title(db, &state, &local)?;
    let local_tags = labels(db, local.payload.as_ref().filter(|_| local.publishable))?;
    let remote_tags = labels(db, state.pending.as_ref().map(|pending| &pending.payload))?;
    let payload = if local.publishable {
        local.payload
    } else {
        None
    };
    Ok(TagSyncSnapshot {
        sync_object_id: tag_sync_object_id(state.kind, &state.entity_id)?,
        kind: state.kind,
        entity_id: state.entity_id,
        entity_title,
        local_tags,
        remote_tags,
        payload,
        local_exists: local.exists,
        local_generation: state.generation,
        dirty: fingerprint.is_some() && fingerprint != state.base,
        local_fingerprint: fingerprint,
        local_impact_token: impact,
        delete_impact,
        remote_version: state.remote_version,
        base_version: state.base_version,
        base_fingerprint: state.base,
        pending: state.pending,
    })
}

fn local_impact_token(db: &Connection, state: &State, local: &Current) -> Result<Option<String>> {
    Ok(local_delete_impact(db, state, local)?.map(|details| details.impact_token))
}

fn local_delete_impact(
    db: &Connection,
    state: &State,
    local: &Current,
) -> Result<Option<crate::TagDetails>> {
    if local.exists
        && matches!(&state.pending, Some(p) if matches!(&p.payload, TagSyncPayload::Tag { tag } if tag.deleted_at.is_some()))
    {
        return Ok(Some(crate::tag_management::details(db, &state.entity_id)?));
    }
    Ok(None)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    kind: TagSyncKind,
    id: String,
    pending_only: bool,
}

impl Library {
    /// Build once per sync pass to protect the reverse direction too: a later
    /// note/view/canvas must not reinterpret a reserved tag wire identity.
    pub fn tag_sync_wire_owners(&self) -> Result<Vec<TagSyncWireOwner>> {
        let mut statement = self
            .connection
            .prepare("SELECT kind,entity_id FROM tag_sync_state ORDER BY kind,entity_id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(kind, entity_id)| {
                let kind = parse_kind(&kind)?;
                Ok(TagSyncWireOwner {
                    sync_object_id: tag_sync_object_id(kind, &entity_id)?,
                    kind,
                    entity_id,
                })
            })
            .collect()
    }

    pub fn tag_sync_snapshot(&self, kind: TagSyncKind, entity_id: &str) -> Result<TagSyncSnapshot> {
        valid_uuid(entity_id)?;
        let tx = self.connection.unchecked_transaction()?;
        snapshot(&tx, state(&tx, kind, entity_id)?.ok_or(Error::NotFound)?)
    }

    pub fn tag_sync_page(&self, request: TagSyncPageRequest) -> Result<TagSyncPage> {
        if !(1..=100).contains(&request.limit) {
            return Err(Error::InvalidInput("invalid tag sync page limit"));
        }
        let cursor = request
            .after
            .as_ref()
            .map(|encoded| -> Result<Cursor> {
                if encoded.len() > 256 {
                    return Err(Error::InvalidInput("invalid tag sync cursor"));
                }
                let cursor: Cursor = serde_json::from_str(encoded)
                    .map_err(|_| Error::InvalidInput("invalid tag sync cursor"))?;
                valid_uuid(&cursor.id)?;
                if cursor.pending_only != request.pending_only {
                    return Err(Error::InvalidInput("tag sync cursor scope changed"));
                }
                Ok(cursor)
            })
            .transpose()?;
        let tx = self.connection.unchecked_transaction()?;
        let total = tx.query_row(
            "SELECT count(*) FROM tag_sync_state WHERE ?1=0 OR pending_payload_json IS NOT NULL",
            [request.pending_only],
            |row| row.get(0),
        )?;
        let mut statement = tx.prepare("SELECT kind,entity_id FROM tag_sync_state
            WHERE (?1=0 OR pending_payload_json IS NOT NULL) AND (?2 IS NULL OR kind>?2 OR (kind=?2 AND entity_id>?3))
            ORDER BY kind,entity_id LIMIT ?4")?;
        let mut keys = statement
            .query_map(
                params![
                    request.pending_only,
                    cursor.as_ref().map(|c| c.kind.as_str()),
                    cursor.as_ref().map(|c| &c.id),
                    request.limit + 1
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = keys.len() > request.limit as usize;
        keys.truncate(request.limit as usize);
        let next_cursor = if more {
            keys.last()
                .map(|(kind, id)| -> Result<String> {
                    Ok(serde_json::to_string(&Cursor {
                        kind: parse_kind(kind)?,
                        id: id.clone(),
                        pending_only: request.pending_only,
                    })?)
                })
                .transpose()?
        } else {
            None
        };
        let items = keys
            .into_iter()
            .map(|(kind, id)| {
                let kind = parse_kind(&kind)?;
                snapshot(&tx, state(&tx, kind, &id)?.ok_or(Error::IntegrityFailure)?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(TagSyncPage {
            items,
            total,
            next_cursor,
        })
    }
}

fn validate_wire_collision(db: &Connection, payload: &TagSyncPayload) -> Result<()> {
    let wire = payload.sync_object_id()?;
    let collision: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1)
          OR EXISTS(SELECT 1 FROM saved_views WHERE id=?1)
          OR EXISTS(SELECT 1 FROM canvases WHERE id=?1)",
        [&wire],
        |r| r.get(0),
    )?;
    if collision {
        return Err(Error::InvalidInput(
            "tag sync wire identity collides with another entity",
        ));
    }
    Ok(())
}

fn dependency_reason(
    db: &Connection,
    payload: &TagSyncPayload,
    local: &Current,
    force: bool,
) -> Result<Option<TagSyncPendingReason>> {
    match payload {
        TagSyncPayload::Tag { tag } if tag.deleted_at.is_none() => {
            let (_, normalized) = normalize_name(&tag.display_name)?;
            let collision: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tags WHERE normalized_name=?1 AND id<>?2)",
                params![normalized, tag.id],
                |r| r.get(0),
            )?;
            if collision {
                return Ok(Some(TagSyncPendingReason::NameConflict));
            }
            if !force
                && matches!(&local.payload,Some(TagSyncPayload::Tag { tag }) if tag.deleted_at.is_some())
            {
                return Ok(Some(TagSyncPendingReason::DeletedTag));
            }
        }
        TagSyncPayload::Tag { tag } => {
            let views: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM saved_views v,json_each(v.filter_ast,'$.tag_ids') j WHERE j.value=?1)", [&tag.id], |r| r.get(0))?;
            if views {
                return Ok(Some(TagSyncPendingReason::ViewDependency));
            }
            if !force {
                let mut stmt = db.prepare(
                    "SELECT object_id FROM object_tags WHERE tag_id=?1 ORDER BY object_id",
                )?;
                let objects = stmt
                    .query_map([&tag.id], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for id in objects {
                    let Some(s) = state(db, TagSyncKind::TagAssignments, &id)? else {
                        return Ok(Some(TagSyncPendingReason::AssignmentChanges));
                    };
                    let current = current(db, &s)?;
                    if current.fingerprint()? != s.base || s.pending.is_some() {
                        return Ok(Some(TagSyncPendingReason::AssignmentChanges));
                    }
                }
            }
        }
        TagSyncPayload::TagAssignments { assignments } => {
            if !local.exists {
                return Ok(Some(TagSyncPendingReason::MissingObject));
            }
            if matches!(&local.payload,Some(TagSyncPayload::TagAssignments { assignments: actual }) if actual.object_kind!=assignments.object_kind)
            {
                return Ok(Some(TagSyncPendingReason::WrongObjectKind));
            }
            let mut missing = false;
            for id in &assignments.tag_ids {
                let deleted: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tag_sync_tombstones WHERE tag_id=?1)",
                    [id],
                    |r| r.get(0),
                )?;
                if deleted {
                    return Ok(Some(TagSyncPendingReason::DeletedTags));
                }
                let exists: bool =
                    db.query_row("SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1)", [id], |r| {
                        r.get(0)
                    })?;
                missing |= !exists;
            }
            if missing {
                return Ok(Some(TagSyncPendingReason::MissingTags));
            }
        }
    }
    Ok(None)
}

fn defer(
    db: &Connection,
    old: &State,
    payload: &TagSyncPayload,
    version: i64,
    reason: TagSyncPendingReason,
    local_fingerprint: Option<String>,
) -> Result<()> {
    let baseline = old
        .pending
        .as_ref()
        .map(|p| p.baseline_fingerprint.clone())
        .unwrap_or(local_fingerprint);
    let received = if version == old.remote_version {
        old.pending.as_ref().map(|p| p.received_at.clone())
    } else {
        None
    }
    .unwrap_or_else(|| Utc::now().to_rfc3339());
    db.execute("UPDATE tag_sync_state SET remote_version=?3,pending_payload_json=?4,
        pending_baseline_fingerprint=?5,pending_reason=?6,pending_received_at=?7 WHERE kind=?1 AND entity_id=?2",
        params![old.kind.as_str(),old.entity_id,version,serde_json::to_string(payload)?,baseline,reason.as_str(),received])?;
    Ok(())
}

fn confirm(
    db: &Connection,
    payload: &TagSyncPayload,
    version: i64,
    fingerprint: &str,
) -> Result<()> {
    db.execute("UPDATE tag_sync_state SET remote_version=?3,base_version=?3,base_fingerprint=?4,
        pending_payload_json=NULL,pending_baseline_fingerprint=NULL,pending_reason=NULL,pending_received_at=NULL
        WHERE kind=?1 AND entity_id=?2",params![payload.kind().as_str(),payload.entity_id(),version,fingerprint])?;
    Ok(())
}

fn bump_object(db: &Connection, id: &str) -> Result<()> {
    if db.execute(
        "UPDATE objects SET row_version=row_version+1 WHERE id=?1 AND row_version<4294967295",
        [id],
    )? != 1
    {
        return Err(Error::InvalidInput("metadata version overflow"));
    }
    Ok(())
}

fn materialize(db: &Connection, payload: &TagSyncPayload, local: &Current) -> Result<Vec<String>> {
    match payload {
        TagSyncPayload::Tag { tag } => {
            let (_, normalized) = normalize_name(&tag.display_name)?;
            let old = match &local.payload {
                Some(TagSyncPayload::Tag { tag }) if tag.deleted_at.is_none() => Some(tag),
                _ => None,
            };
            let rename = old.is_some_and(|old| old.display_name != tag.display_name);
            let deleting = tag.deleted_at.is_some();
            let affected = if rename || deleting {
                let mut statement = db.prepare(
                    "SELECT object_id FROM object_tags WHERE tag_id=?1 ORDER BY object_id",
                )?;
                statement
                    .query_map([&tag.id], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            if let Some(old) = old
                && (rename || deleting)
            {
                let (_, old_normalized) = normalize_name(&old.display_name)?;
                crate::automation::invalidate_tag_suggestions(db, &old_normalized)?;
            }
            for id in &affected {
                bump_object(db, id)?;
            }
            if let Some(deleted_at) = &tag.deleted_at {
                db.execute("DELETE FROM object_tags WHERE tag_id=?1", [&tag.id])?;
                db.execute("DELETE FROM tags WHERE id=?1", [&tag.id])?;
                // The delete trigger captures local history first. The accepted
                // remote tombstone then replaces it atomically and exactly.
                db.execute("INSERT INTO tag_sync_tombstones(tag_id,definition_json,deleted_at) VALUES(?1,?2,?3)
                    ON CONFLICT(tag_id) DO UPDATE SET definition_json=excluded.definition_json,deleted_at=excluded.deleted_at",
                    params![tag.id,serde_json::to_string(tag)?,deleted_at])?;
            } else {
                if local.exists {
                    if db.execute("UPDATE tags SET display_name=?2,normalized_name=?3,description=?4,
                        created_at=?5,updated_at=?6,row_version=row_version+1 WHERE id=?1 AND row_version<4294967295",
                        params![tag.id,tag.display_name,normalized,tag.description,tag.created_at,tag.updated_at])? != 1 {
                        return Err(Error::InvalidInput("tag version overflow"));
                    }
                } else {
                    db.execute("INSERT INTO tags(id,display_name,normalized_name,description,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
                        params![tag.id,tag.display_name,normalized,tag.description,tag.created_at,tag.updated_at])?;
                    // K42 supplies timestamps for local inserts. A received old
                    // NULL remains NULL; no transient generated time is committed.
                    db.execute(
                        "UPDATE tags SET created_at=?2,updated_at=?3 WHERE id=?1",
                        params![tag.id, tag.created_at, tag.updated_at],
                    )?;
                }
                db.execute("DELETE FROM tag_sync_tombstones WHERE tag_id=?1", [&tag.id])?;
            }
            Ok(affected)
        }
        TagSyncPayload::TagAssignments { assignments } => {
            let previous = match &local.payload {
                Some(TagSyncPayload::TagAssignments { assignments }) => &assignments.tag_ids,
                _ => return Err(Error::IntegrityFailure),
            };
            for id in previous
                .iter()
                .filter(|id| assignments.tag_ids.binary_search(id).is_err())
            {
                db.execute(
                    "DELETE FROM object_tags WHERE object_id=?1 AND tag_id=?2",
                    params![assignments.object_id, id],
                )?;
            }
            for id in assignments
                .tag_ids
                .iter()
                .filter(|id| previous.binary_search(id).is_err())
            {
                db.execute(
                    "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2)",
                    params![assignments.object_id, id],
                )?;
            }
            if previous != &assignments.tag_ids {
                bump_object(db, &assignments.object_id)?;
            }
            Ok(vec![assignments.object_id.clone()])
        }
    }
}

fn result(
    db: &Connection,
    kind: TagSyncKind,
    id: &str,
    status: TagSyncApplyStatus,
    changed_objects: Vec<String>,
) -> Result<TagSyncApplyResult> {
    Ok(TagSyncApplyResult {
        status,
        changed_objects,
        snapshot: snapshot(db, state(db, kind, id)?.ok_or(Error::IntegrityFailure)?)?,
    })
}

fn receive(
    db: &Connection,
    payload: &TagSyncPayload,
    version: i64,
    force: bool,
) -> Result<TagSyncApplyResult> {
    let old = ensure_state(db, payload)?;
    let fp = payload.fingerprint()?;
    if version < old.remote_version {
        return result(
            db,
            old.kind,
            &old.entity_id,
            TagSyncApplyStatus::IgnoredOlder,
            Vec::new(),
        );
    }
    if version == old.remote_version {
        let seen = if let Some(pending) = &old.pending {
            Some(pending.payload.fingerprint()?)
        } else if old.base_version == version {
            old.base.clone()
        } else {
            None
        };
        if seen.as_deref() != Some(&fp) {
            return Err(Error::IdempotencyMismatch);
        }
        if old.pending.is_none() {
            // Replaying an acknowledged remote version must not undo a later
            // local edit, even if its bytes no longer match the old payload.
            return result(
                db,
                old.kind,
                &old.entity_id,
                TagSyncApplyStatus::Unchanged,
                Vec::new(),
            );
        }
    }
    let local = current(db, &old)?;
    let local_fp = local.fingerprint()?;
    if local.publishable && local_fp.as_deref() == Some(&fp) {
        confirm(db, payload, version, &fp)?;
        return result(
            db,
            old.kind,
            &old.entity_id,
            TagSyncApplyStatus::Unchanged,
            Vec::new(),
        );
    }
    let changed_after_receipt = old
        .pending
        .as_ref()
        .is_some_and(|p| p.baseline_fingerprint != local_fp);
    let unacknowledged = match &old.base {
        Some(base) => local_fp.as_deref() != Some(base),
        None => old.generation > 0 && local.publishable,
    };
    let reason = dependency_reason(db, payload, &local, force)?;
    let reason = if !force && (changed_after_receipt || unacknowledged) {
        Some(TagSyncPendingReason::LocalChanges)
    } else {
        reason
    };
    if let Some(reason) = reason {
        defer(db, &old, payload, version, reason, local_fp)?;
        return result(
            db,
            old.kind,
            &old.entity_id,
            TagSyncApplyStatus::Pending,
            Vec::new(),
        );
    }
    let changed = materialize(db, payload, &local)?;
    confirm(db, payload, version, &fp)?;
    result(
        db,
        old.kind,
        &old.entity_id,
        TagSyncApplyStatus::Applied,
        changed,
    )
}

impl Library {
    pub fn tag_sync_receive(
        &mut self,
        sync_object_id: &str,
        payload: TagSyncPayload,
        remote_version: i64,
    ) -> Result<TagSyncApplyResult> {
        payload.validate()?;
        if remote_version < 1 || payload.sync_object_id()? != sync_object_id {
            return Err(Error::InvalidInput("invalid tag sync envelope"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_wire_collision(&tx, &payload)?;
        let result = receive(&tx, &payload, remote_version, false)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn tag_sync_retry(
        &mut self,
        kind: TagSyncKind,
        entity_id: &str,
        expected_remote_version: i64,
    ) -> Result<TagSyncApplyResult> {
        valid_uuid(entity_id)?;
        if expected_remote_version < 1 {
            return Err(Error::InvalidInput("invalid tag sync expected version"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = state(&tx, kind, entity_id)?.ok_or(Error::NotFound)?;
        if old.remote_version != expected_remote_version {
            return Err(Error::QueryExpired);
        }
        let Some(pending) = old.pending else {
            return result(
                &tx,
                kind,
                entity_id,
                TagSyncApplyStatus::Unchanged,
                Vec::new(),
            );
        };
        validate_wire_collision(&tx, &pending.payload)?;
        let result = receive(&tx, &pending.payload, expected_remote_version, false)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn tag_sync_keep_remote(
        &mut self,
        kind: TagSyncKind,
        entity_id: &str,
        expected_remote_version: i64,
        expected_local_fingerprint: Option<&str>,
        expected_local_impact_token: Option<&str>,
    ) -> Result<TagSyncApplyResult> {
        valid_uuid(entity_id)?;
        if expected_remote_version < 1 {
            return Err(Error::InvalidInput("invalid tag sync expected version"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = state(&tx, kind, entity_id)?.ok_or(Error::NotFound)?;
        let local = current(&tx, &old)?;
        if old.remote_version != expected_remote_version
            || local.public_fingerprint()?.as_deref() != expected_local_fingerprint
            || local_impact_token(&tx, &old, &local)?.as_deref() != expected_local_impact_token
        {
            return Err(Error::QueryExpired);
        }
        let pending = old.pending.ok_or(Error::NotFound)?;
        validate_wire_collision(&tx, &pending.payload)?;
        let result = receive(&tx, &pending.payload, expected_remote_version, true)?;
        tx.commit()?;
        Ok(result)
    }

    /// Call only after the server has acknowledged this exact payload/version.
    /// A late acknowledgement may advance its own baseline, never newer pending.
    pub fn tag_sync_acknowledge(
        &mut self,
        payload: TagSyncPayload,
        remote_version: i64,
    ) -> Result<TagSyncSnapshot> {
        payload.validate()?;
        if remote_version < 1 {
            return Err(Error::InvalidInput("invalid tag sync acknowledgement"));
        }
        let fp = payload.fingerprint()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_wire_collision(&tx, &payload)?;
        let old = ensure_state(&tx, &payload)?;
        let local = current(&tx, &old)?;
        if !local.publishable
            || (old.generation == 0 && local.fingerprint()?.as_deref() != Some(&fp))
        {
            return Err(Error::InvalidInput(
                "tag acknowledgement has no local outbound state",
            ));
        }
        if remote_version == old.remote_version {
            let seen = old
                .pending
                .as_ref()
                .map(|p| p.payload.fingerprint())
                .transpose()?
                .or_else(|| old.base.clone());
            if seen.as_deref().is_some_and(|seen| seen != fp) {
                return Err(Error::IdempotencyMismatch);
            }
        }
        if remote_version > old.remote_version {
            confirm(&tx, &payload, remote_version, &fp)?;
        } else if remote_version > old.base_version {
            // A newer pending remote version remains in the same row.
            tx.execute("UPDATE tag_sync_state SET base_version=?3,base_fingerprint=?4 WHERE kind=?1 AND entity_id=?2",
                params![payload.kind().as_str(),payload.entity_id(),remote_version,fp])?;
            if remote_version == old.remote_version {
                confirm(&tx, &payload, remote_version, &fp)?;
            }
        } else if remote_version == old.base_version && old.base.as_deref() != Some(&fp) {
            return Err(Error::IdempotencyMismatch);
        }
        let result = snapshot(
            &tx,
            state(&tx, payload.kind(), payload.entity_id())?.ok_or(Error::IntegrityFailure)?,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

/// Structural knowledge validation for full packages. Missing assignment targets
/// are legitimate deferred dependencies, not invalid foreign-key rows.
pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let fingerprint = |value: &Option<String>| {
        value.as_ref().is_none_or(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    };
    let mut statement =
        db.prepare("SELECT kind,entity_id FROM tag_sync_state ORDER BY kind,entity_id")?;
    let keys = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (kind, id) in keys {
        valid_uuid(&id).map_err(|_| Error::IntegrityFailure)?;
        let kind = parse_kind(&kind)?;
        let row = state(db, kind, &id)?.ok_or(Error::IntegrityFailure)?;
        if row.generation < 0
            || row.base_version < 0
            || row.remote_version < row.base_version
            || (row.base_version == 0) != row.base.is_none()
            || (row.remote_version > row.base_version) != row.pending.is_some()
            || !fingerprint(&row.base)
        {
            return Err(Error::IntegrityFailure);
        }
        if let Some(pending) = &row.pending {
            if !fingerprint(&pending.baseline_fingerprint)
                || (row.kind == TagSyncKind::TagAssignments
                    && pending.baseline_fingerprint.is_none())
                || DateTime::parse_from_rfc3339(&pending.received_at).is_err()
            {
                return Err(Error::IntegrityFailure);
            }
            validate_wire_collision(db, &pending.payload)?;
        }
        let local = current(db, &row)?;
        if row.kind == TagSyncKind::Tag && local.payload.is_none() && row.pending.is_none() {
            return Err(Error::IntegrityFailure);
        }
        if let Some(payload) = &local.payload {
            payload.validate()?;
            validate_wire_collision(db, payload)?;
        }
    }
    let incomplete: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags t WHERE NOT EXISTS(SELECT 1 FROM tag_sync_state s WHERE s.kind='tag' AND s.entity_id=t.id))
        OR EXISTS(SELECT 1 FROM tag_sync_tombstones t WHERE NOT EXISTS(SELECT 1 FROM tag_sync_state s WHERE s.kind='tag' AND s.entity_id=t.tag_id))
        OR EXISTS(SELECT 1 FROM object_tags t WHERE NOT EXISTS(SELECT 1 FROM tag_sync_state s WHERE s.kind='tag_assignments' AND s.entity_id=t.object_id))
        OR EXISTS(SELECT 1 FROM tags t JOIN tag_sync_tombstones d ON d.tag_id=t.id)",
        [], |r| r.get(0),
    )?;
    if incomplete {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}
