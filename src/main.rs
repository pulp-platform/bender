// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

#![allow(dead_code)]

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_yaml;

extern crate clap;
extern crate semver;

#[macro_use]
pub mod error;
pub mod util;
pub mod cli;
pub mod config;
pub mod sess;

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

// fn inner_main() -> Result<()> {
// 	// Parse the command line arguments.
// 	let matches = App::new("landa")
// 		.version("0.1.0")
// 		.author("Fabian Schuiki <fschuiki@iis.ee.ethz.ch>")
// 		.about("A dependency management and build tool for hardware projects.\n\nAttendez la crème.")
// 		.arg(Arg::with_name("dir")
// 			.short("d")
// 			.long("dir")
// 			.takes_value(true)
// 			.help("Sets a custom root working directory"))
// 		.subcommand(
// 			SubCommand::with_name("source-files")
// 				.about("shows the list of source files")
// 		)
// 		.get_matches();


// 	// Determine the root working directory, which has either been provided via
// 	// the -d/--dir switch, or by searching upwards in the file system
// 	// hierarchy.
// 	let root_dir = match matches.value_of("dir") {
// 		Some(d) => d.into(),
// 		None => {
// 			use std::os::unix::fs::MetadataExt;
// 			let mut path = PathBuf::from("");
// 			let limit_rdev = std::fs::metadata(path.join(".")).unwrap().dev();
// 			loop {
// 				let current = path.join(".");
// 				let rdev = std::fs::metadata(&current).unwrap().dev();
// 				if rdev != limit_rdev {
// 					print_error!("unable to find package root; stopping at filesystem boundary {:?}", current.canonicalize().unwrap());
// 					std::process::exit(1);
// 				}

// 				if current.canonicalize().unwrap() == Path::new("/") {
// 					print_error!("unable to find package root; stopping at filesystem root \"/\"");
// 					std::process::exit(1);
// 				}

// 				if path.join("ips_list.yml").exists() || path.join("src_files.yml").exists() || path.join("Landa.yml").exists() {
// 					break;
// 				} else {
// 					path.push("..");
// 				}
// 			}
// 			path
// 		}
// 	};


//     // Parse the manifest file.
//     let manifest: config::Manifest = {
//         use std::fs::File;
//         let file = File::open(root_dir.join("Landa.yml"))?;
//         serde_yaml::from_reader(file)?
//     };
//     println!("Manifest: {:#?}", manifest);


// 	// Create an instance of the root structure which represents the root
// 	// package within which the command was executed. This is where the top
// 	// level configuration files will be read from.
// 	let root = Root::new(root_dir)?;
// 	println!("will investigate lock file {:?}", root.lock_file());
// 	println!("root: {:#?}", root);

// 	return Ok(());


// 	// See which subcommand should be executed.
// 	if let Some(matches) = matches.subcommand_matches("source-files") {
// 		// TODO: Extract flag, domain, group, target options and resolve the
// 		// package with these options. This should yield a `ResolvedPackage`
// 		// after all dependencies have been fetched and resolved, or throw some
// 		// big error messages in case stuff goes wrong. From this package,
// 		// extract the list of source files that need to be compiled and dump it
// 		// to stdout, possibly in different formats.
// 		let resolved = resolve(root.package(), &ResolveConfig{}).unwrap();
// 		println!("resolved = {:?}", resolved);
// 	} else {
// 		// Default action...
// 	}

// 	Ok(())
// }
