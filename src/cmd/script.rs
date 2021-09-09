// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use clap::{App, Arg, ArgMatches, SubCommand};
use tokio_core::reactor::Core;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup};
use crate::target::{TargetSet, TargetSpec};

use std::collections::HashSet;

/// Assemble the `script` subcommand.
pub fn new<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("script")
        .about("Emit tool scripts for the package")
        .arg(
            Arg::with_name("target")
                .short("t")
                .long("target")
                .help("Only include sources that match the given target")
                .takes_value(true)
                .multiple(true)
                .number_of_values(1),
        )
        .arg(
            Arg::with_name("format")
                .help("Format of the generated script")
                .required(true)
                .possible_values(&[
                    "flist",
                    "vsim",
                    "vcs",
                    "verilator",
                    "synopsys",
                    "riviera",
                    "genus",
                    "vivado",
                    "vivado-sim",
                ]),
        )
        .arg(
            Arg::with_name("relative-path")
                .long("relative-path")
                .help("Use relative paths (flist generation only)"),
        )
        .arg(
            Arg::with_name("define")
                .short("D")
                .long("define")
                .help("Pass an additional define to all source files")
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("vcom-arg")
                .long("vcom-arg")
                .help("Pass an argument to vcom calls (vsim/vhdlan/riviera only)")
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("vlog-arg")
                .long("vlog-arg")
                .help("Pass an argument to vlog calls (vsim/vlogan/riviera only)")
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("only-defines")
                .long("only-defines")
                .help("Only output commands to define macros (Vivado only)"),
        )
        .arg(
            Arg::with_name("only-includes")
                .long("only-includes")
                .help("Only output commands to define include directories (Vivado only)"),
        )
        .arg(
            Arg::with_name("only-sources")
                .long("only-sources")
                .help("Only output commands to define source files (Vivado only)"),
        )
        .arg(
            Arg::with_name("no-simset")
                .long("no-simset")
                .help("Do not change `simset` fileset (Vivado only)"),
        )
        .arg(
            Arg::with_name("vlogan-bin")
                .long("vlogan-bin")
                .help("Specify a `vlogan` command")
                .takes_value(true)
                .multiple(false)
                .default_value("vlogan")
                .number_of_values(1),
        )
        .arg(
            Arg::with_name("vhdlan-bin")
                .long("vhdlan-bin")
                .help("Specify a `vhdlan` command")
                .takes_value(true)
                .multiple(false)
                .default_value("vhdlan")
                .number_of_values(1),
        )
        .arg(
            Arg::with_name("no-abort-on-error")
                .long("no-abort-on-error")
                .help("Do not abort analysis/compilation on first caught error (only for programs that support early aborting)")
        )
        .arg(
            Arg::with_name("compilation_mode")
                .long("compilation-mode")
                .help("Choose compilation mode for Riviera-PRO option: separate/common (Riviera-PRO only)")
                .takes_value(true)
                .multiple(false)
                 .default_value("common")
                .possible_values(&[
                    "separate",
                    "common",
                ])
        )
}

