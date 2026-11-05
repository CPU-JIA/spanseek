use chrono::Utc;
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, DraftHandoffInput, ReadingPositionInput, ResumeLocator,
    SignedRequest, SyncAccess, SyncAuthority, adopt_draft_request_hash,
    draft_handoff_put_request_hash, draft_handoffs_request_hash, owner_proof_message,
    pairing_proof_message, reading_position_put_request_hash, reading_positions_request_hash,
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

fn signed(
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

fn position(workspace: &str, object: &str, version: i64, y: u64) -> ReadingPositionInput {
    ReadingPositionInput {
        workspace_id: workspace.into(),
        object_id: object.into(),
        content_version: version,
        locator: ResumeLocator::Scroll { position: y },
    }
}

fn draft(id: &str, workspace: &str, object: &str, body: &[u8]) -> DraftHandoffInput {
    DraftHandoffInput {
        handoff_id: id.into(),
        workspace_id: workspace.into(),
        object_id: object.into(),
        base_version: 2,
        base_revision_id: Some("revision-2".into()),
        title: "继续写".into(),
        ciphertext: body.to_vec(),
        key_epoch: 1,
    }
}

#[test]
fn reading_positions_are_per_device_and_cross_library_isolated() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let base = Utc::now().timestamp_millis();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");

    let desktop_position = position("workspace-1", "note-1", 3, 120);
    let desktop = authority
        .put_reading_position(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-position-desktop-1",
                reading_position_put_request_hash(&desktop_position),
            ),
            &desktop_position,
            base,
        )
        .unwrap();
    assert!(!desktop.stale);

    let phone_position = ReadingPositionInput {
        workspace_id: "workspace-1".into(),
        object_id: "note-1".into(),
        content_version: 3,
        locator: ResumeLocator::Pdf {
            page_index: 4,
            offset_milli: 250,
        },
    };
    let phone_saved = authority
        .put_reading_position(
            &signed(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "nonce-position-phone-1",
                reading_position_put_request_hash(&phone_position),
            ),
            &phone_position,
            base + 100,
        )
        .unwrap();
    assert!(!phone_saved.stale);

    let list_request = signed(
        &phone_key,
        &phone,
        SyncAccess::Read,
        "nonce-position-list-1",
        reading_positions_request_hash("workspace-1", "note-1"),
    );
    let positions = authority
        .reading_positions(
            &list_request,
            "workspace-1",
            "note-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(positions.len(), 2);
    assert_eq!(positions[0].device_id, phone.device_id);
    assert_eq!(positions[1].device_id, owner.device_id);
    assert_eq!(
        pkms_sync::latest_reading_position(&positions)
            .unwrap()
            .device_id,
        phone.device_id
    );

    // The same object id in another library must never be returned.
    let other = position("workspace-2", "note-1", 3, 900);
    authority
        .put_reading_position(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-position-other-1",
                reading_position_put_request_hash(&other),
            ),
            &other,
            base + 200,
        )
        .unwrap();
    let list_request = signed(
        &owner_key,
        &owner,
        SyncAccess::Read,
        "nonce-position-list-2",
        reading_positions_request_hash("workspace-1", "note-1"),
    );
    let still_two = authority
        .reading_positions(
            &list_request,
            "workspace-1",
            "note-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(still_two.len(), 2);
    assert!(
        still_two
            .iter()
            .all(|entry| entry.workspace_id == "workspace-1")
    );

    // A same-device older checkpoint must not erase the newer one.
    let stale_input = position("workspace-1", "note-1", 2, 10);
    let stale = authority
        .put_reading_position(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-position-stale-1",
                reading_position_put_request_hash(&stale_input),
            ),
            &stale_input,
            base - 1_000,
        )
        .unwrap();
    assert!(stale.stale);
    assert_eq!(stale.position.content_version, 3);
    assert_eq!(
        stale.position.locator,
        ResumeLocator::Scroll { position: 120 }
    );
}

