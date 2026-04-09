use std::cmp::Reverse;

use indexmap::IndexMap;
use pubgrub::{
    Dependencies, DependencyConstraints, DependencyProvider, PackageResolutionStatistics,
};

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

/// The dependency provider for bender's pubgrub-based resolver.
///
/// This is populated during the pre-fetch phase (async) and then passed to
/// pubgrub's synchronous `resolve()` function.
#[derive(Clone, Debug)]
pub struct BenderProvider {
    /// All pre-fetched package metadata, keyed by package name.
    pub packages: IndexMap<String, PackageInfo>,
    /// Lockfile pins (package name -> locked version).
    pub locked: IndexMap<String, BenderVersion>,
}

/// Error type for the dependency provider.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("unknown package: {0}")]
    UnknownPackage(String),
    #[error("unknown version {version} for package {package}")]
    UnknownVersion { package: String, version: String },
    #[error("dependencies not available for {package} @ {version}")]
    DependenciesNotAvailable { package: String, version: String },
}

impl BenderProvider {
    /// Create a new empty provider.
    pub fn new() -> Self {
        BenderProvider {
            packages: IndexMap::new(),
            locked: IndexMap::new(),
        }
    }

    /// Register a package with its available versions.
    pub fn add_package(&mut self, name: impl Into<String>, info: PackageInfo) {
        self.packages.insert(name.into(), info);
    }

    /// Record a lockfile pin.
    pub fn lock_package(&mut self, name: impl Into<String>, version: BenderVersion) {
        self.locked.insert(name.into(), version);
    }

    /// Count how many versions of a package match the given range.
    fn count_versions_in_range(&self, package: &str, range: &BenderVersionSet) -> usize {
        self.packages
            .get(package)
            .map(|info| info.versions.iter().filter(|v| range.contains(v)).count())
            .unwrap_or(0)
    }
}

impl Default for BenderProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyProvider for BenderProvider {
    type P = BenderPackage;
    type V = BenderVersion;
    type VS = BenderVersionSet;
    /// Priority: higher = resolved first.
    /// We use (conflict_count, is_locked, Reverse(version_count)):
    /// - Packages with more conflicts are prioritized
    /// - Locked packages are prioritized (to pin early)
    /// - Packages with fewer matching versions are prioritized (fail-first)
    type Priority = (u32, bool, Reverse<usize>);
    type M = String;
    type Err = ProviderError;

    fn choose_version(
        &self,
        package: &BenderPackage,
        range: &BenderVersionSet,
    ) -> Result<Option<BenderVersion>, ProviderError> {
        // Prefer the locked version if it satisfies the range.
        if let Some(locked_v) = self.locked.get(package.name()) {
            if range.contains(locked_v) {
                return Ok(Some(locked_v.clone()));
            }
        }

        // Otherwise, pick the highest version in range.
        let Some(info) = self.packages.get(package.name()) else {
            return Ok(None);
        };

        Ok(info
            .versions
            .iter()
            .rev()
            .find(|v| range.contains(v))
            .cloned())
    }

    fn prioritize(
        &self,
        package: &BenderPackage,
        range: &BenderVersionSet,
        stats: &PackageResolutionStatistics,
    ) -> Self::Priority {
        let is_locked = self.locked.contains_key(package.name());
        let version_count = self.count_versions_in_range(package.name(), range);
        (stats.conflict_count(), is_locked, Reverse(version_count))
    }

