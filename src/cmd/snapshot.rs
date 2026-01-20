// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `snapshot` subcommand.

use std::path::PathBuf;
use std::process::Command as SysCommand;

use clap::Args;
use indexmap::IndexMap;
use tokio::runtime::Runtime;

use crate::cli::{remove_symlink_dir, symlink_dir};
use crate::cmd::clone::get_path_subdeps;
use crate::config::{Dependency, Locked, LockedSource};
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{DependencySource, Session, SessionIo};

/// Snapshot the cloned IPs from the working directory into the Bender.lock file
#[derive(Args, Debug)]
pub struct SnapshotArgs {
    /// Working directory to snapshot dependencies from
    #[arg(long, default_value = "working_dir")]
    pub working_dir: String,

    /// Do not skip dependencies that are dirty
    #[arg(long)]
    pub no_skip: bool,

    /// Checkout the dependencies snapshotted into the lockfile
    #[arg(short, long)]
    pub checkout: bool,

    /// Force update of dependencies in a custom checkout_dir. Please use carefully to avoid losing work.
    #[arg(long, requires = "checkout")]
    pub force: bool,
}

/// Execute the `snapshot` subcommand.
pub fn run(sess: &Session, args: &SnapshotArgs) -> Result<()> {
    let mut snapshot_list = Vec::new();

    // Loop through existing deps to find the ones that are overridden to the working directory
    for (name, dep) in sess.config.overrides.iter() {
        if let Dependency::Path {
            path: override_path,
            ..
        } = dep
        {
            if override_path.starts_with(sess.root.join(&args.working_dir)) {
                if let DependencySource::Path(dep_path) =
                    sess.dependency_source(sess.dependency_with_name(name)?)
                {
                    if dep_path == *override_path {
                        // check state, skip & warn if dirty
                        if !SysCommand::new(&sess.config.git)
                            .arg("status")
                            .arg("--porcelain")
                            .current_dir(&dep_path)
                            .output()?
                            .stdout
                            .is_empty()
                            && !args.no_skip
                        {
                            Warnings::SkippingDirtyDep { pkg: name.clone() }.emit();
                            continue;
                        }

                        // Get the git url and hash of the dependency
                        let url = match String::from_utf8(
                            SysCommand::new(&sess.config.git)
                                .arg("remote")
                                .arg("get-url")
                                .arg("origin")
                                .current_dir(&dep_path)
                                .output()?
                                .stdout,
                        ) {
                            Ok(url) => url.trim_end_matches(&['\r', '\n'][..]).to_string(),
                            Err(_) => Err(Error::new("Failed to get git url.".to_string()))?,
                        };
                        let hash = match String::from_utf8(
                            SysCommand::new(&sess.config.git)
                                .arg("rev-parse")
                                .arg("HEAD")
                                .current_dir(&dep_path)
                                .output()?
                                .stdout,
                        ) {
                            Ok(hash) => hash.trim_end_matches(&['\r', '\n'][..]).to_string(),
                            Err(_) => Err(Error::new("Failed to get git hash.".to_string()))?,
                        };

                        eprintln!("Snapshotting {} at {} from {}", name, hash, url);

                        snapshot_list.push((name.clone(), url, hash));
                    }
                }
            }
        }
    }

    // Update the Bender.local to keep changes
    let local_path = sess.root.join("Bender.local");
    if local_path.exists() && !snapshot_list.is_empty() {
        let local_file_str = match std::fs::read_to_string(&local_path) {
            Err(why) => Err(Error::new(format!(
                "Reading Bender.local failed with msg:\n\t{}",
                why
            )))?,
            Ok(local_file_str) => local_file_str,
        };
        let mut new_str = String::new();
        if local_file_str.contains("overrides:") {
            let split = local_file_str.split('\n');
            let test = split.clone().next_back().unwrap().is_empty();
            for i in split {
                for (name, _, _) in &snapshot_list {
                    if i.contains(name) {
                        new_str.push('#');
                    }
                }
                new_str.push_str(i);
                new_str.push('\n');
                if i.contains("overrides:") {
                    for (name, url, hash) in &snapshot_list {
                        let dep_str = format!(
                            "  {}: {{ git: \"{}\", rev: \"{}\" }} # Temporary override by Bender using `bender snapshot` command\n",
                            name, url, hash
                        );
                        new_str.push_str(&dep_str);
                    }
                }
            }
            if test {
                // Ensure trailing newline is not duplicated
                new_str.pop();
            }
            if let Err(why) = std::fs::write(local_path, new_str) {
                Err(Error::new(format!(
                    "Writing new Bender.local failed with msg:\n\t{}",
                    why
                )))?
            }
            eprintln!("Bender.local updated with snapshots.");
        }
    }

    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);
    let mut path_subdeps: IndexMap<String, PathBuf> = IndexMap::new();

    for (name, url, _) in &snapshot_list {
        // let old_path = io.get_package_path(depref);
        // let new_path = io.get_depsource_path(name, &DependencySource::Git(url.clone()));
        get_path_subdeps(
            &io,
            &rt,
            &io.get_depsource_path(name, &DependencySource::Git(url.clone())),
            sess.dependency_with_name(name)?,
        )?
        .into_iter()
        .for_each(|(k, v)| {
            path_subdeps.insert(k, v);
        });
    }

    // Update the Bender.lock file with the new hash
    use std::fs::File;
    let file = File::open(sess.root.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot open lockfile {:?}.", sess.root), cause))?;
    let mut locked: Locked = serde_yaml_ng::from_reader(&file).map_err(|cause| {
        Error::chain(format!("Syntax error in lockfile {:?}.", sess.root), cause)
    })?;

    for (name, url, hash) in &snapshot_list {
        let mut mod_package = locked.packages.get_mut(name).unwrap().clone();
        mod_package.revision = Some(hash.to_string());
        mod_package.version = None;
        mod_package.source = LockedSource::Git(url.to_string());
        locked.packages.insert(name.to_string(), mod_package);
    }

    for (path_dep, path_dep_path) in &path_subdeps {
        let mut mod_package = locked.packages[path_dep].clone();
        mod_package.revision = None;
        mod_package.version = None;
        mod_package.source = LockedSource::Path(
            path_dep_path
                .strip_prefix(sess.root)
                .unwrap_or(path_dep_path)
                .to_path_buf(),
        );
        locked.packages.insert(path_dep.clone(), mod_package);
    }

    let file = File::create(sess.root.join("Bender.lock"))
        .map_err(|cause| Error::chain(format!("Cannot create lockfile {:?}.", sess.root), cause))?;
    serde_yaml_ng::to_writer(&file, &locked)
        .map_err(|cause| Error::chain(format!("Cannot write lockfile {:?}.", sess.root), cause))?;

    if args.checkout {
        sess.load_locked(&locked)?;

        let rt = Runtime::new()?;
        let io = SessionIo::new(sess);
        let _srcs = rt.block_on(io.sources(args.force, &[]))?;
    }

    let snapshotted_deps = snapshot_list
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect::<Vec<&str>>();

    let subdeps = path_subdeps
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<&str>>();

    let updated_deps: Vec<&str> = [snapshotted_deps.clone(), subdeps.clone()].concat();

    // Update any possible workspace symlinks
    for (link_path, pkg_name) in &sess.manifest.workspace.package_links {
        if updated_deps.contains(&pkg_name.as_str()) {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, link_path);

            // Determine the checkout path for this package.
            let pkg_path = if snapshotted_deps.contains(&pkg_name.as_str()) {
                &io.get_depsource_path(
                    pkg_name,
                    &DependencySource::Git(
                        snapshot_list
                            .iter()
                            .find(|(n, _, _)| n == pkg_name)
                            .unwrap()
                            .1
                            .clone(),
                    ),
                )
            } else {
                path_subdeps.get(pkg_name).unwrap()
            };
            // let pkg_path = &path.join(path_mod).join(dep);
            let pkg_path = link_path
                .parent()
                .and_then(|path| pathdiff::diff_paths(pkg_path, path))
                .unwrap_or_else(|| pkg_path.into());

            // Check if there is something at the destination path that needs to be
            // removed.
            if link_path.exists() {
                let meta = link_path.symlink_metadata().map_err(|cause| {
                    Error::chain(
                        format!("Failed to read metadata of path {:?}.", link_path),
                        cause,
                    )
                })?;
                if !meta.file_type().is_symlink() {
                    Warnings::SkippingPackageLink(pkg_name.clone(), link_path.to_path_buf()).emit();
                    continue;
                }
                if link_path.read_link().map(|d| d != pkg_path).unwrap_or(true) {
                    debugln!("main: removing existing link {:?}", link_path);
                    remove_symlink_dir(link_path).map_err(|cause| {
                        Error::chain(
                            format!("Failed to remove symlink at path {:?}.", link_path),
                            cause,
                        )
                    })?;
                }
            }

            // Create the symlink if there is nothing at the destination.
            if !link_path.exists() {
                stageln!("Linking", "{} ({:?})", pkg_name, link_path);
                if let Some(parent) = link_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|cause| {
                        Error::chain(format!("Failed to create directory {:?}.", parent), cause)
                    })?;
                }
                let previous_dir = match link_path.parent() {
                    Some(parent) => {
                        let d = std::env::current_dir().unwrap();
                        std::env::set_current_dir(parent).unwrap();
                        Some(d)
                    }
                    None => None,
                };
                symlink_dir(&pkg_path, link_path).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Failed to create symlink to {:?} at path {:?}.",
                            pkg_path, link_path
                        ),
                        cause,
                    )
                })?;
                if let Some(d) = previous_dir {
                    std::env::set_current_dir(d).unwrap();
                }
            }
            eprintln!("{} symlink updated", pkg_name);
        }
    }

    Ok(())
}
