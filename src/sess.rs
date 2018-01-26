// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A command line session.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Arc};
use error::*;
use config::{self, Manifest, Config};

/// A session on the command line.
///
/// Contains all the information that is iteratively being gathered and
/// generated as a command on the command line is executed.
#[derive(Debug)]
pub struct Session<'ctx> {
    /// The path of the package within which the tool was executed.
    pub root: &'ctx Path,
    /// The manifest of the root package.
    pub manifest: &'ctx Manifest,
    /// The tool configuration.
    pub config: &'ctx Config,
    /// The dependency table.
    deps: Mutex<DependencyTable>,
}

impl<'ctx> Session<'ctx> {
    /// Create a new session.
    pub fn new(root: &'ctx Path, manifest: &'ctx Manifest, config: &'ctx Config) -> Session<'ctx> {
        Session {
            root: root,
            manifest: manifest,
            config: config,
            deps: Mutex::new(DependencyTable::new()),
        }
    }

    /// Load a dependency stated in a manifest for further inspection.
    ///
    /// This internalizes the dependency and returns a lightweight reference to
    /// it. This reference may then be used to further inspect the dependency
    /// and perform resolution.
    pub fn load_dependency(
        &self,
        name: &str,
        cfg: &config::Dependency,
        manifest: &config::Manifest
    ) -> Result<DependencyRef> {
        debugln!("sess: load dependency `{}` as {:?} for package `{}`", name, cfg, manifest.package.name);
        let src = match *cfg {
            config::Dependency::Version(_) => DependencySource::Registry,
            config::Dependency::Path(ref p) => DependencySource::Path(p.clone()),
            config::Dependency::GitRevision(ref g, _) |
            config::Dependency::GitVersion(ref g, _) => DependencySource::Git(g.clone()),
        };
        let entry = Arc::new(DependencyEntry {
            name: name.into(),
            source: src,
        });
        let mut deps = self.deps.lock().unwrap();
        if let Some(&id) = deps.ids.get(&entry) {
            Ok(id)
        } else {
            let id = DependencyRef(deps.list.len());
            deps.list.push(entry.clone());
            deps.ids.insert(entry, id);
            Ok(id)
        }
    }
}

/// A unique identifier for a dependency.
///
/// These are emitted by the session once a dependency is loaded and are used to
/// uniquely identify dependencies.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DependencyRef(usize);

/// An entry in the session's dependency table.
#[derive(PartialEq, Eq, Hash, Debug)]
struct DependencyEntry {
    /// The name of this dependency.
    name: String,
    /// Where this dependency may be obtained from.
    source: DependencySource,
}

/// Where a dependency may be obtained from.
#[derive(PartialEq, Eq, Hash, Debug)]
enum DependencySource {
    Registry,
    Path(PathBuf),
    Git(String),
}

/// A table of internalized dependencies.
#[derive(Debug)]
struct DependencyTable {
    list: Vec<Arc<DependencyEntry>>,
    ids: HashMap<Arc<DependencyEntry>, DependencyRef>,
}

impl DependencyTable {
    /// Create a new dependency table.
    pub fn new() -> DependencyTable {
        DependencyTable {
            list: Vec::new(),
            ids: HashMap::new(),
        }
    }
}
