use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{ConfigConflict, EngineError, ForkPlan, PendingOperation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KnownState {
    pub revision_id: String,
    pub trashed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SyncState {
    pub cursor: i64,
    pub initial_seed_completed: bool,
    pub known: HashMap<String, KnownState>,
    pub versions: HashMap<String, i64>,
    pub config_conflicts: HashMap<String, ConfigConflict>,
    pub pending: HashMap<String, PendingOperation>,
    pub forks: HashMap<String, ForkPlan>,
}

pub(crate) struct SyncStateStore {
    connection: Connection,
}

impl SyncStateStore {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS sync_state(
               singleton INTEGER PRIMARY KEY CHECK(singleton=1),
               cursor INTEGER NOT NULL,
               initial_seed_completed INTEGER NOT NULL DEFAULT 0 CHECK(initial_seed_completed IN (0,1))
             );
             INSERT OR IGNORE INTO sync_state(singleton,cursor) VALUES(1,0);
             CREATE TABLE IF NOT EXISTS sync_known(
               object_id TEXT PRIMARY KEY,
               revision_id TEXT NOT NULL,
               trashed INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_versions(
               object_id TEXT PRIMARY KEY,
               version INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_config_conflicts(
               object_id TEXT PRIMARY KEY,
               kind TEXT NOT NULL,
               local_title TEXT NOT NULL,
               remote_title TEXT NOT NULL,
               remote_version INTEGER NOT NULL,
               local_available INTEGER NOT NULL,
               remote_available INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_pending_operations(
               object_id TEXT PRIMARY KEY,
               operation_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_fork_plans(
               conflict_id TEXT PRIMARY KEY,
               plan_json TEXT NOT NULL
             );",
        )?;
        // A partial seed can already have cursor/baselines/receipts. Neither a
        // populated legacy state nor an existing file proves both seed phases
        // completed. Keep all old data and require one explicit successful seed.
        let migration =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let has_seed_marker: bool = migration.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sync_state') WHERE name='initial_seed_completed')",
            [],
            |row| row.get(0),
        )?;
        if !has_seed_marker {
            migration.execute_batch(
                "ALTER TABLE sync_state ADD COLUMN initial_seed_completed INTEGER NOT NULL DEFAULT 0 CHECK(initial_seed_completed IN (0,1));",
            )?;
        }
        migration.commit()?;
        Ok(Self { connection })
    }

    pub(crate) fn load(&self) -> Result<SyncState, EngineError> {
        let (cursor, initial_seed_completed) = self.connection.query_row(
            "SELECT cursor,initial_seed_completed FROM sync_state WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut known = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT object_id,revision_id,trashed FROM sync_known")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    KnownState {
                        revision_id: row.get(1)?,
                        trashed: row.get::<_, i64>(2)? != 0,
                    },
                ))
            })?;
            for row in rows {
                let (object_id, state) = row?;
                known.insert(object_id, state);
            }
        }
        let mut versions = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT object_id,version FROM sync_versions")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (object_id, version) = row?;
                versions.insert(object_id, version);
            }
        }
        let mut config_conflicts = HashMap::new();
        {
            let mut statement = self.connection.prepare(
                "SELECT object_id,kind,local_title,remote_title,remote_version,local_available,remote_available FROM sync_config_conflicts",
            )?;
            let rows = statement.query_map([], |row| {
                let object_id: String = row.get(0)?;
                Ok((
                    object_id.clone(),
                    ConfigConflict {
                        object_id,
                        kind: row.get(1)?,
                        local_title: row.get(2)?,
                        remote_title: row.get(3)?,
                        remote_version: row.get(4)?,
                        local_available: row.get::<_, i64>(5)? != 0,
                        remote_available: row.get::<_, i64>(6)? != 0,
                    },
                ))
            })?;
            for row in rows {
                let (object_id, conflict) = row?;
                config_conflicts.insert(object_id, conflict);
            }
        }
        let mut pending = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT object_id,operation_json FROM sync_pending_operations")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (object_id, json) = row?;
                pending.insert(object_id, serde_json::from_str(&json)?);
            }
        }
        let mut forks = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT conflict_id,plan_json FROM sync_fork_plans")?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (conflict_id, json) = row?;
                forks.insert(conflict_id, serde_json::from_str(&json)?);
            }
        }
        Ok(SyncState {
            cursor,
            initial_seed_completed,
            known,
            versions,
            config_conflicts,
            pending,
            forks,
        })
    }

    pub(crate) fn save(&self, state: &SyncState) -> Result<(), EngineError> {
        let tx = self.connection.unchecked_transaction()?;
        tx.execute(
            "UPDATE sync_state SET cursor=?1,initial_seed_completed=?2 WHERE singleton=1",
            params![state.cursor, state.initial_seed_completed],
        )?;
        tx.execute("DELETE FROM sync_known", [])?;
        tx.execute("DELETE FROM sync_versions", [])?;
        tx.execute("DELETE FROM sync_config_conflicts", [])?;
        tx.execute("DELETE FROM sync_pending_operations", [])?;
        tx.execute("DELETE FROM sync_fork_plans", [])?;
        for (object_id, known) in &state.known {
            tx.execute(
                "INSERT INTO sync_known(object_id,revision_id,trashed) VALUES(?1,?2,?3)",
                params![
                    object_id,
                    known.revision_id,
                    if known.trashed { 1 } else { 0 }
                ],
            )?;
        }
        for (object_id, version) in &state.versions {
            tx.execute(
                "INSERT INTO sync_versions(object_id,version) VALUES(?1,?2)",
                params![object_id, version],
            )?;
        }
        for conflict in state.config_conflicts.values() {
            tx.execute(
                "INSERT INTO sync_config_conflicts(object_id,kind,local_title,remote_title,remote_version,local_available,remote_available) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    conflict.object_id,
                    conflict.kind,
                    conflict.local_title,
                    conflict.remote_title,
                    conflict.remote_version,
                    if conflict.local_available { 1 } else { 0 },
                    if conflict.remote_available { 1 } else { 0 }
                ],
            )?;
        }
        for (object_id, operation) in &state.pending {
            tx.execute(
                "INSERT INTO sync_pending_operations(object_id,operation_json) VALUES(?1,?2)",
                params![object_id, serde_json::to_string(operation)?],
            )?;
        }
        for (conflict_id, plan) in &state.forks {
            tx.execute(
                "INSERT INTO sync_fork_plans(conflict_id,plan_json) VALUES(?1,?2)",
                params![conflict_id, serde_json::to_string(plan)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConflictTarget;
    use pkms_sync::{OpKind, SyncOp};

    fn partial_state() -> SyncState {
        let known = KnownState {
            revision_id: "retained-revision".into(),
            trashed: false,
        };
        SyncState {
            cursor: 17,
            initial_seed_completed: false,
            known: HashMap::from([("object".into(), known.clone())]),
            versions: HashMap::from([("object".into(), 3)]),
            config_conflicts: HashMap::from([(
                "view".into(),
                ConfigConflict {
                    object_id: "view".into(),
                    kind: "view".into(),
                    local_title: "local".into(),
                    remote_title: "remote".into(),
                    remote_version: 2,
                    local_available: true,
                    remote_available: true,
                },
            )]),
            pending: HashMap::from([(
                "object".into(),
                PendingOperation {
                    op: SyncOp {
                        op_id: "original-request".into(),
                        workspace_id: "workspace".into(),
                        object_id: "object".into(),
                        kind: OpKind::Delete,
                        base_version: 3,
                        key_epoch: 1,
                        ciphertext: None,
                        client_time: Some("2026-11-05T00:00:00Z".into()),
                    },
                    device_id: "device".into(),
                    known: Some(known),
                    clear_config_conflict: false,
                    keep_local_conflict: None,
                    probe: false,
                    may_have_committed: true,
                },
            )]),
            forks: HashMap::from([(
                "conflict".into(),
                ForkPlan {
                    conflict: ConflictTarget {
                        conflict_id: "conflict".into(),
                        object_id: "source".into(),
                        remote_version: 2,
                        remote_tombstone: false,
                    },
                    workspace_id: "workspace".into(),
                    device_id: "device".into(),
                    source_revision_id: "source-revision".into(),
                    source_content_hash: "source-hash".into(),
                    source_row_version: 1,
                    copy_command_id: "copy-command".into(),
                    fork_id: None,
                    fork_revision_id: None,
                    publish_op_id: None,
                    published_version: None,
                    adoption: None,
                    completed: false,
                    superseded: false,
                },
            )]),
        }
    }

    #[test]
    fn legacy_seed_marker_migration_preserves_all_partial_state() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("legacy-state.sqlite3");
        let store = SyncStateStore::open(&path).unwrap();
        let expected = partial_state();
        store.save(&expected).unwrap();
        // Exact pre-marker table shape, including populated durable intents.
        store
            .connection
            .execute_batch("ALTER TABLE sync_state DROP COLUMN initial_seed_completed;")
            .unwrap();
        drop(store);
        let reopened = SyncStateStore::open(&path).unwrap();
        let loaded = reopened.load().unwrap();
        assert!(!loaded.initial_seed_completed);
        assert_eq!(loaded, expected);
        drop(reopened);
        assert_eq!(
            SyncStateStore::open(&path).unwrap().load().unwrap(),
            expected
        );
    }

    #[test]
    fn empty_completed_seed_remains_completed_after_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("empty-state.sqlite3");
        let store = SyncStateStore::open(&path).unwrap();
        assert!(!store.load().unwrap().initial_seed_completed);
        let expected = SyncState {
            initial_seed_completed: true,
            ..SyncState::default()
        };
        store.save(&expected).unwrap();
        drop(store);
        assert_eq!(
            SyncStateStore::open(&path).unwrap().load().unwrap(),
            expected
        );
    }

    #[test]
    fn seed_completion_transaction_failure_keeps_prior_marker_and_receipts() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("failed-completion.sqlite3");
        let store = SyncStateStore::open(&path).unwrap();
        let before = partial_state();
        store.save(&before).unwrap();
        store.connection.execute_batch(
            "CREATE TRIGGER fail_seed_completion BEFORE INSERT ON sync_versions BEGIN SELECT RAISE(ABORT,'injected seed completion failure'); END;",
        ).unwrap();
        let mut completed = before.clone();
        completed.initial_seed_completed = true;
        completed.cursor = 18;
        completed.versions.insert("object".into(), 4);
        assert!(store.save(&completed).is_err());
        assert_eq!(store.load().unwrap(), before);
        drop(store);
        let reopened = SyncStateStore::open(&path).unwrap();
        assert_eq!(reopened.load().unwrap(), before);
        reopened
            .connection
            .execute_batch("DROP TRIGGER fail_seed_completion;")
            .unwrap();
        reopened.save(&completed).unwrap();
        drop(reopened);
        assert_eq!(
            SyncStateStore::open(&path).unwrap().load().unwrap(),
            completed
        );
    }
}
