// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>
//
// Copyright (C) 2017 ETH Zurich
// All rights reserved.

pub mod ipslist;
pub mod srcfiles;

use yaml_rust::{Yaml, YamlLoader};
use std::io::Read;
use std::fs::File;
use std::io::{Result, Error, ErrorKind};
use std::path::Path;


fn yaml_from_file<P: AsRef<Path>>(path: P) -> Result<Yaml> {
	let mut content = String::new();
	File::open(path)?.read_to_string(&mut content)?;
	yaml_from_string(content)
}

fn yaml_from_string<S: AsRef<str>>(string: S) -> Result<Yaml> {
	match YamlLoader::load_from_str(string.as_ref()) {
		Ok(yaml) => Ok(yaml.into_iter().nth(0).unwrap()),
		Err(e) => Err(Error::new(ErrorKind::Other, e)),
	}
}
