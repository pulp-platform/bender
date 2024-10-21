//! The `clean` subcommand.

use clap::{Arg, ArgAction, ArgMatches, Command};
use std::path::Path;

use std::fs;

use crate::error::*;
use crate::sess::Session;

/// Assemble the `clean` subcommand.
pub fn new() -> Command {
    Command::new("clean")
        .about("Clean all bender related dependencies")
        .arg(
            Arg::new("all")
                .long("all")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Include Bender.lock in clean"),
        )
}

/// Execute the `clean` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches, path: &Path) -> Result<()> {
    println!("Cleaning all dependencies");

    // Clean the checkout directory
    if let Some(checkout_dir) = &sess.manifest.workspace.checkout_dir {
        let checkout_path = Path::new(checkout_dir);
        if checkout_path.exists() && checkout_path.is_dir() {
            fs::remove_dir_all(checkout_path).map_err(|e| {
                eprintln!("Failed to clean checkout directory: {:?}", e);
                e
            })?;
            println!("Successfully cleaned {}", checkout_dir.display());
        } else {
            println!("No checkout directory found.");
        }
    }

    // Clean the .bender directory
    let bender_dir = path.join(".bender");
    if bender_dir.exists() && bender_dir.is_dir() {
        fs::remove_dir_all(&bender_dir).map_err(|e| {
            eprintln!("Failed to clean .bender directory: {:?}", e);
            e
        })?;
        println!("Successfully cleaned .bender directory.");
    }

    // Clean the Bender.lock file
    let bender_lock = path.join("Bender.lock");
    if bender_lock.exists() && bender_lock.is_file() && matches.get_flag("all") {
        fs::remove_file(&bender_lock).map_err(|e| {
            eprintln!("Failed to remove Bender.lock file: {:?}", e);
            e
        })?;
        println!("Successfully removed Bender.lock file.");
    }

    Ok(())
}
