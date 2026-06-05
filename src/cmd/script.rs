// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(all(unix, feature = "slang"))]
use std::fs::canonicalize;

#[cfg(all(windows, feature = "slang"))]
use dunce::canonicalize;

use clap::{ArgAction, Args, Subcommand, ValueEnum};
use indexmap::{IndexMap, IndexSet};
use miette::{Context as _, IntoDiagnostic as _};
use serde::Serialize;
use tera::{Context, Tera};
use tokio::runtime::Runtime;

use crate::Result;
use crate::cmd::sources::get_passed_targets;
use crate::config::{Validate, ValidationContext};
use crate::diagnostic::Warnings;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup, SourceType};
use crate::target::TargetSet;

#[cfg(feature = "slang")]
use bender_slang::SlangSession;

/// Emit tool scripts for the package
#[derive(Args, Debug)]
pub struct ScriptArgs {
    /// Only include sources that match the given target
    #[arg(short, long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub target: Vec<String>,

    /// Remove any default targets that may be added to the generated script
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub no_default_target: bool,

    /// Pass an additional define to all source files
    #[arg(short = 'D', long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub define: Vec<String>,

    /// Include source annotations in the generated script
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub source_annotations: bool,

    /// Specify package to show sources for
    #[arg(short, long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short, long, global = true, help_heading = "General Script Options")]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short, long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub exclude: Vec<String>,

    /// Keep export include directories from excluded packages
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub keep_excluded_incdirs: bool,

    /// Add the `rtl` target to any fileset without a target specification
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub assume_rtl: bool,

    /// Ignore passed targets
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub ignore_passed_targets: bool,

    /// Choose compilation mode option
    #[arg(
        long,
        default_value_t,
        value_enum,
        global = true,
        help_heading = "General Script Options"
    )]
    pub compilation_mode: CompilationMode,

    /// Do not abort analysis/compilation on first caught error
    #[arg(long, global = true, help_heading = "General Script Options")]
    pub no_abort_on_error: bool,

    /// Trim unreachable Verilog files via the given top-level module(s)
    #[cfg(feature = "slang")]
    #[arg(long, global = true, help_heading = "Slang Options")]
    pub top: Vec<String>,

    /// Drop unused include directories from the generated script
    #[cfg(feature = "slang")]
    #[arg(
        long,
        value_enum,
        default_value_t,
        global = true,
        help_heading = "Slang Options"
    )]
    pub trim_incdirs: TrimIncdirs,

    /// What to do with files slang reports parse errors on with no `pragma protect` envelope
    /// [implicit default: error when slang runs; no effect otherwise]
    #[cfg(feature = "slang")]
    #[arg(long, value_enum, global = true, help_heading = "Slang Options")]
    pub broken: Option<ParsePolicy>,

    /// What to do with IEEE-1735 encrypted files slang cannot fully parse
    /// [implicit default: keep when slang runs; no effect otherwise]
    #[cfg(feature = "slang")]
    #[arg(long, value_enum, global = true, help_heading = "Slang Options")]
    pub encrypted: Option<ParsePolicy>,

    /// Format of the generated script
    #[command(subcommand)]
    pub format: ScriptFormat,
}

/// Compilation mode enum
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompilationMode {
    #[default]
    /// Compile each source file group separately
    Separate,
    /// Compile all source file groups together in a common compilation unit
    Common,
}

/// Controls whether unused include directories are dropped.
#[cfg(feature = "slang")]
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrimIncdirs {
    /// Drop iff `--top` is set
    #[default]
    Auto,
    /// Always drop unused directories
    Always,
    /// Keep every declared directory
    Never,
}

/// What to do with a class of files slang reports parse errors on.
#[cfg(feature = "slang")]
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsePolicy {
    /// Abort the run if any file of this class is found
    Error,
    /// Tolerate these files and include them in the script
    Keep,
    /// Tolerate these files but drop them from the script
    Drop,
}

