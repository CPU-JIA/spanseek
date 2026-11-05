CREATE TABLE sync_conflicts (
    conflict_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    op_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('put','delete','restore')),
    base_version INTEGER NOT NULL CHECK(base_version >= 0),
    remote_version INTEGER NOT NULL CHECK(remote_version >= 0),
    remote_tombstone INTEGER NOT NULL CHECK(remote_tombstone IN (0,1)),
    reason TEXT NOT NULL CHECK(reason IN ('version_mismatch','deleted','not_found','already_live')),
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    resolution TEXT CHECK(resolution IN ('keep_local','keep_remote','save_as_new')),
    replacement_op_id TEXT,
    replacement_object_id TEXT,
    UNIQUE(device_id, op_id),
    CHECK(
        (resolved_at IS NULL AND resolution IS NULL AND replacement_op_id IS NULL AND replacement_object_id IS NULL)
        OR
        (resolved_at IS NOT NULL AND resolution IS NOT NULL AND (
            (resolution = 'keep_remote' AND replacement_op_id IS NULL AND replacement_object_id IS NULL)
            OR (resolution = 'keep_local' AND replacement_op_id IS NOT NULL AND replacement_object_id IS NULL)
            OR (resolution = 'save_as_new' AND replacement_op_id IS NOT NULL AND replacement_object_id IS NOT NULL)
        ))
    )
) STRICT;
CREATE INDEX sync_conflicts_device_created ON sync_conflicts(device_id, created_at);
PRAGMA user_version = 3;