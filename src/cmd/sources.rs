// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `sources` subcommand.

use std;
use std::io::Write;

use clap::{ArgAction, Args};
use indexmap::{IndexMap, IndexSet};
use serde_json;
use tokio::runtime::Runtime;

use crate::config::Validate;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::target::TargetSet;

/// Emit the source file manifest for the package
#[derive(Args, Debug)]
pub struct SourcesArgs {
    /// Filter sources by target
    #[arg(short = 't', long = "target", action = ArgAction::Append)]
    pub target: Vec<String>,

    /// Flatten JSON struct
    #[arg(short = 'f', long = "flatten", action = ArgAction::SetTrue)]
    pub flatten: bool,

    /// Specify package to show sources for
    #[arg(short = 'p', long = "package", action = ArgAction::Append)]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short = 'n', long = "no-deps", action = ArgAction::SetTrue)]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short = 'e', long = "exclude", action = ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Add the `rtl` target to any fileset without a target specification
    #[arg(long = "assume-rtl", action = ArgAction::SetTrue)]
    pub assume_rtl: bool,

    /// Exports the raw internal source tree.
    #[arg(long = "raw", action = ArgAction::SetTrue)]
    pub raw: bool,

    /// Ignore passed targets
    #[arg(long, action = ArgAction::SetTrue)]
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

    srcs = srcs
        .filter_targets(&targets, !matches.get_flag("ignore-passed-targets"))
        .unwrap_or_default();

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess,
        &get_package_strings(&args.package),
        &get_package_strings(&args.exclude),
        args.no_deps,
    );

    if !args.package.is_empty() || !args.exclude.is_empty() || args.no_deps {
        srcs = srcs.filter_packages(packages).unwrap_or_default();
    }

    srcs = srcs.validate("", false, &sess.suppress_warnings)?;

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
