//! Cross-device sync profile, vault access and note-engine wiring.
//!
//! The profile file contains only endpoint and public identifiers. The device
//! seed and workspace key live in the OS credential vault; neither is written
//! to app-state SQLite, the knowledge library, logs or diagnostics.

use std::collections::HashMap;

use pkms_core::{
    CreateVoiceCapture, TagSyncApplyStatus, TagSyncKind, TagSyncPage, TagSyncPageRequest,
    TagSyncPendingReason, TagSyncSnapshot, VoiceSourceKind,
};
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, DeviceStatus, MemberRecord, MemberRole, SealedData, TrashEntry,
    WorkspaceKey, hex_decode, hex_encode, owner_proof_message, parse_encryption_public_key,
    sha256_hex, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use pkms_sync_engine::{
    ConflictTarget, PushOutcome, SyncEngine, TagPushOutcome, TagSyncDecision, TagWireOwners,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::inference_credentials::{self as vault, Secret};
use crate::{AppError, AppService, Result, WorkspaceIdentity};

const PROFILE_FILE: &str = "sync-profile.json";
const DEVICE_TARGET_PREFIX: &str = "PKMS:sync-device:v1:";
const WSK_TARGET_PREFIX: &str = "PKMS:sync-wsk:v1:";

fn state_file_name(identity: &WorkspaceIdentity, profile: &SyncProfile) -> Result<String> {
    let binding = serde_json::to_vec(&(
        "spanseek-sync-state-v2",
        &identity.local_instance_id,
        &identity.workspace_id,
        &profile.endpoint,
        &profile.device_id,
        &profile.local_id,
        &profile.signing_public_key,
        &profile.encryption_public_key,
    ))?;
    Ok(format!("sync-state-v2-{}.sqlite3", sha256_hex(&binding)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SyncProfile {
    pub local_id: String,
    pub endpoint: String,
    pub device_id: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub signing_public_key: String,
    pub encryption_public_key: String,
    pub workspace_id: Option<String>,
    #[ts(type = "number")]
    pub epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceSyncImport {
    pub imported: u32,
    pub skipped: u32,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncOwnerProof {
    pub public_key: String,
    pub display_name: String,
    pub proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncStatus {
    pub configured: bool,
    pub endpoint: Option<String>,
    pub device_id: Option<String>,
    pub workspace_id: Option<String>,
    pub healthy: bool,
    pub revoked: bool,
    #[ts(type = "number")]
    pub schema_version: Option<i64>,
    #[ts(type = "number")]
    pub protocol_version: Option<i64>,
}

/// A local sync-status snapshot plus an optional network probe. The caller
/// must drop the application-service lock before calling `resolve()` so an
/// unreachable sync endpoint cannot stall unrelated desktop commands.
pub struct SyncStatusProbe {
    status: SyncStatus,
    client: Option<SyncClient>,
    expected_device_id: Option<String>,
}

impl SyncStatusProbe {
    pub fn resolve(mut self) -> SyncStatus {
        if let Some(client) = self.client {
            if let Ok(health) = client.health() {
                self.status.healthy = true;
                self.status.schema_version = Some(health.schema_version);
                self.status.protocol_version = Some(health.protocol_version);
            }
            if self.expected_device_id.is_some()
                && let Err(pkms_sync_client::ClientError::Api { code, .. }) = client.devices()
            {
                self.status.revoked = code == "device_revoked" || code == "member_removed";
            }
        }
        self.status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SyncMemberRole {
    Owner,
    Editor,
    Viewer,
}

impl From<MemberRole> for SyncMemberRole {
    fn from(value: MemberRole) -> Self {
        match value {
            MemberRole::Owner => Self::Owner,
            MemberRole::Editor => Self::Editor,
            MemberRole::Viewer => Self::Viewer,
        }
    }
}

impl From<SyncMemberRole> for MemberRole {
    fn from(value: SyncMemberRole) -> Self {
        match value {
            SyncMemberRole::Owner => Self::Owner,
            SyncMemberRole::Editor => Self::Editor,
            SyncMemberRole::Viewer => Self::Viewer,
        }
    }
}

pub struct SyncWatchProbe {
    engine: SyncEngine,
}

impl SyncWatchProbe {
    pub fn resolve(self, timeout_ms: u64) -> Result<SyncWatchResult> {
        let batch = self
            .engine
            .client()
            .wait_for_events(
                self.engine.workspace_id(),
                self.engine.cursor(),
                1,
                timeout_ms,
            )
            .map_err(sync_error)?;
        Ok(SyncWatchResult {
            configured: true,
            changed: !batch.ops.is_empty() || !batch.conflicts.is_empty(),
            conflict_count: batch.conflicts.len(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncMember {
    pub member_id: String,
    pub display_name: String,
    pub role: SyncMemberRole,
    pub status: String,
    pub created_at: String,
    pub removed_at: Option<String>,
    #[ts(type = "number")]
    pub device_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SyncAuditEvent {
    #[ts(type = "number")]
    pub seq: i64,
    pub at: String,
    pub action: String,
    pub actor_device_id: Option<String>,
    pub target_device_id: Option<String>,
    #[ts(type = "unknown")]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncMemberRemoval {
    pub member: SyncMember,
    pub key_rotated: bool,
    pub key_epoch: Option<u32>,
    pub rotation_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncDevice {
    pub device_id: String,
    pub display_name: String,
    pub fingerprint: String,
    pub member_id: String,
    pub member_name: String,
    pub role: SyncMemberRole,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncAutoPushResult {
    pub configured: bool,
    pub pushed: bool,
    pub conflict: bool,
    #[ts(type = "number")]
    pub version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncWatchResult {
    pub configured: bool,
    pub changed: bool,
    #[ts(type = "number")]
    pub conflict_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncPushSummary {
    #[ts(type = "number")]
    pub version: i64,
    pub unchanged: bool,
    #[ts(type = "number")]
    pub conflict_remote_version: Option<i64>,
    pub conflict_remote_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SyncTagDecision {
    pub kind: TagSyncKind,
    pub entity_id: String,
    #[ts(type = "number")]
    pub remote_version: i64,
    pub local_fingerprint: Option<String>,
    pub local_impact_token: Option<String>,
}

impl From<SyncTagDecision> for TagSyncDecision {
    fn from(value: SyncTagDecision) -> Self {
        Self {
            kind: value.kind,
            entity_id: value.entity_id,
            remote_version: value.remote_version,
            local_fingerprint: value.local_fingerprint,
            local_impact_token: value.local_impact_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncTagChange {
    pub kind: TagSyncKind,
    pub entity_id: String,
    pub sync_object_id: String,
    pub status: TagSyncApplyStatus,
    #[ts(type = "number")]
    pub remote_version: i64,
    pub pending_reason: Option<TagSyncPendingReason>,
    pub changed_objects: Vec<String>,
}

impl From<pkms_sync_engine::TagApplySummary> for SyncTagChange {
    fn from(value: pkms_sync_engine::TagApplySummary) -> Self {
        Self {
            kind: value.kind,
            entity_id: value.entity_id,
            sync_object_id: value.sync_object_id,
            status: value.status,
            remote_version: value.remote_version,
            pending_reason: value.pending_reason,
            changed_objects: value.changed_objects,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncTagPushBatch {
    pub configured: bool,
    pub complete: bool,
    pub published: u32,
    pub received: u32,
    pub unchanged: u32,
    pub pending: u32,
    pub dirty: u32,
    pub unavailable: u32,
    pub changed_objects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncPushBatch {
    pub items: Vec<SyncPushSummary>,
    pub tags: SyncTagPushBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncTrashItem {
    pub object_id: String,
    pub title: Option<String>,
    #[ts(type = "number")]
    pub version: i64,
    pub purged: bool,
    pub deleted_at: Option<String>,
    pub deleted_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncConflictTarget {
    pub conflict_id: String,
    pub object_id: String,
    #[ts(type = "number")]
    pub remote_version: i64,
    pub remote_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncLocalConflictView {
    pub object_id: String,
    pub local_title: String,
    pub local_body: String,
    pub remote_title: String,
    pub remote_body: String,
    #[ts(type = "number")]
    pub remote_version: i64,
    pub remote_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncServerConflictView {
    pub target: SyncConflictTarget,
    pub local_title: String,
    pub local_body: String,
    pub local_trashed: bool,
    pub local_available: bool,
    pub remote_title: String,
    pub remote_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncConfigConflictView {
    pub object_id: String,
    pub kind: String,
    pub local_title: String,
    pub remote_title: String,
    #[ts(type = "number")]
    pub remote_version: i64,
    pub local_available: bool,
    pub remote_available: bool,
    pub local_tombstone: bool,
    pub remote_tombstone: bool,
    pub local_state_token: String,
}

fn config_conflict_view(
    engine: &SyncEngine,
    library: &pkms_core::Library,
    conflict: pkms_sync_engine::ConfigConflict,
) -> Result<SyncConfigConflictView> {
    let local = engine
        .local_config_state(library, &conflict.object_id)
        .map_err(sync_error)?;
    // For these two closed variants an unavailable remote payload is recorded
    // only by a real Delete/Purge. Unknown payload formats are errors, not rows.
    let remote_tombstone =
        !conflict.remote_available && matches!(conflict.kind.as_str(), "view" | "canvas");
    Ok(SyncConfigConflictView {
        object_id: conflict.object_id,
        kind: conflict.kind,
        local_title: local.title,
        remote_title: conflict.remote_title,
        remote_version: conflict.remote_version,
        local_available: local.available,
        remote_available: conflict.remote_available,
        local_tombstone: local.tombstone,
        remote_tombstone,
        local_state_token: local.local_state_token,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SyncConfigResolutionMode {
    KeepLocal,
    KeepRemote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncConfigResolution {
    pub object_id: String,
    pub mode: SyncConfigResolutionMode,
    #[ts(type = "number")]
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncPullView {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub trashed: Vec<String>,
    pub restored: Vec<String>,
    pub conflicts: Vec<SyncLocalConflictView>,
    pub config_conflicts: Vec<SyncConfigConflictView>,
    pub server_conflicts: Vec<SyncServerConflictView>,
    pub tag_results: Vec<SyncTagChange>,
    pub tag_pending: u32,
    #[ts(type = "number")]
    pub next_cursor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SyncInviteView {
    pub invite_id: String,
    pub code: String,
    pub member_id: Option<String>,
    pub member_name: Option<String>,
    pub role: SyncMemberRole,
    pub expires_at: String,
}
fn sync_error(error: impl std::fmt::Display) -> AppError {
    AppError::Sync(error.to_string())
}

fn engine_error(error: pkms_sync_engine::EngineError) -> AppError {
    match error {
        pkms_sync_engine::EngineError::Core(error) => AppError::Core(error),
        other => sync_error(other),
    }
}

fn device_target(local_id: &str) -> String {
    format!("{DEVICE_TARGET_PREFIX}{local_id}")
}

fn wsk_target(workspace_id: &str, epoch: i64) -> String {
    format!("{WSK_TARGET_PREFIX}{workspace_id}:{epoch}")
}

fn validate_endpoint(endpoint: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|_| AppError::InvalidInput("sync_invalid_endpoint"))?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https"
            || (url.scheme() == "http"
                && url
                    .host_str()
                    .is_some_and(pkms_inference::remote_embeddings::loopback)))
    {
        return Err(AppError::InvalidInput("sync_invalid_endpoint"));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl AppService {
    pub fn sync_profile(&self) -> Result<Option<SyncProfile>> {
        let path = self.app_directory.join(PROFILE_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(path)?;
        let profile: SyncProfile = serde_json::from_slice(&bytes)?;
        Ok(Some(profile))
    }

    fn write_sync_profile(&self, profile: &SyncProfile) -> Result<()> {
        let path = self.app_directory.join(PROFILE_FILE);
        let temp = self.app_directory.join(format!("{PROFILE_FILE}.tmp"));
        std::fs::write(&temp, serde_json::to_vec_pretty(profile)?)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(temp, path)?;
        Ok(())
    }

    fn sync_seed(&self, profile: &SyncProfile) -> Result<[u8; 32]> {
        let target = device_target(&profile.local_id);
        let secret = vault::load_at(&target)
            .map_err(sync_error)?
            .ok_or_else(|| AppError::Sync("sync_device_seed_missing".into()))?;
        let bytes = hex_decode(secret.expose_secret())
            .ok_or_else(|| AppError::Sync("sync_device_seed_invalid".into()))?;
        bytes
            .try_into()
            .map_err(|_| AppError::Sync("sync_device_seed_invalid".into()))
    }

    fn sync_device_key(&self, profile: &SyncProfile) -> Result<DeviceKeyPair> {
        DeviceKeyPair::from_seed(self.sync_seed(profile)?).map_err(sync_error)
    }

    fn sync_client(&self, profile: &SyncProfile) -> Result<SyncClient> {
        SyncClient::new(
            &profile.endpoint,
            profile.device_id.clone().unwrap_or_default(),
            self.sync_device_key(profile)?,
        )
        .map_err(sync_error)
    }

    /// Pull voice blobs referenced by synced note markers and create local
    /// captures. Blobs are decrypted with the workspace key; plaintext never
    /// touches the authority. Re-running is safe because the command id is
    /// derived from the content hash.
    pub fn voice_sync_import(&mut self, identity: &WorkspaceIdentity) -> Result<VoiceSyncImport> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::InvalidInput("sync_not_configured"))?;
        let workspace_id = profile
            .workspace_id
            .clone()
            .ok_or_else(|| AppError::InvalidInput("sync_workspace_missing"))?;
        let client = self.sync_client(&profile)?;
        let workspace_key =
            self.sync_workspace_key_for_epoch(&profile, &workspace_id, profile.epoch)?;
        let library = self.library_mut(identity)?;
        let notes = library.list_notes()?;
        let mut imported = 0;
        let mut skipped = 0;
        let mut failed = Vec::new();
        for note in notes {
            if note.trashed {
                continue;
            }
            let plain = serde_json::to_string(&note.document).unwrap_or_default();
            let Some(marker) = parse_voice_marker(&plain) else {
                skipped += 1;
                continue;
            };
            library.record_voice_sync_task(&marker.blob_hash, "syncing", None, None, None)?;
            let result = (|| -> Result<String> {
                let status = client.blob_status(&marker.blob_hash).map_err(sync_error)?;
                if status.size <= 0 || status.chunk_count <= 0 {
                    return Err(AppError::InvalidInput("voice_sync_blob_empty"));
                }
                let mut sealed_wire = Vec::with_capacity(status.size as usize);
                for index in 0..status.chunk_count as u32 {
                    sealed_wire.extend(
                        client
                            .blob_read(&marker.blob_hash, index)
                            .map_err(sync_error)?,
                    );
                }
                if sealed_wire.len() != status.size as usize || sealed_wire.len() <= 12 {
                    return Err(AppError::InvalidInput("voice_sync_blob_invalid"));
                }
                let mut nonce = [0u8; 12];
                nonce.copy_from_slice(&sealed_wire[..12]);
                let aad = voice_blob_aad(&workspace_id, &marker.plaintext_hash, profile.epoch);
                let audio = workspace_key
                    .decrypt(
                        &SealedData {
                            nonce,
                            ciphertext: sealed_wire[12..].to_vec(),
                        },
                        &aad,
                    )
                    .map_err(sync_error)?;
                if sha256_hex(&audio) != marker.plaintext_hash {
                    return Err(AppError::InvalidInput("voice_sync_hash_mismatch"));
                }
                let hash_bytes = hex_decode(&marker.blob_hash)
                    .ok_or_else(|| AppError::InvalidInput("voice_sync_hash_invalid"))?;
                if hash_bytes.len() < 16 {
                    return Err(AppError::InvalidInput("voice_sync_hash_invalid"));
                }
                let mut command_bytes = [0u8; 16];
                command_bytes.copy_from_slice(&hash_bytes[..16]);
                let command_id = Uuid::from_bytes(command_bytes).to_string();
                let capture = library.create_voice_capture(
                    &command_id,
                    CreateVoiceCapture {
                        title: note.title.clone(),
                        duration_ms: marker.duration_ms,
                        source_kind: VoiceSourceKind::Import,
                        device_label: Some("手机同步".into()),
                        language: None,
                    },
                    &audio,
                )?;
                Ok(capture.id)
            })();
            match result {
                Ok(capture_id) => {
                    library.record_voice_sync_task(
                        &marker.blob_hash,
                        "done",
                        Some(&capture_id),
                        None,
                        None,
                    )?;
                    imported += 1;
                }
                Err(error) => {
                    library.record_voice_sync_task(
                        &marker.blob_hash,
                        "failed",
                        None,
                        None,
                        Some("voice_sync_failed"),
                    )?;
                    failed.push(format!("{}: {}", note.id, error));
                }
            }
        }
        Ok(VoiceSyncImport {
            imported,
            skipped,
            failed,
        })
    }

    fn sync_workspace_key_for_epoch(
        &self,
        profile: &SyncProfile,
        workspace_id: &str,
        epoch: i64,
    ) -> Result<WorkspaceKey> {
        let target = wsk_target(workspace_id, epoch);
        if let Some(secret) = vault::load_at(&target).map_err(sync_error)? {
            let bytes = hex_decode(secret.expose_secret())
                .ok_or_else(|| AppError::Sync("sync_workspace_key_invalid".into()))?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| AppError::Sync("sync_workspace_key_invalid".into()))?;
            return Ok(WorkspaceKey::from_bytes(bytes));
        }
        if epoch < 1 {
            return Err(AppError::Sync("sync_workspace_epoch_missing".into()));
        }
        let device_id = profile
            .device_id
            .clone()
            .ok_or_else(|| AppError::Sync("sync_device_not_paired".into()))?;
        let client = self.sync_client(profile)?;
        let envelope = client
            .fetch_key_envelope(workspace_id, epoch)
            .map_err(sync_error)?;
        let seed = self.sync_seed(profile)?;
        let workspace_key = DeviceKeyPair::from_seed(seed)
            .map_err(sync_error)?
            .unwrap_workspace_key(
                &envelope,
                &workspace_key_aad(workspace_id, epoch, &device_id),
            )
            .map_err(sync_error)?;
        let secret = Secret::new(workspace_key.export_hex()).map_err(sync_error)?;
        vault::store_at(&target, &secret).map_err(sync_error)?;
        Ok(workspace_key)
    }

    pub fn sync_configure(&self, endpoint: &str, display_name: &str) -> Result<SyncProfile> {
        let endpoint = validate_endpoint(endpoint)?;
        let display_name = display_name.trim();
        if display_name.is_empty()
            || display_name.chars().count() > 80
            || display_name.chars().any(char::is_control)
        {
            return Err(AppError::InvalidInput("sync_invalid_display_name"));
        }
        let seed = DeviceKeyPair::generate_seed().map_err(sync_error)?;
        let key = DeviceKeyPair::from_seed(seed).map_err(sync_error)?;
        let local_id = Uuid::new_v4().to_string();
        let public_key = key.public_key_hex();
        let encryption_public_key = key.encryption_public_key_hex().map_err(sync_error)?;
        let secret = Secret::new(key.seed_hex()).map_err(sync_error)?;
        vault::store_at(&device_target(&local_id), &secret).map_err(sync_error)?;
        let profile = SyncProfile {
            local_id,
            endpoint,
            device_id: None,
            display_name: display_name.to_owned(),
            signing_public_key: public_key,
            encryption_public_key,
            workspace_id: None,
            epoch: 0,
        };
        self.write_sync_profile(&profile)?;
        Ok(profile)
    }

    pub fn sync_owner_proof(&self) -> Result<SyncOwnerProof> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let key = self.sync_device_key(&profile)?;
        let public_key = if profile.signing_public_key.is_empty() {
            key.public_key_hex()
        } else {
            profile.signing_public_key.clone()
        };
        let proof = key.sign(&owner_proof_message(&public_key, &profile.display_name));
        Ok(SyncOwnerProof {
            public_key,
            display_name: profile.display_name,
            proof: hex_encode(&proof),
        })
    }

    pub fn sync_claim_device(&self, device_id: &str) -> Result<SyncProfile> {
        uuid::Uuid::parse_str(device_id)
            .map_err(|_| AppError::InvalidInput("sync_invalid_device"))?;
        let mut profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let client = SyncClient::new(
            &profile.endpoint,
            device_id,
            self.sync_device_key(&profile)?,
        )
        .map_err(sync_error)?;
        client.devices().map_err(sync_error)?;
        profile.device_id = Some(device_id.to_owned());
        self.write_sync_profile(&profile)?;
        Ok(profile)
    }

    pub fn sync_pair(&self, invite_id: &str, code: &str) -> Result<SyncProfile> {
        let mut profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let key = self.sync_device_key(&profile)?;
        let encryption_public = profile.encryption_public_key.clone();
        let pairing_client = SyncClient::new(
            &profile.endpoint,
            profile.device_id.clone().unwrap_or_default(),
            key,
        )
        .map_err(sync_error)?;
        let device = pairing_client
            .pair_with_code(invite_id, code, &profile.display_name)
            .map_err(sync_error)?;
        SyncClient::new(
            &profile.endpoint,
            &device.device_id,
            self.sync_device_key(&profile)?,
        )
        .map_err(sync_error)?
        .register_encryption_key(&encryption_public)
        .map_err(sync_error)?;
        profile.device_id = Some(device.device_id);
        self.write_sync_profile(&profile)?;
        Ok(profile)
    }

    pub fn sync_create_invite(
        &self,
        member_name: &str,
        role: SyncMemberRole,
    ) -> Result<SyncInviteView> {
        let member_name = member_name.trim();
        if member_name.is_empty() {
            return Err(AppError::InvalidInput("sync_member_name_required"));
        }
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let invite = self
            .sync_client(&profile)?
            .create_member_pairing_invite(600, member_name, role.into())
            .map_err(sync_error)?;
        Ok(SyncInviteView {
            invite_id: invite.invite_id,
            code: invite.code,
            member_id: invite.member_id,
            member_name: invite.member_name,
            role: invite.role.into(),
            expires_at: invite.expires_at,
        })
    }

    pub fn sync_create_device_invite(
        &self,
        member_id: &str,
        role: SyncMemberRole,
    ) -> Result<SyncInviteView> {
        if member_id.trim().is_empty() {
            return Err(AppError::InvalidInput("sync_member_required"));
        }
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let invite = self
            .sync_client(&profile)?
            .create_device_pairing_invite(600, member_id, role.into())
            .map_err(sync_error)?;
        Ok(SyncInviteView {
            invite_id: invite.invite_id,
            code: invite.code,
            member_id: invite.member_id,
            member_name: invite.member_name,
            role: invite.role.into(),
            expires_at: invite.expires_at,
        })
    }

    pub fn sync_members(&self) -> Result<Vec<SyncMember>> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let members = self.sync_client(&profile)?.members().map_err(sync_error)?;
        Ok(members.into_iter().map(member_view).collect())
    }

    pub fn sync_audit(&self, limit: usize) -> Result<Vec<SyncAuditEvent>> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let entries = self
            .sync_client(&profile)?
            .audit_log(limit)
            .map_err(sync_error)?;
        Ok(entries.into_iter().map(audit_view).collect())
    }

    pub fn sync_set_member_role(
        &self,
        member_id: &str,
        role: SyncMemberRole,
    ) -> Result<SyncMember> {
        if member_id.trim().is_empty() {
            return Err(AppError::InvalidInput("sync_member_required"));
        }
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let member = self
            .sync_client(&profile)?
            .set_member_role(member_id, role.into())
            .map_err(sync_error)?;
        Ok(member_view(member))
    }

    pub fn sync_remove_member(&self, member_id: &str, reason: &str) -> Result<SyncMemberRemoval> {
        if member_id.trim().is_empty() {
            return Err(AppError::InvalidInput("sync_member_required"));
        }
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let member = self
            .sync_client(&profile)?
            .remove_member(member_id, reason)
            .map_err(sync_error)?;
        let member = member_view(member);
        let Some(workspace_id) = profile.workspace_id.clone() else {
            return Ok(SyncMemberRemoval {
                member,
                key_rotated: false,
                key_epoch: None,
                rotation_error: None,
            });
        };
        match self.sync_ensure_workspace_key(&workspace_id) {
            Ok(rotated) => match u32::try_from(rotated.epoch) {
                Ok(epoch) => Ok(SyncMemberRemoval {
                    member,
                    key_rotated: true,
                    key_epoch: Some(epoch),
                    rotation_error: None,
                }),
                Err(_) => Ok(SyncMemberRemoval {
                    member,
                    key_rotated: true,
                    key_epoch: None,
                    rotation_error: Some("workspace key epoch is outside the UI range".into()),
                }),
            },
            Err(error) => Ok(SyncMemberRemoval {
                member,
                key_rotated: false,
                key_epoch: None,
                rotation_error: Some(error.to_string()),
            }),
        }
    }

    pub fn sync_devices(&self) -> Result<Vec<SyncDevice>> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let devices = self.sync_client(&profile)?.devices().map_err(sync_error)?;
        Ok(devices.into_iter().map(device_view).collect())
    }

    pub fn sync_set_device_role(
        &self,
        target_device_id: &str,
        role: SyncMemberRole,
    ) -> Result<SyncDevice> {
        if target_device_id.trim().is_empty() {
            return Err(AppError::InvalidInput("sync_device_required"));
        }
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let device = self
            .sync_client(&profile)?
            .set_device_role(target_device_id, role.into())
            .map_err(sync_error)?;
        Ok(device_view(device))
    }
    pub fn sync_revoke_device(&self, target_device_id: &str, reason: &str) -> Result<SyncDevice> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let device = self
            .sync_client(&profile)?
            .revoke_device(target_device_id, reason)
            .map_err(sync_error)?;
        Ok(device_view(device))
    }

    pub fn sync_grant_workspace_key(
        &self,
        workspace_id: &str,
        target_device_id: &str,
    ) -> Result<usize> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        if profile.workspace_id.as_deref() != Some(workspace_id) {
            return Err(AppError::Sync("sync_workspace_not_attached".into()));
        }
        if profile.epoch < 1 {
            return Err(AppError::Sync("sync_workspace_epoch_missing".into()));
        }
        let client = self.sync_client(&profile)?;
        let target = client
            .device_encryption_keys()
            .map_err(sync_error)?
            .into_iter()
            .find(|entry| entry.device_id == target_device_id)
            .ok_or_else(|| AppError::Sync("sync_missing_device_key".into()))?;
        let public_key = parse_encryption_public_key(&target.public_key).map_err(sync_error)?;
        for epoch in 1..=profile.epoch {
            let workspace_key = self.sync_workspace_key_for_epoch(&profile, workspace_id, epoch)?;
            let aad = workspace_key_aad(workspace_id, epoch, target_device_id);
            let envelope =
                wrap_workspace_key(public_key, &workspace_key, &aad).map_err(sync_error)?;
            client
                .grant_workspace_key(workspace_id, epoch, target_device_id, &envelope)
                .map_err(sync_error)?;
        }
        Ok(profile.epoch as usize)
    }

    /// Reads only local sync configuration. The returned probe may be resolved
    /// after releasing the service mutex, keeping network health checks off the
    /// global command lock.
    pub fn sync_status_probe(&self) -> Result<SyncStatusProbe> {
        let Some(profile) = self.sync_profile()? else {
            return Ok(SyncStatusProbe {
                status: SyncStatus {
                    configured: false,
                    endpoint: None,
                    device_id: None,
                    workspace_id: None,
                    healthy: false,
                    revoked: false,
                    schema_version: None,
                    protocol_version: None,
                },
                client: None,
                expected_device_id: None,
            });
        };
        let status = SyncStatus {
            configured: true,
            endpoint: Some(profile.endpoint.clone()),
            device_id: profile.device_id.clone(),
            workspace_id: profile.workspace_id.clone(),
            healthy: false,
            revoked: false,
            schema_version: None,
            protocol_version: None,
        };
        let client = self.sync_client(&profile).ok();
        Ok(SyncStatusProbe {
            status,
            client,
            expected_device_id: profile.device_id,
        })
    }

    pub fn sync_status(&self) -> Result<SyncStatus> {
        Ok(self.sync_status_probe()?.resolve())
    }

    pub fn sync_attach_workspace(&self, workspace_id: &str) -> Result<SyncProfile> {
        uuid::Uuid::parse_str(workspace_id)
            .map_err(|_| AppError::InvalidInput("sync_invalid_workspace"))?;
        let mut profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        if profile.device_id.is_none() {
            return Err(AppError::Sync("sync_device_not_paired".into()));
        }
        let epoch = self
            .sync_client(&profile)?
            .current_key_epoch(workspace_id)
            .map_err(sync_error)?
            .ok_or_else(|| AppError::Sync("sync_workspace_key_missing".into()))?;
        profile.workspace_id = Some(workspace_id.to_owned());
        profile.epoch = epoch;
        self.write_sync_profile(&profile)?;
        Ok(profile)
    }

    /// Creates epoch 1 or rotates to the next epoch and grants every active
    /// device an envelope for the new workspace key. The key is persisted only
    /// in the OS credential vault.
    pub fn sync_ensure_workspace_key(&self, workspace_id: &str) -> Result<SyncProfile> {
        uuid::Uuid::parse_str(workspace_id)
            .map_err(|_| AppError::InvalidInput("sync_invalid_workspace"))?;
        let mut profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        profile
            .device_id
            .clone()
            .ok_or_else(|| AppError::Sync("sync_device_not_paired".into()))?;
        let client = self.sync_client(&profile)?;
        client
            .register_encryption_key(&profile.encryption_public_key)
            .map_err(sync_error)?;
        let devices = client.devices().map_err(sync_error)?;
        let active = devices
            .iter()
            .filter(|device| device.status == DeviceStatus::Active)
            .count();
        let keys = client.device_encryption_keys().map_err(sync_error)?;
        if keys.len() != active {
            return Err(AppError::Sync("sync_missing_device_key".into()));
        }
        let current = client.current_key_epoch(workspace_id).map_err(sync_error)?;
        let new_epoch = current.unwrap_or(0) + 1;
        let workspace_key = WorkspaceKey::generate().map_err(sync_error)?;
        let mut envelopes = Vec::with_capacity(keys.len());
        for entry in &keys {
            let public_key = parse_encryption_public_key(&entry.public_key).map_err(sync_error)?;
            let aad = workspace_key_aad(workspace_id, new_epoch, &entry.device_id);
            let envelope =
                wrap_workspace_key(public_key, &workspace_key, &aad).map_err(sync_error)?;
            envelopes.push((entry.device_id.clone(), envelope));
        }
        client
            .rotate_workspace_key(workspace_id, new_epoch, &envelopes)
            .map_err(sync_error)?;
        let target = wsk_target(workspace_id, new_epoch);
        let secret = Secret::new(workspace_key.export_hex()).map_err(sync_error)?;
        vault::store_at(&target, &secret).map_err(sync_error)?;
        profile.workspace_id = Some(workspace_id.to_owned());
        profile.epoch = new_epoch;
        self.write_sync_profile(&profile)?;
        Ok(profile)
    }

    fn sync_engine(&mut self, identity: &WorkspaceIdentity) -> Result<SyncEngine> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let workspace_id = profile
            .workspace_id
            .clone()
            .ok_or_else(|| AppError::Sync("sync_workspace_not_attached".into()))?;
        if self.library(identity)?.info()?.workspace_id != workspace_id {
            return Err(AppError::InstanceMismatch);
        }
        let device_id = profile
            .device_id
            .clone()
            .ok_or_else(|| AppError::Sync("sync_device_not_paired".into()))?;
        let client = SyncClient::new(
            &profile.endpoint,
            device_id,
            self.sync_device_key(&profile)?,
        )
        .map_err(sync_error)?;
        if profile.epoch < 1 {
            return Err(AppError::Sync("sync_workspace_epoch_missing".into()));
        }
        let mut workspace_keys = HashMap::new();
        for epoch in 1..=profile.epoch {
            workspace_keys.insert(
                epoch,
                self.sync_workspace_key_for_epoch(&profile, &workspace_id, epoch)?,
            );
        }
        // The legacy workspace-only file has no instance/binding provenance.
        // Retain it, but never apply its cursor to a restored library or another
        // endpoint/device. Seed the new scope from matching real remote content.
        let state_path = self
            .app_directory
            .join(state_file_name(identity, &profile)?);
        let mut engine = SyncEngine::with_state_and_keys(
            client,
            workspace_keys,
            workspace_id,
            profile.epoch,
            state_path,
        )
        .map_err(engine_error)?;
        if engine.needs_initial_seed() {
            engine
                .initialize_from_snapshot(self.library_mut(identity)?)
                .map_err(engine_error)?;
        }
        Ok(engine)
    }

    pub fn sync_watch_probe(
        &mut self,
        identity: &WorkspaceIdentity,
    ) -> Result<Option<SyncWatchProbe>> {
        let Some(profile) = self.sync_profile()? else {
            return Ok(None);
        };
        if profile.device_id.is_none() || profile.workspace_id.is_none() {
            return Ok(None);
        }
        Ok(Some(SyncWatchProbe {
            engine: self.sync_engine(identity)?,
        }))
    }

    pub fn sync_auto_push_note(
        &mut self,
        identity: &WorkspaceIdentity,
        note_id: &str,
    ) -> Result<SyncAutoPushResult> {
        let Some(profile) = self.sync_profile()? else {
            return Ok(SyncAutoPushResult {
                configured: false,
                pushed: false,
                conflict: false,
                version: None,
            });
        };
        if profile.device_id.is_none() || profile.workspace_id.is_none() {
            return Ok(SyncAutoPushResult {
                configured: false,
                pushed: false,
                conflict: false,
                version: None,
            });
        }
        let mut engine = self.sync_engine(identity)?;
        let outcome = engine
            .push_note(self.library(identity)?, note_id)
            .map_err(sync_error)?;
        Ok(match outcome {
            PushOutcome::Unchanged { version } => SyncAutoPushResult {
                configured: true,
                pushed: false,
                conflict: false,
                version: Some(version),
            },
            PushOutcome::Applied { version } => SyncAutoPushResult {
                configured: true,
                pushed: true,
                conflict: false,
                version: Some(version),
            },
            PushOutcome::Conflict { .. } => SyncAutoPushResult {
                configured: true,
                pushed: false,
                conflict: true,
                version: None,
            },
        })
    }

    pub fn sync_auto_push_workspace_metadata(
        &mut self,
        identity: &WorkspaceIdentity,
    ) -> Result<SyncAutoPushResult> {
        let Some(profile) = self.sync_profile()? else {
            return Ok(SyncAutoPushResult {
                configured: false,
                pushed: false,
                conflict: false,
                version: None,
            });
        };
        if profile.device_id.is_none() || profile.workspace_id.is_none() {
            return Ok(SyncAutoPushResult {
                configured: false,
                pushed: false,
                conflict: false,
                version: None,
            });
        }
        let mut engine = self.sync_engine(identity)?;
        let outcome = engine
            .push_workspace_metadata(self.library(identity)?)
            .map_err(sync_error)?;
        Ok(match outcome {
            PushOutcome::Unchanged { version } => SyncAutoPushResult {
                configured: true,
                pushed: false,
                conflict: false,
                version: Some(version),
            },
            PushOutcome::Applied { version } => SyncAutoPushResult {
                configured: true,
                pushed: true,
                conflict: false,
                version: Some(version),
            },
            PushOutcome::Conflict { .. } => SyncAutoPushResult {
                configured: true,
                pushed: false,
                conflict: true,
                version: None,
            },
        })
    }

    pub fn sync_wait_for_change(
        &mut self,
        identity: &WorkspaceIdentity,
        timeout_ms: u64,
    ) -> Result<SyncWatchResult> {
        let engine = self.sync_engine(identity)?;
        let batch = engine
            .client()
            .wait_for_events(engine.workspace_id(), engine.cursor(), 1, timeout_ms)
            .map_err(sync_error)?;
        Ok(SyncWatchResult {
            configured: true,
            changed: !batch.ops.is_empty() || !batch.conflicts.is_empty(),
            conflict_count: batch.conflicts.len(),
        })
    }

    pub fn sync_push_note(
        &mut self,
        identity: &WorkspaceIdentity,
        note_id: &str,
    ) -> Result<SyncPushSummary> {
        let mut engine = self.sync_engine(identity)?;
        let outcome = engine
            .push_note(self.library(identity)?, note_id)
            .map_err(sync_error)?;
        Ok(push_summary(outcome))
    }

    pub fn sync_tag_page(
        &self,
        identity: &WorkspaceIdentity,
        request: TagSyncPageRequest,
    ) -> Result<TagSyncPage> {
        Ok(self.library(identity)?.tag_sync_page(request)?)
    }

    pub fn sync_tag_refresh(
        &mut self,
        identity: &WorkspaceIdentity,
        kind: TagSyncKind,
        entity_id: &str,
    ) -> Result<TagSyncSnapshot> {
        let mut engine = self.sync_engine(identity)?;
        engine
            .refresh_tag_record(self.library_mut(identity)?, kind, entity_id)
            .map_err(engine_error)?;
        let current = self.library(identity)?.tag_sync_snapshot(kind, entity_id)?;
        if current.pending.is_some() {
            Ok(self
                .library_mut(identity)?
                .tag_sync_retry(kind, entity_id, current.remote_version)?
                .snapshot)
        } else {
            Ok(current)
        }
    }

    pub fn sync_tag_keep_local(
        &mut self,
        identity: &WorkspaceIdentity,
        decision: SyncTagDecision,
    ) -> Result<TagSyncSnapshot> {
        let mut engine = self.sync_engine(identity)?;
        Ok(engine
            .keep_local_tag_checked(self.library_mut(identity)?, decision.into())
            .map_err(engine_error)?
            .snapshot)
    }

    pub fn sync_tag_keep_remote(
        &mut self,
        identity: &WorkspaceIdentity,
        decision: SyncTagDecision,
    ) -> Result<TagSyncSnapshot> {
        let mut engine = self.sync_engine(identity)?;
        Ok(engine
            .keep_remote_tag_checked(self.library_mut(identity)?, decision.into())
            .map_err(engine_error)?
            .snapshot)
    }

    pub fn sync_auto_push_tags(
        &mut self,
        identity: &WorkspaceIdentity,
    ) -> Result<SyncTagPushBatch> {
        self.library(identity)?;
        let Some(profile) = self.sync_profile()? else {
            return Ok(SyncTagPushBatch::default());
        };
        if profile.device_id.is_none()
            || profile.workspace_id.as_deref() != Some(identity.workspace_id.as_str())
        {
            return Ok(SyncTagPushBatch::default());
        }
        let mut engine = self.sync_engine(identity)?;
        self.push_tag_changes(identity, &mut engine)
    }

    fn retry_tag_dependencies(
        &mut self,
        identity: &WorkspaceIdentity,
        engine: &mut SyncEngine,
        changed: &mut Vec<String>,
    ) -> Result<u32> {
        let mut after = None;
        let mut received = 0;
        loop {
            let page = engine
                .retry_pending_tags(self.library_mut(identity)?, 100, after)
                .map_err(engine_error)?;
            for item in page.items {
                if item.status == TagSyncApplyStatus::Applied {
                    received += 1;
                    changed.push(item.entity_id);
                }
                changed.extend(item.changed_objects);
            }
            after = page.next_cursor;
            if after.is_none() {
                break;
            }
        }
        Ok(received)
    }

    fn finish_tag_batch(
        &self,
        identity: &WorkspaceIdentity,
        batch: &mut SyncTagPushBatch,
    ) -> Result<()> {
        batch.pending = 0;
        batch.dirty = 0;
        batch.unavailable = 0;
        let mut clean: u32 = 0;
        for owner in self.library(identity)?.tag_sync_wire_owners()? {
            let state = self
                .library(identity)?
                .tag_sync_snapshot(owner.kind, &owner.entity_id)?;
            if state.pending.is_some() {
                batch.pending += 1;
            }
            if state.dirty {
                batch.dirty += 1;
            }
            if state.payload.is_none() {
                batch.unavailable += 1;
            }
            if state.pending.is_none() && !state.dirty && state.payload.is_some() {
                clean += 1;
            }
        }
        batch.unchanged = clean.saturating_sub(batch.published.saturating_add(batch.received));
        batch.complete = batch.pending == 0 && batch.dirty == 0 && batch.unavailable == 0;
        batch.changed_objects.sort();
        batch.changed_objects.dedup();
        Ok(())
    }

    fn push_tag_changes(
        &mut self,
        identity: &WorkspaceIdentity,
        engine: &mut SyncEngine,
    ) -> Result<SyncTagPushBatch> {
        let mut batch = SyncTagPushBatch {
            configured: true,
            ..Default::default()
        };
        batch.received =
            self.retry_tag_dependencies(identity, engine, &mut batch.changed_objects)?;
        let mut candidates = engine.pending_tag_records().map_err(engine_error)?;
        for owner in self.library(identity)?.tag_sync_wire_owners()? {
            let state = self
                .library(identity)?
                .tag_sync_snapshot(owner.kind, &owner.entity_id)?;
            if state.dirty && state.pending.is_none() {
                candidates.push((owner.kind, owner.entity_id));
            }
        }
        candidates
            .sort_by(|(a, aid), (b, bid)| a.as_str().cmp(b.as_str()).then_with(|| aid.cmp(bid)));
        candidates.dedup();
        if !candidates.is_empty() {
            let device = engine
                .client()
                .devices()
                .map_err(sync_error)?
                .into_iter()
                .find(|device| device.device_id == engine.client().device_id())
                .ok_or_else(|| AppError::Sync("sync_device_not_paired".into()))?;
            if device.role == MemberRole::Viewer {
                // A downgraded device may still have an earlier committed write.
                // Keep unrelated note pulls on the path that updates open drafts.
                engine
                    .recover_pending_tag_receipts(self.library_mut(identity)?)
                    .map_err(engine_error)?;
            } else {
                for (kind, entity_id) in candidates {
                    let result = engine
                        .push_tag_record(self.library_mut(identity)?, kind, &entity_id)
                        .map_err(engine_error)?;
                    match result.outcome {
                        TagPushOutcome::Published { .. } => batch.published += 1,
                        TagPushOutcome::Received { .. } => {
                            batch.received += 1;
                            batch.changed_objects.push(entity_id);
                        }
                        _ => {}
                    }
                    batch.changed_objects.extend(result.changed_objects);
                }
            }
        }
        self.finish_tag_batch(identity, &mut batch)?;
        // Core can be clean while the exact encrypted request is still unconfirmed.
        if engine.has_pending_tag_requests().map_err(engine_error)? {
            batch.complete = false;
        }
        Ok(batch)
    }

    pub fn sync_push_all(&mut self, identity: &WorkspaceIdentity) -> Result<SyncPushBatch> {
        let mut engine = self.sync_engine(identity)?;
        let notes = self.library(identity)?.list_notes()?;
        let views = self.library(identity)?.list_views()?;
        let canvases = self.library(identity)?.list_canvases()?;
        let tombstones = self.library(identity)?.list_config_tombstones()?;
        let owners = TagWireOwners::from_library(self.library(identity)?).map_err(engine_error)?;
        owners
            .preflight_non_tags(
                notes
                    .iter()
                    .map(|note| note.id.as_str())
                    .chain(views.iter().map(|view| view.id.as_str()))
                    .chain(canvases.iter().map(|canvas| canvas.id.as_str()))
                    .chain(tombstones.iter().map(|deleted| deleted.object_id.as_str())),
            )
            .map_err(engine_error)?;
        let mut summaries = Vec::with_capacity(notes.len());
        for note in notes {
            let outcome = engine
                .push_note(self.library(identity)?, &note.id)
                .map_err(sync_error)?;
            summaries.push(push_summary(outcome));
        }
        summaries.push(push_summary(
            engine
                .push_workspace_metadata(self.library(identity)?)
                .map_err(sync_error)?,
        ));
        // Dictionary and assignments precede tag-filtered view configurations.
        let mut tags = self.push_tag_changes(identity, &mut engine)?;
        for view in views {
            let outcome = engine
                .push_view(self.library(identity)?, &view.id)
                .map_err(sync_error)?;
            summaries.push(push_summary(outcome));
        }
        for canvas in canvases {
            let outcome = engine
                .push_canvas(self.library(identity)?, &canvas.id)
                .map_err(sync_error)?;
            summaries.push(push_summary(outcome));
        }
        for tombstone in tombstones {
            let outcome = match tombstone.kind.as_str() {
                "view" => engine
                    .push_view_remove(&tombstone.object_id)
                    .map_err(sync_error)?,
                "canvas" => engine
                    .push_canvas_remove(&tombstone.object_id)
                    .map_err(sync_error)?,
                _ => return Err(AppError::Sync("sync_unknown_config_tombstone".into())),
            };
            if matches!(outcome, PushOutcome::Applied { .. }) {
                self.library_mut(identity)?
                    .clear_config_tombstone(&tombstone.object_id)?;
            }
            summaries.push(push_summary(outcome));
        }
        // A removed view can unblock a previously deferred tag deletion.
        tags.received +=
            self.retry_tag_dependencies(identity, &mut engine, &mut tags.changed_objects)?;
        self.finish_tag_batch(identity, &mut tags)?;
        if engine.has_pending_tag_requests().map_err(engine_error)? {
            tags.complete = false;
        }
        Ok(SyncPushBatch {
            items: summaries,
            tags,
        })
    }
    pub fn sync_config_conflicts(
        &mut self,
        identity: &WorkspaceIdentity,
    ) -> Result<Vec<SyncConfigConflictView>> {
        let engine = self.sync_engine(identity)?;
        engine
            .pending_config_conflicts()
            .into_iter()
            .map(|conflict| config_conflict_view(&engine, self.library(identity)?, conflict))
            .collect()
    }
    pub fn sync_pull(&mut self, identity: &WorkspaceIdentity) -> Result<SyncPullView> {
        let mut engine = self.sync_engine(identity)?;
        let summary = {
            let library = self.library_mut(identity)?;
            engine.pull_and_apply(library, 100).map_err(engine_error)?
        };
        let conflicts = summary
            .conflicts
            .iter()
            .map(|conflict| SyncLocalConflictView {
                object_id: conflict.object_id.clone(),
                local_title: conflict.local_title.clone(),
                local_body: conflict.local_body.clone(),
                remote_title: conflict.remote_title.clone(),
                remote_body: conflict.remote_body.clone(),
                remote_version: conflict.remote_version,
                remote_tombstone: conflict.remote_tombstone,
            })
            .collect();
        let mut server_conflicts = Vec::with_capacity(summary.server_conflicts.len());
        for conflict in &summary.server_conflicts {
            let local = self.library(identity)?.note(&conflict.object_id).ok();
            let remote = engine.remote_payload(&conflict.object_id).ok().flatten();
            server_conflicts.push(SyncServerConflictView {
                target: SyncConflictTarget {
                    conflict_id: conflict.conflict_id.clone(),
                    object_id: conflict.object_id.clone(),
                    remote_version: conflict.remote_version,
                    remote_tombstone: conflict.remote_tombstone,
                },
                local_title: local
                    .as_ref()
                    .map(|note| note.title.clone())
                    .unwrap_or_default(),
                local_body: local
                    .as_ref()
                    .map(|note| note.document.plain_text())
                    .unwrap_or_default(),
                local_trashed: local.as_ref().is_some_and(|note| note.trashed),
                local_available: local.is_some(),
                remote_title: remote
                    .as_ref()
                    .map(|payload| payload.title.clone())
                    .unwrap_or_default(),
                remote_body: remote
                    .as_ref()
                    .map(|payload| payload.document.plain_text())
                    .unwrap_or_default(),
            });
        }
        Ok(SyncPullView {
            applied: summary.applied,
            skipped: summary.skipped,
            trashed: summary.trashed,
            restored: summary.restored,
            conflicts,
            config_conflicts: summary
                .config_conflicts
                .into_iter()
                .map(|conflict| config_conflict_view(&engine, self.library(identity)?, conflict))
                .collect::<Result<Vec<_>>>()?,
            server_conflicts,
            tag_results: summary.tag_results.into_iter().map(Into::into).collect(),
            tag_pending: self
                .library(identity)?
                .tag_sync_page(TagSyncPageRequest {
                    limit: 1,
                    after: None,
                    pending_only: true,
                })?
                .total,
            next_cursor: summary.next_cursor,
        })
    }

    pub fn sync_config_keep_local(
        &mut self,
        identity: &WorkspaceIdentity,
        object_id: &str,
        remote_version: i64,
        local_state_token: &str,
    ) -> Result<SyncConfigResolution> {
        let mut engine = self.sync_engine(identity)?;
        let outcome = engine
            .keep_local_config_checked(
                self.library(identity)?,
                object_id,
                remote_version,
                local_state_token,
            )
            .map_err(sync_error)?;
        match outcome {
            PushOutcome::Applied { version } | PushOutcome::Unchanged { version } => {
                Ok(SyncConfigResolution {
                    object_id: object_id.into(),
                    mode: SyncConfigResolutionMode::KeepLocal,
                    version,
                })
            }
            PushOutcome::Conflict { .. } => {
                Err(AppError::Sync("sync_config_conflict_changed".into()))
            }
        }
    }

    pub fn sync_config_keep_remote(
        &mut self,
        identity: &WorkspaceIdentity,
        object_id: &str,
        remote_version: i64,
        local_state_token: &str,
    ) -> Result<SyncConfigResolution> {
        let mut engine = self.sync_engine(identity)?;
        let version = {
            let library = self.library_mut(identity)?;
            engine
                .keep_remote_config_checked(library, object_id, remote_version, local_state_token)
                .map_err(sync_error)?
        };
        Ok(SyncConfigResolution {
            object_id: object_id.into(),
            mode: SyncConfigResolutionMode::KeepRemote,
            version,
        })
    }
    pub fn sync_resolve_keep_local(
        &mut self,
        identity: &WorkspaceIdentity,
        conflict: &SyncConflictTarget,
    ) -> Result<SyncPushSummary> {
        let mut engine = self.sync_engine(identity)?;
        let target = conflict_target(conflict);
        let outcome = engine
            .resolve_keep_local_target(self.library(identity)?, &target)
            .map_err(engine_error)?;
        Ok(push_summary(outcome))
    }

    pub fn sync_resolve_keep_remote(
        &mut self,
        identity: &WorkspaceIdentity,
        conflict: &SyncConflictTarget,
    ) -> Result<()> {
        let mut engine = self.sync_engine(identity)?;
        let target = conflict_target(conflict);
        engine
            .resolve_keep_remote_target(self.library_mut(identity)?, &target)
            .map_err(engine_error)
    }

    pub fn sync_resolve_save_as_new(
        &mut self,
        identity: &WorkspaceIdentity,
        conflict: &SyncConflictTarget,
    ) -> Result<String> {
        let mut engine = self.sync_engine(identity)?;
        let target = conflict_target(conflict);
        engine
            .resolve_save_as_new_target(self.library_mut(identity)?, &target)
            .map_err(engine_error)
    }

    pub fn sync_delete_note(
        &mut self,
        identity: &WorkspaceIdentity,
        note_id: &str,
    ) -> Result<SyncPushSummary> {
        let mut engine = self.sync_engine(identity)?;
        engine
            .preflight_non_tag_object(self.library(identity)?, note_id)
            .map_err(engine_error)?;
        self.library_mut(identity)?.set_trashed(note_id, true)?;
        let outcome = engine
            .push_delete(self.library(identity)?, note_id)
            .map_err(sync_error)?;
        Ok(push_summary(outcome))
    }

    pub fn sync_restore_note(
        &mut self,
        identity: &WorkspaceIdentity,
        note_id: &str,
    ) -> Result<SyncPushSummary> {
        let mut engine = self.sync_engine(identity)?;
        engine
            .preflight_non_tag_object(self.library(identity)?, note_id)
            .map_err(engine_error)?;
        self.library_mut(identity)?.set_trashed(note_id, false)?;
        let outcome = engine
            .push_restore(self.library(identity)?, note_id)
            .map_err(sync_error)?;
        Ok(push_summary(outcome))
    }

    pub fn sync_trash(&self, identity: Option<&WorkspaceIdentity>) -> Result<Vec<SyncTrashItem>> {
        let profile = self
            .sync_profile()?
            .ok_or_else(|| AppError::Sync("sync_not_configured".into()))?;
        let workspace_id = profile
            .workspace_id
            .clone()
            .ok_or_else(|| AppError::Sync("sync_workspace_not_attached".into()))?;
        let entries = self
            .sync_client(&profile)?
            .trash(&workspace_id)
            .map_err(sync_error)?;
        let library = identity
            .map(|identity| self.library(identity))
            .transpose()?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let title = library
                    .as_ref()
                    .and_then(|library| library.note(&entry.object_id).ok())
                    .map(|note| note.title);
                trash_view(entry, title)
            })
            .collect())
    }
}

fn member_view(member: MemberRecord) -> SyncMember {
    SyncMember {
        member_id: member.member_id,
        display_name: member.display_name,
        role: member.role.into(),
        status: member.status.as_str().to_owned(),
        created_at: member.created_at,
        removed_at: member.removed_at,
        device_count: member.device_count,
    }
}

fn audit_view(entry: pkms_sync::AuditEntry) -> SyncAuditEvent {
    SyncAuditEvent {
        seq: entry.seq,
        at: entry.at,
        action: entry.action,
        actor_device_id: entry.actor_device_id,
        target_device_id: entry.target_device_id,
        detail: entry.detail,
    }
}

fn device_view(device: DeviceRecord) -> SyncDevice {
    SyncDevice {
        device_id: device.device_id,
        display_name: device.display_name,
        fingerprint: device.fingerprint,
        member_id: device.member_id,
        member_name: device.member_name,
        role: device.role.into(),
        status: device.status.as_str().to_owned(),
        last_seen_at: device.last_seen_at,
        revoked_at: device.revoked_at,
    }
}

fn trash_view(entry: TrashEntry, title: Option<String>) -> SyncTrashItem {
    SyncTrashItem {
        object_id: entry.object_id,
        title,
        version: entry.version,
        purged: entry.purged,
        deleted_at: entry.deleted_at,
        deleted_by: entry.deleted_by,
    }
}

fn conflict_target(conflict: &SyncConflictTarget) -> ConflictTarget {
    ConflictTarget {
        conflict_id: conflict.conflict_id.clone(),
        object_id: conflict.object_id.clone(),
        remote_version: conflict.remote_version,
        remote_tombstone: conflict.remote_tombstone,
    }
}
fn push_summary(outcome: PushOutcome) -> SyncPushSummary {
    match outcome {
        PushOutcome::Unchanged { version } => SyncPushSummary {
            version,
            unchanged: true,
            conflict_remote_version: None,
            conflict_remote_tombstone: false,
        },
        PushOutcome::Applied { version } => SyncPushSummary {
            version,
            unchanged: false,
            conflict_remote_version: None,
            conflict_remote_tombstone: false,
        },
        PushOutcome::Conflict {
            remote_version,
            remote_tombstone,
        } => SyncPushSummary {
            version: 0,
            unchanged: false,
            conflict_remote_version: Some(remote_version),
            conflict_remote_tombstone: remote_tombstone,
        },
    }
}

struct VoiceMarker {
    blob_hash: String,
    plaintext_hash: String,
    duration_ms: u32,
}

fn parse_voice_marker(text: &str) -> Option<VoiceMarker> {
    let start = text.find("<!-- spanseek-voice:v1:")?;
    let rest = &text[start + "<!-- spanseek-voice:v1:".len()..];
    let end = rest.find(" -->")?;
    let mut parts = rest[..end].split(':');
    let blob_hash = parts.next()?.to_owned();
    let plaintext_hash = parts.next()?.to_owned();
    let duration_ms = parts.next()?.parse().ok()?;
    let _mime = parts.next()?;
    if blob_hash.len() != 64 || plaintext_hash.len() != 64 {
        return None;
    }
    Some(VoiceMarker {
        blob_hash,
        plaintext_hash,
        duration_ms,
    })
}

fn canonical_parts(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

fn voice_blob_aad(workspace_id: &str, plaintext_hash: &str, epoch: i64) -> Vec<u8> {
    canonical_parts(&[
        b"spanseek-sync-voice-blob-aad-v1",
        workspace_id.as_bytes(),
        plaintext_hash.as_bytes(),
        &epoch.to_be_bytes(),
    ])
}

#[cfg(test)]
mod voice_sync_tests {
    use super::*;

    #[test]
    fn parses_voice_marker_from_serialized_note_document() {
        let marker = parse_voice_marker(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"<!-- spanseek-voice:v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:24800:audio/webm -->"}]}]}"#,
        )
        .expect("marker");
        assert_eq!(marker.blob_hash.len(), 64);
        assert_eq!(marker.plaintext_hash.len(), 64);
        assert_eq!(marker.duration_ms, 24_800);
    }

    #[test]
    fn voice_blob_aad_matches_length_prefixed_canonical_shape() {
        let aad = voice_blob_aad("workspace-a", "hash", 2);
        assert!(aad.starts_with(&(31u64).to_be_bytes()));
        assert!(aad.windows(11).any(|window| window == b"workspace-a"));
    }
}

#[cfg(test)]
mod voice_sync_import_tests {
    use super::*;
    use pkms_sync::{SyncAuthority, encryption_public_key_hex, owner_proof_message};
    use pkms_sync_client::SyncClient;
    use std::sync::{Arc, Mutex};

    fn start_server(authority: SyncAuthority) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(
                    listener,
                    pkms_sync_server::router(Arc::new(Mutex::new(authority))),
                )
                .await
                .unwrap();
            });
        });
        format!("http://{address}")
    }

    fn create_workspace(app: &mut AppService, path: &std::path::Path) -> WorkspaceIdentity {
        let grant = app
            .grant_path(crate::GrantPurpose::CreateWorkspace, path)
            .unwrap();
        let accepted = app
            .accept_workspace_operation(
                &Uuid::new_v4().to_string(),
                crate::OperationKind::CreateWorkspace,
                &grant.token,
            )
            .unwrap();
        app.execute_operation(&accepted.operation_id)
            .unwrap()
            .result
            .unwrap()
            .identity
    }

    #[test]
    fn imports_a_blob_uploaded_for_the_same_sync_workspace_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let app_path = directory.path().join("app");
        let workspace_path = directory.path().join("knowledge");
        let blob_root = directory.path().join("blobs");
        let mut app = AppService::open(&app_path).unwrap();
        let identity = create_workspace(&mut app, &workspace_path);
        let workspace_id = app.library(&identity).unwrap().info().unwrap().workspace_id;

        let mut authority = SyncAuthority::open_in_memory_with_blobs(&blob_root).unwrap();
        let owner_key = DeviceKeyPair::from_seed([42u8; 32]).unwrap();
        let proof = owner_key.sign(&owner_proof_message(
            &owner_key.public_key_hex(),
            "voice owner",
        ));
        let owner = authority
            .enroll_owner(&owner_key.public_key(), "voice owner", &proof)
            .unwrap();
        let base_url = start_server(authority);
        let owner_encryption =
            encryption_public_key_hex(&owner_key.encryption_public_key().unwrap());
        let owner_client = SyncClient::new(&base_url, &owner.device_id, owner_key).unwrap();
        owner_client
            .register_encryption_key(&owner_encryption)
            .unwrap();
        let invite = owner_client
            .create_pairing_invite_with_role(600, MemberRole::Owner)
            .unwrap();

        app.sync_configure(&base_url, "voice test desktop").unwrap();
        app.sync_pair(&invite.invite_id, &invite.code).unwrap();
        let profile = app.sync_ensure_workspace_key(&workspace_id).unwrap();
        assert_eq!(profile.epoch, 1);
        app.sync_attach_workspace(&workspace_id).unwrap();
        let key = app
            .sync_workspace_key_for_epoch(&profile, &workspace_id, 1)
            .unwrap();

        let audio = b"\x1a\x45\xdf\xa3voice-sync-import-audio".to_vec();
        let plaintext_hash = sha256_hex(&audio);
        let aad = voice_blob_aad(&workspace_id, &plaintext_hash, 1);
        let sealed = key.encrypt(&audio, &aad).unwrap();
        let wire = sealed.encode();
        let server_hash = sha256_hex(&wire);
        let client = app.sync_client(&profile).unwrap();
        client
            .blob_begin(&server_hash, wire.len() as i64, 64 * 1024)
            .unwrap();
        for (index, chunk) in wire.chunks(64 * 1024).enumerate() {
            client
                .blob_chunk(&server_hash, index as u32, chunk)
                .unwrap();
        }
        client.blob_finalize(&server_hash).unwrap();

        let marker = format!(
            "<!-- spanseek-voice:v1:{}:{}:1234:audio/webm -->\n（等待桌面转写）",
            server_hash, plaintext_hash
        );
        app.library_mut(&identity)
            .unwrap()
            .create_note("Phone voice", &marker)
            .unwrap();

        let first = app.voice_sync_import(&identity).unwrap();
        assert_eq!(first.imported, 1);
        assert!(first.failed.is_empty());
        let captures = app
            .library(&identity)
            .unwrap()
            .voice_captures(false)
            .unwrap();
        assert_eq!(captures.items.len(), 1);
        let capture = &captures.items[0];
        assert_eq!(
            app.library(&identity)
                .unwrap()
                .voice_capture_audio(&capture.id)
                .unwrap(),
            audio
        );
        let tasks = app.library(&identity).unwrap().voice_sync_tasks().unwrap();
        assert!(tasks.iter().any(|task| task.status == "done"));

        let second = app.voice_sync_import(&identity).unwrap();
        assert_eq!(second.imported, 1);
        assert_eq!(
            app.library(&identity)
                .unwrap()
                .voice_captures(false)
                .unwrap()
                .items
                .len(),
            1
        );
    }
}

#[cfg(test)]
mod voice_sync_cross_device_tests {
    use super::*;
    use base64::Engine as _;
    use pkms_sync::{
        MemberRole, SyncAuthority, encryption_public_key_hex, owner_proof_message,
        parse_encryption_public_key, workspace_key_aad, wrap_workspace_key,
    };
    use pkms_sync_client::SyncClient;
    use std::sync::{Arc, Mutex};

    fn start_server(authority: SyncAuthority) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(
                    listener,
                    pkms_sync_server::router(Arc::new(Mutex::new(authority))),
                )
                .await
                .unwrap();
            });
        });
        format!("http://{address}")
    }

    fn create_workspace(app: &mut AppService, path: &std::path::Path) -> WorkspaceIdentity {
        let grant = app
            .grant_path(crate::GrantPurpose::CreateWorkspace, path)
            .unwrap();
        let accepted = app
            .accept_workspace_operation(
                &Uuid::new_v4().to_string(),
                crate::OperationKind::CreateWorkspace,
                &grant.token,
            )
            .unwrap();
        app.execute_operation(&accepted.operation_id)
            .unwrap()
            .result
            .unwrap()
            .identity
    }

    #[test]
    fn phone_js_upload_then_desktop_pull_and_import() {
        let directory = tempfile::tempdir().unwrap();
        let app_path = directory.path().join("app");
        let workspace_path = directory.path().join("knowledge");
        let blob_root = directory.path().join("blobs");
        let mut app = AppService::open(&app_path).unwrap();
        let identity = create_workspace(&mut app, &workspace_path);
        let workspace_id = app.library(&identity).unwrap().info().unwrap().workspace_id;

        let mut authority = SyncAuthority::open_in_memory_with_blobs(&blob_root).unwrap();
        let owner_key = DeviceKeyPair::from_seed([47u8; 32]).unwrap();
        let proof = owner_key.sign(&owner_proof_message(&owner_key.public_key_hex(), "owner"));
        let owner = authority
            .enroll_owner(&owner_key.public_key(), "owner", &proof)
            .unwrap();
        let base_url = start_server(authority);
        let owner_encryption = owner_key.encryption_public_key().unwrap();
        let owner_client = SyncClient::new(&base_url, &owner.device_id, owner_key).unwrap();
        owner_client
            .register_encryption_key(&encryption_public_key_hex(&owner_encryption))
            .unwrap();
        let workspace_key = WorkspaceKey::generate().unwrap();
        let owner_envelope = wrap_workspace_key(
            owner_encryption,
            &workspace_key,
            &workspace_key_aad(&workspace_id, 1, &owner.device_id),
        )
        .unwrap();
        owner_client
            .rotate_workspace_key(
                &workspace_id,
                1,
                &[(owner.device_id.clone(), owner_envelope)],
            )
            .unwrap();

        // Phone browser identity: upload through the real mobile JS against this server.
        let phone_seed = [48u8; 32];
        let phone_key = DeviceKeyPair::from_seed(phone_seed).unwrap();
        let phone_encryption = phone_key.encryption_public_key().unwrap();
        let phone_invite = owner_client
            .create_pairing_invite_with_role(600, MemberRole::Editor)
            .unwrap();
        let phone_client =
            SyncClient::new(&base_url, "", DeviceKeyPair::from_seed(phone_seed).unwrap()).unwrap();
        let phone = phone_client
            .pair_with_code(&phone_invite.invite_id, &phone_invite.code, "phone browser")
            .unwrap();
        let phone_authed = SyncClient::new(
            &base_url,
            &phone.device_id,
            DeviceKeyPair::from_seed(phone_seed).unwrap(),
        )
        .unwrap();
        phone_authed
            .register_encryption_key(&encryption_public_key_hex(&phone_encryption))
            .unwrap();
        owner_client
            .grant_workspace_key(
                &workspace_id,
                1,
                &phone.device_id,
                &wrap_workspace_key(
                    phone_encryption,
                    &workspace_key,
                    &workspace_key_aad(&workspace_id, 1, &phone.device_id),
                )
                .unwrap(),
            )
            .unwrap();

        let key_bytes: [u8; 32] = hex_decode(&workspace_key.export_hex())
            .unwrap()
            .try_into()
            .unwrap();
        let phone_public = hex_decode(&phone_key.public_key_hex()).unwrap();
        let fixture = serde_json::json!({
            "profile": {
                "endpoint": base_url,
                "workspaceId": workspace_id,
                "epoch": 1,
                "deviceId": phone.device_id,
                "displayName": "phone browser",
            },
            "edPrivateJwk": {
                "kty": "OKP",
                "crv": "Ed25519",
                "d": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(phone_seed),
                "x": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(phone_public),
            },
            "workspaceKeyBase64": base64::engine::general_purpose::STANDARD.encode(key_bytes),
            "captureId": Uuid::new_v4().to_string(),
        });
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/mobile-capture/tests/upload-real-server.mjs");
        let node = std::env::var("NODE_BIN").unwrap_or_else(|_| "node".into());
        let output = std::process::Command::new(node)
            .arg(script)
            .env("VOICE_SYNC_FIXTURE", fixture.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "phone upload failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Desktop joins the same workspace as an editor and gets the key from the owner.
        let desktop_invite = owner_client
            .create_pairing_invite_with_role(600, MemberRole::Editor)
            .unwrap();
        app.sync_configure(&base_url, "desktop").unwrap();
        let desktop_profile = app
            .sync_pair(&desktop_invite.invite_id, &desktop_invite.code)
            .unwrap();
        let desktop_public =
            parse_encryption_public_key(&desktop_profile.encryption_public_key).unwrap();
        owner_client
            .grant_workspace_key(
                &workspace_id,
                1,
                desktop_profile.device_id.as_deref().unwrap(),
                &wrap_workspace_key(
                    desktop_public,
                    &workspace_key,
                    &workspace_key_aad(
                        &workspace_id,
                        1,
                        desktop_profile.device_id.as_deref().unwrap(),
                    ),
                )
                .unwrap(),
            )
            .unwrap();
        app.sync_attach_workspace(&workspace_id).unwrap();
        let pulled = app.sync_pull(&identity).unwrap();
        assert!(pulled.conflicts.is_empty());
        let imported = app.voice_sync_import(&identity).unwrap();
        assert_eq!(imported.imported, 1);
        let captures = app
            .library(&identity)
            .unwrap()
            .voice_captures(false)
            .unwrap();
        assert_eq!(captures.items.len(), 1);
        assert!(
            app.library(&identity)
                .unwrap()
                .voice_capture_audio(&captures.items[0].id)
                .unwrap()
                .starts_with(b"\x1a\x45\xdf\xa3")
        );
    }
}
