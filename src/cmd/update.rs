// Copyright (c) 2024 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `update` subcommand.

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::config::Locked;
use crate::error::*;
use crate::lockfile::*;
use crate::resolver::DependencyResolver;
use crate::sess::Session;

/// Assemble the `update` subcommand.
pub fn new() -> Command {
    Command::new("update")
        .about("Update the dependencies")
        .arg(
            Arg::new("fetch")
                .short('f')
                .long("fetch")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("forces fetch of git dependencies"),
        )
        .arg(
            Arg::new("no-checkout")
                .long("no-checkout")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Disables checkout of dependencies"),
        )
        .arg(
            Arg::new("ignore-checkout-dir")
                .long("ignore-checkout-dir")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Overwrites modified dependencies in `checkout_dir` if specified"),
        )
}

/// Execute the `update` subcommand.
pub fn setup(matches: &ArgMatches) -> Result<bool> {
    let force_fetch = matches.get_flag("fetch");
    if matches.get_flag("local") && matches.get_flag("fetch") {
        warnln!("As --local argument is set for bender command, no fetching will be performed.");
    }
    Ok(force_fetch)
}

/// Execute an update (for the `update` subcommand or because no lockfile exists).
pub fn run<'ctx>(
    matches: &ArgMatches,
    sess: &'ctx Session<'ctx>,
    existing: Option<&'ctx Locked>,
) -> Result<Locked> {
    if sess.manifest.frozen {
        return Err(Error::new(format!(
            "Refusing to update dependencies because the package is frozen.
            Remove the `frozen: true` from {:?} to proceed; there be dragons.",
            sess.root.join("Bender.yml")
        )));
    }
    debugln!(
        "main: lockfile {:?} outdated",
        sess.root.join("Bender.lock")
    );
    let res = DependencyResolver::new(sess);
    let locked_new = res.resolve(existing, matches.get_flag("ignore-checkout-dir"))?;
    write_lockfile(&locked_new, &sess.root.join("Bender.lock"), sess.root)?;
    Ok(locked_new)
}
