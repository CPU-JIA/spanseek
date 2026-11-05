use crate::{
    Error, InboxState, KnowledgeBrowsePage, KnowledgeBrowseRequest, KnowledgeObject,
    KnowledgeObjectFilter, KnowledgeObjectHit, KnowledgeObjectPage, KnowledgeObjectPageRequest,
    KnowledgeTypeCount, Library, ObjectCollectionRef, ObjectKind, Result,
    annotation_queries::read_annotation_summary,
    document::valid_uuid,
    graph::relation_targets,
    mutations::request_hash,
    organization::read_metadata,
    queries::{read_summary, valid_object_type_token},
    sources::read_source,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct KnowledgeCursor {
    updated_at: String,
    id: String,
    filter_hash: String,
    note_generation: i64,
    source_generation: i64,
    workspace: String,
}

fn normalize_filter(mut filter: KnowledgeObjectFilter) -> Result<KnowledgeObjectFilter> {
    if filter.kinds.is_empty() {
        filter.kinds = vec![ObjectKind::Note, ObjectKind::Source, ObjectKind::Annotation];
    }
    let mut kinds = Vec::new();
    for kind in filter.kinds {
        match kind {
            ObjectKind::Note | ObjectKind::Source | ObjectKind::Annotation => {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            ObjectKind::Collection => {
                return Err(Error::InvalidInput(
                    "collections are categories, not knowledge rows",
                ));
            }
        }
    }
    filter.kinds = kinds;

    if filter.tag_ids.len() > 100 {
        return Err(Error::InvalidInput("knowledge tag filter exceeds 100 tags"));
    }
    for tag in &filter.tag_ids {
        valid_uuid(tag)?;
    }
    filter.tag_ids.sort();
    filter.tag_ids.dedup();

    if filter.object_ids.len() > 100 {
        return Err(Error::InvalidInput(
            "knowledge object filter exceeds 100 IDs",
        ));
    }
    for object_id in &filter.object_ids {
        valid_uuid(object_id)?;
    }
    filter.object_ids.sort();
    filter.object_ids.dedup();

    if let Some(collection) = &filter.collection_id {
        valid_uuid(collection)?;
    }
    if filter.uncategorized && filter.collection_id.is_some() {
        return Err(Error::InvalidInput(
            "uncategorized cannot be combined with a collection filter",
        ));
    }

    if filter.object_types.len() > 64 {
        return Err(Error::InvalidInput(
            "knowledge object type filter exceeds 64 values",
        ));
    }
    for value in &filter.object_types {
        if value != "__untyped" && !valid_object_type_token(value) {
            return Err(Error::InvalidInput("invalid knowledge object type token"));
        }
    }
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

fn read_collections(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Vec<ObjectCollectionRef>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT c.id,c.name
         FROM memberships m
         JOIN collections c ON c.id=m.collection_id
         JOIN objects o ON o.id=c.id
         WHERE (
             m.member_id=?1
             OR m.member_id=(
               SELECT v.source_id
               FROM annotations a JOIN source_versions v ON v.id=a.source_version_id
               WHERE a.id=?1
             )
           )
           AND o.trashed=0
         ORDER BY c.name,c.id",
    )?;
    Ok(statement
        .query_map([id], |row| {
            Ok(ObjectCollectionRef {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn hit_snippet(text: &str, query: &str) -> Option<KnowledgeObjectHit> {
    if query.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let byte = lower.find(query)?;
    let start_codepoint = lower[..byte].chars().count();
    let total = text.chars().count();
    let window_start = start_codepoint.saturating_sub(48);
    let mut snippet: String = text.chars().skip(window_start).take(180).collect();
    if window_start > 0 {
        snippet.insert(0, '…');
    }
    if total > window_start + 180 {
        snippet.push('…');
    }
    Some(KnowledgeObjectHit {
        snippet,
        start_codepoint: start_codepoint as u32,
        page_index: None,
        block_id: None,
    })
}

fn object_hit(
    connection: &rusqlite::Connection,
    kind: ObjectKind,
    id: &str,
    query: &str,
    title: &str,
    annotation: Option<&crate::AnnotationSummary>,
) -> Result<Option<KnowledgeObjectHit>> {
    if query.is_empty() {
        return Ok(None);
    }
    if kind == ObjectKind::Note {
        let row = connection
            .query_row(
                "SELECT b.id,b.text FROM blocks b
                 WHERE b.note_id=?1 AND b.active=1 AND instr(lower(b.text),?2)>0
                 ORDER BY b.position LIMIT 1",
                params![id, query],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((block_id, text)) = row
            && let Some(mut hit) = hit_snippet(&text, query)
        {
            hit.block_id = Some(block_id);
            return Ok(Some(hit));
        }
    } else if kind == ObjectKind::Source {
        let row = connection
            .query_row(
                "SELECT b.id,b.page_index,b.text
                 FROM sources s
                 JOIN source_active_parses ap ON ap.source_version_id=s.current_version_id
                 JOIN source_blocks b ON b.parse_id=ap.parse_id
                 WHERE s.id=?1 AND instr(lower(b.text),?2)>0
                 ORDER BY b.position LIMIT 1",
                params![id, query],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<u32>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((block_id, page_index, text)) = row
            && let Some(mut hit) = hit_snippet(&text, query)
        {
            hit.block_id = Some(block_id);
            hit.page_index = page_index;
            return Ok(Some(hit));
        }
    } else if let Some(annotation) = annotation {
        // The summary intentionally carries only a preview for list rendering.
        // Search against the full immutable excerpt and comment so a query that
        // occurs after the preview cutoff still returns an exact hit snippet.
        let row = connection
            .query_row(
                "SELECT json_extract(locator_json,'$.quote'),coalesce(comment,'')
                 FROM annotations WHERE id=?1",
                [&annotation.id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        row.get::<_, String>(1)?,
                    ))
                },
            )
            .optional()?;
        if let Some((quote_text, comment_text)) = row {
            let quote = hit_snippet(&quote_text, query);
            let comment = hit_snippet(&comment_text, query);
            if quote.is_some() || comment.is_some() {
                let mut hit = quote.or(comment).ok_or(Error::IntegrityFailure)?;
                hit.page_index = annotation.page_index;
                return Ok(Some(hit));
            }
        }
    }
    Ok(hit_snippet(title, query))
}

fn note_exists(connection: &rusqlite::Connection, id: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1 AND trashed=0)",
        [id],
        |row| row.get(0),
    )?)
}

fn related_count(connection: &rusqlite::Connection, id: &str) -> Result<u32> {
    let mut related = std::collections::BTreeSet::new();
    let mut statement = connection.prepare(
        "SELECT target_id FROM body_links WHERE source_id=?1 AND target_id!=?1
         UNION
         SELECT source_id FROM body_links WHERE target_id=?1 AND source_id!=?1
         UNION
         SELECT target_id FROM note_links WHERE source_id=?1 AND target_id!=?1
         UNION
         SELECT source_id FROM note_links WHERE target_id=?1 AND source_id!=?1",
    )?;
    let linked = statement.query_map([id], |row| row.get::<_, String>(0))?;
    for value in linked {
        related.insert(value?);
    }

    let outgoing = connection
        .query_row(
            "SELECT json_extract(r.document_json,'$.content[0].attrs.source')
             FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
             WHERE n.id=?1 AND n.trashed=0
               AND json_extract(r.document_json,'$.content[0].type')='frontmatter'",
            [id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if let Some(source) = outgoing {
        for (target_id, _, _) in relation_targets(&source) {
            if target_id != id && note_exists(connection, &target_id)? {
                related.insert(target_id);
            }
        }
    }

    // The UUID pre-filter keeps this bounded by notes that can actually name
    // this target; YAML remains the authoritative shape check.
    let mut statement = connection.prepare(
        "SELECT n.id,json_extract(r.document_json,'$.content[0].attrs.source')
         FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
         WHERE n.trashed=0 AND n.id!=?1
           AND json_extract(r.document_json,'$.content[0].type')='frontmatter'
           AND instr(json_extract(r.document_json,'$.content[0].attrs.source'),?1)>0",
    )?;
    let incoming = statement.query_map([id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        ))
    })?;
    for value in incoming {
        let (source_id, source) = value?;
        if relation_targets(&source)
            .into_iter()
            .any(|(target_id, _, _)| target_id == id)
        {
            related.insert(source_id);
        }
    }

    Ok(related.len().min(u32::MAX as usize) as u32)
}

const KNOWLEDGE_ROWS: &str = r#"
  SELECT o.id,
         o.kind,
         CASE o.kind
           WHEN 'note' THEN n.updated_at
           WHEN 'source' THEN s.updated_at
           ELSE asource.updated_at
         END AS updated_at,
         n.type_key AS type_key,
         COALESCE(sv.format, av.format) AS source_format,
         n.title AS note_title,
         s.display_title AS source_title,
         asource.display_title AS annotation_source_title,
         json_extract(a.locator_json, '$.quote') AS annotation_quote
  FROM objects o
  LEFT JOIN notes n ON n.id = o.id
  LEFT JOIN sources s ON s.id = o.id
  LEFT JOIN source_versions sv ON sv.id = s.current_version_id
  LEFT JOIN annotations a ON a.id = o.id
  LEFT JOIN source_versions av ON av.id = a.source_version_id
  LEFT JOIN sources asource ON asource.id = av.source_id
  LEFT JOIN objects asource_object ON asource_object.id = asource.id
  WHERE o.kind IN (SELECT value FROM json_each(?1))
    AND (o.kind != 'annotation' OR (asource_object.trashed = 0 AND asource_object.search_enabled = 1))
    AND o.trashed = ?2
    AND (?3 IS NULL OR o.archived = ?3)
    AND (?4 IS NULL OR o.inbox_state = ?4)
    AND (?5 IS NULL OR EXISTS (
      SELECT 1 FROM memberships m
      WHERE (
        m.member_id = o.id
        OR m.member_id = (
          SELECT v.source_id
          FROM annotations a_parent JOIN source_versions v ON v.id = a_parent.source_version_id
          WHERE a_parent.id = o.id
        )
      )
        AND m.collection_id = ?5
    ))
    AND (?6 = 0 OR NOT EXISTS (
      SELECT 1 FROM memberships m
      WHERE m.member_id = o.id
         OR m.member_id = (
           SELECT v.source_id
           FROM annotations a_parent JOIN source_versions v ON v.id = a_parent.source_version_id
           WHERE a_parent.id = o.id
         )
    ))
    AND NOT EXISTS (
      SELECT 1 FROM json_each(?7) wanted
      WHERE NOT EXISTS (
        SELECT 1 FROM object_tags ot WHERE ot.object_id = o.id AND ot.tag_id = wanted.value
      )
    )
    AND (?8 = 0 OR (
      o.kind = 'note' AND EXISTS (
        SELECT 1 FROM json_each(?9) chosen
        WHERE (chosen.value = '__untyped' AND n.type_key IS NULL)
           OR (chosen.value != '__untyped' AND n.type_key = chosen.value)
      )
    ))
    AND (
      ?10 = ''
      OR instr(lower(COALESCE(n.title, s.display_title, asource.display_title, '')), ?10) > 0
      OR instr(lower(COALESCE(json_extract(a.locator_json, '$.quote'), '')), ?10) > 0
      OR instr(lower(COALESCE(a.comment, '')), ?10) > 0
      OR (
        o.kind = 'note'
        AND EXISTS (
          SELECT 1 FROM blocks b
          WHERE b.note_id = o.id AND b.active = 1 AND instr(lower(b.text), ?10) > 0
        )
      )
      OR (
        o.kind = 'source'
        AND EXISTS (
          SELECT 1
          FROM source_active_parses ap
          JOIN source_blocks b ON b.parse_id = ap.parse_id
          WHERE ap.source_version_id = s.current_version_id
            AND instr(lower(b.text), ?10) > 0
        )
      )
    )
    AND (?14 = 0 OR o.id IN (SELECT value FROM json_each(?15)))
"#;

fn read_knowledge_object(
    connection: &rusqlite::Connection,
    id: String,
    kind: ObjectKind,
    updated_at: String,
    query: &str,
) -> Result<KnowledgeObject> {
    let object = read_metadata(connection, &id)?;
    let note = if kind == ObjectKind::Note {
        Some(read_summary(connection, &id)?)
    } else {
        None
    };
    let source = if kind == ObjectKind::Source {
        Some(read_source(connection, &id)?)
    } else {
        None
    };
    let annotation = if kind == ObjectKind::Annotation {
        Some(read_annotation_summary(connection, &id)?)
    } else {
        None
    };
    let object_type = if kind == ObjectKind::Note {
        connection
            .query_row("SELECT type_key FROM notes WHERE id=?1", [&id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten()
    } else {
        None
    };
    let title = note
        .as_ref()
        .map(|value| value.title.clone())
        .or_else(|| source.as_ref().map(|value| value.display_title.clone()))
        .or_else(|| {
            annotation
                .as_ref()
                .map(|value| format!("{} · {}", value.source_title, value.quote_preview))
        })
        .unwrap_or_default();
    let preview = note
        .as_ref()
        .map(|value| value.preview.clone())
        .or_else(|| source.as_ref().and_then(|value| value.preview_text.clone()))
        .or_else(|| {
            annotation.as_ref().map(|value| {
                if value.comment_preview.trim().is_empty() {
                    value.quote_preview.clone()
                } else {
                    value.comment_preview.clone()
                }
            })
        })
        .unwrap_or_default();
    let hit = object_hit(connection, kind, &id, query, &title, annotation.as_ref())?;
    let related_count = related_count(connection, &id)?;
    Ok(KnowledgeObject {
        object,
        note,
        source,
        annotation,
        collections: read_collections(connection, &id)?,
        object_type,
        title,
        preview,
        hit,
        related_count,
        updated_at,
    })
}

impl Library {
    /// Counts, type summaries and the requested page use one read snapshot.
    pub fn knowledge_browse(&self, request: KnowledgeBrowseRequest) -> Result<KnowledgeBrowsePage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 || request.source_formats.len() > 32 {
            return Err(Error::InvalidInput(
                "invalid knowledge browse page or formats",
            ));
        }
        let query = request
            .query
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let title = request
            .title_query
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        for value in [&query, &title] {
            if value.chars().count() > 512 || value.chars().any(char::is_control) {
                return Err(Error::InvalidInput("invalid knowledge search query"));
            }
        }
        let from = request
            .updated_from
            .as_deref()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| Error::InvalidInput("invalid updated-from date"))?;
        let to = request
            .updated_to
            .as_deref()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| Error::InvalidInput("invalid updated-to date"))?;
        if let (Some(from), Some(to)) = (from, to)
            && from > to
        {
            return Err(Error::InvalidInput("updated date range is reversed"));
        }
        let filter = normalize_filter(request.filter.unwrap_or_default())?;
        let kinds = serde_json::to_string(&filter.kinds)?;
        let tags = serde_json::to_string(&filter.tag_ids)?;
        let types = serde_json::to_string(&filter.object_types)?;
        let object_ids = serde_json::to_string(&filter.object_ids)?;
        let formats = serde_json::to_string(&request.source_formats)?;
        let inbox = filter.inbox_state.map(InboxState::as_str);
        let snapshot = self.connection.unchecked_transaction()?;
        let filtered = format!("FROM ({KNOWLEDGE_ROWS}) x
            WHERE (?16='' OR instr(lower(coalesce(x.note_title,x.source_title,x.annotation_source_title,'')),?16)>0)
              AND (?17 IS NULL OR julianday(x.updated_at)>=julianday(?17))
              AND (?18 IS NULL OR julianday(x.updated_at)<=julianday(?18))
              AND (?19=0 OR x.source_format IN (SELECT value FROM json_each(?20)))");
        // Slots 11-13 belong to the cursor reader; sharing the predicate keeps all
        // visibility, membership and full-text rules aligned between both APIs.
        let parameters = params![
            kinds,
            filter.trashed,
            filter.archived,
            inbox,
            filter.collection_id,
            i64::from(filter.uncategorized),
            tags,
            i64::from(!filter.object_types.is_empty()),
            types,
            query,
            Option::<String>::None,
            Option::<String>::None,
            0,
            i64::from(!filter.object_ids.is_empty()),
            object_ids,
            title,
            request.updated_from,
            request.updated_to,
            i64::from(!request.source_formats.is_empty()),
            formats
        ];
        let mut statement = snapshot.prepare(&format!(
            "SELECT x.kind,x.type_key,count(*),max(x.updated_at) {filtered}
             GROUP BY x.kind,x.type_key ORDER BY x.kind,x.type_key"
        ))?;
        let type_counts = statement
            .query_map(parameters, |row| {
                let kind: String = row.get(0)?;
                let kind = match kind.as_str() {
                    "note" => ObjectKind::Note,
                    "source" => ObjectKind::Source,
                    "annotation" => ObjectKind::Annotation,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(KnowledgeTypeCount {
                    kind,
                    object_type: row.get(1)?,
                    count: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let total = type_counts.iter().try_fold(0u32, |total, entry| {
            total
                .checked_add(entry.count)
                .ok_or(Error::IntegrityFailure)
        })?;
        let page = request.page.min(total.div_ceil(request.limit).max(1));
        let offset = u64::from(page - 1) * u64::from(request.limit);
        let mut statement = snapshot.prepare(&format!(
            "SELECT x.id,x.kind,x.updated_at {filtered} ORDER BY x.updated_at DESC,x.id LIMIT {} OFFSET {offset}", request.limit))?;
        let rows = statement
            .query_map(parameters, |row| {
                let kind: String = row.get(1)?;
                let kind = match kind.as_str() {
                    "note" => ObjectKind::Note,
                    "source" => ObjectKind::Source,
                    "annotation" => ObjectKind::Annotation,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok((row.get::<_, String>(0)?, kind, row.get::<_, String>(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let items = rows
            .into_iter()
            .map(|(id, kind, updated)| read_knowledge_object(&snapshot, id, kind, updated, &query))
            .collect::<Result<Vec<_>>>()?;
        let more = page < total.div_ceil(request.limit);
        Ok(KnowledgeBrowsePage {
            items,
            total,
            type_counts,
            page,
            page_size: request.limit,
            next_cursor: more.then(|| (page + 1).to_string()),
            truncated: false,
        })
    }

    pub fn knowledge_objects_page(
        &self,
        request: KnowledgeObjectPageRequest,
    ) -> Result<KnowledgeObjectPage> {
        crate::collections::validate_limit(request.limit)?;
        let query = request
            .query
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if query.chars().count() > 512 || query.chars().any(char::is_control) {
            return Err(Error::InvalidInput("invalid knowledge search query"));
        }
        let filter = normalize_filter(request.filter.unwrap_or_default());
        let filter = filter?;
        let filter_hash = request_hash(
            "00000000-0000-0000-0000-000000000000",
            "knowledge.objects.list",
            &(&filter, &query),
        )?;
        let cursor: Option<KnowledgeCursor> =
            crate::collections::decode_cursor(request.cursor.as_deref())?;
        let snapshot = self.connection.unchecked_transaction()?;
        let note_generation: i64 =
            snapshot.query_row("SELECT generation FROM note_list_state", [], |row| {
                row.get(0)
            })?;
        let source_generation: i64 =
            snapshot.query_row("SELECT generation FROM source_list_state", [], |row| {
                row.get(0)
            })?;
        let workspace: String = snapshot.query_row(
            "SELECT workspace_id FROM workspace_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if let Some(cursor) = &cursor {
            if cursor.filter_hash != filter_hash {
                return Err(Error::InvalidInput(
                    "knowledge cursor belongs to a different query",
                ));
            }
            if cursor.note_generation != note_generation
                || cursor.source_generation != source_generation
                || cursor.workspace != workspace
            {
                return Err(Error::QueryExpired);
            }
        }

        let kinds = serde_json::to_string(&filter.kinds)?;
        let tags = serde_json::to_string(&filter.tag_ids)?;
        let types = serde_json::to_string(&filter.object_types)?;
        let object_ids = serde_json::to_string(&filter.object_ids)?;
        let inbox = filter.inbox_state.map(InboxState::as_str);
        let kinds = kinds.as_str();
        let tags = tags.as_str();
        let types = types.as_str();
        let mut statement = snapshot.prepare(&format!(
            "SELECT x.id,x.kind,x.updated_at FROM ({KNOWLEDGE_ROWS}) x
             WHERE (?11 IS NULL OR x.updated_at < ?11 OR (x.updated_at = ?11 AND x.id > ?12))
             ORDER BY x.updated_at DESC,x.id LIMIT ?13"
        ))?;
        let mut rows = statement.query(params![
            kinds,
            filter.trashed,
            filter.archived,
            inbox,
            filter.collection_id,
            i64::from(filter.uncategorized),
            tags,
            i64::from(!filter.object_types.is_empty()),
            types,
            query,
            cursor.as_ref().map(|value| &value.updated_at),
            cursor.as_ref().map(|value| &value.id),
            request.limit + 1,
            i64::from(!filter.object_ids.is_empty()),
            object_ids,
        ])?;
        let mut rows_data = Vec::new();
        while let Some(row) = rows.next()? {
            let kind: String = row.get(1)?;
            let kind = match kind.as_str() {
                "note" => ObjectKind::Note,
                "annotation" => ObjectKind::Annotation,
                "source" => ObjectKind::Source,
                _ => return Err(Error::IntegrityFailure),
            };
            rows_data.push((row.get::<_, String>(0)?, kind, row.get::<_, String>(2)?));
            if rows_data.len() > request.limit as usize {
                break;
            }
        }
        let has_more = rows_data.len() > request.limit as usize;
        rows_data.truncate(request.limit as usize);
        let mut items = Vec::with_capacity(rows_data.len());
        for (id, kind, updated_at) in rows_data {
            items.push(read_knowledge_object(
                &snapshot, id, kind, updated_at, &query,
            )?);
        }
        let next_cursor = if has_more {
            let last = items.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&KnowledgeCursor {
                updated_at: last.updated_at.clone(),
                id: last.object.id.clone(),
                filter_hash,
                note_generation,
                source_generation,
                workspace,
            })?)
        } else {
            None
        };
        Ok(KnowledgeObjectPage {
            items,
            next_cursor,
            truncated: has_more,
        })
    }
}
