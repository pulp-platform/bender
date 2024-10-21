// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use clap::builder::PossibleValue;
use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use indexmap::{IndexMap, IndexSet};
use tera::{Context, Tera};
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup};
use crate::target::{TargetSet, TargetSpec};

/// Assemble the `script` subcommand.
pub fn new() -> Command {
    Command::new("script")
        .about("Emit tool scripts for the package")
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .help("Only include sources that match the given target")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("no-default-target")
                .long("no-default-target")
                .help("Remove any default targets that may be added to the generated script")
                .num_args(0)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("format")
                .help("Format of the generated script")
                .required(true)
                .num_args(1)
                .value_parser([
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
                ]),
        )
        .arg(
            Arg::new("relative-path")
                .long("relative-path")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Use relative paths (flist generation only)"),
        )
        .arg(
            Arg::new("define")
                .short('D')
                .long("define")
                .help("Pass an additional define to all source files")
                .num_args(1..)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("vcom-arg")
                .long("vcom-arg")
                .help("Pass an argument to vcom calls (vsim/vhdlan/riviera only)")
                .num_args(1..)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("vlog-arg")
                .long("vlog-arg")
                .help("Pass an argument to vlog calls (vsim/vlogan/riviera only)")
                .num_args(1..)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("only-defines")
                .long("only-defines")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Only output commands to define macros (Vivado only)"),
        )
        .arg(
            Arg::new("only-includes")
                .long("only-includes")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Only output commands to define include directories (Vivado only)"),
        )
        .arg(
            Arg::new("only-sources")
                .long("only-sources")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Only output commands to define source files (Vivado only)"),
        )
        .arg(
            Arg::new("no-simset")
                .long("no-simset")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Do not change `simset` fileset (Vivado only)"),
        )
        .arg(
            Arg::new("vlogan-bin")
                .long("vlogan-bin")
                .help("Specify a `vlogan` command")
                .num_args(1)
                .default_value("vlogan")
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("vhdlan-bin")
                .long("vhdlan-bin")
                .help("Specify a `vhdlan` command")
                .num_args(1)
                .default_value("vhdlan")
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("no-abort-on-error")
                .long("no-abort-on-error")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Do not abort analysis/compilation on first caught error (only for programs that support early aborting)")
        )
        .arg(
            Arg::new("compilation_mode")
                .long("compilation-mode")
                .help("Choose compilation mode option: separate/common")
                .num_args(1)
                .default_value("separate")
                .value_parser([
                    PossibleValue::new("separate"),
                    PossibleValue::new("common"),
                ])
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
            Arg::new("template")
                .long("template")
                .required_if_eq("format", "template")
                .help("Path to a file containing the tera template string to be formatted.")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("assume_rtl")
                .long("assume-rtl")
                .help("Add the `rtl` target to any fileset without a target specification")
                .num_args(0)
                .action(ArgAction::SetTrue)
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

/// Execute the `script` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources())?;

    // Format-specific target specifiers.
    let vivado_targets = &["vivado", "fpga", "xilinx"];
    fn concat<T: Clone>(a: &[T], b: &[T]) -> Vec<T> {
        a.iter().chain(b).cloned().collect()
    }
    let format = matches.get_one::<String>("format").unwrap();
    let format_targets: Vec<&str> = if !matches.get_flag("no-default-target") {
        match format.as_str() {
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
    let targets = matches
        .get_many::<String>("target")
        .map(|t| {
            TargetSet::new(
                t.map(|element| element.as_str())
                    .chain(format_targets.clone()),
            )
        })
        .unwrap_or_else(|| TargetSet::new(format_targets));

    if matches.get_flag("assume_rtl") {
        srcs = srcs.assign_target("rtl".to_string());
    }

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

    // Flatten the sources.
    let srcs = srcs.flatten();

    // Validate format-specific options.
    if (matches.contains_id("vcom-arg") || matches.contains_id("vlog-arg"))
        && format != "vsim"
        && format != "vcs"
        && format != "riviera"
        && format != "template"
        && format != "template_json"
    {
        return Err(Error::new(
            "vsim/vcs-only options can only be used for 'vcs', 'vsim' or 'riviera' format!",
        ));
    }
    if (matches.get_flag("only-defines")
        || matches.get_flag("only-includes")
        || matches.get_flag("only-sources")
        || matches.get_flag("no-simset"))
        && !format.starts_with("vivado")
        && format != "template"
        && format != "template_json"
    {
        return Err(Error::new(
            "Vivado-only options can only be used for 'vivado' format!",
        ));
    }

    // Generate the corresponding output.
    match format.as_str() {
        "flist" => emit_template(
            sess,
            include_str!("../script_fmt/flist.tera"),
            matches,
            targets,
            srcs,
        ),
        "flist-plus" => emit_template(
            sess,
            include_str!("../script_fmt/flist-plus.tera"),
            matches,
            targets,
            srcs,
        ),
        "vsim" => emit_template(
            sess,
            include_str!("../script_fmt/vsim_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "vcs" => emit_template(
            sess,
            include_str!("../script_fmt/vcs_sh.tera"),
            matches,
            targets,
            srcs,
        ),
        "verilator" => emit_template(
            sess,
            include_str!("../script_fmt/verilator_sh.tera"),
            matches,
            targets,
            srcs,
        ),
        "synopsys" => emit_template(
            sess,
            include_str!("../script_fmt/synopsys_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "formality" => emit_template(
            sess,
            include_str!("../script_fmt/formality_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "riviera" => emit_template(
            sess,
            include_str!("../script_fmt/riviera_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "genus" => emit_template(
            sess,
            include_str!("../script_fmt/genus_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "vivado" => emit_template(
            sess,
            include_str!("../script_fmt/vivado_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "vivado-sim" => emit_template(
            sess,
            include_str!("../script_fmt/vivado_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "precision" => emit_template(
            sess,
            include_str!("../script_fmt/precision_tcl.tera"),
            matches,
            targets,
            srcs,
        ),
        "template" => {
            let custom_tpl_path = Path::new(matches.get_one::<String>("template").unwrap());
            let custom_tpl_str =
                &String::from_utf8(fs::read(custom_tpl_path)?).map_err(|e| Error::chain("", e))?;
            emit_template(sess, custom_tpl_str, matches, targets, srcs)
        }
        "template_json" => emit_template(sess, JSON, matches, targets, srcs),
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SourceType {
    Verilog,
    Vhdl,
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

fn add_defines_from_matches(defines: &mut IndexMap<String, Option<String>>, matches: &ArgMatches) {
    if let Some(d) = matches.get_many::<String>("define") {
        defines.extend(d.map(|t| {
            let mut parts = t.splitn(2, '=');
            let name = parts.next().unwrap().trim(); // split always has at least one element
            let value = parts.next().map(|v| v.trim().to_string());
            (name.to_string(), value)
        }));
    }
}

static JSON: &str = "json";

fn emit_template(
    sess: &Session,
    template: &str,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    let mut tera_obj = Tera::default();
    let mut tera_context = Context::new();
    tera_context.insert("HEADER_AUTOGEN", HEADER_AUTOGEN);
    tera_context.insert("root", sess.root);
    // tera_context.insert("srcs", &srcs);
    tera_context.insert("abort_on_error", &!matches.get_flag("no-abort-on-error"));

    let mut target_defines: IndexMap<String, Option<String>> = IndexMap::new();
    target_defines.extend(
        targets
            .iter()
            .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
    );
    target_defines.sort_keys();

    let mut global_defines = target_defines.clone();
    add_defines_from_matches(&mut global_defines, matches);
    tera_context.insert("global_defines", &global_defines);

    let mut all_defines = IndexMap::new();
    let mut all_incdirs = vec![];
    let mut all_files = vec![];
    let mut all_verilog = vec![];
    let mut all_vhdl = vec![];
    for src in &srcs {
        all_defines.extend(
            src.defines
                .iter()
                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
        );
        all_incdirs.append(&mut src.clone().get_incdirs());
        all_files.append(&mut src.files.clone());
    }
    all_defines.extend(target_defines.clone());
    add_defines_from_matches(&mut all_defines, matches);
    let all_defines = if (!matches.get_flag("only-includes") && !matches.get_flag("only-sources"))
        || matches.get_flag("only-defines")
    {
        all_defines.into_iter().collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_defines", &all_defines);

    all_incdirs.sort();
    let all_incdirs: IndexSet<PathBuf> = if (!matches.get_flag("only-defines")
        && !matches.get_flag("only-sources"))
        || matches.get_flag("only-includes")
    {
        all_incdirs.into_iter().map(|p| p.to_path_buf()).collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_incdirs", &all_incdirs);
    let all_files: IndexSet<PathBuf> = if (!matches.get_flag("only-defines")
        && !matches.get_flag("only-includes"))
        || matches.get_flag("only-sources")
    {
        all_files
            .into_iter()
            .filter_map(|file| match file {
                SourceFile::File(p) => Some(p.to_path_buf()),
                _ => None,
            })
            .collect()
    } else {
        IndexSet::new()
    };
    tera_context.insert("all_files", &all_files);

    let mut split_srcs = vec![];
    for src in srcs {
        separate_files_in_group(
            src,
            |f| match f {
                SourceFile::File(p) => match p.extension().and_then(std::ffi::OsStr::to_str) {
                    Some("sv") | Some("v") | Some("vp") => Some(SourceType::Verilog),
                    Some("vhd") | Some("vhdl") => Some(SourceType::Vhdl),
                    _ => None,
                },
                _ => None,
            },
            |src, ty, files| {
                split_srcs.push(TplSrcStruct {
                    defines: {
                        let mut local_defines = IndexMap::new();
                        local_defines.extend(
                            src.defines
                                .iter()
                                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                        );
                        local_defines.extend(target_defines.clone());
                        add_defines_from_matches(&mut local_defines, matches);
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
                            SourceFile::File(p) => p.to_path_buf(),
                            SourceFile::Group(_) => unreachable!(),
                        })
                        .collect(),
                    file_type: match ty {
                        SourceType::Verilog => "verilog".to_string(),
                        SourceType::Vhdl => "vhdl".to_string(),
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
    let split_srcs = if !matches.get_flag("only-defines") && !matches.get_flag("only-includes") {
        split_srcs
    } else {
        vec![]
    };
    tera_context.insert("srcs", &split_srcs);

    let all_verilog: IndexSet<PathBuf> =
        if !matches.get_flag("only-defines") && !matches.get_flag("only-includes") {
            all_verilog.into_iter().collect()
        } else {
            IndexSet::new()
        };
    let all_vhdl: IndexSet<PathBuf> =
        if !matches.get_flag("only-defines") && !matches.get_flag("only-includes") {
            all_vhdl.into_iter().collect()
        } else {
            IndexSet::new()
        };
    tera_context.insert("all_verilog", &all_verilog);
    tera_context.insert("all_vhdl", &all_vhdl);

    let vlog_args: Vec<String> = if let Some(args) = matches.get_many::<String>("vlog-arg") {
        args.map(Into::into).collect()
    } else {
        [].to_vec()
    };
    tera_context.insert("vlog_args", &vlog_args);
    let vcom_args: Vec<String> = if let Some(args) = matches.get_many::<String>("vcom-arg") {
        args.map(Into::into).collect()
    } else {
        [].to_vec()
    };
    tera_context.insert("vcom_args", &vcom_args);

    tera_context.insert("vlogan_bin", &matches.get_one::<String>("vlogan-bin"));
    tera_context.insert("vhdlan_bin", &matches.get_one::<String>("vhdlan-bin"));
    tera_context.insert("relativize_path", &matches.get_flag("relative-path"));
    tera_context.insert(
        "compilation_mode",
        &matches.get_one::<String>("compilation_mode"),
    );

    let vivado_filesets = if matches.get_flag("no-simset") {
        vec![""]
    } else {
        vec!["", " -simset"]
    };

    tera_context.insert("vivado_filesets", &vivado_filesets);

    if template == "json" {
        println!("{:#}", tera_context.into_json());
        return Ok(());
    }

    print!(
        "{}",
        tera_obj
            .render_str(template, &tera_context)
            .map_err(|e| { Error::chain("Failed to render template.", e) })?
    );

    Ok(())
}

#[derive(Debug, Serialize)]
struct TplSrcStruct {
    defines: IndexSet<(String, Option<String>)>,
    incdirs: IndexSet<PathBuf>,
    files: IndexSet<PathBuf>,
    file_type: String,
}
