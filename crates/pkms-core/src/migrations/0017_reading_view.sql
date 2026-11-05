ALTER TABLE reading_states ADD COLUMN pdf_view_json TEXT
    CHECK(pdf_view_json IS NULL OR json_valid(pdf_view_json));

-- Identity-only tombstones prevent stale local sessions and retry intents from
-- presenting permanently deleted objects as live or recoverable content.
CREATE TABLE object_tombstones (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('note','source','annotation','collection')),
    command_id TEXT NOT NULL REFERENCES command_receipts(command_id) DEFERRABLE INITIALLY DEFERRED,
    deleted_at TEXT NOT NULL
) STRICT;
