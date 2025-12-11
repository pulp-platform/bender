// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

#[cfg(unix)]
use std::fs::{canonicalize, metadata};

#[cfg(windows)]
use dunce::canonicalize;

use clap::parser::{ValueSource, ValuesRef};
use clap::{value_parser, Arg, ArgAction, Command};
use indexmap::IndexSet;
use serde_yaml_ng;
use tokio::runtime::Runtime;

use crate::cmd;
use crate::config::{Config, Manifest, Merge, PartialConfig, PrefixPaths, Validate};
use crate::error::*;
use crate::lockfile::*;
use crate::sess::{Session, SessionArenas, SessionIo};

/// Inner main function which can return an error.
pub fn main() -> Result<()> {
    let app = Command::new(env!("CARGO_PKG_NAME"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .allow_external_subcommands(true)
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("A dependency management tool for hardware projects.")
        .after_help(
            "Type 'bender <SUBCOMMAND> --help' for more information about a bender subcommand.",
        )
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
        .arg(
            Arg::new("git-throttle")
                .long("git-throttle")
                .global(true)
                .num_args(1)
                .default_value("4")
                .value_parser(value_parser!(usize))
                .help("Sets the maximum number of concurrent git operations"),
        )
        .arg(
            Arg::new("suppress")
                .long("suppress")
                .global(true)
                .num_args(1)
                .action(ArgAction::Append)
                .help("Suppresses specific warnings. Use `all` to suppress all warnings.")
                .value_parser(value_parser!(String)),
        )
        .subcommand(cmd::update::new())
        .subcommand(cmd::path::new())
        .subcommand(cmd::parents::new())
        .subcommand(cmd::clone::new())
        .subcommand(cmd::clean::new())
        .subcommand(cmd::packages::new())
        .subcommand(cmd::sources::new())
        .subcommand(cmd::completion::new())
        .subcommand(cmd::config::new())
        .subcommand(cmd::script::new())
        .subcommand(cmd::checkout::new())
        .subcommand(cmd::vendor::new())
        .subcommand(cmd::fusesoc::new())
        .subcommand(cmd::init::new())
        .subcommand(cmd::snapshot::new())
        .subcommand(cmd::audit::new());

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
    let matches = app.clone().get_matches();

    let mut suppressed_warnings: IndexSet<String> = matches
        .get_many::<String>("suppress")
        .unwrap_or_default()
        .map(|s| s.to_owned())
        .collect();

    if suppressed_warnings.contains("all") || suppressed_warnings.contains("Wall") {
        suppressed_warnings.extend((1..24).map(|i| format!("W{:02}", i)));
    }

    // Enable debug outputs if needed.
    if matches.contains_id("debug") && matches.get_flag("debug") {
        ENABLE_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    if let Some(("init", matches)) = matches.subcommand() {
        return cmd::init::run(matches);
    }

    if let Some(("completion", matches)) = matches.subcommand() {
        let mut app = app;
        return cmd::completion::run(matches, &mut app);
    }

    let mut force_fetch = false;
    if let Some(("update", intern_matches)) = matches.subcommand() {
        force_fetch = cmd::update::setup(intern_matches, &suppressed_warnings)?;
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
    let manifest = read_manifest(&manifest_path, &suppressed_warnings)?;
    debugln!("main: {:#?}", manifest);

    // Gather and parse the tool configuration.
    let config = load_config(
        &root_dir,
        matches!(matches.subcommand(), Some(("update", _))) && !suppressed_warnings.contains("W02"),
        &suppressed_warnings,
    )?;
    debugln!("main: {:#?}", config);

    let git_throttle = if matches.value_source("git-throttle") == Some(ValueSource::CommandLine) {
        *matches.get_one::<usize>("git-throttle").unwrap()
    } else {
        config
            .git_throttle
            .unwrap_or(*matches.get_one::<usize>("git-throttle").unwrap())
    };

    // Assemble the session.
    let sess_arenas = SessionArenas::new();
    let sess = Session::new(
        &root_dir,
        &manifest,
        &config,
        &sess_arenas,
        matches.get_flag("local"),
        force_fetch,
        git_throttle,
        suppressed_warnings,
    );

    if let Some(("clean", intern_matches)) = matches.subcommand() {
        return cmd::clean::run(&sess, intern_matches, &root_dir);
    }

    // Read the existing lockfile.
    let lock_path = root_dir.join("Bender.lock");
    let locked_existing = if lock_path.exists() {
        Some(read_lockfile(&lock_path, &root_dir)?)
    } else {
        None
    };

    // Resolve the dependencies if the lockfile does not exist or is outdated.
    let (locked, update_list) = match matches.subcommand() {
        Some((command, matches)) => {
            #[allow(clippy::unnecessary_unwrap)]
            // execute pre-dependency-fetch commands
            if command == "fusesoc" && matches.get_flag("single") {
                return cmd::fusesoc::run_single(&sess, matches);
            } else if command == "update" {
                cmd::update::run(matches, &sess, locked_existing.as_ref())?
            } else if locked_existing.is_none() {
                cmd::update::run_plain(false, &sess, locked_existing.as_ref(), IndexSet::new())?
            } else {
                debugln!("main: lockfile {:?} up-to-date", lock_path);
                (locked_existing.unwrap(), Vec::new())
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
                rt.block_on(io.checkout(sess.dependency_with_name(pkg_name)?, false, &[]))?;
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
                    if !sess.suppress_warnings.contains("W01") {
                        warnln!(
                            "[W01] Skipping link to package {} at {:?} since there is something there",
                            pkg_name,
                            path
                        );
                    }
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
                symlink_dir(&pkg_path, path).map_err(|cause| {
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
        Some(("update", matches)) => cmd::update::run_final(&sess, matches, &update_list),
        Some(("vendor", matches)) => cmd::vendor::run(&sess, matches),
        Some(("fusesoc", matches)) => cmd::fusesoc::run(&sess, matches),
        Some(("snapshot", matches)) => cmd::snapshot::run(&sess, matches),
        Some(("audit", matches)) => cmd::audit::run(&sess, matches),
        Some((plugin, matches)) => execute_plugin(&sess, plugin, matches.get_many::<OsString>("")),
        _ => Ok(()),
    }
}

#[cfg(target_family = "unix")]
fn symlink_dir(p: &Path, q: &Path) -> Result<()> {
    Ok(std::os::unix::fs::symlink(p, q)?)
}

#[cfg(target_os = "windows")]
fn symlink_dir(p: &Path, q: &Path) -> Result<()> {
    Ok(std::os::windows::fs::symlink_dir(p, q)?)
}

/// Find the root directory of a package.
///
/// Traverses the directory hierarchy upwards until a `Bender.yml` file is found.
fn find_package_root(from: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .map_err(|cause| Error::chain(format!("Failed to canonicalize path {:?}.", from), cause))?;
    debugln!("find_package_root: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    #[cfg(unix)]
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    #[cfg(unix)]
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
        #[cfg(unix)]
        {
            let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
            debugln!("find_package_root: rdev = {:?}", rdev);
            if rdev != limit_rdev {
                return Err(Error::new(format!(
                    "No manifest (`Bender.yml` file) found. Stopped searching at filesystem boundary {:?}.",
                    tested_path
                )));
            }
        }
    }

    Err(Error::new(
        "No manifest (`Bender.yml` file) found. Reached maximum number of search steps.",
    ))
}

/// Read a package manifest from a file.
pub fn read_manifest(path: &Path, suppress_warnings: &IndexSet<String>) -> Result<Manifest> {
    use crate::config::PartialManifest;
    use std::fs::File;
    debugln!("read_manifest: {:?}", path);
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open manifest {:?}.", path), cause))?;
    let partial: PartialManifest = serde_yaml_ng::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in manifest {:?}.", path), cause))?;
    partial
        .prefix_paths(path.parent().unwrap())
        .map_err(|cause| Error::chain(format!("Error in manifest prefixing {:?}.", path), cause))?
        .validate("", false, suppress_warnings)
        .map_err(|cause| Error::chain(format!("Error in manifest {:?}.", path), cause))
}

/// Load a configuration by traversing a directory hierarchy upwards.
fn load_config(
    from: &Path,
    warn_config_loaded: bool,
    suppress_warnings: &IndexSet<String>,
) -> Result<Config> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let mut out = PartialConfig::new();

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .map_err(|cause| Error::chain(format!("Failed to canonicalize path {:?}.", from), cause))?;
    debugln!("load_config: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    #[cfg(unix)]
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    #[cfg(unix)]
    debugln!("load_config: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for _ in 0..100 {
        // Load the optional local configuration.
        if let Some(cfg) = maybe_load_config(&path.join("Bender.local"), warn_config_loaded)? {
            out = out.merge(cfg);
        }

        debugln!("load_config: looking in {:?}", path);

        if let Some(cfg) = maybe_load_config(&path.join(".bender.yml"), warn_config_loaded)? {
            out = out.merge(cfg);
        }

        // Abort if we have reached the filesystem root.
        if !path.pop() {
            break;
        }

        // Abort if we have crossed the filesystem boundary.
        #[cfg(unix)]
        {
            let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
            debugln!("load_config: rdev = {:?}", rdev);
            if rdev != limit_rdev {
                break;
            }
        }
    }

    // Load the user configuration.
    if let Some(mut home) = dirs::home_dir() {
        home.push(".config");
        home.push("bender.yml");
        if let Some(cfg) = maybe_load_config(&home, warn_config_loaded)? {
            out = out.merge(cfg);
        }
    }

    // Load the global configuration.
    if let Some(cfg) = maybe_load_config(Path::new("/etc/bender.yml"), warn_config_loaded)? {
        out = out.merge(cfg);
    }

    // Assemble and merge the default configuration.
    let default_cfg = PartialConfig {
        database: Some(from.join(".bender").to_str().unwrap().to_string()),
        git: Some("git".into()),
        overrides: None,
        plugins: None,
        git_throttle: None,
    };
    out = out.merge(default_cfg);

    // Validate the configuration.
    let mut out = out
        .validate("", false, suppress_warnings)
        .map_err(|cause| Error::chain("Invalid configuration:", cause))?;

    out.overrides = out
        .overrides
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();

    Ok(out)
}

/// Load a configuration file if it exists.
fn maybe_load_config(path: &Path, warn_config_loaded: bool) -> Result<Option<PartialConfig>> {
    use std::fs::File;
    debugln!("maybe_load_config: {:?}", path);
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)
        .map_err(|cause| Error::chain(format!("Cannot open config {:?}.", path), cause))?;
    let partial: PartialConfig = serde_yaml_ng::from_reader(file)
        .map_err(|cause| Error::chain(format!("Syntax error in config {:?}.", path), cause))?;
    if warn_config_loaded {
        warnln!("[W02] Using config at {:?} for overrides.", path)
    };
    Ok(Some(partial.prefix_paths(path.parent().unwrap())?))
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
    let plugins = runtime.block_on(io.plugins(false))?;

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
