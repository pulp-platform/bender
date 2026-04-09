use std::fmt;

/// A package in bender's dependency resolution.
///
/// This is simply a package name. If multiple git URLs provide the same package
/// name, their version lists are merged in the dependency provider. Source URL
/// metadata is tracked separately for checkout.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BenderPackage(pub String);

impl BenderPackage {
    /// Create a new package with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        BenderPackage(name.into())
    }

    /// Returns the package name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BenderPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for BenderPackage {
    fn from(name: String) -> Self {
        BenderPackage(name)
    }
}

impl From<&str> for BenderPackage {
    fn from(name: &str) -> Self {
        BenderPackage(name.to_string())
    }
}
