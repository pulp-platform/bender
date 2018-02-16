// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;

use clap::{App, SubCommand, ArgMatches};
use tokio_core::reactor::Core;
use serde_yaml;

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
    let srcs = core.run(io.sources())?;
    let result = {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        serde_yaml::to_writer(handle, &srcs)
    };
    result.map_err(|cause| Error::chain(
        "Failed to serialize source file manifest.",
        cause
    ))
}
