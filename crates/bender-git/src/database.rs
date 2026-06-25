use std::path::{Path, PathBuf};

use crate::checkout::GitCheckout;
use crate::error::{GitError, Result};
use crate::lock::DatabaseLock;
use crate::progress::{GitProgress, on_fetch_progress};
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
///   `fetch`, `fetch_ref`.
///   Network I/O requiring the system `git` binary.
///
/// - **`gix` operations** (synchronous, no semaphore):
///   `add_remote`, `tag_commit`, `list_refs`, `list_revs`, `cat_file`,
///   `list_files`, `resolve`, `remote_url`.
///   All local reads and writes — fast and safe to run concurrently.
///
/// ## Concurrency across invocations
///
/// A single database directory may be shared by several bender invocations
/// (e.g. parallel CI jobs). Operations that mutate shared, non-content-addressed
/// state — `open_or_init` (init + `add_remote`) and `fetch`/`fetch_ref` (ref and
/// `packed-refs` updates) — hold an exclusive cross-process advisory lock for
/// their duration (see [`crate::lock`]). This guarantees the database is
/// initialised exactly once and serialises fetches so they neither corrupt refs
/// nor do redundant network work. Pure reads and `clone_into` do not lock: the
/// object store tolerates concurrent readers, and automatic gc — the only
/// operation that could prune objects a `--shared` checkout still needs — is
/// disabled at init time via [`open_or_init`](Self::open_or_init).
#[derive(Clone)]
pub struct GitDatabase {
    repo: gix::Repository,
}

impl GitDatabase {
    /// Initialise a new bare repository at `path` and return a handle to it.
    ///
    /// Equivalent to `git init --bare`. The directory must already exist.
    pub fn init_bare(path: impl Into<PathBuf>) -> Result<Self> {
        let repo = gix::init_bare(path.into())?;
        Ok(Self { repo })
    }

