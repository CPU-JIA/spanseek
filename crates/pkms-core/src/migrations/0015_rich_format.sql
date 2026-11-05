-- Keep immutable revisions byte-for-byte while extending their format gate.
-- The migration runner disables FK enforcement before BEGIN, then checks every
-- relationship before COMMIT and restores the connection setting on all exits.
-- Rebuild instead of renaming the old table: dependent foreign keys must retain
-- the canonical note_revisions name. The preview trigger is recreated because
-- it names the table during SQLite's ALTER TABLE schema validation.
DROP TRIGGER note_preview_changed;
CREATE TABLE note_revisions_v3 (
    revision_id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(id),
    revision INTEGER NOT NULL CHECK(revision > 0),
    parent_revision_id TEXT REFERENCES note_revisions(revision_id),
    title TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK(schema_version IN (1, 2, 3)),
    document_json TEXT NOT NULL CHECK(json_valid(document_json)),
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    author_kind TEXT NOT NULL CHECK(author_kind IN ('human', 'restore', 'copy', 'ai_proposal', 'legacy')),
    created_at TEXT NOT NULL,
    UNIQUE(note_id, revision)
) STRICT;
INSERT INTO note_revisions_v3(revision_id,note_id,revision,parent_revision_id,title,schema_version,document_json,content_hash,author_kind,created_at)
    SELECT revision_id,note_id,revision,parent_revision_id,title,schema_version,document_json,content_hash,author_kind,created_at FROM note_revisions;
DROP TABLE note_revisions;
ALTER TABLE note_revisions_v3 RENAME TO note_revisions;
CREATE TRIGGER note_preview_changed AFTER UPDATE OF current_revision_id ON notes
WHEN NEW.current_revision_id IS NOT NULL BEGIN
    INSERT INTO note_previews
    SELECT NEW.id,r.revision_id,COALESCE((SELECT substr(group_concat(value,' '),1,180)
        FROM json_tree(r.document_json) WHERE key='text' AND type='text'),'')
    FROM note_revisions r WHERE r.revision_id=NEW.current_revision_id
    ON CONFLICT(note_id) DO UPDATE SET revision_id=excluded.revision_id,preview=excluded.preview;
END;
