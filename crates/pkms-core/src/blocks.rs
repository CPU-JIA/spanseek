use crate::{
    BlockHistoryItem, BlockHistoryPage, BlockHistoryRequest, BlockPage, BlocksPageRequest, Error,
    Library, NoteBlockSummary, NoteDocument, Result, collections::validate_limit,
};
use rusqlite::{Connection, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug)]
struct BlockRow {
    id: String,
    parent_block_id: Option<String>,
    position: u32,
    depth: u32,
    kind: String,
    text: String,
    alias: Option<String>,
}

fn collect_blocks(
    node: &crate::DocumentNode,
    parent: Option<&str>,
    depth: u32,
    out: &mut Vec<BlockRow>,
) {
    let block_id = (!node.is_inline()).then_some(node.attrs.id.as_str());
    let mut child_depth = depth;
    if let Some(id) = block_id {
        let position = out.len().try_into().unwrap_or(u32::MAX);
        out.push(BlockRow {
            id: id.to_owned(),
            parent_block_id: parent.map(str::to_owned),
            position,
            depth,
            kind: node.kind.storage_name().to_owned(),
            text: node.plain_text(),
            alias: node.attrs.block_id.clone(),
        });
        child_depth += 1;
    }
    for child in &node.content {
        collect_blocks(child, block_id.or(parent), child_depth, out);
    }
}

