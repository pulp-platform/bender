// Copyright (c) 2024 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `update` subcommand.

use std::collections::BTreeMap;
use std::io::Write;

use clap::Args;
use indexmap::IndexSet;
use tabwriter::TabWriter;

use crate::cmd;
use crate::config::{Locked, LockedPackage};
use crate::error::*;
use crate::lockfile::*;
use crate::resolver::DependencyResolver;
use crate::sess::Session;

/// Update the dependencies
#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// forces fetch of git dependencies
    #[arg(short, long)]
    pub fetch: bool,

    /// Disables checkout of dependencies
    #[arg(long)]
    pub no_checkout: bool,

    /// Overwrites modified dependencies in `checkout_dir` if specified
    #[arg(long)]
    pub ignore_checkout_dir: bool,

    /// Dependencies to update
    #[arg(num_args(1..))]
    pub dep: Option<Vec<String>>,

    /// Update requested dependencies recursively, i.e., including their dependencies
    #[arg(long, requires = "dep")]
    pub recursive: bool,
}

/// Execute the `update` subcommand.
pub fn setup(args: &UpdateArgs, local: bool) -> Result<bool> {
    if local && args.fetch {
        Warnings::LocalNoFetch.emit();
    }
    Ok(args.fetch)
}

/// Execute an update (for the `update` subcommand).
pub fn run<'ctx>(
    args: &UpdateArgs,
    sess: &'ctx Session<'ctx>,
    existing: Option<&'ctx Locked>,
) -> Result<(Locked, Vec<String>)> {
    let ignore_checkout_dir = args.ignore_checkout_dir;
    let mut keep_locked = match existing {
        Some(existing) => existing.packages.keys().collect(),
        None => IndexSet::new(),
    };

    let mut requested = match args.dep.as_ref() {
        Some(deps) => deps.iter().cloned().collect(),
        None => {
            keep_locked = IndexSet::new();
            IndexSet::new()
        }
    };

    for dep in requested.iter() {
        if !keep_locked.contains(&dep) {
            return Err(Error::new(format!(
                "Dependency {} is not present, cannot update {}.",
                dep, dep
            )));
        }
    }

    // Unlock dependencies recursively
    if args.recursive {
        if let Some(existing_locked) = existing {
            let mut nochange = true;
            while nochange {
                nochange = false;
                for dep in requested.clone().iter() {
                    for needed_dep in existing_locked.packages[dep].dependencies.iter() {
                        nochange |= requested.insert(needed_dep.clone());
                    }
                }
            }
        }
    }

    keep_locked.retain(|dep| !requested.contains(*dep));

    run_plain(ignore_checkout_dir, sess, existing, keep_locked)
}

/// Execute an update (for the `update` subcommand or because no lockfile exists).
pub fn run_plain<'ctx>(
    ignore_checkout_dir: bool,
    sess: &'ctx Session<'ctx>,
    existing: Option<&'ctx Locked>,
    keep_locked: IndexSet<&'ctx String>,
) -> Result<(Locked, Vec<String>)> {
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
    let locked_new = res.resolve(existing, ignore_checkout_dir, keep_locked)?;
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
    eprintln!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());
    write_lockfile(&locked_new, &sess.root.join("Bender.lock"), sess.root)?;
    Ok((locked_new, update_map.keys().cloned().collect()))
}

/// Execute the final checkout (if not disabled).
pub fn run_final<'ctx>(
    sess: &'ctx Session<'ctx>,
    args: &UpdateArgs,
    update_list: &[String],
) -> Result<()> {
    if args.no_checkout {
        Ok(())
    } else {
        cmd::checkout::run_plain(sess, args.ignore_checkout_dir, update_list)
    }
}
