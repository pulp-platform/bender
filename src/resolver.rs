// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::IsTerminal;
use std::io::{self, Write};
use std::mem;
use std::process::Command as SysCommand;

use futures::future::join_all;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use semver::{Version, VersionReq};
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::config::{self, Locked, LockedPackage, LockedSource, Manifest};
use crate::debugln;
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{
    DependencyConstraint, DependencyRef, DependencySource, DependencyVersion, DependencyVersions,
    Session, SessionIo,
};
use crate::target::TargetSpec;
use crate::util::{version_req_bottom_bound, version_req_top_bound};
use crate::{fmt_path, fmt_pkg, fmt_version};

/// A dependency resolver.
pub struct DependencyResolver<'ctx> {
    /// The session within which resolution occurs.
    sess: &'ctx Session<'ctx>,
    /// The version table which is used to perform resolution.
    table: IndexMap<&'ctx str, Dependency<'ctx>>,
    /// A cache of decisions made by the user during the resolution.
    decisions: IndexMap<&'ctx str, (DependencyConstraint, DependencyRef)>,
    /// Checkout Directory overrides in case checkout_dir is defined and contains unmodified folders.
    checked_out: IndexMap<String, config::Dependency>,
    /// Lockfile data.
    locked: IndexMap<&'ctx str, (DependencyConstraint, DependencyRef, Option<&'ctx str>, bool)>,
    /// A helpful map
    src_ref_map: IndexMap<DependencySource, DependencyRef>,
}