/// Execute the `script` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());
    let srcs = core.run(io.sources())?;

    // Format-specific target specifiers.
    let vivado_targets = &["vivado", "fpga", "xilinx"];
    fn concat<T: Clone>(a: &[T], b: &[T]) -> Vec<T> {
        a.iter().chain(b).cloned().collect()
    }
    let format = matches.value_of("format").unwrap();
    let format_targets: Vec<&str> = match format {
        "flist" => vec!["flist"],
        "vsim" => vec!["vsim", "simulation"],
        "vcs" => vec!["vcs", "simulation"],
        "verilator" => vec!["verilator", "synthesis"],
        "synopsys" => vec!["synopsys", "synthesis"],
        "riviera" => vec!["riviera", "simulation"],
        "genus" => vec!["genus", "synthesis"],
        "vivado" => concat(vivado_targets, &["synthesis"]),
        "vivado-sim" => concat(vivado_targets, &["simulation"]),
        _ => unreachable!(),
    };

    let abort_on_error = !matches.is_present("no-abort-on-error");
    //////riviera compilation mode
    let mut riviera_separate_compilation_mode = false;
    if matches.value_of("compilation_mode").unwrap() == "separate" {
        riviera_separate_compilation_mode = true;
    }
    // Filter the sources by target.
    let targets = matches
        .values_of("target")
        .map(|t| TargetSet::new(t.chain(format_targets.clone())))
        .unwrap_or_else(|| TargetSet::new(format_targets));
    let srcs = srcs
        .filter_targets(&targets)
        .unwrap_or_else(|| SourceGroup {
            package: Default::default(),
            independent: true,
            target: TargetSpec::Wildcard,
            include_dirs: Default::default(),
            defines: Default::default(),
            files: Default::default(),
        });

    // Flatten the sources.
    let srcs = srcs.flatten();

    // Validate format-specific options.
    if (matches.is_present("vcom-arg") || matches.is_present("vlog-arg"))
        && format != "vsim"
        && format != "vcs"
        && format != "riviera"
    {
        return Err(Error::new(
            "vsim/vcs-only options can only be used for 'vcs', 'vsim' or 'riviera' format!",
        ));
    }
    if (matches.is_present("only-defines")
        || matches.is_present("only-includes")
        || matches.is_present("only-sources")
        || matches.is_present("no-simset"))
        && !format.starts_with("vivado")
    {
        return Err(Error::new(
            "Vivado-only options can only be used for 'vivado' format!",
        ));
    }

    // Generate the corresponding output.
    match format {
        "flist" => emit_flist(sess, matches, srcs),
        "vsim" => emit_vsim_tcl(sess, matches, targets, srcs, abort_on_error),
        "vcs" => emit_vcs_sh(sess, matches, targets, srcs),
        "verilator" => emit_verilator_sh(sess, matches, targets, srcs),
        "synopsys" => emit_synopsys_tcl(sess, matches, targets, srcs, abort_on_error),
        "riviera" => emit_riviera_tcl(
            sess,
            matches,
            targets,
            srcs,
            abort_on_error,
            riviera_separate_compilation_mode,
        ),
        "genus" => emit_genus_tcl(sess, matches, targets, srcs),
        "vivado" => emit_vivado_tcl(sess, matches, targets, srcs),
        "vivado-sim" => emit_vivado_tcl(sess, matches, targets, srcs),
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
    for file in std::mem::replace(&mut src.files, vec![]) {
        let new_category = categorize(&file);
        if new_category.is_none() {
            continue;
        }
        if category.is_some() && category != new_category {
            if !files.is_empty() {
                consume(
                    &src,
                    std::mem::replace(&mut category, None).unwrap(),
                    std::mem::replace(&mut files, vec![]),
                );
            }
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

fn quote(s: &(impl std::fmt::Display + ?Sized)) -> String {
    format!("\"{}\"", s)
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

fn header_tcl(sess: &Session) -> String {
    let mut lines = vec![];
    lines.push(format!("# {}", HEADER_AUTOGEN));
    lines.push(format!("set ROOT {}", quote(sess.root.to_str().unwrap())));
    lines.join("\n")
}

fn header_sh(sess: &Session) -> String {
    let mut lines = vec![];
    lines.push("#!/usr/bin/env bash".to_string());
    lines.push(format!("# {}", HEADER_AUTOGEN));
    lines.push(format!("ROOT={}", quote(sess.root.to_str().unwrap())));
    lines.join("\n")
}

fn tcl_catch_prefix(cmd: &str, do_prefix: bool) -> String {
    let prefix = if do_prefix { "if {[catch {" } else { "" };
    return format!("{}{}", prefix, cmd);
}

fn tcl_catch_postfix() -> &'static str {
    return "}]} {return 1}";
}

fn add_defines_from_matches(defines: &mut Vec<(String, Option<String>)>, matches: &ArgMatches) {
    if let Some(d) = matches.values_of("define") {
        defines.extend(d.map(|t| {
            let mut parts = t.splitn(2, "=");
            let name = parts.next().unwrap().trim(); // split always has at least one element
            let value = parts.next().map(|v| v.trim().to_string());
            (name.to_string(), value)
        }));
    }
}

/// Emit a vsim compilation script.
fn emit_vsim_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
    abort_on_error: bool,
) -> Result<()> {
    println!("{}", header_tcl(sess));
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
                let mut lines = vec![];
                match ty {
                    SourceType::Verilog => {
                        lines.push(tcl_catch_prefix("vlog -incr -sv", abort_on_error).to_owned());
                        if let Some(args) = matches.values_of("vlog-arg") {
                            lines.extend(args.map(Into::into));
                        }
                        let mut defines: Vec<(String, Option<String>)> = vec![];
                        defines.extend(
                            src.defines
                                .iter()
                                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                        );
                        defines.extend(
                            targets
                                .iter()
                                .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                        );
                        add_defines_from_matches(&mut defines, matches);
                        defines.sort();
                        for (k, v) in defines {
                            let mut s = format!("+define+{}", k.to_uppercase());
                            if let Some(v) = v {
                                s.push('=');
                                s.push_str(&v);
                            }
                            lines.push(s);
                        }
                        for i in &src.include_dirs {
                            lines
                                .push(quote(&format!("+incdir+{}", relativize_path(i, sess.root))));
                        }
                    }
                    SourceType::Vhdl => {
                        lines.push(tcl_catch_prefix("vcom -2008", abort_on_error).to_owned());
                        if let Some(args) = matches.values_of("vcom-arg") {
                            lines.extend(args.map(Into::into));
                        }
                    }
                }
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    lines.push(quote(&relativize_path(p, sess.root)));
                }
                println!("");
                println!("{}", lines.join(" \\\n    "));
                if abort_on_error {
                    println!("{}", tcl_catch_postfix());
                }
            },
        );
    }
    Ok(())
}

