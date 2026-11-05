//! Reproducible G02 evidence: pair a second device, revoke it, and show that
//! both read and write authorization fail while the owner still works.

use chrono::Utc;
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, SignedRequest, SyncAccess, SyncAuthority, owner_proof_message,
    pairing_proof_message,
};
use serde_json::json;

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
        [3u8; 32],
    )
}

fn outcome(authority: &mut SyncAuthority, request: &SignedRequest) -> String {
    match authority.authorize(request, Utc::now().timestamp()) {
        Ok(_) => "allowed".to_owned(),
        Err(error) => error.code().to_owned(),
    }
}

fn main() {
    let mut authority = SyncAuthority::open_in_memory().expect("authority");
    let owner_key = DeviceKeyPair::generate().expect("owner key");
    let owner_proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "JIA desktop",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "JIA desktop", &owner_proof)
        .expect("owner enrollment");

    let phone_key = DeviceKeyPair::generate().expect("phone key");
    let invite = authority
        .create_pairing_invite(&owner.device_id, 600)
        .expect("invite");
    let pairing_proof = phone_key.sign(&pairing_proof_message(
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
            &pairing_proof,
        )
        .expect("pairing");

    let before_read = outcome(
        &mut authority,
        &request(&phone_key, &phone, SyncAccess::Read, "demo-phone-read-0001"),
    );
    let before_write = outcome(
        &mut authority,
        &request(
            &phone_key,
            &phone,
            SyncAccess::Write,
            "demo-phone-write-001",
        ),
    );

    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost device")
        .expect("revoke");

    let after_read = outcome(
        &mut authority,
        &request(&phone_key, &phone, SyncAccess::Read, "demo-phone-read-0002"),
    );
    let after_write = outcome(
        &mut authority,
        &request(
            &phone_key,
            &phone,
            SyncAccess::Write,
            "demo-phone-write-002",
        ),
    );
    let owner_write = outcome(
        &mut authority,
        &request(
            &owner_key,
            &owner,
            SyncAccess::Write,
            "demo-owner-write-001",
        ),
    );

    let audit_actions = authority
        .audit_log(50)
        .expect("audit")
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-device-identity-demo-v1",
            "devices": authority.list_devices().expect("devices"),
            "before_revocation": {"read": before_read, "write": before_write},
            "after_revocation": {"read": after_read, "write": after_write},
            "owner_write_after_revocation": owner_write,
            "audit_actions": audit_actions,
        }))
        .expect("json")
    );
}
