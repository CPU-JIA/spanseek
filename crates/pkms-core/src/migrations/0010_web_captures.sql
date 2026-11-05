CREATE TABLE source_web_captures (
    source_version_id TEXT PRIMARY KEY REFERENCES source_versions(id),
    final_url TEXT NOT NULL,
    http_status INTEGER NOT NULL CHECK(http_status BETWEEN 200 AND 299),
    content_type TEXT NOT NULL,
    captured_at TEXT NOT NULL
) STRICT;

CREATE TABLE source_parse_staging (
    parse_id TEXT PRIMARY KEY REFERENCES source_parses(id),
    job_id TEXT NOT NULL REFERENCES source_jobs(id),
    lease_token TEXT NOT NULL
) STRICT;
