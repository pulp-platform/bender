// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! The `pickle` subcommand.

use clap::{ArgAction, Args};

use bender_slang::{SlangContextExt, SlangPrintOpts, SyntaxTreeExt};

use crate::error::*;

// TODO(fischeti): Clean up the arguments and options.
// At the moment, they are just directly mirroring the Slang API.
// for debugging purposes.
/// Pickle files
#[derive(Args, Debug)]
pub struct PickleArgs {
    /// Source files to pickle
    #[arg(required = true)]
    files: Vec<String>,

    /// The output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<String>,

    /// Add an include directory
    #[arg(short = 'I', long, action = ArgAction::Append)]
    include_dirs: Vec<String>,

    /// Add defines
    #[arg(short = 'D', long, action = ArgAction::Append)]
    defines: Vec<String>,

    /// The prefix to add to all names
    #[arg(long)]
    prefix: Option<String>,

    /// The suffix to add to all names
    #[arg(long)]
    suffix: Option<String>,

    /// Whether to include preprocessor directives
    #[arg(long, default_value_t = true, action = ArgAction::SetFalse, help_heading = "Print Options")]
    include_directives: bool,

    /// Whether to expand include directives
    #[arg(long, default_value_t = true, action = ArgAction::SetFalse, help_heading = "Print Options")]
    expand_includes: bool,

    /// Whether to expand macros
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Print Options")]
    expand_macros: bool,

    /// Whether to strip comments
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Print Options")]
    strip_comments: bool,

    /// Whether to strip newlines
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, help_heading = "Print Options")]
    strip_newlines: bool,
}

/// Execute the `pickle` subcommand.
pub fn run(args: PickleArgs) -> Result<()> {
    let print_opts = SlangPrintOpts {
        include_directives: args.include_directives,
        expand_includes: args.expand_includes,
        expand_macros: args.expand_macros,
        include_comments: !args.strip_comments,
        squash_newlines: args.strip_newlines,
    };

    let mut slang = bender_slang::new_session()
        .set_includes(&args.include_dirs)
        .set_defines(&args.defines);

    for source in &args.files {
        let tree = slang
            .parse(source)
            .map_err(|cause| Error::new(format!("Cannot parse file {}: {}", source, cause)))?;
        let renamed_tree = tree.rename(args.prefix.as_deref(), args.suffix.as_deref());
        println!("{}", renamed_tree.display(print_opts));
    }

    Ok(())
}
