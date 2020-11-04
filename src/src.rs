// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A source code manifest.
//!
//! This module implements a source code manifest.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::ser::{Serialize, Serializer};

use crate::target::{TargetSet, TargetSpec};

/// A source file group.
#[derive(Serialize, Clone, Debug)]
pub struct SourceGroup<'ctx> {
    /// The package which this source group represents.
    pub package: Option<&'ctx str>,
    /// Whether the source files in this group can be treated in parallel.
    pub independent: bool,
    /// The targets for which the sources should be considered.
    pub target: TargetSpec,
    /// The directories to search for include files.
    pub include_dirs: Vec<&'ctx Path>,
    /// The preprocessor definitions.
    pub defines: HashMap<&'ctx str, Option<&'ctx str>>,
    /// The files in this group.
    pub files: Vec<SourceFile<'ctx>>,
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
        SourceGroup {
            files: files,
            ..self
        }
    }

    /// Filter the sources, keeping only the ones that apply to a target.
    pub fn filter_targets(&self, targets: &TargetSet) -> Option<SourceGroup<'ctx>> {
        if !self.target.matches(targets) {
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
        Some(
            SourceGroup {
                package: self.package,
                independent: self.independent,
                target: self.target.clone(),
                include_dirs: self.include_dirs.clone(),
                defines: self.defines.clone(),
                files: files,
            }
            .simplify(),
        )
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
        let subfiles = std::mem::replace(&mut self.files, vec![]);
        let flush_files = |files: &mut Vec<SourceFile<'ctx>>, into: &mut Vec<SourceGroup<'ctx>>| {
            if files.is_empty() {
                return;
            }
            let files = std::mem::replace(files, vec![]);
            into.push(SourceGroup {
                files: files,
                ..self.clone()
            });
        };
        for file in subfiles {
            match file {
                SourceFile::File(_) => {
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
                    grp.include_dirs = self
                        .include_dirs
                        .iter()
                        .cloned()
                        .chain(grp.include_dirs.into_iter())
                        .collect();
                    grp.defines = self
                        .defines
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .chain(grp.defines.into_iter())
                        .collect();
                    grp.flatten_into(into);
                }
            }
        }
        flush_files(&mut files, into);
    }
}

/// A source file.
///
/// This can either be an individual file, or a subgroup of files.
#[derive(Clone)]
pub enum SourceFile<'ctx> {
    /// A file.
    File(&'ctx Path),
    /// A group of files.
    Group(Box<SourceGroup<'ctx>>),
}

impl<'ctx> fmt::Debug for SourceFile<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            SourceFile::File(path) => fmt::Debug::fmt(path, f),
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
        SourceFile::File(path)
    }
}

// Custom serialization for source files.
impl<'ctx> Serialize for SourceFile<'ctx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            SourceFile::File(path) => path.serialize(serializer),
            SourceFile::Group(ref group) => group.serialize(serializer),
        }
    }
}
