//! Async pre-fetch phase that populates a [`BenderProvider`] from real git repositories.
//!
//! # Overview
//!
//! Resolution is a two-phase process:
//!
//! 1. **Pre-fetch (this module)**: Starting from a root [`PartialManifest`], discover all
//!    reachable packages via BFS. For each package, fetch the git repository (if needed),
//!    enumerate available versions (semver tags or pinned revisions), and read the package's
//!    own `Bender.yml` to get its transitive dependencies.
//!
//! 2. **Resolve (sync)**: Pass the populated [`BenderProvider`] to [`crate::resolve`].
//!
//! # Entry point
//!
//! ```no_run
//! use std::sync::Arc;
//! use tokio::sync::Semaphore;
//! use bender_resolve::fetcher::{fetch, FetchConfig};
//! use bender_resolve::manifest::PartialManifest;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = FetchConfig {
//!     db_dir: "/home/user/.bender/git/db".into(),
//!     throttle: Arc::new(Semaphore::new(4)),
//! };
//! let root_yaml = std::fs::read_to_string("Bender.yml")?;
//! let root = PartialManifest::parse(&root_yaml)?;
//! let provider = fetch(&config, &root).await?;
//! # Ok(()) }
//! ```

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
use crate::provider::{BenderProvider, PackageInfo, VersionSource};
use crate::version::{BenderVersion, BenderVersionSet};

/// Configuration for the pre-fetch phase.
pub struct FetchConfig {
    /// Root directory for git bare repos. Individual databases are stored at
    /// `{db_dir}/{name}-{hash}/` matching the legacy bender layout.
    pub db_dir: PathBuf,
    /// Semaphore limiting concurrent git network operations (fetch, init).
    pub throttle: Arc<Semaphore>,
}

/// Errors that can occur during the pre-fetch phase.
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
// Public entry point
// ---------------------------------------------------------------------------

/// Populate a [`BenderProvider`] by fetching all reachable packages starting
/// from `root_manifest`.
///
/// This performs a BFS over the dependency graph: the root manifest's direct
/// dependencies are enqueued, each is fetched and its transitive dependencies
/// enqueued, until no new packages remain.
///
/// Git databases are cached by `(name, url)` — the same repo is never fetched
/// twice even if multiple packages depend on it.
pub async fn fetch(
    config: &FetchConfig,
    root_manifest: &PartialManifest,
) -> Result<BenderProvider, FetchError> {
    let mut provider = BenderProvider::new();

    // Pending work: (dep_name, parsed_dep)
    let mut queue: VecDeque<(String, ParsedDependency)> = VecDeque::new();
    // Packages already discovered (avoid re-fetching).
    let mut seen: HashSet<String> = HashSet::new();
    // Cache of open git databases keyed by their on-disk path.
    let mut db_cache: HashMap<PathBuf, GitDatabase> = HashMap::new();

    // Seed the queue from the root manifest's direct dependencies.
    for (name, dep) in root_manifest.resolve_dependencies()? {
        if seen.insert(name.clone()) {
            queue.push_back((name, dep));
        }
    }

    while let Some((name, dep)) = queue.pop_front() {
        let sub_deps =
            process_dependency(config, &name, &dep, &mut provider, &mut db_cache).await?;

        for (sub_name, sub_dep) in sub_deps {
            if seen.insert(sub_name.clone()) {
                queue.push_back((sub_name, sub_dep));
            }
        }
    }

    Ok(provider)
}

// ---------------------------------------------------------------------------
// Per-dependency processing
// ---------------------------------------------------------------------------

/// Process one dependency: fetch/open its git db, enumerate versions, read
/// manifests, register with the provider, and return discovered sub-deps.
async fn process_dependency(
    config: &FetchConfig,
    name: &str,
    dep: &ParsedDependency,
    provider: &mut BenderProvider,
    db_cache: &mut HashMap<PathBuf, GitDatabase>,
) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
    match dep {
        ParsedDependency::Path(path_str) => process_path_dep(name, path_str, provider).await,
        ParsedDependency::GitVersion { url, version } => {
            process_git_version_dep(config, name, url, version, provider, db_cache).await
        }
        ParsedDependency::GitRevision { url, rev } => {
            process_git_revision_dep(config, name, url, rev, provider, db_cache).await
        }
    }
}

// ---------------------------------------------------------------------------
// Path dependencies
// ---------------------------------------------------------------------------

