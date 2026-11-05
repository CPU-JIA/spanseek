use chrono::{Duration, Utc};
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, DeviceStatus, MemberRole, SignedRequest, SyncAccess,
    SyncAuthority, SyncError, owner_proof_message, pairing_proof_message,
};

fn enroll_owner(authority: &mut SyncAuthority, name: &str) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().expect("key");
    let proof = key.sign(&owner_proof_message(&key.public_key_hex(), name));
    let device = authority
        .enroll_owner(&key.public_key(), name, &proof)
        .expect("owner enrollment");
    (key, device)
}

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
        .expect("pairing")
}

fn request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    access: SyncAccess,
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        access,
        nonce,
        Utc::now().timestamp(),
        [7u8; 32],
    )
}

#[test]
fn owner_enrollment_is_single_use() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_key, _owner) = enroll_owner(&mut authority, "JIA desktop");
    let second = DeviceKeyPair::generate().unwrap();
    let proof = second.sign(&owner_proof_message(&second.public_key_hex(), "intruder"));
    let error = authority
        .enroll_owner(&second.public_key(), "intruder", &proof)
        .unwrap_err();
    assert_eq!(error.code(), "owner_already_enrolled");
}

#[test]
fn pairing_registers_and_lists_devices() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &phone_key, "JIA phone");
    let devices = authority.list_devices().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].device_id, owner.device_id);
    assert_eq!(devices[1].device_id, phone.device_id);
    assert_eq!(devices[1].status, DeviceStatus::Active);
    assert!(devices[1].fingerprint.starts_with("sha256:"));
}

#[test]
fn pairing_code_is_single_use() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let key = DeviceKeyPair::generate().unwrap();
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .unwrap();
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
            "first",
            &proof,
        )
        .unwrap();
    let error = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &key.public_key(),
            "second",
            &proof,
        )
        .unwrap_err();
    assert_eq!(error.code(), "invite_consumed");
}

#[test]
fn expired_invite_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let now = Utc::now();
    let invite = authority
        .create_pairing_invite_at(&owner.device_id, 60, now)
        .unwrap();
    let key = DeviceKeyPair::generate().unwrap();
    let proof = key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &key.public_key_hex(),
    ));
    let error = authority
        .complete_pairing_at(
            &invite.invite_id,
            &invite.code,
            &key.public_key(),
            "late phone",
            &proof,
            now + Duration::seconds(61),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invite_expired");
}

#[test]
fn wrong_code_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .unwrap();
    let key = DeviceKeyPair::generate().unwrap();
    let proof = key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &key.public_key_hex(),
    ));
    let error = authority
        .complete_pairing(
            &invite.invite_id,
            "AAAA-BBBB-CCCC",
            &key.public_key(),
            "phone",
            &proof,
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_pairing_code");
}

#[test]
fn pairing_proof_must_match_device_key() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let key = DeviceKeyPair::generate().unwrap();
    let other = DeviceKeyPair::generate().unwrap();
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .unwrap();
    let proof = other.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &key.public_key_hex(),
    ));
    let error = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &key.public_key(),
            "phone",
            &proof,
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_signature");
}

#[test]
fn revoked_device_cannot_read_or_write_and_audit_is_queryable() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &phone_key, "JIA phone");

    authority
        .authorize(
            &request(&phone_key, &phone, SyncAccess::Read, "nonce-phone-read-01"),
            Utc::now().timestamp(),
        )
        .unwrap();
    authority
        .authorize(
            &request(&phone_key, &phone, SyncAccess::Write, "nonce-phone-write-1"),
            Utc::now().timestamp(),
        )
        .unwrap();

    let revoked = authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost phone")
        .unwrap();
    assert_eq!(revoked.status, DeviceStatus::Revoked);
    assert_eq!(
        revoked.revoked_by.as_deref(),
        Some(owner.device_id.as_str())
    );

    let read_error = authority
        .authorize(
            &request(&phone_key, &phone, SyncAccess::Read, "nonce-phone-read-02"),
            Utc::now().timestamp(),
        )
        .unwrap_err();
    let write_error = authority
        .authorize(
            &request(&phone_key, &phone, SyncAccess::Write, "nonce-phone-write-2"),
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(read_error.code(), "device_revoked");
    assert_eq!(write_error.code(), "device_revoked");

    authority
        .authorize(
            &request(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-owner-write-01",
            ),
            Utc::now().timestamp(),
        )
        .unwrap();

    let actions = authority
        .audit_log(100)
        .unwrap()
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    assert!(actions.contains(&"device_revoked".to_owned()));
    assert_eq!(
        actions
            .iter()
            .filter(|action| action.as_str() == "access_denied_revoked")
            .count(),
        2
    );
    let phone_record = authority.device(&phone.device_id).unwrap();
    assert_eq!(phone_record.status, DeviceStatus::Revoked);
    assert_eq!(phone_record.revoked_reason.as_deref(), Some("lost phone"));
}

