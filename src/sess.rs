// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A command line session.

#![deny(missing_docs)]

use std;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::canonicalize;
use std::mem::swap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use futures::future::{self, join_all};
use futures::Future;
use semver;
use serde_yaml;
use tokio_core::reactor::Handle;
use typed_arena::Arena;

use crate::cli::read_manifest;
use crate::config::Validate;
use crate::config::{self, Config, Manifest};
use crate::error::*;
use crate::future_throttle::FutureThrottle;
use crate::git::Git;
use crate::src::SourceGroup;
use crate::target::TargetSpec;
use crate::util::try_modification_time;

/// A session on the command line.
///
/// Contains all the information that is iteratively being gathered and
/// generated as a command on the command line is executed.
pub struct Session<'ctx> {
    /// The path of the package within which the tool was executed.
    pub root: &'ctx Path,
    /// The manifest of the root package.
    pub manifest: &'ctx Manifest,
    /// The tool configuration.
    pub config: &'ctx Config,
    /// The arenas into which we allocate various things that need to live as
    /// long as the session.
    arenas: &'ctx SessionArenas,
    /// The manifest modification time.
    pub manifest_mtime: Option<SystemTime>,
    /// Some statistics about the session.
    stats: SessionStatistics,
    /// The dependency table.
    deps: Mutex<DependencyTable<'ctx>>,
    /// The internalized paths.
    paths: Mutex<HashSet<&'ctx Path>>,
    /// The internalized strings.
    strings: Mutex<HashSet<&'ctx str>>,
    /// The package name table.
    names: Mutex<HashMap<String, DependencyRef>>,
    /// The dependency graph.
    graph: Mutex<Arc<HashMap<DependencyRef, HashSet<DependencyRef>>>>,
    /// The topologically sorted list of packages.
    pkgs: Mutex<Arc<Vec<Vec<DependencyRef>>>>,
    /// The source file manifest.
    sources: Mutex<Option<SourceGroup<'ctx>>>,
    /// The plugins declared by packages.
    plugins: Mutex<Option<&'ctx Plugins>>,
    /// The session cache.
    pub cache: SessionCache<'ctx>,
    /// A throttle for futures performing git network operations.
    git_throttle: FutureThrottle,
}

impl<'sess, 'ctx: 'sess> Session<'ctx> {
    /// Create a new session.
    pub fn new(
        root: &'ctx Path,
        manifest: &'ctx Manifest,
        config: &'ctx Config,
        arenas: &'ctx SessionArenas,
    ) -> Session<'ctx> {
        Session {
            root: root,
            manifest: manifest,
            config: config,
            arenas: arenas,
            manifest_mtime: try_modification_time(root.join("Bender.yml")),
            stats: Default::default(),
            deps: Mutex::new(DependencyTable::new()),
            paths: Mutex::new(HashSet::new()),
            strings: Mutex::new(HashSet::new()),
            names: Mutex::new(HashMap::new()),
            graph: Mutex::new(Arc::new(HashMap::new())),
            pkgs: Mutex::new(Arc::new(Vec::new())),
            sources: Mutex::new(None),
            plugins: Mutex::new(None),
            cache: Default::default(),
            git_throttle: FutureThrottle::new(8),
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
        manifest: &config::Manifest,
    ) -> DependencyRef {
        debugln!(
            "sess: load dependency `{}` as {:?} for package `{}`",
            name,
            cfg,
            manifest.package.name
        );
        let src = match *cfg {
            config::Dependency::Version(_) => DependencySource::Registry,
            config::Dependency::Path(ref p) => DependencySource::Path(p.clone()),
            config::Dependency::GitRevision(ref g, _)
            | config::Dependency::GitVersion(ref g, _) => DependencySource::Git(g.clone()),
        };
        self.deps
            .lock()
            .unwrap()
            .add(self.intern_dependency_entry(DependencyEntry {
                name: name.into(),
                source: src,
                revision: None,
                version: None,
            }))
    }

    /// Load a lock file.
    ///
    /// This internalizes the dependency sources, i.e. assigns `DependencyRef`
    /// objects to them, and generates a nametable.
    pub fn load_locked(&self, locked: &config::Locked) {
        let mut deps = self.deps.lock().unwrap();
        let mut names = HashMap::new();
        let mut graph_names = HashMap::new();
        for (name, pkg) in &locked.packages {
            let src = match pkg.source {
                config::LockedSource::Path(ref path) => DependencySource::Path(path.clone()),
                config::LockedSource::Git(ref url) => DependencySource::Git(url.clone()),
                config::LockedSource::Registry(ref _ver) => DependencySource::Registry,
            };
            let id = deps.add(
                self.intern_dependency_entry(DependencyEntry {
                    name: name.clone(),
                    source: src,
                    revision: pkg.revision.clone(),
                    version: pkg
                        .version
                        .as_ref()
                        .map(|s| semver::Version::parse(&s).unwrap()),
                }),
            );
            graph_names.insert(id, &pkg.dependencies);
            names.insert(name.clone(), id);
        }
        drop(deps);

        // Translate the name-based graph into an ID-based graph.
        let graph: HashMap<DependencyRef, HashSet<DependencyRef>> = graph_names
            .into_iter()
            .map(|(k, v)| (k, v.iter().map(|name| names[name]).collect()))
            .collect();

        // Determine the topological ordering of the packages.
        let pkgs = {
            // Assign a rank to each package. A package's rank will be strictly
            // smaller than the rank of all its dependencies. This yields a
            // topological ordering.
            let mut ranks: HashMap<DependencyRef, usize> =
                graph.keys().map(|&id| (id, 0)).collect();
            let mut pending = HashSet::new();
            pending.extend(self.manifest.dependencies.keys().map(|name| names[name]));
            while !pending.is_empty() {
                let mut current_pending = HashSet::new();
                swap(&mut pending, &mut current_pending);
                for id in current_pending {
                    let min_dep_rank = ranks[&id] + 1;
                    for &dep_id in &graph[&id] {
                        if ranks[&dep_id] <= min_dep_rank {
                            ranks.insert(dep_id, min_dep_rank);
                            pending.insert(dep_id);
                        }
                    }
                }
            }
            debugln!("sess: topological ranks {:#?}", ranks);

            // Group together packages with the same rank, to build the final
            // ordering.
            let num_ranks = ranks.values().map(|v| v + 1).max().unwrap_or(0);
            let pkgs: Vec<Vec<DependencyRef>> = (0..num_ranks)
                .rev()
                .map(|rank| {
                    let mut v: Vec<_> = ranks
                        .iter()
                        .filter_map(|(&k, &v)| if v == rank { Some(k) } else { None })
                        .collect();
                    v.sort_by(|&a, &b| self.dependency_name(a).cmp(self.dependency_name(b)));
                    v
                })
                .collect();
            pkgs
        };

        debugln!("sess: names {:?}", names);
        debugln!("sess: graph {:?}", graph);
        debugln!("sess: pkgs {:?}", pkgs);

        *self.names.lock().unwrap() = names;
        *self.graph.lock().unwrap() = Arc::new(graph);
        *self.pkgs.lock().unwrap() = Arc::new(pkgs);
    }

