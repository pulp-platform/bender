use std::cmp::Ordering;
use std::fmt;

/// A version in bender's dependency resolution.
///
/// Bender has three kinds of dependencies, each with different versioning:
/// - **Path**: local filesystem, always exactly one "version"
/// - **Semver**: semantic versions extracted from git tags (`v1.2.3`)
/// - **GitRevision**: pinned git commits, ordered by commit time
///
/// These form non-overlapping strata in the ordering: `Path < Semver < GitRevision`.
/// A given package only ever has versions from one stratum, so cross-stratum
/// comparisons indicate genuine conflicts that pubgrub correctly reports.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum BenderVersion {
    /// Path dependency: exactly one "version" exists.
    Path,
    /// A semantic version resolved from a git tag (e.g. `v1.2.3`).
    Semver(semver::Version),
    /// A pinned git commit. The `index` provides a total ordering derived from
    /// `git rev-list --all --date-order` (newest commit = highest index).
    /// The `hash` is the full SHA-1 hex string.
    GitRevision { index: u64, hash: String },
}

impl BenderVersion {
    /// Returns `true` if this is a path version.
    pub fn is_path(&self) -> bool {
        matches!(self, BenderVersion::Path)
    }

    /// Returns `true` if this is a semver version.
    pub fn is_semver(&self) -> bool {
        matches!(self, BenderVersion::Semver(_))
    }

    /// Returns `true` if this is a git revision.
    pub fn is_git_revision(&self) -> bool {
        matches!(self, BenderVersion::GitRevision { .. })
    }

    /// Returns the semver version if this is a `Semver` variant.
    pub fn as_semver(&self) -> Option<&semver::Version> {
        match self {
            BenderVersion::Semver(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the git hash if this is a `GitRevision` variant.
    pub fn as_git_hash(&self) -> Option<&str> {
        match self {
            BenderVersion::GitRevision { hash, .. } => Some(hash),
            _ => None,
        }
    }
}

/// Ordering: Path < all Semver < all GitRevision.
/// Within Semver: natural semver ordering.
/// Within GitRevision: ordered by index (commit time).
impl Ord for BenderVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (BenderVersion::Path, BenderVersion::Path) => Ordering::Equal,
            (BenderVersion::Path, _) => Ordering::Less,
            (_, BenderVersion::Path) => Ordering::Greater,

            (BenderVersion::Semver(a), BenderVersion::Semver(b)) => a.cmp(b),
            (BenderVersion::Semver(_), BenderVersion::GitRevision { .. }) => Ordering::Less,
            (BenderVersion::GitRevision { .. }, BenderVersion::Semver(_)) => Ordering::Greater,

            (
                BenderVersion::GitRevision { index: a, .. },
                BenderVersion::GitRevision { index: b, .. },
            ) => a.cmp(b),
        }
    }
}

impl PartialOrd for BenderVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for BenderVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BenderVersion::Path => write!(f, "path"),
            BenderVersion::Semver(v) => write!(f, "{v}"),
            BenderVersion::GitRevision { hash, .. } => {
                let short = &hash[..7.min(hash.len())];
                write!(f, "rev:{short}")
            }
        }
    }
}

/// The version set type used with pubgrub. Uses `Ranges<BenderVersion>` which
/// provides set algebra (empty, singleton, complement, intersection, union)
/// for any `V: Ord + Clone + Display + Debug + Eq`.
pub type BenderVersionSet = pubgrub::Ranges<BenderVersion>;

#[cfg(test)]
mod tests {
    use super::*;

    fn semver(major: u64, minor: u64, patch: u64) -> BenderVersion {
        BenderVersion::Semver(semver::Version::new(major, minor, patch))
    }

    fn rev(index: u64, hash: &str) -> BenderVersion {
        BenderVersion::GitRevision {
            index,
            hash: hash.to_string(),
        }
    }

    #[test]
    fn ordering_strata() {
        // Path < Semver < GitRevision
        assert!(BenderVersion::Path < semver(0, 0, 0));
        assert!(semver(999, 999, 999) < rev(0, "abc"));
        assert!(BenderVersion::Path < rev(0, "abc"));
    }

    #[test]
    fn ordering_within_semver() {
        assert!(semver(1, 0, 0) < semver(1, 0, 1));
        assert!(semver(1, 0, 0) < semver(1, 1, 0));
        assert!(semver(1, 0, 0) < semver(2, 0, 0));
        assert_eq!(semver(1, 2, 3), semver(1, 2, 3));
    }

    #[test]
    fn ordering_within_git_revision() {
        assert!(rev(0, "aaa") < rev(1, "bbb"));
        assert!(rev(0, "zzz") < rev(1, "aaa")); // ordered by index, not hash
        assert_eq!(rev(42, "abc"), rev(42, "abc"));
    }

    #[test]
    fn display() {
        assert_eq!(BenderVersion::Path.to_string(), "path");
        assert_eq!(semver(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(rev(0, "abc1234567890").to_string(), "rev:abc1234");
    }

    #[test]
    fn ranges_singleton() {
        let path_range: BenderVersionSet = pubgrub::Ranges::singleton(BenderVersion::Path);
        assert!(path_range.contains(&BenderVersion::Path));
        assert!(!path_range.contains(&semver(1, 0, 0)));
    }

    #[test]
    fn ranges_semver_range() {
        let range: BenderVersionSet =
            pubgrub::Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0));
        assert!(range.contains(&semver(1, 5, 0)));
        assert!(!range.contains(&semver(2, 0, 0)));
        assert!(!range.contains(&semver(0, 9, 0)));
        assert!(!range.contains(&BenderVersion::Path));
        assert!(!range.contains(&rev(0, "abc")));
    }

    #[test]
    fn ranges_complement() {
        let path_range: BenderVersionSet = pubgrub::Ranges::singleton(BenderVersion::Path);
        let not_path = path_range.complement();
        assert!(!not_path.contains(&BenderVersion::Path));
        assert!(not_path.contains(&semver(1, 0, 0)));
        assert!(not_path.contains(&rev(0, "abc")));
    }

    #[test]
    fn ranges_intersection() {
        let semver_range: BenderVersionSet =
            pubgrub::Ranges::from_range_bounds(semver(1, 0, 0)..semver(2, 0, 0));
        let not_path: BenderVersionSet =
            pubgrub::Ranges::<BenderVersion>::singleton(BenderVersion::Path).complement();
        let both = semver_range.intersection(&not_path);
        assert!(both.contains(&semver(1, 5, 0)));
        assert!(!both.contains(&BenderVersion::Path));
    }

    #[test]
    fn ranges_git_revision_singleton() {
        let rev_range: BenderVersionSet = pubgrub::Ranges::singleton(rev(42, "abc1234"));
        assert!(rev_range.contains(&rev(42, "abc1234")));
        assert!(!rev_range.contains(&rev(43, "def5678")));
        assert!(!rev_range.contains(&semver(1, 0, 0)));
    }
}
