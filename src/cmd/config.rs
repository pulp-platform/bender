// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `config` subcommand.

use std;

use clap::{App, SubCommand, ArgMatches};
use serde_json;

use error::*;
use sess::Session;

/// Assemble the `config` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("config")
        .about("Emit the configuration")
}

/// Execute the `config` subcommand.
pub fn run(sess: &Session, _matches: &ArgMatches) -> Result<()> {
    let result = {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        serde_json::to_writer(handle, sess.config)
    };
    result.map_err(|cause| Error::chain(
        "Failed to serialize configuration.",
        cause
    ))
}