    /// Obtain information on a dependency.
    pub fn dependency(&self, dep: DependencyRef) -> &'ctx DependencyEntry {
        // TODO: Don't make any clones! Use an arena instead.
        self.deps.lock().unwrap().list[dep.0]
    }

    /// Determine the name of a dependency.
    pub fn dependency_name(&self, dep: DependencyRef) -> &'ctx str {
        self.intern_string(self.deps.lock().unwrap().list[dep.0].name.as_str())
    }

    /// Determine the source of a dependency.
    pub fn dependency_source(&self, dep: DependencyRef) -> DependencySource {
        // TODO: Don't make any clones! Use an arena instead.
        self.deps.lock().unwrap().list[dep.0].source.clone()
    }

    /// Resolve a dependency name to a reference.
    ///
    /// Returns an error if the dependency does not exist.
    pub fn dependency_with_name(&self, name: &str) -> Result<DependencyRef> {
        let result = self.names.lock().unwrap().get(name).map(|id| *id);
        match result {
            Some(id) => Ok(id),
            None => Err(Error::new(format!(
                "Dependency `{}` does not exist. Did you forget to add it to the manifest?",
                name
            ))),
        }
    }

    /// Internalize a path.
    ///
    /// This allocates the path in the arena and returns a reference to it whose
    /// lifetime is bound to the arena rather than this `Session`. Useful to
    /// obtain a lightweight pointer to a path that is guaranteed to outlive the
    /// `Session`.
    pub fn intern_path<T>(&self, path: T) -> &'ctx Path
    where
        T: Into<PathBuf> + AsRef<Path>,
    {
        let mut paths = self.paths.lock().unwrap();
        if let Some(&p) = paths.get(path.as_ref()) {
            p
        } else {
            let p = self.arenas.path.alloc(path.into());
            paths.insert(p);
            p
        }
    }

    /// Internalize a string.
    ///
    /// This allocates the string in the arena and returns a reference to it
    /// whose lifetime is bound to the arena rather than this `Session`. Useful
    /// to obtain a lightweight pointer to a string that is guaranteed to
    /// outlive the `Session`.
    pub fn intern_string<T>(&self, string: T) -> &'ctx str
    where
        T: Into<String> + AsRef<str>,
    {
        let mut strings = self.strings.lock().unwrap();
        if let Some(&s) = strings.get(string.as_ref()) {
            s
        } else {
            let s = self.arenas.string.alloc(string.into());
            strings.insert(s);
            s
        }
    }

    /// Internalize a manifest.
    ///
    /// This allocates the manifest in the arena and returns a reference to it
    /// whose lifetime is bound to the arena rather than this `Session`. Useful
    /// to obtain a lightweight pointer to a manifest that is guaranteed to
    /// outlive the `Session`.
    pub fn intern_manifest<T>(&self, manifest: T) -> &'ctx Manifest
    where
        T: Into<Manifest>,
    {
        self.arenas.manifest.alloc(manifest.into())
    }

    /// Internalize a dependency entry.
    pub fn intern_dependency_entry(&self, entry: DependencyEntry) -> &'ctx DependencyEntry {
        self.arenas.dependency_entry.alloc(entry)
    }

    /// Access the package dependency graph.
    pub fn graph(&self) -> Arc<HashMap<DependencyRef, HashSet<DependencyRef>>> {
        self.graph.lock().unwrap().clone()
    }

    /// Access the topological sorting of the packages.
    pub fn packages(&self) -> Arc<Vec<Vec<DependencyRef>>> {
        self.pkgs.lock().unwrap().clone()
    }

    /// Load the sources in a manifest into a source group.
    pub fn load_sources(
        &self,
        sources: &'ctx config::Sources,
        package: Option<&'ctx str>,
    ) -> SourceGroup<'ctx> {
        let include_dirs = sources
            .include_dirs
            .iter()
            .map(|d| self.intern_path(d))
            .collect();
        let defines = sources
            .defines
            .iter()
            .map(|(k, v)| {
                (
                    self.intern_string(k),
                    v.as_ref().map(|v| self.intern_string(v)),
                )
            })
            .collect();
        let files = sources
            .files
            .iter()
            .map(|file| match *file {
                config::SourceFile::File(ref path) => (path as &Path).into(),
                config::SourceFile::Group(ref group) => {
                    self.load_sources(group.as_ref(), None).into()
                }
            })
            .collect();
        SourceGroup {
            package: package,
            independent: false,
            target: sources.target.clone(),
            include_dirs: include_dirs,
            defines: defines,
            files: files,
        }
    }
}

