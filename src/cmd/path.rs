// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `path` subcommand.

use clap::{App, Arg, ArgMatches, SubCommand};
use futures::future;
use tokio_core::reactor::Core;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `path` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("path")
        .about("Get the path to a dependency")
        .arg(
            Arg::with_name("name")
                .multiple(true)
                .required(true)
                .help("Package names to get the path for"),
        )
}

/// Execute the `path` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());

    let ids = matches
        .values_of("name")
        .unwrap()
        .map(|n| Ok((n, sess.dependency_with_name(n)?)))
        .collect::<Result<Vec<_>>>()?;
    debugln!("main: obtain checkouts {:?}", ids);
    let checkouts = core.run(future::join_all(
        ids.iter()
            .map(|&(_, id)| io.checkout(id))
            .collect::<Vec<_>>(),
    ))?;
    debugln!("main: checkouts {:#?}", checkouts);
    for c in checkouts {
        if let Some(s) = c.to_str() {
            println!("{}", s);
        }
    }
    Ok(())
}
