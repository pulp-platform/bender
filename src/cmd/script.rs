// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use std::io::Write;
use std::path::PathBuf;

use clap::{ArgAction, Args, Subcommand, ValueEnum};
use indexmap::{IndexMap, IndexSet};
use serde::Serialize;
use tera::{Context, Tera};
use tokio::runtime::Runtime;

use crate::config::Validate;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup, SourceType};
use crate::target::TargetSet;

/// Emit tool scripts for the package
#[derive(Args, Debug)]
pub struct ScriptArgs {
    /// Only include sources that match the given target
    #[arg(short, long, action = ArgAction::Append)]
    pub target: Vec<String>,

    /// Remove any default targets that may be added to the generated script
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_default_target: bool,

    /// Pass an additional define to all source files
    #[arg(short = 'D', long, action = ArgAction::Append)]
    pub define: Vec<String>,

    /// Pass an argument to vcom calls (vsim/vhdlan/riviera/synopsys only)
    #[arg(long, action = ArgAction::Append)]
    pub vcom_arg: Vec<String>,

    /// Pass an argument to vlog calls (vsim/vlogan/riviera/synopsys only)
    #[arg(long, action = ArgAction::Append)]
    pub vlog_arg: Vec<String>,

    /// Only output commands to define macros (Vivado/flist only)
    #[arg(long, action = ArgAction::SetTrue)]
    pub only_defines: bool,

    /// Only output commands to define include directories (Vivado/flist only)
    #[arg(long, action = ArgAction::SetTrue)]
    pub only_includes: bool,

    /// Only output commands to define source files (Vivado/flist only)
    #[arg(long, action = ArgAction::SetTrue)]
    pub only_sources: bool,

    /// Do not change `simset` fileset (Vivado only)
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_simset: bool,

    /// Specify a `vlogan` command
    #[arg(long, default_value = "vlogan")]
    pub vlogan_bin: String,

    /// Specify a `vhdlan` command
    #[arg(long, default_value = "vhdlan")]
    pub vhdlan_bin: String,

    /// Do not abort analysis/compilation on first caught error (only for programs that support early aborting)
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_abort_on_error: bool,

    /// Choose compilation mode option: separate/common
    #[arg(long, default_value = "separate", value_parser = [
        PossibleValue::new("separate"),
        PossibleValue::new("common"),
    ])]
    pub compilation_mode: String,

    /// Remove source annotations from the generated script
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_source_annotations: bool,

    /// Specify package to show sources for
    #[arg(short, long, action = ArgAction::Append)]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short, long, action = ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Add the `rtl` target to any fileset without a target specification
    #[arg(long, action = ArgAction::SetTrue)]
    pub assume_rtl: bool,

    /// Ignore passed targets
    #[arg(long, action = ArgAction::SetTrue)]
    pub ignore_passed_targets: bool,
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

/// Common simulation arguments
#[derive(Args, Debug)]
pub struct CommonSimArgs {
    /// Pass an argument to vcom calls
    #[arg(long, action = ArgAction::Append)]
    pub vcom_arg: Vec<String>,

    /// Pass an argument to vlog calls
    #[arg(long, action = ArgAction::Append)]
    pub vlog_arg: Vec<String>,
}

/// Common compilation arguments
#[derive(Args, Debug)]
pub struct CommonCompileArgs {
    /// Choose compilation mode option
    #[arg(long, default_value_t, value_enum)]
    pub compilation_mode: CompilationMode,

    /// Do not abort analysis/compilation on first caught error
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_abort_on_error: bool,
}

/// Common arguments for Vivado scripts
#[derive(Args, Debug)]
pub struct OnlyArgs {
    /// Only output commands to define macros
    #[arg(long="only-defines", action = ArgAction::SetTrue)]
    pub defines: bool,

    /// Only output commands to define include directories
    #[arg(long="only-includes", action = ArgAction::SetTrue)]
    pub includes: bool,

    /// Only output commands to define source files
    #[arg(long="only-sources", action = ArgAction::SetTrue)]
    pub sources: bool,
}

