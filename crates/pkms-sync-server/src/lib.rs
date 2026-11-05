//! HTTP transport around [`SyncAuthority`].
//!
//! The authority is synchronous SQLite state. Each route locks it for the
//! duration of one request; no database transaction spans an await point.
//! Authentication stays in the signed request, so the HTTP layer never needs a
//! second session token. CORS is permissive because the PWA may be served from
//! another origin; possession of the device key remains the write boundary.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use pkms_sync::transport::{
    AdoptDraftHandoffBody, ApiErrorBody, ApplyOpBody, AuditLogBody, BlobBeginBody, BlobChunkBody,
    BlobChunkResponse, BlobHashBody, BlobReadBody, CurrentKeyEpochBody, DeviceEncryptionKeysBody,
    DevicesBody, DraftHandoffPutBody, DraftHandoffsBody, GrantWorkspaceKeyBody, HealthBody,
    KeyEnvelopeBody, MembersBody, PairingCompleteBody, PairingInviteBody, PullBody, PullWaitBody,
    ReadingPositionPutBody, ReadingPositionsBody, RegisterEncryptionKeyBody, RemoveMemberBody,
    ResolveConflictBody, RevokeDeviceBody, RotateWorkspaceKeyBody, SetDeviceRoleBody,
    SetMemberRoleBody, SnapshotBody, TrashBody, audit_log_request_hash,
    current_key_epoch_request_hash, device_encryption_keys_request_hash, devices_request_hash,
    grant_workspace_key_request_hash, members_request_hash, pairing_invite_request_hash,
    remove_member_request_hash, revoke_device_request_hash, set_device_role_request_hash,
    set_member_role_request_hash,
};
use pkms_sync::{
    ApplyOutcome, BlobBegin, BlobRecord, BlobStatus, ConflictRecord, DeviceRecord, DraftHandoff,
    DraftHandoffOutcome, KeyEnvelope, MemberRecord, MemberRole, ObjectState, PairingInvite,
    PullBatch, ReadingPosition, ReadingPositionOutcome, SYNC_PROTOCOL_VERSION, SyncAccess,
    SyncAuthority, SyncError, TrashEntry, device_encryption_key_request_hash, hex_decode,
    key_envelope_request_hash, pull_wait_request_hash, validate_public_key,
    workspace_key_rotation_request_hash,
};
use tower_http::cors::CorsLayer;

/// Base64 expands a 16 MiB ciphertext to about 21.4 MiB; leave room for JSON
/// fields and signatures.
pub const MAX_WIRE_BODY_BYTES: usize = 32 * 1024 * 1024;

pub type SharedAuthority = Arc<Mutex<SyncAuthority>>;

