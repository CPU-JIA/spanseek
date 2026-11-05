use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn invoke(workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pkms"))
        .arg("--workspace")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap()
}

fn success(workspace: &Path, args: &[&str]) -> Value {
    let output = invoke(workspace, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn separate_processes_persist_conflict_search_and_restore_backup() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("知识库");
    let help = Command::new(env!("CARGO_BIN_EXE_pkms"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(!invoke(&workspace, &["list"]).status.success());
    assert!(!workspace.exists());
    success(&workspace, &["init"]);
    let note = success(
        &workspace,
        &["create", "--title", "知识管理", "--body", "Rust 与中文"],
    );
    let id = note["id"].as_str().unwrap();
    assert_eq!(success(&workspace, &["get", id]), note);
    let saved = success(
        &workspace,
        &[
            "save",
            id,
            "--base-revision",
            "1",
            "--title",
            "知识管理",
            "--body",
            "可恢复的知识",
        ],
    );
    assert_eq!(saved["revision"], 2);
    let conflict = invoke(
        &workspace,
        &[
            "save",
            id,
            "--base-revision",
            "1",
            "--title",
            "冲突",
            "--body",
            "不应覆盖",
        ],
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("revision conflict"));
    assert_eq!(
        success(&workspace, &["search", "知识"])[0]["note"]["id"],
        id
    );
    let backup = dir.path().join("backup.sqlite3");
    success(&workspace, &["backup", backup.to_str().unwrap()]);
    assert!(
        !invoke(&workspace, &["backup", backup.to_str().unwrap()])
            .status
            .success()
    );
    let restored = dir.path().join("恢复库");
    std::fs::create_dir(&restored).unwrap();
    std::fs::copy(&backup, restored.join("library.sqlite3")).unwrap();
    assert_eq!(success(&restored, &["get", id]), saved);
    assert_eq!(
        success(&restored, &["history", id])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    success(&workspace, &["trash", id]);
    assert!(
        success(&workspace, &["search", "知识"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    success(&workspace, &["restore", id]);
    assert_eq!(
        success(&workspace, &["search", "知识"])[0]["note"]["id"],
        id
    );
}

#[test]
fn collection_cli_uses_generic_memberships_and_preserves_members_when_collection_is_trashed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("workspace");
    success(&path, &["init"]);
    let note = success(&path, &["create", "--title", "成员", "--body", "正文"]);
    let note_id = note["id"].as_str().unwrap();
    let collection = success(&path, &["collection-create", "主题"]);
    let collection_id = collection["id"].as_str().unwrap();
    success(&path, &["collection-add", collection_id, note_id]);
    success(&path, &["collection-add", collection_id, note_id]);
    assert_eq!(
        success(&path, &["collection-list", collection_id])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    success(&path, &["trash", collection_id]);
    assert_eq!(success(&path, &["get", note_id]), note);
    success(&path, &["restore", collection_id]);
    assert_eq!(
        success(&path, &["collection-list", collection_id])[0]["id"],
        note_id
    );
    success(&path, &["collection-remove", collection_id, note_id]);
    assert!(
        success(&path, &["collection-list", collection_id])
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(success(&path, &["get", note_id]), note);
}

#[test]
fn workspace_info_survives_processes_and_snapshot_restoration() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("工作区信息");
    assert!(!invoke(&workspace, &["info"]).status.success());
    assert!(!workspace.exists());
    success(&workspace, &["init"]);
    let info = success(&workspace, &["info"]);
    assert_eq!(info["database_schema"], pkms_core::CURRENT_SCHEMA_VERSION);
    assert_eq!(
        info["supported_document_schemas"],
        serde_json::json!([1, 2, 3])
    );
    assert_eq!(info["workspace_id"].as_str().unwrap().len(), 36);
    assert_eq!(
        info["migrations"].as_array().unwrap().len(),
        pkms_core::CURRENT_SCHEMA_VERSION as usize
    );
    for record in info["migrations"].as_array().unwrap() {
        assert_eq!(record["checksum"].as_str().unwrap().len(), 64);
    }
    success(
        &workspace,
        &["create", "--title", "迁移后的写入", "--body", "正文"],
    );
    assert_eq!(success(&workspace, &["info"]), info);
    let backup = dir.path().join("identity.sqlite3");
    success(&workspace, &["backup", backup.to_str().unwrap()]);
    let restored = dir.path().join("恢复副本");
    std::fs::create_dir(&restored).unwrap();
    std::fs::copy(&backup, restored.join("library.sqlite3")).unwrap();
    assert_eq!(success(&restored, &["info"]), info);
    // This is portable logical identity, not the future per-machine instance ID.
    let independent = dir.path().join("独立新库");
    success(&independent, &["init"]);
    assert_ne!(
        success(&independent, &["info"])["workspace_id"],
        info["workspace_id"]
    );
}

#[test]
fn cli_respects_a_running_desktop_workspace_owner() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    success(&workspace, &["init"]);
    let owner = pkms_core::WorkspaceLease::acquire(&workspace).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pkms"))
        .arg("--workspace")
        .arg(&workspace)
        .arg("list")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("busy"));
    drop(owner);
    assert_eq!(success(&workspace, &["list"]), serde_json::json!([]));
}
