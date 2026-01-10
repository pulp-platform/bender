// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Error chaining and reporting facilities.

use std;
use std::fmt;
#[allow(deprecated)]
use std::sync::atomic::{AtomicBool, ATOMIC_BOOL_INIT};
use std::sync::Arc;

#[allow(deprecated)]
pub static ENABLE_DEBUG: AtomicBool = ATOMIC_BOOL_INIT;

/// Print an error.
#[macro_export]
macro_rules! errorln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Error; $($arg)*); }
}

/// Print a warning.
#[macro_export]
macro_rules! warnln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Warning; $($arg)*) }
}

/// Print an informational note.
#[macro_export]
macro_rules! noteln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Note; $($arg)*); }
}

/// Print debug information. Omitted in release builds.
#[macro_export]
#[cfg(debug_assertions)]
macro_rules! debugln {
    ($($arg:tt)*) => {
        if $crate::error::ENABLE_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            diagnostic!($crate::error::Severity::Debug; $($arg)*);
        }
    }
}

/// Print debug information. Omitted in release builds.
#[macro_export]
#[cfg(not(debug_assertions))]
macro_rules! debugln {
    ($fmt:expr $(, $arg:expr)* $(,)?) => { $(let _ = $arg;)* }
    // create an unused binding here so the compiler does not complain
    // about the arguments to debugln! not being used in release builds.
}

/// Emit a diagnostic message.
macro_rules! diagnostic {
    ($severity:expr; $($arg:tt)*) => {
        eprintln!("{} {}", $severity, format!($($arg)*))
    }
}

/// The severity of a diagnostic message.
#[derive(PartialEq, Eq)]
pub enum Severity {
    Debug,
    Note,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (color, prefix) = match *self {
            Severity::Error => ("\x1B[31;1m", "error"),
            Severity::Warning => ("\x1B[33;1m", "warning"),
            Severity::Note => ("\x1B[;1m", "note"),
            Severity::Debug => ("\x1B[34;1m", "debug"),
        };
        write!(f, "{}{}:\x1B[m", color, prefix)
    }
}

/// A result with our custom `Error` type.
pub type Result<T> = std::result::Result<T, Error>;

/// An error message with optional underlying cause.
#[derive(Debug)]
pub struct Error {
    /// A formatted error message.
    pub msg: String,
    /// An optional underlying cause.
    pub cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Create a new error without cause.
    pub fn new<S: Into<String>>(msg: S) -> Error {
        Error {
            msg: msg.into(),
            cause: None,
        }
    }

    /// Create a new error with cause.
    pub fn chain<S, E>(msg: S, cause: E) -> Error
    where
        S: Into<String>,
        E: std::error::Error + Send + Sync + 'static,
    {
        Error {
            msg: msg.into(),
            cause: Some(Arc::new(cause)),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        &self.msg
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self.cause {
            Some(ref b) => Some(b.as_ref()),
            None => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)?;
        if let Some(ref c) = self.cause {
            write!(f, " {}", c)?
        }
        Ok(())
    }
}

impl From<Error> for String {
    fn from(err: Error) -> String {
        format!("{}", err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::chain("Cannot startup runtime.".to_string(), err)
    }
}

/// Format and print stage progress.
#[macro_export]
macro_rules! stageln {
    ($stage:expr, $($arg:tt)*) => {
        $crate::error::println_stage($stage, &format!($($arg)*))
    }
}

/// Print stage progress.
pub fn println_stage(stage: &str, message: &str) {
    eprintln!("\x1B[32;1m{:>12}\x1B[0m {}", stage, message);
}

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use miette::{Diagnostic, ReportHandler};
use owo_colors::OwoColorize;
use thiserror::Error;

static GLOBAL_DIAGNOSTICS: OnceLock<Diagnostics> = OnceLock::new();

/// A diagnostics manager that handles warnings (and errors).
#[derive(Debug)]
pub struct Diagnostics {
    /// A set of suppressed warnings.
    suppressed: HashSet<String>,
    /// Whether all warnings are suppressed.
    all_suppressed: bool,
    /// A set of already emitted warnings.
    /// Requires synchronization as warnings may be emitted from multiple threads.
    emitted: Mutex<HashSet<Warnings>>,
}

impl Diagnostics {
    /// Create a new diagnostics manager.
    pub fn init(suppressed: HashSet<String>) {
        // Set up miette with our custom renderer
        miette::set_hook(Box::new(|_| Box::new(DiagnosticRenderer))).unwrap();
        let diag = Diagnostics {
            all_suppressed: suppressed.contains("all") || suppressed.contains("Wall"),
            suppressed: suppressed,
            emitted: Mutex::new(HashSet::new()),
        };

        GLOBAL_DIAGNOSTICS
            .set(diag)
            .expect("Diagnostics already initialized!");
    }

