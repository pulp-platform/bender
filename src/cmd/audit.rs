// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `audit` subcommand.

use std::collections::HashMap;
use std::io::Write;

use clap::Args;
use futures::future::join_all;
use miette::IntoDiagnostic as _;
use semver::VersionReq;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::Result;
use crate::cmd::parents::get_parent_array;
use crate::sess::{DependencyVersions, Session, SessionIo};

/// Get information about version conflicts and possible updates.
#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Only show packages that can be updated.
    #[arg(long)]
    pub only_update: bool,

    /// Force fetch of git dependencies.
    #[arg(short, long)]
    pub fetch: bool,

    /// Ignore URL conflicts when auditing.
    #[arg(long)]
    pub ignore_url_conflict: bool,
}

/// Execute the `audit` subcommand.
pub fn run(sess: &Session, args: &AuditArgs) -> Result<()> {
    let rt = Runtime::new().into_diagnostic()?;
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
                    io_ref.dependency_versions(
                        *pkg,
                        if args.fetch { Some(true) } else { None },
                        None
                    )
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

    let name_width = pkgs
        .iter()
        .map(|pkg| sess.dependency_name(**pkg).len())
        .max()
        .unwrap_or(10);

    for pkg in pkgs {
        let pkg_name = sess.dependency_name(*pkg);
        let parent_array = get_parent_array(sess, &rt, &io, pkg_name, false)?;
        let current_version = sess.dependency(*pkg).version.clone();
        let current_version_unwrapped = current_version
            .clone()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let current_revision = sess.dependency(*pkg).revision.clone();
        let current_revision_unwrapped = current_revision.clone().unwrap_or_default();
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
        let (default_version, url) = parent_array
            .values()
            .next()
            .map(|v| (v[0].clone(), v[1].clone()))
            .unwrap_or_else(|| ("".to_string(), "".to_string()));
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
            if parent[1] != url && !args.ignore_url_conflict {
                conflicting = true;
            }
        }
        let max_compatible = if version_req_exists {
            compatible_versions.iter().max()
        } else {
            None
        };

        audit_str.push_str(&format!("{:>1$}\t", pkg_name, name_width));

        // if conflicting:
        if conflicting {
            audit_str.push_str(" has a \x1B[31;1mConflict\x1B[m:\t-> check parents\n\t");
        }

        // if path:
        if current_version.is_none() && current_revision.is_none() {
            audit_str.push_str("    uses a \x1B[;1mPath\x1B[m\t\n");
        }

        // if rev:
        if (current_version.is_none() || !version_req_exists) && current_revision.is_some() {
            audit_str.push_str(&format!(
                "    uses a \x1B[31;1mHash\x1B[m:\t{}\n",
                current_revision_unwrapped
            ));
            if let Some(highest_version) = highest_version {
                audit_str.push_str(&format!(
                    "\t\x1B[31;1m\x1B[m\thighest version: {}\n",
                    highest_version
                ));
            }
        }

        // if up-to-date:
        if let Some(ref current_version) = current_version {
            if version_req_exists {
                if let Some(highest_version) = highest_version {
                    if *highest_version == *current_version && !args.only_update {
                        audit_str.push_str(&format!(
                            "  is \x1B[32;1mUp-to-date\x1B[m:\t@ {}\n",
                            current_version_unwrapped
                        ));
                    }
                }
            }
        }

        // if not up-to-date but newest compatible:
        if let Some(ref current_version) = current_version {
            if version_req_exists {
                if let Some(max_compatible) = max_compatible {
                    if *max_compatible > *current_version {
                        audit_str.push_str(&format!(
                            "can \x1B[32;1mAuto-update\x1B[m:\t{} -> {}\n",
                            current_version_unwrapped, max_compatible
                        ));
                    }
                }
            }
        }

        // if not up-to-date and newest incompatible:
        if let Some(current_version) = current_version {
            if version_req_exists {
                if let Some(highest_version) = highest_version {
                    if *highest_version > current_version
                        && (max_compatible.is_none() || *max_compatible.unwrap() < *highest_version)
                    {
                        audit_str.push_str(&format!(
                            "     can \x1B[33;1mUpdate\x1B[m:\t{} -> {}\n",
                            current_version_unwrapped, highest_version
                        ));
                    }
                }
            }
        }
    }

    let mut tw = TabWriter::new(vec![]);
    write!(&mut tw, "{}", audit_str).unwrap();
    tw.flush().unwrap();
    print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());

    Ok(())
}
