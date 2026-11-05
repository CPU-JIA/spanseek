use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use pkms_sync::{
    DeviceKeyPair, KeyEnvelope, MemberRole, SealedData, SyncAuthority, WorkspaceKey, hex_decode,
    object_aad, owner_proof_message, workspace_key_aad, wrap_workspace_key,
};
use pkms_sync_client::SyncClient;
use serde_json::json;
use tempfile::tempdir;

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
        if std::net::TcpStream::connect(address).is_ok() {
            return base_url;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("server did not start");
}

#[test]
fn browser_media_recorder_uploads_to_a_real_sync_server() {
    let directory = tempdir().unwrap();
    let blob_root = directory.path().join("blobs");
    let mut authority = SyncAuthority::open_in_memory_with_blobs(&blob_root).unwrap();
    let owner_key = DeviceKeyPair::from_seed([45u8; 32]).unwrap();
    let owner_proof = owner_key.sign(&owner_proof_message(&owner_key.public_key_hex(), "owner"));
    let owner = authority
        .enroll_owner(&owner_key.public_key(), "owner", &owner_proof)
        .unwrap();
    let base_url = start_server(authority);
    let owner_encryption = owner_key.encryption_public_key().unwrap();
    let owner_client = SyncClient::new(&base_url, &owner.device_id, owner_key).unwrap();
    owner_client
        .register_encryption_key(&pkms_sync::encryption_public_key_hex(&owner_encryption))
        .unwrap();

    let workspace_id = "11111111-1111-4111-8111-111111111111";
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

    let seed = [46u8; 32];
    let pwa_key = DeviceKeyPair::from_seed(seed).unwrap();
    let pwa_encryption = pwa_key.encryption_public_key().unwrap();
    let invite = owner_client
        .create_pairing_invite_with_role(600, MemberRole::Editor)
        .unwrap();
    let pwa_session =
        SyncClient::new(&base_url, "", DeviceKeyPair::from_seed(seed).unwrap()).unwrap();
    let pwa = pwa_session
        .pair_with_code(&invite.invite_id, &invite.code, "phone browser")
        .unwrap();
    let pwa_authed = SyncClient::new(
        &base_url,
        &pwa.device_id,
        DeviceKeyPair::from_seed(seed).unwrap(),
    )
    .unwrap();
    pwa_authed
        .register_encryption_key(&pkms_sync::encryption_public_key_hex(&pwa_encryption))
        .unwrap();
    let pwa_envelope: KeyEnvelope = wrap_workspace_key(
        pwa_encryption,
        &workspace_key,
        &workspace_key_aad(workspace_id, 1, &pwa.device_id),
    )
    .unwrap();
    owner_client
        .grant_workspace_key(workspace_id, 1, &pwa.device_id, &pwa_envelope)
        .unwrap();

    let workspace_key_bytes: [u8; 32] = hex_decode(&workspace_key.export_hex())
        .unwrap()
        .try_into()
        .unwrap();
    let workspace_key_base64 =
        base64::engine::general_purpose::STANDARD.encode(workspace_key_bytes);
    let public = hex_decode(&pwa_key.public_key_hex()).unwrap();
    let ed_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "d": URL_SAFE_NO_PAD.encode(seed),
        "x": URL_SAFE_NO_PAD.encode(public),
    });
    let capture_id = uuid::Uuid::new_v4().to_string();
    let fixture = json!({
        "profile": {
            "endpoint": base_url,
            "workspaceId": workspace_id,
            "epoch": 1,
            "deviceId": pwa.device_id,
            "displayName": "phone browser",
        },
        "edPrivateJwk": ed_jwk,
        "workspaceKeyBase64": workspace_key_base64,
        "captureId": capture_id,
    });

    let fixture_base64 =
        base64::engine::general_purpose::STANDARD.encode(fixture.to_string().as_bytes());
    let port = serve_pwa();
    let chrome = browser_binary();
    let profile = std::env::temp_dir().join(format!("spanseek-chrome-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile).unwrap();
    let url = format!("http://127.0.0.1:{port}/tests/browser-upload.html#{fixture_base64}");
    let debug_port = free_port();
    let mut chrome_process = std::process::Command::new(chrome)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--no-first-run",
            "--use-fake-device-for-media-stream",
            "--use-fake-ui-for-media-stream",
            "--autoplay-policy=no-user-gesture-required",
            &format!("--remote-debugging-port={debug_port}"),
            &format!("--user-data-dir={}", profile.display()),
            &url,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", debug_port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let wait_script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/mobile-capture/tests/browser-wait.mjs");
    let node = std::env::var("NODE_BIN").unwrap_or_else(|_| "node".into());
    let output = std::process::Command::new(node)
        .arg(wait_script)
        .arg(debug_port.to_string())
        .output()
        .unwrap();
    let _ = chrome_process.kill();
    chrome_process.wait().expect("reap browser process");
    let dom = String::from_utf8_lossy(&output.stdout);
    assert!(
        dom.contains("UPLOADED"),
        "browser upload failed: {} / {}",
        dom.chars().take(400).collect::<String>(),
        String::from_utf8_lossy(&output.stderr)
    );

    let batch = owner_client.pull(workspace_id, 0, 20).unwrap();
    let note = batch
        .ops
        .iter()
        .find(|op| op.object_id == capture_id)
        .expect("marker note op");
    let sealed = SealedData::decode(note.ciphertext.as_ref().unwrap()).unwrap();
    let plain = workspace_key
        .decrypt(&sealed, &object_aad(workspace_id, &capture_id, 1))
        .unwrap();
    let text = String::from_utf8(plain).unwrap();
    assert!(text.contains("<!-- spanseek-voice:v1:"));
    assert!(text.contains("800"));
    let marker = text
        .split("<!-- spanseek-voice:v1:")
        .nth(1)
        .unwrap()
        .split(" -->")
        .next()
        .unwrap();
    let server_hash = marker.split(':').next().unwrap();
    assert_eq!(server_hash.len(), 64);
    let status = owner_client.blob_status(server_hash).unwrap();
    assert!(status.complete);
}

fn serve_pwa() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/mobile-capture")
        .canonicalize()
        .unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let Ok(read) = stream.read(&mut buffer) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..read]);
            let path = request
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap();
            let relative = path.trim_start_matches('/');
            let file = root.join(relative);
            let (status, content_type, body) = if file.starts_with(&root) && file.is_file() {
                let body = std::fs::read(&file).unwrap_or_default();
                let content_type = if path.ends_with(".html") {
                    "text/html; charset=utf-8"
                } else {
                    "text/javascript; charset=utf-8"
                };
                ("200 OK", content_type, body)
            } else {
                ("404 Not Found", "text/plain", b"not found".to_vec())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    port
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn browser_binary() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("CHROME_BIN") {
        return path.into();
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        let Some(root) = std::env::var_os(variable) else {
            continue;
        };
        for relative in [
            "Microsoft/Edge/Application/msedge.exe",
            "Google/Chrome/Application/chrome.exe",
        ] {
            let candidate = std::path::Path::new(&root).join(relative);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("Set CHROME_BIN to an installed Chromium/Edge executable for the browser upload test");
}
