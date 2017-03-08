// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! This module provides facilities for loading the package configuration of a
//! directory from disk.

use semver::VersionReq;
use legacy;
use std::path::{Path, PathBuf};
use git::Git;
use std::io::Read;
use std::fs::File;
use std::collections::{HashMap, HashSet};
use errors::{Result, Error};


/// A single package and its configuration loaded from various files.
#[derive(Debug)]
pub struct Package {
	path: PathBuf,
	deps: Vec<Dep>,
	srcs: Vec<SrcGroup>,
}

#[derive(Debug)]
pub struct Dep {
	name: String,
	source: DepSource,
	version: DepVersion,
}

#[derive(Debug)]
pub enum DepSource {
	Path(String),
	Git(String),
}

#[derive(Debug)]
pub enum DepVersion {
	Any,
	Commit(String),
	Version(VersionReq),
}

#[derive(Debug)]
pub struct SrcGroup {
	include: HashSet<String>,
	exclude: HashSet<String>,
	files: Vec<String>,
	incdirs: Vec<String>,
	defines: HashMap<String, Option<String>>,
	tool_args: Vec<(Tool, String)>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub enum Tool {
	Vcom,
	Vlog,
}



impl Package {
	pub fn new<P: Into<PathBuf>, C: Into<Option<String>>>(path: P, commit: C) -> Result<Package> {
		let path = path.into();
		let commit = commit.into();
		Self::new_inner(path.clone(), commit).map_err(|e| e.chain(path.to_str().unwrap()))
	}

	fn new_inner(path: PathBuf, commit: Option<String>) -> Result<Package> {
		let mut any_content = false;
		let mut deps = Vec::new();
		let mut srcs = Vec::new();
		let git = Git::new(&path);

		// Parse the commit if one has been given.
		let commit = match commit {
			Some(rev) => Some(git.parse_rev(&rev)?),
			None => None,
		};

		// Load the legacy src_files.yml file, which is intended to be placed in
		// IP core packages. The file can contain multiple groups, each
		// providing a different set of files, for all or a limited set of
		// targets.
		if let Some(content) = read_file_if_exists(&git, "src_files.yml", commit.as_ref())? {
			any_content = true;
			let srcfiles = legacy::srcfiles::parse_string(content)
				.map_err(|e| e.chain(git.path().join("src_files.yml").to_str().unwrap()))?;
			for (name, group) in srcfiles {
				// Map the targets to inclusion criteria.
				let mut include = HashSet::new();
				for target in group.targets {
					include.insert(target);
				}

				// Map the skip flags to exclusion criteria.
				let mut exclude = HashSet::new();
				if group.flags.contains(&legacy::srcfiles::Flag::SkipSynthesis) {
					exclude.insert("synth".into());
				}
				if group.flags.contains(&legacy::srcfiles::Flag::SkipSimulation) {
					exclude.insert("sim".into());
				}

				// Split the defines at the optional `=` character.
				let mut defines = HashMap::new();
				for define in group.defines {
					let mut sp = define.splitn(2, '=');
					if let Some(name) = sp.next() {
						defines.insert(name.into(), sp.next().map(|x| x.into()));
					}
				}

				// Map the `{vcom,vlog}_opts` fields to tool arguments.
				let mut tool_args = Vec::new();
				for arg in group.vcom_opts {
					tool_args.push((Tool::Vcom, arg));
				}
				for arg in group.vlog_opts {
					tool_args.push((Tool::Vlog, arg));
				}

				srcs.push(SrcGroup {
					include: include,
					exclude: exclude,
					files: group.files,
					incdirs: group.incdirs,
					defines: defines,
					tool_args: tool_args,
				});
			}
		}

		// Load the legacy ips_list.yml file, which is intended to be placed in
		// chip repositories. The file contains a list of dependencies, the
		// commit to check out, as well as a domain specifier.
		if let Some(content) = read_file_if_exists(&git, "ips_list.yml", commit.as_ref())? {
			any_content = true;
			let ipslist = legacy::ipslist::parse_string(content)
				.map_err(|e| e.chain(git.path().join("ips_list.yml").to_str().unwrap()))?;
			for (name, ip) in ipslist {
				let url = format!("git@iis-git.ee.ethz.ch:{}/{}.git", ip.group.unwrap_or("pulp-project".into()), ip.name);
				deps.push(Dep {
					name: ip.name,
					source: DepSource::Git(url),
					version: DepVersion::Commit(ip.commit.unwrap_or("master".into())),
				});
			}
		}

		// Emit an error if no configuration was found.
		if !any_content {
			Err("directory does not contain any configuration files".into())
		} else {
			Ok(Package {
				path: path,
				deps: deps,
				srcs: srcs,
			})
		}
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn deps(&self) -> &[Dep] {
		&self.deps
	}
}

impl Dep {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn source(&self) -> &DepSource {
		&self.source
	}

	pub fn version(&self) -> &DepVersion {
		&self.version
	}
}



fn read_file_if_exists<P: AsRef<str>, C: AsRef<str>>(git: &Git, path: P, commit: Option<C>) -> Result<Option<String>> {
	match commit {
		Some(rev) => {
			let gc = git.at(rev.as_ref());
			if gc.exists(path.as_ref())? {
				Ok(Some(gc.read_file(path.as_ref())?))
			} else {
				Ok(None)
			}
		}
		None => {
			// Assemble the file path.
			let path = git.path().join(path.as_ref());
			if path.exists() {
				let mut content = String::new();
				File::open(path)?.read_to_string(&mut content)?;
				Ok(Some(content))
			} else {
				Ok(None)
			}
		}
	}
}
