//! G10 resume handoff: per-device reading checkpoints and append-only draft
//! handoffs scoped by `workspace_id + object_id`.
//!
//! Reading positions are retained per device so one screen size or device
//! cannot erase another device's place. Draft handoffs are immutable payloads:
//! a new draft gets a new `handoff_id`, and adoption marks it without deleting
//! the ciphertext. Both surfaces are device-authorized and never query across
//! workspace boundaries.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::crypto::{canonical, sha256_array, sha256_hex};
use crate::{SignedRequest, SyncAccess, SyncAuthority, SyncError};

pub const MAX_RESUME_ID_CHARS: usize = 200;
pub const MAX_RESUME_TITLE_CHARS: usize = 300;
pub const MAX_DRAFT_CIPHERTEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_LOCATOR_BLOCK_ID_CHARS: usize = 200;

/// A content locator owned by the client. `content_version` is kept beside it
/// so a desktop can warn when a position belongs to an older revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResumeLocator {
    /// A plain note/capture/HTML scroll surface, in CSS pixels.
    Scroll { position: u64 },
    /// A PDF page plus a vertical offset inside that page in thousandths.
    Pdf { page_index: u32, offset_milli: u32 },
    /// A source block plus a byte offset. The block id is stable in the parse.
    Text { block_id: String, offset: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingPositionInput {
    pub workspace_id: String,
    pub object_id: String,
    pub content_version: i64,
    pub locator: ResumeLocator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub workspace_id: String,
    pub object_id: String,
    pub device_id: String,
    pub content_version: i64,
    pub locator: ResumeLocator,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPositionOutcome {
    pub position: ReadingPosition,
    /// True when a same-device older checkpoint arrived after a newer one; the
    /// newer checkpoint is returned and nothing is overwritten.
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftHandoffInput {
    pub handoff_id: String,
    pub workspace_id: String,
    pub object_id: String,
    pub base_version: i64,
    pub base_revision_id: Option<String>,
    pub title: String,
    /// Client-encrypted document bytes. The authority never decrypts this.
    #[serde(with = "crate::crypto::base64_vec")]
    pub ciphertext: Vec<u8>,
    pub key_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftHandoff {
    pub handoff_id: String,
    pub workspace_id: String,
    pub object_id: String,
    pub device_id: String,
    pub base_version: i64,
    pub base_revision_id: Option<String>,
    pub title: String,
    #[serde(with = "crate::crypto::base64_vec")]
    pub ciphertext: Vec<u8>,
    pub payload_digest: String,
    pub key_epoch: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub adopted_at_ms: Option<i64>,
    pub adopted_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftHandoffOutcome {
    pub handoff: DraftHandoff,
    pub duplicate: bool,
}

impl SyncAuthority {
    pub fn put_reading_position(
        &mut self,
        request: &SignedRequest,
        input: &ReadingPositionInput,
        now_unix_ms: i64,
    ) -> Result<ReadingPositionOutcome, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_position(input)?;
        if request.request_hash != reading_position_put_request_hash(input) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request_without_role_check(request, now_unix_ms / 1000)?;
        self.consume_request_nonce(request, now_unix_ms / 1000)?;
        self.touch_device(&device.device_id, now_unix_ms / 1000)?;
        let locator_json = serde_json::to_string(&input.locator)?;
        self.connection.execute(
            "INSERT INTO sync_reading_positions(workspace_id,object_id,device_id,content_version,locator_json,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,object_id,device_id) DO UPDATE SET content_version=excluded.content_version,locator_json=excluded.locator_json,updated_at_ms=excluded.updated_at_ms WHERE excluded.content_version > sync_reading_positions.content_version OR (excluded.content_version = sync_reading_positions.content_version AND excluded.updated_at_ms >= sync_reading_positions.updated_at_ms)",
            params![
                input.workspace_id,
                input.object_id,
                device.device_id,
                input.content_version,
                locator_json,
                now_unix_ms
            ],
        )?;
        let position =
            self.reading_position(&input.workspace_id, &input.object_id, &device.device_id)?;
        let stale = position.updated_at_ms != now_unix_ms
            || position.content_version != input.content_version
            || position.locator != input.locator;
        Ok(ReadingPositionOutcome { position, stale })
    }

    pub fn reading_positions(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        object_id: &str,
        now_unix: i64,
    ) -> Result<Vec<ReadingPosition>, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_scope(workspace_id, object_id)?;
        if request.request_hash != reading_positions_request_hash(workspace_id, object_id) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let mut statement = self.connection.prepare(
            "SELECT workspace_id,object_id,device_id,content_version,locator_json,updated_at_ms FROM sync_reading_positions WHERE workspace_id=?1 AND object_id=?2 ORDER BY updated_at_ms DESC,device_id",
        )?;
        let rows = statement
            .query_map(params![workspace_id, object_id], position_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn put_draft_handoff(
        &mut self,
        request: &SignedRequest,
        input: &DraftHandoffInput,
        now_unix_ms: i64,
    ) -> Result<DraftHandoffOutcome, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_draft(input)?;
        let expected = draft_handoff_put_request_hash(input);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request_without_role_check(request, now_unix_ms / 1000)?;
        self.consume_request_nonce(request, now_unix_ms / 1000)?;
        self.touch_device(&device.device_id, now_unix_ms / 1000)?;
        let digest = sha256_hex(&draft_payload(input));
        if let Some(existing) = self.draft_handoff(&input.handoff_id)? {
            if existing.device_id == device.device_id
                && existing.workspace_id == input.workspace_id
                && existing.object_id == input.object_id
                && existing.payload_digest == digest
            {
                return Ok(DraftHandoffOutcome {
                    handoff: existing,
                    duplicate: true,
                });
            }
            return Err(SyncError::DraftIdReused);
        }
        self.connection.execute(
            "INSERT INTO sync_draft_handoffs(handoff_id,workspace_id,object_id,device_id,base_version,base_revision_id,title,ciphertext,payload_digest,key_epoch,created_at_ms,updated_at_ms,adopted_at_ms,adopted_by) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,NULL,NULL)",
            params![
                input.handoff_id,
                input.workspace_id,
                input.object_id,
                device.device_id,
                input.base_version,
                input.base_revision_id,
                input.title,
                input.ciphertext,
                digest,
                input.key_epoch,
                now_unix_ms
            ],
        )?;
        Ok(DraftHandoffOutcome {
            handoff: self
                .draft_handoff(&input.handoff_id)?
                .ok_or(SyncError::DraftNotFound)?,
            duplicate: false,
        })
    }

    pub fn draft_handoffs(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        object_id: &str,
        now_unix: i64,
    ) -> Result<Vec<DraftHandoff>, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_scope(workspace_id, object_id)?;
        if request.request_hash != draft_handoffs_request_hash(workspace_id, object_id) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let mut statement = self.connection.prepare(
            "SELECT handoff_id,workspace_id,object_id,device_id,base_version,base_revision_id,title,ciphertext,payload_digest,key_epoch,created_at_ms,updated_at_ms,adopted_at_ms,adopted_by FROM sync_draft_handoffs WHERE workspace_id=?1 AND object_id=?2 ORDER BY updated_at_ms DESC,handoff_id DESC",
        )?;
        let rows = statement
            .query_map(params![workspace_id, object_id], draft_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn adopt_draft_handoff(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        object_id: &str,
        handoff_id: &str,
        now_unix_ms: i64,
    ) -> Result<DraftHandoffOutcome, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_scope(workspace_id, object_id)?;
        validate_id("handoff_id", handoff_id)?;
        let expected = adopt_draft_request_hash(workspace_id, object_id, handoff_id);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request_without_role_check(request, now_unix_ms / 1000)?;
        self.consume_request_nonce(request, now_unix_ms / 1000)?;
        self.touch_device(&device.device_id, now_unix_ms / 1000)?;
        let existing = self
            .draft_handoff(handoff_id)?
            .filter(|draft| draft.workspace_id == workspace_id && draft.object_id == object_id)
            .ok_or(SyncError::DraftNotFound)?;
        if existing.adopted_at_ms.is_some() {
            return Ok(DraftHandoffOutcome {
                handoff: existing,
                duplicate: true,
            });
        }
        self.connection.execute(
            "UPDATE sync_draft_handoffs SET adopted_at_ms=?1,adopted_by=?2 WHERE handoff_id=?3 AND adopted_at_ms IS NULL",
            params![now_unix_ms, device.device_id, handoff_id],
        )?;
        Ok(DraftHandoffOutcome {
            handoff: self
                .draft_handoff(handoff_id)?
                .ok_or(SyncError::DraftNotFound)?,
            duplicate: false,
        })
    }

    fn reading_position(
        &self,
        workspace_id: &str,
        object_id: &str,
        device_id: &str,
    ) -> Result<ReadingPosition, SyncError> {
        self.connection
            .query_row(
                "SELECT workspace_id,object_id,device_id,content_version,locator_json,updated_at_ms FROM sync_reading_positions WHERE workspace_id=?1 AND object_id=?2 AND device_id=?3",
                params![workspace_id, object_id, device_id],
                position_from_row,
            )
            .map_err(SyncError::from)
    }

    fn draft_handoff(&self, handoff_id: &str) -> Result<Option<DraftHandoff>, SyncError> {
        self.connection
            .query_row(
                "SELECT handoff_id,workspace_id,object_id,device_id,base_version,base_revision_id,title,ciphertext,payload_digest,key_epoch,created_at_ms,updated_at_ms,adopted_at_ms,adopted_by FROM sync_draft_handoffs WHERE handoff_id=?1",
                [handoff_id],
                draft_from_row,
            )
            .optional()
            .map_err(SyncError::from)
    }
}

/// Chooses the most recently updated checkpoint without discarding the other
/// device rows. Ties are deterministic by device id.
pub fn latest_reading_position(positions: &[ReadingPosition]) -> Option<&ReadingPosition> {
    positions.iter().max_by(|left, right| {
        (left.updated_at_ms, left.device_id.as_str())
            .cmp(&(right.updated_at_ms, right.device_id.as_str()))
    })
}

pub fn reading_position_put_request_hash(input: &ReadingPositionInput) -> [u8; 32] {
    let locator = serde_json::to_vec(&input.locator).unwrap_or_default();
    sha256_array(&canonical(&[
        b"spanseek-sync-reading-position-put-v1",
        input.workspace_id.as_bytes(),
        input.object_id.as_bytes(),
        &input.content_version.to_be_bytes(),
        &locator,
    ]))
}

pub fn reading_positions_request_hash(workspace_id: &str, object_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-reading-positions-v1",
        workspace_id.as_bytes(),
        object_id.as_bytes(),
    ]))
}

pub fn draft_handoff_put_request_hash(input: &DraftHandoffInput) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-draft-put-v1",
        &draft_payload(input),
    ]))
}

