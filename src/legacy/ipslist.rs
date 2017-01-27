// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>
//
// Copyright (C) 2017 ETH Zurich
// All rights reserved.

use std::collections::{HashMap, HashSet};
use yaml_rust::Yaml;
use std::io::{Result, Error, ErrorKind};
use std::path::Path;


pub type IpsList = HashMap<String, Ip>;

#[derive(Debug)]
pub struct Ip {
	pub name: String,
	pub path: String,
	pub commit: Option<String>,
	pub group: Option<String>,
	pub domains: HashSet<String>,
	pub alternatives: HashSet<String>,
}


pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<IpsList> {
	parse_yaml(&super::yaml_from_file(path)?)
}


pub fn parse_string<S: AsRef<str>>(string: S) -> Result<IpsList> {
	parse_yaml(&super::yaml_from_string(string)?)
}


pub fn parse_yaml(yaml: &Yaml) -> Result<IpsList> {
	let mut v = HashMap::new();
	for (path, config) in yaml.as_hash().unwrap() {
		let path = path.as_str().unwrap();
		v.insert(path.into(), parse_ip(config, path));
	}
	Ok(v)
}


fn parse_ip(yaml: &Yaml, path: &str) -> Ip {

	// Derive the IP name from the path.
	let name = path.split_whitespace().nth(0).unwrap().split("/").last().unwrap();

	// Extract the commit.
	let commit = match yaml["commit"] {
		Yaml::String(ref s) => Some(s.clone()),
		Yaml::BadValue | Yaml::Null => None,
		ref x => panic!("{}: ip {}: `commit` must be a string, got `{:?}` instead", path, name, x)
	};

	// Extract the group.
	let group = match yaml["group"] {
		Yaml::String(ref s) => Some(s.clone()),
		Yaml::BadValue | Yaml::Null => None,
		ref x => panic!("{}: ip {}: `group` must be a string, got `{:?}` instead", path, name, x)
	};

	// Extract the domains.
	let domains: HashSet<String> = match yaml["domain"] {
		Yaml::String(ref s) => { let mut m = HashSet::new(); m.insert(s.clone()); m },
		Yaml::Array(ref a) => a.iter().map(|y| y.as_str().unwrap().into()).collect(),
		ref x => panic!("{}: ip {}: `domain` must be a string or list of strings, got `{:?}` instead", path, name, x),
	};

	// Collect the set of alternatives.
	let alternatives: HashSet<String> = match yaml["alternatives"].as_vec() {
		Some(x) => x.iter().map(|y| y.as_str().unwrap().into()).collect(),
		None => HashSet::new(),
	};

	Ip {
		name: name.into(),
		path: path.into(),
		commit: commit,
		group: group,
		domains: domains,
		alternatives: alternatives,
	}
}