/// An event loop to perform IO within a session.
///
/// This struct wraps a `Session` and keeps an additional event loop. Using the
/// various functions provided, IO can be scheduled on this event loop. The
/// futures may then be driven to completion using the `run()` function.
pub struct SessionIo<'sess, 'ctx: 'sess> {
    /// The underlying session.
    pub sess: &'sess Session<'ctx>,
    /// The event loop where IO will be run.
    pub handle: Handle,
    git_versions: Mutex<HashMap<PathBuf, GitVersions<'ctx>>>,
}

impl<'io, 'sess: 'io, 'ctx: 'sess> SessionIo<'sess, 'ctx> {
    /// Create a new session wrapper.
    pub fn new(sess: &'sess Session<'ctx>, handle: Handle) -> SessionIo<'sess, 'ctx> {
        SessionIo {
            sess: sess,
            handle: handle,
            git_versions: Mutex::new(HashMap::new()),
        }
    }

    /// Determine the available versions for a dependency.
    pub fn dependency_versions(
        &'io self,
        dep_id: DependencyRef,
        force_fetch: bool,
    ) -> Box<dyn Future<Item = DependencyVersions<'ctx>, Error = Error> + 'io> {
        self.sess.stats.num_calls_dependency_versions.increment();
        let dep = self.sess.dependency(dep_id);
        match dep.source {
            DependencySource::Registry => {
                unimplemented!("determine available versions of registry dependency");
            }
            DependencySource::Path(_) => Box::new(future::ok(DependencyVersions::Path)),
            DependencySource::Git(ref url) => Box::new(
                self.git_database(&dep.name, url, force_fetch)
                    .and_then(move |db| self.git_versions(db))
                    .map(DependencyVersions::Git),
            ),
        }
    }

    /// Access the git database for a dependency.
    ///
    /// If the database does not exist, it is created. If the database has not
    /// been updated recently, the remote is fetched.
    fn git_database(
        &'io self,
        name: &str,
        url: &str,
        force_fetch: bool,
    ) -> Box<dyn Future<Item = Git<'io, 'sess, 'ctx>, Error = Error> + 'io> {
        // TODO: Make the assembled future shared and keep it in a lookup table.
        //       Then use that table to return the future if it already exists.
        //       This ensures that the gitdb is setup only once, and makes the
        //       whole process faster for later calls.
        self.sess.stats.num_calls_git_database.increment();

        // Determine the name of the database as the given name and the first
        // 8 bytes (16 hex characters) of the URL's BLAKE2 hash.
        use blake2::{Blake2b, Digest};
        let hash = &format!("{:016x}", Blake2b::digest(url.as_bytes()))[..16];
        let db_name = format!("{}-{}", name, hash);

        // Determine the location of the git database and create it if its does
        // not yet exist.
        let db_dir = self
            .sess
            .config
            .database
            .join("git")
            .join("db")
            .join(db_name);
        let db_dir = self.sess.intern_path(db_dir);
        match std::fs::create_dir_all(db_dir) {
            Ok(_) => (),
            Err(cause) => {
                return Box::new(future::err(Error::chain(
                    format!("Failed to create git database directory {:?}.", db_dir),
                    cause,
                )))
            }
        };
        let git = Git::new(db_dir, self);
        let name2 = String::from(name);
        let url = String::from(url);
        let url2 = url.clone();

        // Either initialize the repository or update it if needed.
        if !db_dir.join("config").exists() {
            // Initialize.
            self.sess.stats.num_database_init.increment();
            Box::new(
                self.sess.git_throttle.spawn(
                    future::lazy(move || {
                        stageln!("Cloning", "{} ({})", name2, url2);
                        Ok(())
                    })
                    .and_then(move |_| git.spawn_with(|c| c.arg("init").arg("--bare")))
                    .and_then(move |_| {
                        git.spawn_with(|c| c.arg("remote").arg("add").arg("origin").arg(url))
                    })
                    .and_then(move |_| git.fetch("origin"))
                    .map_err(move |cause| {
                        Error::chain(
                            format!("Failed to initialize git database in {:?}.", db_dir),
                            cause,
                        )
                    })
                    .map(move |_| git),
                ),
            )
        } else {
            // Update if the manifest has been modified since the last fetch.
            let db_mtime = try_modification_time(db_dir.join("FETCH_HEAD"));
            if self.sess.manifest_mtime < db_mtime && !force_fetch {
                debugln!("sess: skipping fetch of {:?}", db_dir);
                return Box::new(future::ok(git));
            }
            self.sess.stats.num_database_fetch.increment();
            Box::new(
                self.sess.git_throttle.spawn(
                    future::lazy(move || {
                        stageln!("Fetching", "{} ({})", name2, url2);
                        Ok(())
                    })
                    .and_then(move |_| git.fetch("origin"))
                    .map_err(move |cause| {
                        Error::chain(
                            format!("Failed to update git database in {:?}.", db_dir),
                            cause,
                        )
                    })
                    .map(move |_| git),
                ),
            )
        }
    }

    /// Determine the list of versions available for a git dependency.
    fn git_versions(
        &'io self,
        git: Git<'io, 'sess, 'ctx>,
    ) -> Box<dyn Future<Item = GitVersions<'ctx>, Error = Error> + 'io> {
        match self
            .git_versions
            .lock()
            .unwrap()
            .get(&git.path.to_path_buf())
        {
            Some(result) => {
                debugln!("sess: git_versions from stored");
                Box::new(future::ok(GitVersions {
                    versions: result.versions.clone(),
                    refs: result.refs.clone(),
                    revs: result.revs.clone(),
                }))
            }
            None => {
                debugln!("sess: git_versions get new");
                let dep_refs = git.list_refs();
                let dep_revs = git.list_revs();
                let dep_refs_and_revs =
                    dep_refs.and_then(|refs| -> Box<dyn Future<Item = _, Error = Error>> {
                        if refs.is_empty() {
                            Box::new(future::ok((refs, vec![])))
                        } else {
                            Box::new(dep_revs.map(move |revs| (refs, revs)))
                        }
                    });
                let out = dep_refs_and_revs.and_then(move |(refs, revs)| {
                    let refs: Vec<_> = refs
                        .into_iter()
                        .map(|(a, b)| (self.sess.intern_string(a), self.sess.intern_string(b)))
                        .collect();
                    let revs: Vec<_> = revs
                        .into_iter()
                        .map(|s| self.sess.intern_string(s))
                        .collect();
                    debugln!("sess: refs {:?}", refs);
                    let (tags, branches) = {
                        // Create a lookup table for the revisions. This will be used to
                        // only accept refs that point to actual revisions.
                        let rev_ids: HashSet<&str> = revs.iter().map(|s| *s).collect();

                        // Split the refs into tags and branches, discard
                        // everything else.
                        let mut tags = HashMap::<&'ctx str, &'ctx str>::new();
                        let mut branches = HashMap::<&'ctx str, &'ctx str>::new();
                        let tag_pfx = "refs/tags/";
                        let branch_pfx = "refs/remotes/origin/";
                        for (hash, rf) in refs {
                            if !rev_ids.contains(hash) {
                                continue;
                            }
                            if rf.starts_with(tag_pfx) {
                                tags.insert(rf[tag_pfx.len()..].into(), hash);
                            } else if rf.starts_with(branch_pfx) {
                                branches.insert(rf[branch_pfx.len()..].into(), hash);
                            }
                        }
                        (tags, branches)
                    };

                    // Extract the tags that look like semantic versions.
                    let mut versions: Vec<(semver::Version, &'ctx str)> = tags
                        .iter()
                        .filter_map(|(tag, &hash)| {
                            if tag.starts_with("v") {
                                match semver::Version::parse(&tag[1..]) {
                                    Ok(v) => Some((v, hash)),
                                    Err(_) => None,
                                }
                            } else {
                                None
                            }
                        })
                        .collect();
                    versions.sort_by(|a, b| b.cmp(a));

                    // Merge tags and branches.
                    let refs: HashMap<&str, &str> =
                        branches.into_iter().chain(tags.into_iter()).collect();

                    let mut git_versions = self.git_versions.lock().unwrap().clone();

                    let git_path = git.path;

                    git_versions.insert(
                        git_path.to_path_buf(),
                        GitVersions {
                            versions: versions.clone(),
                            refs: refs.clone(),
                            revs: revs.clone(),
                        },
                    );

                    *self.git_versions.lock().unwrap() = git_versions.clone();

                    Ok(GitVersions {
                        versions: versions,
                        refs: refs,
                        revs: revs,
                    })
                });
                Box::new(out)
            }
        }
    }

    /// Ensure that a dependency is checked out and obtain its path.
    pub fn checkout(
        &'io self,
        dep_id: DependencyRef,
    ) -> Box<dyn Future<Item = &'ctx Path, Error = Error> + 'io> {
        // Check if the checkout is already in the cache.
        if let Some(&cached) = self.sess.cache.checkout.lock().unwrap().get(&dep_id) {
            return Box::new(future::ok(cached));
        }

        self.sess.stats.num_calls_checkout.increment();
        let dep = self.sess.dependency(dep_id);

        // Determine the name of the checkout as the given name and the first
        // 8 bytes (16 hex characters) of a BLAKE2 hash of the source and the
        // path to the root package. This ensures that for every dependency and
        // root package we have at most one checkout.
        let hash = {
            use blake2::{Blake2b, Digest};
            let mut hasher = Blake2b::new();
            match dep.source {
                DependencySource::Registry => unimplemented!(),
                DependencySource::Git(ref url) => hasher.update(url.as_bytes()),
                DependencySource::Path(ref path) => {
                    // Determine and canonicalize the dependency path, and
                    // immediately return it.
                    let path = self.sess.root.join(path);
                    let path = match canonicalize(&path) {
                        Ok(p) => p,
                        Err(_) => path,
                    };
                    let path = self.sess.intern_path(path);
                    return Box::new(future::ok(path));
                }
            }
            hasher.update(format!("{:?}", self.sess.root).as_bytes());
            &format!("{:016x}", hasher.finalize())[..16]
        };
        let checkout_name = format!("{}-{}", dep.name, hash);

        // Determine the location of the git checkout. If the workspace has an
        // explicit checkout directory, use that and do not append any hash to
        // the dependency name.
        let checkout_dir = match self.sess.manifest.workspace.checkout_dir {
            Some(ref cd) => self.sess.intern_path(cd.join(&dep.name)),
            None => self.sess.intern_path(
                self.sess
                    .config
                    .database
                    .join("git")
                    .join("checkouts")
                    .join(checkout_name),
            ),
        };

        match dep.source {
            DependencySource::Path(..) => unreachable!(),
            DependencySource::Registry => unimplemented!(),
            DependencySource::Git(ref url) => Box::new(
                self.checkout_git(
                    self.sess.intern_string(&dep.name),
                    checkout_dir,
                    self.sess.intern_string(url),
                    self.sess.intern_string(dep.revision.as_ref().unwrap()),
                )
                .and_then(move |path| {
                    self.sess
                        .cache
                        .checkout
                        .lock()
                        .unwrap()
                        .insert(dep_id, path);
                    Ok(path)
                }),
            ),
        }
    }

    /// Ensure that a proper git checkout exists.
    ///
    /// If the directory is not a proper git repository, it is deleted and
    /// re-created from scratch.
    fn checkout_git(
        &'io self,
        name: &'ctx str,
        path: &'ctx Path,
        url: &'ctx str,
        revision: &'ctx str,
    ) -> Box<dyn Future<Item = &'ctx Path, Error = Error> + 'io> {
        // First check if we have to get rid of the current checkout. This is
        // the case if it either does not exist or the checked out revision does
        // not match what we expect.
        let scrapped = future::lazy(move || future::ok(path.exists()))
            .and_then(move |exists| -> Box<dyn Future<Item = _, Error = Error>> {
                if exists {
                    // Never scrap checkouts the user asked for explicitly in
                    // the workspace configuration.
                    if self.sess.manifest.workspace.checkout_dir.is_some() {
                        return Box::new(future::ok(false));
                    }

                    // Scrap checkouts with the wrong tag.
                    Box::new(
                        Git::new(path, self)
                            .current_checkout()
                            .then(move |current| {
                                future::ok(match current {
                                    Ok(Some(current)) => {
                                        debugln!(
                                            "checkout_git: currently `{}` (want `{}`)",
                                            current,
                                            revision
                                        );
                                        current != revision
                                    }
                                    _ => true,
                                })
                            }),
                    )
                } else {
                    // Don't do anything if there is no checkout.
                    Box::new(future::ok(false))
                }
            })
            .and_then(move |clear| {
                if clear {
                    debugln!("checkout_git: clear checkout {:?}", path);
                    std::fs::remove_dir_all(path)
                        .map_err(|cause| {
                            Error::chain(
                                format!("Failed to remove checkout directory {:?}.", path),
                                cause,
                            )
                        })
                        .into()
                } else {
                    future::ok(())
                }
            });

        // Perform the checkout if necessary.
        let updated = scrapped.and_then(move |_| -> Box<dyn Future<Item = _, Error = Error>> {
            if !path.exists() {
                stageln!("Checkout", "{} ({})", name, url);

                // First generate a tag to be cloned in the database. This is
                // necessary since `git clone` does not accept commits, but only
                // branches or tags for shallow clones.
                let tag_name_0 = format!("bender-tmp-{}", revision);
                let tag_name_1 = tag_name_0.clone();
                let f = self
                    .git_database(name, url, false)
                    .and_then(move |git| {
                        git.clone()
                            .spawn_with(move |c| {
                                c.arg("tag").arg(tag_name_0).arg(revision).arg("--force")
                            })
                            .map(move |_| git)
                    })
                    .and_then(move |git| {
                        git.clone().spawn_with(move |c| {
                            c.arg("clone")
                                .arg(git.path)
                                .arg(path)
                                .arg("--recursive")
                                .arg("--branch")
                                .arg(tag_name_1)
                        })
                    })
                    .map(|_| ());

                Box::new(f)
            } else {
                Box::new(future::ok(()))
            }
        });

        Box::new(updated.map(move |_| path))
    }

    /// Load the manifest for a specific version of a dependency.
    ///
    /// Loads and returns the manifest for a dependency at a specific version.
    /// Returns `None` if the dependency has no manifest.
    pub fn dependency_manifest_version(
        &'io self,
        dep_id: DependencyRef,
        version: DependencyVersion<'ctx>,
    ) -> Box<dyn Future<Item = Option<&'ctx Manifest>, Error = Error> + 'io> {
        // Check if the manifest is already in the cache.
        let cache_key = (dep_id, version.clone());
        if let Some(&cached) = self
            .sess
            .cache
            .dependency_manifest_version
            .lock()
            .unwrap()
            .get(&cache_key)
        {
            return Box::new(future::ok(cached));
        }

        self.sess
            .stats
            .num_calls_dependency_manifest_version
            .increment();
        let dep = self.sess.dependency(dep_id);
        use self::DependencySource as DepSrc;
        use self::DependencyVersion as DepVer;
        match (&dep.source, version) {
            (&DepSrc::Path(ref path), DepVer::Path) => {
                let manifest_path = path.join("Bender.yml");
                if manifest_path.exists() {
                    match read_manifest(&manifest_path) {
                        Ok(m) => Box::new(future::ok(Some(self.sess.intern_manifest(m)))),
                        Err(e) => Box::new(future::err(e)),
                    }
                } else {
                    Box::new(future::ok(None))
                }
            }
            (&DepSrc::Registry, DepVer::Registry(_hash)) => {
                unimplemented!("load manifest of registry dependency");
            }
            (&DepSrc::Git(ref url), DepVer::Git(rev)) => {
                let dep_name = self.sess.intern_string(dep.name.as_str());
                Box::new(
                    self.git_database(&dep.name, url, false)
                        .and_then(move |db| {
                            db.list_files(rev, Some("Bender.yml")).and_then(
                                move |entries| -> Box<dyn Future<Item = _, Error = _>> {
                                    match entries.into_iter().next() {
                                        None => Box::new(future::ok(None)),
                                        Some(entry) => {
                                            Box::new(db.cat_file(entry.hash).map(|f| Some(f)))
                                        }
                                    }
                                },
                            )
                        })
                        .and_then(move |data| match data {
                            Some(data) => {
                                let partial: config::PartialManifest = serde_yaml::from_str(&data)
                                    .map_err(|cause| {
                                        Error::chain(
                                            format!(
                                                "Syntax error in manifest of dependency `{}` at \
                                                 revision `{}`.",
                                                dep_name, rev
                                            ),
                                            cause,
                                        )
                                    })?;
                                let full = partial.validate().map_err(|cause| {
                                    Error::chain(
                                        format!(
                                            "Error in manifest of dependency `{}` at revision \
                                             `{}`.",
                                            dep_name, rev
                                        ),
                                        cause,
                                    )
                                })?;
                                Ok(Some(self.sess.intern_manifest(full)))
                            }
                            None => Ok(None),
                        })
                        .and_then(move |manifest| {
                            self.sess
                                .cache
                                .dependency_manifest_version
                                .lock()
                                .unwrap()
                                .insert(cache_key, manifest);
                            Ok(manifest)
                        }),
                )
            }
            _ => panic!(
                "incompatible source {:?} and version {:?}",
                dep.source, version
            ),
        }
    }

    /// Load the manifest for a dependency.
    ///
    /// Loads and returns the manifest for a dependency at the resolved version.
    pub fn dependency_manifest(
        &'io self,
        dep_id: DependencyRef,
    ) -> Box<dyn Future<Item = Option<&'ctx Manifest>, Error = Error> + 'io> {
        // Check if the manifest is already in the cache.
        if let Some(&cached) = self
            .sess
            .cache
            .dependency_manifest
            .lock()
            .unwrap()
            .get(&dep_id)
        {
            return Box::new(future::ok(cached));
        }

        // Otherwise ensure that there is a checkout of the dependency and read
        // the manifest there.
        self.sess.stats.num_calls_dependency_manifest.increment();
        Box::new(
            self.checkout(dep_id)
                .and_then(move |path| {
                    let manifest_path = path.join("Bender.yml");
                    if manifest_path.exists() {
                        match read_manifest(&manifest_path) {
                            Ok(m) => Ok(Some(self.sess.intern_manifest(m))),
                            Err(e) => Err(e),
                        }
                    } else {
                        Ok(None)
                    }
                })
                .and_then(move |manifest| {
                    self.sess
                        .cache
                        .dependency_manifest
                        .lock()
                        .unwrap()
                        .insert(dep_id, manifest);
                    Ok(manifest)
                }),
        )
    }

    /// Load the source file manifest.
    ///
    /// Loads and returns the source file manifest for the root package and all
    /// its dependencies..
    pub fn sources(&'io self) -> Box<dyn Future<Item = SourceGroup<'ctx>, Error = Error> + 'io> {
        // Check if we already have the source manifest.
        if let Some(ref cached) = *self.sess.sources.lock().unwrap() {
            return Box::new(future::ok((*cached).clone()));
        }

        // Load the manifests of all packages.
        let manifests = join_all(
            self.sess
                .packages()
                .iter()
                .map(move |pkgs| {
                    join_all(
                        pkgs.iter()
                            .map(move |&pkg| self.dependency_manifest(pkg))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        );

        // Extract the sources of each package and concatenate them into a long
        // manifest.
        Box::new(
            manifests
                .and_then(move |ranks| {
                    // Collect a set of exported include directories that will be applied to
                    // all packages.
                    // TODO: Make this more fine-grained, such that only include dirs from
                    // directly depended-on packages are considered.
                    let export_include_dirs: Vec<_> = ranks
                        .iter()
                        .chain(once(&vec![Some(self.sess.manifest)]))
                        .flat_map(|manifests| {
                            manifests
                                .iter()
                                .filter_map(|&m| m)
                                .flat_map(|m| m.export_include_dirs.iter())
                        })
                        .collect();
                    debugln!(
                        "explicitly exported include dirs: {:#?}",
                        export_include_dirs
                    );

                    use std::iter::once;
                    let files = ranks
                        .into_iter()
                        .chain(once(vec![Some(self.sess.manifest)]))
                        .map(|manifests| {
                            let files = manifests
                                .into_iter()
                                .filter_map(|m| m)
                                .filter_map(|m| {
                                    m.sources.as_ref().map(|s| {
                                        self.sess
                                            .load_sources(s, Some(m.package.name.as_str()))
                                            .into()
                                    })
                                })
                                .collect();

                            // Create a source group for this rank.
                            SourceGroup {
                                package: None,
                                independent: true,
                                target: TargetSpec::Wildcard,
                                include_dirs: Vec::new(),
                                defines: HashMap::new(),
                                files: files,
                            }
                            .into()
                        })
                        .collect();

                    // Create a source group covering all ranks, i.e. the root
                    // source group.
                    Ok(SourceGroup {
                        package: None,
                        independent: false,
                        target: TargetSpec::Wildcard,
                        include_dirs: export_include_dirs
                            .iter()
                            .map(|p| self.sess.intern_path(p.clone()))
                            .collect(),
                        defines: HashMap::new(),
                        files: files,
                    }
                    .simplify())
                })
                .and_then(move |sources| {
                    *self.sess.sources.lock().unwrap() = Some(sources.clone());
                    Ok(sources)
                }),
        )
    }

    /// Load the plugins declared by any of the dependencies.
    pub fn plugins(&'io self) -> Box<dyn Future<Item = &'ctx Plugins, Error = Error> + 'io> {
        // Check if we already have the list of plugins.
        if let Some(cached) = *self.sess.plugins.lock().unwrap() {
            return Box::new(future::ok(cached));
        }

        // Load the manifests of all packages.
        let manifests = join_all(
            self.sess
                .packages()
                .iter()
                .map(move |pkgs| {
                    join_all(
                        pkgs.iter()
                            .map(move |&pkg| self.dependency_manifest(pkg).map(move |m| (pkg, m)))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .map(|ranks| {
            ranks
                .into_iter()
                .flat_map(|manifests| {
                    manifests
                        .into_iter()
                        .filter_map(|(pkg, m)| m.map(|m| (pkg, m)))
                })
                .collect::<Vec<_>>()
        });

        // Extract the plugins from the manifests.
        Box::new(
            manifests
                .and_then(move |manifests| {
                    let mut plugins = HashMap::new();
                    for (package, manifest) in manifests {
                        for (name, plugin) in &manifest.plugins {
                            debugln!(
                                "sess: plugin `{}` declared by package `{}`",
                                name,
                                manifest.package.name
                            );
                            let existing = plugins.insert(
                                name.clone(),
                                Plugin {
                                    name: name.clone(),
                                    package: package,
                                    path: plugin.clone(),
                                },
                            );
                            if let Some(existing) = existing {
                                return Err(Error::new(format!(
                                    "Plugin `{}` declared by multiple packages (`{}` and `{}`).",
                                    name,
                                    self.sess.dependency_name(existing.package),
                                    self.sess.dependency_name(package),
                                )));
                            }
                        }
                    }
                    let root_plugins = &self.sess.manifest.plugins;
                    for (name, plugin) in root_plugins.into_iter() {
                        debugln!("sess: plugin `{}` declared by root package", name);
                        let existing = plugins.insert(
                            name.clone(),
                            Plugin {
                                name: name.clone(),
                                package: DependencyRef(0), // FIXME: unclean implementation
                                path: plugin.clone(),
                            },
                        );
                        if let Some(existing) = existing {
                            return Err(Error::new(format!(
                                "Plugin `{}` declared by multiple packages (`{}` and `{}`).",
                                name,
                                self.sess.dependency_name(existing.package),
                                "root",
                            )));
                        }
                    }
                    Ok(plugins)
                })
                .and_then(move |plugins| {
                    let allocd = self.sess.arenas.plugins.alloc(plugins) as &_;
                    *self.sess.plugins.lock().unwrap() = Some(allocd);
                    Ok(allocd)
                }),
        )
    }
}

/// An arena container where all incremental, temporary things are allocated.
pub struct SessionArenas {
    /// An arena to allocate paths in.
    pub path: Arena<PathBuf>,
    /// An arena to allocate strings in.
    pub string: Arena<String>,
    /// An arena to allocate manifests in.
    pub manifest: Arena<Manifest>,
    /// An arena to allocate dependency entries in.
    pub dependency_entry: Arena<DependencyEntry>,
    /// An arena to allocate a table of plugins in.
    pub plugins: Arena<Plugins>,
}

impl SessionArenas {
    /// Create a new arena container.
    pub fn new() -> SessionArenas {
        SessionArenas {
            path: Arena::new(),
            string: Arena::new(),
            manifest: Arena::new(),
            dependency_entry: Arena::new(),
            plugins: Arena::new(),
        }
    }
}

impl fmt::Debug for SessionArenas {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SessionArenas")
    }
}

/// A unique identifier for a dependency.
///
/// These are emitted by the session once a dependency is loaded and are used to
/// uniquely identify dependencies.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyRef(usize);

impl fmt::Display for DependencyRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for DependencyRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// An entry in the session's dependency table.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct DependencyEntry {
    /// The name of this dependency.
    pub name: String,
    /// Where this dependency may be obtained from.
    pub source: DependencySource,
    /// The picked revision.
    pub revision: Option<String>,
    /// The picked version.
    pub version: Option<semver::Version>,
}

impl DependencyEntry {
    /// Obtain the dependency version for this entry.
    pub fn version<'a>(&'a self) -> DependencyVersion<'a> {
        match self.source {
            DependencySource::Registry => unimplemented!(),
            DependencySource::Path(_) => DependencyVersion::Path,
            DependencySource::Git(_) => DependencyVersion::Git(self.revision.as_ref().unwrap()),
        }
    }
}

/// Where a dependency may be obtained from.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DependencySource {
    /// The dependency is coming from a registry.
    Registry,
    /// The dependency is located at a fixed path. No version resolution will be
    /// performed.
    Path(PathBuf),
    /// The dependency is available at a git url.
    Git(String),
}

impl fmt::Display for DependencySource {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            DependencySource::Registry => write!(f, "registry"),
            DependencySource::Path(ref path) => write!(f, "{:?}", path),
            DependencySource::Git(ref url) => write!(f, "`{}`", url),
        }
    }
}

