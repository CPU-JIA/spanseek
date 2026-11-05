use pkms_core::*;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, OpKind, SyncAuthority, SyncOp, WorkspaceKey,
    encryption_public_key_hex, object_aad, owner_proof_message, pairing_proof_message,
    workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use pkms_sync_engine::{EngineError, SyncEngine, TagPushOutcome, TagSyncDecision};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tempfile::{Builder, TempDir};
use uuid::Uuid;

const OWNER_SEED: [u8; 32] = [31; 32];
const PEER_SEED: [u8; 32] = [47; 32];
const TEST_KEY: [u8; 32] = [93; 32];

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn temp() -> TempDir {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let runs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local-archive/test-runs");
        std::fs::create_dir_all(&runs).unwrap();
        let root = std::env::var_os("PKMS_TEST_RUN_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                runs.join(format!(
                    "tag-engine-{}-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S-%f"),
                    std::process::id()
                ))
            });
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        assert!(root.starts_with(runs.canonicalize().unwrap()));
        root
    });
    Builder::new().prefix("http-").tempdir_in(base).unwrap()
}

#[derive(Clone, Default)]
struct Fault {
    lose_receipt: Arc<AtomicBool>,
    submitted: Arc<Mutex<Vec<SyncOp>>>,
}

async fn receipt_fault(
    axum::extract::State(fault): axum::extract::State<Fault>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if request.uri().path() != "/v1/ops" {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024).await.unwrap();
    let data: pkms_sync::transport::ApplyOpBody = serde_json::from_slice(&bytes).unwrap();
    fault.submitted.lock().unwrap().push(data.op);
    let response = next
        .run(axum::extract::Request::from_parts(
            parts,
            axum::body::Body::from(bytes),
        ))
        .await;
    // The actual authority commits first; only the successful HTTP receipt is lost.
    if response.status().is_success() && fault.lose_receipt.swap(false, Ordering::SeqCst) {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    response
}

struct Server {
    base: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn start(authority: SyncAuthority, fault: Fault) -> Self {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
        let (stop, receive) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let socket = tokio::net::TcpListener::from_std(socket).unwrap();
                    let router = pkms_sync_server::router(Arc::new(Mutex::new(authority)))
                        .layer(axum::middleware::from_fn_with_state(fault, receipt_fault));
                    axum::serve(socket, router)
                        .with_graceful_shutdown(async {
                            let _ = receive.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            base: format!("http://{address}"),
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

struct Fixture {
    server: Server,
    workspace: String,
    owner: DeviceRecord,
    peer: DeviceRecord,
    fault: Fault,
}
impl Fixture {
    fn new(root: &Path) -> (Self, Library, Library) {
        let a_path = root.join("a");
        let b_path = root.join("b");
        let a = Library::open(&a_path).unwrap();
        let workspace = a.info().unwrap().workspace_id;
        drop(a);
        std::fs::create_dir_all(&b_path).unwrap();
        std::fs::copy(
            a_path.join("library.sqlite3"),
            b_path.join("library.sqlite3"),
        )
        .unwrap();
        let mut authority = SyncAuthority::open_in_memory().unwrap();
        let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
        let owner = authority
            .enroll_owner(
                &owner_key.public_key(),
                "tag owner",
                &owner_key.sign(&owner_proof_message(
                    &owner_key.public_key_hex(),
                    "tag owner",
                )),
            )
            .unwrap();
        let peer_key = DeviceKeyPair::from_seed(PEER_SEED).unwrap();
        let invite = authority
            .create_pairing_invite(&owner.device_id, 600)
            .unwrap();
        let peer = authority
            .complete_pairing(
                &invite.invite_id,
                &invite.code,
                &peer_key.public_key(),
                "tag peer",
                &peer_key.sign(&pairing_proof_message(
                    &invite.invite_id,
                    &invite.code,
                    &peer_key.public_key_hex(),
                )),
            )
            .unwrap();
        let fault = Fault::default();
        let server = Server::start(authority, fault.clone());
        let owner_client = SyncClient::new(&server.base, &owner.device_id, owner_key).unwrap();
        let peer_client = SyncClient::new(&server.base, &peer.device_id, peer_key).unwrap();
        let key = WorkspaceKey::from_bytes(TEST_KEY);
        let mut envelopes = Vec::new();
        for (record, seed, client) in [
            (&owner, OWNER_SEED, &owner_client),
            (&peer, PEER_SEED, &peer_client),
        ] {
            let keys = DeviceKeyPair::from_seed(seed).unwrap();
            let public = keys.encryption_public_key().unwrap();
            client
                .register_encryption_key(&encryption_public_key_hex(&public))
                .unwrap();
            envelopes.push((
                record.device_id.clone(),
                wrap_workspace_key(
                    public,
                    &key,
                    &workspace_key_aad(&workspace, 1, &record.device_id),
                )
                .unwrap(),
            ));
        }
        owner_client
            .rotate_workspace_key(&workspace, 1, &envelopes)
            .unwrap();
        (
            Self {
                server,
                workspace,
                owner,
                peer,
                fault,
            },
            Library::open(a_path).unwrap(),
            Library::open(b_path).unwrap(),
        )
    }
    fn engine(&self, peer: bool, state: &Path) -> SyncEngine {
        let (record, seed) = if peer {
            (&self.peer, PEER_SEED)
        } else {
            (&self.owner, OWNER_SEED)
        };
        SyncEngine::with_state(
            SyncClient::new(
                &self.server.base,
                &record.device_id,
                DeviceKeyPair::from_seed(seed).unwrap(),
            )
            .unwrap(),
            WorkspaceKey::from_bytes(TEST_KEY),
            &self.workspace,
            1,
            state,
        )
        .unwrap()
    }
    fn raw(
        &self,
        engine: &SyncEngine,
        wire: &str,
        base: i64,
        value: Option<serde_json::Value>,
    ) -> ApplyOutcome {
        let kind = if value.is_some() {
            OpKind::Put
        } else {
            OpKind::Delete
        };
        self.raw_kind(engine, wire, base, kind, value)
    }
    fn raw_kind(
        &self,
        engine: &SyncEngine,
        wire: &str,
        base: i64,
        kind: OpKind,
        value: Option<serde_json::Value>,
    ) -> ApplyOutcome {
        let ciphertext = value.map(|v| {
            WorkspaceKey::from_bytes(TEST_KEY)
                .encrypt(
                    &serde_json::to_vec(&v).unwrap(),
                    &object_aad(&self.workspace, wire, 1),
                )
                .unwrap()
                .encode()
        });
        engine
            .client()
            .apply_op(&SyncOp {
                op_id: id(),
                workspace_id: self.workspace.clone(),
                object_id: wire.into(),
                kind,
                base_version: base,
                key_epoch: 1,
                ciphertext,
                client_time: Some(chrono::Utc::now().to_rfc3339()),
            })
            .unwrap()
    }
}

fn create_tag(db: &mut Library, name: &str) -> TagDetails {
    db.tag_create(
        &id(),
        CreateTag {
            name: name.into(),
            description: "真实标签说明".into(),
        },
    )
    .unwrap()
    .tag
}
fn rename(db: &mut Library, tag: &str, name: &str) {
    let old = db.tag_details(tag).unwrap();
    db.tag_update(
        &id(),
        UpdateTag {
            id: tag.into(),
            base_row_version: old.row_version,
            name: name.into(),
            description: old.description,
        },
    )
    .unwrap();
}
fn assign(db: &mut Library, object: &str, tag: &str) {
    let base = db.object_metadata(object).unwrap().row_version;
    db.assign_tag(
        &id(),
        AssignTag {
            targets: vec![MetadataTarget {
                object_id: object.into(),
                base_row_version: base,
            }],
            tag: TagSelector::Id { id: tag.into() },
        },
    )
    .unwrap();
}
fn remove(db: &mut Library, object: &str, tag: &str) {
    let base = db.object_metadata(object).unwrap().row_version;
    db.remove_tag(
        &id(),
        RemoveTag {
            targets: vec![MetadataTarget {
                object_id: object.into(),
                base_row_version: base,
            }],
            tag_id: tag.into(),
        },
    )
    .unwrap();
}
fn delete_tag(db: &mut Library, tag: &str) {
    let old = db.tag_details(tag).unwrap();
    db.tag_delete(
        &id(),
        DeleteTag {
            id: tag.into(),
            base_row_version: old.row_version,
            impact_token: old.impact_token,
        },
    )
    .unwrap();
}
fn create_note(db: &mut Library, title: &str) -> Note {
    db.create_note(title, "真实保留正文").unwrap()
}
fn note_at(db: &mut Library, object: &str, title: &str) -> Note {
    db.create_document_with_id(
        &id(),
        object,
        CreateNote {
            title: title.into(),
            document_schema: DOCUMENT_SCHEMA,
            document: NoteDocument::from_text(id(), "原始正文"),
            collection_id: None,
            base_collection_row_version: None,
        },
    )
    .unwrap()
    .note
}
fn decision(db: &Library, kind: TagSyncKind, entity: &str) -> TagSyncDecision {
    let s = db.tag_sync_snapshot(kind, entity).unwrap();
    TagSyncDecision {
        kind,
        entity_id: entity.into(),
        remote_version: s.remote_version,
        local_fingerprint: s.local_fingerprint,
        local_impact_token: s.local_impact_token,
    }
}
fn filtered_view(db: &mut Library, tag: &str) -> SavedView {
    db.view_save(
        &id(),
        SaveView {
            view_id: None,
            base_row_version: None,
            name: "标签范围".into(),
            filter_schema: 1,
            filter: NotesFilter {
                tag_ids: vec![tag.into()],
                ..Default::default()
            },
            sort: NoteSort::UpdatedDesc,
            layout: ViewLayout::List,
            config_schema: 1,
            config: ViewConfig::default(),
            is_pinned: false,
            pin_position: None,
        },
    )
    .unwrap()
    .view
}

fn note_json(note: &Note) -> serde_json::Value {
    serde_json::json!({
        "title": note.title,
        "document_schema": note.schema_version,
        "document": note.document,
        "updated_at": note.updated_at,
    })
}

fn local_payload(db: &Library, kind: TagSyncKind, entity: &str) -> TagSyncPayload {
    db.tag_sync_snapshot(kind, entity).unwrap().payload.unwrap()
}

fn assignment_payload(object: &str, kind: ObjectKind, tag: &str) -> TagSyncPayload {
    TagSyncPayload::TagAssignments {
        assignments: SyncedTagAssignments {
            object_id: object.into(),
            object_kind: kind,
            tag_ids: vec![tag.into()],
        },
    }
}

fn restore_copy(db: &Library, root: &Path, name: &str) -> Library {
    let package = root.join(format!("{name}.pkmsbackup"));
    db.create_backup(&package, &mut |_| Ok(())).unwrap();
    let destination = root.join(name);
    prepare_restore(&package, &destination, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    Library::open_existing(destination).unwrap()
}

#[test]
fn real_http_dictionary_assignment_and_filtered_view_use_the_same_ids_and_preserve_body_revisions()
{
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea.sqlite3"));
    let mut eb = env.engine(true, &root.path().join("eb.sqlite3"));
    let note = create_note(&mut a, "资料结论");
    let tag = create_tag(&mut a, "主题甲");
    assign(&mut a, &note.id, &tag.tag.id);
    let view = filtered_view(&mut a, &tag.tag.id);
    ea.push_note(&a, &note.id).unwrap();
    assert!(
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .complete
    );
    assert!(
        ea.push_tag_record(&mut a, TagSyncKind::TagAssignments, &note.id)
            .unwrap()
            .complete
    );
    ea.push_view(&a, &view.id).unwrap();
    let pulled = eb.pull_and_apply(&mut b, 100).unwrap();
    assert!(
        pulled
            .tag_results
            .iter()
            .any(|r| r.kind == TagSyncKind::Tag)
    );
    assert_eq!(b.object_metadata(&note.id).unwrap().tags[0].id, tag.tag.id);
    assert_eq!(
        b.view_run(&view.id, 10, None).unwrap().notes.items[0]
            .note
            .id,
        note.id
    );
    let body = b.note(&note.id).unwrap();
    let assignment_fp = a
        .tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
        .unwrap()
        .local_fingerprint;
    rename(&mut a, &tag.tag.id, "主题改名");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(
        b.object_metadata(&note.id).unwrap().tags[0].display_name,
        "主题改名"
    );
    assert_eq!(
        a.tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
            .unwrap()
            .local_fingerprint,
        assignment_fp
    );
    let old = a.tag_details(&tag.tag.id).unwrap();
    a.tag_update(
        &id(),
        UpdateTag {
            id: tag.tag.id.clone(),
            base_row_version: old.row_version,
            name: old.tag.display_name,
            description: "只改说明".into(),
        },
    )
    .unwrap();
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(b.tag_details(&tag.tag.id).unwrap().description, "只改说明");
    remove(&mut a, &note.id, &tag.tag.id);
    ea.push_tag_record(&mut a, TagSyncKind::TagAssignments, &note.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert!(
        b.view_run(&view.id, 10, None)
            .unwrap()
            .notes
            .items
            .is_empty()
    );
    assert_eq!(b.note(&note.id).unwrap().revision_id, body.revision_id);
    assert_eq!(b.note(&note.id).unwrap().document, body.document);
}

#[test]
fn out_of_order_assignment_is_not_pushed_as_empty_and_applies_after_the_note_arrives() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let note = create_note(&mut a, "稍后传来的笔记");
    let tag = create_tag(&mut a, "先传标签");
    assign(&mut a, &note.id, &tag.tag.id);
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    ea.push_tag_record(&mut a, TagSyncKind::TagAssignments, &note.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 1).unwrap();
    eb.pull_and_apply(&mut b, 1).unwrap();
    let pending = b
        .tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
        .unwrap();
    assert_eq!(
        pending.pending.unwrap().reason,
        TagSyncPendingReason::MissingObject
    );
    assert!(pending.payload.is_none());
    let requests = env.fault.submitted.lock().unwrap().len();
    let skipped = eb
        .push_tag_record(&mut b, TagSyncKind::TagAssignments, &note.id)
        .unwrap();
    assert_eq!(skipped.outcome, TagPushOutcome::Pending);
    assert!(!skipped.complete);
    assert_eq!(env.fault.submitted.lock().unwrap().len(), requests);
    assert!(b.list_notes().unwrap().is_empty());
    ea.push_note(&a, &note.id).unwrap();
    eb.pull_and_apply(&mut b, 1).unwrap();
    assert_eq!(b.object_metadata(&note.id).unwrap().tags[0].id, tag.tag.id);
    assert!(
        b.tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
            .unwrap()
            .pending
            .is_none()
    );
    assert_eq!(b.list_notes().unwrap().len(), 1);
}

#[test]
fn lost_http_receipt_reopen_then_edit_acknowledges_the_exact_old_payload_before_publishing_current()
{
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let state = root.path().join("ea");
    let mut ea = env.engine(false, &state);
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "原始发送");
    env.fault.lose_receipt.store(true, Ordering::SeqCst);
    assert!(
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .is_err()
    );
    assert!(ea.has_pending_tag_requests().unwrap());
    assert_eq!(
        ea.pending_tag_records().unwrap(),
        vec![(TagSyncKind::Tag, tag.tag.id.clone())]
    );
    assert_eq!(
        a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .base_version,
        0
    );
    drop(ea);
    rename(&mut a, &tag.tag.id, "重启后的新修改");
    let mut ea = env.engine(false, &state);
    let result = ea
        .push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    assert_eq!(result.outcome, TagPushOutcome::Published { version: 2 });
    assert!(result.complete);
    assert!(!ea.has_pending_tag_requests().unwrap());
    assert!(ea.pending_tag_records().unwrap().is_empty());
    assert_eq!(result.snapshot.base_version, 2);
    let sent = env.fault.submitted.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0], sent[1]);
    assert_ne!(sent[1].op_id, sent[2].op_id);
    assert_eq!(sent[2].base_version, 1);
    drop(sent);
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(
        b.tag_details(&tag.tag.id).unwrap().tag.display_name,
        "重启后的新修改"
    );
}

#[test]
fn a_core_ack_failure_keeps_the_durable_request_for_recovery() {
    let root = temp();
    let (env, mut a, _b) = Fixture::new(root.path());
    let state = root.path().join("ea");
    let mut ea = env.engine(false, &state);
    let tag = create_tag(&mut a, "待确认");
    let raw = rusqlite::Connection::open(root.path().join("a/library.sqlite3")).unwrap();
    raw.execute_batch("CREATE TRIGGER test_ack_abort BEFORE UPDATE OF base_version ON tag_sync_state WHEN NEW.base_version=1 BEGIN SELECT RAISE(ABORT,'controlled ack failure'); END;").unwrap();
    assert!(
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .is_err()
    );
    assert_eq!(
        a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .base_version,
        0
    );
    raw.execute_batch("DROP TRIGGER test_ack_abort;").unwrap();
    drop(raw);
    drop(ea);
    rename(&mut a, &tag.tag.id, "确认前的新编辑");
    let mut ea = env.engine(false, &state);
    let result = ea
        .push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.snapshot.base_version, 2);
    let sent = env.fault.submitted.lock().unwrap();
    assert_eq!(sent[0], sent[1]);
    assert_eq!(sent[2].base_version, 1);
}

#[test]
fn pull_recovers_an_unknown_tag_receipt_before_replaying_history_or_accepting_later_edits() {
    for deny_replay in [false, true] {
        let root = temp();
        let (env, _owner_library, mut a) = Fixture::new(root.path());
        let state = root.path().join("ea");
        let admin = env.engine(false, &root.path().join("admin"));
        let mut ea = env.engine(true, &state);
        let tag = create_tag(&mut a, "原请求正文");
        let sent_payload = local_payload(&a, TagSyncKind::Tag, &tag.tag.id);
        env.fault.lose_receipt.store(true, Ordering::SeqCst);
        assert!(
            ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
                .is_err()
        );
        drop(ea);
        rename(&mut a, &tag.tag.id, "重启后未同步的编辑");
        if deny_replay {
            admin
                .client()
                .set_device_role(&env.peer.device_id, pkms_sync::MemberRole::Viewer)
                .unwrap();
        }
        let mut ea = env.engine(true, &state);
        // In the denied branch only the exact immutable server log can prove
        // the earlier commit; the replay's rejection is not a non-commit proof.
        ea.pull_and_apply(&mut a, 100).unwrap();
        let current = a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap();
        assert_eq!(current.base_version, 1);
        assert_eq!(
            current.base_fingerprint,
            Some(sent_payload.fingerprint().unwrap())
        );
        assert!(current.dirty);
        assert!(current.pending.is_none());
        assert_eq!(
            a.tag_details(&tag.tag.id).unwrap().tag.display_name,
            "重启后未同步的编辑"
        );
        let pending: i64 = rusqlite::Connection::open(&state)
            .unwrap()
            .query_row("SELECT count(*) FROM sync_pending_operations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
        if deny_replay {
            admin
                .client()
                .set_device_role(&env.peer.device_id, pkms_sync::MemberRole::Editor)
                .unwrap();
        }
        let result = ea
            .push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap();
        assert_eq!(result.outcome, TagPushOutcome::Published { version: 2 });
        assert!(result.complete);
    }
}

#[test]
fn checked_keep_local_unknown_receipt_recovers_its_original_decision_then_current_edit() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let state = root.path().join("ea");
    let mut ea = env.engine(false, &state);
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "共同基准");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    rename(&mut a, &tag.tag.id, "本机决定");
    rename(&mut b, &tag.tag.id, "另端变更");
    eb.push_tag_record(&mut b, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    ea.pull_and_apply(&mut a, 100).unwrap();
    let preview = decision(&a, TagSyncKind::Tag, &tag.tag.id);
    env.fault.lose_receipt.store(true, Ordering::SeqCst);
    assert!(ea.keep_local_tag_checked(&mut a, preview.clone()).is_err());
    drop(ea);
    rename(&mut a, &tag.tag.id, "决定之后又编辑");
    let mut ea = env.engine(false, &state);
    let result = ea.keep_local_tag_checked(&mut a, preview).unwrap();
    assert!(result.complete);
    assert_eq!(result.snapshot.base_version, 4);
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(
        b.tag_details(&tag.tag.id).unwrap().tag.display_name,
        "决定之后又编辑"
    );
    let sent = env.fault.submitted.lock().unwrap();
    assert_eq!(sent[2], sent[3]);
    assert_eq!(sent[4].base_version, 3);
}

#[test]
fn checked_remote_delete_rejects_post_preview_assignments_and_preserves_note_bodies() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "删除比较");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    let first = create_note(&mut a, "先分配");
    let second = create_note(&mut a, "后分配");
    assign(&mut a, &first.id, &tag.tag.id);
    delete_tag(&mut b, &tag.tag.id);
    eb.push_tag_record(&mut b, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    ea.pull_and_apply(&mut a, 100).unwrap();
    let preview = decision(&a, TagSyncKind::Tag, &tag.tag.id);
    assign(&mut a, &second.id, &tag.tag.id);
    assert!(ea.keep_remote_tag_checked(&mut a, preview).is_err());
    assert_eq!(
        a.object_metadata(&second.id).unwrap().tags[0].id,
        tag.tag.id
    );
    let preview = decision(&a, TagSyncKind::Tag, &tag.tag.id);
    assert_eq!(
        ea.keep_remote_tag_checked(&mut a, preview).unwrap().status,
        TagSyncApplyStatus::Applied
    );
    assert!(a.object_metadata(&first.id).unwrap().tags.is_empty());
    assert!(a.object_metadata(&second.id).unwrap().tags.is_empty());
    assert_eq!(a.note(&first.id).unwrap().revision_id, first.revision_id);
    assert_eq!(a.note(&second.id).unwrap().revision_id, second.revision_id);
}

#[test]
fn stale_remote_comparison_is_rejected_without_adopting_the_newer_record() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "基准");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    rename(&mut a, &tag.tag.id, "人工保留");
    rename(&mut b, &tag.tag.id, "远端二版");
    eb.push_tag_record(&mut b, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    ea.pull_and_apply(&mut a, 100).unwrap();
    let preview = decision(&a, TagSyncKind::Tag, &tag.tag.id);
    let before = a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap();
    rename(&mut b, &tag.tag.id, "远端三版");
    eb.push_tag_record(&mut b, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    assert!(matches!(
        ea.keep_remote_tag_checked(&mut a, preview),
        Err(EngineError::ConfigConflictChanged)
    ));
    assert_eq!(
        a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap(),
        before
    );
    assert_eq!(
        a.tag_details(&tag.tag.id).unwrap().tag.display_name,
        "人工保留"
    );
    ea.refresh_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    assert_eq!(
        a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .remote_version,
        3
    );
}

#[test]
fn explicit_unknown_or_malformed_tag_discriminators_never_fall_back_to_notes() {
    for discriminator in [
        serde_json::json!("future"),
        serde_json::Value::Null,
        serde_json::json!("tag"),
    ] {
        let root = temp();
        let (env, mut a, mut b) = Fixture::new(root.path());
        let ea = env.engine(false, &root.path().join("ea"));
        let mut eb = env.engine(true, &root.path().join("eb"));
        let note = create_note(&mut a, "合法旧笔记字段不能掩盖未知类型");
        let mut value = note_json(&note);
        value["sync_kind"] = discriminator;
        assert!(matches!(
            env.raw(&ea, &note.id, 0, Some(value)),
            ApplyOutcome::Applied(_)
        ));
        assert!(eb.seed_from_snapshot(&b).is_err());
        assert!(eb.pull_and_apply(&mut b, 100).is_err());
        assert!(b.list_notes().unwrap().is_empty());
    }
}

#[test]
fn retained_tag_wire_cannot_be_reinterpreted_by_a_later_note_even_after_snapshot_seed() {
    for seeded in [false, true] {
        let root = temp();
        let (env, mut a, mut b) = Fixture::new(root.path());
        let mut ea = env.engine(false, &root.path().join("ea"));
        let mut eb = env.engine(true, &root.path().join("eb"));
        let tag = create_tag(&mut a, "保留类型");
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap();
        let wire = tag_sync_object_id(TagSyncKind::Tag, &tag.tag.id).unwrap();
        let fake = if seeded {
            note_at(&mut b, &wire, "事先存在的普通笔记")
        } else {
            create_note(&mut a, "伪装笔记")
        };
        assert!(matches!(
            env.raw(&ea, &wire, 1, Some(note_json(&fake))),
            ApplyOutcome::Applied(_)
        ));
        if seeded {
            eb.seed_from_snapshot(&b).unwrap();
        }
        assert!(eb.pull_and_apply(&mut b, 100).is_err());
        if seeded {
            assert_eq!(b.note(&wire).unwrap().revision_id, fake.revision_id);
            assert_eq!(b.list_notes().unwrap().len(), 1);
        } else {
            assert!(b.list_notes().unwrap().is_empty());
            assert_eq!(
                b.tag_details(&tag.tag.id).unwrap().tag.display_name,
                "保留类型"
            );
        }
    }
}

#[test]
fn ordinary_note_push_is_rejected_for_both_local_and_remote_tag_wire_owners() {
    for local_owner in [false, true] {
        let root = temp();
        let (env, mut a, mut b) = Fixture::new(root.path());
        let mut ea = env.engine(false, &root.path().join("ea"));
        let mut eb = env.engine(true, &root.path().join("eb"));
        let tag = create_tag(&mut a, "不能覆盖");
        let wire = tag_sync_object_id(TagSyncKind::Tag, &tag.tag.id).unwrap();
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap();
        if local_owner {
            eb.pull_and_apply(&mut b, 100).unwrap();
        }
        note_at(&mut b, &wire, "冲突实体");
        let count = env.fault.submitted.lock().unwrap().len();
        assert!(eb.preflight_non_tag_object(&b, &wire).is_err());
        assert!(eb.push_note(&b, &wire).is_err());
        assert_eq!(env.fault.submitted.lock().unwrap().len(), count);
        let snapshot = ea.client().snapshot(&env.workspace).unwrap();
        assert_eq!(
            snapshot
                .iter()
                .find(|o| o.object_id == wire)
                .unwrap()
                .version,
            1
        );
    }
}

#[test]
fn generic_resolutions_and_configuration_removals_cannot_close_or_copy_a_tag_conflict() {
    for local_owner in [false, true] {
        let root = temp();
        let (env, mut a, mut b) = Fixture::new(root.path());
        let mut ea = env.engine(false, &root.path().join("ea"));
        let mut eb = env.engine(true, &root.path().join("eb"));
        let tag = create_tag(&mut a, "通用确认不得绕过");
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap();
        if local_owner {
            eb.pull_and_apply(&mut b, 100).unwrap();
        }
        let wire = tag_sync_object_id(TagSyncKind::Tag, &tag.tag.id).unwrap();
        let note = note_at(&mut b, &wire, "冲突普通笔记");
        assert!(matches!(
            env.raw(&eb, &wire, 0, Some(note_json(&note))),
            ApplyOutcome::Conflict(_)
        ));
        let conflicts = eb.client().pull(&env.workspace, 0, 1).unwrap().conflicts;
        assert_eq!(conflicts.len(), 1);
        let target = pkms_sync_engine::ConflictTarget::from(&conflicts[0]);
        let count = env.fault.submitted.lock().unwrap().len();
        assert!(eb.resolve_keep_local_target(&b, &target).is_err());
        assert!(eb.resolve_keep_remote_target(&mut b, &target).is_err());
        assert!(eb.resolve_save_as_new_target(&mut b, &target).is_err());
        assert!(eb.keep_remote_config(&mut b, &wire).is_err());
        assert!(eb.push_view_remove(&wire).is_err());
        assert!(eb.push_canvas_remove(&wire).is_err());
        assert_eq!(env.fault.submitted.lock().unwrap().len(), count);
        assert_eq!(b.list_notes().unwrap().len(), 1);
        assert_eq!(b.note(&wire).unwrap().revision_id, note.revision_id);
        assert_eq!(
            eb.client().pull(&env.workspace, 0, 1).unwrap().conflicts,
            conflicts
        );
        assert_eq!(ea.client().snapshot(&env.workspace).unwrap()[0].version, 1);
    }
}

#[test]
fn raw_delete_restore_and_purge_do_not_replace_retained_tag_put_records() {
    for ending in [OpKind::Delete, OpKind::Restore, OpKind::Purge] {
        let root = temp();
        let (env, mut a, mut b) = Fixture::new(root.path());
        let mut ea = env.engine(false, &root.path().join("ea"));
        let mut eb = env.engine(true, &root.path().join("eb"));
        let tag = create_tag(&mut a, "不能用协议墓碑");
        let wire = tag_sync_object_id(TagSyncKind::Tag, &tag.tag.id).unwrap();
        let value = serde_json::to_value(local_payload(&a, TagSyncKind::Tag, &tag.tag.id)).unwrap();
        ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
            .unwrap();
        eb.pull_and_apply(&mut b, 100).unwrap();
        let before = b.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap();
        let ApplyOutcome::Applied(deleted) = env.raw(&ea, &wire, 1, None) else {
            panic!("raw deletion fixture must commit");
        };
        if ending != OpKind::Delete {
            assert!(matches!(
                env.raw_kind(
                    &ea,
                    &wire,
                    2,
                    ending,
                    (ending == OpKind::Restore).then_some(value),
                ),
                ApplyOutcome::Applied(_)
            ));
        }
        if ending != OpKind::Restore {
            assert!(eb.seed_from_snapshot(&b).is_err());
        }
        assert!(eb.pull_and_apply(&mut b, 100).is_err());
        if ending != OpKind::Delete {
            // Test the final malformed operation independently of the earlier
            // Delete rejection, as if an older client had advanced that cursor.
            eb.set_cursor(deleted.server_seq).unwrap();
            assert!(eb.pull_and_apply(&mut b, 100).is_err());
        }
        assert_eq!(
            b.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap(),
            before
        );
        assert_eq!(
            b.tag_details(&tag.tag.id).unwrap().tag.display_name,
            "不能用协议墓碑"
        );
    }
}

#[test]
fn full_backup_and_fresh_engine_preserve_tag_baselines_and_unsynced_local_edits() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "版本一");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    rename(&mut a, &tag.tag.id, "版本二");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    rename(&mut b, &tag.tag.id, "备份中尚未同步的修改");
    let before = b.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id).unwrap();
    let mut restored = restore_copy(&b, root.path(), "restored-edited");
    let mut fresh = env.engine(true, &root.path().join("fresh-engine"));
    fresh.seed_from_snapshot(&restored).unwrap();
    assert!(fresh.seed_tag_snapshot(&mut restored).unwrap().is_empty());
    fresh.pull_and_apply(&mut restored, 100).unwrap();
    assert_eq!(
        restored
            .tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap(),
        before
    );
    let published = fresh
        .push_tag_record(&mut restored, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    assert_eq!(published.outcome, TagPushOutcome::Published { version: 3 });
    assert!(published.complete);
    ea.pull_and_apply(&mut a, 100).unwrap();
    assert_eq!(
        a.tag_details(&tag.tag.id).unwrap().tag.display_name,
        "备份中尚未同步的修改"
    );
}

