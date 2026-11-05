//! Reproducible G05 evidence: stale delete conflicts with a remote edit,
//! tombstone blocks resurrection, purge keeps a tombstone, and remote-only
//! objects survive.

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, OpKind, SignedRequest, SyncAccess, SyncAuthority,
    SyncOp, op_request_hash, owner_proof_message, pairing_proof_message, pull_request_hash,
    snapshot_request_hash, trash_request_hash,
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

fn op(kind: OpKind, op_id: &str, object: &str, base: i64, payload: Option<&[u8]>) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-demo".into(),
        object_id: object.into(),
        kind,
        base_version: base,
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

fn read_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: [u8; 32],
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        hash,
    )
}

fn apply(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
) -> ApplyOutcome {
    authority
        .apply_op(
            &write_request(key, device, op, nonce),
            op,
            Utc::now().timestamp(),
        )
        .expect("apply")
}

fn describe(outcome: &ApplyOutcome) -> serde_json::Value {
    match outcome {
        ApplyOutcome::Applied(applied) => json!({
            "status": "applied",
            "version": applied.version,
            "server_seq": applied.server_seq,
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

    let x = op(OpKind::Put, "op-x", "note-x", 0, Some(b"X base"));
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &x,
        "nonce-demo-long-x-1",
    );
    let x_edit = op(
        OpKind::Put,
        "op-x-edit",
        "note-x",
        1,
        Some(b"X remote edit"),
    );
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &x_edit,
        "nonce-demo-long-x-2",
    );
    let x_stale_delete = op(OpKind::Delete, "op-x-stale-delete", "note-x", 1, None);
    let stale_delete = apply(
        &mut authority,
        &owner_key,
        &owner,
        &x_stale_delete,
        "nonce-demo-long-x-3",
    );

    let y = op(OpKind::Put, "op-y", "note-y", 0, Some(b"Y base"));
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &y,
        "nonce-demo-long-y-1",
    );
    let y_delete = op(OpKind::Delete, "op-y-delete", "note-y", 1, None);
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &y_delete,
        "nonce-demo-long-y-2",
    );
    let y_purge = op(OpKind::Purge, "op-y-purge", "note-y", 2, None);
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &y_purge,
        "nonce-demo-long-y-3",
    );

    let z = op(OpKind::Put, "op-z", "note-z", 0, Some(b"Z phone only"));
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &z,
        "nonce-demo-long-z-1",
    );

    let trash = authority
        .trash(
            &read_request(
                &phone_key,
                &phone,
                trash_request_hash("workspace-demo"),
                "nonce-demo-long-trash-1",
            ),
            "workspace-demo",
            Utc::now().timestamp(),
        )
        .expect("trash");

    let restore_purged = op(
        OpKind::Restore,
        "op-y-restore",
        "note-y",
        3,
        Some(b"Y back"),
    );
    let restore_outcome = apply(
        &mut authority,
        &phone_key,
        &phone,
        &restore_purged,
        "nonce-demo-long-restore-1",
    );
    let put_purged = op(OpKind::Put, "op-y-put", "note-y", 3, Some(b"Y overwrite"));
    let put_outcome = apply(
        &mut authority,
        &phone_key,
        &phone,
        &put_purged,
        "nonce-demo-long-put-1",
    );

    let snapshot = authority
        .snapshot(
            &read_request(
                &owner_key,
                &owner,
                snapshot_request_hash("workspace-demo"),
                "nonce-demo-long-snap-1",
            ),
            "workspace-demo",
            Utc::now().timestamp(),
        )
        .expect("snapshot");
    let pull = authority
        .pull(
            &read_request(
                &owner_key,
                &owner,
                pull_request_hash("workspace-demo", 0, 50),
                "nonce-demo-long-pull-1",
            ),
            "workspace-demo",
            0,
            50,
            Utc::now().timestamp(),
        )
        .expect("pull");

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-trash-demo-v1",
            "stale_delete_against_remote_edit": describe(&stale_delete),
            "remote_edit_survives": snapshot.iter().any(|object| {
                object.object_id == "note-x"
                    && object.version == 2
                    && object.ciphertext.as_deref() == Some(&b"X remote edit"[..])
            }),
            "trash": trash.iter().map(|entry| json!({
                "object_id": entry.object_id,
                "version": entry.version,
                "purged": entry.purged,
                "deleted_at": entry.deleted_at,
                "deleted_by": entry.deleted_by,
            })).collect::<Vec<_>>(),
            "restore_purged": describe(&restore_outcome),
            "put_purged": describe(&put_outcome),
            "remote_only_object_survives": snapshot.iter().any(|object| {
                object.object_id == "note-z"
                    && !object.tombstone
                    && object.ciphertext.as_deref() == Some(&b"Z phone only"[..])
            }),
            "snapshot": snapshot.iter().map(|object| json!({
                "object_id": object.object_id,
                "version": object.version,
                "tombstone": object.tombstone,
                "purged": object.purged,
                "ciphertext_utf8": object.ciphertext.as_deref().map(String::from_utf8_lossy),
            })).collect::<Vec<_>>(),
            "pulled_ops": pull.ops.iter().map(|op| json!({
                "server_seq": op.server_seq,
                "object_id": op.object_id,
                "kind": op.kind,
                "resulting_version": op.resulting_version,
                "tombstone": op.tombstone,
                "purged": op.purged,
            })).collect::<Vec<_>>(),
        }))
        .expect("json")
    );
}
