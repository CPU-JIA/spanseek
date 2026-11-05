//! Per-library chunking parameters with the same versioned, idempotent receipt
//! contract as the retrieval and cleaning settings stored in the same row. The
//! chunk cap and overlap are part of the embedding profile identity, so saving
//! them rebuilds the index and the replaced profile is remembered for rollback.
use crate::*;
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const DEFAULT_CHUNK_TOKENS: u32 = 512;
pub const MIN_CHUNK_TOKENS: u32 = 64;
pub const MAX_CHUNK_TOKENS: u32 = 1024;
pub const DEFAULT_OVERLAP_PERCENT: u32 = 15;
pub const MAX_OVERLAP_PERCENT: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChunkParams {
    /// Upper bound of one indexed chunk, in estimated tokens.
    pub chunk_tokens: u32,
    /// Overlap between consecutive chunks of one block, in percent of the cap.
    pub overlap_percent: u32,
}
impl Default for ChunkParams {
    fn default() -> Self {
        Self {
            chunk_tokens: DEFAULT_CHUNK_TOKENS,
            overlap_percent: DEFAULT_OVERLAP_PERCENT,
        }
    }
}

pub fn validate_chunk_params(params: &ChunkParams) -> Result<()> {
    if !(MIN_CHUNK_TOKENS..=MAX_CHUNK_TOKENS).contains(&params.chunk_tokens)
        || params.overlap_percent > MAX_OVERLAP_PERCENT
    {
        return Err(Error::InvalidInput("invalid chunk params"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChunkSettings {
    pub params: ChunkParams,
    pub row_version: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SetChunkParams {
    pub base_row_version: u32,
    pub params: ChunkParams,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChunkSettingsMutation {
    pub settings: ChunkSettings,
    pub receipt: crate::MetadataReceipt,
}
impl crate::mutations::RecordedResult for ChunkSettingsMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}

pub(crate) fn chunk_settings(db: &Connection) -> Result<ChunkSettings> {
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT chunk_params_json,chunk_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let params = match json {
        Some(json) => serde_json::from_str(&json)?,
        None => ChunkParams::default(),
    };
    validate_chunk_params(&params)?;
    Ok(ChunkSettings {
        params,
        row_version,
    })
}

/// Backups from before schema 23 have no column to check; a stored value must
/// always be internally consistent with its row version.
pub(crate) fn validate_chunk_settings_backup(db: &Connection) -> Result<()> {
    let schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema < 23 {
        return Ok(());
    }
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT chunk_params_json,chunk_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match json {
        Some(json) => {
            let params: ChunkParams =
                serde_json::from_str(&json).map_err(|_| Error::IntegrityFailure)?;
            validate_chunk_params(&params).map_err(|_| Error::IntegrityFailure)?;
            if row_version == 0 {
                return Err(Error::IntegrityFailure);
            }
        }
        None if row_version != 0 => return Err(Error::IntegrityFailure),
        None => {}
    }
    Ok(())
}

impl Library {
    pub fn chunk_settings(&self) -> Result<ChunkSettings> {
        chunk_settings(&self.connection)
    }

    /// Saves the chunk cap and overlap. The caller re-registers the embedding
    /// profile afterwards, which is what actually rebuilds the index.
    pub fn set_chunk_params(
        &mut self,
        command: &str,
        request: SetChunkParams,
    ) -> Result<ChunkSettingsMutation> {
        let kind = "chunk.configure";
        validate_chunk_params(&request.params)?;
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let old = chunk_settings(&tx)?;
        if old.row_version != request.base_row_version {
            return Err(Error::MetadataConflict {
                object_id: "chunk_settings".into(),
                expected: request.base_row_version,
                actual: old.row_version,
            });
        }
        let row_version = old
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput("chunk settings version overflow"))?;
        tx.execute(
            "UPDATE search_settings SET chunk_params_json=?1,chunk_row_version=?2 WHERE singleton=1",
            params![serde_json::to_string(&request.params)?, row_version],
        )?;
        let result = ChunkSettingsMutation {
            settings: ChunkSettings {
                params: request.params,
                row_version,
            },
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(&tx, &hash, kind, "chunk_settings", &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    /// The profile the active one replaced, when the index has not been reverted
    /// yet. Rolling back never re-embeds anything: the old generation rows for
    /// that profile are still in the library.
    pub fn previous_embedding_profile(&self) -> Result<Option<EmbeddingProfile>> {
        let id: Option<String> = self.connection.query_row(
            "SELECT previous_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let Some(id) = id else { return Ok(None) };
        let active: Option<String> = self.connection.query_row(
            "SELECT active_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if active.as_deref() == Some(id.as_str()) {
            return Ok(None);
        }
        crate::index::profile(&self.connection, &id).map(Some)
    }

    /// Re-activates the profile the current one replaced and marks every indexable
    /// object dirty so the index points back at the existing generations.
    pub fn revert_embedding_profile(&mut self, command: &str) -> Result<EmbeddingProfileMutation> {
        let kind = "search.profile_revert";
        let hash = mutations::request_hash(command, kind, &())?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let active: Option<String> = tx.query_row(
            "SELECT active_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let previous_id: Option<String> = tx.query_row(
            "SELECT previous_profile_id FROM search_settings WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let previous_id = previous_id
            .filter(|id| active.as_deref() != Some(id.as_str()))
            .ok_or(Error::InvalidInput("no previous embedding profile"))?;
        let previous = crate::index::profile(&tx, &previous_id)?;
        tx.execute(
            "UPDATE search_settings SET active_profile_id=?1,previous_profile_id=?2 WHERE singleton=1",
            params![previous.id, active],
        )?;
        tx.execute("INSERT INTO index_dirty_objects(object_id) SELECT id FROM objects WHERE kind IN ('note','source') ON CONFLICT(object_id) DO UPDATE SET attempts=0,next_retry_at=NULL,last_error=NULL",[])?;
        let result = EmbeddingProfileMutation {
            profile: previous,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            kind,
            &result.profile.id,
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}
