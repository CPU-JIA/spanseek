use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictReason, DeviceKeyPair, DeviceRecord, MAX_CIPHERTEXT_BYTES, OpKind,
    SignedRequest, SyncAccess, SyncAuthority, SyncOp, op_request_hash, owner_proof_message,
    pairing_proof_message, pull_request_hash, snapshot_request_hash,
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

#[test]
fn pull_conflicts_are_workspace_scoped_before_the_fifty_record_limit() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "multi-workspace phone");
    // The requested workspace's conflict is oldest. Fifty newer conflicts from
    // other workspaces must neither leak nor push it out of the result limit.
    for index in 0..=50 {
        let workspace = if index == 0 {
            "workspace-1".to_owned()
        } else {
            format!("workspace-other-{index}")
        };
        let mut initial = put(
            &format!("scoped-initial-{index}"),
            "same-note",
            0,
            b"remote",
        );
        initial.workspace_id = workspace.clone();
        expect_applied(
            authority
                .apply_op(
                    &write_request(
                        &owner_key,
                        &owner,
                        &initial,
                        &format!("scoped-initial-nonce-{index}"),
                    ),
                    &initial,
                    Utc::now().timestamp(),
                )
                .unwrap(),
        );
        let mut stale = put(&format!("scoped-stale-{index}"), "same-note", 0, b"local");
        stale.workspace_id = workspace;
        expect_conflict(
            authority
                .apply_op(
                    &write_request(
                        &phone_key,
                        &phone,
                        &stale,
                        &format!("scoped-stale-nonce-{index}"),
                    ),
                    &stale,
                    Utc::now().timestamp(),
                )
                .unwrap(),
        );
    }
    for (index, workspace) in ["workspace-1", "workspace-other-50", "workspace-empty"]
        .into_iter()
        .enumerate()
    {
        let request = phone_key.sign_request(
            &phone.device_id,
            SyncAccess::Read,
            &format!("scoped-pull-nonce-{index}"),
            Utc::now().timestamp(),
            pull_request_hash(workspace, 0, 10),
        );
        let batch = authority
            .pull(&request, workspace, 0, 10, Utc::now().timestamp())
            .unwrap();
        assert_eq!(batch.conflicts.len(), usize::from(index < 2));
        assert!(batch.conflicts.iter().all(|conflict| {
            conflict.workspace_id == workspace
                && conflict.device_id == phone.device_id
                && conflict.object_id == "same-note"
        }));
    }
}

#[test]
fn put_and_pull_roundtrip() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    let op = put("op-1", "note-1", 0, b"hello");
    let applied = expect_applied(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &op, "nonce-put-0000001"),
                &op,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(applied.version, 1);
    assert_eq!(applied.server_seq, 1);
    assert!(!applied.duplicate);

    let batch = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 50, "nonce-pull-owner-01"),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.ops[0].ciphertext.as_deref(), Some(&b"hello"[..]));
    assert_eq!(batch.ops[0].kind, OpKind::Put);
    assert!(!batch.ops[0].tombstone);
    assert_eq!(batch.next_cursor, 1);
    assert!(!batch.has_more);

    let snapshot = authority
        .snapshot(
            &snapshot_request(&phone_key, &phone, "nonce-snap-owner-01"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].version, 1);
    assert_eq!(snapshot[0].object_id, "note-1");
}

#[test]
fn duplicate_op_retry_is_idempotent() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let op = put("op-dup", "note-1", 0, b"one");
    let request = write_request(&owner_key, &owner, &op, "nonce-dup-0000001");
    let first = expect_applied(
        authority
            .apply_op(&request, &op, Utc::now().timestamp())
            .unwrap(),
    );
    let second = expect_applied(
        authority
            .apply_op(&request, &op, Utc::now().timestamp())
            .unwrap(),
    );
    assert!(second.duplicate);
    assert_eq!(second.server_seq, first.server_seq);
    assert_eq!(second.version, first.version);

    let retry = write_request(&owner_key, &owner, &op, "nonce-dup-0000002");
    let third = expect_applied(
        authority
            .apply_op(&retry, &op, Utc::now().timestamp())
            .unwrap(),
    );
    assert!(third.duplicate);
    assert_eq!(third.server_seq, first.server_seq);
}