/// Builds the complete versioned-protocol router. Callers own the listening socket,
/// which keeps the library usable from tests and from the CLI.
pub fn router(authority: SharedAuthority) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/pairing/invites", post(create_pairing_invite))
        .route("/v1/pairing/complete", post(complete_pairing))
        .route("/v1/devices/list", post(devices))
        .route("/v1/devices/revoke", post(revoke_device))
        .route("/v1/devices/role", post(set_device_role))
        .route("/v1/members/list", post(members))
        .route("/v1/members/role", post(set_member_role))
        .route("/v1/members/remove", post(remove_member))
        .route("/v1/audit", post(audit_log))
        .route("/v1/keys/device", post(register_encryption_key))
        .route("/v1/keys/rotate", post(rotate_workspace_key))
        .route("/v1/keys/envelope", post(fetch_key_envelope))
        .route("/v1/keys/grant", post(grant_workspace_key))
        .route("/v1/keys/epoch", post(current_key_epoch))
        .route("/v1/keys/devices", post(device_encryption_keys))
        .route("/v1/ops", post(apply_op))
        .route("/v1/pull", post(pull))
        .route("/v1/pull/wait", post(wait_for_events))
        .route("/v1/snapshot", post(snapshot))
        .route("/v1/trash", post(trash))
        .route("/v1/conflicts/resolve", post(resolve_conflict))
        .route("/v1/reading-positions/put", post(put_reading_position))
        .route("/v1/reading-positions/list", post(reading_positions))
        .route("/v1/draft-handoffs/put", post(put_draft_handoff))
        .route("/v1/draft-handoffs/list", post(draft_handoffs))
        .route("/v1/draft-handoffs/adopt", post(adopt_draft_handoff))
        .route("/v1/blobs/begin", post(blob_begin))
        .route("/v1/blobs/chunk", post(blob_chunk))
        .route("/v1/blobs/status", post(blob_status))
        .route("/v1/blobs/finalize", post(blob_finalize))
        .route("/v1/blobs/read", post(blob_read))
        .layer(DefaultBodyLimit::max(MAX_WIRE_BODY_BYTES))
        .layer(CorsLayer::permissive())
        .with_state(authority)
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ApiErrorBody,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    pub fn from_sync(error: SyncError) -> Self {
        Self::new(status_for(error.code()), error.code(), error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

fn status_for(code: &str) -> StatusCode {
    match code {
        "device_not_found" | "device_revoked" | "device_inactive" | "invalid_signature"
        | "stale_request" | "replayed_request" | "invalid_nonce" | "invalid_public_key" => {
            StatusCode::UNAUTHORIZED
        }
        "insufficient_role" => StatusCode::FORBIDDEN,
        "conflict_not_found" | "draft_not_found" | "blob_not_found" | "member_not_found" => {
            StatusCode::NOT_FOUND
        }
        "conflict_already_resolved"
        | "op_id_reused"
        | "key_epoch_mismatch"
        | "draft_id_reused"
        | "blob_conflict"
        | "last_owner"
        | "member_removed"
        | "unsupported_database" => StatusCode::CONFLICT,
        "database_error" | "io_error" | "json_error" => StatusCode::INTERNAL_SERVER_ERROR,
        "random_unavailable" | "crypto_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn lock_authority(
    authority: &SharedAuthority,
) -> Result<std::sync::MutexGuard<'_, SyncAuthority>, ApiError> {
    authority
        .lock()
        .map_err(|_| ApiError::internal("sync authority lock is poisoned"))
}

async fn health(State(authority): State<SharedAuthority>) -> Result<Json<HealthBody>, ApiError> {
    let authority = lock_authority(&authority)?;
    let schema_version = authority.schema_version().map_err(ApiError::from_sync)?;
    Ok(Json(HealthBody {
        status: "ok".into(),
        protocol_version: SYNC_PROTOCOL_VERSION,
        schema_version,
    }))
}

async fn grant_workspace_key(
    State(authority): State<SharedAuthority>,
    Json(body): Json<GrantWorkspaceKeyBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash
        != grant_workspace_key_request_hash(
            &body.workspace_id,
            body.epoch,
            &body.device_id,
            &body.envelope,
        )
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .grant_workspace_key(
            &body.request,
            &body.workspace_id,
            body.epoch,
            &body.device_id,
            &body.envelope,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(serde_json::json!({"granted": true})))
}

async fn current_key_epoch(
    State(authority): State<SharedAuthority>,
    Json(body): Json<CurrentKeyEpochBody>,
) -> Result<Json<Option<i64>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != current_key_epoch_request_hash(&body.workspace_id) {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let epoch = authority
        .current_key_epoch(&body.workspace_id)
        .map_err(ApiError::from_sync)?;
    Ok(Json(epoch))
}

async fn device_encryption_keys(
    State(authority): State<SharedAuthority>,
    Json(body): Json<DeviceEncryptionKeysBody>,
) -> Result<Json<Vec<pkms_sync::DeviceEncryptionKey>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != device_encryption_keys_request_hash() {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let keys = authority
        .list_active_encryption_keys()
        .map_err(ApiError::from_sync)?;
    Ok(Json(keys))
}

async fn apply_op(
    State(authority): State<SharedAuthority>,
    Json(body): Json<ApplyOpBody>,
) -> Result<Json<ApplyOutcome>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let outcome = authority
        .apply_op(&body.request, &body.op, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    Ok(Json(outcome))
}

async fn pull(
    State(authority): State<SharedAuthority>,
    Json(body): Json<PullBody>,
) -> Result<Json<PullBatch>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let batch = authority
        .pull(
            &body.request,
            &body.workspace_id,
            body.after_seq,
            body.limit,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(batch))
}

async fn wait_for_events(
    State(authority): State<SharedAuthority>,
    Json(body): Json<PullWaitBody>,
) -> Result<Json<PullBatch>, ApiError> {
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.after_seq < 0 || body.limit == 0 || body.timeout_ms > 60_000 {
        return Err(ApiError::from_sync(SyncError::InvalidCursor));
    }
    if body.request.request_hash
        != pull_wait_request_hash(
            &body.workspace_id,
            body.after_seq,
            body.limit,
            body.timeout_ms,
        )
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let device_id = {
        let mut authority = lock_authority(&authority)?;
        authority
            .authorize(&body.request, Utc::now().timestamp())
            .map_err(ApiError::from_sync)?
            .device_id
    };
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(body.timeout_ms.min(60_000));
    loop {
        let batch = {
            let authority = lock_authority(&authority)?;
            authority
                .pull_for_device(&device_id, &body.workspace_id, body.after_seq, body.limit)
                .map_err(ApiError::from_sync)?
        };
        if !batch.ops.is_empty() || !batch.conflicts.is_empty() || batch.has_more {
            return Ok(Json(batch));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(Json(batch));
        }
        tokio::time::sleep((deadline - now).min(std::time::Duration::from_millis(200))).await;
    }
}
async fn snapshot(
    State(authority): State<SharedAuthority>,
    Json(body): Json<SnapshotBody>,
) -> Result<Json<Vec<ObjectState>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let objects = authority
        .snapshot(&body.request, &body.workspace_id, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    Ok(Json(objects))
}

async fn trash(
    State(authority): State<SharedAuthority>,
    Json(body): Json<TrashBody>,
) -> Result<Json<Vec<TrashEntry>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let entries = authority
        .trash(&body.request, &body.workspace_id, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    Ok(Json(entries))
}

async fn resolve_conflict(
    State(authority): State<SharedAuthority>,
    Json(body): Json<ResolveConflictBody>,
) -> Result<Json<ConflictRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let conflict = authority
        .resolve_conflict(
            &body.request,
            &body.conflict_id,
            body.resolution,
            body.replacement_op_id.as_deref(),
            body.replacement_object_id.as_deref(),
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(conflict))
}

