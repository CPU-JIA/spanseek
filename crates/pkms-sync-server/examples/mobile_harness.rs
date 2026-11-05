//! Local verification harness for the mobile PWA protocol.
//!
//! This is not a production server bootstrap. It enrolls one disposable owner,
//! prints a pairing invite, grants epoch 1 after the phone registers its X25519
//! key, and prints decrypted note titles as the phone pushes them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pkms_sync::{
    DeviceKeyPair, OpKind, SYNC_PROTOCOL_VERSION, SealedData, SyncAuthority, SyncOp, WorkspaceKey,
    object_aad, owner_proof_message, parse_encryption_public_key, workspace_key_aad,
    wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use serde_json::json;
use uuid::Uuid;

const OWNER_SEED: [u8; 32] = [0x42u8; 32];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let db = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "mobile-harness.sqlite3".into()),
    );
    let bind = args.next().unwrap_or_else(|| "127.0.0.1:8787".into());
    let workspace_id = Uuid::new_v4().to_string();

    let mut authority = SyncAuthority::open(&db)?;
    let owner_key = DeviceKeyPair::from_seed(OWNER_SEED)?;
    let proof = owner_key.sign(&owner_proof_message(
        &owner_key.public_key_hex(),
        "mobile harness owner",
    ));
    let owner = authority.enroll_owner(&owner_key.public_key(), "mobile harness owner", &proof)?;
    let shared = Arc::new(Mutex::new(authority));

    let listener = std::net::TcpListener::bind(&bind)?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(listener, pkms_sync_server::router(shared))
                .await
                .expect("server");
        });
    });

    let base_url = format!("http://{address}");
    let owner_client = SyncClient::new(&base_url, owner.device_id.clone(), owner_key)?;
    owner_client.register_encryption_key(
        &DeviceKeyPair::from_seed(OWNER_SEED)?.encryption_public_key_hex()?,
    )?;
    let invite = owner_client.create_pairing_invite(600)?;
    println!(
        "{}",
        json!({
            "ready": true,
            "base_url": base_url,
            "workspace_id": workspace_id,
            "owner_device_id": owner.device_id,
            "invite_id": invite.invite_id,
            "code": invite.code,
            "protocol_version": SYNC_PROTOCOL_VERSION,
        })
    );

    let deadline = Instant::now() + Duration::from_secs(300);
    let mut workspace_key = None;
    while Instant::now() < deadline {
        let keys = owner_client.device_encryption_keys()?;
        if keys.len() >= 2 {
            let key = WorkspaceKey::generate()?;
            let mut envelopes = Vec::new();
            for entry in &keys {
                let public = parse_encryption_public_key(&entry.public_key)?;
                envelopes.push((
                    entry.device_id.clone(),
                    wrap_workspace_key(
                        public,
                        &key,
                        &workspace_key_aad(&workspace_id, 1, &entry.device_id),
                    )?,
                ));
            }
            let epoch = owner_client.rotate_workspace_key(&workspace_id, 1, &envelopes)?;
            println!(
                "{}",
                json!({"granted": true, "workspace_id": workspace_id, "epoch": epoch})
            );
            workspace_key = Some(key);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let Some(workspace_key) = workspace_key else {
        return Err("phone did not register an encryption key in time".into());
    };

    let mut seen = std::collections::BTreeMap::new();
    let mut desktop_edited = false;
    loop {
        let snapshot = owner_client.snapshot(&workspace_id)?;
        for object in snapshot {
            if object.tombstone {
                continue;
            }
            let Some(ciphertext) = object.ciphertext.as_deref() else {
                continue;
            };
            if seen.get(&object.object_id) == Some(&object.version) {
                continue;
            }
            if let Ok(plaintext) = SealedData::decode(ciphertext).and_then(|sealed| {
                workspace_key.decrypt(
                    &sealed,
                    &object_aad(&workspace_id, &object.object_id, object.key_epoch),
                )
            }) {
                let payload: serde_json::Value = serde_json::from_slice(&plaintext)?;
                println!(
                    "{}",
                    json!({
                        "note": object.object_id,
                        "version": object.version,
                        "title": payload.get("title"),
                        "bytes": plaintext.len(),
                    })
                );
                seen.insert(object.object_id.clone(), object.version);
                if !desktop_edited {
                    let payload = json!({
                        "title": "桌面编辑回传",
                        "document_schema": 3,
                        "document": {
                            "type": "doc",
                            "content": [{
                                "type": "paragraph",
                                "attrs": {"id": Uuid::new_v4().to_string()},
                                "content": [{"type": "text", "text": "桌面端修改后，手机下一次同步应看到这段正文。"}]
                            }]
                        },
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                    let sealed = workspace_key.encrypt(
                        &serde_json::to_vec(&payload)?,
                        &object_aad(&workspace_id, &object.object_id, 1),
                    )?;
                    owner_client.apply_op(&SyncOp {
                        op_id: format!("harness-desktop-edit-{}", object.object_id),
                        workspace_id: workspace_id.clone(),
                        object_id: object.object_id.clone(),
                        kind: OpKind::Put,
                        base_version: object.version,
                        key_epoch: 1,
                        ciphertext: Some(sealed.encode()),
                        client_time: Some(chrono::Utc::now().to_rfc3339()),
                    })?;
                    desktop_edited = true;
                    println!(
                        "{}",
                        json!({"desktop_edit": object.object_id, "base_version": object.version})
                    );
                }
            }
        }
        std::thread::sleep(Duration::from_millis(750));
    }
}
