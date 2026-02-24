// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use std::marker::PhantomData;

use cxx::{SharedPtr, UniquePtr};
use thiserror::Error;

pub use ffi::SlangPrintOpts;

pub type Result<T> = std::result::Result<T, SlangError>;

#[derive(Debug, Error)]
pub enum SlangError {
    #[error("Failed to parse source group: {message}")]
    ParseGroup { message: String },
    #[error("Failed to trim files by top modules: {message}")]
    TrimByTop { message: String },
    #[error("Failed to access parsed syntax tree: {message}")]
    TreeAccess { message: String },
}

#[cxx::bridge]
mod ffi {
    /// Options for the syntax printer
    #[derive(Clone, Copy)]
    struct SlangPrintOpts {
        expand_macros: bool,
        include_comments: bool,
        squash_newlines: bool,
    }

    unsafe extern "C++" {
        include!("bender-slang/cpp/slang_bridge.h");
        include!("slang/syntax/SyntaxTree.h");

        /// Opaque session that owns parse contexts and syntax trees.
        type SlangSession;

        /// Opaque type for the Slang syntax tree.
        #[namespace = "slang::syntax"]
        type SyntaxTree;

        fn new_slang_session() -> UniquePtr<SlangSession>;

        fn parse_group(
            self: Pin<&mut SlangSession>,
            files: &Vec<String>,
            includes: &Vec<String>,
            defines: &Vec<String>,
        ) -> Result<()>;

        fn reachable_tree_indices(session: &SlangSession, tops: &Vec<String>) -> Result<Vec<u32>>;

        fn tree_count(session: &SlangSession) -> usize;

        fn tree_at(session: &SlangSession, index: usize) -> Result<SharedPtr<SyntaxTree>>;

        fn rename(
            tree: SharedPtr<SyntaxTree>,
            prefix: &str,
            suffix: &str,
            excludes: &Vec<String>,
        ) -> SharedPtr<SyntaxTree>;

        fn print_tree(tree: SharedPtr<SyntaxTree>, options: SlangPrintOpts) -> String;

        fn dump_tree_json(tree: SharedPtr<SyntaxTree>) -> String;
    }
}

/// Public owner for all parsed trees and parse contexts.
pub struct SlangSession {
    inner: UniquePtr<ffi::SlangSession>,
}

/// Borrowed syntax-tree handle tied to the owning session lifetime.
pub struct SyntaxTree<'a> {
    inner: SharedPtr<ffi::SyntaxTree>,
    _session: PhantomData<&'a SlangSession>,
}

impl<'a> Clone for SyntaxTree<'a> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _session: PhantomData,
        }
    }
}

impl<'a> SyntaxTree<'a> {
    /// Renames all names in the syntax tree with the given prefix and suffix.
    pub fn rename(&self, prefix: Option<&str>, suffix: Option<&str>, excludes: &[String]) -> Self {
        if prefix.is_none() && suffix.is_none() {
            return self.clone();
        }
        let excludes = excludes.to_vec();
        Self {
            inner: ffi::rename(
                self.inner.clone(),
                prefix.unwrap_or(""),
                suffix.unwrap_or(""),
                &excludes,
            ),
            _session: PhantomData,
        }
    }

    /// Displays the syntax tree as a string with the given options.
    pub fn display(&self, options: SlangPrintOpts) -> String {
        ffi::print_tree(self.inner.clone(), options)
    }

    /// Dumps the syntax tree as JSON for debugging purposes.
    pub fn as_debug(&self) -> String {
        ffi::dump_tree_json(self.inner.clone())
    }
}

impl std::fmt::Display for SyntaxTree<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let options = SlangPrintOpts {
            expand_macros: false,
            include_comments: true,
            squash_newlines: false,
        };
        f.write_str(&self.display(options))
    }
}

impl std::fmt::Debug for SyntaxTree<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_debug())
    }
}

impl SlangSession {
    pub fn new() -> Self {
        Self {
            inner: ffi::new_slang_session(),
        }
    }

    /// Parses one source group with scoped include directories and defines.
    pub fn parse_group(
        &mut self,
        files: &[String],
        includes: &[String],
        defines: &[String],
    ) -> Result<Vec<usize>> {
        let files_vec = files.to_vec();
        let includes_vec = normalize_include_dirs(includes)?;
        let defines_vec = defines.to_vec();

        let start = self.tree_count();
        self.inner
            .pin_mut()
            .parse_group(&files_vec, &includes_vec, &defines_vec)
            .map_err(|cause| SlangError::ParseGroup {
                message: cause.to_string(),
            })?;

        let end = self.tree_count();
        Ok((start..end).collect())
    }

    /// Returns the total number of parsed syntax trees in the session.
    pub fn tree_count(&self) -> usize {
        ffi::tree_count(self.inner.as_ref().unwrap())
    }

    /// Returns all parsed syntax trees in the session.
    pub fn all_trees(&self) -> Result<Vec<SyntaxTree<'_>>> {
        let count = self.tree_count();
        let mut out = Vec::with_capacity(count);
        for idx in 0..count {
            out.push(self.tree(idx)?);
        }
        Ok(out)
    }

    /// Returns the indices of syntax trees reachable from the given top modules.
    pub fn reachable_indices(&self, tops: &[String]) -> Result<Vec<usize>> {
        let tops = tops.to_vec();
        let indices =
            ffi::reachable_tree_indices(self.inner.as_ref().unwrap(), &tops).map_err(|cause| {
                SlangError::TrimByTop {
                    message: cause.to_string(),
                }
            })?;
        Ok(indices.into_iter().map(|i| i as usize).collect())
    }

    /// Returns syntax trees reachable from the given top modules.
    pub fn reachable_trees(&self, tops: &[String]) -> Result<Vec<SyntaxTree<'_>>> {
        let indices = self.reachable_indices(tops)?;
        let mut out = Vec::with_capacity(indices.len());
        for idx in indices {
            out.push(self.tree(idx)?);
        }
        Ok(out)
    }

    /// Returns a handle to the syntax tree at the given index.
    pub fn tree(&self, index: usize) -> Result<SyntaxTree<'_>> {
        Ok(SyntaxTree {
            inner: ffi::tree_at(self.inner.as_ref().unwrap(), index).map_err(|cause| {
                SlangError::TreeAccess {
                    message: cause.to_string(),
                }
            })?,
            _session: PhantomData,
        })
    }
}

impl Default for SlangSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn normalize_include_dirs(includes: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(includes.len());
    for include in includes {
        let canonical = dunce::canonicalize(include).map_err(|cause| SlangError::ParseGroup {
            message: format!(
                "Failed to canonicalize include directory '{}': {}",
                include, cause
            ),
        })?;
        out.push(canonical.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[cfg(unix)]
fn normalize_include_dirs(includes: &[String]) -> Result<Vec<String>> {
    Ok(includes.to_vec())
}
