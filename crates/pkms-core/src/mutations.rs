//! Receipts shared by the three metadata command families.
use crate::{
    AuditUndoCommit, CanvasCommit, ChangeItemCommit, ChangeSetCommit, CollectionCommit, Error,
    MetadataCommit, MetadataReceipt, ResearchCommit, Result, ReviewSettingsCommit, ViewCommit,
    document::valid_uuid,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

pub(crate) trait RecordedResult: Serialize + DeserializeOwned {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt;
}
impl RecordedResult for MetadataCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for crate::TagManagementCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for CollectionCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for ViewCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for CanvasCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for ChangeSetCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for ChangeItemCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for AuditUndoCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for ResearchCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}
impl RecordedResult for ReviewSettingsCommit {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}

pub(crate) fn request_hash(
    command_id: &str,
    kind: &str,
    request: &impl Serialize,
) -> Result<String> {
    valid_uuid(command_id)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(1, kind, request))?)
    ))
}

pub(crate) fn replay<T: RecordedResult>(
    connection: &Connection,
    command_id: &str,
    hash: &str,
    kind: &str,
) -> Result<Option<T>> {
    let row = connection.query_row(
        "SELECT payload_hash,command_kind,result_json,change_seq,committed_at FROM command_receipts WHERE command_id=?1",
        [command_id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?)),
    ).optional()?;
    let Some((expected, saved_kind, json, seq, committed_at)) = row else {
        return Ok(None);
    };
    if expected != hash {
        return Err(Error::IdempotencyMismatch);
    }
    if saved_kind == crate::trash::PURGED_RECEIPT_KIND {
        return Err(Error::NotFound);
    }
    if saved_kind != kind {
        return Err(Error::IdempotencyMismatch);
    }
    let mut result: T = serde_json::from_str(&json.ok_or(Error::IntegrityFailure)?)
        .map_err(|_| Error::IntegrityFailure)?;
    let receipt = result.receipt_mut();
    if receipt.command_id != command_id
        || receipt.change_seq != seq.to_string()
        || receipt.committed_at != committed_at
    {
        return Err(Error::IntegrityFailure);
    }
    receipt.replayed = true;
    Ok(Some(result))
}

pub(crate) fn next_receipt(connection: &Connection, command_id: &str) -> Result<MetadataReceipt> {
    let seq: i64 = connection.query_row(
        "SELECT next_change_seq FROM workspace_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let next = seq
        .checked_add(1)
        .ok_or(Error::InvalidInput("change sequence overflow"))?;
    connection.execute(
        "UPDATE workspace_meta SET next_change_seq=?1 WHERE singleton=1",
        [next],
    )?;
    Ok(MetadataReceipt {
        command_id: command_id.into(),
        change_seq: seq.to_string(),
        committed_at: Utc::now().to_rfc3339(),
        replayed: false,
    })
}

pub(crate) fn store_receipt<T: Serialize>(
    connection: &Connection,
    hash: &str,
    kind: &str,
    object_id: &str,
    receipt: &MetadataReceipt,
    result: &T,
) -> Result<()> {
    let seq = receipt
        .change_seq
        .parse::<i64>()
        .map_err(|_| Error::IntegrityFailure)?;
    connection.execute(
        "INSERT INTO command_receipts(command_id,payload_hash,command_kind,object_id,revision_id,change_seq,committed_at,result_json) VALUES(?1,?2,?3,?4,NULL,?5,?6,?7)",
        params![receipt.command_id, hash, kind, object_id, seq, receipt.committed_at, serde_json::to_string(result)?],
    )?;
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if !(1..=300).contains(&name.chars().count()) || name.contains('\0') {
        return Err(Error::InvalidInput("name must contain 1-300 characters"));
    }
    Ok(name.into())
}

pub(crate) fn validate_pin(is_pinned: bool, pin_position: Option<u32>) -> Result<()> {
    if is_pinned != pin_position.is_some() {
        return Err(Error::InvalidInput(
            "pinned items require a position; unpinned items must omit it",
        ));
    }
    Ok(())
}

pub(crate) fn check_version(id: &str, expected: u32, actual: u32) -> Result<()> {
    if expected == 0 {
        return Err(Error::InvalidInput("metadata version must be positive"));
    }
    if expected != actual {
        return Err(Error::MetadataConflict {
            object_id: id.into(),
            expected,
            actual,
        });
    }
    Ok(())
}
