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
use crate::src::{SourceFile, SourceGroup};
use crate::target::{TargetSet, TargetSpec};

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use crate::filter::{filter_unused, FilterOptions};

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
            Arg::new("raw")
                .long("raw")
                .help("Exports the raw internal source tree.")
                .num_args(0)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("filter-unused")
                .long("filter-unused")
                .help("Filter unused SystemVerilog files via dependency tracing")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("rtl-top")
                .long("rtl-top")
                .help("Top RTL module filename stem")
                .num_args(1)
                .action(ArgAction::Append),
        )
        .arg(
            Arg::new("tb-top")
                .long("tb-top")
                .help("Testbench top stem(s) or pattern(s)")
                .num_args(1)
                .action(ArgAction::Append),
        )
        .arg(
            Arg::new("show-tree")
                .long("show-tree")
                .help("Print dependency tree for chosen start files")
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

fn norm(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn prune_group_by_used<'ctx>(
    mut g: SourceGroup<'ctx>,
    root: &Path,
    used: &HashSet<PathBuf>,
) -> SourceGroup<'ctx> {
    g.files = g
        .files
        .into_iter()
        .filter_map(|sf| match sf {
            SourceFile::File(p) => {
                let abs = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
                if used.contains(&norm(&abs)) {
                    Some(SourceFile::File(p))
                } else {
                    None
                }
            }
            SourceFile::Group(grp) => {
                let pruned = prune_group_by_used(*grp, root, used);
                if pruned.files.is_empty() { None } else { Some(SourceFile::Group(Box::new(pruned))) }
            }
        })
        .collect();
    g
}

/// Execute the `sources` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources(false, &[]))?;

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

    let want_flatten = matches.get_flag("flatten");
    if matches.get_flag("filter-unused") {
        // Build a temporary flat view to compute the reachable set.
        let flat_for_used = srcs.clone().flatten();

        let flat_files: Vec<(String, PathBuf)> = flat_for_used
            .iter()
            .flat_map(|sg| {
                let pkg_name = sg
                    .package
                    .unwrap_or(&sess.manifest.package.name)
                    .to_string();
                sg.files.iter().filter_map(move |f| {
                    if let SourceFile::File(p) = f {
                        let abs = if p.is_absolute() { p.to_path_buf() } else { sess.root.join(p) };
                        Some((pkg_name.clone(), norm(&abs)))
                    } else {
                        None
                    }
                })
            })
            .collect();

        let opts = FilterOptions {
            rtl_tops: matches
                .get_many::<String>("rtl-top")
                .map(|v| v.map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            tb_tops: matches
                .get_many::<String>("tb-top")
                .map(|v| v.map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            show_tree: matches.get_flag("show-tree"),
        };

        let used_btree: BTreeSet<PathBuf> = filter_unused(sess, &flat_files, &opts)?;
        let used: HashSet<PathBuf> = used_btree.into_iter().collect();

        if want_flatten {
            // Filter a flat view for output.
            let filtered_flat = flat_for_used
                .into_iter()
                .map(|mut sg| {
                    sg.files.retain(|f| {
                        if let SourceFile::File(p) = f {
                            let abs = if p.is_absolute() { p.to_path_buf() } else { sess.root.join(p) };
                            used.contains(&norm(&abs))
                        } else {
                            true
                        }
                    });
                    sg
                })
                .filter(|sg| !sg.files.is_empty())
                .collect::<Vec<_>>();

            let stdout = std::io::stdout();
            let handle = stdout.lock();
            println!();
            return serde_json::to_writer_pretty(handle, &filtered_flat)
                .map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause));
        } else {
            // Filter the hierarchical tree for output.
            let pruned = prune_group_by_used(srcs, sess.root, &used).simplify();
            let stdout = std::io::stdout();
            let handle = stdout.lock();
            println!();
            return serde_json::to_writer_pretty(handle, &pruned)
                .map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause));
        }
    }

    let result = {
        let stdout = std::io::stdout();
        let handle = stdout.lock();
        if want_flatten {
            let srcs = srcs.flatten();
            serde_json::to_writer_pretty(handle, &srcs)
        } else {
            serde_json::to_writer_pretty(handle, &srcs)
        }
    };
    println!();
    result.map_err(|cause| Error::chain("Failed to serialize source file manifest.", cause))
}
