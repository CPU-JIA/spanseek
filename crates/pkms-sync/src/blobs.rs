//! G06 immutable blob transfer: content-addressed chunks, dedup and resume.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::crypto::{canonical, hex_encode, sha256_array, sha256_hex};
use crate::model::{BlobBegin, BlobRecord, BlobStatus};
use crate::{SignedRequest, SyncAccess, SyncAuthority, SyncError};

pub const MIN_CHUNK_SIZE: i64 = 64 * 1024;
pub const MAX_CHUNK_SIZE: i64 = 8 * 1024 * 1024;
pub const MAX_BLOB_BYTES: i64 = 8 * 1024 * 1024 * 1024;
pub const MAX_BLOB_CHUNKS: i64 = 100_000;

impl SyncAuthority {
    pub fn begin_blob(
        &mut self,
        request: &SignedRequest,
        blob_hash: &str,
        size: i64,
        chunk_size: i64,
        now_unix: i64,
    ) -> Result<BlobBegin, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        let hash = normalize_blob_hash(blob_hash)?;
        if !(1..=MAX_BLOB_BYTES).contains(&size) {
            return Err(SyncError::InvalidBlob("blob size out of range".into()));
        }
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
            return Err(SyncError::InvalidBlob("chunk size out of range".into()));
        }
        let chunk_count = (size + chunk_size - 1) / chunk_size;
        if !(1..=MAX_BLOB_CHUNKS).contains(&chunk_count) {
            return Err(SyncError::InvalidBlob("too many blob chunks".into()));
        }
        let expected = blob_begin_request_hash(&hash, size, chunk_size);
        if request.request_hash != expected {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        if let Some(record) = self.blob_record(&hash)? {
            if record.complete {
                return Ok(BlobBegin {
                    already_present: true,
                    status: self.blob_status_inner(&hash)?,
                });
            }
            if record.size != size || record.chunk_size != chunk_size {
                return Err(SyncError::BlobConflict);
            }
            return Ok(BlobBegin {
                already_present: false,
                status: self.blob_status_inner(&hash)?,
            });
        }
        let root = self.blob_root()?;
        fs::create_dir_all(staging_dir(root, &hash))?;
        self.connection.execute(
            "INSERT INTO sync_blobs(blob_hash,size,chunk_size,chunk_count,status,created_at,created_by) VALUES(?1,?2,?3,?4,'pending',?5,?6)",
            params![
                hash,
                size,
                chunk_size,
                chunk_count,
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                device.device_id
            ],
        )?;
        Ok(BlobBegin {
            already_present: false,
            status: self.blob_status_inner(&hash)?,
        })
    }

    /// Idempotent chunk upload. A retry with the same bytes is safe; a chunk
    /// may be replaced while the blob is still pending.
    pub fn upload_blob_chunk(
        &mut self,
        request: &SignedRequest,
        blob_hash: &str,
        index: u32,
        data: &[u8],
        now_unix: i64,
    ) -> Result<BlobStatus, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        let hash = normalize_blob_hash(blob_hash)?;
        let record = self.blob_record(&hash)?.ok_or(SyncError::BlobNotFound)?;
        if record.complete {
            return Err(SyncError::BlobAlreadyComplete);
        }
        if i64::from(index) >= record.chunk_count {
            return Err(SyncError::InvalidBlob("chunk index out of range".into()));
        }
        let expected_len = chunk_len(record.size, record.chunk_size, i64::from(index));
        if data.len() as i64 != expected_len {
            return Err(SyncError::BlobChunkMismatch);
        }
        let digest = sha256_hex(data);
        if request.request_hash != blob_chunk_request_hash(&hash, index, data.len() as i64, &digest)
        {
            return Err(SyncError::BlobChunkMismatch);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let root = self.blob_root()?;
        let directory = staging_dir(root, &hash);
        fs::create_dir_all(&directory)?;
        let path = chunk_path(root, &hash, index);
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        self.connection.execute(
            "INSERT INTO sync_blob_chunks(blob_hash,chunk_index,chunk_len,chunk_hash,received_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(blob_hash,chunk_index) DO UPDATE SET chunk_len=excluded.chunk_len,chunk_hash=excluded.chunk_hash,received_at=excluded.received_at",
            params![
                hash,
                index,
                data.len() as i64,
                digest,
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
            ],
        )?;
        self.blob_status_inner(&hash)
    }

    pub fn blob_status(
        &mut self,
        request: &SignedRequest,
        blob_hash: &str,
        now_unix: i64,
    ) -> Result<BlobStatus, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        let hash = normalize_blob_hash(blob_hash)?;
        if request.request_hash != blob_status_request_hash(&hash) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        self.blob_status_inner(&hash)
    }

    pub fn finalize_blob(
        &mut self,
        request: &SignedRequest,
        blob_hash: &str,
        now_unix: i64,
    ) -> Result<BlobRecord, SyncError> {
        if request.access != SyncAccess::Write {
            return Err(SyncError::InvalidAccess);
        }
        let hash = normalize_blob_hash(blob_hash)?;
        if request.request_hash != blob_finalize_request_hash(&hash) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let record = self.blob_record(&hash)?.ok_or(SyncError::BlobNotFound)?;
        if record.complete {
            return Ok(record);
        }
        let status = self.blob_status_inner(&hash)?;
        if !status.missing_chunks.is_empty() {
            return Err(SyncError::BlobNotFinalized);
        }
        let root = self.blob_root()?;
        let final_path = blob_path(root, &hash);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = final_path.with_extension("tmp");
        let _ = fs::remove_file(&temp_path);
        let mut output = File::create(&temp_path)?;
        let mut hasher = Sha256::new();
        let mut total = 0i64;
        for index in 0..record.chunk_count {
            let path = chunk_path(root, &hash, index as u32);
            let data = fs::read(&path)?;
            if data.len() as i64 != chunk_len(record.size, record.chunk_size, index) {
                drop(output);
                let _ = fs::remove_file(&temp_path);
                return Err(SyncError::BlobNotFinalized);
            }
            let stored: Option<String> = self
                .connection
                .query_row(
                    "SELECT chunk_hash FROM sync_blob_chunks WHERE blob_hash=?1 AND chunk_index=?2",
                    params![hash, index],
                    |r| r.get(0),
                )
                .optional()?;
            if stored.as_deref() != Some(sha256_hex(&data).as_str()) {
                drop(output);
                let _ = fs::remove_file(&temp_path);
                return Err(SyncError::BlobChunkMismatch);
            }
            hasher.update(&data);
            output.write_all(&data)?;
            total += data.len() as i64;
        }
        output.sync_all()?;
        drop(output);
        let actual = hex_encode(&hasher.finalize());
        if total != record.size || actual != hash {
            let _ = fs::remove_file(&temp_path);
            return Err(SyncError::BlobHashMismatch);
        }
        if final_path.exists() {
            if !file_matches(&final_path, record.size, &hash)? {
                let _ = fs::remove_file(&temp_path);
                return Err(SyncError::BlobConflict);
            }
            let _ = fs::remove_file(&temp_path);
        } else {
            fs::rename(&temp_path, &final_path)?;
        }
        let finalized_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        self.connection.execute(
            "UPDATE sync_blobs SET status='complete',finalized_at=?1 WHERE blob_hash=?2",
            params![finalized_at, hash],
        )?;
        let staging = staging_dir(root, &hash);
        if staging.exists() {
            let _ = fs::remove_dir_all(staging);
        }
        self.blob_record(&hash)?.ok_or(SyncError::BlobNotFound)
    }

    pub fn read_blob_chunk(
        &mut self,
        request: &SignedRequest,
        blob_hash: &str,
        index: u32,
        now_unix: i64,
    ) -> Result<Vec<u8>, SyncError> {
        if request.access != SyncAccess::Read {
            return Err(SyncError::InvalidAccess);
        }
        let hash = normalize_blob_hash(blob_hash)?;
        if request.request_hash != blob_read_request_hash(&hash, index) {
            return Err(SyncError::InvalidRequestHash);
        }
        let device = self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&device.device_id, now_unix)?;
        let record = self.blob_record(&hash)?.ok_or(SyncError::BlobNotFound)?;
        if !record.complete {
            return Err(SyncError::BlobNotFinalized);
        }
        if i64::from(index) >= record.chunk_count {
            return Err(SyncError::InvalidBlob("chunk index out of range".into()));
        }
        let root = self.blob_root()?;
        let mut file = File::open(blob_path(root, &hash))?;
        file.seek(SeekFrom::Start(
            (i64::from(index) * record.chunk_size) as u64,
        ))?;
        let len = chunk_len(record.size, record.chunk_size, i64::from(index));
        let mut data = vec![0u8; len as usize];
        file.read_exact(&mut data)?;
        let stored: Option<String> = self
            .connection
            .query_row(
                "SELECT chunk_hash FROM sync_blob_chunks WHERE blob_hash=?1 AND chunk_index=?2",
                params![hash, index],
                |r| r.get(0),
            )
            .optional()?;
        if stored.as_deref() != Some(sha256_hex(&data).as_str()) {
            return Err(SyncError::BlobChunkMismatch);
        }
        Ok(data)
    }

    fn blob_root(&self) -> Result<&Path, SyncError> {
        self.blob_root
            .as_deref()
            .ok_or(SyncError::BlobStorageUnavailable)
    }

    fn blob_record(&self, hash: &str) -> Result<Option<BlobRecord>, SyncError> {
        self.connection
            .query_row(
                "SELECT blob_hash,size,chunk_size,chunk_count,status,created_at,created_by,finalized_at FROM sync_blobs WHERE blob_hash=?1",
                [hash],
                |row| {
                    let status: String = row.get(4)?;
                    Ok(BlobRecord {
                        blob_hash: row.get(0)?,
                        size: row.get(1)?,
                        chunk_size: row.get(2)?,
                        chunk_count: row.get(3)?,
                        complete: status == "complete",
                        created_at: row.get(5)?,
                        created_by: row.get(6)?,
                        finalized_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(SyncError::from)
    }

    fn blob_status_inner(&self, hash: &str) -> Result<BlobStatus, SyncError> {
        let record = self.blob_record(hash)?.ok_or(SyncError::BlobNotFound)?;
        let mut received = self
            .connection
            .prepare(
                "SELECT chunk_index FROM sync_blob_chunks WHERE blob_hash=?1 ORDER BY chunk_index",
            )?
            .query_map([hash], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        received.sort_unstable();
        let received_chunks = received
            .iter()
            .map(|index| *index as u32)
            .collect::<Vec<_>>();
        let missing_chunks = if record.complete {
            Vec::new()
        } else {
            (0..record.chunk_count)
                .filter(|index| received.binary_search(index).is_err())
                .map(|index| index as u32)
                .collect()
        };
        Ok(BlobStatus {
            blob_hash: record.blob_hash,
            size: record.size,
            chunk_size: record.chunk_size,
            chunk_count: record.chunk_count,
            complete: record.complete,
            received_chunks,
            missing_chunks,
        })
    }
}

fn file_matches(path: &Path, size: i64, expected: &str) -> Result<bool, SyncError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0i64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as i64;
        hasher.update(&buffer[..read]);
    }
    Ok(total == size && hex_encode(&hasher.finalize()) == expected)
}

fn normalize_blob_hash(value: &str) -> Result<String, SyncError> {
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SyncError::InvalidBlob(
            "blob hash must be 64 hex characters".into(),
        ));
    }
    Ok(normalized)
}

