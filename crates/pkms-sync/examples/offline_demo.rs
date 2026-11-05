//! Reproducible G07 evidence: queued offline edits, conflict + rebase,
//! deterministic reconnect merge, and server-clock skew correction.

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictResolution, DeviceKeyPair, DeviceRecord, OfflineQueue, OpKind,
    SignedRequest, SyncAccess, SyncAuthority, SyncError, SyncOp, op_request_hash,
    owner_proof_message, pairing_proof_message, pull_request_hash, resolve_conflict_request_hash,
    snapshot_request_hash,
};
use serde_json::json;

fn put(op_id: &str, object_id: &str, base_version: i64, payload: &[u8]) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-demo".into(),
        object_id: object_id.into(),
        kind: OpKind::Put,
        base_version,
        key_epoch: 1,
        ciphertext: Some(payload.to_vec()),
        client_time: None,
    }
}

fn sign(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    access: SyncAccess,
    hash: [u8; 32],
    nonce: &str,
    issued_at: i64,
) -> SignedRequest {
    key.sign_request(&device.device_id, access, nonce, issued_at, hash)
}

fn apply(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
    issued_at: i64,
    now: i64,
) -> Result<ApplyOutcome, SyncError> {
    authority.apply_op(
        &sign(
            key,
            device,
            SyncAccess::Write,
            op_request_hash(op),
            nonce,
            issued_at,
        ),
        op,
        now,
    )
}

fn applied(outcome: ApplyOutcome) -> pkms_sync::AppliedOp {
    match outcome {
        ApplyOutcome::Applied(applied) => applied,
        ApplyOutcome::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
    }
}

