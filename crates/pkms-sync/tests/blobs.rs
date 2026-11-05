use chrono::Utc;
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, SignedRequest, SyncAccess, SyncAuthority, blob_begin_request_hash,
    blob_chunk_request_hash, blob_finalize_request_hash, blob_read_request_hash,
    blob_status_request_hash, owner_proof_message, pairing_proof_message, sha256_hex,
};

const CHUNK: i64 = 64 * 1024;

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

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

fn request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    access: SyncAccess,
    hash: [u8; 32],
    nonce: &str,
) -> SignedRequest {
    key.sign_request(
        &device.device_id,
        access,
        nonce,
        Utc::now().timestamp(),
        hash,
    )
}

fn begin_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    size: i64,
    chunk_size: i64,
    nonce: &str,
) -> SignedRequest {
    request(
        key,
        device,
        SyncAccess::Write,
        blob_begin_request_hash(hash, size, chunk_size),
        nonce,
    )
}

fn chunk_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    index: u32,
    data: &[u8],
    nonce: &str,
) -> SignedRequest {
    request(
        key,
        device,
        SyncAccess::Write,
        blob_chunk_request_hash(hash, index, data.len() as i64, &sha256_hex(data)),
        nonce,
    )
}

fn status_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    nonce: &str,
) -> SignedRequest {
    request(
        key,
        device,
        SyncAccess::Read,
        blob_status_request_hash(hash),
        nonce,
    )
}

fn finalize_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    nonce: &str,
) -> SignedRequest {
    request(
        key,
        device,
        SyncAccess::Write,
        blob_finalize_request_hash(hash),
        nonce,
    )
}

fn read_request(
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    index: u32,
    nonce: &str,
) -> SignedRequest {
    request(
        key,
        device,
        SyncAccess::Read,
        blob_read_request_hash(hash, index),
        nonce,
    )
}

fn upload_all(
    authority: &mut SyncAuthority,
    key: &DeviceKeyPair,
    device: &DeviceRecord,
    hash: &str,
    data: &[u8],
) {
    let chunk_size = CHUNK as usize;
    for (index, chunk) in data.chunks(chunk_size).enumerate() {
        let nonce = format!("nonce-upload-{index:04}");
        authority
            .upload_blob_chunk(
                &chunk_request(key, device, hash, index as u32, chunk, &nonce),
                hash,
                index as u32,
                chunk,
                Utc::now().timestamp(),
            )
            .unwrap();
    }
}

fn setup() -> (
    SyncAuthority,
    DeviceKeyPair,
    DeviceRecord,
    DeviceKeyPair,
    DeviceRecord,
) {
    let directory = tempfile::tempdir().unwrap();
    // Leak the TempDir for the duration of the in-memory authority: dropping it
    // would remove the blob root while the authority is still in use.
    let root = directory.keep();
    let mut authority = SyncAuthority::open_in_memory_with_blobs(&root).unwrap();
    let (owner_key, owner) = enroll_owner(&mut authority);
    let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
    (authority, owner_key, owner, phone_key, phone)
}

