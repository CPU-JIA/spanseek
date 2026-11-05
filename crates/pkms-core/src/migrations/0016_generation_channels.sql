-- Channel identity outlives deletion so immutable request snapshots can keep
-- their credential reference without rebinding it to a new channel.
-- Credentials are never part of this table or its portable snapshot.
CREATE TABLE generation_channels (
    id TEXT PRIMARY KEY,
    row_version INTEGER NOT NULL CHECK(row_version > 0 AND row_version <= 4294967295),
    spec_json TEXT NOT NULL CHECK(json_valid(spec_json)),
    deleted INTEGER NOT NULL CHECK(deleted IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
