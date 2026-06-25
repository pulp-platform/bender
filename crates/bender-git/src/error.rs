use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git repository not found at {path}")]
    NotARepository { path: PathBuf },

    #[error("git command failed (exit {code}):\n{stderr}")]
    CommandFailed { code: i32, stderr: String },

    #[error("git command was killed by signal:\n{stderr}")]
    CommandKilled { stderr: String },

    #[error("failed to spawn git process: {0}")]
    SpawnFailed(std::io::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("object not found: {oid}")]
    ObjectNotFound { oid: String },

    #[error("reference not found: {refname}")]
    RefNotFound { refname: String },

    #[error("invalid UTF-8 in git output: {context}")]
    InvalidUtf8 { context: String },

    #[error("semaphore closed (runtime shutting down)")]
    SemaphoreClosed,

    #[error("git binary not found: {0}")]
    GitBinNotFound(String),

    #[error("git binary already configured")]
    GitBinAlreadySet,

    #[error("git subprocess throttle already configured")]
    GitThrottleAlreadySet,

    #[error("git-lfs not found: {0}")]
    LfsNotFound(String),

    #[error("gix error: {0}")]
    Gix(String),
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Generate `From<GixError> for GitError` impls that stringify into
/// [`GitError::Gix`], so gix call sites can use bare `?`.
///
/// gix exposes a distinct concrete error type per operation; coherence forbids
/// a single blanket `impl<E: Display> From<E>`, so each type is listed here.
macro_rules! gix_from {
    ($($t:ty),* $(,)?) => {
        $(impl From<$t> for GitError {
            fn from(e: $t) -> Self {
                GitError::Gix(e.to_string())
            }
        })*
    };
}

gix_from!(
    gix::open::Error,
    gix::init::Error,
    gix::remote::init::Error,
    gix::remote::find::existing::Error,
    gix::remote::save::AsError,
    gix::refspec::parse::Error,
    gix::config::file::init::from_paths::Error,
    gix::config::file::set_raw_value::Error,
    gix::reference::edit::Error,
    gix::reference::head_id::Error,
    gix::reference::iter::init::Error,
    gix::revision::spec::parse::single::Error,
    gix::revision::walk::Error,
    gix::revision::walk::iter::Error,
    gix::object::commit::Error,
    gix::object::find::existing::Error,
    gix::refs::packed::buffer::open::Error,
    gix::submodule::modules::Error,
);

/// Boxed errors surfaced by some gix iterators (e.g. reference iteration).
impl From<Box<dyn std::error::Error + Send + Sync>> for GitError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        GitError::Gix(e.to_string())
    }
}
