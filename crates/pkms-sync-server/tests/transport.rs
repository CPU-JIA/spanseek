use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use pkms_sync::{
    ApplyOutcome, ConflictResolution, DeviceKeyPair, DeviceRecord, DeviceStatus, DraftHandoffInput,
    MemberRole, MemberStatus, OpKind, ReadingPositionInput, ResumeLocator, SealedData,
    SyncAuthority, SyncOp, WorkspaceKey, encryption_public_key_hex, object_aad,
    owner_proof_message, pairing_proof_message, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::{ClientError, SyncClient};

const OWNER_SEED: [u8; 32] = [11u8; 32];
const PHONE_SEED: [u8; 32] = [22u8; 32];

struct Server {
    base_url: String,
}

fn setup() -> (
    SyncAuthority,
    DeviceKeyPair,
    DeviceRecord,
    DeviceKeyPair,
    DeviceRecord,
) {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let owner_proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "JIA desktop",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "JIA desktop", &owner_proof)
        .unwrap();

    let phone_key = DeviceKeyPair::from_seed(PHONE_SEED).unwrap();
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .unwrap();
    let proof = phone_key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &phone_key.public_key_hex(),
    ));
    let phone = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &phone_key.public_key(),
            "JIA phone",
            &proof,
        )
        .unwrap();
    (authority, owner_key, owner, phone_key, phone)
}

fn start_server(authority: SyncAuthority) -> Server {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(
                listener,
                pkms_sync_server::router(Arc::new(Mutex::new(authority))),
            )
            .await
            .unwrap();
        });
    });

    let base_url = format!("http://{address}");
    let probe = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    for _ in 0..200 {
        if probe
            .get(format!("{base_url}/healthz"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Server { base_url };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("sync server did not become ready");
}

fn op(
    op_id: &str,
    object_id: &str,
    kind: OpKind,
    base_version: i64,
    ciphertext: Option<&[u8]>,
) -> SyncOp {
    SyncOp {
        op_id: op_id.into(),
        workspace_id: "workspace-a".into(),
        object_id: object_id.into(),
        kind,
        base_version,
        key_epoch: 1,
        ciphertext: ciphertext.map(ToOwned::to_owned),
        client_time: Some("2026-10-18T12:00:00Z".into()),
    }
}

#[test]
fn health_and_two_way_object_round_trip() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let health = desktop.health().unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.protocol_version, 3);
    assert_eq!(health.schema_version, 9);

    let first = desktop
        .apply_op(&op(
            "op-desktop-1",
            "note-1",
            OpKind::Put,
            0,
            Some(b"from-desktop"),
        ))
        .unwrap();
    let ApplyOutcome::Applied(first) = first else {
        panic!("first put must apply");
    };
    assert_eq!(first.version, 1);

    let batch = mobile.pull("workspace-a", 0, 10).unwrap();
    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.ops[0].op_id, "op-desktop-1");
    assert_eq!(
        batch.ops[0].ciphertext.as_deref(),
        Some(&b"from-desktop"[..])
    );
    let cursor = batch.next_cursor;

    let second = mobile
        .apply_op(&op(
            "op-phone-1",
            "note-1",
            OpKind::Put,
            1,
            Some(b"from-phone"),
        ))
        .unwrap();
    let ApplyOutcome::Applied(second) = second else {
        panic!("second put must apply");
    };
    assert_eq!(second.version, 2);

    let batch = desktop.pull("workspace-a", cursor, 10).unwrap();
    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.ops[0].device_id, phone.device_id);
    assert_eq!(batch.ops[0].ciphertext.as_deref(), Some(&b"from-phone"[..]));

    let objects = desktop.snapshot("workspace-a").unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].version, 2);
    assert_eq!(objects[0].ciphertext.as_deref(), Some(&b"from-phone"[..]));
}

