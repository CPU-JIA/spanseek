CREATE TABLE sync_members (
    member_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('owner','editor','viewer')),
    status TEXT NOT NULL CHECK(status IN ('active','removed')),
    created_at TEXT NOT NULL,
    created_by TEXT REFERENCES sync_devices(device_id),
    removed_at TEXT,
    removed_by TEXT REFERENCES sync_devices(device_id),
    removed_reason TEXT,
    CHECK((status = 'removed') = (removed_at IS NOT NULL)),
    CHECK((removed_at IS NULL) = (removed_by IS NULL)),
    CHECK((removed_at IS NULL) = (removed_reason IS NULL))
) STRICT;
ALTER TABLE sync_devices ADD COLUMN member_id TEXT REFERENCES sync_members(member_id);
ALTER TABLE sync_pairing_invites ADD COLUMN invited_member_id TEXT REFERENCES sync_members(member_id);
ALTER TABLE sync_pairing_invites ADD COLUMN invited_member_name TEXT;
INSERT INTO sync_members(member_id,display_name,role,status,created_at,created_by)
SELECT 'legacy-' || device_id,display_name,role,'active',paired_at,device_id
FROM sync_devices ORDER BY paired_at,device_id;
UPDATE sync_devices SET member_id='legacy-' || device_id;
CREATE INDEX sync_devices_member ON sync_devices(member_id, status);
CREATE INDEX sync_members_status_role ON sync_members(status, role);
PRAGMA user_version = 9;