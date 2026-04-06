use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::error::{gix_err, GitError, Result};
use crate::progress::{GitProgressSink, NoProgress};
use crate::subprocess::SubprocessRunner;
use crate::types::{GitRef, ObjectId, RefKind, TreeEntry, TreeEntryKind};

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
///   `init_bare`, `tag_commit`,
///   `list_refs`, `list_revs`, `cat_file`, `list_files`, `resolve`,
///   `remote_url`.
///   All local reads and writes — fast and safe to run concurrently.
#[derive(Clone)]
pub struct GitDatabase {
    /// Absolute path to the bare repository directory.
    pub path: PathBuf,
    git_bin: PathBuf,
    throttle: Arc<Semaphore>,
}

impl GitDatabase {
    /// Construct a handle to a bare repository at `path`.
    ///
    /// The directory at `path` must already exist. Call [`Self::init_bare`]
    /// if the repository has not yet been initialised.
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
        gix::open(&self.path).map_err(gix_err)
    }

    // ── Initialisation ───────────────────────────────────────────────────────

    /// Initialise a bare repository in the directory (idempotent).
    ///
    /// Equivalent to `git init --bare`.
    pub fn init_bare(&self) -> Result<()> {
        if gix::open(&self.path).is_ok() {
            return Ok(()); // already a valid git repo
        }
        gix::init_bare(&self.path).map_err(gix_err)?;
        Ok(())
    }

    /// Add a remote (e.g. `origin`).
    ///
    /// Equivalent to `git remote add <name> <url>`.
    ///
    /// This uses the `git` subprocess even though it is a local operation.
    /// gix's `remote_at()` creates an in-memory remote only; persisting it to
    /// `.git/config` requires internal gix helpers (`write_remote_to_local_config_file`)
    /// that are not part of the public API. Since `fetch` is also a subprocess
    /// that reads the remote URL from `.git/config` at runtime, the config must
    /// be written to disk anyway — subprocess is the simplest correct path here.
    pub async fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        self.runner()
            .run_discard(&["remote", "add", name, url])
            .await
    }

    // ── Network operations (subprocess, throttled) ────────────────────────────

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
        let repo = self.open_repo()?;
        let oid = parse_oid(commit)?;
        repo.tag_reference(tag_name, oid, PreviousValue::Any)
            .map_err(gix_err)?;
        Ok(())
    }

    // ── Read operations (gix, not throttled) ─────────────────────────────────

    /// List all references in the repository, resolving annotated tags to
    /// their target commits.
    ///
    /// Refs in `refs/notes/` and `refs/stash` are silently skipped.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn list_refs(&self) -> Result<Vec<GitRef>> {
        let repo = self.open_repo()?;
        let mut result = Vec::new();

        for reference in repo
            .references()
            .map_err(gix_err)?
            .all()
            .map_err(gix_err)?
        {
            let mut reference = reference.map_err(gix_err)?;

            let name = reference.name().as_bstr().to_string();

            // Skip ref namespaces that bender doesn't need.
            if name.starts_with("refs/notes/") || name == "refs/stash" {
                continue;
            }

            // Peel annotated tags to the underlying commit OID. References
            // that can't be resolved (e.g. broken symrefs) are silently skipped.
            let commit_oid = match reference.peel_to_id() {
                Ok(id) => ObjectId(id.to_string()),
                Err(_) => continue,
            };

            let kind = if name.starts_with("refs/tags/") {
                RefKind::Tag
            } else if name.starts_with("refs/remotes/") || name.starts_with("refs/heads/") {
                RefKind::Branch
            } else {
                RefKind::Other
            };

            result.push(GitRef {
                name,
                commit: commit_oid,
                kind,
            });
        }

        Ok(result)
    }

    /// List all reachable commits in commit-time order (newest first).
    ///
    /// Equivalent to `git rev-list --all --date-order`.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn list_revs(&self) -> Result<Vec<ObjectId>> {
        let repo = self.open_repo()?;

        // Collect all ref commit OIDs as walk starting points.
        let tips: Vec<gix::ObjectId> = repo
            .references()
            .map_err(gix_err)?
            .all()
            .map_err(gix_err)?
            .filter_map(|r| {
                let mut r = r.ok()?;
                r.peel_to_id().ok().map(|id| id.detach())
            })
            .collect();

        if tips.is_empty() {
            return Ok(vec![]);
        }

        let walk = repo
            .rev_walk(tips)
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                Default::default(),
            ))
            .all()
            .map_err(gix_err)?;

        let mut revs = Vec::new();
        for info in walk {
            let info = info.map_err(gix_err)?;
            revs.push(ObjectId(info.id.to_string()));
        }

        Ok(revs)
    }

    /// Read the raw bytes of a blob object by its hash.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn cat_file(&self, oid: &ObjectId) -> Result<Vec<u8>> {
        let repo = self.open_repo()?;
        let gix_oid = parse_oid(oid)?;
        let obj = repo
            .find_object(gix_oid)
            .map_err(|_| GitError::ObjectNotFound {
                oid: oid.to_string(),
            })?;
        Ok(obj.data.to_vec())
    }

    /// Read a blob object and interpret its content as UTF-8.
    pub fn cat_file_str(&self, oid: &ObjectId) -> Result<String> {
        let bytes = self.cat_file(oid)?;
        String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
            context: format!("object {}", oid),
        })
    }

    /// List the immediate entries of a tree at the given revision.
    ///
    /// - `rev`: a commit OID (or any revspec that resolves to a commit).
    /// - `path`: optional subdirectory path relative to the repo root.
    ///   Only immediate children are returned; the traversal is not recursive.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn list_files(&self, rev: &ObjectId, path: Option<&Path>) -> Result<Vec<TreeEntry>> {
        let repo = self.open_repo()?;
        let commit_gix_oid = parse_oid(rev)?;

        // Resolve to commit (peeling through any annotated tags).
        let commit_obj = repo
            .find_object(commit_gix_oid)
            .map_err(|_| GitError::ObjectNotFound {
                oid: rev.to_string(),
            })?
            .peel_tags_to_end()
            .map_err(gix_err)?;

        let commit = commit_obj
            .try_into_commit()
            .map_err(|_| GitError::Gix(format!("{} is not a commit", rev)))?;

        let tree_id = commit
            .tree_id()
            .map_err(gix_err)?
            .detach();

        // Navigate to the requested subdirectory if `path` is given.
        let target_tree_id = match path {
            None => tree_id,
            Some(p) => find_subtree(&repo, tree_id, p)?,
        };

        // Decode and iterate the target tree's immediate entries.
        let tree_obj = repo
            .find_object(target_tree_id)
            .map_err(|_| GitError::ObjectNotFound {
                oid: target_tree_id.to_string(),
            })?;
        let tree = tree_obj
            .try_into_tree()
            .map_err(|_| GitError::Gix("expected a tree object".into()))?;

        let decoded = tree.decode().map_err(gix_err)?;

        let mut entries = Vec::new();
        for entry in decoded.entries.iter() {
            let kind = entry_mode_to_kind(entry.mode);
            let mode_str = entry_mode_to_str(entry.mode);
            let oid = ObjectId(entry.oid.to_owned().to_string());
            let name = String::from_utf8_lossy(entry.filename);
            let file_path = PathBuf::from(name.as_ref());

            entries.push(TreeEntry {
                mode: mode_str,
                kind,
                oid,
                path: file_path,
            });
        }

        Ok(entries)
    }

    /// Resolve a revision expression (ref name, commit-ish, etc.) to an
    /// `ObjectId`.
    ///
    /// The expression is automatically suffixed with `^{commit}` to ensure the
    /// result is always a commit hash, peeling through tags if necessary.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn resolve(&self, expr: &str) -> Result<ObjectId> {
        let repo = self.open_repo()?;
        let spec = format!("{}^{{commit}}", expr);
        let id = repo
            .rev_parse_single(spec.as_str())
            .map_err(gix_err)?;
        Ok(ObjectId(id.to_string()))
    }

    /// Return the URL of a remote.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
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
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn parse_oid(oid: &ObjectId) -> Result<gix::ObjectId> {
    gix::ObjectId::from_hex(oid.as_str().as_bytes()).map_err(|_| GitError::ObjectNotFound {
        oid: oid.to_string(),
    })
}

