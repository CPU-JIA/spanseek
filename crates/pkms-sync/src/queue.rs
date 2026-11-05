//! Client-side offline queue. Pure ordering/rebase logic; transport is later.

use serde::{Deserialize, Serialize};

use crate::SyncError;
use crate::model::SyncOp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QueueState {
    Pending,
    InFlight,
    Applied {
        server_seq: i64,
        version: i64,
    },
    Conflicted {
        conflict_id: String,
        remote_version: i64,
    },
    Blocked {
        by_op_id: String,
    },
    Failed {
        code: String,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedOp {
    pub op: SyncOp,
    pub state: QueueState,
    pub attempts: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OfflineQueue {
    entries: Vec<QueuedOp>,
}

impl OfflineQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[QueuedOp] {
        &self.entries
    }

    pub fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.state == QueueState::Pending)
            .count()
    }

    pub fn enqueue(&mut self, op: SyncOp) -> Result<(), SyncError> {
        if self.entries.iter().any(|entry| entry.op.op_id == op.op_id) {
            return Err(SyncError::InvalidOp("duplicate queued op_id".into()));
        }
        let blocked_by = self
            .entries
            .iter()
            .rev()
            .find(|entry| {
                entry.op.object_id == op.object_id
                    && matches!(
                        entry.state,
                        QueueState::Conflicted { .. }
                            | QueueState::Blocked { .. }
                            | QueueState::Failed { .. }
                    )
            })
            .map(|entry| entry.op.op_id.clone());
        self.entries.push(QueuedOp {
            op,
            state: blocked_by
                .map(|by_op_id| QueueState::Blocked { by_op_id })
                .unwrap_or(QueueState::Pending),
            attempts: 0,
            last_error: None,
        });
        Ok(())
    }

    /// First pending op whose object has no earlier unfinished op.
    pub fn next(&self) -> Option<&QueuedOp> {
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.state != QueueState::Pending {
                continue;
            }
            let blocked = self.entries[..index].iter().any(|earlier| {
                earlier.op.object_id == entry.op.object_id
                    && !matches!(
                        earlier.state,
                        QueueState::Applied { .. } | QueueState::Cancelled
                    )
            });
            if !blocked {
                return Some(entry);
            }
        }
        None
    }

    pub fn mark_in_flight(&mut self, op_id: &str) -> Result<(), SyncError> {
        let entry = self.entry_mut(op_id)?;
        if entry.state == QueueState::Pending {
            entry.state = QueueState::InFlight;
        }
        Ok(())
    }

    /// Applies the server result and shifts later same-object bases by the
    /// difference between the server version and the local predicted version.
    pub fn mark_applied(
        &mut self,
        op_id: &str,
        server_seq: i64,
        version: i64,
    ) -> Result<(), SyncError> {
        let (object_id, base_version, already_applied) = {
            let entry = self.entry_mut(op_id)?;
            let already_applied = matches!(entry.state, QueueState::Applied { .. });
            let object_id = entry.op.object_id.clone();
            let base_version = entry.op.base_version;
            entry.state = QueueState::Applied {
                server_seq,
                version,
            };
            entry.last_error = None;
            (object_id, base_version, already_applied)
        };
        if already_applied {
            return Ok(());
        }
        let delta = version - (base_version + 1);
        if delta != 0 {
            for entry in self.entries.iter_mut().filter(|entry| {
                entry.op.object_id == object_id
                    && matches!(
                        entry.state,
                        QueueState::Pending | QueueState::Blocked { .. }
                    )
            }) {
                entry.op.base_version = (entry.op.base_version + delta).max(0);
            }
        }
        Ok(())
    }

    pub fn mark_conflict(
        &mut self,
        op_id: &str,
        conflict_id: &str,
        remote_version: i64,
    ) -> Result<(), SyncError> {
        let object_id = {
            let entry = self.entry_mut(op_id)?;
            entry.state = QueueState::Conflicted {
                conflict_id: conflict_id.to_owned(),
                remote_version,
            };
            entry.op.object_id.clone()
        };
        self.block_later(&object_id, op_id);
        Ok(())
    }

    /// Keep the local edit: rebase the conflicted op onto the remote version
    /// and unblock later edits on the same object.
    pub fn resolve_rebase(&mut self, op_id: &str, new_base: i64) -> Result<(), SyncError> {
        let (object_id, delta) = {
            let entry = self.entry_mut(op_id)?;
            if !matches!(entry.state, QueueState::Conflicted { .. }) {
                return Err(SyncError::InvalidOp("op is not conflicted".into()));
            }
            let delta = new_base - entry.op.base_version;
            entry.op.base_version = new_base;
            entry.state = QueueState::Pending;
            (entry.op.object_id.clone(), delta)
        };
        for entry in self.entries.iter_mut().filter(|entry| {
            entry.op.object_id == object_id
                && (matches!(entry.state, QueueState::Blocked { .. })
                    || (entry.op.op_id != op_id && entry.state == QueueState::Pending))
        }) {
            entry.op.base_version = (entry.op.base_version + delta).max(0);
            if matches!(entry.state, QueueState::Blocked { .. }) {
                entry.state = QueueState::Pending;
            }
        }
        Ok(())
    }

    /// Accept the remote edit: cancel the conflicted op and its dependent
    /// local chain.
    pub fn resolve_keep_remote(&mut self, op_id: &str) -> Result<(), SyncError> {
        let object_id = {
            let entry = self.entry_mut(op_id)?;
            if !matches!(entry.state, QueueState::Conflicted { .. }) {
                return Err(SyncError::InvalidOp("op is not conflicted".into()));
            }
            entry.state = QueueState::Cancelled;
            entry.op.object_id.clone()
        };
        for entry in self.entries.iter_mut().filter(|entry| {
            entry.op.object_id == object_id
                && matches!(
                    entry.state,
                    QueueState::Blocked { .. } | QueueState::Pending
                )
        }) {
            entry.state = QueueState::Cancelled;
        }
        Ok(())
    }

    pub fn mark_retry(
        &mut self,
        op_id: &str,
        code: &str,
        retryable: bool,
    ) -> Result<(), SyncError> {
        let object_id = {
            let entry = self.entry_mut(op_id)?;
            entry.attempts += 1;
            entry.last_error = Some(code.to_owned());
            if retryable {
                entry.state = QueueState::Pending;
            } else {
                entry.state = QueueState::Failed {
                    code: code.to_owned(),
                };
            }
            entry.op.object_id.clone()
        };
        if !retryable {
            self.block_later(&object_id, op_id);
        }
        Ok(())
    }

    fn block_later(&mut self, object_id: &str, op_id: &str) {
        for entry in self.entries.iter_mut().filter(|entry| {
            entry.op.object_id == object_id && matches!(entry.state, QueueState::Pending)
        }) {
            entry.state = QueueState::Blocked {
                by_op_id: op_id.to_owned(),
            };
        }
    }

    fn entry_mut(&mut self, op_id: &str) -> Result<&mut QueuedOp, SyncError> {
        self.entries
            .iter_mut()
            .find(|entry| entry.op.op_id == op_id)
            .ok_or_else(|| SyncError::InvalidOp("queued op not found".into()))
    }
}