/// Common arguments for Vivado scripts
#[derive(Args, Debug, Clone)]
pub struct OnlyArgs {
    /// Only output commands to define macros
    #[arg(long = "only-defines", group = "only")]
    pub defines: bool,

    /// Only output commands to define include directories
    #[arg(long = "only-includes", group = "only")]
    pub includes: bool,

    /// Only output commands to define source files
    #[arg(long = "only-sources", group = "only")]
    pub sources: bool,
}

/// Script format enum
#[derive(Subcommand, Debug)]
pub enum ScriptFormat {
    /// A general file list
    Flist {
        /// Use relative paths
        #[arg(long)]
        relative_path: bool,
    },
    /// An extended file list with include dirs and defines
    FlistPlus {
        /// Use relative paths
        #[arg(long)]
        relative_path: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,
    },
    /// ModelSim/QuestaSim script
    Vsim {
        /// Pass arguments to vlog calls
        #[arg(long, action = ArgAction::Append, alias = "vlog-arg")]
        vlog_args: Vec<String>,

        /// Pass arguments to vcom calls
        #[arg(long, action = ArgAction::Append, alias = "vcom-arg")]
        vcom_args: Vec<String>,
    },
    /// Synopsys VCS script
    Vcs {
        /// Pass arguments to vlogan calls
        #[arg(long, action = ArgAction::Append, alias = "vlog-arg")]
        vlogan_args: Vec<String>,

        /// Pass arguments to vhdlan calls
        #[arg(long, action = ArgAction::Append, alias = "vcom-arg")]
        vhdlan_args: Vec<String>,

        /// Specify a `vlogan` command
        #[arg(long, default_value = "vlogan")]
        vlogan_bin: String,

        /// Specify a `vhdlan` command
        #[arg(long, default_value = "vhdlan")]
        vhdlan_bin: String,
    },
    /// Verilator script
    Verilator {
        /// Pass arguments to verilator calls
        #[arg(long, action = ArgAction::Append, alias = "vlog-arg")]
        vlt_args: Vec<String>,
    },
    /// Synopsys EDA tool script
    Synopsys {
        /// Pass arguments to verilog compilation calls
        #[arg(long, action = ArgAction::Append, alias = "vlog-arg")]
        verilog_args: Vec<String>,

        /// Pass arguments to vhdl compilation calls
        #[arg(long, action = ArgAction::Append, alias = "vcom-arg")]
        vhdl_args: Vec<String>,
    },
    /// Synopsys Formality script
    Formality,
    /// Riviera script
    Riviera {
        /// Pass arguments to vlog calls
        #[arg(long, action = ArgAction::Append, alias = "vlog-arg")]
        vlog_args: Vec<String>,

        /// Pass arguments to vcom calls
        #[arg(long, action = ArgAction::Append, alias = "vcom-arg")]
        vcom_args: Vec<String>,
    },
    /// Cadence Genus script
    Genus,
    /// Xilinx Vivado synthesis script
    Vivado {
        /// Do not change `simset` fileset
        #[arg(long)]
        no_simset: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,
    },
    /// Xilinx Vivado simulation script
    VivadoSim {
        /// Do not change `simset` fileset
        #[arg(long)]
        no_simset: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,
    },
    /// Mentor Graphics Precision script
    Precision,
    /// Custom template script
    Template {
        /// Path to a file containing the tera template string to be formatted.
        #[arg(long)]
        template: String,
    },
    /// JSON output
    #[command(alias = "template_json")]
    TemplateJson,
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

/// Execute the `script` subcommand.
pub fn run(sess: &Session, args: &ScriptArgs) -> Result<()> {
    let rt = Runtime::new().into_diagnostic()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources(false, &[]))?;

