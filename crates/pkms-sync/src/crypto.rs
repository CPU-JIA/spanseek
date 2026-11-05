use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{self, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use sha2::{Digest, Sha256};

use crate::{SignedRequest, SyncAccess, SyncError};

pub const SEED_LEN: usize = 32;
pub const PUBLIC_KEY_LEN: usize = 32;
pub const SIGNATURE_LEN: usize = 64;

/// Ed25519 device key pair. The seed never enters SQLite; callers keep it in
/// the platform credential vault (wired in a later task). No Debug/Clone.
pub struct DeviceKeyPair {
    seed: [u8; SEED_LEN],
    pair: Ed25519KeyPair,
}

impl DeviceKeyPair {
    pub fn generate() -> Result<Self, SyncError> {
        Self::from_seed(Self::generate_seed()?)
    }

    /// Generates a seed for callers that must hand it to a platform vault
    /// before constructing the key pair.
    pub fn generate_seed() -> Result<[u8; SEED_LEN], SyncError> {
        let mut seed = [0u8; SEED_LEN];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| SyncError::RandomUnavailable)?;
        Ok(seed)
    }

    /// Exposes the private seed as hex for immediate platform-vault storage.
    /// Never log, persist in SQLite, or put this value in diagnostics.
    pub fn seed_hex(&self) -> String {
        hex_encode(&self.seed)
    }

    pub fn from_seed(seed: [u8; SEED_LEN]) -> Result<Self, SyncError> {
        let pair =
            Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| SyncError::InvalidPublicKey)?;
        Ok(Self { seed, pair })
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.pair
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public key is 32 bytes")
    }

    pub fn public_key_hex(&self) -> String {
        hex_encode(&self.public_key())
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.pair.sign(message).as_ref().to_vec()
    }

    pub fn encryption_public_key(&self) -> Result<[u8; PUBLIC_KEY_LEN], SyncError> {
        let mut secret = crate::keys::derive_device_encryption_secret(&self.seed)?;
        let public = crate::keys::encryption_public_key(&secret);
        crate::keys::wipe_secret(&mut secret);
        Ok(public)
    }

    pub fn encryption_public_key_hex(&self) -> Result<String, SyncError> {
        Ok(hex_encode(&self.encryption_public_key()?))
    }

    pub fn unwrap_workspace_key(
        &self,
        envelope: &crate::keys::KeyEnvelope,
        aad: &[u8],
    ) -> Result<crate::keys::WorkspaceKey, SyncError> {
        let mut secret = crate::keys::derive_device_encryption_secret(&self.seed)?;
        let key = crate::keys::unwrap_workspace_key(secret, envelope, aad);
        crate::keys::wipe_secret(&mut secret);
        key
    }
    pub fn sign_request(
        &self,
        device_id: &str,
        access: SyncAccess,
        nonce: &str,
        issued_at: i64,
        request_hash: [u8; 32],
    ) -> SignedRequest {
        let message = access_message(device_id, access, nonce, issued_at, &request_hash);
        SignedRequest {
            device_id: device_id.to_owned(),
            access,
            nonce: nonce.to_owned(),
            issued_at,
            request_hash,
            signature: self.sign(&message),
        }
    }
}

impl Drop for DeviceKeyPair {
    fn drop(&mut self) {
        for byte in &mut self.seed {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

pub fn public_key_bytes(public_key_hex: &str) -> Result<[u8; PUBLIC_KEY_LEN], SyncError> {
    let bytes = hex_decode(public_key_hex).ok_or(SyncError::InvalidPublicKey)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| SyncError::InvalidPublicKey)
}

pub fn verify(public_key_hex: &str, message: &[u8], signature: &[u8]) -> Result<(), SyncError> {
    let public_key = public_key_bytes(public_key_hex)?;
    if signature.len() != SIGNATURE_LEN {
        return Err(SyncError::InvalidSignature);
    }
    UnparsedPublicKey::new(&signature::ED25519, &public_key)
        .verify(message, signature)
        .map_err(|_| SyncError::InvalidSignature)
}

pub fn fingerprint(public_key: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(public_key))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

pub fn sha256_array(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.as_chunks::<2>().0 {
        let high = nibble(pair[0])?;
        let low = nibble(pair[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Length-prefixed canonical encoding prevents field-concatenation ambiguity.
pub fn canonical(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

pub fn access_message(
    device_id: &str,
    access: SyncAccess,
    nonce: &str,
    issued_at: i64,
    request_hash: &[u8; 32],
) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-access-v1",
        device_id.as_bytes(),
        access.as_str().as_bytes(),
        nonce.as_bytes(),
        &issued_at.to_be_bytes(),
        request_hash,
    ])
}

pub fn pairing_proof_message(invite_id: &str, code: &str, public_key_hex: &str) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-pair-v1",
        invite_id.as_bytes(),
        normalize_pairing_code(code).as_bytes(),
        public_key_hex.as_bytes(),
    ])
}

pub fn owner_proof_message(public_key_hex: &str, display_name: &str) -> Vec<u8> {
    canonical(&[
        b"spanseek-sync-owner-v1",
        public_key_hex.as_bytes(),
        display_name.trim().as_bytes(),
    ])
}

pub fn normalize_pairing_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}
/// Serde helper for a 32-byte hash encoded as lowercase hex.
pub mod hex_array32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::hex_encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = super::hex_decode(&text)
            .ok_or_else(|| serde::de::Error::custom("request hash is not valid hex"))?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("request hash must be 32 bytes"))
    }
}

/// Serde helper for opaque bytes encoded as base64 on the wire.
pub mod base64_vec {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// Serde helper for optional opaque bytes encoded as base64 on the wire.
pub mod base64_opt_vec {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => {
                serializer.serialize_some(&base64::engine::general_purpose::STANDARD.encode(bytes))
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        text.map(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}
