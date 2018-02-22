// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;

use clap::{App, Arg, SubCommand, ArgMatches};
use tokio_core::reactor::Core;
use serde_json;

use error::*;
use sess::{Session, SessionIo};
use target::TargetSet;

/// Assemble the `sources` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("sources")
        .about("Emit the source file manifest for the package")
        .arg(Arg::with_name("target")
            .short("t")
            .long("target")
            .help("Filter sources by target")
            .takes_value(true)
            .multiple(true)
            .number_of_values(1)
        )
}

/// Execute the `sources` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());
    let mut srcs = core.run(io.sources())?;
    if let Some(targets) = matches.values_of("target") {
        srcs = match srcs.filter_targets(&TargetSet::new(targets)) {
            Some(srcs) => srcs,
            None => {
                println!("{{}}");
                return Ok(());
            }
        };
    }
    let result = {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        serde_json::to_writer(handle, &srcs)
    };
    result.map_err(|cause| Error::chain(
        "Failed to serialize source file manifest.",
        cause
    ))
}
