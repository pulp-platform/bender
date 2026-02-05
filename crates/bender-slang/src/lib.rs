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
        fn rename(tree: SharedPtr<SyntaxTree>, prefix: &str, suffix: &str)
        -> SharedPtr<SyntaxTree>;

        /// Print a specific tree
        fn print_tree(tree: SharedPtr<SyntaxTree>, options: SlangPrintOpts) -> String;
    }
}

/// Extension trait for SyntaxTree
pub trait SyntaxTreeExt {
    fn rename(&self, prefix: Option<&str>, suffix: Option<&str>) -> Self;
    fn display(&self, options: SlangPrintOpts) -> String;
}

impl SyntaxTreeExt for SharedPtr<ffi::SyntaxTree> {
    /// Renames all names in the syntax tree with the given prefix and suffix
    fn rename(&self, prefix: Option<&str>, suffix: Option<&str>) -> Self {
        if prefix.is_none() && suffix.is_none() {
            return self.clone();
        }
        ffi::rename(self.clone(), prefix.unwrap_or(""), suffix.unwrap_or(""))
    }

    /// Displays the syntax tree as a string with the given options
    fn display(&self, options: SlangPrintOpts) -> String {
        ffi::print_tree(self.clone(), options)
    }
}

/// Extension trait for SlangContext
pub trait SlangContextExt {
    fn set_includes(self, includes: &Vec<String>) -> Self;
    fn set_defines(self, defines: &Vec<String>) -> Self;
    fn parse(
        &mut self,
        path: &str,
    ) -> Result<SharedPtr<ffi::SyntaxTree>, Box<dyn std::error::Error>>;
}

impl SlangContextExt for UniquePtr<ffi::SlangContext> {
    /// Sets the include directories
    fn set_includes(mut self, includes: &Vec<String>) -> Self {
        self.pin_mut().set_includes(&includes);
        self
    }

    /// Sets the preprocessor defines
    fn set_defines(mut self, defines: &Vec<String>) -> Self {
        self.pin_mut().set_defines(&defines);
        self
    }

    /// Parses a source file and returns the syntax tree
    fn parse(
        &mut self,
        path: &str,
    ) -> Result<SharedPtr<ffi::SyntaxTree>, Box<dyn std::error::Error>> {
        Ok(self.pin_mut().parse_file(path)?)
    }
}

/// Creates a new Slang session
pub fn new_session() -> UniquePtr<ffi::SlangContext> {
    ffi::new_slang_context()
}
