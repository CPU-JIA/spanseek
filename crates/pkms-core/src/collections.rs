use crate::{
    ChangeCollectionMembers, CollectionCommit, CollectionDetail, CollectionMember,
    CollectionMembersPage, CollectionMembersRequest, CollectionPage, CollectionPageRequest,
    CreateCollection, Error, Library, ObjectKind, Result, UpdateCollection,
    document::valid_uuid,
    mutations::{
        check_version, next_receipt, replay, request_hash, store_receipt, validate_name,
        validate_pin,
    },
    organization::{bump_version, read_metadata},
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionCursor {
    pin: i64,
    name: String,
    id: String,
    generation: i64,
    query_hash: String,
    /// Physical library binding. Two libraries can share a query fingerprint
    /// and a generation counter, so a cursor is only replayable inside the
    /// library that issued it. Cursors serialized before this field existed
    /// deserialize with an empty scope and are rejected the same way.
    #[serde(default)]
    scope: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MembersCursor {
    collection_id: String,
    position: u32,
    generation: i64,
    row_version: u32,
    include_trashed: bool,
    #[serde(default)]
    kind: Option<ObjectKind>,
    #[serde(default)]
    source_generation: i64,
    #[serde(default)]
    scope: String,
}

/// Cursors are opaque, but they only mean something inside one physical
/// library: the same query fingerprint and list generation can exist in two
/// libraries. Hashing the database path together with the workspace id (the
/// same recipe `trash.rs` uses for tombstones) makes a foreign cursor
/// detectable instead of silently paginating over another library.
fn library_scope(database_path: &std::path::Path, tx: &Connection) -> Result<String> {
    let workspace: String = tx.query_row(
        "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            database_path.to_string_lossy(),
            workspace
        ))?)
    ))
}

impl Library {
    pub fn collection_get(&self, collection_id: &str) -> Result<CollectionDetail> {
        valid_uuid(collection_id)?;
        let tx = self.connection.unchecked_transaction()?;
        read_collection(&tx, collection_id)
    }