#[test]
fn draft_handoffs_are_append_only_and_adoption_keeps_payload() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let base = Utc::now().timestamp_millis();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");

    let first = draft("draft-1", "workspace-1", "note-1", b"phone body");
    let first_saved = authority
        .put_draft_handoff(
            &signed(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "nonce-draft-phone-1",
                draft_handoff_put_request_hash(&first),
            ),
            &first,
            base,
        )
        .unwrap();
    assert!(!first_saved.duplicate);
    assert_eq!(first_saved.handoff.device_id, phone.device_id);

    let retry = authority
        .put_draft_handoff(
            &signed(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "nonce-draft-phone-2",
                draft_handoff_put_request_hash(&first),
            ),
            &first,
            base + 100,
        )
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.handoff.created_at_ms, base);

    let second = draft("draft-2", "workspace-1", "note-1", b"desktop body");
    authority
        .put_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-owner-1",
                draft_handoff_put_request_hash(&second),
            ),
            &second,
            base + 200,
        )
        .unwrap();

    let list_request = signed(
        &owner_key,
        &owner,
        SyncAccess::Read,
        "nonce-draft-list-1",
        draft_handoffs_request_hash("workspace-1", "note-1"),
    );
    let drafts = authority
        .draft_handoffs(
            &list_request,
            "workspace-1",
            "note-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].handoff_id, "draft-2");
    assert_eq!(drafts[1].handoff_id, "draft-1");

    let adopted = authority
        .adopt_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-adopt-1",
                adopt_draft_request_hash("workspace-1", "note-1", "draft-1"),
            ),
            "workspace-1",
            "note-1",
            "draft-1",
            base + 300,
        )
        .unwrap();
    assert_eq!(
        adopted.handoff.adopted_by.as_deref(),
        Some(owner.device_id.as_str())
    );
    assert_eq!(adopted.handoff.ciphertext, b"phone body");

    let list_request = signed(
        &phone_key,
        &phone,
        SyncAccess::Read,
        "nonce-draft-list-2",
        draft_handoffs_request_hash("workspace-1", "note-1"),
    );
    let after_adoption = authority
        .draft_handoffs(
            &list_request,
            "workspace-1",
            "note-1",
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(after_adoption.len(), 2);

    let other_workspace = authority
        .adopt_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-adopt-2",
                adopt_draft_request_hash("workspace-2", "note-1", "draft-2"),
            ),
            "workspace-2",
            "note-1",
            "draft-2",
            base + 400,
        )
        .unwrap_err();
    assert_eq!(other_workspace.code(), "draft_not_found");
}

#[test]
fn reused_draft_id_with_different_payload_is_rejected() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let base = Utc::now().timestamp_millis();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let first = draft("draft-reuse", "workspace-1", "note-1", b"one");
    authority
        .put_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-reuse-1",
                draft_handoff_put_request_hash(&first),
            ),
            &first,
            base,
        )
        .unwrap();
    let changed = draft("draft-reuse", "workspace-1", "note-1", b"two");
    let error = authority
        .put_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-reuse-2",
                draft_handoff_put_request_hash(&changed),
            ),
            &changed,
            base + 100,
        )
        .unwrap_err();
    assert_eq!(error.code(), "draft_id_reused");
}

#[test]
fn tampered_draft_hash_is_rejected_before_write() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let base = Utc::now().timestamp_millis();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let input = draft("draft-tamper", "workspace-1", "note-1", b"payload");
    let wrong = draft_handoff_put_request_hash(&draft(
        "draft-tamper",
        "workspace-1",
        "note-1",
        b"different",
    ));
    let error = authority
        .put_draft_handoff(
            &signed(
                &owner_key,
                &owner,
                SyncAccess::Write,
                "nonce-draft-tamper-1",
                wrong,
            ),
            &input,
            base,
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request_hash");
}

#[test]
fn revoked_device_cannot_read_or_write_resume_data() {
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let base = Utc::now().timestamp_millis();
    let (_owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    authority
        .revoke_device(&owner.device_id, &phone.device_id, "lost phone")
        .unwrap();

    let input = position("workspace-1", "note-1", 1, 10);
    let write = authority
        .put_reading_position(
            &signed(
                &phone_key,
                &phone,
                SyncAccess::Write,
                "nonce-revoked-resume-1",
                reading_position_put_request_hash(&input),
            ),
            &input,
            base,
        )
        .unwrap_err();
    assert_eq!(write.code(), "device_revoked");

    let read = authority
        .reading_positions(
            &signed(
                &phone_key,
                &phone,
                SyncAccess::Read,
                "nonce-revoked-resume-2",
                reading_positions_request_hash("workspace-1", "note-1"),
            ),
            "workspace-1",
            "note-1",
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(read.code(), "device_revoked");
}
#[test]
fn v6_database_migrates_to_resume_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sync.sqlite3");
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
        connection
            .execute_batch(include_str!("../src/migrations/0004_trash.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../src/migrations/0005_blobs.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../src/migrations/0006_keys.sql"))
            .unwrap();
    }
    let authority = SyncAuthority::open(&path).unwrap();
    assert_eq!(authority.schema_version().unwrap(), 9);
    drop(authority);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name IN ('sync_reading_positions','sync_draft_handoffs') ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        tables,
        vec!["sync_draft_handoffs", "sync_reading_positions"]
    );
}
