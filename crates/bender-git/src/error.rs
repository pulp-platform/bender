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

    #[error("path not found in tree: {path}")]
    PathNotFound { path: String },

    #[error("invalid UTF-8 in git output: {context}")]
    InvalidUtf8 { context: String },

    #[error("semaphore closed (runtime shutting down)")]
    SemaphoreClosed,

    #[error("gix error: {0}")]
    Gix(String),
}

pub type Result<T> = std::result::Result<T, GitError>;
