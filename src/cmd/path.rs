// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `path` subcommand.

use clap::{Arg, ArgMatches, Command};
use futures::future::join_all;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `path` subcommand.
pub fn new() -> Command {
    Command::new("path")
        .about("Get the path to a dependency")
        .arg(
            Arg::new("name")
                .num_args(1..)
                .required(true)
                .help("Package names to get the path for"),
        )
}

/// Execute the `path` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);

    let ids = matches
        .get_many::<String>("name")
        .unwrap()
        .map(|n| Ok((n, sess.dependency_with_name(&n.to_lowercase())?)))
        .collect::<Result<Vec<_>>>()?;
    debugln!("main: obtain checkouts {:?}", ids);
    let checkouts = rt
        .block_on(join_all(
            ids.iter()
                .map(|&(_, id)| io.checkout(id))
                .collect::<Vec<_>>(),
        ))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    debugln!("main: checkouts {:#?}", checkouts);
    for c in checkouts {
        if let Some(s) = c.to_str() {
            println!("{}", s);
        }
    }
    Ok(())
}
