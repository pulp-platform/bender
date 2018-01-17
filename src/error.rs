// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Error chaining and reporting facilities.

use std;
use std::fmt;

/// Print an error.
#[macro_export]
macro_rules! errorln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Error; $($arg)*) }
}

/// Print a warning.
#[macro_export]
macro_rules! warnln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Warning; $($arg)*) }
}

/// Print an informational note.
#[macro_export]
macro_rules! noteln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Note; $($arg)*) }
}

/// Print debug information. Omitted in release builds.
#[macro_export]
#[cfg(debug_assertions)]
macro_rules! debugln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Debug; $($arg)*) }
}

/// Print debug information. Omitted in release builds.
#[macro_export]
#[cfg(not(debug_assertions))]
macro_rules! debugln {
    ($($arg:tt)*) => {}
}

/// Emit a diagnostic message.
macro_rules! diagnostic {
    ($severity:expr; $($arg:tt)*) => {
        eprint!("{} ", $severity);
        eprintln!($($arg)*);
    }
}

/// The severity of a diagnostic message.
pub enum Severity {
    Debug,
    Note,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (color, prefix) = match *self {
            Severity::Error   => ("\x1B[31;1m", "error"),
            Severity::Warning => ("\x1B[33;1m", "warning"),
            Severity::Note    => ("\x1B[;1m", "note"),
            Severity::Debug   => ("\x1B[34;1m", "debug"),
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
    pub cause: Option<Box<std::error::Error>>,
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
    pub fn chain<S,E>(msg: S, cause: E) -> Error
        where S: Into<String>, E: Into<Box<std::error::Error>>
    {
        Error {
            msg: msg.into(),
            cause: Some(cause.into()),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        &self.msg
    }

    fn cause(&self) -> Option<&std::error::Error> {
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