    /// Get the global diagnostics manager.
    fn get() -> &'static Diagnostics {
        GLOBAL_DIAGNOSTICS
            .get()
            .expect("Diagnostics not initialized!")
    }
}

impl Warnings {
    /// Checks suppression, deduplicates, and emits the warning to stderr.
    pub fn emit(self) {
        let diag = Diagnostics::get();

        // Check whether the command is suppressed
        if let Some(code) = self.code() {
            if diag.all_suppressed || diag.suppressed.contains(&code.to_string()) {
                return;
            }
        }

        // Check whether the warning was already emitted
        let mut emitted = diag.emitted.lock().unwrap();
        if emitted.contains(&self) {
            return;
        }
        emitted.insert(self.clone());
        drop(emitted);

        // Print the warning report (consumes self i.e. the warning)
        eprintln!("{:?}", miette::Report::new(self));
    }
}

pub struct DiagnosticRenderer;

impl ReportHandler for DiagnosticRenderer {
    fn debug(&self, diagnostic: &dyn Diagnostic, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Determine severity and the resulting style
        let (severity, style) = match diagnostic.severity().unwrap_or_default() {
            miette::Severity::Error => ("error", owo_colors::Style::new().red().bold()),
            miette::Severity::Warning => ("warning", owo_colors::Style::new().yellow().bold()),
            miette::Severity::Advice => unimplemented!(),
        };

        // Write the severity prefix
        write!(f, "{}", severity.style(style))?;

        // Writ the code, if any
        if let Some(code) = diagnostic.code() {
            write!(f, "{}", format!("[{}]: ", code).style(style))?;
        }

        // Then, we write the diagnostic message
        write!(f, "{}", diagnostic)?;

        // Below the message, there might be an additional help message
        let branch = " ├─›"; // Branching with arrow
        let corner = " ╰─›"; // Final corner with arrow

        if let Some(help) = diagnostic.help() {
            // Convert to string and split by lines
            let help_str = help.to_string();
            let lines: Vec<&str> = help_str.lines().collect();

            // Print each line with the appropriate help prefix and branching
            for (i, line) in lines.iter().enumerate() {
                // Determine the tree character
                let is_last = i == lines.len() - 1;
                let prefix = if is_last { corner } else { branch };

                // Styled messages (e.g. 'pkg.bold()') will reset the style afterwards,
                // so we need to re-apply dimming after each reset.
                let line = line.replace("\x1b[0m", "\x1b[0m\x1b[2m");

                write!(
                    f,
                    "\n{} {} {}",
                    prefix.dimmed(),
                    "help:".bold(),
                    line.dimmed()
                )?;
            }
        }

        Ok(())
    }
}

/// Bold a package name in diagnostic messages.
macro_rules! pkg {
    ($pkg:expr) => {
        $pkg.bold()
    };
}

/// Underline a path in diagnostic messages.
macro_rules! path {
    ($pkg:expr) => {
        $pkg.underline()
    };
}

/// Italicize a field name in diagnostic messages.
macro_rules! field {
    ($field:expr) => {
        $field.italic()
    };
}

#[derive(Error, Diagnostic, Hash, Eq, PartialEq, Debug, Clone)]
pub enum Warnings {
    #[error(
        "Skipping link to package {} at {} since there is something there",
        pkg!(.0),
        path!(.1.display())
    )]
    #[diagnostic(
        severity(Warning),
        code(W01),
        help("Check the existing file or directory that is preventing the link.")
    )]
    SkippingPackageLink(String, PathBuf),

    #[error("Using config at {} for overrides.", path!(path.display()))]
    #[diagnostic(severity(Warning), code(W02))]
    UsingConfigForOverride { path: PathBuf },

    #[error("Ignoring unknown field {} in package {}.", field!(field), pkg!(pkg))]
    #[diagnostic(
        severity(Warning),
        code(W03),
        help("Check for typos in {} or remove it from the {} manifest.", field!(field), pkg!(pkg))
    )]
    IgnoreUnknownField { field: String, pkg: String },

    #[error("Source group in package {} contains no source files.", pkg!(.0))]
    #[diagnostic(
        severity(Warning),
        code(W04),
        help("Add source files to the source group or remove it from the manifest.")
    )]
    NoFilesInSourceGroup(String),

    #[error("No files matched the global pattern {}.", path!(path))]
    #[diagnostic(severity(Warning), code(W05))]
    NoFilesForGlobalPattern { path: String },

    #[error("Dependency {} in checkout_dir {} is not a git repository. Setting as path dependency.", pkg!(.0), path!(.1.display()))]
    #[diagnostic(
        severity(Warning),
        code(W06),
        help("Use `bender clone` to work on git dependencies.\nRun `bender update --ignore-checkout-dir` to overwrite this at your own risk.")
    )]
    NotAGitDependency(String, PathBuf),

    // TODO(fischeti): Why are there two W07 variants?
    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("SSH key might be missing.")]
    #[diagnostic(
        severity(Warning),
        code(W07),
        help("Please ensure your public ssh key is added to the git server.")
    )]
    SshKeyMaybeMissing,

    // TODO(fischeti): Why are there two W07 variants?
    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("SSH key might be missing.")]
    #[diagnostic(
        severity(Warning),
        code(W07),
        help("Please ensure the url is correct and you have access to the repository.")
    )]
    UrlMaybeIncorrect,

    // TODO(fischeti): This is part of an error, not a warning. Move to Error enum later?
    #[error("Revision {} not found in repository {}.", pkg!(.0), pkg!(.1))]
    #[diagnostic(
        severity(Warning),
        code(W08),
        help("Check that the revision exists in the remote repository or run `bender update`.")
    )]
    RevisionNotFound(String, String),

    #[error("Path dependency {} inside git dependency {} detected. This is currently not fully suppored and your milage may vary.", pkg!(pkg), pkg!(top_pkg))]
    #[diagnostic(severity(Warning), code(W09))]
    PathDepInGitDep { pkg: String, top_pkg: String },

    #[error("There may be issues in the path for {}.", pkg!(.0))]
    #[diagnostic(
        severity(Warning),
        code(W10),
        help("Please check that {} is correct and accessible.", path!(.1.display()))
    )]
    MaybePathIssues(String, PathBuf),

    #[error("Dependency package name {} does not match the package name {} in its manifest.", pkg!(.0), pkg!(.1))]
    #[diagnostic(
        severity(Warning),
        code(W11),
        help("Check that the dependency name in your root manifest matches the name in the {} manifest.", pkg!(.0))
    )]
    DepPkgNameNotMatching(String, String),

    #[error("Manifest for package {} not found at {}.", pkg!(pkg), path!(src))]
    #[diagnostic(severity(Warning), code(W12))]
    ManifestNotFound { pkg: String, src: String },

    #[error("Name issue with package {}. `export_include_dirs` cannot be handled.", pkg!(.0))]
    #[diagnostic(
        severity(Warning),
        code(W13),
        help("Could be related to name missmatch, check `bender update`.")
    )]
    ExportDirNameIssue(String),

    #[error("If `--local` is used, no fetching will be performed.")]
    #[diagnostic(severity(Warning), code(W14))]
    LocalNoFetch,

    #[error("No patch directory found for package {} when trying to apply patches from {} to {}. Skipping patch generation.", pkg!(vendor_pkg), path!(from_prefix.display()), path!(to_prefix.display()))]
    #[diagnostic(severity(Warning), code(W15))]
    NoPatchDir {
        vendor_pkg: String,
        from_prefix: PathBuf,
        to_prefix: PathBuf,
    },
}
