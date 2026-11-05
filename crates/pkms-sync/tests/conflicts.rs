use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictReason, ConflictResolution, DeviceKeyPair, DeviceRecord, OpKind,
    SignedRequest, SyncAccess, SyncAuthority, SyncOp, op_request_hash, owner_proof_message,
    pairing_proof_message, pull_request_hash, resolve_conflict_request_hash, snapshot_request_hash,
};

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

fn delete(op_id: &str, object_id: &str, base_version: i64) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-1".into(),
        object_id: object_id.into(),
        kind: OpKind::Delete,
        base_version,
        key_epoch: 1,
        ciphertext: None,
        client_time: None,
    }
}

fn restore(op_id: &str, object_id: &str, base_version: i64, payload: &[u8]) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-1".into(),
        object_id: object_id.into(),
        kind: OpKind::Restore,
        base_version,
        key_epoch: 1,
        ciphertext: Some(payload.to_vec()),
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
        pull_request_hash("workspace-1", after_seq, limit),
    )
}

fn snapshot_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        snapshot_request_hash("workspace-1"),
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
        .unwrap()
}

fn conflicts_for(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) -> Vec<pkms_sync::ConflictRecord> {
    authority
        .pull(
            &pull_request(key, device, 0, 50, nonce),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap()
        .conflicts
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

fn make_conflict(
    authority: &mut SyncAuthority,
    owner_key: &DeviceKeyPair,
    owner: &DeviceRecord,
    phone_key: &DeviceKeyPair,
    phone: &DeviceRecord,
) -> pkms_sync::ConflictRecord {
    let remote = put("op-remote", "note-1", 0, b"remote");
    apply(
        authority,
        phone_key,
        phone,
        &remote,
        "nonce-conflict-remote-1",
    );
    let local = put("op-local", "note-1", 0, b"local");
    let outcome = apply(
        authority,
        owner_key,
        owner,
        &local,
        "nonce-conflict-local-1",
    );
    assert!(matches!(outcome, ApplyOutcome::Conflict(_)));
    conflicts_for(authority, owner_key, owner, "nonce-conflict-list-1")
        .into_iter()
        .next()
        .expect("conflict")
}

#[test]
fn conflict_is_recorded_and_listed() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    assert_eq!(conflict.op_id, "op-local");
    assert_eq!(conflict.remote_version, 1);
    assert_eq!(conflict.reason, ConflictReason::VersionMismatch);
    assert!(conflict.resolved_at.is_none());
    assert!(conflict.resolution.is_none());

    let local = put("op-local", "note-1", 0, b"local");
    let _ = apply(
        &mut authority,
        &owner_key,
        &owner,
        &local,
        "nonce-conflict-local-2",
    );
    let conflicts = conflicts_for(&mut authority, &owner_key, &owner, "nonce-conflict-list-2");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].conflict_id, conflict.conflict_id);
}

#[test]
fn keep_local_requires_rebased_replacement_and_resolves() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    let error = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepLocal,
                None,
                None,
                "nonce-resolve-keeplocal-1",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            None,
            None,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_op");

    let replacement = put("op-local-rebased", "note-1", 1, b"local");
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &replacement,
            "nonce-resolve-keeplocal-2"
        ),
        ApplyOutcome::Applied(_)
    ));
    let resolved = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepLocal,
                Some("op-local-rebased"),
                None,
                "nonce-resolve-keeplocal-3",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-local-rebased"),
            None,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(resolved.resolution, Some(ConflictResolution::KeepLocal));
    assert!(resolved.resolved_at.is_some());
    assert_eq!(
        resolved.replacement_op_id.as_deref(),
        Some("op-local-rebased")
    );
    assert!(
        conflicts_for(
            &mut authority,
            &owner_key,
            &owner,
            "nonce-resolve-keeplocal-4"
        )
        .is_empty()
    );

    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-resolve-keeplocal-5"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].version, 2);
    assert_eq!(snapshot[0].ciphertext.as_deref(), Some(&b"local"[..]));

    let again = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepLocal,
                Some("op-local-rebased"),
                None,
                "nonce-resolve-keeplocal-6",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-local-rebased"),
            None,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(again.conflict_id, conflict.conflict_id);
    let changed = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
                "nonce-resolve-keeplocal-7",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepRemote,
            None,
            None,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(changed.code(), "conflict_already_resolved");
}

