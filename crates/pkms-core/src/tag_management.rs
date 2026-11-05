//! Versioned dictionary management. Assignment APIs keep their existing tag DTO.
use crate::{
    CreateTag, DeleteTag, Error, Library, Result, Tag, TagBrowseItem, TagDetails,
    TagManagementCommit, TagViewUsage, TagsBrowsePage, TagsBrowseRequest, UpdateTag,
    document::valid_uuid,
    mutations::{check_version, next_receipt, replay, request_hash, store_receipt},
    organization::normalize_name,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn description(value: &str) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > 4000 || value.contains('\0') {
        return Err(Error::InvalidInput("tag description exceeds bounds"));
    }
    Ok(value.into())
}

fn ensure_available(db: &Connection, normalized: &str, except: Option<&str>) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE normalized_name=?1 AND (?2 IS NULL OR id<>?2))",
        params![normalized, except],
        |row| row.get(0),
    )?;
    if exists {
        return Err(Error::InvalidInput("tag name is already in use"));
    }
    Ok(())
}

/// The full affected object/version set binds confirmation, not just its count.
pub(crate) fn details(db: &Connection, id: &str) -> Result<TagDetails> {
    let mut result = db.query_row(
        "SELECT id,display_name,normalized_name,description,row_version,created_at,updated_at FROM tags WHERE id=?1",
        [id], |row| Ok(TagDetails {
            tag: Tag { id: row.get(0)?, display_name: row.get(1)?, normalized_name: row.get(2)? },
            description: row.get(3)?, row_version: row.get(4)?, created_at: row.get(5)?, updated_at: row.get(6)?,
            object_count: 0, active_object_count: 0, trashed_object_count: 0,
            note_count: 0, source_count: 0, annotation_count: 0, collection_count: 0,
            saved_views: Vec::new(), impact_token: String::new(),
        }),
    ).optional()?.ok_or(Error::NotFound)?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(serde_json::to_vec(&(id, result.row_version))?);
    let mut statement = db.prepare(
        "SELECT o.id,o.row_version,o.kind,(o.trashed OR COALESCE(parent.trashed,0))
         FROM objects o JOIN object_tags ot ON ot.object_id=o.id
         LEFT JOIN annotations a ON a.id=o.id LEFT JOIN source_versions v ON v.id=a.source_version_id
         LEFT JOIN objects parent ON parent.id=v.source_id
         WHERE ot.tag_id=?1 ORDER BY o.id",
    )?;
    for row in statement.query_map([id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
        ))
    })? {
        let row = row?;
        fingerprint.update(serde_json::to_vec(&row)?);
        result.object_count = result
            .object_count
            .checked_add(1)
            .ok_or(Error::IntegrityFailure)?;
        if row.3 {
            result.trashed_object_count += 1;
        } else {
            result.active_object_count += 1;
        }
        match row.2.as_str() {
            "note" => result.note_count += 1,
            "source" => result.source_count += 1,
            "annotation" => result.annotation_count += 1,
            "collection" => result.collection_count += 1,
            _ => return Err(Error::IntegrityFailure),
        }
    }
    let mut statement = db.prepare(
        "SELECT v.id,v.name,v.row_version FROM saved_views v WHERE EXISTS(SELECT 1 FROM json_each(v.filter_ast,'$.tag_ids') item WHERE item.value=?1) ORDER BY v.id",
    )?;
    for row in statement.query_map([id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
        ))
    })? {
        let (id, name, version) = row?;
        fingerprint.update(serde_json::to_vec(&(&id, &name, version))?);
        result.saved_views.push(TagViewUsage { id, name });
    }
    result.impact_token = format!("{:x}", fingerprint.finalize());
    Ok(result)
}

fn bump_assigned_objects(db: &Connection, tag_id: &str) -> Result<u32> {
    let overflow: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM objects o JOIN object_tags ot ON ot.object_id=o.id WHERE ot.tag_id=?1 AND o.row_version>=4294967295)",
        [tag_id], |row| row.get(0),
    )?;
    if overflow {
        return Err(Error::InvalidInput("metadata version overflow"));
    }
    let changed = db.execute(
        "UPDATE objects SET row_version=row_version+1 WHERE id IN (SELECT object_id FROM object_tags WHERE tag_id=?1)",
        [tag_id],
    )?;
    u32::try_from(changed).map_err(|_| Error::IntegrityFailure)
}

impl Library {
    pub fn tag_details(&self, id: &str) -> Result<TagDetails> {
        valid_uuid(id)?;
        let tx = self.connection.unchecked_transaction()?;
        details(&tx, id)
    }

