// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@student.ethz.ch>

//! The `parents` subcommand.

use clap::{App, Arg, ArgMatches, SubCommand};
use std::collections::HashMap;

use crate::error::*;
use crate::sess::Session;

/// Assemble the `parents` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("parents")
        .about("List packages calling this dependency")
        .arg(
            Arg::with_name("name")
                .required(true)
                .help("Package names to get the parents for"),
        )
}

/// Execute the `parents` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let dep = matches.value_of("name").unwrap();
    sess.dependency_with_name(dep)?;

    let parent_array = {
        let mut map = HashMap::<&str, &str>::new();
        if sess.manifest.dependencies.contains_key(dep) {
            map.insert(&sess.manifest.package.name, "version tbd");
        }
        for (&pkg, deps) in sess.graph().iter() {
            let pkg_name = sess.dependency_name(pkg);
            let dep_names = deps.iter().map(|&id| sess.dependency_name(id));
            for dep_name in dep_names {
                if dep == dep_name {
                    map.insert(pkg_name, "version tbd"); // TODO find each version reference
                }
            }
        }
        map
    };

    if parent_array.len() == 0 {
        println!("No parents found for {}.", dep);
    } else {
        println!("Parents found:");
        for (k, _v) in parent_array.iter() {
            println!("    {}", k);
        }
    }

    Ok(())
}
