CREATE TABLE embedding_profiles(id TEXT PRIMARY KEY,spec_json TEXT NOT NULL CHECK(json_valid(spec_json)),created_at TEXT NOT NULL) STRICT;
CREATE TABLE search_settings(singleton INTEGER PRIMARY KEY CHECK(singleton=1),active_profile_id TEXT REFERENCES embedding_profiles(id),generation_profile_json TEXT CHECK(generation_profile_json IS NULL OR json_valid(generation_profile_json)),generation_row_version INTEGER NOT NULL DEFAULT 0 CHECK(generation_row_version>=0)) STRICT;
INSERT INTO search_settings VALUES(1,NULL,NULL,0);
CREATE TABLE index_generations(
 id TEXT PRIMARY KEY,object_id TEXT NOT NULL REFERENCES objects(id),content_revision TEXT NOT NULL,
 metadata_revision INTEGER NOT NULL,profile_id TEXT NOT NULL REFERENCES embedding_profiles(id),parser_revision TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('running','ready','failed','canceled')),chunk_count INTEGER NOT NULL,
 lease_token TEXT,lease_expires_at TEXT,error_code TEXT,created_at TEXT NOT NULL,request_hash TEXT NOT NULL
) STRICT;
CREATE INDEX index_generations_pending ON index_generations(status,created_at,id);
CREATE TABLE index_chunks(id TEXT NOT NULL,generation_id TEXT NOT NULL REFERENCES index_generations(id),target_json TEXT NOT NULL CHECK(json_valid(target_json)),title TEXT NOT NULL,text TEXT NOT NULL,token_count INTEGER NOT NULL,PRIMARY KEY(generation_id,id)) STRICT;
CREATE INDEX index_chunks_generation ON index_chunks(generation_id,id);
CREATE TABLE active_index_generations(object_id TEXT NOT NULL REFERENCES objects(id),profile_id TEXT NOT NULL REFERENCES embedding_profiles(id),generation_id TEXT NOT NULL REFERENCES index_generations(id),PRIMARY KEY(object_id,profile_id)) STRICT;
CREATE TABLE answers(
 id TEXT PRIMARY KEY,request_json TEXT NOT NULL CHECK(json_valid(request_json)),status TEXT NOT NULL,
 text TEXT NOT NULL DEFAULT '',evidence_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence_json)),
 cited_ids_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(cited_ids_json)),evidence_prepared INTEGER NOT NULL DEFAULT 0 CHECK(evidence_prepared IN (0,1)),error_code TEXT,
 lease_token TEXT,lease_expires_at TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX answers_pending ON answers(status,created_at,id);
CREATE TABLE answer_events(answer_id TEXT NOT NULL REFERENCES answers(id),seq INTEGER NOT NULL,status TEXT NOT NULL,text_delta TEXT,PRIMARY KEY(answer_id,seq)) STRICT;
CREATE TABLE answer_feedback(answer_id TEXT PRIMARY KEY REFERENCES answers(id),rating INTEGER NOT NULL CHECK(rating BETWEEN -1 AND 1),comment TEXT NOT NULL) STRICT;
CREATE TABLE proposals(id TEXT PRIMARY KEY,request_json TEXT NOT NULL CHECK(json_valid(request_json)),status TEXT NOT NULL CHECK(status IN ('pending','applied','rejected')),applied_revision_id TEXT REFERENCES note_revisions(revision_id),created_at TEXT NOT NULL) STRICT;
CREATE TABLE proposal_blobs(proposal_id TEXT NOT NULL REFERENCES proposals(id),blob_hash TEXT NOT NULL REFERENCES blobs(hash),PRIMARY KEY(proposal_id,blob_hash)) STRICT;
CREATE TABLE index_rebuild_jobs(id TEXT PRIMARY KEY,request_json TEXT NOT NULL CHECK(json_valid(request_json)),status TEXT NOT NULL CHECK(status IN ('queued','running','completed','canceled','failed')),after_object_id TEXT,processed INTEGER NOT NULL DEFAULT 0 CHECK(processed>=0),error_code TEXT,lease_token TEXT,lease_expires_at TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL) STRICT;
CREATE INDEX index_rebuild_jobs_pending ON index_rebuild_jobs(status,created_at,id);
CREATE TABLE index_rebuild_objects(job_id TEXT NOT NULL REFERENCES index_rebuild_jobs(id),object_id TEXT NOT NULL REFERENCES objects(id),generation_id TEXT REFERENCES index_generations(id),skip_reason TEXT,content_revision TEXT NOT NULL,metadata_revision INTEGER NOT NULL,parser_revision TEXT NOT NULL,parse_id TEXT,PRIMARY KEY(job_id,object_id)) STRICT;
CREATE INDEX index_rebuild_skips ON index_rebuild_objects(job_id,skip_reason);
ALTER TABLE source_job_events ADD COLUMN attempt INTEGER CHECK(attempt BETWEEN 0 AND 3);
ALTER TABLE body_links ADD COLUMN target_revision_id TEXT;
CREATE TABLE index_dirty_objects(object_id TEXT PRIMARY KEY REFERENCES objects(id) DEFERRABLE INITIALLY DEFERRED,attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 3),next_retry_at TEXT,last_error TEXT) STRICT;
CREATE TRIGGER index_dirty_note_insert AFTER INSERT ON notes BEGIN INSERT INTO index_dirty_objects(object_id) VALUES(NEW.id) ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_note_update AFTER UPDATE ON notes BEGIN INSERT INTO index_dirty_objects(object_id) VALUES(NEW.id) ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_source_insert AFTER INSERT ON sources BEGIN INSERT INTO index_dirty_objects(object_id) VALUES(NEW.id) ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_source_update AFTER UPDATE ON sources BEGIN INSERT INTO index_dirty_objects(object_id) VALUES(NEW.id) ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_object_update AFTER UPDATE ON objects WHEN NEW.kind IN ('note','source') BEGIN INSERT INTO index_dirty_objects(object_id) VALUES(NEW.id) ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_parse_insert AFTER INSERT ON source_active_parses BEGIN INSERT INTO index_dirty_objects(object_id) SELECT source_id FROM source_versions WHERE id=NEW.source_version_id ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
CREATE TRIGGER index_dirty_parse_update AFTER UPDATE ON source_active_parses BEGIN INSERT INTO index_dirty_objects(object_id) SELECT source_id FROM source_versions WHERE id=NEW.source_version_id ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL; END;
INSERT INTO index_dirty_objects(object_id) SELECT id FROM objects WHERE kind IN ('note','source');
