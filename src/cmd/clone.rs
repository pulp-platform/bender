// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `clone` subcommand.

use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

use clap::Args;
use indexmap::IndexMap;
use miette::{Context as _, IntoDiagnostic as _};
use tokio::runtime::Runtime;

use crate::bail;
use crate::cli::{remove_symlink_dir, symlink_dir};
use crate::config;
use crate::config::{Locked, LockedSource};
use crate::diagnostic::Warnings;
use crate::error::*;
use crate::sess::{DependencyRef, DependencySource, Session, SessionIo};
use crate::{debugln, fmt_path, fmt_pkg, stageln};

/// Clone dependency to a working directory
#[derive(Args, Debug)]
pub struct CloneArgs {
    /// Package name to clone to a working directory
    pub name: String,

    /// Relative directory to clone PKG into
    #[arg(short, long, default_value = "working_dir")]
    pub path: String,
}

/// Execute the `clone` subcommand.
pub fn run(sess: &Session, path: &Path, args: &CloneArgs) -> Result<()> {
    let dep = &args.name.to_lowercase();
    let depref = sess.dependency_with_name(dep)?;

    let path_mod = &args.path; // TODO make this option for config in the Bender.yml file?
    // Check current config for matches
    if sess.config.overrides.contains_key(dep) {
        match &sess.config.overrides[dep] {
            config::Dependency::Path { path: p, .. } => {
                bail!(
                    "Dependency `{}` already has a path override at\n\t{}\n\tPlease check Bender.local or .bender.yml",
                    dep,
                    p.to_str().unwrap()
                );
            }
            _ => {
                eprintln!("A non-path override is already present, proceeding anyways");
            }
        }
    }

    // Check if dependency is a git dependency
    match sess.dependency_source(depref) {
        DependencySource::Git { .. } | DependencySource::Registry => {}
        DependencySource::Path { .. } => {
            bail!(
                "Dependency `{}` is a path dependency. `clone` is only implemented for git dependencies.",
                dep
            );
        }
    }

    // Create dir
    if !path.join(path_mod).exists()
        && !SysCommand::new("mkdir")
            .arg(path_mod)
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    {
        bail!("Creating dir {} failed", path_mod);
    }
    let rt = Runtime::new().into_diagnostic()?;
    let io = SessionIo::new(sess);

    // Copy dependency to dir for proper workflow
    if path.join(path_mod).join(dep).exists() {
        eprintln!("{} already has a directory in {}.", dep, path_mod);
        eprintln!("Please manually ensure the correct checkout.");
    } else {
        let id = sess.dependency_with_name(&args.name.to_lowercase())?;
        debugln!("main: obtain checkout {:?}", id);
        let checkout = rt.block_on(io.checkout(id, false, &[]))?;
        debugln!("main: checkout {:#?}", checkout);
        if let Some(s) = checkout.to_str() {
            if !Path::new(s).exists() {
                bail!("`{dep}` path `{s}` does not exist");
            }
            let command = SysCommand::new("cp")
                .arg("-rf")
                .arg(s)
                .arg(path.join(path_mod).join(dep).to_str().unwrap())
                .status();
            if !command.unwrap().success() {
                bail!("Copying {} failed", dep);
            }
        }

        // rename and update git remotes for easier handling
        if !SysCommand::new(&sess.config.git)
            .arg("remote")
            .arg("rename")
            .arg("origin")
            .arg("source")
            .current_dir(path.join(path_mod).join(dep))
            .status()
            .unwrap()
            .success()
        {
            bail!("git renaming remote origin failed");
        }

        if !SysCommand::new(&sess.config.git)
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg(
                sess.dependency(sess.dependency_with_name(dep)?)
                    .source
                    .to_str(),
            )
            .current_dir(path.join(path_mod).join(dep))
            .status()
            .unwrap()
            .success()
        {
            bail!("git adding remote failed");
        }

        if !sess.local_only {
            if !SysCommand::new(&sess.config.git)
                .arg("fetch")
                .arg("--all")
                .current_dir(path.join(path_mod).join(dep))
                .status()
                .unwrap()
                .success()
            {
                bail!("git fetch failed");
            }
        } else {
            Warnings::LocalNoFetch.emit();
        }

        eprintln!(
            "{} checkout added in {:?}",
            dep,
            path.join(path_mod).join(dep)
        );
    }

    // Rewrite Bender.local file to keep changes
    let local_path = path.join("Bender.local");
    let dep_str = format!(
        "  {}: {{ path: \"{}/{0}\" }} # Temporary override by Bender using `bender clone` command\n",
        dep, path_mod
    );
    if local_path.exists() {
        let local_file_str = std::fs::read_to_string(&local_path)
            .into_diagnostic()
            .wrap_err_with(|| format!("Reading Bender.local failed at {:?}.", local_path))?;
        let mut new_str = String::new();
        if local_file_str.contains("overrides:") {
            let split = local_file_str.split('\n');
            for i in split {
                if i.contains(dep) {
                    new_str.push('#');
                }
                new_str.push_str(i);
                new_str.push('\n');
                if i.contains("overrides:") {
                    new_str.push_str(&dep_str);
                }
            }
            if local_file_str.ends_with('\n') {
                // Ensure trailing newline is not duplicated
                new_str.pop();
            }
        } else {
            new_str.push_str("overrides:\n");
            new_str.push_str(&dep_str);
            new_str.push_str(&local_file_str);
        }
        std::fs::write(local_path.clone(), new_str)
            .into_diagnostic()
            .wrap_err_with(|| format!("Writing new Bender.local failed at {:?}.", local_path))?;
    } else {
        std::fs::write(local_path.clone(), format!("overrides:\n{}", dep_str))
            .into_diagnostic()
            .wrap_err_with(|| format!("Writing new Bender.local failed at {:?}.", local_path))?;
    };

    eprintln!("{} dependency added to Bender.local", dep);

    // Update Bender.lock to enforce usage
    use std::fs::File;
    let file = File::open(path.join("Bender.lock"))
        .into_diagnostic()
        .wrap_err_with(|| format!("Cannot open lockfile {:?}.", path))?;
    let mut locked: Locked = serde_yaml_ng::from_reader(&file)
        .into_diagnostic()
        .wrap_err_with(|| format!("Syntax error in lockfile {:?}.", path))?;

    let path_deps = get_path_subdeps(&io, &rt, &path.join(path_mod).join(dep), depref)?;

    let mut mod_package = locked.packages[dep].clone();
    mod_package.revision = None;
    mod_package.version = None;
    mod_package.source = LockedSource::Path(
        path.join(path_mod)
            .join(dep)
            .strip_prefix(path)
            .unwrap_or(&path.join(path_mod).join(dep))
            .to_path_buf(),
    );
    locked.packages.insert(dep.to_string(), mod_package);
    for path_dep in path_deps {
        let mut mod_package = locked.packages[&path_dep.0].clone();
        mod_package.revision = None;
        mod_package.version = None;
        mod_package.source = LockedSource::Path(
            path_dep
                .1
                .strip_prefix(path)
                .unwrap_or(&path_dep.1)
                .to_path_buf(),
        );
        locked.packages.insert(path_dep.0.clone(), mod_package);
    }

    let file = File::create(path.join("Bender.lock"))
        .into_diagnostic()
        .wrap_err_with(|| format!("Cannot create lockfile {:?}.", path))?;
    serde_yaml_ng::to_writer(&file, &locked)
        .into_diagnostic()
        .wrap_err_with(|| format!("Cannot write lockfile {:?}.", path))?;

    eprintln!("Lockfile updated");

    // Update any possible workspace symlinks
    for (link_path, pkg_name) in &sess.manifest.workspace.package_links {
        if pkg_name == dep {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, link_path);

            // Determine the checkout path for this package.
            let pkg_path = &path.join(path_mod).join(dep);
            let pkg_path = link_path
                .parent()
                .and_then(|path| pathdiff::diff_paths(pkg_path, path))
                .unwrap_or_else(|| pkg_path.into());

            // Check if there is something at the destination path that needs to be
            // removed.
            if link_path.exists() {
                let meta = link_path
                    .symlink_metadata()
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("Failed to read metadata of path {:?}.", link_path)
                    })?;
                if !meta.file_type().is_symlink() {
                    Warnings::SkippingPackageLink(pkg_name.clone(), link_path.to_path_buf()).emit();
                    continue;
                }
                if link_path.read_link().map(|d| d != pkg_path).unwrap_or(true) {
                    debugln!("main: removing existing link {:?}", link_path);
                    remove_symlink_dir(link_path).wrap_err_with(|| {
                        format!("Failed to remove symlink at path {:?}.", link_path)
                    })?;
                }
            }

            // Create the symlink if there is nothing at the destination.
            if !link_path.exists() {
                if let Some(parent) = link_path.parent() {
                    std::fs::create_dir_all(parent)
                        .into_diagnostic()
                        .wrap_err_with(|| format!("Failed to create directory {:?}.", parent))?;
                }
                let previous_dir = match link_path.parent() {
                    Some(parent) => {
                        let d = std::env::current_dir().unwrap();
                        std::env::set_current_dir(parent).unwrap();
                        Some(d)
                    }
                    None => None,
                };
                symlink_dir(&pkg_path, link_path).wrap_err_with(|| {
                    format!(
                        "Failed to create symlink to {:?} at path {:?}.",
                        pkg_path, link_path
                    )
                })?;
                if let Some(d) = previous_dir {
                    std::env::set_current_dir(d).unwrap();
                }
                stageln!(
                    "Linked",
                    "{} to {}",
                    fmt_pkg!(pkg_name),
                    fmt_path!(path.display())
                );
            }
            eprintln!("{} symlink updated", dep);
        }
    }

    Ok(())
}

