// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A command line session.

#![deny(missing_docs)]

use std;
use std::fmt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Arc};
use std::process::Command;

use semver;
use futures::Future;
use futures::future;
use tokio_core::reactor::Handle;
use tokio_process::CommandExt;
use typed_arena::Arena;

use error::*;
use config::{self, Manifest, Config};
use git::Git;
use util::{read_file, write_file};

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
    /// The dependency table.
    deps: Mutex<DependencyTable>,
    /// The internalized paths.
    paths: Mutex<HashSet<&'ctx Path>>,
    /// The internalized strings.
    strings: Mutex<HashSet<&'ctx str>>,
    /// The package name table.
    names: Mutex<HashMap<String, DependencyRef>>,
}

impl<'sess, 'ctx: 'sess> Session<'ctx> {
    /// Create a new session.
    pub fn new(root: &'ctx Path, manifest: &'ctx Manifest, config: &'ctx Config, arenas: &'ctx SessionArenas) -> Session<'ctx> {
        Session {
            root: root,
            manifest: manifest,
            config: config,
            arenas: arenas,
            deps: Mutex::new(DependencyTable::new()),
            paths: Mutex::new(HashSet::new()),
            strings: Mutex::new(HashSet::new()),
            names: Mutex::new(HashMap::new()),
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
        self.deps.lock().unwrap().add(DependencyEntry {
            name: name.into(),
            source: src,
            revision: None,
            version: None,
        })
    }

    /// Load a lock file.
    ///
    /// This internalizes the dependency sources, i.e. assigns `DependencyRef`
    /// objects to them, and generates a nametable.
    pub fn load_locked(
        &self,
        locked: &config::Locked,
    ) {
        debugln!("sess: load locked {:#?}", locked);
        let mut deps = self.deps.lock().unwrap();
        let mut names = HashMap::new();
        for (name, pkg) in &locked.packages {
            let src = match pkg.source {
                config::LockedSource::Path(ref path) => DependencySource::Path(path.clone()),
                config::LockedSource::Git(ref url) => DependencySource::Git(url.clone()),
                config::LockedSource::Registry(ref _ver) => DependencySource::Registry,
            };
            let id = deps.add(DependencyEntry {
                name: name.clone(),
                source: src,
                revision: pkg.revision.clone(),
                version: pkg.version.as_ref().map(|s| semver::Version::parse(&s).unwrap()),
            });
            names.insert(name.clone(), id);
        }
        drop(deps);
        *self.names.lock().unwrap() = names;
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

    /// Resolve a dependency name to a reference.
    ///
    /// Returns an error if the dependency does not exist.
    pub fn dependency_with_name(&self, name: &str) -> Result<DependencyRef> {
        let result = self.names.lock().unwrap().get(name).map(|id| *id);
        match result {
            Some(id) => Ok(id),
            None => Err(Error::new(format!("Dependency `{}` does not exist. Did you forget to add it to the manifest?", name))),
        }
    }

    /// Internalize a path.
    ///
    /// This allocates the path in the arena and returns a reference to it whose
    /// lifetime is bound to the arena rather than this `Session`. Useful to
    /// obtain a lightweight pointer to a path that is guaranteed to outlive the
    /// `Session`.
    pub fn intern_path<T>(&self, path: T) -> &'ctx Path
        where T: Into<PathBuf> + AsRef<Path>
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
        where T: Into<String> + AsRef<str>
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
}

impl<'io, 'sess: 'io, 'ctx: 'sess> SessionIo<'sess, 'ctx> {
    /// Create a new session wrapper.
    pub fn new(sess: &'sess Session<'ctx>, handle: Handle) -> SessionIo<'sess, 'ctx> {
        SessionIo {
            sess: sess,
            handle: handle,
        }
    }

