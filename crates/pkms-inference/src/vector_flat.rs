//! Flat-file stand-in for the Lance projection, written to measure the
//! "replace the vector database" maintainability metric (V1.7 WP12 / SRS P1).
//!
//! `FlatProjection` implements the same four-method contract as
//! `crate::vector::VectorProjection` -- `open` / `publish` / `verify_generations`
//! / `search` -- plus the same per-worker connection cache, without LanceDB or
//! Arrow: one JSON file per immutable generation, published as temp file plus
//! atomic rename under the same OS publication lock, and an exact normalized
//! cosine scan over the allowlisted generations.
//!
//! It is deliberately not wired into `main.rs`: production keeps the Lance
//! projection until a swap is approved. Enable it with `--features vector-flat`.
//!
//! `vector-flat` does not enable the `vector` feature, so `VectorRow`,
//! `VectorError`, `identity`, `normalize`, `table_name` and the `MAX_*` caps in
//! `vector.rs` are not compiled here (`pub mod vector` is gated by
//! `#[cfg(feature = "vector")]`). The mirrors below are line-for-line copies of
//! those helpers so the contract stays identical; a real swap deletes
//! `vector.rs` and keeps a single copy. `FlatError` drops the `Lance`/`Arrow`
//! variants, which a non-Lance backend cannot produce.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use pkms_core::VectorCandidate;

/// Mirror of `vector::MAX_DIMENSION`.
pub const MAX_DIMENSION: usize = 8192;
/// Mirror of `vector::MAX_CANDIDATES`.
pub const MAX_CANDIDATES: usize = 1000;
/// Mirror of `vector::MAX_GENERATIONS`.
pub const MAX_GENERATIONS: usize = 100_000;

/// Same publication budget as `VectorProjection::publication_lock`.
const PUBLICATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
/// Same retry cadence as `VectorProjection::publication_lock`.
const PUBLICATION_LOCK_RETRY: Duration = Duration::from_millis(10);

/// Mirror of `vector::VectorError` without the two LanceDB/Arrow variants.
#[derive(Debug, thiserror::Error)]
pub enum FlatError {
    #[error("invalid vector input: {0}")]
    Invalid(&'static str),
    #[error("vector projection is inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("generation_conflict: immutable generation contains different rows")]
    GenerationConflict,
    #[error("projection_busy: another publisher holds the profile lock")]
    Busy,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Mirror of `vector::VectorRow`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlatRow {
    pub chunk_id: String,
    pub object_id: String,
    pub content_revision: String,
    pub vector: Vec<f32>,
}

/// One immutable generation. The envelope carries the scope identity so a file
/// copied from another profile or generation is corruption, never data.
#[derive(Debug, Serialize, Deserialize)]
struct GenerationFile {
    profile_id: String,
    generation_id: String,
    dimension: usize,
    rows: Vec<StoredRow>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRow {
    chunk_id: String,
    object_id: String,
    content_revision: String,
    /// Normalized vector, widened to f64 so the f32 -> JSON -> f32 round trip is
    /// exact and an identical retry compares bit for bit.
    vector: Vec<f64>,
}

/// Mirror of the `identity` helper in `vector.rs`.
fn identity(value: &str) -> Result<(), FlatError> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(FlatError::Invalid("identity"));
    }
    Ok(())
}

/// Mirror of the `table_name` helper in `vector.rs`; also the profile directory
/// name.
fn table_name(profile: &str) -> String {
    let mut hash = Sha256::new();
    hash.update((profile.len() as u64).to_le_bytes());
    hash.update(profile.as_bytes());
    format!("profile_{:x}", hash.finalize())
}

/// Generation file name: the same length-prefixed digest with a distinct
/// prefix, so arbitrary generation ids (SQL punctuation included) stay literal
/// identities and can never collide with a profile directory.
fn generation_name(generation: &str) -> String {
    let mut hash = Sha256::new();
    hash.update((generation.len() as u64).to_le_bytes());
    hash.update(generation.as_bytes());
    format!("generation_{:x}.json", hash.finalize())
}

/// Mirror of the `normalize` helper in `vector.rs`.
fn normalize(vector: &[f32]) -> Result<Vec<f32>, FlatError> {
    if vector.is_empty() || vector.len() > MAX_DIMENSION || vector.iter().any(|v| !v.is_finite()) {
        return Err(FlatError::Invalid("dimension or non-finite vector"));
    }
    let norm = vector
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return Err(FlatError::Invalid("zero or invalid vector norm"));
    }
    Ok(vector
        .iter()
        .map(|v| (f64::from(*v) / norm) as f32)
        .collect())
}

