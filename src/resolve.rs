// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! This module provides the means to recursively resolve the dependencies of a
//! package.

use package::*;
use semver::VersionReq;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;


#[derive(Debug)]
pub struct ResolvedPackage {
}

#[derive(Debug)]
pub struct ResolveConfig {
}


pub fn resolve(package: &Package, config: &ResolveConfig) -> Result<ResolvedPackage, ()> {
	let mut r = Resolver::new(package.path());
	r.resolve_package(package, config);
	Ok(ResolvedPackage {})
}


pub struct Resolver {
	path: PathBuf,
	local_pkgs: HashMap<String, PathBuf>,
}

impl Resolver {
	fn new<P: Into<PathBuf>>(path: P) -> Resolver {
		Resolver {
			path: path.into(),
			local_pkgs: HashMap::new(),
		}
	}

	fn resolve_package(&mut self, package: &Package, config: &ResolveConfig) {
		// Ensure the scratch directory is available.
		let scratch_dir = Path::new(&self.path).join(".landa");
		if !scratch_dir.is_dir() {
			use std::fs::create_dir;
			println!("creating {}", scratch_dir.to_str().unwrap());
			create_dir(&scratch_dir).unwrap();
		}

		// Make sure that for every dependency we have a local package is
		// available. For dependencies with a path source, this is trivial since
		// the directory is already local. For git sources, clone the repository
		// into the .landa directory.
		let mut any_failed = false;
		for dep in package.deps() {
			match *dep.source() {
				DepSource::Git(ref url) => {
					if !self.local_pkgs.contains_key(dep.name()) {
						let local = scratch_dir.join(dep.name());
						if !local.is_dir() {
							if self.clone_url(url, &local).is_err() {
								any_failed = true;
							}
						}
						self.local_pkgs.insert(dep.name().into(), local);
					}
				}
				DepSource::Path(ref path) => {
					self.local_pkgs.insert(dep.name().into(), PathBuf::from(path));
				}
			}
		}

		// Load the package description at each dependency's commit.
		for dep in package.deps() {
			let local = &self.local_pkgs[dep.name()];
			let commit = match *dep.version() {
				DepVersion::Any => None,
				DepVersion::Commit(ref s) => Some(s.clone()),
				DepVersion::Version(ref v) => {
					self.find_version_commit(local, v).unwrap()
				},
			};

			// Load the package configuration at that specific commit.
			// println!("loadpkg {:?} commit {:?}", local, commit);
			let pkg = match Package::new(local, commit) {
				Ok(x) => x,
				Err(e) => {
					print_error!("{}: {}", dep.name(), e);
					any_failed = true;
					continue;
				}
			};
		}
	}


	fn clone_url(&mut self, url: &str, into: &Path) -> Result<(),()> {
		println!("cloning {}", url);
		let output = Command::new("git")
			.arg("clone")
			.arg("-n")
			.arg(url)
			.arg(into.to_str().unwrap())
			.output()
			.expect("failed to execute git clone");

		if output.status.success() {
			Ok(())
		} else {
			println!("failed to clone {} into {:?}", url, into);
			println!("status: {}", output.status);
			println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
			println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
			Err(())
		}
	}


	fn find_version_commit<P: AsRef<Path>>(&self, path: P, req: &VersionReq) -> Result<Option<String>, ()> {
		// TODO: Fetch the repository's list of tags, only look at those
		// starting with a "v", strip the "v", parse as semver, and compare to
		// find the most recent one that matches the requirements.
		unimplemented!();
	}
}
