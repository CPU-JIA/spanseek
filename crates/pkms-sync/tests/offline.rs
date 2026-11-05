use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictResolution, DeviceKeyPair, DeviceRecord, OfflineQueue, OpKind,
    QueueState, SignedRequest, SyncAccess, SyncAuthority, SyncError, SyncOp, op_request_hash,
    owner_proof_message, pairing_proof_message, pull_request_hash, resolve_conflict_request_hash,
    snapshot_request_hash,
};

fn put(op_id: &str, object_id: &str, base_version: i64, payload: &[u8]) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-1".into(),
        object_id: object_id.into(),
        kind: OpKind::Put,
        base_version,
        key_epoch: 1,
        ciphertext: Some(payload.to_vec()),
        client_time: None,
    }
}

fn enroll_owner(authority: &mut SyncAuthority) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().unwrap();
    let proof = key.sign(&owner_proof_message(&key.public_key_hex(), "owner desktop"));
    let device = authority
        .enroll_owner(&key.public_key(), "owner desktop", &proof)
        .unwrap();
    (key, device)
}

fn pair_device(
    authority: &mut SyncAuthority,
    creator: &DeviceRecord,
    name: &str,
) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().unwrap();
    let invite = authority
        .create_pairing_invite(&creator.device_id, 600)
        .unwrap();
    let proof = key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &key.public_key_hex(),
    ));
    let device = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &key.public_key(),
            name,
            &proof,
        )
        .unwrap();
    (key, device)
}

fn signed(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
    issued_at: i64,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        issued_at,
        op_request_hash(op),
    )
}

fn apply_at(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
    issued_at: i64,
    now: i64,
) -> Result<ApplyOutcome, SyncError> {
    authority.apply_op(&signed(key, device, op, nonce, issued_at), op, now)
}

fn setup() -> (
    SyncAuthority,
    DeviceKeyPair,
    DeviceRecord,
    DeviceKeyPair,
    DeviceRecord,
) {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    (authority, owner_key, owner, phone_key, phone)
}

#[test]
fn queue_preserves_object_order_and_allows_other_objects() {
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("op-x1", "note-x", 0, b"x1")).unwrap();
    queue.enqueue(put("op-y1", "note-y", 0, b"y1")).unwrap();
    queue.enqueue(put("op-x2", "note-x", 1, b"x2")).unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "op-x1");
    queue.mark_in_flight("op-x1").unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "op-y1");
    queue.mark_applied("op-x1", 1, 1).unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "op-y1");
    queue.mark_applied("op-y1", 2, 1).unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "op-x2");
}

#[test]
fn applied_result_rebases_later_offline_edits() {
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("op-x1", "note-x", 0, b"x1")).unwrap();
    queue.enqueue(put("op-x2", "note-x", 1, b"x2")).unwrap();
    queue.mark_applied("op-x1", 1, 2).unwrap();
    let next = queue.next().unwrap();
    assert_eq!(next.op.op_id, "op-x2");
    assert_eq!(next.op.base_version, 2);
}

#[test]
fn conflict_blocks_later_edits_until_rebase() {
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("op-x1", "note-x", 1, b"x1")).unwrap();
    queue.enqueue(put("op-x2", "note-x", 2, b"x2")).unwrap();
    queue.mark_conflict("op-x1", "conflict-1", 2).unwrap();
    assert!(queue.next().is_none());
    assert!(matches!(
        queue.entries()[1].state,
        QueueState::Blocked { .. }
    ));
    queue.resolve_rebase("op-x1", 2).unwrap();
    let next = queue.next().unwrap();
    assert_eq!(next.op.op_id, "op-x1");
    assert_eq!(next.op.base_version, 2);
    let later = queue
        .entries()
        .iter()
        .find(|entry| entry.op.op_id == "op-x2")
        .unwrap();
    assert_eq!(later.op.base_version, 3);
    assert_eq!(later.state, QueueState::Pending);
}

#[test]
fn keep_remote_cancels_the_local_chain() {
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("op-x1", "note-x", 1, b"x1")).unwrap();
    queue.enqueue(put("op-x2", "note-x", 2, b"x2")).unwrap();
    queue.mark_conflict("op-x1", "conflict-1", 2).unwrap();
    queue.resolve_keep_remote("op-x1").unwrap();
    assert!(queue.next().is_none());
    assert_eq!(queue.entries()[0].state, QueueState::Cancelled);
    assert_eq!(queue.entries()[1].state, QueueState::Cancelled);
}

#[test]
fn retryable_failure_stays_pending_and_fatal_blocks_later_ops() {
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("op-x1", "note-x", 0, b"x1")).unwrap();
    queue.enqueue(put("op-x2", "note-x", 1, b"x2")).unwrap();
    queue.mark_in_flight("op-x1").unwrap();
    queue.mark_retry("op-x1", "network", true).unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "op-x1");
    assert_eq!(queue.entries()[0].attempts, 1);
    queue.mark_in_flight("op-x1").unwrap();
    queue.mark_retry("op-x1", "fatal", false).unwrap();
    assert!(queue.next().is_none());
    assert_eq!(
        queue.entries()[0].state,
        QueueState::Failed {
            code: "fatal".into()
        }
    );
}