    fn get_dependencies(
        &self,
        package: &BenderPackage,
        version: &BenderVersion,
    ) -> Result<Dependencies<BenderPackage, BenderVersionSet, String>, ProviderError> {
        let Some(info) = self.packages.get(package.name()) else {
            return Ok(Dependencies::Unavailable(format!(
                "unknown package '{}'",
                package.name()
            )));
        };

        let Some(deps_opt) = info.dependencies.get(version) else {
            return Ok(Dependencies::Unavailable(format!(
                "version {} not found for '{}'",
                version,
                package.name()
            )));
        };

        let Some(deps) = deps_opt else {
            return Ok(Dependencies::Unavailable(format!(
                "dependencies not yet loaded for '{}' @ {}",
                package.name(),
                version
            )));
        };

        let constraints: DependencyConstraints<BenderPackage, BenderVersionSet> =
            deps.iter().cloned().collect();

        Ok(Dependencies::Available(constraints))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pubgrub::Ranges;

    fn semver(major: u64, minor: u64, patch: u64) -> BenderVersion {
        BenderVersion::Semver(semver::Version::new(major, minor, patch))
    }

    #[test]
    fn basic_resolution() {
        let mut provider = BenderProvider::new();

        // Root package at version 0.0.0 depends on "dep" ^1.0.0
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
                        BenderPackage::new("dep"),
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

        let solution =
            pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone()).unwrap();

        assert_eq!(solution.get(&BenderPackage::new("root")), Some(&root_v));
        // Should pick highest matching: 1.5.0
        assert_eq!(solution.get(&BenderPackage::new("dep")), Some(&dep_v2));
    }

    #[test]
    fn path_dependency_resolution() {
        let mut provider = BenderProvider::new();

        let root_v = semver(0, 0, 0);

        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        BenderPackage::new("local-dep"),
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

        let solution =
            pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone()).unwrap();

        assert_eq!(
            solution.get(&BenderPackage::new("local-dep")),
            Some(&BenderVersion::Path)
        );
    }

    #[test]
    fn git_revision_resolution() {
        let mut provider = BenderProvider::new();

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
                        BenderPackage::new("git-dep"),
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

        let solution =
            pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone()).unwrap();

        assert_eq!(solution.get(&BenderPackage::new("git-dep")), Some(&rev));
    }

    #[test]
    fn locked_version_preferred() {
        let mut provider = BenderProvider::new();

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
                        BenderPackage::new("dep"),
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

        let solution =
            pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone()).unwrap();

        assert_eq!(solution.get(&BenderPackage::new("dep")), Some(&dep_v1));
    }

    #[test]
    fn conflict_detection() {
        let mut provider = BenderProvider::new();

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
                        (BenderPackage::new("a"), Ranges::singleton(semver(1, 0, 0))),
                        (BenderPackage::new("b"), Ranges::singleton(semver(1, 0, 0))),
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
                        BenderPackage::new("dep"),
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
                        BenderPackage::new("dep"),
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

        let result = pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone());
        assert!(result.is_err(), "expected NoSolution conflict");
    }

    #[test]
    fn cross_strata_semver_vs_git_conflict() {
        let mut provider = BenderProvider::new();

        let root_v = semver(0, 0, 0);
        let rev = BenderVersion::GitRevision {
            index: 0,
            hash: "deadbeef".to_string(),
        };

        // Root depends on both "a" and "b"
        // "a" requires "dep" as semver ^1.0.0
        // "b" requires "dep" as a git revision
        // → conflict: same package, incompatible strata
        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![
                        (BenderPackage::new("a"), Ranges::singleton(semver(1, 0, 0))),
                        (BenderPackage::new("b"), Ranges::singleton(semver(1, 0, 0))),
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
                        BenderPackage::new("dep"),
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
                    Some(vec![(
                        BenderPackage::new("dep"),
                        Ranges::singleton(rev.clone()),
                    )]),
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

        let result = pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone());
        assert!(
            result.is_err(),
            "expected NoSolution for semver vs git revision conflict"
        );
    }

    #[test]
    fn cross_strata_conflict() {
        let mut provider = BenderProvider::new();

        let root_v = semver(0, 0, 0);

        // Root depends on "dep" as path
        // But "dep" only has semver versions → conflict
        provider.add_package(
            "root",
            PackageInfo {
                versions: vec![root_v.clone()],
                dependencies: IndexMap::from([(
                    root_v.clone(),
                    Some(vec![(
                        BenderPackage::new("dep"),
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

        let result = pubgrub::resolve(&provider, BenderPackage::new("root"), root_v.clone());
        assert!(
            result.is_err(),
            "expected NoSolution for path vs semver conflict"
        );
    }
}
