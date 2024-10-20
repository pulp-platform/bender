// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `pickle` subcommand.

// use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use indexmap::IndexSet;
use itertools::concat;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup};
use crate::target::{TargetSet, TargetSpec};

use morty::*;

/// Assemble the `pickle` subcommand.
pub fn new() -> Command {
    Command::new("pickle")
        .about("Beta: Pickle the SystemVerilog source files in the project")
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .help("Filter sources by target")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
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
            Arg::new("inc")
                .short('I')
                .value_name("DIR")
                .help("Add a search path for SystemVerilog includes")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("exclude_rename")
                .long("exclude-rename")
                .value_name("MODULE|INTERFACE|PACKAGE")
                .help("Add module, interface, package which should not be renamed")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("exclude_sv")
                .long("exclude_sv")
                .value_name("MODULE|INTERFACE|PACKAGE")
                .help("Do not include SV module, interface, package in the pickled file list")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("v")
                .short('v')
                .num_args(0)
                .action(ArgAction::Count)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("prefix")
                .short('P')
                .long("prefix")
                .value_name("PREFIX")
                .help("Prepend a name to all global names")
                .num_args(1),
        )
        .arg(
            Arg::new("def")
                .short('D')
                .value_name("DEFINE")
                .help("Define a preprocesor macro")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("suffix")
                .short('S')
                .long("suffix")
                .value_name("SUFFIX")
                .help("Append a name to all global names")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("preproc")
                .short('E')
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Write preprocessed input files to stdout"),
        )
        .arg(
            Arg::new("strip_comments")
                .long("strip-comments")
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Strip comments from the output"),
        )
        .arg(
            Arg::new("docdir")
                .long("doc")
                .value_name("OUTDIR")
                .help("Generate documentation in a directory")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .value_name("FILE")
                .help("Write output to file")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("library_file")
                .long("library-file")
                .help("File to search for SystemVerilog modules")
                .value_name("FILE")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("library_dir")
                .short('y')
                .long("library-dir")
                .help("Directory to search for SystemVerilog modules")
                .value_name("DIR")
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("top_module")
                .long("top")
                .value_name("TOP_MODULE")
                .help("Top module, strip all unneeded modules")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("graph_file")
                .long("graph_file")
                .value_name("FILE")
                .help("Output a DOT graph of the parsed modules")
                .num_args(1)
                .value_parser(value_parser!(String)),
        )
        .arg(
            Arg::new("ignore_unparseable")
                .short('i')
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Ignore files that cannot be parsed"),
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

/// Execute the `pickle` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut srcs = rt.block_on(io.sources())?;

    // Filter the sources by target.
    let targets = matches
        .get_many::<String>("target")
        .map(|t| TargetSet::new(t))
        .unwrap_or_else(|| TargetSet::empty());
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

    let srcs = srcs.flatten();

    let logger_level = matches.get_count("v");

    // Instantiate a new logger with the verbosity level the user requested.
    SimpleLogger::new()
        .with_level(match logger_level {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            3 | _ => LevelFilter::Trace,
        })
        .with_utc_timestamps()
        .init()
        .unwrap();

    // Handle user defines.
    let defines: HashMap<_, _> = match matches.get_many::<String>("def") {
        Some(args) => args
            .map(|x| {
                let mut iter = x.split('=');
                (
                    iter.next().unwrap().to_string(),
                    iter.next().map(String::from),
                )
            })
            .collect(),
        None => HashMap::new(),
    };

    // Prepare a list of include paths.
    let include_dirs: Vec<_> = matches
        .get_many::<String>("inc")
        .into_iter()
        .flatten()
        .map(|x| x.to_string())
        .collect();

    // a hashmap from 'module name' to 'path' for all libraries.
    let mut library_files = HashMap::new();
    // a list of paths for all library files
    let mut library_paths: Vec<PathBuf> = Vec::new();

    // we first accumulate all library files from the 'library_dir' and 'library_file' options into
    // a vector of paths, and then construct the library hashmap.
    for dir in matches
        .get_many::<String>("library_dir")
        .into_iter()
        .flatten()
    {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| {
            eprintln!("error accessing library directory `{}`: {}", dir, e);
            process::exit(1)
        }) {
            let dir = entry.unwrap();
            library_paths.push(dir.path());
        }
    }

    if let Some(library_names) = matches.get_many::<String>("library_file") {
        let files = library_names.map(PathBuf::from).collect();
        library_paths.push(files);
    }

    for p in &library_paths {
        // must have the library extension (.v or .sv).
        if has_libext(p) {
            if let Some(m) = lib_module(p) {
                library_files.insert(m, p.to_owned());
            }
        }
    }

    let library_bundle = LibraryBundle {
        include_dirs: include_dirs.clone(),
        defines: defines.clone(),
        files: library_files,
    };

    // fill in file list from srcs
    let mut file_list: Vec<FileBundle> = srcs.iter().map(|x| FileBundle::from(x)).collect();
    for bundle in &mut file_list {
        bundle.include_dirs.extend(include_dirs.clone());
        bundle.defines.extend(defines.clone());
    }
    println!("{:?}", file_list);

    let (mut exclude_rename, mut exclude_sv) = (HashSet::new(), HashSet::new());
    exclude_rename.extend(
        matches
            .get_many::<String>("exclude_rename")
            .into_iter()
            .flatten(),
    );
    exclude_sv.extend(
        matches
            .get_many::<String>("exclude_sv")
            .into_iter()
            .flatten(),
    );

    let strip_comments = matches.get_flag("strip_comments");

    let syntax_trees = build_syntax_tree(
        &file_list,
        strip_comments,
        matches.get_flag("ignore_unparseable"),
    )?;

    let out = match matches.get_one::<String>("output") {
        Some(file) => {
            info!("Setting output to `{}`", file);
            let path = Path::new(file);
            Box::new(BufWriter::new(File::create(&path).unwrap_or_else(|e| {
                eprintln!("could not create `{}`: {}", file, e);
                process::exit(1);
            }))) as Box<dyn Write>
        }
        None => Box::new(io::stdout()) as Box<dyn Write>,
    };

    // Just preprocess.
    if matches.get_flag("preproc") {
        just_preprocess(syntax_trees, out)?;
        return Ok(());
    }

    info!("Finished reading {} source files.", syntax_trees.len());

    // Emit documentation if requested.
    if let Some(dir) = matches.get_one::<String>("docdir") {
        info!("Generating documentation in `{}`", dir);
        build_doc(syntax_trees, dir)?;
        return Ok(());
    }

    let pickle = do_pickle(
        matches.get_one::<String>("prefix").map(|x| x.as_str()),
        matches.get_one::<String>("suffix").map(|x| x.as_str()),
        exclude_rename.iter().map(|x| x.as_str()).collect(),
        exclude_sv.iter().map(|x| x.as_str()).collect(),
        library_bundle,
        syntax_trees,
        out,
        matches.get_one::<String>("top_module").map(|x| x.as_str()),
    )?;

    if let Some(graph_file) = matches.get_one::<String>("graph_file") {
        write_dot_graph(&pickle, graph_file)?;
    }

    Ok(())
}

impl<'a> From<&SourceGroup<'a>> for FileBundle {
    fn from(group: &SourceGroup) -> FileBundle {
        FileBundle {
            include_dirs: concat(vec![
                group
                    .include_dirs
                    .iter()
                    .map(|x| x.to_str().unwrap().to_string())
                    .collect::<Vec<String>>(),
                group
                    .export_incdirs
                    .iter()
                    .map(|(_, v)| {
                        v.iter()
                            .map(|path| path.to_str().unwrap().to_string())
                            .collect::<Vec<String>>()
                    })
                    .flatten()
                    .collect::<Vec<String>>(),
            ]),
            export_incdirs: HashMap::new(),
            defines: group
                .defines
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        match v {
                            Some(x) => Some(x.to_string()),
                            None => None,
                        },
                    )
                })
                .collect(),
            files: group
                .files
                .iter()
                .map(|x| match x {
                    SourceFile::File(path) => path.to_str().unwrap().to_string(),
                    _ => "".to_string(),
                })
                .collect(),
        }
    }
}
