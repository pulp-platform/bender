// Copyright (c) 2025 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `lint` subcommand.

use clap::{ArgMatches, Command};

use crate::error::*;

/// Assemble the `init` subcommand.
pub fn new() -> Command {
    Command::new("init").about("Initialize a Bender package")
}

/// Execute the `init` subcommand.
pub fn run(_matches: &ArgMatches) -> Result<()> {
    Ok(())
}
