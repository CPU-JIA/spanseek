CREATE TABLE sync_devices (
    device_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    public_key TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN ('active','revoked')),
    paired_at TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at TEXT,
    revoked_by TEXT,
    revoked_reason TEXT,
    CHECK((status = 'revoked') = (revoked_at IS NOT NULL))
) STRICT;
CREATE TABLE sync_pairing_invites (
    invite_id TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES sync_devices(device_id),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    consumed_by TEXT REFERENCES sync_devices(device_id)
) STRICT;
CREATE TABLE sync_device_audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    action TEXT NOT NULL,
    actor_device_id TEXT,
    target_device_id TEXT,
    detail_json TEXT NOT NULL CHECK(json_valid(detail_json))
) STRICT;
CREATE TABLE sync_request_nonces (
    device_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY(device_id, nonce)
) STRICT;
PRAGMA application_id = 1347112258;
PRAGMA user_version = 1;