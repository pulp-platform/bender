// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `script` subcommand.

use clap::{App, Arg, ArgMatches, SubCommand};
use tokio_core::reactor::Core;

use error::*;
use sess::{Session, SessionIo};
use src::{SourceFile, SourceGroup};
use target::{TargetSet, TargetSpec};

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
                .possible_values(&["vsim", "synopsys"]),
        )
        .arg(
            Arg::with_name("vcom-arg")
                .long("vcom-arg")
                .help("Pass an argument to vcom calls")
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("vlog-arg")
                .long("vlog-arg")
                .help("Pass an argument to vlog calls")
                .takes_value(true)
                .multiple(true),
        )
}

/// Execute the `script` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(&sess, core.handle());
    let srcs = core.run(io.sources())?;

    // Format-specific target specifiers.
    let format_targets = match matches.value_of("format").unwrap() {
        "vsim" => &["vsim", "simulation"],
        "synopsys" => &["synopsys", "synthesis"],
        _ => unreachable!(),
    };

    // Filter the sources by target.
    let targets = matches
        .values_of("target")
        .map(|t| TargetSet::new(t.chain(format_targets.into_iter().cloned())))
        .unwrap_or_else(|| TargetSet::empty());
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

    // Generate the corresponding output.
    match matches.value_of("format").unwrap() {
        "vsim" => emit_vsim_tcl(sess, matches, targets, srcs),
        "synopsys" => emit_synopsys_tcl(sess, matches, targets, srcs),
        _ => unreachable!(),
    }
}

/// Subdivide the source files in a group.
///
/// The function `cateogrize` is used to assign a category to each source file.
/// Files with the same category that appear after each other will be kept in
/// the same source group. Files with different cateogries are split into
/// separate groups.
fn separate_files_in_group<F1, F2, T>(mut src: SourceGroup, categorize: F1, consume: F2)
where
    F1: Fn(&SourceFile) -> Option<T>,
    F2: Fn(&SourceGroup, T, Vec<SourceFile>),
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

/// Emit a vsim compilation script.
fn emit_vsim_tcl(
    sess: &Session,
    matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    println!("# This script was generated automatically by bender.");
    println!("set ROOT \"{}\"", sess.root.to_str().unwrap());
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
                        lines.push("vlog -incr -sv".to_owned());
                        if let Some(args) = matches.values_of("vlog-arg") {
                            lines.extend(args.map(Into::into));
                        }
                        for (k, v) in &src.defines {
                            let mut s = format!("+define+{}", k);
                            if let Some(v) = v {
                                s.push('=');
                                s.push_str(v);
                            }
                            lines.push(s);
                        }
                        for t in &targets {
                            lines.push(format!("+define+TARGET_{}", t));
                        }
                        for i in &src.include_dirs {
                            if i.starts_with(sess.root) {
                                lines.push(format!(
                                    "\"+incdir+$ROOT/{}\"",
                                    i.strip_prefix(sess.root).unwrap().to_str().unwrap()
                                ));
                            } else {
                                lines.push(format!("\"+incdir+{}\"", i.to_str().unwrap()));
                            }
                        }
                    }
                    SourceType::Vhdl => {
                        lines.push("vcom -2008".to_owned());
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
                    if p.starts_with(sess.root) {
                        lines.push(format!(
                            "\"$ROOT/{}\"",
                            p.strip_prefix(sess.root).unwrap().to_str().unwrap()
                        ));
                    } else {
                        lines.push(format!("\"{}\"", p.to_str().unwrap()));
                    }
                }
                println!("");
                println!("{}", lines.join(" \\\n    "));
            },
        );
    }
    Ok(())
}

/// Emit a Synopsys Design Compiler compilation script.
fn emit_synopsys_tcl(
    sess: &Session,
    _matches: &ArgMatches,
    targets: TargetSet,
    srcs: Vec<SourceGroup>,
) -> Result<()> {
    println!("# This script was generated automatically by bender.");
    println!("set search_path_initial $search_path");
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
                        lines.push("analyze -format sv".to_owned());
                    }
                    SourceType::Vhdl => {
                        lines.push("analyze -format vhdl -2008".to_owned());
                    }
                }

                // Add defines.
                if !src.defines.is_empty() || !targets.is_empty() {
                    lines.push("-define {".to_owned());
                    for (k, v) in &src.defines {
                        let mut s = format!("    {}", k);
                        if let Some(v) = v {
                            s.push('=');
                            s.push_str(v);
                        }
                        lines.push(s);
                    }
                    for t in &targets {
                        lines.push(format!("    TARGET_{}", t));
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
