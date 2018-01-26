// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

use std::collections::{HashMap, HashSet};
use futures::Future;
use sess::Session;
use error::*;

pub struct DependencyResolver<'ctx> {
    sess: &'ctx Session<'ctx>,
    table: HashMap<&'ctx str, Dep>,
}

impl<'ctx> DependencyResolver<'ctx> {
    /// Create a new dependency resolver.
    pub fn new(sess: &'ctx Session<'ctx>) -> DependencyResolver<'ctx> {
        // TODO: Populate the table with the contents of the lock file.
        DependencyResolver {
            sess: sess,
            table: HashMap::new(),
        }
    }

    /// Resolve dependencies.
    pub fn resolve(self) -> Result<()> {
        let mut v = Vec::new();
        for (name, dep) in &self.sess.manifest.dependencies {
            let dep_id = self.sess.load_dependency(name, dep, self.sess.manifest);
            let versions = self.sess.dependency_versions(dep_id);
            v.push(versions);
        }
        debugln!("resolve: root dependencies internalized");
        for a in v {
            debugln!("resolve: versions {:?}", a.wait());
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Dep {
    /// The current state of the dependency.
    state: State,
    /// The available versions of the dependency.
    available: Vec<Version>,
}

#[derive(Debug)]
enum State {
    /// The dependency has never been seen before and is not constrained.
    Open,
    /// The dependency has been locked in the lockfile.
    Locked(usize),
    /// The dependency may assume any of the listed versions.
    Constrained(HashSet<usize>),
}

#[derive(Debug)]
struct Version {

}
