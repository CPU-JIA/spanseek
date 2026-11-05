-- Per-library retrieval parameters (Top-K, answer citation count, optional
-- similarity floor). NULL means "use the built-in defaults", so an existing
-- library keeps its current behaviour until the user saves explicit values.
ALTER TABLE search_settings ADD COLUMN retrieval_params_json TEXT
    CHECK(retrieval_params_json IS NULL OR json_valid(retrieval_params_json));
ALTER TABLE search_settings ADD COLUMN retrieval_row_version INTEGER NOT NULL DEFAULT 0
    CHECK(retrieval_row_version>=0);

