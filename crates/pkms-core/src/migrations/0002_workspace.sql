-- The first incremental database migration. Rich documents follow separately.
CREATE TABLE workspace_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    workspace_id TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL CHECK(schema_version > 0),
    next_change_seq INTEGER NOT NULL CHECK(next_change_seq > 0)
) STRICT;

CREATE TABLE migration_history (
    version INTEGER PRIMARY KEY CHECK(version > 0),
    checksum TEXT NOT NULL CHECK(length(checksum) = 64),
    applied_at TEXT NOT NULL
) STRICT;
