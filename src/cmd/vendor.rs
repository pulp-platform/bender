// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `vendor` subcommand.

use crate::config::PrefixPaths;
use crate::futures::TryFutureExt;
use clap::{Arg, ArgMatches, Command};
use futures::future::{self};
use tokio::runtime::Runtime;

use crate::config;
use crate::error::*;
use crate::git::Git;
use crate::sess::{DependencySource, Session};
use glob::Pattern;
use std::path::Path;
use tempdir::TempDir;

/// Assemble the `vendor` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("vendor")
        .about("Copy source code from upstream vendor repositories into this repository")
        .arg(
            Arg::new("refetch")
                .long("refetch")
                .help("Replace the vendored files from upstream and apply the patches"),
        )
        .arg(
            Arg::new("no_patch")
                .short('n')
                .long("no_patch")
                .help("Do not apply patches when refetching dependencies"),
        )
        .arg(
            Arg::new("gen_patch")
                .long("gen_patch")
                .help("Generate Patch file from changes to the upstream"),
        )
}

/// Execute the `vendor` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;

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
                    .and_then(|_| git.spawn_with(|c| c.arg("checkout").arg(match vendor_package.upstream {
                        config::Dependency::GitRevision(_, ref rev) => rev,
                        // config::Dependency::GitVersion(_, ref ver) => ver.to_str(),
                        _ => unimplemented!(),
                    })))
                    .and_then(|_| async {
                        let rev_hash = match vendor_package.upstream {
                            config::Dependency::GitRevision(_, ref rev) => rev,
                            _ => unimplemented!(),
                        };
                        if *rev_hash != git.spawn_with(|c| c.arg("rev-parse").arg("--verify").arg(format!("{}^{{commit}}", rev_hash))).await?.trim_end_matches('\n') {
                            Err(Error::new("Please ensure your vendor reference is a commit hash to avoid upstream changes impacting your checkout"))
                        } else {
                            Ok(())
                        }
                    })
                    .await
                })?;

                tmp_path.to_path_buf()
            }
            DependencySource::Registry => unimplemented!(),
        };

        if !matches.is_present("refetch") {
            let git = Git::new(tmp_path, &sess.config.git);
            if !matches.is_present("no_patch") {
                vendor_package
                    .mapping
                    .clone()
                    .into_iter()
                    .try_for_each::<_, Result<_>>(|link| {
                        if let Some(patch) = link.patch_dir {
                            // Create directory in case it does not already exist
                            std::fs::create_dir_all(patch.clone())?;

                            let mut patches = std::fs::read_dir(patch)?
                                .map(move |f| f.unwrap().path())
                                .filter(|f| f.extension().unwrap() == "patch")
                                .collect::<Vec<_>>();
                            patches.sort_by_key(|patch_path| {
                                patch_path.to_str().unwrap().to_lowercase()
                            });

                            // for all patches in this directory, git apply them to the to directory
                            let to_link = link.from;
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
                        };
                        Ok(())
                    })?;
                rt.block_on(async {
                    // Add the changes to have a proper comparison
                    if !git
                        .spawn_with(|c| c.arg("status").arg("--short"))
                        .await?
                        .is_empty()
                    {
                        git.spawn_with(|c| c.arg("add").arg("-A")).await?;
                    }
                    Ok::<(), Error>(())
                })?;
            }

            // Copy files from local to temporary repo
            vendor_package
                .mapping
                .clone()
                .into_iter()
                .try_for_each::<_, Result<_>>(|link| {
                    let link_from = link.from.clone().prefix_paths(&dep_path);

                    // Copy src to dst recursively.
                    match &link.to.is_dir() {
                        true => copy_recursively(
                            &link.to,
                            &link_from,
                            &vendor_package
                                .exclude_from_upstream
                                .clone()
                                .into_iter()
                                .map(|excl| {
                                    format!(
                                        "{}/{}",
                                        &vendor_package.target_dir.to_str().unwrap(),
                                        &excl
                                    )
                                })
                                .collect(),
                        )?,
                        false => {
                            std::fs::copy(&link.to, &link_from)?;
                        }
                    };
                    Ok(())
                })?;

            for link in vendor_package.mapping.clone() {
                let get_diff = rt.block_on(async {
                    git.spawn_with(|c| {
                        c.arg("diff")
                            .arg(format!("--relative={}", link.from.to_str().unwrap()))
                    })
                    .await
                })?;

                if !get_diff.is_empty() {
                    if matches.is_present("gen_patch") {
                        if let Some(patch) = link.patch_dir {
                            // Create directory in case it does not already exist
                            std::fs::create_dir_all(patch.clone())?;

                            let mut patches = std::fs::read_dir(patch.clone())?
                                .map(move |f| f.unwrap().path())
                                .filter(|f| f.extension().unwrap() == "patch")
                                .collect::<Vec<_>>();
                            patches.sort_by_key(|patch_path| {
                                patch_path.to_str().unwrap().to_lowercase()
                            });

                            let new_patch = if matches.is_present("no_patch") {
                                // Remove all old patches
                                for patch_file in patches {
                                    std::fs::remove_file(patch_file)?;
                                }
                                "0001-bender-vendor.patch".to_string()
                            } else {
                                // Get all patch leading numeric keys (0001, ...) and generate new name
                                let leading_numbers = patches
                                    .iter()
                                    .map(|file_path| {
                                        file_path.file_name().unwrap().to_str().unwrap()
                                    })
                                    .map(|s| &s[..4])
                                    .collect::<Vec<_>>();
                                if !leading_numbers
                                    .iter()
                                    .all(|s| s.chars().all(char::is_numeric))
                                {
                                    Err(Error::new(format!("Please ensure all patches start with four numbers for proper ordering in {}:{:?}", vendor_package.name, link.from)))?;
                                }
                                let max_number = leading_numbers
                                    .iter()
                                    .map(|s| s.parse::<i32>().unwrap())
                                    .max()
                                    .unwrap();
                                format!("{:04}-bender-vendor.patch", max_number + 1)
                            };

                            // write patch
                            std::fs::write(patch.join(new_patch), get_diff)?;
                        } else {
                            Err(Error::new(format!(
                                "Please ensure a patch_dir is defined for {}: {:?}",
                                vendor_package.name, link.from
                            )))?;
                        }
                    } else {
                        println!("In {}: {:?}:", vendor_package.name, link.from);
                        println!("{}", get_diff);
                    }
                }
            }
        } else {
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

                    // Copy src to dst recursively.
                    match &link.from.clone().prefix_paths(&dep_path).is_dir() {
                        true => copy_recursively(
                            &link.from.prefix_paths(&dep_path),
                            &link.to,
                            &vendor_package
                                .exclude_from_upstream
                                .clone()
                                .into_iter()
                                .map(|excl| format!("{}/{}", &dep_path.to_str().unwrap(), &excl))
                                .collect(),
                        )?,
                        false => {
                            std::fs::copy(&link.from.prefix_paths(&dep_path), &link.to)?;
                        }
                    };
                    Ok(())
                })?;

            if !matches.is_present("no_patch") {
                vendor_package
                    .mapping
                    .clone()
                    .into_iter()
                    .try_for_each::<_, Result<_>>(|link| {
                        match link.patch_dir {
                            Some(patch) => {
                                // Create directory in case it does not already exist
                                std::fs::create_dir_all(patch.clone())?;

                                let mut patches = std::fs::read_dir(patch)?
                                    .map(move |f| f.unwrap().path())
                                    .filter(|f| f.extension().unwrap() == "patch")
                                    .collect::<Vec<_>>();
                                patches.sort_by_key(|patch_path| {
                                    patch_path.to_str().unwrap().to_lowercase()
                                });

                                // for all patches in this directory, git apply them to the to directory
                                let git = Git::new(sess.root, &sess.config.git);
                                let to_link = link.to.strip_prefix(sess.root).map_err(|cause| {
                                    Error::chain("Failed to strip path.", cause)
                                })?;
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
            }
        }
    }

    Ok(())
}

/// recursive copy function
pub fn copy_recursively(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    ignore: &Vec<String>,
) -> Result<()> {
    std::fs::create_dir_all(&destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;

        if ignore.iter().any(|ignore_path| {
            Pattern::new(ignore_path)
                .unwrap()
                .matches_path(&entry.path())
        }) {
            continue;
        }

        let filetype = entry.file_type()?;
        if filetype.is_dir() {
            copy_recursively(
                entry.path(),
                destination.as_ref().join(entry.file_name()),
                ignore,
            )?;
        } else {
            std::fs::copy(entry.path(), destination.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
