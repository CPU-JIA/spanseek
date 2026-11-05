use crate::{Error, Result};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
};

/// One task supervisor per workspace. Low-level Library connections still use
/// optimistic concurrency within the owning process and in storage tests.
pub struct WorkspaceLease {
    _file: File,
}

impl WorkspaceLease {
    pub fn acquire(workspace: impl AsRef<Path>) -> Result<Self> {
        let directory = workspace.as_ref().canonicalize()?;
        if !directory.is_dir() {
            return Err(Error::InvalidInput("workspace must be a directory"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".pkms-owner.lock"))?;
        file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => Error::WorkspaceBusy,
            TryLockError::Error(error) => Error::Io(error),
        })?;
        Ok(Self { _file: file })
    }
}