async fn put_reading_position(
    State(authority): State<SharedAuthority>,
    Json(body): Json<ReadingPositionPutBody>,
) -> Result<Json<ReadingPositionOutcome>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let outcome = authority
        .put_reading_position(&body.request, &body.input, Utc::now().timestamp_millis())
        .map_err(ApiError::from_sync)?;
    Ok(Json(outcome))
}

async fn reading_positions(
    State(authority): State<SharedAuthority>,
    Json(body): Json<ReadingPositionsBody>,
) -> Result<Json<Vec<ReadingPosition>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let positions = authority
        .reading_positions(
            &body.request,
            &body.workspace_id,
            &body.object_id,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(positions))
}

async fn put_draft_handoff(
    State(authority): State<SharedAuthority>,
    Json(body): Json<DraftHandoffPutBody>,
) -> Result<Json<DraftHandoffOutcome>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let outcome = authority
        .put_draft_handoff(&body.request, &body.input, Utc::now().timestamp_millis())
        .map_err(ApiError::from_sync)?;
    Ok(Json(outcome))
}

async fn draft_handoffs(
    State(authority): State<SharedAuthority>,
    Json(body): Json<DraftHandoffsBody>,
) -> Result<Json<Vec<DraftHandoff>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let handoffs = authority
        .draft_handoffs(
            &body.request,
            &body.workspace_id,
            &body.object_id,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(handoffs))
}