#[test]
fn two_phase_seed_and_completion_receipt_failures_remain_retryable_after_reopen() {
    for fail_core_ack in [false, true] {
        let root = temp();
        let (env, mut a, _b) = Fixture::new(root.path());
        let mut sender = env.engine(false, &root.path().join("sender"));
        let note = create_note(&mut a, "普通种子成功");
        sender.push_note(&a, &note.id).unwrap();
        let tag = create_tag(&mut a, "标签种子随后");
        let payload = local_payload(&a, TagSyncKind::Tag, &tag.tag.id);
        assert!(matches!(
            env.raw(
                &sender,
                &payload.sync_object_id().unwrap(),
                0,
                Some(serde_json::to_value(payload).unwrap())
            ),
            ApplyOutcome::Applied(_)
        ));
        let state = root.path().join("fresh-state");
        let mut fresh = env.engine(false, &state);
        let injection = if fail_core_ack {
            rusqlite::Connection::open(root.path().join("a/library.sqlite3")).unwrap()
        } else {
            rusqlite::Connection::open(&state).unwrap()
        };
        injection.execute_batch(if fail_core_ack {
            "CREATE TRIGGER test_seed_abort BEFORE UPDATE OF base_version ON tag_sync_state WHEN NEW.base_version=1 BEGIN SELECT RAISE(ABORT,'controlled tag seed ack failure'); END;"
        } else {
            "CREATE TRIGGER test_seed_abort BEFORE UPDATE OF initial_seed_completed ON sync_state WHEN NEW.initial_seed_completed=1 BEGIN SELECT RAISE(ABORT,'controlled seed completion failure'); END;"
        }).unwrap();
        assert!(fresh.initialize_from_snapshot(&mut a).is_err());
        assert!(fresh.needs_initial_seed());
        assert_eq!(fresh.synced_version(&note.id), Some(1));
        drop(fresh);
        let mut fresh = env.engine(false, &state);
        assert!(fresh.needs_initial_seed());
        assert_eq!(fresh.synced_version(&note.id), Some(1));
        injection
            .execute_batch("DROP TRIGGER test_seed_abort;")
            .unwrap();
        fresh.initialize_from_snapshot(&mut a).unwrap();
        assert!(!fresh.needs_initial_seed());
        assert_eq!(
            a.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
                .unwrap()
                .base_version,
            1
        );
        drop(fresh);
        let fresh = env.engine(false, &state);
        assert!(!fresh.needs_initial_seed());
        assert_eq!(fresh.synced_version(&note.id), Some(1));
    }
}

