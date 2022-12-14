// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>
// Nils Wistoff <nwistoff@iis.ee.ethz.ch>

//! The `vendor` subcommand.

use crate::config::PrefixPaths;
use crate::futures::TryFutureExt;
use clap::{Arg, ArgMatches, Command, AppSettings};
use futures::future::{self};
use tokio::runtime::Runtime;

use crate::config;
use crate::error::*;
use crate::git::Git;
use crate::sess::{DependencySource, Session};
use glob::Pattern;
use std::path::Path;
use std::path::PathBuf;
use tempdir::TempDir;

/// A patch linkage
#[derive(Clone)]
pub struct PatchLink {
    /// directory
    pub patch_dir: Option<PathBuf>,
    /// prefix for upstream
    pub from_prefix: PathBuf,
    /// prefix for local
    pub to_prefix: PathBuf,
}

/// Assemble the `vendor` subcommand.
pub fn new<'a>() -> Command<'a> {
    Command::new("vendor")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .about("Copy source code from upstream external repositories into this repository. Functions similar to the lowrisc vendor.py script. Type bender vendor <SUBCOMMAND> --help for more information about the subcommands.")
        .subcommand(Command::new("diff")
            .about("Display a diff of the local tree and the upstream tree with patches applied.")
        )
        .subcommand(Command::new("init")
            .about("(Re-)initialize the external dependencies. Copies the upstream files into the target directories and applies existing patches.")
            .arg(
                Arg::new("no_patch")
                    .short('n')
                    .long("no_patch")
                    .help("Do not apply patches when initializing dependencies"),
            )
        )
        .subcommand(Command::new("patch")
            .about("Generate a patch file from staged local changes")
            .arg(
                Arg::new("plain")
                .long("plain")
                .help("Generate a plain diff instead of a format-patch. Includes all local changes (not only the staged ones)."),
            )
        )
}

/// Execute the `vendor` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;

    for vendor_package in &sess.manifest.vendor_package {
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

        // Extract patch dirs of links
        let mut patch_links: Vec<PatchLink> = Vec::new();
        for link in vendor_package.mapping.clone() {
            patch_links.push(
                PatchLink {
                    patch_dir: link.patch_dir,
                    from_prefix: link.from,
                    to_prefix: link.to,
                }
            )
        }

        // If links do not specify patch dirs, use package-wide patch dir
        let patch_links = {
            match patch_links[..] {
                [] => vec![PatchLink {
                    patch_dir: vendor_package.patch_dir.clone(),
                    from_prefix: PathBuf::from(""),
                    to_prefix: PathBuf::from(""),
                }],
                _ => patch_links,
            }
        };

        let git = Git::new(tmp_path, &sess.config.git);

        match matches.subcommand() {
            Some(("diff", _)) => {
                // Apply patches
                patch_links.clone().into_iter().try_for_each( |patch_link| {
                    apply_patches(&rt, git, vendor_package.name.clone(), patch_link).map(|_| ())
                })?;

                // Stage applied patches to clean working tree
                rt.block_on(git.add_all())?;

                // Print diff for each link
                patch_links.into_iter().try_for_each( |patch_link| {
                    let get_diff = diff(&rt,
                                        git,
                                        vendor_package,
                                        patch_link,
                                        dep_path.clone())
                                   .expect("failed to get diff");
                    if !get_diff.is_empty() {
                        print!("{}", get_diff);
                    }
                    Ok(())
                })
            },

            Some(("init", matches)) => {
                patch_links.clone().into_iter().try_for_each( |patch_link| {
                    stageln!("Copying", "{} files from upstream", vendor_package.name);
                    // Remove existing directories before importing them again
                    std::fs::remove_dir_all(patch_link.clone().to_prefix.prefix_paths(&vendor_package.target_dir)).unwrap_or(());
                    // init
                    init(&rt, git, vendor_package, patch_link, dep_path.clone(), matches)
                })
            },

            Some(("patch", matches)) => {
                // Apply patches
                let mut num_patches = 0;
                patch_links.clone().into_iter().try_for_each( |patch_link| {
                    apply_patches(&rt, git, vendor_package.name.clone(), patch_link).map(|num| num_patches += num)
                })?;

                // Commit applied patches to clean working tree
                if num_patches > 0 {
                    rt.block_on(git.add_all())?;
                    rt.block_on(git.commit(Some("pre-patch")))?;
                }

                // Generate patch
                patch_links.clone().into_iter().try_for_each( |patch_link| {
                    match patch_link.patch_dir.clone() {
                        Some(patch_dir) => {
                            if matches.is_present("plain") {
                                let get_diff = diff(&rt,
                                                    git,
                                                    vendor_package,
                                                    patch_link.clone(),
                                                    dep_path.clone())
                                            .expect("failed to get diff");
                                gen_plain_patch(get_diff, patch_dir, false)
                            } else {
                                gen_format_patch(&rt, &sess, git, patch_link, vendor_package.target_dir.clone())
                            }
                        },
                        None => {
                            warnln!("No patch directory specified for package {}, mapping {} => {}. Skipping patch generation.", vendor_package.name.clone(), patch_link.from_prefix.to_str().unwrap(), patch_link.to_prefix.to_str().unwrap());
                            Ok(())
                        },
                    }
                })
            },
            _ => Ok(()),
        }?;
    };

    Ok(())
}

