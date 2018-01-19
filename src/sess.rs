// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A command line session.

#![deny(missing_docs)]

use std::path::Path;
use config::{Manifest, Config};

/// A session on the command line.
///
/// Contains all the information that is iteratively being gathered and
/// generated as a command on the command line is executed.
#[derive(Debug)]
pub struct Session<'ctx> {
    /// The path of the package within which the tool was executed.
    pub root: &'ctx Path,
    /// The manifest of the root package.
    pub manifest: &'ctx Manifest,
    /// The tool configuration.
    pub config: &'ctx Config,
}

impl<'ctx> Session<'ctx> {
    /// Create a new session.
    pub fn new(root: &'ctx Path, manifest: &'ctx Manifest, config: &'ctx Config) -> Session<'ctx> {
        Session {
            root: root,
            manifest: manifest,
            config: config,
        }
    }
}