#[test]
fn conflict_is_visible_and_keep_local_can_be_resolved_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    desktop
        .apply_op(&op("base", "note-c", OpKind::Put, 0, Some(b"base")))
        .unwrap();
    mobile.pull("workspace-a", 0, 10).unwrap();

    desktop
        .apply_op(&op(
            "desktop-edit",
            "note-c",
            OpKind::Put,
            1,
            Some(b"desktop"),
        ))
        .unwrap();
    let conflict = mobile
        .apply_op(&op("phone-edit", "note-c", OpKind::Put, 1, Some(b"phone")))
        .unwrap();
    assert!(matches!(conflict, ApplyOutcome::Conflict(_)));

    let pulled = mobile.pull("workspace-a", 0, 10).unwrap();
    assert_eq!(pulled.conflicts.len(), 1);
    let conflict = pulled.conflicts[0].clone();
    assert_eq!(conflict.remote_version, 2);

    let rebased = mobile
        .apply_op(&op(
            "phone-rebased",
            "note-c",
            OpKind::Put,
            2,
            Some(b"phone-rebased"),
        ))
        .unwrap();
    assert!(matches!(rebased, ApplyOutcome::Applied(_)));
    let resolved = mobile
        .resolve_conflict(
            &conflict.conflict_id,
            ConflictResolution::KeepLocal,
            Some("phone-rebased"),
            None,
        )
        .unwrap();
    assert_eq!(resolved.resolution, Some(ConflictResolution::KeepLocal));

    let pulled = desktop.pull("workspace-a", 0, 10).unwrap();
    assert!(pulled.conflicts.is_empty());
    let latest = desktop.snapshot("workspace-a").unwrap();
    assert_eq!(latest[0].version, 3);
    assert_eq!(latest[0].ciphertext.as_deref(), Some(&b"phone-rebased"[..]));
}

#[test]
fn delete_restore_and_trash_propagate_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    desktop
        .apply_op(&op("put", "note-d", OpKind::Put, 0, Some(b"body")))
        .unwrap();
    mobile.pull("workspace-a", 0, 10).unwrap();

    desktop
        .apply_op(&op("delete", "note-d", OpKind::Delete, 1, None))
        .unwrap();
    let pulled = mobile.pull("workspace-a", 1, 10).unwrap();
    assert_eq!(pulled.ops.len(), 1);
    assert!(pulled.ops[0].tombstone);
    let trash = mobile.trash("workspace-a").unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].object_id, "note-d");
    assert!(!trash[0].purged);

    desktop
        .apply_op(&op("restore", "note-d", OpKind::Restore, 2, Some(b"body")))
        .unwrap();
    let pulled = mobile.pull("workspace-a", 2, 10).unwrap();
    assert_eq!(pulled.ops.len(), 1);
    assert!(!pulled.ops[0].tombstone);
    assert!(mobile.trash("workspace-a").unwrap().is_empty());

    desktop
        .apply_op(&op("delete-again", "note-d", OpKind::Delete, 3, None))
        .unwrap();
    desktop
        .apply_op(&op("purge", "note-d", OpKind::Purge, 4, None))
        .unwrap();
    let trash = mobile.trash("workspace-a").unwrap();
    assert_eq!(trash.len(), 1);
    assert!(trash[0].purged);
    let snapshot = mobile.snapshot("workspace-a").unwrap();
    assert!(snapshot[0].purged);
    assert!(snapshot[0].tombstone);
}

