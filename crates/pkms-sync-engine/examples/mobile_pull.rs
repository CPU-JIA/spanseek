//! Test helper: pull the mobile harness workspace into a fresh desktop library.

use pkms_core::Library;
use pkms_sync::{DeviceKeyPair, workspace_key_aad};
use pkms_sync_client::SyncClient;
use pkms_sync_engine::SyncEngine;

const OWNER_SEED: [u8; 32] = [0x42u8; 32];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let base_url = args.next().expect("base_url");
    let workspace_id = args.next().expect("workspace_id");
    let owner_device_id = args.next().expect("owner_device_id");
    let library_path = args.next().expect("library_path");
    let key = DeviceKeyPair::from_seed(OWNER_SEED)?;
    let client = SyncClient::new(&base_url, &owner_device_id, key)?;
    let envelope = client.fetch_key_envelope(&workspace_id, 1)?;
    let workspace_key = DeviceKeyPair::from_seed(OWNER_SEED)?.unwrap_workspace_key(
        &envelope,
        &workspace_key_aad(&workspace_id, 1, &owner_device_id),
    )?;
    let mut library = Library::open(library_path)?;
    let mut engine = SyncEngine::new(client, workspace_key, workspace_id, 1);
    let summary = engine.pull_and_apply(&mut library, 100)?;
    let notes = library.list_notes()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "applied": summary.applied,
            "conflicts": summary.server_conflicts.len(),
            "notes": notes.iter().map(|note| serde_json::json!({
                "id": note.id,
                "title": note.title,
                "body": note.document.plain_text(),
            })).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
