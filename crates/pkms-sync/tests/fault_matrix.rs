use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, OfflineQueue, OpKind, QueueState, SignedRequest,
    SyncAccess, SyncAuthority, SyncError, SyncOp, blob_begin_request_hash, blob_chunk_request_hash,
    blob_finalize_request_hash, blob_status_request_hash, op_request_hash, owner_proof_message,
    pairing_proof_message, pull_request_hash, sha256_hex, snapshot_request_hash,
};

const CHUNK: i64 = 64 * 1024;

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

fn put(op_id: &str, object_id: &str, base_version: i64, body: &[u8]) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-fault".into(),
        object_id: object_id.into(),
        kind: OpKind::Put,
        base_version,
        key_epoch: 1,
        ciphertext: Some(body.to_vec()),
        client_time: None,
    }
}

fn delete(op_id: &str, object_id: &str, base_version: i64) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-fault".into(),
        object_id: object_id.into(),
        kind: OpKind::Delete,
        base_version,
        key_epoch: 1,
        ciphertext: None,
        client_time: None,
    }
}

fn restore(op_id: &str, object_id: &str, base_version: i64, body: &[u8]) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-fault".into(),
        object_id: object_id.into(),
        kind: OpKind::Restore,
        base_version,
        key_epoch: 1,
        ciphertext: Some(body.to_vec()),
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

fn request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    access: SyncAccess,
    hash: [u8; 32],
    nonce: &str,
    issued_at: i64,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        access,
        &format!("fault-nonce-{nonce}"),
        issued_at,
        hash,
    )
}

fn apply(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
) -> Result<ApplyOutcome, SyncError> {
    let now = Utc::now().timestamp();
    authority.apply_op(
        &request(
            key,
            device,
            SyncAccess::Write,
            op_request_hash(op),
            nonce,
            now,
        ),
        op,
        now,
    )
}

fn applied(outcome: ApplyOutcome) -> pkms_sync::AppliedOp {
    match outcome {
        ApplyOutcome::Applied(value) => value,
        ApplyOutcome::Conflict(value) => panic!("unexpected conflict: {value:?}"),
    }
}

fn pull_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    after: i64,
    limit: usize,
    nonce: &str,
) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Read,
        pull_request_hash("workspace-fault", after, limit),
        nonce,
        now,
    )
}

fn snapshot_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Read,
        snapshot_request_hash("workspace-fault"),
        nonce,
        now,
    )
}

fn begin_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    size: i64,
    chunk: i64,
    nonce: &str,
) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Write,
        blob_begin_request_hash(hash, size, chunk),
        nonce,
        now,
    )
}

fn chunk_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    index: u32,
    data: &[u8],
    nonce: &str,
) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Write,
        blob_chunk_request_hash(hash, index, data.len() as i64, &sha256_hex(data)),
        nonce,
        now,
    )
}

fn status_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    nonce: &str,
) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Read,
        blob_status_request_hash(hash),
        nonce,
        now,
    )
}

fn finalize_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    nonce: &str,
) -> SignedRequest {
    let now = Utc::now().timestamp();
    request(
        key,
        device,
        SyncAccess::Write,
        blob_finalize_request_hash(hash),
        nonce,
        now,
    )
}

#[test]
fn lost_reply_and_duplicate_upload_are_idempotent() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    let op = put("fault-dup", "note-dup", 0, b"once");
    let first = applied(apply(&mut authority, &owner_key, &owner, &op, "fault-dup-1").unwrap());
    let second = applied(apply(&mut authority, &owner_key, &owner, &op, "fault-dup-2").unwrap());
    assert!(second.duplicate);
    assert_eq!(second.server_seq, first.server_seq);
    assert_eq!(second.version, first.version);
    let batch = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 50, "fault-dup-pull"),
            "workspace-fault",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.next_cursor, first.server_seq);
}

