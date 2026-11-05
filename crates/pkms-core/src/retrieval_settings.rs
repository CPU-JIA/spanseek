//! Per-library retrieval parameters with the same versioned, idempotent
//! receipt contract as the generation profile stored in the same row.
use crate::*;
use rusqlite::{Connection, TransactionBehavior, params};

pub(crate) fn retrieval_settings(db: &Connection) -> Result<RetrievalSettings> {
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT retrieval_params_json,retrieval_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let params = match json {
        Some(json) => serde_json::from_str(&json)?,
        None => RetrievalParams::default(),
    };
    validate_retrieval_params(&params)?;
    Ok(RetrievalSettings {
        params,
        row_version,
    })
}

/// Backups from before schema 18 have no column to check; a stored value must
/// always be internally consistent with its row version.
pub(crate) fn validate_retrieval_settings_backup(db: &Connection) -> Result<()> {
    let schema: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema < 18 {
        return Ok(());
    }
    let (json, row_version): (Option<String>, u32) = db.query_row(
        "SELECT retrieval_params_json,retrieval_row_version FROM search_settings WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match json {
        Some(json) => {
            let params: RetrievalParams =
                serde_json::from_str(&json).map_err(|_| Error::IntegrityFailure)?;
            validate_retrieval_params(&params).map_err(|_| Error::IntegrityFailure)?;
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
    pub fn retrieval_settings(&self) -> Result<RetrievalSettings> {
        retrieval_settings(&self.connection)
    }

    pub fn set_retrieval_params(
        &mut self,
        command: &str,
        request: SetRetrievalParams,
    ) -> Result<RetrievalSettingsMutation> {
        let kind = "retrieval.configure";
        validate_retrieval_params(&request.params)?;
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(r);
        }
        let old = retrieval_settings(&tx)?;
        if old.row_version != request.base_row_version {
            return Err(Error::MetadataConflict {
                object_id: "retrieval_settings".into(),
                expected: request.base_row_version,
                actual: old.row_version,
            });
        }
        let row_version = old
            .row_version
            .checked_add(1)
            .ok_or(Error::InvalidInput("retrieval settings version overflow"))?;
        tx.execute(
            "UPDATE search_settings SET retrieval_params_json=?1,retrieval_row_version=?2 WHERE singleton=1",
            params![serde_json::to_string(&request.params)?, row_version],
        )?;
        let result = RetrievalSettingsMutation {
            settings: RetrievalSettings {
                params: request.params,
                row_version,
            },
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            kind,
            "retrieval_settings",
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}