#[test]
fn missing_source_assignment_survives_backup_without_creating_or_publishing_an_empty_entity() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "资料待达");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    let missing = id();
    let payload = assignment_payload(&missing, ObjectKind::Source, &tag.tag.id);
    let wire = payload.sync_object_id().unwrap();
    assert!(matches!(
        env.raw(&ea, &wire, 0, Some(serde_json::to_value(payload).unwrap())),
        ApplyOutcome::Applied(_)
    ));
    eb.pull_and_apply(&mut b, 100).unwrap();
    let before = b
        .tag_sync_snapshot(TagSyncKind::TagAssignments, &missing)
        .unwrap();
    assert_eq!(
        before.pending.as_ref().unwrap().reason,
        TagSyncPendingReason::MissingObject
    );
    assert!(before.payload.is_none());
    let mut restored = restore_copy(&b, root.path(), "restored-source");
    let mut fresh = env.engine(true, &root.path().join("fresh-source-engine"));
    fresh.pull_and_apply(&mut restored, 100).unwrap();
    assert_eq!(
        restored
            .tag_sync_snapshot(TagSyncKind::TagAssignments, &missing)
            .unwrap(),
        before
    );
    let count = env.fault.submitted.lock().unwrap().len();
    let skipped = fresh
        .push_tag_record(&mut restored, TagSyncKind::TagAssignments, &missing)
        .unwrap();
    assert_eq!(skipped.outcome, TagPushOutcome::Pending);
    assert!(!skipped.complete);
    assert_eq!(env.fault.submitted.lock().unwrap().len(), count);
    assert!(matches!(
        restored.object_metadata(&missing),
        Err(Error::NotFound)
    ));
    assert!(restored.list_notes().unwrap().is_empty());
}

