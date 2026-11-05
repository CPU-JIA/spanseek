use chrono::Utc;
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, DraftHandoffInput, ReadingPositionInput, ResumeLocator,
    SealedData, SignedRequest, SyncAccess, SyncAuthority, WorkspaceKey, adopt_draft_request_hash,
    draft_handoff_aad, draft_handoff_put_request_hash, draft_handoffs_request_hash,
    latest_reading_position, owner_proof_message, pairing_proof_message,
    reading_position_put_request_hash, reading_positions_request_hash,
};
use serde_json::json;

fn enroll_owner(authority: &mut SyncAuthority) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().unwrap();
    let proof = key.sign(&owner_proof_message(&key.public_key_hex(), "desktop"));
    let device = authority
        .enroll_owner(&key.public_key(), "desktop", &proof)
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

fn request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    access: SyncAccess,
    nonce: &str,
    hash: [u8; 32],
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        access,
        nonce,
        Utc::now().timestamp(),
        hash,
    )
}

fn main() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let (desktop_key, desktop) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &desktop, "phone");
    let now = Utc::now().timestamp();
    let now_ms = now * 1000;
    let workspace = "workspace-g10";
    let object = "note-g10";
    let key = WorkspaceKey::from_bytes([9u8; 32]);

    let phone_position = ReadingPositionInput {
        workspace_id: workspace.into(),
        object_id: object.into(),
        content_version: 3,
        locator: ResumeLocator::Scroll { position: 420 },
    };
    authority
        .put_reading_position(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "resume-demo-pos-phone",
                reading_position_put_request_hash(&phone_position),
            ),
            &phone_position,
            now_ms,
        )
        .unwrap();

    let desktop_position = ReadingPositionInput {
        workspace_id: workspace.into(),
        object_id: object.into(),
        content_version: 3,
        locator: ResumeLocator::Pdf {
            page_index: 7,
            offset_milli: 125,
        },
    };
    authority
        .put_reading_position(
            &request(
                &desktop_key,
                &desktop,
                SyncAccess::Write,
                "resume-demo-pos-desktop",
                reading_position_put_request_hash(&desktop_position),
            ),
            &desktop_position,
            now_ms + 20,
        )
        .unwrap();

    let list_positions = request(
        &desktop_key,
        &desktop,
        SyncAccess::Read,
        "resume-demo-pos-list",
        reading_positions_request_hash(workspace, object),
    );
    let positions = authority
        .reading_positions(&list_positions, workspace, object, now + 1)
        .unwrap();

    let phone_draft = DraftHandoffInput {
        handoff_id: "draft-phone-g10".into(),
        workspace_id: workspace.into(),
        object_id: object.into(),
        base_version: 3,
        base_revision_id: Some("revision-3".into()),
        title: "手机续写".into(),
        ciphertext: key
            .encrypt(
                "手机上的草稿正文".as_bytes(),
                &draft_handoff_aad(workspace, object, "draft-phone-g10", 1),
            )
            .unwrap()
            .encode(),
        key_epoch: 1,
    };
    authority
        .put_draft_handoff(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "resume-demo-draft-phone",
                draft_handoff_put_request_hash(&phone_draft),
            ),
            &phone_draft,
            now_ms + 40,
        )
        .unwrap();

    let desktop_draft = DraftHandoffInput {
        handoff_id: "draft-desktop-g10".into(),
        workspace_id: workspace.into(),
        object_id: object.into(),
        base_version: 3,
        base_revision_id: Some("revision-3".into()),
        title: "桌面续写".into(),
        ciphertext: key
            .encrypt(
                "桌面上的草稿正文".as_bytes(),
                &draft_handoff_aad(workspace, object, "draft-desktop-g10", 1),
            )
            .unwrap()
            .encode(),
        key_epoch: 1,
    };
    authority
        .put_draft_handoff(
            &request(
                &desktop_key,
                &desktop,
                SyncAccess::Write,
                "resume-demo-draft-desktop",
                draft_handoff_put_request_hash(&desktop_draft),
            ),
            &desktop_draft,
            now_ms + 60,
        )
        .unwrap();

    let list_drafts = request(
        &desktop_key,
        &desktop,
        SyncAccess::Read,
        "resume-demo-draft-list",
        draft_handoffs_request_hash(workspace, object),
    );
    let drafts = authority
        .draft_handoffs(&list_drafts, workspace, object, now + 1)
        .unwrap();
    let phone_body = key
        .decrypt(
            &SealedData::decode(
                &drafts
                    .iter()
                    .find(|draft| draft.handoff_id == "draft-phone-g10")
                    .unwrap()
                    .ciphertext,
            )
            .unwrap(),
            &draft_handoff_aad(workspace, object, "draft-phone-g10", 1),
        )
        .unwrap();

    let adopted = authority
        .adopt_draft_handoff(
            &request(
                &desktop_key,
                &desktop,
                SyncAccess::Write,
                "resume-demo-draft-adopt",
                adopt_draft_request_hash(workspace, object, "draft-phone-g10"),
            ),
            workspace,
            object,
            "draft-phone-g10",
            now_ms + 80,
        )
        .unwrap();

    let other = ReadingPositionInput {
        workspace_id: "workspace-other".into(),
        object_id: object.into(),
        content_version: 1,
        locator: ResumeLocator::Scroll { position: 9 },
    };
    authority
        .put_reading_position(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "resume-demo-pos-other",
                reading_position_put_request_hash(&other),
            ),
            &other,
            now_ms + 100,
        )
        .unwrap();
    let list_positions = request(
        &phone_key,
        &phone,
        SyncAccess::Read,
        "resume-demo-pos-list-2",
        reading_positions_request_hash(workspace, object),
    );
    let positions_after_other = authority
        .reading_positions(&list_positions, workspace, object, now + 1)
        .unwrap();

    let latest = latest_reading_position(&positions).unwrap();
    assert_eq!(positions.len(), 2);
    assert_eq!(latest.device_id, desktop.device_id);
    assert_eq!(drafts.len(), 2);
    assert_eq!(phone_body, "手机上的草稿正文".as_bytes());
    assert_eq!(
        adopted.handoff.adopted_by.as_deref(),
        Some(desktop.device_id.as_str())
    );
    assert_eq!(adopted.handoff.ciphertext, drafts[1].ciphertext);
    assert_eq!(positions_after_other.len(), 2);

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-resume-handoff-verification-v1",
            "sync_schema_version": authority.schema_version().unwrap(),
            "workspace_id": workspace,
            "object_id": object,
            "reading_positions": positions.iter().map(|position| json!({
                "device_id": position.device_id,
                "content_version": position.content_version,
                "locator": position.locator,
                "updated_at_ms": position.updated_at_ms,
            })).collect::<Vec<_>>(),
            "reading_positions_preserved": positions.len() == 2,
            "latest_position_device": latest.device_id,
            "draft_count": drafts.len(),
            "draft_handoffs_preserved": drafts.len() == 2,
            "phone_draft_body_matches": phone_body == "手机上的草稿正文".as_bytes(),
            "adopted_by": adopted.handoff.adopted_by,
            "ciphertext_retained_after_adoption": adopted.handoff.ciphertext == drafts[1].ciphertext,
            "other_workspace_same_object_isolated": positions_after_other.len() == 2,
            "all_checks_passed": positions.len() == 2
                && positions_after_other.len() == 2
                && drafts.len() == 2
                && phone_body == "手机上的草稿正文".as_bytes()
        }))
        .unwrap()
    );
}
