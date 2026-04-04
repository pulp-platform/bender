use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;
use walkdir::WalkDir;

use crate::database::GitDatabase;
use crate::error::{GitError, Result};
use crate::progress::{GitProgressSink, NoProgress};
use crate::subprocess::SubprocessRunner;
use crate::types::ObjectId;

/// A git working tree checkout.
///
/// This corresponds to the `git/checkouts/{name}-{hash}/` directories in
/// bender's local cache. Like [`GitDatabase`], this struct does not manage the
/// filesystem path — the caller creates, moves, and removes directories as
/// needed.
///
/// ## Clone vs. worktree
///
/// Checkouts are created via `git clone --local --branch <tag>` rather than
/// `git worktree add`. The worktree approach was evaluated but rejected because:
///
/// - `git worktree add` on bare repos requires git ≥ 2.37 (compatibility risk).
/// - Worktrees are tightly coupled to the source bare repo's filesystem path,
///   so moving or deleting the database breaks all linked checkouts.
/// - Deleting a worktree requires `git worktree remove`, not just `rm -rf`.
/// - LFS per-checkout URL configuration becomes a concurrency hazard.
///
/// The `--local` clone already shares object storage via
/// `objects/info/alternates`, so disk usage is efficient.
#[derive(Clone)]
pub struct GitCheckout {
    /// Absolute path to the working tree directory.
    pub path: PathBuf,
    git_bin: PathBuf,
    throttle: Arc<Semaphore>,
}

impl GitCheckout {
    /// Construct a handle to a working tree at `path`.
    pub fn new(
        path: impl Into<PathBuf>,
        git_bin: impl Into<PathBuf>,
        throttle: Arc<Semaphore>,
    ) -> Self {
        Self {
            path: path.into(),
            git_bin: git_bin.into(),
            throttle,
        }
    }

    fn runner(&self) -> SubprocessRunner {
        SubprocessRunner::new(
            self.git_bin.clone(),
            self.path.clone(),
            self.throttle.clone(),
        )
    }

    fn open_repo(&self) -> Result<gix::Repository> {
        gix::open(&self.path).map_err(|e| GitError::Gix(e.to_string()))
    }

    // ── Initialisation ────────────────────────────────────────────────────────

