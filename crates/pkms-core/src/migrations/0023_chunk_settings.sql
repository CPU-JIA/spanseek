-- Per-library chunking: the token cap and the overlap between consecutive chunks
-- of one block. Both are part of the embedding profile identity, so saving them
-- registers a new profile and rebuilds the index; the replaced profile id is kept
-- so the previous index can be restored without re-embedding anything.
ALTER TABLE search_settings ADD COLUMN chunk_params_json TEXT
    CHECK(chunk_params_json IS NULL OR json_valid(chunk_params_json));
ALTER TABLE search_settings ADD COLUMN chunk_row_version INTEGER NOT NULL DEFAULT 0
    CHECK(chunk_row_version>=0);
ALTER TABLE search_settings ADD COLUMN previous_profile_id TEXT;