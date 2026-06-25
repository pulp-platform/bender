//! Cross-process advisory locking for a [`GitDatabase`](crate::database::GitDatabase).
//!
//! Multiple bender invocations (e.g. parallel CI jobs) may share a single
//! database directory. Git's object store already tolerates concurrent writers,
//! but two concerns remain:
//!
//! - **Initialization races** — two invocations both finding the database
//!   missing and both running `init` + `add_remote`, the latter racing on the
//!   `config` file.
//! - **Fetch contention** — concurrent `git fetch --prune` colliding on
//!   `packed-refs.lock` (spurious "cannot lock ref" failures) and doing
//!   redundant network work.
//!
//! Both are avoided by serializing the relevant operations behind an exclusive
//! advisory file lock ([`std::fs::File::lock`], which uses `flock(2)` on Unix
//! and `LockFileEx` on Windows). The lock is held for the duration of
//! init/add_remote/fetch and released on drop.
//!
//! Each acquisition opens a fresh file handle, so the lock serializes both
//! across processes *and* across threads within a single process — which is why
//! no additional in-process mutex is required.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{GitError, Result};

/// An exclusive, cross-process advisory lock on a database directory.
///
/// The lock is acquired on construction and released when this guard is
/// dropped. Hold it for as long as the protected operation runs.
pub(crate) struct DatabaseLock {
    file: File,
}

impl DatabaseLock {
    /// Path of the lock file for a database directory.
    ///
    /// The lock lives *next to* the database directory (a sibling
    /// `<name>.bender-lock` file) rather than inside it, so it never appears in
    /// git's view of the repository.
    fn lock_path(db_dir: &Path) -> PathBuf {
        let mut name = db_dir
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".bender-lock");
        db_dir.with_file_name(name)
    }

    /// Acquire the exclusive lock for the database at `db_dir`, blocking the
    /// current thread until it becomes available.
    ///
    /// The parent directory of `db_dir` must exist.
    pub fn acquire_blocking(db_dir: &Path) -> Result<Self> {
        let path = Self::lock_path(db_dir);
        // The lock file is a pure marker; its contents are never read or
        // written, so keep any existing file as-is (no truncation).
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        file.lock()?;
        Ok(Self { file })
    }

    /// Async counterpart of [`acquire_blocking`](Self::acquire_blocking).
    ///
    /// The blocking `flock` is performed on a blocking thread so it does not
    /// stall the async runtime while waiting for a contended lock.
    pub async fn acquire(db_dir: &Path) -> Result<Self> {
        let db_dir = db_dir.to_path_buf();
        tokio::task::spawn_blocking(move || Self::acquire_blocking(&db_dir))
            .await
            .map_err(|e| GitError::Io(std::io::Error::other(e.to_string())))?
    }
}

impl Drop for DatabaseLock {
    fn drop(&mut self) {
        // The lock is also released when the fd is closed; unlock explicitly so
        // it is released promptly. Nothing actionable on failure.
        let _ = self.file.unlock();
    }
}
