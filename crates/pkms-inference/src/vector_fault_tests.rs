//! Test-only process interruption at real Lance maintenance boundaries.
use super::*;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

fn selected(stage: &str, directory: &Path) -> bool {
    std::env::var("PKMS_VECTOR_FAULT_STAGE").is_ok_and(|s| s == stage)
        && std::env::var_os("PKMS_VECTOR_FAULT_ROOT")
            .is_some_and(|p| PathBuf::from(p).join("vectors") == directory)
}

pub(super) async fn checkpoint(stage: &str, directory: &Path) {
    if !selected(stage, directory) {
        return;
    }
    write_checkpoint(directory);
    tokio::time::sleep(Duration::from_secs(30)).await;
    panic!("parent did not terminate the selected test process");
}

/// The parent watches for this file to know the moment it must terminate its
/// own child.
fn write_checkpoint(directory: &Path) {
    std::fs::File::create_new(directory.parent().unwrap().join("checkpoint"))
        .unwrap()
        .sync_all()
        .unwrap();
}

fn partial_index_bytes(directory: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let kind = e.file_type().unwrap();
            if kind.is_dir() {
                partial_index_bytes(&e.path())
            } else if kind.is_file() {
                e.metadata().unwrap().len()
            } else {
                0
            }
        })
        .sum()
}

/// Stop the process while index files are on disk and the build future is still
/// incomplete, so the parent kills a worker whose accelerator is half written
/// and not yet committed.
///
/// The sample has to run on its own thread: a single poll of the build future
/// can occupy the runtime for a hundred milliseconds, so an in-task timer
/// samples far too rarely to catch the window between the first index write and
/// the index commit. `state` keeps both conditions exact instead of approximate:
/// the checkpoint file is written only if the sample wins the race against the
/// completed build, so a finished or committed accelerator can never be reported
/// as an interrupted one.
pub(super) async fn during_build<F>(build: F, directory: &Path) -> Result<(), lancedb::Error>
where
    F: std::future::Future<Output = Result<(), lancedb::Error>>,
{
    if !selected("during_acceleration", directory) {
        return build.await;
    }
    let index_dir = directory
        .join(format!("{}.lance", table_name("p")))
        .join("_indices");
    // 0 = build in flight, 1 = build future completed, 2 = checkpoint written.
    let state = Arc::new(AtomicU8::new(0));
    let watcher = std::thread::spawn({
        let state = Arc::clone(&state);
        let directory = directory.to_path_buf();
        move || {
            while state.load(Ordering::SeqCst) == 0 {
                if partial_index_bytes(&index_dir) > 0
                    && state
                        .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    write_checkpoint(&directory);
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });
    let result = build.await;
    state.store(1, Ordering::SeqCst);
    let _ = watcher.join();
    result
}

fn rows() -> Vec<VectorRow> {
    let mut seed = 17_u64;
    (0..8192)
        .map(|i| VectorRow {
            chunk_id: format!("chunk-{i}"),
            object_id: "object".into(),
            content_revision: "1".into(),
            vector: (0..64)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    ((seed >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.) as f32
                })
                .collect(),
        })
        .collect()
}
fn trigger() -> VectorRow {
    VectorRow {
        chunk_id: "trigger".into(),
        object_id: "trigger".into(),
        content_revision: "1".into(),
        vector: vec![1.; 64],
    }
}

#[test]
fn maintenance_process_child() {
    let Some(root) = std::env::var_os("PKMS_VECTOR_FAULT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        VectorProjection::open(&root.join("vectors"))
            .await
            .unwrap()
            .publish("p", "trigger", 64, &[trigger()])
            .await
            .unwrap();
    });
    panic!("maintenance did not reach the requested checkpoint");
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn crash_and_retry(stage: &str) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("vectors");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let samples = rows();
    let before = runtime.block_on(async {
        let projection = VectorProjection::open(&directory).await.unwrap();
        projection.publish("p", "bulk", 64, &samples).await.unwrap();
        for i in 1..99 {
            let mut row = trigger();
            row.chunk_id = format!("extra-{i}");
            projection
                .publish("p", &format!("extra-{i}"), 64, &[row])
                .await
                .unwrap();
        }
        projection
            .search("p", &["bulk".into()], &samples[417].vector, 240)
            .await
            .unwrap()
    });
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "vector::fault_tests::maintenance_process_child",
            "--nocapture",
        ])
        .env("PKMS_VECTOR_FAULT_ROOT", root.path())
        .env("PKMS_VECTOR_FAULT_STAGE", stage)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = ChildGuard(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(25);
    while !root.path().join("checkpoint").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {stage}"
        );
        assert!(Instant::now() < deadline, "checkpoint timed out: {stage}");
        std::thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    runtime.block_on(async {
        let projection = VectorProjection::open(&directory).await.unwrap();
        let table = projection
            .connection
            .open_table(table_name("p"))
            .execute()
            .await
            .unwrap();
        let pending = directory
            .join(format!("{}.lance", table_name("p")))
            .join(".pkms-maintenance-pending");
        assert!(pending.exists());
        assert_eq!(table.count_rows(None).await.unwrap(), 8291);
        projection
            .verify_generations("p", &[("bulk".into(), 8192), ("trigger".into(), 1)])
            .await
            .unwrap();
        if stage != "after_acceleration" {
            assert!(table.list_indices().await.unwrap().is_empty());
        }
        if stage == "during_acceleration" {
            assert!(
                partial_index_bytes(
                    &directory
                        .join(format!("{}.lance", table_name("p")))
                        .join("_indices")
                ) > 0
            );
        }
        // The unchanged authoritative allowlist still excludes uncommitted rows.
        let found = projection
            .search("p", &["bulk".into()], &samples[417].vector, 240)
            .await
            .unwrap();
        assert!(found.iter().all(|c| c.generation_id == "bulk"));
        assert_eq!(
            before.iter().map(|c| &c.chunk_id).collect::<Vec<_>>(),
            found.iter().map(|c| &c.chunk_id).collect::<Vec<_>>()
        );
        projection
            .publish("p", "trigger", 64, &[trigger()])
            .await
            .unwrap();
        assert!(!pending.exists());
        assert_eq!(table.count_rows(None).await.unwrap(), 8291);
        assert_eq!(table.list_indices().await.unwrap().len(), 1);
        let after = projection
            .search("p", &["bulk".into()], &samples[417].vector, 240)
            .await
            .unwrap();
        for (a, b) in before.iter().zip(&after) {
            assert_eq!(a.chunk_id, b.chunk_id);
            assert!((a.cosine_similarity - b.cosine_similarity).abs() < 1e-5);
        }
        projection
            .publish("p", "trigger", 64, &[trigger()])
            .await
            .unwrap();
        assert_eq!(table.count_rows(None).await.unwrap(), 8291);
        eprintln!("maintenance crash recovered: {stage}; rows=8291; candidates=240; duplicate=0");
    });
}

#[test]
fn crash_after_compaction_resumes_acceleration() {
    crash_and_retry("after_compaction");
}
#[test]
fn crash_during_index_write_discards_partial_index_and_retries() {
    crash_and_retry("during_acceleration");
}
#[test]
fn crash_after_index_commit_replays_without_duplicate_rows() {
    crash_and_retry("after_acceleration");
}
