// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `checkout` subcommand.

use clap::{App, ArgMatches, SubCommand};
use tokio_core::reactor::Core;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `checkout` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("checkout").about("Checkout all dependencies referenced in the Lock file")
}

/// Execute the `checkout` subcommand.
pub fn run(sess: &Session, _matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());
    let _srcs = core.run(io.sources())?;

    Ok(())
}
