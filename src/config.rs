// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Package manifest and configuration files.
//!
//! This module provides reading and writing of package manifests and
//! configuration files.

#![deny(missing_docs)]

use std;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs::File;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use glob::glob;
use indexmap::IndexMap;
use semver;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_yaml_ng::Value;
#[cfg(unix)]
use subst;

use crate::diagnostic::{Diagnostics, Warnings};
use crate::error::*;
use crate::target::TargetSpec;
use crate::util::*;

/// A package manifest.
///
/// This is usually called `Bender.yml` in the root directory of the package.
#[derive(Debug)]
pub struct Manifest {
    /// The package definition.
    pub package: Package,
    /// The dependencies.
    pub dependencies: IndexMap<String, Dependency>,
    /// The source files.
    pub sources: Option<Sources>,
    /// The include directories exported to dependent packages.
    pub export_include_dirs: Vec<PathBuf>,
    /// The plugin binaries.
    pub plugins: IndexMap<String, PathBuf>,
    /// Whether the dependencies of the manifest are frozen.
    pub frozen: bool,
    /// The workspace configuration.
    pub workspace: Workspace,
    /// Vendorized dependencies
    pub vendor_package: Vec<VendorPackage>,
}

impl PrefixPaths for Manifest {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(Manifest {
            package: self.package,
            dependencies: self.dependencies.prefix_paths(prefix)?,
            sources: self
                .sources
                .map_or(Ok::<Option<Sources>, Error>(None), |src| {
                    Ok(Some(src.prefix_paths(prefix)?))
                })?,
            export_include_dirs: self
                .export_include_dirs
                .into_iter()
                .map(|src| src.prefix_paths(prefix))
                .collect::<Result<_>>()?,
            plugins: self.plugins.prefix_paths(prefix)?,
            frozen: self.frozen,
            workspace: self.workspace.prefix_paths(prefix)?,
            vendor_package: self.vendor_package.prefix_paths(prefix)?,
        })
    }
}

/// A package definition.
///
/// Contains the metadata for an individual package.
#[derive(Serialize, Deserialize, Debug)]
pub struct Package {
    /// The name of the package.
    pub name: String,
    /// A list of package authors. Each author should be of the form `John Doe
    /// <john@doe.com>`.
    pub authors: Option<Vec<String>>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

/// A dependency.
///
/// The name of the dependency is given implicitly by the key in the hash map
/// that this `Dependency` is accessible through.
#[derive(Clone, Debug)]
pub enum Dependency {
    /// A dependency that can be found in one of the package repositories.
    Version {
        /// The targets for which the dependency should be considered.
        target: TargetSpec,
        /// The version requirement of the package.
        version: semver::VersionReq,
        /// Targets to pass to the dependency
        pass_targets: Vec<PassedTarget>,
    },
    /// A local path dependency. The exact version of the dependency found at
    /// the given path will be used, regardless of any actual versioning
    /// constraints.
    Path {
        /// The targets for which the dependency should be considered.
        target: TargetSpec,
        /// The path to the dependency.
        path: PathBuf,
        /// Targets to pass to the dependency
        pass_targets: Vec<PassedTarget>,
    },
    /// A git dependency specified by a revision.
    GitRevision {
        /// The targets for which the dependency should be considered.
        target: TargetSpec,
        /// The git URL to the package.
        url: String,
        /// The git revision of the package to use. Can be a commit hash, branch, or tag.
        rev: String,
        /// Targets to pass to the dependency
        pass_targets: Vec<PassedTarget>,
    },
    /// A git dependency specified by a version requirement. Works similarly to
    /// the `GitRevision`, but extracts all tags of the form `v.*` from the
    /// repository and matches the version against that.
    GitVersion {
        /// The targets for which the dependency should be considered.
        target: TargetSpec,
        /// The git URL to the package.
        url: String,
        /// The version requirement of the package.
        version: semver::VersionReq,
        /// Targets to pass to the dependency
        pass_targets: Vec<PassedTarget>,
    },
}

impl PrefixPaths for Dependency {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(match self {
            Dependency::Path {
                target,
                path,
                pass_targets,
            } => Dependency::Path {
                target,
                path: path.prefix_paths(prefix)?,
                pass_targets,
            },
            v => v,
        })
    }
}

impl Serialize for Dependency {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        match *self {
            Dependency::Version {
                ref target,
                ref version,
                ref pass_targets,
            } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("target", target)?;
                map.serialize_entry("version", &format!("{}", version))?;
                map.serialize_entry("pass_targets", pass_targets)?;
                map.end()
            }
            // format!("{}, {:?}", version, pass_targets).serialize(serializer),
            Dependency::Path {
                ref target,
                ref path,
                ref pass_targets,
            } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("target", target)?;
                map.serialize_entry("path", path)?;
                map.serialize_entry("pass_targets", pass_targets)?;
                map.end()
            }

            // path.serialize(serializer),
            Dependency::GitRevision {
                ref target,
                ref url,
                ref rev,
                ref pass_targets,
            } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("target", target)?;
                map.serialize_entry("git", url)?;
                map.serialize_entry("rev", rev)?;
                map.serialize_entry("pass_targets", pass_targets)?;
                map.end()
            }
            Dependency::GitVersion {
                ref target,
                ref url,
                ref version,
                ref pass_targets,
            } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("target", target)?;
                map.serialize_entry("git", url)?;
                map.serialize_entry("version", &format!("{}", version))?;
                map.serialize_entry("pass_targets", pass_targets)?;
                map.end()
            }
        }
    }
}

/// A group of source files.
#[derive(Debug)]
pub struct Sources {
    /// The targets for which the sources should be considered.
    pub target: TargetSpec,
    /// The directories to search for include files.
    pub include_dirs: Vec<PathBuf>,
    /// The preprocessor definitions.
    pub defines: IndexMap<String, Option<String>>,
    /// The source files.
    pub files: Vec<SourceFile>,
}

impl PrefixPaths for Sources {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(Sources {
            target: self.target,
            include_dirs: self.include_dirs.prefix_paths(prefix)?,
            defines: self.defines,
            files: self.files.prefix_paths(prefix)?,
        })
    }
}