pub fn draft_handoffs_request_hash(workspace_id: &str, object_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-draft-list-v1",
        workspace_id.as_bytes(),
        object_id.as_bytes(),
    ]))
}

pub fn adopt_draft_request_hash(workspace_id: &str, object_id: &str, handoff_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-draft-adopt-v1",
        workspace_id.as_bytes(),
        object_id.as_bytes(),
        handoff_id.as_bytes(),
    ]))
}

/// AAD helper for client-side encryption of a handoff payload. The handoff id
/// and object identity are bound into the ciphertext.
pub fn draft_handoff_aad(
    workspace_id: &str,
    object_id: &str,
    handoff_id: &str,
    key_epoch: i64,
) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-draft-aad-v1",
        workspace_id.as_bytes(),
        object_id.as_bytes(),
        handoff_id.as_bytes(),
        &key_epoch.to_be_bytes(),
    ])
}

fn draft_payload(input: &DraftHandoffInput) -> Vec<u8> {
    canonical(&[
        input.handoff_id.as_bytes(),
        input.workspace_id.as_bytes(),
        input.object_id.as_bytes(),
        &input.base_version.to_be_bytes(),
        input.base_revision_id.as_deref().unwrap_or("").as_bytes(),
        input.title.as_bytes(),
        &input.ciphertext,
        &input.key_epoch.to_be_bytes(),
    ])
}

