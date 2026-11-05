use crate::{AnnotationPage, AnnotationSummary, AnnotationsQueryRequest, Error, Library, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// A byte prefix of 1120 bytes always contains the first 280 Unicode codepoints.
// BLOB substr also preserves embedded NULs in comments, unlike SQLite TEXT substr.
// SQLite returns NULL for substr of an empty BLOB; empty comments are valid.
const FIELDS: &str = "a.id,v.source_id,s.display_title,a.source_version_id,
    substr(CAST(json_extract(a.locator_json,'$.quote') AS BLOB),1,1120),
    coalesce(substr(CAST(a.comment AS BLOB),1,1120),X''),o.row_version,o.trashed,parent.trashed,
    parent.search_enabled,v.format,json_extract(a.locator_json,'$.page_index')";
const JOINS: &str = "FROM annotations a JOIN objects o ON o.id=a.id
    JOIN source_versions v ON v.id=a.source_version_id JOIN sources s ON s.id=v.source_id
    JOIN objects parent ON parent.id=s.id";

fn preview(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    let bytes: Vec<u8> = row.get(index)?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Blob,
                    Box::new(error),
                )
            })?
        }
        Err(error) => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Blob,
                Box::new(error),
            ));
        }
    };
    Ok(text.chars().take(280).collect())
}

fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnnotationSummary> {
    let format: String = row.get(10)?;
    let source_format =
        serde_json::from_value(serde_json::Value::String(format)).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                10,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(AnnotationSummary {
        id: row.get(0)?,
        source_id: row.get(1)?,
        source_title: row.get(2)?,
        source_version_id: row.get(3)?,
        quote_preview: preview(row, 4)?,
        comment_preview: preview(row, 5)?,
        row_version: row.get(6)?,
        trashed: row.get(7)?,
        source_trashed: row.get(8)?,
        source_search_enabled: row.get(9)?,
        source_format,
        page_index: row.get(11)?,
    })
}

pub(crate) fn read_annotation_summary(db: &Connection, id: &str) -> Result<AnnotationSummary> {
    crate::document::valid_uuid(id)?;
    db.query_row(
        &format!("SELECT {FIELDS} {JOINS} WHERE a.id=?1"),
        [id],
        summary,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnnotationCursor {
    scope: String,
    after_id: String,
    change_seq: i64,
    source_generation: i64,
    note_generation: i64,
}

impl Library {
    pub fn annotations_query(&self, request: AnnotationsQueryRequest) -> Result<AnnotationPage> {
        crate::collections::validate_limit(request.limit)?;
        let query = crate::collections::search_text(request.query.as_deref())?.to_ascii_lowercase();
        let snapshot = self.connection.unchecked_transaction()?;
        let (workspace, change_seq): (String, i64) = snapshot.query_row(
            "SELECT workspace_id,next_change_seq FROM workspace_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let source_generation: i64 = snapshot.query_row(
            "SELECT generation FROM source_list_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let note_generation: i64 = snapshot.query_row(
            "SELECT generation FROM note_list_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let scope = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                "annotations-query-v1",
                workspace,
                self.database_path.to_string_lossy(),
                &query,
            ))?)
        );
        let cursor: Option<AnnotationCursor> =
            crate::collections::decode_cursor(request.cursor.as_deref())?;
        if let Some(cursor) = &cursor {
            crate::document::valid_uuid(&cursor.after_id)?;
            if cursor.scope != scope
                || cursor.change_seq < 1
                || cursor.source_generation < 1
                || cursor.note_generation < 1
            {
                return Err(Error::InvalidInput(
                    "annotation cursor does not match query or workspace",
                ));
            }
            if cursor.change_seq != change_seq
                || cursor.source_generation != source_generation
                || cursor.note_generation != note_generation
            {
                return Err(Error::QueryExpired);
            }
        }
        let sql = format!(
            "SELECT {FIELDS} {JOINS}
            WHERE o.trashed=0 AND parent.trashed=0
              AND (?1='' OR instr(lower(s.display_title),?1)>0
                OR instr(lower(json_extract(a.locator_json,'$.quote')),?1)>0
                OR instr(lower(a.comment),?1)>0)
              AND (?2 IS NULL OR a.id>?2)
            ORDER BY a.id LIMIT ?3"
        );
        let mut items = snapshot
            .prepare(&sql)?
            .query_map(
                params![
                    query,
                    cursor.as_ref().map(|cursor| &cursor.after_id),
                    request.limit + 1
                ],
                summary,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = items.len() > request.limit as usize;
        items.truncate(request.limit as usize);
        let next_cursor = if more {
            Some(serde_json::to_string(&AnnotationCursor {
                scope,
                after_id: items.last().ok_or(Error::IntegrityFailure)?.id.clone(),
                change_seq,
                source_generation,
                note_generation,
            })?)
        } else {
            None
        };
        snapshot.commit()?;
        Ok(AnnotationPage { items, next_cursor })
    }
}