impl DependencySource {
    /// returns a string of the source
    pub fn to_str(&self) -> String {
        match *self {
            DependencySource::Registry => "registry".to_string(),
            DependencySource::Path(ref path) => format!("{:?}", path),
            DependencySource::Git(ref url) => format!("{}", url),
        }
    }
}

/// A table of internalized dependencies.
#[derive(Debug)]
struct DependencyTable<'ctx> {
    list: Vec<&'ctx DependencyEntry>,
    ids: HashMap<&'ctx DependencyEntry, DependencyRef>,
}

impl<'ctx> DependencyTable<'ctx> {
    /// Create a new dependency table.
    pub fn new() -> DependencyTable<'ctx> {
        DependencyTable {
            list: Vec::new(),
            ids: HashMap::new(),
        }
    }

    /// Add a dependency entry to the table.
    ///
    /// The reference with which the information can later be retrieved is
    /// returned.
    pub fn add(&mut self, entry: &'ctx DependencyEntry) -> DependencyRef {
        if let Some(&id) = self.ids.get(&entry) {
            debugln!("sess: reusing {:?}", id);
            id
        } else {
            let id = DependencyRef(self.list.len());
            debugln!("sess: adding {:?} as {:?}", entry, id);
            self.list.push(entry);
            self.ids.insert(entry, id);
            id
        }
    }
}

