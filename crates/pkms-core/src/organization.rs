//! Short, version-checked organization transactions; note revisions stay untouched.
use crate::{
    AssignTag, Error, InboxState, Library, MetadataCommit, MetadataTarget, ObjectKind,
    ObjectMetadata, RemoveTag, Result, SetInboxState, SetObjectFlag, Tag, TagPage, TagSelector,
    TagsPageRequest,
    document::valid_uuid,
    mutations::{check_version, next_receipt, replay, request_hash, store_receipt},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TagsCursor {
    normalized_name: String,
    query: String,
    generation: i64,
}

impl Library {
    pub fn object_metadata(&self, object_id: &str) -> Result<ObjectMetadata> {
        valid_uuid(object_id)?;
        let tx = self.connection.unchecked_transaction()?;
        read_metadata(&tx, object_id)
    }

    pub fn tags_page(&self, request: TagsPageRequest) -> Result<TagPage> {
        if !(1..=100).contains(&request.limit) {
            return Err(Error::InvalidInput("page limit must be 1-100"));
        }
        let query = normalized_query(request.query.as_deref().unwrap_or_default())?;
        let cursor: Option<TagsCursor> = request
            .cursor
            .as_ref()
            .map(|encoded| {
                if encoded.len() > 2048 {
                    return Err(Error::InvalidInput("invalid tags cursor"));
                }
                let cursor: TagsCursor = serde_json::from_str(encoded)
                    .map_err(|_| Error::InvalidInput("invalid tags cursor"))?;
                if cursor.query != query || cursor.normalized_name.chars().count() > 100 {
                    return Err(Error::InvalidInput("cursor does not match this tags query"));
                }
                Ok(cursor)
            })
            .transpose()?;
        let tx = self.connection.unchecked_transaction()?;
        let generation: i64 = tx.query_row(
            "SELECT generation FROM tag_list_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if cursor.as_ref().is_some_and(|c| c.generation != generation) {
            return Err(Error::QueryExpired);
        }
        let mut statement = tx.prepare(
            "SELECT id,display_name,normalized_name FROM tags
             WHERE instr(normalized_name,?1)>0 AND (?2 IS NULL OR normalized_name>?2)
             ORDER BY normalized_name LIMIT ?3",
        )?;
        let mut items = statement
            .query_map(
                params![
                    query,
                    cursor.as_ref().map(|c| &c.normalized_name),
                    request.limit + 1
                ],
                read_tag_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > request.limit as usize {
            items.truncate(request.limit as usize);
            Some(serde_json::to_string(&TagsCursor {
                normalized_name: items
                    .last()
                    .ok_or(Error::IntegrityFailure)?
                    .normalized_name
                    .clone(),
                query,
                generation,
            })?)
        } else {
            None
        };
        Ok(TagPage { items, next_cursor })
    }

    pub fn assign_tag(&mut self, command_id: &str, request: AssignTag) -> Result<MetadataCommit> {
        let kind = "tags.assign";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let targets = checked_targets(&tx, &request.targets)?;
        let (tag, created) = match &request.tag {
            TagSelector::Id { id } => {
                valid_uuid(id)?;
                (read_tag(&tx, id)?, false)
            }
            TagSelector::Name { name } => {
                let (display_name, normalized_name) = normalize_name(name)?;
                let created = tx.execute(
                    "INSERT INTO tags(id,display_name,normalized_name) VALUES(?1,?2,?3)
                     ON CONFLICT(normalized_name) DO NOTHING",
                    params![Uuid::new_v4().to_string(), display_name, normalized_name],
                )? == 1;
                let tag = tx.query_row(
                    "SELECT id,display_name,normalized_name FROM tags WHERE normalized_name=?1",
                    [normalized_name],
                    read_tag_row,
                )?;
                (tag, created)
            }
        };
        for object in &targets {
            if tx.execute(
                "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2)
                 ON CONFLICT(object_id,tag_id) DO NOTHING",
                params![object.id, tag.id],
            )? == 1
            {
                bump_version(&tx, object)?;
            }
        }
        let result = finish(&tx, command_id, &hash, kind, &targets, Some(tag), created)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn remove_tag(&mut self, command_id: &str, request: RemoveTag) -> Result<MetadataCommit> {
        let kind = "tags.remove";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let targets = checked_targets(&tx, &request.targets)?;
        valid_uuid(&request.tag_id)?;
        let tag = read_tag(&tx, &request.tag_id)?;
        for object in &targets {
            if tx.execute(
                "DELETE FROM object_tags WHERE object_id=?1 AND tag_id=?2",
                params![object.id, tag.id],
            )? == 1
            {
                bump_version(&tx, object)?;
            }
        }
        let result = finish(&tx, command_id, &hash, kind, &targets, Some(tag), false)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn set_inbox_state(
        &mut self,
        command_id: &str,
        request: SetInboxState,
    ) -> Result<MetadataCommit> {
        let kind = "objects.set_inbox_state";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let targets = checked_targets(
            &tx,
            &[MetadataTarget {
                object_id: request.object_id,
                base_row_version: request.base_row_version,
            }],
        )?;
        let object = &targets[0];
        if !matches!(
            object.kind,
            ObjectKind::Note | ObjectKind::Source | ObjectKind::Annotation
        ) {
            return Err(Error::InvalidInput(
                "only notes, sources and annotations have inbox state",
            ));
        }
        if object.inbox_state != Some(request.target) {
            // Keep metadata version, state, query invalidation and receipt atomic.
            bump_version(&tx, object)?;
            tx.execute(
                "UPDATE objects SET inbox_state=?2 WHERE id=?1",
                params![object.id, request.target.as_str()],
            )?;
        }
        let result = finish(&tx, command_id, &hash, kind, &targets, None, false)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn set_object_trashed(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
    ) -> Result<MetadataCommit> {
        self.set_object_flag(command_id, request, "trashed")
    }

    pub fn set_object_archived(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
    ) -> Result<MetadataCommit> {
        self.set_object_flag(command_id, request, "archived")
    }

    pub fn set_object_search_enabled(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
    ) -> Result<MetadataCommit> {
        self.set_object_flag(command_id, request, "search_enabled")
    }

    fn set_object_flag(
        &mut self,
        command_id: &str,
        request: SetObjectFlag,
        flag: &str,
    ) -> Result<MetadataCommit> {
        let kind = match flag {
            "trashed" => "objects.set_trashed",
            "archived" => "objects.set_archived",
            "search_enabled" => "objects.set_search_enabled",
            _ => return Err(Error::InvalidInput("unknown lifecycle flag")),
        };
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        valid_uuid(&request.object_id)?;
        let object = read_metadata(&tx, &request.object_id)?;
        check_version(&object.id, request.base_row_version, object.row_version)?;
        if object.trashed && flag != "trashed" {
            return Err(Error::Trashed);
        }
        let current = match flag {
            "trashed" => object.trashed,
            "archived" => object.archived,
            _ => object.search_enabled,
        };
        if current != request.value {
            bump_version(&tx, &object)?;
            // `flag` comes exclusively from the closed match above, never from IPC.
            tx.execute(
                &format!("UPDATE objects SET {flag}=?2 WHERE id=?1"),
                params![object.id, request.value],
            )?;
            if flag == "trashed" && object.kind == ObjectKind::Note {
                tx.execute(
                    "UPDATE notes SET trashed=?2 WHERE id=?1",
                    params![object.id, request.value],
                )?;
            }
        }
        let result = finish(&tx, command_id, &hash, kind, &[object], None, false)?;
        tx.commit()?;
        Ok(result)
    }
}

fn checked_targets(
    connection: &Connection,
    targets: &[MetadataTarget],
) -> Result<Vec<ObjectMetadata>> {
    if !(1..=100).contains(&targets.len()) {
        return Err(Error::InvalidInput(
            "metadata batch must contain 1-100 objects",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut objects = Vec::with_capacity(targets.len());
    for target in targets {
        valid_uuid(&target.object_id)?;
        if target.base_row_version == 0 || !seen.insert(&target.object_id) {
            return Err(Error::InvalidInput(
                "invalid metadata version or duplicate target",
            ));
        }
        let object = read_metadata(connection, &target.object_id)?;
        if object.trashed {
            return Err(Error::Trashed);
        }
        if object.row_version != target.base_row_version {
            return Err(Error::MetadataConflict {
                object_id: object.id,
                expected: target.base_row_version,
                actual: object.row_version,
            });
        }
        objects.push(object);
    }
    Ok(objects)
}

pub(crate) fn bump_version(connection: &Connection, object: &ObjectMetadata) -> Result<()> {
    let next = object
        .row_version
        .checked_add(1)
        .ok_or(Error::InvalidInput("metadata version overflow"))?;
    if connection.execute(
        "UPDATE objects SET row_version=?2 WHERE id=?1 AND row_version=?3",
        params![object.id, next, object.row_version],
    )? != 1
    {
        return Err(Error::IntegrityFailure);
    }
    Ok(())
}

fn finish(
    connection: &Connection,
    command_id: &str,
    hash: &str,
    kind: &str,
    targets: &[ObjectMetadata],
    tag: Option<Tag>,
    tag_created: bool,
) -> Result<MetadataCommit> {
    let result = MetadataCommit {
        objects: targets
            .iter()
            .map(|t| read_metadata(connection, &t.id))
            .collect::<Result<_>>()?,
        tag,
        tag_created,
        receipt: next_receipt(connection, command_id)?,
    };
    store_receipt(
        connection,
        hash,
        kind,
        &targets[0].id,
        &result.receipt,
        &result,
    )?;
    Ok(result)
}

pub(crate) fn parse_inbox(value: Option<&str>) -> Result<Option<InboxState>> {
    Ok(match value {
        None => None,
        Some("pending") => Some(InboxState::Pending),
        Some("snoozed") => Some(InboxState::Snoozed),
        Some("processed") => Some(InboxState::Processed),
        Some(_) => return Err(Error::IntegrityFailure),
    })
}

pub(crate) fn read_metadata(connection: &Connection, object_id: &str) -> Result<ObjectMetadata> {
    let (kind, row_version, archived, trashed, inbox, search_enabled) = connection.query_row(
        "SELECT kind,row_version,archived,trashed,inbox_state,search_enabled FROM objects WHERE id=?1",
        [object_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, bool>(2)?, r.get::<_, bool>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, bool>(5)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    let kind = match kind.as_str() {
        "note" => ObjectKind::Note,
        "collection" => ObjectKind::Collection,
        "source" => ObjectKind::Source,
        "annotation" => ObjectKind::Annotation,
        _ => return Err(Error::IntegrityFailure),
    };
    let inbox_state = parse_inbox(inbox.as_deref())?;
    // Annotations now enter the same inbox as notes and sources. Older
    // annotation rows may still have no inbox state, so keep them readable while
    // new excerpts are created as pending.
    if row_version == 0
        || match kind {
            ObjectKind::Note | ObjectKind::Source => inbox_state.is_none(),
            ObjectKind::Collection => inbox_state.is_some(),
            ObjectKind::Annotation => false,
        }
    {
        return Err(Error::IntegrityFailure);
    }
    let mut statement = connection.prepare(
        "SELECT t.id,t.display_name,t.normalized_name FROM tags t JOIN object_tags ot ON ot.tag_id=t.id
         WHERE ot.object_id=?1 ORDER BY t.normalized_name",
    )?;
    let tags = statement
        .query_map([object_id], read_tag_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ObjectMetadata {
        id: object_id.into(),
        kind,
        row_version,
        archived,
        trashed,
        inbox_state,
        search_enabled,
        tags,
    })
}

fn read_tag_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        display_name: row.get(1)?,
        normalized_name: row.get(2)?,
    })
}

fn read_tag(connection: &Connection, id: &str) -> Result<Tag> {
    connection
        .query_row(
            "SELECT id,display_name,normalized_name FROM tags WHERE id=?1",
            [id],
            read_tag_row,
        )
        .optional()?
        .ok_or(Error::NotFound)
}

pub(crate) fn normalize_name(name: &str) -> Result<(String, String)> {
    if name.len() > 8192 || name.contains('\0') {
        return Err(Error::InvalidInput("invalid tag name"));
    }
    let display: String = name.trim().nfc().collect();
    if !(1..=100).contains(&display.chars().count()) {
        return Err(Error::InvalidInput(
            "tag name must contain 1-100 Unicode characters",
        ));
    }
    let normalized = display.to_ascii_lowercase();
    Ok((display, normalized))
}

pub(crate) fn assign_tag_names(db: &Connection, object_id: &str, names: &[String]) -> Result<bool> {
    if names.len() > 100 {
        return Err(Error::InvalidInput("at most 100 tags may be assigned"));
    }
    let mut seen = BTreeSet::new();
    let mut changed = false;
    for name in names {
        let (display_name, normalized_name) = normalize_name(name)?;
        if !seen.insert(normalized_name.clone()) {
            continue;
        }
        db.execute(
            "INSERT INTO tags(id,display_name,normalized_name) VALUES(?1,?2,?3)
             ON CONFLICT(normalized_name) DO NOTHING",
            params![Uuid::new_v4().to_string(), display_name, normalized_name],
        )?;
        let tag_id: String = db.query_row(
            "SELECT id FROM tags WHERE normalized_name=?1",
            [normalized_name],
            |row| row.get(0),
        )?;
        changed |= db.execute(
            "INSERT INTO object_tags(object_id,tag_id) VALUES(?1,?2)
             ON CONFLICT(object_id,tag_id) DO NOTHING",
            params![object_id, tag_id],
        )? == 1;
    }
    Ok(changed)
}

fn normalized_query(query: &str) -> Result<String> {
    if query.trim().is_empty() {
        Ok(String::new())
    } else {
        Ok(normalize_name(query)?.1)
    }
}
