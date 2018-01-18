// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std::path::{Path, PathBuf};
use clap::{App, Arg};
use serde_yaml;
use config::Manifest;
use error::*;
use sess::Session;

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

    // Assemble the session.
    let sess = Session::new(&root_dir, &manifest);
    debugln!("main: {:#?}", sess);

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
    use config::{PartialManifest, Validate};
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
