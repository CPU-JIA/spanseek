CREATE TABLE portable_jobs (
    id TEXT PRIMARY KEY,
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','canceled')),
    stage TEXT NOT NULL,
    lease_id TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX portable_jobs_pending ON portable_jobs(status,created_at,id);