/// All available versions of a dependency.
#[derive(Clone, Debug)]
pub enum DependencyVersions<'ctx> {
    /// Path dependencies have no versions, but are exactly as present on disk.
    Path,
    /// Registry dependency versions.
    Registry(RegistryVersions),
    /// Git dependency versions.
    Git(GitVersions<'ctx>),
}

/// All available versions of a registry dependency.
#[derive(Clone, Debug)]
pub struct RegistryVersions;

/// All available versions a git dependency has.
#[derive(Clone, Debug)]
pub struct GitVersions<'ctx> {
    /// The versions available for this dependency. This is basically a sorted
    /// list of tags of the form `v<semver>`.
    pub versions: Vec<(semver::Version, &'ctx str)>,
    /// The named references available for this dependency. This is a mixture of
    /// branch names and tags, where the tags take precedence.
    pub refs: HashMap<&'ctx str, &'ctx str>,
    /// The revisions available for this dependency, newest one first. We obtain
    /// these via `git rev-list --all --date-order`.
    pub revs: Vec<&'ctx str>,
}

/// A single version of a dependency.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DependencyVersion<'ctx> {
    /// A path dependency has no version.
    Path,
    /// The exact hash of a registry dependency.
    Registry(&'ctx str),
    /// The exact revision of a git dependency.
    Git(&'ctx str),
}

impl<'ctx> fmt::Display for DependencyVersion<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            DependencyVersion::Path => write!(f, "path"),
            DependencyVersion::Registry(ref v) => write!(f, "{}", v),
            DependencyVersion::Git(ref r) => write!(f, "{}", r),
        }
    }
}

