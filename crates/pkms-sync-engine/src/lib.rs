//! Note-level sync engine: maps a local library note to an encrypted object
//! operation, pulls remote operations, and keeps conflicts explicit.
//!
//! The engine never overwrites a locally edited note automatically. A local
//! note is updated from a remote op only when its revision and trash state still
//! match the last synced state; otherwise a conflict is returned for the UI.

use std::collections::{HashMap, HashSet};
use std::path::Path;

mod state;
mod tag_sync;
pub use tag_sync::{
    TagApplySummary, TagPushOutcome, TagPushResult, TagRetryPage, TagSyncDecision, TagWireOwners,
};

use state::{KnownState, SyncState, SyncStateStore};

use chrono::Utc;
use pkms_core::{
    Canvas, CopyNote, CreateNote, Library, Note, NoteDocument, SaveNote, SavedView,
    WorkspaceMetadata,
};
use pkms_sync::{
    ApplyOutcome, ConflictRecord, ConflictResolution, OpKind, RemoteOp, SealedData, SyncOp,
    WorkspaceKey, object_aad,
};
use pkms_sync_client::SyncClient;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const WORKSPACE_METADATA_OBJECT_ID: &str = "7d3f2c1a-6b5e-4f10-9a8b-0c1d2e3f4a5b";
const CONFIG_DELETED_FINGERPRINT: &str = "config:deleted";

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Core(#[from] pkms_core::Error),
    #[error(transparent)]
    Sync(#[from] pkms_sync::SyncError),
    #[error(transparent)]
    Client(#[from] pkms_sync_client::ClientError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("remote operation has no ciphertext")]
    MissingCiphertext,
    #[error("local note is trashed")]
    LocalTrashed,
    #[error("remote object is missing after resolution")]
    RemoteMissing,
    #[error("local configuration object is missing")]
    LocalConfigMissing,
    #[error("conflict probe unexpectedly overwrote the remote object")]
    ConflictProbeApplied,
    #[error("workspace key epoch {0} is not available")]
    MissingWorkspaceKey(i64),
    #[error("pending sync request belongs to a different object, workspace or device")]
    PendingRequestMismatch,
    #[error("this save-as-new decision was superseded by another explicit resolution")]
    ForkSuperseded,
    #[error(
        "the preserved fork has a remote conflict and must be reviewed before resolving its source"
    )]
    ForkPublicationConflict,
    #[error(
        "the prior fork publication receipt cannot be confirmed; review the existing copy before resolving the source"
    )]
    ForkReceiptUnavailable,
    #[error(
        "the conflict comparison changed or is no longer available; refresh before resolving it"
    )]
    ConflictTargetChanged,
    #[error("sync_config_local_changed")]
    ConfigPreviewChanged,
    #[error("sync_config_conflict_changed")]
    ConfigConflictChanged,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotePayload {
    pub title: String,
    pub document_schema: u32,
    pub document: NoteDocument,
    pub updated_at: String,
}

impl NotePayload {
    fn from_note(note: &Note) -> Self {
        Self {
            title: note.title.clone(),
            document_schema: note.schema_version,
            document: note.document.clone(),
            updated_at: note.updated_at.clone(),
        }
    }

