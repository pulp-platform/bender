use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::error::{GitError, Result, gix_err};
use crate::progress::GitProgressSink;
use crate::subprocess::SubprocessRunner;
use crate::types::ObjectId;

/// A bare git repository used as a local object cache ("database").
///
/// This corresponds to the `git/db/{name}-{hash}/` directories in bender's
/// local cache. The struct holds no state beyond the path and execution context;
/// all git state is on disk in the repository directory.
///
/// ## Filesystem layout
///
/// The caller is responsible for creating the directory before construction
/// and for managing the directory's location and naming. This struct only
/// accepts an absolute path to the repository root.
///
/// ## Operation categories
///
/// - **Subprocess operations** (async, acquire the throttle semaphore):
///   `fetch`, `fetch_ref`, `add_remote`.
///   Network I/O or operations where gix has no public disk-write API.
///
/// - **`gix` operations** (synchronous, no semaphore):
///   `tag_commit`, `list_refs`, `list_revs`, `cat_file`, `list_files`,
///   `resolve`, `remote_url`.
///   All local reads and writes — fast and safe to run concurrently.
#[derive(Clone)]
pub struct GitDatabase {
    /// Absolute path to the bare repository directory.
    pub path: PathBuf,
    throttle: Arc<Semaphore>,
    repo: gix::ThreadSafeRepository,
}

impl GitDatabase {
    /// Initialise a new bare repository at `path` and return a handle to it.
    ///
    /// Equivalent to `git init --bare`. The directory must already exist.
    pub fn init_bare(path: impl Into<PathBuf>, throttle: Arc<Semaphore>) -> Result<Self> {
        let path = path.into();
        gix::init_bare(&path).map_err(gix_err)?;
        let repo = gix::open(&path).map_err(gix_err)?.into_sync();
        Ok(Self {
            path,
            throttle,
            repo,
        })
    }

    /// Open an existing bare repository at `path`.
    pub fn open(path: impl Into<PathBuf>, throttle: Arc<Semaphore>) -> Result<Self> {
        let path = path.into();
        let repo = gix::open(&path).map_err(gix_err)?.into_sync();
        Ok(Self {
            path,
            throttle,
            repo,
        })
    }

    fn runner(&self) -> SubprocessRunner {
        SubprocessRunner::new(self.path.clone(), self.throttle.clone())
    }