    /// Open an existing bare repository at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let repo = gix::open(path)?;
        Ok(Self { repo })
    }

    /// Open the database at `path`, creating and initialising it on first use.
    ///
    /// This is the recommended constructor when the same directory may be
    /// accessed by multiple concurrent bender invocations (e.g. parallel CI
    /// jobs sharing a cache). It holds an exclusive cross-process lock for its
    /// duration so initialisation happens exactly once: a second invocation
    /// either waits and then opens the finished database, or observes that it
    /// is already initialised.
    ///
    /// On first creation it runs the equivalent of `git init --bare`, installs
    /// `remote` pointing at `url`, and disables automatic gc (so a later
    /// `git fetch` cannot trigger a gc that prunes objects a `--shared`
    /// checkout still references). The directory must already exist.
    pub fn open_or_init(path: impl Into<PathBuf>, remote: &str, url: &str) -> Result<Self> {
        let path = path.into();
        let _lock = DatabaseLock::acquire_blocking(&path)?;

        // Already initialised by us or a concurrent invocation: just open it.
        if let Ok(repo) = gix::open(&path) {
            return Ok(Self { repo });
        }

        let db = Self {
            repo: gix::init_bare(&path)?,
        };
        db.add_remote(remote, url)?;
        db.disable_auto_gc()?;
        Ok(db)
    }

    fn runner(&self) -> Result<SubprocessRunner> {
        SubprocessRunner::new(self.repo.path().to_path_buf())
    }

    /// Atomically replace the repository-local `config` file with `config`.
    ///
    /// Writes to a sibling temp file and renames it over `config`, so a
    /// concurrent reader (e.g. a `git fetch` subprocess) never observes a
    /// truncated or half-written config. This mirrors how git itself edits
    /// config via `config.lock` + rename.
    fn write_config_atomic(config_path: &Path, config: &gix::config::File<'_>) -> Result<()> {
        let mut tmp_name = config_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(".bender-new");
        let tmp_path = config_path.with_file_name(tmp_name);

        let mut tmp = std::fs::File::create(&tmp_path)?;
        config.write_to(&mut tmp)?;
        tmp.sync_all()?;
        drop(tmp);

        std::fs::rename(&tmp_path, config_path)?;
        Ok(())
    }

    /// Disable git's automatic gc on this database.
    ///
    /// bender's checkouts are created with `--shared` and keep no objects of
    /// their own; they reach into this database's object store. An auto-gc
    /// triggered as a side effect of `git fetch` could prune objects a checkout
    /// still references. gc is therefore disabled here and only ever run
    /// explicitly (under the database lock).
    fn disable_auto_gc(&self) -> Result<()> {
        let config_path = self.repo.path().join("config");
        let mut config = gix::config::File::from_path_no_includes(
            config_path.clone(),
            gix::config::Source::Local,
        )?;
        config.set_raw_value("gc.auto", "0")?;
        Self::write_config_atomic(&config_path, &config)?;
        Ok(())
    }

    /// Add a remote (e.g. `origin`).
    ///
    /// Equivalent to `git remote add <name> <url>`.
    ///
    /// This persists the remote to the repository-local `config` file using
    /// `gix` only, including the default fetch refspec Git would install. The
    /// config is replaced atomically (write-temp-then-rename), so a concurrent
    /// reader never sees a torn file.
    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        let refspec = format!("+refs/heads/*:refs/remotes/{name}/*");
        let mut remote = self
            .repo
            .remote_at(url)?
            .with_refspecs(Some(refspec.as_str()), gix::remote::Direction::Fetch)?;

        let config_path = self.repo.path().join("config");
        let mut config = gix::config::File::from_path_no_includes(
            config_path.clone(),
            gix::config::Source::Local,
        )?;
        remote.save_as_to(name, &mut config)?;
        Self::write_config_atomic(&config_path, &config)?;
        Ok(())
    }

    /// Fetch all tags and branches from `remote`.
    ///
    /// Equivalent to `git fetch --tags --prune <remote> --progress`.
    ///
    /// Holds the exclusive database lock for its duration so that concurrent
    /// invocations sharing this database do not collide on `packed-refs` (which
    /// surfaces as spurious "cannot lock ref" failures) or do redundant network
    /// work — a contending fetch waits, then finds the data already present.
    pub async fn fetch(&self, remote: &str, mut progress: impl GitProgress) -> Result<()> {
        let _lock = DatabaseLock::acquire(self.repo.path()).await?;
        let label = self.repo.path().to_str().unwrap_or(remote);
        progress.started(label);
        let result = self
            .runner()?
            .run_drain(
                &["fetch", "--tags", "--prune", remote, "--progress"],
                &[("GIT_TERMINAL_PROMPT", "0")],
                on_fetch_progress(&mut progress),
            )
            .await;
        progress.finished();
        result
    }

    /// Fetch a specific ref from `remote`.
    ///
    /// Useful when a pinned commit hash is not reachable from any named ref
    /// (e.g. after a force-push), in which case the full OID must be fetched
    /// explicitly.
    pub async fn fetch_ref(&self, remote: &str, refspec: &str) -> Result<()> {
        let _lock = DatabaseLock::acquire(self.repo.path()).await?;
        self.runner()?
            .run_discard(&["fetch", remote, refspec], &[("GIT_TERMINAL_PROMPT", "0")])
            .await
    }

    /// Clone this database into `path` and check out `branch_or_tag`, returning
    /// a [`GitCheckout`] handle to the new working tree.
    ///
    /// `branch_or_tag` must be a named ref (branch or tag), not a bare commit
    /// hash, because `git clone --branch` does not accept commit hashes. The
    /// typical caller workflow:
    ///
    /// ```no_run
    /// # use bender_git::database::GitDatabase;
    /// # use bender_git::progress::NoProgress;
    /// # #[tokio::main] async fn main() -> bender_git::error::Result<()> {
    /// # let db = GitDatabase::init_bare("/tmp/db")?;
    /// # let rev = db.resolve("HEAD")?;
    /// # let checkout_path = std::path::PathBuf::from("/tmp/checkout");
    /// let tag = format!("bender-tmp-{}", rev.short(8));
    /// db.tag_commit(&tag, &rev)?;
    /// let checkout = db.clone_into(&checkout_path, &tag).await?;
    /// # Ok(()) }
    /// ```
    pub async fn clone_into(
        &self,
        path: impl Into<PathBuf>,
        branch_or_tag: &str,
    ) -> Result<GitCheckout> {
        let path = path.into();
        let db_path = self.repo.path().to_str().unwrap();
        let checkout_path = path.to_str().unwrap();

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
        self.runner()?
            .run_discard(
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
            .await?;

        GitCheckout::open(path)
    }

    /// Create or overwrite a local tag pointing to `commit`.
    ///
    /// Bender uses this to create short-lived `bender-tmp-<rev>` tags so that
    /// `git clone --branch` can check out an arbitrary commit (since `--branch`
    /// only accepts named refs, not bare hashes).
    pub fn tag_commit(&self, tag_name: &str, commit: &ObjectId) -> Result<()> {
        use gix::refs::transaction::PreviousValue;
        self.repo
            .tag_reference(tag_name, *commit, PreviousValue::Any)?;
        Ok(())
    }

    /// List all tags, returning `(short_name, commit_oid)` pairs.
    ///
    /// Annotated tags are peeled to their target commit. Broken symrefs are
    /// silently skipped.
    pub fn list_tags(&self) -> Result<Vec<(String, ObjectId)>> {
        let mut result = Vec::new();
        for reference in self.repo.references()?.tags()? {
            let mut reference = reference?;
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
        let mut branches = Vec::new();
        for reference in self.repo.references()?.remote_branches()? {
            let mut reference = reference?;
            let Ok(id) = reference.peel_to_id() else {
                continue;
            };
            let name = reference.name().as_bstr().to_string();
            let short = name
                .strip_prefix("refs/remotes/origin/")
                .unwrap_or(&name)
                .to_owned();
            branches.push((short, ObjectId::from(id.detach())));
        }
        Ok(branches)
    }

    /// List all reachable commits in commit-time order (newest first).
    ///
    /// Equivalent to `git rev-list --all --date-order`.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn list_revs(&self) -> Result<Vec<ObjectId>> {
        let tips: Vec<gix::ObjectId> = self
            .repo
            .references()?
            .all()?
            .filter_map(|r| r.ok()?.peel_to_id().ok().map(|id| id.detach()))
            .collect();

        self.repo
            .rev_walk(tips)
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                Default::default(),
            ))
            .all()?
            .map(|info| Ok(ObjectId::from(info?.id)))
            .collect()
    }

    /// Read the content of a file at `path` in the tree at `rev`.
    ///
    /// Returns `None` if the path does not exist in the tree.
    /// This is a pure local read and does not acquire the throttle semaphore.
    pub fn read_file(&self, rev: &ObjectId, path: &Path) -> Result<Option<String>> {
        let commit = self
            .repo
            .find_commit(*rev)
            .map_err(|_| GitError::ObjectNotFound {
                oid: rev.to_string(),
            })?;
        let tree = commit.tree()?;
        let Some(entry) = tree.lookup_entry_by_path(path)? else {
            return Ok(None);
        };
        let blob = entry.object()?;
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
        let spec = format!("{}^{{commit}}", expr);
        if let Ok(id) = self.repo.rev_parse_single(spec.as_str()) {
            return Ok(ObjectId::from(id.detach()));
        }
        // Fall back to remote-tracking ref — in a bare repo, `git fetch` stores
        // branches as refs/remotes/origin/<name> rather than refs/heads/<name>.
        let remote_spec = format!("refs/remotes/origin/{}^{{commit}}", expr);
        let id = self.repo.rev_parse_single(remote_spec.as_str())?;
        Ok(ObjectId::from(id.detach()))
    }

    /// Return the URL of a remote.
    ///
    /// This is a pure local read and does not acquire the throttle semaphore.
    ///
    /// Note: this re-opens the repository on each call rather than using the
    /// cached `repo` handle. `add_remote` writes the remote straight to the
    /// on-disk config, so the handle's config snapshot (taken at open time)
    /// would not include remotes added after construction.
    pub fn remote_url(&self, remote: &str) -> Result<String> {
        let repo = gix::open(self.repo.path())?;
        let remote = repo.find_remote(remote)?;
        let url = remote
            .url(gix::remote::Direction::Fetch)
            .ok_or(GitError::RefNotFound {
                refname: "fetch url".into(),
            })?;
        Ok(url.to_string())
    }
}