// TODO(fischeti): Check if help texts are correct.
/// Script format enum
#[derive(Subcommand, Debug)]
pub enum ScriptFormat {
    /// A general file list
    Flist {
        /// Use relative paths
        #[arg(long, action = ArgAction::SetTrue)]
        relative_path: bool,
    },
    /// An extended file list with include dirs and defines
    FlistPlus {
        /// Use relative paths
        #[arg(long, action = ArgAction::SetTrue)]
        relative_path: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,
    },
    /// ModelSim/QuestaSim script
    Vsim {
        /// Common simulation arguments
        #[command(flatten)]
        common_sim: CommonSimArgs,

        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Synopsys VCS script
    Vcs {
        /// Common simulation arguments
        #[command(flatten)]
        common_sim: CommonSimArgs,

        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,

        /// Specify a `vlogan` command
        #[arg(long, default_value = "vlogan")]
        vlogan_bin: String,

        /// Specify a `vhdlan` command
        #[arg(long, default_value = "vhdlan")]
        vhdlan_bin: String,
    },
    /// Verilator script
    Verilator,
    /// Synopsys EDA tool script
    Synopsys {
        /// Common simulation arguments
        #[command(flatten)]
        common_sim: CommonSimArgs,

        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Synopsys Formality script
    Formality {
        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Riviera script
    Riviera {
        /// Common simulation arguments
        #[command(flatten)]
        common_sim: CommonSimArgs,

        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Cadence Genus script
    Genus {
        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Xilinx Vivado synthesis script
    Vivado {
        /// Do not change `simset` fileset
        #[arg(long, action = ArgAction::SetTrue)]
        no_simset: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,

        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Xilinx Vivado simulation script
    VivadoSim {
        /// Do not change `simset` fileset
        #[arg(long, action = ArgAction::SetTrue)]
        no_simset: bool,

        /// Common arguments for Vivado scripts
        #[command(flatten)]
        only: OnlyArgs,
    },
    /// Mentor Graphics Precision script
    Precision {
        /// Common compilation arguments
        #[command(flatten)]
        common_compile: CommonCompileArgs,
    },
    /// Custom template script
    Template {
        /// Path to a file containing the tera template string to be formatted.
        #[arg(long)]
        template: String,
    },
    /// JSON output
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
    let rt = Runtime::new()?;
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
            ScriptFormat::Verilator => vec!["verilator", "synthesis"],
            ScriptFormat::Synopsys { .. } => vec!["synopsys", "synthesis"],
            ScriptFormat::Formality { .. } => vec!["synopsys", "synthesis", "formality"],
            ScriptFormat::Riviera { .. } => vec!["riviera", "simulation"],
            ScriptFormat::Genus { .. } => vec!["genus", "synthesis"],
            ScriptFormat::Vivado { .. } => concat(vivado_targets, &["synthesis"]),
            ScriptFormat::VivadoSim { .. } => concat(vivado_targets, &["simulation"]),
            ScriptFormat::Precision { .. } => vec!["precision", "fpga", "synthesis"],
            ScriptFormat::Template { .. } => vec![],
            ScriptFormat::TemplateJson => vec![],
        }
    } else {
        vec![]
    };

    // Filter the sources by target.
    let targets = TargetSet::new(
        args.target
            .iter()
            .map(|s| s.as_str())
            .chain(format_targets.into_iter()),
    );

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

    // Flatten and validate the sources.
    let srcs = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate("", false, &sess.suppress_warnings))
        .collect::<Result<Vec<_>>>()?;

    let mut opts: RenderOptions = RenderOptions::default();

    // Generate the corresponding output.
    let template_content = match &args.format {
        ScriptFormat::Flist { relative_path } => {
            opts.relative_path = *relative_path;
            include_str!("../script_fmt/flist.tera").to_string()
        }
        ScriptFormat::FlistPlus {
            relative_path,
            only,
        } => {
            opts.relative_path = *relative_path;
            opts.only_defines = only.defines;
            opts.only_includes = only.includes;
            opts.only_sources = only.sources;
            include_str!("../script_fmt/flist-plus.tera").to_string()
        }
        ScriptFormat::Vsim {
            common_sim,
            common_compile,
        } => {
            opts.vcom_args = common_sim.vcom_arg.clone();
            opts.vlog_args = common_sim.vlog_arg.clone();
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/vsim_tcl.tera").to_string()
        }
        ScriptFormat::Vcs {
            vlogan_bin,
            vhdlan_bin,
            common_compile,
            common_sim,
        } => {
            opts.vcom_args = common_sim.vcom_arg.clone();
            opts.vlog_args = common_sim.vlog_arg.clone();
            opts.vlogan_bin = Some(vlogan_bin.clone());
            opts.vhdlan_bin = Some(vhdlan_bin.clone());
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/vcs_sh.tera").to_string()
        }
        ScriptFormat::Verilator => include_str!("../script_fmt/verilator_sh.tera").to_string(),
        ScriptFormat::Synopsys {
            common_sim,
            common_compile,
        } => {
            opts.vcom_args = common_sim.vcom_arg.clone();
            opts.vlog_args = common_sim.vlog_arg.clone();
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/synopsys_tcl.tera").to_string()
        }
        ScriptFormat::Formality { common_compile } => {
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/formality_tcl.tera").to_string()
        }
        ScriptFormat::Riviera {
            common_sim,
            common_compile,
        } => {
            opts.vcom_args = common_sim.vcom_arg.clone();
            opts.vlog_args = common_sim.vlog_arg.clone();
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/riviera_tcl.tera").to_string()
        }
        ScriptFormat::Genus { common_compile } => {
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/genus_tcl.tera").to_string()
        }
        ScriptFormat::Vivado {
            no_simset,
            only,
            common_compile,
        } => {
            opts.only_defines = only.defines;
            opts.only_includes = only.includes;
            opts.only_sources = only.sources;
            opts.compilation_mode = common_compile.compilation_mode;
            if *no_simset {
                opts.vivado_filesets = vec![""];
            } else {
                opts.vivado_filesets = vec!["", " -simset"];
            };
            include_str!("../script_fmt/vivado_tcl.tera").to_string()
        }
        ScriptFormat::VivadoSim { no_simset, only } => {
            opts.only_defines = only.defines;
            opts.only_includes = only.includes;
            opts.only_sources = only.sources;
            if !*no_simset {
                opts.vivado_filesets = vec!["simset"];
            }
            include_str!("../script_fmt/vivado_tcl.tera").to_string()
        }
        ScriptFormat::Precision { common_compile } => {
            opts.compilation_mode = common_compile.compilation_mode;
            opts.no_abort_on_error = common_compile.no_abort_on_error;
            include_str!("../script_fmt/precision_tcl.tera").to_string()
        }
        ScriptFormat::Template { template } => std::fs::read_to_string(template)?,
        ScriptFormat::TemplateJson => JSON.to_string(),
    };

    emit_template(sess, &template_content, args, opts, targets, srcs)
}

/// Subdivide the source files in a group.
///
/// The function `cateogrize` is used to assign a category to each source file.
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

fn relativize_path(path: &std::path::Path, root: &std::path::Path) -> String {
    if path.starts_with(root) {
        format!(
            "$ROOT/{}",
            path.strip_prefix(root).unwrap().to_str().unwrap()
        )
    } else {
        path.to_str().unwrap().to_string()
    }
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

/// Configuration for the template rendering
#[derive(Default)]
struct RenderOptions {
    // Source filtering options
    only_defines: bool,
    only_includes: bool,
    only_sources: bool,

    // Template variables
    no_abort_on_error: bool,
    relative_path: bool,
    vlog_args: Vec<String>,
    vcom_args: Vec<String>,
    vlogan_bin: Option<String>,
    vhdlan_bin: Option<String>,
    compilation_mode: CompilationMode,

    // Pre-calculated fileset list for Vivado
    vivado_filesets: Vec<&'static str>,
}

static JSON: &str = "json";

fn emit_template(
    sess: &Session,
    template: &str,
    args: &ScriptArgs,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    let mut tera_obj = Tera::default();
    let mut tera_context = Context::new();
    tera_context.insert("HEADER_AUTOGEN", HEADER_AUTOGEN);
    tera_context.insert("root", sess.root);
    // tera_context.insert("srcs", &srcs);
    tera_context.insert("abort_on_error", &!opts.no_abort_on_error);

    let mut global_defines = target_defines.clone();
    add_defines(&mut global_defines, &args.define);
    tera_context.insert("global_defines", &global_defines);

    let mut all_defines = IndexMap::new();
    let mut all_incdirs = vec![];
    let mut all_files = IndexSet::new();
    let mut all_verilog = vec![];
    let mut all_vhdl = vec![];
    for src in &srcs {
        all_defines.extend(
            src.defines
                .iter()
                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
        );
        all_incdirs.append(&mut src.clone().get_incdirs());
        all_files.extend(src.files.iter().filter_map(|file| match file {
            SourceFile::File(p, _) => Some(p.to_string_lossy().to_string()),
            SourceFile::Group(_) => None,
        }));
    }

    add_defines(&mut all_defines, &args.define);
    let all_defines = if (!opts.only_includes && !opts.only_sources) || opts.only_defines {
        all_defines.into_iter().collect()
    } else {
        IndexSet::new()
    };

    tera_context.insert("all_defines", &all_defines);

    all_incdirs.sort();
    let all_incdirs: IndexSet<PathBuf> =
        if (!opts.only_defines && !opts.only_sources) || opts.only_includes {
            all_incdirs.into_iter().map(|p| p.to_path_buf()).collect()
        } else {
            IndexSet::new()
        };
    tera_context.insert("all_incdirs", &all_incdirs);

    let all_files = if (!args.only_defines && !args.only_includes) || args.only_sources {
        all_files
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_files", &all_files);

    let mut split_srcs = vec![];
    for src in srcs {
        separate_files_in_group(
            src,
            |f| match f {
                SourceFile::File(p, fmt) => match fmt {
                    Some(SourceType::Verilog) => Some(SourceType::Verilog),
                    Some(SourceType::Vhdl) => Some(SourceType::Vhdl),
                    _ => match p.extension().and_then(std::ffi::OsStr::to_str) {
                        Some("sv") | Some("v") | Some("vp") => Some(SourceType::Verilog),
                        Some("vhd") | Some("vhdl") => Some(SourceType::Vhdl),
                        _ => Some(SourceType::Unknown),
                    },
                },
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
                                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                        );

                        add_defines(&mut local_defines, &args.define);
                        local_defines.into_iter().collect()
                    },
                    incdirs: {
                        let mut incdirs = src
                            .clone()
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
                            SourceFile::File(p, _) => p.to_path_buf(),
                            SourceFile::Group(_) => unreachable!(),
                        })
                        .collect(),
                    file_type: match ty {
                        SourceType::Verilog => "verilog".to_string(),
                        SourceType::Vhdl => "vhdl".to_string(),
                        SourceType::Unknown => "".to_string(),
                    },
                });
            },
        );
    }
    for src in &split_srcs {
        match src.file_type.as_str() {
            "verilog" => {
                all_verilog.append(&mut src.files.clone().into_iter().collect());
            }
            "vhdl" => {
                all_vhdl.append(&mut src.files.clone().into_iter().collect());
            }
            _ => {}
        }
    }
    let split_srcs = if !opts.only_defines && !opts.only_includes {
        split_srcs
    } else {
        vec![]
    };
    tera_context.insert("srcs", &split_srcs);

