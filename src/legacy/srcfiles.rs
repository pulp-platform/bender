// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>
//
// Copyright (C) 2017 ETH Zurich
// All rights reserved.

use yaml_rust::Yaml;
use std::collections::{HashMap, HashSet};
use std::io::{Result, Error, ErrorKind};
use std::path::Path;


pub type SrcFiles = HashMap<String, SrcGroup>;

#[derive(Debug)]
pub struct SrcGroup {
	pub files: Vec<String>,
	pub flags: HashSet<Flag>,
	pub targets: HashSet<String>,
	pub vcom_opts: Vec<String>,
	pub vlog_opts: Vec<String>,
	pub defines: Vec<String>,
	pub incdirs: Vec<String>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub enum Flag {
	SkipSynthesis,
	SkipSimulation,
	SkipTcsh,
}


pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<SrcFiles> {
	parse_yaml(&super::yaml_from_file(path)?)
}


pub fn parse_string<S: AsRef<str>>(string: S) -> Result<SrcFiles> {
	parse_yaml(&super::yaml_from_string(string)?)
}


pub fn parse_yaml(yaml: &Yaml) -> Result<SrcFiles> {
	let mut groups = HashMap::new();
	match *yaml {
		Yaml::Hash(ref hash) => for (group_name, group) in hash {
			let name = match group_name.as_str() {
				Some(x) => x,
				None => return Err(Error::new(ErrorKind::Other, format!("group name must be a string, got {:?} instead", group_name))),
			};
			groups.insert(name.into(), parse_src_files_group(group)?);
		},
		ref x => return Err(Error::new(ErrorKind::Other, format!("src_files.yml must be a dictionary, got {:?} instead", x)))
	}
	Ok(groups)
}


fn parse_src_files_group(yaml: &Yaml) -> Result<SrcGroup> {
	// incdirs
	let incdirs: Vec<_> = match yaml["incdirs"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => Vec::new(),
	};

	// vlog_opts
	let vlog_opts: Vec<_> = match yaml["vlog_opts"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => Vec::new(),
	};

	// vcom_opts
	let vcom_opts: Vec<_> = match yaml["vcom_opts"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => Vec::new(),
	};

	// defines
	let defines: Vec<_> = match yaml["defines"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => Vec::new(),
	};

	// flags
	let flags: HashSet<Flag> = match yaml["flags"].as_vec() {
		Some(x) => x.iter().map(|y| match y.as_str().unwrap() {
			"skip_synthesis" => Flag::SkipSynthesis,
			"skip_simulation" => Flag::SkipSimulation,
			"skip_tcsh" => Flag::SkipTcsh,
			x => panic!("unknown flag `{}`", x)
		}).collect(),
		None => HashSet::new(),
	};

	// files
	let files: Vec<String> = match yaml["files"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => Vec::new(),
	};

	// targets
	let targets: HashSet<String> = match yaml["targets"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => HashSet::new(),
	};

	Ok(SrcGroup {
		files: files,
		flags: flags,
		targets: targets,
		vcom_opts: vcom_opts,
		vlog_opts: vlog_opts,
		defines: defines,
		incdirs: incdirs,
	})
}