/// Emit a vcs compilation script.
fn emit_vcs_sh(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    println!("{}", header_sh(sess));
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
                let mut lines = vec![];
                match ty {
                    SourceType::Verilog => {
                        lines.push(format!(
                            "{} -sverilog",
                            matches.value_of("vlogan-bin").unwrap()
                        ));
                        // Default flags
                        lines.push("-full64".to_owned());
                        if let Some(args) = matches.values_of("vlog-arg") {
                            lines.extend(args.map(Into::into));
                        }
                        let mut defines: Vec<(String, Option<String>)> = vec![];
                        defines.extend(
                            src.defines
                                .iter()
                                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                        );
                        defines.extend(
                            targets
                                .iter()
                                .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                        );
                        add_defines_from_matches(&mut defines, matches);
                        defines.sort();
                        for (k, v) in defines {
                            let mut s = format!("+define+{}", k.to_uppercase());
                            if let Some(v) = v {
                                s.push('=');
                                s.push_str(&v);
                            }
                            lines.push(s);
                        }
                        for i in &src.include_dirs {
                            lines
                                .push(quote(&format!("+incdir+{}", relativize_path(i, sess.root))));
                        }
                    }
                    SourceType::Vhdl => {
                        lines.push("vhdlan".to_owned());
                        if let Some(args) = matches.values_of("vcom-arg") {
                            lines.extend(args.map(Into::into));
                        }
                    }
                }
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    lines.push(quote(&relativize_path(p, sess.root)));
                }
                println!("");
                println!("{}", lines.join(" \\\n    "));
            },
        );
    }
    Ok(())
}

/// Emit a verilator compilation script.
fn emit_verilator_sh(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    for src in srcs {
        separate_files_in_group(
            src,
            |f| match f {
                SourceFile::File(p) => match p.extension().and_then(std::ffi::OsStr::to_str) {
                    Some("sv") | Some("v") | Some("vp") => Some(SourceType::Verilog),
                    _ => None,
                },
                _ => None,
            },
            |src, ty, files| {
                let mut lines = vec![];
                match ty {
                    SourceType::Verilog => {
                        // Default flags
                        if let Some(args) = matches.values_of("vlog-arg") {
                            lines.extend(args.map(Into::into));
                        }
                        let mut defines: Vec<(String, Option<String>)> = vec![];
                        defines.extend(
                            src.defines
                                .iter()
                                .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                        );
                        defines.extend(
                            targets
                                .iter()
                                .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                        );
                        add_defines_from_matches(&mut defines, matches);
                        defines.sort();
                        for (k, v) in defines {
                            let mut s = format!("+define+{}", k.to_uppercase());
                            if let Some(v) = v {
                                s.push('=');
                                s.push_str(&v);
                            }
                            lines.push(s);
                        }
                        for i in &src.include_dirs {
                            if i.starts_with(sess.root) {
                                lines.push(format!(
                                    "+incdir+{}/{}",
                                    sess.root.to_str().unwrap(),
                                    i.strip_prefix(sess.root).unwrap().to_str().unwrap()
                                ));
                            } else {
                                lines.push(format!("+incdir+{}", i.to_str().unwrap()));
                            }
                        }
                    }
                    _ => {}
                }
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    if p.starts_with(sess.root) {
                        lines.push(format!(
                            "{}/{}",
                            sess.root.to_str().unwrap(),
                            p.strip_prefix(sess.root).unwrap().to_str().unwrap()
                        ));
                    } else {
                        lines.push(format!("{}", p.to_str().unwrap()));
                    }
                }
                println!("");
                println!("{}", lines.join("\n"));
            },
        );
    }
    Ok(())
}

