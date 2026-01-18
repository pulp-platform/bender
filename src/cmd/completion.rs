// Copyright (c) 2017-2024 ETH Zurich
// Philipp Schilk <schilkp@ethz.ch>

//! The `completion` subcommand.

use std::io;

use clap::{Args, Command};
use clap_complete::{generate, Shell};
use miette::Result;

/// Emit shell completion script
#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// Shell completion script style
    #[arg(value_enum)]
    pub shell: Shell,
}

/// Execute the `completion` subcommand.
pub fn run(args: &CompletionArgs, cmd: &mut Command) -> Result<()> {
    generate(args.shell, cmd, "bender", &mut io::stdout());
    Ok(())
}
