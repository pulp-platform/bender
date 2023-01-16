// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

#![deny(missing_docs)]

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::mem;

use futures::future::join_all;
use indexmap::{IndexMap, IndexSet};
use tokio::runtime::Runtime;

extern crate itertools;
use self::itertools::Itertools;

extern crate atty;

use std::io::{self, Write};

use crate::config::{self, Manifest};
use crate::error::*;
use crate::sess::{
    self, DependencyConstraint, DependencyRef, DependencyVersion, DependencyVersions, Session,
    SessionIo,
};

/// A dependency resolver.
pub struct DependencyResolver<'ctx> {
    /// The session within which resolution occurs.
    sess: &'ctx Session<'ctx>,
    /// The version table which is used to perform resolution.
    table: HashMap<&'ctx str, Dependency<'ctx>>,
    /// A cache of decisions made by the user during the resolution.
    decisions: HashMap<&'ctx str, DependencyConstraint>,
    /// Checkout Directory overrides in case checkout_dir is defined and contains folders.
    checked_out: HashMap<String, config::Dependency>,
}

impl<'ctx> DependencyResolver<'ctx> {
    /// Create a new dependency resolver.
    pub fn new(sess: &'ctx Session<'ctx>) -> DependencyResolver<'ctx> {
        // TODO: Populate the table with the contents of the lock file.
        DependencyResolver {
            sess,
            table: HashMap::new(),
            decisions: HashMap::new(),
            checked_out: HashMap::new(),
        }
    }

    fn any_open(&self) -> bool {
        self.table.values().any(|dep| {
            dep.sources
                .values()
                .any(|src| matches!(src.state, State::Open))
        })
    }

    /// Resolve dependencies.
    pub fn resolve(mut self) -> Result<config::Locked> {
        let rt = Runtime::new()?;
        let io = SessionIo::new(self.sess);

        // Store path dependencies already in checkout_dir
        if let Some(checkout) = self.sess.manifest.workspace.checkout_dir.clone() {
            if checkout.exists() {
                for dir in fs::read_dir(checkout).unwrap() {
                    self.checked_out.insert(
                        dir.as_ref()
                            .unwrap()
                            .path()
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string(),
                        config::Dependency::Path(dir.unwrap().path()),
                    );
                }
            }
        }

        // Load the plugin dependencies.
        self.register_dependencies_in_manifest(
            &self.sess.config.plugins,
            self.sess.manifest,
            &rt,
            &io,
        )?;

        // Load the dependencies in the root manifest.
        self.register_dependencies_in_manifest(
            &self.sess.manifest.dependencies,
            self.sess.manifest,
            &rt,
            &io,
        )?;

        let mut iteration = 0;
        let mut any_changes = true;
        while any_changes {
            debugln!(
                "resolve: iteration {} table {:#?}",
                iteration,
                TableDumper(&self.table)
            );
            iteration += 1;

            // Fill in dependencies with state `Open`.
            self.init()?;

            // Go through each dependency's versions and apply the constraints
            // imposed by the others.
            self.mark(&rt, &io)?;

            // Pick a version for each dependency.
            any_changes = self.pick()?;

            // Close the dependency set.
            self.close(&rt, &io)?;
        }
        debugln!("resolve: resolved after {} iterations", iteration);

        // Convert the resolved dependencies into a lockfile.
        let sess = self.sess;
        let packages = self
            .table
            .into_iter()
            .map(|(name, dep)| {
                let deps = match dep.manifest {
                    Some(manifest) => manifest.dependencies.keys().cloned().collect(),
                    None => Default::default(),
                };
                let src = dep.source();
                let sess_src = sess.dependency_source(src.id);
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
                            dependencies: deps,
                        }
                    }
                    DependencyVersions::Registry(ref _rv) => {
                        return Err(Error::new(format!(
                            "Registry dependencies such as `{}` not yet supported.",
                            name
                        )));
                    }
                    DependencyVersions::Git(ref gv) => {
                        let url = match sess_src {
                            sess::DependencySource::Git(u) => u,
                            _ => unreachable!(),
                        };
                        let pick = src.state.pick().unwrap();
                        let rev = gv.revs[pick];
                        let version = gv
                            .versions
                            .iter()
                            .filter(|&&(_, r)| r == rev)
                            .map(|&(ref v, _)| v)
                            .max()
                            .map(|v| v.to_string());
                        config::LockedPackage {
                            revision: Some(String::from(rev)),
                            version,
                            source: config::LockedSource::Git(url),
                            dependencies: deps,
                        }
                    }
                };
                Ok((name.to_string(), pkg))
            })
            .collect::<Result<_>>()?;
        Ok(config::Locked { packages })
    }

    fn register_dependency(
        &mut self,
        name: &'ctx str,
        dep: DependencyRef,
        versions: DependencyVersions<'ctx>,
    ) {
        let entry = self
            .table
            .entry(name)
            .or_insert_with(|| Dependency::new(name));
        entry
            .sources
            .entry(dep)
            .or_insert_with(|| DependencySource::new(dep, versions));
    }

    fn register_dependencies_in_manifest(
        &mut self,
        deps: &'ctx HashMap<String, config::Dependency>,
        manifest: &'ctx config::Manifest,
        rt: &Runtime,
        io: &SessionIo<'ctx, 'ctx>,
    ) -> Result<()> {
        // Map the dependencies to unique IDs.
        let names: HashMap<&str, DependencyRef> = deps
            .iter()
            .map(|(name, dep)| {
                let name = name.as_str();
                let dep = self.checked_out.get(name).unwrap_or(dep);
                let dep = self.sess.config.overrides.get(name).unwrap_or(dep);
                (name, self.sess.load_dependency(name, dep, manifest))
            })
            .collect();
        let ids: HashSet<DependencyRef> = names.iter().map(|(_, &id)| id).collect();
        // debugln!("resolve: dep names {:?}", names);
        // debugln!("resolve: dep ids {:?}", ids);

        // Determine the available versions for the dependencies.
        let versions: Vec<_> = ids
            .iter()
            .map(|&id| async move {
                io.dependency_versions(id, false)
                    .await
                    .map(move |v| (id, v))
            })
            .collect();
        let versions: HashMap<_, _> = rt
            .block_on(join_all(versions))
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .collect::<HashMap<_, _>>();
        // debugln!("resolve: versions {:#?}", versions);

        // Register the versions.
        for (name, id) in names {
            if name == self.sess.manifest.package.name {
                return Err(Error::new(format!(
                    "Please ensure no packages with same name as top package\n\
                    \tCurrently {} is called in {}",
                    name, manifest.package.name
                )));
            }
            if name == manifest.package.name {
                return Err(Error::new(format!(
                    "Please ensure no packages with same name as calling package\n\
                    \tCurrently {} is called in {}",
                    name, manifest.package.name
                )));
            }
            self.register_dependency(name, id, versions[&id].clone());
        }
        Ok(())
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
                    DependencyVersions::Path => (0..1).collect(),
                    DependencyVersions::Registry(ref _rv) => {
                        return Err(Error::new(format!(
                            "Resolution of registry dependency `{}` not yet imlemented",
                            dep.name
                        )));
                    }
                    DependencyVersions::Git(ref gv) => (0..gv.revs.len()).collect(),
                };
                src.state = State::Constrained(ids);
            }
        }
        Ok(())
    }

    /// Apply constraints to each dependency's versions.
    fn mark(&mut self, rt: &Runtime, io: &SessionIo<'ctx, 'ctx>) -> Result<()> {
        use std::iter::once;

        // Gather the constraints from the available manifests. Group them by
        // constraint.
        let cons_map = {
            let mut map = HashMap::<&str, Vec<(&str, DependencyConstraint)>>::new();
            let dep_iter = once(self.sess.manifest)
                .chain(self.table.values().filter_map(|dep| dep.manifest))
                .flat_map(|m| {
                    let pkg_name = self.sess.intern_string(m.package.name.clone());
                    m.dependencies.iter().map(move |(n, d)| (n, (pkg_name, d)))
                })
                .map(|(name, (pkg_name, dep))| {
                    (name, (pkg_name, self.checked_out.get(name).unwrap_or(dep)))
                })
                .map(|(name, (pkg_name, dep))| {
                    (
                        name,
                        pkg_name,
                        self.sess.config.overrides.get(name).unwrap_or(dep),
                    )
                });
            for (name, pkg_name, dep) in dep_iter {
                let v = map.entry(name.as_str()).or_default();
                v.push((pkg_name, DependencyConstraint::from(dep)));
            }
            map
        };

        // // Gather the constraints from locked and picked dependencies.
        // for dep in self.table.values_mut() {
        //     for src in dep.sources.values_mut() {
        //         let _pick = match src.state.pick() {
        //             Some(i) => i,
        //             None => continue,
        //         };
        //         // TODO: Ask session for manifest at the picked version.
        //         // TODO: Map dependencies in manifest to constraints.
        //         // TODO: Add to `cons_map` map.
        //     }
        // }
        debugln!(
            "resolve: gathered constraints {:#?}",
            ConstraintsDumper(&cons_map)
        );

        // Impose the constraints on the dependencies.
        let mut table = mem::take(&mut self.table);
        for (name, cons) in cons_map {
            for &(_, ref con) in &cons {
                debugln!("resolve: impose `{}` on `{}`", con, name);
                for src in table.get_mut(name).unwrap().sources.values_mut() {
                    self.impose(name, con, src, &cons, rt, io)?;
                }
            }
        }
        self.table = table;

        Ok(())
    }

    fn req_indices(
        &self,
        name: &str,
        con: &DependencyConstraint,
        src: &DependencySource<'ctx>,
    ) -> Result<Option<indexmap::IndexSet<usize>>> {
        use self::DependencyConstraint as DepCon;
        use self::DependencyVersions as DepVer;
        match (con, &src.versions) {
            (&DepCon::Path, &DepVer::Path) => Ok(None),
            (&DepCon::Version(ref con), &DepVer::Git(ref gv)) => {
                // TODO: Move this outside somewhere. Very inefficient!
                let hash_ids: HashMap<&str, usize> = gv
                    .revs
                    .iter()
                    .enumerate()
                    .map(|(id, &hash)| (hash, id))
                    .collect();
                let mut revs_tmp: IndexMap<_, _> = gv
                    .versions
                    .iter()
                    .sorted()
                    .filter_map(
                        |&(ref v, h)| {
                            if con.matches(v) {
                                Some((v, h))
                            } else {
                                None
                            }
                        },
                    )
                    .collect();
                revs_tmp.reverse();
                let revs: IndexSet<usize> = revs_tmp
                    .iter()
                    .filter_map(|(v, h)| {
                        if con.matches(v) {
                            Some(hash_ids[h])
                        } else {
                            None
                        }
                    })
                    .collect();
                // debugln!("resolve: `{}` matches version requirement `{}` for revs {:?}", name, con, revs);
                Ok(Some(revs))
            }
            (&DepCon::Revision(ref con), &DepVer::Git(ref gv)) => {
                // TODO: Move this outside somewhere. Very inefficient!
                let mut revs: IndexSet<usize> = gv
                    .refs
                    .get(con.as_str())
                    .map(|rf| {
                        gv.revs
                            .iter()
                            .position(|rev| rev == rf)
                            .into_iter()
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        gv.revs
                            .iter()
                            .enumerate()
                            .filter_map(
                                |(i, rev)| if rev.starts_with(con) { Some(i) } else { None },
                            )
                            .collect()
                    });
                revs.sort();
                // debugln!("resolve: `{}` matches revision `{}` for revs {:?}", name, con, revs);
                Ok(Some(revs))
            }
            (&DepCon::Version(ref _con), &DepVer::Registry(ref _rv)) => Err(Error::new(format!(
                "Constraints on registry dependency `{}` not implemented",
                name
            ))),

            // Handle the error cases.
            // TODO: These need to improve a lot!
            (con, &DepVer::Git(..)) => Err(Error::new(format!(
                "Requirement `{}` cannot be applied to git dependency `{}`",
                con, name
            ))),
            (con, &DepVer::Registry(..)) => Err(Error::new(format!(
                "Requirement `{}` cannot be applied to registry dependency `{}`",
                con, name
            ))),
            (_, &DepVer::Path) => Err(Error::new(format!(
                "`{}` is not declared as a path dependency everywhere.",
                name
            ))),
        }
    }

    /// Impose a constraint on a dependency.
    fn impose(
        &mut self,
        name: &'ctx str,
        con: &DependencyConstraint,
        src: &mut DependencySource<'ctx>,
        all_cons: &[(&str, DependencyConstraint)],
        rt: &Runtime,
        io: &SessionIo<'ctx, 'ctx>,
    ) -> Result<()> {
        let indices = match self.req_indices(name, con, src) {
            Ok(o) => match o {
                Some(v) => v,
                None => return Ok(()),
            },
            Err(e) => return Err(e),
        };
        // debugln!("resolve: restricting `{}` to versions {:?}", name, indices);

        if indices.is_empty() {
            src.versions = rt.block_on(io.dependency_versions(src.id, true))?;

            let indices = match self.req_indices(name, con, src) {
                Ok(o) => match o {
                    Some(v) => v,
                    None => return Ok(()),
                },
                Err(e) => return Err(e),
            };
            if indices.is_empty() {
                return Err(Error::new(format!(
                    "Dependency `{}` from {} cannot satisfy requirement `{}`",
                    name,
                    self.sess.dependency(src.id).source,
                    con
                )));
            }
        }

        // Mark all other versions of the dependency as invalid.
        let new_ids = match src.state {
            State::Open => unreachable!(),
            State::Locked(_) => unreachable!(), // TODO: This needs to do something.
            State::Constrained(ref ids) | State::Picked(_, ref ids) => {
                let is_ids = indices
                    .intersection(ids)
                    .copied()
                    .collect::<IndexSet<usize>>();
                if is_ids.is_empty() {
                    let mut msg = format!(
                        "Requirement `{}` conflicts with other requirements on dependency `{}`.\n",
                        con, name
                    );
                    let mut cons = Vec::new();
                    for &(pkg_name, ref con) in all_cons {
                        let _ = write!(msg, "\n- package `{}` requires `{}`", pkg_name, con);
                        cons.push(con);
                    }
                    cons = cons.into_iter().unique().collect();
                    // Let user resolve conflict if both stderr and stdin go to a TTY.
                    if atty::is(atty::Stream::Stderr) && atty::is(atty::Stream::Stdin) {
                        let decision = if let Some(d) = self.decisions.get(name) {
                            d.clone()
                        } else {
                            eprintln!(
                                "{}\n\nTo resolve this conflict manually, \
                                 select a revision for `{}` among:",
                                msg, name
                            );
                            for (idx, e) in cons.iter().enumerate() {
                                eprintln!("{}) `{}`", idx, e);
                            }
                            loop {
                                eprint!("Enter a number or hit enter to abort: ");
                                io::stdout().flush().unwrap();
                                let mut buffer = String::new();
                                io::stdin().read_line(&mut buffer).unwrap();
                                if buffer.starts_with('\n') {
                                    break Err(Error::new(msg));
                                }
                                let choice = match buffer.trim().parse::<usize>() {
                                    Ok(u) => u,
                                    Err(_) => {
                                        eprintln!("Invalid input!");
                                        continue;
                                    }
                                };
                                let decision = match cons.get(choice) {
                                    Some(c) => c,
                                    None => {
                                        eprintln!("Choice out of bounds!");
                                        continue;
                                    }
                                };
                                self.decisions.insert(name, (*decision).clone());
                                break Ok((*decision).clone());
                            }?
                        };
                        match self.req_indices(name, &decision, src) {
                            Ok(o) => match o {
                                Some(v) => Ok(v),
                                None => unreachable!(),
                            },
                            Err(e) => Err(e),
                        }
                    } else {
                        Err(Error::new(msg))
                    }
                } else {
                    Ok(is_ids)
                }
            }
        };
        match src.state {
            State::Open => unreachable!(),
            State::Locked(_) => unreachable!(),
            State::Constrained(ref mut ids) | State::Picked(_, ref mut ids) => match new_ids {
                Err(e) => Err(e),
                Ok(is) => {
                    *ids = is;
                    Ok(())
                }
            },
        }
    }

    /// Pick a version for each dependency.
    fn pick(&mut self) -> Result<bool> {
        let mut any_changes = false;
        let mut open_pending = HashSet::<&'ctx str>::new();
        for dep in self.table.values_mut() {
            for src in dep.sources.values_mut() {
                src.state = match src.state {
                    State::Open => unreachable!(),
                    State::Locked(id) => State::Locked(id),
                    State::Constrained(ref ids) => {
                        any_changes = true;
                        match src.versions {
                            DependencyVersions::Path => {
                                debugln!(
                                    "resolve: picking path version `{}[{}]`",
                                    dep.name,
                                    src.id
                                );
                                State::Picked(0, IndexSet::new())
                            }
                            DependencyVersions::Git(..) => {
                                debugln!("resolve: picking version for `{}[{}]`", dep.name, src.id);
                                State::Picked(ids.first().copied().unwrap(), ids.clone())
                            }
                            DependencyVersions::Registry(..) => {
                                return Err(Error::new(format!("Version picking for registry dependency `{}` not yet imlemented", dep.name)));
                            }
                        }
                    }
                    State::Picked(id, ref ids) => {
                        if !src.is_path() && !ids.contains(&id) {
                            debugln!(
                                "resolve: picked version for `{}[{}]` no longer valid, resetting",
                                dep.name,
                                src.id
                            );
                            if let Some(manifest) = dep.manifest {
                                open_pending
                                    .extend(manifest.dependencies.keys().map(String::as_str));
                            }
                            any_changes = true;
                            State::Open
                        } else {
                            State::Picked(id, ids.clone())
                        }
                    }
                }
            }
        }

        // Recursively open up dependencies.
        while !open_pending.is_empty() {
            use std::mem::swap;
            let mut open = HashSet::new();
            swap(&mut open_pending, &mut open);
            for dep_name in open {
                debugln!("resolve: resetting `{}`", dep_name);
                let dep = self.table.get_mut(dep_name).unwrap();
                for src in dep.sources.values_mut() {
                    if !src.state.is_open() {
                        any_changes = true;
                        if let Some(manifest) = dep.manifest {
                            open_pending.extend(manifest.dependencies.keys().map(String::as_str));
                        }
                        src.state = State::Open;
                    }
                }
            }
        }

        Ok(any_changes)
    }

    /// Close the set of dependencies.
    fn close(&mut self, rt: &Runtime, io: &SessionIo<'ctx, 'ctx>) -> Result<()> {
        debugln!("resolve: computing closure over dependencies");
        let manifests: Vec<(&str, Option<&Manifest>)> = {
            let mut sub_deps = Vec::new();
            for dep in self.table.values() {
                let src = dep.source();
                let version = match src.pick() {
                    Some(v) => v,
                    None => continue,
                };
                let manifest = io.dependency_manifest_version(src.id, version);
                sub_deps.push(async move { manifest.await.map(move |m| (dep.name, m)) });
            }
            rt.block_on(join_all(sub_deps))
                .into_iter()
                .collect::<Result<Vec<(&str, Option<&Manifest>)>>>()?
        };
        for (name, manifest) in manifests {
            if let Some(m) = manifest {
                debugln!("resolve: for `{}` loaded manifest {:#?}", name, m);
                self.register_dependencies_in_manifest(&m.dependencies, m, rt, io)?;
            }
            let existing = &mut self.table.get_mut(name).unwrap().manifest;
            *existing = manifest;
        }
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
/// # a/Bender.yml
/// dependencies:
///   foo: { git: "alpha@example.com:foo", version: "1.0.0" }
///
/// # b/Bender.yml
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
    sources: HashMap<DependencyRef, DependencySource<'ctx>>,
    /// The picked manifest for this dependency.
    manifest: Option<&'ctx config::Manifest>,
}

