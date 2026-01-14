// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;
use std::collections::BTreeSet;
use std::io::Write;

use clap::{ArgAction, Args};
use futures::future::join_all;
use indexmap::IndexSet;
use serde_json;
use tokio::runtime::Runtime;

use crate::config::{Dependency, Validate};
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::target::{TargetSet, TargetSpec};

/// Emit the source file manifest for the package
#[derive(Args, Debug)]
pub struct SourcesArgs {
    /// Filter sources by target
    #[arg(short, long, action = ArgAction::Append)]
    pub target: Vec<String>,

    /// Flatten JSON struct
    #[arg(short, long)]
    pub flatten: bool,

    /// Specify package to show sources for
    #[arg(short, long, action = ArgAction::Append)]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short, long)]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short, long, action = ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Add the `rtl` target to any fileset without a target specification
    #[arg(long)]
    pub assume_rtl: bool,

    /// Exports the raw internal source tree.
    #[arg(long)]
    pub raw: bool,

    /// Ignore passed targets
    #[arg(long)]
    pub ignore_passed_targets: bool,
}

fn get_package_strings<I>(packages: I) -> IndexSet<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    packages
        .into_iter()
        .map(|t| t.as_ref().to_string().to_lowercase())
        .collect()
}

/// Execute the `sources` subcommand.
pub fn run(sess: &Session, args: &SourcesArgs) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources(false, &[]))?;

    if args.raw {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        return serde_json::to_writer_pretty(handle, &srcs.flatten())
            .map_err(|err| Error::chain("Failed to serialize source file manifest.", err));
    }

    // Filter the sources by target.
    let targets = TargetSet::new(args.target.iter().map(|s| s.as_str()));

    if args.assume_rtl {
        srcs = srcs.assign_target("rtl".to_string());
    }

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess.manifest.package.name.to_string(),
        &get_package_strings(&args.package),
        &get_package_strings(&args.exclude),
        args.no_deps,
    );

    let (all_targets, used_packages) = get_passed_targets(
        sess,
        &rt,
        &io,
        &targets,
        packages,
        &get_package_strings(&args.package),
    )?;

    let targets = if args.ignore_passed_targets {
        targets
    } else {
        all_targets
    };

    let packages = if args.ignore_passed_targets {
        packages.clone()
    } else {
        used_packages
    };

    srcs = srcs.filter_targets(&targets).unwrap_or_default();

    srcs = srcs.filter_packages(&packages).unwrap_or_default();

    srcs = srcs.validate("", false)?;

    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        if args.flatten {
            let srcs = srcs.flatten();
            serde_json::to_writer_pretty(handle, &srcs)
        } else {
            serde_json::to_writer_pretty(handle, &srcs)
        }
    };
    let _ = writeln!(std::io::stdout(),);
    result.map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause))
}

/// Get the targets passed to dependencies from calling packages.
pub fn get_passed_targets(
    sess: &Session,
    rt: &Runtime,
    io: &SessionIo,
    global_targets: &TargetSet,
    used_packages: &IndexSet<String>,
    required_packages: &IndexSet<String>,
) -> Result<(TargetSet, IndexSet<String>)> {
    let mut global_targets = global_targets.clone();
    let mut required_packages = required_packages.clone();
    if used_packages.contains(&sess.manifest.package.name) {
        required_packages.insert(sess.manifest.package.name.clone());
        sess.manifest
            .dependencies
            .iter()
            .for_each(|(name, dep)| match dep {
                Dependency::Version(filter, _, tgts)
                | Dependency::Path(filter, _, tgts)
                | Dependency::GitRevision(filter, _, _, tgts)
                | Dependency::GitVersion(filter, _, _, tgts) => {
                    for t in tgts {
                        if TargetSpec::All(BTreeSet::from([filter.clone(), t.target.clone()]))
                            .matches(&global_targets.reduce_for_dependency(name))
                        {
                            global_targets.insert(format!("{}:{}", name, t.pass));
                        }
                    }
                    if filter.matches(&global_targets.reduce_for_dependency(name)) {
                        required_packages.insert(name.clone());
                    }
                }
            })
    };
    for pkgs in sess.packages().iter().rev() {
        let manifests = rt
            .block_on(join_all(
                pkgs.iter()
                    .map(|pkg| io.dependency_manifest(*pkg, false, &[])),
            ))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        manifests.into_iter().flatten().for_each(|manifest| {
            let pkg_name = &manifest.package.name;
            if used_packages.contains(pkg_name) && required_packages.contains(pkg_name) {
                manifest
                    .dependencies
                    .iter()
                    .for_each(|(name, dep)| match dep {
                        Dependency::Version(filter, _, tgts)
                        | Dependency::Path(filter, _, tgts)
                        | Dependency::GitRevision(filter, _, _, tgts)
                        | Dependency::GitVersion(filter, _, _, tgts) => {
                            for t in tgts {
                                if TargetSpec::All(BTreeSet::from([
                                    filter.clone(),
                                    t.target.clone(),
                                ]))
                                .matches(&global_targets.reduce_for_dependency(pkg_name))
                                {
                                    global_targets.insert(format!("{}:{}", name, t.pass));
                                }
                            }
                            if filter.matches(&global_targets.reduce_for_dependency(pkg_name)) {
                                required_packages.insert(name.clone());
                            }
                        }
                    })
            };
        });
    }

    Ok((
        global_targets,
        required_packages
            .intersection(used_packages)
            .cloned()
            .collect(),
    ))
}
