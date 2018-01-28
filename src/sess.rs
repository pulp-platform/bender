// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A command line session.

#![deny(missing_docs)]

use std::fmt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Arc};

use semver;
use futures::Future;
use futures::future;
use futures_cpupool::CpuPool;
use tokio_core::reactor::Core;
use typed_arena::Arena;

use error::*;
use config::{self, Manifest, Config};
use git::Git;

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
    /// The arenas into which we allocate various things that need to live as
    /// long as the session.
    arenas: &'ctx SessionArenas,
    /// The thread pool which will execute tasks.
    pub pool: CpuPool,
    /// The event loop which will handle IO.
    pub core: Core,
    /// The dependency table.
    deps: Mutex<DependencyTable>,
    /// The internalized paths.
    paths: Mutex<HashSet<&'ctx PathBuf>>,
}

impl<'sess, 'ctx: 'sess> Session<'ctx> {
    /// Create a new session.
    pub fn new(root: &'ctx Path, manifest: &'ctx Manifest, config: &'ctx Config, arenas: &'ctx SessionArenas) -> Session<'ctx> {
        Session {
            root: root,
            manifest: manifest,
            config: config,
            arenas: arenas,
            pool: CpuPool::new_num_cpus(),
            core: Core::new().unwrap(),
            deps: Mutex::new(DependencyTable::new()),
            paths: Mutex::new(HashSet::new()),
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
    ) -> DependencyRef {
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
            debugln!("sess: reusing {:?}", id);
            id
        } else {
            let id = DependencyRef(deps.list.len());
            debugln!("sess: adding {:?} as {:?}", entry, id);
            deps.list.push(entry.clone());
            deps.ids.insert(entry, id);
            id
        }
    }

    /// Obtain information on a dependency.
    pub fn dependency(&self, dep: DependencyRef) -> Arc<DependencyEntry> {
        // TODO: Don't make any clones! Use an arena instead.
        self.deps.lock().unwrap().list[dep.0].clone()
    }

    /// Determine the source of a dependency.
    pub fn dependency_source(&self, dep: DependencyRef) -> DependencySource {
        // TODO: Don't make any clones! Use an arena instead.
        self.deps.lock().unwrap().list[dep.0].source.clone()
    }

    /// Determine the available versions for a dependency.
    pub fn dependency_versions(
        &'sess self,
        dep_id: DependencyRef
    ) -> Box<Future<Item=Vec<DependencyVersion>, Error=Error> + 'sess> {
        let dep = self.dependency(dep_id);
        match dep.source {
            DependencySource::Registry => {
                unimplemented!("determine available versions of registry dependency");
            }
            DependencySource::Path(_) => {
                Box::new(future::ok(vec![DependencyVersion::Unit]))
            }
            DependencySource::Git(ref url) => {
                debugln!("sess: available versions of git repo {:?}", url);
                let db = self.git_database(&dep.name, url);
                Box::new(db.and_then(|_| future::err(Error::new("not implemented: determine available versions of git dependency"))))
            }
        }
    }

    /// Access the git database for a dependency.
    ///
    /// If the database does not exist, it is created. If the database has not
    /// been updated recently, the remote is fetched.
    fn git_database(
        &'sess self,
        name: &str,
        url: &str
    ) -> Box<Future<Item=&'ctx Path, Error=Error> + 'sess> {
        use std;

        // TODO: Make the assembled future shared and keep it in a lookup table.
        //       Then use that table to return the future if it already exists.
        //       This ensures that the gitdb is setup only once, and makes the
        //       whole process faster for later calls.

        // Determine the name of the database as the given name and the first
        // 8 bytes (16 hex characters) of the URL's BLAKE2 hash.
        use blake2::{Blake2b, Digest};
        let hash = &format!("{:016x}", Blake2b::digest_str(url))[..16];
        let db_name = format!("{}-{}", name, hash);

        // Determine the location of the git database and create it if its does
        // not yet exist.
        let db_dir = self.config.database.join("git").join("db").join(db_name);
        let db_dir = self.intern_path(db_dir);
        match std::fs::create_dir_all(db_dir) {
            Ok(_) => (),
            Err(cause) => return Box::new(future::err(Error::chain(
                format!("Failed to create git database directory {:?}.", db_dir),
                cause
            )))
        };
        let git = Git::new(db_dir, self);
        let url = String::from(url);

        // Either initialize the repository or update it if needed.
        if !db_dir.join("config").exists() {
            // Initialize.
            Box::new(git.spawn_with(|c| c
                .arg("init")
                .arg("--bare")
            ).map_err(move |cause| Error::chain(
                format!("Failed to initialize bare git repository in {:?}.", db_dir),
                cause
            )).and_then(move |_| git.spawn_with(|c| c
                .arg("remote")
                .arg("add")
                .arg("origin")
                .arg(url)
            ).map_err(move |cause| Error::chain(
                format!("Failed to add remote to git repository in {:?}.", db_dir),
                cause
            ))).and_then(move |_|
                git.fetch("origin")
            ).map_err(move |cause| Error::chain(
                format!("Failed to initialize git database in {:?}.", db_dir),
                cause
            )).map(move |_|
                db_dir
            ))

            // Box::new(future::err(Error::new("gitdb init not implemented")))
        } else if true {
            // Update.
            Box::new(future::err(Error::new("gitdb update not implemented")))
        } else {
            // Pass.
            // Box::new(future::ok(db_dir))
            Box::new(future::err(Error::new("gitdb pass not implemented")))
        }

        // Perform the actual setup asynchronously.
        // let url = String::from(url);
        // Box::new(self.pool.spawn_fn(move || -> Result<PathBuf> {
        //     use std::process::Command;

        //     // Initialize it as a bare git repository if it is not one yet.
        //     if !db_dir.join("config").exists() {
        //         debugln!("sess-gitdb: init bare repo {:?}", db_dir);
        //         let status = Command::new("git")
        //             .arg("init")
        //             .arg("--bare")
        //             .current_dir(&db_dir)
        //             .status()
        //             .map_err(|cause| Error::chain(
        //                 "Failed to spawn git subprocess.",
        //                 cause
        //             ))?;
        //         if !status.success() {
        //             return Err(Error::new(format!("Failed to initialize bare git repository in {:?}.", db_dir)));
        //         }

        //         // Add the remote.
        //         let status = Command::new("git")
        //             .arg("remote")
        //             .arg("add")
        //             .arg("origin")
        //             .arg(&url)
        //             .current_dir(&db_dir)
        //             .status()
        //             .map_err(|cause| Error::chain(
        //                 "Failed to spawn git subprocess.",
        //                 cause
        //             ))?;
        //         if !status.success() {
        //             return Err(Error::new(format!("Failed to add remote to git repository in {:?}.", db_dir)));
        //         }
        //     }

        //     // Fetch any recent changes if necessary.
        //     debugln!("sess-gitdb: fetch `{}`", url);
        //     let status = Command::new("git")
        //         .arg("fetch")
        //         .arg("--prune")
        //         .arg("origin")
        //         .current_dir(&db_dir)
        //         .status()
        //         .map_err(|cause| Error::chain(
        //             "Failed to spawn git subprocess.",
        //             cause
        //         ))?;
        //     if !status.success() {
        //         return Err(Error::new(format!("Failed to fetch repository `{}`", url)));
        //     }

        //     let status = Command::new("git")
        //         .arg("fetch")
        //         .arg("--tags")
        //         .arg("--prune")
        //         .arg("origin")
        //         .current_dir(&db_dir)
        //         .status()
        //         .map_err(|cause| Error::chain(
        //             "Failed to spawn git subprocess.",
        //             cause
        //         ))?;
        //     if !status.success() {
        //         return Err(Error::new(format!("Failed to fetch repository `{}`", url)));
        //     }

        //     Ok(db_dir)
        // }))
    }

    /// Internalize a path.
    pub fn intern_path(&self, buf: PathBuf) -> &'ctx Path {
        let mut paths = self.paths.lock().unwrap();
        if let Some(&p) = paths.get(&buf) {
            p
        } else {
            let p = self.arenas.path.alloc(buf);
            paths.insert(p);
            p
        }
    }
}

/// An arena container where all incremental, temporary things are allocated.
pub struct SessionArenas {
    /// An arena to allocate paths in.
    pub path: Arena<PathBuf>,
}

impl SessionArenas {
    /// Create a new arena container.
    pub fn new() -> SessionArenas {
        SessionArenas {
            path: Arena::new(),
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
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DependencyRef(usize);

/// An entry in the session's dependency table.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct DependencyEntry {
    /// The name of this dependency.
    name: String,
    /// Where this dependency may be obtained from.
    source: DependencySource,
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

/// A version of a dependency.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DependencyVersion {
    /// A unit version. Path dependencies have exactly one `Unit` version and
    /// are incompatible with any kind of version specification.
    Unit,
    /// A semantic version. These are useful for dependencies distributed as
    /// tarballs via a registry, where no more git information is available.
    Version(semver::Version),
    /// A git revision. These are useful for git repositories that do not use
    /// any form of semantic version tagging. A git dependency will always have
    /// tons of these versions, one for every `git rev-list --all` line.
    Hash(String),
    /// A git revision and semantic version. These are useful for git
    /// dependencies that do use semantic version tagging.
    VersionHash(semver::Version, String),
}
