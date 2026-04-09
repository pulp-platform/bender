use std::cell::RefCell;
use std::cmp::Reverse;

use indexmap::IndexMap;
use pubgrub::{
    Dependencies, DependencyConstraints, DependencyProvider, PackageResolutionStatistics,
};

use crate::fetcher::{DependencyFetcher, FetchError, PackageInfo};
use crate::manifest::{ManifestError, PartialManifest};
use crate::version::{BenderVersion, BenderVersionSet};

/// The dependency provider for bender's pubgrub-based resolver.
///
/// Wraps a [`DependencyFetcher`] and exposes it to pubgrub's synchronous
/// `resolve()` function via interior mutability. Packages are fetched lazily
/// inside `choose_version` when first needed.
///
/// # Usage
///
/// ```no_run
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use bender_resolve::provider::BenderProvider;
/// use bender_resolve::fetcher::{DependencyFetcher, FetchConfig};
/// use bender_resolve::manifest::PartialManifest;
/// use bender_resolve::version::BenderVersion;
///
/// let config = FetchConfig {
///     db_dir: "/home/user/.bender/git/db".into(),
/// };
/// let root_yaml = std::fs::read_to_string("Bender.yml")?;
/// let root = PartialManifest::parse(&root_yaml)?;
///
/// let mut provider = BenderProvider::new(DependencyFetcher::new(config));
/// provider.init_root(&root, "my-project", BenderVersion::Semver(semver::Version::new(0, 1, 0)))?;
/// # Ok(()) }
/// ```
pub struct BenderProvider {
    fetcher: RefCell<DependencyFetcher>,
    /// Lockfile pins (package name -> locked version).
    pub locked: IndexMap<String, BenderVersion>,
}

impl BenderProvider {
    /// Create a new empty provider.
    pub fn new(fetcher: DependencyFetcher) -> Self {
        BenderProvider {
            fetcher: RefCell::new(fetcher),
            locked: IndexMap::new(),
        }
    }

    /// Register the root manifest and add a synthetic root package that
    /// enforces the manifest's version constraints.
    pub fn init_root(
        &mut self,
        root_manifest: &PartialManifest,
        root_name: impl Into<String>,
        root_version: BenderVersion,
    ) -> Result<(), ManifestError> {
        let deps = root_manifest.resolve_dependencies()?;
        let fetcher = self.fetcher.get_mut();
        for (name, dep) in &deps {
            if !fetcher.packages.contains_key(name) {
                fetcher.pending.entry(name.clone()).or_insert(dep.clone());
            }
        }
        let root_deps: Vec<(String, BenderVersionSet)> = deps
            .iter()
            .map(|(name, dep)| (name.clone(), fetcher.dep_to_version_set(name, dep)))
            .collect();
        fetcher.packages.insert(
            root_name.into(),
            PackageInfo {
                versions: vec![root_version.clone()],
                dependencies: IndexMap::from([(root_version, Some(root_deps))]),
                sources: IndexMap::new(),
            },
        );
        Ok(())
    }

    /// Register a package directly, bypassing git fetching.
    ///
    /// Useful for pre-populating the provider in tests.
    pub fn add_package(&mut self, name: impl Into<String>, info: PackageInfo) {
        self.fetcher.get_mut().packages.insert(name.into(), info);
    }

    /// Record a lockfile pin.
    pub fn lock_package(&mut self, name: impl Into<String>, version: BenderVersion) {
        self.locked.insert(name.into(), version);
    }

    /// Synchronous bridge between pubgrub's sync `get_dependencies` and the
    /// async fetcher. This is the callsite to replace when moving to a
    /// channel-based async boundary in the future.
    fn fetch_blocking(&self, name: &str) -> Result<(), FetchError> {
        if self.fetcher.borrow().packages.contains_key(name)
            || !self.fetcher.borrow().pending.contains_key(name)
        {
            return Ok(());
        }
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.fetcher.borrow_mut().fetch(name))
        })
    }

    fn count_versions_in_range(&self, package: &str, range: &BenderVersionSet) -> usize {
        self.fetcher
            .borrow()
            .packages
            .get(package)
            .map(|info| info.versions.iter().filter(|v| range.contains(v)).count())
            .unwrap_or(0)
    }
}

impl DependencyProvider for BenderProvider {
    type P = String;
    type V = BenderVersion;
    type VS = BenderVersionSet;
    /// Priority: higher = resolved first.
    /// We use (conflict_count, is_locked, Reverse(version_count)):
    /// - Packages with more conflicts are prioritized
    /// - Locked packages are prioritized (to pin early)
    /// - Packages with fewer matching versions are prioritized (fail-first)
    type Priority = (u32, bool, Reverse<usize>);
    type M = String;
    type Err = FetchError;

