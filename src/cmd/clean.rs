// Copyright (c) 2024 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! The `clean` subcommand.

use std::path::PathBuf;

use clap::{ArgMatches, Command};

use crate::error::*;

/// Assemble the `clean` subcommand.
pub fn new() -> Command {
  Command::new("clean").about("Clean the bender dependencies and the Lock file")
}

/// Execute the `clean` subcommand.
pub fn run(root_dir: &PathBuf, _matches: &ArgMatches) -> Result<()> {
  if root_dir.join("Bender.lock").exists() {
    std::fs::remove_file(root_dir.join("Bender.lock"))?;
  }
  if root_dir.join(".bender").exists() {
    std::fs::remove_dir_all(root_dir.join(".bender"))?;
  }
  Ok(())
}
