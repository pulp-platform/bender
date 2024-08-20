// Copyright (c) 2022 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>
// Nils Wistoff <nwistoff@iis.ee.ethz.ch>

//! The `vendor` subcommand.

use crate::config::PrefixPaths;
use crate::futures::TryFutureExt;
use clap::{Arg, ArgAction, ArgMatches, Command};
use futures::future::{self};
use tokio::runtime::Runtime;

use crate::config;
use crate::error::*;
use crate::git::Git;
use crate::sess::{DependencySource, Session};
use glob::Pattern;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;

/// A patch linkage
#[derive(Clone)]
pub struct PatchLink {
    /// directory
    pub patch_dir: Option<PathBuf>,
    /// prefix for upstream
    pub from_prefix: PathBuf,
    /// prefix for local
    pub to_prefix: PathBuf,
    /// subdirs and files to exclude
    pub exclude: Vec<PathBuf>,
}

/// Assemble the `vendor` subcommand.
pub fn new() -> Command {
    Command::new("vendor")
        .subcommand_required(true).arg_required_else_help(true)
        .about("Copy source code from upstream external repositories into this repository")
        .long_about("Copy source code from upstream external repositories into this repository. Functions similar to the lowrisc vendor.py script.")
        .after_help("Type 'bender vendor <SUBCOMMAND> --help' for more information about a vendor subcommand.")
        .subcommand(Command::new("diff")
            .about("Display a diff of the local tree and the upstream tree with patches applied.")
            .arg(
                Arg::new("err_on_diff")
                    .long("err_on_diff")
                    .short('e')
                    .num_args(0..=1)
                    .help("Return error code 1 when a diff is encountered. (Optional) override the error message by providing a value."),
            )
        )
        .subcommand(Command::new("init")
            .about("(Re-)initialize the external dependencies.")
            .long_about("(Re-)initialize the external dependencies. Copies the upstream files into the target directories and applies existing patches.")
            .arg(
                Arg::new("no_patch")
                    .short('n')
                    .action(ArgAction::SetTrue)
                    .long("no_patch")
                    .help("Do not apply patches when initializing dependencies"),
            )
        )
        .subcommand(Command::new("patch")
            .about("Generate a patch file from staged local changes")
            .arg(
                Arg::new("plain")
                .action(ArgAction::SetTrue)
                .long("plain")
                .help("Generate a plain diff instead of a format-patch.")
                .long_help("Generate a plain diff instead of a format-patch. Includes all local changes (not only the staged ones)."),
            )
            .arg(
                Arg::new("message")
                .long("message")
                .short('m')
                .num_args(1)
                .action(ArgAction::Append)
                .help("The message to be associated with the format-patch."),
            )
        )
}

