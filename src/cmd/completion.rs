// Copyright (c) 2017-2024 ETH Zurich
// Philipp Schilk <schilkp@ethz.ch>

//! The `completion` subcommand.

use std::io;

use crate::error::*;
use clap::{Args, Command};
use clap_complete::{generate, Shell};

/// Assemble the `completion` subcommand.
// pub fn new() -> Command {
//     Command::new("completion")
//         .about("Emit shell completion script")
//         .arg(
//             Arg::new("completion_shell")
//                 .help("Shell completion script style")
//                 .required(true)
//                 .num_args(1)
//                 .value_name("SHELL")
//                 .value_parser([
//                     PossibleValue::new("bash"),
//                     PossibleValue::new("elvish"),
//                     PossibleValue::new("fish"),
//                     PossibleValue::new("powershell"),
//                     PossibleValue::new("zsh"),
//                 ]),
//         )
// }

/// Emit shell completion script
#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// Shell completion script style
    #[arg(action = clap::ArgAction::Set, value_enum)]
    pub shell: Shell,
}

/// Execute the `completion` subcommand.
pub fn run(args: &CompletionArgs, cmd: &mut Command) -> Result<()> {
    generate(args.shell, cmd, "bender", &mut io::stdout());
    Ok(())
}
