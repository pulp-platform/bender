// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `checkout` subcommand.

use clap::{ArgMatches, Command};
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `checkout` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("checkout").about("Checkout all dependencies referenced in the Lock file")
}

/// Execute the `checkout` subcommand.
pub fn run(sess: &Session, _matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let _srcs = rt.block_on(io.sources())?;

    Ok(())
}
