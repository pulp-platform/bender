// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

#[cfg(unix)]
use std::fs::{canonicalize, metadata};

#[cfg(windows)]
use dunce::canonicalize;

use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{ArgAction, CommandFactory, Parser, Subcommand, value_parser};
use indexmap::IndexSet;
use miette::{Context as _, IntoDiagnostic as _};
use serde_yaml_ng;
use tokio::runtime::Runtime;

use crate::Result;
use crate::cmd;
use crate::cmd::fusesoc::FusesocArgs;
use crate::config::{
    Config, Manifest, Merge, PartialConfig, PrefixPaths, Validate, ValidationContext,
};
use crate::diagnostic::{Diagnostics, ENABLE_DEBUG, Warnings};
use crate::lockfile::*;
use crate::sess::{Session, SessionArenas, SessionIo};
use crate::{bail, err};
use crate::{debugln, fmt_path, fmt_pkg, stageln};

#[derive(Parser, Debug)]
#[command(name = "bender")]
#[command(author, version, about, long_about = None)]
#[command(after_help = "Type 'bender <SUBCOMMAND> --help' for more information...")]
#[command(styles = cli_styles())]
struct Cli {
    /// Sets a custom root working directory
    #[arg(short, long, global = true, help_heading = "Global Options", env = "BENDER_DIR", value_parser = value_parser!(String))]
    dir: Option<String>,

    /// Disables fetching of remotes (e.g. for air-gapped computers)
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "BENDER_LOCAL"
    )]
    local: bool,

    /// Sets the maximum number of concurrent git operations [default: 4]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "BENDER_GIT_THROTTLE"
    )]
    git_throttle: Option<usize>,

    /// Suppresses specific warnings. Use `all` to suppress all warnings.
    #[arg(long, global = true, action = ArgAction::Append, help_heading = "Global Options", env = "BENDER_SUPPRESS_WARNINGS")]
    suppress: Vec<String>,

    /// Disable progress bars
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "BENDER_NO_PROGRESS"
    )]
    no_progress: bool,

    /// Print additional debug information
    #[cfg(debug_assertions)]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "BENDER_DEBUG"
    )]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Update(cmd::update::UpdateArgs),
    Path(cmd::path::PathArgs),
    Parents(cmd::parents::ParentsArgs),
    Clone(cmd::clone::CloneArgs),
    Clean(cmd::clean::CleanArgs),
    Packages(cmd::packages::PackagesArgs),
    Sources(cmd::sources::SourcesArgs),
    Completion(cmd::completion::CompletionArgs),
    /// Emit the configuration
    Config,
    Script(cmd::script::ScriptArgs),
    Checkout(cmd::checkout::CheckoutArgs),
    Vendor(cmd::vendor::VendorArgs),
    Fusesoc(cmd::fusesoc::FusesocArgs),
    /// Initialize a Bender package
    Init,
    Snapshot(cmd::snapshot::SnapshotArgs),
    Audit(cmd::audit::AuditArgs),
    #[command(external_subcommand)]
    Plugin(Vec<String>),
}

// Define a custom style for the CLI
fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Cyan.on_default())
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .invalid(AnsiColor::Yellow.on_default() | Effects::BOLD)
}

