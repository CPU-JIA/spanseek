#![cfg(feature = "vector")]

use pkms_inference::vector::{VectorProjection, VectorRow};

fn row(id: &str, vector: Vec<f32>) -> VectorRow {
    VectorRow {
        chunk_id: id.into(),
        object_id: format!("object-{id}"),
        content_revision: "revision-7".into(),
        vector,
    }
}

#[tokio::test]
async fn real_lance_long_workspace_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("pkms-v1-schema10-20261003-app-vectors")
        .join("e088c356-6e06-471f-9485-aa0044f0f572")
        .join("9c1d5fcd-46b9-4803-8e6a-9516bc608f37")
        .join("nested-workspace-path-".repeat(6));
    let projection = VectorProjection::open(&path).await.unwrap();
    projection
        .publish("real-profile", "generation", 2, &[row("one", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .verify_generations("real-profile", &[("generation".into(), 1)])
        .await
        .unwrap();
    drop(projection);
    let reopened = VectorProjection::open(&path).await.unwrap();
    let candidates = reopened
        .search("real-profile", &["generation".into()], &[1., 0.], 1)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].chunk_id, "one");
}

#[tokio::test]
async fn real_lance_reopen_scope_and_cosine() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("知识 向量");
    let projection = VectorProjection::open(&path).await.unwrap();
    projection
        .publish("profile", "old", 2, &[row("excluded", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .publish(
            "profile",
            "active",
            2,
            &[
                row("same", vec![4., 0.]),
                row("orthogonal", vec![0., 3.]),
                row("opposite", vec![-2., 0.]),
            ],
        )
        .await
        .unwrap();
    projection
        .publish("other-profile", "active", 2, &[row("other", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .publish(
            "profile",
            "active",
            2,
            &[
                row("opposite", vec![-2., 0.]),
                row("same", vec![4., 0.]),
                row("orthogonal", vec![0., 3.]),
            ],
        )
        .await
        .unwrap();
    assert!(
        projection
            .publish("profile", "active", 2, &[row("overwrite", vec![1., 0.])])
            .await
            .is_err()
    );
    drop(projection);
    let reopened = VectorProjection::open(&path).await.unwrap();
    reopened
        .verify_generations("profile", &[("active".into(), 3), ("old".into(), 1)])
        .await
        .unwrap();
    assert!(
        reopened
            .verify_generations("profile", &[("active".into(), 4)])
            .await
            .is_err()
    );
    assert!(
        reopened
            .verify_generations("profile", &[("active".into(), 2)])
            .await
            .is_err()
    );
    assert!(
        reopened
            .verify_generations("profile", &[("missing".into(), 1)])
            .await
            .is_err()
    );
    let candidates = reopened
        .search("profile", &["active".into()], &[100., 0.], 3)
        .await
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|v| v.chunk_id.as_str())
            .collect::<Vec<_>>(),
        vec!["same", "orthogonal", "opposite"]
    );
    for (candidate, expected) in candidates.iter().zip([1.0_f64, 0.0, -1.0]) {
        assert!((candidate.cosine_similarity - expected).abs() < 1e-5);
        assert_eq!(candidate.profile_id, "profile");
        assert_eq!(candidate.generation_id, "active");
        assert_eq!(candidate.content_revision, "revision-7");
    }
    assert!(
        reopened
            .search("profile", &[], &[1., 0.], 3)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        reopened
            .search("profile", &["missing".into()], &[1., 0.], 3)
            .await
            .unwrap()
            .is_empty()
    );
    let union = reopened
        .search("profile", &["active".into(), "old".into()], &[1., 0.], 2)
        .await
        .unwrap();
    assert_eq!(union.len(), 2);
    assert!(
        union
            .iter()
            .all(|v| (v.cosine_similarity - 1.).abs() < 1e-5)
    );
}

#[tokio::test]
async fn quoted_generation_is_data_and_filter_is_not_bypassed() {
    let directory = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(directory.path()).await.unwrap();
    let malicious = "x') OR 1=1 --";
    projection
        .publish("p", "private", 2, &[row("private", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .publish("p", malicious, 2, &[row("allowed", vec![0., 1.])])
        .await
        .unwrap();
    let results = projection
        .search("p", &[malicious.into()], &[1., 0.], 10)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, "allowed");
    assert!(results[0].cosine_similarity.abs() < 1e-5);
}

#[tokio::test]
async fn concurrent_identical_retries_do_not_duplicate_rows() {
    let directory = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(directory.path()).await.unwrap();
    projection
        .publish("p", "seed", 2, &[row("seed", vec![1., 0.])])
        .await
        .unwrap();
    let rows = [row("same", vec![1., 0.])];
    let (first, retry) = tokio::join!(
        projection.publish("p", "concurrent", 2, &rows),
        projection.publish("p", "concurrent", 2, &rows)
    );
    first.unwrap();
    retry.unwrap();
    let results = projection
        .search("p", &["concurrent".into()], &[1., 0.], 10)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
#[ignore = "explicit scale measurement: writes 1000 real generations"]
async fn thousand_generation_warm_search() {
    let directory = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(directory.path()).await.unwrap();
    let mut allowed = Vec::new();
    let publication_start = std::time::Instant::now();
    for i in 0..1000 {
        let generation = format!("g{i}");
        projection
            .publish(
                "p",
                &generation,
                8,
                &[row(&format!("chunk{i}"), vec![1.; 8])],
            )
            .await
            .unwrap();
        allowed.push(generation);
    }
    println!(
        "publication_fixture_elapsed_ms={}",
        publication_start.elapsed().as_millis()
    );
    let expected = allowed.iter().map(|id| (id.clone(), 1)).collect::<Vec<_>>();
    projection.verify_generations("p", &expected).await.unwrap();
    projection
        .search("p", &allowed, &[1.; 8], 20)
        .await
        .unwrap();
    let mut samples = Vec::new();
    for _ in 0..50 {
        let start = std::time::Instant::now();
        projection.verify_generations("p", &expected).await.unwrap();
        let results = projection
            .search("p", &allowed, &[1.; 8], 20)
            .await
            .unwrap();
        samples.push(start.elapsed());
        assert_eq!(results.len(), 20);
    }
    println!(
        "raw_warm_us={:?}",
        samples
            .iter()
            .map(|elapsed| elapsed.as_micros())
            .collect::<Vec<_>>()
    );
    samples.sort();
    println!(
        "1000 generation warm verification + search (50 runs, debug, dim8): p50={:?} p95={:?} max={:?}",
        samples[24], samples[47], samples[49]
    );
}

#[tokio::test]
async fn invalid_vectors_never_publish_or_query() {
    let directory = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(directory.path()).await.unwrap();
    for vector in [
        vec![],
        vec![0., 0.],
        vec![f32::NAN, 1.],
        vec![f32::INFINITY, 1.],
        vec![1.],
    ] {
        assert!(
            projection
                .publish("p", "g", 2, &[row("bad", vector)])
                .await
                .is_err()
        );
    }
    projection
        .publish("p", "g", 2, &[row("valid", vec![1., 0.])])
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
    assert!(
        projection
            .search("p", &["g".into()], &[1., 0.], 0)
            .await
            .is_err()
    );
    assert!(
        projection
            .search("p", &["g".into()], &[1., 0.], 1001)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn generation_allowlist_treats_sql_punctuation_as_literal_identity() {
    let directory = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(directory.path()).await.unwrap();
    let selected = "generation') OR true -- 中文";
    projection
        .publish("profile", selected, 2, &[row("selected", vec![0., 1.])])
        .await
        .unwrap();
    projection
        .publish("profile", "excluded", 2, &[row("excluded", vec![1., 0.])])
        .await
        .unwrap();
    projection
        .verify_generations("profile", &[(selected.into(), 1)])
        .await
        .unwrap();
    let result = projection
        .search("profile", &[selected.into()], &[1., 0.], 10)
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].chunk_id, "selected");
    assert_eq!(result[0].generation_id, selected);
    assert!(
        projection
            .verify_generations("profile", &[(selected.into(), 2)])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cached_connection_observes_external_generations_and_detects_deleted_rows() {
    use pkms_inference::vector::VectorConnectionCache;
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let writer = VectorProjection::open(root.path()).await.unwrap();
    writer
        .publish("p", "old", 2, &[row("old", vec![0., 1.])])
        .await
        .unwrap();
    let mut cache = VectorConnectionCache::default();
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
        .publish("p", "new", 2, &[row("new", vec![1., 0.])])
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
    // Controlled corruption in this test's own table must not be hidden by the cache.
    let mut hash = Sha256::new();
    hash.update(1_u64.to_le_bytes());
    hash.update(b"p");
    let table_name = format!("profile_{:x}", hash.finalize());
    let external = lancedb::connect(root.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    external
        .open_table(table_name)
        .execute()
        .await
        .unwrap()
        .delete("generation_id = 'new'")
        .await
        .unwrap();
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

#[tokio::test]
async fn cached_connection_switches_workspaces_and_reopens_after_invalid_path() {
    use pkms_inference::vector::VectorConnectionCache;
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    for (path, chunk) in [(&first, "first"), (&second, "second")] {
        VectorProjection::open(path)
            .await
            .unwrap()
            .publish("p", "g", 2, &[row(chunk, vec![1., 0.])])
            .await
            .unwrap();
    }
    let mut cache = VectorConnectionCache::default();
    for (path, chunk) in [(&first, "first"), (&second, "second"), (&first, "first")] {
        let reader = cache.get(path).await.unwrap();
        let result = reader
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .unwrap();
        assert_eq!(result[0].chunk_id, chunk);
    }
    let invalid = root.path().join("file-instead-of-directory");
    std::fs::write(&invalid, "test").unwrap();
    assert!(cache.get(&invalid).await.is_err());
    assert_eq!(
        cache
            .get(&second)
            .await
            .unwrap()
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .unwrap()[0]
            .chunk_id,
        "second"
    );
}

fn diverse_rows(count: usize) -> Vec<VectorRow> {
    let mut state = 7_u64;
    (0..count)
        .map(|i| {
            let vector = (0..16)
                .map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    ((state >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.) as f32
                })
                .collect();
            row(&format!("sample-{i}"), vector)
        })
        .collect()
}

async fn profile_table(path: &std::path::Path) -> lancedb::Table {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(1_u64.to_le_bytes());
    hash.update(b"p");
    lancedb::connect(path.to_str().unwrap())
        .read_consistency_interval(std::time::Duration::ZERO)
        .execute()
        .await
        .unwrap()
        .open_table(format!("profile_{:x}", hash.finalize()))
        .execute()
        .await
        .unwrap()
}

fn assert_same_candidates(
    left: &[pkms_inference::vector::VectorCandidate],
    right: &[pkms_inference::vector::VectorCandidate],
) {
    assert_eq!(left.len(), right.len());
    for (a, b) in left.iter().zip(right) {
        assert_eq!(a.chunk_id, b.chunk_id);
        assert_eq!(a.generation_id, b.generation_id);
        assert!((a.cosine_similarity - b.cosine_similarity).abs() < 1e-5);
    }
}

#[tokio::test]
async fn automatic_exact_ivf_preserves_scope_recall_and_unindexed_appends() {
    use lancedb::index::IndexType;
    let root = tempfile::tempdir().unwrap();
    let writer = VectorProjection::open(root.path()).await.unwrap();
    let rows = diverse_rows(8192);
    writer.publish("p", "bulk", 16, &rows).await.unwrap();
    // Reach the existing publication maintenance checkpoint through real writes.
    for i in 1..99 {
        writer
            .publish(
                "p",
                &format!("excluded-{i}"),
                16,
                &[row(&format!("outside-{i}"), rows[0].vector.clone())],
            )
            .await
            .unwrap();
    }
    let table = profile_table(root.path()).await;
    assert!(table.list_indices().await.unwrap().is_empty());
    let mut before = Vec::new();
    for sample in [3, 213, 2001, 8191] {
        before.push(
            writer
                .search("p", &["bulk".into()], &rows[sample].vector, 240)
                .await
                .unwrap(),
        );
    }
    writer
        .publish(
            "p",
            "excluded-99",
            16,
            &[row("outside-99", rows[0].vector.clone())],
        )
        .await
        .unwrap();
    let indices = table.list_indices().await.unwrap();
    assert_eq!(indices.len(), 1);
    assert_eq!(indices[0].index_type, IndexType::IvfFlat);
    // Reopen uses the persisted index; all partitions, not default approximate probes.
    let reader = VectorProjection::open(root.path()).await.unwrap();
    for (i, sample) in [3, 213, 2001, 8191].into_iter().enumerate() {
        let after = reader
            .search("p", &["bulk".into()], &rows[sample].vector, 240)
            .await
            .unwrap();
        assert_same_candidates(&before[i], &after);
        assert!(after.iter().all(|c| c.generation_id == "bulk"));
    }
    let fresh = row("new-unindexed", rows[5].vector.clone());
    writer
        .publish("p", "new", 16, std::slice::from_ref(&fresh))
        .await
        .unwrap();
    assert!(
        table
            .index_stats(&indices[0].name)
            .await
            .unwrap()
            .unwrap()
            .num_unindexed_rows
            > 0
    );
    let found = reader
        .search("p", &["new".into()], &fresh.vector, 240)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chunk_id, fresh.chunk_id);
    assert!((found[0].cosine_similarity - 1.).abs() < 1e-5);
    // A second maintenance checkpoint incorporates deltas without losing rows.
    for i in 100..200 {
        writer
            .publish(
                "p",
                &format!("excluded-{i}"),
                16,
                &[row(&format!("outside-{i}"), rows[0].vector.clone())],
            )
            .await
            .unwrap();
    }
    assert!(
        table
            .index_stats(&indices[0].name)
            .await
            .unwrap()
            .unwrap()
            .num_indexed_rows
            > 8291
    );
    assert_same_candidates(
        &found,
        &reader
            .search("p", &["new".into()], &fresh.vector, 240)
            .await
            .unwrap(),
    );
    reader
        .verify_generations("p", &[("bulk".into(), 8192), ("new".into(), 1)])
        .await
        .unwrap();
    table.delete("generation_id = 'new'").await.unwrap();
    assert!(
        reader
            .verify_generations("p", &[("new".into(), 1)])
            .await
            .is_err()
    );
    assert!(
        reader
            .search("p", &["new".into()], &fresh.vector, 240)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn quantized_or_wrong_metric_indices_cannot_change_exact_cosine_results() {
    use lancedb::{
        DistanceType,
        index::{
            Index,
            vector::{IvfFlatIndexBuilder, IvfSqIndexBuilder},
        },
    };
    let root = tempfile::tempdir().unwrap();
    let projection = VectorProjection::open(root.path()).await.unwrap();
    let rows = diverse_rows(512);
    projection.publish("p", "g", 16, &rows).await.unwrap();
    let query = &rows[45].vector;
    let exact = projection
        .search("p", &["g".into()], query, 100)
        .await
        .unwrap();
    let table = profile_table(root.path()).await;
    for index in [
        Index::IvfSq(
            IvfSqIndexBuilder::default()
                .distance_type(DistanceType::Cosine)
                .num_partitions(4),
        ),
        Index::IvfFlat(
            IvfFlatIndexBuilder::default()
                .distance_type(DistanceType::L2)
                .num_partitions(4),
        ),
    ] {
        table
            .create_index(&["vector"], index)
            .replace(true)
            .execute()
            .await
            .unwrap();
        assert_same_candidates(
            &exact,
            &projection
                .search("p", &["g".into()], query, 100)
                .await
                .unwrap(),
        );
    }
}

#[tokio::test]
async fn cached_connection_reads_recreated_table_at_the_same_path() {
    use pkms_inference::vector::VectorConnectionCache;
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let write = |chunk: &str| row(chunk, vec![1., 0.]);
    VectorProjection::open(root.path())
        .await
        .unwrap()
        .publish("p", "g", 2, &[write("before-rebuild")])
        .await
        .unwrap();
    let mut cache = VectorConnectionCache::default();
    assert_eq!(
        cache
            .get(root.path())
            .await
            .unwrap()
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .unwrap()[0]
            .chunk_id,
        "before-rebuild"
    );
    let mut hash = Sha256::new();
    hash.update(1_u64.to_le_bytes());
    hash.update(b"p");
    let external = lancedb::connect(root.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    // This table belongs exclusively to the test's TempDir.
    external
        .drop_table(format!("profile_{:x}", hash.finalize()), &[])
        .await
        .unwrap();
    assert!(
        cache
            .get(root.path())
            .await
            .unwrap()
            .verify_generations("p", &[("g".into(), 1)])
            .await
            .is_err()
    );
    VectorProjection::open(root.path())
        .await
        .unwrap()
        .publish("p", "g", 2, &[write("after-rebuild")])
        .await
        .unwrap();
    let reader = cache.get(root.path()).await.unwrap();
    reader
        .verify_generations("p", &[("g".into(), 1)])
        .await
        .unwrap();
    assert_eq!(
        reader
            .search("p", &["g".into()], &[1., 0.], 1)
            .await
            .unwrap()[0]
            .chunk_id,
        "after-rebuild"
    );
}

#[tokio::test]
async fn failed_accelerator_creation_is_retried_after_io_recovers() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let writer = VectorProjection::open(root.path()).await.unwrap();
    let rows = diverse_rows(8192);
    writer.publish("p", "bulk", 16, &rows).await.unwrap();
    for i in 1..99 {
        writer
            .publish(
                "p",
                &format!("g-{i}"),
                16,
                &[row(&format!("c-{i}"), rows[0].vector.clone())],
            )
            .await
            .unwrap();
    }
    let table = profile_table(root.path()).await;
    assert_eq!(table.version().await.unwrap(), 99);
    let before = writer
        .search("p", &["bulk".into()], &rows[37].vector, 240)
        .await
        .unwrap();
    let mut hash = Sha256::new();
    hash.update(1_u64.to_le_bytes());
    hash.update(b"p");
    let index_path = root
        .path()
        .join(format!("profile_{:x}.lance", hash.finalize()))
        .join("_indices");
    // Real filesystem failure confined to our table: an ordinary file prevents
    // creating the derived index directory. This is not simulated disk-full.
    std::fs::File::create_new(&index_path)
        .unwrap()
        .sync_all()
        .unwrap();
    let trigger = row("trigger", rows[1].vector.clone());
    assert!(
        writer
            .publish("p", "trigger", 16, std::slice::from_ref(&trigger))
            .await
            .is_err()
    );
    assert!(table.list_indices().await.unwrap().is_empty());
    assert!(table.version().await.unwrap() >= 100);
    writer
        .verify_generations("p", &[("bulk".into(), 8192), ("trigger".into(), 1)])
        .await
        .unwrap();
    assert_same_candidates(
        &before,
        &writer
            .search("p", &["bulk".into()], &rows[37].vector, 240)
            .await
            .unwrap(),
    );
    std::fs::remove_file(&index_path).unwrap();
    drop(writer);
    let recovered = VectorProjection::open(root.path()).await.unwrap();
    recovered
        .publish("p", "trigger", 16, std::slice::from_ref(&trigger))
        .await
        .unwrap();
    assert_eq!(
        table.list_indices().await.unwrap().len(),
        1,
        "failed maintenance must be resumed even when compaction advanced the version"
    );
    assert_eq!(
        table.count_rows(None).await.unwrap(),
        8291,
        "idempotent retry must not duplicate rows"
    );
    assert_same_candidates(
        &before,
        &recovered
            .search("p", &["bulk".into()], &rows[37].vector, 240)
            .await
            .unwrap(),
    );
}
