use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, KeyEnvelope, OpKind, SealedData, SyncAccess,
    SyncAuthority, SyncOp, WorkspaceKey, device_encryption_key_request_hash,
    key_envelope_request_hash, object_aad, op_request_hash, owner_proof_message,
    pairing_proof_message, pull_request_hash, workspace_key_aad,
    workspace_key_rotation_request_hash, wrap_workspace_key,
};

const WORKSPACE: &str = "workspace-keys";

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

fn register_key(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) {
    let public = key.encryption_public_key_hex().unwrap();
    let request = key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        device_encryption_key_request_hash(&public),
    );
    authority
        .register_encryption_key(&request, &public, Utc::now().timestamp())
        .unwrap();
}

fn envelope_for(
    key: &WorkspaceKey,
    device_key: &DeviceKeyPair,
    device: &DeviceRecord,
    epoch: i64,
) -> KeyEnvelope {
    let public = device_key.encryption_public_key().unwrap();
    let aad = workspace_key_aad(WORKSPACE, epoch, &device.device_id);
    wrap_workspace_key(public, key, &aad).unwrap()
}

fn rotate(
    authority: &mut SyncAuthority,
    actor_key: &DeviceKeyPair,
    actor: &DeviceRecord,
    epoch: i64,
    envelopes: &[(String, KeyEnvelope)],
    nonce: &str,
) -> Result<i64, pkms_sync::SyncError> {
    let request = actor_key.sign_request(
        &actor.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        workspace_key_rotation_request_hash(WORKSPACE, epoch, envelopes),
    );
    authority.rotate_workspace_key(
        &request,
        WORKSPACE,
        epoch,
        envelopes,
        Utc::now().timestamp(),
    )
}

fn fetch(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    epoch: i64,
    nonce: &str,
) -> Result<KeyEnvelope, pkms_sync::SyncError> {
    let request = key.sign_request(
        &device.device_id,
        SyncAccess::Read,
        nonce,
        Utc::now().timestamp(),
        key_envelope_request_hash(WORKSPACE, epoch),
    );
    authority.fetch_key_envelope(&request, WORKSPACE, epoch, Utc::now().timestamp())
}

fn apply(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    op: &SyncOp,
    nonce: &str,
) -> Result<ApplyOutcome, pkms_sync::SyncError> {
    let request = key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        op_request_hash(op),
    );
    authority.apply_op(&request, op, Utc::now().timestamp())
}

fn op(op_id: &str, epoch: i64, ciphertext: Vec<u8>) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: WORKSPACE.into(),
        object_id: "note-1".into(),
        kind: OpKind::Put,
        base_version: 0,
        key_epoch: epoch,
        ciphertext: Some(ciphertext),
        client_time: None,
    }
}

