ALTER TABLE sync_devices ADD COLUMN role TEXT NOT NULL DEFAULT 'editor'
    CHECK(role IN ('owner','editor','viewer'));
ALTER TABLE sync_pairing_invites ADD COLUMN invited_role TEXT NOT NULL DEFAULT 'editor'
    CHECK(invited_role IN ('owner','editor','viewer'));
UPDATE sync_devices SET role='owner'
WHERE device_id=(SELECT device_id FROM sync_devices ORDER BY paired_at,device_id LIMIT 1);
PRAGMA user_version = 8;
