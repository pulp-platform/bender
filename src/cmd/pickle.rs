// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! The `pickle` subcommand.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use clap::Args;
use indexmap::{IndexMap, IndexSet};
use tokio::runtime::Runtime;

use crate::cmd::sources::get_passed_targets;
use crate::config::{Validate, ValidationContext};
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup, SourceType};
use crate::target::TargetSet;

use bender_slang::SlangPrintOpts;

/// Pickle files
#[derive(Args, Debug)]
pub struct PickleArgs {
    /// Additional source files to pickle, which are not part of the manifest.
    files: Vec<String>,

    /// The output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<String>,

    /// Only include sources that match the given target
    #[arg(short, long)]
    pub target: Vec<String>,

    /// Specify package to show sources for
    #[arg(short, long)]
    pub package: Vec<String>,

    /// Specify package to exclude from sources
    #[arg(long)]
    pub exclude: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(long)]
    pub no_deps: bool,

    /// Additional include directory, which are not part of the manifest.
    #[arg(short = 'I')]
    include_dir: Vec<String>,

    /// Additional preprocessor definition, which are not part of the manifest.
    #[arg(short = 'D')]
    define: Vec<String>,

    /// One or more top-level modules used to trim unreachable parsed files.
    #[arg(long, help_heading = "Slang Options")]
    top: Vec<String>,

    /// A prefix to add to all names (modules, packages, interfaces)
    #[arg(long, help_heading = "Slang Options")]
    prefix: Option<String>,

    /// A suffix to add to all names (modules, packages, interfaces)
    #[arg(long, help_heading = "Slang Options")]
    suffix: Option<String>,

    /// Names to exclude from renaming (modules, packages, interfaces)
    #[arg(long, help_heading = "Slang Options")]
    exclude_rename: Vec<String>,

    /// Expand macros in the output
    #[arg(long, help_heading = "Slang Options")]
    expand_macros: bool,

    /// Strip comments from the output
    #[arg(long, help_heading = "Slang Options")]
    strip_comments: bool,

    /// Squash newlines in the output
    #[arg(long, help_heading = "Slang Options")]
    squash_newlines: bool,

    /// Dump the syntax trees as JSON instead of the source code
    #[arg(long, help_heading = "Slang Options")]
    ast_json: bool,
}

/// Execute the `pickle` subcommand.
pub fn run(sess: &Session, args: PickleArgs) -> Result<()> {
    // Load the source files
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let srcs = rt.block_on(io.sources(false, &[]))?;

    // Filter the sources by target.
    let targets = TargetSet::new(args.target.iter().map(|s| s.as_str()));

    // Convert vector to sets for packages and excluded packages.
    let package_set = IndexSet::from_iter(args.package);
    let exclude_set = IndexSet::from_iter(args.exclude);

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess.manifest.package.name.to_string(),
        &package_set,
        &exclude_set,
        args.no_deps,
    );

    let (targets, packages) = get_passed_targets(sess, &rt, &io, &targets, packages, &package_set)?;

    // Filter the sources by target and package.
    let srcs = srcs
        .filter_targets(&targets)
        .unwrap_or_default()
        .filter_packages(&packages)
        .unwrap_or_default();

    // Flatten and validate the sources.
    let mut srcs = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate(&ValidationContext::default()))
        .collect::<Result<Vec<_>>>()?;

    if !args.files.is_empty() {
        let include_dirs = args
            .include_dir
            .iter()
            .map(|d| sess.intern_path(Path::new(d)))
            .collect::<IndexSet<_>>();
        let defines = args
            .define
            .iter()
            .map(|d| {
                let mut parts = d.splitn(2, '=');
                let name = parts.next().unwrap_or_default().trim().to_string();
                let value = parts
                    .next()
                    .map(|v| sess.intern_string(v.trim().to_string()));
                (name, value)
            })
            .collect::<IndexMap<_, _>>();
        let files = args
            .files
            .iter()
            .map(|f| SourceFile::File(sess.intern_path(Path::new(f)), Some(SourceType::Verilog)))
            .collect::<Vec<_>>();

        srcs.push(SourceGroup {
            include_dirs,
            defines,
            files,
            ..SourceGroup::default()
        });
    }

    let print_opts = SlangPrintOpts {
        expand_macros: args.expand_macros,
        include_comments: !args.strip_comments,
        squash_newlines: args.squash_newlines,
    };

    // Setup Output Writer, either to file or stdout
    let raw_writer: Box<dyn Write> = match &args.output {
        Some(path) => Box::new(
            File::create(path)
                .map_err(|e| Error::new(format!("Cannot create output file: {}", e)))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    let mut writer = BufWriter::new(raw_writer);

    // Start JSON Array if needed
    if args.ast_json {
        write!(writer, "[")?;
    }

    let mut parsed_trees = bender_slang::SyntaxTrees::new();
    let mut slang = bender_slang::new_session();
    for src_group in srcs {
        // Collect include directories and defines from the source group and command line arguments.
        let include_dirs: Vec<String> = src_group
            .include_dirs
            .iter()
            .chain(src_group.export_incdirs.values().flatten())
            .map(|path| path.to_string_lossy().into_owned())
            .chain(args.include_dir.iter().cloned())
            .collect();

        // Collect defines from the source group and command line arguments.
        let defines: Vec<String> = src_group
            .defines
            .iter()
            .map(|(def, value)| match value {
                Some(v) => format!("{def}={v}"),
                None => def.to_string(),
            })
            .chain(args.define.iter().cloned())
            .collect();

        // Set the include directories and defines in the Slang session.
        slang.set_includes(&include_dirs).set_defines(&defines);

        // Collect file paths from the source group.
        let file_paths: Vec<String> = src_group
            .files
            .iter()
            .filter_map(|source| match source {
                SourceFile::File(path, Some(SourceType::Verilog)) => {
                    Some(path.to_string_lossy().into_owned())
                }
                // Vhdl or unknown file types are not supported by Slang, so we emit a warning and skip them.
                SourceFile::File(path, _) => {
                    Warnings::PickleNonVerilogFile(path.to_path_buf()).emit();
                    None
                }
                // Groups should not exist at this point,
                // as we have already flattened the sources.
                _ => None,
            })
            .collect();

        let group_trees = slang.parse_files(&file_paths)?;
        parsed_trees.append_trees(&group_trees);
    }

    let reachable = if args.top.is_empty() {
        (0..parsed_trees.len()).collect::<Vec<usize>>()
    } else {
        parsed_trees.reachable_indices(&args.top)?
    };

    let mut first_item = true;
    for idx in reachable {
        let tree = parsed_trees.tree_at(idx)?;
        let renamed_tree = tree.rename(
            args.prefix.as_deref(),
            args.suffix.as_deref(),
            &args.exclude_rename,
        );
        if args.ast_json {
            // JSON Array Logic: Prepend comma if not the first item
            if !first_item {
                write!(writer, ",")?;
            }
            write!(writer, "{:?}", renamed_tree)?;
            first_item = false;
        } else {
            write!(writer, "{}", renamed_tree.display(print_opts))?;
        }
    }

    // Close JSON Array
    if args.ast_json {
        writeln!(writer, "]")?;
    }

    Ok(())
}