#[test]
fn partial_failure_does_not_roll_back_successful_objects_and_can_retry() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let mut queue = OfflineQueue::new();
    queue.enqueue(put("fault-x1", "note-x", 0, b"x1")).unwrap();
    queue.enqueue(put("fault-y1", "note-y", 0, b"y1")).unwrap();
    queue.enqueue(put("fault-x2", "note-x", 1, b"x2")).unwrap();

    let x1 = queue.next().unwrap().op.clone();
    queue.mark_in_flight(&x1.op_id).unwrap();
    let x1 = applied(apply(&mut authority, &owner_key, &owner, &x1, "fault-x1-send").unwrap());
    queue
        .mark_applied("fault-x1", x1.server_seq, x1.version)
        .unwrap();

    queue.mark_in_flight("fault-y1").unwrap();
    queue
        .mark_retry("fault-y1", "network_interrupted", true)
        .unwrap();
    assert_eq!(queue.next().unwrap().op.op_id, "fault-y1");

    // An independent object can still make progress while Y retries.
    let x2 = queue
        .entries()
        .iter()
        .find(|entry| entry.op.op_id == "fault-x2")
        .unwrap()
        .op
        .clone();
    let x2 = applied(apply(&mut authority, &owner_key, &owner, &x2, "fault-x2-send").unwrap());
    queue
        .mark_applied("fault-x2", x2.server_seq, x2.version)
        .unwrap();
    assert_eq!(queue.entries()[1].state, QueueState::Pending);

    let y1 = queue
        .entries()
        .iter()
        .find(|entry| entry.op.op_id == "fault-y1")
        .unwrap()
        .op
        .clone();
    let y1 = applied(apply(&mut authority, &owner_key, &owner, &y1, "fault-y1-retry").unwrap());
    queue
        .mark_applied("fault-y1", y1.server_seq, y1.version)
        .unwrap();
    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "fault-snapshot"),
            "workspace-fault",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(
        snapshot
            .iter()
            .find(|item| item.object_id == "note-x")
            .unwrap()
            .version,
        2
    );
    assert_eq!(
        snapshot
            .iter()
            .find(|item| item.object_id == "note-y")
            .unwrap()
            .version,
        1
    );
    assert!(queue.next().is_none());
}

#[test]
fn pull_cursor_survives_partial_page_and_authority_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sync.sqlite3");
    let (owner_key, owner, phone_key, phone) = {
        let mut authority = SyncAuthority::open(&path).unwrap();
        let (owner_key, owner) = enroll_owner(&mut authority);
        let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
        for (index, body) in [b"one".as_slice(), b"two".as_slice()].iter().enumerate() {
            let op = put(
                &format!("fault-page-{index}"),
                &format!("note-{index}"),
                0,
                body,
            );
            applied(
                apply(
                    &mut authority,
                    &owner_key,
                    &owner,
                    &op,
                    &format!("fault-page-send-{index}"),
                )
                .unwrap(),
            );
        }
        (owner_key, owner, phone_key, phone)
    };
    let mut authority = SyncAuthority::open(&path).unwrap();
    let first = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 1, "fault-page-pull-1"),
            "workspace-fault",
            0,
            1,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(first.ops.len(), 1);
    assert!(first.has_more);
    drop(authority);
    let mut authority = SyncAuthority::open(&path).unwrap();
    let second = authority
        .pull(
            &pull_request(
                &phone_key,
                &phone,
                first.next_cursor,
                1,
                "fault-page-pull-2",
            ),
            "workspace-fault",
            first.next_cursor,
            1,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(second.ops.len(), 1);
    assert!(!second.has_more);
    assert!(second.next_cursor > first.next_cursor);
    drop(authority);
    let _ = owner;
    let _ = owner_key;
}

