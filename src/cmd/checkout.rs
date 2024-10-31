// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `checkout` subcommand.

use clap::{Arg, ArgAction, ArgMatches, Command};
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `checkout` subcommand.
pub fn new() -> Command {
    Command::new("checkout")
    .about("Checkout all dependencies referenced in the Lock file")
    .arg(
        Arg::new("forcibly")
            .long("force")
            .num_args(0)
            .action(ArgAction::SetTrue)
            .help("Force update of dependencies in a custom checkout_dir. Please use carefully to avoid losing work."),
    )
}

/// Execute the `checkout` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches, forcibly: bool) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let _srcs = rt.block_on(io.sources(forcibly || matches.get_flag("forcibly")))?;

    Ok(())
}
