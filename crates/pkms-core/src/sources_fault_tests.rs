//! Exact capture boundaries, enabled only in the core unit-test executable.
use super::*;
use crate::{ImportSource, JobStatus, SourceJobLease};
use std::{
    cell::RefCell,
    io::Cursor,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

thread_local! {
    static CHECKPOINT: RefCell<Option<(String, PathBuf)>> = const { RefCell::new(None) };
}

pub(super) fn checkpoint(stage: &str, db: &Connection, path: &Path, size: u64, hash: Option<&str>) {
    CHECKPOINT.with_borrow(|configured| {
        let Some((expected, root)) = configured else {
            return;
        };
        if expected != stage {
            return;
        }
        assert!(
            db.is_autocommit(),
            "capture must precede the source transaction"
        );
        let evidence = serde_json::json!({
            "stage": stage,
            "path": path.strip_prefix(root.join("knowledge")).unwrap(),
            "bytes": size,
            "computed_hash": hash,
            "autocommit": db.is_autocommit(),
        });
        let mut record = File::create_new(root.join("checkpoint.json")).unwrap();
        record
            .write_all(&serde_json::to_vec_pretty(&evidence).unwrap())
            .unwrap();
        record.sync_all().unwrap();
        // The separate ready marker prevents the parent reading partial JSON.
        File::create_new(root.join("ready"))
            .unwrap()
            .sync_all()
            .unwrap();
        std::thread::sleep(Duration::from_secs(30));
        panic!("parent did not terminate the capture test process");
    });
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "subprocess fixture; only started by capture fault tests"]
fn capture_process_child() {
    let root = PathBuf::from(std::env::var_os("PKMS_SOURCE_CAPTURE_FAULT_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    let stage = std::env::var("PKMS_SOURCE_CAPTURE_FAULT_STAGE").unwrap();
    let lease: SourceJobLease =
        serde_json::from_slice(&std::fs::read(root.join("request.json")).unwrap()).unwrap();
    let mut library = Library::open_existing(root.join("knowledge")).unwrap();
    CHECKPOINT.set(Some((stage, root.clone())));
    library
        .capture_source_original(
            &lease.job.id,
            &lease.lease_token,
            &mut File::open(root.join("original.txt")).unwrap(),
            &mut |_| Ok(()),
        )
        .unwrap();
    panic!("capture did not reach the selected checkpoint");
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn input(name: &str) -> ImportSource {
    ImportSource {
        input_token: id(),
        original_name: name.into(),
        format: SourceFormat::Text,
        origin_url: None,
        collection_id: None,
        add_version_to: None,
        base_row_version: None,
    }
}

fn capture(library: &mut Library, lease: &SourceJobLease, bytes: &[u8]) -> SourceVersion {
    library
        .capture_source_original(
            &lease.job.id,
            &lease.lease_token,
            &mut Cursor::new(bytes),
            &mut |_| Ok(()),
        )
        .unwrap()
}

fn snapshot(db: &Connection) -> Vec<(String, String)> {
    [
        "objects",
        "sources",
        "source_versions",
        "blobs",
        "source_file_dedup",
        "source_title_fts",
        "source_jobs",
        "source_job_attempts",
        "source_job_events",
        "command_receipts",
        "workspace_meta",
        "source_parses",
        "source_blocks",
        "source_active_parses",
        "source_parse_staging",
        "source_fts",
    ]
    .into_iter()
    .map(|table| {
        let columns = db
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap()
            .column_count();
        let order = (1..=columns)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        let mut digest = Sha256::new();
        while let Some(row) = rows.next().unwrap() {
            let values = (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i).unwrap())
                .collect::<Vec<_>>();
            digest.update(format!("{values:?}\n"));
        }
        (table.into(), format!("{:x}", digest.finalize()))
    })
    .collect()
}

fn residues(workspace: &Path, db: &Connection) -> Vec<serde_json::Value> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(workspace).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("source-capture-") && name.ends_with(".partial") {
            let bytes = std::fs::read(entry.path()).unwrap();
            records.push(serde_json::json!({
                "path": name, "kind": "capture_partial", "bytes": bytes.len(),
                "sha256": format!("{:x}", Sha256::digest(&bytes)),
            }));
        }
    }
    for shard in std::fs::read_dir(workspace.join("blobs/sha256")).unwrap() {
        for entry in std::fs::read_dir(shard.unwrap().path()).unwrap() {
            let path = entry.unwrap().path();
            let hash = path.file_name().unwrap().to_str().unwrap();
            let registered: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blobs WHERE hash=?1)",
                    [hash],
                    |r| r.get(0),
                )
                .unwrap();
            let referenced: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM source_versions WHERE blob_hash=?1)",
                    [hash],
                    |r| r.get(0),
                )
                .unwrap();
            if !registered && !referenced {
                let bytes = std::fs::read(&path).unwrap();
                let observed = format!("{:x}", Sha256::digest(&bytes));
                assert_eq!(
                    observed, hash,
                    "orphan filename must identify its complete bytes"
                );
                records.push(serde_json::json!({
                    "path": path.strip_prefix(workspace).unwrap(), "kind": "unregistered_blob",
                    "bytes": bytes.len(), "sha256": observed,
                }));
            }
        }
    }
    records.sort_by_key(|record| record["path"].as_str().unwrap().to_owned());
    records
}