async fn adopt_draft_handoff(
    State(authority): State<SharedAuthority>,
    Json(body): Json<AdoptDraftHandoffBody>,
) -> Result<Json<DraftHandoffOutcome>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let outcome = authority
        .adopt_draft_handoff(
            &body.request,
            &body.workspace_id,
            &body.object_id,
            &body.handoff_id,
            Utc::now().timestamp_millis(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(outcome))
}
async fn create_pairing_invite(
    State(authority): State<SharedAuthority>,
    Json(body): Json<PairingInviteBody>,
) -> Result<Json<PairingInvite>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    let role = body
        .role
        .as_deref()
        .map(MemberRole::parse)
        .transpose()
        .map_err(ApiError::from_sync)?;
    if body.request.request_hash
        != pairing_invite_request_hash(
            body.lifetime_seconds,
            role.map(MemberRole::as_str),
            body.member_id.as_deref(),
            body.member_name.as_deref(),
        )
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let invite = if let Some(member_id) = body.member_id.as_deref() {
        authority.create_device_pairing_invite(&actor.device_id, body.lifetime_seconds, member_id)
    } else if let Some(member_name) = body.member_name.as_deref() {
        let role = role
            .ok_or_else(|| SyncError::InvalidRole("missing".into()))
            .map_err(ApiError::from_sync)?;
        authority.create_member_pairing_invite(
            &actor.device_id,
            body.lifetime_seconds,
            member_name,
            role,
        )
    } else {
        let role = role
            .ok_or_else(|| SyncError::InvalidRole("missing".into()))
            .map_err(ApiError::from_sync)?;
        authority.create_pairing_invite_with_role(&actor.device_id, body.lifetime_seconds, role)
    }
    .map_err(ApiError::from_sync)?;
    Ok(Json(invite))
}
async fn complete_pairing(
    State(authority): State<SharedAuthority>,
    Json(body): Json<PairingCompleteBody>,
) -> Result<Json<DeviceRecord>, ApiError> {
    validate_public_key(&body.public_key).map_err(ApiError::from_sync)?;
    let public_key = hex_decode(&body.public_key)
        .ok_or_else(|| ApiError::from_sync(SyncError::InvalidPublicKey))?;
    let mut authority = lock_authority(&authority)?;
    let device = authority
        .complete_pairing(
            &body.invite_id,
            &body.code,
            &public_key,
            &body.display_name,
            &body.proof,
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(device))
}

async fn devices(
    State(authority): State<SharedAuthority>,
    Json(body): Json<DevicesBody>,
) -> Result<Json<Vec<DeviceRecord>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != devices_request_hash() {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let devices = authority.list_devices().map_err(ApiError::from_sync)?;
    Ok(Json(devices))
}

async fn members(
    State(authority): State<SharedAuthority>,
    Json(body): Json<MembersBody>,
) -> Result<Json<Vec<MemberRecord>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != members_request_hash() {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let members = authority.list_members().map_err(ApiError::from_sync)?;
    Ok(Json(members))
}

async fn set_member_role(
    State(authority): State<SharedAuthority>,
    Json(body): Json<SetMemberRoleBody>,
) -> Result<Json<MemberRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    let role = MemberRole::parse(&body.role).map_err(ApiError::from_sync)?;
    if body.request.request_hash != set_member_role_request_hash(&body.member_id, role.as_str()) {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let member = authority
        .set_member_role(&actor.device_id, &body.member_id, role)
        .map_err(ApiError::from_sync)?;
    Ok(Json(member))
}

async fn remove_member(
    State(authority): State<SharedAuthority>,
    Json(body): Json<RemoveMemberBody>,
) -> Result<Json<MemberRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != remove_member_request_hash(&body.member_id, &body.reason) {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let member = authority
        .remove_member(&actor.device_id, &body.member_id, &body.reason)
        .map_err(ApiError::from_sync)?;
    Ok(Json(member))
}

async fn audit_log(
    State(authority): State<SharedAuthority>,
    Json(body): Json<AuditLogBody>,
) -> Result<Json<Vec<pkms_sync::AuditEntry>>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.limit == 0 || body.limit > 500 {
        return Err(ApiError::from_sync(SyncError::InvalidCursor));
    }
    if body.request.request_hash != audit_log_request_hash(body.limit) {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    if actor.role != MemberRole::Owner {
        return Err(ApiError::from_sync(SyncError::InsufficientRole));
    }
    let entries = authority
        .audit_log(body.limit)
        .map_err(ApiError::from_sync)?;
    Ok(Json(entries))
}
async fn revoke_device(
    State(authority): State<SharedAuthority>,
    Json(body): Json<RevokeDeviceBody>,
) -> Result<Json<DeviceRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != revoke_device_request_hash(&body.target_device_id, &body.reason)
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let device = authority
        .revoke_device(&actor.device_id, &body.target_device_id, &body.reason)
        .map_err(ApiError::from_sync)?;
    Ok(Json(device))
}

