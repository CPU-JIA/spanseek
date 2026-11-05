use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pkms_app::inference_credentials as vault;
use pkms_app::{
    AppError, AppService, DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, OperationKind,
    SyncMemberRole, WorkspaceIdentity,
};
use pkms_sync::{
    DeviceKeyPair, MemberRole, SyncAuthority, encryption_public_key_hex, owner_proof_message,
};
use pkms_sync_client::SyncClient;
use tempfile::tempdir;
use uuid::Uuid;

const OWNER_SEED: [u8; 32] = [77u8; 32];

fn start_server(authority: SyncAuthority) -> String {
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(address).is_ok() {
            return base_url;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("sync server did not become ready");
}

fn dispatch(
    app: &mut AppService,
    identity: Option<WorkspaceIdentity>,
    command: DesktopCommand,
) -> DesktopData {
    app.dispatch(DesktopRequest {
        api_version: 1,
        request_id: Uuid::new_v4().to_string(),
        identity,
        command,
    })
    .unwrap()
    .data
}
fn create_workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(
            &Uuid::new_v4().to_string(),
            OperationKind::CreateWorkspace,
            &grant.token,
        )
        .unwrap();
    app.execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity
}

fn open_workspace(app: &mut AppService, path: &Path) -> WorkspaceIdentity {
    let grant = app.grant_path(GrantPurpose::OpenWorkspace, path).unwrap();
    let accepted = app
        .accept_workspace_operation(
            &Uuid::new_v4().to_string(),
            OperationKind::OpenWorkspace,
            &grant.token,
        )
        .unwrap();
    app.execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity
}

fn rename_tag(app: &mut AppService, identity: &WorkspaceIdentity, id: &str, name: &str) {
    let details = app.library(identity).unwrap().tag_details(id).unwrap();
    app.library_mut(identity)
        .unwrap()
        .tag_update(
            &Uuid::new_v4().to_string(),
            pkms_core::UpdateTag {
                id: id.into(),
                base_row_version: details.row_version,
                name: name.into(),
                description: String::new(),
            },
        )
        .unwrap();
}

