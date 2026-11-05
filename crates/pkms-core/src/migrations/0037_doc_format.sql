-- Widen the source-version format gate to legacy Word 97-2003 binary documents.
-- The rebuild mirrors earlier format migrations because SQLite cannot alter a CHECK.
DROP TRIGGER index_dirty_parse_insert;
DROP TRIGGER index_dirty_parse_update;
CREATE TABLE source_versions_v37 (
 id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id), blob_hash TEXT NOT NULL REFERENCES blobs(hash),
 original_name TEXT NOT NULL, origin_url TEXT, format TEXT NOT NULL CHECK(format IN ('text','markdown','pdf','html','docx','doc','epub','mobi','csv','xlsx','xls','pptx')),
 captured_at TEXT NOT NULL, UNIQUE(source_id,id)
) STRICT;
INSERT INTO source_versions_v37(id,source_id,blob_hash,original_name,origin_url,format,captured_at)
    SELECT id,source_id,blob_hash,original_name,origin_url,format,captured_at FROM source_versions;
DROP TABLE source_versions;
ALTER TABLE source_versions_v37 RENAME TO source_versions;
CREATE INDEX source_versions_blob ON source_versions(blob_hash,source_id);
CREATE INDEX source_versions_history ON source_versions(source_id,captured_at DESC,id DESC);
CREATE TRIGGER index_dirty_parse_insert AFTER INSERT ON source_active_parses BEGIN INSERT INTO index_dirty_objects(object_id) SELECT source_id FROM source_versions WHERE id=NEW.source_version_id ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_parse_update AFTER UPDATE ON source_active_parses BEGIN INSERT INTO index_dirty_objects(object_id) SELECT source_id FROM source_versions WHERE id=NEW.source_version_id ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;