/// A source file.
pub enum SourceFile {
    /// A file.
    File(PathBuf),
    /// A subgroup.
    Group(Box<Sources>),
    /// A systemverilog source.
    SvFile(PathBuf),
    /// A verilog source.
    VerilogFile(PathBuf),
    /// A vhdl source.
    VhdlFile(PathBuf),
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            SourceFile::File(ref path) => {
                fmt::Debug::fmt(path, f)?;
                write!(f, " as <unknown>")
            }
            SourceFile::SvFile(ref path) => {
                fmt::Debug::fmt(path, f)?;
                write!(f, " as SystemVerilog")
            }
            SourceFile::VerilogFile(ref path) => {
                fmt::Debug::fmt(path, f)?;
                write!(f, " as Verilog")
            }
            SourceFile::VhdlFile(ref path) => {
                fmt::Debug::fmt(path, f)?;
                write!(f, " as Vhdl")
            }
            SourceFile::Group(ref srcs) => fmt::Debug::fmt(srcs, f),
        }
    }
}

impl PrefixPaths for SourceFile {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(match self {
            SourceFile::File(path) => SourceFile::File(path.prefix_paths(prefix)?),
            SourceFile::SvFile(path) => SourceFile::SvFile(path.prefix_paths(prefix)?),
            SourceFile::VerilogFile(path) => SourceFile::VerilogFile(path.prefix_paths(prefix)?),
            SourceFile::VhdlFile(path) => SourceFile::VhdlFile(path.prefix_paths(prefix)?),
            SourceFile::Group(group) => SourceFile::Group(Box::new(group.prefix_paths(prefix)?)),
        })
    }
}

/// A workspace configuration.
#[derive(Debug, Default)]
pub struct Workspace {
    /// The directory which will contain working copies of the dependencies.
    pub checkout_dir: Option<PathBuf>,
    /// The locally linked packages.
    pub package_links: IndexMap<PathBuf, String>,
}

impl PrefixPaths for Workspace {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(Workspace {
            checkout_dir: self.checkout_dir.prefix_paths(prefix)?,
            package_links: self
                .package_links
                .into_iter()
                .map(|(k, v)| Ok((k.prefix_paths(prefix)?, v)))
                .collect::<Result<_>>()?,
        })
    }
}

/// Converts partial configuration into a validated full configuration.
pub trait Validate {
    /// The output type produced by validation.
    type Output;
    /// The error type produced by validation.
    type Error;
    /// Validate self and convert into the non-partial version.
    fn validate(
        self,
        package_name: &str,
        pre_output: bool,
    ) -> std::result::Result<Self::Output, Self::Error>;
}

// Implement `Validate` for hash maps of validatable values.
impl<K, V> Validate for IndexMap<K, V>
where
    K: Hash + Eq,
    V: Validate<Error = Error>,
{
    type Output = IndexMap<K, V::Output>;
    type Error = (K, Error);
    fn validate(
        self,
        package_name: &str,
        pre_output: bool,
    ) -> std::result::Result<Self::Output, Self::Error> {
        self.into_iter()
            .map(|(k, v)| match v.validate(package_name, pre_output) {
                Ok(v) => Ok((k, v)),
                Err(e) => Err((k, e)),
            })
            .collect()
    }
}

impl<V> Validate for Vec<V>
where
    V: Validate<Error = Error>,
{
    type Output = Vec<V::Output>;
    type Error = Error;
    fn validate(
        self,
        package_name: &str,
        pre_output: bool,
    ) -> std::result::Result<Self::Output, Self::Error> {
        self.into_iter()
            .map(|v| match v.validate(package_name, pre_output) {
                Ok(v) => Ok(v),
                Err(e) => Err(e),
            })
            .collect()
    }
}

// Implement `Validate` for `StringOrStruct` wrapped validatable values.
impl<T> Validate for StringOrStruct<T>
where
    T: Validate,
{
    type Output = T::Output;
    type Error = T::Error;
    fn validate(
        self,
        package_name: &str,
        pre_output: bool,
    ) -> std::result::Result<T::Output, T::Error> {
        self.0.validate(package_name, pre_output)
    }
}

// Implement `Validate` for `SeqOrStruct` wrapped validatable values.
impl<T, F> Validate for SeqOrStruct<T, F>
where
    T: Validate,
{
    type Output = T::Output;
    type Error = T::Error;
    fn validate(
        self,
        package_name: &str,
        pre_output: bool,
    ) -> std::result::Result<T::Output, T::Error> {
        self.0.validate(package_name, pre_output)
    }
}

// Implement `PrefixPaths` for `StringOrStruct` wrapped prefixable values.
impl<T> PrefixPaths for StringOrStruct<T>
where
    T: PrefixPaths,
{
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(StringOrStruct(self.0.prefix_paths(prefix)?))
    }
}

// Implement `Validate` for `SeqOrStruct` wrapped prefixable values.
impl<T, F> PrefixPaths for SeqOrStruct<T, F>
where
    T: PrefixPaths,
{
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(SeqOrStruct::new(self.0.prefix_paths(prefix)?))
    }
}

/// A partial manifest.
///
/// Validation turns this into a `Manifest`.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialManifest {
    /// The package definition.
    pub package: Option<Package>,
    /// The dependencies.
    pub dependencies: Option<IndexMap<String, StringOrStruct<PartialDependency>>>,
    /// The source files.
    pub sources: Option<SeqOrStruct<PartialSources, PartialSourceFile>>,
    /// The include directories exported to dependent packages.
    pub export_include_dirs: Option<Vec<String>>,
    /// The plugin binaries.
    pub plugins: Option<IndexMap<String, String>>,
    /// Whether the dependencies of the manifest are frozen.
    pub frozen: Option<bool>,
    /// The workspace configuration.
    pub workspace: Option<PartialWorkspace>,
    /// External Import dependencies
    pub vendor_package: Option<Vec<PartialVendorPackage>>,
    /// Unknown extra fields
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl PartialManifest {
    /// Before fully cloning a git repo locally a manifest is read without the source files present. Since validate by default now
    /// checks to make sure source files actually exist this will error out when verifying a dependency manifest. To get around this,
    /// the initial check of a dependency manifest that is done before cloning the dependency will ignore whether source files exist
    pub fn validate_ignore_sources(
        mut self,
        package_name: &str,
        pre_output: bool,
    ) -> Result<Manifest> {
        self.sources = Some(SeqOrStruct::new(PartialSources::new_empty()));
        self.validate(package_name, pre_output)
    }
}

