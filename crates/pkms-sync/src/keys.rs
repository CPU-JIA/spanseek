//! Workspace-key encryption, device key wrapping and rotation primitives.
//!
//! The plaintext workspace key never enters SQLite, logs or diagnostics; only
//! per-device X25519/AES-GCM envelopes are persisted by the authority.

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::SyncError;
use crate::crypto::{canonical, constant_time_eq, hex_decode, hex_encode};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// Plaintext workspace key. No Debug/Clone/Serialize; wiped on drop.
pub struct WorkspaceKey {
    bytes: [u8; KEY_LEN],
}

impl WorkspaceKey {
    pub fn generate() -> Result<Self, SyncError> {
        let mut bytes = [0u8; KEY_LEN];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| SyncError::CryptoUnavailable)?;
        Ok(Self { bytes })
    }

    /// Client-side constructor for tests/imported keys. Callers must never
    /// persist the result directly; wrap it per device first.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<SealedData, SyncError> {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| SyncError::CryptoUnavailable)?;
        let ciphertext = seal(&self.bytes, &nonce, plaintext, aad)?;
        Ok(SealedData { nonce, ciphertext })
    }

    pub fn decrypt(&self, sealed: &SealedData, aad: &[u8]) -> Result<Vec<u8>, SyncError> {
        open(&self.bytes, &sealed.nonce, &sealed.ciphertext, aad)
    }

    /// Exposes the key as hex for immediate platform-vault storage. Never log,
    /// persist in SQLite, or put this value in diagnostics.
    pub fn export_hex(&self) -> String {
        hex_encode(&self.bytes)
    }
    pub fn matches(&self, other: &Self) -> bool {
        constant_time_eq(&self.bytes, &other.bytes)
    }
}

impl Drop for WorkspaceKey {
    fn drop(&mut self) {
        wipe(&mut self.bytes);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedData {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl SealedData {
    /// Wire/op encoding: 12-byte nonce followed by ciphertext+tag.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(NONCE_LEN + self.ciphertext.len());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        if bytes.len() <= NONCE_LEN {
            return Err(SyncError::InvalidKeyEnvelope);
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[..NONCE_LEN]);
        Ok(Self {
            nonce,
            ciphertext: bytes[NONCE_LEN..].to_vec(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEnvelope {
    pub ephemeral_public_key: [u8; KEY_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

pub fn parse_encryption_public_key(value: &str) -> Result<[u8; KEY_LEN], SyncError> {
    let bytes = hex_decode(value).ok_or(SyncError::InvalidEncryptionKey)?;
    let key: [u8; KEY_LEN] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SyncError::InvalidEncryptionKey)?;
    if key == [0u8; KEY_LEN] {
        return Err(SyncError::InvalidEncryptionKey);
    }
    Ok(key)
}

pub fn encryption_public_key_hex(public_key: &[u8; KEY_LEN]) -> String {
    hex_encode(public_key)
}

pub fn derive_device_encryption_secret(seed: &[u8; KEY_LEN]) -> Result<[u8; KEY_LEN], SyncError> {
    hkdf_32(
        seed,
        b"spanseek-sync-device-key-salt-v1",
        b"spanseek-sync-device-x25519-v1",
    )
}

pub fn encryption_public_key(secret: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

pub fn wrap_workspace_key(
    recipient_public_key: [u8; KEY_LEN],
    workspace_key: &WorkspaceKey,
    aad: &[u8],
) -> Result<KeyEnvelope, SyncError> {
    if recipient_public_key == [0u8; KEY_LEN] {
        return Err(SyncError::InvalidEncryptionKey);
    }
    let mut ephemeral_seed = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut ephemeral_seed)
        .map_err(|_| SyncError::CryptoUnavailable)?;
    let ephemeral_secret = StaticSecret::from(ephemeral_seed);
    let ephemeral_public = PublicKey::from(&ephemeral_secret).to_bytes();
    let shared = ephemeral_secret.diffie_hellman(&PublicKey::from(recipient_public_key));
    if !shared.was_contributory() {
        wipe(&mut ephemeral_seed);
        return Err(SyncError::InvalidEncryptionKey);
    }
    let mut salt = Vec::with_capacity(64);
    salt.extend_from_slice(&ephemeral_public);
    salt.extend_from_slice(&recipient_public_key);
    let mut wrapping_key = hkdf_32(shared.as_bytes(), &salt, b"spanseek-sync-wsk-wrap-v1")?;
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| SyncError::CryptoUnavailable)?;
    let ciphertext = seal(&wrapping_key, &nonce, &workspace_key.bytes, aad)?;
    wipe(&mut wrapping_key);
    wipe(&mut ephemeral_seed);
    Ok(KeyEnvelope {
        ephemeral_public_key: ephemeral_public,
        nonce,
        ciphertext,
    })
}

pub fn unwrap_workspace_key(
    mut recipient_secret: [u8; KEY_LEN],
    envelope: &KeyEnvelope,
    aad: &[u8],
) -> Result<WorkspaceKey, SyncError> {
    if envelope.ephemeral_public_key == [0u8; KEY_LEN] {
        wipe(&mut recipient_secret);
        return Err(SyncError::InvalidKeyEnvelope);
    }
    let secret = StaticSecret::from(recipient_secret);
    let recipient_public = PublicKey::from(&secret).to_bytes();
    let shared = secret.diffie_hellman(&PublicKey::from(envelope.ephemeral_public_key));
    if !shared.was_contributory() {
        wipe(&mut recipient_secret);
        return Err(SyncError::InvalidKeyEnvelope);
    }
    let mut salt = Vec::with_capacity(64);
    salt.extend_from_slice(&envelope.ephemeral_public_key);
    salt.extend_from_slice(&recipient_public);
    let mut wrapping_key = hkdf_32(shared.as_bytes(), &salt, b"spanseek-sync-wsk-wrap-v1")?;
    let plaintext = open(&wrapping_key, &envelope.nonce, &envelope.ciphertext, aad)?;
    wipe(&mut wrapping_key);
    wipe(&mut recipient_secret);
    if plaintext.len() != KEY_LEN {
        return Err(SyncError::InvalidKeyEnvelope);
    }
    let mut bytes = [0u8; KEY_LEN];
    bytes.copy_from_slice(&plaintext);
    Ok(WorkspaceKey { bytes })
}

pub fn workspace_key_aad(workspace_id: &str, epoch: i64, device_id: &str) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-wsk-aad-v1",
        workspace_id.as_bytes(),
        &epoch.to_be_bytes(),
        device_id.as_bytes(),
    ])
}

pub fn object_aad(workspace_id: &str, object_id: &str, epoch: i64) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-object-aad-v1",
        workspace_id.as_bytes(),
        object_id.as_bytes(),
        &epoch.to_be_bytes(),
    ])
}