#[test]
fn reading_positions_and_draft_handoffs_round_trip_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let input = |position: u64| ReadingPositionInput {
        workspace_id: "workspace-a".into(),
        object_id: "note-r".into(),
        content_version: 1,
        locator: ResumeLocator::Scroll { position },
    };
    desktop.put_reading_position(&input(120)).unwrap();
    mobile.put_reading_position(&input(400)).unwrap();
    let positions = desktop.reading_positions("workspace-a", "note-r").unwrap();
    assert_eq!(positions.len(), 2);
    let latest = pkms_sync::latest_reading_position(&positions).unwrap();
    assert_eq!(latest.locator, ResumeLocator::Scroll { position: 400 });

    let mut stale = input(20);
    stale.content_version = 0;
    assert!(mobile.put_reading_position(&stale).unwrap().stale);
    let positions = desktop.reading_positions("workspace-a", "note-r").unwrap();
    assert_eq!(positions.len(), 2);
    assert_eq!(
        pkms_sync::latest_reading_position(&positions)
            .unwrap()
            .locator,
        ResumeLocator::Scroll { position: 400 }
    );

    let draft = DraftHandoffInput {
        handoff_id: "handoff-1".into(),
        workspace_id: "workspace-a".into(),
        object_id: "note-r".into(),
        base_version: 1,
        base_revision_id: Some("rev-1".into()),
        title: "Phone draft".into(),
        ciphertext: b"draft-from-phone".to_vec(),
        key_epoch: 1,
    };
    let put = mobile.put_draft_handoff(&draft).unwrap();
    assert!(!put.duplicate);
    let list = desktop.draft_handoffs("workspace-a", "note-r").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].ciphertext.as_slice(), b"draft-from-phone");

    let adopted = desktop
        .adopt_draft_handoff("workspace-a", "note-r", "handoff-1")
        .unwrap();
    assert!(!adopted.duplicate);
    assert_eq!(
        adopted.handoff.adopted_by.as_deref(),
        Some(owner.device_id.as_str())
    );
    let again = mobile
        .adopt_draft_handoff("workspace-a", "note-r", "handoff-1")
        .unwrap();
    assert!(again.duplicate);

    let mut reused = draft.clone();
    reused.ciphertext = b"different".to_vec();
    let error = mobile.put_draft_handoff(&reused).unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 409);
            assert_eq!(code, "draft_id_reused");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn revoked_device_is_rejected_over_http() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost phone")
        .unwrap();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    desktop
        .apply_op(&op("owner-put", "note-x", OpKind::Put, 0, Some(b"x")))
        .unwrap();
    let error = mobile.snapshot("workspace-a").unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 401);
            assert_eq!(code, "device_revoked");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(desktop.snapshot("workspace-a").unwrap().len(), 1);
}

#[test]
fn transport_failure_is_reported_and_retry_uses_real_server() {
    let (authority, owner_key, owner, _phone_key, _phone) = setup();
    let server = start_server(authority);
    drop(owner_key);

    let dead = SyncClient::new(
        "http://127.0.0.1:9",
        &owner.device_id,
        DeviceKeyPair::from_seed(OWNER_SEED).unwrap(),
    )
    .unwrap();
    let error = dead.pull("workspace-a", 0, 10).unwrap_err();
    assert!(matches!(error, ClientError::Transport(_)));

    let retry = SyncClient::new(
        &server.base_url,
        &owner.device_id,
        DeviceKeyPair::from_seed(OWNER_SEED).unwrap(),
    )
    .unwrap();
    let batch = retry.pull("workspace-a", 0, 10).unwrap();
    assert!(batch.ops.is_empty());
    assert_eq!(batch.next_cursor, 0);
}

