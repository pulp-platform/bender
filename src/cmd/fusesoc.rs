// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `fusesoc` subcommand.

use serde_yaml_ng::Value;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::fs::read_to_string;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{ArgAction, Args};
use indexmap::{IndexMap, IndexSet};
use is_terminal::IsTerminal;
use itertools::Itertools;
use tokio::runtime::Runtime;
use walkdir::{DirEntry, WalkDir};

use crate::error::*;
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceGroup};
use crate::target::TargetSet;
use crate::target::TargetSpec;

/// Creates a FuseSoC `.core` file for all dependencies where none is present
#[derive(Args, Debug)]
pub struct FusesocArgs {
    /// Only create a `.core` file for the top package, based directly on the `Bender.yml.`
    #[arg(long)]
    pub single: bool,

    /// Additional commented info (e.g. License) to add to the top of the YAML file.
    #[arg(long, action = ArgAction::Append)]
    pub license: Vec<String>,

    /// Vendor string to add for generated `.core` files
    // alias is for backward compatibility
    #[arg(long, alias = "fuse_vendor")]
    pub fuse_vendor: Option<String>,

    /// Version string for the top package to add for generated `.core` file.
    // alias is for backward compatibility
    #[arg(long, alias = "fuse_version")]
    pub fuse_version: Option<String>,
}

/// Execute the `fusesoc --single` subcomand.
pub fn run_single(sess: &Session, args: &FusesocArgs) -> Result<()> {
    let bender_generate_flag = "Created by bender from the available manifest file.";
    let vendor_string = args.fuse_vendor.as_deref().unwrap_or("");
    let version_string = match &args.fuse_version {
        Some(version) => Some(semver::Version::parse(version).map_err(|cause| {
            Error::chain(format!("Unable to parse version {}.", version), cause)
        })?),
        None => None,
    };
    let name = &sess.manifest.package.name;

    let srcs = match &sess.manifest.sources {
        Some(sources) => Ok(sess
            .load_sources(
                sources,
                Some(name.as_str()),
                sess.manifest.dependencies.keys().cloned().collect(),
                IndexMap::new(),
                version_string.clone(),
                IndexMap::new(),
            )
            .flatten()),
        None => Err(Error::new("Error in loading sources")),
    }?;

    let core_path = &sess.root.join(format!("{}.core", name));

    let file_str = match read_to_string(core_path) {
        Ok(file_str) => file_str,
        Err(_) => bender_generate_flag.to_string(),
    };

    if !file_str.contains(bender_generate_flag) {
        Err(Error::new(format!(
            "{}.core already exists, please delete to generate.",
            name
        )))?
    }

    let fuse_depend_string = sess
        .manifest
        .dependencies
        .keys()
        .map(|dep| {
            (
                dep.to_string(),
                format!(
                    "{}:{}:{}:{}", // VLNV
                    vendor_string, // Vendor
                    "",            // Library
                    dep,           // Name
                    "",            // Version
                ),
            )
        })
        .chain([(
            name.to_string(),
            format!(
                "{}:{}:{}:{}", // VLNV
                vendor_string, // Vendor
                "",            // Library
                name,          // Name
                match &version_string {
                    Some(version) => format!("{}", version),
                    None => "".to_string(),
                }  // Version
            ),
        )])
        .collect();

    let pkg_manifest_paths = IndexMap::from([(name.to_string(), sess.root.to_path_buf())]);

    let fuse_str = get_fuse_file_str(
        name,
        &srcs,
        &fuse_depend_string,
        &pkg_manifest_paths,
        bender_generate_flag.to_string(),
        &args.license,
    )?;

    fs::write(core_path, fuse_str).map_err(|cause| {
        Error::chain(format!("Unable to write corefile for {:?}.", &name), cause)
    })?;

    if fuse_depend_string.len() > 1 && !sess.suppress_warnings.contains("W16") {
        warnln!("[W16] Depend strings may be wrong for the included dependencies!");
    }

    Ok(())
}

