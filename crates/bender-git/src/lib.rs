//! Git abstraction library for the bender hardware package manager.
//!
//! This crate provides [`database::GitDatabase`] and [`checkout::GitCheckout`] — the two core
//! abstractions for managing git dependencies. It is designed to be
//! integrated into bender but is otherwise independent of it.
//!
//! ## Design overview
//!
//! ### Two-tier model
//!
//! Following the approach used by Cargo, git state is split into two tiers:
//!
//! ```text
//! git/
//! ├── db/
//! │   └── {name}-{hash}/    ← GitDatabase (bare clone, "object cache")
//! └── checkouts/
//!     └── {name}-{hash}/    ← GitCheckout (working tree)
//! ```
//!
//! This crate does **not** manage the filesystem layout itself. Callers pass
//! absolute paths and are responsible for creating directories.
//!
//! ### `gix` vs subprocess
//!
//! - **Local operations** (`tag_commit`, `list_tags`, `list_branches`, `read_file`,
//!   `resolve`, `current_checkout`, `remote_url`) use `gix`
//!   directly. They are synchronous and do not acquire the throttle semaphore,
//!   so they can run concurrently without limit.
//!
//! - **Subprocess operations** (`fetch`, `clone_into`) spawn the system `git`
//!   binary. They are async and acquire the shared semaphore because they
//!   require credential handling or other network-facing behavior.
//!
//! - **Pure `gix` local operations** (`add_remote`, `tag_commit`, `list_tags`,
//!   `list_branches`, `list_revs`, `read_file`, `resolve`, `current_checkout`,
//!   `remote_url`) avoid subprocesses and operate directly on repository data
//!   and config.
//!
//! ## Typical usage
//!
//! ```no_run
//! use std::path::Path;
//! use bender_git::database::GitDatabase;
//! use bender_git::progress::NoProgress;
//!
//! # async fn run() -> bender_git::error::Result<()> {
//! // Optional: override the git binary (defaults to `which git`).
//! bender_git::set_git_bin("/path/to/git-wrapper.sh")?;
//! // Optional: bound concurrent git subprocesses.
//! bender_git::set_git_throttle(4)?;
//!
//! // --- Database (bare repo) ---
//! let db = GitDatabase::init_bare(Path::new("/cache/db/myrepo-abc123"))?;
//! db.add_remote("origin", "https://github.com/example/repo").await?;
//! db.fetch("origin", NoProgress).await?;
//!
//! // Version listing — fast, no subprocess, no throttle:
//! let tags = db.list_tags()?;
//! let branches = db.list_branches()?;
//! let revs = db.list_revs()?;
//!
//! // Read a file from a specific commit:
//! let rev = db.resolve("v1.2.0")?;
//! let content = db.read_file(&rev, Path::new("Bender.yml"))?;
//!
//! // --- Checkout (working tree) ---
//! let tag = format!("bender-tmp-{}", rev.short(8));
//! db.tag_commit(&tag, &rev)?;
//!
//! let checkout = db.clone_into(Path::new("/cache/checkouts/myrepo-abc123"), &tag).await?;
//! # Ok(())
//! # }
//! ```

pub mod checkout;
pub mod database;
pub mod error;
pub mod progress;
pub mod types;

mod subprocess;

pub use subprocess::{set_git_bin, set_git_throttle};
