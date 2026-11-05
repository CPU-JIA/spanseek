//! Device encryption-key registration and workspace-key epoch rotation.

use chrono::{SecondsFormat, Utc};
use rusqlite::{OptionalExtension, params};

use crate::authority::{audit_on, require_owner};
use crate::crypto::{canonical, hex_decode, hex_encode, sha256_array};
use crate::keys::{KeyEnvelope, parse_encryption_public_key};
use crate::{DeviceEncryptionKey, SignedRequest, SyncAccess, SyncAuthority, SyncError};

const MAX_WORKSPACE_ID_CHARS: usize = 200;

impl SyncAuthority {
    pub fn register_encryption_key(
        &mut self,
        request: &SignedRequest,
        encryption_public_key: &str,
        now_unix: i64,
    ) -> Result<(), SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        let key = parse_encryption_public_key(encryption_public_key)?;
        let normalized = hex_encode(&key);
        if request.request_hash != device_encryption_key_request_hash(&normalized) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request_without_role_check(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let owner: Option<String> = self
            .connection
            .query_row(
                "SELECT device_id FROM sync_device_encryption_keys WHERE public_key=?1",
                [&normalized],
                |row| row.get(0),
            )
            .optional()?;
        if owner.as_deref().is_some_and(|id| id != device.device_id) {
            return Err(SyncError::InvalidEncryptionKey);
        }
        self.connection.execute(
            "INSERT INTO sync_device_encryption_keys(device_id,public_key,registered_at) VALUES(?1,?2,?3) ON CONFLICT(device_id) DO UPDATE SET public_key=excluded.public_key,registered_at=excluded.registered_at",
            params![
                device.device_id,
                normalized,
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
            ],
        )?;
        Ok(())
    }

