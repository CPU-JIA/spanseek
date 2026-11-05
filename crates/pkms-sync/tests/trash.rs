use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictReason, DeviceKeyPair, DeviceRecord, OpKind, SignedRequest, SyncAccess,
    SyncAuthority, SyncOp, op_request_hash, owner_proof_message, pairing_proof_message,
    pull_request_hash, snapshot_request_hash, trash_request_hash,
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

fn purge(op_id: &str, object_id: &str, base_version: i64) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-1".into(),
        object_id: object_id.into(),
        kind: OpKind::Purge,
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

fn pull_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        pull_request_hash("workspace-1", 0, 50),
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

fn trash_request(key: &DeviceKeyPair, device: &DeviceRecord, nonce: &str) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        trash_request_hash("workspace-1"),
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

fn expect_applied(outcome: ApplyOutcome) -> pkms_sync::AppliedOp {
    match outcome {
        ApplyOutcome::Applied(applied) => applied,
        ApplyOutcome::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
    }
}

fn expect_conflict(outcome: ApplyOutcome) -> pkms_sync::Conflict {
    match outcome {
        ApplyOutcome::Conflict(conflict) => conflict,
        ApplyOutcome::Applied(applied) => panic!("unexpected apply: {applied:?}"),
    }
}

fn trash_for(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) -> Vec<pkms_sync::TrashEntry> {
    authority
        .trash(
            &trash_request(key, device, nonce),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap()
}

fn snapshot_for(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) -> Vec<pkms_sync::ObjectState> {
    authority
        .snapshot(
            &snapshot_request(key, device, nonce),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap()
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
fn stale_delete_does_not_remove_remote_edit() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let first = put("op-base", "note-1", 0, b"base");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &first,
        "nonce-trash-base-1",
    ));
    let remote = put("op-remote-edit", "note-1", 1, b"remote edit");
    expect_applied(apply(
        &mut authority,
        &phone_key,
        &phone,
        &remote,
        "nonce-trash-remote-1",
    ));
    let stale_delete = delete("op-stale-delete", "note-1", 1);
    let conflict = expect_conflict(apply(
        &mut authority,
        &owner_key,
        &owner,
        &stale_delete,
        "nonce-trash-delete-1",
    ));
    assert_eq!(conflict.reason, ConflictReason::VersionMismatch);
    assert_eq!(conflict.current_version, 2);
    let snapshot = snapshot_for(&mut authority, &phone_key, &phone, "nonce-trash-snap-1");
    assert_eq!(snapshot.len(), 1);
    assert!(!snapshot[0].tombstone);
    assert_eq!(snapshot[0].version, 2);
    assert_eq!(snapshot[0].ciphertext.as_deref(), Some(&b"remote edit"[..]));
}

#[test]
fn tombstone_prevents_resurrection_until_explicit_restore() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let first = put("op-live", "note-1", 0, b"live");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &first,
        "nonce-trash-live-1",
    ));
    let remove = delete("op-delete", "note-1", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &remove,
        "nonce-trash-delete-2",
    ));
    let stale = put("op-stale-put", "note-1", 1, b"stale");
    let conflict = expect_conflict(apply(
        &mut authority,
        &phone_key,
        &phone,
        &stale,
        "nonce-trash-stale-1",
    ));
    assert_eq!(conflict.reason, ConflictReason::VersionMismatch);
    let known_deleted = put("op-known-deleted", "note-1", 2, b"stale");
    let conflict = expect_conflict(apply(
        &mut authority,
        &phone_key,
        &phone,
        &known_deleted,
        "nonce-trash-stale-2",
    ));
    assert_eq!(conflict.reason, ConflictReason::Deleted);
    let batch = authority
        .pull(
            &pull_request(&phone_key, &phone, "nonce-trash-pull-1"),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(batch.ops.len(), 2);
    assert!(batch.ops[1].tombstone);
    assert!(!batch.ops[1].purged);
    let back = restore("op-restore", "note-1", 2, b"restored");
    expect_applied(apply(
        &mut authority,
        &phone_key,
        &phone,
        &back,
        "nonce-trash-restore-1",
    ));
    let stale_after = put("op-stale-after", "note-1", 2, b"stale");
    let conflict = expect_conflict(apply(
        &mut authority,
        &owner_key,
        &owner,
        &stale_after,
        "nonce-trash-stale-2",
    ));
    assert_eq!(conflict.reason, ConflictReason::VersionMismatch);
}

#[test]
fn trash_lists_deleted_objects_and_restore_removes_entry() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    for (id, name) in [("op-x", "note-x"), ("op-y", "note-y")] {
        let op = put(id, name, 0, name.as_bytes());
        expect_applied(apply(
            &mut authority,
            &owner_key,
            &owner,
            &op,
            &format!("nonce-trash-{id}-1"),
        ));
    }
    let delete_x = delete("op-delete-x", "note-x", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &delete_x,
        "nonce-trash-del-x",
    ));
    let delete_y = delete("op-delete-y", "note-y", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &delete_y,
        "nonce-trash-del-y",
    ));
    let entries = trash_for(&mut authority, &phone_key, &phone, "nonce-trash-list-1");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| !entry.purged));
    assert!(
        entries
            .iter()
            .all(|entry| entry.deleted_by.as_deref() == Some(owner.device_id.as_str()))
    );
    assert!(entries.iter().all(|entry| entry.deleted_at.is_some()));
    let back = restore("op-restore-x", "note-x", 2, b"note-x");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &back,
        "nonce-trash-restore-x",
    ));
    let entries = trash_for(&mut authority, &phone_key, &phone, "nonce-trash-list-2");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].object_id, "note-y");
}

