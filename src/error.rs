// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Error chaining and reporting facilities.

use std;
use std::fmt;
use std::sync::atomic::AtomicBool;

use owo_colors::Style;

pub static ENABLE_DEBUG: AtomicBool = AtomicBool::new(false);

/// Print an error.
#[macro_export]
macro_rules! errorln {
    ($($arg:tt)*) => { diagnostic!($crate::error::Severity::Error; $($arg)*); }
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
        $crate::diagnostic::Diagnostics::eprintln(&format!("{} {}", $severity, format!($($arg)*)))
    }
}

/// The severity of a diagnostic message.
#[derive(PartialEq, Eq)]
pub enum Severity {
    Debug,
    Info,
    Error,
    Stage(&'static str),
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (severity, style) = match *self {
            Severity::Error => ("Error:", Style::new().red().bold()),
            Severity::Info => ("Info:", Style::new().white().bold()),
            Severity::Debug => ("Debug:", Style::new().blue().bold()),
            Severity::Stage(name) => (name, Style::new().green().bold()),
        };
        write!(f, "{:>14}", crate::fmt_with_style!(severity, style))
    }
}