#[test]
fn tag_application_sync_conflicts_restore_scope_and_readonly_do_not_change_note_bodies() {
    use pkms_core::{
        AssignTag, CreateTag, DeleteTag, MetadataTarget, TagSelector, TagSyncKind,
        TagSyncPageRequest,
    };
    let temporary = tempdir().unwrap();
    let mut a = AppService::open(temporary.path().join("app-a")).unwrap();
    let aid = create_workspace(&mut a, &temporary.path().join("a"));
    assert!(!a.sync_auto_push_tags(&aid).unwrap().configured);
    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "tag owner",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "tag owner", &proof)
        .unwrap();
    let base = start_server(authority);
    let owner_client = SyncClient::new(&base, &owner.device_id, owner_key).unwrap();
    owner_client
        .register_encryption_key(&encryption_public_key_hex(
            &DeviceKeyPair::from_seed(OWNER_SEED)
                .unwrap()
                .encryption_public_key()
                .unwrap(),
        ))
        .unwrap();
    let invitation = owner_client
        .create_pairing_invite_with_role(600, MemberRole::Owner)
        .unwrap();
    a.sync_configure(&base, "tag device A").unwrap();
    let ap = a
        .sync_pair(&invitation.invite_id, &invitation.code)
        .unwrap();
    a.sync_ensure_workspace_key(&aid.workspace_id).unwrap();
    a.sync_attach_workspace(&aid.workspace_id).unwrap();
    let note = a
        .library_mut(&aid)
        .unwrap()
        .create_note("保留正文", "正文不得因标签同步重写")
        .unwrap();
    let tag = a
        .library_mut(&aid)
        .unwrap()
        .tag_create(
            &Uuid::new_v4().to_string(),
            CreateTag {
                name: "共享标签".into(),
                description: String::new(),
            },
        )
        .unwrap()
        .tag;
    let row = a
        .library(&aid)
        .unwrap()
        .object_metadata(&note.id)
        .unwrap()
        .row_version;
    a.library_mut(&aid)
        .unwrap()
        .assign_tag(
            &Uuid::new_v4().to_string(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: note.id.clone(),
                    base_row_version: row,
                }],
                tag: TagSelector::Id {
                    id: tag.tag.id.clone(),
                },
            },
        )
        .unwrap();
    let pushed = a.sync_push_all(&aid).unwrap();
    assert!(pushed.tags.complete);
    assert_eq!(pushed.tags.published, 2);
    let package = temporary.path().join("full.pkmsbackup");
    a.library(&aid)
        .unwrap()
        .create_backup(&package, &mut |_| Ok(()))
        .unwrap();
    let bpath = temporary.path().join("restored-b");
    pkms_core::prepare_restore(&package, &bpath, &mut |_| Ok(()))
        .unwrap()
        .publish()
        .unwrap();
    let app_b_path = temporary.path().join("app-b");
    let mut b = AppService::open(&app_b_path).unwrap();
    let bid = open_workspace(&mut b, &bpath);
    assert_ne!(bid.local_instance_id, aid.local_instance_id);
    assert_eq!(bid.workspace_id, aid.workspace_id);
    let invitation = a
        .sync_create_invite("tag peer", SyncMemberRole::Editor)
        .unwrap();
    b.sync_configure(&base, "tag device B").unwrap();
    let bp = b
        .sync_pair(&invitation.invite_id, &invitation.code)
        .unwrap();
    a.sync_grant_workspace_key(&aid.workspace_id, bp.device_id.as_deref().unwrap())
        .unwrap();
    b.sync_attach_workspace(&aid.workspace_id).unwrap();
    assert!(b.sync_auto_push_tags(&bid).unwrap().complete);
    rename_tag(&mut a, &aid, &tag.tag.id, "远端改名");
    rename_tag(&mut b, &bid, &tag.tag.id, "本机改名");
    assert!(a.sync_auto_push_tags(&aid).unwrap().complete);
    let pulled = b.sync_pull(&bid).unwrap();
    assert_eq!(pulled.tag_pending, 1);
    let pending = b
        .sync_tag_page(
            &bid,
            TagSyncPageRequest {
                limit: 1,
                after: None,
                pending_only: true,
            },
        )
        .unwrap();
    assert_eq!(pending.total, 1);
    let state = pending.items[0].clone();
    let decision = pkms_app::SyncTagDecision {
        kind: state.kind,
        entity_id: state.entity_id.clone(),
        remote_version: state.remote_version,
        local_fingerprint: state.local_fingerprint,
        local_impact_token: state.local_impact_token,
    };
    let mut stale = decision.clone();
    stale.local_fingerprint = Some("changed-after-preview".into());
    assert!(b.sync_tag_keep_remote(&bid, stale).is_err());
    assert!(
        b.sync_tag_keep_remote(&bid, decision)
            .unwrap()
            .pending
            .is_none()
    );
    assert_eq!(
        b.library(&bid)
            .unwrap()
            .tag_details(&tag.tag.id)
            .unwrap()
            .tag
            .display_name,
        "远端改名"
    );
    assert_eq!(
        b.library(&bid).unwrap().note(&note.id).unwrap().document,
        note.document
    );
    let member = a
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| Some(&d.device_id) == bp.device_id.as_ref())
        .unwrap()
        .member_id;
    a.sync_set_member_role(&member, SyncMemberRole::Viewer)
        .unwrap();
    rename_tag(&mut b, &bid, &tag.tag.id, "只读设备本机草稿");
    let remote_note = a
        .library_mut(&aid)
        .unwrap()
        .create_note("稍后由正文拉取接收", "保留更新通知路径")
        .unwrap();
    a.sync_push_note(&aid, &remote_note.id).unwrap();
    let readonly = b.sync_auto_push_tags(&bid).unwrap();
    assert!(!readonly.complete);
    assert!(readonly.dirty > 0);
    assert!(matches!(
        b.library(&bid).unwrap().note(&remote_note.id),
        Err(pkms_core::Error::NotFound)
    ));
    a.sync_set_member_role(&member, SyncMemberRole::Editor)
        .unwrap();
    assert!(b.sync_auto_push_tags(&bid).unwrap().complete);
    drop(b);
    let mut b = AppService::open(&app_b_path).unwrap();
    let reopened = open_workspace(&mut b, &bpath);
    assert_eq!(reopened, bid);
    assert!(b.sync_auto_push_tags(&bid).unwrap().complete);
    a.sync_pull(&aid).unwrap();
    let details = a.library(&aid).unwrap().tag_details(&tag.tag.id).unwrap();
    a.library_mut(&aid)
        .unwrap()
        .tag_delete(
            &Uuid::new_v4().to_string(),
            DeleteTag {
                id: tag.tag.id.clone(),
                base_row_version: details.row_version,
                impact_token: details.impact_token,
            },
        )
        .unwrap();
    assert!(a.sync_auto_push_tags(&aid).unwrap().complete);
    b.sync_pull(&bid).unwrap();
    assert!(
        b.library(&bid)
            .unwrap()
            .object_metadata(&note.id)
            .unwrap()
            .tags
            .is_empty()
    );
    assert_eq!(
        b.library(&bid).unwrap().note(&note.id).unwrap().document,
        note.document
    );
    assert!(
        !b.library(&bid)
            .unwrap()
            .tag_sync_snapshot(TagSyncKind::Tag, &tag.tag.id)
            .unwrap()
            .dirty
    );
    for profile in [ap, bp] {
        let _ = vault::delete_at(&format!("PKMS:sync-device:v1:{}", profile.local_id));
    }
    let _ = vault::delete_at(&format!("PKMS:sync-wsk:v1:{}:1", aid.workspace_id));
}

