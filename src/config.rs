// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Package manifest and configuration files.
//!
//! This module provides reading and writing of package manifests and
//! configuration files.

#![deny(missing_docs)]

/// A package manifest.
///
/// This is usually called `Landa.yml` in the root directory of the package.
#[derive(Serialize, Deserialize, Debug)]
pub struct Manifest {
    /// The package definition.
    pub package: Option<Package>,
}

/// A package definition.
///
/// Contains the metadata for an individual package.
#[derive(Serialize, Deserialize, Debug)]
pub struct Package {
    /// The name of the package.
    pub name: String,
    /// A list of package authors. Each author should be of the form `John Doe
    /// <john@doe.com>`.
    pub authors: Option<Vec<String>>,
}