#[test]
fn clock_skew_recovery_and_revocation_are_both_explicit() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    let op = put("fault-clock", "note-clock", 0, b"clock");
    let now = Utc::now().timestamp();
    let stale = authority
        .apply_op(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                op_request_hash(&op),
                "fault-clock-stale",
                now - 7200,
            ),
            &op,
            now,
        )
        .unwrap_err();
    assert_eq!(stale.code(), "stale_request");
    applied(
        authority
            .apply_op(
                &request(
                    &phone_key,
                    &phone,
                    SyncAccess::Write,
                    op_request_hash(&op),
                    "fault-clock-corrected",
                    now,
                ),
                &op,
                now,
            )
            .unwrap(),
    );
    authority
        .revoke_device(&owner.device_id, &phone.device_id, "fault drill")
        .unwrap();
    let denied = authority
        .apply_op(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                op_request_hash(&put("fault-after-revoke", "note-clock", 1, b"no")),
                "fault-after-revoke-1",
                now,
            ),
            &put("fault-after-revoke", "note-clock", 1, b"no"),
            now,
        )
        .unwrap_err();
    assert_eq!(denied.code(), "device_revoked");
    assert!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &put("fault-owner-after-revoke", "note-owner", 0, b"yes"),
            "fault-owner-after-revoke-1"
        )
        .is_ok()
    );
}

#[test]
fn interrupted_blob_upload_resumes_after_reopen_and_duplicate_finalize_is_safe() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("sync.sqlite3");
    let blobs = directory.path().join("blobs");
    let data = payload((CHUNK + 123) as usize);
    let hash = sha256_hex(&data);
    let (owner_key, owner) = {
        let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).unwrap();
        let (key, device) = enroll_owner(&mut authority);
        authority
            .begin_blob(
                &begin_request(
                    &key,
                    &device,
                    &hash,
                    data.len() as i64,
                    CHUNK,
                    "fault-blob-begin",
                ),
                &hash,
                data.len() as i64,
                CHUNK,
                Utc::now().timestamp(),
            )
            .unwrap();
        authority
            .upload_blob_chunk(
                &chunk_request(
                    &key,
                    &device,
                    &hash,
                    0,
                    &data[..CHUNK as usize],
                    "fault-blob-chunk-0",
                ),
                &hash,
                0,
                &data[..CHUNK as usize],
                Utc::now().timestamp(),
            )
            .unwrap();
        (key, device)
    };
    let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).unwrap();
    let status = authority
        .blob_status(
            &status_request(&owner_key, &owner, &hash, "fault-blob-status"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(status.missing_chunks, vec![1]);
    authority
        .upload_blob_chunk(
            &chunk_request(
                &owner_key,
                &owner,
                &hash,
                1,
                &data[CHUNK as usize..],
                "fault-blob-chunk-1",
            ),
            &hash,
            1,
            &data[CHUNK as usize..],
            Utc::now().timestamp(),
        )
        .unwrap();
    let record = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "fault-blob-finalize-1"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(record.complete);
    let duplicate = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "fault-blob-finalize-2"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(duplicate.complete);
}

#[test]
fn delete_then_restore_recovers_without_resurrection_or_data_loss() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    applied(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &put("fault-live", "note-life", 0, b"body"),
            "fault-live-1",
        )
        .unwrap(),
    );
    applied(
        apply(
            &mut authority,
            &phone_key,
            &phone,
            &delete("fault-delete", "note-life", 1),
            "fault-delete-1",
        )
        .unwrap(),
    );
    let stale = apply(
        &mut authority,
        &owner_key,
        &owner,
        &put("fault-stale", "note-life", 1, b"late"),
        "fault-stale-1",
    )
    .unwrap();
    assert!(matches!(stale, ApplyOutcome::Conflict(_)));
    let trash = authority
        .trash(
            &request(
                &owner_key,
                &owner,
                SyncAccess::Read,
                pkms_sync::trash_request_hash("workspace-fault"),
                "fault-trash-1",
                Utc::now().timestamp(),
            ),
            "workspace-fault",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(trash.len(), 1);
    applied(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &restore("fault-restore", "note-life", 2, b"body"),
            "fault-restore-1",
        )
        .unwrap(),
    );
    let snapshot = authority
        .snapshot(
            &snapshot_request(&phone_key, &phone, "fault-snapshot-restore"),
            "workspace-fault",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    assert!(!snapshot[0].tombstone);
    assert_eq!(snapshot[0].version, 3);
}
