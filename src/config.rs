// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Package manifest and configuration files.
//!
//! This module provides reading and writing of package manifests and
//! configuration files.

#![deny(missing_docs)]

use std;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use indexmap::IndexMap;
use semver;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use subst;

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
}

/// A dependency.
///
/// The name of the dependency is given implicitly by the key in the hash map
/// that this `Dependency` is accessible through.
#[derive(Debug)]
pub enum Dependency {
    /// A dependency that can be found in one of the package repositories.
    Version(semver::VersionReq),
    /// A local path dependency. The exact version of the dependency found at
    /// the given path will be used, regardless of any actual versioning
    /// constraints.
    Path(PathBuf),
    /// A git dependency specified by a revision.
    GitRevision(String, String),
    /// A git dependency specified by a version requirement. Works similarly to
    /// the `GitRevision`, but extracts all tags of the form `v.*` from the
    /// repository and matches the version against that.
    GitVersion(String, semver::VersionReq),
}

impl PrefixPaths for Dependency {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(match self {
            Dependency::Path(p) => Dependency::Path(p.prefix_paths(prefix)?),
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
            Dependency::Version(ref version) => format!("{}", version).serialize(serializer),
            Dependency::Path(ref path) => path.serialize(serializer),
            Dependency::GitRevision(ref url, ref rev) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("git", url)?;
                map.serialize_entry("rev", rev)?;
                map.end()
            }
            Dependency::GitVersion(ref url, ref version) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("git", url)?;
                map.serialize_entry("version", &format!("{}", version))?;
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
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            SourceFile::File(ref path) => fmt::Debug::fmt(path, f),
            SourceFile::Group(ref srcs) => fmt::Debug::fmt(srcs, f),
        }
    }
}