#[test]
fn op_id_reuse_with_different_payload_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let first = put("op-reuse", "note-1", 0, b"one");
    authority
        .apply_op(
            &write_request(&owner_key, &owner, &first, "nonce-reuse-000001"),
            &first,
            Utc::now().timestamp(),
        )
        .unwrap();
    let second = put("op-reuse", "note-1", 1, b"two");
    let error = authority
        .apply_op(
            &write_request(&owner_key, &owner, &second, "nonce-reuse-000002"),
            &second,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "op_id_reused");
}

#[test]
fn stale_base_conflicts_without_overwrite() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    let first = put("op-first", "note-1", 0, b"phone-version");
    expect_applied(
        authority
            .apply_op(
                &write_request(&phone_key, &phone, &first, "nonce-conflict-0001"),
                &first,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    let stale = put("op-stale", "note-1", 0, b"owner-version");
    let conflict = expect_conflict(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &stale, "nonce-conflict-0002"),
                &stale,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(conflict.reason, ConflictReason::VersionMismatch);
    assert_eq!(conflict.current_version, 1);

    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-snap-conflict-1"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(
        snapshot[0].ciphertext.as_deref(),
        Some(&b"phone-version"[..])
    );
}

#[test]
fn delete_tombstone_and_restore() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let first = put("op-put", "note-1", 0, b"live");
    authority
        .apply_op(
            &write_request(&owner_key, &owner, &first, "nonce-tomb-000001"),
            &first,
            Utc::now().timestamp(),
        )
        .unwrap();
    let remove = delete("op-delete", "note-1", 1);
    let applied = expect_applied(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &remove, "nonce-tomb-000002"),
                &remove,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(applied.version, 2);

    let batch = authority
        .pull(
            &pull_request(&owner_key, &owner, 0, 50, "nonce-tomb-pull-0001"),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(batch.ops.len(), 2);
    assert!(batch.ops[1].tombstone);
    assert!(batch.ops[1].ciphertext.is_none());

    let late = put("op-late", "note-1", 2, b"late");
    let conflict = expect_conflict(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &late, "nonce-tomb-000003"),
                &late,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(conflict.reason, ConflictReason::Deleted);

    let back = restore("op-restore", "note-1", 2, b"restored");
    let applied = expect_applied(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &back, "nonce-tomb-000004"),
                &back,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(applied.version, 3);
    let snapshot = authority
        .snapshot(
            &snapshot_request(&owner_key, &owner, "nonce-tomb-snap-0001"),
            "workspace-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(!snapshot[0].tombstone);
    assert_eq!(snapshot[0].ciphertext.as_deref(), Some(&b"restored"[..]));
}

#[test]
fn revoked_device_cannot_apply_or_pull() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    let op = put("op-phone", "note-1", 0, b"phone");
    authority
        .apply_op(
            &write_request(&phone_key, &phone, &op, "nonce-revoke-000001"),
            &op,
            Utc::now().timestamp(),
        )
        .unwrap();
    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost")
        .unwrap();
    let next = put("op-phone-2", "note-1", 1, b"later");
    let write_error = authority
        .apply_op(
            &write_request(&phone_key, &phone, &next, "nonce-revoke-000002"),
            &next,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(write_error.code(), "device_revoked");
    let pull_error = authority
        .pull(
            &pull_request(&phone_key, &phone, 0, 50, "nonce-revoke-000003"),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(pull_error.code(), "device_revoked");

    let owner_next = put("op-owner-2", "note-1", 1, b"owner");
    expect_applied(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &owner_next, "nonce-revoke-000004"),
                &owner_next,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
}

#[test]
fn pull_cursor_and_limit() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    for (index, payload) in [b"a", b"b", b"c"].iter().enumerate() {
        let op = put(
            &format!("op-cursor-{index}"),
            "note-1",
            index as i64,
            *payload,
        );
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &op, &format!("nonce-cursor-{index:04}")),
                &op,
                Utc::now().timestamp(),
            )
            .unwrap();
    }
    let first = authority
        .pull(
            &pull_request(&owner_key, &owner, 0, 2, "nonce-cursor-pull-01"),
            "workspace-1",
            0,
            2,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(first.ops.len(), 2);
    assert!(first.has_more);
    assert_eq!(first.next_cursor, 2);
    let second = authority
        .pull(
            &pull_request(&owner_key, &owner, 2, 2, "nonce-cursor-pull-02"),
            "workspace-1",
            2,
            2,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(second.ops.len(), 1);
    assert!(!second.has_more);
    assert_eq!(second.next_cursor, 3);
}

#[test]
fn request_hash_must_match_payload() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let op = put("op-hash", "note-1", 0, b"x");
    let bad = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Write,
        "nonce-hash-0000001",
        Utc::now().timestamp(),
        [0u8; 32],
    );
    let error = authority
        .apply_op(&bad, &op, Utc::now().timestamp())
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request_hash");

    let bad_pull = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Read,
        "nonce-hash-0000002",
        Utc::now().timestamp(),
        [0u8; 32],
    );
    let error = authority
        .pull(&bad_pull, "workspace-1", 0, 50, Utc::now().timestamp())
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request_hash");
}