    fn prioritize(
        &self,
        package: &String,
        range: &BenderVersionSet,
        stats: &PackageResolutionStatistics,
    ) -> Self::Priority {
        let is_locked = self.locked.contains_key(package);
        let version_count = self.count_versions_in_range(package, range);
        (stats.conflict_count(), is_locked, Reverse(version_count))
    }

    fn choose_version(
        &self,
        package: &String,
        range: &BenderVersionSet,
    ) -> Result<Option<BenderVersion>, FetchError> {
        self.fetch_blocking(package)?;

        // Prefer the locked version if it satisfies the range.
        if let Some(locked_v) = self.locked.get(package)
            && range.contains(locked_v)
        {
            return Ok(Some(locked_v.clone()));
        }

        let fetcher = self.fetcher.borrow();
        let Some(info) = fetcher.packages.get(package) else {
            return Ok(None);
        };

        Ok(info
            .versions
            .iter()
            .rev()
            .find(|v| range.contains(v))
            .cloned())
    }

    fn get_dependencies(
        &self,
        package: &String,
        version: &BenderVersion,
    ) -> Result<Dependencies<String, BenderVersionSet, String>, FetchError> {
        self.fetcher
            .borrow_mut()
            .load_version_deps(package, version)?;
        let fetcher = self.fetcher.borrow();

        let Some(info) = fetcher.packages.get(package) else {
            return Ok(Dependencies::Unavailable(format!(
                "unknown package '{}'",
                package
            )));
        };

        let Some(deps_opt) = info.dependencies.get(version) else {
            return Ok(Dependencies::Unavailable(format!(
                "version {} not found for '{}'",
                version, package
            )));
        };

        let Some(deps) = deps_opt else {
            return Ok(Dependencies::Unavailable(format!(
                "dependencies not yet loaded for '{}' @ {}",
                package, version
            )));
        };

        let constraints: DependencyConstraints<String, BenderVersionSet> =
            deps.iter().cloned().collect();

        Ok(Dependencies::Available(constraints))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::{FetchConfig, VersionSource};
    use pubgrub::Ranges;

    fn test_fetcher() -> DependencyFetcher {
        DependencyFetcher::new(FetchConfig {
            db_dir: std::env::temp_dir(),
        })
    }

    fn semver(major: u64, minor: u64, patch: u64) -> BenderVersion {
        BenderVersion::Semver(semver::Version::new(major, minor, patch))
    }

    #[test]
    fn basic_resolution() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);
        let dep_v1 = semver(1, 0, 0);
        let dep_v2 = semver(1, 5, 0);
        let dep_v3 = semver(2, 0, 0);

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0)),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "dep",
            PackageInfo {
                versions: vec![dep_v1.clone(), dep_v2.clone(), dep_v3.clone()],
                dependencies: IndexMap::from([
                    (dep_v1.clone(), Some(vec![])),
                    (dep_v2.clone(), Some(vec![])),
                    (dep_v3.clone(), Some(vec![])),
                ]),
                sources: IndexMap::new(),
            },
        );

        let solution = pubgrub::resolve(&provider, "root".to_string(), root_v.clone()).unwrap();

        assert_eq!(solution.get(&"root".to_string()), Some(&root_v));
        // Should pick highest matching: 1.5.0
        assert_eq!(solution.get(&"dep".to_string()), Some(&dep_v2));
    }

    #[test]
    fn path_dependency_resolution() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        "local-dep".to_string(),
                        Ranges::singleton(BenderVersion::Path),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "local-dep",
            PackageInfo {
                versions: vec![BenderVersion::Path],
                dependencies: IndexMap::from([(BenderVersion::Path, Some(vec![]))]),
                sources: IndexMap::from([(
                    BenderVersion::Path,
                    VersionSource::Path("/some/path".into()),
                )]),
            },
        );

        let solution = pubgrub::resolve(&provider, "root".to_string(), root_v.clone()).unwrap();

        assert_eq!(
            solution.get(&"local-dep".to_string()),
            Some(&BenderVersion::Path)
        );
    }

    #[test]
    fn git_revision_resolution() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);
        let rev = BenderVersion::GitRevision {
            index: 42,
            hash: "abc1234567890".to_string(),
        };

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        "git-dep".to_string(),
                        Ranges::singleton(rev.clone()),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "git-dep",
            PackageInfo {
                versions: vec![rev.clone()],
                dependencies: IndexMap::from([(rev.clone(), Some(vec![]))]),
                sources: IndexMap::from([(
                    rev.clone(),
                    VersionSource::Git("https://github.com/example/repo.git".to_string()),
                )]),
            },
        );

        let solution = pubgrub::resolve(&provider, "root".to_string(), root_v.clone()).unwrap();

        assert_eq!(solution.get(&"git-dep".to_string()), Some(&rev));
    }

    #[test]
    fn locked_version_preferred() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);
        let dep_v1 = semver(1, 0, 0);
        let dep_v2 = semver(1, 5, 0);

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0)),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "dep",
            PackageInfo {
                versions: vec![dep_v1.clone(), dep_v2.clone()],
                dependencies: IndexMap::from([
                    (dep_v1.clone(), Some(vec![])),
                    (dep_v2.clone(), Some(vec![])),
                ]),
                sources: IndexMap::new(),
            },
        );

        // Lock to 1.0.0 even though 1.5.0 is available
        provider.lock_package("dep", dep_v1.clone());

        let solution = pubgrub::resolve(&provider, "root".to_string(), root_v.clone()).unwrap();

        assert_eq!(solution.get(&"dep".to_string()), Some(&dep_v1));
    }

    #[test]
    fn conflict_detection() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);

        // Root depends on both A and B
        // A requires dep >=2.0.0
        // B requires dep <2.0.0
        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![
                        ("a".to_string(), Ranges::singleton(semver(1, 0, 0))),
                        ("b".to_string(), Ranges::singleton(semver(1, 0, 0))),
                    ]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "a",
            PackageInfo {
                versions: vec![semver(1, 0, 0)],
                dependencies: IndexMap::from([(
                    semver(1, 0, 0),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::from_range_bounds(semver(2, 0, 0)..semver(3, 0, 0)),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "b",
            PackageInfo {
                versions: vec![semver(1, 0, 0)],
                dependencies: IndexMap::from([(
                    semver(1, 0, 0),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0)),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "dep",
            PackageInfo {
                versions: vec![semver(1, 0, 0), semver(2, 0, 0)],
                dependencies: IndexMap::from([
                    (semver(1, 0, 0), Some(vec![])),
                    (semver(2, 0, 0), Some(vec![])),
                ]),
                sources: IndexMap::new(),
            },
        );

        let result = pubgrub::resolve(&provider, "root".to_string(), root_v.clone());
        assert!(result.is_err(), "expected NoSolution conflict");
    }

    #[test]
    fn cross_strata_semver_vs_git_conflict() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);
        let rev = BenderVersion::GitRevision {
            index: 0,
            hash: "deadbeef".to_string(),
        };

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![
                        ("a".to_string(), Ranges::singleton(semver(1, 0, 0))),
                        ("b".to_string(), Ranges::singleton(semver(1, 0, 0))),
                    ]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "a",
            PackageInfo {
                versions: vec![semver(1, 0, 0)],
                dependencies: IndexMap::from([(
                    semver(1, 0, 0),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0)),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "b",
            PackageInfo {
                versions: vec![semver(1, 0, 0)],
                dependencies: IndexMap::from([(
                    semver(1, 0, 0),
                    Some(vec![("dep".to_string(), Ranges::singleton(rev.clone()))]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "dep",
            PackageInfo {
                versions: vec![semver(1, 0, 0), rev.clone()],
                dependencies: IndexMap::from([
                    (semver(1, 0, 0), Some(vec![])),
                    (rev.clone(), Some(vec![])),
                ]),
                sources: IndexMap::from([(
                    rev.clone(),
                    VersionSource::Git("https://github.com/example/dep.git".to_string()),
                )]),
            },
        );

        let result = pubgrub::resolve(&provider, "root".to_string(), root_v.clone());
        assert!(
            result.is_err(),
            "expected NoSolution for semver vs git revision conflict"
        );
    }

    #[test]
    fn cross_strata_conflict() {
        let mut provider = BenderProvider::new(test_fetcher());

        let root_v = semver(0, 0, 0);

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        "dep".to_string(),
                        Ranges::singleton(BenderVersion::Path),
                    )]),
                )]),
                sources: IndexMap::new(),
            },
        );

        provider.add_package(
            "dep",
            PackageInfo {
                versions: vec![semver(1, 0, 0)],
                dependencies: IndexMap::from([(semver(1, 0, 0), Some(vec![]))]),
                sources: IndexMap::new(),
            },
        );

        let result = pubgrub::resolve(&provider, "root".to_string(), root_v.clone());
        assert!(
            result.is_err(),
            "expected NoSolution for path vs semver conflict"
        );
    }
}