impl PrefixPaths for PartialManifest {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(PartialManifest {
            package: self.package,
            dependencies: self.dependencies.prefix_paths(prefix)?,
            sources: self.sources.map_or(
                Ok::<Option<SeqOrStruct<PartialSources, PartialSourceFile>>, Error>(None),
                |src| Ok(Some(src.prefix_paths(prefix)?)),
            )?,
            export_include_dirs: match self.export_include_dirs {
                Some(vec_inc) => Some(
                    vec_inc
                        .into_iter()
                        .map(|src| src.prefix_paths(prefix))
                        .collect::<Result<_>>()?,
                ),
                None => None,
            },
            plugins: self.plugins.prefix_paths(prefix)?,
            frozen: self.frozen,
            workspace: self.workspace.prefix_paths(prefix)?,
            vendor_package: self.vendor_package.prefix_paths(prefix)?,
            extra: self.extra,
        })
    }
}

impl Validate for PartialManifest {
    type Output = Manifest;
    type Error = Error;
    fn validate(self, _package_name: &str, pre_output: bool) -> Result<Manifest> {
        let pkg = match self.package {
            Some(mut p) => {
                p.name = p.name.to_lowercase();
                if !pre_output {
                    p.extra.iter().for_each(|(k, _)| {
                        Warnings::IgnoreUnknownField {
                            field: k.clone(),
                            pkg: p.name.clone(),
                        }
                        .emit();
                    });
                }
                p
            }
            None => return Err(Error::new("Missing package information.")),
        };
        let deps = match self.dependencies {
            Some(d) => d
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect::<IndexMap<_, _>>()
                .validate(&pkg.name, pre_output)
                .map_err(|(key, cause)| {
                    Error::chain(
                        format!("In dependency `{}` of package `{}`:", key, pkg.name),
                        cause,
                    )
                })?,
            None => IndexMap::new(),
        };
        let srcs = match self.sources {
            Some(s) => Some(s.validate(&pkg.name, pre_output).map_err(|cause| {
                Error::chain(format!("In source list of package `{}`:", pkg.name), cause)
            })?),
            None => None,
        };
        let exp_inc_dirs = self.export_include_dirs.unwrap_or_default();
        let plugins = match self.plugins {
            Some(s) => s
                .iter()
                .map(|(k, v)| Ok((k.clone(), env_path_from_string(v.to_string())?)))
                .collect::<Result<IndexMap<_, _>>>()?,
            None => IndexMap::new(),
        };
        let frozen = self.frozen.unwrap_or(false);
        let workspace = match self.workspace {
            Some(w) => w
                .validate(&pkg.name, pre_output)
                .map_err(|cause| Error::chain("In workspace configuration:", cause))?,
            None => Workspace::default(),
        };
        let vendor_package = match self.vendor_package {
            Some(vend) => vend
                .validate(&pkg.name, pre_output)
                .map_err(|cause| Error::chain("Unable to parse vendor_package", cause))?,
            None => Vec::new(),
        };
        if !pre_output {
            self.extra.iter().for_each(|(k, _)| {
                Warnings::IgnoreUnknownField {
                    field: k.clone(),
                    pkg: pkg.name.clone(),
                }
                .emit();
            });
        }
        Ok(Manifest {
            package: pkg,
            dependencies: deps,
            sources: match srcs {
                Some(SourceFile::Group(srcs)) => Some(*srcs),
                Some(SourceFile::File(_))
                | Some(SourceFile::SvFile(_))
                | Some(SourceFile::VerilogFile(_))
                | Some(SourceFile::VhdlFile(_)) => Some(Sources {
                    target: TargetSpec::Wildcard,
                    include_dirs: Vec::new(),
                    defines: IndexMap::new(),
                    files: vec![srcs.unwrap()],
                }),
                None => None,
            },
            export_include_dirs: exp_inc_dirs
                .iter()
                .filter_map(|path| match env_path_from_string(path.to_string()) {
                    Ok(parsed_path) => {
                        if !(pre_output || parsed_path.exists() && parsed_path.is_dir()) {
                            Warnings::IncludeDirMissing(parsed_path.clone()).emit();
                        }

                        Some(Ok(parsed_path))
                    }
                    Err(cause) => {
                        if Diagnostics::is_suppressed("E30") {
                            Warnings::IgnoredPath {
                                cause: cause.to_string(),
                            }
                            .emit();
                            None
                        } else {
                            Some(Err(Error::chain("[E30]", cause)))
                        }
                    }
                })
                .collect::<Result<Vec<_>>>()?,
            plugins,
            frozen,
            workspace,
            vendor_package,
        })
    }
}

/// A partial dependency.
///
/// Contains all the necessary information to resolve and find a dependency.
/// The following combinations of fields are valid:
///
/// - `version`
/// - `path`
/// - `git,rev`
/// - `git,version`
///
/// Can be validated into a `Dependency`.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialDependency {
    /// The targets for which the dependency should be considered.
    target: Option<TargetSpec>,
    /// The path to the package.
    path: Option<String>,
    /// The git URL to the package.
    git: Option<String>,
    /// The git revision of the package to use. Can be a commit hash, branch,
    /// tag, or similar.
    rev: Option<String>,
    /// The version requirement of the package. This will be parsed into a
    /// semantic versioning requirement.
    version: Option<String>,
    /// Targets to pass to the dependency
    pass_targets: Option<Vec<StringOrStruct<PartialPassedTarget>>>,
    /// Unknown extra fields
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl FromStr for PartialDependency {
    type Err = Void;
    fn from_str(s: &str) -> std::result::Result<Self, Void> {
        Ok(PartialDependency {
            target: None,
            path: None,
            git: None,
            rev: None,
            version: Some(s.into()),
            pass_targets: None,
            extra: HashMap::new(),
        })
    }
}

impl PrefixPaths for PartialDependency {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(PartialDependency {
            path: self.path.prefix_paths(prefix)?,
            ..self
        })
    }
}

