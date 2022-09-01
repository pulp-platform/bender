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

use clap::{Arg, ArgMatches, Command};
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
pub fn new<'a>() -> Command<'a> {
    Command::new("pickle")
        .about("Beta: Pickle the SystemVerilog source files in the project")
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .help("Filter sources by target")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("package")
                .short('p')
                .long("package")
                .help("Specify package to show sources for")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("no_deps")
                .short('n')
                .long("no-deps")
                .help("Exclude all dependencies, i.e. only top level or specified package(s)"),
        )
        .arg(
            Arg::new("exclude")
                .short('e')
                .long("exclude")
                .help("Specify package to exclude from sources")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("inc")
                .short('I')
                .value_name("DIR")
                .help("Add a search path for SystemVerilog includes")
                .multiple_occurrences(true)
                .takes_value(true),
        )
        .arg(
            Arg::new("exclude_rename")
                .long("exclude-rename")
                .value_name("MODULE|INTERFACE|PACKAGE")
                .help("Add module, interface, package which should not be renamed")
                .multiple_occurrences(true)
                .takes_value(true),
        )
        .arg(
            Arg::new("exclude_sv")
                .long("exclude_sv")
                .value_name("MODULE|INTERFACE|PACKAGE")
                .help("Do not include SV module, interface, package in the pickled file list")
                .multiple_occurrences(true)
                .takes_value(true),
        )
        .arg(
            Arg::new("v")
                .short('v')
                .multiple_occurrences(true)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("prefix")
                .short('P')
                .long("prefix")
                .value_name("PREFIX")
                .help("Prepend a name to all global names")
                .takes_value(true),
        )
        .arg(
            Arg::new("def")
                .short('D')
                .value_name("DEFINE")
                .help("Define a preprocesor macro")
                .multiple_occurrences(true)
                .takes_value(true),
        )
        .arg(
            Arg::new("suffix")
                .short('S')
                .long("suffix")
                .value_name("SUFFIX")
                .help("Append a name to all global names")
                .takes_value(true),
        )
        .arg(
            Arg::new("preproc")
                .short('E')
                .help("Write preprocessed input files to stdout"),
        )
        .arg(
            Arg::new("strip_comments")
                .long("strip-comments")
                .help("Strip comments from the output"),
        )
        .arg(
            Arg::new("docdir")
                .long("doc")
                .value_name("OUTDIR")
                .help("Generate documentation in a directory")
                .takes_value(true),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .value_name("FILE")
                .help("Write output to file")
                .takes_value(true),
        )
        .arg(
            Arg::new("library_file")
                .long("library-file")
                .help("File to search for SystemVerilog modules")
                .value_name("FILE")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("library_dir")
                .short('y')
                .long("library-dir")
                .help("Directory to search for SystemVerilog modules")
                .value_name("DIR")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("top_module")
                .long("top")
                .value_name("TOP_MODULE")
                .help("Top module, strip all unneeded modules")
                .takes_value(true),
        )
        .arg(
            Arg::new("graph_file")
                .long("graph_file")
                .value_name("FILE")
                .help("Output a DOT graph of the parsed modules")
                .takes_value(true),
        )
        .arg(
            Arg::new("ignore_unparseable")
                .short('i')
                .help("Ignore files that cannot be parsed"),
        )
}

fn get_package_strings<I>(packages: I) -> HashSet<String>
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
        .values_of("target")
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
        });

    // Filter the sources by specified packages.
    let packages = &srcs.get_package_list(
        sess,
        &matches
            .values_of("package")
            .map(|p| get_package_strings(p))
            .unwrap_or_else(|| HashSet::new()),
        &matches
            .values_of("exclude")
            .map(|p| get_package_strings(p))
            .unwrap_or_else(|| HashSet::new()),
        matches.is_present("no_deps"),
    );

    if matches.is_present("package")
        || matches.is_present("exclude")
        || matches.is_present("no_deps")
    {
        srcs = srcs
            .filter_packages(&packages)
            .unwrap_or_else(|| SourceGroup {
                package: Default::default(),
                independent: true,
                target: TargetSpec::Wildcard,
                include_dirs: Default::default(),
                export_incdirs: Default::default(),
                defines: Default::default(),
                files: Default::default(),
                dependencies: Default::default(),
            });
    }

    let srcs = srcs.flatten();

    let logger_level = matches.occurrences_of("v");

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
    let defines: HashMap<_, _> = match matches.values_of("def") {
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
        .values_of("inc")
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
    for dir in matches.values_of("library_dir").into_iter().flatten() {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| {
            eprintln!("error accessing library directory `{}`: {}", dir, e);
            process::exit(1)
        }) {
            let dir = entry.unwrap();
            library_paths.push(dir.path());
        }
    }

    if let Some(library_names) = matches.values_of("library_file") {
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
    exclude_rename.extend(matches.values_of("exclude_rename").into_iter().flatten());
    exclude_sv.extend(matches.values_of("exclude_sv").into_iter().flatten());

    let strip_comments = matches.is_present("strip_comments");

    let syntax_trees = build_syntax_tree(
        &file_list,
        strip_comments,
        matches.is_present("ignore_unparseable"),
    )?;

    let out = match matches.value_of("output") {
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
    if matches.is_present("preproc") {
        just_preprocess(syntax_trees, out)?;
        return Ok(());
    }

    info!("Finished reading {} source files.", syntax_trees.len());

    // Emit documentation if requested.
    if let Some(dir) = matches.value_of("docdir") {
        info!("Generating documentation in `{}`", dir);
        build_doc(syntax_trees, dir)?;
        return Ok(());
    }

    do_pickle(
        matches.value_of("prefix"),
        matches.value_of("suffix"),
        exclude_rename,
        exclude_sv,
        library_bundle,
        syntax_trees,
        out,
        matches.value_of("top_module"),
    )?;

    if let Some(graph_file) = matches.value_of("graph_file") {
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