fn verify_crash(stage: &str) {
    let artifact_directory = std::env::var_os("PKMS_CAPTURE_FAULT_ARTIFACT_DIR").map(PathBuf::from);
    let mut root = match &artifact_directory {
        Some(directory) => {
            std::fs::create_dir_all(directory).unwrap();
            tempfile::Builder::new()
                .prefix(&format!("{stage}-"))
                .tempdir_in(directory)
                .unwrap()
        }
        None => tempfile::tempdir().unwrap(),
    };
    root.disable_cleanup(artifact_directory.is_some());
    let workspace = root.path().join("knowledge");
    let mut library = Library::open(&workspace).unwrap();
    let note = library
        .create_note("已有人工笔记", "保持原稿 中文😀")
        .unwrap();
    let old_bytes = b"existing immutable source";
    let old_job = library
        .accept_source_import(&id(), input("existing.txt"))
        .unwrap();
    let old_lease = library.claim_source_job(&old_job.job_id).unwrap().unwrap();
    let old_version = capture(&mut library, &old_lease, old_bytes);
    library.cancel_source_job(&old_job.job_id).unwrap();
    let old_path = library.blob_path(&old_version.blob_hash, false).unwrap();
    let old_modified = std::fs::metadata(&old_path).unwrap().modified().unwrap();
    let bytes = "captured original 中文😀 line\n"
        .repeat(12_000)
        .into_bytes();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    std::fs::write(root.path().join("original.txt"), &bytes).unwrap();
    let accepted = library
        .accept_source_import(&id(), input("original.txt"))
        .unwrap();
    let lease = library.claim_source_job(&accepted.job_id).unwrap().unwrap();
    assert_eq!(lease.job.attempt, 1);
    let before = snapshot(&library.connection);
    drop(library);
    std::fs::write(
        root.path().join("request.json"),
        serde_json::to_vec(&lease).unwrap(),
    )
    .unwrap();
    let output = File::create_new(root.path().join("child.log")).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "sources::fault_tests::capture_process_child",
            "--ignored",
            "--nocapture",
        ])
        .env("PKMS_SOURCE_CAPTURE_FAULT_ROOT", root.path())
        .env("PKMS_SOURCE_CAPTURE_FAULT_STAGE", stage)
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = ChildGuard(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.path().join("ready").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {stage}; inspect child.log"
        );
        assert!(Instant::now() < deadline, "checkpoint timed out: {stage}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let child_id = child.0.id();
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let checkpoint: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("checkpoint.json")).unwrap())
            .unwrap();
    let residue_path = workspace.join(checkpoint["path"].as_str().unwrap());
    let captured = std::fs::read(&residue_path).unwrap();
    if stage == "staging_write" {
        assert_eq!(captured, bytes[..64 * 1024]);
        assert_eq!(checkpoint["bytes"], 64 * 1024);
        assert!(checkpoint["computed_hash"].is_null());
    } else {
        assert_eq!(captured, bytes);
        assert_eq!(checkpoint["computed_hash"], hash);
    }
    let mut reopened = Library::open_existing(&workspace).unwrap();
    assert_eq!(
        snapshot(&reopened.connection),
        before,
        "capture crash changed database rows at {stage}"
    );
    assert_eq!(reopened.note(&note.id).unwrap(), note);
    let pending = reopened.source_job(&lease.job.id).unwrap();
    assert_eq!(pending.status, JobStatus::Running);
    assert!(pending.source_id.is_none() && pending.source_version_id.is_none());
    let orphaned = residues(&workspace, &reopened.connection);
    assert_eq!(orphaned.len(), 1);
    assert_eq!(
        orphaned[0]["kind"],
        if stage == "after_blob" {
            "unregistered_blob"
        } else {
            "capture_partial"
        }
    );
    let orphan_modified = std::fs::metadata(&residue_path)
        .unwrap()
        .modified()
        .unwrap();
    reopened.recover_source_jobs().unwrap();
    let resumed = reopened.claim_source_job(&lease.job.id).unwrap().unwrap();
    assert_eq!(resumed.job.attempt, 2);
    assert_ne!(resumed.lease_token, lease.lease_token);
    let version = capture(&mut reopened, &resumed, &bytes);
    let expected_version = serde_json::to_value(&version).unwrap();
    assert_eq!(
        serde_json::to_value(capture(&mut reopened, &resumed, &bytes)).unwrap(),
        expected_version
    );
    let duplicate = reopened
        .accept_source_import(&id(), input("same-bytes-renamed.txt"))
        .unwrap();
    let duplicate_lease = reopened
        .claim_source_job(&duplicate.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(capture(&mut reopened, &duplicate_lease, &bytes)).unwrap(),
        expected_version
    );
    for table in ["sources", "source_versions", "source_file_dedup", "blobs"] {
        assert_eq!(
            reopened
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2,
            "duplicate rows in {table}"
        );
    }
    let (saved, mut original) = reopened.source_original_file(&version.id).unwrap();
    let mut observed = Vec::new();
    original.read_to_end(&mut observed).unwrap();
    assert_eq!(saved.blob_hash, hash);
    assert_eq!(observed, bytes);
    assert_eq!(
        std::fs::read(root.path().join("original.txt")).unwrap(),
        bytes
    );
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert_eq!(
        std::fs::metadata(&old_path).unwrap().modified().unwrap(),
        old_modified
    );
    assert_eq!(reopened.note(&note.id).unwrap(), note);
    assert_eq!(std::fs::read(&residue_path).unwrap(), captured);
    assert_eq!(
        std::fs::metadata(&residue_path)
            .unwrap()
            .modified()
            .unwrap(),
        orphan_modified
    );
    let remaining = residues(&workspace, &reopened.connection);
    if stage == "after_blob" {
        assert!(
            remaining.is_empty(),
            "existing orphan blob must be registered without rewriting"
        );
    } else {
        assert_eq!(
            remaining, orphaned,
            "partial remains identifiable, never adopted as an original"
        );
    }
    let integrity: String = reopened
        .connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        reopened
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    let result = serde_json::json!({
        "stage": stage, "child_pid": child_id, "checkpoint": checkpoint,
        "database_tables_unchanged": before, "orphaned_after_kill": orphaned,
        "residue_after_retry": remaining, "same_job_attempt": resumed.job.attempt,
        "same_job_id": resumed.job.id, "new_source_id": version.source_id,
        "new_version_id": version.id, "original_sha256": hash,
        "sources": 2, "versions": 2, "blobs": 2, "baseline_sources": 1,
        "same_job_replay_and_new_job_same_bytes": "same source and version",
        "existing_note_and_original_unchanged": true,
        "external_input_unchanged": true, "integrity": integrity, "foreign_key_errors": 0,
    });
    std::fs::write(
        root.path().join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    drop(original);
    drop(reopened);
    eprintln!(
        "verified capture crash stage={stage} child_pid={child_id} attempt=2 sources=2 versions=2 blobs=2 integrity=ok"
    );
    if artifact_directory.is_some() {
        eprintln!("capture crash evidence: {}", root.keep().display());
    }
}

#[test]
fn killed_during_staging_keeps_partial_unregistered_and_retry_deduplicates() {
    verify_crash("staging_write");
}

#[test]
fn killed_after_hash_keeps_complete_partial_unregistered_and_retry_deduplicates() {
    verify_crash("after_hash");
}

#[test]
fn killed_after_blob_before_source_transaction_reuses_orphan_without_rewrite() {
    verify_crash("after_blob");
}