/// Walk `path` components into the tree rooted at `tree_id`, returning the OID
/// of the sub-tree (or sub-blob) at that path.
fn find_subtree(
    repo: &gix::Repository,
    mut tree_id: gix::ObjectId,
    path: &Path,
) -> Result<gix::ObjectId> {
    for component in path.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| GitError::PathNotFound {
                path: path.display().to_string(),
            })?;

        let tree_obj = repo
            .find_object(tree_id)
            .map_err(|_| GitError::ObjectNotFound {
                oid: tree_id.to_string(),
            })?;
        let tree = tree_obj
            .try_into_tree()
            .map_err(|_| GitError::Gix("expected a tree object".into()))?;
        let decoded = tree.decode().map_err(gix_err)?;

        let entry = decoded
            .entries
            .iter()
            .find(|e| e.filename == name.as_bytes())
            .ok_or_else(|| GitError::PathNotFound {
                path: path.display().to_string(),
            })?;

        tree_id = entry.oid.to_owned();
    }
    Ok(tree_id)
}

fn entry_mode_to_kind(mode: gix::objs::tree::EntryMode) -> TreeEntryKind {
    use gix::object::tree::EntryKind;
    match mode.kind() {
        EntryKind::Blob => TreeEntryKind::Blob,
        EntryKind::BlobExecutable => TreeEntryKind::BlobExecutable,
        EntryKind::Tree => TreeEntryKind::Tree,
        EntryKind::Commit => TreeEntryKind::Commit,
        EntryKind::Link => TreeEntryKind::Symlink,
    }
}

fn entry_mode_to_str(mode: gix::objs::tree::EntryMode) -> String {
    use gix::object::tree::EntryKind;
    match mode.kind() {
        EntryKind::Blob => "100644".to_owned(),
        EntryKind::BlobExecutable => "100755".to_owned(),
        EntryKind::Tree => "040000".to_owned(),
        EntryKind::Commit => "160000".to_owned(),
        EntryKind::Link => "120000".to_owned(),
    }
}

// ── No-progress convenience wrappers ─────────────────────────────────────────

impl GitDatabase {
    /// Fetch all refs, reporting no progress.
    pub async fn fetch_silent(&self, remote: &str) -> Result<()> {
        self.fetch(remote, NoProgress).await
    }

    /// Fetch a specific ref, reporting no progress.
    pub async fn fetch_ref_silent(&self, remote: &str, refspec: &str) -> Result<()> {
        self.fetch_ref(remote, refspec, NoProgress).await
    }
}
