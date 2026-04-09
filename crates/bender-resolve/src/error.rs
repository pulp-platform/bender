use pubgrub::{DefaultStringReporter, DerivationTree, PubGrubError, Reporter};

use crate::provider::BenderProvider;

/// Errors that can occur during dependency resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No solution exists for the given dependency constraints.
    #[error("dependency resolution failed:\n{report}")]
    NoSolution {
        report: String,
        derivation_tree: DerivationTree<
            <BenderProvider as pubgrub::DependencyProvider>::P,
            <BenderProvider as pubgrub::DependencyProvider>::VS,
            <BenderProvider as pubgrub::DependencyProvider>::M,
        >,
    },

    /// An error occurred while retrieving dependencies.
    #[error("failed to retrieve dependencies for {package} @ {version}: {source}")]
    DependencyRetrieval {
        package: String,
        version: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// An error occurred while choosing a version.
    #[error("failed to choose version for {package}: {source}")]
    VersionChoice {
        package: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// An I/O error during pre-fetching.
    #[error("failed to fetch package metadata: {0}")]
    Prefetch(String),
}

impl ResolveError {
    /// Create a `NoSolution` error from a pubgrub derivation tree.
    pub fn no_solution(
        mut derivation_tree: DerivationTree<
            <BenderProvider as pubgrub::DependencyProvider>::P,
            <BenderProvider as pubgrub::DependencyProvider>::VS,
            <BenderProvider as pubgrub::DependencyProvider>::M,
        >,
    ) -> Self {
        derivation_tree.collapse_no_versions();
        let report = DefaultStringReporter::report(&derivation_tree);
        ResolveError::NoSolution {
            report,
            derivation_tree,
        }
    }
}

/// Convert a `PubGrubError` into a `ResolveError`.
impl From<PubGrubError<BenderProvider>> for ResolveError {
    fn from(err: PubGrubError<BenderProvider>) -> Self {
        match err {
            PubGrubError::NoSolution(tree) => ResolveError::no_solution(tree),
            PubGrubError::ErrorRetrievingDependencies {
                package,
                version,
                source,
            } => ResolveError::DependencyRetrieval {
                package: package.to_string(),
                version: version.to_string(),
                source: Box::new(source),
            },
            PubGrubError::ErrorChoosingVersion { package, source } => ResolveError::VersionChoice {
                package: package.to_string(),
                source: Box::new(source),
            },
            PubGrubError::ErrorInShouldCancel(err) => ResolveError::VersionChoice {
                package: "<cancelled>".to_string(),
                source: Box::new(err),
            },
        }
    }
}
