//! Test-only fixture: one owner, one phone device, one workspace key.
//! Prints a JSON line with the phone profile, then serves until killed.
use std::io::Write as _;
use std::sync::{Arc, Mutex};

use base64::{Engine as _, engine::general_purpose};
use pkms_sync::{
    DeviceKeyPair, MemberRole, SyncAuthority, WorkspaceKey, encryption_public_key_hex,
    owner_proof_message, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use serde_json::json;

fn main() {
    let db = std::env::args().nth(1).expect("db path");
    let blobs = std::env::args().nth(2).expect("blob root");
    let port: u16 = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "8787".into())
        .parse()
        .expect("port");
    let workspace_id = std::env::args()
        .nth(4)
        .unwrap_or_else(|| "22222222-2222-4222-8222-222222222222".into());

    let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).expect("authority");
    let owner_key = DeviceKeyPair::from_seed([81u8; 32]).unwrap();
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "fixture owner",
    ));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "fixture owner", &proof)
        .unwrap();
    let owner_encryption = owner_key.encryption_public_key().unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
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

    let base_url = format!("http://127.0.0.1:{port}");
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let owner_client = SyncClient::new(&base_url, &owner.device_id, owner_key).unwrap();
    owner_client
        .register_encryption_key(&encryption_public_key_hex(&owner_encryption))
        .unwrap();
    let workspace_id = workspace_id.as_str();
    let workspace_key = WorkspaceKey::generate().unwrap();
    let owner_envelope = wrap_workspace_key(
        owner_encryption,
        &workspace_key,
        &workspace_key_aad(workspace_id, 1, &owner.device_id),
    )
    .unwrap();
    owner_client
        .rotate_workspace_key(
            workspace_id,
            1,
            &[(owner.device_id.clone(), owner_envelope)],
        )
        .unwrap();

    let phone_seed = [82u8; 32];
    let phone_key = DeviceKeyPair::from_seed(phone_seed).unwrap();
    let phone_encryption = phone_key.encryption_public_key().unwrap();
    let invite = owner_client
        .create_pairing_invite_with_role(3600, MemberRole::Editor)
        .unwrap();
    let pairing =
        SyncClient::new(&base_url, "", DeviceKeyPair::from_seed(phone_seed).unwrap()).unwrap();
    let phone = pairing
        .pair_with_code(&invite.invite_id, &invite.code, "OnePlus phone")
        .unwrap();
    let phone_client = SyncClient::new(
        &base_url,
        &phone.device_id,
        DeviceKeyPair::from_seed(phone_seed).unwrap(),
    )
    .unwrap();
    phone_client
        .register_encryption_key(&encryption_public_key_hex(&phone_encryption))
        .unwrap();
    owner_client
        .grant_workspace_key(
            workspace_id,
            1,
            &phone.device_id,
            &wrap_workspace_key(
                phone_encryption,
                &workspace_key,
                &workspace_key_aad(workspace_id, 1, &phone.device_id),
            )
            .unwrap(),
        )
        .unwrap();

    let key_bytes: [u8; 32] = pkms_sync::hex_decode(&workspace_key.export_hex())
        .unwrap()
        .try_into()
        .unwrap();
    let phone_public = pkms_sync::hex_decode(&phone_key.public_key_hex()).unwrap();
    let fixture = json!({
        "endpoint": base_url,
        "workspaceId": workspace_id,
        "epoch": 1,
        "deviceId": phone.device_id,
        "displayName": "OnePlus phone",
        "edPrivateJwk": {
            "kty": "OKP",
            "crv": "Ed25519",
            "d": general_purpose::URL_SAFE_NO_PAD.encode(phone_seed),
            "x": general_purpose::URL_SAFE_NO_PAD.encode(phone_public),
        },
        "workspaceKeyBase64": general_purpose::STANDARD.encode(key_bytes),
    });
    println!("FIXTURE {}", fixture);
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