impl Validate for PartialDependency {
    type Output = Dependency;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<Dependency> {
        let target = self.target.unwrap_or(TargetSpec::Wildcard);
        let pass_targets = self
            .pass_targets
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.validate(package_name, pre_output))
            .collect::<Result<Vec<_>>>()?;
        let version = match self.version {
            Some(v) => Some(semver::VersionReq::parse(&v).map_err(|cause| {
                Error::chain(
                    format!("\"{}\" is not a valid semantic version requirement.", v),
                    cause,
                )
            })?),
            None => None,
        };
        if self.rev.is_some() && version.is_some() {
            return Err(Error::new(
                "A dependency cannot specify `version` and `rev` at the same time.",
            ));
        }
        if !pre_output {
            self.extra.iter().for_each(|(k, _)| {
                Warnings::IgnoreUnknownField {
                    field: k.clone(),
                    pkg: package_name.to_string(),
                }
                .emit();
            });
        }
        if let Some(path) = self.path {
            if let Some(list) = string_list(
                self.git
                    .map(|_| "`git`")
                    .iter()
                    .chain(self.rev.map(|_| "`rev`").iter())
                    .chain(version.map(|_| "`version`").iter()),
                ",",
                "or",
            ) {
                Err(Error::new(format!(
                    "A `path` dependency cannot have a {} field.",
                    list
                )))
            } else {
                Ok(Dependency::Path {
                    target,
                    path: env_path_from_string(path)?,
                    pass_targets,
                })
            }
        } else if let Some(git) = self.git {
            if let Some(rev) = self.rev {
                Ok(Dependency::GitRevision {
                    target,
                    url: git,
                    rev,
                    pass_targets,
                })
            } else if let Some(version) = version {
                Ok(Dependency::GitVersion {
                    target,
                    url: git,
                    version,
                    pass_targets,
                })
            } else {
                Err(Error::new(
                    "A `git` dependency must have either a `rev` or `version` field.",
                ))
            }
        } else if let Some(version) = version {
            Ok(Dependency::Version {
                target,
                version,
                pass_targets,
            })
        } else {
            Err(Error::new(
                "A dependency must specify `version`, `path`, or `git`.",
            ))
        }
    }
}

/// A partial group of source files.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialSources {
    /// The targets for which the sources should be considered.
    pub target: Option<TargetSpec>,
    /// The directories to search for include files.
    pub include_dirs: Option<Vec<String>>,
    /// The preprocessor definitions.
    pub defines: Option<IndexMap<String, Option<String>>>,
    /// The source file paths.
    pub files: Option<Vec<PartialSourceFile>>,
    /// A sv file.
    pub sv: Option<String>,
    /// A verilog file.
    pub v: Option<String>,
    /// A vhdl file.
    pub vhd: Option<String>,
    /// The list of external flists to include.
    pub external_flists: Option<Vec<String>>,
    /// Unknown extra fields
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl PartialSources {
    /// Create new empty PartialSources struct so partial sources can be emptied out
    /// when calling validate on db of git repo since source files won't actually be present
    pub fn new_empty() -> Self {
        PartialSources {
            target: None,
            include_dirs: None,
            defines: None,
            files: Some(Vec::new()),
            sv: None,
            v: None,
            vhd: None,
            external_flists: None,
            extra: HashMap::new(),
        }
    }
}

impl PrefixPaths for PartialSources {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(PartialSources {
            target: self.target,
            include_dirs: self.include_dirs.prefix_paths(prefix)?,
            defines: self.defines,
            files: self.files.prefix_paths(prefix)?,
            sv: self.sv.prefix_paths(prefix)?,
            v: self.v.prefix_paths(prefix)?,
            vhd: self.vhd.prefix_paths(prefix)?,
            external_flists: self.external_flists.prefix_paths(prefix)?,
            extra: self.extra,
        })
    }
}

impl From<Vec<PartialSourceFile>> for PartialSources {
    fn from(v: Vec<PartialSourceFile>) -> Self {
        PartialSources {
            target: None,
            include_dirs: None,
            defines: None,
            files: Some(v),
            sv: None,
            v: None,
            vhd: None,
            external_flists: None,
            extra: HashMap::new(),
        }
    }
}

