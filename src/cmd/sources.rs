// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;

use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use indexmap::IndexSet;
use serde_json;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::SourceGroup;
use crate::target::{TargetSet, TargetSpec};

/// Assemble the `sources` subcommand.
pub fn new() -> Command {
    Command::new("sources")
        .about("Emit the source file manifest for the package")
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .help("Filter sources by target")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("flatten")
                .short('f')
                .long("flatten")
                .help("Flatten JSON struct")
                .num_args(0)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("package")
                .short('p')
                .long("package")
                .help("Specify package to show sources for")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("no_deps")
                .short('n')
                .long("no-deps")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Exclude all dependencies, i.e. only top level or specified package(s)"),
        )
        .arg(
            Arg::new("exclude")
                .short('e')
                .long("exclude")
                .help("Specify package to exclude from sources")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("assume_rtl")
                .long("assume-rtl")
                .help("Add the `rtl` target to any fileset without a target specification")
                .num_args(0)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("raw")
                .long("raw")
                .help("Exports the raw internal source tree.")
                .num_args(0)
                .action(ArgAction::SetTrue),
        )
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
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources())?;

    if matches.get_flag("raw") {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        return serde_json::to_writer_pretty(handle, &srcs.flatten())
            .map_err(|err| Error::chain("Failed to serialize source file manifest.", err));
    }

    // Filter the sources by target.
    let targets = matches
        .get_many::<String>("target")
        .map(TargetSet::new)
        .unwrap_or_else(TargetSet::empty);

    if matches.get_flag("assume_rtl") {
        srcs = srcs.assign_target("rtl".to_string());
    }

    srcs = srcs
        .filter_targets(&targets)
        .unwrap_or_else(|| SourceGroup {
            package: Default::default(),
            independent: true,
            target: TargetSpec::Wildcard,
            include_dirs: Default::default(),
            export_incdirs: Default::default(),
            defines: Default::default(),
            files: Default::default(),
            dependencies: Default::default(),
            version: None,
        });

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess,
        &matches
            .get_many::<String>("package")
            .map(get_package_strings)
            .unwrap_or_default(),
        &matches
            .get_many::<String>("exclude")
            .map(get_package_strings)
            .unwrap_or_default(),
        matches.get_flag("no_deps"),
    );

    if matches.contains_id("package")
        || matches.contains_id("exclude")
        || matches.get_flag("no_deps")
    {
        srcs = srcs
            .filter_packages(packages)
            .unwrap_or_else(|| SourceGroup {
                package: Default::default(),
                independent: true,
                target: TargetSpec::Wildcard,
                include_dirs: Default::default(),
                export_incdirs: Default::default(),
                defines: Default::default(),
                files: Default::default(),
                dependencies: Default::default(),
                version: None,
            });
    }

    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        if matches.get_flag("flatten") {
            let srcs = srcs.flatten();
            serde_json::to_writer_pretty(handle, &srcs)
        } else {
            serde_json::to_writer_pretty(handle, &srcs)
        }
    };
    println!();
    result.map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause))
}
