//! PubGrub-based dependency resolver for the bender hardware package manager.
//!
//! This crate provides a dependency resolver that maps bender's heterogeneous
//! dependency types (semver git tags, path dependencies, git revisions) onto
//! pubgrub's version-solving algorithm.
//!
//! # Architecture
//!
//! Resolution is a two-phase process:
//!
//! 1. **Pre-fetch (async)**: Discover all reachable packages and their available
//!    versions by fetching git repositories and reading manifests. This populates
//!    a [`BenderProvider`] via [`BenderProvider::fetch`].
//!
//! 2. **Resolve (sync)**: Run pubgrub's solver against the populated provider.
//!    This is pure computation with no I/O.
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
pub mod package;
pub mod provider;
pub mod version;

pub use error::ResolveError;
pub use fetcher::{DependencyFetcher, FetchConfig, FetchError, PackageInfo, VersionSource};
pub use package::BenderPackage;
pub use provider::BenderProvider;
pub use version::{BenderVersion, BenderVersionSet};

/// Resolve dependencies using pubgrub.
///
/// Takes a populated [`BenderProvider`] and a root package/version, and returns
/// the selected dependencies or a resolution error.
pub fn resolve(
    provider: &BenderProvider,
    root_package: BenderPackage,
    root_version: BenderVersion,
) -> Result<pubgrub::SelectedDependencies<BenderProvider>, ResolveError> {
    pubgrub::resolve(provider, root_package, root_version).map_err(ResolveError::from)
}