impl<'ctx> Dependency<'ctx> {
    /// Create a new dependency.
    fn new(name: &'ctx str) -> Dependency<'ctx> {
        Dependency {
            name,
            sources: HashMap::new(),
            manifest: None,
        }
    }

    /// Return the main source for this dependency.
    ///
    /// This is currently defined as the very first source found for this
    /// dependency.
    fn source(&self) -> &DependencySource<'ctx> {
        let min = self.sources.keys().min().unwrap();
        &self.sources[min]
    }
}

/// A source for a dependency.
///
/// A dependency may have multiple sources. See `Dependency`.
#[derive(Debug)]
struct DependencySource<'ctx> {
    /// The ID of this dependency.
    id: DependencyRef,
    /// The available versions of the dependency.
    versions: DependencyVersions<'ctx>,
    /// The currently picked version.
    pick: Option<usize>,
    /// The available version options. These are indices into `versions`.
    options: Option<HashSet<usize>>,
    /// The current resolution state.
    state: State,
}

impl<'ctx> DependencySource<'ctx> {
    /// Create a new dependency source.
    fn new(id: DependencyRef, versions: DependencyVersions<'ctx>) -> DependencySource<'ctx> {
        DependencySource {
            id,
            versions,
            pick: None,
            options: None,
            state: State::Open,
        }
    }

    /// Return the picked version, if any.
    ///
    /// In case the state is `Locked` or `Picked`, returns the version that was
    /// picked. Otherwise returns `None`.
    fn pick(&self) -> Option<DependencyVersion<'ctx>> {
        match self.state {
            State::Open | State::Constrained(..) => None,
            State::Locked(id) | State::Picked(id, _) => match self.versions {
                DependencyVersions::Path => Some(DependencyVersion::Path),
                DependencyVersions::Registry(ref _rv) => None,
                DependencyVersions::Git(ref gv) => Some(DependencyVersion::Git(gv.revs[id])),
            },
        }
    }

