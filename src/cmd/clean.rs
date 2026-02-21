//! The `clean` subcommand.

use clap::Args;
use std::path::Path;

use miette::{Context as _, IntoDiagnostic as _};
use std::fs;

use crate::error::*;
use crate::infoln;
use crate::sess::Session;

/// Clean all bender related dependencies
#[derive(Args, Debug)]
pub struct CleanArgs {
    /// Include Bender.lock in clean
    #[arg(long)]
    pub all: bool,
}

/// Execute the `clean` subcommand.
pub fn run(sess: &Session, all: bool, path: &Path) -> Result<()> {
    // Clean the checkout directory
    if let Some(checkout_dir) = &sess.manifest.workspace.checkout_dir {
        let checkout_path = Path::new(checkout_dir);
        if checkout_path.exists() && checkout_path.is_dir() {
            fs::remove_dir_all(checkout_path)
                .into_diagnostic()
                .wrap_err_with(|| {
                    format!("Failed to clean checkout directory {:?}.", checkout_path)
                })?;
            infoln!("Successfully cleaned {}", checkout_dir.display());
        } else {
            infoln!("No checkout directory found.");
        }
    }

    // Clean the .bender directory
    let bender_dir = path.join(".bender");
    if bender_dir.exists() && bender_dir.is_dir() {
        fs::remove_dir_all(&bender_dir)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to clean directory {:?}.", bender_dir))?;
        infoln!("Successfully cleaned .bender directory.");
    }

    // Clean the Bender.lock file
    let bender_lock = path.join("Bender.lock");
    if bender_lock.exists() && bender_lock.is_file() && all {
        fs::remove_file(&bender_lock)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to remove file {:?}.", bender_lock))?;
        infoln!("Successfully removed Bender.lock file.");
    }

    Ok(())
}
