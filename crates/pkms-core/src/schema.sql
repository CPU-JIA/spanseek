CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    current_revision INTEGER NOT NULL CHECK(current_revision > 0),
    anchor_id TEXT NOT NULL UNIQUE,
    trashed INTEGER NOT NULL DEFAULT 0 CHECK(trashed IN (0, 1)),
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE note_revisions (
    note_id TEXT NOT NULL REFERENCES notes(id),
    revision INTEGER NOT NULL,
    title TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
    document_json TEXT NOT NULL CHECK(json_valid(document_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(note_id, revision)
) STRICT;

CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT;
CREATE TABLE memberships (
    collection_id TEXT NOT NULL REFERENCES collections(id),
    note_id TEXT NOT NULL REFERENCES notes(id),
    PRIMARY KEY(collection_id, note_id)
) STRICT;
CREATE INDEX memberships_note ON memberships(note_id);

CREATE TABLE note_links (
    source_id TEXT NOT NULL REFERENCES notes(id),
    target_id TEXT NOT NULL REFERENCES notes(id),
    context TEXT NOT NULL,
    PRIMARY KEY(source_id, target_id)
) STRICT;
CREATE INDEX links_target ON note_links(target_id);

CREATE VIRTUAL TABLE note_fts USING fts5(note_id UNINDEXED, title_tokens, body_tokens);
PRAGMA application_id = 1347112275;
PRAGMA user_version = 1;