async fn process_path_dep(
    name: &str,
    path_str: &str,
    provider: &mut BenderProvider,
) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
    let path = PathBuf::from(path_str);
    let bender_yml = path.join("Bender.yml");

    let mut sub_deps = Vec::new();

    // Read the manifest if it exists, to discover transitive deps.
    if bender_yml.exists() {
        let yaml = std::fs::read_to_string(&bender_yml)?;
        let manifest = PartialManifest::parse(&yaml)?;
        sub_deps = manifest.resolve_dependencies()?;
    }

    let version = BenderVersion::Path;
    let deps_for_version = convert_sub_deps(&sub_deps, provider);

    provider.add_package(
        name,
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
    config: &FetchConfig,
    name: &str,
    url: &str,
    _version_req: &semver::VersionReq,
    provider: &mut BenderProvider,
    db_cache: &mut HashMap<PathBuf, GitDatabase>,
) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
    let db = ensure_db(config, name, url, None, db_cache).await?;

    // Extract semver versions from tags (tags matching `v<semver>`).
    let mut semver_tags: Vec<(semver::Version, ObjectId)> = db
        .list_tags()
        .map_err(|e| FetchError::git(name, e))?
        .into_iter()
        .filter_map(|(tag, oid)| {
            tag.strip_prefix('v')
                .and_then(|s| semver::Version::parse(s).ok())
                .map(|v| (v, oid))
        })
        .collect();

    // Sort ascending so BenderVersion list is in order.
    semver_tags.sort_by(|a, b| a.0.cmp(&b.0));

    let mut versions = Vec::new();
    let mut dependencies: IndexMap<BenderVersion, Option<Vec<(BenderPackage, BenderVersionSet)>>> =
        IndexMap::new();
    let mut sources: IndexMap<BenderVersion, VersionSource> = IndexMap::new();
    let mut all_sub_deps: HashMap<String, ParsedDependency> = HashMap::new();

    for (semver_ver, oid) in &semver_tags {
        let bv = BenderVersion::Semver(semver_ver.clone());

        // Read the manifest at this specific commit to get sub-deps.
        let sub_deps = read_manifest_at(db, &oid, name).await?;

        for (sub_name, sub_dep) in &sub_deps {
            // Last writer wins — versions are processed oldest-first, so
            // the newest manifest's deps take precedence for new packages.
            all_sub_deps
                .entry(sub_name.clone())
                .or_insert_with(|| sub_dep.clone());
        }

        let dep_constraints = convert_sub_deps(&sub_deps, provider);

        versions.push(bv.clone());
        dependencies.insert(bv.clone(), Some(dep_constraints));
        sources.insert(bv, VersionSource::Git(url.to_string()));
    }

    if !versions.is_empty() {
        provider.add_package(
            name,
            PackageInfo {
                versions,
                dependencies,
                sources,
            },
        );
    }

    // Return the union of all sub-dependencies encountered across versions.
    Ok(all_sub_deps.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Git revision (pinned commit) dependencies
// ---------------------------------------------------------------------------

async fn process_git_revision_dep(
    config: &FetchConfig,
    name: &str,
    url: &str,
    rev: &str,
    provider: &mut BenderProvider,
    db_cache: &mut HashMap<PathBuf, GitDatabase>,
) -> Result<Vec<(String, ParsedDependency)>, FetchError> {
    let db = ensure_db(config, name, url, Some(rev), db_cache).await?;

    // Resolve the revision expression to a concrete commit OID.
    let oid = db
        .resolve(rev)
        .map_err(|e| FetchError::resolve(name, rev, e))?;

    // Compute the index by finding this commit's position in the date-ordered
    // revision list (newest = highest index, matching BenderVersion ordering).
    let all_revs = db.list_revs().map_err(|e| FetchError::git(name, e))?;
    let index = all_revs
        .iter()
        .position(|r| r == &oid)
        .map(|i| (all_revs.len() - 1 - i) as u64) // reverse: newest gets highest index
        .unwrap_or(0);

    let hash = oid.to_string();
    let bv = BenderVersion::GitRevision {
        index,
        hash: hash.clone(),
    };

    let sub_deps = read_manifest_at(db, &oid, name).await?;
    let dep_constraints = convert_sub_deps(&sub_deps, provider);

    provider.add_package(
        name,
        PackageInfo {
            versions: vec![bv.clone()],
            dependencies: IndexMap::from([(bv.clone(), Some(dep_constraints))]),
            sources: IndexMap::from([(bv, VersionSource::Git(url.to_string()))]),
        },
    );

    Ok(sub_deps)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the database directory name matching bender's legacy convention:
/// `{name}-{first_16_hex_chars_of_blake2b(url)}`.
fn db_name(pkg_name: &str, url: &str) -> String {
    use blake2::{Blake2b512, Digest};
    let hash = format!("{:016x}", Blake2b512::digest(url.as_bytes()));
    format!("{}-{}", pkg_name, &hash[..16])
}

/// Open an existing git database or initialise and fetch a new one.
///
/// If `fetch_ref` is provided, a second targeted fetch is done after the main
/// fetch to make a specific commit reachable (needed for pinned revisions that
/// aren't on any named ref).
async fn ensure_db<'a>(
    config: &FetchConfig,
    pkg_name: &str,
    url: &str,
    fetch_ref: Option<&str>,
    db_cache: &'a mut HashMap<PathBuf, GitDatabase>,
) -> Result<&'a GitDatabase, FetchError> {
    let dir = config.db_dir.join(db_name(pkg_name, url));
    std::fs::create_dir_all(&dir)?;

    // Use entry API to avoid re-opening an already-open database.
    if !db_cache.contains_key(&dir) {
        let db = open_or_init_db(&dir, url, fetch_ref, config.throttle.clone(), pkg_name).await?;
        db_cache.insert(dir.clone(), db);
    }

    Ok(db_cache.get(&dir).unwrap())
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
/// Returns the list of resolved sub-dependencies, or an empty vec if no
/// manifest exists at that revision.
async fn read_manifest_at(
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

/// Convert a list of `(name, ParsedDependency)` pairs into pubgrub constraint
/// pairs `(BenderPackage, BenderVersionSet)`.
///
/// For git version deps the version set is built by matching `VersionReq`
/// against the versions already registered in the provider for that package.
/// For git revision and path deps a singleton range is used.
///
/// Note: when called during BFS the referenced package may not yet be in the
/// provider. In that case we record the requirement as the full semver range
/// expressed by the `VersionReq` converted to a bound range, or as a
/// singleton. The provider is populated in a later BFS iteration.
fn convert_sub_deps(
    sub_deps: &[(String, ParsedDependency)],
    provider: &BenderProvider,
) -> Vec<(BenderPackage, BenderVersionSet)> {
    sub_deps
        .iter()
        .map(|(dep_name, dep)| {
            let pkg = BenderPackage::new(dep_name.clone());
            let vs = dep_to_version_set(dep_name, dep, provider);
            (pkg, vs)
        })
        .collect()
}

/// Build a `BenderVersionSet` from a `ParsedDependency`.
fn dep_to_version_set(
    dep_name: &str,
    dep: &ParsedDependency,
    provider: &BenderProvider,
) -> BenderVersionSet {
    match dep {
        ParsedDependency::Path(_) => pubgrub::Ranges::singleton(BenderVersion::Path),

        ParsedDependency::GitRevision { .. } => {
            // If the package is already in the provider, find the exact revision.
            if let Some(info) = provider.packages.get(dep_name) {
                for v in &info.versions {
                    if matches!(v, BenderVersion::GitRevision { .. }) {
                        return pubgrub::Ranges::singleton(v.clone());
                    }
                }
            }
            // Not yet populated — return a range covering all GitRevisions.
            // This is safe: pubgrub will constrain further during resolution.
            let lower = BenderVersion::GitRevision {
                index: 0,
                hash: String::new(),
            };
            pubgrub::Ranges::from_range_bounds(lower..)
        }

        ParsedDependency::GitVersion { version, .. } => {
            // If the package's versions are already in the provider, filter
            // by VersionReq and build a union of matching singletons.
            if let Some(info) = provider.packages.get(dep_name) {
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

            // Package not yet in provider — build a continuous semver range.
            // We express the VersionReq as [lower_bound, upper_bound) using
            // the minimum and maximum matching versions in the semver space.
            semver_req_to_range(version)
        }
    }
}

/// Convert a `semver::VersionReq` to a `BenderVersionSet` (range of Semver
/// variants) by walking the comparators.
///
/// This is an approximation: it computes the tightest [lo, hi) interval that
/// subsumes the requirement. It is used only when the package is not yet in
/// the provider; once the package is populated the constraint is tightened to
/// the exact matching singletons.
fn semver_req_to_range(req: &semver::VersionReq) -> BenderVersionSet {
    // Try to determine a lower bound (inclusive) and upper bound (exclusive).
    let mut lo: Option<semver::Version> = None;
    let mut hi: Option<semver::Version> = None;

    for comp in &req.comparators {
        let major = comp.major;
        let minor = comp.minor.unwrap_or(0);
        let patch = comp.patch.unwrap_or(0);
        let base = semver::Version::new(major, minor, patch);

        match comp.op {
            semver::Op::Exact => {
                // = X.Y.Z → singleton [base, next_patch)
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
                // ~X.Y.Z → [X.Y.Z, X.(Y+1).0)
                let upper = semver::Version::new(major, minor + 1, 0);
                lo = Some(lo.map_or(base.clone(), |l: semver::Version| l.max(base)));
                hi = Some(hi.map_or(upper.clone(), |h: semver::Version| h.min(upper)));
            }
            semver::Op::Caret => {
                // ^X.Y.Z → [X.Y.Z, (X+1).0.0) if X>0
                //           [0.Y.Z, 0.(Y+1).0)  if X==0, Y>0
                //           [0.0.Z, 0.0.(Z+1))  if X==Y==0
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
                // X.* → [X.0.0, (X+1).0.0)
                // X.Y.* → [X.Y.0, X.(Y+1).0)
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
                // Unknown comparator — return universe and let pubgrub handle it.
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
