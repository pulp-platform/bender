// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

#![deny(missing_docs)]

use std::fmt;
use std::collections::{HashMap, HashSet};
use futures::Future;
use futures::future::join_all;
use tokio_core::reactor::Core;
use sess::{self, Session, SessionIo, DependencyVersions, DependencyRef, DependencyConstraint};
use error::*;
use config;

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
    pub fn resolve(mut self) -> Result<config::Locked> {
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
        debugln!("resolve: table {:#?}", TableDumper(&self.table));

        // Fill in dependencies with state `Open`.
        self.init()?;
        debugln!("resolve: table {:#?}", TableDumper(&self.table));

        // Go through each dependency's versions and apply the constraints
        // imposed by the others.
        self.mark()?;
        debugln!("resolve: table {:#?}", TableDumper(&self.table));

        // Pick a version for each dependency.
        self.pick()?;
        debugln!("resolve: table {:#?}", TableDumper(&self.table));

        // Close the dependency set.
        self.close()?;
        debugln!("resolve: table {:#?}", TableDumper(&self.table));

        // Convert the resolved dependencies into a lockfile.
        let sess = self.sess;
        let packages = self.table
            .into_iter()
            .map(|(name, mut dep)|{
                if dep.sources.len() > 1 {
                    return Err(Error::new(format!("Dependencies with multiple sources, such as `{}`, are not yet supported.", name)));
                }
                let (id, src) = dep.sources.drain().next().unwrap();
                let pick = src.state.pick().unwrap();
                let sess_src = sess.dependency_source(id);
                let pkg = match src.versions {
                    DependencyVersions::Path => {
                        let path = match sess_src {
                            sess::DependencySource::Path(p) => p,
                            _ => unreachable!(),
                        };
                        config::LockedPackage {
                            revision: None,
                            version: None,
                            source: config::LockedSource::Path(path),
                        }
                    }
                    DependencyVersions::Registry(ref _rv) => {
                        return Err(Error::new(format!("Registry dependencies such as `{}` not yet supported.", name)));
                    }
                    DependencyVersions::Git(ref gv) => {
                        let url = match sess_src {
                            sess::DependencySource::Git(u) => u,
                            _ => unreachable!(),
                        };
                        let rev = gv.revs[pick].clone();
                        let version = gv.versions
                            .iter()
                            .filter(|&&(_, ref r)| *r == rev)
                            .map(|&(ref v, _)| v)
                            .max()
                            .map(|v| v.to_string());
                        config::LockedPackage {
                            revision: Some(rev),
                            version: version,
                            source: config::LockedSource::Git(url),
                        }
                    }
                };
                Ok((name.to_string(), pkg))
            })
            .collect::<Result<_>>()?;
        Ok(config::Locked {
            packages: packages
        })
    }

    fn register_dependency(
        &mut self,
        name: &'ctx str,
        dep: DependencyRef,
        versions: DependencyVersions
    ) {
        let entry = self.table
            .entry(name)
            .or_insert_with(|| Dependency::new(name));
        entry.sources
            .entry(dep)
            .or_insert_with(|| DependencySource::new(dep, versions));
    }

    /// Initialize dependencies with state `Open`.
    ///
    /// This populates the dependency's set of possible versions with all
    /// available versions, such that they may then be constrained.
    fn init(&mut self) -> Result<()> {
        for dep in self.table.values_mut() {
            for src in dep.sources.values_mut() {
                if !src.state.is_open() {
                    continue;
                }
                debugln!("resolve: initializing `{}[{}]`", dep.name, src.id);
                let ids = match src.versions {
                    DependencyVersions::Path => {
                        (0..1).collect()
                    }
                    DependencyVersions::Registry(ref _rv) => {
                        return Err(Error::new(format!("Resolution of registry dependency `{}` not yet imlemented", dep.name)));
                    }
                    DependencyVersions::Git(ref gv) => {
                        (0..gv.revs.len()).collect()
                    }
                };
                src.state = State::Constrained(ids);
            }
        }
        Ok(())
    }

    /// Apply constraints to each dependency's versions.
    fn mark(&mut self) -> Result<()> {
        // Gather the constraints from the root package.
        let cons_map: HashMap<&str, Vec<DependencyConstraint>> =
            self.sess.manifest.dependencies
            .iter()
            .map(|(name, dep)| (
                name.as_str(),
                vec![DependencyConstraint::from(dep)],
            ))
            .collect();

        // Gather the constraints from locked and picked dependencies.
        for dep in self.table.values_mut() {
            for src in dep.sources.values_mut() {
                let _pick = match src.state.pick() {
                    Some(i) => i,
                    None => continue,
                };
                // TODO: Ask session for manifest at the picked version.
                // TODO: Map dependencies in manifest to constraints.
                // TODO: Add to `cons_map` map.
            }
        }
        debugln!("resolve: gathered constraints {:#?}", ConstraintsDumper(&cons_map));

        // Impose the constraints on the dependencies.
        for (name, cons) in cons_map {
            for con in cons {
                debugln!("resolve: impose `{}` on `{}`", con, name);
                for src in self.table.get_mut(name).unwrap().sources.values_mut() {
                    Self::impose(name, &con, src)?;
                }
            }
        }

        Ok(())
    }

    /// Impose a constraint on a dependency.
    fn impose(
        name: &str,
        con: &DependencyConstraint,
        src: &mut DependencySource
    ) -> Result<()> {

        use self::DependencyConstraint as DepCon;
        use self::DependencyVersions as DepVer;
        let indices = match (con, &src.versions) {
            (&DepCon::Path, &DepVer::Path) => return Ok(()),
            (&DepCon::Version(ref con), &DepVer::Git(ref gv)) => {
                // TODO: Move this outside somewhere. Very inefficient!
                let hash_ids: HashMap<&str, usize> = gv.revs
                    .iter()
                    .enumerate()
                    .map(|(id, hash)| (hash.as_str(), id))
                    .collect();
                let revs: HashSet<usize> = gv.versions
                    .iter()
                    .filter_map(|&(ref v, ref h)| if con.matches(v) {
                        Some(hash_ids[h.as_str()])
                    } else {
                        None
                    })
                    .collect();
                debugln!("resolve: `{}` matches version requirement `{}` for revs {:?}", name, con, revs);
                revs
            }
            (&DepCon::Revision(ref con), &DepVer::Git(ref gv)) => {
                // TODO: Move this outside somewhere. Very inefficient!
                let revs: HashSet<usize> = gv.refs
                    .get(con)
                    .map(|rf| gv.revs
                        .iter()
                        .position(|rev| rev == rf)
                        .into_iter()
                        .collect())
                    .unwrap_or_else(|| gv.revs
                        .iter()
                        .enumerate()
                        .filter_map(|(i, rev)| if rev.starts_with(con) {
                            Some(i)
                        } else {
                            None
                        })
                        .collect()
                    );
                debugln!("resolve: `{}` matches revision `{}` for revs {:?}", name, con, revs);
                revs
            }
            (&DepCon::Version(ref _con), &DepVer::Registry(ref _rv)) => {
                return Err(Error::new(format!("Constraints on registry dependency `{}` not implemented", name)));
            }

            // Handle the error cases.
            // TODO: These need to improve a lot!
            (con, &DepVer::Git(..)) => {
                return Err(Error::new(format!("Requirement `{}` cannot be applied to git dependency `{}`", con, name)));
            }
            (con, &DepVer::Registry(..)) => {
                return Err(Error::new(format!("Requirement `{}` cannot be applied to registry dependency `{}`", con, name)));
            }
            (_, &DepVer::Path) => {
                return Err(Error::new(format!("`{}` is not declared as a path dependency everywhere.", name)));
            }
        };
        debugln!("resolve: restricting `{}` to versions {:?}", name, indices);

        if indices.is_empty() {
            return Err(Error::new(format!("Dependency `{}` cannot satisfy requirement `{}`", name, con)));
        }

        // Mark all other versions of the dependency as invalid.
        match src.state {
            State::Open => unreachable!(),
            State::Locked(_) => unreachable!(), // TODO: This needs to do something.
            State::Constrained(ref mut ids) |
            State::Picked(_, ref mut ids) => {
                *ids = (*ids).intersection(&indices).map(|i| *i).collect();
                if ids.is_empty() {
                    return Err(Error::new(format!("Requirement `{}` conflicts with other requirements on dependency `{}`", con, name)));
                }
            }
        };

        Ok(())
    }

    /// Pick a version for each dependency.
    fn pick(&mut self) -> Result<()> {
        for dep in self.table.values_mut() {
            for src in dep.sources.values_mut() {
                debugln!("resolve: picking version for `{}[{}]`", dep.name, src.id);
                src.state = match src.state {
                    State::Open => unreachable!(),
                    State::Locked(id) => State::Locked(id),
                    State::Constrained(ref ids) => {
                        match src.versions {
                            DependencyVersions::Path => unreachable!(),
                            DependencyVersions::Git(..) => {
                                State::Picked(
                                    ids.iter().map(|i| *i).min().unwrap(),
                                    ids.clone()
                                )
                            }
                            DependencyVersions::Registry(..) => {
                                return Err(Error::new(format!("Version picking for registry dependency `{}` not yet imlemented", dep.name)));
                            }
                        }
                    }
                    State::Picked(id, ref ids) => {
                        if !ids.contains(&id) {
                            debugln!("resolve: picked version for `{}[{}]` no longer valid, opening", dep.name, src.id);
                            // TODO: Recursively open up all dependencies.
                            State::Open
                        } else {
                            State::Picked(id, ids.clone())
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Close the set of dependencies.
    fn close(&mut self) -> Result<()> {
        debugln!("resolve: would now compute closure over dependencies");
        Ok(())
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
    /// The dependency had a version picked.
    Picked(usize, HashSet<usize>),
}

impl State {
    /// Check whether the state is `Open`.
    fn is_open(&self) -> bool {
        match *self {
            State::Open => true,
            _ => false,
        }
    }

    /// Return the picked version, if any.
    ///
    /// In case the state is `Locked` or `Picked`, returns the version that was
    /// picked. Otherwise returns `None`.
    fn pick(&self) -> Option<usize> {
        match *self {
            State::Locked(i) | State::Picked(i,_) => Some(i),
            _ => None,
        }
    }
}

struct TableDumper<'a>(&'a HashMap<&'a str, Dependency<'a>>);

impl<'a> fmt::Debug for TableDumper<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        write!(f, "{{")?;
        for name in names {
            let dep = self.0.get(name).unwrap();
            write!(f, "\n    \"{}\":", name)?;
            for (&id, src) in &dep.sources {
                write!(f, "\n        [{}]:", id)?;
                match src.state {
                    State::Open => write!(f, " open")?,
                    State::Locked(idx) => write!(f, " locked {}", idx)?,
                    State::Constrained(ref idcs) => write!(f, " {} possible", idcs.len())?,
                    State::Picked(idx, ref idcs) => write!(f, " picked #{} out of {} possible", idx, idcs.len())?,
                }
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}

struct ConstraintsDumper<'a>(&'a HashMap<&'a str, Vec<DependencyConstraint>>);

impl<'a> fmt::Debug for ConstraintsDumper<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        write!(f, "{{")?;
        for name in names {
            let cons = self.0.get(name).unwrap();
            write!(f, "\n    \"{}\":", name)?;
            for con in cons {
                write!(f, " {}", con)?;
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}
