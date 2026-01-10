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

use std::cell::RefCell;
use std::collections::HashSet;

use miette::Diagnostic;
use thiserror::Error;

/// A diagnostics manager that handles warnings (and errors).
pub struct Diagnostics {
    /// A set of suppressed warnings.
    suppressed: HashSet<String>,
    /// Whether all warnings are suppressed.
    all_suppressed: bool,
    /// A set of already emitted warnings.
    emitted: RefCell<HashSet<Warnings>>,
}

impl Diagnostics {
    /// Create a new diagnostics manager.
    pub fn new(suppressed: HashSet<String>) -> Diagnostics {
        Diagnostics {
            all_suppressed: suppressed.contains("all") || suppressed.contains("Wall"),
            suppressed: suppressed,
            emitted: RefCell::new(HashSet::new()),
        }
    }

    /// Emit a warning if it is not suppressed or already emitted.
    pub fn emit(&self, warning: Warnings) {
        // Extract the code (e.g., "W07") automatically from the derived implementation
        let code = warning.code().map(|c| c.to_string());

        // Check whether the command is suppressed
        if let Some(code) = &code {
            if self.all_suppressed || self.suppressed.contains(code) {
                return;
            }
        }

        // Check whether the warning was already emitted
        let mut emitted = self.emitted.borrow_mut();
        if emitted.contains(&warning) {
            return;
        }

        // Record the emitted warning and print it
        emitted.insert(warning.clone());
        eprintln!("{:?}", miette::Report::new(warning));
    }
}

#[derive(Error, Diagnostic, Hash, Eq, PartialEq, Debug, Clone)]
pub enum Warnings {
    #[error("This is warning 1")]
    #[diagnostic(
        severity(Warning),
        code(W01),
        help("Consider checking the configuration.")
    )]
    Warning1,
    #[error("This is warning 2")]
    #[diagnostic(
        severity(Warning),
        code(W02),
        help("Consider updating your dependencies.")
    )]
    Warning2,
}
