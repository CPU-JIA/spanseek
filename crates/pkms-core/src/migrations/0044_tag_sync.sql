-- Accepted tag metadata and deferred assignments are knowledge, not disposable
-- engine cache. Keep them in complete backups together with their comparison base.
CREATE TABLE tag_sync_state (
    kind TEXT NOT NULL CHECK(kind IN ('tag','tag_assignments')),
    entity_id TEXT NOT NULL,
    object_kind TEXT CHECK(object_kind IN ('note','source','annotation','collection')),
    local_generation INTEGER NOT NULL DEFAULT 0 CHECK(local_generation >= 0),
    remote_version INTEGER NOT NULL DEFAULT 0 CHECK(remote_version >= 0),
    base_version INTEGER NOT NULL DEFAULT 0 CHECK(base_version >= 0 AND base_version <= remote_version),
    base_fingerprint TEXT CHECK(base_fingerprint IS NULL OR length(base_fingerprint)=64),
    pending_payload_json TEXT CHECK(pending_payload_json IS NULL OR json_valid(pending_payload_json)),
    pending_baseline_fingerprint TEXT CHECK(pending_baseline_fingerprint IS NULL OR length(pending_baseline_fingerprint)=64),
    pending_reason TEXT,
    pending_received_at TEXT,
    PRIMARY KEY(kind,entity_id),
    CHECK((kind='tag' AND object_kind IS NULL) OR (kind='tag_assignments' AND object_kind IS NOT NULL)),
    CHECK((pending_payload_json IS NULL AND pending_reason IS NULL AND pending_received_at IS NULL)
       OR (pending_payload_json IS NOT NULL AND pending_reason IS NOT NULL AND pending_received_at IS NOT NULL))
) STRICT;

CREATE TABLE tag_sync_tombstones (
    tag_id TEXT PRIMARY KEY,
    definition_json TEXT NOT NULL CHECK(json_valid(definition_json) AND json_type(definition_json)='object'),
    deleted_at TEXT NOT NULL
) STRICT;

INSERT INTO tag_sync_state(kind,entity_id,local_generation)
SELECT 'tag',id,1 FROM tags;
INSERT INTO tag_sync_state(kind,entity_id,object_kind,local_generation)
SELECT 'tag_assignments',o.id,o.kind,1 FROM objects o
WHERE EXISTS(SELECT 1 FROM object_tags ot WHERE ot.object_id=o.id);

CREATE TRIGGER tag_sync_dictionary_insert AFTER INSERT ON tags BEGIN
    INSERT INTO tag_sync_state(kind,entity_id,local_generation) VALUES('tag',NEW.id,1)
    ON CONFLICT(kind,entity_id) DO UPDATE SET local_generation=local_generation+1;
END;
CREATE TRIGGER tag_sync_dictionary_update AFTER UPDATE OF display_name,description,created_at,updated_at ON tags
WHEN OLD.display_name IS NOT NEW.display_name OR OLD.description IS NOT NEW.description
  OR OLD.created_at IS NOT NEW.created_at OR OLD.updated_at IS NOT NEW.updated_at BEGIN
    INSERT INTO tag_sync_state(kind,entity_id,local_generation) VALUES('tag',NEW.id,1)
    ON CONFLICT(kind,entity_id) DO UPDATE SET local_generation=local_generation+1;
END;
CREATE TRIGGER tag_sync_dictionary_delete BEFORE DELETE ON tags BEGIN
    INSERT INTO tag_sync_tombstones(tag_id,definition_json,deleted_at)
    VALUES(OLD.id,json_object('id',OLD.id,'display_name',OLD.display_name,'description',OLD.description,
      'created_at',OLD.created_at,'updated_at',OLD.updated_at,
      'deleted_at',strftime('%Y-%m-%dT%H:%M:%fZ','now')),strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    ON CONFLICT(tag_id) DO UPDATE SET definition_json=excluded.definition_json,deleted_at=excluded.deleted_at;
    INSERT INTO tag_sync_state(kind,entity_id,local_generation) VALUES('tag',OLD.id,1)
    ON CONFLICT(kind,entity_id) DO UPDATE SET local_generation=local_generation+1;
END;
CREATE TRIGGER tag_sync_assignment_insert AFTER INSERT ON object_tags BEGIN
    INSERT INTO tag_sync_state(kind,entity_id,object_kind,local_generation)
    SELECT 'tag_assignments',NEW.object_id,kind,1 FROM objects WHERE id=NEW.object_id
    ON CONFLICT(kind,entity_id) DO UPDATE SET object_kind=excluded.object_kind,local_generation=local_generation+1;
END;
CREATE TRIGGER tag_sync_assignment_delete AFTER DELETE ON object_tags BEGIN
    INSERT INTO tag_sync_state(kind,entity_id,object_kind,local_generation)
    SELECT 'tag_assignments',OLD.object_id,kind,1 FROM objects WHERE id=OLD.object_id
    ON CONFLICT(kind,entity_id) DO UPDATE SET object_kind=excluded.object_kind,local_generation=local_generation+1;
END;
