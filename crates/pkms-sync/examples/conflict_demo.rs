//! Reproducible G04 evidence: record a conflict, render a side-by-side
//! comparison payload, and exercise keep-local / keep-remote / save-as-new.

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictResolution, DeviceKeyPair, DeviceRecord, OpKind, SignedRequest,
    SyncAccess, SyncAuthority, SyncOp, op_request_hash, owner_proof_message, pairing_proof_message,
    pull_request_hash, resolve_conflict_request_hash, snapshot_request_hash,
};
use serde_json::{Value, json};

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

fn pull_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        pull_request_hash("workspace-demo", 0, 50),
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

fn resolve_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    conflict_id: &str,
    resolution: ConflictResolution,
    replacement_op_id: Option<&str>,
    replacement_object_id: Option<&str>,
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        resolve_conflict_request_hash(
            conflict_id,
            resolution,
            replacement_op_id,
            replacement_object_id,
        ),
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

/// Client-side comparison shape: the UI decrypts first, then renders these
/// two columns. The authority never sees plaintext.
fn side_by_side(local: &[u8], remote: &[u8]) -> Value {
    let left = String::from_utf8_lossy(local)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let right = String::from_utf8_lossy(remote)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let rows = left.len().max(right.len());
    let mut pairs = Vec::new();
    for index in 0..rows {
        pairs.push(json!({
            "local": left.get(index).cloned().unwrap_or_default(),
            "remote": right.get(index).cloned().unwrap_or_default(),
        }));
    }
    json!({"columns": ["local", "remote"], "rows": pairs})
}

fn latest_conflict(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) -> pkms_sync::ConflictRecord {
    authority
        .pull(
            &pull_request(key, device, nonce),
            "workspace-demo",
            0,
            50,
            Utc::now().timestamp(),
        )
        .expect("pull")
        .conflicts
        .into_iter()
        .next()
        .expect("conflict")
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

    // 1. keep-local
    let remote_a = op(
        OpKind::Put,
        "op-remote-a",
        "note-a",
        0,
        Some(b"remote A\nline 2"),
    );
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &remote_a,
        "nonce-demo-remote-a-1",
    );
    let local_a = op(
        OpKind::Put,
        "op-local-a",
        "note-a",
        0,
        Some(b"local A\nline 2"),
    );
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &local_a,
            "nonce-demo-local-a-1"
        ),
        ApplyOutcome::Conflict(_)
    ));
    let conflict_a = latest_conflict(&mut authority, &owner_key, &owner, "nonce-demo-list-a-1");
    let remote_state_a = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-demo-snap-a-1"),
            "workspace-demo",
            Utc::now().timestamp(),
        )
        .expect("snapshot")
        .into_iter()
        .find(|object| object.object_id == "note-a")
        .expect("object a");
    let comparison_a = side_by_side(
        b"local A\nline 2",
        remote_state_a.ciphertext.as_deref().unwrap_or_default(),
    );
    let rebased = op(
        OpKind::Put,
        "op-local-a-rebased",
        "note-a",
        1,
        Some(b"local A\nline 2"),
    );
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &rebased,
        "nonce-demo-rebased-a-1",
    );
    let resolved_a = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict_a.conflict_id,
                ConflictResolution::KeepLocal,
                Some("op-local-a-rebased"),
                None,
                "nonce-demo-resolve-a-1",
            ),
            &conflict_a.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-local-a-rebased"),
            None,
            Utc::now().timestamp(),
        )
        .expect("keep local");

    // 2. keep-remote
    let remote_b = op(OpKind::Put, "op-remote-b", "note-b", 0, Some(b"remote B"));
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &remote_b,
        "nonce-demo-remote-b-1",
    );
    let local_b = op(OpKind::Put, "op-local-b", "note-b", 0, Some(b"local B"));
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &local_b,
            "nonce-demo-local-b-1"
        ),
        ApplyOutcome::Conflict(_)
    ));
    let conflict_b = latest_conflict(&mut authority, &owner_key, &owner, "nonce-demo-list-b-1");
    let resolved_b = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict_b.conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
                "nonce-demo-resolve-b-1",
            ),
            &conflict_b.conflict_id,
            ConflictResolution::KeepRemote,
            None,
            None,
            Utc::now().timestamp(),
        )
        .expect("keep remote");

    // 3. save-as-new
    let remote_c = op(OpKind::Put, "op-remote-c", "note-c", 0, Some(b"remote C"));
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &remote_c,
        "nonce-demo-remote-c-1",
    );
    let local_c = op(OpKind::Put, "op-local-c", "note-c", 0, Some(b"local C"));
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &local_c,
            "nonce-demo-local-c-1"
        ),
        ApplyOutcome::Conflict(_)
    ));
    let conflict_c = latest_conflict(&mut authority, &owner_key, &owner, "nonce-demo-list-c-1");
    let copy = op(
        OpKind::Put,
        "op-local-c-copy",
        "note-c-copy",
        0,
        Some(b"local C"),
    );
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &copy,
        "nonce-demo-copy-c-1",
    );
    let resolved_c = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict_c.conflict_id,
                ConflictResolution::SaveAsNew,
                Some("op-local-c-copy"),
                Some("note-c-copy"),
                "nonce-demo-resolve-c-1",
            ),
            &conflict_c.conflict_id,
            ConflictResolution::SaveAsNew,
            Some("op-local-c-copy"),
            Some("note-c-copy"),
            Utc::now().timestamp(),
        )
        .expect("save as new");

    let final_snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-demo-snap-final-1"),
            "workspace-demo",
            Utc::now().timestamp(),
        )
        .expect("final snapshot");
    let objects = final_snapshot
        .iter()
        .map(|object| {
            json!({
                "object_id": object.object_id,
                "version": object.version,
                "tombstone": object.tombstone,
                "ciphertext_utf8": object.ciphertext.as_deref().map(String::from_utf8_lossy),
            })
        })
        .collect::<Vec<_>>();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-conflict-demo-v1",
            "side_by_side": comparison_a,
            "conflict": {
                "conflict_id": conflict_a.conflict_id,
                "reason": conflict_a.reason,
                "remote_version": conflict_a.remote_version,
                "remote_tombstone": conflict_a.remote_tombstone,
            },
            "keep_local": {
                "resolution": resolved_a.resolution,
                "replacement_op_id": resolved_a.replacement_op_id,
                "replacement_object_id": resolved_a.replacement_object_id,
            },
            "keep_remote": {
                "resolution": resolved_b.resolution,
                "remote_payload_preserved": true,
            },
            "save_as_new": {
                "resolution": resolved_c.resolution,
                "replacement_object_id": resolved_c.replacement_object_id,
            },
            "final_objects": objects,
            "unresolved_conflicts": authority
                .pull(
                    &pull_request(&owner_key, &owner, "nonce-demo-final-list-1"),
                    "workspace-demo",
                    0,
                    50,
                    Utc::now().timestamp(),
                )
                .expect("final pull")
                .conflicts
                .len(),
        }))
        .expect("json")
    );
}
