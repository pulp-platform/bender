//! Async dependency fetching with integrated caching.
//!
//! [`DependencyFetcher`] handles fetching git repositories, reading manifests,
//! and caching the results. It is embedded in [`crate::provider::BenderProvider`]
//! and populated lazily during resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bender_git::database::GitDatabase;
use bender_git::progress::NoProgress;
use bender_git::types::ObjectId;
use indexmap::IndexMap;

pub use crate::error::FetchError;
use crate::manifest::{ParsedDependency, PartialManifest};
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
    pub dependencies: IndexMap<BenderVersion, Option<Vec<(String, BenderVersionSet)>>>,
    /// Source metadata for each version (used for checkout after resolution).
    pub sources: IndexMap<BenderVersion, VersionSource>,
}

impl PackageInfo {
    fn from_versions(entries: Vec<(BenderVersion, VersionSource)>) -> Self {
        let versions = entries.iter().map(|(v, _)| v.clone()).collect();
        let dependencies = entries.iter().map(|(v, _)| (v.clone(), None)).collect();
        let sources = entries.into_iter().collect();
        PackageInfo {
            versions,
            dependencies,
            sources,
        }
    }
}

/// Configuration for the fetch phase.
#[derive(Clone)]
pub struct FetchConfig {
    /// Root directory for git bare repos. Individual databases are stored at
    /// `{db_dir}/{name}-{hash}/` matching the legacy bender layout.
    pub db_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// DependencyFetcher
// ---------------------------------------------------------------------------

/// Fetches and caches package metadata from git repositories.
///
/// Packages are fetched lazily one at a time.
/// When a package is fetched its transitive dependencies are registered in
/// [`pending`](Self::pending) so they can be fetched on demand later.
///
/// Because no [`FetchConfig`] is stored on the struct itself, a
/// `DependencyFetcher` can be constructed and pre-populated via
/// [`packages`](Self::packages) without any git configuration — which
/// is useful in tests.
pub struct DependencyFetcher {
    /// Fetch configuration.
    pub config: FetchConfig,
    /// Open git databases keyed by their on-disk path.
    db_cache: HashMap<PathBuf, GitDatabase>,
    /// All fetched package metadata, keyed by package name.
    pub packages: IndexMap<String, PackageInfo>,
    /// Packages discovered but not yet fetched: name -> how to fetch them.
    pub pending: HashMap<String, ParsedDependency>,
}

impl DependencyFetcher {
    pub fn new(config: FetchConfig) -> Self {
        DependencyFetcher {
            config,
            db_cache: HashMap::new(),
            packages: IndexMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Fetch a single pending package and populate its available versions.
    ///
    /// Reads git tags (or resolves a pinned revision) but does **not** read any
    /// manifests — dependencies are loaded lazily per-version via
    /// [`load_version_deps`](Self::load_version_deps) when pubgrub asks for them.
    ///
    /// Does nothing if `name` is not in `pending` (already fetched or unknown).
    pub(crate) async fn fetch(&mut self, name: &str) -> Result<(), FetchError> {
        let Some(dep) = self.pending.remove(name) else {
            return Ok(());
        };
        let config = self.config.clone();
        match dep {
            ParsedDependency::Path(path_str) => {
                self.packages.insert(
                    name.to_string(),
                    PackageInfo::from_versions(vec![(
                        BenderVersion::Path,
                        VersionSource::Path(PathBuf::from(path_str)),
                    )]),
                );
                Ok(())
            }
            ParsedDependency::GitVersion { url, .. } => {
                self.fetch_git(&config, name, &url, None).await
            }
            ParsedDependency::GitRevision { url, rev } => {
                self.fetch_git(&config, name, &url, Some(&rev)).await
            }
        }
    }

    /// Load and cache the manifest for a specific version, registering its
    /// transitive dependencies as pending.
    ///
    /// Does nothing if the dependencies for this version are already loaded.
    pub(crate) fn load_version_deps(
        &mut self,
        name: &str,
        version: &BenderVersion,
    ) -> Result<(), FetchError> {
        if self
            .packages
            .get(name)
            .and_then(|i| i.dependencies.get(version))
            .is_some_and(|d| d.is_some())
        {
            return Ok(());
        }

        let source = self
            .packages
            .get(name)
            .and_then(|i| i.sources.get(version))
            .cloned();

        let sub_deps: Vec<(String, ParsedDependency)> = match source {
            Some(VersionSource::Path(ref path)) => {
                let bender_yml = path.join("Bender.yml");
                if bender_yml.exists() {
                    let yaml = std::fs::read_to_string(&bender_yml)?;
                    PartialManifest::parse(&yaml)?.resolve_dependencies()?
                } else {
                    Vec::new()
                }
            }
            Some(VersionSource::Git(ref url)) => {
                let rev = match version {
                    BenderVersion::Semver(v) => format!("v{}", v),
                    BenderVersion::GitRevision { hash, .. } => hash.clone(),
                    BenderVersion::Path => unreachable!("Path version cannot have a Git source"),
                };
                let db_path = self.config.db_dir.join(db_name(name, url));
                let db = self.db_cache.get(&db_path).expect("db must be in cache");
                let oid = db
                    .resolve(&rev)
                    .map_err(|e| FetchError::resolve(name, &rev, e))?;
                read_manifest_at(db, &oid, name)?
            }
            None => return Ok(()),
        };

        let deps = self.convert_sub_deps(&sub_deps);
        if let Some(info) = self.packages.get_mut(name) {
            info.dependencies.insert(version.clone(), Some(deps));
        }
        self.register_sub_deps(sub_deps)?;
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // Per-dependency fetch (versions only, no manifest reading)
    // ---------------------------------------------------------------------------

    async fn fetch_git(
        &mut self,
        config: &FetchConfig,
        name: &str,
        url: &str,
        rev: Option<&str>,
    ) -> Result<(), FetchError> {
        let db = self.open_or_init_db(config, name, url, rev).await?;

        let entries: Vec<(BenderVersion, VersionSource)> = match rev {
            None => {
                let mut tags: Vec<semver::Version> = db
                    .list_tags()
                    .map_err(|e| FetchError::git(name, e))?
                    .into_iter()
                    .filter_map(|(tag, _)| {
                        tag.strip_prefix('v')
                            .and_then(|s| semver::Version::parse(s).ok())
                    })
                    .collect();
                tags.sort();
                tags.into_iter()
                    .map(|v| {
                        (
                            BenderVersion::Semver(v),
                            VersionSource::Git(url.to_string()),
                        )
                    })
                    .collect()
            }
            Some(r) => {
                let oid = db.resolve(r).map_err(|e| FetchError::resolve(name, r, e))?;
                // A pinned revision produces exactly one version, so ordering
                // doesn't matter — skip the full commit-graph walk.
                vec![(
                    BenderVersion::GitRevision {
                        index: 0,
                        hash: oid.to_string(),
                    },
                    VersionSource::Git(url.to_string()),
                )]
            }
        };

        if !entries.is_empty() {
            self.packages
                .insert(name.to_string(), PackageInfo::from_versions(entries));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // Database helpers
    // ---------------------------------------------------------------------------

    /// Ensure the git database for `(pkg_name, url)` exists and is up-to-date,
    /// returning a reference to the cached database.
    async fn open_or_init_db(
        &mut self,
        config: &FetchConfig,
        pkg_name: &str,
        url: &str,
        fetch_ref: Option<&str>,
    ) -> Result<&GitDatabase, FetchError> {
        let dir = config.db_dir.join(db_name(pkg_name, url));
        std::fs::create_dir_all(&dir)?;

        // Check cache first to avoid unnecessary git operations. This also ensures we
        // don't accidentally open the same database multiple times if the same package is
        // registered from multiple sources.
        if self.db_cache.contains_key(&dir) {
            return Ok(self.db_cache.get(&dir).unwrap());
        }

        // Open existing database or initialize a new one if it doesn't exist.
        // Track whether this is a fresh init so we know whether to fetch.
        let is_new = !dir.join("config").exists();
        let db = if !is_new {
            GitDatabase::open(&dir).map_err(|e| FetchError::git(pkg_name, e))?
        } else {
            let db = GitDatabase::init_bare(&dir).map_err(|e| FetchError::git(pkg_name, e))?;
            db.add_remote("origin", url)
                .await
                .map_err(|e| FetchError::git(pkg_name, e))?;
            db
        };
        // Fetch on first init to populate the database. On subsequent runs the
        // on-disk database is already up-to-date; skip the network round-trip.
        // Always fetch a pinned ref that may not be reachable from any named ref.
        if is_new {
            db.fetch("origin", NoProgress)
                .await
                .map_err(|e| FetchError::git(pkg_name, e))?;
        }
        if let Some(r) = fetch_ref {
            db.fetch_ref("origin", r)
                .await
                .map_err(|e| FetchError::git(pkg_name, e))?;
        }
        self.db_cache.insert(dir.clone(), db);
        Ok(self.db_cache.get(&dir).unwrap())
    }

    // ---------------------------------------------------------------------------
    // Constraint conversion
    // ---------------------------------------------------------------------------

    /// Convert a list of `(name, ParsedDependency)` pairs into pubgrub constraint
    /// pairs `(String, BenderVersionSet)`.
    fn convert_sub_deps(
        &self,
        sub_deps: &[(String, ParsedDependency)],
    ) -> Vec<(String, BenderVersionSet)> {
        sub_deps
            .iter()
            .map(|(dep_name, dep)| {
                let vs = self.dep_to_version_set(dep_name, dep);
                (dep_name.clone(), vs)
            })
            .collect()
    }

    /// Register newly discovered sub-dependencies as pending.
    ///
    /// Skips packages that are already fetched. Returns an error if the same
    /// package name is registered from two different sources.
    fn register_sub_deps(
        &mut self,
        sub_deps: Vec<(String, ParsedDependency)>,
    ) -> Result<(), FetchError> {
        for (name, dep) in sub_deps {
            if self.packages.contains_key(&name) {
                continue;
            }
            if let Some(existing) = self.pending.get(&name) {
                let existing_url = dep_source_url(existing);
                let new_url = dep_source_url(&dep);
                if normalize_url(existing_url) != normalize_url(new_url) {
                    return Err(FetchError::ConflictingSource {
                        pkg: name,
                        url_a: existing_url.to_string(),
                        url_b: new_url.to_string(),
                    });
                }
            } else {
                self.pending.insert(name, dep);
            }
        }
        Ok(())
    }

    /// Build a `BenderVersionSet` from a `ParsedDependency`, consulting the
    /// already-populated package cache where available.
    pub(crate) fn dep_to_version_set(
        &self,
        dep_name: &str,
        dep: &ParsedDependency,
    ) -> BenderVersionSet {
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

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Extract the source URL or path from a parsed dependency.
fn dep_source_url(dep: &ParsedDependency) -> &str {
    match dep {
        ParsedDependency::Path(p) => p,
        ParsedDependency::GitVersion { url, .. } => url,
        ParsedDependency::GitRevision { url, .. } => url,
    }
}

/// Normalize a git URL for comparison: strip trailing `.git` and `/`.
fn normalize_url(url: &str) -> &str {
    url.trim_end_matches('/').trim_end_matches(".git")
}

/// Compute the database directory name matching bender's legacy convention:
/// `{name}-{first_16_hex_chars_of_blake2b(url)}`.
fn db_name(pkg_name: &str, url: &str) -> String {
    use blake2::{Blake2b512, Digest};
    let hash = format!("{:016x}", Blake2b512::digest(url.as_bytes()));
    format!("{}-{}", pkg_name, &hash[..16])
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
        let mut base = semver::Version::new(major, minor, patch);
        base.pre = comp.pre.clone();

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
