// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;

use clap::{App, Arg, ArgMatches, SubCommand};
use serde_json;
use tokio_core::reactor::Core;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::SourceGroup;
use crate::target::{TargetSet, TargetSpec};

/// Assemble the `sources` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("sources")
        .about("Emit the source file manifest for the package")
        .arg(
            Arg::with_name("target")
                .short("t")
                .long("target")
                .help("Filter sources by target")
                .takes_value(true)
                .multiple(true)
                .number_of_values(1),
        )
        .arg(
            Arg::with_name("flatten")
                .short("f")
                .long("flatten")
                .help("Flatten JSON struct"),
        )
}

/// Execute the `sources` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());
    let srcs = core.run(io.sources())?;

    // Filter the sources by target.
    let targets = matches
        .values_of("target")
        .map(|t| TargetSet::new(t))
        .unwrap_or_else(|| TargetSet::empty());
    let srcs = srcs
        .filter_targets(&targets)
        .unwrap_or_else(|| SourceGroup {
            package: Default::default(),
            independent: true,
            target: TargetSpec::Wildcard,
            include_dirs: Default::default(),
            defines: Default::default(),
            files: Default::default(),
            dependencies: Default::default(),
        });

    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        if matches.is_present("flatten") {
            let srcs = srcs.flatten();
            serde_json::to_writer_pretty(handle, &srcs)
        } else {
            serde_json::to_writer_pretty(handle, &srcs)
        }
    };
    println!("");
    result.map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause))
}
