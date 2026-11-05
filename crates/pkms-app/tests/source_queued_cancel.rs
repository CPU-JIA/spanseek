//! Deterministic queued cancellation through App IPC, before a real supervisor starts.
use pkms_app::{
    AppService, DesktopCommand, DesktopData, DesktopRequest, GrantPurpose, OperationKind,
    SourceWorkerConfig, WorkspaceIdentity,
};
use pkms_core::{ImportSource, JobStatus, SourceFormat, SourceJob};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf};
mod common;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn dispatch(
    app: &mut AppService,
    identity: &WorkspaceIdentity,
    command: DesktopCommand,
) -> DesktopData {
    app.dispatch(DesktopRequest {
        api_version: 1,
        request_id: id(),
        identity: Some(identity.clone()),
        command,
    })
    .unwrap()
    .data
}

fn accept(app: &mut AppService, identity: &WorkspaceIdentity, path: &std::path::Path) -> String {
    let grant = app.grant_path(GrantPurpose::ImportSource, path).unwrap();
    let reply = dispatch(
        app,
        identity,
        DesktopCommand::SourcesImport {
            command_id: id(),
            request: ImportSource {
                input_token: grant.token,
                original_name: grant.display_name,
                format: SourceFormat::Text,
                origin_url: None,
                collection_id: None,
                add_version_to: None,
                base_row_version: None,
            },
            tag_names: Vec::new(),
        },
    );
    let DesktopData::JobAccepted(accepted) = reply else {
        panic!("source import must return the real accepted job");
    };
    accepted.job_id
}

fn job(app: &mut AppService, identity: &WorkspaceIdentity, job_id: &str) -> SourceJob {
    let DesktopData::SourceJob(job) = dispatch(
        app,
        identity,
        DesktopCommand::SourceJobGet {
            job_id: job_id.to_owned(),
        },
    ) else {
        panic!("expected the stored source job");
    };
    job
}