/// Same metric as the Lance path (`1 - cosine distance` over normalized
/// vectors), accumulated in f64 and clamped exactly like `vector.rs` does.
fn cosine_similarity(query: &[f32], stored: &[f64]) -> f64 {
    query
        .iter()
        .zip(stored)
        .map(|(query, stored)| f64::from(*query) * stored)
        .sum::<f64>()
        .clamp(-1.0, 1.0)
}

/// A projection backed by one JSON file per generation in a local directory.
pub struct FlatProjection {
    directory: PathBuf,
    /// 30 s in production. The contract tests shorten it to exercise `Busy`
    /// without idling for the whole budget.
    lock_timeout: Duration,
}

/// Mirrors `vector::VectorConnectionCache`: one projection per worker, the
/// directory is revalidated on every access so a removed or retargeted
/// workspace cannot reuse a previous projection, and a failed open drops the
/// cached handle.
#[derive(Default)]
pub struct FlatConnectionCache {
    current: Option<FlatProjection>,
}

impl FlatConnectionCache {
    pub async fn get(&mut self, path: &Path) -> Result<&FlatProjection, FlatError> {
        let opened = std::fs::create_dir_all(path).and_then(|()| path.canonicalize());
        let directory = match opened {
            Ok(directory) => directory,
            Err(error) => {
                self.current = None;
                return Err(error.into());
            }
        };
        if self
            .current
            .as_ref()
            .is_none_or(|p| p.directory != directory)
        {
            self.current = None;
            self.current = Some(FlatProjection::open(&directory).await?);
        }
        Ok(self
            .current
            .as_ref()
            .expect("projection opened successfully"))
    }
}

impl FlatProjection {
    /// Only local directories are accepted. The canonical path is recorded so a
    /// removed or retargeted workspace cannot keep serving a previous
    /// projection.
    pub async fn open(path: &Path) -> Result<Self, FlatError> {
        let absolute =
            std::path::absolute(path).map_err(|_| FlatError::Invalid("projection path"))?;
        if absolute.to_str().is_none() {
            return Err(FlatError::Invalid("non-UTF8 projection path"));
        }
        std::fs::create_dir_all(&absolute)?;
        Ok(Self {
            directory: std::fs::canonicalize(&absolute)?,
            lock_timeout: PUBLICATION_LOCK_TIMEOUT,
        })
    }

    /// Re-resolves the opened directory on every operation. The projection holds
    /// no cached rows, so a removed or retargeted path is never served.
    fn profile_directory(&self, profile_id: &str) -> Result<PathBuf, FlatError> {
        if std::fs::canonicalize(&self.directory)? != self.directory {
            return Err(FlatError::Corrupt("projection_retargeted"));
        }
        Ok(self.directory.join(table_name(profile_id)))
    }