impl<'ctx> DependencyResolver<'ctx> {
    /// Create a new dependency resolver.
    pub fn new(sess: &'ctx Session<'ctx>) -> DependencyResolver<'ctx> {
        // TODO: Populate the table with the contents of the lock file.
        DependencyResolver {
            sess,
            table: IndexMap::new(),
            decisions: IndexMap::new(),
            checked_out: IndexMap::new(),
            locked: IndexMap::new(),
            src_ref_map: IndexMap::new(),
        }
    }

    /// Resolve dependencies.
    pub fn resolve(
        mut self,
        existing: Option<&'ctx Locked>,
        ignore_checkout: bool,
        keep_locked: IndexSet<&'ctx String>,
    ) -> Result<Locked> {
        let rt = Runtime::new()?;
        let io = SessionIo::new(self.sess);

        // Load the dependencies in the lockfile.
        if let Some(existing) = existing {
            debugln!("resolve: registering lockfile dependencies");
            self.register_dependencies_in_lockfile(existing, keep_locked, &rt, &io)?;
        }

        // Store path dependencies already in checkout_dir
        if let Some(checkout) = self.sess.manifest.workspace.checkout_dir.clone() {
            if checkout.exists() {
                for dir in fs::read_dir(&checkout).unwrap() {
                    let depname = dir
                        .as_ref()
                        .unwrap()
                        .path()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string();

                    let is_git_repo = dir.as_ref().unwrap().path().join(".git").exists();

                    // Only act if the avoiding flag is not set and any of the following match
                    //  - the dependency is not a git repo
                    //  - the dependency is not in a clean state (i.e., was modified)
                    if !ignore_checkout {
                        if !is_git_repo {
                            Warnings::NotAGitDependency(depname.clone(), checkout.clone()).emit();
                            self.checked_out.insert(
                                depname,
                                config::Dependency::Path {
                                    target: TargetSpec::Wildcard,
                                    path: dir.unwrap().path(),
                                    pass_targets: vec![],
                                },
                            );
                        } else if !(SysCommand::new(&self.sess.config.git) // If not in a clean state
                            .arg("status")
                            .arg("--porcelain")
                            .current_dir(dir.as_ref().unwrap().path())
                            .output()?
                            .stdout
                            .is_empty())
                        {
                            Warnings::DirtyGitDependency(depname.clone(), checkout.clone()).emit();
                            self.checked_out.insert(
                                depname,
                                config::Dependency::Path {
                                    target: TargetSpec::Wildcard,
                                    path: dir.unwrap().path(),
                                    pass_targets: vec![],
                                },
                            );
                        }
                    }
                }
            }
        }

        // Load the plugin dependencies.
        self.register_dependencies_in_manifest(
            &self.sess.config.plugins,
            &self.sess.manifest.package.name,
            &rt,
            &io,
        )?;

        // Load the dependencies in the root manifest.
        self.register_dependencies_in_manifest(
            &self.sess.manifest.dependencies,
            &self.sess.manifest.package.name,
            &rt,
            &io,
        )?;

        let mut _iteration = 0;
        let mut any_changes = true;
        while any_changes {
            debugln!(
                "resolve: iteration {} table {:#?}",
                _iteration,
                TableDumper(&self.table)
            );
            _iteration += 1;

            // Constraint all dependencies with state `Open` -> `Constrained`.
            self.init()?;

            // Go through each dependency's versions and apply the constraints
            // imposed by the others.
            self.mark()?;

            // Pick a version for each dependency.
            any_changes = self.pick()?;

            // Close the dependency set.
            any_changes |= self.close(&rt, &io)?;
        }
        debugln!("resolve: resolved after {} iterations", _iteration);

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
                            DependencySource::Path(p) => p,
                            _ => unreachable!(),
                        };
                        LockedPackage {
                            revision: None,
                            version: None,
                            source: LockedSource::Path(path),
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
                            DependencySource::Git(u) => u,
                            _ => unreachable!(),
                        };
                        let pick = dep.state.pick().unwrap();
                        let rev = gv.revs[pick.1];
                        let version = gv
                            .versions
                            .iter()
                            .filter(|&&(_, r)| r == rev)
                            .map(|(v, _)| v)
                            .max()
                            .map(|v| v.to_string());
                        LockedPackage {
                            revision: Some(String::from(rev)),
                            version,
                            source: LockedSource::Git(url),
                            dependencies: deps,
                        }
                    }
                };
                Ok((name.to_string(), pkg))
            })
            .collect::<Result<_>>()?;
        Ok(Locked { packages })
    }

    /// Register a dependency in the table.
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
            .or_insert_with(|| DependencyReference::new(dep, versions));
    }

    /// Register a locked dependency in the table.
    fn register_locked_dependency(
        &mut self,
        name: &'ctx str,
        dep: DependencyRef,
        versions: DependencyVersions<'ctx>,
        locked_index: usize,
    ) {
        let entry = self
            .table
            .entry(name)
            .or_insert_with(|| Dependency::new(name));
        entry
            .sources
            .insert(dep, DependencyReference { id: dep, versions });
        entry.state = State::Locked(dep, locked_index);
    }

    /// Register all dependencies in a manifest, ensuring link to sess.
    fn register_dependencies_in_manifest(
        &mut self,
        deps: &'ctx IndexMap<String, config::Dependency>,
        calling_package: &str,
        rt: &Runtime,
        io: &SessionIo<'ctx, 'ctx>,
    ) -> Result<()> {
        // Map the dependencies to unique IDs.
        let names: IndexMap<&str, DependencyRef> = deps
            .iter()
            .map(|(name, dep)| {
                let name = name.as_str();
                let dep = self.checked_out.get(name).unwrap_or(dep);
                let dep = self.sess.config.overrides.get(name).unwrap_or(dep);
                let tmp = (name, self.sess.load_dependency(name, dep, calling_package));
                self.src_ref_map.insert(DependencySource::from(dep), tmp.1);
                tmp
            })
            .collect();
        let ids: IndexSet<DependencyRef> = names.iter().map(|(_, &id)| id).collect();
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
        let versions: IndexMap<_, _> = rt
            .block_on(join_all(versions))
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .collect::<IndexMap<_, _>>();
        // debugln!("resolve: versions {:#?}", versions);

        // Register the versions.
        for (name, id) in names {
            if name == self.sess.manifest.package.name {
                return Err(Error::new(format!(
                    "Please ensure no packages with same name as top package\n\
                    \tCurrently {} is called in {}",
                    name, calling_package
                )));
            }
            if name == calling_package {
                return Err(Error::new(format!(
                    "Please ensure no packages with same name as calling package\n\
                    \tCurrently {} is called in {}",
                    name, calling_package
                )));
            }
            self.register_dependency(name, id, versions[&id].clone());
        }
        Ok(())
    }

    /// Register all dependencies in the lockfile.
    fn register_dependencies_in_lockfile(
        &mut self,
        locked: &'ctx Locked,
        keep_locked: IndexSet<&'ctx String>,
        rt: &Runtime,
        io: &SessionIo<'ctx, 'ctx>,
    ) -> Result<()> {
        // Map the dependencies to unique IDs.
        let names: IndexMap<&str, (DependencyConstraint, DependencySource, Option<&str>)> = locked
            .packages
            .iter()
            .filter_map(|(name, locked_package)| {
                let name = name.as_str();
                debugln!("resolve: registering {} from lockfile", &name);
                let dep = match &locked_package.source {
                    LockedSource::Path(p) => config::Dependency::Path {
                        target: TargetSpec::Wildcard,
                        path: p.clone(),
                        pass_targets: Vec::new(),
                    },
                    LockedSource::Registry(..) => {
                        unreachable!("Registry dependencies not yet supported.");
                    }
                    LockedSource::Git(u) => {
                        if let Some(version) = &locked_package.version {
                            let parsed_version = Version::parse(version).unwrap();
                            config::Dependency::GitVersion {
                                target: TargetSpec::Wildcard,
                                url: u.clone(),
                                version: VersionReq {
                                    comparators: vec![semver::Comparator {
                                        op: semver::Op::Exact,
                                        major: parsed_version.major,
                                        minor: Some(parsed_version.minor),
                                        patch: Some(parsed_version.patch),
                                        pre: parsed_version.pre,
                                    }],
                                },
                                pass_targets: Vec::new(),
                            }
                        } else {
                            config::Dependency::GitRevision {
                                target: TargetSpec::Wildcard,
                                url: u.clone(),
                                rev: match &locked_package.revision {
                                    Some(r) => r.clone(),
                                    None => {
                                        Warnings::NoRevisionInLockFile {
                                            pkg: name.to_string(),
                                        }
                                        .emit();
                                        return None;
                                    }
                                },
                                pass_targets: Vec::new(),
                            }
                        }
                    }
                };
                let hash = match &locked_package.source {
                    LockedSource::Git(..) => locked_package.revision.as_deref(),
                    _ => None,
                };
                // Checked out not indexed yet.
                // Overrides not considered because already locked.
                Some((
                    name,
                    (
                        DependencyConstraint::from(&dep),
                        DependencySource::from(&dep),
                        hash,
                    ),
                ))
            })
            .collect();
        for (name, (cnstr, src, hash)) in names {
            let config_dep: config::Dependency = match src {
                DependencySource::Registry => {
                    unreachable!("Registry dependencies not yet supported.");
                    // TODO should probably be config::Dependeny::Version(vers, str?)
                }
                DependencySource::Path(p) => config::Dependency::Path {
                    target: TargetSpec::Wildcard,
                    path: p,
                    pass_targets: Vec::new(),
                },
                DependencySource::Git(u) => match &cnstr {
                    DependencyConstraint::Version(v) => config::Dependency::GitVersion {
                        target: TargetSpec::Wildcard,
                        url: u,
                        version: v.clone(),
                        pass_targets: Vec::new(),
                    },
                    DependencyConstraint::Revision(r) => config::Dependency::GitRevision {
                        target: TargetSpec::Wildcard,
                        url: u,
                        rev: r.clone(),
                        pass_targets: Vec::new(),
                    },
                    _ => unreachable!(),
                },
            };
            // Map the dependencies to unique IDs.
            let depref = self.sess.load_dependency(name, &config_dep, "");
            self.src_ref_map
                .insert(DependencySource::from(&config_dep), depref);
            self.locked.insert(
                name,
                (
                    cnstr,
                    depref,
                    hash,
                    keep_locked.contains(&&name.to_string()),
                ),
            );
        }

        let depversions: HashMap<_, _> = rt
            .block_on(join_all(keep_locked.iter().map(|dep| {
                let (_, src, hash, _) = self.locked.get(dep.as_str()).unwrap().clone();
                async move {
                    Ok::<(_, _), Error>((
                        dep.to_string(),
                        (hash, io.dependency_versions(src, false).await?, src),
                    ))
                }
            })))
            .into_iter()
            .collect::<Result<HashMap<_, _>>>()?;

        // Keep locked deps locked.
        for dep in keep_locked {
            let (hash, depversion, depref) = depversions.get(dep.as_str()).unwrap().clone();

            let locked_index = match &depversion {
                DependencyVersions::Path => 0,
                DependencyVersions::Registry(_) => {
                    unreachable!("Registry dependencies not yet supported.")
                }
                DependencyVersions::Git(gv) => {
                    match gv.revs.iter().position(|rev| *rev == hash.unwrap()) {
                        Some(index) => index,
                        None => {
                            Warnings::LockedRevisionNotFound {
                                rev: hash.unwrap().to_string(),
                                pkg: dep.to_string(),
                            }
                            .emit();
                            self.locked.get_mut(dep.as_str()).unwrap().3 = false;
                            continue;
                        }
                    }
                }
            };

            self.register_locked_dependency(dep, depref, depversion, locked_index);
        }
        Ok(())
    }

    /// Initialize dependencies with state `Open`.
    ///
    /// This populates the dependency's set of possible versions with all
    /// available versions, such that they may then be constrained.
    fn init(&mut self) -> Result<()> {
        for dep in self.table.values_mut() {
            let mut constraints = match dep.state {
                State::Open => IndexMap::new(),
                State::Constrained(ref constraints) => constraints.clone(),
                State::Locked(_, _) => {
                    // Already initialized.
                    continue;
                }
                State::Picked(_, _, ref constraints) => constraints.clone(),
            };

            for src in dep.sources.values_mut() {
                debugln!("resolve: initializing `{}[{}]`", dep.name, src.id);
                if constraints.contains_key(&src.id) {
                    // Already initialized.
                    debugln!("resolve: `{}[{}]` already initialized", dep.name, src.id);
                    continue;
                }
                let ids: IndexSet<usize> = match src.versions {
                    DependencyVersions::Path => (0..1).collect(),
                    DependencyVersions::Registry(ref _rv) => {
                        return Err(Error::new(format!(
                            "Resolution of registry dependency `{}` not yet implemented",
                            dep.name
                        )));
                    }
                    DependencyVersions::Git(ref gv) => (0..gv.revs.len()).collect(),
                };
                constraints.insert(src.id, ids.clone());
            }
            match dep.state {
                State::Open | State::Constrained(_) => {
                    dep.state = State::Constrained(constraints);
                }
                State::Locked(..) => unreachable!(),
                State::Picked(a, b, _) => {
                    dep.state = State::Picked(a, b, constraints);
                }
            }
        }
        Ok(())
    }

    /// Apply constraints to each dependency's versions.
    fn mark(&mut self) -> Result<()> {
        use std::iter::once;

        // Gather the constraints from the available manifests. Group them by
        // constraint.
        // cons_map: dep_name->(parent_name, constraint, source)
        let cons_map = {
            let mut map = IndexMap::<&str, Vec<(&str, DependencyConstraint, DependencyRef)>>::new();
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
                })
                .map(|(name, pkg_name, dep)| {
                    (
                        name,
                        pkg_name,
                        match self.locked.get(name.as_str()) {
                            Some((cnstr, src, _, true)) => (cnstr.clone(), *src),
                            _ => (
                                DependencyConstraint::from(dep),
                                *self.src_ref_map.get(&DependencySource::from(dep)).unwrap(),
                            ),
                        },
                    )
                });
            for (name, pkg_name, (dep_constr, dep_src)) in dep_iter {
                let v = map.entry(name.as_str()).or_default();
                v.push((pkg_name, dep_constr, dep_src));
            }
            map
        };

        let _src_cons_map = cons_map
            .iter()
            .map(|(name, cons)| {
                (
                    *name,
                    cons.iter()
                        .map(|(pkg_name, con, src)| {
                            (*pkg_name, con.clone(), self.sess.dependency_source(*src))
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<IndexMap<_, _>>();

        debugln!(
            "resolve: gathered constraints {:#?}",
            ConstraintsDumper(&_src_cons_map)
        );

        // Impose the constraints on the dependencies.
        let mut table = mem::take(&mut self.table);
        for (name, cons) in cons_map {
            for (_, con, dsrc) in &cons {
                debugln!("resolve: impose `{}` at `{}` on `{}`", con, dsrc, name);
                let table_item = table.get_mut(name).unwrap();
                self.impose_dep(name, con, dsrc, table_item, &cons)?;
            }
        }
        self.table = table;

        Ok(())
    }

    fn impose_dep(
        &mut self,
        name: &'ctx str,
        con: &DependencyConstraint,
        con_src: &DependencyRef,
        dep: &mut Dependency<'ctx>,
        all_cons: &[(&str, DependencyConstraint, DependencyRef)],
    ) -> Result<()> {
        let con_indices: IndexMap<DependencyRef, IndexSet<usize>> = dep
            .sources
            .iter()
            .map(|(id, src)| {
                match self.req_indices(name, con, src) {
                    Ok(v) => {
                        // TODO attempt refetch (if not already done)
                        if v.is_empty() && id == con_src {
                            let additional_str = if let DependencyConstraint::Version(__) = con {
                                " Ensure git tags are formatted as `vX.Y.Z`.".to_string()
                            } else {
                                "".to_string()
                            };
                            return Err(Error::new(format!(
                                "Dependency `{}` from `{}` cannot satisfy requirement `{}`.{} You may need to run update with --fetch.",
                                name,
                                self.sess.dependency_source(*id),
                                con,
                                additional_str
                            )));
                        }
                        Ok((*id, v))
                    }
                    Err(e) => {
                        if id == con_src {
                            return Err(e);
                        }
                        Warnings::IgnoringError(name.to_string(), self.sess.dependency_source(*con_src).to_string(), e.to_string()).emit();
                        Ok((*id, IndexSet::new()))
                    }
                }
            })
            .collect::<Result<_>>()?;

        let new_ids: Result<IndexMap<_, _>> = match &dep.state {
            State::Open => unreachable!(),
            State::Locked(src, id) => Ok(vec![(*src, IndexSet::from([*id]))].into_iter().collect()),
            State::Constrained(ids) | State::Picked(_, _, ids) => Ok(con_indices
                .into_iter()
                .map(|(id, indices)| {
                    (
                        id,
                        indices
                            .intersection(ids.get(&id).unwrap())
                            .copied()
                            .collect::<IndexSet<usize>>(),
                    )
                })
                .collect()),
        };

        match dep.state {
            State::Open => unreachable!(),
            State::Locked(_, _) => Ok(()),
            State::Constrained(ref mut ids) | State::Picked(_, _, ref mut ids) => match new_ids {
                Err(e) => Err(e),
                Ok(is) => {
                    *ids = is;
                    if ids.values().all(|v| v.is_empty()) {
                        let decision = self.ask_for_decision(dep.name, all_cons, &dep.sources)?;
                        ids.insert(decision.0, decision.1);
                    }
                    Ok(())
                }
            },
        }
    }

    fn ask_for_decision(
        &mut self,
        name: &'ctx str,
        all_cons: &[(&str, DependencyConstraint, DependencyRef)],
        sources: &IndexMap<DependencyRef, DependencyReference<'ctx>>,
    ) -> Result<(DependencyRef, IndexSet<usize>)> {
        let mut msg = format!(
            "Dependency requirements conflict with each other on dependency {}.\n",
            fmt_pkg!(name)
        );
        let mut cons = Vec::new();
        let mut constr_align = String::from("");
        for &(pkg_name, ref con, ref dsrc) in all_cons {
            constr_align.push_str(&format!(
                "\n- package {}\trequires\t{}{}\tat {}",
                fmt_pkg!(pkg_name),
                fmt_version!(con),
                match con {
                    DependencyConstraint::Version(req) => format!(
                        " ({} <= x < {})",
                        fmt_version!(version_req_bottom_bound(req)?.unwrap()),
                        fmt_version!(version_req_top_bound(req)?.unwrap())
                    ),
                    DependencyConstraint::Revision(_) => "".to_string(),
                    DependencyConstraint::Path => "".to_string(),
                },
                fmt_path!(self.sess.dependency_source(*dsrc)),
            ));
            cons.push((con, dsrc));
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", constr_align).unwrap();
        tw.flush().unwrap();
        let _ = write!(
            msg,
            "{}",
            String::from_utf8(tw.into_inner().unwrap()).unwrap()
        );

        cons = cons.into_iter().unique().collect::<Vec<_>>();
        cons.sort_by(|a, b| a.1.cmp(b.1));
        // sort constraint for identical sources
        cons =
            cons.into_iter()
                .chunk_by(|&(_, src)| src)
                .into_iter()
                .flat_map(|(_src, group)| {
                    let mut g: Vec<_> = group.collect();
                    g.sort_by(|a, b| match (a.0, b.0) {
                        (DependencyConstraint::Version(va), DependencyConstraint::Version(vb)) => {
                            if version_req_top_bound(vb).unwrap_or(Some(semver::Version::new(
                                u64::MAX,
                                u64::MAX,
                                u64::MAX,
                            ))) == version_req_top_bound(va)
                                .unwrap_or(Some(semver::Version::new(u64::MAX, u64::MAX, u64::MAX)))
                            {
                                return version_req_bottom_bound(vb)
                                    .unwrap_or(Some(semver::Version::new(0, 0, 0)))
                                    .cmp(
                                        &version_req_bottom_bound(va)
                                            .unwrap_or(Some(semver::Version::new(0, 0, 0))),
                                    );
                            }
                            version_req_top_bound(vb)
                                .unwrap_or(Some(semver::Version::new(u64::MAX, u64::MAX, u64::MAX)))
                                .cmp(&version_req_top_bound(va).unwrap_or(Some(
                                    semver::Version::new(u64::MAX, u64::MAX, u64::MAX),
                                )))
                        }
                        (
                            DependencyConstraint::Revision(ra),
                            DependencyConstraint::Revision(rb),
                        ) => ra.cmp(rb),
                        (DependencyConstraint::Path, DependencyConstraint::Path) => {
                            std::cmp::Ordering::Equal
                        }
                        (DependencyConstraint::Path, _) => std::cmp::Ordering::Greater,
                        (_, DependencyConstraint::Path) => std::cmp::Ordering::Less,
                        (DependencyConstraint::Version(_), DependencyConstraint::Revision(_)) => {
                            std::cmp::Ordering::Greater
                        }
                        (DependencyConstraint::Revision(_), DependencyConstraint::Version(_)) => {
                            std::cmp::Ordering::Less
                        }
                    });
                    g
                })
                .collect::<Vec<_>>();
        if let Some((cnstr, src, _, _)) = self.locked.get(name) {
            let _ = write!(
                msg,
                "\n\nThe previous lockfile required {} at {}.",
                fmt_version!(cnstr),
                fmt_path!(self.sess.dependency_source(*src))
            );
            cons.insert(0, (cnstr, src));
        }
        // Let user resolve conflict if both stderr and stdin go to a TTY.
        if std::io::stderr().is_terminal() && std::io::stdin().is_terminal() {
            let decision = if let Some(d) = self.decisions.get(name) {
                d.clone()
            } else {
                eprintln!(
                    "{}\n\nTo resolve this conflict manually, \
                        select a revision for {} among:",
                    msg,
                    fmt_pkg!(name)
                );

                let mut tw2 = TabWriter::new(vec![]);
                for (idx, e) in cons.iter().enumerate() {
                    writeln!(
                        &mut tw2,
                        "{})\t{}\tat {}",
                        idx,
                        fmt_version!(e.0),
                        fmt_path!(self.sess.dependency_source(*e.1))
                    )
                    .unwrap();
                }
                tw2.flush().unwrap();
                eprintln!("{}", String::from_utf8(tw2.into_inner().unwrap()).unwrap());

                loop {
                    eprint!("Enter a number or hit enter to abort: ");
                    io::stdout().flush().unwrap();
                    let mut buffer = String::new();
                    io::stdin().read_line(&mut buffer).unwrap();
                    if buffer.starts_with('\n') {
                        break Err(Error::new(format!(
                            "Dependency requirements conflict with each other on dependency `{}`. Manual resolution aborted.\n",
                            name
                        )));
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
                    self.decisions
                        .insert(name, (decision.0.clone(), *decision.1));
                    break Ok((decision.0.clone(), *decision.1));
                }?
            };
            match self.req_indices(name, &decision.0, sources.get(&decision.1).unwrap()) {
                Ok(v) => Ok((decision.1, v)),
                Err(e) => Err(e),
            }
        } else {
            Err(Error::new(msg))
        }
    }

    fn req_indices(
        &self,
        name: &str,
        con: &DependencyConstraint,
        src: &DependencyReference<'ctx>,
    ) -> Result<IndexSet<usize>> {
        use self::DependencyConstraint as DepCon;
        use self::DependencyVersions as DepVer;
        match (con, &src.versions) {
            (&DepCon::Path, &DepVer::Path) => Ok(IndexSet::from([0])),
            (DepCon::Version(con), DepVer::Git(gv)) => {
                // TODO: Move this outside somewhere. Very inefficient!
                let hash_ids: IndexMap<&str, usize> = gv
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
                            if con.matches(v) { Some((v, h)) } else { None }
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
                Ok(revs)
            }
            (DepCon::Revision(con), DepVer::Git(gv)) => {
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
                Ok(revs)
            }
            (DepCon::Version(_con), DepVer::Registry(_rv)) => Err(Error::new(format!(
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

    /// Pick a version for each dependency.
    fn pick(&mut self) -> Result<bool> {
        let mut any_changes = false;
        let mut open_pending = IndexSet::<&'ctx str>::new();
        for dep in self.table.values_mut() {
            dep.state = match &dep.state {
                State::Open => unreachable!(),
                State::Locked(dref, id) => State::Locked(*dref, *id),
                State::Constrained(map) => {
                    any_changes = true;
                    let src_map = dep
                        .sources
                        .values()
                        .map(|src| match src.versions {
                            DependencyVersions::Path => {
                                debugln!(
                                    "resolve: selecting path version for `{}`[{}]",
                                    dep.name,
                                    src.id
                                );
                                Ok(Some((src.id, 0, IndexSet::from([0]))))
                            }
                            DependencyVersions::Git(..) => {
                                debugln!(
                                    "resolve: selecting git version for `{}`[{}]",
                                    dep.name,
                                    src.id
                                );
                                Ok(match map[&src.id].is_empty() {
                                    true => None,
                                    false => Some((
                                        src.id,
                                        map[&src.id].first().copied().unwrap(),
                                        map[&src.id].clone(),
                                    )),
                                })
                            }
                            DependencyVersions::Registry(..) => Err(Error::new(format!(
                                "Version picking for registry dependency `{}` not yet implemented",
                                dep.name
                            ))),
                        })
                        .collect::<Result<Vec<Option<(DependencyRef, usize, IndexSet<_>)>>>>()?;
                    let src_map = src_map
                        .into_iter()
                        .flatten()
                        .map(|(a, b, c)| (a, (b, c)))
                        .collect::<IndexMap<DependencyRef, (usize, IndexSet<_>)>>();
                    // TODO: pick among possible sources.
                    match src_map.first() {
                        Some(first) => {
                            debugln!("resolve: picking ref {} for `{}`", first.0, dep.name);
                            State::Picked(
                                *first.0,
                                first.1.0,
                                src_map.into_iter().map(|(k, (_, ids))| (k, ids)).collect(),
                            )
                        }
                        None => {
                            return Err(Error::new(format!(
                                "No versions available for `{}`. This may be due to a conflict in the dependency requirements.",
                                dep.name
                            )));
                        }
                    }
                }
                State::Picked(dref, id, map) => {
                    if !dep.sources[dref].is_path() && !map[dref].contains(id) {
                        debugln!(
                            "resolve: picked version for `{}`[{}] no longer valid, resetting",
                            dep.name,
                            dref
                        );
                        if let Some(manifest) = dep.manifest {
                            open_pending.extend(manifest.dependencies.keys().map(String::as_str));
                        }
                        any_changes = true;
                        State::Open
                    } else {
                        // Keep the picked state.
                        State::Picked(*dref, *id, map.clone())
                    }
                }
            };
        }

        // Recursively open up dependencies.
        while !open_pending.is_empty() {
            use std::mem::swap;
            let mut open = IndexSet::new();
            swap(&mut open_pending, &mut open);
            for dep_name in open {
                debugln!("resolve: resetting `{}`", dep_name);
                let dep = self.table.get_mut(dep_name).unwrap();
                if dep.state.is_open() {
                    any_changes = true;
                    if let Some(manifest) = dep.manifest {
                        open_pending.extend(manifest.dependencies.keys().map(String::as_str));
                    }
                    dep.state = State::Open;
                }
            }
        }

        Ok(any_changes)
    }

    /// Close the set of dependencies.
    fn close(&mut self, rt: &Runtime, io: &SessionIo<'ctx, 'ctx>) -> Result<bool> {
        debugln!("resolve: computing closure over dependencies");
        let manifests: Vec<(&str, Option<&Manifest>)> = {
            let mut sub_deps = Vec::new();
            for dep in self.table.values() {
                let version = match dep.pick() {
                    Some(v) => v,
                    None => continue,
                };
                let src = dep.source();
                let manifest = io.dependency_manifest_version(src.id, version);
                sub_deps.push(async move { manifest.await.map(move |m| (dep.name, m)) });
            }
            rt.block_on(join_all(sub_deps))
                .into_iter()
                .collect::<Result<Vec<(&str, Option<&Manifest>)>>>()?
        };
        let mut any_changes = false;
        for (name, manifest) in manifests {
            if let Some(m) = manifest {
                debugln!("resolve: for `{}` loaded manifest {:#?}", name, m);
                self.register_dependencies_in_manifest(&m.dependencies, &m.package.name, rt, io)?;
            }
            let existing = &mut self.table.get_mut(name).unwrap().manifest;
            any_changes |= existing.is_none() && manifest.is_some();
            *existing = manifest;
        }
        Ok(any_changes)
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
#[derive(Debug, Default)]
struct Dependency<'ctx> {
    /// The name of the dependency.
    name: &'ctx str,
    /// The set of sources for this dependency.
    sources: IndexMap<DependencyRef, DependencyReference<'ctx>>,
    /// The picked manifest for this dependency.
    manifest: Option<&'ctx config::Manifest>,
    /// The current resolution state.
    state: State,
}

impl<'ctx> Dependency<'ctx> {
    /// Create a new dependency.
    fn new(name: &'ctx str) -> Dependency<'ctx> {
        Dependency {
            name,
            ..Default::default()
        }
    }

    /// Return the main source for this dependency.
    fn source(&self) -> &DependencyReference<'ctx> {
        if let State::Locked(dref, _) = self.state {
            return &self.sources[&dref];
        }
        if let State::Picked(dref, _, _) = self.state {
            return &self.sources[&dref];
        }
        self.sources.first().unwrap().1
    }

    /// Return the picked version, if any.
    ///
    /// In case the state is `Locked` or `Picked`, returns the version that was
    /// picked. Otherwise returns `None`.
    fn pick(&self) -> Option<DependencyVersion<'ctx>> {
        match self.state {
            State::Open | State::Constrained(..) => None,
            State::Locked(dref, id) | State::Picked(dref, id, _) => {
                match self.sources[&dref].versions {
                    DependencyVersions::Path => Some(DependencyVersion::Path),
                    DependencyVersions::Registry(ref _rv) => None,
                    DependencyVersions::Git(ref gv) => Some(DependencyVersion::Git(gv.revs[id])),
                }
            }
        }
    }
}

/// A source for a dependency.
///
/// A dependency may have multiple sources. See `Dependency`.
#[derive(Debug)]
struct DependencyReference<'ctx> {
    /// The ID of this dependency.
    id: DependencyRef,
    /// The available versions of the dependency.
    versions: DependencyVersions<'ctx>,
}

impl<'ctx> DependencyReference<'ctx> {
    /// Create a new dependency source.
    fn new(id: DependencyRef, versions: DependencyVersions<'ctx>) -> DependencyReference<'ctx> {
        DependencyReference { id, versions }
    }

    /// Check whether this is a path dependency.
    fn is_path(&self) -> bool {
        matches!(self.versions, DependencyVersions::Path)
    }
}

#[derive(Debug, Default)]
enum State {
    /// The dependency has never been seen before and is not constrained.
    #[default]
    Open,
    /// The dependency has been locked in the lockfile.
    Locked(DependencyRef, usize),
    /// The dependency may assume any of the listed refs and their listed versions.
    Constrained(IndexMap<DependencyRef, IndexSet<usize>>),
    /// The dependency had a ref and version picked.
    Picked(
        DependencyRef,
        usize,
        IndexMap<DependencyRef, IndexSet<usize>>,
    ),
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
    fn pick(&self) -> Option<(DependencyRef, usize)> {
        match *self {
            State::Locked(i, j) | State::Picked(i, j, _) => Some((i, j)),
            _ => None,
        }
    }
}

#[allow(dead_code)]
struct TableDumper<'a>(&'a IndexMap<&'a str, Dependency<'a>>);

impl fmt::Debug for TableDumper<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        write!(f, "{{")?;
        for name in names {
            let dep = self.0.get(name).unwrap();
            write!(f, "\n    \"{}\":", name)?;
            for (&id, _) in &dep.sources {
                write!(f, "\n        [{}]:", id)?;
            }
            match dep.state {
                State::Open => write!(f, " open")?,
                State::Locked(refr, idx) => write!(f, " locked {} {}", refr, idx)?,
                State::Constrained(ref idcs) => write!(f, " {} possible", idcs.len())?,
                State::Picked(refr, idx, ref idcs) => write!(
                    f,
                    " picked {} #{} out of {} possible",
                    refr,
                    idx,
                    idcs.values().map(|v| v.len()).sum::<usize>()
                )?,
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}

#[allow(dead_code)]
struct ConstraintsDumper<'a>(
    &'a IndexMap<&'a str, Vec<(&'a str, DependencyConstraint, DependencySource)>>,
);

impl fmt::Debug for ConstraintsDumper<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        write!(f, "{{")?;
        for name in names {
            let cons = self.0.get(name).unwrap();
            write!(f, "\n    \"{}\":", name)?;
            for &(pkg_name, ref con, ref src) in cons {
                write!(f, " {} at {} ({});", con, src, pkg_name)?;
            }
        }
        write!(f, "\n}}")?;
        Ok(())
    }
}