    pub fn tags_browse(&self, request: TagsBrowseRequest) -> Result<TagsBrowsePage> {
        if !(1..=100).contains(&request.limit) || request.page == 0 {
            return Err(Error::InvalidInput("invalid tag page"));
        }
        let query = request.query.as_deref().unwrap_or_default().trim();
        let query = if query.is_empty() {
            String::new()
        } else {
            normalize_name(query)?.1
        };
        let tx = self.connection.unchecked_transaction()?;
        if let Some(id) = &request.collection_id {
            valid_uuid(id)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM collections c JOIN objects o ON o.id=c.id WHERE c.id=?1 AND o.trashed=0)",
                [id], |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        // An excerpt inherits the collection membership of its source, matching
        // knowledge browsing. Counts include archived, but never trashed objects.
        let scope = "WITH usage AS (
            SELECT ot.tag_id,count(*) AS n FROM object_tags ot JOIN objects o ON o.id=ot.object_id
            LEFT JOIN annotations a ON a.id=o.id LEFT JOIN source_versions v ON v.id=a.source_version_id
            LEFT JOIN objects parent ON parent.id=v.source_id
            WHERE o.trashed=0 AND COALESCE(parent.trashed,0)=0 AND (?2 IS NULL OR EXISTS(
                SELECT 1 FROM memberships m WHERE m.collection_id=?2 AND (
                    m.member_id=o.id OR m.member_id=(SELECT v.source_id FROM annotations a JOIN source_versions v ON v.id=a.source_version_id WHERE a.id=o.id)
                )
            )) GROUP BY ot.tag_id
        ) ";
        let condition = " FROM tags t LEFT JOIN usage u ON u.tag_id=t.id
            WHERE (instr(t.normalized_name,?1)>0 OR instr(lower(t.description),?1)>0)
            AND (?2 IS NULL OR COALESCE(u.n,0)>0) ";
        let total: u32 = tx.query_row(
            &format!("{scope} SELECT count(*) {condition}"),
            params![query, request.collection_id],
            |row| row.get(0),
        )?;
        let pages = total.max(1).div_ceil(request.limit);
        let page = request.page.min(pages);
        let mut statement = tx.prepare(&format!(
            "{scope} SELECT t.id,t.display_name,t.normalized_name,t.description,t.row_version,t.updated_at,COALESCE(u.n,0)
             {condition} ORDER BY t.normalized_name,t.id LIMIT ?3 OFFSET ?4"
        ))?;
        let items = statement
            .query_map(
                params![
                    query,
                    request.collection_id,
                    request.limit,
                    i64::from(page - 1) * i64::from(request.limit)
                ],
                |row| {
                    Ok(TagBrowseItem {
                        tag: Tag {
                            id: row.get(0)?,
                            display_name: row.get(1)?,
                            normalized_name: row.get(2)?,
                        },
                        description: row.get(3)?,
                        row_version: row.get(4)?,
                        updated_at: row.get(5)?,
                        object_count: row.get(6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(TagsBrowsePage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }

    pub fn tagged_objects(
        &self,
        request: crate::TaggedObjectsRequest,
    ) -> Result<crate::TaggedObjectsPage> {
        valid_uuid(&request.tag_id)?;
        if request.page == 0 || !(1..=100).contains(&request.limit) {
            return Err(Error::InvalidInput("invalid tagged objects page"));
        }
        if let Some(id) = &request.collection_id {
            valid_uuid(id)?;
        }
        let tx = self.connection.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1)",
            [&request.tag_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        if let Some(id) = &request.collection_id {
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM collections c JOIN objects o ON o.id=c.id WHERE c.id=?1 AND o.trashed=0)", [id], |row| row.get(0))?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        let base = "WITH tagged AS (
            SELECT o.id,o.kind,
                CASE o.kind WHEN 'note' THEN n.title WHEN 'collection' THEN c.name WHEN 'source' THEN s.display_title
                    ELSE substr(COALESCE(NULLIF(json_extract(a.locator_json,'$.quote'),''),NULLIF(a.comment,''),parent_source.display_title,''),1,300) END AS title,
                (o.archived OR COALESCE(parent.archived,0)) AS archived,
                (o.trashed OR COALESCE(parent.trashed,0)) AS trashed,
                CASE o.kind WHEN 'source' THEN o.id WHEN 'annotation' THEN v.source_id ELSE NULL END AS source_id,
                CASE o.kind WHEN 'source' THEN s.current_version_id WHEN 'annotation' THEN a.source_version_id ELSE NULL END AS source_version_id,
                COALESCE(parent.trashed,0) AS source_trashed
            FROM objects o JOIN object_tags ot ON ot.object_id=o.id
            LEFT JOIN notes n ON n.id=o.id LEFT JOIN collections c ON c.id=o.id LEFT JOIN sources s ON s.id=o.id
            LEFT JOIN annotations a ON a.id=o.id LEFT JOIN source_versions v ON v.id=a.source_version_id
            LEFT JOIN objects parent ON parent.id=v.source_id LEFT JOIN sources parent_source ON parent_source.id=v.source_id
            WHERE ot.tag_id=?1 AND (?3 IS NULL OR EXISTS(
                SELECT 1 FROM memberships m WHERE m.collection_id=?3 AND (m.member_id=o.id OR m.member_id=v.source_id)
            ))
        ) ";
        let total: u32 = tx.query_row(
            &format!("{base} SELECT count(*) FROM tagged WHERE trashed=?2"),
            params![request.tag_id, request.trashed, request.collection_id],
            |row| row.get(0),
        )?;
        let page = request.page.min(total.max(1).div_ceil(request.limit));
        let mut statement = tx.prepare(&format!("{base} SELECT id,kind,title,archived,trashed,source_id,source_version_id,source_trashed FROM tagged WHERE trashed=?2 ORDER BY title,id LIMIT ?4 OFFSET ?5"))?;
        let items = statement
            .query_map(
                params![
                    request.tag_id,
                    request.trashed,
                    request.collection_id,
                    request.limit,
                    i64::from(page - 1) * i64::from(request.limit)
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, bool>(7)?,
                    ))
                },
            )?
            .map(|row| {
                let (
                    id,
                    kind,
                    title,
                    archived,
                    trashed,
                    source_id,
                    source_version_id,
                    source_trashed,
                ) = row?;
                let kind = match kind.as_str() {
                    "note" => crate::ObjectKind::Note,
                    "source" => crate::ObjectKind::Source,
                    "annotation" => crate::ObjectKind::Annotation,
                    "collection" => crate::ObjectKind::Collection,
                    _ => return Err(Error::IntegrityFailure),
                };
                Ok(crate::TaggedObject {
                    id,
                    kind,
                    title,
                    archived,
                    trashed,
                    source_id,
                    source_version_id,
                    source_trashed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::TaggedObjectsPage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }

    pub fn tag_create(
        &mut self,
        command_id: &str,
        request: CreateTag,
    ) -> Result<TagManagementCommit> {
        let kind = "tags.create";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        let (name, normalized) = normalize_name(&request.name)?;
        let description = description(&request.description)?;
        ensure_available(&tx, &normalized, None)?;
        let id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO tags(id,display_name,normalized_name,description) VALUES(?1,?2,?3,?4)",
            params![id, name, normalized, description],
        )?;
        let result = TagManagementCommit {
            tag: details(&tx, &id)?,
            deleted: false,
            affected_objects: 0,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn tag_update(
        &mut self,
        command_id: &str,
        request: UpdateTag,
    ) -> Result<TagManagementCommit> {
        let kind = "tags.update";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        valid_uuid(&request.id)?;
        let old = details(&tx, &request.id)?;
        check_version(&request.id, request.base_row_version, old.row_version)?;
        let (name, normalized) = normalize_name(&request.name)?;
        let description = description(&request.description)?;
        ensure_available(&tx, &normalized, Some(&request.id))?;
        let renamed = name != old.tag.display_name;
        if renamed {
            crate::automation::invalidate_tag_suggestions(&tx, &old.tag.normalized_name)?;
        }
        let affected_objects = if renamed {
            bump_assigned_objects(&tx, &request.id)?
        } else {
            0
        };
        if renamed || description != old.description {
            let version = old
                .row_version
                .checked_add(1)
                .ok_or(Error::InvalidInput("tag version overflow"))?;
            tx.execute("UPDATE tags SET display_name=?2,normalized_name=?3,description=?4,row_version=?5,updated_at=?6 WHERE id=?1", params![request.id,name,normalized,description,version,Utc::now().to_rfc3339()])?;
        }
        let result = TagManagementCommit {
            tag: details(&tx, &request.id)?,
            deleted: false,
            affected_objects,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &request.id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn tag_delete(
        &mut self,
        command_id: &str,
        request: DeleteTag,
    ) -> Result<TagManagementCommit> {
        let kind = "tags.delete";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        valid_uuid(&request.id)?;
        let old = details(&tx, &request.id)?;
        check_version(&request.id, request.base_row_version, old.row_version)?;
        if request.impact_token != old.impact_token {
            return Err(Error::QueryExpired);
        }
        // Never silently broaden a saved view by removing one of its filters.
        // Its owner must edit the listed views explicitly before deleting a tag.
        if !old.saved_views.is_empty() {
            return Err(Error::InvalidInput("tag is used by saved views"));
        }
        let affected_objects = bump_assigned_objects(&tx, &request.id)?;
        crate::automation::invalidate_tag_suggestions(&tx, &old.tag.normalized_name)?;
        tx.execute("DELETE FROM object_tags WHERE tag_id=?1", [&request.id])?;
        tx.execute("DELETE FROM tags WHERE id=?1", [&request.id])?;
        let result = TagManagementCommit {
            tag: old,
            deleted: true,
            affected_objects,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &request.id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
}