    /// Determine the available versions for a dependency.
    pub fn dependency_versions(
        &'io self,
        dep_id: DependencyRef
    ) -> Box<Future<Item=DependencyVersions, Error=Error> + 'io> {
        let dep = self.sess.dependency(dep_id);
        match dep.source {
            DependencySource::Registry => {
                unimplemented!("determine available versions of registry dependency");
            }
            DependencySource::Path(_) => {
                Box::new(future::ok(DependencyVersions::Path))
            }
            DependencySource::Git(ref url) => {
                Box::new(self
                    .git_database(&dep.name, url)
                    .and_then(move |db| self.git_versions(db))
                    .map(DependencyVersions::Git))
            }
        }
    }

    /// Access the git database for a dependency.
    ///
    /// If the database does not exist, it is created. If the database has not
    /// been updated recently, the remote is fetched.
    fn git_database(
        &'io self,
        name: &str,
        url: &str
    ) -> Box<Future<Item=Git<'io, 'sess, 'ctx>, Error=Error> + 'io> {
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
        let db_dir = self.sess.config.database.join("git").join("db").join(db_name);
        let db_dir = self.sess.intern_path(db_dir);
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
            stageln!("Cloning", "{}", url);
            Box::new(
                git.spawn_with(|c| c
                    .arg("init")
                    .arg("--bare"))
                .and_then(move |_| git.spawn_with(|c| c
                    .arg("remote")
                    .arg("add")
                    .arg("origin")
                    .arg(url)))
                .and_then(move |_| git.fetch("origin"))
                .map_err(move |cause| Error::chain(
                    format!("Failed to initialize git database in {:?}.", db_dir),
                    cause))
                .map(move |_| git)
            )
        } else {
            // Update.
            // TODO: Don't always do this, but rather, check if the manifest has
            //       been modified since the last fetch, and only then proceed.
            Box::new(git.fetch("origin").map(move |_| git))
        }
    }

    /// Determine the list of versions available for a git dependency.
    fn git_versions(
        &'io self,
        git: Git<'io, 'sess, 'ctx>,
    ) -> Box<Future<Item=GitVersions, Error=Error> + 'io> {
        let dep_refs = git.list_refs();
        let dep_revs = git.list_revs();
        let dep_refs_and_revs = dep_refs
            .and_then(|refs| -> Box<Future<Item=_, Error=Error>> {
                if refs.is_empty() {
                    Box::new(future::ok((refs, vec![])))
                } else {
                    Box::new(dep_revs.map(move |revs| (refs, revs)))
                }
            });
        let out = dep_refs_and_revs.and_then(move |(refs, revs)|{
            debugln!("sess: gitdb: refs {:?}", refs);
            let (tags, branches) = {
                // Create a lookup table for the revisions. This will be used to
                // only accept refs that point to actual revisions.
                let rev_ids: HashSet<&str> = revs.iter().map(String::as_str).collect();

                // Split the refs into tags and branches, discard
                // everything else.
                let mut tags = HashMap::<String, String>::new();
                let mut branches = HashMap::<String, String>::new();
                let tag_pfx = "refs/tags/";
                let branch_pfx = "refs/remotes/origin/";
                for (hash, rf) in refs {
                    if !rev_ids.contains(hash.as_str()) {
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
            let mut versions: Vec<(semver::Version, String)> = tags
                .iter()
                .filter_map(|(tag, hash)|{
                    if tag.starts_with("v") {
                        match semver::Version::parse(&tag[1..]) {
                            Ok(v) => Some((v, hash.clone())),
                            Err(_) => None,
                        }
                    } else {
                        None
                    }
                })
                .collect();
            versions.sort_by(|a,b| b.cmp(a));

            // Merge tags and branches.
            let refs = branches.into_iter().chain(tags.into_iter()).collect();

            Ok(GitVersions {
                versions: versions,
                refs: refs,
                revs: revs,
            })
        });
        Box::new(out)
    }

    /// Ensure that a dependency is checked out and obtain its path.
    pub fn checkout(
        &'io self,
        dep_id: DependencyRef
    ) -> Box<Future<Item=&'ctx Path, Error=Error> + 'io> {
        // Find the exact source of the dependency.
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
                DependencySource::Git(ref url) => hasher.input(url.as_bytes()),
                DependencySource::Path(ref path) => return Box::new(
                    future::ok(self.sess.intern_path(self.sess.root.join(path)))
                ),
            }
            hasher.input(format!("{:?}", self.sess.root).as_bytes());
            &format!("{:016x}", hasher.result())[..16]
        };
        let checkout_name = format!("{}-{}", dep.name, hash);

        // Determine the location of the git checkout.
        let checkout_dir = self.sess.intern_path(self.sess.config.database
            .join("git")
            .join("checkouts")
            .join(checkout_name));

        match dep.source {
            DependencySource::Path(..) => unreachable!(),
            DependencySource::Registry => unimplemented!(),
            DependencySource::Git(ref url) => {
                self.checkout_git(
                    self.sess.intern_string(dep.name.as_ref()),
                    checkout_dir,
                    self.sess.intern_string(url.as_ref()),
                    self.sess.intern_string(dep.revision.as_ref().unwrap().as_ref())
                )
            }
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
    ) -> Box<Future<Item=&'ctx Path, Error=Error> + 'io> {
        // Determine the path and contents of the tag file.
        let tagpath = self.sess.intern_path(path.join(".bender-tag"));
        let tagname = self.sess.intern_string(format!("git {}", revision));
        let archive_path = self.sess.intern_path(path.join(".bender-archive.tar"));

        // First check if we have to get rid of the current checkout. This is
        // the case if either it or the tag does not exist, or the content of
        // the tag does not match what we expect.
        let scrapped = future::lazy(move ||{
            let clear = if tagpath.exists() {
                let current_tagname = read_file(tagpath).map_err(|cause| Error::chain(
                    format!("Failed to read tagfile {:?}.", tagpath),
                    cause
                ))?;
                let current_tagname = current_tagname.trim();
                debugln!("checkout_git: currently `{}` (want `{}`) at {:?}", current_tagname, tagname, tagpath);
                // Scrap checkouts with the wrong tag.
                current_tagname != tagname
            } else if path.exists() {
                // Scrap checkouts without a tag.
                true
            } else {
                // Don't do anything if there is no checkout.
                false
            };
            if clear {
                debugln!("checkout_git: clear checkout {:?}", path);
                std::fs::remove_dir_all(path).map_err(|cause| Error::chain(
                    format!("Failed to remove checkout directory {:?}.", path),
                    cause
                ))?;
            }
            Ok(())
        });

        // Create the checkout directory if it does not exist yet.
        let created = scrapped.and_then(move |_|{
            if !path.exists() {
                debugln!("checkout_git: create directory {:?}", path);
                std::fs::create_dir_all(path).map_err(|cause| Error::chain(
                    format!("Failed to create git checkout directory {:?}.", path),
                    cause
                ))?;
                Ok(true)
            } else {
                Ok(false)
            }
        });

        // Perform the checkout if necessary.
        let updated = created.and_then(move |need_checkout| -> Box<Future<Item=_, Error=Error>> {
            if need_checkout {
                // In the database repository, generate a TAR archive from the
                // revision.
                debugln!("checkout_git: create archive {:?}", archive_path);
                let f = self
                    .git_database(name, url)
                    .and_then(move |git| git.spawn_with(|c| c
                        .arg("archive")
                        .arg("--format")
                        .arg("tar")
                        .arg("--output")
                        .arg(archive_path)
                        .arg(revision)
                    ))
                    .map(|_| ());

                // Unpack the archive.
                let f = f.and_then(move |_|{
                    debugln!("checkout_git: unpack archive {:?}", archive_path);
                    let mut cmd = Command::new("tar");
                    cmd.arg("xf").arg(archive_path)
                        .current_dir(path)
                        .output_async(&self.handle)
                        .map_err(|cause| Error::chain(
                            "Failed to spawn child process.",
                            cause
                        ))
                        .and_then(move |output|{
                            if output.status.success() {
                                Ok(())
                            } else {
                                Err(Error::new(format!("Failed to unpack archive. Command ({:?}) in directory {:?} failed.", cmd, path)))
                            }
                        })
                });

                // Create the tagfile in the checkout such that we know what we've
                // checked out the next time around.
                let f = f.and_then(move |_|{
                    debugln!("checkout_git: remove archive {:?}", archive_path);
                    std::fs::remove_file(archive_path).map_err(|cause| Error::chain(
                        format!("Failed to remove archive {:?}.", path),
                        cause
                    ))?;
                    debugln!("checkout_git: write tag `{}` to {:?}", tagname, tagpath);
                    write_file(tagpath, tagname).map_err(|cause| Error::chain(
                        format!("Failed to write tagfile {:?}.", tagpath),
                        cause
                    ))?;
                    Ok(())
                });
                Box::new(f.map(|_| ()))
            } else {
                Box::new(future::ok(()))
            }
        });

        Box::new(updated.map(move |_| path))
    }
}

