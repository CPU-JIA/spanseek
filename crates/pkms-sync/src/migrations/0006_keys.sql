CREATE TABLE sync_device_encryption_keys (
    device_id TEXT PRIMARY KEY REFERENCES sync_devices(device_id),
    public_key TEXT NOT NULL UNIQUE CHECK(length(public_key) = 64),
    registered_at TEXT NOT NULL
) STRICT;
CREATE TABLE sync_workspace_keys (
    workspace_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 1),
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES sync_devices(device_id),
    retired_at TEXT,
    PRIMARY KEY(workspace_id, epoch)
) STRICT;
CREATE TABLE sync_key_envelopes (
    workspace_id TEXT NOT NULL,
    epoch INTEGER NOT NULL,
    device_id TEXT NOT NULL REFERENCES sync_devices(device_id),
    ephemeral_public_key TEXT NOT NULL CHECK(length(ephemeral_public_key) = 64),
    nonce TEXT NOT NULL CHECK(length(nonce) = 24),
    ciphertext BLOB NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(workspace_id, epoch, device_id),
    FOREIGN KEY(workspace_id, epoch) REFERENCES sync_workspace_keys(workspace_id, epoch) ON DELETE CASCADE
) STRICT;
PRAGMA user_version = 6;