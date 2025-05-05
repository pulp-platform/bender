// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A collection of subcommands.
//!
//! This module implements the subcommands of the command line tool.

#![deny(missing_docs)]

pub mod checkout;
pub mod clean;
pub mod clone;
pub mod completion;
pub mod config;
pub mod fusesoc;
pub mod init;
pub mod packages;
pub mod parents;
pub mod path;
pub mod script;
pub mod sources;
pub mod update;
pub mod vendor;