/// Emit a flat file list
fn emit_flist(sess: &Session, matches: &ArgMatches, srcs: Vec<SourceGroup>) -> Result<()> {
    let mut lines = vec![];
    let mut inc_dirs = HashSet::new();
    let mut files = vec![];
    // Gobble double includes with a HashSet.
    for src in srcs {
        inc_dirs = src
            .include_dirs
            .into_iter()
            .fold(HashSet::new(), |mut acc, inc_dir| {
                acc.insert(inc_dir);
                acc
            });
        files.append(&mut src.files.clone());
    }

    let mut root = format!("{}/", sess.root.to_str().unwrap());
    if matches.is_present("relative-path") {
        root = "".to_string();
    }

    for i in &inc_dirs {
        if i.starts_with(sess.root) {
            lines.push(format!(
                "+incdir+{}{}",
                root,
                i.strip_prefix(sess.root).unwrap().to_str().unwrap()
            ));
        } else {
            lines.push(format!("+incdir+{}", i.to_str().unwrap()));
        }
    }

    for file in files {
        let p = match file {
            SourceFile::File(p) => p,
            _ => continue,
        };
        if p.starts_with(sess.root) {
            lines.push(format!(
                "{}{}",
                root,
                p.strip_prefix(sess.root).unwrap().to_str().unwrap()
            ));
        } else {
            lines.push(format!("{}", p.to_str().unwrap()));
        }
    }
    println!("{}", lines.join("\n"));
    Ok(())
}

/// Emit a Synopsys Design Compiler compilation script.
fn emit_synopsys_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
    abort_on_error: bool,
) -> Result<()> {
    println!("{}", header_tcl(sess));
    println!("set search_path_initial $search_path");
    let relativize_path = |p: &std::path::Path| quote(&relativize_path(p, sess.root));
    for src in srcs {
        // Adjust the search path.
        println!("");
        println!("set search_path $search_path_initial");
        for i in &src.include_dirs {
            println!("lappend search_path {}", relativize_path(i));
        }

        // Emit analyze commands.
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
                let mut lines = vec![];
                lines.push(
                    tcl_catch_prefix(
                        &format!(
                            "analyze -format {}",
                            match ty {
                                SourceType::Verilog => {
                                    "sv"
                                }
                                SourceType::Vhdl => {
                                    "vhdl"
                                }
                            }
                        ),
                        abort_on_error,
                    )
                    .to_owned(),
                );

                // Add defines.
                let mut defines: Vec<(String, Option<String>)> = vec![];
                defines.extend(
                    src.defines
                        .iter()
                        .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                );
                defines.extend(
                    targets
                        .iter()
                        .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                );
                add_defines_from_matches(&mut defines, matches);
                defines.sort();
                if !defines.is_empty() {
                    lines.push("-define {".to_owned());
                    for (k, v) in defines {
                        let mut s = format!("    {}", k);
                        if let Some(v) = v {
                            s.push('=');
                            s.push_str(&v);
                        }
                        lines.push(s);
                    }
                    lines.push("}".to_owned());
                }

                // Add files.
                lines.push("[list".to_owned());
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    lines.push(format!("    {}", relativize_path(p)));
                }
                lines.push("]".to_owned());
                println!("");
                println!("{}", lines.join(" \\\n    "));
                if abort_on_error {
                    println!("{}", tcl_catch_postfix());
                }
            },
        );
    }
    println!("");
    println!("set search_path $search_path_initial");
    Ok(())
}

