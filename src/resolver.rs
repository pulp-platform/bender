// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

#![deny(missing_docs)]

use std::collections::{HashMap, HashSet};
use futures::Future;
use futures::future::join_all;
use tokio_core::reactor::Core;
use sess::{Session, SessionIo, DependencyVersions, DependencyRef};
use error::*;

/// A dependency resolver.
pub struct DependencyResolver<'ctx> {
    /// The session within which resolution occurs.
    sess: &'ctx Session<'ctx>,
    /// The version table which is used to perform resolution.
    table: HashMap<&'ctx str, Dependency<'ctx>>,
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
    pub fn resolve(mut self) -> Result<()> {
        let mut core = Core::new().unwrap();
        let io = SessionIo::new(self.sess, core.handle());

        // Map the dependencies to unique IDs.
        let names: HashMap<&str, DependencyRef> = self.sess.manifest.dependencies
            .iter()
            .map(|(name, dep)|{
                (name.as_str(), self.sess.load_dependency(name, dep, self.sess.manifest))
            })
            .collect();
        let ids: HashSet<DependencyRef> = names.iter().map(|(_, &id)| id).collect();
        debugln!("resolve: dep names {:?}", names);
        debugln!("resolve: dep ids {:?}", ids);

        // Determine the available versions for the dependencies.
        let versions: Vec<_> = ids.iter().map(|&id| io
            .dependency_versions(id)
            .map(move |v| (id, v))
        ).collect();
        let versions: HashMap<_,_> = core
            .run(join_all(versions))?
            .into_iter()
            .collect();
        debugln!("resolve: versions {:#?}", versions);

        // Register the versions.
        for (name, id) in names {
            self.register_dependency(name, id, versions[&id].clone());
        }
        Ok(())
    }

    fn register_dependency(
        &mut self,
        name: &'ctx str,
        dep: DependencyRef,
        versions: DependencyVersions
    ) {
        use std::collections::hash_map::Entry;
        let entry = self.table
            .entry(name)
            .or_insert_with(|| Dependency::new(name));
        entry.sources
            .entry(dep)
            .or_insert_with(|| DependencySource::new(dep, versions));
    }
}

/// A dependency in the version table.
///
/// One such entry exists per dependency name. Note that multiple sources may
/// exist for each dependency. This happens if two packages specify the same
/// dependency name, but two different sources:
///
/// ```ignore
/// # a/Landa.yml
/// dependencies:
///   foo: { git: "alpha@example.com:foo", version: "1.0.0" }
///
/// # b/Landa.yml
/// dependencies:
///   foo: { git: "beta@example.com:foo", version: "1.0.0" }
/// ```
///
/// Note that despite the different sources, they might refer to the same
/// dependency and be compatible, e.g. via the git hash.
#[derive(Debug)]
struct Dependency<'ctx> {
    /// The name of the dependency.
    name: &'ctx str,
    /// The set of sources for this dependency.
    sources: HashMap<DependencyRef, DependencySource>,
}

impl<'ctx> Dependency<'ctx> {
    /// Create a new dependency.
    fn new(name: &'ctx str) -> Dependency<'ctx> {
        Dependency {
            name: name,
            sources: HashMap::new(),
        }
    }
}

/// A source for a dependency.
///
/// A dependency may have multiple sources. See `Dependency`.
#[derive(Debug)]
struct DependencySource {
    /// The ID of this dependency.
    id: DependencyRef,
    /// The available versions of the dependency.
    versions: DependencyVersions,
    /// The current resolution state.
    state: State,
}

impl DependencySource {
    /// Create a new dependency source.
    fn new(id: DependencyRef, versions: DependencyVersions) -> DependencySource {
        DependencySource {
            id: id,
            versions: versions,
            state: State::Open,
        }
    }
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
