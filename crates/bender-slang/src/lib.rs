// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use cxx::{SharedPtr, UniquePtr};

pub use ffi::SlangPrintOpts;

#[cxx::bridge]
mod ffi {
    /// Options for the syntax printer
    #[derive(Clone, Copy)]
    struct SlangPrintOpts {
        include_directives: bool,
        expand_includes: bool,
        expand_macros: bool,
        include_comments: bool,
        squash_newlines: bool,
    }

    unsafe extern "C++" {
        include!("bender-slang/cpp/slang_bridge.h");
        // Include Slang header to define SyntaxTree type for CXX
        include!("slang/syntax/SyntaxTree.h");

        /// Opaque type for the Slang Driver wrapper
        type SlangContext;

        /// Opaque type for the Slang SyntaxTree
        #[namespace = "slang::syntax"]
        type SyntaxTree;

        /// Create a new persistent context (owns the Driver)
        fn new_slang_context() -> UniquePtr<SlangContext>;

        // Methods on SlangContext
        fn add_source(self: Pin<&mut SlangContext>, path: &str);
        fn add_include(self: Pin<&mut SlangContext>, path: &str);
        fn add_define(self: Pin<&mut SlangContext>, def: &str);

        /// Parse all added sources. Returns true on success.
        fn parse(self: Pin<&mut SlangContext>) -> Result<bool>;

        /// Retrieves the number of parsed syntax trees
        fn get_tree_count(self: &SlangContext) -> usize;

        /// Retrieves a shared pointer to a specific syntax tree by index
        fn get_tree(self: &SlangContext, index: usize) -> SharedPtr<SyntaxTree>;

        /// Print a specific tree using the context's SourceManager
        fn print_tree(self: &SlangContext, tree: &SyntaxTree, options: SlangPrintOpts) -> String;
    }
}

/// A persistent Slang session
pub struct SlangSession {
    ctx: UniquePtr<ffi::SlangContext>,
}

impl SlangSession {
    /// Creates a new Slang session
    pub fn new() -> Self {
        Self {
            ctx: ffi::new_slang_context(),
        }
    }

    /// Adds a source file to be parsed
    pub fn add_source(&mut self, path: &str) {
        self.ctx.pin_mut().add_source(path);
    }

    /// Adds an include directory
    pub fn add_include(&mut self, path: &str) {
        self.ctx.pin_mut().add_include(path);
    }

    /// Adds a preprocessor define
    pub fn add_define(&mut self, define: &str) {
        self.ctx.pin_mut().add_define(define);
    }

    /// Parses all added source files into syntax trees
    pub fn parse(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(self.ctx.pin_mut().parse()?)
    }

    /// Returns the parsed syntax trees as a Rust vector
    pub fn get_trees(&self) -> Vec<SharedPtr<ffi::SyntaxTree>> {
        let count = self.ctx.get_tree_count();
        let mut trees = Vec::with_capacity(count);
        for i in 0..count {
            trees.push(self.ctx.get_tree(i));
        }
        trees
    }

    /// Returns an iterator over the parsed syntax trees
    pub fn trees_iter(&self) -> impl Iterator<Item = SharedPtr<ffi::SyntaxTree>> + '_ {
        (0..self.ctx.get_tree_count()).map(|i| self.ctx.get_tree(i))
    }

    /// Prints a syntax tree with given printing options
    pub fn print_tree(&self, tree: &ffi::SyntaxTree, opts: ffi::SlangPrintOpts) -> String {
        self.ctx.print_tree(tree, opts)
    }
}