impl Validate for PartialSources {
    type Output = SourceFile;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<SourceFile> {
        match self {
            PartialSources {
                target: None,
                include_dirs: None,
                defines: None,
                files: None,
                sv: Some(sv),
                v: None,
                vhd: None,
                external_flists: None,
                extra: _,
            } => PartialSourceFile::SvFile(sv).validate(package_name, pre_output),
            PartialSources {
                target: None,
                include_dirs: None,
                defines: None,
                files: None,
                sv: None,
                v: Some(v),
                vhd: None,
                external_flists: None,
                extra: _,
            } => PartialSourceFile::VerilogFile(v).validate(package_name, pre_output),
            PartialSources {
                target: None,
                include_dirs: None,
                defines: None,
                files: None,
                sv: None,
                v: None,
                vhd: Some(vhd),
                external_flists: None,
                extra: _,
            } => PartialSourceFile::VhdlFile(vhd).validate(package_name, pre_output),
            PartialSources {
                target,
                include_dirs,
                defines,
                files,
                sv: None,
                v: None,
                vhd: None,
                external_flists,
                extra,
            } => {
                let external_flists: Result<Vec<_>> = external_flists
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|path| match env_path_from_string(path.to_string()) {
                        Ok(p) => Some(Ok(p)),
                        Err(cause) => {
                            if Diagnostics::is_suppressed("E30") {
                                Warnings::IgnoredPath {cause: cause.to_string()}.emit();
                                None
                            } else {
                                Some(Err(Error::chain("[E30]", cause)))
                            }
                        }
                    })
                    .collect();

                let external_flist_list: Result<Vec<(PathBuf, Vec<String>)>> = external_flists?
                    .into_iter()
                    .map(|filename| {
                        let file = File::open(&filename).map_err(|cause| {
                            Error::chain(
                                format!("Unable to open external flist file {:?}", filename),
                                cause,
                            )
                        })?;
                        let reader = BufReader::new(file);
                        let lines: Vec<String> = reader
                            .lines()
                            .map(|line| {
                                line.map_err(|cause| {
                                    Error::chain(
                                        format!("Error reading external flist file {:?}", filename),
                                        cause,
                                    )
                                })
                            })
                            .collect::<Result<Vec<String>>>()?;
                        let lines = lines
                            .iter()
                            .filter_map(|line| {
                                let line = line.trim();
                                if line.is_empty()
                                    || line.starts_with('#')
                                    || line.starts_with("//")
                                {
                                    None
                                } else {
                                    Some(
                                        line.split_whitespace()
                                            .map(|s| s.to_string())
                                            .collect::<Vec<_>>(),
                                    )
                                }
                            })
                            .flatten()
                            .collect();
                        Ok((filename.parent().unwrap().to_path_buf(), lines))
                    })
                    .collect();

                let external_flist_groups: Result<Vec<PartialSourceFile>> = external_flist_list?
                    .into_iter()
                    .map(|(flist_dir, flist)| {
                        Ok(PartialSourceFile::Group(Box::new(PartialSources {
                            target: None,
                            include_dirs: Some(
                                flist
                                    .clone()
                                    .into_iter()
                                    .filter_map(|file| {
                                        if file.starts_with("+incdir+") {
                                            Some(file.trim_start_matches("+incdir+").to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .flat_map(|s| {
                                        s.split('+').map(|s| s.to_string()).collect::<Vec<_>>()
                                    })
                                    .map(|dir| dir.prefix_paths(&flist_dir))
                                    .collect::<Result<_>>()?,
                            ),
                            defines: Some(
                                flist
                                    .clone()
                                    .into_iter()
                                    .filter_map(|file| {
                                        if file.starts_with("+define+") {
                                            Some(file.trim_start_matches("+define+").to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .flat_map(|s| {
                                        s.split('+').map(|s| s.to_string()).collect::<Vec<_>>()
                                    })
                                    .map(|file| {
                                        if let Some(eq_idx) = file.find("=") {
                                            (
                                                file[..eq_idx].to_string(),
                                                Some(file[eq_idx + 1..].to_string()),
                                            )
                                        } else {
                                            (file.to_string(), None)
                                        }
                                    })
                                    .collect(),
                            ),
                            files: Some(flist
                                .into_iter()
                                .filter_map(|file| {
                                    if file.starts_with("+") {
                                        None
                                    } else {
                                        // prefix path
                                        Some(PartialSourceFile::File(file))
                                    }
                                })
                                .map(|file| file.prefix_paths(&flist_dir))
                                .collect::<Result<Vec<_>>>()?),
                            sv: None,
                            v: None,
                            vhd: None,
                            external_flists: None,
                            extra: HashMap::new(),
                        })))
                    })
                    .collect();

                let post_env_files: Vec<PartialSourceFile> = if let Some(fls) = files {
                    fls
                    .into_iter()
                    .chain(external_flist_groups?.into_iter())
                    .filter_map(|file| match file {
                        PartialSourceFile::File(ref filename)
                        | PartialSourceFile::SvFile(ref filename)
                        | PartialSourceFile::VerilogFile(ref filename)
                        | PartialSourceFile::VhdlFile(ref filename) => match env_string_from_string(filename.to_string()) {
                            Ok(p) => match file {
                                PartialSourceFile::File(_) => Some(Ok(PartialSourceFile::File(p))),
                                PartialSourceFile::SvFile(_) => Some(Ok(PartialSourceFile::SvFile(p))),
                                PartialSourceFile::VerilogFile(_) => Some(Ok(PartialSourceFile::VerilogFile(p))),
                                PartialSourceFile::VhdlFile(_) => Some(Ok(PartialSourceFile::VhdlFile(p))),
                                _ => unreachable!(),
                            },
                            Err(cause) => {
                                if Diagnostics::is_suppressed("E30") {
                                    Warnings::IgnoredPath {cause: cause.to_string()}.emit();
                                    None
                                } else {
                                    Some(Err(Error::chain("[E30]", cause)))
                                }
                            }
                        },
                        other => Some(Ok(other)),
                    })
                    .collect::<Result<Vec<_>>>()?
                } else {
                    Vec::new()
                };
                let post_glob_files: Vec<PartialSourceFile> = post_env_files
                    .into_iter()
                    .map(|pre_glob_file| {
                        match pre_glob_file {
                            PartialSourceFile::File(_)
                            | PartialSourceFile::SvFile(_)
                            | PartialSourceFile::VerilogFile(_)
                            | PartialSourceFile::VhdlFile(_) => {
                                // PartialSources .files item is pointing to PartialSourceFiles::file so do glob extension
                                pre_glob_file.glob_file()
                            }
                            _ => {
                                // PartialSources .files item is pointing to PartialSourceFiles::group so pass on for recursion
                                // to do glob extension in the groups.sources.files list of PartialSourceFiles::file
                                Ok(vec![pre_glob_file])
                            }
                        }
                    })
                    .collect::<Result<Vec<Vec<PartialSourceFile>>>>()?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();

                let include_dirs: Result<Vec<_>> = include_dirs
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|path| match env_path_from_string(path.to_string()) {
                        Ok(p) => Some(Ok(p)),
                        Err(cause) => {
                            if Diagnostics::is_suppressed("E30") {
                                Warnings::IgnoredPath {cause: cause.to_string()}.emit();
                                None
                            } else {
                                Some(Err(Error::chain("[E30]", cause)))
                            }
                        }
                    })
                    .collect();

                let defines = defines.unwrap_or_default();
                let files: Result<Vec<_>> = post_glob_files
                    .into_iter()
                    .map(|f| f.validate(package_name, pre_output))
                    .collect();
                let files: Vec<SourceFile> = files?;
                let files: Vec<SourceFile> = files.into_iter().collect();
                if files.is_empty() && !pre_output {
                    Warnings::NoFilesInSourceGroup(package_name.to_string()).emit();
                }
                if !pre_output {
                    extra.iter().for_each(|(k, _)| {
                        Warnings::IgnoreUnknownField {
                            field: k.clone(),
                            pkg: package_name.to_string(),
                        }
                        .emit();
                    });
                }
                Ok(SourceFile::Group(Box::new(Sources {
                    target: target.unwrap_or_default(),
                    include_dirs: include_dirs?,
                    defines,
                    files,
                })))
            }
            PartialSources {
                target: None,
                include_dirs: None,
                defines: None,
                files: None,
                sv: _sv,
                v: _v,
                vhd: _vhd,
                external_flists: None,
                extra: _,
            } => {
                Err(Error::new("Only a single source with a single type is supported."))
            },
            _ => {
                Err(Error::new(
                    "Do not mix `sv`, `v`, or `vhd` with `files`, `target`, `include_dirs`, and `defines`.",
                ))
            }
        }
    }
}

/// A partial source file.
#[derive(Debug)]
pub enum PartialSourceFile {
    /// A single file.
    File(String),
    /// A subgroup of sources.
    Group(Box<PartialSources>),
    /// A systemverilog source.
    SvFile(String),
    /// A verilog source.
    VerilogFile(String),
    /// A vhdl source.
    VhdlFile(String),
}

impl PrefixPaths for PartialSourceFile {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(match self {
            PartialSourceFile::File(path) => PartialSourceFile::File(path.prefix_paths(prefix)?),
            PartialSourceFile::Group(group) => {
                PartialSourceFile::Group(Box::new(group.prefix_paths(prefix)?))
            }
            PartialSourceFile::SvFile(path) => {
                PartialSourceFile::SvFile(path.prefix_paths(prefix)?)
            }
            PartialSourceFile::VerilogFile(path) => {
                PartialSourceFile::VerilogFile(path.prefix_paths(prefix)?)
            }
            PartialSourceFile::VhdlFile(path) => {
                PartialSourceFile::VhdlFile(path.prefix_paths(prefix)?)
            }
        })
    }
}

// Custom serialization for partial source files.
impl Serialize for PartialSourceFile {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            PartialSourceFile::File(ref path) => path.serialize(serializer),
            PartialSourceFile::Group(ref srcs) => srcs.serialize(serializer),
            PartialSourceFile::SvFile(ref path) => path.serialize(serializer),
            PartialSourceFile::VerilogFile(ref path) => path.serialize(serializer),
            PartialSourceFile::VhdlFile(ref path) => path.serialize(serializer),
        }
    }
}

// Custom deserialization for partial source files.
impl<'de> Deserialize<'de> for PartialSourceFile {
    fn deserialize<D>(deserializer: D) -> std::result::Result<PartialSourceFile, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;
        use std::result::Result;
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = PartialSourceFile;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("string or map")
            }

            // Parse a single source file.
            fn visit_str<E>(self, value: &str) -> Result<PartialSourceFile, E>
            where
                E: de::Error,
            {
                Ok(PartialSourceFile::File(value.into()))
            }

            // Parse an entire source file group.
            fn visit_map<M>(self, visitor: M) -> Result<PartialSourceFile, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                let srcs =
                    PartialSources::deserialize(de::value::MapAccessDeserializer::new(visitor))?;
                Ok(PartialSourceFile::Group(Box::new(srcs)))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl Validate for PartialSourceFile {
    type Output = SourceFile;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<SourceFile> {
        match self {
            PartialSourceFile::File(path) => Ok(SourceFile::File(PathBuf::from(path))),
            // PartialSourceFile::Group(srcs) => Ok(Some(SourceFile::Group(Box::new(
            //     srcs.validate(package_name, pre_output, suppress_warnings)?,
            // )))),
            PartialSourceFile::Group(srcs) => Ok(srcs.validate(package_name, pre_output)?),
            PartialSourceFile::SvFile(path) => Ok(SourceFile::SvFile(env_path_from_string(path)?)),
            PartialSourceFile::VerilogFile(path) => {
                Ok(SourceFile::VerilogFile(env_path_from_string(path)?))
            }
            PartialSourceFile::VhdlFile(path) => {
                Ok(SourceFile::VhdlFile(env_path_from_string(path)?))
            }
        }
    }
}

/// Converts a file with glob in name to a list of matching files
pub trait GlobFile {
    /// The output type produced by validation.
    type Output;
    /// The error type produced by validation.
    type Error;
    /// Validate self and convert to a full list of paths that exist
    fn glob_file(self) -> Result<Self::Output>;
}

impl GlobFile for PartialSourceFile {
    type Output = Vec<PartialSourceFile>;
    type Error = Error;

    fn glob_file(self) -> Result<Vec<PartialSourceFile>> {
        // let mut partial_source_files_vec: Vec<PartialSourceFile> = Vec::new();

        // Only operate on files, not groups
        match self {
            PartialSourceFile::File(ref path)
            | PartialSourceFile::SvFile(ref path)
            | PartialSourceFile::VerilogFile(ref path)
            | PartialSourceFile::VhdlFile(ref path) => {
                // Check if glob patterns used
                if path.contains("*") || path.contains("?") {
                    let glob_matches = glob(path).map_err(|cause| {
                        Error::chain(format!("Invalid glob pattern for {:?}", path), cause)
                    })?;
                    let out = glob_matches
                        .map(|glob_match| {
                            let file_str = glob_match
                                .map_err(|cause| {
                                    Error::chain(format!("Glob match failed for {:?}", path), cause)
                                })?
                                .to_str()
                                .unwrap()
                                .to_string();
                            Ok(match self {
                                PartialSourceFile::File(_) => PartialSourceFile::File(file_str),
                                PartialSourceFile::SvFile(_) => PartialSourceFile::SvFile(file_str),
                                PartialSourceFile::VerilogFile(_) => {
                                    PartialSourceFile::VerilogFile(file_str)
                                }
                                PartialSourceFile::VhdlFile(_) => {
                                    PartialSourceFile::VhdlFile(file_str)
                                }
                                _ => unreachable!(),
                            })
                        })
                        .collect::<Result<Vec<PartialSourceFile>>>()?;
                    if out.is_empty() {
                        Warnings::NoFilesForGlobPattern { path: path.clone() }.emit();
                    }
                    Ok(out)
                } else {
                    // Return self if not a glob pattern
                    Ok(vec![self])
                }
            }
            _ => {
                // Return self if not a glob pattern
                Ok(vec![self])
            }
        }
    }
}

/// A partial workspace configuration.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialWorkspace {
    /// The directory which will contain working copies of the dependencies.
    pub checkout_dir: Option<String>,
    /// The locally linked packages.
    pub package_links: Option<IndexMap<String, String>>,
    /// Unknown extra fields
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl PrefixPaths for PartialWorkspace {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(PartialWorkspace {
            checkout_dir: self.checkout_dir.prefix_paths(prefix)?,
            package_links: match self.package_links {
                Some(idx_map) => Some(
                    idx_map
                        .into_iter()
                        .map(|(k, v)| Ok((k.prefix_paths(prefix)?, v)))
                        .collect::<Result<_>>()?,
                ),
                None => None,
            },
            extra: self.extra,
        })
    }
}

impl Validate for PartialWorkspace {
    type Output = Workspace;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<Workspace> {
        let package_links: Result<IndexMap<_, _>> = self
            .package_links
            .unwrap_or_default()
            .iter()
            .map(|(k, v)| Ok((env_path_from_string(k.to_string())?, v.clone())))
            .collect();
        if !pre_output {
            self.extra.iter().for_each(|(k, _)| {
                Warnings::IgnoreUnknownField {
                    field: k.clone(),
                    pkg: package_name.to_string(),
                }
                .emit();
            });
        }
        Ok(Workspace {
            checkout_dir: match self.checkout_dir {
                Some(dir) => Some(env_path_from_string(dir)?),
                None => None,
            },
            package_links: package_links?,
        })
    }
}

/// Merges missing information from another struct.
pub trait Merge {
    /// Populate missing fields from `other`.
    fn merge(self, other: Self) -> Self;
}

/// Prefixes relative paths.
pub trait PrefixPaths {
    /// Prefixes all paths with `prefix`. Does not touch absolute paths.
    fn prefix_paths(self, prefix: &Path) -> Result<Self>
    where
        Self: std::marker::Sized;
}

impl PrefixPaths for PathBuf {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(prefix.join(self))
    }
}

impl PrefixPaths for String {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        // env is resolved later, convert back to string here.
        Ok(prefix.join(PathBuf::from(&self)).display().to_string())
    }
}