    /// Clone from a local bare database and check out `branch_or_tag`.
    ///
    /// `branch_or_tag` must be a named ref (branch or tag), not a bare commit
    /// hash, because `git clone --branch` does not accept commit hashes. The
    /// typical caller workflow:
    ///
    /// ```no_run
    /// # async fn example() -> bender_git::error::Result<()> {
    /// # use std::sync::Arc;
    /// # use tokio::sync::Semaphore;
    /// # use std::path::Path;
    /// # use bender_git::database::GitDatabase;
    /// # use bender_git::checkout::GitCheckout;
    /// # use bender_git::types::ObjectId;
    /// # use bender_git::progress::NoProgress;
    /// # let throttle = Arc::new(Semaphore::new(4));
    /// # let db = GitDatabase::new(Path::new("/db"), "git", throttle.clone());
    /// # let rev = ObjectId("abc123".repeat(6).chars().take(40).collect());
    /// # let checkout = GitCheckout::new(Path::new("/checkout"), "git", throttle);
    /// // 1. Tag the commit so git clone can reference it by name.
    /// let tag = format!("bender-tmp-{}", rev.short(8));
    /// db.tag_commit(&tag, &rev).await?;
    ///
    /// // 2. Clone from the database using the tag.
    /// checkout.clone_from(&db, &tag, NoProgress).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clone_from(
        &self,
        database: &GitDatabase,
        branch_or_tag: &str,
        _progress: impl GitProgressSink,
    ) -> Result<()> {
        let db_path = database.path.to_str().ok_or_else(|| GitError::Gix(
            "database path is not valid UTF-8".into(),
        ))?;
        let checkout_path = self.path.to_str().ok_or_else(|| GitError::Gix(
            "checkout path is not valid UTF-8".into(),
        ))?;

        // Use a SubprocessRunner rooted at the *parent* directory since the
        // checkout directory does not exist yet.
        let parent = self.path.parent().ok_or_else(|| GitError::Gix(
            "checkout path has no parent directory".into(),
        ))?;
        let runner = SubprocessRunner::new(
            self.git_bin.clone(),
            parent.to_path_buf(),
            self.throttle.clone(),
        );

        runner
            .run_discard(&[
                "clone",
                "--local",
                "--branch",
                branch_or_tag,
                db_path,
                checkout_path,
            ])
            .await
    }

    // ── Interrogation ─────────────────────────────────────────────────────────

    /// Return the commit OID currently checked out (`HEAD^{commit}`), or
    /// `None` if the repository has no commits.
    ///
    /// This is a pure local read implemented via `gix`; no semaphore acquired.
    pub fn current_checkout(&self) -> Result<Option<ObjectId>> {
        let repo = self.open_repo()?;
        match repo.head_id() {
            Ok(id) => Ok(Some(ObjectId(id.to_string()))),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("unborn") || msg.contains("no commits") {
                    Ok(None)
                } else {
                    Err(GitError::Gix(msg))
                }
            }
        }
    }

    /// Return the URL of a remote (used to verify the checkout points at the
    /// expected database).
    ///
    /// This is a pure local read via `gix`; no semaphore acquired.
    pub fn remote_url(&self, remote: &str) -> Result<String> {
        let repo = self.open_repo()?;
        let key = format!("remote.{}.url", remote);
        let snapshot = repo.config_snapshot();
        let url = snapshot
            .string(key.as_str())
            .ok_or_else(|| GitError::RefNotFound {
                refname: format!("remote.{}.url", remote),
            })?;
        Ok(url.to_string())
    }

    /// Return `true` if the working tree has any staged or unstaged changes.
    pub async fn is_dirty(&self) -> Result<bool> {
        let output = self
            .runner()
            .run_str(&["status", "--porcelain"], true)
            .await?;
        Ok(!output.trim().is_empty())
    }

    // ── Update ────────────────────────────────────────────────────────────────

    /// Fetch from all remotes and check out `rev_or_tag`.
    pub async fn fetch_and_checkout(
        &self,
        rev_or_tag: &str,
        _progress: impl GitProgressSink,
    ) -> Result<()> {
        let runner = self.runner();
        runner
            .run_discard(&["fetch", "--all", "--tags", "--prune"])
            .await?;
        runner
            .run_discard(&["checkout", "--force", rev_or_tag])
            .await
    }

    /// Initialise and update git submodules recursively.
    pub async fn update_submodules(&self, _progress: impl GitProgressSink) -> Result<()> {
        self.runner()
            .run_discard(&[
                "submodule",
                "update",
                "--init",
                "--recursive",
            ])
            .await
    }

    // ── LFS ───────────────────────────────────────────────────────────────────

    /// Return `true` if any `.gitattributes` file in the working tree contains
    /// a `filter=lfs` line.
    ///
    /// Runs the file walk in a blocking thread via `tokio::task::spawn_blocking`
    /// to avoid blocking the async runtime.
    pub async fn uses_lfs_attributes(&self) -> Result<bool> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Ok(WalkDir::new(&path)
                .into_iter()
                .flatten()
                .any(|entry| {
                    if entry.file_type().is_file()
                        && entry.file_name() == ".gitattributes"
                    {
                        std::fs::read_to_string(entry.path())
                            .map(|c| c.contains("filter=lfs"))
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }))
        })
        .await
        .map_err(|e| GitError::Gix(format!("blocking task failed: {}", e)))?
    }

    /// Return `true` if the checkout actually tracks any files via LFS.
    ///
    /// Runs `git lfs ls-files`; returns `false` if git-lfs is not installed.
    pub async fn uses_lfs(&self) -> Result<bool> {
        let output = self
            .runner()
            .run_str(&["lfs", "ls-files"], false)
            .await?;
        Ok(!output.trim().is_empty())
    }

    /// Configure the LFS URL and pull LFS objects.
    pub async fn lfs_pull(&self, lfs_url: &str) -> Result<()> {
        let runner = self.runner();
        runner
            .run_discard(&["config", "lfs.url", lfs_url])
            .await?;
        runner.run_discard(&["lfs", "pull"]).await
    }
}

// ── No-progress convenience wrappers ─────────────────────────────────────────

impl GitCheckout {
    /// Clone from `database`, reporting no progress.
    pub async fn clone_from_silent(
        &self,
        database: &GitDatabase,
        branch_or_tag: &str,
    ) -> Result<()> {
        self.clone_from(database, branch_or_tag, NoProgress).await
    }

    /// Fetch and check out `rev_or_tag`, reporting no progress.
    pub async fn fetch_and_checkout_silent(&self, rev_or_tag: &str) -> Result<()> {
        self.fetch_and_checkout(rev_or_tag, NoProgress).await
    }
}