#[test]
fn last_active_device_cannot_be_revoked() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let error = authority
        .revoke_device(&owner.device_id, &owner.device_id, "mistake")
        .unwrap_err();
    assert_eq!(error.code(), "last_active_device");
    let actions = authority
        .audit_log(10)
        .unwrap()
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    assert!(actions.contains(&"revoke_refused_last_active".to_owned()));
}

#[test]
fn tampered_signature_is_rejected_and_audited() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &key, "JIA phone");
    let mut signed = request(&key, &phone, SyncAccess::Write, "nonce-tampered-0001");
    signed.signature[0] ^= 0xff;
    let error = authority
        .authorize(&signed, Utc::now().timestamp())
        .unwrap_err();
    assert_eq!(error.code(), "invalid_signature");
    assert!(matches!(error, SyncError::InvalidSignature));
    let actions = authority
        .audit_log(10)
        .unwrap()
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    assert!(actions.contains(&"access_denied_invalid_signature".to_owned()));
}

#[test]
fn replayed_request_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &key, "JIA phone");
    let signed = request(&key, &phone, SyncAccess::Read, "nonce-replay-000001");
    let now = Utc::now().timestamp();
    authority.authorize(&signed, now).unwrap();
    let error = authority.authorize(&signed, now).unwrap_err();
    assert_eq!(error.code(), "replayed_request");
}

#[test]
fn stale_request_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &key, "JIA phone");
    let now = Utc::now().timestamp();
    let signed = key.sign_request(
        &phone.device_id,
        SyncAccess::Write,
        "nonce-stale-0000001",
        now - 301,
        [7u8; 32],
    );
    let error = authority.authorize(&signed, now).unwrap_err();
    assert_eq!(error.code(), "stale_request");
}

#[test]
fn registry_and_audit_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sync-authority.sqlite3");
    let (owner_id, phone_id) = {
        let mut authority = SyncAuthority::open(&path).unwrap();
        let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
        let phone_key = DeviceKeyPair::generate().unwrap();
        let phone = pair_device(&mut authority, &owner, &phone_key, "JIA phone");
        authority
            .revoke_device(&owner.device_id, &phone.device_id, "replaced")
            .unwrap();
        (owner.device_id, phone.device_id)
    };
    let authority = SyncAuthority::open(&path).unwrap();
    assert_eq!(authority.list_devices().unwrap().len(), 2);
    assert_eq!(
        authority.device(&owner_id).unwrap().status,
        DeviceStatus::Active
    );
    assert_eq!(
        authority.device(&phone_id).unwrap().status,
        DeviceStatus::Revoked
    );
    let actions = authority
        .audit_log(100)
        .unwrap()
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    assert!(actions.contains(&"device_revoked".to_owned()));
}

#[test]
fn revoked_inviter_invalidates_pending_invite() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &phone_key, "JIA phone");
    authority
        .set_device_role(&owner.device_id, &phone.device_id, MemberRole::Owner)
        .unwrap();
    let invite = authority
        .create_pairing_invite(&phone.device_id, 600)
        .unwrap();
    authority
        .revoke_device(&owner.device_id, &phone.device_id, "stolen phone")
        .unwrap();
    let new_key = DeviceKeyPair::generate().unwrap();
    let proof = new_key.sign(&pairing_proof_message(
        &invite.invite_id,
        &invite.code,
        &new_key.public_key_hex(),
    ));
    let error = authority
        .complete_pairing(
            &invite.invite_id,
            &invite.code,
            &new_key.public_key(),
            "tablet",
            &proof,
        )
        .unwrap_err();
    assert_eq!(error.code(), "device_revoked");
}

#[test]
fn pairing_role_is_enforced_and_owners_can_change_it() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let invite = authority
        .create_pairing_invite_with_role(&owner.device_id, 600, MemberRole::Viewer)
        .unwrap();
    assert_eq!(invite.role, MemberRole::Viewer);
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
            "read-only phone",
            &proof,
        )
        .unwrap();
    assert_eq!(phone.role, MemberRole::Viewer);

    let viewer_write = request(
        &phone_key,
        &phone,
        SyncAccess::Write,
        "nonce-viewer-write-01",
    );
    let error = authority
        .authorize(&viewer_write, Utc::now().timestamp())
        .unwrap_err();
    assert_eq!(error.code(), "insufficient_role");
    let viewer_read = request(
        &phone_key,
        &phone,
        SyncAccess::Read,
        "nonce-viewer-read-001",
    );
    authority
        .authorize(&viewer_read, Utc::now().timestamp())
        .unwrap();

    let editor = authority
        .set_device_role(&owner.device_id, &phone.device_id, MemberRole::Editor)
        .unwrap();
    assert_eq!(editor.role, MemberRole::Editor);
    let editor_write = request(
        &phone_key,
        &editor,
        SyncAccess::Write,
        "nonce-editor-write-01",
    );
    authority
        .authorize(&editor_write, Utc::now().timestamp())
        .unwrap();

    let error = authority
        .create_pairing_invite(&phone.device_id, 600)
        .unwrap_err();
    assert_eq!(error.code(), "insufficient_role");
    let error = authority
        .set_device_role(&phone.device_id, &owner.device_id, MemberRole::Viewer)
        .unwrap_err();
    assert_eq!(error.code(), "insufficient_role");
    let error = authority
        .revoke_device(&phone.device_id, &owner.device_id, "not allowed")
        .unwrap_err();
    assert_eq!(error.code(), "insufficient_role");

    let owner_again = authority
        .set_device_role(&owner.device_id, &owner.device_id, MemberRole::Owner)
        .unwrap();
    assert_eq!(owner_again.role, MemberRole::Owner);
    assert_eq!(owner_again.role, MemberRole::Owner);
}

