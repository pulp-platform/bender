// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

use std::collections::{HashMap, HashSet};
use futures::Future;
use futures::future::join_all;
use tokio_core::reactor::Core;
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
        let mut core = Core::new().unwrap();
        let handle = core.handle();
        // let sio = SessionIo::new(handle);

        let mut dep_vers = Vec::new();
        for (name, dep) in &self.sess.manifest.dependencies {
            let dep_id = self.sess.load_dependency(name, dep, self.sess.manifest);
            let dep_ver = self.sess.dependency_versions(dep_id);
            dep_vers.push(dep_ver.map(move |v| (dep_id, v)));
        }
        debugln!("resolve: root dependencies internalized");
        let dep_vers = join_all(dep_vers);
        debugln!("resolve: waiting for versions");
        debugln!("resolve: versions {:#?}", core.run(dep_vers));
        // for a in v {
        //     debugln!("resolve: versions {:?}", a.wait());
        // }
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
