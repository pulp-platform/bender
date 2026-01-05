//! The `clean` subcommand.

use clap::{ArgAction, Args};
use std::path::Path;

use std::fs;

use crate::error::*;
use crate::sess::Session;

/// Clean all bender related dependencies
#[derive(Args, Debug)]
pub struct CleanArgs {
    /// Include Bender.lock in clean
    #[arg(long, action = ArgAction::SetTrue)]
    pub all: bool,
}

/// Execute the `clean` subcommand.
pub fn run(sess: &Session, all: bool, path: &Path) -> Result<()> {
    eprintln!("Cleaning all dependencies");

    // Clean the checkout directory
    if let Some(checkout_dir) = &sess.manifest.workspace.checkout_dir {
        let checkout_path = Path::new(checkout_dir);
        if checkout_path.exists() && checkout_path.is_dir() {
            fs::remove_dir_all(checkout_path).map_err(|e| {
                eprintln!("Failed to clean checkout directory: {:?}", e);
                e
            })?;
            eprintln!("Successfully cleaned {}", checkout_dir.display());
        } else {
            eprintln!("No checkout directory found.");
        }
    }

    // Clean the .bender directory
    let bender_dir = path.join(".bender");
    if bender_dir.exists() && bender_dir.is_dir() {
        fs::remove_dir_all(&bender_dir).map_err(|e| {
            eprintln!("Failed to clean .bender directory: {:?}", e);
            e
        })?;
        eprintln!("Successfully cleaned .bender directory.");
    }

    // Clean the Bender.lock file
    let bender_lock = path.join("Bender.lock");
    if bender_lock.exists() && bender_lock.is_file() && all {
        fs::remove_file(&bender_lock).map_err(|e| {
            eprintln!("Failed to remove Bender.lock file: {:?}", e);
            e
        })?;
        eprintln!("Successfully removed Bender.lock file.");
    }

    Ok(())
}
