use std::collections::HashMap;
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pkms_core::{
    BoardGroupBy, CanvasNode, CreateNote, DOCUMENT_SCHEMA, Library, Note, NoteDocument, NoteSort,
    NotesFilter, SaveCanvas, SaveNote, SaveView, UpdateWorkspaceMetadata, ViewConfig, ViewLayout,
    ViewProperty,
};
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, SyncAuthority, WorkspaceKey, encryption_public_key_hex,
    owner_proof_message, pairing_proof_message, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use pkms_sync_engine::{PushOutcome, SyncEngine};
use uuid::Uuid;

const OWNER_SEED: [u8; 32] = [55u8; 32];
const PHONE_SEED: [u8; 32] = [66u8; 32];

#[derive(Clone, Default)]
struct ReceiptFault {
    lose_next: Arc<std::sync::atomic::AtomicBool>,
    lose_resolution: Arc<std::sync::atomic::AtomicBool>,
    submitted: Arc<Mutex<Vec<pkms_sync::SyncOp>>>,
}

async fn lose_committed_receipt(
    axum::extract::State(fault): axum::extract::State<ReceiptFault>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if request.uri().path() == "/v1/conflicts/resolve" {
        let response = next.run(request).await;
        if response.status().is_success()
            && fault
                .lose_resolution
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        return response;
    }
    if request.uri().path() != "/v1/ops" {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024).await.unwrap();
    let body: pkms_sync::transport::ApplyOpBody = serde_json::from_slice(&bytes).unwrap();
    fault.submitted.lock().unwrap().push(body.op);
    let response = next
        .run(axum::extract::Request::from_parts(
            parts,
            axum::body::Body::from(bytes),
        ))
        .await;
    if response.status().is_success()
        && fault
            .lose_next
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    {
        // The real authority has already committed the signed operation.
        // Only its successful receipt is replaced; no product behavior is mocked.
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    response
}

fn start_fault_server(authority: SyncAuthority, fault: ReceiptFault) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let router = pkms_sync_server::router(Arc::new(Mutex::new(authority))).layer(
                    axum::middleware::from_fn_with_state(fault, lose_committed_receipt),
                );
                axum::serve(listener, router).await.unwrap();
            });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(address).is_ok() {
            return format!("http://{address}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("fault server did not become ready");
}

fn retry_view(library: &mut Library, name: &str) -> pkms_core::SavedView {
    library
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: name.into(),
                filter_schema: 1,
                filter: NotesFilter::default(),
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

fn retry_rename_view(library: &mut Library, view: &pkms_core::SavedView, name: &str) {
    library
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: Some(view.id.clone()),
                base_row_version: Some(view.row_version),
                name: name.into(),
                filter_schema: view.filter_schema,
                filter: view.filter.clone(),
                sort: view.sort,
                layout: view.layout,
                config_schema: view.config_schema,
                config: view.config.clone(),
                is_pinned: view.is_pinned,
                pin_position: view.pin_position,
            },
        )
        .unwrap();
}

#[test]
fn retry_seed_snapshot_initializes_every_configuration_baseline() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let note = create_note(&mut a, "seed note", "seed body");
    ea.push_note(&a, &note.id).unwrap();
    let view = retry_view(&mut a, "seed view");
    ea.push_view(&a, &view.id).unwrap();
    let canvas = a
        .canvas_save(
            &Uuid::new_v4().to_string(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "seed canvas".into(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: note.id.clone(),
                    x: 1.0,
                    y: 2.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    ea.push_canvas(&a, &canvas.id).unwrap();
    ea.push_workspace_metadata(&a).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    drop(eb);
    let mut fresh = reopen_engine(
        &base,
        &workspace,
        &phone.device_id,
        PHONE_SEED,
        &temporary.path().join("fresh-state.sqlite3"),
    );
    fresh.seed_from_snapshot(&b).unwrap();
    for object in ea.client().snapshot(&workspace).unwrap() {
        assert_eq!(
            fresh.synced_version(&object.object_id),
            Some(object.version),
            "unseeded object {}",
            object.object_id
        );
    }
    // A later peer edit must apply cleanly, not appear as a local/remote conflict.
    retry_rename_view(&mut a, &view, "changed after seed");
    ea.push_view(&a, &view.id).unwrap();
    let pulled = fresh.pull_and_apply(&mut b, 50).unwrap();
    assert!(pulled.config_conflicts.is_empty());
    assert_eq!(b.view_get(&view.id).unwrap().name, "changed after seed");
}

#[test]
fn retry_seed_clone_devices_do_not_reuse_each_others_operation_ids() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let view = retry_view(&mut a, "identical cloned view");
    b.view_apply_remote(view.clone()).unwrap();
    assert_eq!(b.view_get(&view.id).unwrap(), view);
    ea.push_view(&a, &view.id).unwrap();
    assert_eq!(
        eb.push_view(&b, &view.id).unwrap(),
        PushOutcome::Conflict {
            remote_version: 1,
            remote_tombstone: false
        }
    );
    let submitted = fault.submitted.lock().unwrap();
    assert_ne!(submitted[0].op_id, submitted[1].op_id);
}

#[test]
fn retry_seed_lost_configuration_receipt_survives_reopen_and_new_local_edit() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("owner-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let view = retry_view(&mut a, "first intended value");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_view(&a, &view.id).is_err());
    assert_eq!(ea.client().snapshot(&workspace).unwrap()[0].version, 1);
    drop(ea);
    retry_rename_view(&mut a, &view, "edited while receipt was unknown");
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_view(&a, &view.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );
    eb.pull_and_apply(&mut b, 50).unwrap();
    assert_eq!(
        b.view_get(&view.id).unwrap().name,
        "edited while receipt was unknown"
    );
    let submitted = fault.submitted.lock().unwrap();
    assert_eq!(submitted.len(), 3);
    assert_eq!(
        submitted[0], submitted[1],
        "retry must reuse ciphertext, base, time and operation ID"
    );
    assert_ne!(submitted[1].op_id, submitted[2].op_id);
    assert_ne!(submitted[1].ciphertext, submitted[2].ciphertext);
}

#[test]
fn retry_seed_note_put_delete_and_restore_replay_the_exact_request_after_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("owner-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let note = create_note(&mut a, "durable note", "content remains exact");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_note(&a, &note.id).is_err());
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_note(&a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 1 }
    );
    a.set_trashed(&note.id, true).unwrap();
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_delete(&a, &note.id).is_err());
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_delete(&a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 2 }
    );
    a.set_trashed(&note.id, false).unwrap();
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_restore(&a, &note.id).is_err());
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_restore(&a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 3 }
    );
    eb.pull_and_apply(&mut b, 50).unwrap();
    assert_eq!(b.note(&note.id).unwrap().document, note.document);
    assert!(!b.note(&note.id).unwrap().trashed);
    let submitted = fault.submitted.lock().unwrap();
    assert_eq!(submitted.len(), 6);
    for pair in submitted.as_chunks::<2>().0 {
        assert_eq!(pair[0], pair[1]);
    }
}

#[test]
fn retry_seed_keep_local_resolution_receipt_can_be_retried_after_new_edit() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("phone-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
    let note = create_note(&mut a, "initial", "base");
    ea.push_note(&a, &note.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    edit_note(&mut a, &note, "owner changes", "remote");
    ea.push_note(&a, &note.id).unwrap();
    let original_b = b.note(&note.id).unwrap();
    let edited_b = edit_note(&mut b, &original_b, "phone chooses local", "local");
    assert!(matches!(
        eb.push_note(&b, &note.id).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflict = eb.client().pull(&workspace, 0, 50).unwrap().conflicts[0].clone();
    fault
        .lose_resolution
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(eb.resolve_keep_local(&b, &conflict).is_err());
    drop(eb);
    edit_note(
        &mut b,
        &edited_b,
        "phone edits while unknown",
        "latest authorized body",
    );
    let mut eb = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
    assert_eq!(
        eb.resolve_keep_local(&b, &conflict).unwrap(),
        PushOutcome::Applied { version: 4 }
    );
    assert!(
        eb.client()
            .pull(&workspace, 0, 50)
            .unwrap()
            .conflicts
            .is_empty()
    );
    assert_eq!(
        eb.remote_payload(&note.id).unwrap().unwrap().title,
        "phone edits while unknown"
    );
    let submitted = fault.submitted.lock().unwrap();
    assert_eq!(
        submitted[3], submitted[4],
        "resolution retry must refer to the original applied op"
    );
}

#[test]
fn retry_seed_permission_rejection_preserves_local_content_and_allows_retry() {
    let temporary = tempfile::tempdir().unwrap();
    let (a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let state = temporary.path().join("phone-state.sqlite3");
    let (ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
    let view = retry_view(&mut b, "preserved while permission denied");
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Viewer)
        .unwrap();
    assert!(eb.push_view(&b, &view.id).is_err());
    assert!(ea.client().snapshot(&workspace).unwrap().is_empty());
    assert_eq!(b.view_get(&view.id).unwrap(), view);
    drop(eb);
    let mut eb = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
    // A definitive rejected write must not disable read-only synchronization.
    assert!(eb.pull_and_apply(&mut b, 50).unwrap().applied.is_empty());
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Editor)
        .unwrap();
    assert_eq!(
        eb.push_view(&b, &view.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    assert_eq!(a.info().unwrap().workspace_id, workspace);
}

#[test]
fn retry_seed_persistence_failure_prevents_network_write_and_legacy_state_reopens() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, _, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("owner-state.sqlite3");
    let (mut ea, _) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let note = create_note(&mut a, "cannot send before durable intent", "body");
    let db = rusqlite::Connection::open(&state).unwrap();
    db.execute_batch("CREATE TRIGGER reject_pending BEFORE INSERT ON sync_pending_operations BEGIN SELECT RAISE(ABORT,'injected pending write failure'); END;").unwrap();
    assert!(ea.push_note(&a, &note.id).is_err());
    assert!(fault.submitted.lock().unwrap().is_empty());
    db.execute_batch("DROP TRIGGER reject_pending;").unwrap();
    assert_eq!(
        ea.push_note(&a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 1 }
    );
    drop(ea);
    // Recreate the previous on-disk shape, which has no pending table.
    db.execute_batch("DROP TABLE sync_pending_operations;")
        .unwrap();
    drop(db);
    let ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(ea.synced_version(&note.id), Some(1));
    assert_eq!(ea.client().snapshot(&workspace).unwrap().len(), 1);
}

#[test]
fn retry_seed_old_pull_pages_do_not_lower_a_seeded_or_recovered_version() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let mut note = create_note(&mut a, "v1", "base");
    ea.push_note(&a, &note.id).unwrap();
    for version in 2..=4 {
        note = edit_note(&mut a, &note, &format!("v{version}"), "latest");
        ea.push_note(&a, &note.id).unwrap();
    }
    eb.pull_and_apply(&mut b, 50).unwrap();
    drop(eb);
    let mut eb = reopen_engine(
        &base,
        &workspace,
        &phone.device_id,
        PHONE_SEED,
        &temporary.path().join("seeded.sqlite3"),
    );
    eb.seed_from_snapshot(&b).unwrap();
    assert_eq!(eb.synced_version(&note.id), Some(4));
    eb.pull_and_apply(&mut b, 1).unwrap();
    assert_eq!(eb.synced_version(&note.id), Some(4));
    let current = b.note(&note.id).unwrap();
    edit_note(&mut b, &current, "local after seed", "latest local");
    assert_eq!(
        eb.push_note(&b, &note.id).unwrap(),
        PushOutcome::Applied { version: 5 }
    );
}

#[test]
fn retry_seed_pending_requests_cannot_be_signed_by_another_device() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, _, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("bound-state.sqlite3");
    let (mut ea, _) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let note = create_note(&mut a, "device-bound request", "body");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_note(&a, &note.id).is_err());
    drop(ea);
    let mut wrong_device = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
    assert!(matches!(
        wrong_device.push_note(&a, &note.id),
        Err(pkms_sync_engine::EngineError::PendingRequestMismatch)
    ));
    assert_eq!(fault.submitted.lock().unwrap().len(), 1);
    drop(wrong_device);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_note(&a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 1 }
    );
}

