-- Text cleaning is a per-library choice: the row keeps both the options and its
-- own version so two windows cannot overwrite each other silently, and every
-- parse keeps the report of what its cleaning step removed. Originals are never
-- touched; cleaning only shapes the derived text blocks.
ALTER TABLE search_settings ADD COLUMN cleaning_params_json TEXT
    CHECK(cleaning_params_json IS NULL OR json_valid(cleaning_params_json));
ALTER TABLE search_settings ADD COLUMN cleaning_row_version INTEGER NOT NULL DEFAULT 0
    CHECK(cleaning_row_version>=0);
ALTER TABLE source_parses ADD COLUMN cleaning_json TEXT
    CHECK(cleaning_json IS NULL OR json_valid(cleaning_json));