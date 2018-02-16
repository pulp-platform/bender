// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use clap::{App, SubCommand, ArgMatches};
use tokio_core::reactor::Core;

use error::*;
use sess::{Session, SessionIo};

/// Assemble the `sources` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("sources")
        .about("Emit the source file manifest for the package")
}

/// Execute the `sources` subcommand.
pub fn run(sess: &Session, _matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());

    // Obtain the sources from the session.
    let srcs = core.run(io.sources())?;
    debugln!("sources: {:#?}", srcs);

    Ok(())
}
