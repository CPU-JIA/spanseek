CREATE TABLE sync_reading_positions (
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    content_version INTEGER NOT NULL CHECK(content_version >= 0),
    locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
    updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
    PRIMARY KEY(workspace_id, object_id, device_id)
) STRICT;
CREATE INDEX sync_reading_positions_updated
    ON sync_reading_positions(workspace_id, object_id, updated_at_ms DESC);
CREATE TABLE sync_draft_handoffs (
    handoff_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    base_version INTEGER NOT NULL CHECK(base_version >= 0),
    base_revision_id TEXT,
    title TEXT NOT NULL,
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) > 0),
    payload_digest TEXT NOT NULL,
    key_epoch INTEGER NOT NULL CHECK(key_epoch >= 1),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
    adopted_at_ms INTEGER,
    adopted_by TEXT REFERENCES sync_devices(device_id),
    CHECK(adopted_at_ms IS NULL OR adopted_at_ms >= created_at_ms),
    CHECK((adopted_at_ms IS NULL) = (adopted_by IS NULL))
) STRICT;
CREATE INDEX sync_draft_handoffs_scope
    ON sync_draft_handoffs(workspace_id, object_id, updated_at_ms DESC);
PRAGMA user_version = 7;