/// Rebuilds the current block projection inside the caller's write transaction.
/// Rows that disappear stay inactive so their identity cannot be reused by a
/// different note; note revision history remains the immutable document truth.
pub(crate) fn replace_block_metadata(
    connection: &Connection,
    note_id: &str,
    revision_id: &str,
    document: &NoteDocument,
    updated_at: &str,
) -> Result<()> {
    let projection_ready = connection
        .prepare("PRAGMA table_info(blocks)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?
        .contains("revision_id");
    if !projection_ready {
        // Pre-v33 fixtures can be seeded through the public note API before
        // their upgrade runs. Preserve the legacy active-block projection so
        // those fixtures stay usable without pretending the rich columns exist.
        connection.execute("UPDATE blocks SET active=0 WHERE note_id=?1", [note_id])?;
        for node in document.nodes() {
            if !node.attrs.id.is_empty() {
                connection.execute(
                    "INSERT INTO blocks(id,note_id,active) VALUES(?1,?2,1) ON CONFLICT(id) DO UPDATE SET active=1",
                    rusqlite::params![node.attrs.id, note_id],
                )?;
            }
        }
        return Ok(());
    }
    connection.execute("UPDATE blocks SET active=0 WHERE note_id=?1", [note_id])?;
    let mut rows = Vec::new();
    for node in &document.content {
        collect_blocks(node, None, 0, &mut rows);
    }
    for row in rows {
        connection.execute(
            "INSERT INTO blocks(id,note_id,active,revision_id,parent_block_id,position,depth,kind,text,block_alias,updated_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET note_id=excluded.note_id,active=1,revision_id=excluded.revision_id,parent_block_id=excluded.parent_block_id,position=excluded.position,depth=excluded.depth,kind=excluded.kind,text=excluded.text,block_alias=excluded.block_alias,updated_at=excluded.updated_at",
            rusqlite::params![
                row.id,
                note_id,
                revision_id,
                row.parent_block_id,
                row.position,
                row.depth,
                row.kind,
                row.text,
                row.alias,
                updated_at
            ],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockState {
    present: bool,
    kind: Option<String>,
    text: Option<String>,
    alias: Option<String>,
    parent_block_id: Option<String>,
    position: Option<u32>,
    depth: Option<u32>,
}

impl BlockState {
    fn missing() -> Self {
        Self {
            present: false,
            kind: None,
            text: None,
            alias: None,
            parent_block_id: None,
            position: None,
            depth: None,
        }
    }

    fn from_document(document: &NoteDocument, block_id: &str) -> Self {
        let mut rows = Vec::new();
        for node in &document.content {
            collect_blocks(node, None, 0, &mut rows);
        }
        let Some(row) = rows.into_iter().find(|row| row.id == block_id) else {
            return Self::missing();
        };
        Self {
            present: true,
            kind: Some(row.kind),
            text: Some(history_text(row.text)),
            alias: row.alias,
            parent_block_id: row.parent_block_id,
            position: Some(row.position),
            depth: Some(row.depth),
        }
    }
}

struct HistoryRow {
    revision_id: String,
    revision: u32,
    note_title: String,
    created_at: String,
    state: BlockState,
}

impl HistoryRow {
    fn item(self) -> BlockHistoryItem {
        BlockHistoryItem {
            revision_id: self.revision_id,
            revision: self.revision,
            note_title: self.note_title,
            created_at: self.created_at,
            present: self.state.present,
            kind: self.state.kind,
            text: self.state.text,
            alias: self.state.alias,
            parent_block_id: self.state.parent_block_id,
            position: self.state.position,
            depth: self.state.depth,
        }
    }
}
fn history_text(value: String) -> String {
    const LIMIT: usize = 16_000;
    let mut characters = value.chars();
    let text = characters.by_ref().take(LIMIT).collect::<String>();
    if characters.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockCursor {
    filter_hash: String,
    title: String,
    position: u32,
    id: String,
}

impl Library {
    /// Searches the current block projection with a stable keyset cursor. Every
    /// whitespace-separated term must occur in the block text, alias or note title.
    pub fn blocks_page(&self, request: BlocksPageRequest) -> Result<BlockPage> {
        validate_limit(request.limit)?;
        let query = request.query.unwrap_or_default().trim().to_owned();
        if query.chars().count() > 512
            || query.contains('\0')
            || query.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput("block query exceeds bounds"));
        }
        if let Some(note_id) = request.note_id.as_deref() {
            crate::document::valid_uuid(note_id)?;
        }
        let filter_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(&request.note_id, &query))?)
        );
        let cursor: Option<BlockCursor> = request
            .cursor
            .as_deref()
            .map(|raw| {
                if raw.len() > 4096 {
                    return Err(Error::InvalidInput("cursor exceeds bounds"));
                }
                let parsed: BlockCursor =
                    serde_json::from_str(raw).map_err(|_| Error::InvalidInput("invalid cursor"))?;
                if parsed.filter_hash != filter_hash
                    || crate::document::valid_uuid(&parsed.id).is_err()
                {
                    return Err(Error::InvalidInput(
                        "cursor does not match this block query",
                    ));
                }
                Ok(parsed)
            })
            .transpose()?;
        let terms = query
            .split_whitespace()
            .take(12)
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut sql = String::from(
            "SELECT b.id,b.note_id,n.title,b.revision_id,b.parent_block_id,b.position,b.depth,b.kind,b.text,b.block_alias,(SELECT count(*) FROM body_links l WHERE l.target_id=b.note_id AND l.target_block_id=b.id) FROM blocks b JOIN notes n ON n.id=b.note_id WHERE b.active=1 AND n.trashed=0 AND (?1 IS NULL OR b.note_id=?1) AND (?2 IS NULL OR lower(n.title)>?2 OR (lower(n.title)=?2 AND (b.position>?3 OR (b.position=?3 AND b.id>?4))))",
        );
        let mut args = vec![
            request.note_id.map(Value::from).unwrap_or(Value::Null),
            cursor
                .as_ref()
                .map(|c| Value::from(c.title.clone()))
                .unwrap_or(Value::Null),
            cursor
                .as_ref()
                .map(|c| Value::from(i64::from(c.position)))
                .unwrap_or(Value::Null),
            cursor
                .as_ref()
                .map(|c| Value::from(c.id.clone()))
                .unwrap_or(Value::Null),
        ];
        for term in terms {
            let placeholder = args.len() + 1;
            sql.push_str(&format!(
                " AND instr(lower(b.text || ' ' || n.title || ' ' || coalesce(b.block_alias,'')), ?{placeholder})>0"
            ));
            args.push(Value::from(term));
        }
        let limit_placeholder = args.len() + 1;
        sql.push_str(&format!(
            " ORDER BY lower(n.title),b.position,b.id LIMIT ?{limit_placeholder}"
        ));
        args.push(Value::from(i64::from(request.limit) + 1));
        let mut statement = self.connection.prepare(&sql)?;
        let mut items = statement
            .query_map(params_from_iter(args), |row| {
                let position: i64 = row.get(5)?;
                let depth: i64 = row.get(6)?;
                Ok(NoteBlockSummary {
                    id: row.get(0)?,
                    note_id: row.get(1)?,
                    note_title: row.get(2)?,
                    revision_id: row.get(3)?,
                    parent_block_id: row.get(4)?,
                    position: u32::try_from(position)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, position))?,
                    depth: u32::try_from(depth)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(6, depth))?,
                    kind: row.get(7)?,
                    text: row.get(8)?,
                    alias: row.get(9)?,
                    reference_count: row.get(10)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > request.limit as usize {
            items.truncate(request.limit as usize);
            items
                .last()
                .map(|last| {
                    serde_json::to_string(&BlockCursor {
                        filter_hash,
                        title: last.note_title.to_lowercase(),
                        position: last.position,
                        id: last.id.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(BlockPage { items, next_cursor })
    }
    /// Returns revisions where this block changed relative to its parent. A
    /// title-only revision therefore does not appear as a block revision.
    pub fn block_history(&self, request: BlockHistoryRequest) -> Result<BlockHistoryPage> {
        validate_limit(request.limit)?;
        crate::document::valid_uuid(&request.note_id)?;
        crate::document::valid_uuid(&request.block_id)?;
        if request.before == Some(0) {
            return Err(Error::InvalidInput("block history cursor must be positive"));
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM blocks WHERE note_id=?1 AND id=?2)",
            rusqlite::params![request.note_id, request.block_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        if let Some(before) = request.before {
            let exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM note_revisions WHERE note_id=?1 AND revision=?2)",
                rusqlite::params![request.note_id, before],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::NotFound);
            }
        }
        let mut statement = self.connection.prepare(
            "SELECT revision_id,revision,title,schema_version,document_json,created_at FROM note_revisions WHERE note_id=?1 AND (?2 IS NULL OR revision<?2) ORDER BY revision DESC",
        )?;
        let rows = statement
            .query_map(rusqlite::params![request.note_id, request.before], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut items = Vec::new();
        let mut newer: Option<HistoryRow> = None;
        for (revision_id, revision, title, schema, json, created_at) in rows {
            let document: NoteDocument = serde_json::from_str(&json)?;
            document.validate(schema)?;
            let row = HistoryRow {
                revision_id,
                revision,
                note_title: title,
                created_at,
                state: BlockState::from_document(&document, &request.block_id),
            };
            let changed = newer
                .as_ref()
                .is_some_and(|newer_row| newer_row.state != row.state);
            if let Some(newer_row) = newer.replace(row)
                && changed
            {
                items.push(newer_row.item());
            }
            if items.len() > request.limit as usize {
                break;
            }
        }
        if items.len() <= request.limit as usize
            && let Some(oldest) = newer
            && oldest.state.present
        {
            items.push(oldest.item());
        }
        let next_before = if items.len() > request.limit as usize {
            items.truncate(request.limit as usize);
            items.last().map(|item| item.revision)
        } else {
            None
        };
        Ok(BlockHistoryPage { items, next_before })
    }
}