    /// Add a remote (e.g. `origin`).
    ///
    /// Equivalent to `git remote add <name> <url>`.
    ///
    /// This uses the `git` subprocess even though it is a local operation.
    /// gix's `remote_at()` creates an in-memory remote only; there is currently
    /// no public API to persist it to `.git/config`.
    pub async fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        self.runner()
            .run_discard(&["remote", "add", name, url])
            .await
    }

    /// Fetch all tags and branches from `remote`.
    ///
    /// Equivalent to `git fetch --tags --prune <remote> --progress`.
    pub async fn fetch(&self, remote: &str, _progress: impl GitProgressSink) -> Result<()> {
        // Progress integration is stubbed for v1; see progress.rs for the
        // planned trait boundary. The `--progress` flag causes git to write
        // progress to stderr, which is currently discarded.
        self.runner()
            .run_discard(&["fetch", "--tags", "--prune", remote, "--progress"])
            .await
    }

    /// Fetch a specific ref from `remote`.
    ///
    /// Useful when a pinned commit hash is not reachable from any named ref
    /// (e.g. after a force-push), in which case the full OID must be fetched
    /// explicitly.
    pub async fn fetch_ref(
        &self,
        remote: &str,
        refspec: &str,
        _progress: impl GitProgressSink,
    ) -> Result<()> {
        self.runner()
            .run_discard(&["fetch", remote, refspec, "--progress"])
            .await
    }

    /// Create or overwrite a local tag pointing to `commit`.
    ///
    /// Bender uses this to create short-lived `bender-tmp-<rev>` tags so that
    /// `git clone --branch` can check out an arbitrary commit (since `--branch`
    /// only accepts named refs, not bare hashes).
    pub fn tag_commit(&self, tag_name: &str, commit: &ObjectId) -> Result<()> {
        use gix::refs::transaction::PreviousValue;
        let repo = self.repo.to_thread_local();
        repo.tag_reference(tag_name, *commit, PreviousValue::Any)
            .map_err(gix_err)?;
        Ok(())
    }

    /// List all tags, returning `(short_name, commit_oid)` pairs.
    ///
    /// Annotated tags are peeled to their target commit. Broken symrefs are
    /// silently skipped.
    pub fn list_tags(&self) -> Result<Vec<(String, ObjectId)>> {
        let repo = self.repo.to_thread_local();
        let mut result = Vec::new();
        for reference in repo
            .references()
            .map_err(gix_err)?
            .tags()
            .map_err(gix_err)?
        {
            let mut reference = reference.map_err(gix_err)?;
            let name = reference.name().as_bstr().to_string();
            let Ok(id) = reference.peel_to_id() else {
                continue;
            };
            let short = name.strip_prefix("refs/tags/").unwrap_or(&name).to_owned();
            result.push((short, ObjectId::from(id.detach())));
        }
        Ok(result)
    }

    /// List all remote-tracking branches, returning `(short_name, commit_oid)` pairs.
    ///
    /// Short names have the `refs/remotes/origin/` prefix stripped. Broken
    /// symrefs are silently skipped.
    pub fn list_branches(&self) -> Result<Vec<(String, ObjectId)>> {
        let repo = self.repo.to_thread_local();
        let mut result = Vec::new();
        for reference in repo
            .references()
            .map_err(gix_err)?
            .remote_branches()
            .map_err(gix_err)?
        {
            let mut reference = reference.map_err(gix_err)?;
            let name = reference.name().as_bstr().to_string();
            let Ok(id) = reference.peel_to_id() else {
                continue;
            };
            let short = name
                .strip_prefix("refs/remotes/origin/")
                .unwrap_or(&name)
                .to_owned();
            result.push((short, ObjectId::from(id.detach())));
        }
        Ok(result)
    }

    /// List all reachable commits in commit-time order (newest first).
    ///
    /// Equivalent to `git rev-list --all --date-order`.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn list_revs(&self) -> Result<Vec<ObjectId>> {
        let repo = self.repo.to_thread_local();

        let tips: Vec<gix::ObjectId> = repo
            .references()
            .map_err(gix_err)?
            .all()
            .map_err(gix_err)?
            .filter_map(|r| r.ok()?.peel_to_id().ok().map(|id| id.detach()))
            .collect();

        repo.rev_walk(tips)
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                Default::default(),
            ))
            .all()
            .map_err(gix_err)?
            .map(|info| Ok(ObjectId::from(info.map_err(gix_err)?.id)))
            .collect()
    }

    /// Read a blob object by its hash and return its content as UTF-8.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn cat_file(&self, oid: &ObjectId) -> Result<String> {
        let repo = self.repo.to_thread_local();
        let obj = repo
            .find_object(*oid)
            .map_err(|_| GitError::ObjectNotFound {
                oid: oid.to_string(),
            })?;
        String::from_utf8(obj.data.to_vec()).map_err(|_| GitError::InvalidUtf8 {
            context: format!("object {}", oid),
        })
    }

    /// Read the content of a file at `path` in the tree at `rev`.
    ///
    /// Returns `None` if the path does not exist in the tree.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn read_file(&self, rev: &ObjectId, path: &Path) -> Result<Option<String>> {
        let repo = self.repo.to_thread_local();
        let commit = repo
            .find_commit(*rev)
            .map_err(|_| GitError::ObjectNotFound {
                oid: rev.to_string(),
            })?;
        let tree = commit.tree().map_err(gix_err)?;
        let Some(entry) = tree.lookup_entry_by_path(path).map_err(gix_err)? else {
            return Ok(None);
        };
        let blob = entry.object().map_err(gix_err)?;
        let content = String::from_utf8(blob.data.to_vec()).map_err(|_| GitError::InvalidUtf8 {
            context: path.display().to_string(),
        })?;
        Ok(Some(content))
    }

    /// Resolve a revision expression (ref name, commit-ish, etc.) to an
    /// `ObjectId`.
    ///
    /// The expression is automatically suffixed with `^{commit}` to ensure the
    /// result is always a commit hash, peeling through tags if necessary.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn resolve(&self, expr: &str) -> Result<ObjectId> {
        let repo = self.repo.to_thread_local();
        let spec = format!("{}^{{commit}}", expr);
        let id = repo.rev_parse_single(spec.as_str()).map_err(gix_err)?;
        Ok(ObjectId::from(id.detach()))
    }

    /// Return the URL of a remote.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    ///
    /// Note: this re-opens the repository on each call rather than using the
    /// cached `ThreadSafeRepository`. Remotes are added via subprocess after
    /// construction, so the cached config snapshot would not include them.
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
}
