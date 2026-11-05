//! Machine-local model credentials. Only opaque binding hashes reach the OS vault.
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error("invalid credential binding")]
    InvalidBinding,
    #[error("invalid API credential")]
    InvalidSecret,
    #[error("system credential storage unavailable")]
    Unavailable,
    #[error("system credential storage unsupported on this platform")]
    Unsupported,
}

/// Intentionally has no Debug, Serialize, or Clone implementation.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(value: String) -> Result<Self, CredentialError> {
        Self::from_bytes(value.into_bytes())
    }

    fn from_bytes(value: Vec<u8>) -> Result<Self, CredentialError> {
        let secret = Self(value);
        if secret.0.is_empty()
            || secret.0.len() > 2560
            || !secret.0.iter().all(|byte| byte.is_ascii_graphic())
        {
            return Err(CredentialError::InvalidSecret);
        }
        Ok(secret)
    }

    pub fn expose_secret(&self) -> &str {
        // Construction accepts ASCII only.
        std::str::from_utf8(&self.0).expect("validated credential encoding")
    }
}

fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        // Volatile stores prevent dead-store elimination of this owned buffer.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

#[derive(PartialEq, Eq)]
pub struct CredentialBinding {
    target: String,
}

impl CredentialBinding {
    pub fn new(
        local_instance: &str,
        workspace_id: &str,
        protocol: &str,
        endpoint: &str,
        model: &str,
    ) -> Result<Self, CredentialError> {
        let local =
            uuid::Uuid::parse_str(local_instance).map_err(|_| CredentialError::InvalidBinding)?;
        let workspace =
            uuid::Uuid::parse_str(workspace_id).map_err(|_| CredentialError::InvalidBinding)?;
        let mut url = reqwest::Url::parse(endpoint).map_err(|_| CredentialError::InvalidBinding)?;
        if local.is_nil()
            || workspace.is_nil()
            || !matches!(
                protocol,
                "openai_chat"
                    | "openai_responses"
                    | "anthropic"
                    | "gemini"
                    | "openai_embeddings"
                    | "gemini_embeddings"
                    | "openai_transcriptions"
            )
            || endpoint.trim() != endpoint
            || endpoint.chars().any(char::is_control)
            || !(url.scheme() == "https"
                || (url.scheme() == "http"
                    && url
                        .host_str()
                        .is_some_and(pkms_inference::remote_embeddings::loopback)))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || model.is_empty()
            || model.len() > 1024
            || model.trim() != model
            || model.chars().any(char::is_control)
        {
            return Err(CredentialError::InvalidBinding);
        }
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        let mut hash = Sha256::new();
        // Length-prefix every field, preventing ambiguous tuple concatenations.
        for field in [
            local.as_bytes().as_slice(),
            workspace.as_bytes().as_slice(),
            protocol.as_bytes(),
            url.as_str().as_bytes(),
            model.as_bytes(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        Ok(Self {
            target: format!("PKMS:model-api:v1:{:x}", hash.finalize()),
        })
    }

    /// ASR keys bind to the workspace endpoint and model under their own vault
    /// namespace, so transcription credentials never collide with chat or
    /// embedding credentials.
    pub fn new_asr(
        local_instance: &str,
        workspace_id: &str,
        endpoint: &str,
        model: &str,
    ) -> Result<Self, CredentialError> {
        let binding = Self::new(
            local_instance,
            workspace_id,
            "openai_transcriptions",
            endpoint,
            model,
        )?;
        Ok(Self {
            target: binding
                .target
                .replacen("PKMS:model-api:v1:", "PKMS:asr-api:v1:", 1),
        })
    }

    /// Channel credentials deliberately omit the model and use a separate vault namespace.
    /// Legacy profile targets above stay byte-for-byte stable; neither path falls back to the other.
    pub fn new_channel(
        local_instance: &str,
        workspace_id: &str,
        channel_id: &str,
        protocol: &str,
        endpoint: &str,
    ) -> Result<Self, CredentialError> {
        // Reuse the established endpoint/protocol validation without changing legacy hashing.
        Self::new(local_instance, workspace_id, protocol, endpoint, channel_id)?;
        let channel =
            uuid::Uuid::parse_str(channel_id).map_err(|_| CredentialError::InvalidBinding)?;
        if channel.is_nil() || protocol == "openai_embeddings" {
            return Err(CredentialError::InvalidBinding);
        }
        let local =
            uuid::Uuid::parse_str(local_instance).map_err(|_| CredentialError::InvalidBinding)?;
        let workspace =
            uuid::Uuid::parse_str(workspace_id).map_err(|_| CredentialError::InvalidBinding)?;
        let mut url = reqwest::Url::parse(endpoint).map_err(|_| CredentialError::InvalidBinding)?;
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        let mut hash = Sha256::new();
        for field in [
            local.as_bytes().as_slice(),
            workspace.as_bytes().as_slice(),
            channel.as_bytes().as_slice(),
            protocol.as_bytes(),
            url.as_str().as_bytes(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        Ok(Self {
            target: format!("PKMS:channel-api:v1:{:x}", hash.finalize()),
        })
    }
}

fn valid_target(target: &str) -> bool {
    (1..=512).contains(&target.len()) && target.bytes().all(|byte| byte.is_ascii_graphic())
}

pub fn store(binding: &CredentialBinding, secret: &Secret) -> Result<(), CredentialError> {
    store_at(&binding.target, secret)
}

pub fn load(binding: &CredentialBinding) -> Result<Option<Secret>, CredentialError> {
    load_at(&binding.target)
}

pub fn delete(binding: &CredentialBinding) -> Result<(), CredentialError> {
    delete_at(&binding.target)
}

/// Generic vault entry used by non-model secrets such as the sync device seed
/// and workspace key. Callers construct a namespaced target and never log it.
pub fn store_at(target: &str, secret: &Secret) -> Result<(), CredentialError> {
    if !valid_target(target) {
        return Err(CredentialError::InvalidBinding);
    }
    platform::store(target, secret)
}

pub fn load_at(target: &str) -> Result<Option<Secret>, CredentialError> {
    if !valid_target(target) {
        return Err(CredentialError::InvalidBinding);
    }
    platform::load(target)
}

pub fn delete_at(target: &str) -> Result<(), CredentialError> {
    if !valid_target(target) {
        return Err(CredentialError::InvalidBinding);
    }
    platform::delete(target)
}

pub fn exists(binding: &CredentialBinding) -> Result<bool, CredentialError> {
    Ok(load(binding)?.is_some())
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{Foundation::ERROR_NOT_FOUND, Security::Credentials::*};

    fn target(target: &str) -> Vec<u16> {
        target.encode_utf16().chain(Some(0)).collect()
    }

    pub fn store(target_name: &str, secret: &Secret) -> Result<(), CredentialError> {
        let mut name = target(target_name);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            CredentialBlobSize: secret.0.len() as u32,
            CredentialBlob: secret.0.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // CredWriteW copies the borrowed blob synchronously and does not mutate it.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(CredentialError::Unavailable);
        }
        Ok(())
    }

    struct CredentialAllocation(*mut CREDENTIALW);
    impl Drop for CredentialAllocation {
        fn drop(&mut self) {
            // CredReadW owns one allocation; wipe the blob before releasing it.
            unsafe {
                let credential = &mut *self.0;
                if !credential.CredentialBlob.is_null() && credential.CredentialBlobSize > 0 {
                    wipe(std::slice::from_raw_parts_mut(
                        credential.CredentialBlob,
                        credential.CredentialBlobSize as usize,
                    ));
                }
                CredFree(self.0.cast());
            }
        }
    }

    pub fn load(target_name: &str) -> Result<Option<Secret>, CredentialError> {
        let name = target(target_name);
        let mut ptr = std::ptr::null_mut();
        if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) } == 0 {
            return if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_NOT_FOUND as i32)
            {
                Ok(None)
            } else {
                Err(CredentialError::Unavailable)
            };
        }
        if ptr.is_null() {
            return Err(CredentialError::Unavailable);
        }
        let allocation = CredentialAllocation(ptr);
        // The allocation remains alive while its blob is copied into owned Secret memory.
        let credential = unsafe { &*allocation.0 };
        if credential.CredentialBlob.is_null()
            || credential.CredentialBlobSize == 0
            || credential.CredentialBlobSize > CRED_MAX_CREDENTIAL_BLOB_SIZE
        {
            return Err(CredentialError::InvalidSecret);
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        Secret::from_bytes(bytes.to_vec()).map(Some)
    }

    pub fn delete(target_name: &str) -> Result<(), CredentialError> {
        let name = target(target_name);
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
            && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NOT_FOUND as i32)
        {
            return Err(CredentialError::Unavailable);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn store(_: &str, _: &Secret) -> Result<(), CredentialError> {
        Err(CredentialError::Unsupported)
    }
    pub fn load(_: &str) -> Result<Option<Secret>, CredentialError> {
        Err(CredentialError::Unsupported)
    }
    pub fn delete(_: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unsupported)
    }
}

#[cfg(test)]
#[path = "inference_credentials_tests.rs"]
mod tests;
