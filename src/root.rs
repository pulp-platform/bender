// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! This module provides a struct to represent the root package within which the
//! program was executed.

use std::path::{Path, PathBuf};
use package::Package;
use errors::Result;


/// The root repository within which the tool was invoked. This is where the
/// relevant configuration files (Landa.yml, ips_list.yml, src_files.yml) shall
/// be searched.
#[derive(Debug)]
pub struct Root {
	pkg: Package,
}

impl Root {
	/// Create a new root package at the given path.
	pub fn new<P: Into<PathBuf>>(path: P) -> Result<Root> {
		Ok(Root {
			pkg: Package::new(path, None)?,
		})
	}

	/// Return the path to the root package.
	pub fn path(&self) -> &Path {
		self.pkg.path()
	}

	/// Return the path to the scratch directory where the cloned repositories
	/// and other semi-temporary files will be placed.
	pub fn scratch_path(&self) -> PathBuf {
		self.path().join(".landa")
	}

	/// Return the path to the lock file which contains a list of the checked
	/// out revisions of all dependencies.
	pub fn lock_file(&self) -> PathBuf {
		self.path().join("Landa.lock")
	}

	/// Return the Package structure that captures the configuration and
	/// dependencies of the root package.
	pub fn package(&self) -> &Package {
		&self.pkg
	}
}