    fn matches_note(&self, note: &Note) -> bool {
        self.title == note.title
            && self.document_schema == note.schema_version
            && self.document == note.document
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigConflict {
    pub object_id: String,
    pub kind: String,
    pub local_title: String,
    pub remote_title: String,
    pub remote_version: i64,
    pub local_available: bool,
    pub remote_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigLocalState {
    pub title: String,
    pub available: bool,
    pub tombstone: bool,
    pub local_state_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictTarget {
    pub conflict_id: String,
    pub object_id: String,
    pub remote_version: i64,
    pub remote_tombstone: bool,
}

impl From<&ConflictRecord> for ConflictTarget {
    fn from(conflict: &ConflictRecord) -> Self {
        Self {
            conflict_id: conflict.conflict_id.clone(),
            object_id: conflict.object_id.clone(),
            remote_version: conflict.remote_version,
            remote_tombstone: conflict.remote_tombstone,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullConflict {
    pub object_id: String,
    pub local_title: String,
    pub local_body: String,
    pub remote_title: String,
    pub remote_body: String,
    pub remote_version: i64,
    pub remote_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sync_kind", rename_all = "snake_case")]
enum ConfigPayload {
    View { view: SavedView },
    Canvas { canvas: Canvas },
    WorkspaceMetadata { metadata: WorkspaceMetadata },
}

enum Payload {
    Note(NotePayload),
    Tag(pkms_core::TagSyncPayload),
    View(SavedView),
    Canvas(Canvas),
    WorkspaceMetadata(WorkspaceMetadata),
}

/// The exact encrypted request and the local state it represents. This is
/// written before network I/O, so an unknown result never needs new encryption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingOperation {
    op: SyncOp,
    device_id: String,
    known: Option<KnownState>,
    clear_config_conflict: bool,
    keep_local_conflict: Option<String>,
    probe: bool,
    #[serde(default = "previous_request_is_uncertain")]
    may_have_committed: bool,
}

fn previous_request_is_uncertain() -> bool {
    true
}

/// Stable Core command receipts own copied text and random document identities.
/// The transport plan stores only identities, immutable revision/hash references
/// and remote log coordinates; it is not another plaintext document store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForkPlan {
    conflict: ConflictTarget,
    workspace_id: String,
    device_id: String,
    source_revision_id: String,
    source_content_hash: String,
    source_row_version: u32,
    copy_command_id: String,
    fork_id: Option<String>,
    fork_revision_id: Option<String>,
    publish_op_id: Option<String>,
    published_version: Option<i64>,
    adoption: Option<ForkAdoption>,
    completed: bool,
    superseded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ForkAdoption {
    server_seq: i64,
    version: i64,
    tombstone: bool,
    ciphertext_hash: Option<String>,
    command_id: String,
    applied: Option<KnownState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushOutcome {
    Unchanged {
        version: i64,
    },
    Applied {
        version: i64,
    },
    Conflict {
        remote_version: i64,
        remote_tombstone: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullSummary {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub trashed: Vec<String>,
    pub restored: Vec<String>,
    pub conflicts: Vec<PullConflict>,
    pub config_conflicts: Vec<ConfigConflict>,
    pub server_conflicts: Vec<ConflictRecord>,
    pub next_cursor: i64,
    pub tag_results: Vec<TagApplySummary>,
}

pub struct SyncEngine {
    client: SyncClient,
    workspace_keys: HashMap<i64, WorkspaceKey>,
    workspace_id: String,
    epoch: i64,
    known: HashMap<String, KnownState>,
    versions: HashMap<String, i64>,
    config_conflicts: HashMap<String, ConfigConflict>,
    pending: HashMap<String, PendingOperation>,
    forks: HashMap<String, ForkPlan>,
    cursor: i64,
    initial_seed_completed: bool,
    state_store: Option<SyncStateStore>,
}

impl SyncEngine {
    pub fn new(
        client: SyncClient,
        workspace_key: WorkspaceKey,
        workspace_id: impl Into<String>,
        epoch: i64,
    ) -> Self {
        Self::with_keys(
            client,
            HashMap::from([(epoch, workspace_key)]),
            workspace_id,
            epoch,
        )
    }

    pub fn with_keys(
        client: SyncClient,
        workspace_keys: HashMap<i64, WorkspaceKey>,
        workspace_id: impl Into<String>,
        epoch: i64,
    ) -> Self {
        Self {
            client,
            workspace_keys,
            workspace_id: workspace_id.into(),
            epoch,
            known: HashMap::new(),
            versions: HashMap::new(),
            config_conflicts: HashMap::new(),
            pending: HashMap::new(),
            forks: HashMap::new(),
            cursor: 0,
            initial_seed_completed: false,
            state_store: None,
        }
    }

    pub fn with_state(
        client: SyncClient,
        workspace_key: WorkspaceKey,
        workspace_id: impl Into<String>,
        epoch: i64,
        path: impl AsRef<Path>,
    ) -> Result<Self, EngineError> {
        Self::with_state_and_keys(
            client,
            HashMap::from([(epoch, workspace_key)]),
            workspace_id,
            epoch,
            path,
        )
    }

    pub fn with_state_and_keys(
        client: SyncClient,
        workspace_keys: HashMap<i64, WorkspaceKey>,
        workspace_id: impl Into<String>,
        epoch: i64,
        path: impl AsRef<Path>,
    ) -> Result<Self, EngineError> {
        let store = SyncStateStore::open(path)?;
        let state = store.load()?;
        Ok(Self {
            client,
            workspace_keys,
            workspace_id: workspace_id.into(),
            epoch,
            known: state.known,
            versions: state.versions,
            config_conflicts: state.config_conflicts,
            pending: state.pending,
            forks: state.forks,
            cursor: state.cursor,
            initial_seed_completed: state.initial_seed_completed,
            state_store: Some(store),
        })
    }

    pub fn persist(&self) -> Result<(), EngineError> {
        if let Some(store) = &self.state_store {
            store.save(&SyncState {
                cursor: self.cursor,
                initial_seed_completed: self.initial_seed_completed,
                known: self.known.clone(),
                versions: self.versions.clone(),
                config_conflicts: self.config_conflicts.clone(),
                pending: self.pending.clone(),
                forks: self.forks.clone(),
            })?;
        }
        Ok(())
    }
    pub fn client(&self) -> &SyncClient {
        &self.client
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    pub fn needs_initial_seed(&self) -> bool {
        !self.initial_seed_completed
    }

    pub fn pending_config_conflicts(&self) -> Vec<ConfigConflict> {
        let mut conflicts: Vec<_> = self.config_conflicts.values().cloned().collect();
        conflicts.sort_by(|left, right| left.object_id.cmp(&right.object_id));
        conflicts
    }

    fn scoped_conflicts(&self, conflicts: Vec<ConflictRecord>) -> Vec<ConflictRecord> {
        conflicts
            .into_iter()
            .filter(|conflict| {
                conflict.workspace_id == self.workspace_id
                    && conflict.device_id == self.client.device_id()
            })
            .collect()
    }

    fn require_current_conflict(&self, target: &ConflictTarget) -> Result<(), EngineError> {
        let conflicts = self.scoped_conflicts(
            self.client
                .pull(&self.workspace_id, self.cursor, 1)?
                .conflicts,
        );
        if conflicts.iter().any(|conflict| {
            conflict.conflict_id == target.conflict_id
                && conflict.object_id == target.object_id
                && conflict.remote_version == target.remote_version
                && conflict.remote_tombstone == target.remote_tombstone
        }) {
            Ok(())
        } else {
            Err(EngineError::ConflictTargetChanged)
        }
    }

    pub fn set_cursor(&mut self, cursor: i64) -> Result<(), EngineError> {
        self.cursor = cursor;
        self.persist()
    }

    fn fresh_op_id(&self) -> String {
        let material = format!(
            "spanseek-sync-op-v2\0{}\0{}\0{}",
            self.client.device_id(),
            self.workspace_id,
            Uuid::new_v4()
        );
        format!("spanseek-op-{}", pkms_sync::sha256_hex(material.as_bytes()))
    }

    fn submit_operation(
        &mut self,
        op: SyncOp,
        known: Option<KnownState>,
        clear_config_conflict: bool,
        keep_local_conflict: Option<String>,
        probe: bool,
    ) -> Result<ApplyOutcome, EngineError> {
        self.protect_non_tag_server_wire(&op.object_id)?;
        self.submit_operation_with(
            op,
            known,
            clear_config_conflict,
            keep_local_conflict,
            probe,
            |_, _| Ok(()),
        )
    }

    fn submit_operation_with<F>(
        &mut self,
        op: SyncOp,
        known: Option<KnownState>,
        clear_config_conflict: bool,
        keep_local_conflict: Option<String>,
        probe: bool,
        on_applied: F,
    ) -> Result<ApplyOutcome, EngineError>
    where
        F: FnOnce(&PendingOperation, i64) -> Result<(), EngineError>,
    {
        let object_id = op.object_id.clone();
        if self.pending.contains_key(&object_id) {
            return Err(EngineError::PendingRequestMismatch);
        }
        self.pending.insert(
            object_id.clone(),
            PendingOperation {
                op,
                device_id: self.client.device_id().into(),
                known,
                clear_config_conflict,
                keep_local_conflict,
                probe,
                may_have_committed: false,
            },
        );
        self.resume_pending_operation_with(&object_id, on_applied)?
            .ok_or(EngineError::PendingRequestMismatch)
    }

    /// Re-signing the transport envelope is safe, but the logical operation
    /// (including random ciphertext and client time) must remain byte-identical.
    fn resume_pending_operation(
        &mut self,
        object_id: &str,
    ) -> Result<Option<ApplyOutcome>, EngineError> {
        self.reject_tag_pending(object_id)?;
        self.protect_non_tag_server_wire(object_id)?;
        self.resume_pending_operation_with(object_id, |_, _| Ok(()))
    }

    fn resume_pending_operation_with<F>(
        &mut self,
        object_id: &str,
        on_applied: F,
    ) -> Result<Option<ApplyOutcome>, EngineError>
    where
        F: FnOnce(&PendingOperation, i64) -> Result<(), EngineError>,
    {
        let Some(pending) = self.pending.get(object_id).cloned() else {
            return Ok(None);
        };
        if pending.op.object_id != object_id
            || pending.op.workspace_id != self.workspace_id
            || pending.device_id != self.client.device_id()
        {
            return Err(EngineError::PendingRequestMismatch);
        }
        // Retry persistence as well: an earlier failed write must never allow
        // an in-memory-only request to reach the authority on the next call.
        self.pending
            .get_mut(object_id)
            .ok_or(EngineError::PendingRequestMismatch)?
            .may_have_committed = true;
        self.persist()?;
        let outcome = match self.client.apply_op(&pending.op) {
            Ok(outcome) => outcome,
            Err(error) => {
                // Only a rejection of the first attempt proves non-commit.
                // Denying a replay cannot erase an earlier unknown receipt.
                if !pending.may_have_committed
                    && matches!(&error, pkms_sync_client::ClientError::Api { status, .. } if (400..500).contains(status))
                {
                    self.pending.remove(object_id);
                    self.reset_unpublished_fork(&pending.op.op_id);
                    self.persist()?;
                }
                return Err(error.into());
            }
        };
        if let ApplyOutcome::Applied(applied) = &outcome {
            if pending.probe {
                return Err(EngineError::ConflictProbeApplied);
            }
            if let Some(conflict_id) = &pending.keep_local_conflict {
                self.client.resolve_conflict(
                    conflict_id,
                    ConflictResolution::KeepLocal,
                    Some(&pending.op.op_id),
                    None,
                )?;
            }
            on_applied(&pending, applied.version)?;
            self.record_pending_receipt(object_id, &pending, applied.version, true)?;
        } else {
            self.pending.remove(object_id);
            self.reset_unpublished_fork(&pending.op.op_id);
            self.persist()?;
        }
        Ok(Some(outcome))
    }

    fn reset_unpublished_fork(&mut self, op_id: &str) {
        for plan in self.forks.values_mut() {
            if plan.publish_op_id.as_deref() == Some(op_id) && plan.published_version.is_none() {
                plan.publish_op_id = None;
            }
        }
    }

    fn record_pending_receipt(
        &mut self,
        object_id: &str,
        pending: &PendingOperation,
        version: i64,
        complete: bool,
    ) -> Result<(), EngineError> {
        if self
            .versions
            .get(object_id)
            .is_none_or(|known| *known <= version)
        {
            if let Some(known) = &pending.known {
                self.known.insert(object_id.into(), known.clone());
            } else {
                self.known.remove(object_id);
            }
            self.versions.insert(object_id.into(), version);
        }
        if pending.clear_config_conflict {
            self.config_conflicts.remove(object_id);
        }
        for plan in self.forks.values_mut() {
            if plan.publish_op_id.as_deref() == Some(pending.op.op_id.as_str()) {
                plan.published_version = Some(version);
            }
        }
        if complete {
            self.pending.remove(object_id);
        }
        self.persist()
    }

    fn pending_receipt_from_log(
        &self,
        remote: &RemoteOp,
    ) -> Result<Option<PendingOperation>, EngineError> {
        let Some(pending) = self.pending.get(&remote.object_id).cloned() else {
            return Ok(None);
        };
        let op = &pending.op;
        if op.op_id != remote.op_id {
            return Ok(None);
        }
        if op.workspace_id != remote.workspace_id
            || op.object_id != remote.object_id
            || pending.device_id != remote.device_id
            || op.kind != remote.kind
            || op.base_version != remote.base_version
            || op.key_epoch != remote.key_epoch
            || op.ciphertext != remote.ciphertext
            || op.client_time != remote.client_time
            || Some(remote.resulting_version) != op.base_version.checked_add(1)
        {
            return Err(EngineError::PendingRequestMismatch);
        }
        Ok(Some(pending))
    }

    fn acknowledge_pending_from_log(&mut self, remote: &RemoteOp) -> Result<(), EngineError> {
        let Some(pending) = self.pending_receipt_from_log(remote)? else {
            return Ok(());
        };
        if pending.probe {
            return Err(EngineError::ConflictProbeApplied);
        }
        // A viewer may confirm an earlier write by reading its exact immutable
        // log entry. A separate keep-local confirmation still needs write access.
        self.record_pending_receipt(
            &remote.object_id,
            &pending,
            remote.resulting_version,
            pending.keep_local_conflict.is_none(),
        )
    }

    fn resume_before_push(&mut self, object_id: &str) -> Result<Option<PushOutcome>, EngineError> {
        Ok(self.resume_pending_operation(object_id)?.map(push_outcome))
    }

    /// Marks local objects as the sync baseline when their current content is
    /// byte-for-byte equivalent to the encrypted remote object. Use this once
    /// after copying a library to a second device, before the first pull.
    pub fn seed_from_snapshot(&mut self, library: &Library) -> Result<(), EngineError> {
        let mut tag_owners = TagWireOwners::from_library(library)?;
        let objects = self.client.snapshot(&self.workspace_id)?;
        for object in objects {
            let inspected = self.inspect_snapshot_for_tags(&object, &mut tag_owners)?;
            if self.pending.contains_key(&object.object_id) {
                // The original request still owns its baseline until its
                // receipt is recovered; a snapshot cannot replace that receipt.
                continue;
            }
            if object.tombstone {
                if let Some(note) = optional_local(library.note(&object.object_id))?
                    && note.trashed
                {
                    self.mark_known(&object.object_id, &note, object.version);
                } else if library.config_tombstone_kind(&object.object_id)?.is_some() {
                    self.known
                        .insert(object.object_id.clone(), deleted_config_state());
                    self.versions
                        .insert(object.object_id.clone(), object.version);
                } else if optional_local(library.note(&object.object_id))?.is_none()
                    && optional_local(library.view_get(&object.object_id))?.is_none()
                    && optional_local(library.canvas(&object.object_id))?.is_none()
                {
                    // A full backup may retain neither a deleted configuration
                    // nor its acknowledged local tombstone. Keep the remote
                    // deletion's history floor without inventing a local kind.
                    self.versions
                        .insert(object.object_id.clone(), object.version);
                }
                continue;
            }
            let Some(payload) = inspected else {
                continue;
            };
            match payload {
                Payload::Note(payload) => {
                    if let Some(note) = optional_local(library.note(&object.object_id))?
                        && !note.trashed
                        && payload.matches_note(&note)
                    {
                        self.mark_known(&object.object_id, &note, object.version);
                    }
                }
                Payload::View(view)
                    if view.id == object.object_id
                        && optional_local(library.view_get(&object.object_id))?
                            .is_some_and(|local| local == view) =>
                {
                    let fingerprint = view_fingerprint(&view)?;
                    self.mark_known_config(&object.object_id, fingerprint, object.version);
                }
                Payload::Canvas(canvas)
                    if canvas.id == object.object_id
                        && optional_local(library.canvas(&object.object_id))?
                            .is_some_and(|local| local == canvas) =>
                {
                    let fingerprint = canvas_fingerprint(&canvas)?;
                    self.mark_known_config(&object.object_id, fingerprint, object.version);
                }
                Payload::WorkspaceMetadata(metadata)
                    if object.object_id == WORKSPACE_METADATA_OBJECT_ID
                        && library.workspace_metadata()? == metadata =>
                {
                    let fingerprint = workspace_metadata_fingerprint(&metadata)?;
                    self.mark_known_config(&object.object_id, fingerprint, object.version);
                }
                _ => {}
            }
        }
        self.persist()?;
        Ok(())
    }

    pub fn push_note(
        &mut self,
        library: &Library,
        note_id: &str,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, note_id)?;
        let recovered = self.resume_before_push(note_id)?;
        if let Some(conflict @ PushOutcome::Conflict { .. }) = &recovered {
            return Ok(conflict.clone());
        }
        let note = library.note(note_id)?;
        if note.trashed {
            if recovered.is_some() {
                return self.push_delete(library, note_id);
            }
            return Err(EngineError::LocalTrashed);
        }
        let base_version = self.versions.get(note_id).copied().unwrap_or(0);
        let snapshot = self.client.snapshot(&self.workspace_id)?;
        let current = snapshot.iter().find(|object| object.object_id == note_id);
        if let Some(current) = current {
            self.protect_non_tag_remote_object(current)?;
        }
        if let Some(current) = current
            && current.version == base_version
            && !current.tombstone
            && let Some(ciphertext) = current.ciphertext.as_deref()
            && let Ok(payload) = self.open_note_payload(note_id, current.key_epoch, ciphertext)
            && payload.matches_note(&note)
        {
            self.mark_known(note_id, &note, current.version);
            self.persist()?;
            return Ok(PushOutcome::Unchanged {
                version: current.version,
            });
        }

        let ciphertext = self.seal_payload(note_id, &NotePayload::from_note(&note))?;
        let op_id = self.fresh_op_id();
        let op = self.op(op_id, note_id, OpKind::Put, base_version, Some(ciphertext));
        self.submit_operation(op, Some(note_state(&note)), false, None, false)
            .map(push_outcome)
    }

    pub fn push_view(
        &mut self,
        library: &Library,
        view_id: &str,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, view_id)?;
        let view = library.view_get(view_id)?;
        let fingerprint = view_fingerprint(&view)?;
        self.push_config_object(ConfigPayload::View { view }, view_id, fingerprint)
    }

    pub fn push_canvas(
        &mut self,
        library: &Library,
        canvas_id: &str,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, canvas_id)?;
        let canvas = library.canvas(canvas_id)?;
        let fingerprint = canvas_fingerprint(&canvas)?;
        self.push_config_object(ConfigPayload::Canvas { canvas }, canvas_id, fingerprint)
    }

    pub fn push_workspace_metadata(
        &mut self,
        library: &Library,
    ) -> Result<PushOutcome, EngineError> {
        let metadata = library.workspace_metadata()?;
        let fingerprint = workspace_metadata_fingerprint(&metadata)?;
        self.push_config_object(
            ConfigPayload::WorkspaceMetadata { metadata },
            WORKSPACE_METADATA_OBJECT_ID,
            fingerprint,
        )
    }

    fn config_conflict_id(
        &self,
        object_id: &str,
        remote_version: i64,
    ) -> Result<Option<String>, EngineError> {
        Ok(self
            .scoped_conflicts(
                self.client
                    .pull(&self.workspace_id, self.cursor, 1)?
                    .conflicts,
            )
            .into_iter()
            .find(|conflict| {
                conflict.object_id == object_id && conflict.remote_version == remote_version
            })
            .map(|conflict| conflict.conflict_id))
    }

    pub fn local_config_state(
        &self,
        library: &Library,
        object_id: &str,
    ) -> Result<ConfigLocalState, EngineError> {
        tag_sync::protect_non_tag_wire(library, object_id)?;
        let (title, available, tombstone, local_fingerprint) =
            if object_id == WORKSPACE_METADATA_OBJECT_ID {
                let metadata = library.workspace_metadata()?;
                (
                    "Workspace metadata".into(),
                    true,
                    false,
                    workspace_metadata_fingerprint(&metadata)?,
                )
            } else if let Some(view) = optional_local(library.view_get(object_id))? {
                let fingerprint = view_fingerprint(&view)?;
                (view.name, true, false, fingerprint)
            } else if let Some(canvas) = optional_local(library.canvas(object_id))? {
                let fingerprint = canvas_fingerprint(&canvas)?;
                (canvas.name, true, false, fingerprint)
            } else if let Some(deleted) = library
                .list_config_tombstones()?
                .into_iter()
                .find(|deleted| deleted.object_id == object_id)
            {
                let fingerprint = pkms_sync::sha256_hex(&serde_json::to_vec(&(
                    deleted.object_id,
                    deleted.kind,
                    library
                        .config_tombstone_stamp(object_id)?
                        .ok_or(EngineError::ConfigPreviewChanged)?,
                ))?);
                (String::new(), false, true, fingerprint)
            } else {
                (String::new(), false, false, "missing".into())
            };
        let token = pkms_sync::sha256_hex(&serde_json::to_vec(&(
            "local-config-preview-v1",
            &self.workspace_id,
            object_id,
            available,
            tombstone,
            local_fingerprint,
        ))?);
        Ok(ConfigLocalState {
            title,
            available,
            tombstone,
            local_state_token: token,
        })
    }

    pub fn keep_local_config_checked(
        &mut self,
        library: &Library,
        object_id: &str,
        remote_version: i64,
        local_state_token: &str,
    ) -> Result<PushOutcome, EngineError> {
        if self
            .local_config_state(library, object_id)?
            .local_state_token
            != local_state_token
        {
            return Err(EngineError::ConfigPreviewChanged);
        }
        self.keep_local_config(library, object_id, remote_version)
    }

    pub fn keep_local_config(
        &mut self,
        library: &Library,
        object_id: &str,
        remote_version: i64,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, object_id)?;
        if object_id == WORKSPACE_METADATA_OBJECT_ID {
            let metadata = library.workspace_metadata()?;
            let fingerprint = workspace_metadata_fingerprint(&metadata)?;
            return self.push_config_object_at(
                ConfigPayload::WorkspaceMetadata { metadata },
                object_id,
                fingerprint,
                Some(remote_version),
            );
        }
        match library.view_get(object_id) {
            Ok(view) => {
                let fingerprint = view_fingerprint(&view)?;
                return self.push_config_object_at(
                    ConfigPayload::View { view },
                    object_id,
                    fingerprint,
                    Some(remote_version),
                );
            }
            Err(pkms_core::Error::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        match library.canvas(object_id) {
            Ok(canvas) => {
                let fingerprint = canvas_fingerprint(&canvas)?;
                return self.push_config_object_at(
                    ConfigPayload::Canvas { canvas },
                    object_id,
                    fingerprint,
                    Some(remote_version),
                );
            }
            Err(pkms_core::Error::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        if library.config_tombstone_kind(object_id)?.is_some() {
            return self.push_config_delete_at(object_id, Some(remote_version));
        }
        Err(EngineError::LocalConfigMissing)
    }

    /// UI callers should bind the displayed local and remote states with the checked variant.
    pub fn keep_remote_config(
        &mut self,
        library: &mut Library,
        object_id: &str,
    ) -> Result<i64, EngineError> {
        self.keep_remote_config_inner(library, object_id, None)
    }

    pub fn keep_remote_config_checked(
        &mut self,
        library: &mut Library,
        object_id: &str,
        remote_version: i64,
        local_state_token: &str,
    ) -> Result<i64, EngineError> {
        self.keep_remote_config_inner(
            library,
            object_id,
            Some((remote_version, local_state_token)),
        )
    }

    fn keep_remote_config_inner(
        &mut self,
        library: &mut Library,
        object_id: &str,
        expected: Option<(i64, &str)>,
    ) -> Result<i64, EngineError> {
        tag_sync::protect_non_tag_wire(library, object_id)?;
        self.resume_pending_operation(object_id)?;
        let snapshot = self.client.snapshot(&self.workspace_id)?;
        let object = snapshot
            .iter()
            .find(|object| object.object_id == object_id)
            .ok_or(EngineError::RemoteMissing)?;
        self.protect_non_tag_remote_object(object)?;
        let version = object.version;
        if let Some((remote_version, local_state_token)) = expected {
            if version != remote_version {
                return Err(EngineError::ConfigConflictChanged);
            }
            if self
                .local_config_state(library, object_id)?
                .local_state_token
                != local_state_token
            {
                return Err(EngineError::ConfigPreviewChanged);
            }
        }

        let server_conflict_id = self.config_conflict_id(object_id, version)?;
        if object.tombstone {
            if object_id == WORKSPACE_METADATA_OBJECT_ID {
                return Err(EngineError::RemoteMissing);
            }
            if library.view_get(object_id).is_ok() {
                library.view_remove_remote(object_id)?;
            } else if library.canvas(object_id).is_ok() {
                library.canvas_remove_remote(object_id)?;
            } else if library.config_tombstone_kind(object_id)?.is_none() {
                return Err(EngineError::LocalConfigMissing);
            }
            self.known.insert(object_id.into(), deleted_config_state());
        } else {
            let ciphertext = object
                .ciphertext
                .as_deref()
                .ok_or(EngineError::MissingCiphertext)?;
            let fingerprint =
                match self.open_any_payload(object_id, object.key_epoch, ciphertext)? {
                    Payload::View(view) if view.id == object_id => {
                        library.view_apply_remote(view.clone())?;
                        view_fingerprint(&view)?
                    }
                    Payload::Canvas(canvas) if canvas.id == object_id => {
                        library.canvas_apply_remote(canvas.clone())?;
                        canvas_fingerprint(&canvas)?
                    }
                    Payload::WorkspaceMetadata(metadata)
                        if object_id == WORKSPACE_METADATA_OBJECT_ID =>
                    {
                        library.workspace_metadata_apply_remote(metadata.clone())?;
                        workspace_metadata_fingerprint(&metadata)?
                    }
                    _ => return Err(EngineError::RemoteMissing),
                };
            self.mark_known_config(object_id, fingerprint, version);
        }
        self.versions.insert(object_id.to_owned(), version);
        if let Some(conflict_id) = server_conflict_id {
            self.client.resolve_conflict(
                &conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
            )?;
        }
        self.config_conflicts.remove(object_id);
        self.persist()?;
        Ok(version)
    }

    fn push_config_object(
        &mut self,
        payload: ConfigPayload,
        object_id: &str,
        fingerprint: String,
    ) -> Result<PushOutcome, EngineError> {
        self.push_config_object_at(payload, object_id, fingerprint, None)
    }

    fn push_config_object_at(
        &mut self,
        payload: ConfigPayload,
        object_id: &str,
        fingerprint: String,
        mut base_override: Option<i64>,
    ) -> Result<PushOutcome, EngineError> {
        match self.resume_before_push(object_id)? {
            Some(PushOutcome::Conflict { remote_version, .. })
                if base_override == Some(remote_version) => {}
            Some(conflict @ PushOutcome::Conflict { .. }) => return Ok(conflict),
            Some(PushOutcome::Applied { version }) if base_override.is_some() => {
                base_override = Some(version)
            }
            _ => {}
        }
        let base_version = base_override
            .or_else(|| self.versions.get(object_id).copied())
            .unwrap_or(0);
        let snapshot = self.client.snapshot(&self.workspace_id)?;
        let current = snapshot.iter().find(|object| object.object_id == object_id);
        if let Some(current) = current {
            self.protect_non_tag_remote_object(current)?;
        }
        if let Some(current) = current
            && current.version == base_version
            && !current.tombstone
            && let Some(ciphertext) = current.ciphertext.as_deref()
            && let Ok(remote) = self.open_any_payload(object_id, current.key_epoch, ciphertext)
            && config_payload_matches(&remote, &payload)
        {
            if base_override.is_some()
                && let Some(conflict_id) = self.config_conflict_id(object_id, base_version)?
            {
                // Both sides already contain the requested local value.
                self.client.resolve_conflict(
                    &conflict_id,
                    ConflictResolution::KeepRemote,
                    None,
                    None,
                )?;
            }
            self.mark_known_config(object_id, fingerprint, current.version);
            self.config_conflicts.remove(object_id);
            self.persist()?;
            return Ok(PushOutcome::Unchanged {
                version: current.version,
            });
        }
        let ciphertext = self.seal_payload(object_id, &payload)?;
        let op_id = self.fresh_op_id();
        let kind = if base_override.is_some()
            && current.is_some_and(|object| {
                object.version == base_version && object.tombstone && !object.purged
            }) {
            OpKind::Restore
        } else {
            OpKind::Put
        };
        let conflict_id = if base_override.is_some() {
            self.config_conflict_id(object_id, base_version)?
        } else {
            None
        };
        let op = self.op(op_id, object_id, kind, base_version, Some(ciphertext));
        self.submit_operation(
            op,
            Some(KnownState {
                revision_id: fingerprint.clone(),
                trashed: false,
            }),
            true,
            conflict_id,
            false,
        )
        .map(push_outcome)
    }

    pub fn push_view_remove(&mut self, object_id: &str) -> Result<PushOutcome, EngineError> {
        self.push_config_delete(object_id)
    }

    pub fn push_canvas_remove(&mut self, object_id: &str) -> Result<PushOutcome, EngineError> {
        self.push_config_delete(object_id)
    }

    fn push_config_delete(&mut self, object_id: &str) -> Result<PushOutcome, EngineError> {
        self.push_config_delete_at(object_id, None)
    }

    fn push_config_delete_at(
        &mut self,
        object_id: &str,
        mut base_override: Option<i64>,
    ) -> Result<PushOutcome, EngineError> {
        match self.resume_before_push(object_id)? {
            Some(PushOutcome::Conflict { remote_version, .. })
                if base_override == Some(remote_version) => {}
            Some(conflict @ PushOutcome::Conflict { .. }) => return Ok(conflict),
            Some(PushOutcome::Applied { version }) if base_override.is_some() => {
                base_override = Some(version)
            }
            _ => {}
        }
        let base_version = base_override
            .or_else(|| self.versions.get(object_id).copied())
            .ok_or(EngineError::RemoteMissing)?;
        if self
            .client
            .snapshot(&self.workspace_id)?
            .iter()
            .any(|object| {
                object.object_id == object_id && object.version == base_version && object.tombstone
            })
        {
            if base_override.is_some()
                && let Some(conflict_id) = self.config_conflict_id(object_id, base_version)?
            {
                self.client.resolve_conflict(
                    &conflict_id,
                    ConflictResolution::KeepRemote,
                    None,
                    None,
                )?;
            }
            self.known.insert(object_id.into(), deleted_config_state());
            self.config_conflicts.remove(object_id);
            self.persist()?;
            return Ok(PushOutcome::Unchanged {
                version: base_version,
            });
        }
        let op_id = self.fresh_op_id();
        let op = self.op(op_id, object_id, OpKind::Delete, base_version, None);
        let conflict_id = if base_override.is_some() {
            self.config_conflict_id(object_id, base_version)?
        } else {
            None
        };
        self.submit_operation(op, Some(deleted_config_state()), true, conflict_id, false)
            .map(push_outcome)
    }
    pub fn pull_and_apply(
        &mut self,
        library: &mut Library,
        limit: usize,
    ) -> Result<PullSummary, EngineError> {
        let mut tag_owners = TagWireOwners::from_library(library)?;
        // Recover our own acknowledged/unknown writes before comparing local
        // content against the remote stream. Otherwise a lost receipt can look
        // like an unrelated edit and manufacture a local conflict.
        let mut pending_ids: Vec<_> = self.pending.keys().cloned().collect();
        pending_ids.sort();
        for object_id in pending_ids {
            match self.resume_pending_with_core(library, &object_id, &mut tag_owners) {
                Err(EngineError::Client(pkms_sync_client::ClientError::Api {
                    status: 403,
                    ..
                })) => {}
                result => {
                    result?;
                }
            }
        }
        let batch = self.client.pull(&self.workspace_id, self.cursor, limit)?;
        let mut summary = PullSummary {
            server_conflicts: self.scoped_conflicts(batch.conflicts.clone()),
            next_cursor: batch.next_cursor,
            ..PullSummary::default()
        };
        let mut conflicted = HashSet::new();
        for op in &batch.ops {
            let parsed = self.decode_remote_for_tags(op, &mut tag_owners)?;
            self.acknowledge_pending_with_core(library, op)?;
            // Ops at or below the last version known by this engine are its own
            // already-applied history. Replaying them must not reset the local
            // baseline (for example, an unsent local delete).
            if !matches!(&parsed, Some(Payload::Tag(_)))
                && op.resulting_version <= self.versions.get(&op.object_id).copied().unwrap_or(0)
            {
                summary.skipped.push(op.object_id.clone());
                continue;
            }
            let conflicts_before = summary.conflicts.len() + summary.config_conflicts.len();
            match parsed {
                Some(Payload::Tag(payload)) => {
                    summary
                        .tag_results
                        .push(self.apply_remote_tag(library, op, payload)?);
                }
                Some(Payload::Note(payload)) => {
                    self.apply_remote_live(library, op, payload, &mut summary)?
                }
                Some(Payload::View(view)) => {
                    self.apply_remote_view(library, view, op.resulting_version, &mut summary)?
                }
                Some(Payload::Canvas(canvas)) => {
                    self.apply_remote_canvas(library, canvas, op.resulting_version, &mut summary)?
                }
                Some(Payload::WorkspaceMetadata(metadata)) => self
                    .apply_remote_workspace_metadata(
                        library,
                        metadata,
                        op.resulting_version,
                        &mut summary,
                    )?,
                None => self.apply_remote_delete(library, op, &mut summary)?,
            }
            if summary.conflicts.len() + summary.config_conflicts.len() > conflicts_before {
                conflicted.insert(op.object_id.clone());
            }
        }
        // A pull-side local/remote divergence must become a durable server
        // conflict, not just a transient comparison row. Submit a probe op at
        // the last synced base; the authority records the conflict without
        // changing the object. A subsequent read returns the conflict id for
        // the explicit comparison UI.
        let mut probed = false;
        for object_id in &conflicted {
            if summary
                .server_conflicts
                .iter()
                .any(|conflict| conflict.object_id == *object_id)
            {
                continue;
            }
            if let Ok(note) = library.note(object_id)
                && self.probe_conflict(&note)?
            {
                probed = true;
            }
        }
        if probed
            && let Ok(conflicts) = self
                .client
                .pull(&self.workspace_id, batch.next_cursor, 1)
                .map(|next| next.conflicts)
            && !conflicts.is_empty()
        {
            summary.server_conflicts = self.scoped_conflicts(conflicts);
        }
        // Keep the last known version for conflicted objects. Advancing it
        // here would let a later push overwrite the remote edit without a
        // decision from the comparison UI.
        for op in &batch.ops {
            if !conflicted.contains(&op.object_id) {
                self.versions
                    .entry(op.object_id.clone())
                    .and_modify(|version| *version = (*version).max(op.resulting_version))
                    .or_insert(op.resulting_version);
            }
        }
        // A permanent dependency in an early page must not starve later entries.
        let mut retry_after = None;
        loop {
            let retried = self.retry_pending_tags(library, 100, retry_after)?;
            summary.tag_results.extend(retried.items);
            retry_after = retried.next_cursor;
            if retry_after.is_none() {
                break;
            }
        }
        summary
            .server_conflicts
            .retain(|conflict| !tag_owners.contains(&conflict.object_id));
        self.cursor = batch.next_cursor;
        for conflict in &summary.config_conflicts {
            self.config_conflicts
                .insert(conflict.object_id.clone(), conflict.clone());
        }
        summary.config_conflicts = self.pending_config_conflicts();
        self.persist()?;
        Ok(summary)
    }

    pub fn push_delete(
        &mut self,
        library: &Library,
        note_id: &str,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, note_id)?;
        let recovered = self.resume_before_push(note_id)?;
        if let Some(conflict @ PushOutcome::Conflict { .. }) = &recovered {
            return Ok(conflict.clone());
        }
        let note = library.note(note_id)?;
        if !note.trashed {
            if recovered.is_some() {
                return self.push_restore(library, note_id);
            }
            return Err(pkms_core::Error::InvalidInput(
                "sync deletion requires a locally trashed note",
            )
            .into());
        }
        let base_version = self
            .versions
            .get(note_id)
            .copied()
            .ok_or(EngineError::RemoteMissing)?;
        if self
            .client
            .snapshot(&self.workspace_id)?
            .iter()
            .any(|object| {
                object.object_id == note_id && object.version == base_version && object.tombstone
            })
        {
            self.mark_known(note_id, &note, base_version);
            self.persist()?;
            return Ok(PushOutcome::Unchanged {
                version: base_version,
            });
        }
        let op_id = self.fresh_op_id();
        let op = self.op(op_id, note_id, OpKind::Delete, base_version, None);
        self.submit_operation(op, Some(note_state(&note)), false, None, false)
            .map(push_outcome)
    }

    pub fn push_restore(
        &mut self,
        library: &Library,
        note_id: &str,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, note_id)?;
        let recovered = self.resume_before_push(note_id)?;
        if let Some(conflict @ PushOutcome::Conflict { .. }) = &recovered {
            return Ok(conflict.clone());
        }
        let note = library.note(note_id)?;
        if note.trashed {
            if recovered.is_some() {
                return self.push_delete(library, note_id);
            }
            return Err(EngineError::LocalTrashed);
        }
        let base_version = self
            .versions
            .get(note_id)
            .copied()
            .ok_or(EngineError::RemoteMissing)?;
        if let Some(object) = self
            .client
            .snapshot(&self.workspace_id)?
            .iter()
            .find(|object| {
                object.object_id == note_id && object.version == base_version && !object.tombstone
            })
        {
            let ciphertext = object
                .ciphertext
                .as_deref()
                .ok_or(EngineError::MissingCiphertext)?;
            if !self
                .open_note_payload(note_id, object.key_epoch, ciphertext)?
                .matches_note(&note)
            {
                // A previously submitted restore is now live. Subsequent local
                // edits are puts at its confirmed base, not another restore.
                return self.push_note(library, note_id);
            }
            self.mark_known(note_id, &note, base_version);
            self.persist()?;
            return Ok(PushOutcome::Unchanged {
                version: base_version,
            });
        }
        let ciphertext = self.seal_payload(note_id, &NotePayload::from_note(&note))?;
        let op_id = self.fresh_op_id();
        let op = self.op(
            op_id,
            note_id,
            OpKind::Restore,
            base_version,
            Some(ciphertext),
        );
        self.submit_operation(op, Some(note_state(&note)), false, None, false)
            .map(push_outcome)
    }
    pub fn resolve_keep_local(
        &mut self,
        library: &Library,
        conflict: &ConflictRecord,
    ) -> Result<PushOutcome, EngineError> {
        self.resolve_keep_local_target(library, &ConflictTarget::from(conflict))
    }

    pub fn resolve_keep_local_target(
        &mut self,
        library: &Library,
        conflict: &ConflictTarget,
    ) -> Result<PushOutcome, EngineError> {
        tag_sync::protect_non_tag_wire(library, &conflict.object_id)?;
        let result = self.resolve_keep_local_target_inner(library, conflict)?;
        if !matches!(result, PushOutcome::Conflict { .. }) {
            self.supersede_fork_plan(&conflict.conflict_id)?;
        }
        Ok(result)
    }

    fn resolve_keep_local_target_inner(
        &mut self,
        library: &Library,
        conflict: &ConflictTarget,
    ) -> Result<PushOutcome, EngineError> {
        let recovering_resolution = self
            .pending
            .get(&conflict.object_id)
            .is_some_and(|pending| {
                pending.keep_local_conflict.as_deref() == Some(conflict.conflict_id.as_str())
            });
        match self.resume_before_push(&conflict.object_id)? {
            Some(conflict @ PushOutcome::Conflict { .. }) => return Ok(conflict),
            Some(outcome @ PushOutcome::Applied { .. }) if recovering_resolution => {
                let note = library.note(&conflict.object_id)?;
                if self.is_known(&note.id, &note) {
                    return Ok(outcome);
                }
                return if note.trashed {
                    self.push_delete(library, &note.id)
                } else if self.known.get(&note.id).is_some_and(|known| known.trashed) {
                    self.push_restore(library, &note.id)
                } else {
                    self.push_note(library, &note.id)
                };
            }
            _ => {}
        }
        let note = library.note(&conflict.object_id)?;
        let (kind, ciphertext) = if note.trashed {
            (OpKind::Delete, None)
        } else if conflict.remote_tombstone {
            (
                OpKind::Restore,
                Some(self.seal_payload(&conflict.object_id, &NotePayload::from_note(&note))?),
            )
        } else {
            (
                OpKind::Put,
                Some(self.seal_payload(&conflict.object_id, &NotePayload::from_note(&note))?),
            )
        };
        let op_id = self.fresh_op_id();
        let op = self.op(
            op_id.clone(),
            &conflict.object_id,
            kind,
            conflict.remote_version,
            ciphertext,
        );
        self.submit_operation(
            op,
            Some(note_state(&note)),
            false,
            Some(conflict.conflict_id.clone()),
            false,
        )
        .map(push_outcome)
    }

    pub fn resolve_keep_remote(
        &mut self,
        library: &mut Library,
        conflict: &ConflictRecord,
    ) -> Result<(), EngineError> {
        self.resolve_keep_remote_target(library, &ConflictTarget::from(conflict))
    }

    pub fn resolve_keep_remote_target(
        &mut self,
        library: &mut Library,
        conflict: &ConflictTarget,
    ) -> Result<(), EngineError> {
        tag_sync::protect_non_tag_wire(library, &conflict.object_id)?;
        self.resume_pending_operation(&conflict.object_id)?;
        self.client.resolve_conflict(
            &conflict.conflict_id,
            ConflictResolution::KeepRemote,
            None,
            None,
        )?;
        self.apply_remote_force(library, &conflict.object_id)?;
        self.supersede_fork_plan(&conflict.conflict_id)?;
        Ok(())
    }

    pub fn resolve_save_as_new(
        &mut self,
        library: &mut Library,
        conflict: &ConflictRecord,
    ) -> Result<String, EngineError> {
        self.resolve_save_as_new_target(library, &ConflictTarget::from(conflict))
    }

    pub fn resolve_save_as_new_target(
        &mut self,
        library: &mut Library,
        conflict: &ConflictTarget,
    ) -> Result<String, EngineError> {
        tag_sync::protect_non_tag_wire(library, &conflict.object_id)?;
        self.protect_non_tag_server_wire(&conflict.object_id)?;
        let key = &conflict.conflict_id;
        if !self.forks.contains_key(key) {
            self.resume_pending_operation(&conflict.object_id)?;
            self.require_current_conflict(conflict)?;
            let source = library.note(&conflict.object_id)?;
            let metadata = library.object_metadata(&conflict.object_id)?;
            let checked = library.note(&conflict.object_id)?;
            if source.trashed || metadata.trashed || checked.trashed {
                return Err(EngineError::LocalTrashed);
            }
            if source.revision_id != checked.revision_id {
                return Err(pkms_core::Error::RevisionConflict {
                    expected: source.revision_id,
                    actual: checked.revision_id,
                }
                .into());
            }
            if library.info()?.workspace_id != self.workspace_id {
                return Err(EngineError::PendingRequestMismatch);
            }
            let copy_command_id = self.fork_copy_command_id(conflict)?;
            self.forks.insert(
                key.clone(),
                ForkPlan {
                    conflict: conflict.clone(),
                    workspace_id: self.workspace_id.clone(),
                    device_id: self.client.device_id().into(),
                    source_revision_id: source.revision_id,
                    source_content_hash: source.content_hash,
                    source_row_version: metadata.row_version,
                    copy_command_id,
                    fork_id: None,
                    fork_revision_id: None,
                    publish_op_id: None,
                    published_version: None,
                    adoption: None,
                    completed: false,
                    superseded: false,
                },
            );
        }
        let mut plan = self.fork_plan(key)?;
        if plan.workspace_id != self.workspace_id
            || plan.device_id != self.client.device_id()
            || plan.conflict.object_id != conflict.object_id
            || plan.conflict.conflict_id != *key
        {
            return Err(EngineError::PendingRequestMismatch);
        }
        if plan.superseded {
            return Err(EngineError::ForkSuperseded);
        }
        // This also retries a previously failed plan write before any Core write.
        self.persist()?;
        if plan.completed {
            return plan.fork_id.ok_or(EngineError::PendingRequestMismatch);
        }

        let source = library.revision(&conflict.object_id, &plan.source_revision_id)?;
        if source.content_hash != plan.source_content_hash {
            return Err(pkms_core::Error::IntegrityFailure.into());
        }
        // Core checks this command receipt before allocating the copy object,
        // block IDs and occurrence IDs. Rebuilding random copies is not a retry.
        let copy = library.copy_document(
            &plan.copy_command_id,
            CopyNote {
                note_id: conflict.object_id.clone(),
                source_revision_id: plan.source_revision_id.clone(),
                title: source.title,
            },
        )?;
        if plan.fork_id.as_ref().is_some_and(|id| *id != copy.note.id)
            || plan
                .fork_revision_id
                .as_ref()
                .is_some_and(|id| *id != copy.note.revision_id)
        {
            return Err(EngineError::PendingRequestMismatch);
        }
        let fork_id = copy.note.id.clone();
        tag_sync::protect_non_tag_wire(library, &fork_id)?;
        plan.fork_id = Some(fork_id.clone());
        plan.fork_revision_id = Some(copy.note.revision_id.clone());
        self.store_fork_plan(key, plan.clone())?;
        if library.note(&fork_id)?.trashed {
            return Err(EngineError::LocalTrashed);
        }

        if copy.receipt.replayed && plan.publish_op_id.is_none() && plan.published_version.is_none()
        {
            self.recover_fork_publication(key, &copy.note)?;
            plan = self.fork_plan(key)?;
        }
        if plan.published_version.is_none() {
            let outcome = if let Some(op_id) = &plan.publish_op_id {
                if !self
                    .pending
                    .get(&fork_id)
                    .is_some_and(|pending| pending.op.op_id == *op_id)
                {
                    return Err(EngineError::PendingRequestMismatch);
                }
                self.resume_pending_operation(&fork_id)?
                    .ok_or(EngineError::PendingRequestMismatch)?
            } else {
                let ciphertext =
                    self.seal_payload(&fork_id, &NotePayload::from_note(&copy.note))?;
                let op_id = self.fresh_op_id();
                let op = self.op(op_id.clone(), &fork_id, OpKind::Put, 0, Some(ciphertext));
                plan.publish_op_id = Some(op_id);
                self.forks.insert(key.clone(), plan);
                // Staging the original operation and its plan reference uses
                // the same state transaction before the request reaches HTTP.
                self.submit_operation(op, Some(note_state(&copy.note)), false, None, false)?
            };
            if matches!(outcome, ApplyOutcome::Conflict(_)) {
                return Err(EngineError::ForkPublicationConflict);
            }
            plan = self.fork_plan(key)?;
        }
        if plan.adoption.is_none() {
            let remote = self
                .client
                .snapshot(&self.workspace_id)?
                .into_iter()
                .find(|object| object.object_id == conflict.object_id)
                .ok_or(EngineError::RemoteMissing)?;
            self.protect_non_tag_remote_object(&remote)?;
            if remote.last_seq < 1 || remote.version < plan.conflict.remote_version {
                return Err(EngineError::RemoteMissing);
            }
            plan.adoption = Some(ForkAdoption {
                server_seq: remote.last_seq,
                version: remote.version,
                tombstone: remote.tombstone,
                ciphertext_hash: remote
                    .ciphertext
                    .as_ref()
                    .map(|bytes| pkms_sync::sha256_hex(bytes)),
                command_id: Uuid::new_v4().to_string(),
                applied: None,
            });
            self.store_fork_plan(key, plan.clone())?;
        }
        let adoption = plan
            .adoption
            .clone()
            .ok_or(EngineError::PendingRequestMismatch)?;
        if adoption.applied.is_none() {
            self.require_current_conflict(&plan.conflict)?;
            let remote = self
                .client
                .pull(&self.workspace_id, adoption.server_seq - 1, 1)?
                .ops
                .into_iter()
                .find(|op| op.server_seq == adoption.server_seq)
                .ok_or(EngineError::RemoteMissing)?;
            if remote.object_id != conflict.object_id
                || remote.workspace_id != self.workspace_id
                || remote.resulting_version != adoption.version
                || remote.tombstone != adoption.tombstone
                || remote
                    .ciphertext
                    .as_ref()
                    .map(|bytes| pkms_sync::sha256_hex(bytes))
                    != adoption.ciphertext_hash
            {
                return Err(EngineError::PendingRequestMismatch);
            }
            let mut owners = TagWireOwners::from_library(library)?;
            let parsed = self.decode_remote_for_tags(&remote, &mut owners)?;
            if matches!(parsed, Some(Payload::Tag(_))) {
                return Err(pkms_core::Error::InvalidInput(
                    "tag metadata cannot be adopted through a note conflict",
                )
                .into());
            }
            let adopted = if adoption.tombstone {
                library.set_object_trashed(
                    &adoption.command_id,
                    pkms_core::SetObjectFlag {
                        object_id: conflict.object_id.clone(),
                        base_row_version: plan.source_row_version,
                        value: true,
                    },
                )?;
                KnownState {
                    revision_id: plan.source_revision_id.clone(),
                    trashed: true,
                }
            } else {
                let bytes = remote
                    .ciphertext
                    .as_deref()
                    .ok_or(EngineError::MissingCiphertext)?;
                let payload =
                    self.open_note_payload(&conflict.object_id, remote.key_epoch, bytes)?;
                let commit = library.save_document(
                    &adoption.command_id,
                    SaveNote {
                        note_id: conflict.object_id.clone(),
                        base_revision_id: plan.source_revision_id.clone(),
                        title: payload.title,
                        document_schema: payload.document_schema,
                        document: payload.document,
                    },
                )?;
                note_state(&commit.note)
            };
            // The Core command checks the captured revision/metadata version.
            // If the source was edited while a receipt was unknown, this fails
            // without closing the conflict or overwriting the later edit.
            self.known
                .insert(conflict.object_id.clone(), adopted.clone());
            self.versions
                .insert(conflict.object_id.clone(), adoption.version);
            plan.adoption
                .as_mut()
                .ok_or(EngineError::PendingRequestMismatch)?
                .applied = Some(adopted);
            self.store_fork_plan(key, plan.clone())?;
        }
        self.client.resolve_conflict(
            key,
            ConflictResolution::SaveAsNew,
            plan.publish_op_id.as_deref(),
            Some(&fork_id),
        )?;
        plan.completed = true;
        self.store_fork_plan(key, plan)?;
        Ok(fork_id)
    }

    fn fork_copy_command_id(&self, conflict: &ConflictTarget) -> Result<String, EngineError> {
        let hash = pkms_sync::sha256_hex(&serde_json::to_vec(&(
            "save-as-new-copy-command-v1",
            &self.workspace_id,
            self.client.device_id(),
            &conflict.conflict_id,
            &conflict.object_id,
        ))?);
        // This is a Core idempotency identity, never an encryption nonce.
        Uuid::parse_str(&hash[..32])
            .map(|id| id.to_string())
            .map_err(|_| EngineError::PendingRequestMismatch)
    }

    fn recover_fork_publication(
        &mut self,
        conflict_id: &str,
        copy: &Note,
    ) -> Result<(), EngineError> {
        let Some(object) = self
            .client
            .snapshot(&self.workspace_id)?
            .into_iter()
            .find(|object| object.object_id == copy.id)
        else {
            return Ok(());
        };
        self.protect_non_tag_remote_object(&object)?;
        // A restored Core receipt already owns this object ID. Never publish a
        // different copy just because its transport plan was not in the backup.
        if object.version != 1 || object.tombstone || object.purged || object.last_seq < 1 {
            return Err(EngineError::ForkReceiptUnavailable);
        }
        let op = self
            .client
            .pull(&self.workspace_id, object.last_seq - 1, 1)?
            .ops
            .into_iter()
            .find(|op| op.server_seq == object.last_seq)
            .ok_or(EngineError::ForkReceiptUnavailable)?;
        if op.workspace_id != self.workspace_id
            || op.object_id != copy.id
            || op.device_id != self.client.device_id()
            || op.kind != OpKind::Put
            || op.base_version != 0
            || op.resulting_version != 1
            || op.tombstone
            || op.purged
            || op.key_epoch != object.key_epoch
            || op.ciphertext != object.ciphertext
        {
            return Err(EngineError::ForkReceiptUnavailable);
        }
        let bytes = op
            .ciphertext
            .as_deref()
            .ok_or(EngineError::MissingCiphertext)?;
        if !self
            .open_note_payload(&copy.id, op.key_epoch, bytes)?
            .matches_note(copy)
        {
            return Err(EngineError::ForkReceiptUnavailable);
        }
        let mut plan = self.fork_plan(conflict_id)?;
        plan.publish_op_id = Some(op.op_id);
        plan.published_version = Some(1);
        self.mark_known(&copy.id, copy, 1);
        self.store_fork_plan(conflict_id, plan)
    }

    fn fork_plan(&self, conflict_id: &str) -> Result<ForkPlan, EngineError> {
        self.forks
            .get(conflict_id)
            .cloned()
            .ok_or(EngineError::PendingRequestMismatch)
    }

    fn store_fork_plan(&mut self, conflict_id: &str, plan: ForkPlan) -> Result<(), EngineError> {
        self.forks.insert(conflict_id.into(), plan);
        self.persist()
    }

    fn supersede_fork_plan(&mut self, conflict_id: &str) -> Result<(), EngineError> {
        if let Some(plan) = self.forks.get_mut(conflict_id)
            && !plan.completed
        {
            plan.superseded = true;
        }
        self.persist()
    }

    /// Returns the decrypted remote payload for one object, or `None` when the
    /// object is absent or tombstoned. Used by conflict comparison surfaces.
    pub fn remote_payload(&self, object_id: &str) -> Result<Option<NotePayload>, EngineError> {
        let objects = self.client.snapshot(&self.workspace_id)?;
        let Some(object) = objects.iter().find(|object| object.object_id == object_id) else {
            return Ok(None);
        };
        self.protect_non_tag_remote_object(object)?;
        if object.tombstone {
            return Ok(None);
        }
        let Some(ciphertext) = object.ciphertext.as_deref() else {
            return Ok(None);
        };
        self.open_note_payload(object_id, object.key_epoch, ciphertext)
            .map(Some)
    }
    /// Records a durable conflict without changing the remote object. The
    /// authority only writes a conflict row when the probe base is stale.
    fn probe_conflict(&mut self, note: &Note) -> Result<bool, EngineError> {
        if let Some(outcome) = self.resume_pending_operation(&note.id)? {
            return match outcome {
                ApplyOutcome::Conflict(_) => Ok(true),
                ApplyOutcome::Applied(_) => Err(EngineError::ConflictProbeApplied),
            };
        }
        let base_version = self.versions.get(&note.id).copied().unwrap_or(0);
        let (kind, ciphertext) = if note.trashed {
            (OpKind::Delete, None)
        } else {
            (
                OpKind::Put,
                Some(self.seal_payload(&note.id, &NotePayload::from_note(note))?),
            )
        };
        let op_id = self.fresh_op_id();
        let op = self.op(op_id, &note.id, kind, base_version, ciphertext);
        match self.submit_operation(op, None, false, None, true)? {
            ApplyOutcome::Conflict(_) => Ok(true),
            ApplyOutcome::Applied(_) => Err(EngineError::ConflictProbeApplied),
        }
    }

    fn apply_remote_view(
        &mut self,
        library: &mut Library,
        view: SavedView,
        version: i64,
        summary: &mut PullSummary,
    ) -> Result<(), EngineError> {
        let remote_fingerprint = view_fingerprint(&view)?;
        match library.view_get(&view.id) {
            Ok(local) => {
                let local_fingerprint = view_fingerprint(&local)?;
                if local == view {
                    summary.skipped.push(view.id.clone());
                    self.mark_known_config(&view.id, local_fingerprint, version);
                    self.config_conflicts.remove(&view.id);
                } else if self.is_known_config(&view.id, &local_fingerprint) {
                    library.view_apply_remote(view.clone())?;
                    summary.applied.push(view.id.clone());
                    self.mark_known_config(&view.id, remote_fingerprint, version);
                    self.config_conflicts.remove(&view.id);
                } else {
                    summary.config_conflicts.push(ConfigConflict {
                        object_id: view.id.clone(),
                        kind: "view".into(),
                        local_title: local.name,
                        remote_title: view.name,
                        remote_version: version,
                        local_available: true,
                        remote_available: true,
                    });
                }
            }
            Err(pkms_core::Error::NotFound) => {
                if library.config_tombstone_kind(&view.id)?.is_some()
                    && !self.is_known_config_deleted(&view.id)
                {
                    summary.config_conflicts.push(ConfigConflict {
                        object_id: view.id.clone(),
                        kind: "view".into(),
                        local_title: String::new(),
                        remote_title: view.name,
                        remote_version: version,
                        local_available: false,
                        remote_available: true,
                    });
                } else {
                    library.view_apply_remote(view.clone())?;
                    summary.applied.push(view.id.clone());
                    self.mark_known_config(&view.id, remote_fingerprint, version);
                    self.config_conflicts.remove(&view.id);
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn apply_remote_workspace_metadata(
        &mut self,
        library: &mut Library,
        metadata: WorkspaceMetadata,
        version: i64,
        summary: &mut PullSummary,
    ) -> Result<(), EngineError> {
        let remote_fingerprint = workspace_metadata_fingerprint(&metadata)?;
        let local = library.workspace_metadata()?;
        if local == metadata {
            summary.skipped.push(WORKSPACE_METADATA_OBJECT_ID.into());
            self.mark_known_config(WORKSPACE_METADATA_OBJECT_ID, remote_fingerprint, version);
            self.config_conflicts.remove(WORKSPACE_METADATA_OBJECT_ID);
        } else {
            let local_fingerprint = workspace_metadata_fingerprint(&local)?;
            if self.is_known_config(WORKSPACE_METADATA_OBJECT_ID, &local_fingerprint)
                || workspace_metadata_is_pristine(&local)
            {
                library.workspace_metadata_apply_remote(metadata.clone())?;
                summary.applied.push(WORKSPACE_METADATA_OBJECT_ID.into());
                self.mark_known_config(WORKSPACE_METADATA_OBJECT_ID, remote_fingerprint, version);
                self.config_conflicts.remove(WORKSPACE_METADATA_OBJECT_ID);
            } else {
                summary.config_conflicts.push(ConfigConflict {
                    object_id: WORKSPACE_METADATA_OBJECT_ID.into(),
                    kind: "workspace_metadata".into(),
                    local_title: "Workspace metadata".into(),
                    remote_title: "Workspace metadata".into(),
                    remote_version: version,
                    local_available: true,
                    remote_available: true,
                });
            }
        }
        Ok(())
    }

    fn apply_remote_canvas(
        &mut self,
        library: &mut Library,
        canvas: Canvas,
        version: i64,
        summary: &mut PullSummary,
    ) -> Result<(), EngineError> {
        let remote_fingerprint = canvas_fingerprint(&canvas)?;
        match library.canvas(&canvas.id) {
            Ok(local) => {
                let local_fingerprint = canvas_fingerprint(&local)?;
                if local == canvas {
                    summary.skipped.push(canvas.id.clone());
                    self.mark_known_config(&canvas.id, local_fingerprint, version);
                    self.config_conflicts.remove(&canvas.id);
                } else if self.is_known_config(&canvas.id, &local_fingerprint) {
                    library.canvas_apply_remote(canvas.clone())?;
                    summary.applied.push(canvas.id.clone());
                    self.mark_known_config(&canvas.id, remote_fingerprint, version);
                    self.config_conflicts.remove(&canvas.id);
                } else {
                    summary.config_conflicts.push(ConfigConflict {
                        object_id: canvas.id.clone(),
                        kind: "canvas".into(),
                        local_title: local.name,
                        remote_title: canvas.name,
                        remote_version: version,
                        local_available: true,
                        remote_available: true,
                    });
                }
            }
            Err(pkms_core::Error::NotFound) => {
                if library.config_tombstone_kind(&canvas.id)?.is_some()
                    && !self.is_known_config_deleted(&canvas.id)
                {
                    summary.config_conflicts.push(ConfigConflict {
                        object_id: canvas.id.clone(),
                        kind: "canvas".into(),
                        local_title: String::new(),
                        remote_title: canvas.name,
                        remote_version: version,
                        local_available: false,
                        remote_available: true,
                    });
                } else {
                    library.canvas_apply_remote(canvas.clone())?;
                    summary.applied.push(canvas.id.clone());
                    self.mark_known_config(&canvas.id, remote_fingerprint, version);
                    self.config_conflicts.remove(&canvas.id);
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
    fn apply_remote_live(
        &mut self,
        library: &mut Library,
        op: &pkms_sync::RemoteOp,
        payload: NotePayload,
        summary: &mut PullSummary,
    ) -> Result<(), EngineError> {
        match library.note(&op.object_id) {
            Ok(local) => {
                if payload.matches_note(&local) {
                    if op.kind == OpKind::Restore && local.trashed {
                        library.set_trashed(&op.object_id, false)?;
                        summary.restored.push(op.object_id.clone());
                    } else {
                        summary.skipped.push(op.object_id.clone());
                    }
                    let restored = library.note(&op.object_id)?;
                    self.mark_known(&op.object_id, &restored, op.resulting_version);
                } else if self.is_known(&op.object_id, &local) {
                    if local.trashed {
                        library.set_trashed(&op.object_id, false)?;
                        summary.restored.push(op.object_id.clone());
                    }
                    let current = library.note(&op.object_id)?;
                    let commit = library.save_document(
                        &Uuid::new_v4().to_string(),
                        SaveNote {
                            note_id: op.object_id.clone(),
                            base_revision_id: current.revision_id,
                            title: payload.title,
                            document_schema: payload.document_schema,
                            document: payload.document,
                        },
                    )?;
                    summary.applied.push(op.object_id.clone());
                    self.mark_known(&op.object_id, &commit.note, op.resulting_version);
                } else {
                    summary.conflicts.push(conflict_from_payload(
                        op,
                        &local,
                        Some(payload),
                        op.resulting_version,
                        false,
                    ));
                }
            }
            Err(pkms_core::Error::NotFound) => {
                let commit = library.create_document_with_id(
                    &Uuid::new_v4().to_string(),
                    &op.object_id,
                    CreateNote {
                        title: payload.title,
                        document_schema: payload.document_schema,
                        document: payload.document,
                        collection_id: None,
                        base_collection_row_version: None,
                    },
                )?;
                summary.applied.push(op.object_id.clone());
                self.mark_known(&op.object_id, &commit.note, op.resulting_version);
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn apply_remote_delete(
        &mut self,
        library: &mut Library,
        op: &pkms_sync::RemoteOp,
        summary: &mut PullSummary,
    ) -> Result<(), EngineError> {
        if let Ok(local) = library.view_get(&op.object_id) {
            let fingerprint = view_fingerprint(&local)?;
            if self.is_known_config(&op.object_id, &fingerprint) {
                library.view_remove_remote(&op.object_id)?;
                summary.applied.push(op.object_id.clone());
                self.known
                    .insert(op.object_id.clone(), deleted_config_state());
                self.config_conflicts.remove(&op.object_id);
                self.versions
                    .insert(op.object_id.clone(), op.resulting_version);
            } else {
                summary.config_conflicts.push(ConfigConflict {
                    object_id: op.object_id.clone(),
                    kind: "view".into(),
                    local_title: local.name,
                    remote_title: String::new(),
                    remote_version: op.resulting_version,
                    local_available: true,
                    remote_available: false,
                });
            }
            return Ok(());
        }
        if let Ok(local) = library.canvas(&op.object_id) {
            let fingerprint = canvas_fingerprint(&local)?;
            if self.is_known_config(&op.object_id, &fingerprint) {
                library.canvas_remove_remote(&op.object_id)?;
                summary.applied.push(op.object_id.clone());
                self.known
                    .insert(op.object_id.clone(), deleted_config_state());
                self.config_conflicts.remove(&op.object_id);
                self.versions
                    .insert(op.object_id.clone(), op.resulting_version);
            } else {
                summary.config_conflicts.push(ConfigConflict {
                    object_id: op.object_id.clone(),
                    kind: "canvas".into(),
                    local_title: local.name,
                    remote_title: String::new(),
                    remote_version: op.resulting_version,
                    local_available: true,
                    remote_available: false,
                });
            }
            return Ok(());
        }
        if library.config_tombstone_kind(&op.object_id)?.is_some() {
            self.known
                .insert(op.object_id.clone(), deleted_config_state());
            self.versions
                .insert(op.object_id.clone(), op.resulting_version);
            summary.skipped.push(op.object_id.clone());
            return Ok(());
        }
        match library.note(&op.object_id) {
            Ok(local) => {
                if local.trashed {
                    summary.skipped.push(op.object_id.clone());
                    self.mark_known(&op.object_id, &local, op.resulting_version);
                } else if self.is_known(&op.object_id, &local) {
                    library.set_trashed(&op.object_id, true)?;
                    summary.trashed.push(op.object_id.clone());
                    let updated = library.note(&op.object_id)?;
                    self.mark_known(&op.object_id, &updated, op.resulting_version);
                } else {
                    summary.conflicts.push(PullConflict {
                        object_id: op.object_id.clone(),
                        local_title: local.title.clone(),
                        local_body: local.document.plain_text(),
                        remote_title: String::new(),
                        remote_body: String::new(),
                        remote_version: op.resulting_version,
                        remote_tombstone: true,
                    });
                }
            }
            Err(pkms_core::Error::NotFound) => {
                summary.skipped.push(op.object_id.clone());
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn apply_remote_force(
        &mut self,
        library: &mut Library,
        object_id: &str,
    ) -> Result<(), EngineError> {
        tag_sync::protect_non_tag_wire(library, object_id)?;
        let objects = self.client.snapshot(&self.workspace_id)?;
        let object = objects
            .iter()
            .find(|object| object.object_id == object_id)
            .ok_or(EngineError::RemoteMissing)?;
        self.protect_non_tag_remote_object(object)?;
        if object.tombstone {
            if library.note(object_id).is_ok() {
                library.set_trashed(object_id, true)?;
            }
        } else {
            let ciphertext = object
                .ciphertext
                .as_deref()
                .ok_or(EngineError::MissingCiphertext)?;
            let payload = self.open_note_payload(object_id, object.key_epoch, ciphertext)?;
            match library.note(object_id) {
                Ok(local) => {
                    if local.trashed {
                        library.set_trashed(object_id, false)?;
                    }
                    let current = library.note(object_id)?;
                    library.save_document(
                        &Uuid::new_v4().to_string(),
                        SaveNote {
                            note_id: object_id.into(),
                            base_revision_id: current.revision_id,
                            title: payload.title,
                            document_schema: payload.document_schema,
                            document: payload.document,
                        },
                    )?;
                }
                Err(pkms_core::Error::NotFound) => {
                    library.create_document_with_id(
                        &Uuid::new_v4().to_string(),
                        object_id,
                        CreateNote {
                            title: payload.title,
                            document_schema: payload.document_schema,
                            document: payload.document,
                            collection_id: None,
                            base_collection_row_version: None,
                        },
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        if let Ok(note) = library.note(object_id) {
            self.mark_known(object_id, &note, object.version);
        }
        Ok(())
    }

    fn seal_payload(
        &self,
        object_id: &str,
        payload: &impl Serialize,
    ) -> Result<Vec<u8>, EngineError> {
        let plaintext = serde_json::to_vec(payload)?;
        let key = self
            .workspace_keys
            .get(&self.epoch)
            .ok_or(EngineError::MissingWorkspaceKey(self.epoch))?;
        let sealed = key.encrypt(
            &plaintext,
            &object_aad(&self.workspace_id, object_id, self.epoch),
        )?;
        Ok(sealed.encode())
    }

    fn open_any_payload(
        &self,
        object_id: &str,
        epoch: i64,
        ciphertext: &[u8],
    ) -> Result<Payload, EngineError> {
        let sealed = SealedData::decode(ciphertext)?;
        let key = self
            .workspace_keys
            .get(&epoch)
            .ok_or(EngineError::MissingWorkspaceKey(epoch))?;
        let plaintext = key.decrypt(&sealed, &object_aad(&self.workspace_id, object_id, epoch))?;
        tag_sync::decode_plaintext(object_id, &plaintext)
    }

    fn open_note_payload(
        &self,
        object_id: &str,
        epoch: i64,
        ciphertext: &[u8],
    ) -> Result<NotePayload, EngineError> {
        match self.open_any_payload(object_id, epoch, ciphertext)? {
            Payload::Note(note) => Ok(note),
            _ => Err(EngineError::MissingCiphertext),
        }
    }

    fn op(
        &self,
        op_id: String,
        object_id: &str,
        kind: OpKind,
        base_version: i64,
        ciphertext: Option<Vec<u8>>,
    ) -> SyncOp {
        SyncOp {
            op_id,
            workspace_id: self.workspace_id.clone(),
            object_id: object_id.into(),
            kind,
            base_version,
            key_epoch: self.epoch,
            ciphertext,
            client_time: Some(Utc::now().to_rfc3339()),
        }
    }

    fn is_known(&self, object_id: &str, note: &Note) -> bool {
        self.known.get(object_id).is_some_and(|known| {
            known.revision_id == note.revision_id && known.trashed == note.trashed
        })
    }

    fn mark_known(&mut self, object_id: &str, note: &Note, version: i64) {
        self.known.insert(
            object_id.to_owned(),
            KnownState {
                revision_id: note.revision_id.clone(),
                trashed: note.trashed,
            },
        );
        self.versions.insert(object_id.to_owned(), version);
    }

    fn is_known_config(&self, object_id: &str, fingerprint: &str) -> bool {
        self.known
            .get(object_id)
            .is_some_and(|known| known.revision_id == fingerprint && !known.trashed)
    }

    fn is_known_config_deleted(&self, object_id: &str) -> bool {
        self.known
            .get(object_id)
            .is_some_and(|known| known.trashed && known.revision_id == CONFIG_DELETED_FINGERPRINT)
    }

    fn mark_known_config(&mut self, object_id: &str, fingerprint: String, version: i64) {
        self.known.insert(
            object_id.to_owned(),
            KnownState {
                revision_id: fingerprint,
                trashed: false,
            },
        );
        self.versions.insert(object_id.to_owned(), version);
    }
    pub fn synced_version(&self, object_id: &str) -> Option<i64> {
        self.versions.get(object_id).copied()
    }
}

fn config_payload_matches(payload: &Payload, expected: &ConfigPayload) -> bool {
    match (payload, expected) {
        (Payload::View(local), ConfigPayload::View { view }) => local == view,
        (Payload::Canvas(local), ConfigPayload::Canvas { canvas }) => local == canvas,
        (Payload::WorkspaceMetadata(local), ConfigPayload::WorkspaceMetadata { metadata }) => {
            local == metadata
        }
        _ => false,
    }
}

fn view_fingerprint(view: &SavedView) -> Result<String, EngineError> {
    Ok(pkms_sync::sha256_hex(&serde_json::to_vec(view)?))
}

fn workspace_metadata_is_pristine(metadata: &WorkspaceMetadata) -> bool {
    const EMPTY_REGISTRY: &str = r#"{"schema":1,"workspaces":{}}"#;
    metadata.row_version == 1
        && metadata.updated_at == "1970-01-01T00:00:00Z"
        && metadata.object_type_registry == EMPTY_REGISTRY
        && metadata.note_template_registry == EMPTY_REGISTRY
}
fn workspace_metadata_fingerprint(metadata: &WorkspaceMetadata) -> Result<String, EngineError> {
    Ok(pkms_sync::sha256_hex(&serde_json::to_vec(metadata)?))
}
fn canvas_fingerprint(canvas: &Canvas) -> Result<String, EngineError> {
    Ok(pkms_sync::sha256_hex(&serde_json::to_vec(canvas)?))
}
fn conflict_from_payload(
    op: &pkms_sync::RemoteOp,
    local: &Note,
    payload: Option<NotePayload>,
    remote_version: i64,
    remote_tombstone: bool,
) -> PullConflict {
    PullConflict {
        object_id: op.object_id.clone(),
        local_title: local.title.clone(),
        local_body: local.document.plain_text(),
        remote_title: payload
            .as_ref()
            .map(|p| p.title.clone())
            .unwrap_or_default(),
        remote_body: payload
            .as_ref()
            .map(|p| p.document.plain_text())
            .unwrap_or_default(),
        remote_version,
        remote_tombstone,
    }
}

fn push_outcome(outcome: ApplyOutcome) -> PushOutcome {
    match outcome {
        ApplyOutcome::Applied(applied) => PushOutcome::Applied {
            version: applied.version,
        },
        ApplyOutcome::Conflict(conflict) => PushOutcome::Conflict {
            remote_version: conflict.current_version,
            remote_tombstone: conflict.current_tombstone,
        },
    }
}

fn optional_local<T>(result: pkms_core::Result<T>) -> Result<Option<T>, EngineError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(pkms_core::Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn note_state(note: &Note) -> KnownState {
    KnownState {
        revision_id: note.revision_id.clone(),
        trashed: note.trashed,
    }
}

fn deleted_config_state() -> KnownState {
    KnownState {
        revision_id: CONFIG_DELETED_FINGERPRINT.into(),
        trashed: true,
    }
}