pub fn blob_aad(blob_hash: &str) -> Vec<u8> {
    canonical(&[b"spanseek-sync-blob-aad-v1", blob_hash.as_bytes()])
}

fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, SyncError> {
    let key = aead_key(key)?;
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(*nonce),
        Aad::from(aad),
        &mut in_out,
    )
    .map_err(|_| SyncError::CryptoUnavailable)?;
    Ok(in_out)
}

fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, SyncError> {
    let key = aead_key(key)?;
    let mut in_out = ciphertext.to_vec();
    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(*nonce),
            Aad::from(aad),
            &mut in_out,
        )
        .map_err(|_| SyncError::CryptoUnavailable)?;
    Ok(plaintext.to_vec())
}

fn aead_key(bytes: &[u8; KEY_LEN]) -> Result<LessSafeKey, SyncError> {
    let unbound = UnboundKey::new(&AES_256_GCM, bytes).map_err(|_| SyncError::CryptoUnavailable)?;
    Ok(LessSafeKey::new(unbound))
}

fn hkdf_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; KEY_LEN], SyncError> {
    struct Len(usize);
    impl KeyType for Len {
        fn len(&self) -> usize {
            self.0
        }
    }
    let salt = Salt::new(HKDF_SHA256, salt);
    let prk = salt.extract(ikm);
    let info = [info];
    let okm = prk
        .expand(&info, Len(KEY_LEN))
        .map_err(|_| SyncError::CryptoUnavailable)?;
    let mut out = [0u8; KEY_LEN];
    okm.fill(&mut out)
        .map_err(|_| SyncError::CryptoUnavailable)?;
    Ok(out)
}

pub(crate) fn wipe_secret(bytes: &mut [u8; KEY_LEN]) {
    wipe(bytes);
}

fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