fn position_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReadingPosition> {
    let locator_json: String = row.get(4)?;
    let locator = serde_json::from_str(&locator_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ReadingPosition {
        workspace_id: row.get(0)?,
        object_id: row.get(1)?,
        device_id: row.get(2)?,
        content_version: row.get(3)?,
        locator,
        updated_at_ms: row.get(5)?,
    })
}

fn draft_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DraftHandoff> {
    Ok(DraftHandoff {
        handoff_id: row.get(0)?,
        workspace_id: row.get(1)?,
        object_id: row.get(2)?,
        device_id: row.get(3)?,
        base_version: row.get(4)?,
        base_revision_id: row.get(5)?,
        title: row.get(6)?,
        ciphertext: row.get(7)?,
        payload_digest: row.get(8)?,
        key_epoch: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
        adopted_at_ms: row.get(12)?,
        adopted_by: row.get(13)?,
    })
}

fn validate_scope(workspace_id: &str, object_id: &str) -> Result<(), SyncError> {
    validate_id("workspace_id", workspace_id)?;
    validate_id("object_id", object_id)
}

fn validate_position(input: &ReadingPositionInput) -> Result<(), SyncError> {
    validate_scope(&input.workspace_id, &input.object_id)?;
    if input.content_version < 0 {
        return Err(SyncError::InvalidResume(
            "content_version must not be negative".into(),
        ));
    }
    if let ResumeLocator::Text { block_id, .. } = &input.locator {
        validate_id("block_id", block_id)?;
        if block_id.chars().count() > MAX_LOCATOR_BLOCK_ID_CHARS {
            return Err(SyncError::InvalidResume("block_id is too long".into()));
        }
    }
    Ok(())
}

fn validate_draft(input: &DraftHandoffInput) -> Result<(), SyncError> {
    validate_id("handoff_id", &input.handoff_id)?;
    validate_scope(&input.workspace_id, &input.object_id)?;
    if input.base_version < 0 {
        return Err(SyncError::InvalidResume(
            "base_version must not be negative".into(),
        ));
    }
    if let Some(revision) = &input.base_revision_id {
        validate_id("base_revision_id", revision)?;
    }
    if input.title.chars().count() > MAX_RESUME_TITLE_CHARS {
        return Err(SyncError::InvalidResume("draft title is too long".into()));
    }
    if input.ciphertext.is_empty() {
        return Err(SyncError::InvalidResume(
            "draft ciphertext must not be empty".into(),
        ));
    }
    if input.ciphertext.len() > MAX_DRAFT_CIPHERTEXT_BYTES {
        return Err(SyncError::InvalidResume(
            "draft ciphertext is too large".into(),
        ));
    }
    if input.key_epoch < 1 {
        return Err(SyncError::InvalidResume(
            "draft key_epoch must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_id(name: &str, value: &str) -> Result<(), SyncError> {
    if value.is_empty() || value.chars().count() > MAX_RESUME_ID_CHARS {
        return Err(SyncError::InvalidResume(format!("invalid {name}")));
    }
    Ok(())
}
