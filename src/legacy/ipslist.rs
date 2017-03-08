// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

use std::collections::{HashMap, HashSet};
use yaml_rust::Yaml;
use std::path::Path;
use super::{parse_yaml_file, parse_yaml_string};
use errors::{Result, Error};


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
	parse_yaml(parse_yaml_file(&path)?).map_err(|e| e.chain(format!("{}", path.as_ref().to_str().unwrap())))
}


pub fn parse_string<S: AsRef<str>>(string: S) -> Result<IpsList> {
	parse_yaml(parse_yaml_string(string)?)
}


/// Read a `ips_list.yml` file and convert it into a IpsList structure.
pub fn parse_yaml(yamls: Vec<Yaml>) -> Result<IpsList> {
	let mut v = HashMap::new();
	for yaml in yamls {
		if let Some(hash) = yaml.as_hash() {
			for (path, config) in hash {
				let path = match path.as_str() {
					Some(x) => x,
					None => return Err(format!("IP name must be a string, got `{:?}` instead", path).into()),
				};
				let ip = parse_ip(config, path).map_err(|e| e.chain(path))?;
				v.insert(path.into(), ip);
			}
		} else {
			return Err("file is not a dictionary of key-value pairs".into());
		}
	}
	Ok(v)
}


fn parse_ip(yaml: &Yaml, path: &str) -> Result<Ip> {

	// Derive the IP name from the path.
	let name = path.split_whitespace().nth(0).unwrap().split("/").last().unwrap();

	// Extract the commit.
	let commit = match yaml["commit"] {
		Yaml::String(ref s) => Some(s.clone()),
		Yaml::BadValue | Yaml::Null => None,
		ref x => return Err(format!("`commit` must be a string, got `{:?}` instead", x).into())
	};

	// Extract the group.
	let group = match yaml["group"] {
		Yaml::String(ref s) => Some(s.clone()),
		Yaml::BadValue | Yaml::Null => None,
		ref x => return Err(format!("`group` must be a string, got `{:?}` instead", x).into())
	};

	// Extract the domains.
	let domains: HashSet<String> = match yaml["domain"] {
		Yaml::String(ref s) => { let mut m = HashSet::new(); m.insert(s.clone()); m },
		Yaml::Array(ref a) => {
			let mut set = HashSet::new();
			for y in a {
				set.insert(match y.as_str() {
					Some(v) => v.into(),
					None => return Err(format!("`domain` must contain strings, got `{:?}` instead", y).into()),
				});
			}
			set
		}
		ref x => return Err(format!("`domain` must be a string or list of strings, got `{:?}` instead", x).into()),
	};

	// Collect the set of alternatives.
	let alternatives: HashSet<String> = match yaml["alternatives"] {
		Yaml::Array(ref alts) => {
			let mut set = HashSet::new();
			for y in alts {
				set.insert(match y.as_str() {
					Some(v) => v.into(),
					None => return Err(format!("`alternatives` must contain strings, got `{:?}` instead", y).into()),
				});
			}
			set
		},
		Yaml::BadValue | Yaml::Null => HashSet::new(),
		ref x => return Err(format!("`alternatives` must be a list of strings, got `{:?}` instead", x).into()),
	};

	Ok(Ip {
		name: name.into(),
		path: path.into(),
		commit: commit,
		group: group,
		domains: domains,
		alternatives: alternatives,
	})
}
