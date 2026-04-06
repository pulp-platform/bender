/// A resolved git object ID (SHA-1).
///
/// Wraps [`gix::ObjectId`] to ensure validity at construction time.
/// Use [`str::parse`] to construct from a hex string, or [`Display`] /
/// [`ToString`] to get the 40-character hex representation.
///
/// [`Display`]: std::fmt::Display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(gix::ObjectId);

impl ObjectId {
    /// Returns the first `n` hex characters (short hash).
    pub fn short(self, n: usize) -> String {
        let hex = self.0.to_hex().to_string();
        hex[..n.min(hex.len())].to_owned()
    }
}

impl From<gix::ObjectId> for ObjectId {
    fn from(id: gix::ObjectId) -> Self {
        ObjectId(id)
    }
}

impl From<ObjectId> for gix::ObjectId {
    fn from(id: ObjectId) -> Self {
        id.0
    }
}

impl std::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ObjectId {
    type Err = crate::error::GitError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        gix::ObjectId::from_hex(s.as_bytes())
            .map(ObjectId)
            .map_err(|_| crate::error::GitError::ObjectNotFound { oid: s.to_owned() })
    }
}
