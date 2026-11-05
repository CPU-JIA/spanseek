CREATE TABLE app_preferences (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    state_version INTEGER NOT NULL CHECK(state_version>0),
    preferences_json TEXT NOT NULL CHECK(json_valid(preferences_json))
) STRICT;
CREATE TABLE workspace_catalog (
    local_instance_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    storage_identity TEXT NOT NULL,
    last_opened_at TEXT NOT NULL,
    show_in_recents INTEGER NOT NULL CHECK(show_in_recents IN (0,1)),
    UNIQUE(canonical_path,storage_identity,workspace_id),
    UNIQUE(local_instance_id,workspace_id)
) STRICT;
CREATE TABLE local_sessions (
    local_instance_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    state_version INTEGER NOT NULL CHECK(state_version>0),
    session_json TEXT NOT NULL CHECK(json_valid(session_json)),
    PRIMARY KEY(local_instance_id,workspace_id),
    FOREIGN KEY(local_instance_id,workspace_id) REFERENCES workspace_catalog(local_instance_id,workspace_id)
) STRICT;
CREATE TABLE recent_objects (
    local_instance_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    last_opened_at TEXT NOT NULL,
    PRIMARY KEY(local_instance_id,workspace_id,object_id),
    FOREIGN KEY(local_instance_id,workspace_id) REFERENCES workspace_catalog(local_instance_id,workspace_id)
) STRICT;
CREATE TABLE local_resource_grants (
    token TEXT PRIMARY KEY,
    purpose TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    path_identity TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    local_instance_id TEXT,
    workspace_id TEXT,
    CHECK((local_instance_id IS NULL) = (workspace_id IS NULL)),
    FOREIGN KEY(local_instance_id,workspace_id) REFERENCES workspace_catalog(local_instance_id,workspace_id)
) STRICT;
CREATE TABLE app_operations (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    token TEXT NOT NULL REFERENCES local_resource_grants(token),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','canceled')),
    stage TEXT NOT NULL,
    planned_instance_id TEXT NOT NULL,
    checkpoint_json TEXT CHECK(checkpoint_json IS NULL OR json_valid(checkpoint_json)),
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    error_code TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE app_command_receipts (
    command_id TEXT PRIMARY KEY,
    payload_hash TEXT NOT NULL,
    operation_id TEXT NOT NULL REFERENCES app_operations(id)
) STRICT;
PRAGMA application_id = 1347112257;
PRAGMA user_version = 1;