    let all_verilog: IndexSet<PathBuf> = if !opts.only_defines && !opts.only_includes {
        all_verilog.into_iter().collect()
    } else {
        IndexSet::new()
    };
    let all_vhdl: IndexSet<PathBuf> = if !opts.only_defines && !opts.only_includes {
        all_vhdl.into_iter().collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_verilog", &all_verilog);
    tera_context.insert("all_vhdl", &all_vhdl);

    tera_context.insert("vlog_args", &opts.vlog_args);
    tera_context.insert("vcom_args", &opts.vcom_args);

    tera_context.insert("vlogan_bin", &args.vlogan_bin);
    tera_context.insert("vhdlan_bin", &args.vhdlan_bin);
    tera_context.insert("relativize_path", &args.relative_path);
    tera_context.insert("source_annotations", &!args.no_source_annotations);
    tera_context.insert("compilation_mode", &args.compilation_mode);

    let vivado_filesets = if args.no_simset {
        vec![""]
    } else {
        vec!["", " -simset"]
    };

    tera_context.insert("vivado_filesets", &vivado_filesets);

    if template == "json" {
        let _ = writeln!(std::io::stdout(), "{:#}", tera_context.into_json());
        return Ok(());
    }

    let _ = write!(
        std::io::stdout(),
        "{}",
        tera_obj
            .render_str(template, &tera_context)
            .map_err(|e| { Error::chain("Failed to render template.", e) })?
    );

    Ok(())
}

#[derive(Debug, Serialize)]
struct TplSrcStruct {
    metadata: String,
    defines: IndexSet<(String, Option<String>)>,
    incdirs: IndexSet<PathBuf>,
    files: IndexSet<PathBuf>,
    file_type: String,
}
