//! Tag transport adapter. Core owns accepted versions, comparison baselines and
//! deferred knowledge; the engine owns encrypted requests and their receipts.
use super::{
    ConfigPayload, EngineError, KnownState, NotePayload, Payload, PendingOperation, SyncEngine,
    WORKSPACE_METADATA_OBJECT_ID,
};
use pkms_core::{
    Library, TagSyncApplyResult, TagSyncApplyStatus, TagSyncKind, TagSyncPageRequest,
    TagSyncPayload, TagSyncPendingReason, TagSyncSnapshot, TagSyncWireOwner, tag_sync_object_id,
};
use pkms_sync::{ApplyOutcome, ConflictResolution, ObjectState, OpKind, RemoteOp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

fn invalid(message: &'static str) -> EngineError {
    pkms_core::Error::InvalidInput(message).into()
}

/// An explicit discriminator never falls back to the legacy note shape. This
/// parser is also used before history-version skips and snapshot seeding.
pub(crate) fn decode_plaintext(object_id: &str, plaintext: &[u8]) -> Result<Payload, EngineError> {
    let value: serde_json::Value = serde_json::from_slice(plaintext)?;
    if let Some(discriminator) = value.get("sync_kind") {
        return match discriminator.as_str() {
            Some("tag" | "tag_assignments") => {
                let payload: TagSyncPayload = serde_json::from_value(value)?;
                payload.validate()?;
                if payload.sync_object_id()? != object_id {
                    return Err(invalid(
                        "tag payload does not match the encrypted object identity",
                    ));
                }
                Ok(Payload::Tag(payload))
            }
            Some("view" | "canvas" | "workspace_metadata") => {
                match serde_json::from_value::<ConfigPayload>(value)? {
                    ConfigPayload::View { view } if view.id == object_id => Ok(Payload::View(view)),
                    ConfigPayload::Canvas { canvas } if canvas.id == object_id => {
                        Ok(Payload::Canvas(canvas))
                    }
                    ConfigPayload::WorkspaceMetadata { metadata }
                        if object_id == WORKSPACE_METADATA_OBJECT_ID =>
                    {
                        Ok(Payload::WorkspaceMetadata(metadata))
                    }
                    _ => Err(invalid(
                        "configuration payload identity does not match its envelope",
                    )),
                }
            }
            _ => Err(invalid("unsupported sync payload discriminator")),
        };
    }
    let note: NotePayload = serde_json::from_value(value)?;
    note.document.validate(note.document_schema)?;
    Ok(Payload::Note(note))
}

/// Immutable thin ownership keys for one pass. Refresh for a new pass; register
/// every newly seen tag payload before considering later operations in that pass.
#[derive(Debug, Clone)]
pub struct TagWireOwners {
    workspace_id: String,
    owners: HashMap<String, (TagSyncKind, String)>,
}

impl TagWireOwners {
    pub fn from_library(library: &Library) -> Result<Self, EngineError> {
        let mut result = Self {
            workspace_id: library.info()?.workspace_id,
            owners: HashMap::new(),
        };
        for owner in library.tag_sync_wire_owners()? {
            result.insert(owner)?;
        }
        Ok(result)
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn contains(&self, object_id: &str) -> bool {
        self.owners.contains_key(object_id)
    }

    fn insert(&mut self, owner: TagSyncWireOwner) -> Result<(), EngineError> {
        if let Some((kind, id)) = self.owners.get(&owner.sync_object_id) {
            if *kind != owner.kind || id != &owner.entity_id {
                return Err(invalid("tag sync wire has multiple owners"));
            }
        } else {
            self.owners
                .insert(owner.sync_object_id, (owner.kind, owner.entity_id));
        }
        Ok(())
    }

    pub fn register(&mut self, payload: &TagSyncPayload) -> Result<(), EngineError> {
        payload.validate()?;
        self.insert(TagSyncWireOwner {
            sync_object_id: payload.sync_object_id()?,
            kind: payload.kind(),
            entity_id: payload.entity_id().into(),
        })
    }

    pub fn validate_non_tag(&self, object_id: &str) -> Result<(), EngineError> {
        if self.owners.contains_key(object_id) {
            return Err(invalid("sync wire identity is reserved for tag metadata"));
        }
        Ok(())
    }

    pub fn preflight_non_tags<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), EngineError> {
        for id in ids {
            self.validate_non_tag(id)?;
        }
        Ok(())
    }

    pub(crate) fn validate_payload(
        &mut self,
        object_id: &str,
        payload: &Payload,
    ) -> Result<(), EngineError> {
        match payload {
            Payload::Tag(tag) => {
                if tag.sync_object_id()? != object_id || object_id == WORKSPACE_METADATA_OBJECT_ID {
                    return Err(invalid("tag sync payload does not match its wire identity"));
                }
                self.register(tag)
            }
            _ => self.validate_non_tag(object_id),
        }
    }
}

/// Generated tag wires are exact, canonical UUIDv8 strings. Ordinary UUIDv4
/// note/config IDs cannot collide and do not need a full owner enumeration.
pub(crate) fn possible_tag_wire(object_id: &str) -> bool {
    Uuid::parse_str(object_id)
        .is_ok_and(|id| id.get_version_num() == 8 && id.to_string() == object_id)
}

pub(crate) fn protect_non_tag_wire(library: &Library, object_id: &str) -> Result<(), EngineError> {
    if possible_tag_wire(object_id) {
        TagWireOwners::from_library(library)?.validate_non_tag(object_id)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TagPushOutcome {
    Published { version: i64 },
    Unchanged { version: i64 },
    Received { version: i64 },
    Pending,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagPushResult {
    pub outcome: TagPushOutcome,
    /// A skipped pending/unavailable record, or a concurrent later edit, is not
    /// a completed synchronization merely because no transport error occurred.
    pub complete: bool,
    pub snapshot: TagSyncSnapshot,
    pub changed_objects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagApplySummary {
    pub kind: TagSyncKind,
    pub entity_id: String,
    pub sync_object_id: String,
    pub status: TagSyncApplyStatus,
    pub remote_version: i64,
    pub pending_reason: Option<TagSyncPendingReason>,
    pub changed_objects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRetryPage {
    pub items: Vec<TagApplySummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagSyncDecision {
    pub kind: TagSyncKind,
    pub entity_id: String,
    pub remote_version: i64,
    pub local_fingerprint: Option<String>,
    pub local_impact_token: Option<String>,
}

impl TagSyncDecision {
    fn matches(&self, current: &TagSyncSnapshot) -> bool {
        self.kind == current.kind
            && self.entity_id == current.entity_id
            && self.remote_version == current.remote_version
            && self.local_fingerprint == current.local_fingerprint
            && self.local_impact_token == current.local_impact_token
    }
}

fn push_result(
    outcome: TagPushOutcome,
    snapshot: TagSyncSnapshot,
    changed_objects: Vec<String>,
) -> TagPushResult {
    let complete = snapshot.pending.is_none() && !snapshot.dirty && snapshot.payload.is_some();
    TagPushResult {
        outcome,
        snapshot,
        complete,
        changed_objects,
    }
}

fn summary(result: &TagSyncApplyResult) -> TagApplySummary {
    TagApplySummary {
        kind: result.snapshot.kind,
        entity_id: result.snapshot.entity_id.clone(),
        sync_object_id: result.snapshot.sync_object_id.clone(),
        status: result.status,
        remote_version: result.snapshot.remote_version,
        pending_reason: result
            .snapshot
            .pending
            .as_ref()
            .map(|pending| pending.reason),
        changed_objects: result.changed_objects.clone(),
    }
}

fn available<T>(result: pkms_core::Result<T>) -> Result<Option<T>, EngineError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(pkms_core::Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl SyncEngine {
    /// Public preflight for callers that change local trash state before sending
    /// an ordinary note/configuration operation. Never mutate first and discover
    /// that the encrypted wire belongs to tag metadata afterward.
    pub fn preflight_non_tag_object(
        &self,
        library: &Library,
        object_id: &str,
    ) -> Result<(), EngineError> {
        self.require_tag_library(library)?;
        protect_non_tag_wire(library, object_id)?;
        self.protect_non_tag_server_wire(object_id)
    }

    /// A cheap local hint for periodic work; clean Core state may still have an
    /// immutable encrypted request whose acknowledgement must be recovered.
    pub fn has_pending_tag_requests(&self) -> Result<bool, EngineError> {
        for pending in self.pending.values() {
            if possible_tag_wire(&pending.op.object_id)
                && self.pending_tag_payload(pending)?.is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Thin identities let automatic synchronization recover only the affected
    /// requests instead of polling every clean tag's remote snapshot.
    pub fn pending_tag_records(&self) -> Result<Vec<(TagSyncKind, String)>, EngineError> {
        let mut records = Vec::new();
        for pending in self.pending.values() {
            if possible_tag_wire(&pending.op.object_id)
                && let Some(payload) = self.pending_tag_payload(pending)?
            {
                records.push((payload.kind(), payload.entity_id().to_owned()));
            }
        }
        records.sort_by(|(left_kind, left_id), (right_kind, right_id)| {
            left_kind
                .as_str()
                .cmp(right_kind.as_str())
                .then_with(|| left_id.cmp(right_id))
        });
        records.dedup();
        Ok(records)
    }

    /// Recover only tag receipts; unrelated note changes stay on the pull path
    /// whose caller refreshes open drafts as well as metadata.
    pub fn recover_pending_tag_receipts(
        &mut self,
        library: &mut Library,
    ) -> Result<(), EngineError> {
        self.require_tag_library(library)?;
        let mut owners = TagWireOwners::from_library(library)?;
        for (kind, entity_id) in self.pending_tag_records()? {
            let wire = tag_sync_object_id(kind, &entity_id)?;
            match self.resume_pending_with_core(library, &wire, &mut owners) {
                Err(EngineError::Client(pkms_sync_client::ClientError::Api {
                    status: 403,
                    ..
                })) => {
                    // The original request remains pending. Permission is not
                    // proof of non-commit; callers must check pending before
                    // reporting completion.
                }
                result => {
                    result?;
                }
            }
        }
        Ok(())
    }

    /// An explicit durable boundary covers both snapshot passes. Neither a
    /// partial generic seed nor Core tag acknowledgement alone completes it.
    pub fn initialize_from_snapshot(&mut self, library: &mut Library) -> Result<(), EngineError> {
        self.require_tag_library(library)?;
        self.initial_seed_completed = false;
        self.persist()?;
        self.seed_from_snapshot(library)?;
        self.seed_tag_snapshot(library)?;
        self.initial_seed_completed = true;
        if let Err(error) = self.persist() {
            self.initial_seed_completed = false;
            return Err(error);
        }
        Ok(())
    }

    fn require_tag_library(&self, library: &Library) -> Result<(), EngineError> {
        if library.info()?.workspace_id != self.workspace_id {
            return Err(invalid(
                "tag sync library does not match the bound workspace",
            ));
        }
        Ok(())
    }

    pub(crate) fn decode_remote_for_tags(
        &self,
        op: &RemoteOp,
        owners: &mut TagWireOwners,
    ) -> Result<Option<Payload>, EngineError> {
        if op.workspace_id != self.workspace_id || owners.workspace_id() != self.workspace_id {
            return Err(invalid(
                "remote operation workspace does not match tag owner scope",
            ));
        }
        match op.kind {
            OpKind::Put | OpKind::Restore => {
                let bytes = op
                    .ciphertext
                    .as_deref()
                    .ok_or(EngineError::MissingCiphertext)?;
                let payload = self.open_any_payload(&op.object_id, op.key_epoch, bytes)?;
                owners.validate_payload(&op.object_id, &payload)?;
                if matches!(&payload, Payload::Tag(_)) && op.kind != OpKind::Put {
                    return Err(invalid(
                        "tag metadata uses retained Put records, not raw restore operations",
                    ));
                }
                Ok(Some(payload))
            }
            OpKind::Delete | OpKind::Purge => {
                owners.validate_non_tag(&op.object_id)?;
                Ok(None)
            }
        }
    }

    pub(crate) fn inspect_snapshot_for_tags(
        &self,
        object: &ObjectState,
        owners: &mut TagWireOwners,
    ) -> Result<Option<Payload>, EngineError> {
        if object.workspace_id != self.workspace_id || owners.workspace_id() != self.workspace_id {
            return Err(invalid("snapshot workspace does not match tag owner scope"));
        }
        if object.tombstone || object.purged {
            owners.validate_non_tag(&object.object_id)?;
            if let Some(bytes) = object.ciphertext.as_deref()
                && matches!(
                    self.open_any_payload(&object.object_id, object.key_epoch, bytes)?,
                    Payload::Tag(_)
                )
            {
                return Err(invalid(
                    "tag dictionary deletions require retained configuration payloads",
                ));
            }
            return Ok(None);
        }
        let bytes = object
            .ciphertext
            .as_deref()
            .ok_or(EngineError::MissingCiphertext)?;
        let payload = self.open_any_payload(&object.object_id, object.key_epoch, bytes)?;
        owners.validate_payload(&object.object_id, &payload)?;
        Ok(Some(payload))
    }

    /// No-op receipt wrappers must never consume a tag request without its Core ack.
    pub(crate) fn reject_tag_pending(&self, object_id: &str) -> Result<(), EngineError> {
        if let Some(pending) = self.pending.get(object_id)
            && self.pending_tag_payload(pending)?.is_some()
        {
            return Err(invalid(
                "tag request recovery requires its Core acknowledgement",
            ));
        }
        Ok(())
    }

    /// Ordinary v4 IDs need no extra snapshot request. For a possible reserved
    /// v8 ID, inspect a remote owner even when this Core instance has not seen it.
    pub(crate) fn protect_non_tag_server_wire(&self, object_id: &str) -> Result<(), EngineError> {
        if !possible_tag_wire(object_id) {
            return Ok(());
        }
        if let Some(object) = self
            .client
            .snapshot(&self.workspace_id)?
            .into_iter()
            .find(|o| o.object_id == object_id)
        {
            self.protect_non_tag_remote_object(&object)?;
        }
        Ok(())
    }

    fn validate_tag_slot(
        &self,
        library: &Library,
        payload: &TagSyncPayload,
    ) -> Result<(), EngineError> {
        let wire = payload.sync_object_id()?;
        if wire == WORKSPACE_METADATA_OBJECT_ID
            || available(library.note(&wire))?.is_some()
            || available(library.view_get(&wire))?.is_some()
            || available(library.canvas(&wire))?.is_some()
        {
            return Err(invalid(
                "tag sync wire identity is already used by another local entity",
            ));
        }
        Ok(())
    }

    fn tag_payload_from_object(&self, object: &ObjectState) -> Result<TagSyncPayload, EngineError> {
        if object.workspace_id != self.workspace_id || object.tombstone || object.purged {
            return Err(invalid(
                "tag metadata requires its versioned configuration payload",
            ));
        }
        let bytes = object
            .ciphertext
            .as_deref()
            .ok_or(EngineError::MissingCiphertext)?;
        match self.open_any_payload(&object.object_id, object.key_epoch, bytes)? {
            Payload::Tag(payload) if payload.sync_object_id()? == object.object_id => Ok(payload),
            _ => Err(invalid("remote wire belongs to a different payload kind")),
        }
    }

    fn remote_tag(&self, wire: &str) -> Result<Option<(TagSyncPayload, i64)>, EngineError> {
        let objects = self.client.snapshot(&self.workspace_id)?;
        objects
            .into_iter()
            .find(|object| object.object_id == wire)
            .map(|object| Ok((self.tag_payload_from_object(&object)?, object.version)))
            .transpose()
    }

    fn pending_tag_payload(
        &self,
        pending: &PendingOperation,
    ) -> Result<Option<TagSyncPayload>, EngineError> {
        let Some(bytes) = pending.op.ciphertext.as_deref() else {
            return Ok(None);
        };
        match self.open_any_payload(&pending.op.object_id, pending.op.key_epoch, bytes)? {
            Payload::Tag(payload) => {
                if pending.op.kind != OpKind::Put
                    || payload.sync_object_id()? != pending.op.object_id
                {
                    return Err(invalid("invalid retained tag operation"));
                }
                Ok(Some(payload))
            }
            _ => Ok(None),
        }
    }

    /// Parent pull must use this dispatcher for pending flush. A Core receipt is
    /// committed before the shared helper clears its exact encrypted operation.
    pub(crate) fn resume_pending_with_core(
        &mut self,
        library: &mut Library,
        object_id: &str,
        owners: &mut TagWireOwners,
    ) -> Result<Option<ApplyOutcome>, EngineError> {
        self.require_tag_library(library)?;
        let Some(pending) = self.pending.get(object_id).cloned() else {
            return Ok(None);
        };
        let Some(payload) = self.pending_tag_payload(&pending)? else {
            owners.validate_non_tag(object_id)?;
            return self.resume_pending_operation(object_id);
        };
        owners.register(&payload)?;
        self.validate_tag_slot(library, &payload)?;
        self.resume_pending_operation_with(object_id, move |actual, version| {
            if actual.op != pending.op {
                return Err(EngineError::PendingRequestMismatch);
            }
            library.tag_sync_acknowledge(payload, version)?;
            Ok(())
        })
    }

    /// This must run before the parent's version skip. Even a read-only server
    /// grant may confirm our earlier write from its identical immutable log entry.
    pub(crate) fn acknowledge_pending_with_core(
        &mut self,
        library: &mut Library,
        op: &RemoteOp,
    ) -> Result<(), EngineError> {
        if let Some(pending) = self.pending_receipt_from_log(op)?
            && let Some(payload) = self.pending_tag_payload(&pending)?
        {
            self.require_tag_library(library)?;
            library.tag_sync_acknowledge(payload, op.resulting_version)?;
        }
        self.acknowledge_pending_from_log(op)
    }

    fn record_tag_state(&mut self, snapshot: &TagSyncSnapshot) {
        self.versions
            .entry(snapshot.sync_object_id.clone())
            .and_modify(|v| *v = (*v).max(snapshot.remote_version))
            .or_insert(snapshot.remote_version);
        if let Some(base) = &snapshot.base_fingerprint {
            self.known.insert(
                snapshot.sync_object_id.clone(),
                KnownState {
                    revision_id: base.clone(),
                    trashed: false,
                },
            );
        }
    }

    /// Parent pull calls this for tag payloads even when its disposable engine
    /// version map is already ahead. Core's durable seen/base rules are authority.
    pub(crate) fn apply_remote_tag(
        &mut self,
        library: &mut Library,
        op: &RemoteOp,
        payload: TagSyncPayload,
    ) -> Result<TagApplySummary, EngineError> {
        self.require_tag_library(library)?;
        if op.workspace_id != self.workspace_id || op.kind != OpKind::Put {
            return Err(invalid("invalid incoming tag operation scope or kind"));
        }
        let result = library.tag_sync_receive(&op.object_id, payload, op.resulting_version)?;
        self.record_tag_state(&result.snapshot);
        Ok(summary(&result))
    }

    /// Snapshot readers for ordinary objects must reject a remote tag payload,
    /// even if this library has not yet received that tag's Core ownership row.
    pub(crate) fn protect_non_tag_remote_object(
        &self,
        object: &ObjectState,
    ) -> Result<(), EngineError> {
        if let Some(bytes) = object.ciphertext.as_deref()
            && matches!(
                self.open_any_payload(&object.object_id, object.key_epoch, bytes)?,
                Payload::Tag(_)
            )
        {
            return Err(invalid("remote wire identity is reserved for tag metadata"));
        }
        Ok(())
    }

    fn current_tag_result(
        &self,
        library: &Library,
        kind: TagSyncKind,
        entity_id: &str,
        outcome: TagPushOutcome,
        changed: Vec<String>,
    ) -> Result<TagPushResult, EngineError> {
        Ok(push_result(
            outcome,
            library.tag_sync_snapshot(kind, entity_id)?,
            changed,
        ))
    }

    fn accept_tag_head(
        &mut self,
        library: &mut Library,
        kind: TagSyncKind,
        entity_id: &str,
    ) -> Result<TagSyncApplyResult, EngineError> {
        let wire = tag_sync_object_id(kind, entity_id)?;
        let (remote, version) = self.remote_tag(&wire)?.ok_or(EngineError::RemoteMissing)?;
        if remote.kind() != kind || remote.entity_id() != entity_id {
            return Err(invalid(
                "remote tag owner does not match the requested entity",
            ));
        }
        let before = library.tag_sync_snapshot(kind, entity_id)?;
        if version < before.base_version {
            return Err(invalid(
                "remote tag version is older than the retained baseline",
            ));
        }
        let result = library.tag_sync_receive(&wire, remote, version)?;
        self.record_tag_state(&result.snapshot);
        self.persist()?;
        Ok(result)
    }

    fn result_after_tag_conflict(
        &mut self,
        library: &mut Library,
        kind: TagSyncKind,
        entity_id: &str,
    ) -> Result<TagPushResult, EngineError> {
        let result = self.accept_tag_head(library, kind, entity_id)?;
        let outcome = if result.snapshot.pending.is_some() {
            TagPushOutcome::Pending
        } else if result.status == TagSyncApplyStatus::Applied {
            TagPushOutcome::Received {
                version: result.snapshot.remote_version,
            }
        } else {
            TagPushOutcome::Unchanged {
                version: result.snapshot.remote_version,
            }
        };
        Ok(push_result(
            outcome,
            result.snapshot,
            result.changed_objects,
        ))
    }

    fn publish_tag_payload(
        &mut self,
        library: &mut Library,
        payload: TagSyncPayload,
        base_version: i64,
        conflict_id: Option<String>,
    ) -> Result<TagPushResult, EngineError> {
        self.validate_tag_slot(library, &payload)?;
        let wire = payload.sync_object_id()?;
        let fingerprint = payload.fingerprint()?;
        let ciphertext = self.seal_payload(&wire, &payload)?;
        let op = self.op(
            self.fresh_op_id(),
            &wire,
            OpKind::Put,
            base_version,
            Some(ciphertext),
        );
        let original = op.clone();
        let sent = payload.clone();
        let kind = payload.kind();
        let entity_id = payload.entity_id().to_owned();
        let outcome = self.submit_operation_with(
            op,
            Some(KnownState {
                revision_id: fingerprint,
                trashed: false,
            }),
            false,
            conflict_id,
            false,
            |actual, version| {
                if actual.op != original {
                    return Err(EngineError::PendingRequestMismatch);
                }
                library.tag_sync_acknowledge(sent, version)?;
                Ok(())
            },
        )?;
        match outcome {
            ApplyOutcome::Applied(applied) => self.current_tag_result(
                library,
                kind,
                &entity_id,
                TagPushOutcome::Published {
                    version: applied.version,
                },
                Vec::new(),
            ),
            ApplyOutcome::Conflict(_) => self.result_after_tag_conflict(library, kind, &entity_id),
        }
    }

    pub fn push_tag_record(
        &mut self,
        library: &mut Library,
        kind: TagSyncKind,
        entity_id: &str,
    ) -> Result<TagPushResult, EngineError> {
        self.require_tag_library(library)?;
        let wire = tag_sync_object_id(kind, entity_id)?;
        let mut owners = TagWireOwners::from_library(library)?;
        let recovered = self.resume_pending_with_core(library, &wire, &mut owners)?;
        if matches!(recovered, Some(ApplyOutcome::Conflict(_))) {
            return self.result_after_tag_conflict(library, kind, entity_id);
        }
        let mut current = library.tag_sync_snapshot(kind, entity_id)?;
        if current.pending.is_some() {
            return Ok(push_result(TagPushOutcome::Pending, current, Vec::new()));
        }
        let Some(payload) = current.payload.clone() else {
            return Ok(push_result(
                TagPushOutcome::Unavailable,
                current,
                Vec::new(),
            ));
        };
        owners.register(&payload)?;
        self.validate_tag_slot(library, &payload)?;
        let mut changed = Vec::new();
        if let Some((remote, version)) = self.remote_tag(&wire)? {
            if remote.kind() != kind
                || remote.entity_id() != entity_id
                || version < current.base_version
            {
                return Err(invalid("remote tag owner or retained version changed"));
            }
            let received = library.tag_sync_receive(&wire, remote, version)?;
            self.record_tag_state(&received.snapshot);
            self.persist()?;
            changed = received.changed_objects;
            current = received.snapshot;
            if current.pending.is_some() {
                return Ok(push_result(TagPushOutcome::Pending, current, changed));
            }
            if !current.dirty {
                let outcome = if received.status == TagSyncApplyStatus::Applied {
                    TagPushOutcome::Received { version }
                } else if matches!(recovered, Some(ApplyOutcome::Applied(_))) {
                    TagPushOutcome::Published { version }
                } else {
                    TagPushOutcome::Unchanged { version }
                };
                return Ok(push_result(outcome, current, changed));
            }
        } else if current.base_version != 0 {
            return Err(EngineError::RemoteMissing);
        }
        let Some(payload) = current.payload else {
            return self.current_tag_result(
                library,
                kind,
                entity_id,
                TagPushOutcome::Unavailable,
                changed,
            );
        };
        let mut result = self.publish_tag_payload(library, payload, current.base_version, None)?;
        result.changed_objects.extend(changed);
        result.changed_objects.sort();
        result.changed_objects.dedup();
        Ok(result)
    }

    fn validate_tag_decision(
        &self,
        library: &Library,
        decision: &TagSyncDecision,
    ) -> Result<TagSyncSnapshot, EngineError> {
        if decision.remote_version < 1 {
            return Err(invalid("invalid tag comparison version"));
        }
        let current = library.tag_sync_snapshot(decision.kind, &decision.entity_id)?;
        if current.remote_version != decision.remote_version {
            return Err(EngineError::ConfigConflictChanged);
        }
        if !decision.matches(&current) {
            return Err(EngineError::ConfigPreviewChanged);
        }
        Ok(current)
    }

    fn tag_conflict_id(&self, wire: &str, version: i64) -> Result<Option<String>, EngineError> {
        Ok(self
            .scoped_conflicts(
                self.client
                    .pull(&self.workspace_id, self.cursor, 1)?
                    .conflicts,
            )
            .into_iter()
            .find(|conflict| {
                conflict.object_id == wire
                    && conflict.remote_version == version
                    && !conflict.remote_tombstone
            })
            .map(|conflict| conflict.conflict_id))
    }

    pub fn keep_local_tag_checked(
        &mut self,
        library: &mut Library,
        decision: TagSyncDecision,
    ) -> Result<TagPushResult, EngineError> {
        self.require_tag_library(library)?;
        let wire = tag_sync_object_id(decision.kind, &decision.entity_id)?;
        let mut owners = TagWireOwners::from_library(library)?;
        let recovering_same_decision = self
            .pending
            .get(&wire)
            .map(|pending| -> Result<bool, EngineError> {
                let Some(payload) = self.pending_tag_payload(pending)? else {
                    return Ok(false);
                };
                Ok(pending.may_have_committed
                    && pending.op.base_version == decision.remote_version
                    && Some(payload.fingerprint()?) == decision.local_fingerprint)
            })
            .transpose()?
            .unwrap_or(false);
        if recovering_same_decision {
            let recovered = self.resume_pending_with_core(library, &wire, &mut owners)?;
            // The exact earlier decision is now confirmed. Push a subsequently
            // saved local edit from that real baseline instead of reporting it done.
            if matches!(recovered, Some(ApplyOutcome::Applied(_))) {
                return self.push_tag_record(library, decision.kind, &decision.entity_id);
            }
            if matches!(recovered, Some(ApplyOutcome::Conflict(_))) {
                return self.result_after_tag_conflict(library, decision.kind, &decision.entity_id);
            }
        }
        self.validate_tag_decision(library, &decision)?;
        self.resume_pending_with_core(library, &wire, &mut owners)?;
        let current = self.validate_tag_decision(library, &decision)?;
        let Some(payload) = current.payload.clone() else {
            return Err(EngineError::LocalConfigMissing);
        };
        let pending = current
            .pending
            .as_ref()
            .ok_or(EngineError::ConfigConflictChanged)?;
        let (remote, version) = self.remote_tag(&wire)?.ok_or(EngineError::RemoteMissing)?;
        if version != decision.remote_version {
            return Err(EngineError::ConfigConflictChanged);
        }
        if remote != pending.payload {
            return Err(pkms_core::Error::IdempotencyMismatch.into());
        }
        let conflict_id = self.tag_conflict_id(&wire, version)?;
        self.publish_tag_payload(library, payload, version, conflict_id)
    }

    pub fn keep_remote_tag_checked(
        &mut self,
        library: &mut Library,
        decision: TagSyncDecision,
    ) -> Result<TagSyncApplyResult, EngineError> {
        self.require_tag_library(library)?;
        let initial = self.validate_tag_decision(library, &decision)?;
        let wire = initial.sync_object_id;
        let mut owners = TagWireOwners::from_library(library)?;
        self.resume_pending_with_core(library, &wire, &mut owners)?;
        let current = self.validate_tag_decision(library, &decision)?;
        let pending = current
            .pending
            .as_ref()
            .ok_or(EngineError::ConfigConflictChanged)?;
        let (remote, version) = self.remote_tag(&wire)?.ok_or(EngineError::RemoteMissing)?;
        if version != decision.remote_version {
            return Err(EngineError::ConfigConflictChanged);
        }
        if remote != pending.payload {
            return Err(pkms_core::Error::IdempotencyMismatch.into());
        }
        if let Some(conflict_id) = self.tag_conflict_id(&wire, version)? {
            self.client.resolve_conflict(
                &conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
            )?;
        }
        // This transaction repeats the final local fingerprint and deletion
        // impact check after all network waits; a stale preview cannot delete
        // a newly assigned object's tag.
        let result = library.tag_sync_keep_remote(
            decision.kind,
            &decision.entity_id,
            decision.remote_version,
            decision.local_fingerprint.as_deref(),
            decision.local_impact_token.as_deref(),
        )?;
        self.record_tag_state(&result.snapshot);
        self.persist()?;
        Ok(result)
    }

    pub fn retry_pending_tags(
        &mut self,
        library: &mut Library,
        limit: u32,
        after: Option<String>,
    ) -> Result<TagRetryPage, EngineError> {
        self.require_tag_library(library)?;
        let page = library.tag_sync_page(TagSyncPageRequest {
            limit,
            after,
            pending_only: true,
        })?;
        if page.items.is_empty() {
            return Ok(TagRetryPage {
                items: Vec::new(),
                next_cursor: page.next_cursor,
            });
        }
        let mut items = Vec::with_capacity(page.items.len());
        for snapshot in page.items {
            let result = library.tag_sync_retry(
                snapshot.kind,
                &snapshot.entity_id,
                snapshot.remote_version,
            )?;
            self.record_tag_state(&result.snapshot);
            items.push(summary(&result));
        }
        self.persist()?;
        Ok(TagRetryPage {
            items,
            next_cursor: page.next_cursor,
        })
    }

    /// An explicit refresh uses ordinary Core receive rules. A rejected checked
    /// decision itself never adopts a newly changed remote comparison.
    pub fn refresh_tag_record(
        &mut self,
        library: &mut Library,
        kind: TagSyncKind,
        entity_id: &str,
    ) -> Result<TagSyncApplyResult, EngineError> {
        self.require_tag_library(library)?;
        self.accept_tag_head(library, kind, entity_id)
    }

    /// Complements the legacy immutable generic seed method. Only exact local
    /// content may be acknowledged from the remote snapshot; pending and changed
    /// records remain Core-owned comparisons for the subsequent full pull.
    pub fn seed_tag_snapshot(&mut self, library: &mut Library) -> Result<Vec<String>, EngineError> {
        self.require_tag_library(library)?;
        let mut owners = TagWireOwners::from_library(library)?;
        let objects = self.client.snapshot(&self.workspace_id)?;
        let mut seeded = Vec::new();
        for object in objects {
            let Some(Payload::Tag(payload)) =
                self.inspect_snapshot_for_tags(&object, &mut owners)?
            else {
                continue;
            };
            if self.pending.contains_key(&object.object_id) {
                continue;
            }
            let Some(local) =
                available(library.tag_sync_snapshot(payload.kind(), payload.entity_id()))?
            else {
                continue;
            };
            if local.pending.is_some() || local.payload.as_ref() != Some(&payload) {
                continue;
            }
            if object.version < local.base_version {
                return Err(invalid(
                    "remote tag snapshot regressed below retained knowledge",
                ));
            }
            let acknowledged = library.tag_sync_acknowledge(payload, object.version)?;
            self.record_tag_state(&acknowledged);
            seeded.push(object.object_id);
        }
        self.persist()?;
        Ok(seeded)
    }
}