#[test]
fn last_owner_cannot_be_demoted_or_revoked() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &phone_key, "editor phone");
    assert_eq!(phone.role, MemberRole::Editor);
    let error = authority
        .set_device_role(&owner.device_id, &owner.device_id, MemberRole::Editor)
        .unwrap_err();
    assert_eq!(error.code(), "last_owner");
    let error = authority
        .revoke_device(&owner.device_id, &owner.device_id, "remove self")
        .unwrap_err();
    assert_eq!(error.code(), "last_owner");
}

#[test]
fn owner_enrollment_creates_a_bound_member_identity() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    assert!(!owner.member_id.is_empty());
    assert_eq!(owner.member_name, "JIA desktop");
    assert_eq!(owner.role, MemberRole::Owner);
    let member = authority.member(&owner.member_id).unwrap();
    assert_eq!(member.display_name, "JIA desktop");
    assert_eq!(member.role, MemberRole::Owner);
    assert_eq!(member.status, pkms_sync::MemberStatus::Active);
    assert_eq!(member.device_count, 1);
    assert_eq!(authority.list_members().unwrap().len(), 1);
}

#[test]
fn member_and_device_invites_preserve_human_identity() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let invite = authority
        .create_member_pairing_invite(&owner.device_id, 600, "Alex", MemberRole::Editor)
        .unwrap();
    assert_eq!(invite.member_name.as_deref(), Some("Alex"));
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
            "Alex phone",
            &proof,
        )
        .unwrap();
    assert_eq!(phone.member_name, "Alex");
    assert_eq!(phone.role, MemberRole::Editor);
    assert_eq!(authority.member(&phone.member_id).unwrap().device_count, 1);

    let tablet_key = DeviceKeyPair::generate().unwrap();
    let device_invite = authority
        .create_device_pairing_invite(&owner.device_id, 600, &phone.member_id)
        .unwrap();
    assert_eq!(
        device_invite.member_id.as_deref(),
        Some(phone.member_id.as_str())
    );
    let proof = tablet_key.sign(&pairing_proof_message(
        &device_invite.invite_id,
        &device_invite.code,
        &tablet_key.public_key_hex(),
    ));
    let tablet = authority
        .complete_pairing(
            &device_invite.invite_id,
            &device_invite.code,
            &tablet_key.public_key(),
            "Alex tablet",
            &proof,
        )
        .unwrap();
    assert_eq!(tablet.member_id, phone.member_id);
    assert_eq!(tablet.member_name, "Alex");
    assert_eq!(authority.member(&phone.member_id).unwrap().device_count, 2);
}

#[test]
fn member_role_changes_cover_devices_and_member_removal_revokes_them() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (_owner_key, owner) = enroll_owner(&mut authority, "JIA desktop");
    let phone_key = DeviceKeyPair::generate().unwrap();
    let phone = pair_device(&mut authority, &owner, &phone_key, "JIA phone");
    assert_eq!(authority.member(&phone.member_id).unwrap().device_count, 1);
    let viewer = authority
        .set_member_role(&owner.device_id, &phone.member_id, MemberRole::Viewer)
        .unwrap();
    assert_eq!(viewer.role, MemberRole::Viewer);
    assert_eq!(
        authority.device(&phone.device_id).unwrap().role,
        MemberRole::Viewer
    );
    let error = authority
        .set_member_role(&phone.device_id, &owner.member_id, MemberRole::Viewer)
        .unwrap_err();
    assert_eq!(error.code(), "insufficient_role");
    let error = authority
        .remove_member(&owner.device_id, &owner.member_id, "self removal")
        .unwrap_err();
    assert_eq!(error.code(), "last_owner");
    let removed = authority
        .remove_member(&owner.device_id, &phone.member_id, "left the team")
        .unwrap();
    assert_eq!(removed.status, pkms_sync::MemberStatus::Removed);
    assert_eq!(
        authority.device(&phone.device_id).unwrap().status,
        DeviceStatus::Revoked
    );
    assert_eq!(authority.member(&phone.member_id).unwrap().device_count, 0);
}