impl<'ctx> DependencyVersion<'ctx> {
    /// returns a string of the version
    pub fn to_str(&self) -> String {
        match *self {
            DependencyVersion::Path => "path".to_string(),
            DependencyVersion::Registry(ref v) => format!("{}", v),
            DependencyVersion::Git(ref r) => format!("{}", r),
        }
    }
}

/// A constraint on a dependency.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DependencyConstraint {
    /// A path constraint. If a package has a path dependency, it imposes a path
    /// constraint on it.
    Path,
    /// A version constraint. These may occur for registry or git dependencies.
    Version(semver::VersionReq),
    /// A revision constraint. These occur for git dependencies.
    Revision(String),
}

impl<'a> From<&'a config::Dependency> for DependencyConstraint {
    fn from(cfg: &'a config::Dependency) -> DependencyConstraint {
        match *cfg {
            config::Dependency::Path(..) => DependencyConstraint::Path,
            config::Dependency::Version(ref v) | config::Dependency::GitVersion(_, ref v) => {
                DependencyConstraint::Version(v.clone())
            }
            config::Dependency::GitRevision(_, ref r) => DependencyConstraint::Revision(r.clone()),
        }
    }
}

impl fmt::Display for DependencyConstraint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            DependencyConstraint::Path => write!(f, "path"),
            DependencyConstraint::Version(ref v) => write!(f, "{}", v),
            DependencyConstraint::Revision(ref r) => write!(f, "{}", r),
        }
    }
}

