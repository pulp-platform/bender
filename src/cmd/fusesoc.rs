// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `fusesoc` subcommand.

use crate::src::{SourceFile, SourceGroup};
use crate::target::TargetSet;
use crate::target::TargetSpec;
use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use itertools::Itertools;
use std::collections::HashMap;
use walkdir::{DirEntry, WalkDir};

use std::ffi::OsStr;
use std::fs::read_to_string;

use std::path::PathBuf;

use tokio::runtime::Runtime;

use std::fs;

use crate::error::*;
use crate::sess::{Session, SessionIo};

/// Assemble the `fusesoc` subcommand.
pub fn new() -> Command {
    Command::new("fusesoc")
        .about("Creates a FuseSoC `.core` file for all dependencies where none is present.")
        .arg(
            Arg::new("license")
                .long("license")
                .help(
                    "Additional commented info (e.g. License) to add to the top of the YAML file.",
                )
                .num_args(1)
                .action(ArgAction::Append)
                .value_parser(value_parser!(String)),
        )
}

/// Execute the `fusesoc` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let bender_generate_flag = "Created by bender from the available manifest file.";
    let lic_string = matches.get_many::<String>("license").unwrap_or_default();

    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let srcs = rt.block_on(io.sources())?;

    let dep_pkgs = sess.packages();
    let mut pkg_manifest_paths = dep_pkgs
        .iter()
        .flat_map(|pkgs| {
            pkgs.iter().map(|&id| {
                (
                    sess.dependency_name(id).to_string(),
                    io.get_package_path(id),
                )
            })
        })
        .collect::<HashMap<String, _>>();

    pkg_manifest_paths.insert(sess.manifest.package.name.clone(), sess.root.to_path_buf());

    let present_core_files = &pkg_manifest_paths
        .iter()
        .map(|(pkg, dir)| {
            let paths = fs::read_dir(dir)
                .map_err(|err| {
                    Error::chain(format!("Unable to read package directory {:?}", dir), err)
                })?
                .filter(|path| {
                    path.as_ref().unwrap().path().extension() == Some(OsStr::new("core"))
                })
                .map(|path| path.unwrap().path())
                .collect::<Vec<_>>();
            Ok((pkg.to_string(), paths))
        })
        .collect::<Result<HashMap<String, _>>>()?;

    // List of files to generate
    let mut generate_files: HashMap<String, _> = HashMap::new();

    // FuseSoC `name` and `depend` strings
    let mut fuse_depend_string: HashMap<String, String> = HashMap::new();

    // Determine `.core` file names and locations
    for pkg in present_core_files.keys() {
        if present_core_files[pkg].is_empty() {
            generate_files.insert(
                pkg.to_string(),
                pkg_manifest_paths[pkg]
                    .clone()
                    .join(format!("{}.core", pkg)),
            );
            let src_packages = &srcs
                .filter_packages(&vec![pkg.to_string()].into_iter().collect())
                .unwrap_or(SourceGroup {
                    package: Default::default(),
                    independent: true,
                    target: TargetSpec::Wildcard,
                    include_dirs: Default::default(),
                    export_incdirs: Default::default(),
                    defines: Default::default(),
                    files: Default::default(),
                    dependencies: Default::default(),
                    version: None,
                })
                .flatten();

            fuse_depend_string.insert(
                pkg.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    "",
                    "",
                    pkg,
                    match &src_packages.clone()[0].version {
                        Some(version) => format!("{}", version),
                        None => "".to_string(),
                    }
                ),
            );
        } else {
            if present_core_files[pkg].len() > 1 {
                unimplemented!("Multiple core files present!");
            }
            let file_str = read_to_string(&present_core_files[pkg][0]).map_err(|cause| {
                Error::chain(
                    format!("Cannot open .core file {:?}.", &present_core_files[pkg][0]),
                    cause,
                )
            })?;

            if file_str.contains(bender_generate_flag) {
                generate_files.insert(pkg.to_string(), present_core_files[pkg][0].clone());
                let src_packages = &srcs
                    .filter_packages(&vec![pkg.to_string()].into_iter().collect())
                    .unwrap_or(SourceGroup {
                        package: Default::default(),
                        independent: true,
                        target: TargetSpec::Wildcard,
                        include_dirs: Default::default(),
                        export_incdirs: Default::default(),
                        defines: Default::default(),
                        files: Default::default(),
                        dependencies: Default::default(),
                        version: None,
                    })
                    .flatten();

                fuse_depend_string.insert(
                    pkg.to_string(),
                    format!(
                        "{}:{}:{}:{}",
                        "",
                        "",
                        pkg,
                        match &src_packages.clone()[0].version {
                            Some(version) => format!("{}", version),
                            None => "".to_string(),
                        }
                    ),
                );
            } else {
                let fuse_core: FuseSoCCAPI2 = serde_yaml::from_str(&file_str).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Unable to parse core file {:?}.",
                            &present_core_files[pkg][0]
                        ),
                        cause,
                    )
                })?;
                fuse_depend_string.insert(pkg.to_string(), fuse_core.name.clone());
            }
        }
    }

    // Generate new `.core` files
    for pkg in generate_files.keys() {
        let src_packages = &srcs
            .filter_packages(&vec![pkg.to_string()].into_iter().collect())
            .unwrap_or(SourceGroup {
                package: Default::default(),
                independent: true,
                target: TargetSpec::Wildcard,
                include_dirs: Default::default(),
                export_incdirs: Default::default(),
                defines: Default::default(),
                files: Default::default(),
                dependencies: Default::default(),
                version: None,
            })
            .flatten();

        let mut fuse_str = "CAPI=2:\n".to_string();
        fuse_str.push_str(&format!("# {}\n\n", bender_generate_flag));

        for line in lic_string.clone() {
            fuse_str.push_str("# ");
            fuse_str.push_str(line);
            fuse_str.push('\n');
        }

        let fuse_pkg = FuseSoCCAPI2 {
            name: fuse_depend_string[&pkg.to_string()].clone(),
            description: None,
            filesets: {
                src_packages
                    .iter()
                    .map(|file_pkg| {
                        (
                            get_fileset_name(&file_pkg.target, true),
                            FuseSoCFileSet {
                                file_type: Some("systemVerilogSource".to_string()),
                                // logical_name: None,
                                files: {
                                    get_fileset_files(file_pkg, pkg_manifest_paths[pkg].clone())
                                        .into_iter()
                                        .chain(file_pkg.include_dirs.iter().flat_map(|incdir| {
                                            get_include_files(
                                                &incdir.to_path_buf(),
                                                pkg_manifest_paths[pkg].clone(),
                                            )
                                        }))
                                        .collect()
                                },
                                depend: file_pkg
                                    .dependencies
                                    .iter()
                                    .map(|dep| fuse_depend_string[dep].clone())
                                    .collect(),
                            },
                        )
                    })
                    .chain(
                        vec![(
                            "files_rtl".to_string(),
                            FuseSoCFileSet {
                                file_type: Some("systemVerilogSource".to_string()),
                                // logical_name: None,
                                files: {
                                    if src_packages[0]
                                        .export_incdirs
                                        .get(pkg)
                                        .unwrap_or(&Vec::new())
                                        .is_empty()
                                    {
                                        Vec::new()
                                    } else {
                                        src_packages[0]
                                            .export_incdirs
                                            .get(pkg)
                                            .unwrap_or(&Vec::new())
                                            .iter()
                                            .flat_map(|incdir| {
                                                get_include_files(
                                                    &incdir.to_path_buf(),
                                                    pkg_manifest_paths[pkg].clone(),
                                                )
                                            })
                                            .collect()
                                    }
                                },
                                depend: src_packages[0]
                                    .dependencies
                                    .iter()
                                    .map(|dep| fuse_depend_string[dep].clone())
                                    .collect(),
                            },
                        )]
                        .into_iter(),
                    )
                    .into_group_map()
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            FuseSoCFileSet {
                                file_type: v[0].file_type.clone(),
                                // logical_name: None,
                                files: v.iter().flat_map(|e| e.files.clone()).collect(),
                                depend: v.iter().flat_map(|e| e.depend.clone()).unique().collect(),
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>()
            },
            targets: HashMap::from([
                (
                    "default".to_string(),
                    HashMap::from([(
                        "filesets".to_string(),
                        src_packages
                            .iter()
                            .filter(|pack| pack.target.matches(&TargetSet::empty()))
                            .map(|pack| get_fileset_name(&pack.target, true))
                            // .chain(vec!["files_rtl".to_string()])
                            .unique()
                            .collect(),
                    )]),
                ),
                (
                    "simulation".to_string(),
                    HashMap::from([(
                        "filesets".to_string(),
                        src_packages
                            .iter()
                            .filter(|pack| {
                                pack.target
                                    .matches(&TargetSet::new(vec!["simulation", "test"]))
                            })
                            .map(|pack| get_fileset_name(&pack.target, true))
                            // .chain(vec!["files_rtl".to_string()])
                            .unique()
                            .collect(),
                    )]),
                ),
            ]),
        };

        fuse_str.push('\n');
        fuse_str.push_str(
            &serde_yaml::to_string(&fuse_pkg)
                .map_err(|err| Error::chain("Failed to serialize.", err))?,
        );

        // println!("{}", fuse_str);
        fs::write(&generate_files[pkg], fuse_str).map_err(|cause| {
            Error::chain(format!("Unable to write corefile for {:?}.", &pkg), cause)
        })?;
    }

    Ok(())
}

