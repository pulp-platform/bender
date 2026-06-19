// Copyright (c) 2025 ETH Zurich

//! Cross-process filesystem advisory locks.
//!
//! Bender uses these to coordinate concurrent invocations against the same git
//! database. The lock is taken on a sentinel file kept as a sibling of the
//! resource it guards -- `<database>/git/db/<name>-<hash>.lock` for a bare
//! database, or `<checkout>.lock` for a working-tree checkout: writers
//! (database initialization and fetches) acquire it exclusively, while readers
//! (version resolution and checkouts, which never mutate the database) acquire
//! it shared so they can proceed in parallel. The lock is released
//! automatically when the [`FsLockGuard`] is dropped.

#![deny(missing_docs)]

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

use miette::{Context as _, IntoDiagnostic as _};

use crate::Result;

/// A cross-process advisory lock identified by a sentinel file path.
pub struct FsLock {
    path: PathBuf,
}

impl FsLock {
    /// Create a lock on the sentinel file at `path`.
    ///
    /// This does not touch the filesystem; the file is created and locked when
    /// [`acquire_exclusive`](Self::acquire_exclusive) or
    /// [`acquire_shared`](Self::acquire_shared) is called.
    pub fn new(path: PathBuf) -> Self {
        FsLock { path }
    }

    /// Acquire an exclusive (writer) lock, creating the file if missing.
    ///
    /// Use this for operations that mutate the git database (initialization or
    /// fetching). See [`acquire`](Self::acquire) for the blocking behavior.
    pub async fn acquire_exclusive(&self) -> Result<FsLockGuard> {
        self.acquire(true).await
    }

    /// Acquire a shared (reader) lock, creating the file if missing.
    ///
    /// Multiple processes may hold a shared lock simultaneously; a shared lock
    /// only excludes exclusive lockers. Use this for read-only access to the
    /// git database, such as resolving versions or creating a checkout. This
    /// is what lets concurrent bender invocations check out in parallel against
    /// a shared, pre-populated database.
    pub async fn acquire_shared(&self) -> Result<FsLockGuard> {
        self.acquire(false).await
    }

    /// Acquire the lock, creating the file if missing.
    ///
    /// If the lock is contended, the call blocks until the lock is available.
    async fn acquire(&self, exclusive: bool) -> Result<FsLockGuard> {
        let path = self.path.clone();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .into_diagnostic()
                .wrap_err_with(|| format!("Failed to create lock directory {:?}.", parent))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to open lock file {:?}.", path))?;

        let file = tokio::task::spawn_blocking(move || -> Result<File> {
            let try_lock = if exclusive {
                file.try_lock()
            } else {
                file.try_lock_shared()
            };
            match try_lock {
                Ok(()) => Ok(file),
                Err(TryLockError::WouldBlock) => {
                    log::info!("waiting for lock on {:?}", path);
                    let blocking_lock = if exclusive {
                        file.lock()
                    } else {
                        file.lock_shared()
                    };
                    blocking_lock
                        .into_diagnostic()
                        .wrap_err_with(|| format!("Failed to acquire lock on {:?}.", path))?;
                    Ok(file)
                }
                Err(TryLockError::Error(e)) => Err(e)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("Failed to try-lock {:?}.", path)),
            }
        })
        .await
        .into_diagnostic()
        .wrap_err("Lock acquisition task panicked.")??;

        Ok(FsLockGuard { file })
    }
}

/// A held [`FsLock`], released when this guard is dropped (or the process exits).
pub struct FsLockGuard {
    file: File,
}

impl Drop for FsLockGuard {
    fn drop(&mut self) {
        // Best-effort: errors here are not actionable, and the OS releases the
        // lock automatically when the file handle closes.
        let _ = self.file.unlock();
    }
}
