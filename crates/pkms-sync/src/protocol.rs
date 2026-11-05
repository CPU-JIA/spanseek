//! G03 object-level protocol: encrypted ops over stable IDs, server-assigned
//! order, explicit conflicts and tombstone propagation. Ciphertext is opaque.

use chrono::{SecondsFormat, Utc};
use rusqlite::{OptionalExtension, params};

use crate::crypto::{canonical, sha256_array};
use crate::model::{
    AppliedOp, ApplyOutcome, Conflict, ConflictReason, ConflictRecord, ConflictResolution,
    ObjectState, OpKind, PullBatch, RemoteOp, SyncOp, TrashEntry,
};
use crate::{SignedRequest, SyncAccess, SyncAuthority, SyncError};

pub const MAX_PULL_LIMIT: usize = 500;
pub const MAX_CIPHERTEXT_BYTES: usize = 16 * 1024 * 1024;

const MAX_ID_CHARS: usize = 200;
const MAX_OP_ID_CHARS: usize = 128;
const MAX_CLIENT_TIME_CHARS: usize = 64;

type StoredOpRow = (i64, i64, String, String, String, String, i64, Vec<u8>);
type CurrentObject = (i64, i64, i64, Option<String>, Option<String>);

struct ReplacementOp {
    device_id: String,
    workspace_id: String,
    object_id: String,
    kind: OpKind,
    base_version: i64,
    resulting_version: i64,
    tombstone: bool,
}

