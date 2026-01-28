pub use ffi::SlangPrintOpts;

#[cxx::bridge]
mod ffi {

    /// Options for the syntax printer
    #[derive(Clone)]
    struct SlangPrintOpts {
        /// Whether to include preprocessor directives
        include_directives: bool,
        /// Whether to expand include directives
        expand_includes: bool,
        /// Whether to expand macros
        expand_macros: bool,
        /// Whether to print comments
        include_comments: bool,
        /// Whether to squash newlines
        squash_newlines: bool,
    }

    unsafe extern "C++" {
        include!("bender-slang/cpp/slang_bridge.h");

        fn pickle(
            sources: Vec<String>,
            include_dirs: Vec<String>,
            defines: Vec<String>,
            options: SlangPrintOpts,
        ) -> Result<String>;
    }
}

/// Main interface for Slang bindings
pub struct Slang {
    /// Source files to be pickled
    sources: Vec<String>,
    /// Include directories
    include_dirs: Vec<String>,
    /// Defines
    defines: Vec<String>,
    /// Print options
    print_opts: ffi::SlangPrintOpts,
}

/// Main interface for interfacing with Slang
impl Slang {
    pub fn new() -> Self {
        Slang {
            sources: Vec::new(),
            include_dirs: Vec::new(),
            defines: Vec::new(),
            print_opts: ffi::SlangPrintOpts {
                include_directives: true,
                expand_includes: true,
                expand_macros: true,
                include_comments: true,
                squash_newlines: true,
            },
        }
    }

    /// Adds source files to be pickled.
    pub fn add_sources(&mut self, sources: Vec<String>) {
        self.sources.extend(sources);
    }

    /// Adds source sources to be pickled, returning self for chaining.
    pub fn with_sources(mut self, sources: Vec<String>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// Adds include directories.
    pub fn add_include_dirs(&mut self, dirs: Vec<String>) {
        self.include_dirs.extend(dirs);
    }

    /// Adds include directories, returning self for chaining.
    pub fn with_include_dirs(mut self, dirs: Vec<String>) -> Self {
        self.include_dirs.extend(dirs);
        self
    }

    /// Adds defines.
    pub fn add_defines(&mut self, defines: Vec<String>) {
        self.defines.extend(defines);
    }

    /// Adds defines, returning self for chaining.
    pub fn with_defines(mut self, defines: Vec<String>) -> Self {
        self.defines.extend(defines);
        self
    }

    /// Sets print options.
    pub fn set_print_options(&mut self, print_opts: ffi::SlangPrintOpts) {
        self.print_opts = print_opts;
    }

    /// Sets print options, returning self for chaining.
    pub fn with_print_options(mut self, print_opts: ffi::SlangPrintOpts) -> Self {
        self.print_opts = print_opts;
        self
    }

    /// Pickles files based on the provided configuration.
    /// Returns the pickled content or an error if parsing/processing failed.
    pub fn pickle(&self) -> Result<String, Box<dyn std::error::Error>> {
        // call the C++ function; errors are propagated as Rust Results
        let result = ffi::pickle(
            self.sources.clone(),
            self.include_dirs.clone(),
            self.defines.clone(),
            self.print_opts.clone(),
        )?;
        Ok(result)
    }
}