#[test]
fn replayed_signed_request_is_rejected_over_http() {
    use pkms_sync::pull_request_hash;

    let (authority, owner_key, owner, _phone_key, _phone) = setup();
    let server = start_server(authority);

    let issued_at = Utc::now().timestamp();
    let request = owner_key.sign_request(
        &owner.device_id,
        pkms_sync::SyncAccess::Read,
        "replay-nonce-0123456789",
        issued_at,
        pull_request_hash("workspace-a", 0, 10),
    );
    let body = pkms_sync::transport::PullBody {
        request,
        workspace_id: "workspace-a".into(),
        after_seq: 0,
        limit: 10,
    };
    let http = reqwest::blocking::Client::new();
    let url = format!("{}/v1/pull", server.base_url);
    let first = http.post(&url).json(&body).send().unwrap();
    assert!(first.status().is_success());
    let second = http.post(&url).json(&body).send().unwrap();
    assert_eq!(second.status().as_u16(), 401);
    let error: pkms_sync::transport::ApiErrorBody = second.json().unwrap();
    assert_eq!(error.code, "replayed_request");
}
#[test]
fn pairing_device_list_and_revoke_round_trip_over_http() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let owner_key = DeviceKeyPair::from_seed([33u8; 32]).unwrap();
    let owner_proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "pairing owner",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "pairing owner", &owner_proof)
        .unwrap();
    let server = start_server(authority);

    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let phone_key = DeviceKeyPair::from_seed([44u8; 32]).unwrap();
    let phone_key_for_client = DeviceKeyPair::from_seed([44u8; 32]).unwrap();
    // The invite is created by the enrolled owner over HTTP.
    let invite = desktop.create_pairing_invite(600).unwrap();
    let phone_id = {
        let pairing_client = SyncClient::new(&server.base_url, "", phone_key_for_client).unwrap();
        // A not-yet-registered device needs a temporary id only for request
        // construction; the pairing proof binds the actual public key.
        let device = pairing_client
            .pair_with_code(&invite.invite_id, &invite.code, "pairing phone")
            .unwrap();
        assert_eq!(device.status, DeviceStatus::Active);
        device.device_id
    };

    let devices = desktop.devices().unwrap();
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().any(|device| device.device_id == phone_id));

    let phone = SyncClient::new(&server.base_url, &phone_id, phone_key).unwrap();
    assert!(phone.pull("workspace-a", 0, 10).is_ok());
    let revoked = desktop.revoke_device(&phone_id, "lost phone").unwrap();
    assert_eq!(revoked.status, DeviceStatus::Revoked);
    let error = phone.pull("workspace-a", 0, 10).unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 401);
            assert_eq!(code, "device_revoked");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let devices = desktop.devices().unwrap();
    assert_eq!(
        devices
            .iter()
            .find(|device| device.device_id == phone_id)
            .unwrap()
            .status,
        DeviceStatus::Revoked
    );
}
#[test]
fn workspace_keys_are_registered_wrapped_and_opened_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let owner_encryption_public = owner_key.encryption_public_key().unwrap();
    let phone_encryption_public = phone_key.encryption_public_key().unwrap();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    desktop
        .register_encryption_key(&encryption_public_key_hex(&owner_encryption_public))
        .unwrap();
    mobile
        .register_encryption_key(&encryption_public_key_hex(&phone_encryption_public))
        .unwrap();

    let workspace_key = WorkspaceKey::generate().unwrap();
    let owner_envelope = wrap_workspace_key(
        owner_encryption_public,
        &workspace_key,
        &workspace_key_aad("workspace-a", 1, &owner.device_id),
    )
    .unwrap();
    let phone_envelope = wrap_workspace_key(
        phone_encryption_public,
        &workspace_key,
        &workspace_key_aad("workspace-a", 1, &phone.device_id),
    )
    .unwrap();
    let epoch = desktop
        .rotate_workspace_key(
            "workspace-a",
            1,
            &[
                (owner.device_id.clone(), owner_envelope),
                (phone.device_id.clone(), phone_envelope),
            ],
        )
        .unwrap();
    assert_eq!(epoch, 1);
    assert_eq!(desktop.current_key_epoch("workspace-a").unwrap(), Some(1));
    let encryption_keys = desktop.device_encryption_keys().unwrap();
    assert_eq!(encryption_keys.len(), 2);
    assert!(
        encryption_keys
            .iter()
            .any(|key| key.device_id == owner.device_id)
    );

    let regrant = wrap_workspace_key(
        owner_encryption_public,
        &workspace_key,
        &workspace_key_aad("workspace-a", 1, &owner.device_id),
    )
    .unwrap();
    desktop
        .grant_workspace_key("workspace-a", 1, &owner.device_id, &regrant)
        .unwrap();

    let owner_fetched = desktop.fetch_key_envelope("workspace-a", 1).unwrap();
    let owner_key_again = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let owner_wsk = owner_key_again
        .unwrap_workspace_key(
            &owner_fetched,
            &workspace_key_aad("workspace-a", 1, &owner.device_id),
        )
        .unwrap();
    let phone_fetched = mobile.fetch_key_envelope("workspace-a", 1).unwrap();
    let phone_key_again = DeviceKeyPair::from_seed(PHONE_SEED).unwrap();
    let phone_wsk = phone_key_again
        .unwrap_workspace_key(
            &phone_fetched,
            &workspace_key_aad("workspace-a", 1, &phone.device_id),
        )
        .unwrap();
    assert!(owner_wsk.matches(&phone_wsk));

    let aad = object_aad("workspace-a", "note-k", 1);
    let sealed = owner_wsk.encrypt(b"encrypted note body", &aad).unwrap();
    let wire = sealed.encode();
    let decoded = SealedData::decode(&wire).unwrap();
    assert_eq!(
        phone_wsk.decrypt(&decoded, &aad).unwrap(),
        b"encrypted note body"
    );
}