    /// Check whether this is a path dependency.
    fn is_path(&self) -> bool {
        matches!(self.versions, DependencyVersions::Path)
    }
}

#[derive(Debug)]
enum State {
    /// The dependency has never been seen before and is not constrained.
    Open,
    /// The dependency has been locked in the lockfile.
    Locked(usize),
    /// The dependency may assume any of the listed versions.
    Constrained(IndexSet<usize>),
    /// The dependency had a version picked.
    Picked(usize, IndexSet<usize>),
}

impl State {
    /// Check whether the state is `Open`.
    fn is_open(&self) -> bool {
        matches!(*self, State::Open)
    }

    /// Return the index of the picked version, if any.
    ///
    /// In case the state is `Locked` or `Picked`, returns the version that was
    /// picked. Otherwise returns `None`.
    fn pick(&self) -> Option<usize> {
        match *self {
            State::Locked(i) | State::Picked(i, _) => Some(i),
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
                    State::Picked(idx, ref idcs) => {
                        write!(f, " picked #{} out of {} possible", idx, idcs.len())?
                    }
                }
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}

struct ConstraintsDumper<'a>(&'a HashMap<&'a str, Vec<(&'a str, DependencyConstraint)>>);

impl<'a> fmt::Debug for ConstraintsDumper<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        write!(f, "{{")?;
        for name in names {
            let cons = self.0.get(name).unwrap();
            write!(f, "\n    \"{}\":", name)?;
            for &(pkg_name, ref con) in cons {
                write!(f, " {} ({});", con, pkg_name)?;
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}
