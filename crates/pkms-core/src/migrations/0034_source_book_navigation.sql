-- Ordered table-of-contents and extracted cover assets for reflowable books.
-- The immutable source original remains authoritative; these rows are a
-- rebuildable reading projection published atomically with its source parse.
ALTER TABLE source_parses ADD COLUMN toc_json TEXT CHECK(toc_json IS NULL OR json_valid(toc_json));
CREATE TABLE source_covers (
 parse_id TEXT PRIMARY KEY REFERENCES source_parses(id),
 blob_hash TEXT NOT NULL REFERENCES blobs(hash),
 created_at TEXT NOT NULL
) STRICT;
CREATE INDEX source_covers_blob ON source_covers(blob_hash);