/// initialize the external dependency
pub fn init(
    rt: &Runtime,
    git: Git,
    vendor_package: &config::VendorPackage,
    patch_link: PatchLink,
    dep_path: impl AsRef<Path>,
    matches: &ArgMatches,
) -> Result<()> {
    // import necessary files from upstream, apply patches
    let dep_path = dep_path.as_ref();

    // Make sure the target directory actually exists
    let link_to = patch_link.to_prefix.clone().prefix_paths(&vendor_package.target_dir);
    let link_from = patch_link.from_prefix.clone().prefix_paths(&dep_path);
    std::fs::create_dir_all(&link_to.parent().unwrap())?;

    if !matches.is_present("no_patch") {
        apply_patches(
            &rt,
            git,
            vendor_package.name.clone(),
            patch_link.clone()
        )?;
    }

    // Copy src to dst recursively.
    match &patch_link.from_prefix.clone().prefix_paths(&dep_path).is_dir() {
        true => copy_recursively(
            &link_from,
            &link_to,
            &extend_paths(&vendor_package.include_from_upstream, dep_path),
            &vendor_package
                .exclude_from_upstream
                .clone()
                .into_iter()
                .map(|excl| format!("{}/{}", &dep_path.to_str().unwrap(), &excl))
                .collect(),
        )?,
        false => {
            std::fs::copy(&link_from, &link_to)?;
        }
    };

    Ok(())
}

