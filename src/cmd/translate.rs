// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `translate` subcommand.

use crate::config::Dependency;
use crate::config::Package;
use crate::config::Workspace;
use clap::builder::PossibleValue;
use clap::{Arg, ArgMatches, Command};
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use serde_yaml;
use serde_yaml::Value;
use std::fs::{canonicalize, File};
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

use crate::config::{Manifest, PartialManifest, SourceFile, Sources, Validate};
use crate::error::*;
use crate::target::TargetSpec;

/// Assemble the `translate` subcommand.
pub fn new() -> Command {
    Command::new("translate")
        .about("Translates a manifest file to a different format")
        .arg(
            Arg::new("output")
                .short('o')
                .num_args(1)
                .help("Output format")
                .value_parser([
                    PossibleValue::new("core"),
                    PossibleValue::new("bender"),
                    PossibleValue::new("flist"),
                ])
                .default_value("bender"),
        )
        .arg(
            Arg::new("file")
                .long("file")
                .help("Source file for translation")
                .num_args(1),
        )
}

/// Execute the `translate` subcommand.
pub fn run(matches: &ArgMatches) -> Result<()> {
    let input_file = canonicalize(Path::new(
        matches
            .get_one::<String>("file")
            .unwrap_or(&"Bender.yml".to_string()),
    ))
    .map_err(|cause| Error::chain("Input file not found.", cause))?;

    let manifest = match input_file.extension().unwrap().to_str().unwrap() {
        "yml" => {
            // do bender stuff
            if input_file.file_name().unwrap().to_str() == Some("Bender.yml") {
                let file = File::open(&input_file).map_err(|cause| {
                    Error::chain(format!("Cannot open manifest {:?}.", &input_file), cause)
                })?;
                let partial: PartialManifest = serde_yaml::from_reader(file).map_err(|cause| {
                    Error::chain(
                        format!("Syntax error in manifest {:?}.", &input_file),
                        cause,
                    )
                })?;
                let manifest = partial.validate().map_err(|cause| {
                    Error::chain(format!("Error in manifest {:?}.", &input_file), cause)
                })?;
                Ok(manifest)
            } else {
                Err(Error::new(
                    "YAML files that are not `Bender.yml` not yet supported.",
                ))
            }
        }
        "core" => {
            // do fusesoc stuff
            let file = File::open(&input_file).map_err(|cause| {
                Error::chain(format!("Cannot open fuseSoC {:?}.", &input_file), cause)
            })?;
            let de = serde_yaml::Deserializer::from_reader(file);
            let core = Value::deserialize(de).map_err(|cause| {
                Error::chain(format!("Cannot parse fuseSoC {:?}.", &input_file), cause)
            })?;

            let package = if let Value::String(x) = &core["name"] {
                Package {
                    name: x.split(':').collect::<Vec<_>>()[2].to_string(),
                    authors: Some(vec![x.split(':').collect::<Vec<_>>()[0].to_string()]),
                }
            } else {
                return Err(Error::new(format!(
                    "Name not a string in fuseSoC {:?}",
                    &input_file
                )));
            };

            let mut dependencies: IndexMap<String, Dependency> = IndexMap::new();

            // let mut sources = Sources::new();

            let mut export_include_dirs = Vec::new();

            // Parse all dependencies
            if let Value::Mapping(filesets) = &core["filesets"] {
                for (_fileset_name, fileset) in filesets {
                    if let Value::Sequence(depend) = &fileset["depend"] {
                        for dependency in depend {
                            let (dep_name, dep_version) = if let Value::String(x) = dependency {
                                let split = x.split(':').collect::<Vec<_>>();
                                if split.len() > 3 {
                                    (split[2].to_string(), Some(split[3].to_string()))
                                } else {
                                    (split[2].to_string(), None)
                                }
                            } else {
                                return Err(Error::new(format!(
                                    "Dependency format not parseable: {:?}.",
                                    dependency
                                )));
                            };
                            if let Some(version) = dep_version {
                                if let Ok(semver_version) = semver::VersionReq::parse(&version) {
                                    dependencies.insert(
                                        dep_name.clone(),
                                        Dependency::Version(semver_version),
                                    );
                                } else {
                                    dependencies.insert(
                                        dep_name.clone(),
                                        Dependency::Path(Path::new(&dep_name).to_path_buf()),
                                    );
                                }
                            } else {
                                dependencies.insert(
                                    dep_name.clone(),
                                    Dependency::Path(Path::new(&dep_name).to_path_buf()),
                                );
                            }
                        }
                    }
                }
            }

            let sources = if let Value::Mapping(filesets) = &core["filesets"] {
                let files = filesets
                    .iter()
                    .map(|(fileset_name, fileset)| {
                        export_include_dirs.extend(if let Value::Mapping(int_fileset) = fileset {
                            if let Value::Sequence(files) = &int_fileset["files"] {
                                files
                                    .iter()
                                    .filter_map(|file| match file {
                                        Value::Mapping(include_directive) => {
                                            if let Some(Value::Mapping(include_mapping)) =
                                                &include_directive.values().next()
                                            {
                                                if let Value::String(include_path) =
                                                    &include_mapping["include_path"]
                                                {
                                                    Some(Path::new(&include_path).to_path_buf())
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        });
                        SourceFile::Group(Box::new(Sources {
                            target: if let Value::String(target_name) = fileset_name {
                                TargetSpec::Name(target_name.to_string())
                            } else {
                                TargetSpec::Wildcard
                            },
                            include_dirs: Vec::new(),
                            defines: IndexMap::new(),
                            files: if let Value::Mapping(int_fileset) = fileset {
                                if let Value::Sequence(files) = &int_fileset["files"] {
                                    files
                                        .iter()
                                        .filter_map(|file| match file {
                                            Value::String(x) => {
                                                Some(SourceFile::File(Path::new(x).to_path_buf()))
                                            }
                                            _ => None,
                                        })
                                        .collect()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                Vec::new()
                            },
                        }))
                    })
                    .collect();
                Some(Sources {
                    target: TargetSpec::Wildcard,
                    include_dirs: Vec::new(),
                    defines: IndexMap::new(),
                    files,
                })
            } else {
                None
            };

            // println!("{:?}", core);

            Ok(Manifest {
                package,
                dependencies,
                sources,
                export_include_dirs: export_include_dirs.into_iter().unique().collect(),
                plugins: IndexMap::new(),
                frozen: false,
                workspace: Workspace {
                    checkout_dir: None,
                    package_links: IndexMap::new(),
                },
                vendor_package: Vec::new(),
            })

            // Err(Error::new("unimplemented"))
        }
        "flist" => {
            // do flist stuff
            Err(Error::new("unimplemented"))
        }
        "json" => {
            // do morty json stuff?
            Err(Error::new("unimplemented"))
        }
        _ => Err(Error::new("Unsupported file type.")),
    }?;

    match matches.get_one::<String>("output").unwrap().as_str() {
        "bender" => {
            // Catch unnecessary conversion
            if input_file.file_name().unwrap().to_str() == Some("Bender.yml") {
                return Err(Error::new("Input and output both Bender.yml files."));
            }

            let mut out =
                Box::new(BufWriter::new(File::create("Bender.yml").unwrap())) as Box<dyn Write>;
            writeln!(
                out,
                "{}",
                serde_yaml::to_string(&manifest)
                    .map_err(|cause| Error::chain("Unable to serialize", cause))?
            )?;
        }
        "core" => {
            // Catch unnecessary conversion
            if input_file.extension().unwrap().to_str() == Some("core") {
                return Err(Error::new("Input and output both .core files."));
            }
            unimplemented!()
        }
        "flist" => {
            unimplemented!();
        }
        _ => unreachable!(),
    };

    Ok(())
}

// /// A partial FuseSoC file
// pub struct PartialFuse {
//     pub capi: Option<u32>,
//     pub name: Option<String>,
//     pub description: Option<String>,
//     pub filesets: Option<HashMap<String, >>
// }

// pub struct PartialFuseFileSet {
//     pub files: Option<Vec<>>
// }
