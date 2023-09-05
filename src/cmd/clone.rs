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
pub fn new() -> Command {
    Command::new("clone")
        .about("Clone dependency to a working directory")
        .arg(
            Arg::new("name")
                .required(true)
                .num_args(1)
                .help("Package name to clone to a working directory"),
        )
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .help("Relative directory to clone PKG into (default: working_dir)")
                .num_args(1)
                .default_value("working_dir"),
        )
}

/// Execute the `clone` subcommand.
pub fn run(sess: &Session, path: &Path, matches: &ArgMatches) -> Result<()> {
    let dep = &matches.get_one::<String>("name").unwrap().to_lowercase();
    sess.dependency_with_name(dep)?;

    let path_mod = matches.get_one::<String>("path").unwrap(); // TODO make this option for config in the Bender.yml file?

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
    if !path.join(path_mod).exists()
        && !SysCommand::new("mkdir")
            .arg(path_mod)
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    {
        Err(Error::new(format!("Creating dir {} failed", path_mod,)))?;
    }

    // Copy dependency to dir for proper workflow
    if path.join(path_mod).join(dep).exists() {
        println!("{} already has a directory in {}.", dep, path_mod);
        println!("Please manually ensure the correct checkout.");
    } else {
        let rt = Runtime::new()?;
        let io = SessionIo::new(sess);

        let ids = matches
            .get_many::<String>("name")
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
            Err(Error::new("git renaming remote origin failed".to_string()))?;
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
            Err(Error::new("git adding remote failed".to_string()))?;
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
                Err(Error::new("git fetch failed".to_string()))?;
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
            let split = local_file_str.split('\n');
            let test = split.clone().last().unwrap().is_empty();
            for i in split {
                if i.contains(dep) {
                    new_str.push('#');
                }
                new_str.push_str(i);
                new_str.push('\n');
                if i.contains("overrides:") {
                    new_str.push_str(&dep_str);
                }
            }
            if test {
                // Ensure trailing newline is not duplicated
                new_str.pop();
            }
        } else {
            new_str.push_str("overrides:\n");
            new_str.push_str(&dep_str);
            new_str.push_str(&local_file_str);
        }
        if let Err(why) = std::fs::write(local_path, new_str) {
            Err(Error::new(format!(
                "Writing new Bender.local failed with msg:\n\t{}",
                why
            )))?
        }
    } else if let Err(why) = std::fs::write(local_path, format!("overrides:\n{}", dep_str)) {
        Err(Error::new(format!(
            "Writing new Bender.local failed with msg:\n\t{}",
            why
        )))?
    };

    println!("{} dependency added to Bender.local", dep);

    // Update Bender.lock to enforce usage
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

    // Update any possible workspace symlinks
    for (link_path, pkg_name) in &sess.manifest.workspace.package_links {
        if pkg_name == dep {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, link_path);

            // Determine the checkout path for this package.
            let pkg_path = &path.join(path_mod).join(dep);
            let pkg_path = link_path
                .parent()
                .and_then(|path| pathdiff::diff_paths(pkg_path, path))
                .unwrap_or_else(|| pkg_path.into());

            // Check if there is something at the destination path that needs to be
            // removed.
            if link_path.exists() {
                let meta = link_path.symlink_metadata().map_err(|cause| {
                    Error::chain(
                        format!("Failed to read metadata of path {:?}.", link_path),
                        cause,
                    )
                })?;
                if !meta.file_type().is_symlink() {
                    warnln!(
                        "Skipping link to package {} at {:?} since there is something there",
                        pkg_name,
                        link_path
                    );
                    continue;
                }
                if link_path.read_link().map(|d| d != pkg_path).unwrap_or(true) {
                    debugln!("main: removing existing link {:?}", link_path);
                    std::fs::remove_file(link_path).map_err(|cause| {
                        Error::chain(
                            format!("Failed to remove symlink at path {:?}.", link_path),
                            cause,
                        )
                    })?;
                }
            }

            // Create the symlink if there is nothing at the destination.
            if !link_path.exists() {
                stageln!("Linking", "{} ({:?})", pkg_name, link_path);
                if let Some(parent) = link_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|cause| {
                        Error::chain(format!("Failed to create directory {:?}.", parent), cause)
                    })?;
                }
                let previous_dir = match link_path.parent() {
                    Some(parent) => {
                        let d = std::env::current_dir().unwrap();
                        std::env::set_current_dir(parent).unwrap();
                        Some(d)
                    }
                    None => None,
                };
                std::os::unix::fs::symlink(&pkg_path, link_path).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Failed to create symlink to {:?} at path {:?}.",
                            pkg_path, link_path
                        ),
                        cause,
                    )
                })?;
                if let Some(d) = previous_dir {
                    std::env::set_current_dir(d).unwrap();
                }
            }
            println!("{} symlink updated", dep);
        }
    }

    Ok(())
}
