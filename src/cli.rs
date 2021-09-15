// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::fs::{canonicalize, metadata};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{App, AppSettings, Arg, OsValues, SubCommand};
use serde_yaml;

use crate::cmd;
use crate::config::{Config, Locked, Manifest, Merge, PartialConfig, PrefixPaths, Validate};
use crate::error::*;
use crate::resolver::DependencyResolver;
use crate::sess::{Session, SessionArenas, SessionIo};
use tokio_core::reactor::Core;

/// Inner main function which can return an error.
pub fn main() -> Result<()> {
    let app = App::new(env!("CARGO_PKG_NAME"))
        .setting(AppSettings::AllowExternalSubcommands)
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("A dependency management tool for hardware projects.")
        .arg(
            Arg::with_name("dir")
                .short("d")
                .long("dir")
                .takes_value(true)
                .global(true)
                .help("Sets a custom root working directory"),
        )
        .arg(
            Arg::with_name("local")
                .short("l")
                .long("local")
                .global(true)
                .help("Disables fetching of remotes (e.g. for air-gapped computers)"),
        )
        .subcommand(
            SubCommand::with_name("update")
                .about("Update the dependencies")
                .arg(
                    Arg::with_name("fetch")
                        .short("f")
                        .long("fetch")
                        .help("forces fetch of git dependencies"),
                ),
        )
        .subcommand(cmd::path::new())
        .subcommand(cmd::parents::new())
        .subcommand(cmd::clone::new())
        .subcommand(cmd::packages::new())
        .subcommand(cmd::sources::new())
        .subcommand(cmd::config::new())
        .subcommand(cmd::script::new());

    // Add the `--debug` option in debug builds.
    let app = if cfg!(debug_assertions) {
        app.arg(
            Arg::with_name("debug")
                .long("debug")
                .global(true)
                .help("Print additional debug information"),
        )
    } else {
        app
    };

    // Parse the arguments.
    let matches = app.get_matches();

    // Enable debug outputs if needed.
    if matches.is_present("debug") {
        ENABLE_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let mut force_fetch = false;
    match matches.subcommand() {
        ("update", Some(intern_matches)) => {
            force_fetch = intern_matches.is_present("fetch");
            if matches.is_present("local") && intern_matches.is_present("fetch") {
                warnln!(
                    "As --local argument is set for bender command, no fetching will be performed."
                );
            }
        }
        _ => {}
    }

    // Determine the root working directory, which has either been provided via
    // the -d/--dir switch, or by searching upwards in the file system
    // hierarchy.
    let root_dir: PathBuf = match matches.value_of("dir") {
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
        matches.is_present("local"),
        force_fetch,
    );

    // Read the existing lockfile.
    let lock_path = root_dir.join("Bender.lock");
    let locked_existing = if lock_path.exists() {
        Some(read_lockfile(&lock_path)?)
    } else {
        None
    };

    // Resolve the dependencies if the lockfile does not exist or is outdated.
    let locked = if matches.subcommand().0 == "update" || locked_existing.is_none() {
        if manifest.frozen {
            return Err(Error::new(format!("Refusing to update dependencies because the package is frozen. Remove the `frozen: true` from {:?} to proceed; there be dragons.", manifest_path)));
        }
        debugln!("main: lockfile {:?} outdated", lock_path);
        let res = DependencyResolver::new(&sess);
        let locked_new = res.resolve()?;
        write_lockfile(&locked_new, &root_dir.join("Bender.lock"))?;
        locked_new
    } else {
        debugln!("main: lockfile {:?} up-to-date", lock_path);
        locked_existing.unwrap()
    };
    sess.load_locked(&locked);

    // Ensure the locally linked packages are up-to-date.
    {
        let mut core = Core::new().unwrap();
        let io = SessionIo::new(&sess, core.handle());
        for (path, pkg_name) in &sess.manifest.workspace.package_links {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, path);

            // Determine the checkout path for this package.
            let pkg_path = core.run(io.checkout(sess.dependency_with_name(pkg_name)?))?;
            let pkg_path = path
                .parent()
                .and_then(|path| pathdiff::diff_paths(pkg_path, path))
                .unwrap_or(pkg_path.into());

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
        ("path", Some(matches)) => cmd::path::run(&sess, matches),
        ("parents", Some(matches)) => cmd::parents::run(&sess, matches),
        ("clone", Some(matches)) => cmd::clone::run(&sess, &root_dir, matches),
        ("packages", Some(matches)) => cmd::packages::run(&sess, matches),
        ("sources", Some(matches)) => cmd::sources::run(&sess, matches),
        ("config", Some(matches)) => cmd::config::run(&sess, matches),
        ("script", Some(matches)) => cmd::script::run(&sess, matches),
        ("update", _) => Ok(()),
        (plugin, Some(matches)) => execute_plugin(&sess, plugin, matches.values_of_os("")),
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
                "Stopped at filesystem root {:?}.",
                path
            )));
        }

        // Abort if we have crossed the filesystem boundary.
        let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
        debugln!("find_package_root: rdev = {:?}", rdev);
        if rdev != limit_rdev {
            return Err(Error::new(format!(
                "Stopped at filesystem boundary {:?}.",
                tested_path
            )));
        }
    }

    Err(Error::new("Reached maximum number of search steps."))
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
    Ok(manifest.prefix_paths(path.parent().unwrap()))
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
        database: Some(from.join(".bender")),
        git: Some("git".into()),
        overrides: None,
        plugins: None,
    };
    out = out.merge(default_cfg);

    // Validate the configuration.
    out.validate()
        .map_err(|cause| Error::chain("Invalid configuration:", cause))
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
    Ok(Some(partial.prefix_paths(path.parent().unwrap())))
}

/// Read a lock file.
fn read_lockfile(path: &Path) -> Result<Locked> {
    debugln!("read_lockfile: {:?}", path);
    use std::fs::File;
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open lockfile {:?}.", path), cause))?;
    serde_yaml::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in lockfile {:?}.", path), cause))
}

/// Write a lock file.
fn write_lockfile(locked: &Locked, path: &Path) -> Result<()> {
    debugln!("write_lockfile: {:?}", path);
    use std::fs::File;
    let file = File::create(path)
        .map_err(|cause| Error::chain(format!("Cannot create lockfile {:?}.", path), cause))?;
    serde_yaml::to_writer(file, locked)
        .map_err(|cause| Error::chain(format!("Cannot write lockfile {:?}.", path), cause))?;
    Ok(())
}

/// Execute a plugin.
fn execute_plugin(sess: &Session, plugin: &str, matches: Option<OsValues>) -> Result<()> {
    debugln!("main: execute plugin `{}`", plugin);

    // Obtain a list of declared plugins.
    let mut core = Core::new().unwrap();
    let io = SessionIo::new(sess, core.handle());
    let plugins = core.run(io.plugins())?;

    // Lookup the requested plugin and complain if it does not exist.
    let plugin = match plugins.get(plugin) {
        Some(p) => p,
        None => return Err(Error::new(format!("Unknown command `{}`.", plugin))),
    };
    debugln!("main: found plugin {:#?}", plugin);

    // Assemble a command that executes the plugin with the appropriate
    // environment and forwards command line arguments.
    let mut cmd = Command::new(&plugin.path);
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
    cmd.current_dir(&sess.root);
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