impl<T> PrefixPaths for Option<T>
where
    T: PrefixPaths,
{
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        self.map_or(Ok(None), |inner| Ok(Some(inner.prefix_paths(prefix)?)))
    }
}

// Implement `PrefixPaths` for hash maps of prefixable values.
impl<K, V> PrefixPaths for IndexMap<K, V>
where
    K: Hash + Eq,
    V: PrefixPaths,
{
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        self.into_iter()
            .map(|(k, v)| Ok((k, v.prefix_paths(prefix)?)))
            .collect()
    }
}

// Implement `PrefixPaths` for vectors of prefixable values.
impl<V> PrefixPaths for Vec<V>
where
    V: PrefixPaths,
{
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        self.into_iter().map(|v| v.prefix_paths(prefix)).collect()
    }
}

/// A configuration.
///
/// This struct encapsulates every setting of the tool that can be changed by
/// the user by some means. It is constructed from a partial configuration.
#[derive(Serialize, Debug)]
pub struct Config {
    /// The path to the database directory.
    pub database: PathBuf,
    /// The git command or path to the binary.
    pub git: String,
    /// The dependency overrides.
    pub overrides: IndexMap<String, Dependency>,
    /// The auxiliary plugin dependencies.
    pub plugins: IndexMap<String, Dependency>,
    /// The git throttle value to use unless overridden by the user.
    pub git_throttle: Option<usize>,
}