#[test]
fn retry_seed_configuration_delete_recovers_its_receipt_and_seeds_tombstones() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("owner-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let view = retry_view(&mut a, "deleted view");
    ea.push_view(&a, &view.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    a.view_remove(
        &Uuid::new_v4().to_string(),
        pkms_core::RemoveView {
            view_id: view.id.clone(),
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_view_remove(&view.id).is_err());
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_view_remove(&view.id).unwrap(),
        PushOutcome::Unchanged { version: 2 }
    );
    eb.pull_and_apply(&mut b, 50).unwrap();
    assert!(matches!(
        b.view_get(&view.id),
        Err(pkms_core::Error::NotFound)
    ));
    let submitted = fault.submitted.lock().unwrap();
    assert_eq!(submitted[1], submitted[2]);
    drop(submitted);
    let mut freshly_seeded = reopen_engine(
        &base,
        &workspace,
        &owner.device_id,
        OWNER_SEED,
        &temporary.path().join("new-state.sqlite3"),
    );
    freshly_seeded.seed_from_snapshot(&a).unwrap();
    assert_eq!(freshly_seeded.synced_version(&view.id), Some(2));
}

#[test]
fn retry_seed_latest_trash_restore_and_content_intents_follow_the_recovered_request() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, _, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("owner-state.sqlite3");
    let (mut ea, _) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    let note = create_note(&mut a, "created while online", "base");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_note(&a, &note.id).is_err());
    a.set_trashed(&note.id, true).unwrap();
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    // Retrying a put first confirms it, then sends the newer local deletion.
    assert_eq!(
        ea.push_note(&a, &note.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );
    a.set_trashed(&note.id, false).unwrap();
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_restore(&a, &note.id).is_err());
    let current = a.note(&note.id).unwrap();
    edit_note(
        &mut a,
        &current,
        "edited after restore",
        "new local content",
    );
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_restore(&a, &note.id).unwrap(),
        PushOutcome::Applied { version: 4 }
    );
    assert_eq!(
        ea.remote_payload(&note.id).unwrap().unwrap().title,
        "edited after restore"
    );
    a.set_trashed(&note.id, true).unwrap();
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(ea.push_delete(&a, &note.id).is_err());
    a.set_trashed(&note.id, false).unwrap();
    drop(ea);
    let mut ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert_eq!(
        ea.push_delete(&a, &note.id).unwrap(),
        PushOutcome::Applied { version: 6 }
    );
    assert!(!ea.client().snapshot(&workspace).unwrap()[0].tombstone);
}

fn start_server(authority: SyncAuthority) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(
                listener,
                pkms_sync_server::router(Arc::new(Mutex::new(authority))),
            )
            .await
            .unwrap();
        });
    });
    let base_url = format!("http://{address}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(address).is_ok() {
            return base_url;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("sync server did not become ready");
}

fn setup_authority() -> (SyncAuthority, DeviceRecord, DeviceRecord) {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "engine desktop",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "engine desktop", &proof)
        .unwrap();

    let phone_key = DeviceKeyPair::from_seed(PHONE_SEED).unwrap();
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .unwrap();
    let proof = phone_key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &phone_key.public_key_hex(),
    ));
    let phone = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &phone_key.public_key(),
            "engine phone",
            &proof,
        )
        .unwrap();
    (authority, owner, phone)
}

fn setup_engines(
    base_url: &str,
    workspace_id: &str,
    owner: &DeviceRecord,
    phone: &DeviceRecord,
) -> (SyncEngine, SyncEngine) {
    setup_engines_at(base_url, workspace_id, owner, phone, None, None)
}