#[test]
fn keep_remote_resolves_without_changing_current_state() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    let resolved = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
                "nonce-resolve-keepremote-1",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepRemote,
            None,
            None,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(resolved.resolution, Some(ConflictResolution::KeepRemote));
    assert!(
        conflicts_for(
            &mut authority,
            &owner_key,
            &owner,
            "nonce-resolve-keepremote-2"
        )
        .is_empty()
    );
    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-resolve-keepremote-3"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot[0].ciphertext.as_deref(), Some(&b"remote"[..]));
}

#[test]
fn save_as_new_creates_a_second_object() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    let copy = put("op-local-copy", "note-1-copy", 0, b"local-copy");
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &copy,
            "nonce-resolve-copy-1"
        ),
        ApplyOutcome::Applied(_)
    ));
    let resolved = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::SaveAsNew,
                Some("op-local-copy"),
                Some("note-1-copy"),
                "nonce-resolve-copy-2",
            ),
            &conflict.conflict_id,
            ConflictResolution::SaveAsNew,
            Some("op-local-copy"),
            Some("note-1-copy"),
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(resolved.resolution, Some(ConflictResolution::SaveAsNew));
    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-resolve-copy-3"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.iter().any(|object| {
        object.object_id == "note-1-copy"
            && object.version == 1
            && object.ciphertext.as_deref() == Some(&b"local-copy"[..])
    }));
    assert!(snapshot.iter().any(|object| {
        object.object_id == "note-1"
            && object.version == 1
            && object.ciphertext.as_deref() == Some(&b"remote"[..])
    }));
}

#[test]
fn save_as_new_rejects_the_original_object_id() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    let error = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::SaveAsNew,
                Some("op-local"),
                Some("note-1"),
                "nonce-resolve-copy-4",
            ),
            &conflict.conflict_id,
            ConflictResolution::SaveAsNew,
            Some("op-local"),
            Some("note-1"),
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_op");
}

#[test]
fn other_device_cannot_resolve_conflict() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
    let (tablet_key, tablet) = pair_device(&mut authority, &owner, "tablet");
    let error = authority
        .resolve_conflict(
            &resolve_request(
                &tablet_key,
                &tablet,
                &conflict.conflict_id,
                ConflictResolution::KeepRemote,
                None,
                None,
                "nonce-resolve-other-1",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepRemote,
            None,
            None,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "conflict_not_found");
}

#[test]
fn keep_local_on_tombstone_requires_restore() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let first = put("op-remote", "note-1", 0, b"remote");
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &first,
        "nonce-conflict-tomb-1",
    );
    let remove = delete("op-remote-delete", "note-1", 1);
    apply(
        &mut authority,
        &phone_key,
        &phone,
        &remove,
        "nonce-conflict-tomb-2",
    );
    let local = put("op-local-tomb", "note-1", 2, b"local");
    let outcome = apply(
        &mut authority,
        &owner_key,
        &owner,
        &local,
        "nonce-conflict-tomb-3",
    );
    assert!(matches!(outcome, ApplyOutcome::Conflict(_)));
    let conflict = conflicts_for(&mut authority, &owner_key, &owner, "nonce-conflict-tomb-4")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(conflict.reason, ConflictReason::Deleted);
    assert!(conflict.remote_tombstone);
    let back = restore("op-local-restore", "note-1", 2, b"local-restored");
    apply(
        &mut authority,
        &owner_key,
        &owner,
        &back,
        "nonce-conflict-tomb-5",
    );
    let resolved = authority
        .resolve_conflict(
            &resolve_request(
                &owner_key,
                &owner,
                &conflict.conflict_id,
                ConflictResolution::KeepLocal,
                Some("op-local-restore"),
                None,
                "nonce-conflict-tomb-6",
            ),
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("op-local-restore"),
            None,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(resolved.resolution, Some(ConflictResolution::KeepLocal));
}

#[test]
fn conflict_record_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.sqlite3");
    let (owner_key, owner, conflict_id) = {
        let mut authority = SyncAuthority::open(&path).unwrap();
        let (owner_key, owner) = enroll_owner(&mut authority);
        let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
        let conflict = make_conflict(&mut authority, &owner_key, &owner, &phone_key, &phone);
        (owner_key, owner, conflict.conflict_id)
    };
    let mut authority = SyncAuthority::open(&path).unwrap();
    let conflicts = conflicts_for(&mut authority, &owner_key, &owner, "nonce-reopen-list-1");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].conflict_id, conflict_id);
}
