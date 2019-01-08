// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

#![allow(dead_code)]

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_yaml;
extern crate serde_json;

extern crate futures;
extern crate futures_cpupool;
extern crate tokio_core;
extern crate tokio_process;

extern crate clap;
extern crate dirs;
extern crate semver;
extern crate blake2;
extern crate typed_arena;

#[macro_use]
pub mod error;
pub mod util;
pub mod cli;
pub mod config;
pub mod sess;
pub mod resolver;
pub mod git;
pub mod cmd;
pub mod src;
pub mod target;
pub mod future_throttle;

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