/// apply existing patches
pub fn apply_patches(
    rt: &Runtime,
    git: Git,
    package_name: String,
    patch_link: PatchLink,
) -> Result<usize> {
    if let Some(patch_dir) = patch_link.patch_dir.clone() {
        // Create directory in case it does not already exist
        std::fs::create_dir_all(patch_dir.clone())?;

        let mut patches = std::fs::read_dir(patch_dir.clone())?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().is_some())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| {
            patch_path.to_str().unwrap().to_lowercase()
        });

        for patch in patches.clone() {
            rt.block_on(async {
                // TODO MICHAERO: May need throttle
                future::lazy(|_| {
                    stageln!(
                        "Patching",
                        "{} with {}",
                        package_name,
                        patch.file_name().unwrap().to_str().unwrap()
                    );
                    Ok(())
                })
                .and_then(|_| {
                    git.spawn_with(|c| {
                        c.arg("apply")
                            .arg("--directory")
                            .arg(patch_link.from_prefix.clone().to_str().unwrap())
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
        Ok(patches.len())
    } else {
        Ok(0)
    }
}

/// Generate diff
pub fn diff(
    rt: &Runtime,
    git: Git,
    vendor_package: &config::VendorPackage,
    patch_link: PatchLink,
    dep_path: impl AsRef<Path>

) -> Result<String> {
        // Copy files from local to temporary repo
        let link_from = patch_link.from_prefix.clone().prefix_paths(dep_path.as_ref()); // dep_path: path to temporary clone. link_to: targetdir/link.to
        let link_to = patch_link.to_prefix.clone().prefix_paths(vendor_package.target_dir.as_ref());
        // Copy src to dst recursively.
        match &link_to.is_dir() {
            true => copy_recursively(
                &link_to,
                &link_from,
                &extend_paths(&vendor_package.include_from_upstream, &vendor_package.target_dir),
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
                std::fs::copy(&link_to, &link_from)?;
            }
        };
        // Get diff
        rt.block_on(async {
            git.spawn_with(|c| c.arg("diff").arg(format!("--relative={}", patch_link.from_prefix.to_str().expect("Failed to convert from_prefix to string."))))
                .await
        })
}

/// Generate a plain patch from a diff
pub fn gen_plain_patch(
    diff: String,
    patch_dir: impl AsRef<Path>,
    no_patch: bool,
) -> Result<()> {
    if !diff.is_empty() {
        // if let Some(patch) = patch_dir {
        // Create directory in case it does not already exist
        std::fs::create_dir_all(patch_dir.as_ref().clone())?;

        let mut patches = std::fs::read_dir(patch_dir.as_ref())?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| {
            patch_path.to_str().unwrap().to_lowercase()
        });

        let new_patch = if no_patch || patches.is_empty()
        {
            // Remove all old patches
            for patch_file in patches {
                std::fs::remove_file(patch_file)?;
            }
            "0001-bender-import.patch".to_string()
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
                Err(Error::new(format!("Please ensure all patches start with four numbers for proper ordering in {}", patch_dir.as_ref().to_str().unwrap())))?;
            }
            let max_number = leading_numbers
                .iter()
                .map(|s| s.parse::<i32>().unwrap())
                .max()
                .unwrap();
            format!("{:04}-bender-import.patch", max_number + 1)
        };

        // write patch
        std::fs::write(patch_dir.as_ref().join(new_patch), diff)?;
        // }
    }

    Ok(())
}

/// Commit changes staged in ghost repo and generate format patch
pub fn gen_format_patch(
    rt: &Runtime,
    sess: &Session,
    git: Git,
    patch_link: PatchLink,
    target_dir: impl AsRef<Path>,
) -> Result<()> {
    // Local git
    let git_parent = Git::new(sess.root, &sess.config.git);

    // We assume that patch_dir matches Some() was checked outside this function.
    let patch_dir = patch_link.patch_dir.clone().unwrap();

    // Get staged changes in dependency
    let get_diff_cached = rt.block_on(async {
        git_parent.spawn_with(|c| c.arg("diff").arg(format!("--relative={}", patch_link.to_prefix.clone().prefix_paths(target_dir.as_ref()).to_str().unwrap())).arg("--cached"))
        .await
    })?;

    if !get_diff_cached.is_empty()
    {
        // Write diff into new temp dir. TODO: pipe directly to "git apply"
        let tmp_format_dir = TempDir::new(".bender.format.tmp")?;
        let tmp_format_path = tmp_format_dir.path();
        let diff_cached_path = tmp_format_path.join("staged.diff");
        std::fs::write(diff_cached_path.clone(), get_diff_cached.clone())?;

        // Apply diff and stage changes in ghost repo
        rt.block_on(async {
            git.spawn_with(|c| {
                c.arg("apply")
                    .arg("--directory")
                    .arg(&patch_link.from_prefix)
                    .arg("-p1")
                    .arg(&diff_cached_path)
            })
            .and_then(|_| {
                git.spawn_with (|c| {
                    c.arg("add")
                    .arg("--all")
                })
            })
            .await
        })?;

        // Commit all staged changes in ghost repo
        rt.block_on(git.commit(None))?;

        // Create directory in case it does not already exist
        std::fs::create_dir_all(patch_dir.clone())?;

        let mut patches = std::fs::read_dir(patch_dir.clone())?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().is_some())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| {
            patch_path.to_str().unwrap().to_lowercase()
        });

        // Determine max number
        let max_number = if patches.is_empty() {
            0
        } else {
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
                Err(Error::new(format!("Please ensure all patches start with four numbers for proper ordering in {}", patch_dir.to_str().unwrap())))?;
            }
            leading_numbers
                .iter()
                .map(|s| s.parse::<i32>().unwrap())
                .max()
                .unwrap()
        };

        // Generate format-patch
        rt.block_on(async {
            git.spawn_with(|c| {
                c.arg("format-patch")
                    .arg("-o")
                    .arg(patch_dir.to_str().unwrap())
                    .arg("-1")
                    .arg(format!("--start-number={}", max_number + 1))
                    .arg(format!("--relative={}", patch_link.from_prefix.to_str().unwrap()))
                    .arg("HEAD")
            })
            .await
        })?;
    }
    Ok(())
}


/// recursive copy function
pub fn copy_recursively(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    includes: &Vec<String>,
    ignore: &Vec<String>,
) -> Result<()> {
    std::fs::create_dir_all(&destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;

        if !includes.iter().any(|include| {
            PathBuf::from(include).ancestors().any(|include_path| {
                Pattern::new(include_path.to_str().unwrap())
                    .unwrap()
                    .matches_path(&entry.path())
            })
        })
        || ignore.iter().any(|ignore_path| {
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
                includes,
                ignore,
            )?;
        } else {
            std::fs::copy(entry.path(), destination.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Prefix paths with prefix. Append ** to directories.
pub fn extend_paths(
    include_from_upstream: &Vec<String>,
    prefix: impl AsRef<Path>,
) -> Vec<String> {
    include_from_upstream.into_iter().map(|pattern| {
        let pattern_long = PathBuf::from(pattern).prefix_paths(prefix.as_ref());
        if pattern_long.is_dir() {
            String::from(pattern_long.join("**").to_str().unwrap())
        } else {
            String::from(pattern_long.to_str().unwrap())
        }
    }).collect()
}
