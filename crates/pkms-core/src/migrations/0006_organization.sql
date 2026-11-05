CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 100),
    normalized_name TEXT NOT NULL UNIQUE CHECK(length(normalized_name) BETWEEN 1 AND 100)
) STRICT;
CREATE TABLE object_tags (
    object_id TEXT NOT NULL REFERENCES objects(id),
    tag_id TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY(object_id,tag_id)
) STRICT;
CREATE INDEX object_tags_by_tag ON object_tags(tag_id,object_id);
CREATE INDEX objects_inbox ON objects(kind,trashed,inbox_state,id);

-- A list cursor describes both note content and its organization at one point.
-- Keep invalidation in the database so legacy CLI writes are covered too.
CREATE TRIGGER object_metadata_list_update AFTER UPDATE ON objects BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER object_tag_list_insert AFTER INSERT ON object_tags BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER object_tag_list_delete AFTER DELETE ON object_tags BEGIN
    UPDATE note_list_state SET generation=generation+1 WHERE singleton=1;
END;

CREATE TABLE tag_list_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL CHECK(generation>0)
) STRICT;
INSERT INTO tag_list_state VALUES(1,1);
CREATE TRIGGER tag_list_insert AFTER INSERT ON tags BEGIN
    UPDATE tag_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER tag_list_update AFTER UPDATE ON tags BEGIN
    UPDATE tag_list_state SET generation=generation+1 WHERE singleton=1;
END;
CREATE TRIGGER tag_list_delete AFTER DELETE ON tags BEGIN
    UPDATE tag_list_state SET generation=generation+1 WHERE singleton=1;
END;