/// An arena container where all incremental, temporary things are allocated.
pub struct SessionArenas {
    /// An arena to allocate paths in.
    pub path: Arena<PathBuf>,
    /// An arena to allocate strings in.
    pub string: Arena<String>,
}

impl SessionArenas {
    /// Create a new arena container.
    pub fn new() -> SessionArenas {
        SessionArenas {
            path: Arena::new(),
            string: Arena::new(),
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

impl fmt::Display for DependencyRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// An entry in the session's dependency table.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct DependencyEntry {
    /// The name of this dependency.
    name: String,
    /// Where this dependency may be obtained from.
    source: DependencySource,
    /// The picked revision.
    revision: Option<String>,
    /// The picked version.
    version: Option<semver::Version>,
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

    /// Add a dependency entry to the table.
    ///
    /// The reference with which the information can later be retrieved is
    /// returned.
    pub fn add(&mut self, entry: DependencyEntry) -> DependencyRef {
        let entry = Arc::new(entry);
        if let Some(&id) = self.ids.get(&entry) {
            debugln!("sess: reusing {:?}", id);
            id
        } else {
            let id = DependencyRef(self.list.len());
            debugln!("sess: adding {:?} as {:?}", entry, id);
            self.list.push(entry.clone());
            self.ids.insert(entry, id);
            id
        }
    }
}

/// All available versions of a dependency.
#[derive(Clone, Debug)]
pub enum DependencyVersions {
    /// Path dependencies have no versions, but are exactly as present on disk.
    Path,
    /// Registry dependency versions.
    Registry(RegistryVersions),
    /// Git dependency versions.
    Git(GitVersions),
}

/// All available versions of a registry dependency.
#[derive(Clone, Debug)]
pub struct RegistryVersions;

/// All available versions a git dependency has.
#[derive(Clone, Debug)]
pub struct GitVersions {
    /// The versions available for this dependency. This is basically a sorted
    /// list of tags of the form `v<semver>`.
    pub versions: Vec<(semver::Version, String)>,
    /// The named references available for this dependency. This is a mixture of
    /// branch names and tags, where the tags take precedence.
    pub refs: HashMap<String, String>,
    /// The revisions available for this dependency, newest one first. We obtain
    /// these via `git rev-list --all --date-order`.
    pub revs: Vec<String>,
}

/// A constraint on a dependency.
#[derive(Clone, Debug)]
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
            config::Dependency::Path(..) => {
                DependencyConstraint::Path
            }
            config::Dependency::Version(ref v) |
            config::Dependency::GitVersion(_, ref v) => {
                DependencyConstraint::Version(v.clone())
            }
            config::Dependency::GitRevision(_, ref r) => {
                DependencyConstraint::Revision(r.clone())
            }
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
