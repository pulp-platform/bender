//! PubGrub-based dependency resolver for the bender hardware package manager.
//!
//! This crate provides a dependency resolver that maps bender's heterogeneous
//! dependency types (semver git tags, path dependencies, git revisions) onto
//! pubgrub's version-solving algorithm.
//!
//! # Architecture
//!
//! Packages are fetched lazily on demand as pubgrub's solver requests them.
//! Initialize a [`BenderProvider`] with [`BenderProvider::init_root`] (sync) to
//! register the root manifest's direct dependencies, then call [`resolve`].
//! Each package is fetched at most once, the first time pubgrub picks a version
//! for it.
//!
//! # Version Model
//!
//! Bender has three kinds of dependency versions, modeled as variants of
//! [`BenderVersion`]:
//!
//! - **Path**: local filesystem dependency, exactly one "version"
//! - **Semver**: semantic versions from git tags (`v1.2.3`)
//! - **GitRevision**: pinned git commits, ordered by commit time
//!
//! These form non-overlapping strata in the version ordering, allowing
//! `pubgrub::Ranges<BenderVersion>` to serve as the version set type
//! without a custom `VersionSet` implementation.

pub mod error;
pub mod fetcher;
pub mod manifest;
pub mod provider;
pub mod version;

pub use error::ResolveError;
pub use fetcher::{DependencyFetcher, FetchConfig, FetchError, PackageInfo, VersionSource};
pub use provider::BenderProvider;
pub use version::{BenderVersion, BenderVersionSet};

/// Resolve dependencies using pubgrub.
///
/// Takes a populated [`BenderProvider`] and a root package/version, and returns
/// the selected dependencies or a resolution error.
pub fn resolve(
    provider: &BenderProvider,
    root_package: String,
    root_version: BenderVersion,
) -> Result<pubgrub::SelectedDependencies<BenderProvider>, ResolveError> {
    pubgrub::resolve(provider, root_package, root_version).map_err(ResolveError::from)
}
