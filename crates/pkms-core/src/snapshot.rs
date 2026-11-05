//! Consistent SQLite snapshots, published without replacing an existing file.
use crate::{Error, Result};
use rusqlite::Connection;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[cfg(test)]
thread_local! {
    static FORCE_INTEGRITY_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only seam: make the next snapshot fail its integrity gate.
#[cfg(test)]
pub(crate) fn force_integrity_failure() {
    FORCE_INTEGRITY_FAILURE.with(|flag| flag.set(true));
}

pub(crate) fn backup_to(connection: &Connection, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(Error::InvalidInput("backup destination already exists"));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix("pkms-backup-")
        .suffix(".partial")
        .tempfile_in(parent)?;
    {
        let mut output = Connection::open(temporary.path())?;
        {
            let backup = rusqlite::backup::Backup::new(connection, &mut output)?;
            let start = Instant::now();
            loop {
                if start.elapsed() > Duration::from_secs(30) {
                    return Err(Error::BackupTimeout);
                }
                match backup.step(128)? {
                    rusqlite::backup::StepResult::Done => break,
                    rusqlite::backup::StepResult::More => {}
                    _ => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
        #[cfg(test)]
        if FORCE_INTEGRITY_FAILURE.with(|flag| flag.replace(false)) {
            return Err(Error::IntegrityFailure);
        }
        let integrity: String = output.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::IntegrityFailure);
        }
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(destination)
        .map_err(|e| Error::Io(e.error))?;
    // Unix also allows syncing the publication directory. On Windows the snapshot
    // file is flushed before the atomic, non-replacing rename.
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
