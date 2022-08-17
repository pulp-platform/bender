// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;

use clap::{Arg, ArgMatches, Command};
use serde_json;
use std::collections::HashSet;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::SourceGroup;
use crate::target::{TargetSet, TargetSpec};

/// Assemble the `sources` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("sources")
        .about("Emit the source file manifest for the package")
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .help("Filter sources by target")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("flatten")
                .short('f')
                .long("flatten")
                .help("Flatten JSON struct"),
        )
        .arg(
            Arg::new("package")
                .short('p')
                .long("package")
                .help("Specify package to show sources for")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("no_deps")
                .short('n')
                .long("no-deps")
                .help("Exclude all dependencies, i.e. only top level or specified package(s)"),
        )
        .arg(
            Arg::new("exclude")
                .short('e')
                .long("exclude")
                .help("Specify package to exclude from sources")
                .takes_value(true)
                .multiple_occurrences(true),
        )
}

fn get_package_strings<I>(packages: I) -> HashSet<String>
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

    // Filter the sources by target.
    let targets = matches
        .values_of("target")
        .map(TargetSet::new)
        .unwrap_or_else(TargetSet::empty);
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
        });

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess,
        &matches
            .values_of("package")
            .map(get_package_strings)
            .unwrap_or_default(),
        &matches
            .values_of("exclude")
            .map(get_package_strings)
            .unwrap_or_default(),
        matches.is_present("no_deps"),
    );

    if matches.is_present("package")
        || matches.is_present("exclude")
        || matches.is_present("no_deps")
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
            });
    }

    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        if matches.is_present("flatten") {
            let srcs = srcs.flatten();
            serde_json::to_writer_pretty(handle, &srcs)
        } else {
            serde_json::to_writer_pretty(handle, &srcs)
        }
    };
    println!();
    result.map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause))
}
