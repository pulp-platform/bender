// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A source code manifest.
//!
//! This module implements a source code manifest.

#![deny(missing_docs)]

use std::path::Path;

/// A source file group.
#[derive(Clone, Debug)]
pub struct SourceGroup<'ctx> {
    /// The base path for all relative files in this group.
    pub path: &'ctx Path,
    /// Whether the source files in this group can be treated in parallel.
    pub independent: bool,
    /// The files in this group.
    pub files: Vec<SourceFile<'ctx>>,
}

/// A source file.
///
/// This can either be an individual file, or a subgroup of files.
#[derive(Clone, Debug)]
pub enum SourceFile<'ctx> {
    /// A file.
    File(&'ctx Path),
    /// A group of files.
    Group(Box<SourceGroup<'ctx>>),
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
