// Copyright (c) 2025 ETH Zurich

//! Cross-process filesystem advisory locks.
//!
//! Bender uses these to coordinate concurrent invocations against the same git
//! database. The lock is taken on a sentinel file in
//! `<database>/git/locks/<name>-<hash>.lock`: writers (database initialization
//! and fetches) acquire it exclusively, while readers (version resolution and
//! checkouts, which never mutate the database) acquire it shared so they can
//! proceed in parallel. The lock is released automatically when the [`FsLock`]
//! guard is dropped.

#![deny(missing_docs)]

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use miette::{Context as _, IntoDiagnostic as _};

use crate::Result;

/// An exclusive, cross-process advisory lock held on a sentinel file.
///
/// The lock is released when this guard is dropped (or when the process exits).
pub struct FsLock {
    file: Option<File>,
    path: PathBuf,
}

impl FsLock {
    /// Acquire an exclusive (writer) lock on `path`, creating the file if
    /// missing.
    ///
    /// Use this for operations that mutate the git database (initialization or
    /// fetching). See [`acquire`](Self::acquire) for the blocking behavior.
    pub async fn acquire_exclusive(path: PathBuf) -> Result<Self> {
        Self::acquire(path, true).await
    }

    /// Acquire a shared (reader) lock on `path`, creating the file if missing.
    ///
    /// Multiple processes may hold a shared lock simultaneously; a shared lock
    /// only excludes exclusive lockers. Use this for read-only access to the
    /// git database, such as resolving versions or creating a checkout. This
    /// is what lets concurrent bender invocations check out in parallel against
    /// a shared, pre-populated database.
    pub async fn acquire_shared(path: PathBuf) -> Result<Self> {
        Self::acquire(path, false).await
    }

    /// Acquire a lock on `path`, creating the file if missing.
    ///
    /// If the lock is contended, an info message is logged so the user can see
    /// why bender is waiting, and the call then blocks until the lock is
    /// available. The actual lock acquisition runs on a blocking worker so it
    /// does not stall the tokio runtime.
    async fn acquire(path: PathBuf, exclusive: bool) -> Result<Self> {
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

        let path_for_blocking = path.clone();
        let file = tokio::task::spawn_blocking(move || -> Result<File> {
            let try_lock = if exclusive {
                file.try_lock()
            } else {
                file.try_lock_shared()
            };
            match try_lock {
                Ok(()) => Ok(file),
                Err(TryLockError::WouldBlock) => {
                    log::info!("waiting for lock on {:?}", path_for_blocking);
                    let blocking_lock = if exclusive {
                        file.lock()
                    } else {
                        file.lock_shared()
                    };
                    blocking_lock.into_diagnostic().wrap_err_with(|| {
                        format!("Failed to acquire lock on {:?}.", path_for_blocking)
                    })?;
                    Ok(file)
                }
                Err(TryLockError::Error(e)) => Err(e)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("Failed to try-lock {:?}.", path_for_blocking)),
            }
        })
        .await
        .into_diagnostic()
        .wrap_err("Lock acquisition task panicked.")??;

        Ok(FsLock {
            file: Some(file),
            path,
        })
    }

    /// The path of the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FsLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Best-effort: errors here are not actionable, and the OS releases
            // the lock automatically when the file handle closes.
            let _ = file.unlock();
        }
    }
}
