// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `config` subcommand.

use std;
use std::io::Write;

use serde_json;

use crate::error::*;
use crate::sess::Session;

/// Execute the `config` subcommand.
pub fn run(sess: &Session) -> Result<()> {
    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        serde_json::to_writer_pretty(handle, sess.config)
    };
    let _ = writeln!(std::io::stdout(),);
    result.map_err(|cause| Error::chain("Failed to serialize configuration.", cause))
}
