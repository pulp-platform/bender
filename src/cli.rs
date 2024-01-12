// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::ffi::OsString;
use std::fs::{canonicalize, metadata};
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

use clap::parser::ValuesRef;
use clap::{Arg, ArgAction, Command};
use serde_yaml;

use crate::cmd;
use crate::config::{
    Config, Locked, LockedPackage, LockedSource, Manifest, Merge, PartialConfig, PrefixPaths,
    Validate,
};
use crate::error::*;
use crate::resolver::DependencyResolver;
use crate::sess::{Session, SessionArenas, SessionIo};
use tokio::runtime::Runtime;

/// Inner main function which can return an error.
pub fn main() -> Result<()> {
    let app = Command::new(env!("CARGO_PKG_NAME"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .allow_external_subcommands(true)
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("A dependency management tool for hardware projects.")
        .arg(
            Arg::new("dir")
                .short('d')
                .long("dir")
                .num_args(1)
                .global(true)
                .help("Sets a custom root working directory"),
        )
        .arg(
            Arg::new("local")
                .long("local")
                .global(true)
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Disables fetching of remotes (e.g. for air-gapped computers)"),
        )
        .subcommand(
            Command::new("update")
                .about("Update the dependencies")
                .arg(
                    Arg::new("fetch")
                        .short('f')
                        .long("fetch")
                        .num_args(0)
                        .action(ArgAction::SetTrue)
                        .help("forces fetch of git dependencies"),
                )
                .arg(
                    Arg::new("no-checkout")
                        .long("no-checkout")
                        .num_args(0)
                        .action(ArgAction::SetTrue)
                        .help("Disables checkout of dependencies"),
                ),
        )
        .subcommand(cmd::path::new())
        .subcommand(cmd::parents::new())
        .subcommand(cmd::clone::new())
        .subcommand(cmd::packages::new())
        .subcommand(cmd::sources::new())
        .subcommand(cmd::config::new())
        .subcommand(cmd::script::new())
        .subcommand(cmd::checkout::new())
        .subcommand(cmd::vendor::new())
        .subcommand(cmd::fusesoc::new())
        .subcommand(cmd::init::new());

    // Add the `--debug` option in debug builds.
    let app = if cfg!(debug_assertions) {
        app.arg(
            Arg::new("debug")
                .long("debug")
                .global(true)
                .num_args(0)
                .action(ArgAction::SetTrue)
                .help("Print additional debug information"),
        )
    } else {
        app
    };

    // Parse the arguments.
    let matches = app.get_matches();

    // Enable debug outputs if needed.
    if matches.contains_id("debug") && matches.get_flag("debug") {
        ENABLE_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    if let Some(("init", matches)) = matches.subcommand() {
        return cmd::init::run(matches);
    }

    let mut force_fetch = false;
    if let Some(("update", intern_matches)) = matches.subcommand() {
        force_fetch = intern_matches.get_flag("fetch");
        if matches.get_flag("local") && intern_matches.get_flag("fetch") {
            warnln!(
                "As --local argument is set for bender command, no fetching will be performed."
            );
        }
    }

    // Determine the root working directory, which has either been provided via
    // the -d/--dir switch, or by searching upwards in the file system
    // hierarchy.
    let root_dir: PathBuf = match matches.get_one::<String>("dir") {
        Some(d) => canonicalize(d).map_err(|cause| {
            Error::chain(format!("Failed to canonicalize path {:?}.", d), cause)
        })?,
        None => find_package_root(Path::new("."))
            .map_err(|cause| Error::chain("Cannot find root directory of package.", cause))?,
    };
    debugln!("main: root dir {:?}", root_dir);

    // Parse the manifest file of the package.
    let manifest_path = root_dir.join("Bender.yml");
    let manifest = read_manifest(&manifest_path)?;
    debugln!("main: {:#?}", manifest);

    // Gather and parse the tool configuration.
    let config = load_config(&root_dir)?;
    debugln!("main: {:#?}", config);

    // Assemble the session.
    let sess_arenas = SessionArenas::new();
    let sess = Session::new(
        &root_dir,
        &manifest,
        &config,
        &sess_arenas,
        matches.get_flag("local"),
        force_fetch,
    );

    // Read the existing lockfile.
    let lock_path = root_dir.join("Bender.lock");
    let locked_existing = if lock_path.exists() {
        Some(read_lockfile(&lock_path, &root_dir)?)
    } else {
        None
    };

    // Resolve the dependencies if the lockfile does not exist or is outdated.
    let locked = match matches.subcommand() {
        Some((command, matches)) => {
            #[allow(clippy::unnecessary_unwrap)]
            // execute pre-dependency-fetch commands
            if command == "fusesoc" && matches.get_flag("single") {
                return cmd::fusesoc::run_single(&sess, matches);
            } else if command == "update" || locked_existing.is_none() {
                if manifest.frozen {
                    return Err(Error::new(format!(
                        "Refusing to update dependencies because the package is frozen.
                        Remove the `frozen: true` from {:?} to proceed; there be dragons.",
                        manifest_path
                    )));
                }
                debugln!("main: lockfile {:?} outdated", lock_path);
                let res = DependencyResolver::new(&sess);
                let locked_new = res.resolve()?;
                write_lockfile(&locked_new, &root_dir.join("Bender.lock"), &root_dir)?;
                locked_new
            } else {
                debugln!("main: lockfile {:?} up-to-date", lock_path);
                locked_existing.unwrap()
            }
        }
        None => {
            return Err(Error::new("Please specify a command.".to_string()));
        }
    };
    sess.load_locked(&locked)?;

    // Ensure the locally linked packages are up-to-date.
    {
        let io = SessionIo::new(&sess);
        for (path, pkg_name) in &sess.manifest.workspace.package_links {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, path);

            // Determine the checkout path for this package.
            let pkg_path = io.get_package_path(sess.dependency_with_name(pkg_name)?);

            // Checkout if we are running update or package path does not exist yet
            if matches.subcommand_name() == Some("update") || !pkg_path.clone().exists() {
                let rt = Runtime::new()?;
                rt.block_on(io.checkout(sess.dependency_with_name(pkg_name)?))?;
            }

            // Convert to relative path
            let pkg_path = path
                .parent()
                .and_then(|path| pathdiff::diff_paths(pkg_path.clone(), path))
                .unwrap_or(pkg_path);

            // Check if there is something at the destination path that needs to be
            // removed.
            if path.exists() {
                let meta = path.symlink_metadata().map_err(|cause| {
                    Error::chain(
                        format!("Failed to read metadata of path {:?}.", path),
                        cause,
                    )
                })?;
                if !meta.file_type().is_symlink() {
                    warnln!(
                        "Skipping link to package {} at {:?} since there is something there",
                        pkg_name,
                        path
                    );
                    continue;
                }
                if path.read_link().map(|d| d != pkg_path).unwrap_or(true) {
                    debugln!("main: removing existing link {:?}", path);
                    std::fs::remove_file(path).map_err(|cause| {
                        Error::chain(
                            format!("Failed to remove symlink at path {:?}.", path),
                            cause,
                        )
                    })?;
                }
            }

            // Create the symlink if there is nothing at the destination.
            if !path.exists() {
                stageln!("Linking", "{} ({:?})", pkg_name, path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|cause| {
                        Error::chain(format!("Failed to create directory {:?}.", parent), cause)
                    })?;
                }
                let previous_dir = match path.parent() {
                    Some(parent) => {
                        let d = std::env::current_dir().unwrap();
                        std::env::set_current_dir(parent).unwrap();
                        Some(d)
                    }
                    None => None,
                };
                std::os::unix::fs::symlink(&pkg_path, path).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Failed to create symlink to {:?} at path {:?}.",
                            pkg_path, path
                        ),
                        cause,
                    )
                })?;
                if let Some(d) = previous_dir {
                    std::env::set_current_dir(d).unwrap();
                }
            }
        }
    }

    // Dispatch the different subcommands.
    match matches.subcommand() {
        Some(("path", matches)) => cmd::path::run(&sess, matches),
        Some(("parents", matches)) => cmd::parents::run(&sess, matches),
        Some(("clone", matches)) => cmd::clone::run(&sess, &root_dir, matches),
        Some(("packages", matches)) => cmd::packages::run(&sess, matches),
        Some(("sources", matches)) => cmd::sources::run(&sess, matches),
        Some(("config", matches)) => cmd::config::run(&sess, matches),
        Some(("script", matches)) => cmd::script::run(&sess, matches),
        Some(("checkout", matches)) => cmd::checkout::run(&sess, matches),
        Some(("update", matches)) => {
            if matches.get_flag("no-checkout") {
                Ok(())
            } else {
                cmd::checkout::run(&sess, matches)
            }
        }
        Some(("vendor", matches)) => cmd::vendor::run(&sess, matches),
        Some(("fusesoc", matches)) => cmd::fusesoc::run(&sess, matches),
        Some((plugin, matches)) => execute_plugin(&sess, plugin, matches.get_many::<OsString>("")),
        _ => Ok(()),
    }
}

