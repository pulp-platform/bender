// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use cxx::{SharedPtr, UniquePtr};
use thiserror::Error;

pub use ffi::SlangPrintOpts;

pub type Result<T> = std::result::Result<T, SlangError>;

#[derive(Debug, Error)]
pub enum SlangError {
    #[error("Failed to parse file: {message}")]
    Parse { message: String },
    #[error("Failed to parse files: {message}")]
    ParseFiles { message: String },
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
        // Include Slang header to define SyntaxTree type for CXX
        include!("slang/syntax/SyntaxTree.h");

        /// Opaque type for the Slang Context
        type SlangContext;

        /// Opaque type for the Slang SyntaxTree
        #[namespace = "slang::syntax"]
        type SyntaxTree;
        /// Opaque type for a batch of parsed syntax trees.
        type SyntaxTrees;

        /// Create a new persistent context
        fn new_slang_context() -> UniquePtr<SlangContext>;

        /// Set the include directories
        fn set_includes(self: Pin<&mut SlangContext>, includes: &Vec<String>);
        /// Set the preprocessor defines
        fn set_defines(self: Pin<&mut SlangContext>, defines: &Vec<String>);

        /// Parse all added sources. Returns a syntax tree on success, or an error message on failure.
        fn parse_file(self: Pin<&mut SlangContext>, path: &str) -> Result<SharedPtr<SyntaxTree>>;
        /// Parse multiple source files and return a batch of syntax trees.
        fn parse_files(
            self: Pin<&mut SlangContext>,
            paths: &Vec<String>,
        ) -> Result<UniquePtr<SyntaxTrees>>;
        /// Create an empty syntax-tree batch.
        fn new_syntax_trees() -> UniquePtr<SyntaxTrees>;
        /// Appends trees from src into dst.
        fn append_trees(dst: Pin<&mut SyntaxTrees>, src: &SyntaxTrees);
        /// Computes reachable tree indices from the provided top names.
        fn reachable_tree_indices(trees: &SyntaxTrees, tops: &Vec<String>) -> Result<Vec<u32>>;
        /// Returns the number of trees in the batch.
        fn tree_count(trees: &SyntaxTrees) -> usize;
        /// Returns tree at index from the batch.
        fn tree_at(trees: &SyntaxTrees, index: usize) -> Result<SharedPtr<SyntaxTree>>;

        /// Rename names in the syntax tree with a given prefix and suffix
        fn rename(
            tree: SharedPtr<SyntaxTree>,
            prefix: &str,
            suffix: &str,
            excludes: &Vec<String>,
        ) -> SharedPtr<SyntaxTree>;

        /// Print a specific tree
        fn print_tree(tree: SharedPtr<SyntaxTree>, options: SlangPrintOpts) -> String;

        /// Dump the syntax tree as JSON for debugging purposes
        fn dump_tree_json(tree: SharedPtr<SyntaxTree>) -> String;
    }
}

/// Wrapper around an opaque Slang syntax tree.
pub struct SyntaxTree {
    inner: SharedPtr<ffi::SyntaxTree>,
}

impl Clone for SyntaxTree {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl SyntaxTree {
    /// Renames all names in the syntax tree with the given prefix and suffix
    pub fn rename(
        &self,
        prefix: Option<&str>,
        suffix: Option<&str>,
        excludes: &Vec<String>,
    ) -> Self {
        if prefix.is_none() && suffix.is_none() {
            return self.clone();
        }
        Self {
            inner: ffi::rename(
                self.inner.clone(),
                prefix.unwrap_or(""),
                suffix.unwrap_or(""),
                excludes,
            ),
        }
    }

    /// Displays the syntax tree as a string with the given options
    pub fn display(&self, options: SlangPrintOpts) -> String {
        ffi::print_tree(self.inner.clone(), options)
    }

    pub fn as_debug(&self) -> String {
        ffi::dump_tree_json(self.inner.clone())
    }
}

impl std::fmt::Display for SyntaxTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let options = SlangPrintOpts {
            expand_macros: false,
            include_comments: true,
            squash_newlines: false,
        };
        f.write_str(&self.display(options))
    }
}

impl std::fmt::Debug for SyntaxTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_debug())
    }
}

/// Wrapper around an opaque Slang context.
pub struct SlangContext {
    inner: UniquePtr<ffi::SlangContext>,
}

/// Wrapper around an opaque batch of syntax trees.
pub struct SyntaxTrees {
    inner: UniquePtr<ffi::SyntaxTrees>,
}

impl SyntaxTrees {
    /// Creates an empty syntax-tree batch.
    pub fn new() -> Self {
        Self {
            inner: ffi::new_syntax_trees(),
        }
    }

    /// Appends all trees from src into self.
    pub fn append_trees(&mut self, src: &SyntaxTrees) {
        ffi::append_trees(
            self.inner.pin_mut(),
            src.inner
                .as_ref()
                .expect("syntax trees pointer must be valid"),
        );
    }

    /// Returns tree count in this batch.
    pub fn len(&self) -> usize {
        ffi::tree_count(
            self.inner
                .as_ref()
                .expect("syntax trees pointer must be valid"),
        )
    }

    /// Returns true if the batch contains no trees.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns indices reachable from top names.
    pub fn reachable_indices(&self, tops: &Vec<String>) -> Result<Vec<usize>> {
        let indices = ffi::reachable_tree_indices(
            self.inner
                .as_ref()
                .expect("syntax trees pointer must be valid"),
            tops,
        )
        .map_err(|cause| SlangError::TrimByTop {
            message: cause.to_string(),
        })?;
        Ok(indices.into_iter().map(|i| i as usize).collect())
    }

    /// Returns a tree at the provided index.
    pub fn tree_at(&self, index: usize) -> Result<SyntaxTree> {
        Ok(SyntaxTree {
            inner: ffi::tree_at(
                self.inner
                    .as_ref()
                    .expect("syntax trees pointer must be valid"),
                index,
            )
            .map_err(|cause| SlangError::TreeAccess {
                message: cause.to_string(),
            })?,
        })
    }
}

impl SlangContext {
    /// Creates a new Slang session.
    pub fn new() -> Self {
        Self {
            inner: ffi::new_slang_context(),
        }
    }

    /// Sets the include directories.
    pub fn set_includes(&mut self, includes: &Vec<String>) -> &mut Self {
        self.inner.pin_mut().set_includes(includes);
        self
    }

    /// Sets the preprocessor defines.
    pub fn set_defines(&mut self, defines: &Vec<String>) -> &mut Self {
        self.inner.pin_mut().set_defines(defines);
        self
    }

    /// Parses a source file and returns the syntax tree.
    pub fn parse(&mut self, path: &str) -> Result<SyntaxTree> {
        Ok(SyntaxTree {
            inner: self
                .inner
                .pin_mut()
                .parse_file(path)
                .map_err(|cause| SlangError::Parse {
                    message: cause.to_string(),
                })?,
        })
    }

    /// Parses multiple source files and returns a batch of syntax trees.
    pub fn parse_files(&mut self, paths: &Vec<String>) -> Result<SyntaxTrees> {
        Ok(SyntaxTrees {
            inner: self.inner.pin_mut().parse_files(paths).map_err(|cause| {
                SlangError::ParseFiles {
                    message: cause.to_string(),
                }
            })?,
        })
    }
}

/// Creates a new Slang session
pub fn new_session() -> SlangContext {
    SlangContext::new()
}
