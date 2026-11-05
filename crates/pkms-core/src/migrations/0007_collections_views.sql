ALTER TABLE collections ADD COLUMN overview_note_id TEXT REFERENCES notes(id);
ALTER TABLE collections ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0 CHECK(is_pinned IN (0,1));
ALTER TABLE collections ADD COLUMN pin_position INTEGER CHECK(
    (is_pinned=0 AND pin_position IS NULL) OR
    (is_pinned=1 AND pin_position IS NOT NULL AND pin_position BETWEEN 0 AND 4294967295)
);
-- Earlier collections did not record either timestamp. Unknown remains NULL.
ALTER TABLE collections ADD COLUMN created_at TEXT;
ALTER TABLE collections ADD COLUMN updated_at TEXT;
ALTER TABLE memberships RENAME TO legacy_memberships;
CREATE TABLE memberships (
    collection_id TEXT NOT NULL REFERENCES collections(id),
    member_id TEXT NOT NULL REFERENCES objects(id),
    position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 4294967295),
    PRIMARY KEY(collection_id,member_id),
    UNIQUE(collection_id,position)
) STRICT;
INSERT INTO memberships(collection_id,member_id,position)
SELECT collection_id,note_id,row_number() OVER(PARTITION BY collection_id ORDER BY note_id)-1
FROM legacy_memberships;
DROP TABLE legacy_memberships;
CREATE INDEX memberships_by_member ON memberships(member_id,collection_id);
CREATE INDEX notes_title_page_order ON notes(trashed,title,id);

CREATE TABLE collection_list_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL CHECK(generation>0)
) STRICT;
INSERT INTO collection_list_state VALUES(1,1);
CREATE TRIGGER collection_list_insert AFTER INSERT ON collections BEGIN
    UPDATE collection_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER collection_list_update AFTER UPDATE ON collections BEGIN
    UPDATE collection_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER collection_metadata_list_update AFTER UPDATE ON objects WHEN NEW.kind='collection' BEGIN
    UPDATE collection_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER membership_list_insert AFTER INSERT ON memberships BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
    UPDATE collection_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER membership_list_delete AFTER DELETE ON memberships BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
    UPDATE collection_list_state SET generation=generation+1 WHERE singleton=1;
END;

CREATE TABLE saved_views (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 300),
    filter_schema INTEGER NOT NULL CHECK(filter_schema=1),
    filter_ast TEXT NOT NULL CHECK(json_valid(filter_ast)),
    sort TEXT NOT NULL CHECK(sort IN ('updated_desc','updated_asc','title_asc','title_desc')),
    layout TEXT NOT NULL CHECK(layout IN ('list','table')),
    is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
    pin_position INTEGER CHECK(
        (is_pinned=0 AND pin_position IS NULL) OR
        (is_pinned=1 AND pin_position IS NOT NULL AND pin_position BETWEEN 0 AND 4294967295)
    ),
    row_version INTEGER NOT NULL CHECK(row_version BETWEEN 1 AND 4294967295),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE view_list_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL CHECK(generation>0)
) STRICT;
INSERT INTO view_list_state VALUES(1,1);
CREATE TRIGGER view_list_insert AFTER INSERT ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER view_list_update AFTER UPDATE ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER view_list_delete AFTER DELETE ON saved_views BEGIN
    UPDATE view_list_state SET generation=generation+1 WHERE singleton=1;
END;