/// Find the root directory of a package.
///
/// Traverses the directory hierarchy upwards until a `Bender.yml` file is found.
fn find_package_root(from: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .map_err(|cause| Error::chain(format!("Failed to canonicalize path {:?}.", from), cause))?;
    debugln!("find_package_root: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    debugln!("find_package_root: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for _ in 0..100 {
        debugln!("find_package_root: looking in {:?}", path);

        // Check if we can find a package manifest here.
        if path.join("Bender.yml").exists() {
            return Ok(path);
        }

        // Abort if we have reached the filesystem root.
        let tested_path = path.clone();
        if !path.pop() {
            return Err(Error::new(format!(
                "No manifest (`Bender.yml` file) found. Stopped searching at filesystem root {:?}.",
                path
            )));
        }

        // Abort if we have crossed the filesystem boundary.
        let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
        debugln!("find_package_root: rdev = {:?}", rdev);
        if rdev != limit_rdev {
            return Err(Error::new(format!(
                "No manifest (`Bender.yml` file) found. Stopped searching at filesystem boundary {:?}.",
                tested_path
            )));
        }
    }

    Err(Error::new(
        "No manifest (`Bender.yml` file) found. Reached maximum number of search steps.",
    ))
}

/// Read a package manifest from a file.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    use crate::config::PartialManifest;
    use std::fs::File;
    debugln!("read_manifest: {:?}", path);
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open manifest {:?}.", path), cause))?;
    let partial: PartialManifest = serde_yaml::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in manifest {:?}.", path), cause))?;
    let manifest = partial
        .validate()
        .map_err(|cause| Error::chain(format!("Error in manifest {:?}.", path), cause))?;
    manifest.prefix_paths(path.parent().unwrap())
}

