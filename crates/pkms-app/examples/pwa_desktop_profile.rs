//! Test-only helper: register a desktop device on the phone fixture server and
//! write its sync profile plus vault secrets into an isolated app directory.
use base64::{Engine as _, engine::general_purpose};
use pkms_app::inference_credentials::{self as vault, Secret};
use pkms_sync::{
    DeviceKeyPair, MemberRole, encryption_public_key_hex, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use serde_json::json;

fn main() {
    let endpoint = std::env::args().nth(1).expect("endpoint");
    let workspace_id = std::env::args().nth(2).expect("workspace id");
    let workspace_key_base64 = std::env::args().nth(3).expect("workspace key");
    let app_dir = std::env::args().nth(4).expect("app data dir");
    let owner_device_id = std::env::args().nth(5).expect("owner device id");

    let owner_seed = [81u8; 32];
    let owner_key = DeviceKeyPair::from_seed(owner_seed).unwrap();
    let owner_client = SyncClient::new(&endpoint, &owner_device_id, owner_key).unwrap();

    let desktop_seed = [83u8; 32];
    let desktop_key = DeviceKeyPair::from_seed(desktop_seed).unwrap();
    let desktop_encryption = desktop_key.encryption_public_key().unwrap();
    let desktop_device_id = if let Some(existing) = std::env::args().nth(6) {
        existing
    } else {
        let invite = owner_client
            .create_pairing_invite_with_role(3600, MemberRole::Editor)
            .unwrap();
        let pairing = SyncClient::new(
            &endpoint,
            "",
            DeviceKeyPair::from_seed(desktop_seed).unwrap(),
        )
        .unwrap();
        pairing
            .pair_with_code(&invite.invite_id, &invite.code, "phone test desktop")
            .unwrap()
            .device_id
    };
    let desktop_client = SyncClient::new(
        &endpoint,
        &desktop_device_id,
        DeviceKeyPair::from_seed(desktop_seed).unwrap(),
    )
    .unwrap();
    desktop_client
        .register_encryption_key(&encryption_public_key_hex(&desktop_encryption))
        .unwrap();
    let workspace_key_bytes = general_purpose::STANDARD
        .decode(workspace_key_base64)
        .unwrap();
    let workspace_key = pkms_sync::WorkspaceKey::from_bytes(
        workspace_key_bytes
            .try_into()
            .expect("32-byte workspace key"),
    );
    owner_client
        .grant_workspace_key(
            &workspace_id,
            1,
            &desktop_device_id,
            &wrap_workspace_key(
                desktop_encryption,
                &workspace_key,
                &workspace_key_aad(&workspace_id, 1, &desktop_device_id),
            )
            .unwrap(),
        )
        .unwrap();

    let local_id = uuid::Uuid::new_v4().to_string();
    let profile = json!({
        "local_id": local_id,
        "endpoint": endpoint,
        "device_id": desktop_device_id,
        "display_name": "phone test desktop",
        "signing_public_key": desktop_key.public_key_hex(),
        "encryption_public_key": encryption_public_key_hex(&desktop_encryption),
        "workspace_id": workspace_id,
        "epoch": 1,
    });
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(
        std::path::Path::new(&app_dir).join("sync-profile.json"),
        serde_json::to_vec_pretty(&profile).unwrap(),
    )
    .unwrap();
    vault::store_at(
        &format!("PKMS:sync-device:v1:{local_id}"),
        &Secret::new(desktop_key.seed_hex()).unwrap(),
    )
    .unwrap();
    vault::store_at(
        &format!("PKMS:sync-wsk:v1:{workspace_id}:1"),
        &Secret::new(workspace_key.export_hex()).unwrap(),
    )
    .unwrap();
    println!("PROFILE_WRITTEN {local_id}");
}