fn setup_engines_at(
    base_url: &str,
    workspace_id: &str,
    owner: &DeviceRecord,
    phone: &DeviceRecord,
    state_a: Option<&Path>,
    state_b: Option<&Path>,
) -> (SyncEngine, SyncEngine) {
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let phone_key = DeviceKeyPair::from_seed(PHONE_SEED).unwrap();
    let owner_encryption = owner_key.encryption_public_key().unwrap();
    let phone_encryption = phone_key.encryption_public_key().unwrap();

    let desktop = SyncClient::new(base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(base_url, &phone.device_id, phone_key).unwrap();
    desktop
        .register_encryption_key(&encryption_public_key_hex(&owner_encryption))
        .unwrap();
    mobile
        .register_encryption_key(&encryption_public_key_hex(&phone_encryption))
        .unwrap();

    let workspace_key = WorkspaceKey::generate().unwrap();
    let owner_aad = workspace_key_aad(workspace_id, 1, &owner.device_id);
    let phone_aad = workspace_key_aad(workspace_id, 1, &phone.device_id);
    let owner_envelope = wrap_workspace_key(owner_encryption, &workspace_key, &owner_aad).unwrap();
    let phone_envelope = wrap_workspace_key(phone_encryption, &workspace_key, &phone_aad).unwrap();
    desktop
        .rotate_workspace_key(
            workspace_id,
            1,
            &[
                (owner.device_id.clone(), owner_envelope.clone()),
                (phone.device_id.clone(), phone_envelope.clone()),
            ],
        )
        .unwrap();

    let owner_wsk = DeviceKeyPair::from_seed(OWNER_SEED)
        .unwrap()
        .unwrap_workspace_key(&owner_envelope, &owner_aad)
        .unwrap();
    let phone_wsk = DeviceKeyPair::from_seed(PHONE_SEED)
        .unwrap()
        .unwrap_workspace_key(&phone_envelope, &phone_aad)
        .unwrap();
    let engine_a = match state_a {
        Some(path) => SyncEngine::with_state(desktop, owner_wsk, workspace_id, 1, path).unwrap(),
        None => SyncEngine::new(desktop, owner_wsk, workspace_id, 1),
    };
    let engine_b = match state_b {
        Some(path) => SyncEngine::with_state(mobile, phone_wsk, workspace_id, 1, path).unwrap(),
        None => SyncEngine::new(mobile, phone_wsk, workspace_id, 1),
    };
    (engine_a, engine_b)
}

fn workspace_libraries(root: &Path) -> (Library, Library, String) {
    let directory_a = root.join("a");
    let directory_b = root.join("b");
    let library = Library::open(&directory_a).unwrap();
    let workspace_id = library.info().unwrap().workspace_id;
    drop(library);
    std::fs::create_dir_all(&directory_b).unwrap();
    std::fs::copy(
        directory_a.join("library.sqlite3"),
        directory_b.join("library.sqlite3"),
    )
    .unwrap();
    (
        Library::open(&directory_a).unwrap(),
        Library::open(&directory_b).unwrap(),
        workspace_id,
    )
}

fn create_note(library: &mut Library, title: &str, body: &str) -> Note {
    library
        .create_document(
            &Uuid::new_v4().to_string(),
            CreateNote {
                title: title.into(),
                document_schema: DOCUMENT_SCHEMA,
                document: NoteDocument::from_text(Uuid::new_v4().to_string(), body),
                collection_id: None,
                base_collection_row_version: None,
            },
        )
        .unwrap()
        .note
}

fn edit_note(library: &mut Library, note: &Note, title: &str, body: &str) -> Note {
    library
        .save_document(
            &Uuid::new_v4().to_string(),
            SaveNote {
                note_id: note.id.clone(),
                base_revision_id: note.revision_id.clone(),
                title: title.into(),
                document_schema: note.schema_version,
                document: NoteDocument::from_text(Uuid::new_v4().to_string(), body),
            },
        )
        .unwrap()
        .note
}

#[test]
fn note_syncs_between_two_isolated_libraries_over_http() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "A note", "body A");
    assert_eq!(
        engine_a.push_note(&library_a, &note.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    );

    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert_eq!(pulled.applied, vec![note.id.clone()]);
    assert_eq!(library_b.note(&note.id).unwrap().title, "A note");

    let phone_note = library_b.note(&note.id).unwrap();
    let phone_edit = edit_note(&mut library_b, &phone_note, "B edit", "body B");
    assert_eq!(
        engine_b.push_note(&library_b, &phone_edit.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );

    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert!(pulled.applied.contains(&note.id));
    assert_eq!(library_a.note(&note.id).unwrap().title, "B edit");
}

#[test]
fn conflict_preserves_local_and_keep_local_resolves() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "base", "base body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    let a_edit = edit_note(&mut library_a, &note, "A edit", "body A");
    let b_note = library_b.note(&note.id).unwrap();
    let b_edit = edit_note(&mut library_b, &b_note, "B edit", "body B");
    assert_eq!(
        engine_b.push_note(&library_b, &b_edit.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );

    let outcome = engine_a.push_note(&library_a, &a_edit.id).unwrap();
    assert_eq!(
        outcome,
        PushOutcome::Conflict {
            remote_version: 2,
            remote_tombstone: false
        }
    );
    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert_eq!(pulled.server_conflicts.len(), 1);
    assert_eq!(library_a.note(&note.id).unwrap().title, "A edit");

    let conflict = pulled.server_conflicts[0].clone();
    assert_eq!(
        engine_a.resolve_keep_local(&library_a, &conflict).unwrap(),
        PushOutcome::Applied { version: 3 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.applied.contains(&note.id));
    assert_eq!(library_b.note(&note.id).unwrap().title, "A edit");
}

#[test]
fn pull_side_divergence_creates_durable_conflict_without_overwriting() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "base", "base body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    // A has a local edit that was never pushed. B publishes a different edit.
    let a_edit = edit_note(&mut library_a, &note, "A local", "body A");
    let b_note = library_b.note(&note.id).unwrap();
    let b_edit = edit_note(&mut library_b, &b_note, "B remote", "body B");
    engine_b.push_note(&library_b, &b_edit.id).unwrap();

    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert_eq!(pulled.conflicts.len(), 1);
    assert_eq!(pulled.server_conflicts.len(), 1);
    assert_eq!(pulled.server_conflicts[0].object_id, note.id);
    assert_eq!(library_a.note(&note.id).unwrap().title, "A local");

    // A retry cannot overwrite the remote object before an explicit choice.
    assert_eq!(
        engine_a.push_note(&library_a, &a_edit.id).unwrap(),
        PushOutcome::Conflict {
            remote_version: 2,
            remote_tombstone: false
        }
    );
    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert_eq!(pulled.server_conflicts.len(), 1);
    assert_eq!(library_a.note(&note.id).unwrap().title, "A local");
    assert_eq!(library_b.note(&note.id).unwrap().title, "B remote");

    let conflict = pulled.server_conflicts[0].clone();
    engine_a
        .resolve_keep_remote(&mut library_a, &conflict)
        .unwrap();
    assert_eq!(library_a.note(&note.id).unwrap().title, "B remote");
}

#[test]
fn local_delete_vs_remote_edit_requires_an_explicit_keep_local_delete() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "base", "base body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    library_a.set_trashed(&note.id, true).unwrap();
    let b_note = library_b.note(&note.id).unwrap();
    let b_edit = edit_note(&mut library_b, &b_note, "B edit", "body B");
    engine_b.push_note(&library_b, &b_edit.id).unwrap();

    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert_eq!(pulled.server_conflicts.len(), 1);
    assert!(library_a.note(&note.id).unwrap().trashed);
    let conflict = pulled.server_conflicts[0].clone();
    assert_eq!(
        engine_a.resolve_keep_local(&library_a, &conflict).unwrap(),
        PushOutcome::Applied { version: 3 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.trashed.contains(&note.id));
    assert!(library_b.note(&note.id).unwrap().trashed);
}

#[test]
fn reopen_after_successful_push_finds_unchanged_not_a_duplicate_version() {
    let temporary = tempfile::tempdir().unwrap();
    let library_root = temporary.path().join("libraries");
    std::fs::create_dir_all(&library_root).unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(&library_root);
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let state_a = temporary.path().join("state-a.sqlite3");
    let state_b = temporary.path().join("state-b.sqlite3");
    let (mut engine_a, mut engine_b) = setup_engines_at(
        &base_url,
        &workspace_id,
        &owner,
        &phone,
        Some(&state_a),
        Some(&state_b),
    );
    let note = create_note(&mut library_a, "once", "body");
    assert_eq!(
        engine_a.push_note(&library_a, &note.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    drop(engine_a);
    let mut engine_a = reopen_engine(
        &base_url,
        &workspace_id,
        &owner.device_id,
        OWNER_SEED,
        &state_a,
    );
    assert_eq!(
        engine_a.push_note(&library_a, &note.id).unwrap(),
        PushOutcome::Unchanged { version: 1 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert_eq!(pulled.applied, vec![note.id]);
}

#[test]
fn keep_remote_and_save_as_new_are_available() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "base", "base body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    let a_edit = edit_note(&mut library_a, &note, "A edit", "body A");
    let b_note = library_b.note(&note.id).unwrap();
    let b_edit = edit_note(&mut library_b, &b_note, "B edit", "body B");
    engine_b.push_note(&library_b, &b_edit.id).unwrap();
    engine_a.push_note(&library_a, &a_edit.id).unwrap();
    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = pulled.server_conflicts[0].clone();

    engine_a
        .resolve_keep_remote(&mut library_a, &conflict)
        .unwrap();
    assert_eq!(library_a.note(&note.id).unwrap().title, "B edit");

    let a_current = library_a.note(&note.id).unwrap();
    let a_second = edit_note(&mut library_a, &a_current, "A second", "second A");
    let b_current = library_b.note(&note.id).unwrap();
    let b_second = edit_note(&mut library_b, &b_current, "B second", "second B");
    engine_b.push_note(&library_b, &b_second.id).unwrap();
    engine_a.push_note(&library_a, &a_second.id).unwrap();
    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = pulled.server_conflicts[0].clone();
    let new_id = engine_a
        .resolve_save_as_new(&mut library_a, &conflict)
        .unwrap();
    assert_ne!(new_id, note.id);
    assert_eq!(library_a.note(&note.id).unwrap().title, "B second");
    assert_eq!(
        library_a.note(&note.id).unwrap().document.plain_text(),
        "second B"
    );
    assert_eq!(library_a.note(&new_id).unwrap().title, "A second");
    assert_eq!(
        library_a.note(&new_id).unwrap().document.plain_text(),
        "second A"
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.applied.contains(&new_id));
    assert_eq!(library_b.note(&new_id).unwrap().title, "A second");
}

#[test]
fn workspace_key_rotation_keeps_old_ops_readable_and_uses_the_new_epoch() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let owner_encryption = DeviceKeyPair::from_seed(OWNER_SEED)
        .unwrap()
        .encryption_public_key()
        .unwrap();
    let phone_encryption = DeviceKeyPair::from_seed(PHONE_SEED)
        .unwrap()
        .encryption_public_key()
        .unwrap();

    let key1 = WorkspaceKey::from_bytes([19u8; 32]);
    let owner_envelope1 = wrap_workspace_key(
        owner_encryption,
        &key1,
        &workspace_key_aad(&workspace_id, 1, &owner.device_id),
    )
    .unwrap();
    let phone_envelope1 = wrap_workspace_key(
        phone_encryption,
        &key1,
        &workspace_key_aad(&workspace_id, 1, &phone.device_id),
    )
    .unwrap();
    let desktop = SyncClient::new(
        &base_url,
        &owner.device_id,
        DeviceKeyPair::from_seed(OWNER_SEED).unwrap(),
    )
    .unwrap();
    desktop
        .register_encryption_key(&encryption_public_key_hex(&owner_encryption))
        .unwrap();
    SyncClient::new(
        &base_url,
        &phone.device_id,
        DeviceKeyPair::from_seed(PHONE_SEED).unwrap(),
    )
    .unwrap()
    .register_encryption_key(&encryption_public_key_hex(&phone_encryption))
    .unwrap();
    desktop
        .rotate_workspace_key(
            &workspace_id,
            1,
            &[
                (owner.device_id.clone(), owner_envelope1),
                (phone.device_id.clone(), phone_envelope1),
            ],
        )
        .unwrap();
    let state_a = temporary.path().join("rotation-state-a.sqlite3");
    let mut engine_a1 = SyncEngine::with_state_and_keys(
        SyncClient::new(
            &base_url,
            &owner.device_id,
            DeviceKeyPair::from_seed(OWNER_SEED).unwrap(),
        )
        .unwrap(),
        HashMap::from([(1, WorkspaceKey::from_bytes([19u8; 32]))]),
        &workspace_id,
        1,
        &state_a,
    )
    .unwrap();
    let note = create_note(&mut library_a, "epoch one", "old body");
    engine_a1.push_note(&library_a, &note.id).unwrap();

    let key2 = WorkspaceKey::from_bytes([29u8; 32]);
    let owner_envelope2 = wrap_workspace_key(
        owner_encryption,
        &key2,
        &workspace_key_aad(&workspace_id, 2, &owner.device_id),
    )
    .unwrap();
    let phone_envelope2 = wrap_workspace_key(
        phone_encryption,
        &key2,
        &workspace_key_aad(&workspace_id, 2, &phone.device_id),
    )
    .unwrap();
    desktop
        .rotate_workspace_key(
            &workspace_id,
            2,
            &[
                (owner.device_id.clone(), owner_envelope2),
                (phone.device_id.clone(), phone_envelope2),
            ],
        )
        .unwrap();

    let keys = || {
        HashMap::from([
            (1, WorkspaceKey::from_bytes([19u8; 32])),
            (2, WorkspaceKey::from_bytes([29u8; 32])),
        ])
    };
    drop(engine_a1);
    let mut engine_a2 = SyncEngine::with_state_and_keys(
        SyncClient::new(
            &base_url,
            &owner.device_id,
            DeviceKeyPair::from_seed(OWNER_SEED).unwrap(),
        )
        .unwrap(),
        keys(),
        &workspace_id,
        2,
        &state_a,
    )
    .unwrap();
    let a_note = library_a.note(&note.id).unwrap();
    let a_edit = edit_note(&mut library_a, &a_note, "epoch two", "new body");
    assert_eq!(
        engine_a2.push_note(&library_a, &a_edit.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );
    let mut engine_b = SyncEngine::with_keys(
        SyncClient::new(
            &base_url,
            &phone.device_id,
            DeviceKeyPair::from_seed(PHONE_SEED).unwrap(),
        )
        .unwrap(),
        keys(),
        &workspace_id,
        2,
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.applied.contains(&note.id));
    assert_eq!(library_b.note(&note.id).unwrap().title, "epoch two");
}

#[test]
fn delete_and_restore_propagate_between_libraries() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "delete me", "body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    library_a.set_trashed(&note.id, true).unwrap();
    engine_a.push_delete(&library_a, &note.id).unwrap();
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.trashed.contains(&note.id));
    assert!(library_b.note(&note.id).unwrap().trashed);

    library_a.set_trashed(&note.id, false).unwrap();
    engine_a.push_restore(&library_a, &note.id).unwrap();
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.restored.contains(&note.id));
    assert!(!library_b.note(&note.id).unwrap().trashed);
}
fn reopen_engine(
    base_url: &str,
    workspace_id: &str,
    device_id: &str,
    seed: [u8; 32],
    state_path: &Path,
) -> SyncEngine {
    let client =
        SyncClient::new(base_url, device_id, DeviceKeyPair::from_seed(seed).unwrap()).unwrap();
    let envelope = client.fetch_key_envelope(workspace_id, 1).unwrap();
    let aad = workspace_key_aad(workspace_id, 1, device_id);
    let workspace_key = DeviceKeyPair::from_seed(seed)
        .unwrap()
        .unwrap_workspace_key(&envelope, &aad)
        .unwrap();
    SyncEngine::with_state(client, workspace_key, workspace_id, 1, state_path).unwrap()
}

#[test]
fn state_store_resumes_cursor_and_baseline_after_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let library_root = temporary.path().join("libraries");
    std::fs::create_dir_all(&library_root).unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(&library_root);
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let state_a = temporary.path().join("state-a.sqlite3");
    let state_b = temporary.path().join("state-b.sqlite3");
    let (mut engine_a, mut engine_b) = setup_engines_at(
        &base_url,
        &workspace_id,
        &owner,
        &phone,
        Some(&state_a),
        Some(&state_b),
    );

    let note = create_note(&mut library_a, "persist", "v1");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    drop(engine_a);

    let mut engine_a = reopen_engine(
        &base_url,
        &workspace_id,
        &owner.device_id,
        OWNER_SEED,
        &state_a,
    );
    assert_eq!(engine_a.synced_version(&note.id), Some(1));

    let phone_note = library_b.note(&note.id).unwrap();
    let phone_edit = edit_note(&mut library_b, &phone_note, "phone v2", "v2");
    engine_b.push_note(&library_b, &phone_edit.id).unwrap();
    let pulled = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert!(pulled.applied.contains(&note.id));
    assert_eq!(library_a.note(&note.id).unwrap().title, "phone v2");
}
#[test]
fn view_and_canvas_configuration_sync_without_silently_overwriting_either_side() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);

    let note = create_note(&mut library_a, "Shared note", "body");
    assert!(matches!(
        engine_a.push_note(&library_a, &note.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    ));
    assert!(
        engine_b
            .pull_and_apply(&mut library_b, 50)
            .unwrap()
            .applied
            .contains(&note.id)
    );

    let view = library_a
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "Research board".into(),
                filter_schema: 1,
                filter: NotesFilter::default(),
                sort: NoteSort::UpdatedDesc,
                layout: ViewLayout::Board,
                config_schema: 1,
                config: ViewConfig {
                    visible_properties: vec![
                        ViewProperty::InboxState,
                        ViewProperty::Tags,
                        ViewProperty::UpdatedAt,
                    ],
                    board_group_by: BoardGroupBy::InboxState,
                    calendar_month: None,
                    ..Default::default()
                },
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .view;
    assert_eq!(
        engine_a.push_view(&library_a, &view.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.applied.contains(&view.id));
    let observed = library_b.view_get(&view.id).unwrap();
    assert_eq!(observed.layout, ViewLayout::Board);
    assert_eq!(observed.config.board_group_by, BoardGroupBy::InboxState);

    let canvas = library_a
        .canvas_save(
            &Uuid::new_v4().to_string(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "Research canvas".into(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: note.id.clone(),
                    x: 50.0,
                    y: 60.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    assert_eq!(
        engine_a.push_canvas(&library_a, &canvas.id).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(pulled.applied.contains(&canvas.id));
    let remote_canvas = library_b.canvas_get(&canvas.id).unwrap();
    assert_eq!(remote_canvas.canvas.nodes.len(), 1);
    assert_eq!(remote_canvas.notes[0].note.id, note.id);

    // B edits the view first. A edits from the same base, so its push must
    // conflict and a pull must preserve A's local change for explicit review.
    let edit_b = library_b.view_get(&view.id).unwrap();
    library_b
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: Some(edit_b.id.clone()),
                base_row_version: Some(edit_b.row_version),
                name: "Research board B".into(),
                filter_schema: edit_b.filter_schema,
                filter: edit_b.filter.clone(),
                sort: edit_b.sort,
                layout: edit_b.layout,
                config_schema: edit_b.config_schema,
                config: edit_b.config.clone(),
                is_pinned: edit_b.is_pinned,
                pin_position: edit_b.pin_position,
            },
        )
        .unwrap();
    assert!(matches!(
        engine_b.push_view(&library_b, &view.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    ));

    let edit_a = library_a.view_get(&view.id).unwrap();
    library_a
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: Some(edit_a.id.clone()),
                base_row_version: Some(edit_a.row_version),
                name: "Research board A".into(),
                filter_schema: edit_a.filter_schema,
                filter: edit_a.filter.clone(),
                sort: edit_a.sort,
                layout: edit_a.layout,
                config_schema: edit_a.config_schema,
                config: edit_a.config.clone(),
                is_pinned: edit_a.is_pinned,
                pin_position: edit_a.pin_position,
            },
        )
        .unwrap();
    assert!(matches!(
        engine_a.push_view(&library_a, &view.id).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflicted = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = conflicted
        .config_conflicts
        .iter()
        .find(|conflict| conflict.object_id == view.id)
        .cloned()
        .expect("view conflict");
    assert_eq!(conflict.remote_version, 2);
    assert_eq!(
        library_a.view_get(&view.id).unwrap().name,
        "Research board A"
    );
    assert_eq!(
        engine_a
            .keep_remote_config(&mut library_a, &view.id)
            .unwrap(),
        2
    );
    assert_eq!(
        library_a.view_get(&view.id).unwrap().name,
        "Research board B"
    );

    let edit_b = library_b.view_get(&view.id).unwrap();
    library_b
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: Some(edit_b.id.clone()),
                base_row_version: Some(edit_b.row_version),
                name: "Research board B2".into(),
                filter_schema: edit_b.filter_schema,
                filter: edit_b.filter.clone(),
                sort: edit_b.sort,
                layout: edit_b.layout,
                config_schema: edit_b.config_schema,
                config: edit_b.config.clone(),
                is_pinned: edit_b.is_pinned,
                pin_position: edit_b.pin_position,
            },
        )
        .unwrap();
    assert_eq!(
        engine_b.push_view(&library_b, &view.id).unwrap(),
        PushOutcome::Applied { version: 3 }
    );

    let edit_a = library_a.view_get(&view.id).unwrap();
    library_a
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: Some(edit_a.id.clone()),
                base_row_version: Some(edit_a.row_version),
                name: "Research board A2".into(),
                filter_schema: edit_a.filter_schema,
                filter: edit_a.filter.clone(),
                sort: edit_a.sort,
                layout: edit_a.layout,
                config_schema: edit_a.config_schema,
                config: edit_a.config.clone(),
                is_pinned: edit_a.is_pinned,
                pin_position: edit_a.pin_position,
            },
        )
        .unwrap();
    assert!(matches!(
        engine_a.push_view(&library_a, &view.id).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflicted = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = conflicted
        .config_conflicts
        .iter()
        .find(|conflict| conflict.object_id == view.id)
        .cloned()
        .expect("second view conflict");
    assert_eq!(conflict.remote_version, 3);
    assert!(
        engine_a
            .pending_config_conflicts()
            .iter()
            .any(|item| item.object_id == conflict.object_id)
    );
    assert_eq!(
        engine_a
            .keep_local_config(&library_a, &conflict.object_id, conflict.remote_version,)
            .unwrap(),
        PushOutcome::Applied { version: 4 }
    );
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert_eq!(
        library_b.view_get(&view.id).unwrap().name,
        "Research board A2"
    );
}
#[test]
fn workspace_metadata_sync_preserves_both_sides_on_conflict() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);
    let registry = |value: &str| {
        serde_json::json!({
            "schema": 1,
            "workspaces": { "workspace-a": [{ "id": value }] }
        })
        .to_string()
    };

    let initial_a = library_a.workspace_metadata().unwrap();
    let saved_a = library_a
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: initial_a.row_version,
            object_type_registry: registry("project"),
            note_template_registry: registry("brief"),
        })
        .unwrap();
    assert_eq!(
        engine_a.push_workspace_metadata(&library_a).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(
        !pulled
            .config_conflicts
            .iter()
            .any(|conflict| conflict.kind == "workspace_metadata")
    );
    assert_eq!(library_b.workspace_metadata().unwrap(), saved_a);

    let base_b = library_b.workspace_metadata().unwrap();
    let saved_b = library_b
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: base_b.row_version,
            object_type_registry: registry("phone-project"),
            note_template_registry: base_b.note_template_registry.clone(),
        })
        .unwrap();
    assert_eq!(
        engine_b.push_workspace_metadata(&library_b).unwrap(),
        PushOutcome::Applied { version: 2 }
    );

    let base_a = library_a.workspace_metadata().unwrap();
    let saved_a_second = library_a
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: base_a.row_version,
            object_type_registry: registry("desktop-project"),
            note_template_registry: base_a.note_template_registry.clone(),
        })
        .unwrap();
    assert!(matches!(
        engine_a.push_workspace_metadata(&library_a).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflicted = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = conflicted
        .config_conflicts
        .iter()
        .find(|conflict| conflict.kind == "workspace_metadata")
        .cloned()
        .expect("workspace metadata conflict");
    assert_eq!(conflict.remote_version, 2);
    assert!(
        engine_a
            .pending_config_conflicts()
            .iter()
            .any(|item| item.object_id == conflict.object_id)
    );
    assert!(conflict.local_available);
    assert!(conflict.remote_available);
    assert_eq!(library_a.workspace_metadata().unwrap(), saved_a_second);
    assert_ne!(saved_a_second, saved_b);

    assert_eq!(
        engine_a
            .keep_remote_config(&mut library_a, &conflict.object_id)
            .unwrap(),
        2
    );
    assert_eq!(library_b.workspace_metadata().unwrap(), saved_b);
    assert_eq!(library_a.workspace_metadata().unwrap(), saved_b);
    assert!(engine_a.pending_config_conflicts().is_empty());

    let base_b = library_b.workspace_metadata().unwrap();
    let saved_b_second = library_b
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: base_b.row_version,
            object_type_registry: registry("phone-project-2"),
            note_template_registry: base_b.note_template_registry.clone(),
        })
        .unwrap();
    assert_eq!(
        engine_b.push_workspace_metadata(&library_b).unwrap(),
        PushOutcome::Applied { version: 3 }
    );

    let base_a = library_a.workspace_metadata().unwrap();
    let saved_a_third = library_a
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: base_a.row_version,
            object_type_registry: registry("desktop-project-2"),
            note_template_registry: base_a.note_template_registry.clone(),
        })
        .unwrap();
    assert!(matches!(
        engine_a.push_workspace_metadata(&library_a).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflicted = engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    let conflict = conflicted
        .config_conflicts
        .iter()
        .find(|conflict| conflict.kind == "workspace_metadata")
        .cloned()
        .expect("second workspace metadata conflict");
    assert_eq!(conflict.remote_version, 3);
    assert!(
        engine_a
            .pending_config_conflicts()
            .iter()
            .any(|item| item.object_id == conflict.object_id)
    );
    assert_eq!(
        engine_a
            .keep_local_config(&library_a, &conflict.object_id, conflict.remote_version,)
            .unwrap(),
        PushOutcome::Applied { version: 4 }
    );
    let pulled_b = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(
        !pulled_b
            .config_conflicts
            .iter()
            .any(|conflict| conflict.kind == "workspace_metadata")
    );
    assert_eq!(library_b.workspace_metadata().unwrap(), saved_a_third);
    assert_ne!(saved_a_third, saved_b_second);
    assert!(engine_a.pending_config_conflicts().is_empty());
}
#[test]
fn pending_workspace_metadata_conflict_survives_engine_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let library_root = temporary.path().join("libraries");
    std::fs::create_dir_all(&library_root).unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(&library_root);
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let state_a = temporary.path().join("metadata-conflict-a.sqlite3");
    let state_b = temporary.path().join("metadata-conflict-b.sqlite3");
    let (mut engine_a, mut engine_b) = setup_engines_at(
        &base_url,
        &workspace_id,
        &owner,
        &phone,
        Some(&state_a),
        Some(&state_b),
    );
    let registry = |value: &str| {
        serde_json::json!({
            "schema": 1,
            "workspaces": { "workspace-a": [{ "id": value }] }
        })
        .to_string()
    };

    let initial_a = library_a.workspace_metadata().unwrap();
    library_a
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: initial_a.row_version,
            object_type_registry: registry("base"),
            note_template_registry: registry("base-template"),
        })
        .unwrap();
    assert_eq!(
        engine_a.push_workspace_metadata(&library_a).unwrap(),
        PushOutcome::Applied { version: 1 }
    );
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    let owner_base = library_a.workspace_metadata().unwrap();
    library_a
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: owner_base.row_version,
            object_type_registry: registry("owner"),
            note_template_registry: owner_base.note_template_registry.clone(),
        })
        .unwrap();
    assert_eq!(
        engine_a.push_workspace_metadata(&library_a).unwrap(),
        PushOutcome::Applied { version: 2 }
    );

    let phone_base = library_b.workspace_metadata().unwrap();
    library_b
        .update_workspace_metadata(UpdateWorkspaceMetadata {
            base_row_version: phone_base.row_version,
            object_type_registry: registry("phone"),
            note_template_registry: phone_base.note_template_registry.clone(),
        })
        .unwrap();
    assert!(matches!(
        engine_b.push_workspace_metadata(&library_b).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let pulled = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    let conflict = pulled
        .config_conflicts
        .iter()
        .find(|item| item.kind == "workspace_metadata")
        .cloned()
        .expect("metadata conflict");
    assert!(
        engine_b
            .pending_config_conflicts()
            .iter()
            .any(|item| item.object_id == conflict.object_id)
    );
    drop(engine_b);

    let mut engine_b = reopen_engine(
        &base_url,
        &workspace_id,
        &phone.device_id,
        PHONE_SEED,
        &state_b,
    );
    let pending = engine_b.pending_config_conflicts();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].remote_version, 2);
    let repeated = engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    assert!(
        repeated
            .config_conflicts
            .iter()
            .any(|item| item.object_id == conflict.object_id && item.remote_version == 2)
    );
    assert_eq!(
        engine_b
            .keep_local_config(&library_b, &conflict.object_id, conflict.remote_version,)
            .unwrap(),
        PushOutcome::Applied { version: 3 }
    );
    assert!(engine_b.pending_config_conflicts().is_empty());
    drop(engine_b);
    let engine_b = reopen_engine(
        &base_url,
        &workspace_id,
        &phone.device_id,
        PHONE_SEED,
        &state_b,
    );
    assert!(engine_b.pending_config_conflicts().is_empty());
}
#[test]
fn view_and_canvas_deletions_reach_the_other_device_through_config_tombstones() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut library_a, mut library_b, workspace_id) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base_url = start_server(authority);
    let (mut engine_a, mut engine_b) = setup_engines(&base_url, &workspace_id, &owner, &phone);
    let note = create_note(&mut library_a, "Deletion note", "body");
    engine_a.push_note(&library_a, &note.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    let view = library_a
        .view_save(
            &Uuid::new_v4().to_string(),
            SaveView {
                view_id: None,
                base_row_version: None,
                name: "Delete me".into(),
                filter_schema: 1,
                filter: NotesFilter::default(),
                sort: NoteSort::UpdatedDesc,
                layout: ViewLayout::Timeline,
                config_schema: 1,
                config: ViewConfig::default(),
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .view;
    engine_a.push_view(&library_a, &view.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();
    let canvas = library_a
        .canvas_save(
            &Uuid::new_v4().to_string(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "Delete canvas".into(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: note.id,
                    x: 40.0,
                    y: 40.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;
    engine_a.push_canvas(&library_a, &canvas.id).unwrap();
    engine_b.pull_and_apply(&mut library_b, 50).unwrap();

    let remote_view = library_b.view_get(&view.id).unwrap();
    library_b
        .view_remove(
            &Uuid::new_v4().to_string(),
            pkms_core::RemoveView {
                view_id: remote_view.id.clone(),
                base_row_version: remote_view.row_version,
            },
        )
        .unwrap();
    let remote_canvas = library_b.canvas(&canvas.id).unwrap();
    library_b
        .canvas_remove(
            &Uuid::new_v4().to_string(),
            pkms_core::RemoveCanvas {
                canvas_id: remote_canvas.id.clone(),
                base_row_version: remote_canvas.row_version,
            },
        )
        .unwrap();
    let tombstones = library_b.list_config_tombstones().unwrap();
    assert!(
        tombstones
            .iter()
            .any(|item| item.object_id == view.id && item.kind == "view")
    );
    assert!(
        tombstones
            .iter()
            .any(|item| item.object_id == canvas.id && item.kind == "canvas")
    );
    assert!(matches!(
        engine_b.push_view_remove(&view.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    ));
    assert!(matches!(
        engine_b.push_canvas_remove(&canvas.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    ));
    engine_a.pull_and_apply(&mut library_a, 50).unwrap();
    assert!(matches!(
        library_a.view_get(&view.id),
        Err(pkms_core::Error::NotFound)
    ));
    assert!(matches!(
        library_a.canvas(&canvas.id),
        Err(pkms_core::Error::NotFound)
    ));
}

#[test]
fn retry_review_accepted_write_rejected_retry_keeps_receipt_until_permission_recovers() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("phone-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
    let view = retry_view(&mut b, "first accepted value");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(eb.push_view(&b, &view.id).is_err());
    assert_eq!(ea.client().snapshot(&workspace).unwrap()[0].version, 1);
    retry_rename_view(&mut b, &view, "edited while the first receipt was unknown");
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Viewer)
        .unwrap();
    assert!(eb.push_view(&b, &view.id).is_err());
    drop(eb);
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Editor)
        .unwrap();
    let mut eb = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
    assert_eq!(
        eb.push_view(&b, &view.id).unwrap(),
        PushOutcome::Applied { version: 2 },
        "a rejected replay cannot erase the earlier accepted request and manufacture a self-conflict",
    );
    ea.pull_and_apply(&mut a, 50).unwrap();
    assert_eq!(
        a.view_get(&view.id).unwrap().name,
        "edited while the first receipt was unknown"
    );
    assert!(
        eb.client()
            .pull(&workspace, 0, 50)
            .unwrap()
            .conflicts
            .is_empty()
    );
}

#[test]
fn retry_review_keep_local_configuration_restores_a_remote_tombstone() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let view = retry_view(&mut a, "shared view");
    ea.push_view(&a, &view.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    let local = b.view_get(&view.id).unwrap();
    retry_rename_view(&mut b, &local, "preserve my local configuration");
    a.view_remove(
        &Uuid::new_v4().to_string(),
        pkms_core::RemoveView {
            view_id: view.id.clone(),
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    ea.push_view_remove(&view.id).unwrap();
    let pulled = eb.pull_and_apply(&mut b, 50).unwrap();
    let conflict = pulled
        .config_conflicts
        .iter()
        .find(|c| c.object_id == view.id)
        .unwrap();
    assert!(!conflict.remote_available && conflict.local_available);
    assert_eq!(
        eb.keep_local_config(&b, &view.id, conflict.remote_version)
            .unwrap(),
        PushOutcome::Applied { version: 3 },
        "keeping a local configuration over a tombstone must submit Restore, not Put",
    );
    ea.pull_and_apply(&mut a, 50).unwrap();
    assert_eq!(
        a.view_get(&view.id).unwrap().name,
        "preserve my local configuration"
    );
}

#[test]
fn retry_review_note_restoration_baseline_accepts_the_next_remote_edit() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let note = create_note(&mut a, "restorable note", "initial");
    ea.push_note(&a, &note.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    a.set_trashed(&note.id, true).unwrap();
    ea.push_delete(&a, &note.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    a.set_trashed(&note.id, false).unwrap();
    ea.push_restore(&a, &note.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    assert!(!b.note(&note.id).unwrap().trashed);
    let current = a.note(&note.id).unwrap();
    edit_note(
        &mut a,
        &current,
        "next remote edit",
        "must apply without a fake conflict",
    );
    ea.push_note(&a, &note.id).unwrap();
    let pulled = eb.pull_and_apply(&mut b, 50).unwrap();
    assert!(pulled.conflicts.is_empty());
    assert_eq!(b.note(&note.id).unwrap().title, "next remote edit");
}

struct ForkFixture {
    local: Library,
    workspace: String,
    base: String,
    phone: DeviceRecord,
    fault: ReceiptFault,
    engine: SyncEngine,
    conflict: pkms_sync::ConflictRecord,
    source: Note,
    state: std::path::PathBuf,
}

impl ForkFixture {
    fn reopen(&mut self) {
        self.engine = reopen_engine(
            &self.base,
            &self.workspace,
            &self.phone.device_id,
            PHONE_SEED,
            &self.state,
        );
    }
    fn resolve(&mut self) -> Result<String, pkms_sync_engine::EngineError> {
        self.engine
            .resolve_save_as_new(&mut self.local, &self.conflict)
    }
    fn local_forks(&self) -> Vec<Note> {
        self.local
            .list_notes()
            .unwrap()
            .into_iter()
            .filter(|note| note.id != self.source.id)
            .collect()
    }
    fn remote_forks(&self) -> Vec<pkms_sync::ObjectState> {
        self.engine
            .client()
            .snapshot(&self.workspace)
            .unwrap()
            .into_iter()
            .filter(|object| object.object_id != self.source.id)
            .collect()
    }
}

fn fork_fixture(root: &Path, remote_deleted: bool) -> ForkFixture {
    let (mut a, mut b, workspace) = workspace_libraries(root);
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = root.join("fork-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
    let original = create_note(&mut a, "original source", "baseline");
    ea.push_note(&a, &original.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    if remote_deleted {
        a.set_trashed(&original.id, true).unwrap();
        ea.push_delete(&a, &original.id).unwrap();
    } else {
        edit_note(&mut a, &original, "remote source", "remote evidence");
        ea.push_note(&a, &original.id).unwrap();
    }
    let local = b.note(&original.id).unwrap();
    let source = edit_note(
        &mut b,
        &local,
        "local fork source",
        "captured local evidence",
    );
    assert!(matches!(
        eb.push_note(&b, &source.id).unwrap(),
        PushOutcome::Conflict { .. }
    ));
    let conflict = eb.client().pull(&workspace, 0, 50).unwrap().conflicts[0].clone();
    ForkFixture {
        local: b,
        workspace,
        base,
        phone,
        fault,
        engine: eb,
        conflict,
        source,
        state,
    }
}

#[test]
fn retry_fork_unknown_publish_and_completed_replay_create_one_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let mut f = fork_fixture(temporary.path(), false);
    f.fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(f.resolve().is_err());
    let copy = f.local_forks().remove(0);
    assert_eq!(f.remote_forks().len(), 1);
    f.reopen();
    let id = f.resolve().unwrap();
    assert_eq!(id, copy.id);
    assert_eq!(f.local.note(&id).unwrap().document, copy.document);
    assert_eq!(f.local_forks().len(), 1);
    assert_eq!(f.remote_forks().len(), 1);
    assert_eq!(f.local.note(&f.source.id).unwrap().title, "remote source");
    let revised = f.local.note(&f.source.id).unwrap();
    edit_note(
        &mut f.local,
        &revised,
        "user edited after completion",
        "retain this newer original",
    );
    f.reopen();
    assert_eq!(f.resolve().unwrap(), id);
    assert_eq!(
        f.local.note(&f.source.id).unwrap().title,
        "user edited after completion"
    );
    assert_eq!(f.local_forks().len(), 1);
    let ops = f.fault.submitted.lock().unwrap();
    let copy_ops: Vec<_> = ops.iter().filter(|op| op.object_id == id).collect();
    assert_eq!(copy_ops.len(), 2);
    assert_eq!(copy_ops[0], copy_ops[1]);
}

#[test]
fn retry_fork_core_and_state_boundaries_reuse_the_same_copy_and_command() {
    for boundary in ["planned", "copied", "published", "adopted", "completed"] {
        let temporary = tempfile::tempdir().unwrap();
        let mut f = fork_fixture(temporary.path(), false);
        let db = rusqlite::Connection::open(&f.state).unwrap();
        let condition = match boundary {
            "planned" => "1",
            "copied" => "json_extract(NEW.plan_json,'$.fork_id') IS NOT NULL",
            "published" => "json_extract(NEW.plan_json,'$.published_version') IS NOT NULL",
            "adopted" => "json_extract(NEW.plan_json,'$.adoption.applied') IS NOT NULL",
            "completed" => "json_extract(NEW.plan_json,'$.completed')=1",
            _ => unreachable!(),
        };
        db.execute_batch(&format!("CREATE TRIGGER reject_fork BEFORE INSERT ON sync_fork_plans WHEN {condition} BEGIN SELECT RAISE(ABORT,'injected fork boundary'); END;")).unwrap();
        assert!(f.resolve().is_err(), "boundary {boundary}");
        let prior_copies = f.local_forks();
        if boundary == "planned" {
            assert!(prior_copies.is_empty());
        } else {
            assert_eq!(prior_copies.len(), 1);
        }
        db.execute_batch("DROP TRIGGER reject_fork;").unwrap();
        if boundary == "adopted" || boundary == "completed" {
            let current = f.local.note(&f.source.id).unwrap();
            edit_note(
                &mut f.local,
                &current,
                "edit after Core committed",
                "must not be overwritten by replay",
            );
        }
        f.reopen();
        let id = f.resolve().unwrap();
        if let Some(prior) = prior_copies.first() {
            assert_eq!(id, prior.id);
            assert_eq!(f.local.note(&id).unwrap().document, prior.document);
        }
        assert_eq!(f.local_forks().len(), 1, "boundary {boundary}");
        assert_eq!(f.remote_forks().len(), 1, "boundary {boundary}");
        if boundary == "adopted" || boundary == "completed" {
            assert_eq!(
                f.local.note(&f.source.id).unwrap().title,
                "edit after Core committed"
            );
        }
        assert!(
            f.engine
                .client()
                .pull(&f.workspace, 0, 50)
                .unwrap()
                .conflicts
                .is_empty()
        );
        let plan_json: String = db
            .query_row("SELECT plan_json FROM sync_fork_plans", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(
            !plan_json.contains("captured local evidence")
                && !plan_json.contains("remote evidence")
        );
    }
}

#[test]
fn retry_fork_unknown_confirmation_does_not_repeat_original_adoption() {
    let temporary = tempfile::tempdir().unwrap();
    let mut f = fork_fixture(temporary.path(), false);
    f.fault
        .lose_resolution
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(f.resolve().is_err());
    let current = f.local.note(&f.source.id).unwrap();
    assert_eq!(current.title, "remote source");
    let latest = edit_note(
        &mut f.local,
        &current,
        "new edit after resolution",
        "preserve later human text",
    );
    f.reopen();
    f.resolve().unwrap();
    assert_eq!(
        f.local.note(&f.source.id).unwrap().revision_id,
        latest.revision_id
    );
    assert_eq!(f.local_forks().len(), 1);
    assert_eq!(f.remote_forks().len(), 1);
}

#[test]
fn retry_fork_new_source_edit_conflicts_without_overwrite_and_can_keep_latest_local() {
    let temporary = tempfile::tempdir().unwrap();
    let mut f = fork_fixture(temporary.path(), false);
    f.fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(f.resolve().is_err());
    let current = f.local.note(&f.source.id).unwrap();
    let latest = edit_note(
        &mut f.local,
        &current,
        "new edit while publication unknown",
        "new text must survive",
    );
    f.reopen();
    assert!(matches!(
        f.resolve(),
        Err(pkms_sync_engine::EngineError::Core(
            pkms_core::Error::RevisionConflict { .. }
        ))
    ));
    assert_eq!(
        f.local.note(&f.source.id).unwrap().revision_id,
        latest.revision_id
    );
    assert_eq!(f.local_forks().len(), 1);
    assert_eq!(f.remote_forks().len(), 1);
    assert_eq!(
        f.engine
            .client()
            .pull(&f.workspace, 0, 50)
            .unwrap()
            .conflicts
            .len(),
        1
    );
    assert!(matches!(
        f.engine.resolve_keep_local(&f.local, &f.conflict).unwrap(),
        PushOutcome::Applied { .. }
    ));
    assert_eq!(
        f.local.note(&f.source.id).unwrap().revision_id,
        latest.revision_id
    );
    assert!(matches!(
        f.resolve(),
        Err(pkms_sync_engine::EngineError::ForkSuperseded)
    ));
    assert_eq!(f.remote_forks().len(), 1);
}

#[test]
fn retry_fork_tombstone_adoption_replays_metadata_receipt_without_retrashing_later_restore() {
    let temporary = tempfile::tempdir().unwrap();
    let mut f = fork_fixture(temporary.path(), true);
    let db = rusqlite::Connection::open(&f.state).unwrap();
    db.execute_batch("CREATE TRIGGER reject_fork_adopt BEFORE INSERT ON sync_fork_plans WHEN json_extract(NEW.plan_json,'$.adoption.applied') IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected after metadata commit'); END;").unwrap();
    assert!(f.resolve().is_err());
    assert!(f.local.note(&f.source.id).unwrap().trashed);
    f.local.set_trashed(&f.source.id, false).unwrap();
    db.execute_batch("DROP TRIGGER reject_fork_adopt;").unwrap();
    f.reopen();
    f.resolve().unwrap();
    assert!(!f.local.note(&f.source.id).unwrap().trashed);
    assert_eq!(f.local_forks().len(), 1);
    assert_eq!(f.remote_forks().len(), 1);
}

#[test]
fn retry_review_viewer_pull_can_confirm_an_accepted_request_without_losing_later_edits() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let fault = ReceiptFault::default();
    let base = start_fault_server(authority, fault.clone());
    let state = temporary.path().join("viewer-state.sqlite3");
    let (mut ea, mut eb) = setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
    let view = retry_view(&mut b, "accepted before permission changed");
    fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(eb.push_view(&b, &view.id).is_err());
    retry_rename_view(&mut b, &view, "later local edit");
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Viewer)
        .unwrap();
    assert!(eb.push_view(&b, &view.id).is_err());
    let received = eb.pull_and_apply(&mut b, 50).unwrap();
    assert!(received.config_conflicts.is_empty());
    assert_eq!(b.view_get(&view.id).unwrap().name, "later local edit");
    assert_eq!(eb.synced_version(&view.id), Some(1));
    drop(eb);
    ea.client()
        .set_device_role(&phone.device_id, pkms_sync::MemberRole::Editor)
        .unwrap();
    let mut eb = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
    assert_eq!(
        eb.push_view(&b, &view.id).unwrap(),
        PushOutcome::Applied { version: 2 }
    );
    ea.pull_and_apply(&mut a, 50).unwrap();
    assert_eq!(a.view_get(&view.id).unwrap().name, "later local edit");
}

#[test]
fn retry_review_initial_seed_requires_explicit_completion_not_file_or_nonempty_maps() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, _, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let state = temporary.path().join("seed-state.sqlite3");
    let (mut ea, _) = setup_engines_at(&base, &workspace, &owner, &phone, Some(&state), None);
    assert!(state.exists());
    assert!(ea.needs_initial_seed());
    drop(ea);
    ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert!(ea.needs_initial_seed());
    let note = create_note(&mut a, "now seeded", "body");
    ea.push_note(&a, &note.id).unwrap();
    assert!(ea.needs_initial_seed());
    ea.seed_from_snapshot(&a).unwrap();
    assert!(ea.needs_initial_seed());
    ea.initialize_from_snapshot(&mut a).unwrap();
    assert!(!ea.needs_initial_seed());
    drop(ea);
    let ea = reopen_engine(&base, &workspace, &owner.device_id, OWNER_SEED, &state);
    assert!(!ea.needs_initial_seed());
}

#[test]
fn retry_review_checked_configuration_choices_reject_edits_after_preview() {
    for keep_local in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
        let (authority, owner, phone) = setup_authority();
        let base = start_server(authority);
        let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
        let view = retry_view(&mut a, "initial configuration");
        ea.push_view(&a, &view.id).unwrap();
        eb.pull_and_apply(&mut b, 50).unwrap();
        retry_rename_view(&mut a, &view, "remote preview");
        ea.push_view(&a, &view.id).unwrap();
        let local = b.view_get(&view.id).unwrap();
        retry_rename_view(&mut b, &local, "local preview");
        eb.pull_and_apply(&mut b, 50).unwrap();
        let preview = eb.local_config_state(&b, &view.id).unwrap();
        assert!(preview.available && !preview.tombstone);
        let local = b.view_get(&view.id).unwrap();
        retry_rename_view(&mut b, &local, "local edit after confirmation opened");
        if keep_local {
            assert!(matches!(
                eb.keep_local_config_checked(&b, &view.id, 2, &preview.local_state_token),
                Err(pkms_sync_engine::EngineError::ConfigPreviewChanged)
            ));
        } else {
            assert!(matches!(
                eb.keep_remote_config_checked(&mut b, &view.id, 2, &preview.local_state_token),
                Err(pkms_sync_engine::EngineError::ConfigPreviewChanged)
            ));
        }
        assert_eq!(
            b.view_get(&view.id).unwrap().name,
            "local edit after confirmation opened"
        );
        assert_eq!(ea.client().snapshot(&workspace).unwrap()[0].version, 2);
        let refreshed = eb.local_config_state(&b, &view.id).unwrap();
        assert_ne!(preview.local_state_token, refreshed.local_state_token);
        if keep_local {
            assert_eq!(
                eb.keep_local_config_checked(&b, &view.id, 2, &refreshed.local_state_token)
                    .unwrap(),
                PushOutcome::Applied { version: 3 }
            );
        } else {
            assert_eq!(
                eb.keep_remote_config_checked(&mut b, &view.id, 2, &refreshed.local_state_token)
                    .unwrap(),
                2
            );
            assert_eq!(b.view_get(&view.id).unwrap().name, "remote preview");
        }
    }
}

#[test]
fn retry_review_checked_remote_configuration_rejects_a_newer_remote_head() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
    let view = retry_view(&mut a, "initial view");
    ea.push_view(&a, &view.id).unwrap();
    eb.pull_and_apply(&mut b, 50).unwrap();
    retry_rename_view(&mut a, &view, "remote version two");
    ea.push_view(&a, &view.id).unwrap();
    let local = b.view_get(&view.id).unwrap();
    retry_rename_view(&mut b, &local, "local edit");
    eb.pull_and_apply(&mut b, 50).unwrap();
    let preview = eb.local_config_state(&b, &view.id).unwrap();
    let remote = a.view_get(&view.id).unwrap();
    retry_rename_view(&mut a, &remote, "remote version three");
    ea.push_view(&a, &view.id).unwrap();
    assert!(matches!(
        eb.keep_remote_config_checked(&mut b, &view.id, 2, &preview.local_state_token),
        Err(pkms_sync_engine::EngineError::ConfigConflictChanged)
    ));
    assert_eq!(b.view_get(&view.id).unwrap().name, "local edit");
    eb.pull_and_apply(&mut b, 50).unwrap();
    assert_eq!(
        eb.keep_remote_config_checked(&mut b, &view.id, 3, &preview.local_state_token)
            .unwrap(),
        3
    );
    assert_eq!(b.view_get(&view.id).unwrap().name, "remote version three");
}

#[test]
fn retry_review_deleted_configuration_choices_keep_their_real_state_and_close_conflicts() {
    for remote_deleted in [true, false] {
        let temporary = tempfile::tempdir().unwrap();
        let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
        let (authority, owner, phone) = setup_authority();
        let base = start_server(authority);
        let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
        let view = retry_view(&mut a, "deletion choice");
        ea.push_view(&a, &view.id).unwrap();
        eb.pull_and_apply(&mut b, 50).unwrap();
        if remote_deleted {
            let local = b.view_get(&view.id).unwrap();
            retry_rename_view(
                &mut b,
                &local,
                "keep my local edit until explicitly deleted",
            );
            a.view_remove(
                &Uuid::new_v4().to_string(),
                pkms_core::RemoveView {
                    view_id: view.id.clone(),
                    base_row_version: view.row_version,
                },
            )
            .unwrap();
            ea.push_view_remove(&view.id).unwrap();
            eb.pull_and_apply(&mut b, 50).unwrap();
            assert!(matches!(
                eb.push_view(&b, &view.id).unwrap(),
                PushOutcome::Conflict { .. }
            ));
            let preview = eb.local_config_state(&b, &view.id).unwrap();
            assert_eq!(
                eb.keep_remote_config_checked(&mut b, &view.id, 2, &preview.local_state_token)
                    .unwrap(),
                2
            );
            assert!(matches!(
                b.view_get(&view.id),
                Err(pkms_core::Error::NotFound)
            ));
        } else {
            retry_rename_view(&mut a, &view, "remote edit");
            ea.push_view(&a, &view.id).unwrap();
            let local = b.view_get(&view.id).unwrap();
            b.view_remove(
                &Uuid::new_v4().to_string(),
                pkms_core::RemoveView {
                    view_id: view.id.clone(),
                    base_row_version: local.row_version,
                },
            )
            .unwrap();
            eb.pull_and_apply(&mut b, 50).unwrap();
            assert!(matches!(
                eb.push_view_remove(&view.id).unwrap(),
                PushOutcome::Conflict { .. }
            ));
            let preview = eb.local_config_state(&b, &view.id).unwrap();
            assert!(!preview.available && preview.tombstone);
            assert_eq!(
                eb.keep_local_config_checked(&b, &view.id, 2, &preview.local_state_token)
                    .unwrap(),
                PushOutcome::Applied { version: 3 }
            );
            ea.pull_and_apply(&mut a, 50).unwrap();
            assert!(matches!(
                a.view_get(&view.id),
                Err(pkms_core::Error::NotFound)
            ));
        }
        assert!(
            eb.client()
                .pull(&workspace, 0, 50)
                .unwrap()
                .conflicts
                .is_empty()
        );
    }
}

#[test]
fn retry_review_tombstone_preview_token_changes_after_restore_and_delete_again() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut a, _, workspace) = workspace_libraries(temporary.path());
    let (authority, owner, phone) = setup_authority();
    let base = start_server(authority);
    let (mut ea, _) = setup_engines(&base, &workspace, &owner, &phone);
    let view = retry_view(&mut a, "deleted twice");
    ea.push_view(&a, &view.id).unwrap();
    a.view_remove(
        &Uuid::new_v4().to_string(),
        pkms_core::RemoveView {
            view_id: view.id.clone(),
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    let before = ea.local_config_state(&a, &view.id).unwrap();
    a.view_apply_remote(view.clone()).unwrap();
    a.view_remove(
        &Uuid::new_v4().to_string(),
        pkms_core::RemoveView {
            view_id: view.id.clone(),
            base_row_version: view.row_version,
        },
    )
    .unwrap();
    let after = ea.local_config_state(&a, &view.id).unwrap();
    assert!(before.tombstone && after.tombstone);
    assert_ne!(before.local_state_token, after.local_state_token);
    assert!(matches!(
        ea.keep_local_config_checked(&a, &view.id, 1, &before.local_state_token),
        Err(pkms_sync_engine::EngineError::ConfigPreviewChanged)
    ));
}

#[test]
fn retry_review_view_and_canvas_restore_replay_receipts_and_clear_server_conflicts() {
    for is_canvas in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
        let (authority, owner, phone) = setup_authority();
        let fault = ReceiptFault::default();
        let base = start_fault_server(authority, fault.clone());
        let state = temporary.path().join("restore-state.sqlite3");
        let (mut ea, mut eb) =
            setup_engines_at(&base, &workspace, &owner, &phone, None, Some(&state));
        let id = if is_canvas {
            let canvas = a
                .canvas_save(
                    &Uuid::new_v4().to_string(),
                    SaveCanvas {
                        canvas_id: None,
                        base_row_version: None,
                        name: "restorable canvas".into(),
                        config_schema: 1,
                        nodes: vec![],
                        is_pinned: false,
                        pin_position: None,
                    },
                )
                .unwrap()
                .canvas;
            ea.push_canvas(&a, &canvas.id).unwrap();
            eb.pull_and_apply(&mut b, 50).unwrap();
            let local = b.canvas(&canvas.id).unwrap();
            b.canvas_save(
                &Uuid::new_v4().to_string(),
                SaveCanvas {
                    canvas_id: Some(local.id.clone()),
                    base_row_version: Some(local.row_version),
                    name: "restored local canvas".into(),
                    config_schema: local.config_schema,
                    nodes: local.nodes,
                    is_pinned: local.is_pinned,
                    pin_position: local.pin_position,
                },
            )
            .unwrap();
            a.canvas_remove(
                &Uuid::new_v4().to_string(),
                pkms_core::RemoveCanvas {
                    canvas_id: canvas.id.clone(),
                    base_row_version: canvas.row_version,
                },
            )
            .unwrap();
            ea.push_canvas_remove(&canvas.id).unwrap();
            canvas.id
        } else {
            let view = retry_view(&mut a, "restorable view");
            ea.push_view(&a, &view.id).unwrap();
            eb.pull_and_apply(&mut b, 50).unwrap();
            let local = b.view_get(&view.id).unwrap();
            retry_rename_view(&mut b, &local, "restored local view");
            a.view_remove(
                &Uuid::new_v4().to_string(),
                pkms_core::RemoveView {
                    view_id: view.id.clone(),
                    base_row_version: view.row_version,
                },
            )
            .unwrap();
            ea.push_view_remove(&view.id).unwrap();
            view.id
        };
        eb.pull_and_apply(&mut b, 50).unwrap();
        let pushed = if is_canvas {
            eb.push_canvas(&b, &id)
        } else {
            eb.push_view(&b, &id)
        };
        assert!(matches!(pushed.unwrap(), PushOutcome::Conflict { .. }));
        let preview = eb.local_config_state(&b, &id).unwrap();
        fault
            .lose_next
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            eb.keep_local_config_checked(&b, &id, 2, &preview.local_state_token)
                .is_err()
        );
        drop(eb);
        let mut eb = reopen_engine(&base, &workspace, &phone.device_id, PHONE_SEED, &state);
        assert_eq!(
            eb.keep_local_config_checked(&b, &id, 2, &preview.local_state_token)
                .unwrap(),
            PushOutcome::Unchanged { version: 3 }
        );
        ea.pull_and_apply(&mut a, 50).unwrap();
        if is_canvas {
            assert_eq!(a.canvas(&id).unwrap().name, "restored local canvas");
        } else {
            assert_eq!(a.view_get(&id).unwrap().name, "restored local view");
        }
        assert!(
            eb.client()
                .pull(&workspace, 0, 50)
                .unwrap()
                .conflicts
                .is_empty()
        );
        let submitted = fault.submitted.lock().unwrap();
        let restores: Vec<_> = submitted
            .iter()
            .filter(|op| op.object_id == id && op.kind == pkms_sync::OpKind::Restore)
            .collect();
        assert_eq!(restores.len(), 2);
        assert_eq!(restores[0], restores[1]);
    }
}

#[test]
fn retry_fork_full_backup_with_fresh_engine_state_reuses_the_core_copy_receipt() {
    for published in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let mut f = fork_fixture(temporary.path(), false);
        let db = rusqlite::Connection::open(&f.state).unwrap();
        if published {
            f.fault
                .lose_next
                .store(true, std::sync::atomic::Ordering::SeqCst);
        } else {
            db.execute_batch("CREATE TRIGGER stop_after_copy BEFORE INSERT ON sync_fork_plans WHEN json_extract(NEW.plan_json,'$.fork_id') IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected after Core copy'); END;").unwrap();
        }
        assert!(f.resolve().is_err());
        let copy = f.local_forks().remove(0);
        let package = temporary.path().join("fork-backup.pkmsbackup");
        f.local.create_backup(&package, &mut |_| Ok(())).unwrap();
        let restored_path = temporary.path().join("restored-workspace");
        pkms_core::prepare_restore(&package, &restored_path, &mut |_| Ok(()))
            .unwrap()
            .publish()
            .unwrap();
        let mut restored = Library::open_existing(&restored_path).unwrap();
        assert_eq!(restored.info().unwrap().workspace_id, f.workspace);
        let new_state = temporary.path().join("new-instance-state.sqlite3");
        let mut engine = reopen_engine(
            &f.base,
            &f.workspace,
            &f.phone.device_id,
            PHONE_SEED,
            &new_state,
        );
        assert!(engine.needs_initial_seed());
        engine.seed_from_snapshot(&restored).unwrap();
        let id = engine
            .resolve_save_as_new(&mut restored, &f.conflict)
            .unwrap();
        assert_eq!(id, copy.id);
        assert_eq!(restored.note(&id).unwrap().document, copy.document);
        assert_eq!(
            restored
                .list_notes()
                .unwrap()
                .iter()
                .filter(|note| note.id != f.source.id)
                .count(),
            1
        );
        assert_eq!(
            engine
                .client()
                .snapshot(&f.workspace)
                .unwrap()
                .iter()
                .filter(|object| object.object_id != f.source.id)
                .count(),
            1
        );
        assert_eq!(restored.note(&f.source.id).unwrap().title, "remote source");
        assert_eq!(
            f.local.note(&f.source.id).unwrap().title,
            "local fork source",
            "restoration must not modify its source library"
        );
        let ops = f.fault.submitted.lock().unwrap();
        assert_eq!(
            ops.iter().filter(|op| op.object_id == id).count(),
            1,
            "restoration reads an existing create receipt instead of publishing another op"
        );
    }
}

#[test]
fn retry_fork_fresh_state_rejects_a_different_source_revision_without_another_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let mut f = fork_fixture(temporary.path(), false);
    f.fault
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(f.resolve().is_err());
    let current = f.local.note(&f.source.id).unwrap();
    let later = edit_note(
        &mut f.local,
        &current,
        "later source edit",
        "preserve this after restore",
    );
    let package = temporary.path().join("edited-fork-backup.pkmsbackup");
    f.local.create_backup(&package, &mut |_| Ok(())).unwrap();
    let restored_path = temporary.path().join("restored-edited-workspace");
    pkms_core::prepare_restore(&package, &restored_path, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let mut restored = Library::open_existing(&restored_path).unwrap();
    let mut engine = reopen_engine(
        &f.base,
        &f.workspace,
        &f.phone.device_id,
        PHONE_SEED,
        &temporary.path().join("edited-new-state.sqlite3"),
    );
    engine.seed_from_snapshot(&restored).unwrap();
    assert!(matches!(
        engine.resolve_save_as_new(&mut restored, &f.conflict),
        Err(pkms_sync_engine::EngineError::Core(
            pkms_core::Error::IdempotencyMismatch
        ))
    ));
    assert_eq!(
        restored.note(&f.source.id).unwrap().revision_id,
        later.revision_id
    );
    assert_eq!(
        restored
            .list_notes()
            .unwrap()
            .iter()
            .filter(|note| note.id != f.source.id)
            .count(),
        1
    );
    assert_eq!(
        engine
            .client()
            .snapshot(&f.workspace)
            .unwrap()
            .iter()
            .filter(|object| object.object_id != f.source.id)
            .count(),
        1
    );
}

#[test]
fn retry_seed_full_backup_without_config_tombstone_does_not_replay_deleted_history() {
    for is_canvas in [false, true] {
        for restore_receiver in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let (mut a, mut b, workspace) = workspace_libraries(temporary.path());
            let (authority, owner, phone) = setup_authority();
            let base = start_server(authority);
            let (mut ea, mut eb) = setup_engines(&base, &workspace, &owner, &phone);
            let note = create_note(&mut a, "retained note", "body");
            ea.push_note(&a, &note.id).unwrap();
            let id = if is_canvas {
                let canvas = a
                    .canvas_save(
                        &Uuid::new_v4().to_string(),
                        SaveCanvas {
                            canvas_id: None,
                            base_row_version: None,
                            name: "deleted canvas".into(),
                            config_schema: 1,
                            nodes: vec![CanvasNode {
                                note_id: note.id,
                                x: 1.0,
                                y: 2.0,
                                width: 250.0,
                                height: 148.0,
                                z_index: 0,
                            }],
                            is_pinned: false,
                            pin_position: None,
                        },
                    )
                    .unwrap()
                    .canvas;
                ea.push_canvas(&a, &canvas.id).unwrap();
                eb.pull_and_apply(&mut b, 50).unwrap();
                a.canvas_remove(
                    &Uuid::new_v4().to_string(),
                    pkms_core::RemoveCanvas {
                        canvas_id: canvas.id.clone(),
                        base_row_version: canvas.row_version,
                    },
                )
                .unwrap();
                assert_eq!(
                    ea.push_canvas_remove(&canvas.id).unwrap(),
                    PushOutcome::Applied { version: 2 }
                );
                canvas.id
            } else {
                let view = retry_view(&mut a, "deleted view");
                ea.push_view(&a, &view.id).unwrap();
                eb.pull_and_apply(&mut b, 50).unwrap();
                a.view_remove(
                    &Uuid::new_v4().to_string(),
                    pkms_core::RemoveView {
                        view_id: view.id.clone(),
                        base_row_version: view.row_version,
                    },
                )
                .unwrap();
                assert_eq!(
                    ea.push_view_remove(&view.id).unwrap(),
                    PushOutcome::Applied { version: 2 }
                );
                view.id
            };
            // Match App's successful push-all acknowledgement. The receiving
            // side independently removes remote configs without Core tombstones.
            a.clear_config_tombstone(&id).unwrap();
            eb.pull_and_apply(&mut b, 50).unwrap();
            assert!(a.config_tombstone_kind(&id).unwrap().is_none());
            assert!(b.config_tombstone_kind(&id).unwrap().is_none());
            let source = if restore_receiver { &b } else { &a };
            let package = temporary.path().join("deleted-config.pkmsbackup");
            source.create_backup(&package, &mut |_| Ok(())).unwrap();
            let restored_path = temporary.path().join("restored-library");
            pkms_core::prepare_restore(&package, &restored_path, &mut |_| Ok(()))
                .unwrap()
                .publish()
                .unwrap();
            let mut restored = Library::open_existing(&restored_path).unwrap();
            let (device_id, seed) = if restore_receiver {
                (&phone.device_id, PHONE_SEED)
            } else {
                (&owner.device_id, OWNER_SEED)
            };
            let mut fresh = reopen_engine(
                &base,
                &workspace,
                device_id,
                seed,
                &temporary.path().join("fresh-state.sqlite3"),
            );
            assert!(fresh.needs_initial_seed());
            fresh.seed_from_snapshot(&restored).unwrap();
            // The snapshot represents an absent local object at version 2.
            // Replaying v1 on a small page would resurrect the deleted config.
            assert_eq!(fresh.synced_version(&id), Some(2));
            for _ in 0..4 {
                let summary = fresh.pull_and_apply(&mut restored, 1).unwrap();
                assert!(summary.config_conflicts.is_empty());
                assert!(!summary.applied.contains(&id));
                if is_canvas {
                    assert!(matches!(
                        restored.canvas(&id),
                        Err(pkms_core::Error::NotFound)
                    ));
                } else {
                    assert!(matches!(
                        restored.view_get(&id),
                        Err(pkms_core::Error::NotFound)
                    ));
                }
                assert_eq!(fresh.synced_version(&id), Some(2));
            }
        }
    }
}