/// A partial configuration.
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialConfig {
    /// The path to the database directory.
    pub database: Option<String>,
    /// The git command or path to the binary.
    pub git: Option<String>,
    /// The dependency overrides.
    pub overrides: Option<IndexMap<String, PartialDependency>>,
    /// The auxiliary plugin dependencies.
    pub plugins: Option<IndexMap<String, PartialDependency>>,
    /// The git throttle value to use unless overridden by the user.
    pub git_throttle: Option<usize>,
}

impl PartialConfig {
    /// Create a new empty configuration.
    pub fn new() -> PartialConfig {
        PartialConfig {
            database: None,
            git: None,
            overrides: None,
            plugins: None,
            git_throttle: None,
        }
    }
}

impl Default for PartialConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixPaths for PartialConfig {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(PartialConfig {
            database: self.database.prefix_paths(prefix)?,
            overrides: self.overrides.prefix_paths(prefix)?,
            plugins: self.plugins.prefix_paths(prefix)?,
            ..self
        })
    }
}

impl Merge for PartialConfig {
    fn merge(self, other: PartialConfig) -> PartialConfig {
        PartialConfig {
            database: self.database.or(other.database),
            git: self.git.or(other.git),
            overrides: match (self.overrides, other.overrides) {
                (Some(o), None) | (None, Some(o)) => Some(o),
                (Some(mut o1), Some(o2)) => {
                    o1.extend(o2);
                    Some(o1)
                }
                (None, None) => None,
            },
            plugins: match (self.plugins, other.plugins) {
                (Some(o), None) | (None, Some(o)) => Some(o),
                (Some(mut o1), Some(o2)) => {
                    o1.extend(o2);
                    Some(o1)
                }
                (None, None) => None,
            },
            git_throttle: self.git_throttle.or(other.git_throttle),
        }
    }
}

impl Validate for PartialConfig {
    type Output = Config;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<Config> {
        Ok(Config {
            database: match self.database {
                Some(db) => env_path_from_string(db)?,
                None => return Err(Error::new("Database directory not configured")),
            },
            git: match self.git {
                Some(git) => git,
                None => return Err(Error::new("Git command or path to binary not configured")),
            },
            overrides: match self.overrides {
                Some(d) => d
                    .validate(package_name, pre_output)
                    .map_err(|(key, cause)| {
                        Error::chain(format!("In override `{}`:", key), cause)
                    })?,
                None => IndexMap::new(),
            },
            plugins: match self.plugins {
                Some(d) => d
                    .validate(package_name, pre_output)
                    .map_err(|(key, cause)| Error::chain(format!("In plugin `{}`:", key), cause))?,
                None => IndexMap::new(),
            },
            git_throttle: self.git_throttle,
        })
    }
}

/// An external import dependency
#[derive(Serialize, Debug)]
pub struct VendorPackage {
    /// External dependency name
    pub name: String,
    /// Target folder for imported dependency
    pub target_dir: PathBuf,
    /// Upstream dependency reference
    pub upstream: Dependency,
    /// Import mapping
    pub mapping: Vec<FromToLink>,
    /// Folder containing patch files
    pub patch_dir: Option<PathBuf>,
    /// include from upstream
    pub include_from_upstream: Vec<String>,
    /// exclude from upstream
    pub exclude_from_upstream: Vec<String>,
}

impl PrefixPaths for VendorPackage {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        let patch_root = self.patch_dir.prefix_paths(prefix)?;
        Ok(VendorPackage {
            name: self.name,
            target_dir: self.target_dir.prefix_paths(prefix)?,
            upstream: self.upstream,
            mapping: self
                .mapping
                .into_iter()
                .map(|ftl| {
                    Ok(FromToLink {
                        from: ftl.from,
                        to: ftl.to,
                        patch_dir: ftl.patch_dir.map_or(
                            Ok::<Option<PathBuf>, Error>(None),
                            |dir| {
                                Ok(Some({
                                    dir.prefix_paths(&patch_root.clone().expect(
                            "A mapping has a local patch_dir, but no global patch_dir is defined.",
                        ))?
                                }))
                            },
                        )?,
                    })
                })
                .collect::<Result<_>>()?,
            patch_dir: patch_root,
            include_from_upstream: self.include_from_upstream,
            exclude_from_upstream: self.exclude_from_upstream,
        })
    }
}

