// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `clone` subcommand.

use clap::{Arg, ArgMatches, Command};
use futures::future::join_all;
use std::path::Path;
use std::process::Command as SysCommand;
use tokio::runtime::Runtime;

use crate::config;
use crate::config::{Locked, LockedSource};
use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `clone` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("clone")
        .about("Clone dependency to a working directory")
        .arg(
            Arg::new("name")
                .required(true)
                .help("Package name to clone to a working directory"),
        )
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .help("Relative directory to clone PKG into (default: working_dir)")
                .takes_value(true),
        )
}

/// Execute the `clone` subcommand.
pub fn run(sess: &Session, path: &Path, matches: &ArgMatches) -> Result<()> {
    let dep = &matches.value_of("name").unwrap().to_lowercase();
    sess.dependency_with_name(dep)?;

    let path_mod = matches.value_of("path").unwrap_or_else(|| "working_dir"); // TODO make this option for config in the Bender.yml file?

    // Check current config for matches
    if sess.config.overrides.contains_key(dep) {
        match &sess.config.overrides[dep] {
            config::Dependency::Path(p) => {
                Err(Error::new(format!(
                    "Dependency `{}` already has a path override at\n\t{}\n\tPlease check Bender.local or .bender.yml",
                    dep,
                    p.to_str().unwrap()
                )))?;
            }
            _ => {
                println!("A non-path override is already present, proceeding anyways");
            }
        }
    }

    // Create dir
    if !path.join(path_mod).exists() {
        if !SysCommand::new("mkdir")
            .arg(path_mod)
            .current_dir(path)
            .status()
            .unwrap()
            .success()
        {
            Err(Error::new(format!("Creating dir {} failed", path_mod,)))?;
        }
    }

    // Copy dependency to dir for proper workflow
    if path.join(path_mod).join(dep).exists() {
        println!("{} already has a directory in {}.", dep, path_mod);
        println!("Please manually ensure the correct checkout.");
    } else {
        let rt = Runtime::new()?;
        let io = SessionIo::new(&sess);

        let ids = matches
            .values_of("name")
            .unwrap()
            .map(|n| Ok((n, sess.dependency_with_name(n)?)))
            .collect::<Result<Vec<_>>>()?;
        debugln!("main: obtain checkouts {:?}", ids);
        let checkouts = rt
            .block_on(join_all(
                ids.iter()
                    .map(|&(_, id)| io.checkout(id))
                    .collect::<Vec<_>>(),
            ))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        debugln!("main: checkouts {:#?}", checkouts);
        for c in checkouts {
            if let Some(s) = c.to_str() {
                let command = SysCommand::new("cp")
                    .arg("-rf")
                    .arg(s)
                    .arg(path.join(path_mod).join(dep).to_str().unwrap())
                    .status();
                if !command.unwrap().success() {
                    Err(Error::new(format!("Copying {} failed", dep,)))?;
                }
                // println!("{:?}", command);
            }
        }

        // rename and update git remotes for easier handling
        if !SysCommand::new(&sess.config.git)
            .arg("remote")
            .arg("rename")
            .arg("origin")
            .arg("source")
            .current_dir(path.join(path_mod).join(dep))
            .status()
            .unwrap()
            .success()
        {
            Err(Error::new(format!("git renaming remote origin failed")))?;
        }

        if !SysCommand::new(&sess.config.git)
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg(
                &sess
                    .dependency(sess.dependency_with_name(dep)?)
                    .source
                    .to_str(),
            )
            .current_dir(path.join(path_mod).join(dep))
            .status()
            .unwrap()
            .success()
        {
            Err(Error::new(format!("git adding remote failed")))?;
        }

        if !sess.local_only {
            if !SysCommand::new(&sess.config.git)
                .arg("fetch")
                .arg("--all")
                .current_dir(path.join(path_mod).join(dep))
                .status()
                .unwrap()
                .success()
            {
                Err(Error::new(format!("git fetch failed")))?;
            }
        } else {
            warnln!("fetch not performed due to --local argument.");
        }

        println!(
            "{} checkout added in {:?}",
            dep,
            path.join(path_mod).join(dep)
        );
    }

    // Rewrite Bender.local file to keep changes
    let local_path = path.join("Bender.local");
    let dep_str = format!(
        "  {}: {{ path: \"{}/{0}\" }} # Temporary override by Bender using `bender clone` command\n",
        dep, path_mod
    );
    if local_path.exists() {
        let local_file_str = match std::fs::read_to_string(&local_path) {
            Err(why) => Err(Error::new(format!(
                "Reading Bender.local failed with msg:\n\t{}",
                why
            )))?,
            Ok(local_file_str) => local_file_str,
        };
        let mut new_str = String::new();
        if local_file_str.contains("overrides:") {
            let split = local_file_str.split("\n");
            let test = split.clone().last().unwrap().is_empty();
            for i in split {
                if i.contains(dep) {
                    new_str.push('#');
                }
                new_str.push_str(i);
                new_str.push_str("\n");
                if i.contains("overrides:") {
                    new_str.push_str(&dep_str);
                }
            }
            if test {
                // Ensure trailing newline is not duplicated
                new_str.pop();
            }
        } else {
            new_str.push_str(&format!("overrides:\n{}", dep_str));
            new_str.push_str(&local_file_str);
        }
        match std::fs::write(local_path, new_str) {
            Err(why) => Err(Error::new(format!(
                "Writing new Bender.local failed with msg:\n\t{}",
                why
            )))?,
            Ok(_) => (),
        }
    } else {
        match std::fs::write(local_path, format!("overrides:\n{}", dep_str)) {
            Err(why) => Err(Error::new(format!(
                "Writing new Bender.local failed with msg:\n\t{}",
                why
            )))?,
            Ok(_) => (),
        };
    }

    println!("{} dependency added to Bender.local", dep);

    use std::fs::File;
    let file = File::open(path.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot open lockfile {:?}.", path), cause))?;
    let mut locked: Locked = serde_yaml::from_reader(&file)
        .map_err(|cause| Error::chain(format!("Syntax error in lockfile {:?}.", path), cause))?;

    let mut mod_package = locked.packages[dep].clone();
    mod_package.revision = None;
    mod_package.version = None;
    mod_package.source = LockedSource::Path(path.join(path_mod).join(dep));
    locked.packages.insert(dep.to_string(), mod_package);

    let file = File::create(path.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot create lockfile {:?}.", path), cause))?;
    serde_yaml::to_writer(&file, &locked)
        .map_err(|cause| Error::chain(format!("Cannot write lockfile {:?}.", path), cause))?;

    println!("Lockfile updated");

    Ok(())
}
