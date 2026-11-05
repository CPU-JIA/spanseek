//! Reproducible G06 evidence: interrupted chunk upload, resume after reopen,
//! content hash verification, dedup and read-back.

use chrono::Utc;
use pkms_sync::{
    DeviceKeyPair, DeviceRecord, SignedRequest, SyncAccess, SyncAuthority, blob_begin_request_hash,
    blob_chunk_request_hash, blob_finalize_request_hash, blob_read_request_hash,
    blob_status_request_hash, owner_proof_message, pairing_proof_message, sha256_hex,
};
use serde_json::json;

const CHUNK: i64 = 64 * 1024;

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

fn enroll_owner(authority: &mut SyncAuthority) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().expect("owner key");
    let proof = key.sign(&owner_proof_message(&key.public_key_hex(), "owner"));
    let device = authority
        .enroll_owner(&key.public_key(), "owner", &proof)
        .expect("owner");
    (key, device)
}

fn pair_device(
    authority: &mut SyncAuthority,
    creator: &DeviceRecord,
    name: &str,
) -> (DeviceKeyPair, DeviceRecord) {
    let key = DeviceKeyPair::generate().expect("phone key");
    let invite = authority
        .create_pairing_invite(&creator.device_id, 600)
        .expect("invite");
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
        .expect("pair");
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

fn main() {
    let directory = tempfile::tempdir().expect("tempdir");
    let db = directory.path().join("authority.sqlite3");
    let blobs = directory.path().join("blobs");
    let data = payload((CHUNK * 2 + CHUNK / 2) as usize);
    let hash = sha256_hex(&data);

    let (owner_key, owner, phone_key, phone) = {
        let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).expect("authority");
        let (owner_key, owner) = enroll_owner(&mut authority);
        let (phone_key, phone) = pair_device(&mut authority, &owner, "phone");
        authority
            .begin_blob(
                &request(
                    &owner_key,
                    &owner,
                    SyncAccess::Write,
                    blob_begin_request_hash(&hash, data.len() as i64, CHUNK),
                    "nonce-demo-blob-begin-1",
                ),
                &hash,
                data.len() as i64,
                CHUNK,
                Utc::now().timestamp(),
            )
            .expect("begin");
        for index in [0usize, 2] {
            let start = index * CHUNK as usize;
            let end = ((index + 1) * CHUNK as usize).min(data.len());
            let chunk = &data[start..end];
            authority
                .upload_blob_chunk(
                    &request(
                        &owner_key,
                        &owner,
                        SyncAccess::Write,
                        blob_chunk_request_hash(
                            &hash,
                            index as u32,
                            chunk.len() as i64,
                            &sha256_hex(chunk),
                        ),
                        &format!("nonce-demo-blob-chunk-{index}"),
                    ),
                    &hash,
                    index as u32,
                    chunk,
                    Utc::now().timestamp(),
                )
                .expect("chunk");
        }
        let status = authority
            .blob_status(
                &request(
                    &owner_key,
                    &owner,
                    SyncAccess::Read,
                    blob_status_request_hash(&hash),
                    "nonce-demo-blob-status-1",
                ),
                &hash,
                Utc::now().timestamp(),
            )
            .expect("status");
        assert_eq!(status.missing_chunks, vec![1]);
        (owner_key, owner, phone_key, phone)
    };

    let mut authority = SyncAuthority::open_with_blobs(&db, &blobs).expect("reopen");
    let resumed = authority
        .blob_status(
            &request(
                &owner_key,
                &owner,
                SyncAccess::Read,
                blob_status_request_hash(&hash),
                "nonce-demo-blob-status-2",
            ),
            &hash,
            Utc::now().timestamp(),
        )
        .expect("resumed status");
    let chunk = &data[CHUNK as usize..2 * CHUNK as usize];
    authority
        .upload_blob_chunk(
            &request(
                &owner_key,
                &owner,
                SyncAccess::Write,
                blob_chunk_request_hash(&hash, 1, chunk.len() as i64, &sha256_hex(chunk)),
                "nonce-demo-blob-chunk-1",
            ),
            &hash,
            1,
            chunk,
            Utc::now().timestamp(),
        )
        .expect("resume chunk");
    let record = authority
        .finalize_blob(
            &request(
                &owner_key,
                &owner,
                SyncAccess::Write,
                blob_finalize_request_hash(&hash),
                "nonce-demo-blob-final-1",
            ),
            &hash,
            Utc::now().timestamp(),
        )
        .expect("finalize");
    let dedup = authority
        .begin_blob(
            &request(
                &phone_key,
                &phone,
                SyncAccess::Write,
                blob_begin_request_hash(&hash, data.len() as i64, CHUNK),
                "nonce-demo-blob-begin-2",
            ),
            &hash,
            data.len() as i64,
            CHUNK,
            Utc::now().timestamp(),
        )
        .expect("dedup");

    let mut rebuilt = Vec::new();
    for index in 0..3u32 {
        let chunk = authority
            .read_blob_chunk(
                &request(
                    &phone_key,
                    &phone,
                    SyncAccess::Read,
                    blob_read_request_hash(&hash, index),
                    &format!("nonce-demo-blob-read-{index}"),
                ),
                &hash,
                index,
                Utc::now().timestamp(),
            )
            .expect("read chunk");
        rebuilt.extend_from_slice(&chunk);
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-blob-demo-v1",
            "blob_hash": hash,
            "size": data.len(),
            "chunk_size": CHUNK,
            "chunk_count": record.chunk_count,
            "interrupted_missing_chunks": [1],
            "resumed_missing_chunks": resumed.missing_chunks,
            "resumed_received_chunks": resumed.received_chunks,
            "finalized": record.complete,
            "dedup_already_present": dedup.already_present,
            "reassembled_matches_original": sha256_hex(&rebuilt) == hash,
            "reassembled_hash": sha256_hex(&rebuilt),
        }))
        .expect("json")
    );
}
