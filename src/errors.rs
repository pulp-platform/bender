// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! This module provides error reporting and chaining facilities.

use std;
use std::path::PathBuf;
use std::fmt;
use yaml_rust;


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



pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error {
	error: Box<std::error::Error>,
	chain: Option<Box<Error>>,
}

impl Error {
	pub fn new<T: Into<Box<std::error::Error>>>(error: T) -> Error {
		Error {
			error: error.into(),
			chain: None,
		}
	}

	pub fn chain<T: Into<Box<std::error::Error>>>(self, error: T) -> Error {
		Error {
			error: error.into(),
			chain: Some(Box::new(self)),
		}
	}
}

// impl std::error::Error for Error {
// 	fn description(&self) -> &str {
// 		match self.kind {
// 			ErrorKind::Message(ref msg) => msg,
// 			ErrorKind::File(ref path) => path.to_str().unwrap(),
// 			ErrorKind::Other(ref error) => error.description(),
// 		}
// 	}

// 	fn cause(&self) -> Option<&std::error::Error> {
// 		match self.error {
// 			Some(ref e) => Some(&*e),
// 			None => None,
// 		}
// 	}
// }

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.error)?;
		if let Some(ref chain) = self.chain {
			write!(f, ": {}", chain)?;
		}
		Ok(())
	}
}

impl<T: Into<Box<std::error::Error>>> From<T> for Error {
	fn from(error: T) -> Error {
		Error::new(error)
	}
}

// impl<T: Into<String>> From<T> for ErrorKind {
// 	fn from(msg: T) -> ErrorKind {
// 		ErrorKind::Message(msg.into())
// 	}
// }

// impl<T: Into<Box<std::error::Error>>> From<T> for ErrorKind {
// 	fn from(error: T) -> ErrorKind {
// 		ErrorKind::Other(error.into())
// 	}
// }

// impl From<Box<std::error::Error>> for Error {
// 	fn from(error: Box<std::error::Error>) -> Error {
// 		Error::new(ErrorKind::Other(error))
// 	}
// }

// impl From<std::io::Error> for Error {
// 	fn from(error: std::io::Error) -> Error {
// 		Error::new(error.into())
// 	}
// }


// impl Error {
// 	pub fn chain<T>(self, inner: T) -> Error where T: Into<Box<std::error::Error>> {
// 		Error {
// 			inner: Some(inner.into()),
// 			..self
// 		}
// 	}
// }

// impl<T: Into<String>> From<T> for Error {
// 	fn from(msg: T) -> Error {
// 		Error::Message(msg.into(), None)
// 	}
// }

// impl From<std::io::Error> for Error {
// 	fn from(error: std::io::Error) -> Error {
// 		Error::Other(Box::new(error))
// 	}
// }

// impl From<yaml_rust::ScanError> for Error {
// 	fn from(error: yaml_rust::ScanError) -> Error {
// 		Error::Other(Box::new(error))
// 	}
// }