    // Format-specific target specifiers.
    let vivado_targets = &["vivado", "fpga", "xilinx"];
    fn concat<T: Clone>(a: &[T], b: &[T]) -> Vec<T> {
        a.iter().chain(b).cloned().collect()
    }
    let format_targets: Vec<&str> = if !args.no_default_target {
        match args.format {
            ScriptFormat::Flist { .. } => vec!["flist"],
            ScriptFormat::FlistPlus { .. } => vec!["flist"],
            ScriptFormat::Vsim { .. } => vec!["vsim", "simulation"],
            ScriptFormat::Vcs { .. } => vec!["vcs", "simulation"],
            ScriptFormat::Verilator { .. } => vec!["verilator", "synthesis"],
            ScriptFormat::Synopsys { .. } => vec!["synopsys", "synthesis"],
            ScriptFormat::Formality => vec!["synopsys", "synthesis", "formality"],
            ScriptFormat::Riviera { .. } => vec!["riviera", "simulation"],
            ScriptFormat::Genus => vec!["genus", "synthesis"],
            ScriptFormat::Vivado { .. } => concat(vivado_targets, &["synthesis"]),
            ScriptFormat::VivadoSim { .. } => concat(vivado_targets, &["simulation"]),
            ScriptFormat::Precision => vec!["precision", "fpga", "synthesis"],
            ScriptFormat::Template { .. } => vec![],
            ScriptFormat::TemplateJson => vec![],
        }
    } else {
        vec![]
    };

    // Filter the sources by target.
    let targets = TargetSet::new(args.target.iter().map(|s| s.as_str()).chain(format_targets));

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

    srcs = srcs
        .filter_packages(&packages, args.keep_excluded_incdirs)
        .unwrap_or_default();

    // Flatten and validate the sources.
    let srcs = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate(&ValidationContext::default()))
        .collect::<Result<Vec<_>>>()?;

    // Slang-based filtering: trim unreachable Verilog files (when `--top` is given) and/or
    // unused include directories (per `--trim-incdirs`), with per-class policies for files
    // slang couldn't fully parse (`--broken`, `--encrypted`).
    #[cfg(feature = "slang")]
    let (srcs, unparseable_paths) = {
        let trim_incdirs = match args.trim_incdirs {
            TrimIncdirs::Always => true,
            TrimIncdirs::Never => false,
            TrimIncdirs::Auto => !args.top.is_empty(),
        };
        // Skip the slang pass entirely when no flag requires it. `Keep` (the implicit default
        // for encrypted, and what an explicit `--broken keep` / `--encrypted keep` says) is a
        // no-op without slang — we'd keep the file anyway since no filter touched it. Only an
        // explicit `error` or `drop` value needs slang to actually classify files.
        let broken_policy = args.broken.unwrap_or(ParsePolicy::Error);
        let encrypted_policy = args.encrypted.unwrap_or(ParsePolicy::Keep);
        let policies_need_slang = matches!(
            args.broken,
            Some(ParsePolicy::Error) | Some(ParsePolicy::Drop)
        ) || matches!(
            args.encrypted,
            Some(ParsePolicy::Error) | Some(ParsePolicy::Drop)
        );
        if args.top.is_empty() && !trim_incdirs && !policies_need_slang {
            (srcs, std::collections::HashSet::<PathBuf>::new())
        } else {
            apply_slang_filters(
                srcs,
                &args.top,
                trim_incdirs,
                broken_policy,
                encrypted_policy,
            )?
        }
    };
    #[cfg(not(feature = "slang"))]
    let unparseable_paths = std::collections::HashSet::<PathBuf>::new();

    let mut tera_context = Context::new();
    let mut only_args = OnlyArgs {
        defines: false,
        includes: false,
        sources: false,
    };

    // Generate the corresponding output.
    let template_content = match &args.format {
        ScriptFormat::Flist { relative_path } => {
            tera_context.insert("relativize_path", relative_path);
            include_str!("../script_fmt/flist.tera")
        }
        ScriptFormat::FlistPlus {
            relative_path,
            only,
        } => {
            tera_context.insert("relativize_path", relative_path);
            only_args = only.clone();
            include_str!("../script_fmt/flist-plus.tera")
        }
        ScriptFormat::Vsim {
            vlog_args,
            vcom_args,
        } => {
            tera_context.insert("vlog_args", vlog_args);
            tera_context.insert("vcom_args", vcom_args);
            include_str!("../script_fmt/vsim_tcl.tera")
        }
        ScriptFormat::Vcs {
            vlogan_bin,
            vhdlan_bin,
            vlogan_args,
            vhdlan_args,
        } => {
            tera_context.insert("vlogan_args", vlogan_args);
            tera_context.insert("vhdlan_args", vhdlan_args);
            tera_context.insert("vlogan_bin", vlogan_bin);
            tera_context.insert("vhdlan_bin", vhdlan_bin);
            include_str!("../script_fmt/vcs_sh.tera")
        }
        ScriptFormat::Verilator { vlt_args } => {
            tera_context.insert("vlt_args", vlt_args);
            include_str!("../script_fmt/verilator_sh.tera")
        }
        ScriptFormat::Synopsys {
            verilog_args,
            vhdl_args,
        } => {
            tera_context.insert("verilog_args", verilog_args);
            tera_context.insert("vhdl_args", vhdl_args);
            include_str!("../script_fmt/synopsys_tcl.tera")
        }
        ScriptFormat::Formality => include_str!("../script_fmt/formality_tcl.tera"),
        ScriptFormat::Riviera {
            vlog_args,
            vcom_args,
        } => {
            tera_context.insert("vlog_args", vlog_args);
            tera_context.insert("vcom_args", vcom_args);
            include_str!("../script_fmt/riviera_tcl.tera")
        }
        ScriptFormat::Genus => include_str!("../script_fmt/genus_tcl.tera"),
        ScriptFormat::Vivado { no_simset, only } | ScriptFormat::VivadoSim { no_simset, only } => {
            only_args = only.clone();
            tera_context.insert("vivado_filesets", &{
                if *no_simset {
                    vec![""]
                } else {
                    vec!["", " -simset"]
                }
            });
            include_str!("../script_fmt/vivado_tcl.tera")
        }
        ScriptFormat::Precision => include_str!("../script_fmt/precision_tcl.tera"),
        ScriptFormat::Template { template } => {
            &std::fs::read_to_string(template).into_diagnostic()?
        }
        ScriptFormat::TemplateJson => JSON,
    };

    emit_template(
        sess,
        tera_context,
        template_content,
        args,
        only_args,
        srcs,
        &unparseable_paths,
    )
}