#[test]
fn queued_cancel_stays_attempt_zero_when_real_supervisor_processes_another_source() {
    let artifacts = std::env::var_os("PKMS_SOURCE_QUEUED_ARTIFACT_DIR").map(PathBuf::from);
    let mut root = match &artifacts {
        Some(path) => {
            std::fs::create_dir_all(path).unwrap();
            tempfile::Builder::new()
                .prefix("queued-cancel-")
                .tempdir_in(path)
                .unwrap()
        }
        None => tempfile::tempdir().unwrap(),
    };
    root.disable_cleanup(artifacts.is_some());
    let knowledge = root.path().join("knowledge");
    let mut app = AppService::open(root.path().join("app")).unwrap();
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, &knowledge)
        .unwrap();
    let accepted = app
        .accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)
        .unwrap();
    let identity = app
        .execute_operation(&accepted.operation_id)
        .unwrap()
        .result
        .unwrap()
        .identity;

    let canceled_path = root.path().join("cancel-before-worker.txt");
    let canceled_bytes = b"This original must never be captured or parsed.";
    std::fs::write(&canceled_path, canceled_bytes).unwrap();
    let canceled_hash = format!("{:x}", Sha256::digest(canceled_bytes));
    let canceled_id = accept(&mut app, &identity, &canceled_path);
    // App IPC acceptance does not start the desktop scheduler. No supervisor exists yet.
    let before = job(&mut app, &identity, &canceled_id);
    assert_eq!(before.status, JobStatus::Queued);
    assert_eq!(before.stage, "capture_pending");
    assert_eq!(before.attempt, 0);
    assert!(!before.cancel_requested);
    assert!(before.source_id.is_none() && before.source_version_id.is_none());

    let DesktopData::SourceJob(canceled) = dispatch(
        &mut app,
        &identity,
        DesktopCommand::SourceJobCancel {
            job_id: canceled_id.clone(),
        },
    ) else {
        panic!("cancel must return the actual stored job");
    };
    assert_eq!(canceled.status, JobStatus::Canceled);
    assert_eq!(canceled.stage, "canceled");
    assert_eq!(canceled.attempt, 0);
    assert!(canceled.cancel_requested);
    assert!(canceled.source_id.is_none() && canceled.source_version_id.is_none());
    let canceled_events = app
        .library(&identity)
        .unwrap()
        .source_job_events(&canceled_id, 0, 100)
        .unwrap();
    assert_eq!(canceled_events.len(), 2);
    assert_eq!(canceled_events[0].status, JobStatus::Queued);
    assert_eq!(canceled_events[1].status, JobStatus::Canceled);
    assert!(canceled_events.iter().all(|event| event.attempt == Some(0)));

    // A second, tiny source is a positive control: the scheduler and real child must run.
    let control_path = root.path().join("supervisor-positive-control.txt");
    let control_bytes = "The real worker parses this separate source. 中文正对照。";
    std::fs::write(&control_path, control_bytes).unwrap();
    let control_id = accept(&mut app, &identity, &control_path);
    let control_before = job(&mut app, &identity, &control_id);
    assert_eq!(control_before.status, JobStatus::Queued);
    assert_eq!(control_before.attempt, 0);
    let executable = std::env::var_os("PKMS_SOURCE_QUEUED_WORKER")
        .map(PathBuf::from)
        .unwrap_or_else(common::worker_executable)
        .canonicalize()
        .expect("build pkms-worker or provide PKMS_SOURCE_QUEUED_WORKER");
    let worker_hash = format!("{:x}", Sha256::digest(std::fs::read(&executable).unwrap()));
    app.source_jobs_worker()
        .unwrap()
        .unwrap()
        .run(SourceWorkerConfig {
            executable: executable.clone(),
            pdfium_library_path: None,
            ocr: None,
            shutdown: Default::default(),
        })
        .unwrap();

    let after = job(&mut app, &identity, &canceled_id);
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&canceled).unwrap()
    );
    let after_events = app
        .library(&identity)
        .unwrap()
        .source_job_events(&canceled_id, 0, 100)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&after_events).unwrap(),
        serde_json::to_value(&canceled_events).unwrap()
    );
    assert!(
        app.library(&identity)
            .unwrap()
            .pending_source_jobs()
            .unwrap()
            .is_empty()
    );
    let control_after = job(&mut app, &identity, &control_id);
    assert_eq!(
        control_after.status,
        JobStatus::Succeeded,
        "{control_after:?}"
    );
    assert_eq!(control_after.stage, "lexical_ready");
    assert_eq!(control_after.attempt, 1);
    let parsed = app
        .library(&identity)
        .unwrap()
        .source_text_page(
            control_after.source_version_id.as_ref().unwrap(),
            None,
            0,
            10,
        )
        .unwrap();
    assert_eq!(parsed.blocks.len(), 1);
    assert_eq!(parsed.blocks[0].text, control_bytes);

    let db = Connection::open_with_flags(
        knowledge.join("library.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let canceled_attempts: u32 = db
        .query_row(
            "SELECT count(*) FROM source_job_attempts WHERE job_id=?1",
            [&canceled_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canceled_attempts, 0);
    let canceled_blobs: u32 = db
        .query_row(
            "SELECT count(*) FROM blobs WHERE hash=?1",
            [&canceled_hash],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canceled_blobs, 0);
    let canceled_blob_path = knowledge
        .join("blobs/sha256")
        .join(&canceled_hash[..2])
        .join(&canceled_hash);
    assert!(!canceled_blob_path.exists());
    let sources: u32 = db
        .query_row("SELECT count(*) FROM sources", [], |row| row.get(0))
        .unwrap();
    let versions: u32 = db
        .query_row("SELECT count(*) FROM source_versions", [], |row| row.get(0))
        .unwrap();
    let parses: u32 = db
        .query_row("SELECT count(*) FROM source_parses", [], |row| row.get(0))
        .unwrap();
    assert_eq!((sources, versions, parses), (1, 1, 1));
    assert_eq!(std::fs::read(&canceled_path).unwrap(), canceled_bytes);
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let foreign_key_issues: u32 = db
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_issues, 0);

    if artifacts.is_some() {
        let record = serde_json::json!({
            "layer": "real App IPC + source supervisor + real worker positive control; no native UI",
            "identity": identity,
            "worker_executable": executable,
            "worker_sha256": worker_hash,
            "queued_before_cancel": before,
            "canceled_before_supervisor": canceled,
            "after_supervisor": after,
            "canceled_events": canceled_events,
            "after_supervisor_events": after_events,
            "control_before": control_before,
            "control_after": control_after,
            "canceled_attempt_rows": canceled_attempts,
            "canceled_blob_rows": canceled_blobs,
            "canceled_blob_exists": canceled_blob_path.exists(),
            "original_sha256": canceled_hash,
            "sources": sources, "source_versions": versions, "source_parses": parses,
            "integrity_check": integrity, "foreign_key_issues": foreign_key_issues,
        });
        let path = root.path().join("result.json");
        std::fs::File::create_new(&path)
            .unwrap()
            .write_all(&serde_json::to_vec_pretty(&record).unwrap())
            .unwrap();
        println!("queued cancellation evidence: {}", path.display());
    }
}