fn chunk_len(size: i64, chunk_size: i64, index: i64) -> i64 {
    let start = index * chunk_size;
    (size - start).min(chunk_size)
}

fn objects_dir(root: &Path) -> PathBuf {
    root.join("objects")
}

fn staging_dir(root: &Path, hash: &str) -> PathBuf {
    root.join("staging").join(hash)
}

fn blob_path(root: &Path, hash: &str) -> PathBuf {
    objects_dir(root)
        .join(&hash[..2])
        .join(format!("{hash}.blob"))
}

fn chunk_path(root: &Path, hash: &str, index: u32) -> PathBuf {
    staging_dir(root, hash).join(format!("{index:08}.part"))
}

pub fn blob_begin_request_hash(hash: &str, size: i64, chunk_size: i64) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-blob-begin-v1",
        hash.as_bytes(),
        &size.to_be_bytes(),
        &chunk_size.to_be_bytes(),
    ]))
}

pub fn blob_chunk_request_hash(hash: &str, index: u32, length: i64, digest: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-blob-chunk-v1",
        hash.as_bytes(),
        &index.to_be_bytes(),
        &length.to_be_bytes(),
        digest.as_bytes(),
    ]))
}

pub fn blob_status_request_hash(hash: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-blob-status-v1",
        hash.as_bytes(),
    ]))
}

pub fn blob_finalize_request_hash(hash: &str) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-blob-finalize-v1",
        hash.as_bytes(),
    ]))
}

pub fn blob_read_request_hash(hash: &str, index: u32) -> [u8; 32] {
    sha256_array(&canonical(&[
        b"spanseek-sync-blob-read-v1",
        hash.as_bytes(),
        &index.to_be_bytes(),
    ]))
}