fn main() {
    let mut authority = SyncAuthority::open_in_memory().expect("authority");
    let owner_key = DeviceKeyPair::generate().expect("owner key");
    let owner_proof = owner_key.sign(&owner_proof_message(&owner_key.public_key_hex(), "owner"));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "owner", &owner_proof)
        .expect("owner");
    let phone_key = DeviceKeyPair::generate().expect("phone key");
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .expect("invite");
    let phone_proof = phone_key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &phone_key.public_key_hex(),
    ));
    let phone = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &phone_key.public_key(),
            "phone",
            &phone_proof,
        )
        .expect("phone");

    let now = Utc::now().timestamp();
    applied(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &put("op-base-x", "note-x", 0, b"base x"),
            "nonce-demo-offline-base-1",
            now,
            now,
        )
        .expect("base x"),
    );
    applied(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &put("op-base-y", "note-y", 0, b"base y"),
            "nonce-demo-offline-base-2",
            now,
            now,
        )
        .expect("base y"),
    );

    let mut queue = OfflineQueue::new();
    queue
        .enqueue(put("op-x2", "note-x", 1, b"phone x2"))
        .expect("queue x2");
    queue
        .enqueue(put("op-x3", "note-x", 2, b"phone x3"))
        .expect("queue x3");
    queue
        .enqueue(put("op-y2", "note-y", 1, b"phone y2"))
        .expect("queue y2");
    let queued = queue
        .entries()
        .iter()
        .map(|entry| entry.op.op_id.clone())
        .collect::<Vec<_>>();

    applied(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &put("op-remote-x2", "note-x", 1, b"owner x2"),
            "nonce-demo-offline-remote-1",
            now,
            now,
        )
        .expect("remote x2"),
    );

    let x2 = queue.next().expect("next x2").op.clone();
    let first_outcome = apply(
        &mut authority,
        &phone_key,
        &phone,
        &x2,
        "nonce-demo-offline-send-1",
        now,
        now,
    )
    .expect("send x2");
    let batch = authority
        .pull(
            &sign(
                &phone_key,
                &phone,
                SyncAccess::Read,
                pull_request_hash("workspace-demo", 0, 50),
                "nonce-demo-offline-pull-1",
                now,
            ),
            "workspace-demo",
            0,
            50,
            now,
        )
        .expect("pull");
    let conflict = batch
        .conflicts
        .into_iter()
        .find(|conflict| conflict.op_id == "op-x2")
        .expect("conflict");
    queue
        .mark_conflict("op-x2", &conflict.conflict_id, 2)
        .expect("mark conflict");

    let y2 = queue.next().expect("next y2").op.clone();
    let y2_applied = applied(
        apply(
            &mut authority,
            &phone_key,
            &phone,
            &y2,
            "nonce-demo-offline-send-2",
            now,
            now,
        )
        .expect("send y2"),
    );
    queue
        .mark_applied("op-y2", y2_applied.server_seq, y2_applied.version)
        .expect("applied y2");

    queue.resolve_rebase("op-x2", 2).expect("rebase x2");
    let x2_after = queue
        .entries()
        .iter()
        .find(|entry| entry.op.op_id == "op-x2")
        .expect("x2 entry")
        .op
        .base_version;
    let x3_after = queue
        .entries()
        .iter()
        .find(|entry| entry.op.op_id == "op-x3")
        .expect("x3 entry")
        .op
        .base_version;
    let x2 = queue.next().expect("rebased x2").op.clone();
    let x2_applied = applied(
        apply(
            &mut authority,
            &phone_key,
            &phone,
            &x2,
            "nonce-demo-offline-send-3",
            now,
            now,
        )
        .expect("send rebased x2"),
    );
    queue
        .mark_applied("op-x2", x2_applied.server_seq, x2_applied.version)
        .expect("applied x2");
    authority
        .resolve_conflict(
            &sign(
                &phone_key,
                &phone,
                SyncAccess::Write,
                resolve_conflict_request_hash(
                    &conflict.conflict_id,
                    ConflictResolution::KeepLocal,
                    Some("op-x2"),
                    None,
                ),
                "nonce-demo-offline-resolve-1",
                now,
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-x2"),
            None,
            now,
        )
        .expect("resolve conflict");

    let x3 = queue.next().expect("next x3").op.clone();
    let x3_applied = applied(
        apply(
            &mut authority,
            &phone_key,
            &phone,
            &x3,
            "nonce-demo-offline-send-4",
            now,
            now,
        )
        .expect("send x3"),
    );
    queue
        .mark_applied("op-x3", x3_applied.server_seq, x3_applied.version)
        .expect("applied x3");

    let snapshot = authority
        .snapshot(
            &sign(
                &phone_key,
                &phone,
                SyncAccess::Read,
                snapshot_request_hash("workspace-demo"),
                "nonce-demo-offline-snap-1",
                now,
            ),
            "workspace-demo",
            now,
        )
        .expect("snapshot");

    let server_now = authority.server_time_unix();
    let local_now = server_now + 7_200;
    let skew_op = put("op-clock", "note-clock", 0, b"clock");
    let uncorrected = apply(
        &mut authority,
        &owner_key,
        &owner,
        &skew_op,
        "nonce-demo-offline-clock-1",
        local_now,
        server_now,
    )
    .expect_err("uncorrected skew");
    let corrected = local_now + (server_now - local_now);
    let corrected_outcome = apply(
        &mut authority,
        &owner_key,
        &owner,
        &skew_op,
        "nonce-demo-offline-clock-2",
        corrected,
        server_now,
    )
    .expect("corrected skew");

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-offline-demo-v1",
            "queued": queued,
            "first_send": match first_outcome {
                ApplyOutcome::Conflict(conflict) => json!({
                    "status": "conflict",
                    "reason": conflict.reason,
                    "current_version": conflict.current_version,
                }),
                ApplyOutcome::Applied(applied) => json!({"status": "applied", "version": applied.version}),
            },
            "y2_applied_version": y2_applied.version,
            "after_rebase": {"x2_base": x2_after, "x3_base": x3_after},
            "x2_applied_version": x2_applied.version,
            "x3_applied_version": x3_applied.version,
            "queue_empty": queue.next().is_none(),
            "final_objects": snapshot.iter().map(|object| json!({
                "object_id": object.object_id,
                "version": object.version,
                "ciphertext_utf8": object.ciphertext.as_deref().map(String::from_utf8_lossy),
            })).collect::<Vec<_>>(),
            "clock_skew": {
                "offset_seconds": 7_200,
                "uncorrected": uncorrected.code(),
                "corrected": match corrected_outcome {
                    ApplyOutcome::Applied(applied) => json!({"status": "applied", "version": applied.version}),
                    ApplyOutcome::Conflict(conflict) => json!({"status": "conflict", "reason": conflict.reason}),
                },
            },
        }))
        .expect("json")
    );
}
