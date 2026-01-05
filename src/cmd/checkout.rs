// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `checkout` subcommand.

use clap::{ArgAction, Args};
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Checkout all dependencies referenced in the Lock file
#[derive(Args, Debug)]
pub struct CheckoutArgs {
    /// Force update of dependencies in a custom checkout_dir. Please use carefully to avoid losing work.
    #[arg(long = "force", action = ArgAction::SetTrue)]
    pub forcibly: bool,
}

/// Execute the `checkout` subcommand.
pub fn run(sess: &Session, args: &CheckoutArgs) -> Result<()> {
    run_plain(sess, args.forcibly, &[])
}

/// Execute a checkout (for the `checkout` subcommand).
pub fn run_plain(sess: &Session, forcibly: bool, update_list: &[String]) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let _srcs = rt.block_on(io.sources(forcibly, update_list))?;

    Ok(())
}