/// A helper function to recursively get all path subdependencies of a dependency.
pub fn get_path_subdeps(
    io: &SessionIo,
    rt: &Runtime,
    path: &Path,
    depref: DependencyRef,
) -> Result<IndexMap<String, PathBuf>> {
    let binding = IndexMap::new();
    let old_path = io.get_package_path(depref);
    let mut path_deps = match rt.block_on(io.dependency_manifest(depref, false, &[]))? {
        Some(m) => &m.dependencies,
        None => &binding,
    }
    .iter()
    .filter_map(|(k, v)| match v {
        config::Dependency::Path { path: p, .. } => {
            if p.starts_with(&old_path) {
                Some((
                    k.clone(),
                    path.join(p.strip_prefix(&old_path).unwrap()).to_path_buf(),
                ))
            } else {
                None
            }
        }
        _ => None,
    })
    .collect::<IndexMap<String, std::path::PathBuf>>();
    let path_dep_list = path_deps
        .iter()
        .map(|(k, _)| k.clone())
        .collect::<Vec<String>>();
    for name in &path_dep_list {
        get_path_subdeps(io, rt, path, io.sess.dependency_with_name(name)?)?
            .into_iter()
            .for_each(|(k, v)| {
                path_deps.insert(k.clone(), v.clone());
            });
    }
    Ok(path_deps)
}