/// Subdivide the source files in a group.
///
/// The function `categorize` is used to assign a category to each source file.
/// Files with the same category that appear after each other will be kept in
/// the same source group. Files with different cateogries are split into
/// separate groups.
fn separate_files_in_group<F1, F2, T>(mut src: SourceGroup, categorize: F1, mut consume: F2)
where
    F1: Fn(&SourceFile) -> Option<T>,
    F2: FnMut(&SourceGroup, T, Vec<SourceFile>),
    T: Eq,
{
    let mut category = None;
    let mut files = vec![];
    for file in std::mem::take(&mut src.files) {
        let new_category = categorize(&file);
        if new_category.is_none() {
            continue;
        }
        if category.is_some() && category != new_category && !files.is_empty() {
            consume(&src, category.take().unwrap(), std::mem::take(&mut files));
        }
        files.push(file);
        category = new_category;
    }
    if !files.is_empty() {
        consume(&src, category.unwrap(), files);
    }
}

/// Filter source groups using slang's view of the design.
///
/// When `top` is non-empty, Verilog files not reachable from any of those top modules are
/// dropped (VHDL and untyped files are always retained, and any group that ends up with no
/// files is dropped). When `trim_incdirs` is true, include directories slang did not resolve
/// an `include through are dropped from `include_dirs` and `export_incdirs`.
///
/// Files slang couldn't fully parse are classified into:
///   * encrypted — slang emitted a `ProtectedEnvelope` diag (IEEE-1735 protect block)
///   * broken    — parse errors with no such diag (looks like a real syntax bug)
///
/// Each class is handled per its `ParsePolicy`: `Error` aborts the run, `Keep` includes the
/// file in the script, `Drop` tolerates but excludes it. Sensible defaults are `broken=Error`
/// and `encrypted=Keep`.
///
/// Non-Verilog files (VHDL, untyped) intentionally bypass every filter here — slang doesn't
/// process them, so we have no reachability information and conservatively retain them all.
///
/// Returns the filtered groups plus the set of unparseable file paths that survived filtering,
/// so the caller can annotate them in `source_annotations` output.
#[cfg(feature = "slang")]
fn apply_slang_filters<'a>(
    srcs: Vec<SourceGroup<'a>>,
    top: &[String],
    trim_incdirs: bool,
    broken_policy: ParsePolicy,
    encrypted_policy: ParsePolicy,
) -> Result<(Vec<SourceGroup<'a>>, std::collections::HashSet<PathBuf>)> {
    use std::collections::HashSet;

    let mut session = SlangSession::new();

    for src_group in &srcs {
        // Collect include dirs
        let include_dirs: Vec<String> = src_group
            .include_dirs
            .iter()
            .chain(src_group.export_incdirs.values().flatten())
            .map(|(_, path)| path.to_string_lossy().into_owned())
            .collect::<IndexSet<_>>()
            .into_iter()
            .collect();

        // Collect defines
        let defines: Vec<String> = src_group
            .defines
            .iter()
            .map(|(def, (_, value))| match value {
                Some(v) => format!("{def}={v}"),
                None => def.to_string(),
            })
            .collect::<IndexSet<_>>()
            .into_iter()
            .collect();

        // Collect only Verilog file paths.
        let paths: Vec<&Path> = src_group
            .files
            .iter()
            .filter_map(|f| match f {
                SourceFile::File(p, Some(SourceType::Verilog)) => Some(*p),
                _ => None,
            })
            .collect();

        if !paths.is_empty() {
            let file_paths: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            session
                .parse_group(&file_paths, &include_dirs, &defines)
                .into_diagnostic()?;
        }
    }

    // Discriminate encrypted IP (legal SystemVerilog, just hard to parse) from genuinely broken
    // files (real syntax bugs). Encrypted ⇔ slang emitted a `ProtectedEnvelope` diag on that
    // tree. Each tree carries its own source path, so we identify files by path directly.
    let all_trees = session.all_trees();
    let mut encrypted_paths: HashSet<PathBuf> = HashSet::new();
    let mut broken_paths: HashSet<PathBuf> = HashSet::new();
    for parsed in &all_trees {
        if parsed.parsed_ok {
            continue;
        }
        if parsed.encrypted {
            encrypted_paths.insert(PathBuf::from(&parsed.path));
        } else {
            broken_paths.insert(PathBuf::from(&parsed.path));
        }
    }

    let list = |set: &HashSet<PathBuf>| -> String {
        let mut v: Vec<String> = set.iter().map(|p| p.display().to_string()).collect();
        v.sort();
        v.join("\n  ")
    };

    // Policy enforcement: abort up front for any `Error` class.
    if broken_policy == ParsePolicy::Error && !broken_paths.is_empty() {
        return Err(miette::miette!(
            "slang reported parse errors in {} file(s) with no `pragma protect` envelope (looks like real syntax bugs):\n  {}\n\
             see diagnostics above; pass --broken keep or --broken drop to continue",
            broken_paths.len(),
            list(&broken_paths)
        ));
    }
    if encrypted_policy == ParsePolicy::Error && !encrypted_paths.is_empty() {
        return Err(miette::miette!(
            "slang reported parse errors in {} encrypted file(s) and --encrypted error was requested:\n  {}",
            encrypted_paths.len(),
            list(&encrypted_paths)
        ));
    }

    // After Error-class abort: any remaining unparseable file is either Kept or Dropped.
    let broken_kept = broken_policy == ParsePolicy::Keep;
    let encrypted_kept = encrypted_policy == ParsePolicy::Keep;

    // Determine which trees feed into the include / file-retention questions. With `--top` we
    // only look at trees reachable from those top modules; without `--top` we use every tree
    // (relevant when the caller asked for include-dir trimming but no file filtering).
    let filter_files = !top.is_empty();
    let kept_trees = if filter_files {
        session.reachable_trees(top).into_diagnostic()?
    } else {
        all_trees
    };
    let kept_paths: HashSet<&Path> = if filter_files {
        kept_trees
            .iter()
            .map(|t| Path::new(t.path.as_str()))
            .collect()
    } else {
        HashSet::new()
    };

    // Strict include-dir trimming: a directory survives only if slang actually resolved at least
    // one `include directive through it. Canonicalize both sides so symlinks / `.` / `..` don't
    // cause spurious mismatches.
    let resolved_includes: Vec<PathBuf> = if trim_incdirs {
        session
            .resolved_include_paths(&kept_trees)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        Vec::new()
    };
    let dir_is_used = |dir: &Path| -> bool {
        let canon = canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        resolved_includes.iter().any(|f| f.starts_with(&canon))
    };

    // Single retention rule per Verilog file:
    //   * broken    files keep iff `broken_policy == Keep`    (regardless of reachability)
    //   * encrypted files keep iff `encrypted_policy == Keep` (regardless of reachability)
    //   * everything else keep iff no --top filter is active, or it's reachable from one
    let retain_verilog = |p: &Path| -> bool {
        if broken_paths.contains(p) {
            broken_kept
        } else if encrypted_paths.contains(p) {
            encrypted_kept
        } else {
            !filter_files || kept_paths.contains(p)
        }
    };
    let drop_anything = broken_policy == ParsePolicy::Drop || encrypted_policy == ParsePolicy::Drop;
    let run_file_filter = filter_files || drop_anything;

    let filtered: Vec<SourceGroup<'a>> = srcs
        .into_iter()
        .map(|mut group| {
            if run_file_filter {
                group.files.retain(|f| match f {
                    SourceFile::File(p, Some(SourceType::Verilog)) => retain_verilog(p),
                    _ => true,
                });
            }
            if trim_incdirs {
                group.include_dirs.retain(|(_, p)| dir_is_used(p));
                for paths in group.export_incdirs.values_mut() {
                    paths.retain(|(_, p)| dir_is_used(p));
                }
            }
            group
        })
        // Remove empty groups that may have resulted from filtering out all Verilog files.
        .filter(|group| !group.files.is_empty())
        .collect();

    // Summary lines: report encrypted and broken classes separately so users can tell apart
    // automatic vs explicit tolerance and the inclusion verdict at a glance.
    let kept_unparseable: HashSet<PathBuf> = encrypted_paths
        .iter()
        .filter(|_| encrypted_kept)
        .chain(broken_paths.iter().filter(|_| broken_kept))
        .cloned()
        .collect();
    let class_summary = |label: &str, set: &HashSet<PathBuf>, policy: ParsePolicy| {
        if set.is_empty() || policy == ParsePolicy::Error {
            // Error was handled above; Keep/Drop are the only branches that reach here.
            return;
        }
        let verb = match policy {
            ParsePolicy::Keep => "kept in script output",
            ParsePolicy::Drop => "dropped",
            ParsePolicy::Error => unreachable!(),
        };
        let mut names: Vec<String> = set.iter().map(|p| p.display().to_string()).collect();
        names.sort();
        eprintln!(
            "warning: {} {} file(s) ({}): {}",
            names.len(),
            label,
            verb,
            names.join(", "),
        );
    };
    class_summary("encrypted", &encrypted_paths, encrypted_policy);
    class_summary("broken", &broken_paths, broken_policy);

    Ok((filtered, kept_unparseable))
}

