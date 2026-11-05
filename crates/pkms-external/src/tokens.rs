use pkms_core::{ExternalErrorCode, ExternalScope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

const TOKEN_FORMAT_VERSION: u32 = 1;
const TOKEN_PREFIX: &str = "ssk_";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenRecord {
    pub id: String,
    pub label: String,
    pub scopes: Vec<ExternalScope>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    /// SHA-256 hex of the bearer secret. The plaintext is never written here.
    pub secret_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenSummary {
    pub id: String,
    pub label: String,
    pub scopes: Vec<ExternalScope>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl From<&TokenRecord> for TokenSummary {
    fn from(value: &TokenRecord) -> Self {
        Self {
            id: value.id.clone(),
            label: value.label.clone(),
            scopes: value.scopes.clone(),
            created_at: value.created_at.clone(),
            revoked_at: value.revoked_at.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedToken {
    pub token: String,
    pub summary: TokenSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenAuthError {
    Unauthorized,
    Forbidden { id: String, scope: ExternalScope },
    Unavailable,
}

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("token store I/O failure")]
    Io(#[from] std::io::Error),
    #[error("token store encoding failure")]
    Json(#[from] serde_json::Error),
    #[error("invalid token input: {0}")]
    Invalid(&'static str),
    #[error("token not found")]
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenFile {
    format_version: u32,
    tokens: Vec<TokenRecord>,
}

impl Default for TokenFile {
    fn default() -> Self {
        Self {
            format_version: TOKEN_FORMAT_VERSION,
            tokens: Vec::new(),
        }
    }
}

pub struct TokenStore {
    path: PathBuf,
    file: TokenFile,
}

impl TokenStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TokenError> {
        let path = path.as_ref().to_owned();
        let file = if path.exists() {
            let bytes = fs::read(&path)?;
            let file: TokenFile = serde_json::from_slice(&bytes)?;
            if file.format_version != TOKEN_FORMAT_VERSION {
                return Err(TokenError::Invalid("unsupported token store version"));
            }
            file
        } else {
            TokenFile::default()
        };
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create(
        &mut self,
        label: &str,
        scopes: Vec<ExternalScope>,
    ) -> Result<CreatedToken, TokenError> {
        if label.trim().is_empty() || label.chars().count() > 80 {
            return Err(TokenError::Invalid("label must contain 1-80 characters"));
        }
        if scopes.is_empty() {
            return Err(TokenError::Invalid("at least one scope is required"));
        }
        let mut unique = Vec::new();
        for scope in scopes {
            if unique.contains(&scope) {
                return Err(TokenError::Invalid("duplicate scope"));
            }
            unique.push(scope);
        }
        let token = new_secret();
        let record = TokenRecord {
            id: uuid::Uuid::new_v4().to_string(),
            label: label.trim().to_owned(),
            scopes: unique,
            created_at: chrono::Utc::now().to_rfc3339(),
            revoked_at: None,
            secret_hash: hash_secret(&token),
        };
        let summary = TokenSummary::from(&record);
        self.file.tokens.push(record);
        self.persist()?;
        Ok(CreatedToken { token, summary })
    }

    pub fn list(&self) -> Vec<TokenSummary> {
        self.file.tokens.iter().map(TokenSummary::from).collect()
    }

    pub fn revoke(&mut self, id: &str) -> Result<TokenSummary, TokenError> {
        let record = self
            .file
            .tokens
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(TokenError::NotFound)?;
        if record.revoked_at.is_none() {
            record.revoked_at = Some(chrono::Utc::now().to_rfc3339());
        }
        let summary = TokenSummary::from(&*record);
        self.persist()?;
        Ok(summary)
    }

    pub fn authenticate(
        &self,
        raw: &str,
        required: ExternalScope,
    ) -> Result<TokenSummary, TokenAuthError> {
        if !raw.starts_with(TOKEN_PREFIX) || raw.len() > 256 {
            return Err(TokenAuthError::Unauthorized);
        }
        let candidate = hash_secret(raw);
        let mut matched = None;
        for record in &self.file.tokens {
            if record.revoked_at.is_none() && constant_time_eq(&candidate, &record.secret_hash) {
                matched = Some(record);
            }
        }
        let Some(record) = matched else {
            return Err(TokenAuthError::Unauthorized);
        };
        if !record.scopes.contains(&required) {
            return Err(TokenAuthError::Forbidden {
                id: record.id.clone(),
                scope: required,
            });
        }
        Ok(TokenSummary::from(record))
    }

    /// Re-reads the token file for every request so a CLI revocation takes
    /// effect on an already-running HTTP server. Corrupt or missing token state
    /// is an availability failure, never an unauthenticated decision.
    pub fn authenticate_current(
        &self,
        raw: &str,
        required: ExternalScope,
    ) -> Result<TokenSummary, TokenAuthError> {
        let current = TokenStore::open(&self.path).map_err(|_| TokenAuthError::Unavailable)?;
        current.authenticate(raw, required)
    }

    fn persist(&self) -> Result<(), TokenError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self
            .path
            .with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let bytes = serde_json::to_vec_pretty(&self.file)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if let Err(error) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
    }
}

pub fn auth_status(error: &TokenAuthError) -> (ExternalErrorCode, Option<ExternalScope>) {
    match error {
        TokenAuthError::Unauthorized => (ExternalErrorCode::Unauthorized, None),
        TokenAuthError::Forbidden { scope, .. } => (ExternalErrorCode::Forbidden, Some(*scope)),
        TokenAuthError::Unavailable => (ExternalErrorCode::Unavailable, None),
    }
}

fn new_secret() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn hash_secret(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_is_shown_once_and_never_written_to_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api-tokens.json");
        let mut store = TokenStore::open(&path).unwrap();
        let created = store
            .create("agent", vec![ExternalScope::Read, ExternalScope::Search])
            .unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(&created.token));
        assert!(raw.contains(&created.summary.id));
        let authenticated = store
            .authenticate(&created.token, ExternalScope::Search)
            .unwrap();
        assert_eq!(authenticated.id, created.summary.id);
        assert_eq!(
            store.authenticate(&created.token, ExternalScope::Propose),
            Err(TokenAuthError::Forbidden {
                id: created.summary.id.clone(),
                scope: ExternalScope::Propose,
            })
        );
    }

    #[test]
    fn revoked_tokens_are_rejected_after_reload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api-tokens.json");
        let created = {
            let mut store = TokenStore::open(&path).unwrap();
            store.create("agent", vec![ExternalScope::Read]).unwrap()
        };
        {
            let mut store = TokenStore::open(&path).unwrap();
            let summary = store.revoke(&created.summary.id).unwrap();
            assert!(summary.revoked_at.is_some());
        }
        let store = TokenStore::open(&path).unwrap();
        assert_eq!(
            store.authenticate(&created.token, ExternalScope::Read),
            Err(TokenAuthError::Unauthorized)
        );
    }

    #[test]
    fn a_live_store_sees_revocation_and_unavailable_state_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api-tokens.json");
        let created = {
            let mut store = TokenStore::open(&path).unwrap();
            store.create("agent", vec![ExternalScope::Read]).unwrap()
        };
        let live = TokenStore::open(&path).unwrap();
        assert!(
            live.authenticate_current(&created.token, ExternalScope::Read)
                .is_ok()
        );
        {
            let mut writer = TokenStore::open(&path).unwrap();
            writer.revoke(&created.summary.id).unwrap();
        }
        assert_eq!(
            live.authenticate_current(&created.token, ExternalScope::Read),
            Err(TokenAuthError::Unauthorized)
        );
        fs::write(&path, b"not-json").unwrap();
        assert_eq!(
            live.authenticate_current(&created.token, ExternalScope::Read),
            Err(TokenAuthError::Unavailable)
        );
    }
}
