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
    SpawnFailed(#[from] std::io::Error),

    #[error("object not found: {oid}")]
    ObjectNotFound { oid: String },

    #[error("reference not found: {refname}")]
    RefNotFound { refname: String },

    #[error("invalid UTF-8 in git output: {context}")]
    InvalidUtf8 { context: String },

    #[error("semaphore closed (runtime shutting down)")]
    SemaphoreClosed,

    #[error("git binary already configured")]
    GitBinAlreadySet,

    #[error("git-lfs not found: {0}")]
    LfsNotFound(String),

    #[error("gix error: {0}")]
    Gix(String),
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Convert any [`Display`](std::fmt::Display)-able error into [`GitError::Gix`].
///
/// Use as `.map_err(gix_err)` at gix call sites instead of the verbose
/// `.map_err(|e| GitError::Gix(e.to_string()))` lambda.
pub fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}
