// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `path` subcommand.

use std::io::Write;

use clap::Args;
use futures::future::join_all;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Get the path to a dependency
#[derive(Args, Debug)]
pub struct PathArgs {
    /// Package names to get the path for
    #[arg(num_args(1..))]
    pub name: Vec<String>,

    /// Force check out of dependency.
    #[arg(long)]
    pub checkout: bool,
}

/// Execute the `path` subcommand.
pub fn run(sess: &Session, args: &PathArgs) -> Result<()> {
    let ids = args
        .name
        .iter()
        .map(|n| Ok((n, sess.dependency_with_name(&n.to_lowercase())?)))
        .collect::<Result<Vec<_>>>()?;

    let io = SessionIo::new(sess);

    // Get paths
    let paths = ids
        .iter()
        .map(|&(_, id)| io.get_package_path(id))
        .collect::<Vec<_>>();

    // Check out if requested or not done yet
    if args.checkout || !paths.iter().all(|p| p.exists()) {
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
            let _ = writeln!(std::io::stdout(), "{}", s);
        }
    }

    Ok(())
}
