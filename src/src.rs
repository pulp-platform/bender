// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A source code manifest.
//!
//! This module implements a source code manifest.

#![deny(missing_docs)]

use std::fmt;
use std::iter::FromIterator;
use std::path::Path;

use indexmap::{IndexMap, IndexSet};
use serde::{Serialize, Serializer};

use crate::config::{Validate, ValidationContext};
use crate::diagnostic::{Diagnostics, Warnings};
use crate::error::Error;
use crate::target::{TargetSet, TargetSpec};
use semver;

/// A source file group.
#[derive(Serialize, Clone, Debug, Default)]
pub struct SourceGroup<'ctx> {
    /// The package which this source group represents.
    pub package: Option<&'ctx str>,
    /// Whether the source files in this group can be treated in parallel.
    pub independent: bool,
    /// The targets for which the sources should be considered.
    pub target: TargetSpec,
    /// The directories to search for include files.
    pub include_dirs: IndexSet<&'ctx Path>,
    /// The directories exported by dependent package for include files, with their target specs.
    pub export_incdirs: IndexMap<String, Vec<(TargetSpec, &'ctx Path)>>,
    /// The preprocessor definitions.
    pub defines: IndexMap<String, Option<&'ctx str>>,
    /// The files in this group.
    pub files: Vec<SourceFile<'ctx>>,
    /// Package dependencies of this source group
    pub dependencies: IndexSet<String>,
    /// Version information of the package
    pub version: Option<semver::Version>,
    /// This group will override files in previous packages
    pub override_files: bool,
}

impl<'ctx> Validate for SourceGroup<'ctx> {
    type Output = SourceGroup<'ctx>;
    type Error = Error;
    fn validate(self, vctx: &ValidationContext) -> crate::error::Result<SourceGroup<'ctx>> {
        Ok(SourceGroup {
            files: self
                .files
                .into_iter()
                .map(|f| f.validate(vctx))
                .collect::<Result<Vec<_>, Error>>()?,
            include_dirs: self
                .include_dirs
                .into_iter()
                .map(|p| {
                    if !p.exists() || !p.is_dir() {
                        Warnings::IncludeDirMissing(p.to_path_buf()).emit();
                    }
                    Ok(p)
                })
                .collect::<Result<IndexSet<_>, Error>>()?,
            export_incdirs: self
                .export_incdirs
                .into_iter()
                .map(|(pkg, dirs)| {
                    let checked = dirs
                        .into_iter()
                        .map(|(trgt, p)| {
                            if !p.exists() || !p.is_dir() {
                                Warnings::IncludeDirMissing(p.to_path_buf()).emit();
                            }
                            Ok((trgt, p))
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    Ok((pkg, checked))
                })
                .collect::<Result<IndexMap<_, _>, Error>>()?,
            ..self
        })
    }
}

impl<'ctx> SourceGroup<'ctx> {
    /// Simplify the source group. Removes empty subgroups and inlines subgroups
    /// with the same configuration.
    pub fn simplify(self) -> Self {
        let files = self
            .files
            .into_iter()
            .filter_map(|s| match s {
                SourceFile::Group(group) => {
                    let group = group.simplify();

                    // Discard empty groups.
                    if group.files.is_empty() {
                        return None;
                    }

                    // Drop groups with only one file.
                    if group.files.len() == 1
                        && group.include_dirs.is_empty()
                        && group.defines.is_empty()
                        && group.target.is_wildcard()
                        && group.package.is_none()
                    {
                        return Some(group.files.into_iter().next().unwrap());
                    }

                    // Preserve the rest.
                    Some(SourceFile::Group(Box::new(group)))
                }
                other => Some(other),
            })
            .collect();
        SourceGroup { files, ..self }
    }

    /// Filter the sources, keeping only the ones that apply to a target.
    pub fn filter_targets(&self, targets: &TargetSet) -> Option<SourceGroup<'ctx>> {
        let all_targets = match self.package {
            Some(pkg) => targets.reduce_for_dependency(pkg),
            None => targets.clone(),
        };

        if !self.target.matches(&all_targets) {
            return None;
        }
        let files = self
            .files
            .iter()
            .filter_map(|file| match *file {
                SourceFile::Group(ref group) => group
                    .filter_targets(targets)
                    .map(|g| SourceFile::Group(Box::new(g))),
                ref other => Some(other.clone()),
            })
            .collect();
        let mut target_defs = all_targets
            .into_iter()
            .map(|t| "TARGET_".to_owned() + &t.to_uppercase())
            .collect::<IndexSet<_>>();
        target_defs.sort();
        let mut defines: IndexMap<String, Option<&str>> = self.defines.clone();
        if self.package.is_some() {
            defines.extend(target_defs.into_iter().map(|t| (t, None)));
        }

