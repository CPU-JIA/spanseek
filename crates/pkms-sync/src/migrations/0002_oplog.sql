CREATE TABLE sync_objects (
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version >= 1),
    tombstone INTEGER NOT NULL CHECK(tombstone IN (0,1)),
    ciphertext BLOB,
    key_epoch INTEGER NOT NULL CHECK(key_epoch >= 1),
    last_seq INTEGER NOT NULL CHECK(last_seq >= 1),
    updated_by TEXT NOT NULL REFERENCES sync_devices(device_id),
    PRIMARY KEY(workspace_id, object_id),
    CHECK((tombstone = 1 AND ciphertext IS NULL) OR (tombstone = 0 AND ciphertext IS NOT NULL))
) STRICT;
CREATE TABLE sync_ops (
    server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('put','delete','restore')),
    base_version INTEGER NOT NULL CHECK(base_version >= 0),
    resulting_version INTEGER NOT NULL CHECK(resulting_version >= 1),
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    key_epoch INTEGER NOT NULL CHECK(key_epoch >= 1),
    ciphertext BLOB,
    tombstone INTEGER NOT NULL CHECK(tombstone IN (0,1)),
    request_hash BLOB NOT NULL CHECK(length(request_hash) = 32),
    client_time TEXT,
    server_time TEXT NOT NULL,
    CHECK((tombstone = 1 AND ciphertext IS NULL) OR (tombstone = 0 AND ciphertext IS NOT NULL))
) STRICT;
CREATE INDEX sync_ops_workspace_seq ON sync_ops(workspace_id, server_seq);
PRAGMA user_version = 2;