/// Emit a Cadence Genus compilation script.
fn emit_genus_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    println!("# This script was generated automatically by bender.");
    println!("if [ info exists search_path ] {{");
    println!("  set search_path_initial $search_path");
    println!("}} else {{");
    println!("  set search_path_initial {{}}");
    println!("}}");
    println!("set ROOT \"{}\"", sess.root.to_str().unwrap());
    let relativize_path = |p: &std::path::Path| {
        if p.starts_with(sess.root) {
            format!(
                "\"$ROOT/{}\"",
                p.strip_prefix(sess.root).unwrap().to_str().unwrap()
            )
        } else {
            format!("\"{}\"", p.to_str().unwrap())
        }
    };
    for src in srcs {
        // Adjust the search path.
        println!("");
        println!("set search_path $search_path_initial");
        for i in &src.include_dirs {
            println!("lappend search_path {}", relativize_path(i));
        }

        println!("set_db init_hdl_search_path $search_path");

        // Emit analyze commands.
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
                let mut lines = vec![];
                match ty {
                    SourceType::Verilog => {
                        lines.push("read_hdl -language sv".to_owned());
                    }
                    SourceType::Vhdl => {
                        lines.push("read_hdl -language vhdl".to_owned());
                    }
                }

                // Add defines.
                let mut defines: Vec<(String, Option<String>)> = vec![];
                defines.extend(
                    src.defines
                        .iter()
                        .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                );
                defines.extend(
                    targets
                        .iter()
                        .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                );
                add_defines_from_matches(&mut defines, matches);
                defines.sort();
                if !defines.is_empty() {
                    lines.push("-define {".to_owned());
                    for (k, v) in defines {
                        let mut s = format!("    {}", k);
                        if let Some(v) = v {
                            s.push('=');
                            s.push_str(&v);
                        }
                        lines.push(s);
                    }
                    lines.push("}".to_owned());
                }

                // Add files.
                lines.push("[list".to_owned());
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    lines.push(format!("    {}", relativize_path(p)));
                }
                lines.push("]".to_owned());
                println!("");
                println!("{}", lines.join(" \\\n    "));
            },
        );
    }
    println!("");
    println!("set search_path $search_path_initial");
    Ok(())
}

/// Emit a script to add sources to Vivado.
fn emit_vivado_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    // Determine the components that are part of the output.
    #[derive(Default)]
    struct OutputComponents {
        include_dirs: bool,
        defines: bool,
        sources: bool,
    }
    let mut output_components = OutputComponents::default();
    if !matches.is_present("only-defines")
        && !matches.is_present("only-includes")
        && !matches.is_present("only-sources")
    {
        // Print everything if user specified no restriction.
        output_components = OutputComponents {
            include_dirs: true,
            defines: true,
            sources: true,
        };
    } else {
        if matches.is_present("only-defines") {
            output_components.defines = true;
        }
        if matches.is_present("only-includes") {
            output_components.include_dirs = true;
        }
        if matches.is_present("only-sources") {
            output_components.sources = true;
        }
    }

    println!("{}", header_tcl(sess));
    let mut include_dirs = vec![];
    let mut defines: Vec<(String, Option<String>)> = vec![];
    let filesets = if matches.is_present("no-simset") {
        vec![""]
    } else {
        vec!["", " -simset"]
    };
    for src in srcs {
        for i in &src.include_dirs {
            include_dirs.push(relativize_path(i, sess.root));
        }
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
            |src, _ty, files| {
                let mut lines = vec![];
                lines.push("add_files -norecurse -fileset [current_fileset] [list".to_owned());
                for file in files {
                    let p = match file {
                        SourceFile::File(p) => p,
                        _ => continue,
                    };
                    lines.push(relativize_path(p, sess.root));
                }
                if output_components.sources {
                    println!("{} \\\n]", lines.join(" \\\n    "));
                }
                defines.extend(
                    src.defines
                        .iter()
                        .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                );
            },
        );
    }
    if !include_dirs.is_empty() && output_components.include_dirs {
        include_dirs.sort();
        include_dirs.dedup();
        for arg in &filesets {
            println!("");
            println!(
                "set_property include_dirs [list \\\n    {} \\\n] [current_fileset{}]",
                include_dirs.join(" \\\n    "),
                arg
            );
        }
    }
    defines.extend(
        targets
            .iter()
            .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
    );
    add_defines_from_matches(&mut defines, matches);
    if !defines.is_empty() && output_components.defines {
        defines.sort();
        defines.dedup();
        for arg in &filesets {
            println!("");
            println!("set_property verilog_define [list \\");
            for (k, v) in &defines {
                let s = match v {
                    Some(s) => format!("{}={}", k, s),
                    None => format!("{}", k),
                };
                println!("    {} \\", s);
            }
            println!("] [current_fileset{}]", arg);
        }
    }
    Ok(())
}

