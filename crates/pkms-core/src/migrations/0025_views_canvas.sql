-- V1.9.3: saved views keep their existing identity and gain a versioned
-- layout configuration. The layout union now includes the database-style
-- board, calendar and timeline presentations; the filter remains a bounded
-- NotesFilter JSON document, never SQL.
DROP TRIGGER IF EXISTS view_list_insert;
DROP TRIGGER IF EXISTS view_list_update;
DROP TRIGGER IF EXISTS view_list_delete;
ALTER TABLE saved_views RENAME TO saved_views_v24;
CREATE TABLE saved_views (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 300),
    filter_schema INTEGER NOT NULL CHECK(filter_schema=1),
    filter_ast TEXT NOT NULL CHECK(json_valid(filter_ast)),
    sort TEXT NOT NULL CHECK(sort IN ('updated_desc','updated_asc','title_asc','title_desc')),
    layout TEXT NOT NULL CHECK(layout IN ('list','table','board','calendar','timeline')),
    config_schema INTEGER NOT NULL DEFAULT 1 CHECK(config_schema=1),
    config_json TEXT NOT NULL DEFAULT '{"visible_properties":["inbox_state","tags","updated_at"],"board_group_by":"inbox_state","date_field":"updated_at","calendar_month":null}' CHECK(json_valid(config_json) AND json_type(config_json)='object'),
    is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
    pin_position INTEGER CHECK(
        (is_pinned=0 AND pin_position IS NULL) OR
        (is_pinned=1 AND pin_position IS NOT NULL AND pin_position BETWEEN 0 AND 4294967295)
    ),
    row_version INTEGER NOT NULL CHECK(row_version BETWEEN 1 AND 4294967295),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
INSERT INTO saved_views(id,name,filter_schema,filter_ast,sort,layout,config_schema,config_json,is_pinned,pin_position,row_version,created_at,updated_at)
SELECT id,name,filter_schema,filter_ast,sort,layout,1,
       '{"visible_properties":["inbox_state","tags","updated_at"],"board_group_by":"inbox_state","date_field":"updated_at","calendar_month":null}',
       is_pinned,pin_position,row_version,created_at,updated_at
FROM saved_views_v24;
DROP TABLE saved_views_v24;
CREATE TRIGGER view_list_insert AFTER INSERT ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER view_list_update AFTER UPDATE ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER view_list_delete AFTER DELETE ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;

-- A canvas stores only layout and references to existing knowledge objects.
-- Note bodies remain in note_revisions; deleting or trashing a note never
-- copies or rewrites it here.
CREATE TABLE canvases (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 300),
    config_schema INTEGER NOT NULL CHECK(config_schema=1),
    nodes_json TEXT NOT NULL CHECK(json_valid(nodes_json) AND json_type(nodes_json)='array'),
    is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
    pin_position INTEGER CHECK(
        (is_pinned=0 AND pin_position IS NULL) OR
        (is_pinned=1 AND pin_position IS NOT NULL AND pin_position BETWEEN 0 AND 4294967295)
    ),
    row_version INTEGER NOT NULL CHECK(row_version BETWEEN 1 AND 4294967295),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE canvas_list_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL CHECK(generation>0)
) STRICT;
INSERT INTO canvas_list_state VALUES(1,1);
CREATE TRIGGER canvas_list_insert AFTER INSERT ON canvases BEGIN
    UPDATE canvas_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER canvas_list_update AFTER UPDATE ON canvases BEGIN
    UPDATE canvas_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER canvas_list_delete AFTER DELETE ON canvases BEGIN
    UPDATE canvas_list_state SET generation=generation+1 WHERE singleton=1;
END;

-- Local deletions must survive until the next sync, otherwise a device that
-- deletes a view/canvas offline would silently leave the remote copy alive.
CREATE TABLE sync_config_tombstones (
    object_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('view','canvas')),
    deleted_at TEXT NOT NULL
) STRICT;