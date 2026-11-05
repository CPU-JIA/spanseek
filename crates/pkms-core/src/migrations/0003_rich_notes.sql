ALTER TABLE note_revisions RENAME TO legacy_note_revisions;
CREATE TABLE note_revisions (
    revision_id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(id),
    revision INTEGER NOT NULL CHECK(revision > 0),
    parent_revision_id TEXT REFERENCES note_revisions(revision_id),
    title TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK(schema_version IN (1, 2)),
    document_json TEXT NOT NULL CHECK(json_valid(document_json)),
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    author_kind TEXT NOT NULL CHECK(author_kind IN ('human', 'restore', 'copy', 'ai_proposal', 'legacy')),
    created_at TEXT NOT NULL,
    UNIQUE(note_id, revision)
) STRICT;
ALTER TABLE notes ADD COLUMN current_revision_id TEXT REFERENCES note_revisions(revision_id);
CREATE TABLE objects (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('note', 'collection', 'source', 'annotation')),
    row_version INTEGER NOT NULL DEFAULT 1,
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1)),
    trashed INTEGER NOT NULL DEFAULT 0 CHECK(trashed IN (0, 1)),
    inbox_state TEXT CHECK(inbox_state IN ('pending', 'snoozed', 'processed')),
    search_enabled INTEGER NOT NULL DEFAULT 1 CHECK(search_enabled IN (0, 1))
) STRICT;
INSERT INTO objects(id,kind,trashed,inbox_state) SELECT id,'note',trashed,'pending' FROM notes;
INSERT INTO objects(id,kind) SELECT id,'collection' FROM collections;
CREATE TABLE blocks (
    id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(id),
    active INTEGER NOT NULL CHECK(active IN (0, 1))
) STRICT;
CREATE TABLE note_revision_blocks (
    revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    block_id TEXT NOT NULL REFERENCES blocks(id),
    PRIMARY KEY(revision_id,block_id)
) STRICT;
CREATE TABLE body_links (
    source_id TEXT NOT NULL REFERENCES notes(id),
    source_revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    occurrence_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    target_block_id TEXT,
    context TEXT NOT NULL,
    PRIMARY KEY(source_id,occurrence_id)
) STRICT;
CREATE INDEX body_links_target ON body_links(target_id);
CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY,
    payload_hash TEXT NOT NULL,
    command_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    change_seq INTEGER NOT NULL UNIQUE,
    committed_at TEXT NOT NULL
) STRICT;
CREATE TABLE note_index_queue (
    note_id TEXT PRIMARY KEY REFERENCES notes(id),
    revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    queued_at TEXT NOT NULL
) STRICT;