#[test]
fn access_kind_is_enforced() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let op = put("op-access", "note-1", 0, b"x");
    let read_for_write = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Read,
        "nonce-access-000001",
        Utc::now().timestamp(),
        op_request_hash(&op),
    );
    assert_eq!(
        authority
            .apply_op(&read_for_write, &op, Utc::now().timestamp())
            .unwrap_err()
            .code(),
        "invalid_access"
    );
    let write_for_read = owner_key.sign_request(
        &owner.device_id,
        SyncAccess::Write,
        "nonce-access-000002",
        Utc::now().timestamp(),
        pull_request_hash("workspace-1", 0, 50),
    );
    assert_eq!(
        authority
            .pull(
                &write_for_read,
                "workspace-1",
                0,
                50,
                Utc::now().timestamp()
            )
            .unwrap_err()
            .code(),
        "invalid_access"
    );
}

#[test]
fn client_time_does_not_order_ops() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let mut first = put("op-time-1", "note-1", 0, b"first");
    first.client_time = Some("2100-01-01T00:00:00Z".into());
    authority
        .apply_op(
            &write_request(&owner_key, &owner, &first, "nonce-time-0000001"),
            &first,
            Utc::now().timestamp(),
        )
        .unwrap();
    let mut second = put("op-time-2", "note-1", 1, b"second");
    second.client_time = Some("1990-01-01T00:00:00Z".into());
    authority
        .apply_op(
            &write_request(&owner_key, &owner, &second, "nonce-time-0000002"),
            &second,
            Utc::now().timestamp(),
        )
        .unwrap();
    let batch = authority
        .pull(
            &pull_request(&owner_key, &owner, 0, 50, "nonce-time-pull-001"),
            "workspace-1",
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(batch.ops[0].op_id, "op-time-1");
    assert_eq!(batch.ops[1].op_id, "op-time-2");
    assert!(batch.ops[0].server_seq < batch.ops[1].server_seq);
}

#[test]
fn oversize_ciphertext_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let op = put(
        "op-large",
        "note-1",
        0,
        &vec![0u8; MAX_CIPHERTEXT_BYTES + 1],
    );
    let error = authority
        .apply_op(
            &write_request(&owner_key, &owner, &op, "nonce-large-000001"),
            &op,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_op");
}

#[test]
fn v1_authority_database_migrates_to_current_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v1-authority.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../src/schema.sql"))
            .unwrap();
    }
    let mut authority = SyncAuthority::open(&path).unwrap();
    assert_eq!(authority.schema_version().unwrap(), 9);
    let (owner_key, owner) = enroll_owner(&mut authority);
    let op = put("op-migrated", "note-1", 0, b"after migration");
    let applied = expect_applied(
        authority
            .apply_op(
                &write_request(&owner_key, &owner, &op, "nonce-migrate-00001"),
                &op,
                Utc::now().timestamp(),
            )
            .unwrap(),
    );
    assert_eq!(applied.version, 1);
}