static HEADER_AUTOGEN: &str = "This script was generated automatically by bender.";

fn add_defines(defines: &mut IndexMap<String, Option<String>>, define_args: &[String]) {
    defines.extend(define_args.iter().map(|t| {
        let mut parts = t.splitn(2, '=');
        let name = parts.next().unwrap().trim();
        let value = parts.next().map(|v| v.trim().to_string());
        (name.to_string(), value)
    }));
}

static JSON: &str = "json";

fn emit_template(
    sess: &Session,
    mut tera_context: Context,
    template: &str,
    args: &ScriptArgs,
    only: OnlyArgs,
    srcs: Vec<SourceGroup>,
    unparseable_paths: &std::collections::HashSet<PathBuf>,
) -> Result<()> {
    // Helper for annotating FileEntry.comment on files that survived filtering despite slang
    // failing to parse them; visible to users with `--source-annotations`.
    let unparseable_comment = |p: &Path| -> Option<String> {
        if unparseable_paths.contains(p) {
            Some("UNPARSEABLE: slang reported parse errors".to_string())
        } else {
            None
        }
    };
    tera_context.insert("HEADER_AUTOGEN", HEADER_AUTOGEN);
    tera_context.insert("root", sess.root);
    // tera_context.insert("srcs", &srcs);
    tera_context.insert("abort_on_error", &!args.no_abort_on_error);

    let mut global_defines = IndexMap::new();
    let emit_sources = !only.defines && !only.includes;
    let emit_defines = !only.includes && !only.sources;
    let emit_incdirs = !only.defines && !only.sources;

    add_defines(&mut global_defines, &args.define);
    tera_context.insert("global_defines", &global_defines);

    let mut all_defines = IndexMap::new();
    let mut all_incdirs = IndexSet::new();
    let mut all_files = IndexSet::new();
    let mut all_verilog = vec![];
    let mut all_vhdl = vec![];
    let mut unknown_files = vec![];
    let mut all_override_files: IndexSet<(&Path, &str)> = IndexSet::new();
    for src in &srcs {
        all_defines.extend(
            src.defines
                .iter()
                .map(|(k, (_, v))| (k.to_string(), v.map(String::from))),
        );
        all_incdirs.extend(src.get_incdirs());

        // If override_files is set, source files are not automatically included, only to replace files with matching basenames.
        if src.override_files {
            all_override_files.extend(src.files.iter().filter_map(|file| match file {
                SourceFile::File(p, _) => Some((*p, src.package.unwrap_or("None"))),
                SourceFile::Group(_) => None,
            }));
        } else {
            all_files.extend(src.files.iter().filter_map(|file| match file {
                SourceFile::File(p, _) => Some((*p, None::<String>)),
                SourceFile::Group(_) => None,
            }));
        }
    }

    add_defines(&mut all_defines, &args.define);
    let all_defines = if emit_defines {
        all_defines.into_iter().collect()
    } else {
        IndexSet::new()
    };

    tera_context.insert("all_defines", &all_defines);

    all_incdirs.sort();
    let all_incdirs: IndexSet<PathBuf> = if emit_incdirs {
        all_incdirs.into_iter().map(|p| p.to_path_buf()).collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_incdirs", &all_incdirs);

    // replace files in all_files with override files
    let override_map = all_override_files
        .iter()
        .map(|(f, pkg)| {
            (
                f.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or(""),
                (*f, pkg),
            )
        })
        .collect::<IndexMap<_, _>>();
    let all_files = all_files
        .into_iter()
        .map(|file| {
            let basename = file
                .0
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("");
            match override_map.get(&basename) {
                Some((new_path, pkg)) => FileEntry {
                    file: new_path.to_path_buf(),
                    comment: Some(format!(
                        "OVERRIDDEN from {}: {}",
                        pkg,
                        file.0.to_string_lossy()
                    )),
                },
                None => FileEntry {
                    file: file.0.to_path_buf(),
                    comment: file.1.or_else(|| unparseable_comment(file.0)),
                },
            }
        })
        .collect::<IndexSet<_>>();

    if emit_sources {
        tera_context.insert("all_files", &all_files);
    }

    let mut split_srcs = vec![];
    for src in srcs {
        if src.override_files {
            continue;
        }
        separate_files_in_group(
            src,
            |f| match f {
                SourceFile::File(_, fmt) => *fmt,
                _ => None,
            },
            |src, ty, files| {
                split_srcs.push(TplSrcStruct {
                    metadata: {
                        let package = src.package.unwrap_or("None");
                        let target = src.target.reduce().to_string();
                        format!("Package({package}) Target({target})")
                    },
                    defines: {
                        let mut local_defines = IndexMap::new();
                        local_defines.extend(
                            src.defines
                                .iter()
                                .map(|(k, (_, v))| (k.to_string(), v.map(String::from))),
                        );

                        add_defines(&mut local_defines, &args.define);
                        local_defines.into_iter().collect()
                    },
                    incdirs: {
                        let mut incdirs = src
                            .get_incdirs()
                            .iter()
                            .map(|p| p.to_path_buf())
                            .collect::<IndexSet<_>>();
                        incdirs.sort();
                        incdirs
                    },
                    files: files
                        .iter()
                        .map(|f| match f {
                            SourceFile::File(p, _) => {
                                let basename = p
                                    .file_name()
                                    .and_then(std::ffi::OsStr::to_str)
                                    .unwrap_or("");
                                match override_map.get(&basename) {
                                    Some((new_path, pkg)) => FileEntry {
                                        file: new_path.to_path_buf(),
                                        comment: Some(format!(
                                            "OVERRIDDEN from {}: {}",
                                            pkg,
                                            p.to_string_lossy()
                                        )),
                                    },
                                    None => FileEntry {
                                        file: p.to_path_buf(),
                                        comment: unparseable_comment(p),
                                    },
                                }
                            }
                            SourceFile::Group(_) => unreachable!(),
                        })
                        .collect(),
                    file_type: Some(ty),
                });
            },
        );
    }
    for src in &split_srcs {
        match src.file_type {
            Some(SourceType::Verilog) => {
                all_verilog.extend(src.files.iter().cloned());
            }
            Some(SourceType::Vhdl) => {
                all_vhdl.extend(src.files.iter().cloned());
            }
            _ => {
                unknown_files.extend(src.files.iter().cloned());
            }
        }
    }
    let split_srcs = if emit_sources { split_srcs } else { vec![] };
    tera_context.insert("srcs", &split_srcs);

    let all_verilog = if emit_sources {
        all_verilog.into_iter().collect()
    } else {
        IndexSet::new()
    };
    let all_vhdl = if emit_sources {
        all_vhdl.into_iter().collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_verilog", &all_verilog);
    tera_context.insert("all_vhdl", &all_vhdl);
    if !unknown_files.is_empty() && template.contains("file_type") {
        Warnings::UnknownFileType(unknown_files.iter().map(|x| x.file.clone()).collect()).emit();
    }

    tera_context.insert("source_annotations", &args.source_annotations);
    tera_context.insert("compilation_mode", &args.compilation_mode);

    if template == "json" {
        let _ = writeln!(std::io::stdout(), "{:#}", tera_context.into_json());
        return Ok(());
    }

    let _ = write!(
        std::io::stdout(),
        "{}",
        Tera::default()
            .render_str(template, &tera_context)
            .into_diagnostic()
            .wrap_err("Failed to render template.")?
    );

    Ok(())
}

#[derive(Debug, Serialize, Hash, Eq, PartialEq, Clone)]
struct FileEntry {
    file: PathBuf,
    comment: Option<String>,
}

#[derive(Debug, Serialize)]
struct TplSrcStruct {
    metadata: String,
    defines: IndexSet<(String, Option<String>)>,
    incdirs: IndexSet<PathBuf>,
    files: IndexSet<FileEntry>,
    file_type: Option<SourceType>,
}
