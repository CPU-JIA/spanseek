//! Contract tests for the flat replacement backend.
//!
//! Every case mirrors a contract point that already exists for the Lance path in
//! `crates/pkms-inference/tests/vector.rs` (named in each doc comment) and only
//! uses the public `FlatProjection` surface, so the suite can be pointed at
//! another store without rewriting assertions.

use std::{
    fs::File,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::*;

const DIMENSION: usize = 2;

fn row(id: &str, vector: Vec<f32>) -> FlatRow {
    FlatRow {
        chunk_id: id.into(),
        object_id: format!("object-{id}"),
        content_revision: "revision-7".into(),
        vector,
    }
}

fn diverse_rows(count: usize, dimension: usize) -> Vec<FlatRow> {
    let mut seed = 17_u64;
    (0..count)
        .map(|index| {
            let mut vector: Vec<f32> = (0..dimension)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    ((seed >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.) as f32
                })
                .collect();
            if vector.iter().all(|value| *value == 0.) {
                vector[0] = 1.;
            }
            FlatRow {
                chunk_id: format!("chunk-{index}"),
                object_id: format!("object-{index}"),
                content_revision: "revision-7".into(),
                vector,
            }
        })
        .collect()
}

/// The publication lock file the backend uses for this profile.
fn lock_path(projection: &FlatProjection, profile_id: &str) -> PathBuf {
    projection
        .directory
        .join(format!(".{}.lock", table_name(profile_id)))
}

fn hold_lock(projection: &FlatProjection, profile_id: &str) -> File {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(projection, profile_id))
        .unwrap();
    file.try_lock().unwrap();
    file
}

/// Publish from a plain OS thread: `#[tokio::test]` runs a current-thread
/// runtime, and the publication lock waits with a blocking sleep, so a second
/// publisher must not share that runtime.
fn spawn_publisher(
    path: PathBuf,
    ids: Vec<String>,
    dimension: usize,
    rows: Vec<FlatRow>,
    done: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let projection = FlatProjection::open(&path).await.unwrap();
            for id in &ids {
                projection.publish("p", id, dimension, &rows).await.unwrap();
            }
        });
        done.store(true, Ordering::SeqCst);
    })
}

/// Mirrors `real_lance_long_workspace_path`: a deep, non-ASCII local path works,
/// survives a reopen and still answers scoped queries.
#[tokio::test]
async fn flat_long_workspace_path_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("pkms-v1-schema10-20261003-app-vectors")
        .join("e088c356-6e06-471f-9485-aa0044f0f572")
        .join("9c1d5fcd-46b9-4803-8e6a-9516bc608f37")
        .join("nested-workspace-path-".repeat(6))
        .join("知识 向量");
    let projection = FlatProjection::open(&path).await.unwrap();
    projection
        .publish(
            "real-profile",
            "generation",
            DIMENSION,
            &[row("one", vec![1., 0.])],
        )
        .await
        .unwrap();
    projection
        .verify_generations("real-profile", &[("generation".into(), 1)])
        .await
        .unwrap();
    drop(projection);
    let reopened = FlatProjection::open(&path).await.unwrap();
    let candidates = reopened
        .search("real-profile", &["generation".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].chunk_id, "one");
    assert_eq!(candidates[0].profile_id, "real-profile");
    assert_eq!(candidates[0].generation_id, "generation");
    assert_eq!(candidates[0].content_revision, "revision-7");
    assert_eq!(candidates[0].object_id, "object-one");
}

