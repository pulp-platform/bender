// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

#![allow(dead_code)]

#[macro_use]
extern crate serde;
extern crate serde_json;
extern crate serde_yaml;

extern crate futures;
extern crate tokio;

extern crate blake2;
extern crate clap;
extern crate dirs;
extern crate pathdiff;
extern crate semver;
extern crate typed_arena;

#[macro_use]
pub mod error;
pub mod cli;
pub mod cmd;
pub mod config;
// pub mod future_throttle;
pub mod git;
pub mod resolver;
pub mod sess;
pub mod src;
pub mod target;
pub mod util;

fn main() {
    match cli::main() {
        Ok(()) => {
            std::process::exit(0);
        }
        Err(e) => {
            errorln!("{}", e);
            std::process::exit(1);
        }
    }
}