/// Execute the `vendor` subcommand.
pub fn run(sess: &Session, matches: &ArgMatches) -> Result<()> {
    let rt = Runtime::new()?;

    for vendor_package in &sess.manifest.vendor_package {
        // Clone upstream into a temporary directory (or make use of .bender/db?)
        let dep_src = DependencySource::from(&vendor_package.upstream);
        let tmp_dir = TempDir::new()?;
        let tmp_path = tmp_dir.path();
        let dep_path = match dep_src {
            DependencySource::Path(path) => path,
            DependencySource::Git(ref url) => {
                let git = Git::new(tmp_path, &sess.config.git);
                rt.block_on(async {
                    stageln!("Cloning", "{} ({})", vendor_package.name, url);
                    git.spawn_with(|c| c.arg("clone").arg(url).arg("."))
                    .map_err(move |cause| {
                        if url.contains("git@") {
                            warnln!("Please ensure your public ssh key is added to the git server.");
                        }
                        warnln!("Please ensure the url is correct and you have access to the repository.");
                        Error::chain(
                            format!("Failed to initialize git database in {:?}.", tmp_path),
                            cause,
                        )
                    }).await?;
                    let rev_hash = match vendor_package.upstream {
                        config::Dependency::GitRevision(_, ref rev) => Ok(rev),
                        _ => Err(Error::new("Please ensure your vendor reference is a commit hash to avoid upstream changes impacting your checkout")),
                    }?;
                    git.spawn_with(|c| c.arg("checkout").arg(rev_hash)).await?;
                    if *rev_hash != git.spawn_with(|c| c.arg("rev-parse").arg("--verify").arg(format!("{}^{{commit}}", rev_hash))).await?.trim_end_matches('\n') {
                        Err(Error::new("Please ensure your vendor reference is a commit hash to avoid upstream changes impacting your checkout"))
                    } else {
                        Ok(())
                    }
                })?;

                tmp_path.to_path_buf()
            }
            DependencySource::Registry => unimplemented!(),
        };

        // Extract patch dirs of links
        let mut patch_links: Vec<PatchLink> = Vec::new();
        for link in vendor_package.mapping.clone() {
            patch_links.push(PatchLink {
                patch_dir: link.patch_dir,
                from_prefix: link.from,
                to_prefix: link.to,
                exclude: vec![],
            })
        }

        // If links do not specify patch dirs, use package-wide patch dir
        let patch_links = {
            match patch_links[..] {
                [] => vec![PatchLink {
                    patch_dir: vendor_package.patch_dir.clone(),
                    from_prefix: PathBuf::from(""),
                    to_prefix: PathBuf::from(""),
                    exclude: vec![],
                }],
                _ => patch_links,
            }
        };

        // sort patch_links so more specific links have priority
        // 1. file links over directory links eg 'a/file -> c/file' before 'b/ -> c/'
        // 2. subdirs (deeper paths) first eg 'a/aa/ -> c/aa' before 'a/ab -> c/'
        let mut sorted_links: Vec<_> = patch_links.clone();
        sorted_links.sort_by(|a, b| {
            let a_is_file = a.to_prefix.is_file();
            let b_is_file = b.to_prefix.is_file();

            if a_is_file != b_is_file {
                return b_is_file.cmp(&a_is_file);
            }

            let a_depth = a.to_prefix.iter().count();
            let b_depth = b.to_prefix.iter().count();

            b_depth.cmp(&a_depth)
        });

        // Add all subdirs and files to the exclude list of above dirs
        // avoids duplicate handling of the same changes
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        for patch_link in sorted_links.iter_mut() {
            patch_link.exclude = seen_paths
                .iter()
                .filter(|path| path.starts_with(&patch_link.to_prefix)) // subdir?
                .cloned()
                .collect();

            seen_paths.insert(patch_link.to_prefix.clone());
        }
        let git = Git::new(tmp_path, &sess.config.git);

        match matches.subcommand() {
            Some(("diff", matches)) => {
                // Apply patches
                sorted_links
                    .clone()
                    .into_iter()
                    .try_for_each(|patch_link| {
                        apply_patches(&rt, git, vendor_package.name.clone(), patch_link).map(|_| ())
                    })?;

                // Stage applied patches to clean working tree
                rt.block_on(git.add_all())?;

                // Print diff for each link
                sorted_links.into_iter().try_for_each(|patch_link| {
                    let get_diff = diff(&rt, git, vendor_package, patch_link, dep_path.clone())
                        .map_err(|cause| Error::chain("Failed to get diff.", cause))?;
                    if !get_diff.is_empty() {
                        print!("{}", get_diff);
                        // If desired, return an error (e.g. for CI)
                        if matches.contains_id("err_on_diff") {
                            let err_msg : Option<&String> = matches.get_one("err_on_diff");
                            let err_msg = match err_msg {
                                Some(err_msg) => err_msg.to_string(),
                                _ => "Found differences, please patch (e.g. using bender vendor patch).".to_string()
                            };
                            return Err(Error::new(err_msg))
                        }
                    }
                    Ok(())
                })
            }

            Some(("init", matches)) => {
                sorted_links.into_iter().rev().try_for_each(|patch_link| {
                    stageln!("Copying", "{} files from upstream", vendor_package.name);
                    // Remove existing directories before importing them again
                    let target_path = patch_link
                        .clone()
                        .to_prefix
                        .prefix_paths(&vendor_package.target_dir)?;
                    if target_path.exists() {
                        if target_path.is_dir() {
                            std::fs::remove_dir_all(target_path.clone())
                        } else {
                            std::fs::remove_file(target_path.clone())
                        }
                        .map_err(|cause| {
                            Error::chain(format!("Failed to remove {:?}.", target_path), cause)
                        })?;
                    }

                    // init
                    init(
                        &rt,
                        git,
                        vendor_package,
                        patch_link,
                        dep_path.clone(),
                        matches,
                    )
                })
            }

            Some(("patch", matches)) => {
                // Apply patches
                let mut num_patches = 0;
                sorted_links
                    .clone()
                    .into_iter()
                    .try_for_each(|patch_link| {
                        apply_patches(&rt, git, vendor_package.name.clone(), patch_link)
                            .map(|num| num_patches += num)
                    })
                    .map_err(|cause| Error::chain("Failed to apply patch.", cause))?;

                // Commit applied patches to clean working tree
                if num_patches > 0 {
                    rt.block_on(git.add_all())?;
                    rt.block_on(git.commit(Some(&"pre-patch".to_string())))?;
                }

                // Generate patch
                sorted_links.into_iter().try_for_each( |patch_link| {
                    match patch_link.patch_dir.clone() {
                        Some(patch_dir) => {
                            if matches.get_flag("plain") {
                                let get_diff = diff(&rt,
                                                    git,
                                                    vendor_package,
                                                    patch_link,
                                                    dep_path.clone())
                                            .map_err(|cause| Error::chain("Failed to get diff.", cause))?;
                                gen_plain_patch(get_diff, patch_dir, false)
                            } else {
                                gen_format_patch(&rt, sess, git, patch_link, vendor_package.target_dir.clone(), matches.get_one("message"))
                            }
                        },
                        None => {
                            warnln!("No patch directory specified for package {}, mapping {} => {}. Skipping patch generation.", vendor_package.name.clone(), patch_link.from_prefix.to_str().unwrap(), patch_link.to_prefix.to_str().unwrap());
                            Ok(())
                        },
                    }
                })
            }
            _ => Ok(()),
        }?;
    }

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
    let link_to = patch_link
        .to_prefix
        .clone()
        .prefix_paths(&vendor_package.target_dir)?;
    let link_from = patch_link.from_prefix.clone().prefix_paths(dep_path)?;
    std::fs::create_dir_all(link_to.parent().unwrap()).map_err(|cause| {
        Error::chain(
            format!("Failed to create directory {:?}", link_to.parent()),
            cause,
        )
    })?;

    if !matches.get_flag("no_patch") {
        apply_patches(rt, git, vendor_package.name.clone(), patch_link.clone())?;
    }

    // Check if includes exist
    for path in vendor_package.include_from_upstream.clone() {
        if !PathBuf::from(extend_paths(&[path.clone()], dep_path, true)?[0].clone()).exists() {
            warnln!("{} not found in upstream, continuing.", path);
        }
    }

    // Copy src to dst recursively.
    match link_from.is_dir() {
        true => copy_recursively(
            &link_from,
            &link_to,
            &extend_paths(&vendor_package.include_from_upstream, dep_path, false)?,
            &vendor_package
                .exclude_from_upstream
                .clone()
                .into_iter()
                .map(|excl| format!("{}/{}", &dep_path.to_str().unwrap(), &excl))
                .collect(),
        )?,
        false => {
            if link_from.exists() {
                std::fs::copy(&link_from, &link_to).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Failed to copy {} to {}.",
                            link_from.to_str().unwrap(),
                            link_to.to_str().unwrap(),
                        ),
                        cause,
                    )
                })?;
            } else {
                warnln!(
                    "{} not found in upstream, continuing.",
                    link_from.to_str().unwrap()
                );
            }
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
        std::fs::create_dir_all(patch_dir.clone()).map_err(|cause| {
            Error::chain(
                format!("Failed to create directory {:?}", patch_dir.clone()),
                cause,
            )
        })?;

        let mut patches = std::fs::read_dir(patch_dir)?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().is_some())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| patch_path.to_str().unwrap().to_lowercase());

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
                        let is_file = patch_link
                            .from_prefix
                            .clone()
                            .prefix_paths(git.path)
                            .unwrap()
                            .is_file();

                        let current_patch_target = if is_file {
                            patch_link.from_prefix.parent().unwrap().to_str().unwrap()
                        } else {
                            patch_link.from_prefix.as_path().to_str().unwrap()
                        };

                        c.arg("apply")
                            .arg("--directory")
                            .arg(current_patch_target)
                            .arg("-p1")
                            .arg(&patch);

                        // limit to specific file for file links
                        if is_file {
                            let file_path = patch_link.from_prefix.to_str().unwrap();
                            c.arg("--include").arg(file_path);
                        }

                        c
                    })
                })
                .await
                .map_err(move |cause| {
                    Error::chain(format!("Failed to apply patch {:?}.", patch), cause)
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
    dep_path: impl AsRef<Path>,
) -> Result<String> {
    // Copy files from local to temporary repo
    let link_from = patch_link
        .from_prefix
        .clone()
        .prefix_paths(dep_path.as_ref())?; // dep_path: path to temporary clone. link_to: targetdir/link.to
    let link_to = patch_link
        .to_prefix
        .clone()
        .prefix_paths(vendor_package.target_dir.as_ref())?;
    if !&link_to.exists() {
        return Err(Error::new(format!(
            "Could not find {}. Did you run bender vendor init?",
            link_to.to_str().unwrap()
        )));
    }
    // Copy src to dst recursively.
    match &link_to.is_dir() {
        true => copy_recursively(
            &link_to,
            &link_from,
            &extend_paths(
                &vendor_package.include_from_upstream,
                &vendor_package.target_dir,
                false,
            )?,
            &vendor_package
                .exclude_from_upstream
                .clone()
                .into_iter()
                .map(|excl| format!("{}/{}", &vendor_package.target_dir.to_str().unwrap(), &excl))
                .collect(),
        )?,
        false => {
            std::fs::copy(&link_to, &link_from).map_err(|cause| {
                Error::chain(
                    format!(
                        "Failed to copy {} to {}.",
                        link_to.to_str().unwrap(),
                        link_from.to_str().unwrap(),
                    ),
                    cause,
                )
            })?;
        }
    };
    // Get diff
    rt.block_on(async {
        git.spawn_with(|c| {
            c.arg("diff").arg(format!(
                "--relative={}",
                patch_link
                    .from_prefix
                    .to_str()
                    .expect("Failed to convert from_prefix to string.")
            ))
        })
        .await
    })
}

/// Generate a plain patch from a diff
pub fn gen_plain_patch(diff: String, patch_dir: impl AsRef<Path>, no_patch: bool) -> Result<()> {
    if !diff.is_empty() {
        // if let Some(patch) = patch_dir {
        // Create directory in case it does not already exist
        std::fs::create_dir_all(patch_dir.as_ref()).map_err(|cause| {
            Error::chain(
                format!("Failed to create directory {:?}", patch_dir.as_ref()),
                cause,
            )
        })?;

        let mut patches = std::fs::read_dir(patch_dir.as_ref())?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| patch_path.to_str().unwrap().to_lowercase());

        let new_patch = if no_patch || patches.is_empty() {
            // Remove all old patches
            for patch_file in patches {
                std::fs::remove_file(patch_file)?;
            }
            "0001-bender-vendor.patch".to_string()
        } else {
            // Get all patch leading numeric keys (0001, ...) and generate new name
            let leading_numbers = patches
                .iter()
                .map(|file_path| file_path.file_name().unwrap().to_str().unwrap())
                .map(|s| &s[..4])
                .collect::<Vec<_>>();
            if !leading_numbers
                .iter()
                .all(|s| s.chars().all(char::is_numeric))
            {
                Err(Error::new(format!(
                    "Please ensure all patches start with four numbers for proper ordering in {}",
                    patch_dir.as_ref().to_str().unwrap()
                )))?;
            }
            let max_number = leading_numbers
                .iter()
                .map(|s| s.parse::<i32>().unwrap())
                .max()
                .unwrap();
            format!("{:04}-bender-vendor.patch", max_number + 1)
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
    message: Option<&String>,
) -> Result<()> {
    // Local git
    let to_path = patch_link
        .to_prefix
        .clone()
        .prefix_paths(target_dir.as_ref())?;
    if !&to_path.exists() {
        return Err(Error::new(format!(
            "Could not find {}. Did you run bender vendor init?",
            to_path.to_str().unwrap()
        )));
    }
    let git_parent = Git::new(
        if to_path.is_dir() {
            &to_path
        } else {
            to_path.parent().unwrap()
        },
        &sess.config.git,
    );

    // If the patch link maps a file, use the parent directory for the following git operations.
    let from_path_relative = if to_path.is_dir() {
        patch_link.from_prefix.clone()
    } else {
        patch_link.from_prefix.parent().unwrap().to_path_buf()
    };

    // We assume that patch_dir matches Some() was checked outside this function.
    let patch_dir = patch_link.patch_dir.clone().unwrap();

    // If the patch link maps a file, we operate in the file's parent directory
    // Therefore, only get the diff for that file.
    let include_pathspec = if !to_path.is_dir() {
        patch_link
            .to_prefix
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    } else {
        ".".to_string()
    };

    // Build the exclude pathspec to diff only the applicable files
    let exclude_pathspecs: Vec<String> = patch_link
        .exclude
        .iter()
        .map(|path| format!(":!{}", path.to_str().unwrap()))
        .collect();

    let mut diff_args = vec![
        "diff".to_string(),
        "--relative".to_string(),
        "--cached".to_string(),
    ];

    diff_args.push(include_pathspec);
    for exclude_path in exclude_pathspecs {
        diff_args.push(exclude_path);
    }

    // Get staged changes in dependency
    let get_diff_cached = rt
        .block_on(async { git_parent.spawn_with(|c| c.args(&diff_args)).await })
        .map_err(|cause| Error::chain("Failed to generate diff", cause))?;

    if !get_diff_cached.is_empty() {
        // Write diff into new temp dir. TODO: pipe directly to "git apply"
        let tmp_format_dir = TempDir::new()?;
        let tmp_format_path = tmp_format_dir.into_path();
        let diff_cached_path = tmp_format_path.join("staged.diff");
        std::fs::write(diff_cached_path.clone(), get_diff_cached)?;

        // Apply diff and stage changes in ghost repo
        rt.block_on(async {
            git.spawn_with(|c| {
                c.arg("apply")
                    .arg("--directory")
                    .arg(&from_path_relative)
                    .arg("-p1")
                    .arg(&diff_cached_path)
            })
            .and_then(|_| git.spawn_with(|c| c.arg("add").arg("--all")))
            .await
        }).map_err(|cause| Error::chain("Could not apply staged changes on top of patched upstream repository. Did you commit all previously patched modifications?", cause))?;

        // Commit all staged changes in ghost repo
        rt.block_on(git.commit(message))?;

        // Create directory in case it does not already exist
        std::fs::create_dir_all(patch_dir.clone()).map_err(|cause| {
            Error::chain(
                format!("Failed to create directory {:?}", patch_dir.clone()),
                cause,
            )
        })?;

        let mut patches = std::fs::read_dir(patch_dir.clone())?
            .map(move |f| f.unwrap().path())
            .filter(|f| f.extension().is_some())
            .filter(|f| f.extension().unwrap() == "patch")
            .collect::<Vec<_>>();
        patches.sort_by_key(|patch_path| patch_path.to_str().unwrap().to_lowercase());

        // Determine max number
        let max_number = if patches.is_empty() {
            0
        } else {
            let leading_numbers = patches
                .iter()
                .map(|file_path| file_path.file_name().unwrap().to_str().unwrap())
                .map(|s| &s[..4])
                .collect::<Vec<_>>();
            if !leading_numbers
                .iter()
                .all(|s| s.chars().all(char::is_numeric))
            {
                Err(Error::new(format!(
                    "Please ensure all patches start with four numbers for proper ordering in {}",
                    patch_dir.to_str().unwrap()
                )))?;
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
                    .arg(format!(
                        "--relative={}",
                        from_path_relative.to_str().unwrap()
                    ))
                    .arg("HEAD")
            })
            .await
        })?;
    }
    Ok(())
}