/// Inner main function which can return an error.
pub fn main() -> Result<()> {
    // Parse command line arguments.
    let cli = Cli::parse();

    let mut suppressed_warnings: HashSet<String> =
        cli.suppress.into_iter().map(|s| s.to_owned()).collect();

    // split suppress strings on commas and spaces
    suppressed_warnings = suppressed_warnings
        .into_iter()
        .flat_map(|s| {
            s.split(&[',', ' '][..])
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        })
        .collect();

    // Initialize warning and error handling with the suppression arguments.
    Diagnostics::init(suppressed_warnings);

    #[cfg(debug_assertions)]
    if cli.debug {
        ENABLE_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // Handle commands that do not require a session.
    match &cli.command {
        Commands::Completion(args) => {
            let mut cmd = Cli::command();
            return cmd::completion::run(args, &mut cmd);
        }
        Commands::Init => {
            return cmd::init::run();
        }
        _ => {}
    }

    let force_fetch = match cli.command {
        Commands::Update(ref args) => cmd::update::setup(args, cli.local)?,
        _ => false,
    };

    // Determine the root working directory, which has either been provided via
    // the -d/--dir switch, or by searching upwards in the file system
    // hierarchy.
    let root_dir: PathBuf = match &cli.dir {
        Some(d) => canonicalize(d)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to canonicalize path {:?}.", d))?,
        None => {
            find_package_root(Path::new(".")).wrap_err("Cannot find root directory of package.")?
        }
    };
    debugln!("main: root dir {:?}", root_dir);

    // Parse the manifest file of the package.
    let manifest_path = root_dir.join("Bender.yml");
    let manifest = read_manifest(&manifest_path)?;
    debugln!("main: {:#?}", manifest);

    // Gather and parse the tool configuration.
    let config = load_config(&root_dir, matches!(cli.command, Commands::Update(_)))?;
    debugln!("main: {:#?}", config);

    // Determine git throttle. The precedence is: CLI argument, env variable, config file, default (4).
    let git_throttle = cli.git_throttle.or(config.git_throttle).unwrap_or(4);

    // Assemble the session.
    let sess_arenas = SessionArenas::new();
    let sess = Session::new(
        &root_dir,
        &manifest,
        &config,
        &sess_arenas,
        cli.local,
        force_fetch,
        git_throttle,
        cli.no_progress,
    );

    if let Commands::Clean(args) = cli.command {
        return cmd::clean::run(&sess, args.all, &root_dir);
    }

    // Read the existing lockfile.
    let lock_path = root_dir.join("Bender.lock");
    let locked_existing = if lock_path.exists() {
        Some(read_lockfile(&lock_path, &root_dir)?)
    } else {
        None
    };

    // Resolve the dependencies if the lockfile does not exist or is outdated.
    let (locked_list, update_list) = match &cli.command {
        Commands::Fusesoc(args @ FusesocArgs { single: true, .. }) => {
            return cmd::fusesoc::run_single(&sess, args);
        }
        Commands::Update(args) => cmd::update::run(args, &sess, locked_existing.as_ref())?,
        _ if locked_existing.is_none() => {
            cmd::update::run_plain(false, &sess, locked_existing.as_ref(), IndexSet::new())?
        }
        _ => {
            debugln!("main: lockfile {:?} up-to-date", lock_path);
            (locked_existing.unwrap(), Vec::new())
        }
    };

    sess.load_locked(&locked_list)?;

    // Ensure the locally linked packages are up-to-date.
    {
        let io = SessionIo::new(&sess);
        for (path, pkg_name) in &sess.manifest.workspace.package_links {
            debugln!("main: maintaining link to {} at {:?}", pkg_name, path);

            // Determine the checkout path for this package.
            let pkg_path = io.get_package_path(sess.dependency_with_name(pkg_name)?);

            // Checkout if we are running update or package path does not exist yet
            if matches!(cli.command, Commands::Update(_)) || !pkg_path.clone().exists() {
                let rt = Runtime::new().into_diagnostic()?;
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
                let meta = path
                    .symlink_metadata()
                    .into_diagnostic()
                    .wrap_err_with(|| format!("Failed to read metadata of path {:?}.", path))?;
                if !meta.file_type().is_symlink() {
                    Warnings::SkippingPackageLink(pkg_name.clone(), path.clone()).emit();
                    continue;
                }
                if path.read_link().map(|d| d != pkg_path).unwrap_or(true) {
                    debugln!("main: removing existing link {:?}", path);
                    remove_symlink_dir(path).wrap_err_with(|| {
                        format!("Failed to remove symlink at path {:?}.", path)
                    })?;
                }
            }

            // Create the symlink if there is nothing at the destination.
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .into_diagnostic()
                        .wrap_err_with(|| format!("Failed to create directory {:?}.", parent))?;
                }
                let previous_dir = match path.parent() {
                    Some(parent) => {
                        let d = std::env::current_dir().unwrap();
                        std::env::set_current_dir(parent).unwrap();
                        Some(d)
                    }
                    None => None,
                };
                symlink_dir(&pkg_path, path).wrap_err_with(|| {
                    format!(
                        "Failed to create symlink to {:?} at path {:?}.",
                        pkg_path, path
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
        }
    }

    // Dispatch the different subcommands.

    match cli.command {
        Commands::Path(args) => cmd::path::run(&sess, &args),
        Commands::Parents(args) => cmd::parents::run(&sess, &args),
        Commands::Clone(args) => cmd::clone::run(&sess, &root_dir, &args),
        Commands::Packages(args) => cmd::packages::run(&sess, &args),
        Commands::Sources(args) => cmd::sources::run(&sess, &args),
        Commands::Config => cmd::config::run(&sess),
        Commands::Script(args) => cmd::script::run(&sess, &args),
        Commands::Checkout(args) => cmd::checkout::run(&sess, &args),
        Commands::Update(args) => cmd::update::run_final(&sess, &args, &update_list),
        Commands::Vendor(args) => cmd::vendor::run(&sess, &args),
        Commands::Fusesoc(args) => cmd::fusesoc::run(&sess, &args),
        Commands::Snapshot(args) => cmd::snapshot::run(&sess, &args),
        Commands::Audit(args) => cmd::audit::run(&sess, &args),
        Commands::Plugin(args) => {
            let (plugin_name, plugin_args) = args
                .split_first()
                .ok_or_else(|| err!("No command specified."))?;
            execute_plugin(&sess, plugin_name, plugin_args)
        }
        Commands::Completion(_) | Commands::Init | Commands::Clean(_) => {
            unreachable!()
        }
    }
}

#[cfg(unix)]
pub fn symlink_dir(p: &Path, q: &Path) -> Result<()> {
    std::os::unix::fs::symlink(p, q).into_diagnostic()
}

#[cfg(windows)]
pub fn symlink_dir(p: &Path, q: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(p, q).into_diagnostic()
}

#[cfg(unix)]
pub fn remove_symlink_dir(path: &Path) -> Result<()> {
    std::fs::remove_file(path).into_diagnostic()
}

#[cfg(windows)]
pub fn remove_symlink_dir(path: &Path) -> Result<()> {
    std::fs::remove_dir(path).into_diagnostic()
}

/// Find the root directory of a package.
///
/// Traverses the directory hierarchy upwards until a `Bender.yml` file is found.
fn find_package_root(from: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path {:?}.", from))?;
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
        if !path.pop() {
            bail!(
                "No manifest (`Bender.yml` file) found. Stopped searching at filesystem root {:?}.",
                path
            );
        }

        // Abort if we have crossed the filesystem boundary.
        #[cfg(unix)]
        {
            let rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
            debugln!("find_package_root: rdev = {:?}", rdev);
            if rdev != limit_rdev {
                bail!(
                    "No manifest (`Bender.yml` file) found. Stopped searching at filesystem boundary {:?}.",
                    path
                );
            }
        }
    }

    Err(err!(
        "No manifest (`Bender.yml` file) found. Reached maximum number of search steps.",
    ))
}

/// Read a package manifest from a file.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    use crate::config::PartialManifest;
    use std::fs::File;
    debugln!("read_manifest: {:?}", path);
    let file = File::open(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Cannot open manifest {:?}.", path))?;
    let partial: PartialManifest = serde_yaml_ng::from_reader(file)
        .into_diagnostic()
        .wrap_err_with(|| format!("Syntax error in manifest {:?}.", path))?;
    partial
        .prefix_paths(path.parent().unwrap())
        .wrap_err_with(|| format!("Error in manifest prefixing {:?}.", path))?
        .validate(&ValidationContext::default())
        .wrap_err_with(|| format!("Error in manifest {:?}.", path))
}

/// Load a configuration by traversing a directory hierarchy upwards.
fn load_config(from: &Path, warn_config_loaded: bool) -> Result<Config> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let mut out = PartialConfig::new();

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path {:?}.", from))?;
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
        git_lfs: None,
    };
    out = out.merge(default_cfg);

    // Validate the configuration.
    let mut out = out
        .validate(&ValidationContext::default())
        .wrap_err("Invalid configuration:")?;

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
        .into_diagnostic()
        .wrap_err_with(|| format!("Cannot open config {:?}.", path))?;
    let partial: PartialConfig = serde_yaml_ng::from_reader(file)
        .into_diagnostic()
        .wrap_err_with(|| format!("Syntax error in config {:?}.", path))?;
    if warn_config_loaded {
        Warnings::UsingConfigForOverride {
            path: path.to_path_buf(),
        }
        .emit();
    }
    Ok(Some(partial.prefix_paths(path.parent().unwrap())?))
}