    /// Dimension already recorded by any generation of the profile. The Lance
    /// path enforces one dimension per profile through the table schema; the
    /// flat backend enforces the same invariant explicitly.
    fn profile_dimension(directory: &Path) -> Result<Option<usize>, FlatError> {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            if let Ok(generation) = serde_json::from_slice::<GenerationFile>(&bytes) {
                return Ok(Some(generation.dimension));
            }
        }
        Ok(None)
    }

    /// Same shape as `VectorProjection::publication_lock`: one OS file lock per
    /// profile, a 30 s budget and `Busy` on expiry. `std::thread::sleep` stands
    /// in for the tokio timer because `vector-flat` deliberately has no tokio
    /// dependency; the wait is bounded by the deadline.
    async fn publication_lock(&self, profile_id: &str) -> Result<File, FlatError> {
        let path = self
            .directory
            .join(format!(".{}.lock", table_name(profile_id)));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(PUBLICATION_LOCK_RETRY)
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(FlatError::Busy),
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    /// Create once, never overwrite an existing generation. Exact retries verify
    /// every persisted row and vector. The generation is written to a temp file
    /// and renamed into place, so a reader or a killed publisher never observes
    /// half a generation.
    pub async fn publish(
        &self,
        profile_id: &str,
        generation_id: &str,
        dimension: usize,
        rows: &[FlatRow],
    ) -> Result<(), FlatError> {
        identity(profile_id)?;
        identity(generation_id)?;
        if dimension == 0 || dimension > MAX_DIMENSION || rows.is_empty() {
            return Err(FlatError::Invalid("empty generation or dimension"));
        }
        let mut chunks = HashSet::new();
        let mut stored = Vec::with_capacity(rows.len());
        for row in rows {
            identity(&row.chunk_id)?;
            identity(&row.object_id)?;
            identity(&row.content_revision)?;
            if row.vector.len() != dimension || !chunks.insert(row.chunk_id.as_str()) {
                return Err(FlatError::Invalid("revision, dimension or duplicate chunk"));
            }
            stored.push(StoredRow {
                chunk_id: row.chunk_id.clone(),
                object_id: row.object_id.clone(),
                content_revision: row.content_revision.clone(),
                vector: normalize(&row.vector)?.into_iter().map(f64::from).collect(),
            });
        }
        let directory = self.profile_directory(profile_id)?;
        let _lock = self.publication_lock(profile_id).await?;
        let file = directory.join(generation_name(generation_id));
        if let Some(existing) = read_generation(&file, profile_id, generation_id)? {
            if existing.dimension != dimension {
                return Err(FlatError::GenerationConflict);
            }
            let mut remaining: HashMap<&str, &StoredRow> = existing
                .rows
                .iter()
                .map(|row| (row.chunk_id.as_str(), row))
                .collect();
            if remaining.len() != existing.rows.len() {
                return Err(FlatError::GenerationConflict);
            }
            for row in &stored {
                let previous = remaining
                    .remove(row.chunk_id.as_str())
                    .ok_or(FlatError::GenerationConflict)?;
                if previous.object_id != row.object_id
                    || previous.content_revision != row.content_revision
                    || previous.vector != row.vector
                {
                    return Err(FlatError::GenerationConflict);
                }
            }
            if !remaining.is_empty() {
                return Err(FlatError::GenerationConflict);
            }
            return Ok(());
        }
        std::fs::create_dir_all(&directory)?;
        if Self::profile_dimension(&directory)?.is_some_and(|current| current != dimension) {
            return Err(FlatError::Invalid("revision, dimension or duplicate chunk"));
        }
        write_generation(
            &file,
            &GenerationFile {
                profile_id: profile_id.to_string(),
                generation_id: generation_id.to_string(),
                dimension,
                rows: stored,
            },
        )
    }

    /// Row count per `(generation_id, count)`. A missing generation, a short
    /// generation or an extra row is `Corrupt("projection_incomplete")`.
    pub async fn verify_generations(
        &self,
        profile_id: &str,
        expected: &[(String, u32)],
    ) -> Result<(), FlatError> {
        identity(profile_id)?;
        if expected.len() > MAX_GENERATIONS {
            return Err(FlatError::Invalid("generation limit"));
        }
        if expected.is_empty() {
            return Ok(());
        }
        let mut remaining = HashMap::new();
        for (id, count) in expected {
            identity(id)?;
            if remaining.insert(id.as_str(), *count).is_some() {
                return Err(FlatError::Invalid("duplicate generation manifest"));
            }
        }
        let directory = self.profile_directory(profile_id)?;
        for (generation, count) in remaining {
            let file = directory.join(generation_name(generation));
            let stored = read_generation(&file, profile_id, generation)?
                .ok_or(FlatError::Corrupt("projection_incomplete"))?;
            if stored.rows.len() != count as usize {
                return Err(FlatError::Corrupt("projection_incomplete"));
            }
        }
        Ok(())
    }

    /// Cosine candidates only from the explicit allowlist, highest similarity
    /// first, ties broken by generation id then chunk id.
    pub async fn search(
        &self,
        profile_id: &str,
        allowed_generations: &[String],
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorCandidate>, FlatError> {
        identity(profile_id)?;
        if limit == 0 || limit > MAX_CANDIDATES || allowed_generations.len() > MAX_GENERATIONS {
            return Err(FlatError::Invalid("candidate or generation limit"));
        }
        if allowed_generations.is_empty() {
            return Ok(Vec::new());
        }
        let query = normalize(query)?;
        let mut seen = HashSet::new();
        for generation in allowed_generations {
            identity(generation)?;
            seen.insert(generation.as_str());
        }
        let directory = self.profile_directory(profile_id)?;
        if !directory.is_dir() {
            // The Lance path reports a missing profile table as an error, never
            // as "no matches".
            return Err(FlatError::Corrupt("projection_incomplete"));
        }
        let mut candidates = Vec::new();
        for generation in seen {
            let file = directory.join(generation_name(generation));
            let Some(stored) = read_generation(&file, profile_id, generation)? else {
                continue;
            };
            if stored.dimension != query.len() {
                return Err(FlatError::Corrupt("dimension"));
            }
            for row in &stored.rows {
                candidates.push(VectorCandidate {
                    chunk_id: row.chunk_id.clone(),
                    object_id: row.object_id.clone(),
                    content_revision: row.content_revision.clone(),
                    profile_id: profile_id.into(),
                    generation_id: generation.into(),
                    cosine_similarity: cosine_similarity(&query, &row.vector),
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.cosine_similarity
                .total_cmp(&a.cosine_similarity)
                .then(a.generation_id.cmp(&b.generation_id))
                .then(a.chunk_id.cmp(&b.chunk_id))
        });
        candidates.truncate(limit);
        Ok(candidates)
    }
}

/// Read one generation file, or `None` while it does not exist yet. A file that
/// does not carry the requested scope or shape is corruption, never data.
fn read_generation(
    file: &Path,
    profile_id: &str,
    generation_id: &str,
) -> Result<Option<GenerationFile>, FlatError> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let generation: GenerationFile =
        serde_json::from_slice(&bytes).map_err(|_| FlatError::Corrupt("generation_file"))?;
    let rows_are_valid = generation.dimension > 0
        && generation.dimension <= MAX_DIMENSION
        && !generation.rows.is_empty()
        && generation.rows.iter().all(|row| {
            row.vector.len() == generation.dimension
                && row.vector.iter().all(|value| value.is_finite())
        });
    if generation.profile_id != profile_id
        || generation.generation_id != generation_id
        || !rows_are_valid
    {
        return Err(FlatError::Corrupt("generation_file"));
    }
    Ok(Some(generation))
}

/// Temp file plus atomic rename inside the same directory, so a reader sees
/// either the previous generation file or the complete new one.
fn write_generation(file: &Path, generation: &GenerationFile) -> Result<(), FlatError> {
    let bytes =
        serde_json::to_vec(generation).map_err(|_| FlatError::Invalid("generation encoding"))?;
    let temporary = file.with_extension("json.tmp");
    let written = std::fs::File::create(&temporary)
        .and_then(|mut handle| {
            handle.write_all(&bytes)?;
            handle.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, file));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "vector_flat_tests.rs"]
mod contract_tests;
