use crate::{
    ConfigTombstone, Error, Library, NoteSort, NotesPageRequest, RemoveView, Result, SaveView,
    SavedView, ViewCommit, ViewConfig, ViewLayout, ViewNote, ViewNotePage, ViewNoteProperty,
    ViewPage, ViewPageRequest, ViewPropertyKind, ViewRun,
    collections::{decode_cursor, search_text, validate_limit},
    document::valid_uuid,
    mutations::{
        check_version, next_receipt, replay, request_hash, store_receipt, validate_name,
        validate_pin,
    },
    organization::read_metadata,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::collections::HashSet;
use uuid::Uuid;

const MAX_CUSTOM_VIEW_PROPERTIES: usize = 12;
const MAX_VIEW_COLUMNS: usize = 16;
const MAX_VIEW_FRONTMATTER_CHARS: usize = 64_000;
const MAX_VIEW_FRONTMATTER_PROPERTIES: usize = 100;
const MAX_VIEW_PROPERTY_VALUE_CHARS: usize = 512;
const RESERVED_VIEW_PROPERTY_KEYS: &[&str] = &[
    "title",
    "tags",
    "aliases",
    "inbox_state",
    "archived",
    "search_enabled",
    "updated_at",
];

fn normalize_custom_property_key(raw: &str) -> Result<String> {
    let key = raw.trim();
    if key.is_empty() || key.chars().count() > 64 || key.chars().any(char::is_control) {
        return Err(Error::InvalidInput("custom view property key is invalid"));
    }
    if RESERVED_VIEW_PROPERTY_KEYS
        .iter()
        .any(|reserved| key.eq_ignore_ascii_case(reserved))
    {
        return Err(Error::InvalidInput(
            "custom view property key conflicts with built-in metadata",
        ));
    }
    Ok(key.to_owned())
}

fn bounded_property_value(value: String) -> (String, bool) {
    let truncated = value.chars().count() > MAX_VIEW_PROPERTY_VALUE_CHARS;
    (
        value.chars().take(MAX_VIEW_PROPERTY_VALUE_CHARS).collect(),
        truncated,
    )
}

fn text_property_kind(value: &str) -> ViewPropertyKind {
    if chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok() {
        ViewPropertyKind::Date
    } else if chrono::DateTime::parse_from_rfc3339(value).is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").is_ok()
    {
        ViewPropertyKind::Datetime
    } else {
        ViewPropertyKind::Text
    }
}

fn property_from_yaml(value: &Value) -> (ViewPropertyKind, String, bool) {
    match value {
        Value::Null => (ViewPropertyKind::Null, String::new(), false),
        Value::Bool(value) => (ViewPropertyKind::Boolean, value.to_string(), false),
        Value::Number(value) => (ViewPropertyKind::Number, value.to_string(), false),
        Value::String(value) => {
            let (value, truncated) = bounded_property_value(value.clone());
            (text_property_kind(&value), value, truncated)
        }
        Value::Sequence(items)
            if items.iter().all(|item| {
                matches!(item, Value::String(_) | Value::Bool(_) | Value::Number(_))
            }) =>
        {
            let raw = items
                .iter()
                .map(|item| match item {
                    Value::String(value) => value.clone(),
                    Value::Bool(value) => value.to_string(),
                    Value::Number(value) => value.to_string(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let (value, truncated) = bounded_property_value(raw);
            (ViewPropertyKind::List, value, truncated)
        }
        _ => (ViewPropertyKind::Nested, String::new(), false),
    }
}

fn parse_frontmatter_properties(source: &str) -> (Vec<ViewNoteProperty>, bool, bool) {
    if source.chars().count() > MAX_VIEW_FRONTMATTER_CHARS {
        return (Vec::new(), true, false);
    }
    match serde_yaml_ng::from_str::<Value>(source) {
        Ok(Value::Mapping(mapping)) => {
            let mut properties = Vec::new();
            let mut truncated = mapping.len() > MAX_VIEW_FRONTMATTER_PROPERTIES;
            for (key, value) in mapping.iter().take(MAX_VIEW_FRONTMATTER_PROPERTIES) {
                let Value::String(key) = key else {
                    continue;
                };
                if normalize_custom_property_key(key).is_err() {
                    continue;
                }
                let (kind, value, value_truncated) = property_from_yaml(value);
                truncated |= value_truncated;
                properties.push(ViewNoteProperty {
                    key: key.clone(),
                    value,
                    kind,
                    truncated: value_truncated,
                });
            }
            (properties, truncated, false)
        }
        Ok(_) => (Vec::new(), false, true),
        Err(_) => (Vec::new(), false, true),
    }
}

fn read_view_properties(
    connection: &Connection,
    note_id: &str,
) -> Result<(Vec<ViewNoteProperty>, bool, bool)> {
    let source: Option<Option<String>> = connection
        .query_row(
            "SELECT json_extract(r.document_json,'$.content[0].attrs.source')
             FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
             WHERE n.id=?1 AND json_extract(r.document_json,'$.content[0].type')='frontmatter'",
            [note_id],
            |row| row.get(0),
        )
        .optional()?;
    match source.flatten() {
        Some(source) => Ok(parse_frontmatter_properties(&source)),
        None => Ok((Vec::new(), false, false)),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewCursor {
    pin: i64,
    name: String,
    id: String,
    generation: i64,
    query_hash: String,
}

impl Library {
    pub fn view_get(&self, view_id: &str) -> Result<SavedView> {
        valid_uuid(view_id)?;
        read_view(&self.connection, view_id)
    }

    /// Configuration sync needs a bounded inventory without inventing a second
    /// list query. The page API remains the UI contract; this is the complete
    /// local snapshot used by the sync engine.
    pub fn list_views(&self) -> Result<Vec<SavedView>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM saved_views ORDER BY id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| read_view(&self.connection, id))
            .collect()
    }

    pub fn views_page(&self, request: ViewPageRequest) -> Result<ViewPage> {
        validate_limit(request.limit)?;
        let query = search_text(request.query.as_deref())?;
        let query_hash = request_hash(
            "00000000-0000-0000-0000-000000000000",
            "views.list",
            &(request.pinned_only, &query),
        )?;
        let cursor: Option<ViewCursor> = decode_cursor(request.cursor.as_deref())?;
        if cursor.as_ref().is_some_and(|c| c.query_hash != query_hash) {
            return Err(Error::InvalidInput(
                "view cursor belongs to a different query",
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let generation: i64 =
            tx.query_row("SELECT generation FROM view_list_state", [], |r| r.get(0))?;
        if cursor.as_ref().is_some_and(|c| c.generation != generation) {
            return Err(Error::QueryExpired);
        }
        let mut statement=tx.prepare("SELECT id FROM saved_views WHERE (?1=0 OR is_pinned=1) AND instr(lower(name),lower(?2))>0
            AND (?3 IS NULL OR (COALESCE(pin_position,4294967296),name,id)>(?3,?4,?5))
            ORDER BY COALESCE(pin_position,4294967296),name,id LIMIT ?6")?;
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
            .map(|id| read_view(&tx, id))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            let last = items.last().ok_or(Error::IntegrityFailure)?;
            Some(serde_json::to_string(&ViewCursor {
                pin: last.pin_position.map(i64::from).unwrap_or(4294967296),
                name: last.name.clone(),
                id: last.id.clone(),
                generation,
                query_hash,
            })?)
        } else {
            None
        };
        Ok(ViewPage { items, next_cursor })
    }

    pub fn view_save(&mut self, command_id: &str, request: SaveView) -> Result<ViewCommit> {
        let kind = "views.save";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        if request.filter_schema != 1 {
            return Err(Error::InvalidInput("unsupported saved view filter schema"));
        }
        if request.config_schema != 1 {
            return Err(Error::InvalidInput("unsupported saved view config schema"));
        }
        let name = validate_name(&request.name)?;
        validate_pin(request.is_pinned, request.pin_position)?;
        let filter = crate::queries::normalize_filter(request.filter)?;
        let config = normalize_config(request.config)?;
        for tag in &filter.tag_ids {
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1)",
                [tag],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(Error::NotFound);
            }
        }
        if let Some(collection) = &filter.collection_id {
            crate::collections::read_collection(&tx, collection)?;
        }
        let now = Utc::now().to_rfc3339();
        let config_json = serde_json::to_string(&config)?;
        let id = match (&request.view_id, request.base_row_version) {
            (None, None) => {
                let id = Uuid::new_v4().to_string();
                tx.execute("INSERT INTO saved_views(id,name,filter_schema,filter_ast,sort,layout,config_schema,config_json,is_pinned,pin_position,row_version,created_at,updated_at)
                    VALUES(?1,?2,1,?3,?4,?5,1,?6,?7,?8,1,?9,?9)",params![id,name,serde_json::to_string(&filter)?,sort_name(request.sort),layout_name(request.layout),config_json,request.is_pinned,request.pin_position,now])?;
                id
            }
            (Some(id), Some(base)) => {
                valid_uuid(id)?;
                let current = read_view(&tx, id)?;
                check_version(id, base, current.row_version)?;
                if current.name != name
                    || current.filter != filter
                    || current.sort != request.sort
                    || current.layout != request.layout
                    || current.config != config
                    || current.is_pinned != request.is_pinned
                    || current.pin_position != request.pin_position
                {
                    let next = current
                        .row_version
                        .checked_add(1)
                        .ok_or(Error::InvalidInput("view version overflow"))?;
                    tx.execute("UPDATE saved_views SET name=?2,filter_ast=?3,sort=?4,layout=?5,config_schema=1,config_json=?6,is_pinned=?7,pin_position=?8,row_version=?9,updated_at=?10 WHERE id=?1",
                        params![id,name,serde_json::to_string(&filter)?,sort_name(request.sort),layout_name(request.layout),config_json,request.is_pinned,request.pin_position,next,now])?;
                }
                id.clone()
            }
            _ => {
                return Err(Error::InvalidInput(
                    "creating a view omits identity/version; updating requires both",
                ));
            }
        };
        let result = ViewCommit {
            view: read_view(&tx, &id)?,
            removed: false,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn view_remove(&mut self, command_id: &str, request: RemoveView) -> Result<ViewCommit> {
        let kind = "views.remove";
        let hash = request_hash(command_id, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash, kind)? {
            return Ok(result);
        }
        valid_uuid(&request.view_id)?;
        let view = read_view(&tx, &request.view_id)?;
        check_version(&view.id, request.base_row_version, view.row_version)?;
        tx.execute("DELETE FROM saved_views WHERE id=?1", [&view.id])?;
        tx.execute(
            "INSERT INTO sync_config_tombstones(object_id,kind,deleted_at) VALUES(?1,'view',?2) ON CONFLICT(object_id) DO UPDATE SET kind=excluded.kind,deleted_at=excluded.deleted_at",
            params![view.id, Utc::now().to_rfc3339()],
        )?;
        let result = ViewCommit {
            view,
            removed: true,
            receipt: next_receipt(&tx, command_id)?,
        };
        store_receipt(&tx, &hash, kind, &result.view.id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn view_run(&self, view_id: &str, limit: u32, cursor: Option<String>) -> Result<ViewRun> {
        valid_uuid(view_id)?;
        let tx = self.connection.unchecked_transaction()?;
        let view = read_view(&tx, view_id)?;
        let page = crate::queries::notes_page_in(
            &tx,
            NotesPageRequest {
                query: None,
                limit,
                cursor,
                trashed: false,
                filter: Some(view.filter.clone()),
                sort: Some(view.sort),
            },
            Some((&view.id, view.row_version)),
        )?;
        let mut items = Vec::with_capacity(page.items.len());
        for note in page.items {
            let metadata = read_metadata(&tx, &note.id)?;
            let (properties, properties_truncated, properties_error) =
                read_view_properties(&tx, &note.id)?;
            items.push(ViewNote {
                note,
                metadata,
                properties,
                properties_truncated,
                properties_error,
            });
        }
        Ok(ViewRun {
            view,
            notes: ViewNotePage {
                items,
                next_cursor: page.next_cursor,
            },
        })
    }

    /// Applies an already-validated remote configuration snapshot. This is the
    /// only write path that preserves the remote row version; user commands must
    /// still go through `view_save` and its optimistic concurrency check.
    pub fn view_apply_remote(&mut self, view: SavedView) -> Result<()> {
        valid_uuid(&view.id)?;
        if view.filter_schema != 1 || view.config_schema != 1 || view.row_version == 0 {
            return Err(Error::InvalidInput("unsupported remote saved view"));
        }
        let name = validate_name(&view.name)?;
        validate_pin(view.is_pinned, view.pin_position)?;
        let filter = crate::queries::normalize_filter(view.filter)?;
        let config = normalize_config(view.config)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM saved_views WHERE id=?1)",
            [&view.id],
            |r| r.get(0),
        )?;
        let filter_json = serde_json::to_string(&filter)?;
        let config_json = serde_json::to_string(&config)?;
        tx.execute(
            "DELETE FROM sync_config_tombstones WHERE object_id=?1 AND kind='view'",
            [&view.id],
        )?;
        if exists {
            tx.execute(
                "UPDATE saved_views SET name=?2,filter_schema=1,filter_ast=?3,sort=?4,layout=?5,config_schema=1,config_json=?6,is_pinned=?7,pin_position=?8,row_version=?9,updated_at=?10 WHERE id=?1",
                params![view.id,name,filter_json,sort_name(view.sort),layout_name(view.layout),config_json,view.is_pinned,view.pin_position,view.row_version,view.updated_at],
            )?;
        } else {
            tx.execute(
                "INSERT INTO saved_views(id,name,filter_schema,filter_ast,sort,layout,config_schema,config_json,is_pinned,pin_position,row_version,created_at,updated_at) VALUES(?1,?2,1,?3,?4,?5,1,?6,?7,?8,?9,?10,?11)",
                params![view.id,name,filter_json,sort_name(view.sort),layout_name(view.layout),config_json,view.is_pinned,view.pin_position,view.row_version,view.created_at,view.updated_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn config_tombstone_kind(&self, object_id: &str) -> Result<Option<String>> {
        valid_uuid(object_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT kind FROM sync_config_tombstones WHERE object_id=?1",
                [object_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn list_config_tombstones(&self) -> Result<Vec<ConfigTombstone>> {
        let mut statement = self.connection.prepare(
            "SELECT object_id,kind FROM sync_config_tombstones ORDER BY deleted_at,object_id",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(ConfigTombstone {
                    object_id: row.get(0)?,
                    kind: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Local deletion identity for confirmation tokens, separate from the wire
    /// content fingerprint. A later delete must invalidate an earlier preview.
    pub fn config_tombstone_stamp(&self, object_id: &str) -> Result<Option<String>> {
        valid_uuid(object_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT deleted_at FROM sync_config_tombstones WHERE object_id=?1",
                [object_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn clear_config_tombstone(&mut self, object_id: &str) -> Result<()> {
        valid_uuid(object_id)?;
        self.connection.execute(
            "DELETE FROM sync_config_tombstones WHERE object_id=?1",
            [object_id],
        )?;
        Ok(())
    }
    pub fn view_remove_remote(&mut self, view_id: &str) -> Result<()> {
        valid_uuid(view_id)?;
        self.connection
            .execute("DELETE FROM saved_views WHERE id=?1", [view_id])?;
        Ok(())
    }
}

pub(crate) fn normalize_config(mut config: ViewConfig) -> Result<ViewConfig> {
    if config.visible_properties.is_empty() || config.visible_properties.len() > 5 {
        return Err(Error::InvalidInput(
            "saved view must expose between 1 and 5 properties",
        ));
    }
    let mut seen = HashSet::new();
    for property in &config.visible_properties {
        if !seen.insert(*property) {
            return Err(Error::InvalidInput("duplicate saved view property"));
        }
    }
    let mut custom = Vec::with_capacity(config.custom_properties.len());
    let mut custom_seen = HashSet::new();
    for raw in &config.custom_properties {
        let key = normalize_custom_property_key(raw)?;
        if !custom_seen.insert(key.clone()) {
            return Err(Error::InvalidInput("duplicate custom saved view property"));
        }
        custom.push(key);
    }
    if custom.len() > MAX_CUSTOM_VIEW_PROPERTIES
        || config.visible_properties.len() + custom.len() > MAX_VIEW_COLUMNS
    {
        return Err(Error::InvalidInput(
            "saved view exposes too many built-in and custom properties",
        ));
    }
    config.custom_properties = custom;
    if let Some(raw) = config.date_property.take() {
        let key = normalize_custom_property_key(&raw)?;
        if !config.custom_properties.contains(&key) {
            return Err(Error::InvalidInput(
                "calendar date property must be one of the custom view properties",
            ));
        }
        config.date_property = Some(key);
    }
    if let Some(month) = &config.calendar_month {
        let valid = month.len() == 7
            && month.as_bytes().get(4) == Some(&b'-')
            && chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_ok();
        if !valid {
            return Err(Error::InvalidInput("calendar month must use YYYY-MM"));
        }
    }
    Ok(config)
}

pub(crate) fn config_is_supported(config: &ViewConfig) -> bool {
    normalize_config(config.clone()).is_ok()
}

fn read_view(connection: &Connection, id: &str) -> Result<SavedView> {
    let (name,schema,json,sort,layout,config_schema,config_json,is_pinned,pin_position,row_version,created_at,updated_at)=connection.query_row(
        "SELECT name,filter_schema,filter_ast,sort,layout,config_schema,config_json,is_pinned,pin_position,row_version,created_at,updated_at FROM saved_views WHERE id=?1",[id],
        |r|Ok((r.get(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,u32>(5)?,r.get::<_,String>(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    if schema != 1 || config_schema != 1 {
        return Err(Error::IntegrityFailure);
    }
    let filter = crate::queries::normalize_filter(
        serde_json::from_str(&json).map_err(|_| Error::IntegrityFailure)?,
    )
    .map_err(|_| Error::IntegrityFailure)?;
    let config =
        normalize_config(serde_json::from_str(&config_json).map_err(|_| Error::IntegrityFailure)?)
            .map_err(|_| Error::IntegrityFailure)?;
    if !config_is_supported(&config) {
        return Err(Error::IntegrityFailure);
    }
    let sort = match sort.as_str() {
        "updated_desc" => NoteSort::UpdatedDesc,
        "updated_asc" => NoteSort::UpdatedAsc,
        "title_asc" => NoteSort::TitleAsc,
        "title_desc" => NoteSort::TitleDesc,
        _ => return Err(Error::IntegrityFailure),
    };
    let layout = match layout.as_str() {
        "list" => ViewLayout::List,
        "table" => ViewLayout::Table,
        "board" => ViewLayout::Board,
        "calendar" => ViewLayout::Calendar,
        "timeline" => ViewLayout::Timeline,
        _ => return Err(Error::IntegrityFailure),
    };
    Ok(SavedView {
        id: id.into(),
        name,
        filter_schema: schema,
        filter,
        sort,
        layout,
        config_schema,
        config,
        is_pinned,
        pin_position,
        row_version,
        created_at,
        updated_at,
    })
}
fn sort_name(sort: NoteSort) -> &'static str {
    match sort {
        NoteSort::UpdatedDesc => "updated_desc",
        NoteSort::UpdatedAsc => "updated_asc",
        NoteSort::TitleAsc => "title_asc",
        NoteSort::TitleDesc => "title_desc",
    }
}
fn layout_name(layout: ViewLayout) -> &'static str {
    match layout {
        ViewLayout::List => "list",
        ViewLayout::Table => "table",
        ViewLayout::Board => "board",
        ViewLayout::Calendar => "calendar",
        ViewLayout::Timeline => "timeline",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_properties_are_flat_bounded_and_typed() {
        let source = r#"
due: 2026-11-01
priority: 3
done: false
empty:
tags: [ignored, builtin]
authors:
  - Alice
  - Bob
nested:
  child: hidden
"#;
        let (properties, truncated, error) = parse_frontmatter_properties(source);
        assert!(!truncated);
        assert!(!error);
        assert_eq!(
            properties
                .iter()
                .map(|property| (
                    property.key.as_str(),
                    property.kind,
                    property.value.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("due", ViewPropertyKind::Date, "2026-11-01"),
                ("priority", ViewPropertyKind::Number, "3"),
                ("done", ViewPropertyKind::Boolean, "false"),
                ("empty", ViewPropertyKind::Null, ""),
                ("authors", ViewPropertyKind::List, "Alice, Bob"),
                ("nested", ViewPropertyKind::Nested, ""),
            ]
        );

        let long = format!("note: {}", "x".repeat(MAX_VIEW_PROPERTY_VALUE_CHARS + 5));
        let (properties, truncated, error) = parse_frontmatter_properties(&long);
        assert!(!error);
        assert!(truncated);
        assert!(properties[0].truncated);
        assert_eq!(
            properties[0].value.chars().count(),
            MAX_VIEW_PROPERTY_VALUE_CHARS
        );

        let (_, _, error) = parse_frontmatter_properties("broken: [");
        assert!(error);
    }

    #[test]
    fn custom_view_properties_are_normalized_and_cross_validated() {
        let config = ViewConfig {
            custom_properties: vec![" due ".into(), "priority".into()],
            date_property: Some("due".into()),
            ..Default::default()
        };
        let normalized = normalize_config(config).expect("valid custom properties");
        assert_eq!(normalized.custom_properties, ["due", "priority"]);
        assert_eq!(normalized.date_property.as_deref(), Some("due"));

        for invalid in [
            ViewConfig {
                custom_properties: vec!["due".into()],
                date_property: Some("other".into()),
                ..Default::default()
            },
            ViewConfig {
                custom_properties: vec!["tags".into()],
                ..Default::default()
            },
            ViewConfig {
                custom_properties: vec!["due".into(), "due".into()],
                ..Default::default()
            },
        ] {
            assert!(normalize_config(invalid).is_err());
        }
    }

    #[test]
    fn legacy_view_config_without_custom_property_fields_remains_readable() {
        let legacy: ViewConfig = serde_json::from_str(
            r#"{"visible_properties":["inbox_state","tags"],"board_group_by":"inbox_state","calendar_month":null}"#,
        )
        .expect("legacy view config must remain readable");
        assert!(legacy.custom_properties.is_empty());
        assert!(legacy.date_property.is_none());
    }
}
