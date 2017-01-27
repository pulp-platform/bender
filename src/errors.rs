// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>
//
// Copyright (C) 2017 ETH Zurich
// All rights reserved.

#[macro_export]
macro_rules! print_error {
	($($arg:expr),*) => { ::errors::print_diagnostic(::errors::Severity::Error, &format!($($arg),*)) }
}

#[macro_export]
macro_rules! print_warning {
	($($arg:expr),*) => { ::errors::print_diagnostic(::errors::Severity::Warning, &format!($($arg),*)) }
}

#[macro_export]
macro_rules! print_note {
	($($arg:expr),*) => { ::errors::print_diagnostic(::errors::Severity::Note, &format!($($arg),*)) }
}


pub enum Severity {
	Note,
	Warning,
	Error,
}


pub fn print_diagnostic(severity: Severity, msg: &str) {
	let (color, prefix) = match severity {
		Severity::Error   => ("\x1B[31;1m", "error"),
		Severity::Warning => ("\x1B[33;1m", "warning"),
		Severity::Note    => ("\x1B[34;1m", "note"),
	};
	println!("{}{}:\x1B[m {}", color, prefix, msg);
}
