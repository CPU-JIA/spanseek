CREATE TABLE blobs (
    hash TEXT PRIMARY KEY CHECK(length(hash) = 64),
    size INTEGER NOT NULL CHECK(size > 0),
    media_type TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE note_revision_blobs (
    revision_id TEXT NOT NULL REFERENCES note_revisions(revision_id),
    blob_hash TEXT NOT NULL REFERENCES blobs(hash),
    node_id TEXT NOT NULL REFERENCES blocks(id),
    original_name TEXT,
    PRIMARY KEY(revision_id,node_id)
) STRICT;
CREATE INDEX revision_blobs_hash ON note_revision_blobs(blob_hash);

ALTER TABLE command_receipts RENAME TO legacy_command_receipts;
CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY,
    payload_hash TEXT NOT NULL,
    command_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    revision_id TEXT REFERENCES note_revisions(revision_id),
    change_seq INTEGER NOT NULL UNIQUE,
    committed_at TEXT NOT NULL,
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json))
) STRICT;
INSERT INTO command_receipts(command_id,payload_hash,command_kind,object_id,revision_id,change_seq,committed_at)
    SELECT command_id,payload_hash,command_kind,object_id,revision_id,change_seq,committed_at FROM legacy_command_receipts;
DROP TABLE legacy_command_receipts;
