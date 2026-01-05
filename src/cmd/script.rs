// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use clap::builder::PossibleValue;
use clap::{ArgAction, Args};
use indexmap::{IndexMap, IndexSet};
use tera::{Context, Tera};
use tokio::runtime::Runtime;

use crate::config::Validate;
use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup, SourceType};
use crate::target::TargetSet;

/// Assemble the `script` subcommand.
// pub fn new() -> Command {
//     Command::new("script")
//         .about("Emit tool scripts for the package")
//         .arg(
//             Arg::new("target")
//                 .short('t')
//                 .long("target")
//                 .help("Only include sources that match the given target")
//                 .num_args(1)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("no-default-target")
//                 .long("no-default-target")
//                 .help("Remove any default targets that may be added to the generated script")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue),
//         )
//         .arg(
//             Arg::new("format")
//                 .help("Format of the generated script")
//                 .required(true)
//                 .num_args(1)
//                 .value_parser([
//                     PossibleValue::new("flist"),
//                     PossibleValue::new("flist-plus"),
//                     PossibleValue::new("vsim"),
//                     PossibleValue::new("vcs"),
//                     PossibleValue::new("verilator"),
//                     PossibleValue::new("synopsys"),
//                     PossibleValue::new("formality"),
//                     PossibleValue::new("riviera"),
//                     PossibleValue::new("genus"),
//                     PossibleValue::new("vivado"),
//                     PossibleValue::new("vivado-sim"),
//                     PossibleValue::new("precision"),
//                     PossibleValue::new("template"),
//                     PossibleValue::new("template_json"),
//                 ]),
//         )
//         .arg(
//             Arg::new("relative-path")
//                 .long("relative-path")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Use relative paths (flist generation only)"),
//         )
//         .arg(
//             Arg::new("define")
//                 .short('D')
//                 .long("define")
//                 .help("Pass an additional define to all source files")
//                 .num_args(1..)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("vcom-arg")
//                 .long("vcom-arg")
//                 .help("Pass an argument to vcom calls (vsim/vhdlan/riviera/synopsys only)")
//                 .num_args(1..)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("vlog-arg")
//                 .long("vlog-arg")
//                 .help("Pass an argument to vlog calls (vsim/vlogan/riviera/synopsys only)")
//                 .num_args(1..)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("only-defines")
//                 .long("only-defines")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Only output commands to define macros (Vivado/flist only)"),
//         )
//         .arg(
//             Arg::new("only-includes")
//                 .long("only-includes")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Only output commands to define include directories (Vivado/flist only)"),
//         )
//         .arg(
//             Arg::new("only-sources")
//                 .long("only-sources")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Only output commands to define source files (Vivado/flist only)"),
//         )
//         .arg(
//             Arg::new("no-simset")
//                 .long("no-simset")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Do not change `simset` fileset (Vivado only)"),
//         )
//         .arg(
//             Arg::new("vlogan-bin")
//                 .long("vlogan-bin")
//                 .help("Specify a `vlogan` command")
//                 .num_args(1)
//                 .default_value("vlogan")
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("vhdlan-bin")
//                 .long("vhdlan-bin")
//                 .help("Specify a `vhdlan` command")
//                 .num_args(1)
//                 .default_value("vhdlan")
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("no-abort-on-error")
//                 .long("no-abort-on-error")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Do not abort analysis/compilation on first caught error (only for programs that support early aborting)")
//         )
//         .arg(
//             Arg::new("compilation_mode")
//                 .long("compilation-mode")
//                 .help("Choose compilation mode option: separate/common")
//                 .num_args(1)
//                 .default_value("separate")
//                 .value_parser([
//                     PossibleValue::new("separate"),
//                     PossibleValue::new("common"),
//                 ])
//         )
//         .arg(
//             Arg::new("package")
//                 .short('p')
//                 .long("package")
//                 .help("Specify package to show sources for")
//                 .num_args(1)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("no_deps")
//                 .short('n')
//                 .long("no-deps")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//                 .help("Exclude all dependencies, i.e. only top level or specified package(s)"),
//         )
//         .arg(
//             Arg::new("exclude")
//                 .short('e')
//                 .long("exclude")
//                 .help("Specify package to exclude from sources")
//                 .num_args(1)
//                 .action(ArgAction::Append)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("template")
//                 .long("template")
//                 .required_if_eq("format", "template")
//                 .help("Path to a file containing the tera template string to be formatted.")
//                 .num_args(1)
//                 .value_parser(value_parser!(String)),
//         )
//         .arg(
//             Arg::new("assume_rtl")
//                 .long("assume-rtl")
//                 .help("Add the `rtl` target to any fileset without a target specification")
//                 .num_args(0)
//                 .action(ArgAction::SetTrue)
//         )
// }