/// Mirrors `real_lance_reopen_scope_and_cosine` (immutability) and
/// `concurrent_identical_retries_do_not_duplicate_rows`.
#[tokio::test]
async fn flat_publish_creates_once_and_exact_retries_verify_every_row() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    let rows = vec![
        row("same", vec![4., 0.]),
        row("orthogonal", vec![0., 3.]),
        row("opposite", vec![-2., 0.]),
    ];
    projection
        .publish("profile", "active", DIMENSION, &rows)
        .await
        .unwrap();
    // An exact retry in a different row order is still the same generation.
    let reordered = vec![rows[2].clone(), rows[0].clone(), rows[1].clone()];
    projection
        .publish("profile", "active", DIMENSION, &reordered)
        .await
        .unwrap();
    // Concurrent identical retries collapse into a single publication.
    let (first, second) = tokio::join!(
        projection.publish("profile", "active", DIMENSION, &rows),
        projection.publish("profile", "active", DIMENSION, &rows)
    );
    first.unwrap();
    second.unwrap();
    projection
        .verify_generations("profile", &[("active".into(), 3)])
        .await
        .unwrap();
    // Any other content for the same generation conflicts and never rewrites.
    let conflicts = vec![
        vec![FlatRow {
            content_revision: "revision-8".into(),
            ..rows[0].clone()
        }],
        vec![FlatRow {
            object_id: "object-elsewhere".into(),
            ..rows[0].clone()
        }],
        vec![row("same", vec![0., 5.])],
        vec![row("different", vec![1., 0.])],
        vec![rows[0].clone(), rows[1].clone()],
        vec![
            rows[0].clone(),
            rows[1].clone(),
            rows[2].clone(),
            row("extra", vec![1., 0.]),
        ],
    ];
    for conflict in conflicts {
        let error = projection
            .publish("profile", "active", DIMENSION, &conflict)
            .await
            .unwrap_err();
        assert!(matches!(error, FlatError::GenerationConflict), "{error}");
    }
    projection
        .verify_generations("profile", &[("active".into(), 3)])
        .await
        .unwrap();
    let candidates = projection
        .search("profile", &["active".into()], &[100., 0.], 3)
        .await
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["same", "orthogonal", "opposite"]
    );
    assert_eq!(candidates[0].object_id, "object-same");
}

/// Mirrors `invalid_vectors_never_publish_or_query`; the variants and messages
/// are the ones `vector.rs` produces for the same input.
#[tokio::test]
async fn flat_invalid_inputs_are_rejected_like_the_lance_path() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    let generation_cases = vec![
        (DIMENSION, Vec::new(), "empty generation or dimension"),
        (
            0,
            vec![row("bad", vec![1., 0.])],
            "empty generation or dimension",
        ),
        (
            MAX_DIMENSION + 1,
            vec![row("bad", vec![1., 0.])],
            "empty generation or dimension",
        ),
        (
            DIMENSION,
            vec![row("bad", vec![1., 0., 0.])],
            "revision, dimension or duplicate chunk",
        ),
        (
            DIMENSION,
            vec![row("dup", vec![1., 0.]), row("dup", vec![0., 1.])],
            "revision, dimension or duplicate chunk",
        ),
    ];
    for (dimension, rows, message) in generation_cases {
        let error = projection
            .publish("p", "g", dimension, &rows)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid vector input: {message}")
        );
    }
    let vector_cases = vec![
        // A row whose length differs from the manifest dimension is rejected by
        // the row check in `vector.rs` before `normalize` is reached, so it
        // keeps that message (verified against the Lance path).
        (vec![], "revision, dimension or duplicate chunk"),
        (vec![0., 0.], "zero or invalid vector norm"),
        (vec![f32::NAN, 1.], "dimension or non-finite vector"),
        (vec![f32::INFINITY, 1.], "dimension or non-finite vector"),
    ];
    for (vector, message) in vector_cases {
        let error = projection
            .publish("p", "g", DIMENSION, &[row("bad", vector)])
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid vector input: {message}")
        );
    }
    // Identities: empty, over-long and control characters, for the profile, the
    // generation and every row field.
    let control = "bad\u{7}id";
    for (profile, generation) in [("", "g"), ("p", ""), (control, "g"), ("p", control)] {
        assert!(
            projection
                .publish(profile, generation, DIMENSION, &[row("ok", vec![1., 0.])])
                .await
                .is_err()
        );
    }
    assert!(
        projection
            .publish(
                "p",
                &"x".repeat(1025),
                DIMENSION,
                &[row("ok", vec![1., 0.])]
            )
            .await
            .is_err()
    );
    let mut empty_chunk = row("ok", vec![1., 0.]);
    empty_chunk.chunk_id = String::new();
    let mut empty_object = row("ok", vec![1., 0.]);
    empty_object.object_id = String::new();
    let mut empty_revision = row("ok", vec![1., 0.]);
    empty_revision.content_revision = String::new();
    let mut control_chunk = row("ok", vec![1., 0.]);
    control_chunk.chunk_id = control.into();
    for invalid in [empty_chunk, empty_object, empty_revision, control_chunk] {
        let error = projection
            .publish("p", "g", DIMENSION, &[invalid])
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid vector input: identity");
    }
    // Queries are validated the same way and the caps are enforced.
    projection
        .publish("p", "g", DIMENSION, &[row("valid", vec![1., 0.])])
        .await
        .unwrap();
    for query in [vec![], vec![0., 0.], vec![f32::NAN, 1.], vec![1.]] {
        assert!(
            projection
                .search("p", &["g".into()], &query, 1)
                .await
                .is_err()
        );
    }
    for limit in [0, MAX_CANDIDATES + 1] {
        let error = projection
            .search("p", &["g".into()], &[1., 0.], limit)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid vector input: candidate or generation limit"
        );
    }
    assert_eq!(
        projection
            .search("p", &["g".into()], &[1., 0.], MAX_CANDIDATES)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        projection
            .search("", &["g".into()], &[1., 0.], 1)
            .await
            .is_err()
    );
    assert!(
        projection
            .search("p", &["".into()], &[1., 0.], 1)
            .await
            .is_err()
    );
}

