//! Wire payloads shared by the HTTP server and client.
//!
//! The authority remains transport-agnostic. These structs only bind a signed
//! request to the exact payload each endpoint forwards, so the server and
//! client cannot drift into different JSON shapes.

use serde::{Deserialize, Serialize};

use crate::crypto::{canonical, sha256_array};
use crate::{
    ConflictResolution, DraftHandoffInput, KeyEnvelope, ReadingPositionInput, SignedRequest, SyncOp,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyOpBody {
    pub request: SignedRequest,
    pub op: SyncOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub after_seq: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullWaitBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub after_seq: i64,
    pub limit: usize,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBody {
    pub request: SignedRequest,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashBody {
    pub request: SignedRequest,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveConflictBody {
    pub request: SignedRequest,
    pub conflict_id: String,
    pub resolution: ConflictResolution,
    pub replacement_op_id: Option<String>,
    pub replacement_object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingPositionPutBody {
    pub request: SignedRequest,
    pub input: ReadingPositionInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingPositionsBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub object_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftHandoffPutBody {
    pub request: SignedRequest,
    pub input: DraftHandoffInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftHandoffsBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub object_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptDraftHandoffBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub object_id: String,
    pub handoff_id: String,
}

/// Stable response union for endpoints whose success value is already a
/// domain type. Kept here so clients do not need to know server internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthBody {
    pub status: String,
    pub protocol_version: i64,
    pub schema_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingInviteBody {
    pub request: SignedRequest,
    pub lifetime_seconds: i64,
    pub role: Option<String>,
    pub member_id: Option<String>,
    pub member_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingCompleteBody {
    pub invite_id: String,
    pub code: String,
    pub public_key: String,
    pub display_name: String,
    #[serde(with = "crate::crypto::base64_vec")]
    pub proof: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevicesBody {
    pub request: SignedRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembersBody {
    pub request: SignedRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMemberRoleBody {
    pub request: SignedRequest,
    pub member_id: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveMemberBody {
    pub request: SignedRequest,
    pub member_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLogBody {
    pub request: SignedRequest,
    pub limit: usize,
}

pub fn members_request_hash() -> [u8; 32] {
    sha256_array(&canonical(&[b"spanseek-sync-members-v1"]))
}

pub fn set_member_role_request_hash(member_id: &str, role: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-set-member-role-v1",
        member_id.as_bytes(),
        role.as_bytes(),
    ]))
}

pub fn remove_member_request_hash(member_id: &str, reason: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-remove-member-v1",
        member_id.as_bytes(),
        reason.as_bytes(),
    ]))
}

pub fn audit_log_request_hash(limit: usize) -> [u8; 32] {
    let limit = limit.to_string();
    sha256_array(&canonical(&[
        b"spanseek-sync-audit-log-v1",
        limit.as_bytes(),
    ]))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeDeviceBody {
    pub request: SignedRequest,
    pub target_device_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDeviceRoleBody {
    pub request: SignedRequest,
    pub target_device_id: String,
    pub role: String,
}

pub fn pairing_invite_request_hash(
    lifetime_seconds: i64,
    role: Option<&str>,
    member_id: Option<&str>,
    member_name: Option<&str>,
) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-pairing-invite-v3",
        lifetime_seconds.to_be_bytes().as_slice(),
        role.unwrap_or_default().as_bytes(),
        member_id.unwrap_or_default().as_bytes(),
        member_name.unwrap_or_default().as_bytes(),
    ]))
}

pub fn devices_request_hash() -> [u8; 32] {
    sha256_array(&canonical(&[b"spanseek-sync-devices-v1"]))
}

pub fn revoke_device_request_hash(target_device_id: &str, reason: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-revoke-v1",
        target_device_id.as_bytes(),
        reason.as_bytes(),
    ]))
}

pub fn set_device_role_request_hash(target_device_id: &str, role: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-set-device-role-v1",
        target_device_id.as_bytes(),
        role.as_bytes(),
    ]))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterEncryptionKeyBody {
    pub request: SignedRequest,
    pub encryption_public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateWorkspaceKeyBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub new_epoch: i64,
    pub envelopes: Vec<(String, KeyEnvelope)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEnvelopeBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantWorkspaceKeyBody {
    pub request: SignedRequest,
    pub workspace_id: String,
    pub epoch: i64,
    pub device_id: String,
    pub envelope: KeyEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentKeyEpochBody {
    pub request: SignedRequest,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceEncryptionKeysBody {
    pub request: SignedRequest,
}
pub fn current_key_epoch_request_hash(workspace_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-current-key-epoch-v1",
        workspace_id.as_bytes(),
    ]))
}

pub fn device_encryption_keys_request_hash() -> [u8; 32] {
    sha256_array(&canonical(&[b"spanseek-sync-device-encryption-keys-v1"]))
}
pub fn grant_workspace_key_request_hash(
    workspace_id: &str,
    epoch: i64,
    device_id: &str,
    envelope: &KeyEnvelope,
) -> [u8; 32] {
    crate::grant_workspace_key_request_hash(workspace_id, epoch, device_id, envelope)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobBeginBody {
    pub request: SignedRequest,
    pub blob_hash: String,
    pub size: i64,
    pub chunk_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobChunkBody {
    pub request: SignedRequest,
    pub blob_hash: String,
    pub index: u32,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobHashBody {
    pub request: SignedRequest,
    pub blob_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobReadBody {
    pub request: SignedRequest,
    pub blob_hash: String,
    pub index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobChunkResponse {
    pub data: String,
}
