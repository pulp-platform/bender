//! Async dependency fetching with integrated caching.
//!
//! [`DependencyFetcher`] handles fetching git repositories, reading manifests,
//! and caching the results. It is embedded in [`crate::provider::BenderProvider`]
//! and populated via [`DependencyFetcher::fetch_all`] before resolution begins.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bender_git::database::GitDatabase;
use bender_git::progress::NoProgress;
use bender_git::types::ObjectId;
use indexmap::IndexMap;
use tokio::sync::Semaphore;

use crate::manifest::{ManifestError, ParsedDependency, PartialManifest};
use crate::package::BenderPackage;
use crate::version::{BenderVersion, BenderVersionSet};

/// Metadata about the source of a specific version, used after resolution
/// to determine how to check out the dependency.
#[derive(Clone, Debug)]
pub enum VersionSource {
    /// A local path dependency.
    Path(std::path::PathBuf),
    /// A git dependency from the given URL.
    Git(String),
}

/// Information about a package's available versions and their dependencies.
#[derive(Clone, Debug)]
pub struct PackageInfo {
    /// All available versions for this package, sorted.
    pub versions: Vec<BenderVersion>,
    /// Dependencies for each version. `None` means the manifest hasn't been loaded yet.
    pub dependencies: IndexMap<BenderVersion, Option<Vec<(BenderPackage, BenderVersionSet)>>>,
    /// Source metadata for each version (used for checkout after resolution).
    pub sources: IndexMap<BenderVersion, VersionSource>,
}

/// Configuration for the fetch phase.
pub struct FetchConfig {
    /// Root directory for git bare repos. Individual databases are stored at
    /// `{db_dir}/{name}-{hash}/` matching the legacy bender layout.
    pub db_dir: PathBuf,
    /// Semaphore limiting concurrent git network operations (fetch, init).
    pub throttle: Arc<Semaphore>,
}