#[test]
fn device_roles_are_enforced_and_owner_can_change_them_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let error = mobile
        .create_pairing_invite_with_role(600, MemberRole::Viewer)
        .unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "insufficient_role");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let viewer = desktop
        .set_device_role(&phone.device_id, MemberRole::Viewer)
        .unwrap();
    assert_eq!(viewer.role, MemberRole::Viewer);
    assert!(mobile.pull("workspace-a", 0, 10).is_ok());
    let error = mobile
        .apply_op(&op(
            "viewer-write",
            "note-1",
            OpKind::Put,
            0,
            Some(b"viewer"),
        ))
        .unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "insufficient_role");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let editor = desktop
        .set_device_role(&phone.device_id, MemberRole::Editor)
        .unwrap();
    assert_eq!(editor.role, MemberRole::Editor);
    mobile
        .apply_op(&op(
            "editor-write",
            "note-1",
            OpKind::Put,
            0,
            Some(b"editor"),
        ))
        .unwrap();

    let second_owner = desktop
        .set_device_role(&phone.device_id, MemberRole::Owner)
        .unwrap();
    assert_eq!(second_owner.role, MemberRole::Owner);
    let former_owner = mobile
        .set_device_role(&owner.device_id, MemberRole::Editor)
        .unwrap();
    assert_eq!(former_owner.role, MemberRole::Editor);
}

#[test]
fn member_identity_and_device_invites_round_trip_over_http() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let members = desktop.members().unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|member| member.device_count == 1));

    let invite = desktop
        .create_member_pairing_invite(600, "Sam", MemberRole::Viewer)
        .unwrap();
    assert_eq!(invite.member_name.as_deref(), Some("Sam"));
    let sam_key = DeviceKeyPair::from_seed([55u8; 32]).unwrap();
    let sam_session = DeviceKeyPair::from_seed([55u8; 32]).unwrap();
    let sam = SyncClient::new(&server.base_url, "", sam_session).unwrap();
    let sam_phone = sam
        .pair_with_code(&invite.invite_id, &invite.code, "Sam phone")
        .unwrap();
    assert_eq!(sam_phone.member_name, "Sam");
    assert_eq!(sam_phone.role, MemberRole::Viewer);
    let members = desktop.members().unwrap();
    assert_eq!(members.len(), 3);

    let device_invite = desktop
        .create_device_pairing_invite(600, &sam_phone.member_id, MemberRole::Viewer)
        .unwrap();
    let tablet_key = DeviceKeyPair::from_seed([56u8; 32]).unwrap();
    let tablet_identity = DeviceKeyPair::from_seed([56u8; 32]).unwrap();
    let tablet = SyncClient::new(&server.base_url, "", tablet_identity)
        .unwrap()
        .pair_with_code(&device_invite.invite_id, &device_invite.code, "Sam tablet")
        .unwrap();
    assert_eq!(tablet.member_id, sam_phone.member_id);
    assert_eq!(tablet.member_name, "Sam");
    let sam_member = desktop
        .members()
        .unwrap()
        .into_iter()
        .find(|member| member.member_id == sam_phone.member_id)
        .unwrap();
    assert_eq!(sam_member.device_count, 2);

    let mobile_error = mobile
        .set_member_role(&sam_phone.member_id, MemberRole::Editor)
        .unwrap_err();
    match mobile_error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "insufficient_role");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let editor = desktop
        .set_member_role(&sam_phone.member_id, MemberRole::Editor)
        .unwrap();
    assert_eq!(editor.role, MemberRole::Editor);
    let removed = desktop
        .remove_member(&sam_phone.member_id, "left the team")
        .unwrap();
    assert_eq!(removed.status, MemberStatus::Removed);
    assert_eq!(removed.device_count, 0);
    let sam_client = SyncClient::new(&server.base_url, &sam_phone.device_id, sam_key).unwrap();
    assert!(sam_client.pull("workspace-a", 0, 10).is_err());
    let tablet_client = SyncClient::new(&server.base_url, &tablet.device_id, tablet_key).unwrap();
    assert!(tablet_client.pull("workspace-a", 0, 10).is_err());
}

