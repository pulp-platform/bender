// Copyright (c) 2025 ETH Zurich

pub mod cli;
pub mod cmd;
pub mod config;
pub mod diagnostic;
pub mod git;
pub mod lockfile;
pub mod progress;
pub mod resolver;
pub mod sess;
pub mod src;
pub mod target;
pub mod util;

pub use miette::{bail, ensure, miette as err};

pub type Error = miette::Report;
pub type Result<T> = miette::Result<T>;