impl PrefixPaths for SourceFile {
    fn prefix_paths(self, prefix: &Path) -> Result<Self> {
        Ok(match self {
            SourceFile::File(path) => SourceFile::File(path.prefix_paths(prefix)?),
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
    fn validate(self) -> std::result::Result<Self::Output, Self::Error>;
}

// Implement `Validate` for hash maps of validatable values.
impl<K, V> Validate for IndexMap<K, V>
where
    K: Hash + Eq,
    V: Validate<Error = Error>,
{
    type Output = IndexMap<K, V::Output>;
    type Error = (K, Error);
    fn validate(self) -> std::result::Result<Self::Output, Self::Error> {
        self.into_iter()
            .map(|(k, v)| match v.validate() {
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
    fn validate(self) -> std::result::Result<Self::Output, Self::Error> {
        self.into_iter()
            .map(|v| match v.validate() {
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
    fn validate(self) -> std::result::Result<T::Output, T::Error> {
        self.0.validate()
    }
}

// Implement `Validate` for `SeqOrStruct` wrapped validatable values.
impl<T, F> Validate for SeqOrStruct<T, F>
where
    T: Validate,
{
    type Output = T::Output;
    type Error = T::Error;
    fn validate(self) -> std::result::Result<T::Output, T::Error> {
        self.0.validate()
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
}

impl Validate for PartialManifest {
    type Output = Manifest;
    type Error = Error;
    fn validate(self) -> Result<Manifest> {
        let pkg = match self.package {
            Some(mut p) => {
                p.name = p.name.to_lowercase();
                p
            }
            None => return Err(Error::new("Missing package information.")),
        };
        let deps = match self.dependencies {
            Some(d) => d
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect::<IndexMap<_, _>>()
                .validate()
                .map_err(|(key, cause)| {
                    Error::chain(
                        format!("In dependency `{}` of package `{}`:", key, pkg.name),
                        cause,
                    )
                })?,
            None => IndexMap::new(),
        };
        let srcs = match self.sources {
            Some(s) => Some(s.validate().map_err(|cause| {
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
                .validate()
                .map_err(|cause| Error::chain("In workspace configuration:", cause))?,
            None => Workspace::default(),
        };
        let vendor_package = match self.vendor_package {
            Some(vend) => vend
                .validate()
                .map_err(|cause| Error::chain("Unable to parse vendor_package", cause))?,
            None => Vec::new(),
        };
        Ok(Manifest {
            package: pkg,
            dependencies: deps,
            sources: srcs,
            export_include_dirs: exp_inc_dirs
                .iter()
                .map(|path| env_path_from_string(path.to_string()))
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
}

impl FromStr for PartialDependency {
    type Err = Void;
    fn from_str(s: &str) -> std::result::Result<Self, Void> {
        Ok(PartialDependency {
            path: None,
            git: None,
            rev: None,
            version: Some(s.into()),
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
    fn validate(self) -> Result<Dependency> {
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
                Ok(Dependency::Path(env_path_from_string(path)?))
            }
        } else if let Some(git) = self.git {
            if let Some(rev) = self.rev {
                Ok(Dependency::GitRevision(git, rev))
            } else if let Some(version) = version {
                Ok(Dependency::GitVersion(git, version))
            } else {
                Err(Error::new(
                    "A `git` dependency must have either a `rev` or `version` field.",
                ))
            }
        } else if let Some(version) = version {
            Ok(Dependency::Version(version))
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
    pub files: Vec<PartialSourceFile>,
}

impl From<Vec<PartialSourceFile>> for PartialSources {
    fn from(v: Vec<PartialSourceFile>) -> Self {
        PartialSources {
            target: None,
            include_dirs: None,
            defines: None,
            files: v,
        }
    }
}

impl Validate for PartialSources {
    type Output = Sources;
    type Error = Error;
    fn validate(self) -> Result<Sources> {
        let include_dirs: Result<Vec<_>> = self
            .include_dirs
            .unwrap_or_default()
            .iter()
            .map(|path| env_path_from_string(path.to_string()))
            .collect();
        let defines = self.defines.unwrap_or_default();
        let files: Result<Vec<_>> = self.files.into_iter().map(|f| f.validate()).collect();
        Ok(Sources {
            target: self.target.unwrap_or(TargetSpec::Wildcard),
            include_dirs: include_dirs?,
            defines,
            files: files?,
        })
    }
}

/// A partial source file.
#[derive(Debug)]
pub enum PartialSourceFile {
    /// A single file.
    File(String),
    /// A subgroup of sources.
    Group(Box<PartialSources>),
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
    fn validate(self) -> Result<SourceFile> {
        match self {
            PartialSourceFile::File(path) => Ok(SourceFile::File(env_path_from_string(path)?)),
            PartialSourceFile::Group(srcs) => Ok(SourceFile::Group(Box::new(srcs.validate()?))),
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
}

impl Validate for PartialWorkspace {
    type Output = Workspace;
    type Error = Error;
    fn validate(self) -> Result<Workspace> {
        let package_links: Result<IndexMap<_, _>> = self
            .package_links
            .unwrap_or_default()
            .iter()
            .map(|(k, v)| Ok((env_path_from_string(k.to_string())?, v.clone())))
            .collect();
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
        Ok(prefix
            .join(env_path_from_string(self)?)
            .display()
            .to_string())
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
}

impl PartialConfig {
    /// Create a new empty configuration.
    pub fn new() -> PartialConfig {
        PartialConfig {
            database: None,
            git: None,
            overrides: None,
            plugins: None,
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
        }
    }
}

impl Validate for PartialConfig {
    type Output = Config;
    type Error = Error;
    fn validate(self) -> Result<Config> {
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
                Some(d) => d.validate().map_err(|(key, cause)| {
                    Error::chain(format!("In override `{}`:", key), cause)
                })?,
                None => IndexMap::new(),
            },
            plugins: match self.plugins {
                Some(d) => d
                    .validate()
                    .map_err(|(key, cause)| Error::chain(format!("In plugin `{}`:", key), cause))?,
                None => IndexMap::new(),
            },
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

impl Validate for PartialVendorPackage {
    type Output = VendorPackage;
    type Error = Error;
    fn validate(self) -> Result<VendorPackage> {
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
                Some(upstream) => upstream.validate().map_err(|cause| {
                    Error::chain("Unable to parse external import upstream", cause)
                })?,
                None => return Err(Error::new("external import upstream missing")),
            },
            mapping: match self.mapping {
                Some(mapping) => mapping,
                None => Vec::new(),
            },
            patch_dir: match self.patch_dir {
                Some(patch_dir) => Some(env_path_from_string(patch_dir)?),
                None => None,
            },
            include_from_upstream: match self.include_from_upstream {
                Some(include_from_upstream) => include_from_upstream,
                None => vec![String::from("")],
            },
            exclude_from_upstream: {
                let mut excl = match self.exclude_from_upstream {
                    Some(exclude_from_upstream) => exclude_from_upstream,
                    None => Vec::new(),
                };
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
    #[serde(with = "serde_yaml::with::singleton_map")]
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
fn env_path_from_string(path_str: String) -> Result<PathBuf> {
    Ok(PathBuf::from(
        subst::substitute(&path_str, &subst::Env).map_err(|cause| {
            Error::chain(
                format!("Unable to substitute with env: {}", path_str),
                cause,
            )
        })?,
    ))
}

#[cfg(windows)]
fn env_path_from_string(path_str: String) -> Result<PathBuf> {
    Ok(PathBuf::from(path_str))
}