        let export_incdirs = self
            .export_incdirs
            .iter()
            .map(|(pkg, dirs)| {
                let pkg_targets = targets.reduce_for_dependency(pkg.as_str());
                let filtered = dirs
                    .iter()
                    .filter(|(trgt, _)| trgt.matches(&pkg_targets))
                    .cloned()
                    .collect();
                (pkg.clone(), filtered)
            })
            .collect();

        Some(
            SourceGroup {
                defines: defines.clone(),
                export_incdirs,
                files,
                ..self.clone()
            }
            .simplify(),
        )
    }

    /// Assigns target to SourceGroup without target
    pub fn assign_target(&self, target: String) -> SourceGroup<'ctx> {
        let files = self
            .files
            .iter()
            .filter_map(|file| match *file {
                SourceFile::Group(ref group) => Some(group.assign_target(target.clone()))
                    .map(|g| SourceFile::Group(Box::new(g))),
                ref other => Some(other.clone()),
            })
            .collect();

        SourceGroup {
            target: if self.target.is_wildcard() {
                TargetSpec::Name(target)
            } else {
                self.target.clone()
            },
            files,
            ..self.clone()
        }
    }

    /// Recursively get dependency names.
    fn get_deps(
        &self,
        packages: &IndexSet<String>,
        excludes: &IndexSet<String>,
    ) -> IndexSet<String> {
        let mut result = packages.clone();

        if let Some(x) = self.package {
            if result.contains(x) {
                result.extend(IndexSet::<String>::from_iter(self.dependencies.clone()));
                result = &result - excludes;
            }
        }

        for file in &self.files {
            if let SourceFile::Group(group) = file {
                result.extend(group.get_deps(&result, excludes));
            }
        }

        result
    }

    /// Get list of packages based on constraints.
    pub fn get_package_list(
        &self,
        top_package: String,
        packages: &IndexSet<String>,
        excludes: &IndexSet<String>,
        no_deps: bool,
    ) -> IndexSet<String> {
        let mut result = IndexSet::new();

        if !packages.is_empty() {
            result.extend(packages.clone());
        } else {
            result.insert(top_package);
        }

        result = &result - excludes;

        if !no_deps {
            let mut curr_length = 0;
            while curr_length < result.len() {
                curr_length = result.len();
                result.extend(self.get_deps(&result, excludes));
            }
        }

        result
    }

    /// Filter the sources, keeping only the ones that apply to the selected packages.
    pub fn filter_packages(&self, packages: &IndexSet<String>) -> Option<SourceGroup<'ctx>> {
        let mut files = Vec::new();

        if self.package.is_none() || packages.contains(self.package.unwrap()) {
            files = self
                .files
                .iter()
                .filter_map(|file| match *file {
                    SourceFile::Group(ref group) => group
                        .filter_packages(packages)
                        .map(|g| SourceFile::Group(Box::new(g))),
                    ref other => Some(other.clone()),
                })
                .collect();
        }

        let export_incdirs = self.export_incdirs.clone();
        Some(
            SourceGroup {
                export_incdirs,
                files,
                ..self.clone()
            }
            .simplify(),
        )
    }

    /// Return list of unique include directories for the current src
    pub fn get_incdirs(self) -> Vec<&'ctx Path> {
        let incdirs = self
            .include_dirs
            .into_iter()
            .chain(
                self.export_incdirs
                    .into_iter()
                    .flat_map(|(_, v)| v.into_iter().map(|(_, path)| path)),
            )
            .fold(IndexSet::new(), |mut acc, inc_dir| {
                acc.insert(inc_dir);
                acc
            });
        incdirs.into_iter().collect()
    }

    /// Flatten nested source groups.
    ///
    /// Removes all levels of hierarchy and produces a canonical list of source
    /// groups.
    pub fn flatten(self) -> Vec<SourceGroup<'ctx>> {
        let mut v = vec![];
        self.flatten_into(&mut v);
        v
    }

    fn flatten_into(mut self, into: &mut Vec<SourceGroup<'ctx>>) {
        let mut files = vec![];
        let subfiles = std::mem::take(&mut self.files);
        let flush_files = |files: &mut Vec<SourceFile<'ctx>>, into: &mut Vec<SourceGroup<'ctx>>| {
            if files.is_empty() {
                return;
            }
            let files = std::mem::take(files);
            into.push(SourceGroup {
                files,
                ..self.clone()
            });
        };
        for file in subfiles {
            match file {
                SourceFile::File(_, _) => {
                    files.push(file);
                }
                SourceFile::Group(grp) => {
                    let mut grp = *grp;
                    if !self.independent {
                        flush_files(&mut files, into);
                    }
                    grp.package = grp.package.or(self.package);
                    grp.independent &= self.independent;
                    grp.target = TargetSpec::All(
                        [&self.target, &grp.target]
                            .iter()
                            .map(|&i| i.clone())
                            .collect(),
                    );
                    grp.include_dirs = IndexSet::<&Path>::from_iter(
                        self.include_dirs
                            .iter()
                            .cloned()
                            .chain(grp.include_dirs.into_iter()),
                    )
                    .into_iter()
                    .collect();
                    grp.defines = self
                        .defines
                        .iter()
                        .map(|(k, v)| (k.clone(), *v))
                        .chain(grp.defines.into_iter())
                        .collect();
                    grp.flatten_into(into);
                }
            }
        }
        flush_files(&mut files, into);
    }

    /// Get available targets in sourcegroup.
    pub fn get_avail_targets(&self) -> IndexSet<String> {
        let mut targets = self.target.get_avail();
        for file in &self.files {
            match file {
                SourceFile::File(..) => {}
                SourceFile::Group(group) => {
                    targets.extend(group.get_avail_targets());
                }
            }
        }
        targets
    }
}

