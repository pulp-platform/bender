// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! Main command line tool implementation.

use std;
use std::path::{Path, PathBuf};
use clap::{App, Arg};

pub fn main() -> Result<(), String> {
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
        None => find_package_root(".").map_err(|cause| Error::chain(
            "Cannot find package root directory.",
            cause,
        ))?,
    };
    print!("main: root dir {:?}", root_dir);

    Ok(())
}

/// Find the root directory of a package.
///
/// Traverses the directory hierarchy upwards until a `Landa.yml` file is found.
fn find_package_root<P>(from: P) -> Result<PathBuf, Error>
    where P: AsRef<Path>
{
    use std::fs::{canonicalize, metadata};
    use std::os::unix::fs::MetadataExt;

    // Canonicalize the path. This will resolve any intermediate links.
    let mut path = canonicalize(from.as_ref()).map_err(|cause| Error::chain(
        format!("Failed to canonicalize path {:?}.", from.as_ref()),
        cause,
    ))?;
    println!("find_package_root: canonicalized to {:?}", path);

    // Look up the device at the current path. This information will then be
    // used to stop at filesystem boundaries.
    let limit_rdev: Option<_> = metadata(&path).map(|m| m.dev()).ok();
    println!("find_package_root: limit rdev = {:?}", limit_rdev);

    // Step upwards through the path hierarchy.
    for i in 0..100 {
        println!("find_package_root: looking in {:?}", path);

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
        println!("find_package_root: rdev = {:?}", rdev);
        if rdev != limit_rdev {
            return Err(Error::new(format!(
                "Stopped at filesystem boundary {:?}.",
                tested_path
            )));
        }
    }

    Err(Error::new("Reached maximum number of search steps."))
}

#[derive(Debug)]
pub struct Error {
    /// A formatted error message.
    pub msg: String,
    /// An optional underlying cause.
    pub cause: Option<Box<std::error::Error>>,
}

impl Error {
    /// Create a new error without cause.
    pub fn new<S: Into<String>>(msg: S) -> Error {
        Error {
            msg: msg.into(),
            cause: None,
        }
    }

    /// Create a new error with cause.
    pub fn chain<S,E>(msg: S, cause: E) -> Error
        where S: Into<String>, E: Into<Box<std::error::Error>>
    {
        Error {
            msg: msg.into(),
            cause: Some(cause.into()),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        &self.msg
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match self.cause {
            Some(ref b) => Some(b.as_ref()),
            None => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.msg)?;
        if let Some(ref c) = self.cause {
            write!(f, " {}", c)?
        }
        Ok(())
    }
}

impl From<Error> for String {
    fn from(err: Error) -> String {
        format!("{}", err)
    }
}