/// Mirrors the `verify_generations` half of `real_lance_reopen_scope_and_cosine`.
#[tokio::test]
async fn flat_verify_generations_counts_rows_per_generation() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    projection
        .publish(
            "profile",
            "old",
            DIMENSION,
            &[row("excluded", vec![1., 0.])],
        )
        .await
        .unwrap();
    projection
        .publish(
            "profile",
            "active",
            DIMENSION,
            &[
                row("same", vec![4., 0.]),
                row("orthogonal", vec![0., 3.]),
                row("opposite", vec![-2., 0.]),
            ],
        )
        .await
        .unwrap();
    projection
        .verify_generations("profile", &[("active".into(), 3), ("old".into(), 1)])
        .await
        .unwrap();
    assert!(projection.verify_generations("profile", &[]).await.is_ok());
    for expected in [
        vec![("active".into(), 4u32), ("old".into(), 1)],
        vec![("active".into(), 2u32), ("old".into(), 1)],
        vec![("missing".into(), 1u32)],
        vec![("active".into(), 3u32), ("missing".into(), 1)],
    ] {
        let error = projection
            .verify_generations("profile", &expected)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "vector projection is inconsistent: projection_incomplete"
        );
    }
    let duplicate = vec![("active".into(), 3u32), ("active".into(), 3u32)];
    let error = projection
        .verify_generations("profile", &duplicate)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlatError::Invalid("duplicate generation manifest")),
        "{error}"
    );
    assert!(projection.verify_generations("", &[]).await.is_err());
    let too_many: Vec<(String, u32)> = (0..=MAX_GENERATIONS)
        .map(|index| (format!("g-{index}"), 1))
        .collect();
    let error = projection
        .verify_generations("profile", &too_many)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlatError::Invalid("generation limit")),
        "{error}"
    );
    let too_many_generations: Vec<String> = (0..=MAX_GENERATIONS)
        .map(|index| format!("g-{index}"))
        .collect();
    let error = projection
        .search("profile", &too_many_generations, &[1., 0.], 1)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlatError::Invalid("candidate or generation limit")),
        "{error}"
    );
}

/// Mirrors `generation_allowlist_treats_sql_punctuation_as_literal_identity`
/// and `quoted_generation_is_data_and_filter_is_not_bypassed`.
#[tokio::test]
async fn flat_generation_ids_stay_literal_identities() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    let selected = "generation') OR true -- 中文";
    projection
        .publish(
            "profile",
            selected,
            DIMENSION,
            &[row("selected", vec![0., 1.])],
        )
        .await
        .unwrap();
    projection
        .publish(
            "profile",
            "excluded",
            DIMENSION,
            &[row("excluded", vec![1., 0.])],
        )
        .await
        .unwrap();
    projection
        .verify_generations("profile", &[(selected.into(), 1)])
        .await
        .unwrap();
    let found = projection
        .search("profile", &[selected.into()], &[1., 0.], 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chunk_id, "selected");
    assert_eq!(found[0].generation_id, selected);
    assert!(found[0].cosine_similarity.abs() < 1e-5);
    assert!(
        projection
            .verify_generations("profile", &[(selected.into(), 2)])
            .await
            .is_err()
    );
    // A generation that differs only by case is a different identity.
    projection
        .publish(
            "profile",
            "Generation",
            DIMENSION,
            &[row("upper", vec![1., 0.])],
        )
        .await
        .unwrap();
    let upper = projection
        .search("profile", &["Generation".into()], &[1., 0.], 10)
        .await
        .unwrap();
    assert_eq!(
        upper
            .iter()
            .map(|candidate| candidate.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["upper"]
    );
}

