-- V1.9.6 external request audit.
--
-- This table deliberately excludes request bodies, search terms, excerpts,
-- paths, and bearer secrets. principal_id is a token ID for HTTP and the
-- fixed local principal for stdio MCP. Granted scopes are stored as JSON so a
-- later contract addition remains readable without reparsing free text.

CREATE TABLE external_audit_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL CHECK(length(request_id) BETWEEN 1 AND 128),
    occurred_at TEXT NOT NULL,
    transport TEXT NOT NULL CHECK(transport IN ('mcp','http')),
    principal_id TEXT NOT NULL CHECK(length(principal_id) BETWEEN 1 AND 128),
    method TEXT NOT NULL CHECK(length(method) BETWEEN 1 AND 64),
    required_scope TEXT NOT NULL CHECK(required_scope IN ('read','search','propose','audit')),
    granted_scopes_json TEXT NOT NULL CHECK(json_valid(granted_scopes_json) AND json_type(granted_scopes_json)='array'),
    outcome TEXT NOT NULL CHECK(outcome IN ('allowed','denied','failed')),
    error_code TEXT,
    subject_id TEXT CHECK(subject_id IS NULL OR length(subject_id)<=128),
    duration_ms INTEGER NOT NULL CHECK(duration_ms>=0)
) STRICT;

CREATE INDEX external_audit_events_time ON external_audit_events(seq DESC);
CREATE INDEX external_audit_events_request ON external_audit_events(request_id,seq DESC);
CREATE INDEX external_audit_events_principal ON external_audit_events(principal_id,seq DESC);
CREATE INDEX external_audit_events_subject ON external_audit_events(subject_id,seq DESC);
