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
            Arg::new("no-checkout")
                .long("no-checkout")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Prevents check out of dependency."),
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
    if !matches.get_flag("no-checkout") {
        debugln!("main: obtain checkouts {:?}", ids);
        let rt = Runtime::new()?;
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
    } else {
        let paths = ids
            .iter()
            .map(|&(_, id)| io.get_package_path(id))
            .collect::<Vec<_>>();
        for c in paths {
            if let Some(s) = c.to_str() {
                println!("{}", s);
            }
        }
    }
    Ok(())
}