#[test]
fn rotation_revocation_and_plaintext_key_isolation() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("authority.sqlite3");
    let mut authority = SyncAuthority::open(&db).unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    register_key(
        &mut authority,
        &owner_key,
        &owner,
        "nonce-key-register-owner-1",
    );
    register_key(
        &mut authority,
        &phone_key,
        &phone,
        "nonce-key-register-phone-1",
    );

    let key1_bytes = [
        0x11, 0x23, 0x35, 0x47, 0x59, 0x6b, 0x7d, 0x8f, 0x91, 0xa3, 0xb5, 0xc7, 0xd9, 0xeb, 0xfd,
        0x1f, 0x21, 0x33, 0x45, 0x57, 0x69, 0x7b, 0x8d, 0x9f, 0xa1, 0xb3, 0xc5, 0xd7, 0xe9, 0xfb,
        0x0d, 0x1e,
    ];
    let key1 = WorkspaceKey::from_bytes(key1_bytes);
    let envelopes = vec![
        (
            owner.device_id.clone(),
            envelope_for(&key1, &owner_key, &owner, 1),
        ),
        (
            phone.device_id.clone(),
            envelope_for(&key1, &phone_key, &phone, 1),
        ),
    ];
    assert_eq!(
        rotate(
            &mut authority,
            &owner_key,
            &owner,
            1,
            &envelopes,
            "nonce-key-rotate-1"
        )
        .unwrap(),
        1
    );
    assert_eq!(authority.current_key_epoch(WORKSPACE).unwrap(), Some(1));

    let owner_envelope = fetch(
        &mut authority,
        &owner_key,
        &owner,
        1,
        "nonce-key-fetch-owner-1",
    )
    .unwrap();
    let owner_unwrapped = owner_key
        .unwrap_workspace_key(
            &owner_envelope,
            &workspace_key_aad(WORKSPACE, 1, &owner.device_id),
        )
        .unwrap();
    assert!(owner_unwrapped.matches(&key1));
    let sealed = key1
        .encrypt(b"secret note body", &object_aad(WORKSPACE, "note-1", 1))
        .unwrap();
    let first = op("op-key-1", 1, sealed.encode());
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &first,
            "nonce-key-apply-1"
        )
        .unwrap(),
        ApplyOutcome::Applied(_)
    ));

    let phone_envelope = fetch(
        &mut authority,
        &phone_key,
        &phone,
        1,
        "nonce-key-fetch-phone-1",
    )
    .unwrap();
    let phone_unwrapped = phone_key
        .unwrap_workspace_key(
            &phone_envelope,
            &workspace_key_aad(WORKSPACE, 1, &phone.device_id),
        )
        .unwrap();
    let batch = authority
        .pull(
            &phone_key.sign_request(
                &phone.device_id,
                SyncAccess::Read,
                "nonce-key-pull-phone-1",
                Utc::now().timestamp(),
                pull_request_hash(WORKSPACE, 0, 50),
            ),
            WORKSPACE,
            0,
            50,
            Utc::now().timestamp(),
        )
        .unwrap();
    let remote = SealedData::decode(batch.ops[0].ciphertext.as_deref().unwrap()).unwrap();
    assert_eq!(
        phone_unwrapped
            .decrypt(&remote, &object_aad(WORKSPACE, "note-1", 1))
            .unwrap(),
        b"secret note body"
    );

    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost phone")
        .unwrap();
    let revoked_fetch = fetch(
        &mut authority,
        &phone_key,
        &phone,
        1,
        "nonce-key-fetch-revoked-1",
    )
    .unwrap_err();
    assert_eq!(revoked_fetch.code(), "device_revoked");

    let key2_bytes = [
        0x7a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8,
        0x09, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7,
        0xf8, 0x09,
    ];
    let key2 = WorkspaceKey::from_bytes(key2_bytes);
    let owner_envelopes = vec![(
        owner.device_id.clone(),
        envelope_for(&key2, &owner_key, &owner, 2),
    )];
    assert_eq!(
        rotate(
            &mut authority,
            &owner_key,
            &owner,
            2,
            &owner_envelopes,
            "nonce-key-rotate-2"
        )
        .unwrap(),
        2
    );
    assert_eq!(authority.current_key_epoch(WORKSPACE).unwrap(), Some(2));
    let epoch2_envelope = fetch(
        &mut authority,
        &owner_key,
        &owner,
        2,
        "nonce-key-fetch-epoch-2",
    )
    .unwrap();
    let epoch2_key = owner_key
        .unwrap_workspace_key(
            &epoch2_envelope,
            &workspace_key_aad(WORKSPACE, 2, &owner.device_id),
        )
        .unwrap();
    assert!(epoch2_key.matches(&key2));

    let sealed2 = key2
        .encrypt(b"new epoch body", &object_aad(WORKSPACE, "note-1", 2))
        .unwrap();
    let mut second = op("op-key-2", 2, sealed2.encode());
    second.base_version = 1;
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &second,
            "nonce-key-apply-2"
        )
        .unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let stale = op("op-key-stale", 1, sealed2.encode());
    let error = apply(
        &mut authority,
        &owner_key,
        &owner,
        &stale,
        "nonce-key-apply-stale-1",
    )
    .unwrap_err();
    assert_eq!(error.code(), "key_epoch_mismatch");

    let db_bytes = std::fs::read(&db).unwrap();
    assert!(
        !db_bytes
            .windows(key1_bytes.len())
            .any(|window| window == key1_bytes)
    );
    assert!(
        !db_bytes
            .windows(key2_bytes.len())
            .any(|window| window == key2_bytes)
    );
}

#[test]
fn rotation_requires_every_active_device_to_register_an_encryption_key() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (_phone_key, _phone) = pair_device(&mut authority, &owner, "phone");
    register_key(
        &mut authority,
        &owner_key,
        &owner,
        "nonce-key-missing-owner-1",
    );
    let key = WorkspaceKey::from_bytes([3u8; 32]);
    let envelopes = vec![(
        owner.device_id.clone(),
        envelope_for(&key, &owner_key, &owner, 1),
    )];
    let error = rotate(
        &mut authority,
        &owner_key,
        &owner,
        1,
        &envelopes,
        "nonce-key-missing-rotate-1",
    )
    .unwrap_err();
    assert_eq!(error.code(), "missing_encryption_key");
}

#[test]
fn rotation_rejects_malformed_envelopes() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    register_key(&mut authority, &owner_key, &owner, "nonce-key-bad-owner-1");
    let mut envelope = envelope_for(&WorkspaceKey::from_bytes([4u8; 32]), &owner_key, &owner, 1);
    envelope.ciphertext = vec![0u8; 10];
    let envelopes = vec![(owner.device_id.clone(), envelope)];
    let error = rotate(
        &mut authority,
        &owner_key,
        &owner,
        1,
        &envelopes,
        "nonce-key-bad-rotate-1",
    )
    .unwrap_err();
    assert_eq!(error.code(), "invalid_key_envelope");
}

#[test]
fn sealed_data_rejects_wrong_aad_and_tampering() {
    let key = WorkspaceKey::from_bytes([5u8; 32]);
    let sealed = key.encrypt(b"payload", b"aad-one").unwrap();
    assert_eq!(key.decrypt(&sealed, b"aad-one").unwrap(), b"payload");
    assert!(key.decrypt(&sealed, b"aad-two").is_err());
    let mut tampered = sealed.clone();
    tampered.ciphertext[0] ^= 0xff;
    assert!(key.decrypt(&tampered, b"aad-one").is_err());
    assert_eq!(SealedData::decode(&sealed.encode()).unwrap(), sealed);
}
