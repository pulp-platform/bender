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

use crate::cmd::sources::get_passed_targets;
use crate::config::{Validate, ValidationContext};
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup, SourceType};
use crate::target::TargetSet;

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

    /// Remove source annotations from the generated script
    #[arg(long, help_heading = "General Script Options")]
    pub no_source_annotations: bool,

    /// Specify package to show sources for
    #[arg(short, long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short, long, global = true, help_heading = "General Script Options")]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short, long, action = ArgAction::Append, global = true, help_heading = "General Script Options")]
    pub exclude: Vec<String>,

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

    srcs = srcs.filter_packages(&packages).unwrap_or_default();

    // Flatten and validate the sources.
    let srcs = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate(&ValidationContext::default()))
        .collect::<Result<Vec<_>>>()?;

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
        ScriptFormat::Template { template } => &std::fs::read_to_string(template)?,
        ScriptFormat::TemplateJson => JSON,
    };

    emit_template(sess, tera_context, template_content, args, only_args, srcs)
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
) -> Result<()> {
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
    let mut all_incdirs = vec![];
    let mut all_files = IndexSet::new();
    let mut all_verilog = vec![];
    let mut all_vhdl = vec![];
    let mut unknown_files = vec![];
    for src in &srcs {
        all_defines.extend(
            src.defines
                .iter()
                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
        );
        all_incdirs.append(&mut src.clone().get_incdirs());
        all_files.extend(src.files.iter().filter_map(|file| match file {
            SourceFile::File(p, _) => Some((p.to_string_lossy().to_string(), None::<String>)),
            SourceFile::Group(_) => None,
        }));
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

    if emit_sources {
        tera_context.insert("all_files", &all_files);
    }

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
                            SourceFile::File(p, _) => (p.to_path_buf(), None),
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
            _ => {
                unknown_files.append(&mut src.files.clone().into_iter().collect());
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
        Warnings::UnknownFileType(unknown_files).emit();
    }

    tera_context.insert("source_annotations", &!args.no_source_annotations);
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
            .map_err(|e| { Error::chain("Failed to render template.", e) })?
    );

    Ok(())
}

#[derive(Debug, Serialize)]
struct TplSrcStruct {
    metadata: String,
    defines: IndexSet<(String, Option<String>)>,
    incdirs: IndexSet<PathBuf>,
    files: IndexSet<(PathBuf, Option<String>)>,
    file_type: String,
}
