ALTER TABLE sync_objects ADD COLUMN purged INTEGER NOT NULL DEFAULT 0 CHECK(purged IN (0,1));
ALTER TABLE sync_objects ADD COLUMN deleted_at TEXT;
ALTER TABLE sync_objects ADD COLUMN deleted_by TEXT;

CREATE TABLE sync_ops_v4 (
    server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('put','delete','restore','purge')),
    base_version INTEGER NOT NULL CHECK(base_version >= 0),
    resulting_version INTEGER NOT NULL CHECK(resulting_version >= 1),
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    key_epoch INTEGER NOT NULL CHECK(key_epoch >= 1),
    ciphertext BLOB,
    tombstone INTEGER NOT NULL CHECK(tombstone IN (0,1)),
    purged INTEGER NOT NULL DEFAULT 0 CHECK(purged IN (0,1)),
    deleted_at TEXT,
    deleted_by TEXT,
    request_hash BLOB NOT NULL CHECK(length(request_hash) = 32),
    client_time TEXT,
    server_time TEXT NOT NULL,
    CHECK((tombstone = 1 AND ciphertext IS NULL) OR (tombstone = 0 AND ciphertext IS NOT NULL))
) STRICT;
INSERT INTO sync_ops_v4(server_seq,op_id,workspace_id,object_id,kind,base_version,resulting_version,device_id,key_epoch,ciphertext,tombstone,purged,deleted_at,deleted_by,request_hash,client_time,server_time)
SELECT server_seq,op_id,workspace_id,object_id,kind,base_version,resulting_version,device_id,key_epoch,ciphertext,tombstone,0,NULL,NULL,request_hash,client_time,server_time FROM sync_ops;
DROP TABLE sync_ops;
ALTER TABLE sync_ops_v4 RENAME TO sync_ops;
CREATE INDEX sync_ops_workspace_seq ON sync_ops(workspace_id, server_seq);
UPDATE sqlite_sequence SET seq=(SELECT COALESCE(MAX(server_seq),0) FROM sync_ops) WHERE name='sync_ops';
INSERT INTO sqlite_sequence(name,seq) SELECT 'sync_ops',(SELECT COALESCE(MAX(server_seq),0) FROM sync_ops) WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='sync_ops');

UPDATE sync_objects SET
    deleted_by = updated_by,
    deleted_at = (
        SELECT server_time FROM sync_ops
        WHERE workspace_id = sync_objects.workspace_id
          AND object_id = sync_objects.object_id
          AND kind = 'delete'
        ORDER BY server_seq DESC
        LIMIT 1
    )
WHERE tombstone = 1;

CREATE TABLE sync_conflicts_v4 (
    conflict_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    op_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('put','delete','restore','purge')),
    base_version INTEGER NOT NULL CHECK(base_version >= 0),
    remote_version INTEGER NOT NULL CHECK(remote_version >= 0),
    remote_tombstone INTEGER NOT NULL CHECK(remote_tombstone IN (0,1)),
    reason TEXT NOT NULL CHECK(reason IN ('version_mismatch','deleted','not_found','already_live','purged')),
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
INSERT INTO sync_conflicts_v4(conflict_id,device_id,workspace_id,object_id,op_id,kind,base_version,remote_version,remote_tombstone,reason,created_at,resolved_at,resolution,replacement_op_id,replacement_object_id)
SELECT conflict_id,device_id,workspace_id,object_id,op_id,kind,base_version,remote_version,remote_tombstone,reason,created_at,resolved_at,resolution,replacement_op_id,replacement_object_id FROM sync_conflicts;
DROP TABLE sync_conflicts;
ALTER TABLE sync_conflicts_v4 RENAME TO sync_conflicts;
CREATE INDEX sync_conflicts_device_created ON sync_conflicts(device_id, created_at);
PRAGMA user_version = 4;