/// File types for a source file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SourceType {
    /// A Verilog file.
    Verilog,
    // /// A SystemVerilog file.
    // SystemVerilog,
    /// A VHDL file.
    Vhdl,
    /// Unknown file type
    Unknown,
}

/// A source file.
///
/// This can either be an individual file, or a subgroup of files.
#[derive(Clone)]
pub enum SourceFile<'ctx> {
    /// A file.
    File(&'ctx Path, &'ctx Option<SourceType>),
    /// A group of files.
    Group(Box<SourceGroup<'ctx>>),
}

impl<'ctx> fmt::Debug for SourceFile<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            SourceFile::File(path, ty) => {
                fmt::Debug::fmt(path, f)?;
                if let Some(t) = ty {
                    write!(f, " as {:?}", t)
                } else {
                    write!(f, " as <unknown>")
                }
            }
            SourceFile::Group(ref srcs) => fmt::Debug::fmt(srcs, f),
        }
    }
}

impl<'ctx> From<SourceGroup<'ctx>> for SourceFile<'ctx> {
    fn from(group: SourceGroup<'ctx>) -> SourceFile<'ctx> {
        SourceFile::Group(Box::new(group))
    }
}

impl<'ctx> From<&'ctx Path> for SourceFile<'ctx> {
    fn from(path: &'ctx Path) -> SourceFile<'ctx> {
        SourceFile::File(path, &None)
    }
}

impl<'ctx> Validate for SourceFile<'ctx> {
    type Output = SourceFile<'ctx>;
    type Error = Error;
    fn validate(self, vctx: &ValidationContext) -> Result<SourceFile<'ctx>, Error> {
        match self {
            SourceFile::File(path, ty) => {
                let env_path_buf =
                    crate::config::env_path_from_string(path.to_string_lossy().to_string())?;
                let exists = env_path_buf.exists() && env_path_buf.is_file();
                if exists || Diagnostics::is_suppressed("E31") {
                    if !exists {
                        Warnings::FileMissing {
                            path: env_path_buf.clone(),
                        }
                        .emit();
                    }
                    Ok(SourceFile::File(path, ty))
                } else {
                    Err(Error::new(format!(
                        "[E31] File {} doesn't exist",
                        env_path_buf.to_string_lossy()
                    )))
                }
            }
            SourceFile::Group(srcs) => Ok(SourceFile::Group(Box::new(srcs.validate(vctx)?))),
        }
    }
}

// Custom serialization for source files.
impl<'ctx> Serialize for SourceFile<'ctx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            SourceFile::File(path, _) => path.serialize(serializer),
            SourceFile::Group(ref group) => group.serialize(serializer),
        }
    }
}
