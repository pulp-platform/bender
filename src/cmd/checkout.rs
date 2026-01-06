// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `checkout` subcommand.

use clap::Args;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::util::fmt_duration;

/// Checkout all dependencies referenced in the Lock file
#[derive(Args, Debug)]
pub struct CheckoutArgs {
    /// Force update of dependencies in a custom checkout_dir. Please use carefully to avoid losing work.
    #[arg(long)]
    pub force: bool,
}

/// Execute the `checkout` subcommand.
pub fn run(sess: &Session, args: &CheckoutArgs) -> Result<()> {
    run_plain(sess, args.force, &[])
}

/// Execute a checkout (for the `checkout` subcommand).
pub fn run_plain(sess: &Session, force: bool, update_list: &[String]) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let start_time = std::time::Instant::now();
    let _srcs = rt.block_on(io.sources(forcibly, update_list))?;
    let num_dependencies = io.sess.packages().iter().flatten().count();
    infoln!(
        "{} {} dependencies {}",
        dim!("Checked out"),
        num_dependencies,
        dim!(fmt_duration(start_time.elapsed()))
    );

    Ok(())
}
