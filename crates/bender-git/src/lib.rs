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
//! - **Local operations** (`init_bare`, `tag_commit`, `list_refs`, `cat_file`,
//!   `list_files`, `resolve`, `current_checkout`, `remote_url`) use `gix`
//!   directly. They are synchronous and do not acquire the throttle semaphore,
//!   so they can run concurrently without limit.
//!
//! - **Subprocess operations** (`fetch`, `clone`, `add_remote`) spawn the
//!   system `git` binary. They are async and acquire the shared semaphore.
//!   `fetch` and `clone` require credential handling; `add_remote` is local
//!   but gix has no public API for persisting a remote to `.git/config`
//!   (the relevant helper is `pub(crate)` in gix).
//!
//! ## Typical usage
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::path::Path;
//! use tokio::sync::Semaphore;
//! use bender_git::database::GitDatabase;
//! use bender_git::checkout::GitCheckout;
//! use bender_git::progress::NoProgress;
//!
//! # async fn run() -> bender_git::error::Result<()> {
//! // Optional: override the git binary (defaults to `which git`).
//! bender_git::set_git_bin("/path/to/git-wrapper.sh")?;
//! let throttle = Arc::new(Semaphore::new(4));
//!
//! // --- Database (bare repo) ---
//! let db = GitDatabase::new(Path::new("/cache/db/myrepo-abc123"), throttle.clone());
//! db.init_bare()?;
//! db.add_remote("origin", "https://github.com/example/repo").await?;
//! db.fetch("origin", NoProgress).await?;
//!
//! // Version listing — fast, no subprocess, no throttle:
//! let refs = db.list_refs()?;
//! let revs = db.list_revs()?;
//!
//! // Read a file from a specific commit:
//! let rev = db.resolve("v1.2.0")?;
//! let entries = db.list_files(&rev, None)?;
//! if let Some(e) = entries.iter().find(|e| e.path.as_os_str() == "Bender.yml") {
//!     let content = db.cat_file_str(&e.oid)?;
//! }
//!
//! // --- Checkout (working tree) ---
//! let tag = format!("bender-tmp-{}", rev.short(8));
//! db.tag_commit(&tag, &rev)?;
//!
//! let checkout = GitCheckout::new(Path::new("/cache/checkouts/myrepo-abc123"), throttle);
//! checkout.clone_from(&db, &tag, NoProgress).await?;
//! # Ok(())
//! # }
//! ```

pub mod checkout;
pub mod database;
pub mod error;
pub mod progress;
pub mod types;

mod subprocess;

pub use subprocess::set_git_bin;