/// Statistics about a session.
///
/// This struct contains statistics about commands executed in a session. It is
/// automatically printed upon deconstruction.
#[derive(Debug, Default)]
pub struct SessionStatistics {
    num_calls_dependency_versions: StatisticCounter,
    num_calls_git_database: StatisticCounter,
    num_calls_checkout: StatisticCounter,
    num_calls_dependency_manifest_version: StatisticCounter,
    num_calls_dependency_manifest: StatisticCounter,
    num_database_init: StatisticCounter,
    num_database_fetch: StatisticCounter,
}

impl<'ctx> Drop for SessionStatistics {
    fn drop(&mut self) {
        debugln!("{:#?}", self);
    }
}

#[derive(Default)]
struct StatisticCounter(AtomicUsize);

impl StatisticCounter {
    fn increment(&self) {
        use std::sync::atomic::Ordering;
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl fmt::Debug for StatisticCounter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use std::sync::atomic::Ordering;
        write!(f, "{}", self.0.load(Ordering::SeqCst))
    }
}

/// A cache for the session.
#[derive(Default)]
pub struct SessionCache<'ctx> {
    dependency_manifest_version:
        Mutex<HashMap<(DependencyRef, DependencyVersion<'ctx>), Option<&'ctx config::Manifest>>>,
    dependency_manifest: Mutex<HashMap<DependencyRef, Option<&'ctx config::Manifest>>>,
    checkout: Mutex<HashMap<DependencyRef, &'ctx Path>>,
}

impl<'ctx> fmt::Debug for SessionCache<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SessionCache")
    }
}

/// A list of plugins.
pub type Plugins = HashMap<String, Plugin>;

/// A plugin declared by a package.
#[derive(Debug)]
pub struct Plugin {
    /// The name of the plugin.
    pub name: String,
    /// Which package declared the plugin.
    pub package: DependencyRef,
    /// What binary implements the plugin.
    pub path: PathBuf,
}
