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
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenameStats {
    pub renamed_declarations: u64,
    pub renamed_references: u64,
}

#[cxx::bridge]
mod ffi {
    /// Options for the syntax printer
    #[derive(Clone, Copy)]
    struct SlangPrintOpts {
        expand_macros: bool,
        include_directives: bool,
        include_comments: bool,
        squash_newlines: bool,
    }

    /// A parsed syntax tree bundled with the per-file facts slang reported while parsing it.
    struct ParsedTree {
        /// The parsed syntax tree (possibly partial if `parsed_ok` is false).
        tree: SharedPtr<SyntaxTree>,
        /// The source path as it was handed to `parse_group`.
        path: String,
        /// False if slang reported any error diagnostic for this file.
        parsed_ok: bool,
        /// True if slang emitted a `pragma protect` envelope diag (IEEE-1735 encrypted IP).
        encrypted: bool,
    }

    unsafe extern "C++" {
        include!("bender-slang/cpp/slang_bridge.h");
        include!("slang/syntax/SyntaxTree.h");

        /// Opaque session that owns parse contexts and syntax trees.
        type SlangSession;

        /// Opaque type for the Slang syntax tree.
        #[namespace = "slang::syntax"]
        type SyntaxTree;
        type SyntaxTreeRewriter;

        fn new_slang_session() -> UniquePtr<SlangSession>;

        fn parse_group(
            self: Pin<&mut SlangSession>,
            files: &Vec<String>,
            includes: &Vec<String>,
            defines: &Vec<String>,
        ) -> Result<()>;

        fn all_trees(session: &SlangSession) -> Vec<ParsedTree>;

        fn reachable_trees(session: &SlangSession, tops: &Vec<String>) -> Result<Vec<ParsedTree>>;

        fn resolved_include_paths_for(trees: &Vec<ParsedTree>) -> Vec<String>;

        fn new_syntax_tree_rewriter() -> UniquePtr<SyntaxTreeRewriter>;
        fn set_suffix(self: Pin<&mut SyntaxTreeRewriter>, suffix: &str);
        fn set_excludes(self: Pin<&mut SyntaxTreeRewriter>, excludes: Vec<String>);
        fn rewrite_declarations(
            self: Pin<&mut SyntaxTreeRewriter>,
            tree: SharedPtr<SyntaxTree>,
        ) -> SharedPtr<SyntaxTree>;
        fn set_prefix(self: Pin<&mut SyntaxTreeRewriter>, prefix: &str);
        fn rewrite_references(
            self: Pin<&mut SyntaxTreeRewriter>,
            tree: SharedPtr<SyntaxTree>,
        ) -> SharedPtr<SyntaxTree>;
        fn renamed_declarations(rewriter: &SyntaxTreeRewriter) -> u64;
        fn renamed_references(rewriter: &SyntaxTreeRewriter) -> u64;

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

/// A parsed syntax tree bundled with the per-file facts slang reported while parsing it.
pub struct ParsedTree<'a> {
    /// The parsed syntax tree (possibly partial when `parsed_ok` is false).
    pub tree: SyntaxTree<'a>,
    /// The source path exactly as it was handed to [`SlangSession::parse_group`].
    pub path: String,
    /// False if slang reported any error diagnostic for this file.
    pub parsed_ok: bool,
    /// True if slang emitted a `pragma protect` envelope diagnostic, i.e. the file is
    /// IEEE-1735 encrypted IP rather than a genuinely broken source file.
    pub encrypted: bool,
}

impl<'a> ParsedTree<'a> {
    fn from_ffi(parsed: ffi::ParsedTree) -> Self {
        Self {
            tree: SyntaxTree {
                inner: parsed.tree,
                _session: PhantomData,
            },
            path: parsed.path,
            parsed_ok: parsed.parsed_ok,
            encrypted: parsed.encrypted,
        }
    }
}

pub struct SyntaxTreeRewriter {
    inner: UniquePtr<ffi::SyntaxTreeRewriter>,
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
            include_directives: true,
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
    ) -> Result<()> {
        let files_vec = files.to_vec();
        let includes_vec = normalize_include_dirs(includes)?;
        let defines_vec = defines.to_vec();

        self.inner
            .pin_mut()
            .parse_group(&files_vec, &includes_vec, &defines_vec)
            .map_err(|cause| SlangError::ParseGroup {
                message: cause.to_string(),
            })
    }

    /// Returns every parsed tree in the session, each bundled with its per-file facts (path,
    /// parse success, encryption). The order matches parse order across all `parse_group` calls.
    pub fn all_trees(&self) -> Vec<ParsedTree<'_>> {
        ffi::all_trees(self.inner.as_ref().unwrap())
            .into_iter()
            .map(ParsedTree::from_ffi)
            .collect()
    }

    /// Returns the parsed trees reachable from the given top modules, each bundled with its
    /// per-file facts.
    pub fn reachable_trees(&self, tops: &[String]) -> Result<Vec<ParsedTree<'_>>> {
        let tops = tops.to_vec();
        let trees = ffi::reachable_trees(self.inner.as_ref().unwrap(), &tops).map_err(|cause| {
            SlangError::TrimByTop {
                message: cause.to_string(),
            }
        })?;
        Ok(trees.into_iter().map(ParsedTree::from_ffi).collect())
    }

    /// Returns the canonical filesystem paths of every header that was actually loaded via an
    /// `include directive while parsing the given trees. Useful for figuring out which include
    /// directories were actually consulted.
    pub fn resolved_include_paths(&self, trees: &[ParsedTree]) -> Vec<String> {
        let ffi_trees: Vec<ffi::ParsedTree> = trees
            .iter()
            .map(|t| ffi::ParsedTree {
                tree: t.tree.inner.clone(),
                path: t.path.clone(),
                parsed_ok: t.parsed_ok,
                encrypted: t.encrypted,
            })
            .collect();
        ffi::resolved_include_paths_for(&ffi_trees)
    }
}

impl Default for SlangSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SyntaxTreeRewriter {
    pub fn new() -> Self {
        Self {
            inner: ffi::new_syntax_tree_rewriter(),
        }
    }

    pub fn set_prefix(&mut self, prefix: impl Into<String>) {
        let prefix = prefix.into();
        self.inner.pin_mut().set_prefix(&prefix);
    }

    pub fn set_suffix(&mut self, suffix: impl Into<String>) {
        let suffix = suffix.into();
        self.inner.pin_mut().set_suffix(&suffix);
    }

    pub fn set_excludes(&mut self, excludes: Vec<String>) {
        self.inner.pin_mut().set_excludes(excludes);
    }

    pub fn rewrite_declarations<'a>(&mut self, tree: &SyntaxTree<'a>) -> SyntaxTree<'a> {
        SyntaxTree {
            inner: self
                .inner
                .pin_mut()
                .rewrite_declarations(tree.inner.clone()),
            _session: PhantomData,
        }
    }

    pub fn rewrite_references<'a>(&mut self, tree: &SyntaxTree<'a>) -> SyntaxTree<'a> {
        SyntaxTree {
            inner: self.inner.pin_mut().rewrite_references(tree.inner.clone()),
            _session: PhantomData,
        }
    }

    pub fn stats(&self) -> RenameStats {
        let rewriter = self.inner.as_ref().unwrap();
        RenameStats {
            renamed_declarations: ffi::renamed_declarations(rewriter),
            renamed_references: ffi::renamed_references(rewriter),
        }
    }
}

impl Default for SyntaxTreeRewriter {
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
