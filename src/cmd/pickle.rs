// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! The `pickle` subcommand.

use clap::{ArgAction, Args};
use indexmap::IndexSet;
use tokio::runtime::Runtime;

use crate::cmd::sources::get_passed_targets;
use crate::config::{Validate, ValidationContext};
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceType};
use crate::target::TargetSet;

use bender_slang::{SlangContextExt, SlangPrintOpts, SyntaxTreeExt};

// TODO(fischeti): Clean up the arguments and options.
// At the moment, they are just directly mirroring the Slang API.
// for debugging purposes.
/// Pickle files
#[derive(Args, Debug)]
pub struct PickleArgs {
    /// Additional source files to pickle
    files: Vec<String>,

    /// The output file (defaults to stdout)
    // TODO(fischeti): Actually implement this.
    #[arg(short, long)]
    output: Option<String>,

    /// Only include sources that match the given target
    #[arg(short, long, action = ArgAction::Append, global = true)]
    pub target: Vec<String>,

    /// Specify package to show sources for
    #[arg(short, long, action = ArgAction::Append, global = true)]
    pub package: Vec<String>,

    /// Specify package to exclude from sources
    #[arg(long, action = ArgAction::Append, global = true)]
    pub exclude: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(long, global = true)]
    pub no_deps: bool,

    /// Additional include directory
    #[arg(short = 'I', action = ArgAction::Append)]
    include_dir: Vec<String>,

    /// Additional preprocessor definition
    #[arg(short = 'D', action = ArgAction::Append)]
    define: Vec<String>,

    /// The prefix to add to all names
    #[arg(long, help_heading = "Slang Options")]
    prefix: Option<String>,

    /// The suffix to add to all names
    #[arg(long, help_heading = "Slang Options")]
    suffix: Option<String>,

    /// Whether to include preprocessor directives
    #[arg(long, default_value_t = true, action = ArgAction::SetFalse, help_heading = "Slang Options")]
    include_directives: bool,

    /// Whether to expand include directives
    #[arg(long, default_value_t = true, action = ArgAction::SetFalse, help_heading = "Slang Options")]
    expand_includes: bool,

    /// Whether to expand macros
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Slang Options")]
    expand_macros: bool,

    /// Whether to strip comments
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Slang Options")]
    strip_comments: bool,

    /// Whether to strip newlines
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Slang Options")]
    strip_newlines: bool,
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
    let srcs = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate(&ValidationContext::default()))
        .collect::<Result<Vec<_>>>()?;

    let print_opts = SlangPrintOpts {
        include_directives: args.include_directives,
        expand_includes: args.expand_includes,
        expand_macros: args.expand_macros,
        include_comments: !args.strip_comments,
        squash_newlines: args.strip_newlines,
    };

    for src_group in srcs {
        let mut slang = bender_slang::new_session();

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
        let file_paths = src_group.files.iter().filter_map(|source| match source {
            SourceFile::File(path, Some(SourceType::Verilog)) => path.to_str(),
            // Vhdl or unknown file types are not supported by Slang, so we emit a warning and skip them.
            SourceFile::File(path, _) => {
                Warnings::PickleNonVerilogFile(path.to_path_buf()).emit();
                None
            }
            // Groups should not exist at this point,
            // as we have already flattened the sources.
            _ => None,
        });

        for file_path in file_paths {
            let tree = slang.parse(file_path).map_err(|cause| {
                Error::new(format!("Cannot parse file {}: {}", file_path, cause))
            })?;
            let renamed_tree = tree.rename(args.prefix.as_deref(), args.suffix.as_deref());
            println!("{}", renamed_tree.display(print_opts));
        }
    }

    Ok(())
}
