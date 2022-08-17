// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `vendor` subcommand.

use crate::config::PrefixPaths;
use crate::futures::TryFutureExt;
use clap::{ArgMatches, Command};
use futures::future::{self};
use tokio::runtime::Runtime;

use crate::config;
use crate::error::*;
use crate::git::Git;
use crate::sess::{DependencySource, Session};
use std::path::Path;
use tempdir::TempDir;

/// Assemble the `vendor` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("vendor")
        .about("Copy source code from upstream vendor repositories into this repository")
}

/// Execute the `vendor` subcommand.
pub fn run(sess: &Session, _matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;
    println!("{:?}", sess.manifest.vendor);

    for vendor_package in &sess.manifest.vendor {
        // Clone upstream into a temporary directory (or make use of .bender/db?)
        let dep_src = DependencySource::from(&vendor_package.upstream);
        let tmp_dir = TempDir::new(&vendor_package.name)?;
        let tmp_path = tmp_dir.path();
        let dep_path = match dep_src {
            DependencySource::Path(path) => path,
            DependencySource::Git(ref url) => {
                let git = Git::new(tmp_path, &sess.config.git);
                rt.block_on(async {
                    // TODO MICHAERO: May need throttle
                    future::lazy(|_| {
                        stageln!("Cloning", "{} ({})", vendor_package.name, url);
                        Ok(())
                    })
                    .and_then(|_| git.spawn_with(|c| c.arg("clone").arg(url).arg(".")))
                    .and_then(|_| git.spawn_with(|c| c.arg("checkout").arg(match vendor_package.upstream {
                        config::Dependency::GitRevision(_, ref rev) => rev,
                        // config::Dependency::GitVersion(_, ref ver) => ver.to_str(),
                        _ => unimplemented!(),
                    })))
                    .await
                    .map_err(move |cause| {
                        if url.contains("git@") {
                            warnln!("Please ensure your public ssh key is added to the git server.");
                        }
                        warnln!("Please ensure the url is correct and you have access to the repository.");
                        Error::chain(
                            format!("Failed to initialize git database in {:?}.", tmp_path),
                            cause,
                        )
                    })
                    .map(move |_| git)
                })?;
                tmp_path.to_path_buf()
            }
            DependencySource::Registry => unimplemented!(),
        };

        // import necessary files from upstream, apply patches
        stageln!("Copying", "{} files from upstream", vendor_package.name);
        // Remove existing directories before importing them again
        std::fs::remove_dir_all(vendor_package.target_dir.clone()).unwrap_or(());

        vendor_package
            .mapping
            .clone()
            .into_iter()
            .try_for_each::<_, Result<_>>(|link| {
                // Make sure the target directory actually exists
                std::fs::create_dir_all(&link.to.parent().unwrap())?;

                // Copy src to dst recursively. For directories, we can use
                // shutil.copytree. This doesn't support files, though, so we have to
                // check for them first.
                // Also set patch path here as parent directory iff dst is a file.
                match &link.from.clone().prefix_paths(&dep_path).is_dir() {
                    true => copy_recursively(&link.from.prefix_paths(&dep_path), &link.to)?,
                    false => {
                        std::fs::copy(&link.from.prefix_paths(&dep_path), &link.to)?;
                    }
                };
                Ok(())
            })?;

        vendor_package
            .mapping
            .clone()
            .into_iter()
            .try_for_each::<_, Result<_>>(|link| {
                match link.patch_dir {
                    Some(patch) => {
                        let patches = std::fs::read_dir(patch)
                            .unwrap()
                            .map(move |f| f.unwrap().path())
                            .filter(|f| f.extension().unwrap() == "patch")
                            .collect::<Vec<_>>();

                        // for all patches in this directory, git apply them to the to directory
                        let git = Git::new(&sess.root, &sess.config.git);
                        let to_link = link
                            .to
                            .strip_prefix(sess.root)
                            .map_err(|cause| Error::chain("Failed to strip path.", cause))?;
                        for patch in patches {
                            rt.block_on(async {
                                // TODO MICHAERO: May need throttle
                                future::lazy(|_| {
                                    stageln!(
                                        "Patching",
                                        "{} with {}",
                                        vendor_package.name,
                                        patch.file_name().unwrap().to_str().unwrap()
                                    );
                                    Ok(())
                                })
                                .and_then(|_| {
                                    git.spawn_with(|c| {
                                        c.arg("apply")
                                            .arg("--directory")
                                            .arg(&to_link)
                                            .arg("-p1")
                                            .arg(&patch)
                                    })
                                })
                                .await
                                .map_err(move |cause| {
                                    Error::chain(
                                        format!("Failed to apply patch {:?}.", patch),
                                        cause,
                                    )
                                })
                                .map(move |_| git)
                            })?;
                        }
                    }
                    None => {}
                };
                Ok(())
            })?;

        // Update lockfile

        println!("{:?}", tmp_path);
        // tmp_dir.close()?;
    }

    Ok(())
}

/// recursive copy function
pub fn copy_recursively(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let filetype = entry.file_type()?;
        if filetype.is_dir() {
            copy_recursively(entry.path(), destination.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), destination.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
