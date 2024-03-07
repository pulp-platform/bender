// Copyright (c) 2017-2024 ETH Zurich
// Philipp Schilk <schilkp@ethz.ch>

//! The `completion` subcommand.

use std::io;

use crate::error::*;
use clap::{builder::PossibleValue, Arg, ArgMatches, Command};

/// Assemble the `completion` subcommand.
pub fn new() -> Command {
    Command::new("completion")
        .about("Emit shell completion script")
        .arg(
            Arg::new("completion_shell")
                .help("Shell completion script style")
                .required(true)
                .num_args(1)
                .value_name("SHELL")
                .value_parser([
                    PossibleValue::new("bash"),
                    PossibleValue::new("elvish"),
                    PossibleValue::new("fish"),
                    PossibleValue::new("powershell"),
                    PossibleValue::new("zsh"),
                ]),
        )
}

/// Execute the `completion` subcommand.
pub fn run(matches: &ArgMatches, app: &mut Command) -> Result<()> {
    let shell = matches.get_one::<String>("completion_shell").unwrap();
    let shell = match shell.as_str() {
        "bash" => clap_complete::Shell::Bash,
        "elvish" => clap_complete::Shell::Elvish,
        "fish" => clap_complete::Shell::Fish,
        "powershell" => clap_complete::Shell::PowerShell,
        "zsh" => clap_complete::Shell::Zsh,
        _ => unreachable!(),
    };
    clap_complete::generate(shell, app, "bender", &mut io::stdout());
    Ok(())
}