#[test]
fn profile_pairing_key_distribution_push_pull_and_revoke() {
    let temporary = tempdir().unwrap();
    let app_path = temporary.path().join("app");
    let workspace_path = temporary.path().join("knowledge");
    let mut app = AppService::open(&app_path).unwrap();
    let identity = create_workspace(&mut app, &workspace_path);
    let workspace_id = app.library(&identity).unwrap().info().unwrap().workspace_id;
    let note = app
        .library_mut(&identity)
        .unwrap()
        .create_note("Sync note", "encrypted body")
        .unwrap();

    let mut authority = SyncAuthority::open_in_memory().unwrap();
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED).unwrap();
    let owner_encryption = owner_key.encryption_public_key().unwrap();
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "test owner",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "test owner", &proof)
        .unwrap();
    let base_url = start_server(authority);
    let owner_client = SyncClient::new(&base_url, &owner.device_id, owner_key).unwrap();
    owner_client
        .register_encryption_key(&encryption_public_key_hex(&owner_encryption))
        .unwrap();
    let invite = owner_client
        .create_pairing_invite_with_role(600, MemberRole::Owner)
        .unwrap();

    app.sync_configure(&base_url, "test laptop").unwrap();
    let profile = app.sync_pair(&invite.invite_id, &invite.code).unwrap();
    let app_device_id = profile.device_id.clone().unwrap();
    assert!(!app_device_id.is_empty());
    let devices = app.sync_devices().unwrap();
    assert_eq!(
        devices
            .iter()
            .find(|device| device.device_id == app_device_id)
            .unwrap()
            .role,
        SyncMemberRole::Owner
    );
    let owner_proof = app.sync_owner_proof().unwrap();
    assert_eq!(owner_proof.public_key.len(), 64);
    assert_eq!(owner_proof.proof.len(), 128);
    assert_eq!(
        app.sync_claim_device(&app_device_id)
            .unwrap()
            .device_id
            .as_deref(),
        Some(app_device_id.as_str())
    );

    let error = app.sync_attach_workspace(&workspace_id).unwrap_err();
    assert!(matches!(error, AppError::Sync(_)));
    let profile = app.sync_ensure_workspace_key(&workspace_id).unwrap();
    assert_eq!(profile.epoch, 1);
    assert!(app.sync_attach_workspace(&workspace_id).is_ok());
    assert_eq!(
        app.sync_grant_workspace_key(&workspace_id, &owner.device_id)
            .unwrap(),
        1
    );

    let status = app.sync_status().unwrap();
    assert!(status.configured);
    assert!(status.healthy);
    assert_eq!(status.schema_version, Some(9));
    assert_eq!(status.protocol_version, Some(3));
    assert_eq!(app.sync_devices().unwrap().len(), 2);
    assert_eq!(app.sync_members().unwrap().len(), 2);
    let viewer_owner = app
        .sync_set_member_role(&owner.member_id, SyncMemberRole::Viewer)
        .unwrap();
    assert_eq!(viewer_owner.role, SyncMemberRole::Viewer);
    let owner_again = app
        .sync_set_member_role(&owner.member_id, SyncMemberRole::Owner)
        .unwrap();
    assert_eq!(owner_again.role, SyncMemberRole::Owner);
    let invite = app
        .sync_create_invite("Read-only teammate", SyncMemberRole::Viewer)
        .unwrap();
    assert_eq!(invite.role, SyncMemberRole::Viewer);
    assert_eq!(invite.member_name.as_deref(), Some("Read-only teammate"));

    let viewer_key = DeviceKeyPair::from_seed([99u8; 32]).unwrap();
    let viewer_encryption = encryption_public_key_hex(&viewer_key.encryption_public_key().unwrap());
    let viewer = SyncClient::new(&base_url, "", DeviceKeyPair::from_seed([99u8; 32]).unwrap())
        .unwrap()
        .pair_with_code(&invite.invite_id, &invite.code, "Read-only phone")
        .unwrap();
    let viewer_session = SyncClient::new(
        &base_url,
        &viewer.device_id,
        DeviceKeyPair::from_seed([99u8; 32]).unwrap(),
    )
    .unwrap();
    viewer_session
        .register_encryption_key(&viewer_encryption)
        .unwrap();
    assert_eq!(app.sync_devices().unwrap().len(), 3);
    assert_eq!(app.sync_members().unwrap().len(), 3);
    let removal = app
        .sync_remove_member(&viewer.member_id, "test member removal")
        .unwrap();
    assert_eq!(removal.member.status, "removed");
    assert!(removal.key_rotated);
    assert_eq!(removal.key_epoch, Some(2));
    assert!(viewer_session.pull("workspace-a", 0, 10).is_err());
    let audit = app.sync_audit(50).unwrap();
    assert!(audit.iter().any(|entry| entry.action == "member_removed"));

    let pushed = app.sync_push_note(&identity, &note.id).unwrap();
    assert_eq!(pushed.version, 1);
    assert!(!pushed.unchanged);
    let auto_push = app.sync_auto_push_note(&identity, &note.id).unwrap();
    assert!(auto_push.configured);
    assert!(!auto_push.pushed);
    assert!(!auto_push.conflict);

    let status = dispatch(&mut app, None, DesktopCommand::SyncStatus);
    assert!(matches!(status, DesktopData::SyncStatus(status) if status.healthy));
    let devices = dispatch(&mut app, None, DesktopCommand::SyncDevices);
    assert!(matches!(devices, DesktopData::SyncDevices(devices) if devices.len() == 3));
    let members = dispatch(&mut app, None, DesktopCommand::SyncMembers);
    assert!(matches!(members, DesktopData::SyncMembers(members) if members.len() == 3));
    let audit = dispatch(&mut app, None, DesktopCommand::SyncAudit { limit: 50 });
    assert!(matches!(audit, DesktopData::SyncAudit(entries) if !entries.is_empty()));
    let pushed_all = dispatch(
        &mut app,
        Some(identity.clone()),
        DesktopCommand::SyncPushAll,
    );
    assert!(
        matches!(pushed_all, DesktopData::SyncPushAll(batch) if batch.items.len() == 2 && batch.tags.complete)
    );

    let pulled = app.sync_pull(&identity).unwrap();
    assert!(pulled.skipped.contains(&note.id));
    assert!(pulled.conflicts.is_empty());
    let idle_watch = app.sync_wait_for_change(&identity, 50).unwrap();
    assert!(idle_watch.configured);
    assert!(!idle_watch.changed);
    let pulled = dispatch(&mut app, Some(identity.clone()), DesktopCommand::SyncPull);
    assert!(matches!(pulled, DesktopData::SyncPull(_)));

    let deleted = app.sync_delete_note(&identity, &note.id).unwrap();
    assert_eq!(deleted.version, 2);
    let trash = app.sync_trash(Some(&identity)).unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].object_id, note.id);
    assert_eq!(trash[0].title.as_deref(), Some("Sync note"));

    let restored = app.sync_restore_note(&identity, &note.id).unwrap();
    assert_eq!(restored.version, 3);
    assert!(app.sync_trash(Some(&identity)).unwrap().is_empty());

    let profile_json = std::fs::read_to_string(app_path.join("sync-profile.json")).unwrap();
    assert!(profile_json.contains("encryption_public_key"));
    assert!(!profile_json.contains("seed"));
    assert!(!profile_json.contains("workspace_key"));
    let local_id = app.sync_profile().unwrap().unwrap().local_id;
    let device_secret = vault::load_at(&format!("PKMS:sync-device:v1:{local_id}"))
        .unwrap()
        .unwrap();
    assert_eq!(device_secret.expose_secret().len(), 64);
    let wsk_secret = vault::load_at(&format!("PKMS:sync-wsk:v1:{workspace_id}:1"))
        .unwrap()
        .unwrap();
    assert_eq!(wsk_secret.expose_secret().len(), 64);

    owner_client
        .revoke_device(&app_device_id, "test cleanup")
        .unwrap();
    let error = app.sync_pull(&identity).unwrap_err();
    assert!(matches!(error, AppError::Sync(_)));

    let _ = vault::delete_at(&format!("PKMS:sync-device:v1:{local_id}"));
    let _ = vault::delete_at(&format!("PKMS:sync-wsk:v1:{workspace_id}:1"));
}
