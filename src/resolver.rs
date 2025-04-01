// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A dependency resolver.

#![deny(missing_docs)]

use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::mem;
use std::path::PathBuf;
use std::process::Command as SysCommand;

use futures::future::join_all;
use indexmap::{IndexMap, IndexSet};
use is_terminal::IsTerminal;
use itertools::Itertools;
use semver::{Version, VersionReq};
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::config::{self, Locked, LockedPackage, LockedSource, Manifest};
use crate::error::*;
use crate::git::Git;
use crate::sess::{
    DependencyConstraint, DependencyRef, DependencySource, DependencyVersion, DependencyVersions,
    Session, SessionIo,
};

/// A dependency resolver.
pub struct DependencyResolver<'ctx> {
    /// The session within which resolution occurs.
    sess: &'ctx Session<'ctx>,
    /// The version table which is used to perform resolution.
    table: IndexMap<&'ctx str, Dependency<'ctx>>,
    /// A cache of decisions made by the user during the resolution.
    decisions: IndexMap<&'ctx str, (DependencyConstraint, DependencySource)>,
    /// Checkout Directory overrides in case checkout_dir is defined and contains unmodified folders.
    checked_out: IndexMap<String, config::Dependency>,
    /// Lockfile data.
    locked: IndexMap<&'ctx str, (DependencyConstraint, DependencySource, Option<&'ctx str>)>,
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
    pub fn resolve(
        mut self,
        existing: Option<&'ctx Locked>,
        ignore_checkout: bool,
        keep_locked: Vec<&'ctx String>,
    ) -> Result<Locked> {
        let rt = Runtime::new()?;
        let io = SessionIo::new(self.sess);

        // Load the dependencies in the lockfile.
        if existing.is_some() {
            debugln!("resolve: registering lockfile dependencies");
            self.register_dependencies_in_lockfile(existing.unwrap(), keep_locked, &rt, &io)?;
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

                    // TODO this compares to lockfile's url, but ideally we check if the new url is the same, but we don't know that one yet...
                    let full_url = self.locked.get(depname.as_str()).map(|(_, src, _)| src);
                    let url_correct = match full_url {
                        Some(DependencySource::Git(u)) => {
                            let dep_path = dir.as_ref().unwrap().path();
                            let dep_git = Git::new(dep_path.as_path(), &self.sess.config.git);
                            let output = rt
                                .block_on(dep_git.remote_url("origin"))
                                .unwrap_or("".to_string());

                            if let Some(remote_name) = PathBuf::from(output).file_name() {
                                remote_name.to_str().unwrap() == self.sess.git_db_name(&depname, u)
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };

                    // Only act if the avoiding flag is not set and any of the following match
                    //  - the dependency is not a git repo
                    //  - the dependency's remote url is not correct
                    //  - the dependency is not in a clean state (i.e., was modified)
                    if !ignore_checkout {
                        if !is_git_repo {
                            warnln!("Dependency `{}` in checkout_dir `{}` is not a git repository. Setting as path dependency.\n\
                                    \tRun `bender update --ignore-checkout-dir` to overwrite this at your own risk.",
                                dir.as_ref().unwrap().path().file_name().unwrap().to_str().unwrap(),
                                &checkout.display());
                            self.checked_out
                                .insert(depname, config::Dependency::Path(dir.unwrap().path()));
                        } else if !url_correct {
                            warnln!("Dependency `{}` in checkout_dir `{}` is linked to the wrong upstream. Setting as path dependency.\n\
                                    \tRun `bender update --ignore-checkout-dir` to overwrite this at your own risk.",
                                dir.as_ref().unwrap().path().file_name().unwrap().to_str().unwrap(),
                                &checkout.display());
                            self.checked_out
                                .insert(depname, config::Dependency::Path(dir.unwrap().path()));
                        } else if !(SysCommand::new(&self.sess.config.git) // If not in a clean state
                            .arg("status")
                            .arg("--porcelain")
                            .current_dir(dir.as_ref().unwrap().path())
                            .output()?
                            .stdout
                            .is_empty())
                        {
                            warnln!("Dependency `{}` in checkout_dir `{}` is not in a clean state. Setting as path dependency.\n\
                                    \tRun `bender update --ignore-checkout-dir` to overwrite this at your own risk.",
                                dir.as_ref().unwrap().path().file_name().unwrap().to_str().unwrap(),
                                &checkout.display());
                            self.checked_out
                                .insert(depname, config::Dependency::Path(dir.unwrap().path()));
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
                        let pick = src.state.pick().unwrap();
                        let rev = gv.revs[pick];
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
        entry.sources.insert(
            dep,
            DependencyReference {
                id: dep,
                versions,
                pick: None,
                options: None,
                state: State::Locked(locked_index),
            },
        );
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
                (name, self.sess.load_dependency(name, dep, calling_package))
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
        keep_locked: Vec<&'ctx String>,
        rt: &Runtime,
        io: &SessionIo<'ctx, 'ctx>,
    ) -> Result<()> {
        // Map the dependencies to unique IDs.
        let names: IndexMap<&str, (DependencyConstraint, DependencySource, Option<&str>)> = locked
            .packages
            .iter()
            .map(|(name, locked_package)| {
                let name = name.as_str();
                debugln!("resolve: registering {} from lockfile", &name);
                let dep = match &locked_package.source {
                    LockedSource::Path(p) => config::Dependency::Path(p.clone()),
                    LockedSource::Registry(..) => {
                        unreachable!("Registry dependencies not yet supported.");
                    }
                    LockedSource::Git(u) => {
                        if let Some(version) = &locked_package.version {
                            let parsed_version = Version::parse(version).unwrap();
                            config::Dependency::GitVersion(
                                u.clone(),
                                VersionReq {
                                    comparators: vec![semver::Comparator {
                                        op: semver::Op::Exact,
                                        major: parsed_version.major,
                                        minor: Some(parsed_version.minor),
                                        patch: Some(parsed_version.patch),
                                        pre: parsed_version.pre,
                                    }],
                                },
                            )
                        } else {
                            config::Dependency::GitRevision(
                                u.clone(),
                                locked_package.revision.clone().unwrap(),
                            )
                        }
                    }
                };
                let hash = match &locked_package.source {
                    LockedSource::Git(..) => locked_package.revision.as_deref(),
                    _ => None,
                };
                // Checked out not indexed yet.
                // Overrides not considered because already locked.
                (
                    name,
                    (
                        DependencyConstraint::from(&dep),
                        DependencySource::from(&dep),
                        hash,
                    ),
                )
            })
            .collect();
        for (name, (cnstr, src, hash)) in names {
            self.locked.insert(name, (cnstr, src, hash));
        }

        // Keep locked deps locked.
        for dep in keep_locked {
            let (cnstr, src, hash) = self.locked.get(dep.as_str()).unwrap().clone();
            let config_dep: config::Dependency = match src {
                DependencySource::Registry => {
                    unreachable!("Registry dependencies not yet supported.");
                    // TODO should probably be config::Dependeny::Version(vers, str?)
                }
                DependencySource::Path(p) => config::Dependency::Path(p),
                DependencySource::Git(u) => match cnstr {
                    DependencyConstraint::Version(v) => config::Dependency::GitVersion(u, v),
                    DependencyConstraint::Revision(r) => config::Dependency::GitRevision(u, r),
                    _ => unreachable!(),
                },
            };
            // Map the dependencies to unique IDs.
            let depref = self.sess.load_dependency(dep, &config_dep, "");
            let depversions: DependencyVersions =
                rt.block_on(io.dependency_versions(depref, false))?;

            let locked_index = match &depversions {
                DependencyVersions::Path => 0,
                DependencyVersions::Registry(_) => {
                    unreachable!("Registry dependencies not yet supported.")
                }
                DependencyVersions::Git(gv) => gv
                    .revs
                    .iter()
                    .position(|rev| *rev == hash.unwrap())
                    .unwrap(),
            };

            self.register_locked_dependency(dep, depref, depversions, locked_index);
            println!("Locked {} at {}", dep, locked_index);
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
                            "Resolution of registry dependency `{}` not yet implemented",
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
            let mut map =
                IndexMap::<&str, Vec<(&str, DependencyConstraint, DependencySource)>>::new();
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
                v.push((
                    pkg_name,
                    DependencyConstraint::from(dep),
                    DependencySource::from(dep),
                ));
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
            for (_, con, dsrc) in &cons {
                debugln!("resolve: impose `{}` at `{}` on `{}`", con, dsrc, name);
                let mut count = 0;
                let table_item = table.get_mut(name).unwrap();
                for (id, src) in table_item.sources.iter_mut() {
                    if self.sess.dependency_source(*id) == *dsrc {
                        self.impose(name, con, src, &cons, rt, io)?;
                        count += 1;
                        table_item.picked_source = Some(*id);
                    } else {
                        match self.impose(name, con, src, &cons, rt, io) {
                            Ok(_) => {
                                count += 1;
                                table_item.picked_source = Some(*id);
                            }
                            Err(cause) => {
                                warnln!("{}", cause);
                            }
                        };
                    }
                }
                debugln!(
                    "resolve: imposed {} constraint out of {} on `{}`",
                    count,
                    table.get_mut(name).unwrap().sources.len(),
                    name
                );
                if count == 0 {
                    return Err(Error::new(format!(
                        "Constraint matching failed for `{}` at `{}`.",
                        name, dsrc
                    )));
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
        src: &DependencyReference<'ctx>,
    ) -> Result<Option<indexmap::IndexSet<usize>>> {
        use self::DependencyConstraint as DepCon;
        use self::DependencyVersions as DepVer;
        match (con, &src.versions) {
            (&DepCon::Path, &DepVer::Path) => Ok(None),
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
                // debugln!("resolve: `{}` matches revision `{}` for revs {:?}", name, con, revs);
                Ok(Some(revs))
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

    /// Impose a constraint on a dependency.
    fn impose(
        &mut self,
        name: &'ctx str,
        con: &DependencyConstraint,
        src: &mut DependencyReference<'ctx>,
        all_cons: &[(&str, DependencyConstraint, DependencySource)],
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
            State::Locked(id) => Ok(vec![id].into_iter().collect()),
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
                    let mut constr_align = String::from("");
                    for &(pkg_name, ref con, ref dsrc) in all_cons {
                        constr_align.push_str(&format!(
                            "\n- package `{}`\trequires\t`{}` at `{}`",
                            pkg_name, con, dsrc
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

                    cons = cons.into_iter().unique().collect();
                    if let Some((cnstr, src, _)) = self.locked.get(name) {
                        let _ = write!(
                            msg,
                            "\n\nThe previous lockfile required `{}` at `{}`",
                            cnstr, src
                        );
                    }
                    // Let user resolve conflict if both stderr and stdin go to a TTY.
                    if std::io::stderr().is_terminal() && std::io::stdin().is_terminal() {
                        let decision = if let Some(d) = self.decisions.get(name) {
                            d.0.clone()
                        } else {
                            eprintln!(
                                "{}\n\nTo resolve this conflict manually, \
                                 select a revision for `{}` among:",
                                msg, name
                            );
                            for (idx, e) in cons.iter().enumerate() {
                                eprintln!("{}) `{}` at `{}`", idx, e.0, e.1);
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
                                self.decisions
                                    .insert(name, (decision.0.clone(), decision.1.clone()));
                                break Ok(decision.0.clone());
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
            State::Locked(_) => Ok(()),
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
        let mut open_pending = IndexSet::<&'ctx str>::new();
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
                                return Err(Error::new(format!("Version picking for registry dependency `{}` not yet implemented", dep.name)));
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
            let mut open = IndexSet::new();
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
                self.register_dependencies_in_manifest(&m.dependencies, &m.package.name, rt, io)?;
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
    /// The picked ID among the sources.
    picked_source: Option<DependencyRef>,
    /// The set of sources for this dependency.
    sources: IndexMap<DependencyRef, DependencyReference<'ctx>>,
    /// The picked manifest for this dependency.
    manifest: Option<&'ctx config::Manifest>,
}

impl<'ctx> Dependency<'ctx> {
    /// Create a new dependency.
    fn new(name: &'ctx str) -> Dependency<'ctx> {
        Dependency {
            name,
            picked_source: None,
            sources: IndexMap::new(),
            manifest: None,
        }
    }

    /// Return the main source for this dependency.
    fn source(&self) -> &DependencyReference<'ctx> {
        for src in self.sources.values() {
            if let State::Locked(_) = src.state {
                return src;
            }
        }
        match self.picked_source {
            Some(id) => &self.sources[&id],
            None => {
                let min = self.sources.keys().min().unwrap();
                &self.sources[min]
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
    /// The currently picked version.
    pick: Option<usize>,
    /// The available version options. These are indices into `versions`.
    options: Option<IndexSet<usize>>,
    /// The current resolution state.
    state: State,
}

impl<'ctx> DependencyReference<'ctx> {
    /// Create a new dependency source.
    fn new(id: DependencyRef, versions: DependencyVersions<'ctx>) -> DependencyReference<'ctx> {
        DependencyReference {
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

struct TableDumper<'a>(&'a IndexMap<&'a str, Dependency<'a>>);

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

struct ConstraintsDumper<'a>(
    &'a IndexMap<&'a str, Vec<(&'a str, DependencyConstraint, DependencySource)>>,
);

impl<'a> fmt::Debug for ConstraintsDumper<'a> {
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
