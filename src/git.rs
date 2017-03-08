// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! This module provides an abstraction to work with Git repositories.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::process::Command;
use std::io::{Result, Error, ErrorKind};
use std::fmt;
use std::error;
use std::process::Output;


pub struct Git {
	path: PathBuf,
	revs: Arc<Mutex<HashMap<String, String>>>,
}

impl Git {
	pub fn new<P: Into<PathBuf>>(path: P) -> Git {
		Git {
			path: path.into(),
			revs: Arc::new(Mutex::new(HashMap::new())),
		}
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	fn make_command(&self) -> Command {
		let mut cmd = Command::new("git");
		cmd.current_dir(&self.path);
		cmd
	}

	pub fn parse_rev<S: AsRef<str>>(&self, rev: S) -> Result<String> {
		// First attempt to parse the revision as a branch/tag on the origin
		// remote.
		let parsed = unpack_stdout(Command::new("git")
			.current_dir(&self.path)
			.arg("rev-parse")
			.arg("--revs-only")
			.arg(format!("origin/{}", rev.as_ref()))
			.output()?)?;
		if !parsed.is_empty() {
			return Ok(parsed);
		}

		// Since the above failed, try to simply parse the revision as it is,
		// which will expand SHA1 and local branches appropriately.
		let parsed = unpack_stdout(Command::new("git")
			.current_dir(&self.path)
			.arg("rev-parse")
			.arg("--revs-only")
			.arg(rev.as_ref())
			.output()?)?;
		if !parsed.is_empty() {
			return Ok(parsed);
		}

		Err(Error::new(ErrorKind::Other, format!("`{}` is neither a known branch/tag nor a revision", rev.as_ref())))
	}

	pub fn at<'a>(&'a self, rev: &'a str) -> GitRev {
		GitRev {
			git: self,
			rev: rev,
		}
	}
}



fn unpack_stdout(output: Output) -> Result<String> {
	use std;

	// Check whether the parse was successful.
	if !output.status.success() {
		match std::str::from_utf8(&output.stderr) {
			Ok(x) => return Err(Error::new(ErrorKind::Other, GitError::new(x))),
			Err(e) => return Err(Error::new(ErrorKind::Other, e)),
		}
	}

	// Convert the output into a string and return.
	match std::str::from_utf8(&output.stdout) {
		Ok(x) => Ok(x.trim().into()),
		Err(e) => Err(Error::new(ErrorKind::Other, e)),
	}
}



pub struct GitRev<'tgit> {
	git: &'tgit Git,
	rev: &'tgit str,
}

impl<'tgit> GitRev<'tgit> {
	pub fn exists(&self, path: &str) -> Result<bool> {
		Ok(!unpack_stdout(self.git.make_command()
			.arg("ls-tree")
			.arg("--name-only")
			.arg(self.rev)
			.arg(path)
			.output()?)?.is_empty())
	}

	pub fn read_file(&self, path: &str) -> Result<String> {
		unpack_stdout(self.git.make_command()
			.arg("show")
			.arg(format!("{}:{}", self.rev, path))
			.output()?)
	}
}



#[derive(Debug)]
pub struct GitError {
	msg: String,
}

impl GitError {
	pub fn new<S: AsRef<str>>(msg: S) -> GitError {
		GitError {
			msg: msg.as_ref().trim().into(),
		}
	}
}

impl error::Error for GitError {
	fn description(&self) -> &str {
		&self.msg
	}
}

impl fmt::Display for GitError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "git failed with the following error:\n{}", self.msg)
	}
}