/// Execute the `fusesoc` subcommand.
pub fn run(sess: &Session, args: &FusesocArgs) -> Result<()> {
    let bender_generate_flag = "Created by bender from the available manifest file.";
    let vendor_string = args.fuse_vendor.as_deref().unwrap_or("");
    let version_string = match &args.fuse_version {
        Some(version) => Some(semver::Version::parse(version).map_err(|cause| {
            Error::chain(format!("Unable to parse version {}.", version), cause)
        })?),
        None => None,
    };

    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let srcs = rt.block_on(io.sources(false, &[]))?;

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
        .collect::<IndexMap<String, _>>();

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
        .collect::<Result<IndexMap<String, _>>>()?;

    // List of files to generate
    let mut generate_files: IndexMap<String, _> = IndexMap::new();

    // FuseSoC `name` and `depend` strings
    let mut fuse_depend_string: IndexMap<String, String> = IndexMap::new();
    let top = &sess.manifest.package.name;

    // Determine `.core` file names and locations
    for pkg in present_core_files.keys() {
        if present_core_files[pkg].is_empty() {
            generate_files.insert(
                pkg.to_string(),
                pkg_manifest_paths[pkg]
                    .clone()
                    .join(format!("{}.core", pkg)),
            );

            fuse_depend_string.insert(
                pkg.to_string(),
                get_fuse_depend_string(
                    pkg,
                    &srcs,
                    vendor_string.to_string(),
                    top,
                    version_string.clone(),
                ),
            );
        } else {
            let mut index = 0;
            if present_core_files[pkg].len() > 1 {
                let mut msg = format!("Multiple `.core` files already present for {}.\n", pkg);
                writeln!(
                    msg,
                    "Please pick a `.core` file to use for this dependency.\n"
                )
                .unwrap();
                for i in 0..present_core_files[pkg].len() {
                    let file_str =
                        read_to_string(&present_core_files[pkg][index]).map_err(|cause| {
                            Error::chain(
                                format!(
                                    "Cannot open .core file {:?}.",
                                    &present_core_files[pkg][index]
                                ),
                                cause,
                            )
                        })?;

                    let fuse_core = parse_fuse_file(
                        file_str,
                        present_core_files[pkg][i].display().to_string(),
                    )?;
                    writeln!(
                        msg,
                        "{}) {} : {}",
                        i,
                        present_core_files[pkg][i]
                            .as_path()
                            .strip_prefix(&pkg_manifest_paths[pkg])
                            .unwrap()
                            .display(),
                        fuse_core.name
                    )
                    .unwrap();
                }
                let _ = writeln!(std::io::stdout(), "{}", msg);
                // Let user resolve conflict if both stderr and stdin go to a TTY.
                if std::io::stderr().is_terminal() && std::io::stdin().is_terminal() {
                    index = {
                        loop {
                            eprint!("Enter a number or hit enter to abort: ");
                            io::stdout().flush().unwrap();
                            let mut buffer = String::new();
                            io::stdin().read_line(&mut buffer).unwrap();
                            if buffer.starts_with('\n') {
                                break Err(Error::new(msg));
                            }
                            let choice = match buffer.trim().parse::<usize>() {
                                Ok(u) => u,
                                Err(_) => {
                                    eprintln!("Invalid input!");
                                    continue;
                                }
                            };
                            if choice > present_core_files[pkg].len() {
                                eprintln!("Choice out of bounds!");
                                continue;
                            }
                            break Ok(choice);
                        }?
                    };
                }
            }
            let file_str = read_to_string(&present_core_files[pkg][index]).map_err(|cause| {
                Error::chain(
                    format!(
                        "Cannot open .core file {:?}.",
                        &present_core_files[pkg][index]
                    ),
                    cause,
                )
            })?;

            if file_str.contains(bender_generate_flag) {
                generate_files.insert(pkg.to_string(), present_core_files[pkg][index].clone());

                fuse_depend_string.insert(
                    pkg.to_string(),
                    get_fuse_depend_string(
                        pkg,
                        &srcs,
                        vendor_string.to_string(),
                        top,
                        version_string.clone(),
                    ),
                );
            } else {
                let fuse_core = parse_fuse_file(
                    file_str,
                    present_core_files[pkg][index].display().to_string(),
                )?;
                fuse_depend_string.insert(pkg.to_string(), fuse_core.name.clone());
            }
        }
    }

    // Generate new `.core` files
    for pkg in generate_files.keys() {
        let src_packages = &srcs
            .filter_packages(&vec![pkg.to_string()].into_iter().collect())
            .unwrap_or_default()
            .flatten();

        let fuse_str = get_fuse_file_str(
            pkg,
            src_packages,
            &fuse_depend_string,
            &pkg_manifest_paths,
            bender_generate_flag.to_string(),
            &args.license,
        )?;

        fs::write(&generate_files[pkg], fuse_str).map_err(|cause| {
            Error::chain(format!("Unable to write corefile for {:?}.", &pkg), cause)
        })?;
    }

    Ok(())
}