/// Execute a plugin.
fn execute_plugin(sess: &Session, plugin: &str, args: &[String]) -> Result<()> {
    debugln!("main: execute plugin `{}`", plugin);

    // Obtain a list of declared plugins.
    let runtime = Runtime::new().into_diagnostic()?;
    let io = SessionIo::new(sess);
    let plugins = runtime.block_on(io.plugins(false))?;

    // Lookup the requested plugin and complain if it does not exist.
    let plugin = match plugins.get(plugin) {
        Some(p) => p,
        None => bail!("Unknown command `{}`.", plugin),
    };
    debugln!("main: found plugin {:#?}", plugin);

    // Assemble a command that executes the plugin with the appropriate
    // environment and forwards command line arguments.
    let mut cmd = SysCommand::new(&plugin.path);
    cmd.env(
        "BENDER",
        std::env::current_exe()
            .into_diagnostic()
            .wrap_err("Failed to determine current executable.")?,
    );
    cmd.env(
        "BENDER_CALL_DIR",
        std::env::current_dir()
            .into_diagnostic()
            .wrap_err("Failed to determine current directory.")?,
    );
    cmd.env("BENDER_MANIFEST_DIR", sess.root);
    cmd.current_dir(sess.root);
    cmd.args(args);

    debugln!("main: executing plugin {:#?}", cmd);
    let stat = cmd.status().into_diagnostic().wrap_err_with(|| {
        format!(
            "Unable to spawn process for plugin `{}`. Command was {:#?}.",
            plugin.name, cmd
        )
    })?;

    // Don't bother to do anything after the plugin was run.
    std::process::exit(stat.code().unwrap_or(1));
}