/// recursive copy function
pub fn copy_recursively(
    source: impl AsRef<Path> + std::fmt::Debug,
    destination: impl AsRef<Path> + std::fmt::Debug,
    includes: &Vec<String>,
    ignore: &Vec<String>,
) -> Result<()> {
    std::fs::create_dir_all(&destination).map_err(|cause| {
        Error::chain(
            format!("Failed to create directory {:?}", &destination),
            cause,
        )
    })?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;

        if !includes.iter().any(|include| {
            PathBuf::from(include).ancestors().any(|include_path| {
                Pattern::new(include_path.to_str().unwrap())
                    .unwrap()
                    .matches_path(&entry.path())
            })
        }) || ignore.iter().any(|ignore_path| {
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
        } else if filetype.is_symlink() {
            let orig = std::fs::read_link(entry.path());
            symlink_dir(orig.unwrap(), destination.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), destination.as_ref().join(entry.file_name())).map_err(
                |cause| {
                    Error::chain(
                        format!(
                            "Failed to copy {} to {}.",
                            entry.path().to_str().unwrap(),
                            destination
                                .as_ref()
                                .join(entry.file_name())
                                .to_str()
                                .unwrap()
                        ),
                        cause,
                    )
                },
            )?;
        }
    }
    Ok(())
}

/// Prefix paths with prefix. Append ** to directories.
pub fn extend_paths(
    include_from_upstream: &[String],
    prefix: impl AsRef<Path>,
    dir_only: bool,
) -> Result<Vec<String>> {
    include_from_upstream
        .iter()
        .map(|pattern| {
            let pattern_long = PathBuf::from(pattern).prefix_paths(prefix.as_ref())?;
            if pattern_long.is_dir() && !dir_only {
                Ok(String::from(pattern_long.join("**").to_str().unwrap()))
            } else {
                Ok(String::from(pattern_long.to_str().unwrap()))
            }
        })
        .collect::<Result<_>>()
}

#[cfg(unix)]
fn symlink_dir(p: PathBuf, q: PathBuf) -> Result<()> {
    Ok(std::os::unix::fs::symlink(p, q)?)
}

#[cfg(windows)]
fn symlink_dir(p: PathBuf, q: PathBuf) -> Result<()> {
    Ok(std::os::windows::fs::symlink_dir(p, q)?)
}