    pub fn collections_page(&self, request: CollectionPageRequest) -> Result<CollectionPage> {
        validate_limit(request.limit)?;
        let query = search_text(request.query.as_deref())?;
        let query_hash = request_hash(
            "00000000-0000-0000-0000-000000000000",
            "collections.list",
            &(
                request.trashed,
                request.archived,
                request.pinned_only,
                &query,
            ),
        )?;
        let cursor: Option<CollectionCursor> = decode_cursor(request.cursor.as_deref())?;
        if cursor.as_ref().is_some_and(|c| c.query_hash != query_hash) {
            return Err(Error::InvalidInput(
                "collection cursor belongs to a different query",
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let generation: i64 =
            tx.query_row("SELECT generation FROM collection_list_state", [], |r| {
                r.get(0)
            })?;
        let scope = library_scope(&self.database_path, &tx)?;
        if cursor
            .as_ref()
            .is_some_and(|c| c.generation != generation || c.scope != scope)
        {
            return Err(Error::QueryExpired);
        }
        let mut statement = tx.prepare(
            "SELECT c.id FROM collections c JOIN objects o ON o.id=c.id
             WHERE o.trashed=?1 AND (?1=1 OR o.archived=?2) AND (?3=0 OR c.is_pinned=1)
               AND instr(lower(c.name),lower(?4))>0
               AND (?5 IS NULL OR (COALESCE(c.pin_position,4294967296),c.name,c.id)>(?5,?6,?7))
             ORDER BY COALESCE(c.pin_position,4294967296),c.name,c.id LIMIT ?8",
        )?;
        let ids = statement
            .query_map(
                params![
                    request.trashed,
                    request.archived,
                    request.pinned_only,
                    query,
                    cursor.as_ref().map(|c| c.pin),
                    cursor.as_ref().map(|c| &c.name),
                    cursor.as_ref().map(|c| &c.id),
                    request.limit + 1
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = ids.len() > request.limit as usize;
        let items = ids
            .iter()
            .take(request.limit as usize)
            .map(|id| read_collection(&tx, id))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            let last = items.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&CollectionCursor {
                pin: last.pin_position.map(i64::from).unwrap_or(4294967296),
                name: last.name.clone(),
                id: last.id.clone(),
                generation,
                query_hash,
                scope,
            })?)
        } else {
            None
        };
        Ok(CollectionPage { items, next_cursor })
    }

    pub fn collection_members_page(
        &self,
        request: CollectionMembersRequest,
    ) -> Result<CollectionMembersPage> {
        validate_limit(request.limit)?;
        valid_uuid(&request.collection_id)?;
        let kind = match request.kind {
            None => None,
            Some(ObjectKind::Note) => Some("note"),
            Some(ObjectKind::Source) => Some("source"),
            Some(ObjectKind::Annotation) => Some("annotation"),
            Some(ObjectKind::Collection) => {
                return Err(Error::InvalidInput("nested collections are not supported"));
            }
        };
        let cursor: Option<MembersCursor> = decode_cursor(request.cursor.as_deref())?;
        if cursor.as_ref().is_some_and(|c| {
            c.collection_id != request.collection_id
                || c.include_trashed != request.include_trashed
                || c.kind != request.kind
        }) {
            return Err(Error::InvalidInput(
                "member cursor belongs to a different query",
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let collection = read_collection(&tx, &request.collection_id)?;
        let generation: i64 =
            tx.query_row("SELECT generation FROM note_list_state", [], |r| r.get(0))?;
        let source_generation: i64 =
            tx.query_row("SELECT generation FROM source_list_state", [], |r| r.get(0))?;
        let scope = library_scope(&self.database_path, &tx)?;
        if cursor.as_ref().is_some_and(|c| {
            c.generation != generation
                || c.row_version != collection.metadata.row_version
                || c.source_generation != source_generation
                || c.scope != scope
        }) {
            return Err(Error::QueryExpired);
        }
        let mut statement = tx.prepare("SELECT m.member_id,m.position FROM memberships m JOIN objects o ON o.id=m.member_id
            WHERE m.collection_id=?1 AND (?2=1 OR o.trashed=0) AND (?3 IS NULL OR m.position>?3) AND (?5 IS NULL OR o.kind=?5) ORDER BY m.position LIMIT ?4")?;
        let members = statement
            .query_map(
                params![
                    request.collection_id,
                    request.include_trashed,
                    cursor.as_ref().map(|c| c.position),
                    request.limit + 1,
                    kind,
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = members.len() > request.limit as usize;
        let items = members
            .iter()
            .take(request.limit as usize)
            .map(|(id, position)| {
                let object = read_metadata(&tx, id)?;
                let note = if object.kind == ObjectKind::Note {
                    Some(crate::queries::read_summary(&tx, id)?)
                } else {
                    None
                };
                let source = if object.kind == ObjectKind::Source {
                    Some(crate::sources::read_source(&tx, id)?)
                } else {
                    None
                };
                let annotation = if object.kind == ObjectKind::Annotation {
                    Some(crate::annotation_queries::read_annotation_summary(&tx, id)?)
                } else {
                    None
                };
                Ok(CollectionMember {
                    position: *position,
                    object,
                    note,
                    source,
                    annotation,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            Some(serde_json::to_string(&MembersCursor {
                collection_id: request.collection_id.clone(),
                position: items.last().ok_or(Error::IntegrityFailure)?.position,
                generation,
                row_version: collection.metadata.row_version,
                include_trashed: request.include_trashed,
                kind: request.kind,
                source_generation,
                scope,
            })?)
        } else {
            None
        };
        Ok(CollectionMembersPage {
            collection_id: request.collection_id,
            collection_row_version: collection.metadata.row_version,
            items,
            next_cursor,
        })
    }

    pub fn collection_create(
        &mut self,
        command_id: &str,
        request: CreateCollection,
    ) -> Result<CollectionCommit> {
        let kind = "collections.create";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let name = validate_name(&request.name)?;
        validate_pin(request.is_pinned, request.pin_position)?;
        validate_overview(&tx, request.overview_note_id.as_deref())?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO objects(id,kind) VALUES(?1,'collection')",
            [&id],
        )?;
        tx.execute("INSERT INTO collections(id,name,overview_note_id,is_pinned,pin_position,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",params![id,name,request.overview_note_id,request.is_pinned,request.pin_position,now])?;
        let result = finish(&tx, command_id, &hash, kind, &id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn collection_update(
        &mut self,
        command_id: &str,
        request: UpdateCollection,
    ) -> Result<CollectionCommit> {
        let kind = "collections.update";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let collection = checked_collection(&tx, &request.collection_id, request.base_row_version)?;
        let name = validate_name(&request.name)?;
        validate_pin(request.is_pinned, request.pin_position)?;
        if collection.overview_note_id != request.overview_note_id {
            validate_overview(&tx, request.overview_note_id.as_deref())?;
        }
        if collection.name != name
            || collection.overview_note_id != request.overview_note_id
            || collection.is_pinned != request.is_pinned
            || collection.pin_position != request.pin_position
        {
            bump_version(&tx, &collection.metadata)?;
            tx.execute("UPDATE collections SET name=?2,overview_note_id=?3,is_pinned=?4,pin_position=?5,updated_at=?6 WHERE id=?1",params![collection.id,name,request.overview_note_id,request.is_pinned,request.pin_position,Utc::now().to_rfc3339()])?;
        }
        let result = finish(&tx, command_id, &hash, kind, &collection.id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn collection_add_members(
        &mut self,
        command_id: &str,
        request: ChangeCollectionMembers,
    ) -> Result<CollectionCommit> {
        self.change_collection_members(command_id, request, MemberChange::Add)
    }
    pub fn collection_remove_members(
        &mut self,
        command_id: &str,
        request: ChangeCollectionMembers,
    ) -> Result<CollectionCommit> {
        self.change_collection_members(command_id, request, MemberChange::Remove)
    }
    pub fn collection_reorder(
        &mut self,
        command_id: &str,
        request: ChangeCollectionMembers,
    ) -> Result<CollectionCommit> {
        self.change_collection_members(command_id, request, MemberChange::Reorder)
    }

    fn change_collection_members(
        &mut self,
        command_id: &str,
        request: ChangeCollectionMembers,
        action: MemberChange,
    ) -> Result<CollectionCommit> {
        let kind = match action {
            MemberChange::Add => "collections.add_members",
            MemberChange::Remove => "collections.remove_members",
            MemberChange::Reorder => "collections.reorder",
        };
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let collection = checked_collection(&tx, &request.collection_id, request.base_row_version)?;
        let max_batch = if matches!(action, MemberChange::Reorder) {
            10000
        } else {
            100
        };
        if request.member_ids.len() > max_batch
            || (request.member_ids.is_empty() && !matches!(action, MemberChange::Reorder))
        {
            return Err(Error::InvalidInput("member batch exceeds limits"));
        }
        let mut seen = BTreeSet::new();
        for id in &request.member_ids {
            valid_uuid(id)?;
            if !seen.insert(id) {
                return Err(Error::InvalidInput("duplicate collection member"));
            }
            if matches!(action, MemberChange::Reorder) {
                continue;
            }
            let object = read_metadata(&tx, id)?;
            if object.kind == ObjectKind::Collection {
                return Err(Error::InvalidInput("nested collections are not supported"));
            }
            if matches!(action, MemberChange::Add) && object.trashed {
                return Err(Error::Trashed);
            }
        }
        let existing=tx.prepare("SELECT member_id,position FROM memberships WHERE collection_id=?1 ORDER BY position")?
            .query_map([&collection.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut changed = false;
        match action {
            MemberChange::Add => {
                let existing_ids: BTreeSet<_> = existing.iter().map(|(id, _)| id).collect();
                let new_ids: Vec<_> = request
                    .member_ids
                    .iter()
                    .filter(|id| !existing_ids.contains(*id))
                    .collect();
                if existing.len() + new_ids.len() > 10000 {
                    return Err(Error::InvalidInput("collection exceeds 10000 members"));
                }
                let first_position = existing
                    .last()
                    .map(|(_, position)| u64::from(*position) + 1)
                    .unwrap_or(0);
                for (position, id) in (first_position..).zip(new_ids) {
                    let slot = u32::try_from(position).map_err(|_| {
                        Error::InvalidInput("member position overflow; reorder collection first")
                    })?;
                    tx.execute("INSERT INTO memberships(collection_id,member_id,position) VALUES(?1,?2,?3)",params![collection.id,id,slot])?;
                    changed = true;
                }
            }
            MemberChange::Remove => {
                for id in &request.member_ids {
                    changed |= tx.execute(
                        "DELETE FROM memberships WHERE collection_id=?1 AND member_id=?2",
                        params![collection.id, id],
                    )? > 0;
                }
            }
            MemberChange::Reorder => {
                if existing.len() != request.member_ids.len()
                    || existing.iter().any(|(id, _)| !seen.contains(id))
                {
                    return Err(Error::InvalidInput(
                        "reorder requires every current member exactly once",
                    ));
                }
                changed = existing.iter().zip(&request.member_ids).enumerate().any(
                    |(position, ((id, old_position), new_id))| {
                        id != new_id || *old_position as usize != position
                    },
                );
                if changed {
                    tx.execute(
                        "DELETE FROM memberships WHERE collection_id=?1",
                        [&collection.id],
                    )?;
                    for (position, id) in request.member_ids.iter().enumerate() {
                        tx.execute("INSERT INTO memberships(collection_id,member_id,position) VALUES(?1,?2,?3)",params![collection.id,id,position as u32])?;
                    }
                }
            }
        }
        if changed {
            bump_version(&tx, &collection.metadata)?;
            tx.execute(
                "UPDATE collections SET updated_at=?2 WHERE id=?1",
                params![collection.id, Utc::now().to_rfc3339()],
            )?;
        }
        let result = finish(&tx, command_id, &hash, kind, &collection.id)?;
        tx.commit()?;
        Ok(result)
    }
}

enum MemberChange {
    Add,
    Remove,
    Reorder,
}

pub(crate) fn read_collection(connection: &Connection, id: &str) -> Result<CollectionDetail> {
    let (name,overview_note_id,is_pinned,pin_position,created_at,updated_at)=connection.query_row(
        "SELECT name,overview_note_id,is_pinned,pin_position,created_at,updated_at FROM collections WHERE id=?1",[id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    let metadata = read_metadata(connection, id)?;
    if metadata.kind != ObjectKind::Collection {
        return Err(Error::IntegrityFailure);
    }
    let member_count = connection.query_row(
        "SELECT count(*) FROM memberships WHERE collection_id=?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(CollectionDetail {
        id: id.into(),
        name,
        overview_note_id,
        is_pinned,
        pin_position,
        created_at,
        updated_at,
        metadata,
        member_count,
    })
}

fn checked_collection(connection: &Connection, id: &str, base: u32) -> Result<CollectionDetail> {
    valid_uuid(id)?;
    let collection = read_collection(connection, id)?;
    if collection.metadata.trashed {
        return Err(Error::Trashed);
    }
    check_version(id, base, collection.metadata.row_version)?;
    Ok(collection)
}
/// Called only inside notes.create's write transaction; its existing receipt
/// covers both the initial revision and membership, including lost-ack retries.
pub(crate) fn attach_new_note(
    connection: &Connection,
    collection_id: &str,
    base: u32,
    note_id: &str,
) -> Result<()> {
    let collection = checked_collection(connection, collection_id, base)?;
    if collection.member_count >= 10000 {
        return Err(Error::InvalidInput("collection exceeds 10000 members"));
    }
    let last: Option<u32> = connection.query_row(
        "SELECT max(position) FROM memberships WHERE collection_id=?1",
        [collection_id],
        |row| row.get(0),
    )?;
    let position = match last {
        None => 0,
        Some(last) => last.checked_add(1).ok_or(Error::InvalidInput(
            "member position overflow; reorder collection first",
        ))?,
    };
    connection.execute(
        "INSERT INTO memberships(collection_id,member_id,position) VALUES(?1,?2,?3)",
        params![collection_id, note_id, position],
    )?;
    bump_version(connection, &collection.metadata)?;
    connection.execute(
        "UPDATE collections SET updated_at=?2 WHERE id=?1",
        params![collection_id, Utc::now().to_rfc3339()],
    )?;
    connection.execute(
        "UPDATE objects
         SET row_version=row_version+CASE WHEN inbox_state='processed' THEN 0 ELSE 1 END,
             inbox_state='processed'
         WHERE id=?1",
        [note_id],
    )?;
    Ok(())
}
fn validate_overview(connection: &Connection, id: Option<&str>) -> Result<()> {
    if let Some(id) = id {
        valid_uuid(id)?;
        let object = read_metadata(connection, id)?;
        if object.kind != ObjectKind::Note {
            return Err(Error::InvalidInput(
                "collection overview must reference a note",
            ));
        }
        if object.trashed {
            return Err(Error::Trashed);
        }
    }
    Ok(())
}
fn finish(
    connection: &Connection,
    command_id: &str,
    hash: &str,
    kind: &str,
    id: &str,
) -> Result<CollectionCommit> {
    let result = CollectionCommit {
        collection: read_collection(connection, id)?,
        receipt: next_receipt(connection, command_id)?,
    };
    store_receipt(connection, hash, kind, id, &result.receipt, &result)?;
    Ok(result)
}
pub(crate) fn validate_limit(limit: u32) -> Result<()> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput("page limit must be 1-100"));
    }
    Ok(())
}
pub(crate) fn search_text(query: Option<&str>) -> Result<String> {
    let query = query.unwrap_or_default().trim();
    if query.chars().count() > 300 || query.contains('\0') {
        return Err(Error::InvalidInput("query exceeds bounds"));
    }
    Ok(query.into())
}
pub(crate) fn decode_cursor<T: serde::de::DeserializeOwned>(
    cursor: Option<&str>,
) -> Result<Option<T>> {
    cursor
        .map(|value| {
            if value.len() > 4096 {
                return Err(Error::InvalidInput("cursor exceeds bounds"));
            }
            serde_json::from_str(value).map_err(|_| Error::InvalidInput("invalid cursor"))
        })
        .transpose()
}