impl SyncAuthority {
    /// Applies one encrypted object op. The signed request binds the op payload
    /// through `op_request_hash`; the gate runs before any server state changes.
    pub fn apply_op(
        &mut self,
        request: &SignedRequest,
        op: &SyncOp,
        now_unix: i64,
    ) -> Result<ApplyOutcome, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_op(op)?;
        let expected = op_request_hash(op);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        if let Some(existing) = self.existing_op(op, &device.device_id, &expected)? {
            return Ok(ApplyOutcome::Applied(existing));
        }
        if let Some(current_epoch) = self.current_key_epoch(&op.workspace_id)?
            && op.key_epoch != current_epoch
        {
            return Err(SyncError::KeyEpochMismatch);
        }
        self.consume_request_nonce(request, now_unix)?;
        let outcome = self.apply_in_transaction(op, &device.device_id, &expected)?;
        self.touch_device(&device.device_id, now_unix)?;
        Ok(outcome)
    }

    /// Returns applied ops after a cursor. Deletes appear as tombstone ops; a
    /// client must apply them before accepting later object versions.
    pub fn pull(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        after_seq: i64,
        limit: usize,
        now_unix: i64,
    ) -> Result<PullBatch, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_identifier("workspace_id", workspace_id, MAX_ID_CHARS)?;
        if after_seq < 0 || limit == 0 || limit > MAX_PULL_LIMIT {
            return Err(SyncError::InvalidCursor);
        }
        let expected = pull_request_hash(workspace_id, after_seq, limit);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        self.pull_for_device(&device.device_id, workspace_id, after_seq, limit)
    }

    /// Read-only pull after the HTTP layer has authorized the signed request.
    /// Long-poll transports call this repeatedly without consuming a nonce for
    /// every 250 ms wake-up.
    pub fn pull_for_device(
        &self,
        device_id: &str,
        workspace_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<PullBatch, SyncError> {
        let limit_plus = limit as i64 + 1;
        let mut statement = self.connection.prepare(
            "SELECT server_seq,op_id,workspace_id,object_id,kind,base_version,resulting_version,device_id,key_epoch,ciphertext,tombstone,purged,deleted_at,deleted_by,client_time FROM sync_ops WHERE workspace_id=?1 AND server_seq>?2 ORDER BY server_seq LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![workspace_id, after_seq, limit_plus], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, Option<Vec<u8>>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = rows.len() > limit;
        let mut ops = Vec::with_capacity(rows.len().min(limit));
        for row in rows.into_iter().take(limit) {
            ops.push(RemoteOp {
                server_seq: row.0,
                op_id: row.1,
                workspace_id: row.2,
                object_id: row.3,
                kind: OpKind::parse(&row.4)?,
                base_version: row.5,
                resulting_version: row.6,
                device_id: row.7,
                key_epoch: row.8,
                ciphertext: row.9,
                tombstone: row.10 != 0,
                purged: row.11 != 0,
                deleted_at: row.12,
                deleted_by: row.13,
                client_time: row.14,
            });
        }
        let next_cursor = ops.last().map(|op| op.server_seq).unwrap_or(after_seq);
        let conflicts = self.unresolved_conflicts(device_id, workspace_id)?;
        Ok(PullBatch {
            ops,
            next_cursor,
            has_more,
            conflicts,
        })
    }
    /// Current object states for recovery and diagnostics. Tombstones are
    /// returned explicitly; ciphertext stays opaque.
    pub fn snapshot(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        now_unix: i64,
    ) -> Result<Vec<ObjectState>, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_identifier("workspace_id", workspace_id, MAX_ID_CHARS)?;
        let expected = snapshot_request_hash(workspace_id);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let mut statement = self.connection.prepare(
            "SELECT workspace_id,object_id,version,tombstone,ciphertext,key_epoch,last_seq,updated_by,purged,deleted_at,deleted_by FROM sync_objects WHERE workspace_id=?1 ORDER BY object_id",
        )?;
        let rows = statement
            .query_map([workspace_id], |row| {
                Ok(ObjectState {
                    workspace_id: row.get(0)?,
                    object_id: row.get(1)?,
                    version: row.get(2)?,
                    tombstone: row.get::<_, i64>(3)? != 0,
                    ciphertext: row.get(4)?,
                    key_epoch: row.get(5)?,
                    last_seq: row.get(6)?,
                    updated_by: row.get(7)?,
                    purged: row.get::<_, i64>(8)? != 0,
                    deleted_at: row.get(9)?,
                    deleted_by: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Cross-device trash: tombstoned objects, including purged entries.
    /// Purged entries keep a tombstone so stale devices cannot resurrect them.
    pub fn trash(
        &mut self,
        request: &SignedRequest,
        workspace_id: &str,
        now_unix: i64,
    ) -> Result<Vec<TrashEntry>, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        validate_identifier("workspace_id", workspace_id, MAX_ID_CHARS)?;
        let expected = trash_request_hash(workspace_id);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let mut statement = self.connection.prepare(
            "SELECT workspace_id,object_id,version,purged,deleted_at,deleted_by,last_seq FROM sync_objects WHERE workspace_id=?1 AND tombstone=1 ORDER BY last_seq DESC,object_id",
        )?;
        let rows = statement
            .query_map([workspace_id], |row| {
                Ok(TrashEntry {
                    workspace_id: row.get(0)?,
                    object_id: row.get(1)?,
                    version: row.get(2)?,
                    purged: row.get::<_, i64>(3)? != 0,
                    deleted_at: row.get(4)?,
                    deleted_by: row.get(5)?,
                    last_seq: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Records how the user resolved a conflict. The replacement op must have
    /// already been applied: keep-local rebases onto the remote version,
    /// save-as-new writes a fresh object. Keep-remote carries no replacement.
    pub fn resolve_conflict(
        &mut self,
        request: &SignedRequest,
        conflict_id: &str,
        resolution: ConflictResolution,
        replacement_op_id: Option<&str>,
        replacement_object_id: Option<&str>,
        now_unix: i64,
    ) -> Result<ConflictRecord, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        validate_identifier("conflict_id", conflict_id, MAX_OP_ID_CHARS)?;
        if let Some(op_id) = replacement_op_id {
            validate_identifier("replacement_op_id", op_id, MAX_OP_ID_CHARS)?;
        }
        if let Some(object_id) = replacement_object_id {
            validate_identifier("replacement_object_id", object_id, MAX_ID_CHARS)?;
        }
        let expected = resolve_conflict_request_hash(
            conflict_id,
            resolution,
            replacement_op_id,
            replacement_object_id,
        );
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        let conflict = self.conflict(conflict_id)?;
        if conflict.device_id != device.device_id {
            return Err(SyncError::ConflictNotFound);
        }
        if conflict.resolved_at.is_some() {
            if conflict.resolution == Some(resolution)
                && conflict.replacement_op_id.as_deref() == replacement_op_id
                && conflict.replacement_object_id.as_deref() == replacement_object_id
            {
                return Ok(conflict);
            }
            return Err(SyncError::ConflictAlreadyResolved);
        }
        match resolution {
            ConflictResolution::KeepRemote => {
                if replacement_op_id.is_some() || replacement_object_id.is_some() {
                    return Err(SyncError::InvalidOp(
                        "keep_remote must not carry replacement ids".into(),
                    ));
                }
            }
            ConflictResolution::KeepLocal => {
                if replacement_object_id.is_some() {
                    return Err(SyncError::InvalidOp(
                        "keep_local must keep the original object_id".into(),
                    ));
                }
                let op_id = replacement_op_id.ok_or_else(|| {
                    SyncError::InvalidOp("keep_local requires replacement_op_id".into())
                })?;
                let replacement = self.replacement_op(op_id)?;
                let expected_kind = if conflict.remote_tombstone {
                    OpKind::Restore
                } else {
                    OpKind::Put
                };
                let keeps_local_delete =
                    replacement.kind == OpKind::Delete && replacement.tombstone;
                if replacement.device_id != conflict.device_id
                    || replacement.workspace_id != conflict.workspace_id
                    || replacement.object_id != conflict.object_id
                    || (replacement.kind != expected_kind && !keeps_local_delete)
                    || replacement.base_version != conflict.remote_version
                    || replacement.resulting_version != conflict.remote_version + 1
                    || (replacement.tombstone && !keeps_local_delete)
                {
                    return Err(SyncError::InvalidOp(
                        "keep_local replacement must rebase the same object onto the remote version"
                            .into(),
                    ));
                }
            }
            ConflictResolution::SaveAsNew => {
                let op_id = replacement_op_id.ok_or_else(|| {
                    SyncError::InvalidOp("save_as_new requires replacement_op_id".into())
                })?;
                let object_id = replacement_object_id.ok_or_else(|| {
                    SyncError::InvalidOp("save_as_new requires replacement_object_id".into())
                })?;
                if object_id == conflict.object_id {
                    return Err(SyncError::InvalidOp(
                        "save_as_new requires a new object_id".into(),
                    ));
                }
                let replacement = self.replacement_op(op_id)?;
                if replacement.device_id != conflict.device_id
                    || replacement.workspace_id != conflict.workspace_id
                    || replacement.object_id != object_id
                    || replacement.kind != OpKind::Put
                    || replacement.base_version != 0
                    || replacement.resulting_version != 1
                    || replacement.tombstone
                {
                    return Err(SyncError::InvalidOp(
                        "save_as_new replacement must create the new object at version 1".into(),
                    ));
                }
            }
        }
        let resolved_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        self.connection.execute(
            "UPDATE sync_conflicts SET resolved_at=?1,resolution=?2,replacement_op_id=?3,replacement_object_id=?4 WHERE conflict_id=?5 AND resolved_at IS NULL",
            params![
                resolved_at,
                resolution.as_str(),
                replacement_op_id,
                replacement_object_id,
                conflict_id
            ],
        )?;
        self.touch_device(&device.device_id, now_unix)?;
        self.conflict(conflict_id)
    }

    fn conflict(&self, conflict_id: &str) -> Result<ConflictRecord, SyncError> {
        self.connection
            .query_row(
                "SELECT conflict_id,device_id,workspace_id,object_id,op_id,kind,base_version,remote_version,remote_tombstone,reason,created_at,resolved_at,resolution,replacement_op_id,replacement_object_id FROM sync_conflicts WHERE conflict_id=?1",
                [conflict_id],
                conflict_from_row,
            )
            .optional()?
            .ok_or(SyncError::ConflictNotFound)
    }

    fn unresolved_conflicts(
        &self,
        device_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<ConflictRecord>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT conflict_id,device_id,workspace_id,object_id,op_id,kind,base_version,remote_version,remote_tombstone,reason,created_at,resolved_at,resolution,replacement_op_id,replacement_object_id FROM sync_conflicts WHERE device_id=?1 AND workspace_id=?2 AND resolved_at IS NULL ORDER BY created_at DESC,conflict_id LIMIT 50",
        )?;
        let rows = statement
            .query_map(params![device_id, workspace_id], conflict_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn replacement_op(&self, op_id: &str) -> Result<ReplacementOp, SyncError> {
        let row: Option<(String, String, String, String, i64, i64, i64)> = self
            .connection
            .query_row(
                "SELECT device_id,workspace_id,object_id,kind,base_version,resulting_version,tombstone FROM sync_ops WHERE op_id=?1",
                [op_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((device, workspace, object, kind, base, resulting, tombstone)) = row else {
            return Err(SyncError::InvalidOp(
                "replacement op has not been applied".into(),
            ));
        };
        Ok(ReplacementOp {
            device_id: device,
            workspace_id: workspace,
            object_id: object,
            kind: OpKind::parse(&kind)?,
            base_version: base,
            resulting_version: resulting,
            tombstone: tombstone != 0,
        })
    }
    fn existing_op(
        &self,
        op: &SyncOp,
        device_id: &str,
        request_hash: &[u8; 32],
    ) -> Result<Option<AppliedOp>, SyncError> {
        let row: Option<StoredOpRow> = self
            .connection
            .query_row(
                "SELECT server_seq,resulting_version,device_id,workspace_id,object_id,kind,base_version,request_hash FROM sync_ops WHERE op_id=?1",
                [op.op_id.as_str()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((server_seq, version, stored_device, workspace, object, kind, base, stored_hash)) =
            row
        else {
            return Ok(None);
        };
        if stored_device != device_id
            || workspace != op.workspace_id
            || object != op.object_id
            || kind != op.kind.as_str()
            || base != op.base_version
            || stored_hash.as_slice() != request_hash.as_slice()
        {
            return Err(SyncError::OpIdReused);
        }
        Ok(Some(AppliedOp {
            op_id: op.op_id.clone(),
            server_seq,
            version,
            duplicate: true,
        }))
    }

    fn apply_in_transaction(
        &mut self,
        op: &SyncOp,
        device_id: &str,
        request_hash: &[u8; 32],
    ) -> Result<ApplyOutcome, SyncError> {
        let tx = self.connection.transaction()?;
        let current: Option<CurrentObject> = tx
            .query_row(
                "SELECT version,tombstone,purged,deleted_at,deleted_by FROM sync_objects WHERE workspace_id=?1 AND object_id=?2",
                params![op.workspace_id, op.object_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let plan = plan_op(
            op,
            current
                .as_ref()
                .map(|(version, tombstone, purged, _, _)| (*version, *tombstone, *purged)),
        );
        match plan {
            Plan::Conflict(conflict) => {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT conflict_id FROM sync_conflicts WHERE device_id=?1 AND workspace_id=?2 AND object_id=?3 AND resolved_at IS NULL ORDER BY created_at DESC LIMIT 1",
                        params![device_id, op.workspace_id, op.object_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(conflict_id) = existing {
                    // Repeated retries or a pull-side probe must not multiply
                    // unresolved rows for the same object. Keep one durable
                    // comparison and refresh its remote baseline.
                    tx.execute(
                        "UPDATE sync_conflicts SET remote_version=?1,remote_tombstone=?2,reason=?3 WHERE conflict_id=?4",
                        params![
                            conflict.current_version,
                            if conflict.current_tombstone { 1 } else { 0 },
                            conflict.reason.as_str(),
                            conflict_id
                        ],
                    )?;
                } else {
                    let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
                    tx.execute(
                        "INSERT INTO sync_conflicts(conflict_id,device_id,workspace_id,object_id,op_id,kind,base_version,remote_version,remote_tombstone,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(device_id,op_id) DO NOTHING",
                        params![
                            uuid::Uuid::new_v4().to_string(),
                            device_id,
                            op.workspace_id,
                            op.object_id,
                            op.op_id,
                            op.kind.as_str(),
                            op.base_version,
                            conflict.current_version,
                            if conflict.current_tombstone { 1 } else { 0 },
                            conflict.reason.as_str(),
                            created_at
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(ApplyOutcome::Conflict(conflict))
            }
            Plan::Apply {
                version,
                tombstone,
                purged,
                ciphertext,
            } => {
                let tombstone_flag = if tombstone { 1 } else { 0 };
                let purged_flag = if purged { 1 } else { 0 };
                let server_time = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
                let deleted_at = if tombstone {
                    Some(if purged {
                        current
                            .as_ref()
                            .and_then(|(_, _, _, at, _)| at.clone())
                            .unwrap_or_else(|| server_time.clone())
                    } else {
                        server_time.clone()
                    })
                } else {
                    None
                };
                let deleted_by = if tombstone {
                    Some(if purged {
                        current
                            .as_ref()
                            .and_then(|(_, _, _, _, by)| by.clone())
                            .unwrap_or_else(|| device_id.to_owned())
                    } else {
                        device_id.to_owned()
                    })
                } else {
                    None
                };
                tx.execute(
                    "INSERT INTO sync_ops(op_id,workspace_id,object_id,kind,base_version,resulting_version,device_id,key_epoch,ciphertext,tombstone,purged,deleted_at,deleted_by,request_hash,client_time,server_time) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                    params![
                        op.op_id,
                        op.workspace_id,
                        op.object_id,
                        op.kind.as_str(),
                        op.base_version,
                        version,
                        device_id,
                        op.key_epoch,
                        ciphertext.as_deref(),
                        tombstone_flag,
                        purged_flag,
                        deleted_at,
                        deleted_by,
                        request_hash.as_slice(),
                        op.client_time,
                        server_time
                    ],
                )?;
                let server_seq = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO sync_objects(workspace_id,object_id,version,tombstone,purged,ciphertext,key_epoch,last_seq,updated_by,deleted_at,deleted_by) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(workspace_id,object_id) DO UPDATE SET version=excluded.version,tombstone=excluded.tombstone,purged=excluded.purged,ciphertext=excluded.ciphertext,key_epoch=excluded.key_epoch,last_seq=excluded.last_seq,updated_by=excluded.updated_by,deleted_at=excluded.deleted_at,deleted_by=excluded.deleted_by",
                    params![
                        op.workspace_id,
                        op.object_id,
                        version,
                        tombstone_flag,
                        purged_flag,
                        ciphertext.as_deref(),
                        op.key_epoch,
                        server_seq,
                        device_id,
                        deleted_at,
                        deleted_by
                    ],
                )?;
                tx.commit()?;
                Ok(ApplyOutcome::Applied(AppliedOp {
                    op_id: op.op_id.clone(),
                    server_seq,
                    version,
                    duplicate: false,
                }))
            }
        }
    }
}
enum Plan {
    Apply {
        version: i64,
        tombstone: bool,
        purged: bool,
        ciphertext: Option<Vec<u8>>,
    },
    Conflict(Conflict),
}

fn plan_op(op: &SyncOp, current: Option<(i64, i64, i64)>) -> Plan {
    let conflict = |reason, current_version, current_tombstone| {
        Plan::Conflict(Conflict {
            reason,
            current_version,
            current_tombstone: current_tombstone != 0,
        })
    };
    match (op.kind, current) {
        (OpKind::Put, None) if op.base_version == 0 => Plan::Apply {
            version: 1,
            tombstone: false,
            purged: false,
            ciphertext: op.ciphertext.clone(),
        },
        (OpKind::Put, None) => conflict(ConflictReason::VersionMismatch, 0, 0),
        (OpKind::Put, Some((version, 0, _))) if op.base_version == version => Plan::Apply {
            version: version + 1,
            tombstone: false,
            purged: false,
            ciphertext: op.ciphertext.clone(),
        },
        (OpKind::Put, Some((version, 1, _))) if op.base_version == version => {
            conflict(ConflictReason::Deleted, version, 1)
        }
        (OpKind::Put, Some((version, tombstone, _))) => {
            conflict(ConflictReason::VersionMismatch, version, tombstone)
        }
        (OpKind::Delete, None) => conflict(ConflictReason::NotFound, 0, 0),
        (OpKind::Delete, Some((version, 1, 1))) if op.base_version == version => {
            conflict(ConflictReason::Purged, version, 1)
        }
        (OpKind::Delete, Some((version, 1, 0))) if op.base_version == version => {
            conflict(ConflictReason::Deleted, version, 1)
        }
        (OpKind::Delete, Some((version, 0, _))) if op.base_version == version => Plan::Apply {
            version: version + 1,
            tombstone: true,
            purged: false,
            ciphertext: None,
        },
        (OpKind::Delete, Some((version, tombstone, _))) => {
            conflict(ConflictReason::VersionMismatch, version, tombstone)
        }
        (OpKind::Purge, None) => conflict(ConflictReason::NotFound, 0, 0),
        (OpKind::Purge, Some((version, 1, 1))) if op.base_version == version => {
            conflict(ConflictReason::Purged, version, 1)
        }
        (OpKind::Purge, Some((version, 1, 0))) if op.base_version == version => Plan::Apply {
            version: version + 1,
            tombstone: true,
            purged: true,
            ciphertext: None,
        },
        (OpKind::Purge, Some((version, 0, _))) if op.base_version == version => {
            conflict(ConflictReason::AlreadyLive, version, 0)
        }
        (OpKind::Purge, Some((version, tombstone, _))) => {
            conflict(ConflictReason::VersionMismatch, version, tombstone)
        }
        (OpKind::Restore, None) => conflict(ConflictReason::NotFound, 0, 0),
        (OpKind::Restore, Some((version, 1, 1))) if op.base_version == version => {
            conflict(ConflictReason::Purged, version, 1)
        }
        (OpKind::Restore, Some((version, 0, _))) if op.base_version == version => {
            conflict(ConflictReason::AlreadyLive, version, 0)
        }
        (OpKind::Restore, Some((version, 1, 0))) if op.base_version == version => Plan::Apply {
            version: version + 1,
            tombstone: false,
            purged: false,
            ciphertext: op.ciphertext.clone(),
        },
        (OpKind::Restore, Some((version, tombstone, _))) => {
            conflict(ConflictReason::VersionMismatch, version, tombstone)
        }
    }
}
fn conflict_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConflictRecord> {
    let kind: String = row.get(5)?;
    let reason: String = row.get(9)?;
    let resolution: Option<String> = row.get(12)?;
    Ok(ConflictRecord {
        conflict_id: row.get(0)?,
        device_id: row.get(1)?,
        workspace_id: row.get(2)?,
        object_id: row.get(3)?,
        op_id: row.get(4)?,
        kind: OpKind::parse(&kind).map_err(|_| invalid_text_column(5, &kind))?,
        base_version: row.get(6)?,
        remote_version: row.get(7)?,
        remote_tombstone: row.get::<_, i64>(8)? != 0,
        reason: ConflictReason::parse(&reason).map_err(|_| invalid_text_column(9, &reason))?,
        created_at: row.get(10)?,
        resolved_at: row.get(11)?,
        resolution: resolution
            .map(|value| ConflictResolution::parse(&value))
            .transpose()
            .map_err(|_| invalid_text_column(12, "resolution"))?,
        replacement_op_id: row.get(13)?,
        replacement_object_id: row.get(14)?,
    })
}

fn invalid_text_column(index: usize, value: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(index, value.to_owned(), rusqlite::types::Type::Text)
}
fn validate_op(op: &SyncOp) -> Result<(), SyncError> {
    validate_identifier("op_id", &op.op_id, MAX_OP_ID_CHARS)?;
    validate_identifier("workspace_id", &op.workspace_id, MAX_ID_CHARS)?;
    validate_identifier("object_id", &op.object_id, MAX_ID_CHARS)?;
    if op.base_version < 0 {
        return Err(SyncError::InvalidOp("base_version must be >= 0".into()));
    }
    if op.key_epoch < 1 {
        return Err(SyncError::InvalidOp("key_epoch must be >= 1".into()));
    }
    match op.kind {
        OpKind::Put | OpKind::Restore => match &op.ciphertext {
            Some(ciphertext)
                if !ciphertext.is_empty() && ciphertext.len() <= MAX_CIPHERTEXT_BYTES => {}
            Some(_) => {
                return Err(SyncError::InvalidOp(
                    "ciphertext is empty or exceeds the op size limit".into(),
                ));
            }
            None => {
                return Err(SyncError::InvalidOp(
                    "put/restore requires ciphertext".into(),
                ));
            }
        },
        OpKind::Delete | OpKind::Purge if op.ciphertext.is_some() => {
            return Err(SyncError::InvalidOp(
                "delete/purge must not carry ciphertext".into(),
            ));
        }
        OpKind::Delete => {}
        OpKind::Purge => {
            if op.base_version < 1 {
                return Err(SyncError::InvalidOp(
                    "purge requires an existing tombstone base_version".into(),
                ));
            }
        }
    }
    if let Some(client_time) = &op.client_time
        && (client_time.is_empty()
            || client_time.chars().count() > MAX_CLIENT_TIME_CHARS
            || client_time.chars().any(char::is_control))
    {
        return Err(SyncError::InvalidOp("invalid client_time".into()));
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str, max_chars: usize) -> Result<(), SyncError> {
    let count = value.chars().count();
    if count == 0
        || count > max_chars
        || value.trim().is_empty()
        || value.chars().any(char::is_control)
    {
        return Err(SyncError::InvalidOp(format!("invalid {name}")));
    }
    Ok(())
}

pub fn op_request_hash(op: &SyncOp) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-op-v1",
        op.op_id.as_bytes(),
        op.workspace_id.as_bytes(),
        op.object_id.as_bytes(),
        op.kind.as_str().as_bytes(),
        &op.base_version.to_be_bytes(),
        &op.key_epoch.to_be_bytes(),
        op.ciphertext.as_deref().unwrap_or(&[]),
        op.client_time.as_deref().unwrap_or("").as_bytes(),
    ]))
}

pub fn pull_request_hash(workspace_id: &str, after_seq: i64, limit: usize) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-pull-v1",
        workspace_id.as_bytes(),
        &after_seq.to_be_bytes(),
        &(limit as u64).to_be_bytes(),
    ]))
}

pub fn pull_wait_request_hash(
    workspace_id: &str,
    after_seq: i64,
    limit: usize,
    timeout_ms: u64,
) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-pull-wait-v1",
        workspace_id.as_bytes(),
        &after_seq.to_be_bytes(),
        &(limit as u64).to_be_bytes(),
        &timeout_ms.to_be_bytes(),
    ]))
}

pub fn resolve_conflict_request_hash(
    conflict_id: &str,
    resolution: ConflictResolution,
    replacement_op_id: Option<&str>,
    replacement_object_id: Option<&str>,
) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-resolve-v1",
        conflict_id.as_bytes(),
        resolution.as_str().as_bytes(),
        replacement_op_id.unwrap_or("").as_bytes(),
        replacement_object_id.unwrap_or("").as_bytes(),
    ]))
}

pub fn trash_request_hash(workspace_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-trash-v1",
        workspace_id.as_bytes(),
    ]))
}

pub fn snapshot_request_hash(workspace_id: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-snapshot-v1",
        workspace_id.as_bytes(),
    ]))
}
