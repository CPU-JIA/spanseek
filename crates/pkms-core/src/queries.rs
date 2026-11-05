use crate::{
    Error, Library, NoteDocument, NotePage, NoteSort, NoteSummary, NotesFilter, NotesPageRequest,
    Result, RevisionPage, RevisionSummary,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotesCursor {
    sort_value: String,
    id: String,
    trashed: bool,
    filter_hash: String,
    generation: i64,
    scope: Option<(String, u32)>,
}

impl Library {
    /// Bounded summaries for the desktop. Documents never cross IPC in list responses.
    pub fn notes_page(&self, request: NotesPageRequest) -> Result<NotePage> {
        let snapshot = self.connection.unchecked_transaction()?;
        notes_page_in(&snapshot, request, None)
    }

    pub fn revision_page(
        &self,
        note_id: &str,
        before: Option<u32>,
        limit: u32,
    ) -> Result<RevisionPage> {
        if !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput("revision page limit must be 1-100"));
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1)",
            [note_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        let mut statement=self.connection.prepare("SELECT revision_id,revision,title,schema_version,author_kind,created_at FROM note_revisions WHERE note_id=?1 AND (?2 IS NULL OR revision<?2) ORDER BY revision DESC LIMIT ?3")?;
        let mut items = statement
            .query_map(params![note_id, before, limit + 1], |row| {
                Ok(RevisionSummary {
                    revision_id: row.get(0)?,
                    revision: row.get(1)?,
                    title: row.get(2)?,
                    document_schema: row.get(3)?,
                    author_kind: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_before = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|last| last.revision)
        } else {
            None
        };
        Ok(RevisionPage { items, next_before })
    }
}

/// A bounded lookup key for object-type browsing. The YAML remains the source
/// of truth; this only mirrors the first `type`/`object_type` scalar so list
/// filters can use an index instead of parsing every note document.
pub(crate) fn frontmatter_type_key(document: &NoteDocument) -> Option<String> {
    let frontmatter = document.content.first()?;
    if frontmatter.kind != crate::NodeType::Frontmatter {
        return None;
    }
    let source = frontmatter.attrs.source.as_deref()?.trim();
    let serde_yaml_ng::Value::Mapping(mapping) = serde_yaml_ng::from_str(source).ok()? else {
        return None;
    };
    for (key, value) in mapping.iter() {
        let serde_yaml_ng::Value::String(key) = key else {
            continue;
        };
        if key != "type" && key != "object_type" {
            continue;
        }
        let serde_yaml_ng::Value::String(value) = value else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() && value.chars().count() <= 48 && !value.chars().any(char::is_control)
        {
            return Some(value.to_lowercase());
        }
        return None;
    }
    None
}
pub(crate) fn normalize_filter(mut filter: NotesFilter) -> Result<NotesFilter> {
    if filter.tag_ids.len() > 100 {
        return Err(Error::InvalidInput("tag filter exceeds 100 tags"));
    }
    for id in &filter.tag_ids {
        crate::document::valid_uuid(id)?;
    }
    if let Some(id) = &filter.collection_id {
        crate::document::valid_uuid(id)?;
    }
    if filter.object_types.len() > 64 {
        return Err(Error::InvalidInput("object type filter exceeds 64 values"));
    }
    for value in &filter.object_types {
        if value != "__untyped" && !valid_object_type_token(value) {
            return Err(Error::InvalidInput("invalid object type filter token"));
        }
    }
    filter.tag_ids.sort();
    filter.tag_ids.dedup();
    filter.object_types = filter
        .object_types
        .into_iter()
        .map(|value| value.to_lowercase())
        .collect();
    filter.object_types.sort();
    filter.object_types.dedup();
    filter.archived = Some(filter.archived.unwrap_or(false));
    Ok(filter)
}

pub(crate) fn valid_object_type_token(value: &str) -> bool {
    let Some(first) = value.chars().next() else {
        return false;
    };
    value.chars().count() <= 48
        && first.is_alphanumeric()
        && value
            .chars()
            .all(|char| char.is_alphanumeric() || char == '_' || char == '-')
}

// A saved view is read and executed inside the same SQLite snapshot. Its identity
// and row version also scope the cursor, so editing a view cannot mix result pages.
pub(crate) fn notes_page_in(
    connection: &Connection,
    request: NotesPageRequest,
    scope: Option<(&str, u32)>,
) -> Result<NotePage> {
    crate::collections::validate_limit(request.limit)?;
    let filter = normalize_filter(request.filter.unwrap_or_default())?;
    let sort = request.sort.unwrap_or_default();
    let query = request.query.unwrap_or_default();
    if query.chars().count() > 512 || query.trim().chars().any(char::is_control) {
        return Err(Error::InvalidInput(
            "title query exceeds 512 characters or contains control characters",
        ));
    }
    let query = query.trim().to_ascii_lowercase();
    let filter_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&filter, sort, scope, &query))?)
    );
    let cursor: Option<NotesCursor> = crate::collections::decode_cursor(request.cursor.as_deref())?;
    if let Some(cursor) = &cursor {
        match (&cursor.scope, scope) {
            (Some((id, version)), Some((current, current_version))) if id == current => {
                if *version != current_version {
                    return Err(Error::QueryExpired);
                }
            }
            (None, None) => {}
            _ => return Err(Error::InvalidInput("cursor belongs to a different view")),
        }
        if cursor.trashed != request.trashed
            || cursor.filter_hash != filter_hash
            || uuid::Uuid::parse_str(&cursor.id).is_err()
        {
            return Err(Error::InvalidInput(
                "cursor does not match this notes query",
            ));
        }
        if matches!(sort, NoteSort::UpdatedAsc | NoteSort::UpdatedDesc)
            && chrono::DateTime::parse_from_rfc3339(&cursor.sort_value).is_err()
        {
            return Err(Error::InvalidInput("invalid cursor timestamp"));
        }
    }
    let generation: i64 =
        connection.query_row("SELECT generation FROM note_list_state", [], |r| r.get(0))?;
    if cursor.as_ref().is_some_and(|c| c.generation != generation) {
        return Err(Error::QueryExpired);
    }
    let (column, direction, comparison) = match sort {
        NoteSort::UpdatedDesc => ("n.updated_at", "DESC", "<"),
        NoteSort::UpdatedAsc => ("n.updated_at", "ASC", ">"),
        NoteSort::TitleAsc => ("n.title", "ASC", ">"),
        NoteSort::TitleDesc => ("n.title", "DESC", "<"),
    };
    // Only fixed enum choices supply SQL identifiers/operators; all user values bind.
    let sql=format!("SELECT n.id,r.title,r.revision_id,r.schema_version,n.updated_at,p.preview,o.row_version,o.inbox_state,o.archived,o.search_enabled,n.trashed
        FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
        JOIN note_previews p ON p.note_id=n.id AND p.revision_id=r.revision_id
        JOIN objects o ON o.id=n.id AND o.kind='note'
        WHERE n.trashed=?1 AND (?2 IS NULL OR {column}{comparison}?2 OR ({column}=?2 AND n.id>?3))
        AND (?5 IS NULL OR o.inbox_state=?5)
        AND NOT EXISTS(SELECT 1 FROM json_each(?6) selected WHERE NOT EXISTS(SELECT 1 FROM object_tags ot WHERE ot.object_id=n.id AND ot.tag_id=selected.value))
        AND (?7 IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.member_id=n.id AND m.collection_id=?7))
        AND (?1=1 OR o.archived=?8) AND (?9 IS NULL OR o.search_enabled=?9)
        AND (?11=1 OR EXISTS(
            SELECT 1 FROM json_each(?12) chosen
            WHERE (chosen.value='__untyped' AND n.type_key IS NULL)
               OR (chosen.value!='__untyped' AND n.type_key=chosen.value)
        ))
        AND (?10='' OR instr(lower(n.title),?10)>0)
        ORDER BY {column} {direction},n.id LIMIT ?4");
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params![
        request.trashed,
        cursor.as_ref().map(|c| &c.sort_value),
        cursor.as_ref().map(|c| &c.id),
        request.limit + 1,
        filter.inbox_state.map(crate::InboxState::as_str),
        serde_json::to_string(&filter.tag_ids)?,
        filter.collection_id,
        filter.archived,
        filter.search_enabled,
        query,
        i64::from(filter.object_types.is_empty()),
        serde_json::to_string(&filter.object_types)?
    ])?;
    let mut items = vec![];
    let mut more = false;
    while let Some(row) = rows.next()? {
        if items.len() == request.limit as usize {
            more = true;
            break;
        }
        items.push(summary_from_row(row)?);
    }
    let next_cursor = if more {
        let last = items.last().ok_or(Error::IntegrityFailure)?;
        let sort_value = match sort {
            NoteSort::UpdatedAsc | NoteSort::UpdatedDesc => &last.updated_at,
            _ => &last.title,
        };
        Some(serde_json::to_string(&NotesCursor {
            sort_value: sort_value.clone(),
            id: last.id.clone(),
            trashed: request.trashed,
            filter_hash,
            generation,
            scope: scope.map(|(id, version)| (id.to_owned(), version)),
        })?)
    } else {
        None
    };
    Ok(NotePage { items, next_cursor })
}

pub(crate) fn read_summary(connection: &Connection, id: &str) -> Result<NoteSummary> {
    let mut statement=connection.prepare("SELECT n.id,r.title,r.revision_id,r.schema_version,n.updated_at,p.preview,o.row_version,o.inbox_state,o.archived,o.search_enabled,n.trashed
        FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
        JOIN note_previews p ON p.note_id=n.id AND p.revision_id=r.revision_id JOIN objects o ON o.id=n.id WHERE n.id=?1")?;
    let mut rows = statement.query([id])?;
    let row = rows.next()?.ok_or(Error::NotFound)?;
    summary_from_row(row)
}
fn summary_from_row(row: &rusqlite::Row<'_>) -> Result<NoteSummary> {
    Ok(NoteSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        revision_id: row.get(2)?,
        document_schema: row.get(3)?,
        updated_at: row.get(4)?,
        preview: row.get(5)?,
        row_version: row.get(6)?,
        inbox_state: crate::organization::parse_inbox(row.get::<_, Option<String>>(7)?.as_deref())?,
        archived: row.get(8)?,
        search_enabled: row.get(9)?,
        trashed: row.get(10)?,
    })
}
