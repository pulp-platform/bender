// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use cxx::{SharedPtr, UniquePtr};

pub use ffi::SlangPrintOpts;

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

        /// Create a new persistent context
        fn new_slang_context() -> UniquePtr<SlangContext>;

        /// Set the include directories
        fn set_includes(self: Pin<&mut SlangContext>, includes: &Vec<String>);
        /// Set the preprocessor defines
        fn set_defines(self: Pin<&mut SlangContext>, defines: &Vec<String>);

        /// Parse all added sources. Returns a syntax tree on success, or an error message on failure.
        fn parse_file(self: Pin<&mut SlangContext>, path: &str) -> Result<SharedPtr<SyntaxTree>>;

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
    pub fn rename(&self, prefix: Option<&str>, suffix: Option<&str>, excludes: &Vec<String>) -> Self {
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
    pub fn parse(
        &mut self,
        path: &str,
    ) -> Result<SyntaxTree, Box<dyn std::error::Error>> {
        Ok(SyntaxTree {
            inner: self.inner.pin_mut().parse_file(path)?,
        })
    }
}

/// Creates a new Slang session
pub fn new_session() -> SlangContext {
    SlangContext::new()
}
