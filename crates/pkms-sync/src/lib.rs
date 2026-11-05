//! Cross-device identity, pairing, authorization and revocation primitives.
//!
//! G02 covers identity/revocation; G03 adds the encrypted object protocol.
//! Transport, client vault, workspace grants and key epochs are later tasks.

mod authority;
mod blobs;
mod compatibility;
mod crypto;
mod error;
mod keyring;
mod keys;
mod model;
mod protocol;
mod queue;
mod resume;
pub mod transport;

pub use authority::{SyncAuthority, validate_public_key};
pub use blobs::{
    MAX_BLOB_BYTES, MAX_BLOB_CHUNKS, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE, blob_begin_request_hash,
    blob_chunk_request_hash, blob_finalize_request_hash, blob_read_request_hash,
    blob_status_request_hash,
};
pub use compatibility::{
    CURRENT_AUTHORITY_SCHEMA, MIN_SUPPORTED_AUTHORITY_SCHEMA, RejectReason, SYNC_PROTOCOL_VERSION,
    SchemaDecision, classify_authority_database,
};
pub use crypto::{
    DeviceKeyPair, PUBLIC_KEY_LEN, SIGNATURE_LEN, fingerprint, hex_decode, hex_encode,
    normalize_pairing_code, owner_proof_message, pairing_proof_message, sha256_hex,
};
pub use error::SyncError;
pub use keyring::{
    device_encryption_key_request_hash, grant_workspace_key_request_hash,
    key_envelope_request_hash, workspace_key_rotation_request_hash,
};
pub use keys::{
    KeyEnvelope, SealedData, WorkspaceKey, blob_aad, derive_device_encryption_secret,
    encryption_public_key, encryption_public_key_hex, object_aad, parse_encryption_public_key,
    unwrap_workspace_key, workspace_key_aad, wrap_workspace_key,
};
pub use model::{
    AppliedOp, ApplyOutcome, AuditEntry, BlobBegin, BlobRecord, BlobStatus, Conflict,
    ConflictReason, ConflictRecord, ConflictResolution, DeviceEncryptionKey, DeviceRecord,
    DeviceStatus, MemberRecord, MemberRole, MemberStatus, ObjectState, OpKind, PairingInvite,
    PullBatch, RemoteOp, SignedRequest, SyncAccess, SyncOp, TrashEntry,
};
pub use protocol::{
    MAX_CIPHERTEXT_BYTES, MAX_PULL_LIMIT, op_request_hash, pull_request_hash,
    pull_wait_request_hash, resolve_conflict_request_hash, snapshot_request_hash,
    trash_request_hash,
};
pub use queue::{OfflineQueue, QueueState, QueuedOp};
pub use resume::{
    DraftHandoff, DraftHandoffInput, DraftHandoffOutcome, MAX_DRAFT_CIPHERTEXT_BYTES,
    MAX_RESUME_ID_CHARS, MAX_RESUME_TITLE_CHARS, ReadingPosition, ReadingPositionInput,
    ReadingPositionOutcome, ResumeLocator, adopt_draft_request_hash, draft_handoff_aad,
    draft_handoff_put_request_hash, draft_handoffs_request_hash, latest_reading_position,
    reading_position_put_request_hash, reading_positions_request_hash,
};
