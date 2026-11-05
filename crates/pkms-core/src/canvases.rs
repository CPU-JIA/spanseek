use crate::{
    Canvas, CanvasCommit, CanvasNode, CanvasNote, CanvasPage, CanvasPageRequest, CanvasView, Error,
    Library, RemoveCanvas, Result, SaveCanvas,
    collections::{decode_cursor, search_text, validate_limit},
    document::valid_uuid,
    mutations::{
        check_version, next_receipt, replay, request_hash, store_receipt, validate_name,
        validate_pin,
    },
    organization::read_metadata,
    queries::read_summary,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

const MAX_CANVAS_NODES: usize = 1000;
const MAX_COORDINATE: f64 = 1_000_000.0;
const MIN_NODE_WIDTH: f64 = 120.0;
const MAX_NODE_WIDTH: f64 = 2_000.0;
const MIN_NODE_HEIGHT: f64 = 80.0;
const MAX_NODE_HEIGHT: f64 = 2_000.0;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanvasCursor {
    pin: i64,
    name: String,
    id: String,
    generation: i64,
    query_hash: String,
}

impl Library {
    pub fn canvas_get(&self, canvas_id: &str) -> Result<CanvasView> {
        valid_uuid(canvas_id)?;
        let tx = self.connection.unchecked_transaction()?;
        let canvas = read_canvas(&tx, canvas_id)?;
        let mut notes = Vec::with_capacity(canvas.nodes.len());
        let mut missing_note_ids = Vec::new();
        for node in &canvas.nodes {
            match read_summary(&tx, &node.note_id) {
                Ok(note) => {
                    let metadata = read_metadata(&tx, &note.id)?;
                    notes.push(CanvasNote { note, metadata });
                }
                Err(Error::NotFound) => missing_note_ids.push(node.note_id.clone()),
                Err(error) => return Err(error),
            }
        }
        Ok(CanvasView {
            canvas,
            notes,
            missing_note_ids,
        })
    }

    pub fn canvas(&self, canvas_id: &str) -> Result<Canvas> {
        valid_uuid(canvas_id)?;
        read_canvas(&self.connection, canvas_id)
    }

    pub fn list_canvases(&self) -> Result<Vec<Canvas>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM canvases ORDER BY id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| read_canvas(&self.connection, id))
            .collect()
    }

    pub fn canvases_page(&self, request: CanvasPageRequest) -> Result<CanvasPage> {
        validate_limit(request.limit)?;
        let query = search_text(request.query.as_deref())?;
        let query_hash = request_hash(
            "00000000-0000-0000-0000-000000000000",
            "canvases.list",
            &(request.pinned_only, &query),
        )?;
        let cursor: Option<CanvasCursor> = decode_cursor(request.cursor.as_deref())?;
        if cursor.as_ref().is_some_and(|c| c.query_hash != query_hash) {
            return Err(Error::InvalidInput(
                "canvas cursor belongs to a different query",
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let generation: i64 =
            tx.query_row("SELECT generation FROM canvas_list_state", [], |r| r.get(0))?;
        if cursor.as_ref().is_some_and(|c| c.generation != generation) {
            return Err(Error::QueryExpired);
        }
        let mut statement = tx.prepare(
            "SELECT id FROM canvases WHERE (?1=0 OR is_pinned=1) AND instr(lower(name),lower(?2))>0
            AND (?3 IS NULL OR (COALESCE(pin_position,4294967296),name,id)>(?3,?4,?5))
            ORDER BY COALESCE(pin_position,4294967296),name,id LIMIT ?6",
        )?;
        let ids = statement
            .query_map(
                params![
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
            .map(|id| read_canvas(&tx, id))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            let last = items.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&CanvasCursor {
                pin: last.pin_position.map(i64::from).unwrap_or(4294967296),
                name: last.name.clone(),
                id: last.id.clone(),
                generation,
                query_hash,
            })?)
        } else {
            None
        };
        Ok(CanvasPage { items, next_cursor })
    }

    pub fn canvas_save(&mut self, command_id: &str, request: SaveCanvas) -> Result<CanvasCommit> {
        let kind = "canvases.save";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        if request.config_schema != 1 {
            return Err(Error::InvalidInput("unsupported canvas config schema"));
        }
        let name = validate_name(&request.name)?;
        validate_pin(request.is_pinned, request.pin_position)?;
        let nodes = normalize_nodes(&tx, request.nodes, true)?;
        let nodes_json = serde_json::to_string(&nodes)?;
        let now = Utc::now().to_rfc3339();
        let id = match (&request.canvas_id, request.base_row_version) {
            (None, None) => {
                let id = Uuid::new_v4().to_string();
                tx.execute("INSERT INTO canvases(id,name,config_schema,nodes_json,is_pinned,pin_position,row_version,created_at,updated_at)
                    VALUES(?1,?2,1,?3,?4,?5,1,?6,?6)",params![id,name,nodes_json,request.is_pinned,request.pin_position,now])?;
                id
            }
            (Some(id), Some(base)) => {
                valid_uuid(id)?;
                let current = read_canvas(&tx, id)?;
                check_version(id, base, current.row_version)?;
                if current.name != name
                    || current.nodes != nodes
                    || current.is_pinned != request.is_pinned
                    || current.pin_position != request.pin_position
                {
                    let next = current
                        .row_version
                        .checked_add(1)
                        .ok_or(Error::InvalidInput("canvas version overflow"))?;
                    tx.execute("UPDATE canvases SET name=?2,nodes_json=?3,is_pinned=?4,pin_position=?5,row_version=?6,updated_at=?7 WHERE id=?1",
                        params![id,name,nodes_json,request.is_pinned,request.pin_position,next,now])?;
                }
                id.clone()
            }
            _ => {
                return Err(Error::InvalidInput(
                    "creating a canvas omits identity/version; updating requires both",
                ));
            }
        };
        let result = CanvasCommit {
            canvas: read_canvas(&tx, &id)?,
            removed: false,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn canvas_remove(
        &mut self,
        command_id: &str,
        request: RemoveCanvas,
    ) -> Result<CanvasCommit> {
        let kind = "canvases.remove";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        valid_uuid(&request.canvas_id)?;
        let canvas = read_canvas(&tx, &request.canvas_id)?;
        check_version(&canvas.id, request.base_row_version, canvas.row_version)?;
        tx.execute("DELETE FROM canvases WHERE id=?1", [&canvas.id])?;
        tx.execute(
            "INSERT INTO sync_config_tombstones(object_id,kind,deleted_at) VALUES(?1,'canvas',?2) ON CONFLICT(object_id) DO UPDATE SET kind=excluded.kind,deleted_at=excluded.deleted_at",
            params![canvas.id, Utc::now().to_rfc3339()],
        )?;
        let result = CanvasCommit {
            canvas,
            removed: true,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(
            &tx,
            &hash,
            kind,
            &result.canvas.id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn canvas_apply_remote(&mut self, canvas: Canvas) -> Result<()> {
        valid_uuid(&canvas.id)?;
        if canvas.config_schema != 1 || canvas.row_version == 0 {
            return Err(Error::InvalidInput("unsupported remote canvas"));
        }
        let name = validate_name(&canvas.name)?;
        validate_pin(canvas.is_pinned, canvas.pin_position)?;
        let nodes = normalize_nodes(&self.connection, canvas.nodes, false)?;
        let nodes_json = serde_json::to_string(&nodes)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM sync_config_tombstones WHERE object_id=?1 AND kind='canvas'",
            [&canvas.id],
        )?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM canvases WHERE id=?1)",
            [&canvas.id],
            |r| r.get(0),
        )?;
        if exists {
            tx.execute(
                "UPDATE canvases SET name=?2,config_schema=1,nodes_json=?3,is_pinned=?4,pin_position=?5,row_version=?6,updated_at=?7 WHERE id=?1",
                params![canvas.id,name,nodes_json,canvas.is_pinned,canvas.pin_position,canvas.row_version,canvas.updated_at],
            )?;
        } else {
            tx.execute(
                "INSERT INTO canvases(id,name,config_schema,nodes_json,is_pinned,pin_position,row_version,created_at,updated_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8)",
                params![canvas.id,name,nodes_json,canvas.is_pinned,canvas.pin_position,canvas.row_version,canvas.created_at,canvas.updated_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn canvas_remove_remote(&mut self, canvas_id: &str) -> Result<()> {
        valid_uuid(canvas_id)?;
        self.connection
            .execute("DELETE FROM canvases WHERE id=?1", [canvas_id])?;
        Ok(())
    }
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let mut statement =
        db.prepare("SELECT id,name,is_pinned,pin_position,row_version FROM canvases")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, Option<u32>>(3)?,
            row.get::<_, u32>(4)?,
        ))
    })?;
    let mut seen = HashSet::new();
    for row in rows {
        let (id, name, is_pinned, pin_position, row_version) = row?;
        valid_uuid(&id)?;
        if !seen.insert(id.clone()) {
            return Err(Error::IntegrityFailure);
        }
        if validate_name(&name).map_err(|_| Error::IntegrityFailure)? != name {
            return Err(Error::IntegrityFailure);
        }
        validate_pin(is_pinned, pin_position).map_err(|_| Error::IntegrityFailure)?;
        if row_version == 0 {
            return Err(Error::IntegrityFailure);
        }
        read_canvas(db, &id)?;
    }
    Ok(())
}

fn normalize_nodes(
    connection: &Connection,
    mut nodes: Vec<CanvasNode>,
    require_notes: bool,
) -> Result<Vec<CanvasNode>> {
    if nodes.len() > MAX_CANVAS_NODES {
        return Err(Error::InvalidInput("canvas exceeds 1000 notes"));
    }
    let mut seen = HashSet::new();
    for node in &nodes {
        valid_uuid(&node.note_id)?;
        if !seen.insert(node.note_id.clone()) {
            return Err(Error::InvalidInput("canvas contains the same note twice"));
        }
        if !node.x.is_finite()
            || !node.y.is_finite()
            || !node.width.is_finite()
            || !node.height.is_finite()
            || !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&node.x)
            || !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&node.y)
            || !(MIN_NODE_WIDTH..=MAX_NODE_WIDTH).contains(&node.width)
            || !(MIN_NODE_HEIGHT..=MAX_NODE_HEIGHT).contains(&node.height)
        {
            return Err(Error::InvalidInput("canvas node geometry is out of bounds"));
        }
        if require_notes {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1)",
                [&node.note_id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(Error::InvalidInput("canvas references a missing note"));
            }
        }
    }
    nodes.sort_by(|a, b| {
        a.z_index
            .cmp(&b.z_index)
            .then_with(|| a.note_id.cmp(&b.note_id))
    });
    Ok(nodes)
}

fn read_canvas(connection: &Connection, id: &str) -> Result<Canvas> {
    let (name, schema, nodes_json, is_pinned, pin_position, row_version, created_at, updated_at) =
        connection.query_row(
            "SELECT name,config_schema,nodes_json,is_pinned,pin_position,row_version,created_at,updated_at FROM canvases WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        ).optional()?.ok_or(Error::NotFound)?;
    if schema != 1 {
        return Err(Error::IntegrityFailure);
    }
    let nodes: Vec<CanvasNode> =
        serde_json::from_str(&nodes_json).map_err(|_| Error::IntegrityFailure)?;
    let nodes = normalize_nodes(connection, nodes, false).map_err(|_| Error::IntegrityFailure)?;
    Ok(Canvas {
        id: id.into(),
        name,
        config_schema: schema,
        nodes,
        is_pinned,
        pin_position,
        row_version,
        created_at,
        updated_at,
    })
}
