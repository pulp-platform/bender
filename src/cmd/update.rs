// Copyright (c) 2024 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `update` subcommand.

use std::collections::BTreeMap;
use std::io::Write;

use clap::{Arg, ArgAction, ArgMatches, Command};
use tabwriter::TabWriter;

use crate::cmd;
use crate::config::{Locked, LockedPackage};
use crate::error::*;
use crate::lockfile::*;
use crate::resolver::DependencyResolver;
use crate::sess::Session;

/// Assemble the `update` subcommand.
pub fn new() -> Command {
    Command::new("update")
        .about("Update the dependencies")
        .arg(
            Arg::new("fetch")
                .short('f')
                .long("fetch")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("forces fetch of git dependencies"),
        )
        .arg(
            Arg::new("no-checkout")
                .long("no-checkout")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Disables checkout of dependencies"),
        )
        .arg(
            Arg::new("ignore-checkout-dir")
                .long("ignore-checkout-dir")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Overwrites modified dependencies in `checkout_dir` if specified"),
        )
}

/// Execute the `update` subcommand.
pub fn setup(matches: &ArgMatches) -> Result<bool> {
    let force_fetch = matches.get_flag("fetch");
    if matches.get_flag("local") && matches.get_flag("fetch") {
        warnln!("As --local argument is set for bender command, no fetching will be performed.");
    }
    Ok(force_fetch)
}

/// Execute an update (for the `update` subcommand).
pub fn run<'ctx>(
    matches: &ArgMatches,
    sess: &'ctx Session<'ctx>,
    existing: Option<&'ctx Locked>,
) -> Result<Locked> {
    let ignore_checkout_dir = matches.get_flag("ignore-checkout-dir");
    run_plain(ignore_checkout_dir, sess, existing)
}

/// Execute an update (for the `update` subcommand or because no lockfile exists).
pub fn run_plain<'ctx>(
    ignore_checkout_dir: bool,
    sess: &'ctx Session<'ctx>,
    existing: Option<&'ctx Locked>,
) -> Result<Locked> {
    if sess.manifest.frozen {
        return Err(Error::new(format!(
            "Refusing to update dependencies because the package is frozen.
            Remove the `frozen: true` from {:?} to proceed; there be dragons.",
            sess.root.join("Bender.yml")
        )));
    }
    debugln!(
        "main: lockfile {:?} outdated",
        sess.root.join("Bender.lock")
    );
    let res = DependencyResolver::new(sess);
    let locked_new = res.resolve(existing, ignore_checkout_dir)?;
    let update_map: BTreeMap<String, (Option<LockedPackage>, Option<LockedPackage>)> = locked_new
        .packages
        .iter()
        .filter_map(|(name, dep)| {
            if let Some(existing) = existing {
                if let Some(existing_dep) = existing.packages.get(name) {
                    if existing_dep.revision != dep.revision {
                        Some((
                            name.clone(),
                            (Some(existing_dep.clone()), Some(dep.clone())),
                        ))
                    } else {
                        None
                    }
                } else {
                    Some((name.clone(), (None, Some(dep.clone()))))
                }
            } else {
                Some((name.clone(), (None, Some(dep.clone()))))
            }
        })
        .collect();
    let removed_map: BTreeMap<String, (Option<LockedPackage>, Option<LockedPackage>)> =
        if let Some(existing_unwrapped) = existing {
            existing_unwrapped
                .packages
                .iter()
                .filter_map(|(name, dep)| {
                    if !locked_new.packages.contains_key(name) {
                        Some((name.clone(), (Some(dep.clone()), None)))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            BTreeMap::new()
        };
    let update_map: BTreeMap<String, (Option<LockedPackage>, Option<LockedPackage>)> =
        update_map.into_iter().chain(removed_map).collect();
    let mut update_str = String::from("");
    for (name, (existing_dep, new_dep)) in update_map.clone() {
        update_str.push_str(&format!("\x1B[32;1m{:>12}\x1B[0m {}:\t", "Updating", name));
        if let Some(existing_dep) = existing_dep {
            update_str.push_str(
                &existing_dep
                    .version
                    .unwrap_or(existing_dep.revision.unwrap_or("path".to_string())),
            );
        }
        update_str.push_str("\t-> ");
        if let Some(new_dep) = new_dep {
            update_str.push_str(
                &new_dep
                    .version
                    .unwrap_or(new_dep.revision.unwrap_or("path".to_string())),
            );
        }
        update_str.push('\n');
    }
    let mut tw = TabWriter::new(vec![]);
    write!(&mut tw, "{}", update_str).unwrap();
    tw.flush().unwrap();
    println!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());
    write_lockfile(&locked_new, &sess.root.join("Bender.lock"), sess.root)?;
    Ok(locked_new)
}

/// Execute the final checkout (if not disabled).
pub fn run_final<'ctx>(sess: &'ctx Session<'ctx>, matches: &ArgMatches) -> Result<()> {
    if matches.get_flag("no-checkout") {
        Ok(())
    } else {
        cmd::checkout::run(sess, matches, true)
    }
}