/// Load a configuration by traversing a directory hierarchy upwards.
fn load_config(from: &Path) -> Result<Config> {
    use std::os::unix::fs::MetadataExt;
    let mut out = PartialConfig::new();

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .map_err(|cause| Error::chain(format!("Failed to canonicalize path {:?}.", from), cause))?;
    debugln!("load_config: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    debugln!("load_config: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for _ in 0..100 {
        // Load the optional local configuration.
        if let Some(cfg) = maybe_load_config(&path.join("Bender.local"))? {
            out = out.merge(cfg);
        }

        debugln!("load_config: looking in {:?}", path);

        if let Some(cfg) = maybe_load_config(&path.join(".bender.yml"))? {
            out = out.merge(cfg);
        }

        // Abort if we have reached the filesystem root.
        if !path.pop() {
            break;
        }

        // Abort if we have crossed the filesystem boundary.
        let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
        debugln!("load_config: rdev = {:?}", rdev);
        if rdev != limit_rdev {
            break;
        }
    }

    // Load the user configuration.
    if let Some(mut home) = dirs::home_dir() {
        home.push(".config");
        home.push("bender.yml");
        if let Some(cfg) = maybe_load_config(&home)? {
            out = out.merge(cfg);
        }
    }

    // Load the global configuration.
    if let Some(cfg) = maybe_load_config(Path::new("/etc/bender.yml"))? {
        out = out.merge(cfg);
    }

    // Assemble and merge the default configuration.
    let default_cfg = PartialConfig {
        database: Some(from.join(".bender").to_str().unwrap().to_string()),
        git: Some("git".into()),
        overrides: None,
        plugins: None,
    };
    out = out.merge(default_cfg);

    // Validate the configuration.
    let mut out = out
        .validate()
        .map_err(|cause| Error::chain("Invalid configuration:", cause))?;

    out.overrides = out
        .overrides
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();

    Ok(out)
}

/// Load a configuration file if it exists.
fn maybe_load_config(path: &Path) -> Result<Option<PartialConfig>> {
    use std::fs::File;
    debugln!("maybe_load_config: {:?}", path);
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open config {:?}.", path), cause))?;
    let partial: PartialConfig = serde_yaml::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in config {:?}.", path), cause))?;
    Ok(Some(partial.prefix_paths(path.parent().unwrap())?))
}

/// Read a lock file.
fn read_lockfile(path: &Path, root_dir: &Path) -> Result<Locked> {
    debugln!("read_lockfile: {:?}", path);
    use std::fs::File;
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open lockfile {:?}.", path), cause))?;
    let locked_loaded: Result<Locked> = serde_yaml::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in lockfile {:?}.", path), cause));
    // Make relative paths absolute
    Ok(Locked {
        packages: locked_loaded?
            .packages
            .iter()
            .map(|pack| {
                Ok(if let LockedSource::Path(path) = &pack.1.source {
                    (
                        pack.0.clone(),
                        LockedPackage {
                            revision: pack.1.revision.clone(),
                            version: pack.1.version.clone(),
                            source: LockedSource::Path(if path.is_relative() {
                                path.clone().prefix_paths(root_dir)?
                            } else {
                                path.clone()
                            }),
                            dependencies: pack.1.dependencies.clone(),
                        },
                    )
                } else {
                    (pack.0.clone(), pack.1.clone())
                })
            })
            .collect::<Result<_>>()?,
    })
}

/// Write a lock file.
fn write_lockfile(locked: &Locked, path: &Path, root_dir: &Path) -> Result<()> {
    debugln!("write_lockfile: {:?}", path);
    // Adapt paths within main repo to be relative
    let adapted_locked = Locked {
        packages: locked
            .packages
            .iter()
            .map(|pack| {
                if let LockedSource::Path(path) = &pack.1.source {
                    (
                        pack.0.clone(),
                        LockedPackage {
                            revision: pack.1.revision.clone(),
                            version: pack.1.version.clone(),
                            source: LockedSource::Path(
                                path.strip_prefix(root_dir).unwrap_or(path).to_path_buf(),
                            ),
                            dependencies: pack.1.dependencies.clone(),
                        },
                    )
                } else {
                    (pack.0.clone(), pack.1.clone())
                }
            })
            .collect(),
    };

    use std::fs::File;
    let file = File::create(path)
        .map_err(|cause| Error::chain(format!("Cannot create lockfile {:?}.", path), cause))?;
    serde_yaml::to_writer(file, &adapted_locked)
        .map_err(|cause| Error::chain(format!("Cannot write lockfile {:?}.", path), cause))?;
    Ok(())
}

/// Execute a plugin.
fn execute_plugin(
    sess: &Session,
    plugin: &str,
    matches: Option<ValuesRef<OsString>>,
) -> Result<()> {
    debugln!("main: execute plugin `{}`", plugin);

    // Obtain a list of declared plugins.
    let runtime = Runtime::new()?;
    let io = SessionIo::new(sess);
    let plugins = runtime.block_on(io.plugins())?;

    // Lookup the requested plugin and complain if it does not exist.
    let plugin = match plugins.get(plugin) {
        Some(p) => p,
        None => return Err(Error::new(format!("Unknown command `{}`.", plugin))),
    };
    debugln!("main: found plugin {:#?}", plugin);

    // Assemble a command that executes the plugin with the appropriate
    // environment and forwards command line arguments.
    let mut cmd = SysCommand::new(&plugin.path);
    cmd.env(
        "BENDER",
        std::env::current_exe()
            .map_err(|cause| Error::chain("Failed to determine current executable.", cause))?,
    );
    cmd.env(
        "BENDER_CALL_DIR",
        std::env::current_dir()
            .map_err(|cause| Error::chain("Failed to determine current directory.", cause))?,
    );
    cmd.env("BENDER_MANIFEST_DIR", sess.root);
    cmd.current_dir(sess.root);
    if let Some(args) = matches {
        cmd.args(args);
    }
    debugln!("main: executing plugin {:#?}", cmd);
    let stat = cmd.status().map_err(|cause| {
        Error::chain(
            format!(
                "Unable to spawn process for plugin `{}`. Command was {:#?}.",
                plugin.name, cmd
            ),
            cause,
        )
    })?;

    // Don't bother to do anything after the plugin was run.
    std::process::exit(stat.code().unwrap_or(1));
}