#[test]
fn offline_chain_reconnect_rebases_and_resolves_predictably() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let base_x = put("op-base-x", "note-x", 0, b"base x");
    authority
        .apply_op(
            &signed(
                &owner_key,
                &owner,
                &base_x,
                "nonce-offline-base-1",
                Utc::now().timestamp(),
            ),
            &base_x,
            Utc::now().timestamp(),
        )
        .unwrap();
    let base_y = put("op-base-y", "note-y", 0, b"base y");
    authority
        .apply_op(
            &signed(
                &owner_key,
                &owner,
                &base_y,
                "nonce-offline-base-2",
                Utc::now().timestamp(),
            ),
            &base_y,
            Utc::now().timestamp(),
        )
        .unwrap();

    let mut queue = OfflineQueue::new();
    queue
        .enqueue(put("op-x2", "note-x", 1, b"phone x2"))
        .unwrap();
    queue
        .enqueue(put("op-x3", "note-x", 2, b"phone x3"))
        .unwrap();
    queue
        .enqueue(put("op-y2", "note-y", 1, b"phone y2"))
        .unwrap();

    // Another device updates X while this queue is offline.
    let remote = put("op-remote-x2", "note-x", 1, b"owner x2");
    authority
        .apply_op(
            &signed(
                &owner_key,
                &owner,
                &remote,
                "nonce-offline-remote-1",
                Utc::now().timestamp(),
            ),
            &remote,
            Utc::now().timestamp(),
        )
        .unwrap();

    let first = queue.next().unwrap().op.clone();
    let conflict_outcome = apply_at(
        &mut authority,
        &phone_key,
        &phone,
        &first,
        "nonce-offline-send-1",
        Utc::now().timestamp(),
        Utc::now().timestamp(),
    )
    .unwrap();
    assert!(matches!(conflict_outcome, ApplyOutcome::Conflict(_)));
    let batch = authority
        .pull(
            &phone_key.sign_request(
                &phone.device_id,
                SyncAccess::Read,
                "nonce-offline-pull-1",
                Utc::now().timestamp(),
                pull_request_hash("workspace-1", 0, 50),
            ),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    let conflict = batch
        .conflicts
        .into_iter()
        .find(|conflict| conflict.op_id == "op-x2")
        .expect("conflict");
    queue
        .mark_conflict("op-x2", &conflict.conflict_id, 2)
        .unwrap();

    let y2 = queue.next().unwrap().op.clone();
    let applied_y = apply_at(
        &mut authority,
        &phone_key,
        &phone,
        &y2,
        "nonce-offline-send-2",
        Utc::now().timestamp(),
        Utc::now().timestamp(),
    )
    .unwrap();
    let applied_y = match applied_y {
        ApplyOutcome::Applied(applied) => applied,
        ApplyOutcome::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
    };
    queue
        .mark_applied("op-y2", applied_y.server_seq, applied_y.version)
        .unwrap();

    queue.resolve_rebase("op-x2", 2).unwrap();
    let x2 = queue.next().unwrap().op.clone();
    let applied_x2 = match apply_at(
        &mut authority,
        &phone_key,
        &phone,
        &x2,
        "nonce-offline-send-3",
        Utc::now().timestamp(),
        Utc::now().timestamp(),
    )
    .unwrap()
    {
        ApplyOutcome::Applied(applied) => applied,
        ApplyOutcome::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
    };
    queue
        .mark_applied("op-x2", applied_x2.server_seq, applied_x2.version)
        .unwrap();
    authority
        .resolve_conflict(
            &phone_key.sign_request(
                &phone.device_id,
                SyncAccess::Write,
                "nonce-offline-resolve-1",
                Utc::now().timestamp(),
                resolve_conflict_request_hash(
                    &conflict.conflict_id,
                    ConflictResolution::KeepLocal,
                    Some("op-x2"),
                    None,
                ),
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-x2"),
            None,
            Utc::now().timestamp(),
        )
        .unwrap();

    let x3 = queue.next().unwrap().op.clone();
    let applied_x3 = match apply_at(
        &mut authority,
        &phone_key,
        &phone,
        &x3,
        "nonce-offline-send-4",
        Utc::now().timestamp(),
        Utc::now().timestamp(),
    )
    .unwrap()
    {
        ApplyOutcome::Applied(applied) => applied,
        ApplyOutcome::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
    };
    queue
        .mark_applied("op-x3", applied_x3.server_seq, applied_x3.version)
        .unwrap();
    assert!(queue.next().is_none());

    let snapshot = authority
        .snapshot(
            &phone_key.sign_request(
                &phone.device_id,
                SyncAccess::Read,
                "nonce-offline-snap-1",
                Utc::now().timestamp(),
                snapshot_request_hash("workspace-1"),
            ),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    let x = snapshot
        .iter()
        .find(|object| object.object_id == "note-x")
        .unwrap();
    let y = snapshot
        .iter()
        .find(|object| object.object_id == "note-y")
        .unwrap();
    assert_eq!(x.version, 4);
    assert_eq!(x.ciphertext.as_deref(), Some(&b"phone x3"[..]));
    assert_eq!(y.version, 2);
    assert_eq!(y.ciphertext.as_deref(), Some(&b"phone y2"[..]));
}

#[test]
fn clock_skew_is_corrected_by_server_time() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let server_now = authority.server_time_unix();
    let local_now = server_now + 7_200;
    let op = put("op-clock", "note-clock", 0, b"clock");
    let stale = apply_at(
        &mut authority,
        &owner_key,
        &owner,
        &op,
        "nonce-clock-uncorrected-1",
        local_now,
        server_now,
    )
    .unwrap_err();
    assert_eq!(stale.code(), "stale_request");

    let offset = server_now - local_now;
    let corrected = local_now + offset;
    let applied = apply_at(
        &mut authority,
        &owner_key,
        &owner,
        &op,
        "nonce-clock-corrected-1",
        corrected,
        server_now,
    )
    .unwrap();
    assert!(matches!(applied, ApplyOutcome::Applied(_)));
}
