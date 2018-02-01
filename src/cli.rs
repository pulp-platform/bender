// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::path::{Path, PathBuf};
use clap::{App, Arg, SubCommand};
use serde_yaml;
use tokio_core::reactor::Core;
use futures::future;
use config::{Config, PartialConfig, Manifest, Merge, Validate, Locked};
use error::*;
use sess::{Session, SessionArenas, SessionIo};
use resolver::DependencyResolver;

/// Inner main function which can return an error.
pub fn main() -> Result<()> {
    let app = App::new("landa")
        .version("0.1.0")
        .author("Fabian Schuiki <fschuiki@iis.ee.ethz.ch>")
        .about("A dependency management tool for hardware projects.\n\nAttendez la crème.")
        .arg(Arg::with_name("dir")
            .short("d")
            .long("dir")
            .takes_value(true)
            .help("Sets a custom root working directory")
        )
        .subcommand(SubCommand::with_name("package")
            .about("Get the path to a dependency")
            .arg(Arg::with_name("name")
                .multiple(true)
                .required(true)
                .help("Package names to get the path for")
            )
        );
    let matches = app.get_matches();

    // Determine the root working directory, which has either been provided via
    // the -d/--dir switch, or by searching upwards in the file system
    // hierarchy.
    let root_dir: PathBuf = match matches.value_of("dir") {
        Some(d) => d.into(),
        None => find_package_root(Path::new(".")).map_err(|cause| Error::chain(
            "Cannot find root directory of package.",
            cause,
        ))?,
    };
    debugln!("main: root dir {:?}", root_dir);

    // Parse the manifest file of the package.
    let manifest = read_manifest(&root_dir.join("Landa.yml"))?;
    debugln!("main: {:#?}", manifest);

    // Gather and parse the tool configuration.
    let config = load_config(&root_dir)?;

    // Assemble the session.
    let sess_arenas = SessionArenas::new();
    let sess = Session::new(&root_dir, &manifest, &config, &sess_arenas);
    debugln!("main: {:#?}", sess);

    // Resolve the dependencies.
    let lock_path = root_dir.join("Landa.lock");
    let locked = if lock_path.exists() {
        Some(read_lockfile(&lock_path)?)
    } else {
        None
    };
    debugln!("main: loaded {:#?}", locked);
    let res = DependencyResolver::new(&sess);
    let locked = res.resolve()?;
    debugln!("main: resolved {:#?}", locked);
    write_lockfile(&locked, &root_dir.join("Landa.lock"))?;
    sess.load_locked(&locked);

    // Dispatch the different subcommands.
    if let Some(matches) = matches.subcommand_matches("package") {
        let mut core = Core::new().unwrap();
        let io = SessionIo::new(&sess, core.handle());

        let ids = matches
            .values_of("name")
            .unwrap()
            .map(|n| Ok((n, sess.dependency_with_name(n)?)))
            .collect::<Result<Vec<_>>>()?;
        debugln!("main: obtain checkouts {:#?}", ids);
        let checkouts = core.run(future::join_all(ids
            .iter()
            .map(|&(_, id)| io.checkout(id))
            .collect::<Vec<_>>()
        ))?;
        debugln!("main: checkouts {:#?}", checkouts);
        for c in checkouts {
            if let Some(s) = c.to_str() {
                println!("{}", s);
            }
        }
    }

    Ok(())
}

/// Find the root directory of a package.
///
/// Traverses the directory hierarchy upwards until a `Landa.yml` file is found.
fn find_package_root(from: &Path) -> Result<PathBuf> {
    use std::fs::{canonicalize, metadata};
    use std::os::unix::fs::MetadataExt;

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from).map_err(|cause| Error::chain(
        format!("Failed to canonicalize path {:?}.", from),
        cause,
    ))?;
    debugln!("find_package_root: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    debugln!("find_package_root: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for _ in 0..100 {
        debugln!("find_package_root: looking in {:?}", path);

        // Check if we can find a package manifest here.
        if path.join("Landa.yml").exists() {
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
fn read_manifest(path: &Path) -> Result<Manifest> {
    use std::fs::File;
    use config::PartialManifest;
    debugln!("read_manifest: {:?}", path);
    let file = File::open(path).map_err(|cause| Error::chain(
        format!("Cannot open manifest {:?}.", path),
        cause
    ))?;
    let partial: PartialManifest = serde_yaml::from_reader(file).map_err(|cause| Error::chain(
        format!("Syntax error in manifest {:?}.", path),
        cause
    ))?;
    partial.validate().map_err(|cause| Error::chain(
        format!("Error in manifest {:?}.", path),
        cause
    ))
}

/// Load a configuration by traversing a directory hierarchy upwards.
fn load_config(from: &Path) -> Result<Config> {
    use std::fs::{canonicalize, metadata};
    use std::os::unix::fs::MetadataExt;
    let mut out = PartialConfig::new();

    // Load the optional local configuration.
    if let Some(cfg) = maybe_load_config(&from.join("Landa.local"))? {
        out = out.merge(cfg);
    }

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from).map_err(|cause| Error::chain(
        format!("Failed to canonicalize path {:?}.", from),
        cause,
    ))?;
    debugln!("load_config: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    debugln!("load_config: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for _ in 0..100 {
        debugln!("load_config: looking in {:?}", path);

        if let Some(cfg) = maybe_load_config(&path.join(".landa.yml"))? {
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
    if let Some(mut home) = std::env::home_dir() {
        home.push(".config");
        home.push("landa.yml");
        if let Some(cfg) = maybe_load_config(&home)? {
            out = out.merge(cfg);
        }
    }

    // Load the global configuration.
    if let Some(cfg) = maybe_load_config(Path::new("/etc/landa.yml"))? {
        out = out.merge(cfg);
    }

    // Assemble and merge the default configuration.
    let default_cfg = PartialConfig {
        database: {
            let mut db = std::env::home_dir().unwrap_or_else(|| from.into());
            db.push(".landa");
            Some(db)
        },
        git: Some("git".into()),
    };
    out = out.merge(default_cfg);

    // Validate the configuration.
    out.validate().map_err(|cause| Error::chain("Invalid configuration:", cause))
}

/// Load a configuration file if it exists.
fn maybe_load_config(path: &Path) -> Result<Option<PartialConfig>> {
    use std::fs::File;
    debugln!("maybe_load_config: {:?}", path);
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path).map_err(|cause| Error::chain(
        format!("Cannot open config {:?}.", path),
        cause
    ))?;
    let partial: PartialConfig = serde_yaml::from_reader(file).map_err(|cause| Error::chain(
        format!("Syntax error in config {:?}.", path),
        cause
    ))?;
    Ok(Some(partial))
}

/// Read a lock file.
fn read_lockfile(path: &Path) -> Result<Locked> {
    debugln!("read_lockfile: {:?}", path);
    use std::fs::File;
    let file = File::open(path).map_err(|cause| Error::chain(
        format!("Cannot open lockfile {:?}.", path),
        cause
    ))?;
    serde_yaml::from_reader(file).map_err(|cause| Error::chain(
        format!("Syntax error in lockfile {:?}.", path),
        cause
    ))
}

/// Write a lock file.
fn write_lockfile(locked: &Locked, path: &Path) -> Result<()> {
    debugln!("write_lockfile: {:?}", path);
    use std::fs::File;
    let file = File::create(path).map_err(|cause| Error::chain(
        format!("Cannot create lockfile {:?}.", path),
        cause
    ))?;
    serde_yaml::to_writer(file, locked).map_err(|cause| Error::chain(
        format!("Cannot write lockfile {:?}.", path),
        cause
    ))?;
    Ok(())
}