/// A partial external import dependency
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialVendorPackage {
    /// External dependency name
    pub name: Option<String>,
    /// Target folder for imported dependency
    pub target_dir: Option<String>,
    /// Upstream dependency reference
    pub upstream: Option<PartialDependency>,
    /// Import mapping
    pub mapping: Option<Vec<FromToLink>>,
    /// Folder containing patch files
    pub patch_dir: Option<String>,
    // /// Dependency containing patches
    // pub patch_repo: Option<PartialDependency>,
    /// include from upstream
    pub include_from_upstream: Option<Vec<String>>,
    /// exclude from upstream
    pub exclude_from_upstream: Option<Vec<String>>,
}

impl PrefixPaths for PartialVendorPackage {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        let patch_root = self.patch_dir.prefix_paths(prefix)?;
        Ok(PartialVendorPackage {
            name: self.name,
            target_dir: self.target_dir.prefix_paths(prefix)?,
            upstream: self.upstream,
            mapping: match self.mapping {
                Some(mapping_vec) => Some(mapping_vec
                    .into_iter()
                    .map(|ftl| {
                        Ok(FromToLink {
                            from: ftl.from,
                            to: ftl.to,
                            patch_dir: ftl.patch_dir.map_or(
                                Ok::<Option<PathBuf>, Error>(None),
                                |dir| {
                                    Ok(Some({
                                        dir.prefix_paths(Path::new(&patch_root.clone().expect(
                                "A mapping has a local patch_dir, but no global patch_dir is defined.",
                            )))?
                                    }))
                                },
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?),
                None => None,
            },
            patch_dir: patch_root,
            include_from_upstream: self.include_from_upstream,
            exclude_from_upstream: self.exclude_from_upstream,
        })
    }
}

impl Validate for PartialVendorPackage {
    type Output = VendorPackage;
    type Error = Error;
    fn validate(self, package_name: &str, pre_output: bool) -> Result<VendorPackage> {
        Ok(VendorPackage {
            name: match self.name {
                Some(name) => name,
                None => return Err(Error::new("external import name missing")),
            },
            target_dir: match self.target_dir {
                Some(target_dir) => env_path_from_string(target_dir)?,
                None => return Err(Error::new("external import target dir missing")),
            },
            upstream: match self.upstream {
                Some(upstream) => upstream
                    .validate(package_name, pre_output)
                    .map_err(|cause| {
                        Error::chain("Unable to parse external import upstream", cause)
                    })?,
                None => return Err(Error::new("external import upstream missing")),
            },
            mapping: self.mapping.unwrap_or_default(),
            patch_dir: match self.patch_dir {
                Some(patch_dir) => Some(env_path_from_string(patch_dir)?),
                None => None,
            },
            include_from_upstream: match self.include_from_upstream {
                Some(include_from_upstream) => include_from_upstream,
                None => vec![String::from("")],
            },
            exclude_from_upstream: {
                let mut excl = self.exclude_from_upstream.unwrap_or_default();
                excl.push(String::from(".git"));
                excl
            },
        })
    }
}

/// An external import linkage
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FromToLink {
    /// from string
    pub from: PathBuf,
    /// to string
    pub to: PathBuf,
    /// directory
    pub patch_dir: Option<PathBuf>,
}

/// A passed target
#[derive(Clone, Default, Serialize, Debug, Hash, PartialEq, Eq)]
pub struct PassedTarget {
    /// Target name
    pub target: TargetSpec,
    /// Target value
    pub pass: String,
}

/// A partial passed target
#[derive(Serialize, Deserialize, Debug)]
pub struct PartialPassedTarget {
    /// Filtering target specification
    pub target: Option<TargetSpec>,
    /// Target to pass
    pub pass: Option<String>,
}

impl Validate for PartialPassedTarget {
    type Output = PassedTarget;
    type Error = Error;
    fn validate(self, _package_name: &str, _pre_output: bool) -> Result<PassedTarget> {
        Ok(PassedTarget {
            target: self.target.unwrap_or_default(),
            pass: match self.pass {
                Some(p) => p.to_lowercase(),
                None => return Err(Error::new("passed target missing pass value")),
            },
        })
    }
}

impl FromStr for PartialPassedTarget {
    type Err = Void;
    fn from_str(s: &str) -> std::result::Result<Self, Void> {
        Ok(PartialPassedTarget {
            target: None,
            pass: Some(s.to_string()),
        })
    }
}

impl fmt::Display for PassedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "target: `{}`: `{}`", self.target, self.pass)
    }
}

/// A lock file.
///
/// This struct encapsulates the result of dependency resolution. For every
/// dependency in the package it lists the exact source and version.
#[derive(Serialize, Deserialize, Debug)]
pub struct Locked {
    /// The locked package versions.
    pub packages: BTreeMap<String, LockedPackage>,
}

/// A locked dependency.
///
/// Encapsualtes the exact source and version of a dependency.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct LockedPackage {
    /// The revision hash of the dependency.
    pub revision: Option<String>,
    /// The version of the dependency.
    pub version: Option<String>,
    /// The source of the dependency.
    #[serde(with = "serde_yaml_ng::with::singleton_map")]
    pub source: LockedSource,
    /// Other packages this package depends on.
    pub dependencies: BTreeSet<String>,
}

/// A source description for a locked dependency.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum LockedSource {
    /// A path on the system.
    Path(PathBuf),
    /// A git URL.
    Git(String),
    /// A registry.
    Registry(String),
}

#[cfg(unix)]
fn env_string_from_string(path_str: String) -> Result<String> {
    subst::substitute(&path_str, &subst::Env).map_err(|cause| {
        Error::chain(
            format!("Unable to substitute with env: {}", path_str),
            cause,
        )
    })
}

#[cfg(windows)]
fn env_string_from_string(path_str: String) -> Result<String> {
    Ok(path_str)
}

pub(crate) fn env_path_from_string(path_str: String) -> Result<PathBuf> {
    Ok(PathBuf::from(env_string_from_string(path_str)?))
}
