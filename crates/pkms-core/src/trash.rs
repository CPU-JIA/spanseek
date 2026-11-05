//! Confirmed, atomic trash deletion. Registered blobs are deliberately retained.
use crate::{
    DeleteTrash, Error, Library, Result, TrashCommit, TrashDeletedObject, TrashItem, TrashPage,
    TrashPageRequest, TrashPreview, TrashPreviewRequest, TrashSelection, document::valid_uuid,
    mutations,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const KIND: &str = "trash.delete";
pub(crate) const PURGED_RECEIPT_KIND: &str = "objects.permanently_deleted";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after_id: String,
    query: String,
    generation: i64,
    scope: String,
}

fn count(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::InvalidInput("too many trash objects"))
}

fn ids(db: &Connection, sql: &str, selected: &str) -> Result<Vec<String>> {
    Ok(db
        .prepare(sql)?
        .query_map([selected], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

fn read_item(db: &Connection, id: &str) -> Result<TrashItem> {
    db.query_row(
        "SELECT o.kind,o.row_version,coalesce(n.title,s.display_title,c.name,ps.display_title,''),
            CASE o.kind WHEN 'note' THEN coalesce(p.preview,'') WHEN 'source' THEN coalesce(v.original_name,'')
                WHEN 'annotation' THEN substr(coalesce(json_extract(a.locator_json,'$.quote'),'')||CASE WHEN a.comment='' THEN '' ELSE ' · '||a.comment END,1,240) ELSE '' END,
            coalesce(n.updated_at,s.updated_at,c.updated_at,ps.updated_at),av.source_id,coalesce(po.trashed,0)
         FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN note_previews p ON p.note_id=n.id
         LEFT JOIN collections c ON c.id=o.id LEFT JOIN sources s ON s.id=o.id
         LEFT JOIN source_versions v ON v.id=s.current_version_id LEFT JOIN annotations a ON a.id=o.id
         LEFT JOIN source_versions av ON av.id=a.source_version_id LEFT JOIN sources ps ON ps.id=av.source_id
         LEFT JOIN objects po ON po.id=ps.id WHERE o.id=?1",
        [id],
        |r| Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional()?.ok_or(Error::NotFound).and_then(|row| Ok(TrashItem {
        object_id: id.into(),
        kind: serde_json::from_value(serde_json::Value::String(row.0))?,
        row_version: row.1, title: row.2, preview: row.3, updated_at: row.4,
        parent_source_id: row.5, source_trashed: row.6,
    }))
}

fn canonical_selection(selection: &TrashSelection) -> Result<TrashSelection> {
    match selection {
        TrashSelection::All => Ok(TrashSelection::All),
        TrashSelection::Objects { object_ids } => {
            if object_ids.is_empty() || object_ids.len() > 10_000 {
                return Err(Error::InvalidInput(
                    "trash selection must contain 1-10000 objects",
                ));
            }
            let mut unique = BTreeSet::new();
            for id in object_ids {
                valid_uuid(id)?;
                if !unique.insert(id.clone()) {
                    return Err(Error::InvalidInput("duplicate trash target"));
                }
            }
            Ok(TrashSelection::Objects {
                object_ids: unique.into_iter().collect(),
            })
        }
    }
}

fn preview(db: &Connection, selection: &TrashSelection, scope: &str) -> Result<TrashPreview> {
    let selection = canonical_selection(selection)?;
    let selected = match &selection {
        TrashSelection::All => ids(
            db,
            "SELECT id FROM objects WHERE trashed=1 AND ?1='' ORDER BY id",
            "",
        )?,
        TrashSelection::Objects { object_ids } => object_ids.clone(),
    };
    let mut affected: BTreeSet<String> = selected.iter().cloned().collect();
    for id in &selected {
        let row: Option<(String, bool)> = db
            .query_row("SELECT kind,trashed FROM objects WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((kind, true)) = row else {
            return Err(Error::QueryExpired);
        };
        if kind == "source" {
            affected.extend(ids(db,
                "SELECT a.id FROM annotations a JOIN source_versions v ON v.id=a.source_version_id WHERE v.source_id=?1 ORDER BY a.id", id)?);
        }
    }
    let items = affected
        .iter()
        .map(|id| read_item(db, id))
        .collect::<Result<Vec<_>>>()?;
    // The compact fingerprint includes exact note heads, every immutable source
    // version/active parse and annotation row versions, without exposing bodies.
    let affected_json = serde_json::to_string(&affected)?;
    let heads = ids(db,
        "SELECT json_array(o.id,o.kind,o.row_version,o.trashed,n.current_revision_id,s.current_version_id,a.source_version_id)
         FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN sources s ON s.id=o.id LEFT JOIN annotations a ON a.id=o.id
         WHERE o.id IN (SELECT value FROM json_each(?1)) ORDER BY o.id", &affected_json)?;
    let versions = ids(
        db,
        "SELECT json_array(v.id,v.source_id,v.blob_hash,p.parse_id) FROM source_versions v
         LEFT JOIN source_active_parses p ON p.source_version_id=v.id
         WHERE v.source_id IN (SELECT value FROM json_each(?1)) ORDER BY v.id",
        &affected_json,
    )?;
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let snapshot_token = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            1,
            workspace_id,
            scope,
            &selection,
            &items,
            heads,
            versions
        ))?)
    );
    Ok(TrashPreview {
        selection,
        snapshot_token,
        selected_count: count(selected.len())?,
        dependent_count: count(items.len() - selected.len())?,
        total_count: count(items.len())?,
        items,
    })
}

impl Library {
    fn trash_scope(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(self.database_path.to_string_lossy().as_bytes())
        )
    }

    pub fn permanently_deleted_ids(&self, object_ids: &[String]) -> Result<Vec<String>> {
        ids(
            &self.connection,
            "SELECT id FROM object_tombstones WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY id",
            &serde_json::to_string(object_ids)?,
        )
    }

    pub fn trash_page(&self, request: TrashPageRequest) -> Result<TrashPage> {
        if !(1..=100).contains(&request.limit) {
            return Err(Error::InvalidInput("page limit must be 1-100"));
        }
        let query = request.query.unwrap_or_default().trim().to_lowercase();
        if query.chars().count() > 300 || query.contains('\0') {
            return Err(Error::InvalidInput("invalid trash query"));
        }
        let cursor: Option<Cursor> = request
            .cursor
            .map(|s| {
                if s.len() > 4096 {
                    return Err(Error::InvalidInput("invalid trash cursor"));
                }
                serde_json::from_str(&s).map_err(|_| Error::InvalidInput("invalid trash cursor"))
            })
            .transpose()?;
        let tx = self.connection.unchecked_transaction()?;
        let scope = self.trash_scope();
        let generation: i64 = tx.query_row(
            "SELECT next_change_seq FROM workspace_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if let Some(cursor) = &cursor {
            valid_uuid(&cursor.after_id)?;
            if cursor.query != query {
                return Err(Error::InvalidInput("cursor does not match trash query"));
            }
            if cursor.scope != scope {
                return Err(Error::InvalidInput(
                    "cursor belongs to another local library",
                ));
            }
            if cursor.generation != generation {
                return Err(Error::QueryExpired);
            }
        }
        let matched = "FROM objects o LEFT JOIN notes n ON n.id=o.id LEFT JOIN sources s ON s.id=o.id
             LEFT JOIN collections c ON c.id=o.id LEFT JOIN annotations a ON a.id=o.id
             LEFT JOIN source_versions v ON v.id=a.source_version_id LEFT JOIN sources ps ON ps.id=v.source_id
             WHERE o.trashed=1 AND (?1='' OR instr(lower(coalesce(n.title,s.display_title,c.name,ps.display_title,'')||' '||coalesce(a.comment,'')||' '||coalesce(json_extract(a.locator_json,'$.quote'),'')),?1)>0)";
        let total = tx.query_row(&format!("SELECT count(*) {matched}"), [&query], |r| {
            r.get(0)
        })?;
        let page_ids = tx
            .prepare(&format!(
                "SELECT o.id {matched} AND (?2 IS NULL OR o.id>?2) ORDER BY o.id LIMIT ?3"
            ))?
            .query_map(
                params![
                    query,
                    cursor.as_ref().map(|c| &c.after_id),
                    request.limit + 1
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if page_ids.len() > request.limit as usize {
            Some(serde_json::to_string(&Cursor {
                after_id: page_ids[request.limit as usize - 1].clone(),
                query,
                generation,
                scope,
            })?)
        } else {
            None
        };
        let items = page_ids
            .iter()
            .take(request.limit as usize)
            .map(|id| read_item(&tx, id))
            .collect::<Result<_>>()?;
        Ok(TrashPage {
            items,
            next_cursor,
            total,
        })
    }

    pub fn preview_trash_delete(&self, request: TrashPreviewRequest) -> Result<TrashPreview> {
        let tx = self.connection.unchecked_transaction()?;
        preview(&tx, &request.selection, &self.trash_scope())
    }

    pub fn delete_trash(&mut self, command_id: &str, request: DeleteTrash) -> Result<TrashCommit> {
        let hash = mutations::request_hash(command_id, KIND, &request)?;
        let scope = self.trash_scope();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay(&tx, command_id, &hash, KIND)? {
            return Ok(result);
        }
        let snapshot = preview(&tx, &request.selection, &scope)?;
        if request.snapshot_token != snapshot.snapshot_token {
            return Err(Error::QueryExpired);
        }
        let deleted_objects = snapshot
            .items
            .iter()
            .map(|item| TrashDeletedObject {
                object_id: item.object_id.clone(),
                kind: item.kind,
            })
            .collect();
        delete_objects(&tx, &snapshot.items)?;
        let result = TrashCommit {
            deleted_objects,
            receipt: mutations::next_receipt(&tx, command_id)?,
        };
        for item in &result.deleted_objects {
            tx.execute(
                "INSERT INTO object_tombstones(id,kind,command_id,deleted_at) VALUES(?1,?2,?3,?4)",
                params![
                    item.object_id,
                    serde_json::to_value(item.kind)?.as_str(),
                    command_id,
                    result.receipt.committed_at
                ],
            )?;
        }
        // Purge receipts use their own identity, so future deletion cannot redact
        // an earlier purge's replayable identity-only result.
        mutations::store_receipt(&tx, &hash, KIND, command_id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
}

fn delete_objects(db: &Connection, items: &[TrashItem]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let object_ids: Vec<_> = items.iter().map(|item| &item.object_id).collect();
    let selected = serde_json::to_string(&object_ids)?;
    // Deferred FK checking handles the intentionally cyclic current revision /
    // current source version relationships. It never disables integrity checks.
    db.pragma_update(None, "defer_foreign_keys", true)?;

    let proposal_ids = ids(db,
        "SELECT id FROM proposals WHERE json_extract(request_json,'$.note_id') IN (SELECT value FROM json_each(?1))
         OR applied_revision_id IN (SELECT revision_id FROM note_revisions WHERE note_id IN (SELECT value FROM json_each(?1)))", &selected)?;
    let proposal_json = serde_json::to_string(&proposal_ids)?;
    let job_ids = ids(db,
        "SELECT id FROM source_jobs WHERE source_id IN (SELECT value FROM json_each(?1))
         OR source_version_id IN (SELECT id FROM source_versions WHERE source_id IN (SELECT value FROM json_each(?1)))
         OR json_extract(input_json,'$.add_version_to') IN (SELECT value FROM json_each(?1))", &selected)?;
    let job_json = serde_json::to_string(&job_ids)?;
    let rebuild_ids = ids(
        db,
        "SELECT DISTINCT job_id FROM index_rebuild_objects WHERE object_id IN (SELECT value FROM json_each(?1))",
        &selected,
    )?;
    let rebuild_json = serde_json::to_string(&rebuild_ids)?;

    // A receipt is a retry tombstone, not an alternate archive of deleted text.
    // Also redact mixed metadata receipts containing a deleted object's snapshot.
    db.execute(
        "UPDATE command_receipts SET command_kind=?2,revision_id=NULL,result_json=NULL
         WHERE command_kind!=?3 AND (object_id IN (SELECT value FROM json_each(?1))
          OR revision_id IN (SELECT revision_id FROM note_revisions WHERE note_id IN (SELECT value FROM json_each(?1)))
          OR object_id IN (SELECT value FROM json_each(?4)) OR object_id IN (SELECT value FROM json_each(?5))
          OR ((command_kind IN ('tags.assign','tags.remove','search.index_begin') OR command_kind LIKE 'collections.%' OR command_kind LIKE 'review.%') AND EXISTS(
              SELECT 1 FROM json_tree(command_receipts.result_json) j WHERE j.type='text' AND j.value IN (SELECT value FROM json_each(?1)))))",
        params![selected, PURGED_RECEIPT_KIND, KIND, proposal_json, job_json],
    )?;
    db.execute(
        "DELETE FROM proposal_blobs WHERE proposal_id IN (SELECT value FROM json_each(?1))",
        [&proposal_json],
    )?;
    db.execute(
        "DELETE FROM proposals WHERE id IN (SELECT value FROM json_each(?1))",
        [&proposal_json],
    )?;
    // A shared file's dedup owner may be deleted while another source still
    // retains that exact immutable version. Keep duplicate import deterministic.
    db.execute(
        "UPDATE source_file_dedup AS d SET source_id=(SELECT v.source_id FROM source_versions v JOIN objects o ON o.id=v.source_id
          WHERE v.blob_hash=d.blob_hash AND v.origin_url IS NULL AND v.source_id NOT IN (SELECT value FROM json_each(?1)) ORDER BY o.trashed,v.captured_at,v.source_id LIMIT 1)
         WHERE d.source_id IN (SELECT value FROM json_each(?1)) AND EXISTS(SELECT 1 FROM source_versions v WHERE v.blob_hash=d.blob_hash AND v.origin_url IS NULL AND v.source_id NOT IN (SELECT value FROM json_each(?1)))", [&selected])?;

    // Surviving collections keep their members and identity; removal/overview
    // invalidates their optimistic version, including already-open editors.
    db.execute(
        "UPDATE objects SET row_version=row_version+1 WHERE kind='collection' AND id NOT IN (SELECT value FROM json_each(?1))
         AND (id IN (SELECT collection_id FROM memberships WHERE member_id IN (SELECT value FROM json_each(?1)))
          OR id IN (SELECT id FROM collections WHERE overview_note_id IN (SELECT value FROM json_each(?1))))", [&selected])?;
    db.execute("UPDATE collections SET overview_note_id=NULL,updated_at=?2 WHERE overview_note_id IN (SELECT value FROM json_each(?1))", params![selected,chrono::Utc::now().to_rfc3339()])?;

    for sql in [
        "DELETE FROM review_events WHERE entry_id IN (SELECT id FROM review_entries WHERE object_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM review_entries WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM index_rebuild_objects WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM active_index_generations WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM index_chunks WHERE generation_id IN (SELECT id FROM index_generations WHERE object_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM index_generations WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM index_dirty_objects WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM memberships WHERE collection_id IN (SELECT value FROM json_each(?1)) OR member_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM object_tags WHERE object_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_links WHERE source_id IN (SELECT value FROM json_each(?1)) OR target_id IN (SELECT value FROM json_each(?1))",
        // Incoming body_links are retained: their owning documents are immutable
        // independent snapshots, and resolution now truthfully returns missing.
        "DELETE FROM body_links WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_fts WHERE note_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_index_queue WHERE note_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_previews WHERE note_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_revision_blobs WHERE revision_id IN (SELECT revision_id FROM note_revisions WHERE note_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM note_revision_blocks WHERE revision_id IN (SELECT revision_id FROM note_revisions WHERE note_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM blocks WHERE note_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM note_revisions WHERE note_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM notes WHERE id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM annotations WHERE id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM reading_states WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_fts WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_title_fts WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_parse_staging WHERE parse_id IN (SELECT p.id FROM source_parses p JOIN source_versions v ON v.id=p.source_version_id WHERE v.source_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM source_blocks WHERE parse_id IN (SELECT p.id FROM source_parses p JOIN source_versions v ON v.id=p.source_version_id WHERE v.source_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM source_active_parses WHERE source_version_id IN (SELECT id FROM source_versions WHERE source_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM source_parses WHERE source_version_id IN (SELECT id FROM source_versions WHERE source_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM source_web_captures WHERE source_version_id IN (SELECT id FROM source_versions WHERE source_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM source_file_dedup WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_versions WHERE source_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM sources WHERE id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM collections WHERE id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM objects WHERE id IN (SELECT value FROM json_each(?1))",
    ] {
        db.execute(sql, [&selected])?;
    }
    for sql in [
        "DELETE FROM source_parse_staging WHERE job_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_job_events WHERE job_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_job_attempts WHERE job_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM source_jobs WHERE id IN (SELECT value FROM json_each(?1))",
    ] {
        db.execute(sql, [&job_json])?;
    }
    // A rebuild's processed counter and cursor are derived from its surviving
    // manifest. Keeping the old count would make complete-package restore fail.
    db.execute(
        "UPDATE index_rebuild_jobs AS j SET
           processed=(SELECT count(*) FROM index_rebuild_objects m WHERE m.job_id=j.id AND (m.generation_id IS NOT NULL OR m.skip_reason IS NOT NULL)),
           after_object_id=(SELECT max(object_id) FROM index_rebuild_objects m WHERE m.job_id=j.id AND (m.generation_id IS NOT NULL OR m.skip_reason IS NOT NULL)),
           status=CASE WHEN status IN ('queued','running') THEN 'canceled' ELSE status END,
           error_code=CASE WHEN status IN ('queued','running') THEN 'objects_deleted' ELSE error_code END,
           lease_token=NULL,lease_expires_at=NULL,updated_at=?2
         WHERE id IN (SELECT value FROM json_each(?1))", params![rebuild_json,chrono::Utc::now().to_rfc3339()])?;
    // Existing generations do not have delete triggers for every object kind.
    for table in [
        "note_list_state",
        "source_list_state",
        "collection_list_state",
    ] {
        db.execute(
            &format!("UPDATE {table} SET generation=generation+1 WHERE singleton=1"),
            [],
        )?;
    }
    Ok(())
}