/// Mirrors the search half of `real_lance_reopen_scope_and_cosine`: scope,
/// cosine sign, descending order, truncation and the empty allowlist.
#[tokio::test]
async fn flat_search_ranks_cosine_inside_the_allowlist() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    projection
        .publish(
            "profile",
            "old",
            DIMENSION,
            &[row("excluded", vec![1., 0.])],
        )
        .await
        .unwrap();
    projection
        .publish(
            "profile",
            "active",
            DIMENSION,
            &[
                row("same", vec![4., 0.]),
                row("orthogonal", vec![0., 3.]),
                row("opposite", vec![-2., 0.]),
            ],
        )
        .await
        .unwrap();
    projection
        .publish(
            "other-profile",
            "active",
            DIMENSION,
            &[row("other", vec![1., 0.])],
        )
        .await
        .unwrap();
    let candidates = projection
        .search("profile", &["active".into()], &[100., 0.], 3)
        .await
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["same", "orthogonal", "opposite"]
    );
    for (candidate, expected) in candidates.iter().zip([1.0_f64, 0.0, -1.0]) {
        assert!((candidate.cosine_similarity - expected).abs() < 1e-5);
        assert_eq!(candidate.profile_id, "profile");
        assert_eq!(candidate.generation_id, "active");
        assert_eq!(candidate.content_revision, "revision-7");
    }
    // Truncation keeps the best candidates, in order.
    let truncated = projection
        .search("profile", &["active".into()], &[100., 0.], 2)
        .await
        .unwrap();
    assert_eq!(
        truncated
            .iter()
            .map(|candidate| candidate.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["same", "orthogonal"]
    );
    // Only the allowlisted generations are visible, and the union is scoped too.
    assert!(
        projection
            .search("profile", &[], &[1., 0.], 3)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        projection
            .search("profile", &["missing".into()], &[1., 0.], 3)
            .await
            .unwrap()
            .is_empty()
    );
    let union = projection
        .search("profile", &["active".into(), "old".into()], &[1., 0.], 2)
        .await
        .unwrap();
    assert_eq!(union.len(), 2);
    assert!(
        union
            .iter()
            .all(|candidate| (candidate.cosine_similarity - 1.).abs() < 1e-5)
    );
    assert_eq!(union[0].chunk_id, "same");
    assert_eq!(union[1].chunk_id, "excluded");
    // The same generation id under another profile is never mixed in, and an
    // unknown profile is an error rather than an empty result.
    let other = projection
        .search("other-profile", &["active".into()], &[1., 0.], 3)
        .await
        .unwrap();
    assert_eq!(
        other
            .iter()
            .map(|candidate| candidate.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["other"]
    );
    assert!(
        projection
            .search("never-published", &["active".into()], &[1., 0.], 3)
            .await
            .is_err()
    );
}

/// The publication lock is the same OS file lock as the Lance path
/// (`.{table_name}.lock`): a second publisher waits for the holder instead of
/// writing concurrently.
#[tokio::test]
async fn flat_publication_lock_serializes_publishers() {
    let directory = tempfile::tempdir().unwrap();
    let projection = FlatProjection::open(directory.path()).await.unwrap();
    let holder = hold_lock(&projection, "p");
    let done = Arc::new(AtomicBool::new(false));
    let writer = spawn_publisher(
        directory.path().to_path_buf(),
        vec!["g".into()],
        DIMENSION,
        vec![row("locked", vec![1., 0.])],
        Arc::clone(&done),
    );
    thread::sleep(Duration::from_millis(250));
    assert!(
        !done.load(Ordering::SeqCst),
        "publish must wait for the profile lock"
    );
    drop(holder);
    writer.join().unwrap();
    assert!(done.load(Ordering::SeqCst));
    projection
        .verify_generations("p", &[("g".into(), 1)])
        .await
        .unwrap();
}

/// `Busy` is returned once the publication budget expires. The budget is
/// shortened here so the test does not idle for the production 30 s;
/// `flat_constants_match_the_lance_contract` pins the production value.
#[tokio::test]
async fn flat_publication_lock_reports_busy_after_the_budget() {
    let directory = tempfile::tempdir().unwrap();
    let mut projection = FlatProjection::open(directory.path()).await.unwrap();
    let holder = hold_lock(&projection, "p");
    projection.lock_timeout = Duration::from_millis(120);
    let started = Instant::now();
    let error = projection
        .publish("p", "g", DIMENSION, &[row("blocked", vec![1., 0.])])
        .await
        .unwrap_err();
    assert!(matches!(error, FlatError::Busy), "{error}");
    assert!(started.elapsed() >= Duration::from_millis(120));
    // Once the holder is gone the same publication succeeds.
    drop(holder);
    projection
        .publish("p", "g", DIMENSION, &[row("blocked", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .verify_generations("p", &[("g".into(), 1)])
        .await
        .unwrap();
}

/// The projection holds no cached rows: a path that is removed or that now
/// serves different content can never answer with a previous projection.
#[tokio::test]
async fn flat_projection_never_reuses_a_replaced_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vectors");
    let projection = FlatProjection::open(&path).await.unwrap();
    projection
        .publish("p", "g", DIMENSION, &[row("before", vec![1., 0.])])
        .await
        .unwrap();
    let before = projection
        .search("p", &["g".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(before[0].chunk_id, "before");
    // The same handle, a different store at the same path.
    std::fs::rename(&path, root.path().join("moved-away")).unwrap();
    FlatProjection::open(&path)
        .await
        .unwrap()
        .publish("p", "g", DIMENSION, &[row("after", vec![1., 0.])])
        .await
        .unwrap();
    let after = projection
        .search("p", &["g".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(after[0].chunk_id, "after");
    // A removed directory is an error, never the previous rows.
    std::fs::rename(&path, root.path().join("removed")).unwrap();
    assert!(
        projection
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .is_err()
    );
    assert!(
        projection
            .verify_generations("p", &[("g".into(), 1)])
            .await
            .is_err()
    );
    assert!(
        projection
            .publish("p", "g2", DIMENSION, &[row("after", vec![1., 0.])])
            .await
            .is_err()
    );
    // Direct guard check: if the opened path resolves somewhere else, the
    // projection refuses instead of serving the retargeted location.
    let mut retargeted = FlatProjection::open(&path).await.unwrap();
    retargeted.directory = root.path().to_path_buf();
    let error = retargeted
        .search("p", &["g".into()], &[1., 0.], 1)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlatError::Corrupt("projection_retargeted")),
        "{error}"
    );
}

/// Mirrors `cached_connection_switches_workspaces_and_reopens_after_invalid_path`
/// and `cached_connection_reads_recreated_table_at_the_same_path`.
#[tokio::test]
async fn flat_connection_cache_revalidates_every_access() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    for (path, chunk) in [(&first, "first"), (&second, "second")] {
        FlatProjection::open(path)
            .await
            .unwrap()
            .publish("p", "g", DIMENSION, &[row(chunk, vec![1., 0.])])
            .await
            .unwrap();
    }
    let mut cache = FlatConnectionCache::default();
    for (path, chunk) in [(&first, "first"), (&second, "second"), (&first, "first")] {
        let found = cache
            .get(path)
            .await
            .unwrap()
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .unwrap();
        assert_eq!(found[0].chunk_id, chunk);
    }
    // A path that is a file, not a directory, drops the cached projection.
    let invalid = root.path().join("file-instead-of-directory");
    std::fs::write(&invalid, "test").unwrap();
    assert!(cache.get(&invalid).await.is_err());
    let recovered = cache
        .get(&second)
        .await
        .unwrap()
        .search("p", &["g".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(recovered[0].chunk_id, "second");
    // A rebuilt store at the same path is served, not the old rows.
    std::fs::remove_dir_all(&second).unwrap();
    FlatProjection::open(&second)
        .await
        .unwrap()
        .publish("p", "g", DIMENSION, &[row("rebuilt", vec![1., 0.])])
        .await
        .unwrap();
    let rebuilt = cache
        .get(&second)
        .await
        .unwrap()
        .search("p", &["g".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(rebuilt[0].chunk_id, "rebuilt");
}

/// Mirrors `cached_connection_observes_external_generations_and_detects_deleted_rows`.
#[tokio::test]
async fn flat_connection_cache_observes_external_writes_and_deletions() {
    let root = tempfile::tempdir().unwrap();
    let writer = FlatProjection::open(root.path()).await.unwrap();
    writer
        .publish("p", "old", DIMENSION, &[row("old", vec![0., 1.])])
        .await
        .unwrap();
    let mut cache = FlatConnectionCache::default();
    let reader = cache.get(root.path()).await.unwrap();
    reader
        .verify_generations("p", &[("old".into(), 1)])
        .await
        .unwrap();
    assert_eq!(
        reader
            .search("p", &["old".into()], &[1., 0.], 2)
            .await
            .unwrap()
            .len(),
        1
    );
    writer
        .publish("p", "new", DIMENSION, &[row("new", vec![1., 0.])])
        .await
        .unwrap();
    let reader = cache.get(root.path()).await.unwrap();
    reader
        .verify_generations("p", &[("old".into(), 1), ("new".into(), 1)])
        .await
        .unwrap();
    let found = reader
        .search("p", &["new".into()], &[1., 0.], 2)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chunk_id, "new");
    assert!((found[0].cosine_similarity - 1.).abs() < 1e-5);
    // External deletion of a generation file must be visible, not hidden by the
    // cached projection.
    let file = writer
        .directory
        .join(table_name("p"))
        .join(generation_name("new"));
    std::fs::remove_file(&file).unwrap();
    assert!(
        cache
            .get(root.path())
            .await
            .unwrap()
            .verify_generations("p", &[("new".into(), 1)])
            .await
            .is_err()
    );
    cache
        .get(root.path())
        .await
        .unwrap()
        .verify_generations("p", &[("old".into(), 1)])
        .await
        .unwrap();
}

/// A reader must observe a generation either not at all or completely, even
/// while another publisher is writing generations.
#[tokio::test]
async fn flat_concurrent_readers_never_observe_a_partial_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vectors");
    let projection = FlatProjection::open(&path).await.unwrap();
    projection
        .publish(
            "p",
            "seed",
            8,
            &[row("seed", vec![1., 0., 0., 0., 0., 0., 0., 0.])],
        )
        .await
        .unwrap();
    let per_generation = 600usize;
    let ids: Vec<String> = (0..8).map(|index| format!("bulk-{index}")).collect();
    let rows = diverse_rows(per_generation, 8);
    let query = rows[7].vector.clone();
    let done = Arc::new(AtomicBool::new(false));
    let writer = spawn_publisher(path.clone(), ids.clone(), 8, rows, Arc::clone(&done));
    let mut partial = 0usize;
    while !writer.is_finished() {
        for id in &ids {
            let found = projection
                .search("p", std::slice::from_ref(id), &query, per_generation)
                .await
                .unwrap();
            assert!(
                found.iter().all(|candidate| candidate.generation_id == *id),
                "reader saw rows from another generation"
            );
            if !found.is_empty() && found.len() != per_generation {
                partial = found.len();
            }
            match projection
                .verify_generations("p", &[(id.clone(), per_generation as u32)])
                .await
            {
                Ok(()) => {}
                Err(error) => assert!(
                    matches!(error, FlatError::Corrupt("projection_incomplete")),
                    "{error}"
                ),
            }
        }
    }
    writer.join().unwrap();
    assert!(done.load(Ordering::SeqCst));
    assert_eq!(partial, 0, "a partially written generation was visible");
    for id in &ids {
        assert_eq!(
            projection
                .search("p", std::slice::from_ref(id), &query, per_generation)
                .await
                .unwrap()
                .len(),
            per_generation
        );
        projection
            .verify_generations("p", &[(id.clone(), per_generation as u32)])
            .await
            .unwrap();
    }
}

/// The stored representation must round-trip f32 exactly, otherwise an exact
/// retry after a reopen would be rejected as `GenerationConflict`.
#[tokio::test]
async fn flat_exact_retry_after_reopen_is_bit_exact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vectors");
    let mut vectors = vec![
        vec![0.1, 0.2, 0.3, 0.4],
        vec![1e-30, 3.4028235e38, -1e-30, 1.1754944e-38],
        vec![0.6, 0.3, -0.1, 0.7],
    ];
    vectors.extend(diverse_rows(64, 4).into_iter().map(|row| row.vector));
    let rows: Vec<FlatRow> = vectors
        .into_iter()
        .enumerate()
        .map(|(index, vector)| row(&format!("chunk-{index}"), vector))
        .collect();
    FlatProjection::open(&path)
        .await
        .unwrap()
        .publish("p", "g", 4, &rows)
        .await
        .unwrap();
    let reopened = FlatProjection::open(&path).await.unwrap();
    reopened.publish("p", "g", 4, &rows).await.unwrap();
    reopened
        .verify_generations("p", &[("g".into(), rows.len() as u32)])
        .await
        .unwrap();
    let found = reopened
        .search("p", &["g".into()], &rows[0].vector, rows.len())
        .await
        .unwrap();
    assert_eq!(found.len(), rows.len());
    assert!((found[0].cosine_similarity - 1.).abs() < 1e-6);
}

/// The flat backend must keep the Lance path's caps and lock budget.
#[test]
fn flat_constants_match_the_lance_contract() {
    assert_eq!(
        (MAX_DIMENSION, MAX_CANDIDATES, MAX_GENERATIONS),
        (8192, 1000, 100_000)
    );
    assert_eq!(PUBLICATION_LOCK_TIMEOUT, Duration::from_secs(30));
    assert_eq!(PUBLICATION_LOCK_RETRY, Duration::from_millis(10));
    #[cfg(feature = "vector")]
    {
        assert_eq!(MAX_DIMENSION, crate::vector::MAX_DIMENSION);
        assert_eq!(MAX_CANDIDATES, crate::vector::MAX_CANDIDATES);
        assert_eq!(MAX_GENERATIONS, crate::vector::MAX_GENERATIONS);
    }
}

/// With both features enabled the same corpus is published to both backends and
/// the contract points are compared directly: scope, order, scores, rejection
/// messages and generation verification.
#[cfg(feature = "vector")]
#[tokio::test]
async fn flat_matches_the_lance_path_on_the_same_corpus() {
    use crate::vector::{VectorProjection, VectorRow};

    let flat_root = tempfile::tempdir().unwrap();
    let lance_root = tempfile::tempdir().unwrap();
    let flat = FlatProjection::open(flat_root.path()).await.unwrap();
    let lance = VectorProjection::open(lance_root.path()).await.unwrap();
    let flat_rows = diverse_rows(64, 8);
    let lance_rows: Vec<VectorRow> = flat_rows
        .iter()
        .map(|row| VectorRow {
            chunk_id: row.chunk_id.clone(),
            object_id: row.object_id.clone(),
            content_revision: row.content_revision.clone(),
            vector: row.vector.clone(),
        })
        .collect();
    flat.publish("p", "g", 8, &flat_rows).await.unwrap();
    lance.publish("p", "g", 8, &lance_rows).await.unwrap();
    flat.publish("p", "other", 8, &flat_rows[..1])
        .await
        .unwrap();
    lance
        .publish("p", "other", 8, &lance_rows[..1])
        .await
        .unwrap();
    // Identical scope, order, scores and row fields for several queries.
    let queries = [
        flat_rows[3].vector.clone(),
        flat_rows[17].vector.clone(),
        flat_rows[0]
            .vector
            .iter()
            .map(|value| -value)
            .collect::<Vec<f32>>(),
    ];
    for query in queries {
        let expected = lance.search("p", &["g".into()], &query, 16).await.unwrap();
        let actual = flat.search("p", &["g".into()], &query, 16).await.unwrap();
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual.chunk_id, expected.chunk_id);
            assert_eq!(actual.object_id, expected.object_id);
            assert_eq!(actual.content_revision, expected.content_revision);
            assert_eq!(actual.generation_id, expected.generation_id);
            assert_eq!(actual.profile_id, expected.profile_id);
            assert!((actual.cosine_similarity - expected.cosine_similarity).abs() < 1e-5);
        }
    }
    // Both require the allowlist and both report a missing profile as an error.
    assert!(
        flat.search("p", &[], &flat_rows[0].vector, 4)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        lance
            .search("p", &[], &lance_rows[0].vector, 4)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        flat.search("p", &["other".into()], &flat_rows[1].vector, 4)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lance
            .search("p", &["other".into()], &lance_rows[1].vector, 4)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        flat.search("absent", &["g".into()], &flat_rows[0].vector, 4)
            .await
            .is_err()
    );
    assert!(
        lance
            .search("absent", &["g".into()], &lance_rows[0].vector, 4)
            .await
            .is_err()
    );
    // Same messages for the same invalid input.
    let zero = vec![0.; 8];
    let flat_zero = flat
        .publish(
            "p",
            "bad",
            8,
            &[FlatRow {
                chunk_id: "x".into(),
                object_id: "o".into(),
                content_revision: "1".into(),
                vector: zero.clone(),
            }],
        )
        .await
        .unwrap_err()
        .to_string();
    let lance_zero = lance
        .publish(
            "p",
            "bad",
            8,
            &[VectorRow {
                chunk_id: "x".into(),
                object_id: "o".into(),
                content_revision: "1".into(),
                vector: zero,
            }],
        )
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_zero, lance_zero);
    let flat_duplicate = flat
        .publish("p", "bad", 8, &[flat_rows[0].clone(), flat_rows[0].clone()])
        .await
        .unwrap_err()
        .to_string();
    let lance_duplicate = lance
        .publish(
            "p",
            "bad",
            8,
            &[lance_rows[0].clone(), lance_rows[0].clone()],
        )
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_duplicate, lance_duplicate);
    let flat_empty = flat
        .publish("p", "bad", 8, &[])
        .await
        .unwrap_err()
        .to_string();
    let lance_empty = lance
        .publish("p", "bad", 8, &[])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_empty, lance_empty);
    let flat_limit = flat
        .search("p", &["g".into()], &flat_rows[0].vector, 0)
        .await
        .unwrap_err()
        .to_string();
    let lance_limit = lance
        .search("p", &["g".into()], &lance_rows[0].vector, 0)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_limit, lance_limit);
    // Same verification outcome and message.
    flat.verify_generations("p", &[("g".into(), 64)])
        .await
        .unwrap();
    lance
        .verify_generations("p", &[("g".into(), 64)])
        .await
        .unwrap();
    let flat_incomplete = flat
        .verify_generations("p", &[("g".into(), 63)])
        .await
        .unwrap_err()
        .to_string();
    let lance_incomplete = lance
        .verify_generations("p", &[("g".into(), 63)])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_incomplete, lance_incomplete);
    assert_eq!(
        flat_incomplete,
        "vector projection is inconsistent: projection_incomplete"
    );
    // Same conflict for a changed immutable generation.
    let flat_conflict = flat
        .publish("p", "g", 8, &[flat_rows[0].clone()])
        .await
        .unwrap_err()
        .to_string();
    let lance_conflict = lance
        .publish("p", "g", 8, &[lance_rows[0].clone()])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(flat_conflict, lance_conflict);
    assert_eq!(
        flat_conflict,
        "generation_conflict: immutable generation contains different rows"
    );
    // Both refuse a second dimension inside one profile. The variants differ
    // because a flat backend cannot produce an Arrow schema error.
    let mixed = vec![1., 0., 0., 0.];
    assert!(
        flat.publish(
            "p",
            "mixed",
            4,
            &[FlatRow {
                chunk_id: "m".into(),
                object_id: "o".into(),
                content_revision: "1".into(),
                vector: mixed.clone(),
            }]
        )
        .await
        .is_err()
    );
    assert!(
        lance
            .publish(
                "p",
                "mixed",
                4,
                &[VectorRow {
                    chunk_id: "m".into(),
                    object_id: "o".into(),
                    content_revision: "1".into(),
                    vector: mixed,
                }]
            )
            .await
            .is_err()
    );
}
