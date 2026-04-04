use std::path::PathBuf;

/// A resolved git object ID (SHA-1, 40-char hex string).
///
/// This is a newtype over `String` so that `gix::ObjectId` remains an
/// implementation detail and callers are not coupled to a specific gix version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectId(pub String);

impl ObjectId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the first `n` hex characters (short hash).
    pub fn short(&self, n: usize) -> &str {
        &self.0[..n.min(self.0.len())]
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
        if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(ObjectId(s.to_owned()))
        } else {
            Err(crate::error::GitError::ObjectNotFound {
                oid: s.to_owned(),
            })
        }
    }
}

/// Categorisation of a git reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    Tag,
    Branch,
    /// Any other ref (notes, pull requests, etc.).
    Other,
}

/// A git reference with its resolved commit OID.
///
/// Annotated tags are automatically peeled to their target commit, so `commit`
/// always holds a commit hash rather than a tag object hash.
#[derive(Debug, Clone)]
pub struct GitRef {
    /// Full ref name, e.g. `refs/tags/v1.2.3` or `refs/remotes/origin/main`.
    pub name: String,
    /// The commit OID this reference points to (always dereferenced through tags).
    pub commit: ObjectId,
    pub kind: RefKind,
}

impl GitRef {
    /// Short name with the well-known prefix stripped.
    ///
    /// - `refs/tags/v1.2.3` → `v1.2.3`
    /// - `refs/remotes/origin/main` → `main`
    /// - `refs/heads/main` → `main`
    pub fn short_name(&self) -> &str {
        match self.kind {
            RefKind::Tag => self
                .name
                .strip_prefix("refs/tags/")
                .unwrap_or(&self.name),
            RefKind::Branch => self
                .name
                .strip_prefix("refs/remotes/origin/")
                .or_else(|| self.name.strip_prefix("refs/heads/"))
                .unwrap_or(&self.name),
            RefKind::Other => &self.name,
        }
    }
}

/// Kind of a git tree entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntryKind {
    Blob,
    BlobExecutable,
    Tree,
    /// Submodule (gitlink).
    Commit,
    Symlink,
}

impl std::fmt::Display for TreeEntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeEntryKind::Blob => write!(f, "blob"),
            TreeEntryKind::BlobExecutable => write!(f, "blob"),
            TreeEntryKind::Tree => write!(f, "tree"),
            TreeEntryKind::Commit => write!(f, "commit"),
            TreeEntryKind::Symlink => write!(f, "blob"),
        }
    }
}

/// A single entry in a git tree (the result of `git ls-tree`).
#[derive(Debug, Clone)]
pub struct TreeEntry {
    /// The git file mode string (e.g. `100644`, `040000`).
    pub mode: String,
    pub kind: TreeEntryKind,
    /// The object hash of the entry's content.
    pub oid: ObjectId,
    /// Path relative to the tree root, no leading `/`.
    pub path: PathBuf,
}
