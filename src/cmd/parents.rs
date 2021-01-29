// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@student.ethz.ch>

//! The `parents` subcommand.

use clap::{App, Arg, ArgMatches, SubCommand};
use std::collections::HashMap;
use std::io::Write;
use tabwriter::TabWriter;
use tokio_core::reactor::Core;

use crate::error::*;
use crate::sess::DependencyConstraint;
use crate::sess::{Session, SessionIo};

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
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());

    let parent_array = {
        let mut map = HashMap::<String, String>::new();
        if sess.manifest.dependencies.contains_key(dep) {
            let dep_str = format!(
                "{}",
                DependencyConstraint::from(&sess.manifest.dependencies[dep])
            );
            map.insert(sess.manifest.package.name.clone(), dep_str);
            println!("Testing: {:?}", &sess.manifest.dependencies[dep]);
        }
        for (&pkg, deps) in sess.graph().iter() {
            let pkg_name = sess.dependency_name(pkg);
            let all_deps = deps.iter().map(|&id| sess.dependency(id));
            for current_dep in all_deps {
                if dep == current_dep.name.as_str() {
                    let dep_manifest = core.run(io.dependency_manifest(pkg)).unwrap().unwrap();
                    map.insert(
                        pkg_name.to_string(),
                        format!(
                            "{}",
                            DependencyConstraint::from(&dep_manifest.dependencies[dep])
                        ),
                    );
                }
            }
        }
        map
    };

    if parent_array.len() == 0 {
        println!("No parents found for {}.", dep);
    } else {
        println!("Parents found:");
        let mut res = String::from("");
        for (k, v) in parent_array.iter() {
            res.push_str(&format!("    {}\trequires: {}\n", k, v).to_string());
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", res).unwrap();
        tw.flush().unwrap();
        print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());
    }

    Ok(())
}