    /// Creates epoch 1 or rotates to `new_epoch = current + 1`. The client has
    /// already wrapped the new workspace key for every active device; the
    /// server only stores metadata and envelopes.
    pub fn rotate_workspace_key(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        new_epoch: i64,
        envelopes: &[(String, KeyEnvelope)],
        now_unix: i64,
    ) -> Result<i64, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_workspace_id(workspace_id)?;
        if new_epoch < 1 {
            return Err(SyncError::InvalidKeyEnvelope);
        }
        if request.request_hash
            != workspace_key_rotation_request_hash(workspace_id, new_epoch, envelopes)
        {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        require_owner(&device)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let current = self.current_key_epoch(workspace_id)?;
        let expected_epoch = current.unwrap_or(0) + 1;
        if new_epoch != expected_epoch {
            return Err(SyncError::KeyEpochMismatch);
        }
        let active = self.active_devices_with_encryption_keys()?;
        if active.len() != envelopes.len() {
            return Err(SyncError::MissingEncryptionKey);
        }
        let mut seen = std::collections::BTreeSet::new();
        for (device_id, envelope) in envelopes {
            if !seen.insert(device_id.as_str()) {
                return Err(SyncError::InvalidKeyEnvelope);
            }
            if !active.iter().any(|active_id| active_id == device_id) {
                return Err(SyncError::InvalidKeyEnvelope);
            }
            if envelope.ephemeral_public_key == [0u8; 32] || envelope.ciphertext.len() != 48 {
                return Err(SyncError::InvalidKeyEnvelope);
            }
        }
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let tx = self.connection.transaction()?;
        if let Some(previous) = current {
            tx.execute(
                "UPDATE sync_workspace_keys SET retired_at=?1 WHERE workspace_id=?2 AND epoch=?3 AND retired_at IS NULL",
                params![created_at, workspace_id, previous],
            )?;
        }
        let inserted = tx.execute(
            "INSERT INTO sync_workspace_keys(workspace_id,epoch,created_at,created_by) VALUES(?1,?2,?3,?4)",
            params![workspace_id, new_epoch, created_at, device.device_id],
        );
        if let Err(rusqlite::Error::SqliteFailure(error, _)) = &inserted
            && error.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return Err(SyncError::KeyEpochMismatch);
        }
        inserted?;
        for (device_id, envelope) in envelopes {
            tx.execute(
                "INSERT INTO sync_key_envelopes(workspace_id,epoch,device_id,ephemeral_public_key,nonce,ciphertext,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    workspace_id,
                    new_epoch,
                    device_id,
                    hex_encode(&envelope.ephemeral_public_key),
                    hex_encode(&envelope.nonce),
                    envelope.ciphertext,
                    created_at
                ],
            )?;
        }
        audit_on(
            &tx,
            "workspace_key_rotated",
            Some(&device.device_id),
            None,
            serde_json::json!({
                "workspace_id": workspace_id,
                "epoch": new_epoch,
                "device_count": envelopes.len()
            }),
        )?;
        tx.commit()?;
        Ok(new_epoch)
    }

    /// Grants an existing epoch key envelope to an active device that joined
    /// after that epoch was created. The caller must already hold the key and
    /// produce the envelope locally; the authority never sees plaintext keys.
    pub fn grant_workspace_key(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        epoch: i64,
        target_device_id: &str,
        envelope: &KeyEnvelope,
        now_unix: i64,
    ) -> Result<(), SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_workspace_id(workspace_id)?;
        if epoch < 1
            || request.request_hash
                != grant_workspace_key_request_hash(workspace_id, epoch, target_device_id, envelope)
        {
            return Err(SyncError::InvalidRequestHash);
        }
        let actor = self.verify_device_request(request, now_unix)?;
        require_owner(&actor)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&actor.device_id, now_unix)?;
        let active: Option<String> = self
            .connection
            .query_row(
                "SELECT status FROM sync_devices WHERE device_id=?1",
                [target_device_id],
                |row| row.get(0),
            )
            .optional()?;
        if active.as_deref() != Some("active") {
            return Err(SyncError::DeviceInactive);
        }
        let key_registered: i64 = self.connection.query_row(
            "SELECT count(*) FROM sync_device_encryption_keys WHERE device_id=?1",
            [target_device_id],
            |row| row.get(0),
        )?;
        if key_registered == 0 {
            return Err(SyncError::MissingEncryptionKey);
        }
        if envelope.ephemeral_public_key == [0u8; 32] || envelope.ciphertext.len() != 48 {
            return Err(SyncError::InvalidKeyEnvelope);
        }
        let epoch_exists: i64 = self.connection.query_row(
            "SELECT count(*) FROM sync_workspace_keys WHERE workspace_id=?1 AND epoch=?2",
            params![workspace_id, epoch],
            |row| row.get(0),
        )?;
        if epoch_exists == 0 {
            return Err(SyncError::KeyEpochMismatch);
        }
        self.connection.execute(
            "INSERT INTO sync_key_envelopes(workspace_id,epoch,device_id,ephemeral_public_key,nonce,ciphertext,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(workspace_id,epoch,device_id) DO UPDATE SET ephemeral_public_key=excluded.ephemeral_public_key,nonce=excluded.nonce,ciphertext=excluded.ciphertext,created_at=excluded.created_at",
            params![
                workspace_id,
                epoch,
                target_device_id,
                hex_encode(&envelope.ephemeral_public_key),
                hex_encode(&envelope.nonce),
                envelope.ciphertext,
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
            ],
        )?;
        audit_on(
            &self.connection,
            "workspace_key_granted",
            Some(&actor.device_id),
            Some(target_device_id),
            serde_json::json!({"workspace_id": workspace_id, "epoch": epoch}),
        )?;
        Ok(())
    }

    pub fn fetch_key_envelope(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        epoch: i64,
        now_unix: i64,
    ) -> Result<KeyEnvelope, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_workspace_id(workspace_id)?;
        if epoch < 1 || request.request_hash != key_envelope_request_hash(workspace_id, epoch) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let row: Option<(String, String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT ephemeral_public_key,nonce,ciphertext FROM sync_key_envelopes WHERE workspace_id=?1 AND epoch=?2 AND device_id=?3",
                params![workspace_id, epoch, device.device_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (ephemeral, nonce, ciphertext) = row.ok_or(SyncError::KeyNotFound)?;
        Ok(KeyEnvelope {
            ephemeral_public_key: parse_32(&ephemeral)?,
            nonce: parse_12(&nonce)?,
            ciphertext,
        })
    }

    pub fn current_key_epoch(&self, workspace_id: &str) -> Result<Option<i64>, SyncError> {
        self.connection
            .query_row(
                "SELECT MAX(epoch) FROM sync_workspace_keys WHERE workspace_id=?1",
                [workspace_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(SyncError::from)
    }

    pub fn list_active_encryption_keys(&self) -> Result<Vec<DeviceEncryptionKey>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT d.device_id,k.public_key FROM sync_devices d JOIN sync_device_encryption_keys k ON k.device_id=d.device_id WHERE d.status='active' ORDER BY d.device_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(DeviceEncryptionKey {
                    device_id: row.get(0)?,
                    public_key: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn active_devices_with_encryption_keys(&self) -> Result<Vec<String>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT d.device_id,k.public_key FROM sync_devices d LEFT JOIN sync_device_encryption_keys k ON k.device_id=d.device_id WHERE d.status='active' ORDER BY d.device_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.iter().any(|(_, key)| key.is_none()) {
            return Err(SyncError::MissingEncryptionKey);
        }
        Ok(rows.into_iter().map(|(device, _)| device).collect())
    }
}

fn validate_workspace_id(workspace_id: &str) -> Result<(), SyncError> {
    let count = workspace_id.chars().count();
    if count == 0
        || count > MAX_WORKSPACE_ID_CHARS
        || workspace_id.trim().is_empty()
        || workspace_id.chars().any(char::is_control)
    {
        return Err(SyncError::InvalidBlob("invalid workspace id".into()));
    }
    Ok(())
}

fn parse_32(value: &str) -> Result<[u8; 32], SyncError> {
    let bytes = hex_decode(value).ok_or(SyncError::InvalidKeyEnvelope)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| SyncError::InvalidKeyEnvelope)
}

fn parse_12(value: &str) -> Result<[u8; 12], SyncError> {
    let bytes = hex_decode(value).ok_or(SyncError::InvalidKeyEnvelope)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| SyncError::InvalidKeyEnvelope)
}

pub fn device_encryption_key_request_hash(public_key_hex: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-device-encryption-v1",
        public_key_hex.as_bytes(),
    ]))
}

