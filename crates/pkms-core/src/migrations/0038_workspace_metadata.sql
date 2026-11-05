-- Type keys keep object-type browsing indexed without duplicating user YAML.
ALTER TABLE notes ADD COLUMN type_key TEXT;
CREATE INDEX notes_type_key_order ON notes(type_key, updated_at DESC, id);

-- Workspace-scoped configuration copied into every backup and synchronized as
-- one small config object. The registries retain their existing JSON contract.
CREATE TABLE workspace_metadata (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    object_type_registry TEXT NOT NULL,
    note_template_registry TEXT NOT NULL,
    row_version INTEGER NOT NULL CHECK(row_version>0),
    updated_at TEXT NOT NULL
);
INSERT INTO workspace_metadata(
    singleton,object_type_registry,note_template_registry,row_version,updated_at
) VALUES(
    1,
    '{"schema":1,"workspaces":{}}',
    '{"schema":1,"workspaces":{}}',
    1,
    '1970-01-01T00:00:00Z'
);