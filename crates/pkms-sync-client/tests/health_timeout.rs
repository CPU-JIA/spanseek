use std::io::Read;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use pkms_sync::DeviceKeyPair;
use pkms_sync_client::{ClientError, SyncClient};

#[test]
fn health_times_out_without_holding_callers_for_the_general_client_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        thread::sleep(Duration::from_secs(2));
    });

    let seed = DeviceKeyPair::generate_seed().unwrap();
    let device = DeviceKeyPair::from_seed(seed).unwrap();
    let client = SyncClient::new(format!("http://{address}"), "test-device", device).unwrap();

    let started = Instant::now();
    let error = client.health().unwrap_err();
    let elapsed = started.elapsed();

    assert!(
        matches!(error, ClientError::Transport(ref source) if source.is_timeout()),
        "expected a transport timeout, got {error:?}"
    );
    assert!(
        elapsed < Duration::from_millis(2_500),
        "health check took {elapsed:?}"
    );
    server.join().unwrap();
}
