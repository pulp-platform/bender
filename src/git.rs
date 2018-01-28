// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A git repository and context for command execution.

#![deny(missing_docs)]

use std::path::PathBuf;
use sess::Session;

/// A git repository.
///
/// This struct is used to interact with git repositories on disk. It makes
/// heavy use of futures to execute the different tasks.
pub struct Git<'sess> {
    /// The path to the repository.
    pub path: PathBuf,
    /// The session within which commands will be executed.
    pub sess: &'sess Session<'sess>,
}

impl<'sess> Git<'sess> {
    /// Create a new git context.
    pub fn new(path: PathBuf, sess: &'sess Session<'sess>) -> Git<'sess> {
        Git {
            path: path,
            sess: sess,
        }
    }
}
