// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `audit` subcommand.

use std::collections::HashMap;
use std::io::Write;

use clap::Args;
use futures::future::join_all;
use miette::IntoDiagnostic as _;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::Result;
use crate::cmd::parents::get_parent_requirements;
use crate::sess::{DependencyConstraint, DependencyVersions, Session, SessionIo};

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
        let parents = get_parent_requirements(sess, &rt, &io, pkg_name)?;
        let current_version = sess.dependency(*pkg).version.clone();
        let current_version_unwrapped = current_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let current_revision = sess.dependency(*pkg).revision.clone();
        let current_revision_unwrapped = current_revision.as_deref().unwrap_or_default();
        // Align update suggestions with the namespace the dependency is
        // currently resolved under. When the current checkout is not a version
        // (e.g. a path or revision), fall back to the default `v` namespace.
        let current_prefix = if current_version.is_some() {
            sess.dependency(*pkg)
                .version_prefix
                .as_deref()
                .unwrap_or(crate::config::DEFAULT_VERSION_PREFIX)
        } else {
            crate::config::DEFAULT_VERSION_PREFIX
        };
        // Every version on this package's lines comes from `current_prefix`, so name the
        // namespace once rather than prefixing each number. Left off for the default `v`, which
        // keeps existing output unchanged.
        let namespace_note = match current_prefix {
            crate::config::DEFAULT_VERSION_PREFIX => String::new(),
            "" => "  (unprefixed namespace)".to_string(),
            prefix => format!("  (namespace `{}`)", prefix),
        };
        let available_versions = match dep_versions.get(pkg).unwrap() {
            DependencyVersions::Git(versions) => versions
                .versions
                .iter()
                .filter(|tv| tv.prefix == current_prefix)
                .map(|tv| tv.version.clone())
                .collect(),
            _ => vec![],
        };
        let highest_version = available_versions.iter().max();

        let mut conflicting = false;
        let mut version_req_exists = false;
        let mut compatible_versions = available_versions.clone();
        let (default_constraint, url) = parents
            .values()
            .next()
            .map(|p| (Some(p.constraint.clone()), p.source.clone()))
            .unwrap_or((None, String::new()));
        for parent in parents.values() {
            // The namespace is already fixed by resolution, so only the requirement matters here.
            match &parent.constraint {
                DependencyConstraint::Version { req, .. } => {
                    compatible_versions.retain(|v| req.matches(v));
                    version_req_exists = true;
                }
                other => {
                    if Some(other) != default_constraint.as_ref() {
                        conflicting = true;
                    }
                }
            }
            if parent.source != url && !args.ignore_url_conflict {
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
                    "\t\x1B[31;1m\x1B[m\thighest version: {}{}\n",
                    highest_version, namespace_note
                ));
            }
        }

        // if up-to-date:
        if let Some(ref current_version) = current_version
            && version_req_exists
            && let Some(highest_version) = highest_version
            && *highest_version == *current_version
            && !args.only_update
        {
            audit_str.push_str(&format!(
                "  is \x1B[32;1mUp-to-date\x1B[m:\t@ {}{}\n",
                current_version_unwrapped, namespace_note
            ));
        }

        // if not up-to-date but newest compatible:
        if let Some(ref current_version) = current_version
            && version_req_exists
            && let Some(max_compatible) = max_compatible
            && *max_compatible > *current_version
        {
            audit_str.push_str(&format!(
                "can \x1B[32;1mAuto-update\x1B[m:\t{} -> {}{}\n",
                current_version_unwrapped, max_compatible, namespace_note
            ));
        }

        // if not up-to-date and newest incompatible:
        if let Some(current_version) = current_version
            && version_req_exists
            && let Some(highest_version) = highest_version
            && *highest_version > current_version
            && (max_compatible.is_none() || *max_compatible.unwrap() < *highest_version)
        {
            audit_str.push_str(&format!(
                "     can \x1B[33;1mUpdate\x1B[m:\t{} -> {}{}\n",
                current_version_unwrapped, highest_version, namespace_note
            ));
        }
    }

    let mut tw = TabWriter::new(vec![]);
    write!(&mut tw, "{}", audit_str).unwrap();
    tw.flush().unwrap();
    print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());

    Ok(())
}
