// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Error chaining and reporting facilities.

use std;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use owo_colors::Style;

/// Print an error.
#[macro_export]
macro_rules! errorln {
    ($($arg:tt)*) => { $crate::diagnostic!($crate::error::Severity::Error; $($arg)*); }
}

/// Print an informational note.
#[macro_export]
macro_rules! infoln {
    ($($arg:tt)*) => { $crate::diagnostic!($crate::error::Severity::Info; $($arg)*); }
}

/// Format and print stage progress.
#[macro_export]
macro_rules! stageln {
    ($stage_name:expr, $($arg:tt)*) => { $crate::diagnostic!($crate::error::Severity::Stage($stage_name); $($arg)*); }
}

/// Emit a diagnostic message.
#[macro_export]
macro_rules! diagnostic {
    ($severity:expr; $($arg:tt)*) => {
        $crate::diagnostic::Diagnostics::eprintln(&format!("{} {}", $severity, format!($($arg)*)))
    }
}

/// The severity of a diagnostic message.
#[derive(PartialEq, Eq)]
pub enum Severity {
    Info,
    Error,
    Stage(&'static str),
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (severity, style) = match *self {
            Severity::Error => ("Error:", Style::new().red().bold()),
            Severity::Info => ("Info:", Style::new().white().bold()),
            Severity::Stage(name) => (name, Style::new().green().bold()),
        };
        write!(f, "{:>14}", crate::fmt_with_style!(severity, style))
    }
}

/// A result with our custom `Error` type.
pub type Error = miette::Report;
pub type Result<T> = miette::Result<T>;
