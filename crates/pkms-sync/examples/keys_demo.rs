//! Reproducible G08 evidence: epoch-1 object encryption/decryption, device
//! revocation, epoch-2 rotation, stale-epoch rejection and at-rest key scan.

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, DeviceKeyPair, DeviceRecord, KeyEnvelope, OpKind, SealedData, SyncAccess,
    SyncAuthority, SyncOp, WorkspaceKey, device_encryption_key_request_hash,
    key_envelope_request_hash, object_aad, op_request_hash, owner_proof_message,
    pairing_proof_message, pull_request_hash, workspace_key_aad,
    workspace_key_rotation_request_hash, wrap_workspace_key,
};
use serde_json::json;

const WORKSPACE: &str = "workspace-security";

fn enroll_owner(authority: &mut SyncAuthority) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().expect("owner key");
    let proof = key.sign(&owner_proof_message(&key.public_key_hex(), "owner"));
    let device = authority
        .enroll_owner(&key.public_key(), "owner", &proof)
        .expect("owner");
    (key, device)
}

fn pair_device(
    authority: &mut SyncAuthority,
    creator: &DeviceRecord,
    name: &str,
) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().expect("phone key");
    let invite = authority
        .create_pairing_invite(&creator.device_id, 600)
        .expect("invite");
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
        .expect("pair");
    (key, device)
}

fn register_key(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    nonce: &str,
) {
    let public = key.encryption_public_key_hex().expect("public");
    let request = key.sign_request(
        &device.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        device_encryption_key_request_hash(&public),
    );
    authority
        .register_encryption_key(&request, &public, Utc::now().timestamp())
        .expect("register key");
}

fn envelope_for(
    key: &WorkspaceKey,
    device_key: &DeviceKeyPair,
    device: &DeviceRecord,
    epoch: i64,
) -> KeyEnvelope {
    let public = device_key
        .encryption_public_key()
        .expect("encryption public");
    let aad = workspace_key_aad(WORKSPACE, epoch, &device.device_id);
    wrap_workspace_key(public, key, &aad).expect("wrap")
}