#[test]
fn upload_finalize_and_download_roundtrip() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let data = payload((CHUNK * 2 + CHUNK / 2) as usize);
    let hash = sha256_hex(&data);
    let started = authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                &hash,
                data.len() as i64,
                CHUNK,
                "nonce-begin-0001",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(!started.already_present);
    assert_eq!(started.status.chunk_count, 3);
    upload_all(&mut authority, &owner_key, &owner, &hash, &data);
    let status = authority
        .blob_status(
            &status_request(&owner_key, &owner, &hash, "nonce-status-0001"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(status.missing_chunks.is_empty());
    let record = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "nonce-finalize-0001"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(record.complete);
    assert_eq!(record.size, data.len() as i64);
    assert_eq!(record.blob_hash, hash);

    let mut rebuilt = Vec::new();
    for index in 0..3u32 {
        let chunk = authority
            .read_blob_chunk(
                &read_request(
                    &phone_key,
                    &phone,
                    &hash,
                    index,
                    &format!("nonce-read-chunk-{index:04}"),
                ),
                &hash,
                index,
                Utc::now().timestamp(),
            )
            .unwrap();
        rebuilt.extend_from_slice(&chunk);
    }
    assert_eq!(sha256_hex(&rebuilt), hash);
    assert_eq!(rebuilt, data);
}

#[test]
fn dedup_reports_already_present_for_second_device() {
    let (mut authority, owner_key, owner, phone_key, phone) = setup();
    let data = payload(CHUNK as usize);
    let hash = sha256_hex(&data);
    authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                &hash,
                data.len() as i64,
                CHUNK,
                "nonce-dedup-0001",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    upload_all(&mut authority, &owner_key, &owner, &hash, &data);
    authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "nonce-dedup-0002"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    let second = authority
        .begin_blob(
            &begin_request(
                &phone_key,
                &phone,
                &hash,
                data.len() as i64,
                CHUNK,
                "nonce-dedup-0003",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(second.already_present);
    assert!(second.status.complete);
    let chunk = authority
        .read_blob_chunk(
            &read_request(&phone_key, &phone, &hash, 0, "nonce-dedup-0004"),
            &hash,
            0,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(chunk, data);
    let error = authority
        .upload_blob_chunk(
            &chunk_request(&owner_key, &owner, &hash, 0, &data, "nonce-dedup-0005"),
            &hash,
            0,
            &data,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "blob_already_complete");
}

#[test]
fn interrupted_upload_resumes_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("authority.sqlite3");
    let blobs = directory.path().join("blobs");
    let data = payload((CHUNK * 3) as usize);
    let hash = sha256_hex(&data);
    let (owner_key, owner) = {
        let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).unwrap();
        let (owner_key, owner) = enroll_owner(&mut authority);
        authority
            .begin_blob(
                &begin_request(
                    &owner_key,
                    &owner,
                    &hash,
                    data.len() as i64,
                    CHUNK,
                    "nonce-resume-0001",
                ),
                &hash,
                data.len() as i64,
                CHUNK,
                Utc::now().timestamp(),
            )
            .unwrap();
        for index in [0usize, 2] {
            let chunk = &data[index * CHUNK as usize..(index + 1) * CHUNK as usize];
            authority
                .upload_blob_chunk(
                    &chunk_request(
                        &owner_key,
                        &owner,
                        &hash,
                        index as u32,
                        chunk,
                        &format!("nonce-resume-chunk-{index}"),
                    ),
                    &hash,
                    index as u32,
                    chunk,
                    Utc::now().timestamp(),
                )
                .unwrap();
        }
        (owner_key, owner)
    };

    let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).unwrap();
    let status = authority
        .blob_status(
            &status_request(&owner_key, &owner, &hash, "nonce-resume-status-1"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(status.received_chunks, vec![0, 2]);
    assert_eq!(status.missing_chunks, vec![1]);
    let chunk = &data[CHUNK as usize..2 * CHUNK as usize];
    authority
        .upload_blob_chunk(
            &chunk_request(&owner_key, &owner, &hash, 1, chunk, "nonce-resume-chunk-1"),
            &hash,
            1,
            chunk,
            Utc::now().timestamp(),
        )
        .unwrap();
    let record = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "nonce-resume-final-1"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(record.complete);
    let mut rebuilt = Vec::new();
    for index in 0..3u32 {
        rebuilt.extend_from_slice(
            &authority
                .read_blob_chunk(
                    &read_request(
                        &owner_key,
                        &owner,
                        &hash,
                        index,
                        &format!("nonce-resume-read-{index}"),
                    ),
                    &hash,
                    index,
                    Utc::now().timestamp(),
                )
                .unwrap(),
        );
    }
    assert_eq!(sha256_hex(&rebuilt), hash);
}

#[test]
fn tampered_chunk_is_rejected_and_can_be_retried() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let data = payload(CHUNK as usize);
    let hash = sha256_hex(&data);
    authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                &hash,
                data.len() as i64,
                CHUNK,
                "nonce-tamper-0001",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    let mut tampered = data.clone();
    tampered[0] ^= 0xff;
    let error = authority
        .upload_blob_chunk(
            &chunk_request(&owner_key, &owner, &hash, 0, &data, "nonce-tamper-0002"),
            &hash,
            0,
            &tampered,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "blob_chunk_mismatch");
    let status = authority
        .blob_status(
            &status_request(&owner_key, &owner, &hash, "nonce-tamper-0003"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(status.missing_chunks, vec![0]);
    authority
        .upload_blob_chunk(
            &chunk_request(&owner_key, &owner, &hash, 0, &data, "nonce-tamper-0004"),
            &hash,
            0,
            &data,
            Utc::now().timestamp(),
        )
        .unwrap();
}

#[test]
fn wrong_manifest_hash_fails_closed() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let data = payload(CHUNK as usize);
    let manifest = sha256_hex(b"a different original");
    authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                &manifest,
                data.len() as i64,
                CHUNK,
                "nonce-wronghash-0001",
            ),
            &manifest,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    upload_all(&mut authority, &owner_key, &owner, &manifest, &data);
    let error = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &manifest, "nonce-wronghash-0002"),
            &manifest,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "blob_hash_mismatch");
    let error = authority
        .read_blob_chunk(
            &read_request(&owner_key, &owner, &manifest, 0, "nonce-wronghash-0003"),
            &manifest,
            0,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "blob_not_finalized");
}

#[test]
fn blob_access_and_hash_validation() {
    let (mut authority, owner_key, owner, _phone_key, _phone) = setup();
    let hash = sha256_hex(b"x");
    let wrong_access = request(
        &owner_key,
        &owner,
        SyncAccess::Write,
        blob_status_request_hash(&hash),
        "nonce-blob-access-0001",
    );
    assert_eq!(
        authority
            .blob_status(&wrong_access, &hash, Utc::now().timestamp())
            .unwrap_err()
            .code(),
        "invalid_access"
    );
    let wrong_hash = request(
        &owner_key,
        &owner,
        SyncAccess::Read,
        [0u8; 32],
        "nonce-blob-access-0002",
    );
    assert_eq!(
        authority
            .blob_status(&wrong_hash, &hash, Utc::now().timestamp())
            .unwrap_err()
            .code(),
        "invalid_request_hash"
    );
    let invalid = authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                "not-a-hash",
                1,
                CHUNK,
                "nonce-blob-access-0003",
            ),
            "not-a-hash",
            1,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap_err();
    assert_eq!(invalid.code(), "invalid_blob");
}

#[test]
fn v4_database_migrates_to_v5_blob_schema() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("v4.sqlite3");
    let blobs = directory.path().join("blobs");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
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
    }
    let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).unwrap();
    assert_eq!(authority.schema_version().unwrap(), 9);
    let (owner_key, owner) = enroll_owner(&mut authority);
    let data = payload(CHUNK as usize);
    let hash = sha256_hex(&data);
    authority
        .begin_blob(
            &begin_request(
                &owner_key,
                &owner,
                &hash,
                data.len() as i64,
                CHUNK,
                "nonce-migrate-blob-1",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .unwrap();
    upload_all(&mut authority, &owner_key, &owner, &hash, &data);
    let record = authority
        .finalize_blob(
            &finalize_request(&owner_key, &owner, &hash, "nonce-migrate-blob-2"),
            &hash,
            Utc::now().timestamp(),
        )
        .unwrap();
    assert!(record.complete);
}
