use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Active,
    Revoked,
}

impl DeviceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            other => Err(crate::SyncError::UnsupportedDatabase(format!(
                "unknown device status {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncAccess {
    Read,
    Write,
}

impl SyncAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Owner,
    Editor,
    Viewer,
}

impl MemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
        }
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    pub fn can_administer(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "owner" => Ok(Self::Owner),
            "editor" => Ok(Self::Editor),
            "viewer" => Ok(Self::Viewer),
            other => Err(crate::SyncError::InvalidRole(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Active,
    Removed,
}

impl MemberStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Removed => "removed",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "active" => Ok(Self::Active),
            "removed" => Ok(Self::Removed),
            other => Err(crate::SyncError::UnsupportedDatabase(format!(
                "unknown member status {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub member_id: String,
    pub display_name: String,
    pub role: MemberRole,
    pub status: MemberStatus,
    pub created_at: String,
    pub removed_at: Option<String>,
    pub removed_by: Option<String>,
    pub removed_reason: Option<String>,
    pub device_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub display_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub member_id: String,
    pub member_name: String,
    pub role: MemberRole,
    pub status: DeviceStatus,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
    pub revoked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEncryptionKey {
    pub device_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingInvite {
    pub invite_id: String,
    /// Shown once to the pairing user. Only a hash is stored.
    pub code: String,
    pub created_by: String,
    pub member_id: Option<String>,
    pub member_name: Option<String>,
    pub role: MemberRole,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRequest {
    pub device_id: String,
    pub access: SyncAccess,
    pub nonce: String,
    /// Unix seconds chosen by the client; the authority only accepts a small
    /// window and never uses it for ordering.
    pub issued_at: i64,
    /// SHA-256 of the request payload computed by the caller.
    #[serde(with = "crate::crypto::hex_array32")]
    pub request_hash: [u8; 32],
    #[serde(with = "crate::crypto::base64_vec")]
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: i64,
    pub at: String,
    pub action: String,
    pub actor_device_id: Option<String>,
    pub target_device_id: Option<String>,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Put,
    Delete,
    Restore,
    Purge,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Put => "put",
            Self::Delete => "delete",
            Self::Restore => "restore",
            Self::Purge => "purge",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "put" => Ok(Self::Put),
            "delete" => Ok(Self::Delete),
            "restore" => Ok(Self::Restore),
            "purge" => Ok(Self::Purge),
            other => Err(crate::SyncError::InvalidOp(format!(
                "unknown op kind {other}"
            ))),
        }
    }
}

/// One encrypted object operation. `ciphertext` is opaque to the authority;
/// key management and encryption live on the client (G08).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOp {
    pub op_id: String,
    pub workspace_id: String,
    pub object_id: String,
    pub kind: OpKind,
    pub base_version: i64,
    pub key_epoch: i64,
    #[serde(with = "crate::crypto::base64_opt_vec")]
    pub ciphertext: Option<Vec<u8>>,
    /// Client metadata only; never used for ordering or conflict decisions.
    pub client_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedOp {
    pub op_id: String,
    pub server_seq: i64,
    pub version: i64,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictReason {
    VersionMismatch,
    Deleted,
    NotFound,
    AlreadyLive,
    Purged,
}

impl ConflictReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VersionMismatch => "version_mismatch",
            Self::Deleted => "deleted",
            Self::NotFound => "not_found",
            Self::AlreadyLive => "already_live",
            Self::Purged => "purged",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "version_mismatch" => Ok(Self::VersionMismatch),
            "deleted" => Ok(Self::Deleted),
            "not_found" => Ok(Self::NotFound),
            "already_live" => Ok(Self::AlreadyLive),
            "purged" => Ok(Self::Purged),
            other => Err(crate::SyncError::InvalidOp(format!(
                "unknown conflict reason {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub reason: ConflictReason,
    pub current_version: i64,
    pub current_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied(AppliedOp),
    Conflict(Conflict),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOp {
    pub server_seq: i64,
    pub op_id: String,
    pub workspace_id: String,
    pub object_id: String,
    pub kind: OpKind,
    pub base_version: i64,
    pub resulting_version: i64,
    pub device_id: String,
    pub key_epoch: i64,
    #[serde(with = "crate::crypto::base64_opt_vec")]
    pub ciphertext: Option<Vec<u8>>,
    pub tombstone: bool,
    pub purged: bool,
    pub deleted_at: Option<String>,
    pub deleted_by: Option<String>,
    pub client_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullBatch {
    pub ops: Vec<RemoteOp>,
    pub next_cursor: i64,
    pub has_more: bool,
    /// Unresolved conflicts for this device, newest first.
    pub conflicts: Vec<ConflictRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    KeepLocal,
    KeepRemote,
    SaveAsNew,
}

impl ConflictResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KeepLocal => "keep_local",
            Self::KeepRemote => "keep_remote",
            Self::SaveAsNew => "save_as_new",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::SyncError> {
        match value {
            "keep_local" => Ok(Self::KeepLocal),
            "keep_remote" => Ok(Self::KeepRemote),
            "save_as_new" => Ok(Self::SaveAsNew),
            other => Err(crate::SyncError::InvalidOp(format!(
                "unknown conflict resolution {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub conflict_id: String,
    pub device_id: String,
    pub workspace_id: String,
    pub object_id: String,
    pub op_id: String,
    pub kind: OpKind,
    pub base_version: i64,
    pub remote_version: i64,
    pub remote_tombstone: bool,
    pub reason: ConflictReason,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolution: Option<ConflictResolution>,
    pub replacement_op_id: Option<String>,
    pub replacement_object_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectState {
    pub workspace_id: String,
    pub object_id: String,
    pub version: i64,
    pub tombstone: bool,
    #[serde(with = "crate::crypto::base64_opt_vec")]
    pub ciphertext: Option<Vec<u8>>,
    pub key_epoch: i64,
    pub last_seq: i64,
    pub updated_by: String,
    pub purged: bool,
    pub deleted_at: Option<String>,
    pub deleted_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashEntry {
    pub workspace_id: String,
    pub object_id: String,
    pub version: i64,
    pub purged: bool,
    pub deleted_at: Option<String>,
    pub deleted_by: Option<String>,
    pub last_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRecord {
    pub blob_hash: String,
    pub size: i64,
    pub chunk_size: i64,
    pub chunk_count: i64,
    pub complete: bool,
    pub created_at: String,
    pub created_by: String,
    pub finalized_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobStatus {
    pub blob_hash: String,
    pub size: i64,
    pub chunk_size: i64,
    pub chunk_count: i64,
    pub complete: bool,
    pub received_chunks: Vec<u32>,
    pub missing_chunks: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobBegin {
    pub already_present: bool,
    pub status: BlobStatus,
}