fn rotate(
    authority: &mut SyncAuthority,
    actor_key: &DeviceKeyPair,
    actor: &DeviceRecord,
    epoch: i64,
    envelopes: &[(String, KeyEnvelope)],
    nonce: &str,
) -> i64 {
    let request = actor_key.sign_request(
        &actor.device_id,
        SyncAccess::Write,
        nonce,
        Utc::now().timestamp(),
        workspace_key_rotation_request_hash(WORKSPACE, epoch, envelopes),
    );
    authority
        .rotate_workspace_key(
            &request,
            WORKSPACE,
            epoch,
            envelopes,
            Utc::now().timestamp(),
        )
        .expect("rotate")
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

fn main() {
    let directory = tempfile::tempdir().expect("tempdir");
    let db = directory.path().join("authority.sqlite3");
    let mut authority = SyncAuthority::open(&db).expect("authority");
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    register_key(&mut authority, &owner_key, &owner, "nonce-demo-key-owner-1");
    register_key(&mut authority, &phone_key, &phone, "nonce-demo-key-phone-1");

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
            "nonce-demo-key-rotate-1"
        ),
        1
    );

    let owner_envelope = fetch(
        &mut authority,
        &owner_key,
        &owner,
        1,
        "nonce-demo-key-fetch-1",
    )
    .expect("owner envelope");
    let owner_key1 = owner_key
        .unwrap_workspace_key(
            &owner_envelope,
            &workspace_key_aad(WORKSPACE, 1, &owner.device_id),
        )
        .expect("unwrap owner");
    let sealed = owner_key1
        .encrypt(b"secret body", &object_aad(WORKSPACE, "note-1", 1))
        .expect("encrypt");
    let op1 = SyncOp {
        op_id: "op-demo-key-1".into(),
        workspace_id: WORKSPACE.into(),
        object_id: "note-1".into(),
        kind: OpKind::Put,
        base_version: 0,
        key_epoch: 1,
        ciphertext: Some(sealed.encode()),
        client_time: None,
    };
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &op1,
            "nonce-demo-key-apply-1"
        ),
        Ok(ApplyOutcome::Applied(_))
    ));
    let phone_envelope = fetch(
        &mut authority,
        &phone_key,
        &phone,
        1,
        "nonce-demo-key-fetch-2",
    )
    .expect("phone envelope");
    let phone_key1 = phone_key
        .unwrap_workspace_key(
            &phone_envelope,
            &workspace_key_aad(WORKSPACE, 1, &phone.device_id),
        )
        .expect("unwrap phone");
    let batch = authority
        .pull(
            &phone_key.sign_request(
                &phone.device_id,
                SyncAccess::Read,
                "nonce-demo-key-pull-1",
                Utc::now().timestamp(),
                pull_request_hash(WORKSPACE, 0, 50),
            ),
            WORKSPACE,
            0,
            50,
            Utc::now().timestamp(),
        )
        .expect("pull");
    let phone_plaintext = phone_key1
        .decrypt(
            &SealedData::decode(batch.ops[0].ciphertext.as_deref().expect("ciphertext"))
                .expect("decode"),
            &object_aad(WORKSPACE, "note-1", 1),
        )
        .expect("decrypt phone");

    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost phone")
        .expect("revoke");
    let revoked_fetch = fetch(
        &mut authority,
        &phone_key,
        &phone,
        1,
        "nonce-demo-key-fetch-3",
    )
    .expect_err("revoked fetch");

    let key2_bytes = [
        0x7a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8,
        0x09, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7,
        0xf8, 0x09,
    ];
    let key2 = WorkspaceKey::from_bytes(key2_bytes);
    let owner_only = vec![(
        owner.device_id.clone(),
        envelope_for(&key2, &owner_key, &owner, 2),
    )];
    assert_eq!(
        rotate(
            &mut authority,
            &owner_key,
            &owner,
            2,
            &owner_only,
            "nonce-demo-key-rotate-2"
        ),
        2
    );
    let epoch2_envelope = fetch(
        &mut authority,
        &owner_key,
        &owner,
        2,
        "nonce-demo-key-fetch-4",
    )
    .expect("epoch2 envelope");
    let owner_key2 = owner_key
        .unwrap_workspace_key(
            &epoch2_envelope,
            &workspace_key_aad(WORKSPACE, 2, &owner.device_id),
        )
        .expect("unwrap epoch2");
    let sealed2 = owner_key2
        .encrypt(b"new epoch body", &object_aad(WORKSPACE, "note-1", 2))
        .expect("encrypt epoch2");
    let mut op2 = SyncOp {
        op_id: "op-demo-key-2".into(),
        workspace_id: WORKSPACE.into(),
        object_id: "note-1".into(),
        kind: OpKind::Put,
        base_version: 1,
        key_epoch: 2,
        ciphertext: Some(sealed2.encode()),
        client_time: None,
    };
    assert!(matches!(
        apply(
            &mut authority,
            &owner_key,
            &owner,
            &op2,
            "nonce-demo-key-apply-2"
        ),
        Ok(ApplyOutcome::Applied(_))
    ));
    op2.op_id = "op-demo-key-stale".into();
    op2.base_version = 2;
    op2.key_epoch = 1;
    let stale = apply(
        &mut authority,
        &owner_key,
        &owner,
        &op2,
        "nonce-demo-key-apply-3",
    )
    .expect_err("stale epoch");

    let db_bytes = std::fs::read(&db).expect("db bytes");
    let contains_key1 = db_bytes
        .windows(key1_bytes.len())
        .any(|window| window == key1_bytes);
    let contains_key2 = db_bytes
        .windows(key2_bytes.len())
        .any(|window| window == key2_bytes);

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-security-demo-v1",
            "epoch1": 1,
            "phone_decrypted_epoch1": phone_plaintext == b"secret body",
            "revoked_fetch_error": revoked_fetch.code(),
            "epoch2": 2,
            "owner_can_unwrap_epoch2": true,
            "stale_epoch_op": stale.code(),
            "plaintext_key_in_db": contains_key1 || contains_key2,
            "db_bytes": db_bytes.len(),
        }))
        .expect("json")
    );
}
