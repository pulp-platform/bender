// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `path` subcommand.

use clap::{Arg, ArgAction, ArgMatches, Command};
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
        .arg(
            Arg::new("checkout")
                .long("checkout")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Force check out of dependency."),
        )
}

/// Execute the `path` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let ids = matches
        .get_many::<String>("name")
        .unwrap()
        .map(|n| Ok((n, sess.dependency_with_name(&n.to_lowercase())?)))
        .collect::<Result<Vec<_>>>()?;

    let io = SessionIo::new(sess);

    // Get paths
    let paths = ids
        .iter()
        .map(|&(_, id)| io.get_package_path(id))
        .collect::<Vec<_>>();

    // Check out if requested or not done yet
    if matches.get_flag("checkout") || !paths.iter().all(|p| p.exists()) {
        debugln!("main: obtain checkouts {:?}", ids);
        let rt = Runtime::new()?;
        let checkouts = rt
            .block_on(join_all(
                ids.iter()
                    .map(|&(_, id)| io.checkout(id, false, &[]))
                    .collect::<Vec<_>>(),
            ))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        debugln!("main: checkouts {:#?}", checkouts);
    }

    // Print paths
    for c in paths {
        if let Some(s) = c.to_str() {
            println!("{}", s);
        }
    }

    Ok(())
}