fn get_fuse_file_str(
    pkg: &String,
    src_packages: &[SourceGroup],
    fuse_depend_string: &IndexMap<String, String>,
    pkg_manifest_paths: &IndexMap<String, PathBuf>,
    bender_generate_flag: String,
    lic_string: &[String],
) -> Result<String> {
    let mut fuse_str = "CAPI=2:\n".to_string();
    fuse_str.push_str(&format!("# {}\n\n", bender_generate_flag));

    for line in lic_string {
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
                .chain(vec![(
                    "files_rtl".to_string(),
                    FuseSoCFileSet {
                        file_type: Some("systemVerilogSource".to_string()),
                        // logical_name: None,
                        files: {
                            if src_packages[0]
                                .export_incdirs
                                .get(pkg)
                                .unwrap_or(&IndexSet::new())
                                .is_empty()
                            {
                                Vec::new()
                            } else {
                                src_packages[0]
                                    .export_incdirs
                                    .get(pkg)
                                    .unwrap_or(&IndexSet::new())
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
                )])
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
                .collect::<IndexMap<_, _>>()
        },
        targets: IndexMap::from([
            (
                "default".to_string(),
                IndexMap::from([(
                    "filesets".to_string(),
                    StringOrVec::Vec(
                        src_packages
                            .iter()
                            .filter(|pack| pack.target.matches(&TargetSet::empty()))
                            .map(|pack| get_fileset_name(&pack.target, true))
                            // .chain(vec!["files_rtl".to_string()])
                            .unique()
                            .collect(),
                    ),
                )]),
            ),
            (
                "simulation".to_string(),
                IndexMap::from([(
                    "filesets".to_string(),
                    StringOrVec::Vec(
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
                    ),
                )]),
            ),
        ]),
    };

    fuse_str.push('\n');
    fuse_str.push_str(
        &serde_yaml_ng::to_string(&fuse_pkg)
            .map_err(|err| Error::chain("Failed to serialize.", err))?,
    );
    Ok(fuse_str)
}

fn parse_fuse_file(file_str: String, filename: String) -> Result<FuseSoCCAPI2> {
    serde_yaml_ng::from_value({
        let mut value = serde_yaml_ng::from_str::<Value>(&file_str).map_err(|cause| {
            Error::chain(
                format!("Unable to parse core file to value {:?}.", &filename),
                cause,
            )
        })?;
        value.apply_merge().map_err(|cause| {
            Error::chain(
                format!("Unable to apply merge to file {:?}.", &filename),
                cause,
            )
        })?;
        value
    })
    .map_err(|cause| Error::chain(format!("Unable to parse core file {:?}.", &filename), cause))
}

fn get_fuse_depend_string(
    pkg: &String,
    srcs: &SourceGroup,
    vendor_string: String,
    top: &String,
    version_string: Option<semver::Version>,
) -> String {
    let src_packages = srcs
        .filter_packages(&vec![pkg.to_string()].into_iter().collect())
        .unwrap_or_default()
        .flatten();

    let src_packages = if pkg == top {
        src_packages
            .iter()
            .map(|group| SourceGroup {
                package: group.package,
                independent: group.independent,
                target: group.target.clone(),
                include_dirs: group.include_dirs.clone(),
                export_incdirs: group.export_incdirs.clone(),
                defines: group.defines.clone(),
                files: group.files.clone(),
                dependencies: group.dependencies.clone(),
                version: version_string.clone(),
                passed_targets: TargetSet::empty(),
            })
            .collect()
    } else {
        src_packages.clone()
    };

    format!(
        "{}:{}:{}:{}", // VLNV
        vendor_string, // Vendor
        "",            // Library
        pkg,           // Name
        match &src_packages.clone()[0].version {
            Some(version) => format!("{}", version),
            None => "".to_string(),
        }  // Version
    )
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
            SourceFile::File(intern_file, _) => Some(
                match intern_file.extension().and_then(std::ffi::OsStr::to_str) {
                    Some("vhd") | Some("vhdl") => FuseFileType::IndexMap(IndexMap::from([(
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
                    Some("v") => FuseFileType::IndexMap(IndexMap::from([(
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
            FuseFileType::IndexMap(IndexMap::from([(
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    description: Option<String>,
    filesets: IndexMap<String, FuseSoCFileSet>,
    targets: IndexMap<String, IndexMap<String, StringOrVec>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum FuseFileType {
    PathBuf(PathBuf),
    IndexMap(IndexMap<PathBuf, FuseSoCFile>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum StringOrVec {
    Value(Value),
    Vec(Vec<String>),
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