/// Emit tool scripts for the package
#[derive(Args, Debug)]
pub struct ScriptArgs {
    /// Format of the generated script
    #[arg(value_parser = [
        PossibleValue::new("flist"),
        PossibleValue::new("flist-plus"),
        PossibleValue::new("vsim"),
        PossibleValue::new("vcs"),
        PossibleValue::new("verilator"),
        PossibleValue::new("synopsys"),
        PossibleValue::new("formality"),
        PossibleValue::new("riviera"),
        PossibleValue::new("genus"),
        PossibleValue::new("vivado"),
        PossibleValue::new("vivado-sim"),
        PossibleValue::new("precision"),
        PossibleValue::new("template"),
        PossibleValue::new("template_json"),
    ])]
    pub format: String,

    /// Only include sources that match the given target
    #[arg(short = 't', long = "target", action = ArgAction::Append)]
    pub target: Vec<String>,

    /// Remove any default targets that may be added to the generated script
    #[arg(long = "no-default-target", action = ArgAction::SetTrue)]
    pub no_default_target: bool,

    /// Use relative paths (flist generation only)
    #[arg(long = "relative-path", action = ArgAction::SetTrue)]
    pub relative_path: bool,

    /// Pass an additional define to all source files
    #[arg(short = 'D', long = "define", action = ArgAction::Append)]
    pub define: Vec<String>,

    /// Pass an argument to vcom calls (vsim/vhdlan/riviera/synopsys only)
    #[arg(long = "vcom-arg", action = ArgAction::Append)]
    pub vcom_arg: Vec<String>,

    /// Pass an argument to vlog calls (vsim/vlogan/riviera/synopsys only)
    #[arg(long = "vlog-arg", action = ArgAction::Append)]
    pub vlog_arg: Vec<String>,

    /// Only output commands to define macros (Vivado/flist only)
    #[arg(long = "only-defines", action = ArgAction::SetTrue)]
    pub only_defines: bool,

    /// Only output commands to define include directories (Vivado/flist only)
    #[arg(long = "only-includes", action = ArgAction::SetTrue)]
    pub only_includes: bool,

    /// Only output commands to define source files (Vivado/flist only)
    #[arg(long = "only-sources", action = ArgAction::SetTrue)]
    pub only_sources: bool,

    /// Do not change `simset` fileset (Vivado only)
    #[arg(long = "no-simset", action = ArgAction::SetTrue)]
    pub no_simset: bool,

    /// Specify a `vlogan` command
    #[arg(long = "vlogan-bin", default_value = "vlogan")]
    pub vlogan_bin: String,

    /// Specify a `vhdlan` command
    #[arg(long = "vhdlan-bin", default_value = "vhdlan")]
    pub vhdlan_bin: String,

    /// Do not abort analysis/compilation on first caught error (only for programs that support early aborting)
    #[arg(long = "no-abort-on-error", action = ArgAction::SetTrue)]
    pub no_abort_on_error: bool,

    /// Choose compilation mode option: separate/common
    #[arg(long = "compilation_mode", default_value = "separate", value_parser = [
        PossibleValue::new("separate"),
        PossibleValue::new("common"),
    ])]
    pub compilation_mode: String,

    /// Remove source annotations from the generated script
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_source_annotations: bool,

    /// Specify package to show sources for
    #[arg(short = 'p', long = "package", action = ArgAction::Append)]
    pub package: Vec<String>,

    /// Exclude all dependencies, i.e. only top level or specified package(s)
    #[arg(short = 'n', long = "no-deps", action = ArgAction::SetTrue)]
    pub no_deps: bool,

    /// Specify package to exclude from sources
    #[arg(short = 'e', long = "exclude", action = ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Path to a file containing the tera template string to be formatted.
    #[arg(long = "template", required_if_eq("format", "template"))]
    pub template: Option<String>,

    /// Add the `rtl` target to any fileset without a target specification
    #[arg(long = "assume-rtl", action = ArgAction::SetTrue)]
    pub assume_rtl: bool,

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
        match args.format.as_str() {
            "flist" => vec!["flist"],
            "flist-plus" => vec!["flist"],
            "vsim" => vec!["vsim", "simulation"],
            "vcs" => vec!["vcs", "simulation"],
            "verilator" => vec!["verilator", "synthesis"],
            "synopsys" => vec!["synopsys", "synthesis"],
            "formality" => vec!["synopsys", "synthesis", "formality"],
            "riviera" => vec!["riviera", "simulation"],
            "genus" => vec!["genus", "synthesis"],
            "vivado" => concat(vivado_targets, &["synthesis"]),
            "vivado-sim" => concat(vivado_targets, &["simulation"]),
            "precision" => vec!["precision", "fpga", "synthesis"],
            "template" => vec![],
            "template_json" => vec![],
            _ => unreachable!(),
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

    // Validate format-specific options.
    if (!args.vcom_arg.is_empty() || !args.vlog_arg.is_empty())
        && args.format != "vsim"
        && args.format != "vcs"
        && args.format != "riviera"
        && args.format != "synopsys"
        && args.format != "template"
        && args.format != "template_json"
    {
        return Err(Error::new(
            "vsim/vcs-only options can only be used for 'vcs', 'vsim' or 'riviera' format!",
        ));
    }
    if (args.only_defines || args.only_includes || args.only_sources)
        && !args.format.starts_with("vivado")
        && args.format != "template"
        && args.format != "template_json"
        && !args.format.starts_with("flist")
    {
        return Err(Error::new(
            "only-x options can only be used for 'vivado', 'flist', or custom format!",
        ));
    }

    if args.no_simset && !args.format.starts_with("vivado") {
        return Err(Error::new(
            "Vivado-only options can only be used for 'vivado' format!",
        ));
    }

    // Generate the corresponding output.
    match args.format.as_str() {
        "flist" => emit_template(sess, include_str!("../script_fmt/flist.tera"), args, srcs),
        "flist-plus" => emit_template(
            sess,
            include_str!("../script_fmt/flist-plus.tera"),
            args,
            srcs,
        ),
        "vsim" => emit_template(
            sess,
            include_str!("../script_fmt/vsim_tcl.tera"),
            args,
            srcs,
        ),
        "vcs" => emit_template(sess, include_str!("../script_fmt/vcs_sh.tera"), args, srcs),
        "verilator" => emit_template(
            sess,
            include_str!("../script_fmt/verilator_sh.tera"),
            args,
            srcs,
        ),
        "synopsys" => emit_template(
            sess,
            include_str!("../script_fmt/synopsys_tcl.tera"),
            args,
            srcs,
        ),
        "formality" => emit_template(
            sess,
            include_str!("../script_fmt/formality_tcl.tera"),
            args,
            srcs,
        ),
        "riviera" => emit_template(
            sess,
            include_str!("../script_fmt/riviera_tcl.tera"),
            args,
            srcs,
        ),
        "genus" => emit_template(
            sess,
            include_str!("../script_fmt/genus_tcl.tera"),
            args,
            srcs,
        ),
        "vivado" => emit_template(
            sess,
            include_str!("../script_fmt/vivado_tcl.tera"),
            args,
            srcs,
        ),
        "vivado-sim" => emit_template(
            sess,
            include_str!("../script_fmt/vivado_tcl.tera"),
            args,
            srcs,
        ),
        "precision" => emit_template(
            sess,
            include_str!("../script_fmt/precision_tcl.tera"),
            args,
            srcs,
        ),
        "template" => {
            let custom_tpl_path = Path::new(args.template.as_ref().unwrap());
            let custom_tpl_str =
                &String::from_utf8(fs::read(custom_tpl_path)?).map_err(|e| Error::chain("", e))?;
            emit_template(sess, custom_tpl_str, args, srcs)
        }
        "template_json" => emit_template(sess, JSON, args, srcs),
        _ => unreachable!(),
    }
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
    tera_context.insert("abort_on_error", &!args.no_abort_on_error);

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
    let all_defines = if (!args.only_includes && !args.only_sources) || args.only_defines {
        all_defines.into_iter().collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_defines", &all_defines);

    all_incdirs.sort();
    let all_incdirs: IndexSet<PathBuf> =
        if (!args.only_defines && !args.only_sources) || args.only_includes {
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
    let split_srcs = if !args.only_defines && !args.only_includes {
        split_srcs
    } else {
        vec![]
    };
    tera_context.insert("srcs", &split_srcs);

    let all_verilog: IndexSet<PathBuf> = if !args.only_defines && !args.only_includes {
        all_verilog.into_iter().collect()
    } else {
        IndexSet::new()
    };
    let all_vhdl: IndexSet<PathBuf> = if !args.only_defines && !args.only_includes {
        all_vhdl.into_iter().collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_verilog", &all_verilog);
    tera_context.insert("all_vhdl", &all_vhdl);

    tera_context.insert("vlog_args", &args.vlog_arg);
    tera_context.insert("vcom_args", &args.vcom_arg);

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
