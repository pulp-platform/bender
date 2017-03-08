// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

pub mod ipslist;
pub mod srcfiles;

use yaml_rust::{Yaml, YamlLoader};
use std::io::Read;
use std::fs::File;
use std::path::Path;
use errors::{Result, Error};


/// Read a YAML file.
fn parse_yaml_file<P: AsRef<Path>>(path: P) -> Result<Vec<Yaml>> {
	let mut content = String::new();
	File::open(path)?.read_to_string(&mut content)?;
	parse_yaml_string(content).into()
}

/// Interpret a string as YAML input.
fn parse_yaml_string<S: AsRef<str>>(string: S) -> Result<Vec<Yaml>> {
	match YamlLoader::load_from_str(string.as_ref()) {
		Ok(yamls) => Ok(yamls),
		Err(e) => Err(Error::new(e).chain("YAML syntax error")),
	}
}