async fn set_device_role(
    State(authority): State<SharedAuthority>,
    Json(body): Json<SetDeviceRoleBody>,
) -> Result<Json<DeviceRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    let role = MemberRole::parse(&body.role).map_err(ApiError::from_sync)?;
    if body.request.request_hash
        != set_device_role_request_hash(&body.target_device_id, role.as_str())
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let actor = authority
        .authorize(&body.request, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    let device = authority
        .set_device_role(&actor.device_id, &body.target_device_id, role)
        .map_err(ApiError::from_sync)?;
    Ok(Json(device))
}
async fn register_encryption_key(
    State(authority): State<SharedAuthority>,
    Json(body): Json<RegisterEncryptionKeyBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != device_encryption_key_request_hash(&body.encryption_public_key)
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    authority
        .register_encryption_key(
            &body.request,
            &body.encryption_public_key,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(serde_json::json!({"registered": true})))
}

async fn rotate_workspace_key(
    State(authority): State<SharedAuthority>,
    Json(body): Json<RotateWorkspaceKeyBody>,
) -> Result<Json<i64>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Write {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash
        != workspace_key_rotation_request_hash(&body.workspace_id, body.new_epoch, &body.envelopes)
    {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let epoch = authority
        .rotate_workspace_key(
            &body.request,
            &body.workspace_id,
            body.new_epoch,
            &body.envelopes,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(epoch))
}

async fn fetch_key_envelope(
    State(authority): State<SharedAuthority>,
    Json(body): Json<KeyEnvelopeBody>,
) -> Result<Json<KeyEnvelope>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    if body.request.access != SyncAccess::Read {
        return Err(ApiError::from_sync(SyncError::InvalidAccess));
    }
    if body.request.request_hash != key_envelope_request_hash(&body.workspace_id, body.epoch) {
        return Err(ApiError::from_sync(SyncError::InvalidRequestHash));
    }
    let envelope = authority
        .fetch_key_envelope(
            &body.request,
            &body.workspace_id,
            body.epoch,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(envelope))
}

async fn blob_begin(
    State(authority): State<SharedAuthority>,
    Json(body): Json<BlobBeginBody>,
) -> Result<Json<BlobBegin>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let begin = authority
        .begin_blob(
            &body.request,
            &body.blob_hash,
            body.size,
            body.chunk_size,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(begin))
}

async fn blob_chunk(
    State(authority): State<SharedAuthority>,
    Json(body): Json<BlobChunkBody>,
) -> Result<Json<BlobStatus>, ApiError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let data = STANDARD.decode(body.data.as_bytes()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_blob_chunk",
            "chunk is not valid base64",
        )
    })?;
    let mut authority = lock_authority(&authority)?;
    let status = authority
        .upload_blob_chunk(
            &body.request,
            &body.blob_hash,
            body.index,
            &data,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(status))
}

async fn blob_status(
    State(authority): State<SharedAuthority>,
    Json(body): Json<BlobHashBody>,
) -> Result<Json<BlobStatus>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let status = authority
        .blob_status(&body.request, &body.blob_hash, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    Ok(Json(status))
}

async fn blob_finalize(
    State(authority): State<SharedAuthority>,
    Json(body): Json<BlobHashBody>,
) -> Result<Json<BlobRecord>, ApiError> {
    let mut authority = lock_authority(&authority)?;
    let record = authority
        .finalize_blob(&body.request, &body.blob_hash, Utc::now().timestamp())
        .map_err(ApiError::from_sync)?;
    Ok(Json(record))
}

async fn blob_read(
    State(authority): State<SharedAuthority>,
    Json(body): Json<BlobReadBody>,
) -> Result<Json<BlobChunkResponse>, ApiError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut authority = lock_authority(&authority)?;
    let data = authority
        .read_blob_chunk(
            &body.request,
            &body.blob_hash,
            body.index,
            Utc::now().timestamp(),
        )
        .map_err(ApiError::from_sync)?;
    Ok(Json(BlobChunkResponse {
        data: STANDARD.encode(data),
    }))
}
