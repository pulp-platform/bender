use std::path::PathBuf;

use crate::error::{GitError, Result};
use crate::subprocess::{GIT_LFS, SubprocessRunner};
use crate::types::ObjectId;

/// A git working tree checkout.
///
/// This corresponds to the `git/checkouts/{name}-{hash}/` directories in
/// bender's local cache. Like [`GitDatabase`](crate::database::GitDatabase), this struct does not manage the
/// filesystem path — the caller creates, moves, and removes directories as
/// needed.
///
/// Checkouts are cloned from a [`GitDatabase`](crate::database::GitDatabase) with `--shared`, which sets up
/// `.git/objects/info/alternates` so all objects in the database are visible
/// without copying them. This keeps disk usage minimal while allowing the
/// checkout to be moved or deleted independently of the database.
#[derive(Clone)]
pub struct GitCheckout {
    repo: gix::Repository,
}

impl GitCheckout {
    /// Open an existing working tree at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let repo = gix::open(path)?;
        Ok(Self { repo })
    }

    fn runner(&self) -> Result<SubprocessRunner> {
        let work_dir = self.repo.workdir().expect("checkout should have work dir");
        SubprocessRunner::new(work_dir.to_path_buf())
    }

    /// Return the commit OID currently checked out (`HEAD^{commit}`).
    ///
    /// This is a pure local read implemented via `gix`; no semaphore acquired.
    pub fn current_checkout(&self) -> Result<ObjectId> {
        let id = self.repo.head_id()?;
        Ok(ObjectId::from(id.detach()))
    }

    /// Return the URL of a remote (used to verify the checkout points at the
    /// expected database).
    ///
    /// This is a pure local read via `gix`; no semaphore acquired.
    pub fn remote_url(&self, remote: &str) -> Result<String> {
        let remote = self.repo.find_remote(remote)?;
        let url = remote
            .url(gix::remote::Direction::Fetch)
            .ok_or(GitError::RefNotFound {
                refname: "fetch url".into(),
            })?;
        Ok(url.to_string())
    }

    /// Return `true` if the working tree has any modifications.
    ///
    /// Uses `git status --porcelain` via subprocess rather than `gix::Repository::is_dirty()`
    /// because gix deliberately excludes untracked files from its dirty check, whereas bender
    /// must treat untracked files as dirty too — a user may have created new files in a
    /// dependency checkout that bender is about to overwrite.
    pub async fn is_dirty(&self) -> Result<bool> {
        let output = self.runner()?.run(&["status", "--porcelain"], &[]).await?;
        let output = String::from_utf8_lossy(&output);
        Ok(!output.trim().is_empty())
    }

    /// Switch the working tree to `rev`.
    ///
    /// Since checkouts are cloned with `--shared`, all objects in the database
    /// are accessible via alternates — no fetch required before switching.
    /// LFS smudging is disabled; call `lfs_pull` afterwards if needed.
    pub async fn switch(&self, rev: &ObjectId) -> Result<()> {
        self.runner()?
            .run_discard(
                &["switch", "--detach", "--force", &rev.to_string()],
                &[("GIT_LFS_SKIP_SMUDGE", "1")],
            )
            .await
    }

    /// Initialise and update git submodules recursively.
    ///
    /// Submodule updates are not progress-tracked.
    pub async fn update_submodules(&self) -> Result<()> {
        if self.repo.submodules()?.is_none() {
            return Ok(());
        }

        self.runner()?
            .run_discard(
                &["submodule", "update", "--init", "--recursive"],
                &[("GIT_TERMINAL_PROMPT", "0"), ("GIT_LFS_SKIP_SMUDGE", "1")],
            )
            .await
    }

    /// Pull LFS objects for the current checkout, pointing git-lfs at `lfs_url`.
    ///
    /// Runs `git lfs ls-files` first; if no LFS-tracked files are present this
    /// is a no-op and returns `Ok(false)`. Returns `Ok(true)` if LFS objects
    /// were actually pulled. Callers should only call this when LFS is enabled
    /// in the bender config — if LFS files are present but this is never called,
    /// the working tree will contain raw LFS pointer files instead of content.
    ///
    /// `lfs_url` must be the URL of the original remote (not the local database
    /// path), since LFS objects are stored on the LFS server, not in the bare repo.
    pub async fn lfs_pull(&self, lfs_url: &str) -> Result<bool> {
        // Verify git-lfs is installed before doing anything. This is checked
        // once and cached for the lifetime of the process (LazyLock). Returning
        // an error here lets the caller surface a clear warning rather than
        // silently leaving raw LFS pointer files in the working tree.
        GIT_LFS
            .as_ref()
            .map_err(|e| GitError::LfsNotFound(e.clone()))?;

        let runner = self.runner()?;
        let output = runner.run(&["lfs", "ls-files"], &[]).await?;
        let output = String::from_utf8_lossy(&output);
        if output.trim().is_empty() {
            return Ok(false);
        }
        runner
            .run_discard(&["config", "lfs.url", lfs_url], &[])
            .await?;
        runner
            .run_discard(&["lfs", "pull"], &[("GIT_TERMINAL_PROMPT", "0")])
            .await?;
        Ok(true)
    }
}
