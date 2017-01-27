// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>
//
// Copyright (C) 2017 ETH Zurich
// All rights reserved.

#![allow(dead_code)]

extern crate clap;
extern crate yaml_rust;
extern crate semver;

#[macro_use]
mod errors;
mod legacy;
mod package;
mod resolve;
mod git;

use clap::{Arg, App, SubCommand};
use std::path::{Path, PathBuf};
use package::*;
use resolve::*;



fn main() {
	let matches = App::new("landa")
		.version("0.1.0")
		.author("Fabian Schuiki <fschuiki@iis.ee.ethz.ch>")
		.about("A dependency management and build tool for hardware projects.\n\nAttendez la crème.")
		.arg(Arg::with_name("dir")
			.short("d")
			.long("dir")
			.takes_value(true)
			.help("Sets a custom root working directory"))
		.subcommand(
			SubCommand::with_name("source-files")
				.about("shows the list of source files")
		)
		.get_matches();

	// Create the structure that represents the root repository where all the
	// configuration files will be read from.
	let root = Root::new(matches.value_of("dir").unwrap_or(""));

	// See which subcommand should be executed.
	if let Some(matches) = matches.subcommand_matches("source-files") {
		// TODO: Extract flag, domain, group, target options and resolve the
		// package with these options. This should yield a `ResolvedPackage`
		// after all dependencies have been fetched and resolved, or throw some
		// big error messages in case stuff goes wrong. From this package,
		// extract the list of source files that need to be compiled and dump it
		// to stdout, possibly in different formats.
		let resolved = resolve(root.package(), &ResolveConfig{}).unwrap();
		println!("resolved = {:?}", resolved);
	} else {
		// Default action...
	}
}



/// The root repository within which the tool was invoked. This is where the
/// relevant configuration files (Landa.yml, ips_list.yml, src_files.yml) shall
/// be searched.
struct Root {
	pkg: Package,
}

impl Root {
	fn new<P: Into<PathBuf>>(path: P) -> Root {
		Root {
			pkg: Package::new(path, None).unwrap(),
		}
	}

	fn path(&self) -> &Path {
		self.pkg.path()
	}

	fn scratch_path(&self) -> PathBuf {
		Path::new(self.path()).join(".landa")
	}

	fn package(&self) -> &Package {
		&self.pkg
	}
}