fn get_fileset_name(spec: &TargetSpec, top: bool) -> String {
    let tmp_str = match spec {
        TargetSpec::Wildcard => "".to_string(),
        TargetSpec::Name(ref name) => name.to_string(),
        TargetSpec::Any(ref specs) => {
            let mut spec_str = "".to_string();
            for spec in specs.iter() {
                let mystr = get_fileset_name(spec, false);
                if !spec_str.is_empty() && !mystr.is_empty() {
                    spec_str.push_str("_or_");
                }
                spec_str.push_str(&mystr);
            }
            spec_str.to_string()
        }
        TargetSpec::All(ref specs) => {
            let mut spec_str = "".to_string();
            for spec in specs.iter() {
                let mystr = get_fileset_name(spec, false);
                if !spec_str.is_empty() && !mystr.is_empty() {
                    spec_str.push('_');
                }
                spec_str.push_str(&mystr);
            }
            spec_str.to_string()
        }
        TargetSpec::Not(ref spec) => format!("not{}", get_fileset_name(spec, false)),
    };
    if top && tmp_str == *"" {
        "files_rtl".to_string()
    } else {
        tmp_str
    }
}

fn get_fileset_files(file_pkg: &SourceGroup, root_dir: PathBuf) -> Vec<FuseFileType> {
    file_pkg
        .files
        .iter()
        .filter_map(|src_file| match src_file {
            SourceFile::File(intern_file) => Some(
                match intern_file.extension().and_then(std::ffi::OsStr::to_str) {
                    Some("vhd") | Some("vhdl") => FuseFileType::HashMap(HashMap::from([(
                        intern_file
                            .strip_prefix(root_dir.clone())
                            .unwrap()
                            .to_path_buf(),
                        FuseSoCFile {
                            is_include_file: None,
                            include_path: None,
                            file_type: Some("vhdlSource".to_string()),
                        },
                    )])),
                    Some("v") => FuseFileType::HashMap(HashMap::from([(
                        intern_file
                            .strip_prefix(root_dir.clone())
                            .unwrap()
                            .to_path_buf(),
                        FuseSoCFile {
                            is_include_file: None,
                            include_path: None,
                            file_type: Some("verilogSource".to_string()),
                        },
                    )])),
                    _ => FuseFileType::PathBuf(
                        intern_file
                            .strip_prefix(root_dir.clone())
                            .unwrap()
                            .to_path_buf(),
                    ),
                },
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
}

fn is_not_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| entry.depth() == 0 || !s.starts_with('.'))
        .unwrap_or(false)
}

fn get_include_files(dir: &PathBuf, base_path: PathBuf) -> Vec<FuseFileType> {
    let incdir_files = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(is_not_hidden)
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension() == Some(OsStr::new("svh"))
                || e.path().extension() == Some(OsStr::new("vh"))
        })
        .map(|e| e.path().to_path_buf());
    incdir_files
        .map(|incdir_file| {
            FuseFileType::HashMap(HashMap::from([(
                incdir_file
                    .strip_prefix(base_path.clone())
                    .unwrap()
                    .to_path_buf(),
                FuseSoCFile {
                    is_include_file: Some(true),
                    include_path: Some(dir.strip_prefix(base_path.clone()).unwrap().to_path_buf()),
                    file_type: None,
                },
            )]))
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FuseSoCCAPI2 {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    filesets: HashMap<String, FuseSoCFileSet>,
    targets: HashMap<String, HashMap<String, Vec<String>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum FuseFileType {
    PathBuf(PathBuf),
    HashMap(HashMap<PathBuf, FuseSoCFile>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FuseSoCFileSet {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    file_type: Option<String>,
    // #[serde(skip_serializing_if = "Option::is_none", default)]
    // logical_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    files: Vec<FuseFileType>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    depend: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FuseSoCFile {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    is_include_file: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    include_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    file_type: Option<String>,
}
