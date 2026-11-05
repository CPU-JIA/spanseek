//! Test helper: make a signed desktop edit against the disposable mobile harness.

use pkms_sync::{DeviceKeyPair, OpKind, SyncOp, object_aad, workspace_key_aad};
use pkms_sync_client::SyncClient;

const OWNER_SEED: [u8; 32] = [0x42u8; 32];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let base_url = args.next().expect("base_url");
    let workspace_id = args.next().expect("workspace_id");
    let object_id = args.next().expect("object_id");
    let base_version: i64 = args.next().expect("base_version").parse()?;
    let owner_device_id = args.next().expect("owner_device_id");
    let key = DeviceKeyPair::from_seed(OWNER_SEED)?;
    let _ = key;
    let client = SyncClient::new(
        &base_url,
        &owner_device_id,
        DeviceKeyPair::from_seed(OWNER_SEED)?,
    )?;
    let envelope = client.fetch_key_envelope(&workspace_id, 1)?;
    let workspace_key = DeviceKeyPair::from_seed(OWNER_SEED)?.unwrap_workspace_key(
        &envelope,
        &workspace_key_aad(&workspace_id, 1, &owner_device_id),
    )?;
    let payload = serde_json::json!({
        "title": "桌面编辑回传",
        "document_schema": 3,
        "document": {
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": {"id": uuid::Uuid::new_v4().to_string()},
                "content": [{"type": "text", "text": "桌面端修改后，手机下一次同步应看到这段正文。"}]
            }]
        },
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    let sealed = workspace_key.encrypt(
        &serde_json::to_vec(&payload)?,
        &object_aad(&workspace_id, &object_id, 1),
    )?;
    let applied = client.apply_op(&SyncOp {
        op_id: format!("harness-desktop-edit-{object_id}"),
        workspace_id,
        object_id,
        kind: OpKind::Put,
        base_version,
        key_epoch: 1,
        ciphertext: Some(sealed.encode()),
        client_time: Some(chrono::Utc::now().to_rfc3339()),
    })?;
    println!("{}", serde_json::to_string(&applied)?);
    Ok(())
}