/// Emit a riviera compilation script.
fn emit_riviera_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
    abort_on_error: bool,
    riviera_separate_compilation_mode: bool,
) -> Result<()> {
    println!("{}", header_tcl(sess));
    println!("vlib work");

    if riviera_separate_compilation_mode {
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
                    let mut lines = vec![];
                    match ty {
                        SourceType::Verilog => {
                            lines.push(tcl_catch_prefix("vlog -sv", abort_on_error).to_owned());
                            if let Some(args) = matches.values_of("vlog-arg") {
                                lines.extend(args.map(Into::into));
                            }
                            let mut defines: Vec<(String, Option<String>)> = vec![];
                            defines.extend(
                                src.defines
                                    .iter()
                                    .map(|(k, &v)| (k.to_string(), v.map(String::from))),
                            );
                            defines.extend(
                                targets
                                    .iter()
                                    .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
                            );
                            add_defines_from_matches(&mut defines, matches);
                            defines.sort();
                            for (k, v) in defines {
                                let mut s = format!("+define+{}", k.to_uppercase());
                                if let Some(v) = v {
                                    s.push('=');
                                    s.push_str(&v);
                                }
                                lines.push(s);
                            }
                            for i in &src.include_dirs {
                                lines.push(quote(&format!(
                                    "+incdir+{}",
                                    relativize_path(i, sess.root)
                                )));
                            }
                        }
                        SourceType::Vhdl => {
                            lines.push(tcl_catch_prefix("vcom -2008", abort_on_error).to_owned());
                            if let Some(args) = matches.values_of("vcom-arg") {
                                lines.extend(args.map(Into::into));
                            }
                        }
                    }
                    for file in files {
                        let p = match file {
                            SourceFile::File(p) => p,
                            _ => continue,
                        };
                        lines.push(quote(&relativize_path(p, sess.root)));
                    }
                    println!("");
                    println!("{}", lines.join(" \\\n    "));
                    if abort_on_error {
                        println!("{}", tcl_catch_postfix());
                    }
                },
            );
        }
    } else {
        let mut lines = vec![];
        let mut file_lines = vec![];
        let mut inc_dirs = HashSet::new();
        let mut files = vec![];
        let mut defines: Vec<(String, Option<String>)> = vec![];
        let mut t: bool = false;
        for src in srcs {
            inc_dirs = src
                .include_dirs
                .into_iter()
                .fold(HashSet::new(), |mut acc, inc_dir| {
                    acc.insert(inc_dir);
                    acc
                });
            files.append(&mut src.files.clone());
            defines.extend(
                src.defines
                    .iter()
                    .map(|(k, &v)| (k.to_string(), v.map(String::from))),
            );
        }
        for file in files {
            let p = match file {
                SourceFile::File(p) => p,
                _ => continue,
            };
            if p.to_str().unwrap().contains(".sv")
                || p.to_str().unwrap().contains(".v")
                || p.to_str().unwrap().contains(".vp")
            {
                t = true;
            } else if p.to_str().unwrap().contains(".vhd") || p.to_str().unwrap().contains(".vhdl")
            {
                t = false;
            }
            file_lines.push(quote(&relativize_path(p, sess.root)));
        }
        if t {
            lines.push(tcl_catch_prefix("vlog -sv", abort_on_error).to_owned());
            if let Some(args) = matches.values_of("vlog-arg") {
                lines.extend(args.map(Into::into));
            }
            defines.extend(
                targets
                    .iter()
                    .map(|t| (format!("TARGET_{}", t.to_uppercase()), None)),
            );
            add_defines_from_matches(&mut defines, matches);
            defines.sort();
            for (k, v) in defines {
                let mut s = format!("+define+{}", k.to_uppercase());
                if let Some(v) = v {
                    s.push('=');
                    s.push_str(&v);
                }
                lines.push(s);
            }

            for i in &inc_dirs {
                lines.push(quote(&format!("+incdir+{}", relativize_path(i, sess.root))));
            }
        } else {
            lines.push(tcl_catch_prefix("vcom -2008", abort_on_error).to_owned());
            if let Some(args) = matches.values_of("vcom-arg") {
                lines.extend(args.map(Into::into));
            }
        }
        lines.extend(file_lines.iter().cloned());
        println!("{}", lines.join("\\\n"));
        if abort_on_error {
            println!("{}", tcl_catch_postfix());
        }
    }
    Ok(())
}
