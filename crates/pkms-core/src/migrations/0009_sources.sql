CREATE TABLE sources (
 id TEXT PRIMARY KEY REFERENCES objects(id), display_title TEXT NOT NULL CHECK(length(display_title) BETWEEN 1 AND 300),
 current_version_id TEXT NOT NULL REFERENCES source_versions(id) DEFERRABLE INITIALLY DEFERRED,
 metadata_revision INTEGER NOT NULL DEFAULT 1 CHECK(metadata_revision>0),
 created_at TEXT NOT NULL, updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE source_versions (
 id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id), blob_hash TEXT NOT NULL REFERENCES blobs(hash),
 original_name TEXT NOT NULL, origin_url TEXT, format TEXT NOT NULL CHECK(format IN ('text','markdown','pdf','html')),
 captured_at TEXT NOT NULL, UNIQUE(source_id,id)
) STRICT;
CREATE INDEX source_versions_blob ON source_versions(blob_hash,source_id);
CREATE INDEX source_versions_history ON source_versions(source_id,captured_at DESC,id DESC);
CREATE INDEX sources_page_order ON sources(updated_at DESC,id);
CREATE TABLE source_file_dedup (blob_hash TEXT PRIMARY KEY REFERENCES blobs(hash),source_id TEXT NOT NULL REFERENCES sources(id)) STRICT;
CREATE TABLE source_parses (
 id TEXT PRIMARY KEY, source_version_id TEXT NOT NULL REFERENCES source_versions(id), parser_version TEXT NOT NULL,
 page_count INTEGER CHECK(page_count>0), needs_ocr INTEGER NOT NULL CHECK(needs_ocr IN (0,1)),created_at TEXT NOT NULL
) STRICT;
CREATE TABLE source_active_parses (source_version_id TEXT PRIMARY KEY REFERENCES source_versions(id),parse_id TEXT NOT NULL REFERENCES source_parses(id)) STRICT;
CREATE TABLE source_blocks (
 id TEXT PRIMARY KEY,parse_id TEXT NOT NULL REFERENCES source_parses(id),position INTEGER NOT NULL CHECK(position>=0),
 text TEXT NOT NULL,page_index INTEGER CHECK(page_index>=0),geometry_json TEXT CHECK(geometry_json IS NULL OR json_valid(geometry_json)),UNIQUE(parse_id,position)
) STRICT;
CREATE INDEX source_blocks_page ON source_blocks(parse_id,page_index,position);
CREATE VIRTUAL TABLE source_fts USING fts5(source_id UNINDEXED,source_version_id UNINDEXED,block_id UNINDEXED,title_tokens,body_tokens);
CREATE TABLE annotations (
 id TEXT PRIMARY KEY REFERENCES objects(id),source_version_id TEXT NOT NULL REFERENCES source_versions(id),
 locator_schema INTEGER NOT NULL CHECK(locator_schema=1),locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),comment TEXT NOT NULL
) STRICT;
CREATE INDEX annotations_by_version ON annotations(source_version_id,id);
CREATE TABLE reading_states (
 source_id TEXT PRIMARY KEY REFERENCES sources(id),source_version_id TEXT NOT NULL REFERENCES source_versions(id),
 locator_json TEXT CHECK(locator_json IS NULL OR json_valid(locator_json)),status TEXT NOT NULL CHECK(status IN ('unread','reading','finished')),
 row_version INTEGER NOT NULL CHECK(row_version>0)
) STRICT;
CREATE TABLE source_jobs (
 id TEXT PRIMARY KEY,input_json TEXT NOT NULL CHECK(json_valid(input_json)),
 status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','canceled')),stage TEXT NOT NULL,
 attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt BETWEEN 0 AND 3),config_revision INTEGER NOT NULL DEFAULT 1 CHECK(config_revision=1),lease_token TEXT,lease_expires_at TEXT,
 cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
 source_id TEXT REFERENCES sources(id),source_version_id TEXT REFERENCES source_versions(id),
 error_code TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX source_jobs_pending ON source_jobs(status,created_at,id);
CREATE INDEX source_jobs_history ON source_jobs(created_at DESC,id DESC);
CREATE TABLE source_job_attempts (
 job_id TEXT NOT NULL REFERENCES source_jobs(id),attempt INTEGER NOT NULL,lease_token TEXT NOT NULL UNIQUE,
 started_at TEXT NOT NULL,finished_at TEXT,error_code TEXT,PRIMARY KEY(job_id,attempt)
) STRICT;
CREATE TABLE source_job_events (
 job_id TEXT NOT NULL REFERENCES source_jobs(id),seq INTEGER NOT NULL CHECK(seq>0),status TEXT NOT NULL,stage TEXT NOT NULL,created_at TEXT NOT NULL,
 PRIMARY KEY(job_id,seq)
) STRICT;
CREATE TABLE source_list_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),generation INTEGER NOT NULL) STRICT;
INSERT INTO source_list_state VALUES(1,1);
CREATE TRIGGER source_list_insert AFTER INSERT ON sources BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_list_update AFTER UPDATE ON sources BEGIN UPDATE source_list_state SET generation=generation+1; END;
CREATE TRIGGER source_metadata_update AFTER UPDATE ON objects WHEN NEW.kind='source' BEGIN UPDATE source_list_state SET generation=generation+1; END;