#[test]
fn purge_keeps_tombstone_and_blocks_restore() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let first = put("op-live", "note-1", 0, b"live");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &first,
        "nonce-purge-live-1",
    ));
    let remove = delete("op-delete", "note-1", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &remove,
        "nonce-purge-delete-1",
    ));
    let purge = purge("op-purge", "note-1", 2);
    let applied = expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &purge,
        "nonce-purge-purge-1",
    ));
    assert_eq!(applied.version, 3);
    let entries = trash_for(&mut authority, &phone_key, &phone, "nonce-purge-list-1");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].purged);
    assert_eq!(entries[0].version, 3);
    let restore_attempt = restore("op-restore", "note-1", 3, b"restore");
    let conflict = expect_conflict(apply(
        &mut authority,
        &phone_key,
        &phone,
        &restore_attempt,
        "nonce-purge-restore-1",
    ));
    assert_eq!(conflict.reason, ConflictReason::Purged);
    let put_attempt = put("op-put-stale", "note-1", 3, b"stale");
    let conflict = expect_conflict(apply(
        &mut authority,
        &phone_key,
        &phone,
        &put_attempt,
        "nonce-purge-put-1",
    ));
    assert_eq!(conflict.reason, ConflictReason::Deleted);
    let snapshot = snapshot_for(&mut authority, &phone_key, &phone, "nonce-purge-snap-1");
    assert!(snapshot[0].tombstone);
    assert!(snapshot[0].purged);
    assert!(snapshot[0].ciphertext.is_none());
    assert_eq!(snapshot[0].version, 3);
}

#[test]
fn purge_requires_a_tombstone() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let first = put("op-live", "note-1", 0, b"live");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &first,
        "nonce-purge-live-2",
    ));
    let early = purge("op-purge-live", "note-1", 1);
    let conflict = expect_conflict(apply(
        &mut authority,
        &owner_key,
        &owner,
        &early,
        "nonce-purge-live-3",
    ));
    assert_eq!(conflict.reason, ConflictReason::AlreadyLive);
    let missing = purge("op-purge-missing", "note-missing", 1);
    let conflict = expect_conflict(apply(
        &mut authority,
        &owner_key,
        &owner,
        &missing,
        "nonce-purge-missing-1",
    ));
    assert_eq!(conflict.reason, ConflictReason::NotFound);
}

#[test]
fn deleting_one_object_does_not_touch_remote_only_objects() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let x = put("op-x", "note-x", 0, b"x");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &x,
        "nonce-remote-only-1",
    ));
    let y = put("op-y", "note-y", 0, b"y");
    expect_applied(apply(
        &mut authority,
        &phone_key,
        &phone,
        &y,
        "nonce-remote-only-2",
    ));
    let remove = delete("op-delete-x", "note-x", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &remove,
        "nonce-remote-only-3",
    ));
    let snapshot = snapshot_for(&mut authority, &owner_key, &owner, "nonce-remote-only-4");
    assert_eq!(snapshot.len(), 2);
    let x = snapshot
        .iter()
        .find(|object| object.object_id == "note-x")
        .unwrap();
    let y = snapshot
        .iter()
        .find(|object| object.object_id == "note-y")
        .unwrap();
    assert!(x.tombstone);
    assert!(!y.tombstone);
    assert_eq!(y.ciphertext.as_deref(), Some(&b"y"[..]));
    let entries = trash_for(&mut authority, &owner_key, &owner, "nonce-remote-only-5");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].object_id, "note-x");
}

#[test]
fn trash_access_is_authorized() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let write = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Write,
        "nonce-trash-access-1",
        Utc::now().timestamp(),
        trash_request_hash("workspace-1"),
    );
    assert_eq!(
        authority
            .trash(&write, "workspace-1", Utc::now().timestamp())
            .unwrap_err()
            .code(),
        "invalid_access"
    );
    let wrong_hash = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Read,
        "nonce-trash-access-2",
        Utc::now().timestamp(),
        [0u8; 32],
    );
    assert_eq!(
        authority
            .trash(&wrong_hash, "workspace-1", Utc::now().timestamp())
            .unwrap_err()
            .code(),
        "invalid_request_hash"
    );
}

#[test]
fn v3_database_migrates_to_current_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v3-authority.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../src/schema.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../src/migrations/0002_oplog.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../src/migrations/0003_conflicts.sql"))
            .unwrap();
    }
    let mut authority = SyncAuthority::open(&path).unwrap();
    assert_eq!(authority.schema_version().unwrap(), 9);
    let (owner_key, owner) = enroll_owner(&mut authority);
    let first = put("op-migrated", "note-1", 0, b"live");
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &first,
        "nonce-migrate-trash-1",
    ));
    let remove = delete("op-migrated-delete", "note-1", 1);
    expect_applied(apply(
        &mut authority,
        &owner_key,
        &owner,
        &remove,
        "nonce-migrate-trash-2",
    ));
    let entries = trash_for(&mut authority, &owner_key, &owner, "nonce-migrate-trash-3");
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].purged);
}