/// Errors that can occur during fetching.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("manifest parse error: {0}")]
    Manifest(#[from] ManifestError),

    #[error("git error for package `{pkg}`: {source}")]
    Git {
        pkg: String,
        source: bender_git::error::GitError,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to resolve revision `{rev}` for package `{pkg}`: {source}")]
    Resolve {
        pkg: String,
        rev: String,
        source: bender_git::error::GitError,
    },
}

impl FetchError {
    fn git(pkg: impl Into<String>, source: bender_git::error::GitError) -> Self {
        FetchError::Git {
            pkg: pkg.into(),
            source,
        }
    }

    fn resolve(
        pkg: impl Into<String>,
        rev: impl Into<String>,
        source: bender_git::error::GitError,
    ) -> Self {
        FetchError::Resolve {
            pkg: pkg.into(),
            rev: rev.into(),
            source,
        }
    }
}

// ---------------------------------------------------------------------------
// DependencyFetcher
// ---------------------------------------------------------------------------

/// Fetches and caches package metadata from git repositories.
///
/// Populated via [`DependencyFetcher::fetch_all`] before resolution begins,
/// then queried by [`crate::provider::BenderProvider`] during resolution.
///
/// Because no [`FetchConfig`] is stored on the struct itself, a
/// `DependencyFetcher` can be constructed and pre-populated with
/// [`add_package`](Self::add_package) without any git configuration — which
/// is useful in tests.
pub struct DependencyFetcher {
    /// Open git databases keyed by their on-disk path.
    db_cache: HashMap<PathBuf, GitDatabase>,
    /// All pre-fetched package metadata, keyed by package name.
    pub packages: IndexMap<String, PackageInfo>,
}

impl DependencyFetcher {
    pub fn new() -> Self {
        DependencyFetcher {
            db_cache: HashMap::new(),
            packages: IndexMap::new(),
        }
    }

    /// Register a package directly, bypassing git fetching.
    ///
    /// Useful for pre-populating the cache in tests or for inserting root
    /// package metadata.
    pub fn add_package(&mut self, name: impl Into<String>, info: PackageInfo) {
        self.packages.insert(name.into(), info);
    }

    /// Look up a package's metadata by name.
    pub fn get_package(&self, name: &str) -> Option<&PackageInfo> {
        self.packages.get(name)
    }

    /// Populate the cache by BFS-ing from `root_manifest`.
    ///
    /// Starting from the root manifest's direct dependencies, fetches each
    /// package's git repository (or reads from the local path), enumerates
    /// available versions, reads transitive manifests, and registers everything
    /// in the internal package cache.
    pub async fn fetch_all(
        &mut self,
        config: &FetchConfig,
        root_manifest: &PartialManifest,
    ) -> Result<(), FetchError> {
        let mut queue: VecDeque<(String, ParsedDependency)> = VecDeque::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (name, dep) in root_manifest.resolve_dependencies()? {
            if seen.insert(name.clone()) {
                queue.push_back((name, dep));
            }
        }

        while let Some((name, dep)) = queue.pop_front() {
            let sub_deps = self.process_dependency(config, &name, &dep).await?;
            for (sub_name, sub_dep) in sub_deps {
                if seen.insert(sub_name.clone()) {
                    queue.push_back((sub_name, sub_dep));
                }
            }
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // Per-dependency processing
    // ---------------------------------------------------------------------------

    async fn process_dependency(
        &mut self,
        config: &FetchConfig,
        name: &str,
        dep: &ParsedDependency,
    ) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
        match dep {
            ParsedDependency::Path(path_str) => self.process_path_dep(name, path_str).await,
            ParsedDependency::GitVersion { url, version } => {
                self.process_git_version_dep(config, name, url, version)
                    .await
            }
            ParsedDependency::GitRevision { url, rev } => {
                self.process_git_revision_dep(config, name, url, rev).await
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Path dependencies
    // ---------------------------------------------------------------------------

    async fn process_path_dep(
        &mut self,
        name: &str,
        path_str: &str,
    ) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
        let path = PathBuf::from(path_str);
        let bender_yml = path.join("Bender.yml");

        let sub_deps = if bender_yml.exists() {
            let yaml = std::fs::read_to_string(&bender_yml)?;
            PartialManifest::parse(&yaml)?.resolve_dependencies()?
        } else {
            Vec::new()
        };

        let version = BenderVersion::Path;
        let deps_for_version = self.convert_sub_deps(&sub_deps);

        self.packages.insert(
            name.to_string(),
            PackageInfo {
                versions: vec![version.clone()],
                dependencies: IndexMap::from([(version.clone(), Some(deps_for_version))]),
                sources: IndexMap::from([(version, VersionSource::Path(path))]),
            },
        );

        Ok(sub_deps)
    }

    // ---------------------------------------------------------------------------
    // Git version (semver tag) dependencies
    // ---------------------------------------------------------------------------

    async fn process_git_version_dep(
        &mut self,
        config: &FetchConfig,
        name: &str,
        url: &str,
        _version_req: &semver::VersionReq,
    ) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
        // Step 1: ensure db is fetched. Returns the cache key path so we don't
        // hold a borrow across the async boundary.
        let dir = self.ensure_db_dir(config, name, url, None).await?;

        // Step 2: list semver tags (sync, scoped borrow of db_cache).
        let mut semver_tags: Vec<(semver::Version, ObjectId)> = {
            let db = self.db_cache.get(&dir).unwrap();
            db.list_tags()
                .map_err(|e| FetchError::git(name, e))?
                .into_iter()
                .filter_map(|(tag, oid)| {
                    tag.strip_prefix('v')
                        .and_then(|s| semver::Version::parse(s).ok())
                        .map(|v| (v, oid))
                })
                .collect()
        }; // db borrow released

        semver_tags.sort_by(|a, b| a.0.cmp(&b.0));

        // Step 3: for each version, read manifest and build constraints.
        let mut versions = Vec::new();
        let mut dependencies: IndexMap<
            BenderVersion,
            Option<Vec<(BenderPackage, BenderVersionSet)>>,
        > = IndexMap::new();
        let mut sources: IndexMap<BenderVersion, VersionSource> = IndexMap::new();
        let mut all_sub_deps: HashMap<String, ParsedDependency> = HashMap::new();

        for (semver_ver, oid) in &semver_tags {
            // Read manifest (sync, scoped borrow of db_cache).
            let sub_deps: Vec<(String, ParsedDependency)> = {
                let db = self.db_cache.get(&dir).unwrap();
                read_manifest_at(db, oid, name)?
            }; // db borrow released

            for (sub_name, sub_dep) in &sub_deps {
                // Last writer wins — versions are processed oldest-first, so
                // the newest manifest's deps take precedence for new packages.
                all_sub_deps
                    .entry(sub_name.clone())
                    .or_insert_with(|| sub_dep.clone());
            }

            let dep_constraints = self.convert_sub_deps(&sub_deps);
            let bv = BenderVersion::Semver(semver_ver.clone());

            versions.push(bv.clone());
            dependencies.insert(bv.clone(), Some(dep_constraints));
            sources.insert(bv, VersionSource::Git(url.to_string()));
        }

        // Step 4: register package (mutates self.packages).
        if !versions.is_empty() {
            self.packages.insert(
                name.to_string(),
                PackageInfo {
                    versions,
                    dependencies,
                    sources,
                },
            );
        }

        Ok(all_sub_deps.into_iter().collect())
    }

    // ---------------------------------------------------------------------------
    // Git revision (pinned commit) dependencies
    // ---------------------------------------------------------------------------

    async fn process_git_revision_dep(
        &mut self,
        config: &FetchConfig,
        name: &str,
        url: &str,
        rev: &str,
    ) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
        // Step 1: ensure db is fetched.
        let dir = self.ensure_db_dir(config, name, url, Some(rev)).await?;

        // Step 2: resolve OID and compute chronological index (sync, scoped borrow).
        let (oid, index) = {
            let db = self.db_cache.get(&dir).unwrap();
            let oid = db
                .resolve(rev)
                .map_err(|e| FetchError::resolve(name, rev, e))?;
            let all_revs = db.list_revs().map_err(|e| FetchError::git(name, e))?;
            let index = all_revs
                .iter()
                .position(|r| r == &oid)
                .map(|i| (all_revs.len() - 1 - i) as u64)
                .unwrap_or(0);
            (oid, index)
        }; // db borrow released

        let bv = BenderVersion::GitRevision {
            index,
            hash: oid.to_string(),
        };

        // Step 3: read manifest (sync, scoped borrow).
        let sub_deps: Vec<(String, ParsedDependency)> = {
            let db = self.db_cache.get(&dir).unwrap();
            read_manifest_at(db, &oid, name)?
        }; // db borrow released

        let dep_constraints = self.convert_sub_deps(&sub_deps);

        // Step 4: register package.
        self.packages.insert(
            name.to_string(),
            PackageInfo {
                versions: vec![bv.clone()],
                dependencies: IndexMap::from([(bv.clone(), Some(dep_constraints))]),
                sources: IndexMap::from([(bv, VersionSource::Git(url.to_string()))]),
            },
        );

        Ok(sub_deps)
    }

    // ---------------------------------------------------------------------------
    // Database helpers
    // ---------------------------------------------------------------------------

    /// Ensure the git database for `(pkg_name, url)` exists and is up-to-date.
    ///
    /// Returns the on-disk directory path used as the cache key. Returning a
    /// path instead of a `&GitDatabase` avoids holding a borrow on `self`
    /// across subsequent operations.
    async fn ensure_db_dir(
        &mut self,
        config: &FetchConfig,
        pkg_name: &str,
        url: &str,
        fetch_ref: Option<&str>,
    ) -> Result<PathBuf, FetchError> {
        let dir = config.db_dir.join(db_name(pkg_name, url));
        std::fs::create_dir_all(&dir)?;

        if !self.db_cache.contains_key(&dir) {
            let db =
                open_or_init_db(&dir, url, fetch_ref, config.throttle.clone(), pkg_name).await?;
            self.db_cache.insert(dir.clone(), db);
        }

        Ok(dir)
    }

    // ---------------------------------------------------------------------------
    // Constraint conversion
    // ---------------------------------------------------------------------------

    /// Convert a list of `(name, ParsedDependency)` pairs into pubgrub constraint
    /// pairs `(BenderPackage, BenderVersionSet)`.
    fn convert_sub_deps(
        &self,
        sub_deps: &[(String, ParsedDependency)],
    ) -> Vec<(BenderPackage, BenderVersionSet)> {
        sub_deps
            .iter()
            .map(|(dep_name, dep)| {
                let pkg = BenderPackage::new(dep_name.clone());
                let vs = self.dep_to_version_set(dep_name, dep);
                (pkg, vs)
            })
            .collect()
    }

    /// Build a `BenderVersionSet` from a `ParsedDependency`, consulting the
    /// already-populated package cache where available.
    fn dep_to_version_set(&self, dep_name: &str, dep: &ParsedDependency) -> BenderVersionSet {
        match dep {
            ParsedDependency::Path(_) => pubgrub::Ranges::singleton(BenderVersion::Path),

            ParsedDependency::GitRevision { .. } => {
                if let Some(info) = self.packages.get(dep_name) {
                    for v in &info.versions {
                        if matches!(v, BenderVersion::GitRevision { .. }) {
                            return pubgrub::Ranges::singleton(v.clone());
                        }
                    }
                }
                // Not yet populated — return a range covering all GitRevisions.
                let lower = BenderVersion::GitRevision {
                    index: 0,
                    hash: String::new(),
                };
                pubgrub::Ranges::from_range_bounds(lower..)
            }

            ParsedDependency::GitVersion { version, .. } => {
                if let Some(info) = self.packages.get(dep_name) {
                    let matching: Vec<BenderVersion> = info
                        .versions
                        .iter()
                        .filter(|v| {
                            if let BenderVersion::Semver(sv) = v {
                                version.matches(sv)
                            } else {
                                false
                            }
                        })
                        .cloned()
                        .collect();

                    if !matching.is_empty() {
                        return matching
                            .into_iter()
                            .fold(pubgrub::Ranges::empty(), |acc, v| {
                                acc.union(&pubgrub::Ranges::singleton(v))
                            });
                    }
                }

                // Package not yet in cache — build a continuous semver range.
                semver_req_to_range(version)
            }
        }
    }
}

impl Default for DependencyFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Compute the database directory name matching bender's legacy convention:
/// `{name}-{first_16_hex_chars_of_blake2b(url)}`.
fn db_name(pkg_name: &str, url: &str) -> String {
    use blake2::{Blake2b512, Digest};
    let hash = format!("{:016x}", Blake2b512::digest(url.as_bytes()));
    format!("{}-{}", pkg_name, &hash[..16])
}

async fn open_or_init_db(
    dir: &Path,
    url: &str,
    fetch_ref: Option<&str>,
    throttle: Arc<Semaphore>,
    pkg_name: &str,
) -> Result<GitDatabase, FetchError> {
    let config_file = dir.join("config");

    if config_file.exists() {
        // Already initialised — just fetch updates.
        let db =
            GitDatabase::open(dir, throttle.clone()).map_err(|e| FetchError::git(pkg_name, e))?;
        db.fetch("origin", NoProgress)
            .await
            .map_err(|e| FetchError::git(pkg_name, e))?;
        if let Some(r) = fetch_ref {
            db.fetch_ref("origin", r, NoProgress)
                .await
                .map_err(|e| FetchError::git(pkg_name, e))?;
        }
        Ok(db)
    } else {
        // First time — init bare, add remote, fetch.
        let db = GitDatabase::init_bare(dir, throttle.clone())
            .map_err(|e| FetchError::git(pkg_name, e))?;
        db.add_remote("origin", url)
            .await
            .map_err(|e| FetchError::git(pkg_name, e))?;
        db.fetch("origin", NoProgress)
            .await
            .map_err(|e| FetchError::git(pkg_name, e))?;
        if let Some(r) = fetch_ref {
            db.fetch_ref("origin", r, NoProgress)
                .await
                .map_err(|e| FetchError::git(pkg_name, e))?;
        }
        Ok(db)
    }
}

/// Read and parse `Bender.yml` from a specific commit in a git database.
/// Returns resolved sub-dependencies, or an empty vec if no manifest exists.
fn read_manifest_at(
    db: &GitDatabase,
    oid: &ObjectId,
    pkg_name: &str,
) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
    let content = db
        .read_file(oid, Path::new("Bender.yml"))
        .map_err(|e| FetchError::git(pkg_name, e))?;

    let Some(yaml) = content else {
        return Ok(Vec::new());
    };

    let manifest = PartialManifest::parse(&yaml)?;
    Ok(manifest.resolve_dependencies()?)
}

/// Convert a `semver::VersionReq` to a `BenderVersionSet` (range of Semver
/// variants) by walking the comparators.
///
/// This is an approximation used only when the package is not yet in the cache;
/// once the package is populated the constraint is tightened to exact singletons.
fn semver_req_to_range(req: &semver::VersionReq) -> BenderVersionSet {
    let mut lo: Option<semver::Version> = None;
    let mut hi: Option<semver::Version> = None;

    for comp in &req.comparators {
        let major = comp.major;
        let minor = comp.minor.unwrap_or(0);
        let patch = comp.patch.unwrap_or(0);
        let base = semver::Version::new(major, minor, patch);

        match comp.op {
            semver::Op::Exact => {
                let next = semver::Version::new(major, minor, patch + 1);
                lo = Some(lo.map_or(base.clone(), |l: semver::Version| l.max(base.clone())));
                hi = Some(hi.map_or(next.clone(), |h: semver::Version| h.min(next)));
            }
            semver::Op::GreaterEq => {
                lo = Some(lo.map_or(base.clone(), |l: semver::Version| l.max(base)));
            }
            semver::Op::Greater => {
                let next = semver::Version::new(major, minor, patch + 1);
                lo = Some(lo.map_or(next.clone(), |l: semver::Version| l.max(next)));
            }
            semver::Op::Less => {
                hi = Some(hi.map_or(base.clone(), |h: semver::Version| h.min(base)));
            }
            semver::Op::LessEq => {
                let next = semver::Version::new(major, minor, patch + 1);
                hi = Some(hi.map_or(next.clone(), |h: semver::Version| h.min(next)));
            }
            semver::Op::Tilde => {
                let upper = semver::Version::new(major, minor + 1, 0);
                lo = Some(lo.map_or(base.clone(), |l: semver::Version| l.max(base)));
                hi = Some(hi.map_or(upper.clone(), |h: semver::Version| h.min(upper)));
            }
            semver::Op::Caret => {
                let upper = if major > 0 {
                    semver::Version::new(major + 1, 0, 0)
                } else if minor > 0 {
                    semver::Version::new(0, minor + 1, 0)
                } else {
                    semver::Version::new(0, 0, patch + 1)
                };
                lo = Some(lo.map_or(base.clone(), |l: semver::Version| l.max(base)));
                hi = Some(hi.map_or(upper.clone(), |h: semver::Version| h.min(upper)));
            }
            semver::Op::Wildcard => {
                let lower = semver::Version::new(major, minor, 0);
                let upper = if comp.minor.is_some() {
                    semver::Version::new(major, minor + 1, 0)
                } else {
                    semver::Version::new(major + 1, 0, 0)
                };
                lo = Some(lo.map_or(lower.clone(), |l: semver::Version| l.max(lower)));
                hi = Some(hi.map_or(upper.clone(), |h: semver::Version| h.min(upper)));
            }
            _ => {
                return pubgrub::Ranges::full();
            }
        }
    }

    match (lo, hi) {
        (Some(l), Some(h)) => {
            pubgrub::Ranges::from_range_bounds(BenderVersion::Semver(l)..BenderVersion::Semver(h))
        }
        (Some(l), None) => pubgrub::Ranges::from_range_bounds(BenderVersion::Semver(l)..),
        (None, Some(h)) => pubgrub::Ranges::from_range_bounds(..BenderVersion::Semver(h)),
        (None, None) => pubgrub::Ranges::full(),
    }
}