pub fn workspace_key_rotation_request_hash(
    workspace_id: &str,
    epoch: i64,
    envelopes: &[(String, KeyEnvelope)],
) -> [u8; 32] {
    let mut parts: Vec<Vec<u8>> = vec![
        b"spanseek-sync-wsk-rotate-v1".to_vec(),
        workspace_id.as_bytes().to_vec(),
        epoch.to_be_bytes().to_vec(),
        (envelopes.len() as u64).to_be_bytes().to_vec(),
    ];
    for (device_id, envelope) in envelopes {
        parts.push(device_id.as_bytes().to_vec());
        parts.push(envelope.ephemeral_public_key.to_vec());
        parts.push(envelope.nonce.to_vec());
        parts.push(envelope.ciphertext.clone());
    }
    let refs = parts.iter().map(Vec::as_slice).collect::<Vec<_>>();
    sha256_array(&canonical(&refs))
}

pub fn key_envelope_request_hash(workspace_id: &str, epoch: i64) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-key-envelope-v1",
        workspace_id.as_bytes(),
        &epoch.to_be_bytes(),
    ]))
}
pub fn grant_workspace_key_request_hash(
    workspace_id: &str,
    epoch: i64,
    target_device_id: &str,
    envelope: &KeyEnvelope,
) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-grant-key-v1",
        workspace_id.as_bytes(),
        &epoch.to_be_bytes(),
        target_device_id.as_bytes(),
        &envelope.ephemeral_public_key,
        &envelope.nonce,
        &envelope.ciphertext,
    ]))
}
