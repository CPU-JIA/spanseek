//! Blocking versioned-protocol client for the spanseek sync authority.
//!
//! The client owns one device key pair and signs every read or write request.
//! It intentionally performs no local database work: callers decide when to
//! decrypt, merge and persist a pulled batch.

use std::time::{SystemTime, UNIX_EPOCH};

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
    ApplyOutcome, AuditEntry, BlobBegin, BlobRecord, BlobStatus, ConflictRecord,
    ConflictResolution, DeviceEncryptionKey, DeviceKeyPair, DeviceRecord, DraftHandoff,
    DraftHandoffInput, DraftHandoffOutcome, KeyEnvelope, MAX_PULL_LIMIT, MemberRecord, MemberRole,
    ObjectState, PairingInvite, PullBatch, ReadingPosition, ReadingPositionInput,
    ReadingPositionOutcome, SignedRequest, SyncAccess, SyncOp, TrashEntry,
    adopt_draft_request_hash, blob_begin_request_hash, blob_chunk_request_hash,
    blob_finalize_request_hash, blob_read_request_hash, blob_status_request_hash,
    device_encryption_key_request_hash, draft_handoff_put_request_hash,
    draft_handoffs_request_hash, key_envelope_request_hash, op_request_hash, pairing_proof_message,
    pull_request_hash, pull_wait_request_hash, reading_position_put_request_hash,
    reading_positions_request_hash, resolve_conflict_request_hash, sha256_hex,
    snapshot_request_hash, trash_request_hash, workspace_key_rotation_request_hash,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid client input: {0}")]
    InvalidInput(String),
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("HTTP request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("server rejected request (HTTP {status}, {code}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("invalid server response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("server returned HTTP {0} without a structured error")]
    UnexpectedStatus(u16),
}

pub struct SyncClient {
    base_url: String,
    device_id: String,
    device: DeviceKeyPair,
    http: reqwest::blocking::Client,
}

impl SyncClient {
    pub fn new(
        base_url: impl Into<String>,
        device_id: impl Into<String>,
        device: DeviceKeyPair,
    ) -> Result<Self, ClientError> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|error| ClientError::InvalidBaseUrl(error.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ClientError::InvalidBaseUrl(
                "expected an http(s) origin without query or fragment".into(),
            ));
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("spanseek-sync-client/1")
            .build()?;
        Ok(Self {
            base_url,
            device_id: device_id.into(),
            device,
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn health(&self) -> Result<HealthBody, ClientError> {
        // Health is rendered by the always-mounted shell. Keep it fast even
        // when a configured endpoint is unreachable; general sync operations
        // still use the client-wide 20 second timeout.
        let response = self
            .http
            .get(format!("{}/healthz", self.base_url))
            .timeout(std::time::Duration::from_millis(1_500))
            .send()?;
        self.decode_response(response)
    }

    pub fn create_pairing_invite(
        &self,
        lifetime_seconds: i64,
    ) -> Result<PairingInvite, ClientError> {
        self.create_pairing_invite_with_role(lifetime_seconds, MemberRole::Editor)
    }

    pub fn create_pairing_invite_with_role(
        &self,
        lifetime_seconds: i64,
        role: MemberRole,
    ) -> Result<PairingInvite, ClientError> {
        self.create_pairing_invite_target(lifetime_seconds, Some(role), None, None)
    }

    pub fn create_member_pairing_invite(
        &self,
        lifetime_seconds: i64,
        member_name: &str,
        role: MemberRole,
    ) -> Result<PairingInvite, ClientError> {
        self.create_pairing_invite_target(lifetime_seconds, Some(role), None, Some(member_name))
    }

    pub fn create_device_pairing_invite(
        &self,
        lifetime_seconds: i64,
        member_id: &str,
        role: MemberRole,
    ) -> Result<PairingInvite, ClientError> {
        self.create_pairing_invite_target(lifetime_seconds, Some(role), Some(member_id), None)
    }

    fn create_pairing_invite_target(
        &self,
        lifetime_seconds: i64,
        role: Option<MemberRole>,
        member_id: Option<&str>,
        member_name: Option<&str>,
    ) -> Result<PairingInvite, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            pairing_invite_request_hash(
                lifetime_seconds,
                role.map(MemberRole::as_str),
                member_id,
                member_name,
            ),
        );
        self.post(
            "/v1/pairing/invites",
            &PairingInviteBody {
                request,
                lifetime_seconds,
                role: role.map(|role| role.as_str().to_owned()),
                member_id: member_id.map(ToOwned::to_owned),
                member_name: member_name.map(ToOwned::to_owned),
            },
        )
    }
    pub fn pair_with_code(
        &self,
        invite_id: &str,
        code: &str,
        display_name: &str,
    ) -> Result<DeviceRecord, ClientError> {
        let proof = self.device.sign(&pairing_proof_message(
            invite_id,
            code,
            &self.device.public_key_hex(),
        ));
        self.post(
            "/v1/pairing/complete",
            &PairingCompleteBody {
                invite_id: invite_id.to_owned(),
                code: code.to_owned(),
                public_key: self.device.public_key_hex(),
                display_name: display_name.to_owned(),
                proof,
            },
        )
    }

    pub fn devices(&self) -> Result<Vec<DeviceRecord>, ClientError> {
        let request = self.signed(SyncAccess::Read, devices_request_hash());
        self.post("/v1/devices/list", &DevicesBody { request })
    }

    pub fn members(&self) -> Result<Vec<MemberRecord>, ClientError> {
        let request = self.signed(SyncAccess::Read, members_request_hash());
        self.post("/v1/members/list", &MembersBody { request })
    }

    pub fn set_member_role(
        &self,
        member_id: &str,
        role: MemberRole,
    ) -> Result<MemberRecord, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            set_member_role_request_hash(member_id, role.as_str()),
        );
        self.post(
            "/v1/members/role",
            &SetMemberRoleBody {
                request,
                member_id: member_id.to_owned(),
                role: role.as_str().to_owned(),
            },
        )
    }

    pub fn remove_member(
        &self,
        member_id: &str,
        reason: &str,
    ) -> Result<MemberRecord, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            remove_member_request_hash(member_id, reason),
        );
        self.post(
            "/v1/members/remove",
            &RemoveMemberBody {
                request,
                member_id: member_id.to_owned(),
                reason: reason.to_owned(),
            },
        )
    }

    pub fn audit_log(&self, limit: usize) -> Result<Vec<AuditEntry>, ClientError> {
        if limit == 0 || limit > 500 {
            return Err(ClientError::InvalidInput(
                "audit limit must be between 1 and 500".into(),
            ));
        }
        let request = self.signed(SyncAccess::Read, audit_log_request_hash(limit));
        self.post("/v1/audit", &AuditLogBody { request, limit })
    }

    pub fn revoke_device(
        &self,
        target_device_id: &str,
        reason: &str,
    ) -> Result<DeviceRecord, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            revoke_device_request_hash(target_device_id, reason),
        );
        self.post(
            "/v1/devices/revoke",
            &RevokeDeviceBody {
                request,
                target_device_id: target_device_id.to_owned(),
                reason: reason.to_owned(),
            },
        )
    }

    pub fn set_device_role(
        &self,
        target_device_id: &str,
        role: MemberRole,
    ) -> Result<DeviceRecord, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            set_device_role_request_hash(target_device_id, role.as_str()),
        );
        self.post(
            "/v1/devices/role",
            &SetDeviceRoleBody {
                request,
                target_device_id: target_device_id.to_owned(),
                role: role.as_str().to_owned(),
            },
        )
    }
    pub fn register_encryption_key(&self, encryption_public_key: &str) -> Result<(), ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            device_encryption_key_request_hash(encryption_public_key),
        );
        let _: serde_json::Value = self.post(
            "/v1/keys/device",
            &RegisterEncryptionKeyBody {
                request,
                encryption_public_key: encryption_public_key.to_owned(),
            },
        )?;
        Ok(())
    }

    pub fn rotate_workspace_key(
        &self,
        workspace_id: &str,
        new_epoch: i64,
        envelopes: &[(String, KeyEnvelope)],
    ) -> Result<i64, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            workspace_key_rotation_request_hash(workspace_id, new_epoch, envelopes),
        );
        self.post(
            "/v1/keys/rotate",
            &RotateWorkspaceKeyBody {
                request,
                workspace_id: workspace_id.to_owned(),
                new_epoch,
                envelopes: envelopes.to_vec(),
            },
        )
    }

    pub fn fetch_key_envelope(
        &self,
        workspace_id: &str,
        epoch: i64,
    ) -> Result<KeyEnvelope, ClientError> {
        let request = self.signed(
            SyncAccess::Read,
            key_envelope_request_hash(workspace_id, epoch),
        );
        self.post(
            "/v1/keys/envelope",
            &KeyEnvelopeBody {
                request,
                workspace_id: workspace_id.to_owned(),
                epoch,
            },
        )
    }
    pub fn apply_op(&self, op: &SyncOp) -> Result<ApplyOutcome, ClientError> {
        let request = self.signed(SyncAccess::Write, op_request_hash(op));
        self.post(
            "/v1/ops",
            &ApplyOpBody {
                request,
                op: op.clone(),
            },
        )
    }

    pub fn pull(
        &self,
        workspace_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<PullBatch, ClientError> {
        if after_seq < 0 || limit == 0 || limit > MAX_PULL_LIMIT {
            return Err(ClientError::InvalidInput(
                "pull requires after_seq >= 0 and 1 <= limit <= MAX_PULL_LIMIT".into(),
            ));
        }
        let request = self.signed(
            SyncAccess::Read,
            pull_request_hash(workspace_id, after_seq, limit),
        );
        self.post(
            "/v1/pull",
            &PullBody {
                request,
                workspace_id: workspace_id.to_owned(),
                after_seq,
                limit,
            },
        )
    }

    pub fn wait_for_events(
        &self,
        workspace_id: &str,
        after_seq: i64,
        limit: usize,
        timeout_ms: u64,
    ) -> Result<PullBatch, ClientError> {
        if after_seq < 0 || limit == 0 || limit > MAX_PULL_LIMIT || timeout_ms > 60_000 {
            return Err(ClientError::InvalidInput(
                "wait_for_events requires after_seq >= 0, 1 <= limit <= MAX_PULL_LIMIT and timeout_ms <= 60000".into(),
            ));
        }
        let request = self.signed(
            SyncAccess::Read,
            pull_wait_request_hash(workspace_id, after_seq, limit, timeout_ms),
        );
        self.post(
            "/v1/pull/wait",
            &PullWaitBody {
                request,
                workspace_id: workspace_id.to_owned(),
                after_seq,
                limit,
                timeout_ms,
            },
        )
    }

    pub fn snapshot(&self, workspace_id: &str) -> Result<Vec<ObjectState>, ClientError> {
        let request = self.signed(SyncAccess::Read, snapshot_request_hash(workspace_id));
        self.post(
            "/v1/snapshot",
            &SnapshotBody {
                request,
                workspace_id: workspace_id.to_owned(),
            },
        )
    }

    pub fn trash(&self, workspace_id: &str) -> Result<Vec<TrashEntry>, ClientError> {
        let request = self.signed(SyncAccess::Read, trash_request_hash(workspace_id));
        self.post(
            "/v1/trash",
            &TrashBody {
                request,
                workspace_id: workspace_id.to_owned(),
            },
        )
    }

    pub fn resolve_conflict(
        &self,
        conflict_id: &str,
        resolution: ConflictResolution,
        replacement_op_id: Option<&str>,
        replacement_object_id: Option<&str>,
    ) -> Result<ConflictRecord, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            resolve_conflict_request_hash(
                conflict_id,
                resolution,
                replacement_op_id,
                replacement_object_id,
            ),
        );
        self.post(
            "/v1/conflicts/resolve",
            &ResolveConflictBody {
                request,
                conflict_id: conflict_id.to_owned(),
                resolution,
                replacement_op_id: replacement_op_id.map(str::to_owned),
                replacement_object_id: replacement_object_id.map(str::to_owned),
            },
        )
    }

    pub fn grant_workspace_key(
        &self,
        workspace_id: &str,
        epoch: i64,
        device_id: &str,
        envelope: &KeyEnvelope,
    ) -> Result<(), ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            grant_workspace_key_request_hash(workspace_id, epoch, device_id, envelope),
        );
        let _: serde_json::Value = self.post(
            "/v1/keys/grant",
            &GrantWorkspaceKeyBody {
                request,
                workspace_id: workspace_id.to_owned(),
                epoch,
                device_id: device_id.to_owned(),
                envelope: envelope.clone(),
            },
        )?;
        Ok(())
    }

    pub fn current_key_epoch(&self, workspace_id: &str) -> Result<Option<i64>, ClientError> {
        let request = self.signed(
            SyncAccess::Read,
            current_key_epoch_request_hash(workspace_id),
        );
        self.post(
            "/v1/keys/epoch",
            &CurrentKeyEpochBody {
                request,
                workspace_id: workspace_id.to_owned(),
            },
        )
    }

    pub fn device_encryption_keys(&self) -> Result<Vec<DeviceEncryptionKey>, ClientError> {
        let request = self.signed(SyncAccess::Read, device_encryption_keys_request_hash());
        self.post("/v1/keys/devices", &DeviceEncryptionKeysBody { request })
    }

    pub fn put_reading_position(
        &self,
        input: &ReadingPositionInput,
    ) -> Result<ReadingPositionOutcome, ClientError> {
        let request = self.signed(SyncAccess::Write, reading_position_put_request_hash(input));
        self.post(
            "/v1/reading-positions/put",
            &ReadingPositionPutBody {
                request,
                input: input.clone(),
            },
        )
    }

    pub fn reading_positions(
        &self,
        workspace_id: &str,
        object_id: &str,
    ) -> Result<Vec<ReadingPosition>, ClientError> {
        let request = self.signed(
            SyncAccess::Read,
            reading_positions_request_hash(workspace_id, object_id),
        );
        self.post(
            "/v1/reading-positions/list",
            &ReadingPositionsBody {
                request,
                workspace_id: workspace_id.to_owned(),
                object_id: object_id.to_owned(),
            },
        )
    }

    pub fn blob_begin(
        &self,
        blob_hash: &str,
        size: i64,
        chunk_size: i64,
    ) -> Result<BlobBegin, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            blob_begin_request_hash(blob_hash, size, chunk_size),
        );
        self.post(
            "/v1/blobs/begin",
            &BlobBeginBody {
                request,
                blob_hash: blob_hash.to_owned(),
                size,
                chunk_size,
            },
        )
    }

    pub fn blob_chunk(
        &self,
        blob_hash: &str,
        index: u32,
        data: &[u8],
    ) -> Result<BlobStatus, ClientError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let request = self.signed(
            SyncAccess::Write,
            blob_chunk_request_hash(blob_hash, index, data.len() as i64, &sha256_hex(data)),
        );
        self.post(
            "/v1/blobs/chunk",
            &BlobChunkBody {
                request,
                blob_hash: blob_hash.to_owned(),
                index,
                data: STANDARD.encode(data),
            },
        )
    }

    pub fn blob_status(&self, blob_hash: &str) -> Result<BlobStatus, ClientError> {
        let request = self.signed(SyncAccess::Read, blob_status_request_hash(blob_hash));
        self.post(
            "/v1/blobs/status",
            &BlobHashBody {
                request,
                blob_hash: blob_hash.to_owned(),
            },
        )
    }

    pub fn blob_finalize(&self, blob_hash: &str) -> Result<BlobRecord, ClientError> {
        let request = self.signed(SyncAccess::Write, blob_finalize_request_hash(blob_hash));
        self.post(
            "/v1/blobs/finalize",
            &BlobHashBody {
                request,
                blob_hash: blob_hash.to_owned(),
            },
        )
    }

    pub fn blob_read(&self, blob_hash: &str, index: u32) -> Result<Vec<u8>, ClientError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let request = self.signed(SyncAccess::Read, blob_read_request_hash(blob_hash, index));
        let response: BlobChunkResponse = self.post(
            "/v1/blobs/read",
            &BlobReadBody {
                request,
                blob_hash: blob_hash.to_owned(),
                index,
            },
        )?;
        STANDARD
            .decode(response.data.as_bytes())
            .map_err(|_| ClientError::InvalidInput("server blob chunk is not valid base64".into()))
    }

    pub fn put_draft_handoff(
        &self,
        input: &DraftHandoffInput,
    ) -> Result<DraftHandoffOutcome, ClientError> {
        let request = self.signed(SyncAccess::Write, draft_handoff_put_request_hash(input));
        self.post(
            "/v1/draft-handoffs/put",
            &DraftHandoffPutBody {
                request,
                input: input.clone(),
            },
        )
    }

    pub fn draft_handoffs(
        &self,
        workspace_id: &str,
        object_id: &str,
    ) -> Result<Vec<DraftHandoff>, ClientError> {
        let request = self.signed(
            SyncAccess::Read,
            draft_handoffs_request_hash(workspace_id, object_id),
        );
        self.post(
            "/v1/draft-handoffs/list",
            &DraftHandoffsBody {
                request,
                workspace_id: workspace_id.to_owned(),
                object_id: object_id.to_owned(),
            },
        )
    }

    pub fn adopt_draft_handoff(
        &self,
        workspace_id: &str,
        object_id: &str,
        handoff_id: &str,
    ) -> Result<DraftHandoffOutcome, ClientError> {
        let request = self.signed(
            SyncAccess::Write,
            adopt_draft_request_hash(workspace_id, object_id, handoff_id),
        );
        self.post(
            "/v1/draft-handoffs/adopt",
            &AdoptDraftHandoffBody {
                request,
                workspace_id: workspace_id.to_owned(),
                object_id: object_id.to_owned(),
                handoff_id: handoff_id.to_owned(),
            },
        )
    }

    fn signed(&self, access: SyncAccess, request_hash: [u8; 32]) -> SignedRequest {
        let issued_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let nonce = Uuid::new_v4().simple().to_string();
        self.device
            .sign_request(&self.device_id, access, &nonce, issued_at, request_hash)
    }

    fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, ClientError> {
        let response = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .json(body)
            .send()?;
        self.decode_response(response)
    }

    fn decode_response<R: DeserializeOwned>(
        &self,
        response: reqwest::blocking::Response,
    ) -> Result<R, ClientError> {
        let status = response.status();
        let bytes = response.bytes()?;
        if !status.is_success() {
            if let Ok(error) = serde_json::from_slice::<ApiErrorBody>(&bytes) {
                return Err(ClientError::Api {
                    status: status.as_u16(),
                    code: error.code,
                    message: error.message,
                });
            }
            return Err(ClientError::UnexpectedStatus(status.as_u16()));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}