#[test]
fn pending_dependency_retry_reaches_later_pages_behind_one_hundred_permanently_missing_objects() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "不饿死后页");
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    // Core state is the durable queue. These records all sort before the real note.
    for n in 1..=100_u32 {
        let missing = format!("00000000-0000-4000-8000-{n:012x}");
        let payload = assignment_payload(&missing, ObjectKind::Source, &tag.tag.id);
        b.tag_sync_receive(&payload.sync_object_id().unwrap(), payload, 1)
            .unwrap();
    }
    let note = note_at(&mut a, "ffffffff-ffff-4fff-bfff-ffffffffffff", "最后一页");
    let payload = assignment_payload(&note.id, ObjectKind::Note, &tag.tag.id);
    b.tag_sync_receive(&payload.sync_object_id().unwrap(), payload, 1)
        .unwrap();
    ea.push_note(&a, &note.id).unwrap();
    let result = eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(b.object_metadata(&note.id).unwrap().tags[0].id, tag.tag.id);
    assert!(
        b.tag_sync_snapshot(TagSyncKind::TagAssignments, &note.id)
            .unwrap()
            .pending
            .is_none()
    );
    assert!(
        result
            .tag_results
            .iter()
            .any(|r| r.entity_id == note.id && r.status == TagSyncApplyStatus::Applied)
    );
    let first = b
        .tag_sync_page(TagSyncPageRequest {
            limit: 100,
            after: None,
            pending_only: true,
        })
        .unwrap();
    assert_eq!(first.items.len(), 100);
    assert!(first.next_cursor.is_none());
}

#[test]
fn dictionary_deletion_waits_for_its_filtered_view_removal_then_retries_automatically() {
    let root = temp();
    let (env, mut a, mut b) = Fixture::new(root.path());
    let mut ea = env.engine(false, &root.path().join("ea"));
    let mut eb = env.engine(true, &root.path().join("eb"));
    let tag = create_tag(&mut a, "视图依赖");
    let view = filtered_view(&mut a, &tag.tag.id);
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    ea.push_view(&a, &view.id).unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    a.view_remove(
        &id(),
        RemoveView {
            view_id: view.id.clone(),
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    delete_tag(&mut a, &tag.tag.id);
    ea.push_tag_record(&mut a, TagSyncKind::Tag, &tag.tag.id)
        .unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert_eq!(
        b.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .pending
            .unwrap()
            .reason,
        TagSyncPendingReason::ViewDependency
    );
    ea.push_view_remove(&view.id).unwrap();
    eb.pull_and_apply(&mut b, 100).unwrap();
    assert!(
        b.tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .pending
            .is_none()
    );
    assert!(matches!(b.tag_details(&tag.tag.id), Err(Error::NotFound)));
    assert!(matches!(b.view_get(&view.id), Err(Error::NotFound)));
}
