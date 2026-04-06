use std::path::PathBuf;
use std::sync::Arc;

use crate::database::GitDatabase;
use crate::error::{GitError, Result, gix_err};
use crate::progress::GitProgressSink;
use crate::subprocess::{GIT_LFS, SubprocessRunner};
use crate::types::ObjectId;
use tokio::sync::Semaphore;

/// A git working tree checkout.
///
/// This corresponds to the `git/checkouts/{name}-{hash}/` directories in
/// bender's local cache. Like [`GitDatabase`], this struct does not manage the
/// filesystem path — the caller creates, moves, and removes directories as
/// needed.
///
/// Checkouts are cloned from a [`GitDatabase`] with `--shared`, which sets up
/// `.git/objects/info/alternates` so all objects in the database are visible
/// without copying them. This keeps disk usage minimal while allowing the
/// checkout to be moved or deleted independently of the database.
#[derive(Clone)]
pub struct GitCheckout {
    /// Absolute path to the working tree directory.
    pub path: PathBuf,
    throttle: Arc<Semaphore>,
}

impl GitCheckout {
    /// Construct a handle to a working tree at `path`.
    pub fn new(path: impl Into<PathBuf>, throttle: Arc<Semaphore>) -> Self {
        Self {
            path: path.into(),
            throttle,
        }
    }

    fn runner(&self) -> SubprocessRunner {
        SubprocessRunner::new(self.path.clone(), self.throttle.clone())
    }

    /// Clone from a local bare database and check out `branch_or_tag`.
    ///
    /// `branch_or_tag` must be a named ref (branch or tag), not a bare commit
    /// hash, because `git clone --branch` does not accept commit hashes. The
    /// typical caller workflow:
    ///
    /// ```no_run
    /// // 1. Tag the commit so git clone can reference it by name.
    /// let tag = format!("bender-tmp-{}", rev.short(8));
    /// db.tag_commit(&tag, &rev)?;
    ///
    /// // 2. Clone from the database using the tag.
    /// checkout.clone_from(&db, &tag, NoProgress).await?;
    /// ```
    pub async fn clone_from(
        &self,
        database: &GitDatabase,
        branch_or_tag: &str,
        _progress: impl GitProgressSink,
    ) -> Result<()> {
        let db_path = database
            .path
            .to_str()
            .ok_or_else(|| GitError::Gix("database path is not valid UTF-8".into()))?;
        let checkout_path = self
            .path
            .to_str()
            .ok_or_else(|| GitError::Gix("checkout path is not valid UTF-8".into()))?;

        // Use a SubprocessRunner rooted at the *parent* directory since the
        // checkout directory does not exist yet.
        let parent = self
            .path
            .parent()
            .ok_or_else(|| GitError::Gix("checkout path has no parent directory".into()))?;
        let runner = SubprocessRunner::new(parent.to_path_buf(), self.throttle.clone());

        // --shared sets up .git/objects/info/alternates pointing at the
        // database's object directory. The checkout owns no objects itself;
        // all object lookups fall through to the database. This means any
        // commit fetched into the database is immediately visible to the
        // checkout, so updating to a newer revision requires no fetch step.
        //
        // The risk of --shared is that git-gc in the database can prune
        // objects still needed by the checkout. This is safe here because
        // bender always creates a bender-tmp-<hash> tag in the database
        // before updating a checkout to that commit, and only removes old
        // bender-tmp-* tags after the checkout has moved on — so gc will
        // never see the referenced objects as unreachable.
        // GIT_LFS_SKIP_SMUDGE=1 prevents git-lfs from downloading LFS objects
        // during clone. LFS objects are pulled explicitly afterwards via
        // lfs_pull() so bender can decide whether LFS is needed.
        // filter.lfs.required=false is a safety net: if git-lfs is registered
        // in the git config but not installed, the clone won't fail.
        runner
            .run_discard_with_env(
                &[
                    "clone",
                    "--shared",
                    "--branch",
                    branch_or_tag,
                    db_path,
                    checkout_path,
                ],
                &[("GIT_LFS_SKIP_SMUDGE", "1")],
            )
            .await
    }

    /// Return the commit OID currently checked out (`HEAD^{commit}`).
    ///
    /// This is a pure local read implemented via `gix`; no semaphore acquired.
    pub fn current_checkout(&self) -> Result<ObjectId> {
        let repo = gix::open(&self.path).map_err(gix_err)?;
        let id = repo.head_id().map_err(gix_err)?;
        Ok(ObjectId::from(id.detach()))
    }

    /// Return the URL of a remote (used to verify the checkout points at the
    /// expected database).
    ///
    /// This is a pure local read via `gix`; no semaphore acquired.
    pub fn remote_url(&self, remote: &str) -> Result<String> {
        let repo = gix::open(&self.path).map_err(gix_err)?;
        let remote = repo.find_remote(remote).map_err(gix_err)?;
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
        let output = self
            .runner()
            .run_str(&["status", "--porcelain"], true)
            .await?;
        Ok(!output.trim().is_empty())
    }

    /// Switch the working tree to `rev`.
    ///
    /// Since checkouts are cloned with `--shared`, all objects in the database
    /// are accessible via alternates — no fetch required before switching.
    /// LFS smudging is disabled; call `lfs_pull` afterwards if needed.
    pub async fn switch(&self, rev: &ObjectId) -> Result<()> {
        self.runner()
            .run_discard_with_env(
                &["switch", "--detach", "--force", &rev.to_string()],
                &[("GIT_LFS_SKIP_SMUDGE", "1")],
            )
            .await
    }

    /// Initialise and update git submodules recursively.
    pub async fn update_submodules(&self, _progress: impl GitProgressSink) -> Result<()> {
        self.runner()
            .run_discard_with_env(
                &["submodule", "update", "--init", "--recursive"],
                &[("GIT_LFS_SKIP_SMUDGE", "1")],
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

        let runner = self.runner();
        let output = runner.run_str(&["lfs", "ls-files"], true).await?;
        if output.trim().is_empty() {
            return Ok(false);
        }
        runner.run_discard(&["config", "lfs.url", lfs_url]).await?;
        runner.run_discard(&["lfs", "pull"]).await?;
        Ok(true)
    }
}
