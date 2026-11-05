//! Shared fixtures for pkms-app integration tests. Each test binary compiles
//! this module separately, so unused helpers are expected.
#![allow(dead_code)]

use std::path::PathBuf;

/// Locates the `pkms-worker` binary built next to the running test binary.
///
/// Tests must not hard-code `../../target/debug`: cargo is invoked with an
/// isolated `CARGO_TARGET_DIR` for reproducible test runs, and a stale or
/// dangling workspace `target` junction must never decide whether a test runs.
pub fn worker_executable() -> PathBuf {
    let suffix = if cfg!(windows) {
        "pkms-worker.exe"
    } else {
        "pkms-worker"
    };
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(dir).join("debug"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(debug) = exe.parent().and_then(|deps| deps.parent())
    {
        candidates.push(debug.to_path_buf());
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug"));
    let searched = format!("{candidates:?}");
    candidates
        .into_iter()
        .map(|dir| dir.join(suffix))
        .find(|path| path.exists())
        .unwrap_or_else(|| panic!("build pkms-worker first: no {suffix} in {searched}"))
        .canonicalize()
        .expect("canonicalize pkms-worker")
}