#[test]
fn audit_log_is_visible_to_owners_only() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let entries = desktop.audit_log(50).unwrap();
    assert!(entries.iter().any(|entry| entry.action == "owner_enrolled"));
    assert!(entries.iter().any(|entry| entry.action == "device_paired"));

    desktop
        .set_device_role(&phone.device_id, MemberRole::Viewer)
        .unwrap();
    let error = mobile.audit_log(50).unwrap_err();
    match error {
        ClientError::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "insufficient_role");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn long_poll_wakes_for_remote_ops_and_times_out_without_data() {
    let (authority, owner_key, owner, phone_key, phone) = setup();
    let server = start_server(authority);
    let desktop = SyncClient::new(&server.base_url, &owner.device_id, owner_key).unwrap();
    let mobile = SyncClient::new(&server.base_url, &phone.device_id, phone_key).unwrap();

    let waiter =
        std::thread::spawn(move || mobile.wait_for_events("workspace-a", 0, 10, 5_000).unwrap());
    std::thread::sleep(Duration::from_millis(250));
    desktop
        .apply_op(&op("wait-op", "note-wait", OpKind::Put, 0, Some(b"live")))
        .unwrap();
    let batch = waiter.join().unwrap();
    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.ops[0].op_id, "wait-op");

    let empty = desktop
        .wait_for_events("workspace-a", batch.next_cursor, 10, 250)
        .unwrap();
    assert!(empty.ops.is_empty());
    assert!(empty.conflicts.is_empty());
}

#[test]
fn blob_upload_and_read_round_trip_through_the_http_transport() {
    let directory = tempfile::tempdir().unwrap();
    let mut authority = SyncAuthority::open_in_memory_with_blobs(directory.path()).unwrap();
    let owner_key = DeviceKeyPair::from_seed([31u8; 32]).unwrap();
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "blob owner",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "blob owner", &proof)
        .unwrap();
    let server = start_server(authority);
    let client = SyncClient::new(server.base_url, owner.device_id, owner_key).unwrap();

    let data = (0..(64 * 1024 * 2 + 1234))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<u8>>();
    let hash = pkms_sync::sha256_hex(&data);
    let begin = client
        .blob_begin(&hash, data.len() as i64, 64 * 1024)
        .unwrap();
    assert!(!begin.already_present);
    let status = client.blob_chunk(&hash, 0, &data[..64 * 1024]).unwrap();
    assert_eq!(status.received_chunks, vec![0]);
    client
        .blob_chunk(&hash, 1, &data[64 * 1024..128 * 1024])
        .unwrap();
    let status = client.blob_chunk(&hash, 2, &data[128 * 1024..]).unwrap();
    assert!(status.missing_chunks.is_empty());
    let record = client.blob_finalize(&hash).unwrap();
    assert!(record.complete);
    assert_eq!(record.size, data.len() as i64);

    let mut restored = Vec::new();
    for index in 0..3u32 {
        restored.extend(client.blob_read(&hash, index).unwrap());
    }
    assert_eq!(restored, data);
}
