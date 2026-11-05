use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("device not found")]
    DeviceNotFound,
    #[error("device is revoked")]
    DeviceRevoked,
    #[error("device is not active")]
    DeviceInactive,
    #[error("device is already registered")]
    DuplicateDevice,
    #[error("owner is already enrolled")]
    OwnerAlreadyEnrolled,
    #[error("invalid device public key")]
    InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("request timestamp outside the allowed window")]
    StaleRequest,
    #[error("request nonce already used")]
    ReplayedRequest,
    #[error("invalid request nonce")]
    InvalidNonce,
    #[error("pairing invite not found")]
    InviteNotFound,
    #[error("pairing invite already used")]
    InviteConsumed,
    #[error("pairing invite expired")]
    InviteExpired,
    #[error("invalid pairing code")]
    InvalidPairingCode,
    #[error("invalid device display name")]
    InvalidDisplayName,
    #[error("invalid revocation reason")]
    InvalidRevocationReason,
    #[error("request access does not match the operation")]
    InvalidAccess,
    #[error("request hash does not match the operation")]
    InvalidRequestHash,
    #[error("invalid sync operation: {0}")]
    InvalidOp(String),
    #[error("op_id was reused with different content")]
    OpIdReused,
    #[error("invalid pull cursor")]
    InvalidCursor,
    #[error("conflict is already resolved")]
    ConflictAlreadyResolved,
    #[error("conflict not found")]
    ConflictNotFound,
    #[error("blob storage is not configured")]
    BlobStorageUnavailable,
    #[error("blob not found")]
    BlobNotFound,
    #[error("blob is already complete")]
    BlobAlreadyComplete,
    #[error("blob is not finalized")]
    BlobNotFinalized,
    #[error("blob metadata conflicts with an existing upload")]
    BlobConflict,
    #[error("blob hash does not match content")]
    BlobHashMismatch,
    #[error("blob chunk does not match its signed digest or expected length")]
    BlobChunkMismatch,
    #[error("invalid blob: {0}")]
    InvalidBlob(String),
    #[error("cryptographic operation failed")]
    CryptoUnavailable,
    #[error("invalid device encryption key")]
    InvalidEncryptionKey,
    #[error("device has no registered encryption key")]
    MissingEncryptionKey,
    #[error("workspace key not found")]
    KeyNotFound,
    #[error("operation uses a stale key epoch")]
    KeyEpochMismatch,
    #[error("invalid key envelope")]
    InvalidKeyEnvelope,
    #[error("invalid resume handoff: {0}")]
    InvalidResume(String),
    #[error("draft handoff was reused with different content")]
    DraftIdReused,
    #[error("draft handoff not found")]
    DraftNotFound,
    #[error("sync authority io error")]
    Io(#[from] std::io::Error),
    #[error("cannot revoke the last active device")]
    LastActiveDevice,
    #[error("cannot revoke the last active owner")]
    LastOwner,
    #[error("sync member not found")]
    MemberNotFound,
    #[error("sync member was removed")]
    MemberRemoved,
    #[error("insufficient member role for this operation")]
    InsufficientRole,
    #[error("invalid member role: {0}")]
    InvalidRole(String),
    #[error("invalid pairing lifetime")]
    InvalidLifetime,
    #[error("random source unavailable")]
    RandomUnavailable,
    #[error("unsupported sync authority database: {0}")]
    UnsupportedDatabase(String),
    #[error("sync authority database error")]
    Database(#[from] rusqlite::Error),
    #[error("sync authority json error")]
    Json(#[from] serde_json::Error),
}

impl SyncError {
    /// Stable wire-facing category. Callers must not parse Display text.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound => "device_not_found",
            Self::DeviceRevoked => "device_revoked",
            Self::DeviceInactive => "device_inactive",
            Self::DuplicateDevice => "duplicate_device",
            Self::OwnerAlreadyEnrolled => "owner_already_enrolled",
            Self::InvalidPublicKey => "invalid_public_key",
            Self::InvalidSignature => "invalid_signature",
            Self::StaleRequest => "stale_request",
            Self::ReplayedRequest => "replayed_request",
            Self::InvalidNonce => "invalid_nonce",
            Self::InviteNotFound => "invite_not_found",
            Self::InviteConsumed => "invite_consumed",
            Self::InviteExpired => "invite_expired",
            Self::InvalidPairingCode => "invalid_pairing_code",
            Self::InvalidDisplayName => "invalid_display_name",
            Self::InvalidRevocationReason => "invalid_revocation_reason",
            Self::InvalidAccess => "invalid_access",
            Self::InvalidRequestHash => "invalid_request_hash",
            Self::InvalidOp(_) => "invalid_op",
            Self::OpIdReused => "op_id_reused",
            Self::InvalidCursor => "invalid_cursor",
            Self::ConflictAlreadyResolved => "conflict_already_resolved",
            Self::ConflictNotFound => "conflict_not_found",
            Self::BlobStorageUnavailable => "blob_storage_unavailable",
            Self::BlobNotFound => "blob_not_found",
            Self::BlobAlreadyComplete => "blob_already_complete",
            Self::BlobNotFinalized => "blob_not_finalized",
            Self::BlobConflict => "blob_conflict",
            Self::BlobHashMismatch => "blob_hash_mismatch",
            Self::BlobChunkMismatch => "blob_chunk_mismatch",
            Self::InvalidBlob(_) => "invalid_blob",
            Self::CryptoUnavailable => "crypto_unavailable",
            Self::InvalidEncryptionKey => "invalid_encryption_key",
            Self::MissingEncryptionKey => "missing_encryption_key",
            Self::KeyNotFound => "key_not_found",
            Self::KeyEpochMismatch => "key_epoch_mismatch",
            Self::InvalidKeyEnvelope => "invalid_key_envelope",
            Self::InvalidResume(_) => "invalid_resume",
            Self::DraftIdReused => "draft_id_reused",
            Self::DraftNotFound => "draft_not_found",
            Self::Io(_) => "io_error",
            Self::LastActiveDevice => "last_active_device",
            Self::LastOwner => "last_owner",
            Self::MemberNotFound => "member_not_found",
            Self::MemberRemoved => "member_removed",
            Self::InsufficientRole => "insufficient_role",
            Self::InvalidRole(_) => "invalid_role",
            Self::InvalidLifetime => "invalid_lifetime",
            Self::RandomUnavailable => "random_unavailable",
            Self::UnsupportedDatabase(_) => "unsupported_database",
            Self::Database(_) => "database_error",
            Self::Json(_) => "json_error",
        }
    }
}
