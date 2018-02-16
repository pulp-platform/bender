// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A source code manifest.
//!
//! This module implements a source code manifest.

#![deny(missing_docs)]

use std::fmt;
use std::path::Path;

use serde::ser::{Serialize, Serializer};

/// A source file group.
#[derive(Serialize, Clone, Debug)]
pub struct SourceGroup<'ctx> {
    /// Whether the source files in this group can be treated in parallel.
    pub independent: bool,
    /// The files in this group.
    pub files: Vec<SourceFile<'ctx>>,
}

impl<'ctx> SourceGroup<'ctx> {
    /// Simplify the source group. Removes empty subgroups and inlines subgroups
    /// with the same configuration.
    pub fn simplify(self) -> Self {
        let files = self.files.into_iter().filter_map(|s| match s {
            SourceFile::Group(group) => {
                let group = group.simplify();

                // Discard empty groups.
                if group.files.is_empty() {
                    return None;
                }

                // Drop groups with only one file.
                if group.files.len() == 1 {
                    return Some(group.files.into_iter().next().unwrap());
                }

                // Preserve the rest.
                Some(SourceFile::Group(Box::new(group)))
            }
            other => Some(other),
        }).collect();
        SourceGroup {
            files: files,
            ..self
        }
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
            SourceFile::File(path)  => fmt::Debug::fmt(path, f),
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
        where S: Serializer
    {
        match *self {
            SourceFile::File(path) => path.serialize(serializer),
            SourceFile::Group(ref group) => group.serialize(serializer),
        }
    }
}
