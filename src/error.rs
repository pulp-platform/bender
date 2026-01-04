// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Error chaining and reporting facilities.

use std::fmt;
use std::path::PathBuf;
#[allow(deprecated)]
use std::sync::atomic::{AtomicBool, ATOMIC_BOOL_INIT};
use std::sync::{Arc, RwLock};

use console::style;
use indexmap::IndexSet;
use indicatif::MultiProgress;

#[allow(deprecated)]
pub static ENABLE_DEBUG: AtomicBool = ATOMIC_BOOL_INIT;

/// A global hook for the progress bar
pub static GLOBAL_MULTI_PROGRESS: RwLock<Option<MultiProgress>> = RwLock::new(None);

/// Helper function to print diagnostics safely without messing up progress bars.
pub fn print_diagnostic(severity: Severity, msg: &str) {
    let text = format!("{} {}", severity, msg);

    // Try to acquire read access to the global progress bar
    if let Ok(guard) = GLOBAL_MULTI_PROGRESS.read() {
        if let Some(mp) = &*guard {
            // SUSPEND: Hides progress bars, prints the message, then redraws bars.
            mp.suspend(|| {
                eprintln!("{}", text);
            });
            return;
        }
    }

    // Fallback: Just print if no bar is registered or lock is poisoned
    eprintln!("{}", text);
}

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
macro_rules! infoln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Info; $($arg)*); }
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

/// Format and print stage progress.
#[macro_export]
macro_rules! stageln {
    ($stage_name:expr, $($arg:tt)*) => { diagnostic!($crate::error::Severity::Stage($stage_name); $($arg)*); }
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
        $crate::error::print_diagnostic($severity, &format!($($arg)*))
    }
}

/// The severity of a diagnostic message.
#[derive(PartialEq, Eq)]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Stage(&'static str),
}

/// Style a message in green bold.
#[macro_export]
macro_rules! green_bold {
    ($arg:expr) => {
        console::style($arg).green().bold()
    };
}

/// Style a message in green bold.
#[macro_export]
macro_rules! red_bold {
    ($arg:expr) => {
        console::style($arg).red().bold()
    };
}

/// Style a message in dimmed text.
#[macro_export]
macro_rules! dim {
    ($arg:expr) => {
        console::style($arg).dim()
    };
}

/// Style a message in bold text.
#[macro_export]
macro_rules! bold {
    ($arg:expr) => {
        console::style($arg).bold()
    };
}

/// Style a message with underlined text.
#[macro_export]
macro_rules! underline {
    ($arg:expr) => {
        console::style($arg).underlined()
    };
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let styled_str = match *self {
            Severity::Error => style("Error:").red().bold(),
            Severity::Warning => style("Warning:").yellow().bold(),
            Severity::Info => style("Info:").white().bold(),
            Severity::Debug => style("Debug:").blue().bold(),
            Severity::Stage(name) => style(name).green().bold(),
        };
        write!(f, "  {}", styled_str)
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

#[repr(i32)]
pub enum Warnings {
    SkipPackageLink { pkg_name: String, path: PathBuf } = 1,
    UsingConfigForOverrides { path: PathBuf } = 2,
    _NumWarnings,
}

impl Warnings {
    /// Get the warning code as an integer, e.g., 1.
    fn code(&self) -> i32 {
        // From rust docs:
        // If the enumeration specifies a primitive representation,
        // then the discriminant may be reliably accessed via unsafe pointer casting:
        // https://doc.rust-lang.org/reference/items/enumerations.html#r-items.enum.discriminant.access-memory
        // Rust stores the discrimanant as the same type as the representation, which
        // can be accesed by casting the enum reference to a pointer of that type.
        unsafe { *(self as *const Self as *const i32) }
    }

    /// Get the warning code as a string, e.g., "W01".
    fn code_str(&self) -> String {
        format!("W{:02}", self.code())
    }
}

/// A diagnostics handler for warnings and errors.
pub struct Diagnostics {
    /// A set of suppressed warning codes.
    suppress_warnings: IndexSet<i32>,
}

impl Diagnostics {
    pub fn new(suppress_warnings: IndexSet<String>) -> Diagnostics {
        // Build the set of suppressed warning codes, either all...
        if suppress_warnings.contains("all") || suppress_warnings.contains("Wall") {
            return Diagnostics {
                suppress_warnings: (1..=Warnings::_NumWarnings.code()).collect(),
            };
        }
        // ...or only specific ones.
        let suppress_warnings = suppress_warnings
            .iter()
            .filter_map(|s| s.strip_prefix('W'))
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();

        Diagnostics { suppress_warnings }
    }

    /// Emit a warning if it is not suppressed.
    pub fn emit(&self, warning: Warnings) {
        if !self.suppress_warnings.contains(&warning.code()) {
            eprintln!("{}", warning);
        }
    }

    /// Emit a warning if
    pub fn emit_if(&self, condition: bool, warning: Warnings) {
        if condition {
            self.emit(warning);
        }
    }
}

impl fmt::Display for Warnings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Helper macro to write a warning message, with given code and format args.
        macro_rules! warn {
            ($($arg:tt)*) => {{
                let prefix = console::style(format!("  Warning[{}]", self.code_str())).yellow().bold();
                write!(f, "{}: ", prefix)?;
                write!(f, $($arg)*)
            }};
        }
        match self {
            Warnings::SkipPackageLink { pkg_name, path } => {
                warn!(
                    "Skipping link to package {} at path {}, which does not exist.",
                    bold!(pkg_name),
                    underline!(path.display())
                )
            }
            _ => Ok(()),
        }
    }
}
