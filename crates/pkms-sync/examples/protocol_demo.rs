//! Reproducible G03 evidence: conflict without overwrite, idempotent retry,
//! tombstone propagation, explicit restore, cursor pull and revoked-device
//! denial.

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, OpKind, SignedRequest, SyncAccess, SyncAuthority,
    SyncOp, op_request_hash, owner_proof_message, pairing_proof_message, pull_request_hash,
    snapshot_request_hash,
};
use serde_json::json;

fn pair_device(
    authority: &mut SyncAuthority,
    creator: &DeviceRecord,
    key: &DeviceKeyPair,
    name: &str,
) -> DeviceRecord {
    let invite = authority
        .create_pairing_invite(&creator.device_id, 600)
        .expect("invite");
    let proof = key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &key.public_key_hex(),
    ));
    authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &key.public_key(),
            name,
            &proof,
        )
        .expect("pair")
}

fn op(kind: OpKind, op_id: &str, base_version: i64, payload: Option<&[u8]>) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-demo".into(),
        object_id: "note-demo".into(),
        kind,
        base_version,
        key_epoch: 1,
        ciphertext: payload.map(|value| value.to_vec()),
        client_time: None,
    }
}

fn write_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        op_request_hash(op),
    )
}

fn pull_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    after_seq: i64,
    limit: usize,
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        pull_request_hash("workspace-demo", after_seq, limit),
    )
}

fn snapshot_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        snapshot_request_hash("workspace-demo"),
    )
}

fn describe(outcome: &ApplyOutcome) -> serde_json::Value {
    match outcome {
        ApplyOutcome::Applied(applied) => json!({
            "status": "applied",
            "op_id": applied.op_id,
            "server_seq": applied.server_seq,
            "version": applied.version,
            "duplicate": applied.duplicate,
        }),
        ApplyOutcome::Conflict(conflict) => json!({
            "status": "conflict",
            "reason": conflict.reason,
            "current_version": conflict.current_version,
            "current_tombstone": conflict.current_tombstone,
        }),
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
    let phone = pair_device(&mut authority, &owner, &phone_key, "phone");

    let phone_put = op(OpKind::Put, "op-phone-put", 0, Some(b"phone-edit"));
    let first = authority
        .apply_op(
            &write_request(&phone_key, &phone, &phone_put, "demo-phone-put-0001"),
            &phone_put,
            Utc::now().timestamp(),
        )
        .expect("phone put");

    let stale = op(OpKind::Put, "op-owner-stale", 0, Some(b"owner-edit"));
    let stale_outcome = authority
        .apply_op(
            &write_request(&owner_key, &owner, &stale, "demo-owner-stale-01"),
            &stale,
            Utc::now().timestamp(),
        )
        .expect("stale put");

    let fresh = op(OpKind::Put, "op-owner-fresh", 1, Some(b"owner-edit"));
    let fresh_outcome = authority
        .apply_op(
            &write_request(&owner_key, &owner, &fresh, "demo-owner-fresh-01"),
            &fresh,
            Utc::now().timestamp(),
        )
        .expect("fresh put");

    let remove = op(OpKind::Delete, "op-phone-delete", 2, None);
    let delete_outcome = authority
        .apply_op(
            &write_request(&phone_key, &phone, &remove, "demo-phone-delete-1"),
            &remove,
            Utc::now().timestamp(),
        )
        .expect("delete");

    let late = op(OpKind::Put, "op-owner-late", 3, Some(b"late-edit"));
    let late_outcome = authority
        .apply_op(
            &write_request(&owner_key, &owner, &late, "demo-owner-late-01"),
            &late,
            Utc::now().timestamp(),
        )
        .expect("late put");

    let restore = op(OpKind::Restore, "op-owner-restore", 3, Some(b"restored"));
    let restore_outcome = authority
        .apply_op(
            &write_request(&owner_key, &owner, &restore, "demo-owner-restore"),
            &restore,
            Utc::now().timestamp(),
        )
        .expect("restore");

    let retry = authority
        .apply_op(
            &write_request(&owner_key, &owner, &restore, "demo-owner-restore"),
            &restore,
            Utc::now().timestamp(),
        )
        .expect("idempotent retry");

    let batch = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 50, "demo-phone-pull-001"),
            "workspace-demo",
            0,
            50,
            Utc::now().timestamp(),
        )
        .expect("pull");
    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "demo-owner-snap-01"),
            "workspace-demo",
            Utc::now().timestamp(),
        )
        .expect("snapshot");

    authority
        .revoke_device(&owner.device_id, &phone.device_id, "demo revoke")
        .expect("revoke");
    let revoked_pull = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 50, "demo-phone-pull-002"),
            "workspace-demo",
            0,
            50,
            Utc::now().timestamp(),
        )
        .expect_err("revoked pull");

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-object-protocol-demo-v1",
            "phone_put": describe(&first),
            "stale_owner_put": describe(&stale_outcome),
            "fresh_owner_put": describe(&fresh_outcome),
            "phone_delete": describe(&delete_outcome),
            "late_put_after_delete": describe(&late_outcome),
            "owner_restore": describe(&restore_outcome),
            "idempotent_retry": describe(&retry),
            "pulled_ops": batch.ops.iter().map(|op| json!({
                "server_seq": op.server_seq,
                "op_id": op.op_id,
                "kind": op.kind,
                "resulting_version": op.resulting_version,
                "tombstone": op.tombstone,
            })).collect::<Vec<_>>(),
            "next_cursor": batch.next_cursor,
            "snapshot": snapshot,
            "revoked_pull_error": revoked_pull.code(),
        }))
        .expect("json")
    );
}
