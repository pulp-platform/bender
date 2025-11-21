// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `snapshot` subcommand.

use clap::{Arg, ArgAction, ArgMatches, Command};
use std::process::Command as SysCommand;
use tokio::runtime::Runtime;

use crate::config::{Dependency, Locked, LockedSource};
use crate::error::*;
use crate::sess::{DependencySource, Session, SessionIo};

/// Assemble the `snapshot` subcommand.
pub fn new() -> Command {
    Command::new("snapshot")
        .about("Snapshot the cloned IPs from the working directory into the Bender.lock file")
        .arg(
            Arg::new("working_dir")
                .long("working-dir")
                .num_args(1)
                .required(false)
                .default_value("working_dir")
                .help("Working directory to snapshot dependencies from"),
        )
        .arg(
            Arg::new("no_skip")
                .long("no-skip")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Do not skip dependencies that are dirty"),
        )
        .arg(
            Arg::new("checkout")
                .long("checkout")
                .short('c')
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Checkout the dependencies snapshotted into the lockfile"),
        )
        .arg(
            Arg::new("forcibly")
                .long("force")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .requires("checkout")
                .help("Force update of dependencies in a custom checkout_dir. Please use carefully to avoid losing work."),
        )
}

/// Execute the `snapshot` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let working_dir = matches.get_one::<String>("working_dir").unwrap();

    let mut snapshot_list = Vec::new();

    // Loop through existing deps to find the ones that are overridden to the working directory
    for (name, dep) in sess.config.overrides.iter() {
        if let Dependency::Path(override_path) = dep {
            if override_path.starts_with(sess.root.join(working_dir)) {
                if let DependencySource::Path(dep_path) =
                    sess.dependency_source(sess.dependency_with_name(name)?)
                {
                    if dep_path == *override_path {
                        // check state, skip & warn if dirty
                        if !SysCommand::new(&sess.config.git)
                            .arg("status")
                            .arg("--porcelain")
                            .current_dir(&dep_path)
                            .output()?
                            .stdout
                            .is_empty()
                            && !matches.get_flag("no_skip")
                        {
                            warnln!(
                                "Skipping dirty dependency {}\
                                        \t use `--no-skip` to still snapshot.",
                                name
                            );
                            continue;
                        }

                        // Get the git url and hash of the dependency
                        let url = match String::from_utf8(
                            SysCommand::new(&sess.config.git)
                                .arg("remote")
                                .arg("get-url")
                                .arg("origin")
                                .current_dir(&dep_path)
                                .output()?
                                .stdout,
                        ) {
                            Ok(url) => url.trim_end_matches(&['\r', '\n'][..]).to_string(),
                            Err(_) => Err(Error::new("Failed to get git url.".to_string()))?,
                        };
                        let hash = match String::from_utf8(
                            SysCommand::new(&sess.config.git)
                                .arg("rev-parse")
                                .arg("HEAD")
                                .current_dir(&dep_path)
                                .output()?
                                .stdout,
                        ) {
                            Ok(hash) => hash.trim_end_matches(&['\r', '\n'][..]).to_string(),
                            Err(_) => Err(Error::new("Failed to get git hash.".to_string()))?,
                        };

                        eprintln!("Snapshotting {} at {} from {}", name, hash, url);

                        snapshot_list.push((name.clone(), url, hash));
                    }
                }
            }
        }
    }

    // Update the Bender.local to keep changes
    let local_path = sess.root.join("Bender.local");
    if local_path.exists() && !snapshot_list.is_empty() {
        let local_file_str = match std::fs::read_to_string(&local_path) {
            Err(why) => Err(Error::new(format!(
                "Reading Bender.local failed with msg:\n\t{}",
                why
            )))?,
            Ok(local_file_str) => local_file_str,
        };
        let mut new_str = String::new();
        if local_file_str.contains("overrides:") {
            let split = local_file_str.split('\n');
            let test = split.clone().next_back().unwrap().is_empty();
            for i in split {
                for (name, _, _) in &snapshot_list {
                    if i.contains(name) {
                        new_str.push('#');
                    }
                }
                new_str.push_str(i);
                new_str.push('\n');
                if i.contains("overrides:") {
                    for (name, url, hash) in &snapshot_list {
                        let dep_str = format!(
                            "  {}: {{ git: \"{}\", rev: \"{}\" }} # Temporary override by Bender using `bender snapshot` command\n",
                            name, url, hash
                        );
                        new_str.push_str(&dep_str);
                    }
                }
            }
            if test {
                // Ensure trailing newline is not duplicated
                new_str.pop();
            }
            if let Err(why) = std::fs::write(local_path, new_str) {
                Err(Error::new(format!(
                    "Writing new Bender.local failed with msg:\n\t{}",
                    why
                )))?
            }
            eprintln!("Bender.local updated with snapshots.");
        }
    }

    // Update the Bender.lock file with the new hash
    use std::fs::File;
    let file = File::open(sess.root.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot open lockfile {:?}.", sess.root), cause))?;
    let mut locked: Locked = serde_yaml_ng::from_reader(&file).map_err(|cause| {
        Error::chain(format!("Syntax error in lockfile {:?}.", sess.root), cause)
    })?;

    for (name, url, hash) in snapshot_list {
        let mut mod_package = locked.packages.get_mut(&name).unwrap().clone();
        mod_package.revision = Some(hash);
        mod_package.version = None;
        mod_package.source = LockedSource::Git(url);
        locked.packages.insert(name, mod_package);
    }

    let file = File::create(sess.root.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot create lockfile {:?}.", sess.root), cause))?;
    serde_yaml_ng::to_writer(&file, &locked)
        .map_err(|cause| Error::chain(format!("Cannot write lockfile {:?}.", sess.root), cause))?;

    if matches.get_flag("checkout") {
        sess.load_locked(&locked)?;

        let rt = Runtime::new()?;
        let io = SessionIo::new(sess);
        let _srcs = rt.block_on(io.sources(matches.get_flag("forcibly"), &[]))?;

        // TODO may need to update symlinks
    }

    Ok(())
}
