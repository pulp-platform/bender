// Copyright (c) 2024 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! The `clean` subcommand.

use clap::{Arg, ArgAction, Command};

use crate::config::Dependency;
use crate::error::*;
use crate::sess::Session;

/// Assemble the `clean` subcommand.
pub fn new() -> Command {
    Command::new("clean")
        .about("Clean the bender dependencies and the Lock file")
        .arg(
            Arg::new("force")
                .short('f')
                .long("force")
                .help("Force clean also the overrides specified in `Bender.local` files")
                .action(ArgAction::SetTrue),
        )
}

/// Execute the `clean` subcommand.
pub fn run(sess: &Session, force_clean: bool) -> Result<()> {
    // Remove the database i.e. the `.bender` directory
    if sess.config.database.exists() {
        std::fs::remove_dir_all(&sess.config.database)?;
    }

    // Remove the lock file
    if sess.root.join("Bender.lock").exists() {
        std::fs::remove_file(sess.root.join("Bender.lock"))?;
    }

    // Dependency overrides that are paths are not removed by default,
    // since they might contain uncommitted changes. The user is warned
    // about this and can force the removal with the `--force` flag.
    for (name, dep) in sess.config.overrides.iter() {
        match (dep, force_clean) {
            (Dependency::Path(_), false) => {
                warnln!("Override for {} is a path, will not be removed", name)
            }
            (Dependency::Path(path), true) => {
                debugln!("Removing override for {} at {}", name, path.display());
                std::fs::remove_dir_all(path)?;
            }
            _ => (),
        }
    }
    Ok(())
}
