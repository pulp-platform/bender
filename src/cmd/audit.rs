// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `audit` subcommand.

use std::collections::HashMap;
use std::io::Write;

use clap::{Arg, ArgAction, ArgMatches, Command};
use futures::future::join_all;
use semver::VersionReq;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::cmd::parents::get_parent_array;
use crate::error::*;
use crate::sess::{DependencyVersions, Session, SessionIo};

/// Assemble the `audit` subcommand.
pub fn new() -> Command {
    Command::new("audit")
        .about("Get information about version conflicts and possible updates.")
        .arg(
            Arg::new("only-update")
                .long("only-update")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Only show packages that can be updated."),
        )
        .arg(
            Arg::new("fetch")
                .long("fetch")
                .short('f')
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Force fetch of git dependencies."),
        )
}

/// Execute the `audit` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);

    let binding = sess.packages().clone();
    let pkgs = binding.iter().flatten().collect::<Vec<_>>();

    let io_ref = &io;
    let dep_versions = rt.block_on(async {
        let futures = pkgs
            .clone()
            .iter()
            .map(|&pkg| async move {
                futures::join!(
                    async { *pkg },
                    io_ref.dependency_versions(*pkg, matches.get_flag("fetch"))
                )
            })
            .collect::<Vec<_>>();
        join_all(futures).await
    });

    let dep_versions = dep_versions
        .into_iter()
        .map(|(k, v)| match v {
            Ok(val) => Ok((k, val)),
            Err(e) => Err(e),
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let mut audit_str = String::from("");

    for pkg in pkgs {
        let pkg_name = sess.dependency_name(*pkg);
        let parent_array = get_parent_array(sess, &rt, &io, pkg_name, false)?;
        let current_version = sess.dependency(*pkg).version.clone();
        let current_version_unwrapped = if current_version.is_some() {
            format!("{}", current_version.clone().unwrap())
        } else {
            "".to_string()
        };
        let current_revision = sess.dependency(*pkg).revision.clone();
        let current_revision_unwrapped = if current_revision.is_some() {
            current_revision.clone().unwrap().to_string()
        } else {
            "".to_string()
        };
        let available_versions = match dep_versions.get(pkg).unwrap().clone() {
            DependencyVersions::Git(versions) => versions.versions,
            _ => vec![],
        }
        .into_iter()
        .map(|(v, _)| v)
        .collect::<Vec<semver::Version>>();
        let highest_version = available_versions.iter().max();

        let mut conflicting = false;
        let mut version_req_exists = false;
        let mut compatible_versions = available_versions.clone();
        let default_version = parent_array
            .values()
            .next()
            .unwrap_or(&vec!["".to_string()])[0]
            .clone();
        let url = parent_array
            .values()
            .next()
            .unwrap_or(&vec!["".to_string(), "".to_string()])[1]
            .clone();
        for parent in parent_array.values() {
            match VersionReq::parse(&parent[0]) {
                Ok(parent_version) => {
                    compatible_versions.retain(|v| parent_version.matches(v));
                    version_req_exists = true;
                }
                Err(_) => {
                    if parent[0] != default_version {
                        conflicting = true;
                    }
                }
            }
            if parent[1] != url {
                conflicting = true;
            }
        }
        let max_compatible = if version_req_exists {
            compatible_versions.iter().max()
        } else {
            None
        };

        // if conflicting:
        if conflicting {
            audit_str.push_str(&format!(
                "\x1B[31;1m{:>12}:\x1B[m {} \t-> check parents\n",
                "Conflict", pkg_name
            ));
        }

        // if path:
        if current_version.is_none() && current_revision.is_none() {
            audit_str.push_str(&format!("\x1B[;1m{:>12}:\x1B[m {}\t\n", "Path", pkg_name));
        }

        // if rev:
        if (current_version.is_none() || !version_req_exists) && current_revision.is_some() {
            audit_str.push_str(&format!(
                "\x1B[31;1m{:>12}:\x1B[m {} \t{}\n",
                "Hash", pkg_name, current_revision_unwrapped
            ));
            if highest_version.is_some() {
                audit_str.push_str(&format!(
                    "\x1B[31;1m{:>12} \x1B[m highest: \t{}\n",
                    "",
                    highest_version.unwrap()
                ));
            }
        }

        // if up-to-date:
        if current_version.is_some()
            && version_req_exists
            && highest_version.is_some()
            && *highest_version.unwrap() == current_version.clone().unwrap()
            && !matches.get_flag("only-update")
        {
            audit_str.push_str(&format!(
                "\x1B[32;1m{:>12}:\x1B[m {} \t@ {}\n",
                "Up-to-date", pkg_name, current_version_unwrapped
            ));
        }

        // if not up-to-date but newest compatible:
        if current_version.is_some()
            && version_req_exists
            && max_compatible.is_some()
            && *max_compatible.unwrap() > current_version.clone().unwrap()
        {
            audit_str.push_str(&format!(
                "\x1B[32;1m{:>12}:\x1B[m {} \t{} -> {}\n",
                "Auto-update",
                pkg_name,
                current_version_unwrapped,
                max_compatible.unwrap()
            ));
        }

        // if not up-to-date and newest incompatible:
        if current_version.is_some()
            && version_req_exists
            && highest_version.is_some()
            && *highest_version.unwrap() > current_version.clone().unwrap()
            && (max_compatible.is_none() || *max_compatible.unwrap() < *highest_version.unwrap())
        {
            audit_str.push_str(&format!(
                "\x1B[33;1m{:>12}:\x1B[m {} \t{} -> {}\n",
                "Update",
                pkg_name,
                current_version_unwrapped,
                highest_version.unwrap()
            ));
        }
    }

    let mut tw = TabWriter::new(vec![]);
    write!(&mut tw, "{}", audit_str).unwrap();
    tw.flush().unwrap();
    print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());

    Ok(())
}
