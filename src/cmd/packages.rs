// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! The `packages` subcommand.

use std::io::Write;

use clap::{ArgAction, Args};
use indexmap::IndexSet;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::error::*;
use crate::sess::{DependencySource, Session, SessionIo};

/// Information about the dependency graph
#[derive(Args, Debug)]
#[command(alias = "package")]
pub struct PackagesArgs {
    /// Print the dependencies for each package
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub graph: bool,

    /// Do not group packages by topological rank
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub flat: bool,

    /// Print the version of each package
    #[arg(long, action = ArgAction::SetTrue)]
    pub version: bool,

    /// Print the targets available for each package
    #[arg(long, action = ArgAction::SetTrue)]
    pub targets: bool,
}

/// Execute the `packages` subcommand.
pub fn run(sess: &Session, args: &PackagesArgs) -> Result<()> {
    if args.graph && args.version {
        return Err(Error::new("cannot specify both --graph and --version"));
    }
    if args.targets {
        if args.graph {
            return Err(Error::new("cannot specify both --graph and --targets"));
        }
        let rt = Runtime::new()?;
        let io = SessionIo::new(sess);
        let srcs = rt.block_on(io.sources(false, &[]))?;
        let mut target_str = String::from("");
        for pkgs in sess.packages().iter() {
            let pkg_names = pkgs.iter().map(|&id| sess.dependency_name(id));
            for pkg_name in pkg_names {
                target_str.push_str(&format!(
                    "{}:\t{:?}\n",
                    pkg_name,
                    srcs.filter_packages(&IndexSet::from([pkg_name.into()]))
                        .unwrap_or_default()
                        .get_avail_targets()
                ));
            }
        }
        target_str.push_str(&format!(
            "{}:\t{:?}\n",
            &sess.manifest.package.name,
            srcs.filter_packages(&IndexSet::from([sess.manifest.package.name.clone()]))
                .unwrap_or_default()
                .get_avail_targets()
        ));
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", target_str).unwrap();
        tw.flush().unwrap();
        let _ = write!(
            std::io::stdout(),
            "{}",
            String::from_utf8(tw.into_inner().unwrap()).unwrap()
        );
    } else if args.graph {
        let mut graph_str = String::from("");
        for (&pkg, deps) in sess.graph().iter() {
            let pkg_name = sess.dependency_name(pkg);
            let dep_names = deps.iter().map(|&id| sess.dependency_name(id));
            if args.flat {
                // Print one line per dependency.
                for dep_name in dep_names {
                    graph_str.push_str(&format!("{}\t{}\n", pkg_name, dep_name));
                }
            } else {
                // Print all dependencies on one line.
                graph_str.push_str(&format!("{}\t", pkg_name));
                for (i, dep_name) in dep_names.enumerate() {
                    if i > 0 {
                        graph_str.push_str(&format!(" {}", dep_name));
                    } else {
                        graph_str.push_str(dep_name);
                    }
                }
                graph_str.push('\n');
            }
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", graph_str).unwrap();
        tw.flush().unwrap();
        let _ = write!(
            std::io::stdout(),
            "{}",
            String::from_utf8(tw.into_inner().unwrap()).unwrap()
        );
    } else {
        let mut version_str = String::from("");
        for pkgs in sess.packages().iter() {
            let pkg_names = pkgs.iter().map(|&id| sess.dependency_name(id));
            let pkg_sources = pkgs.iter().map(|&id| sess.dependency(id));
            if args.version {
                for pkg_source in pkg_sources {
                    version_str.push_str(&format!(
                        "{}:\t{}\tat {}\t{}\n",
                        pkg_source.name,
                        match pkg_source.version {
                            Some(ref v) => format!("v{}", v),
                            None => "".to_string(),
                        },
                        pkg_source.source,
                        match pkg_source.source {
                            DependencySource::Path { .. } => " as path".to_string(),
                            DependencySource::Git(_) =>
                                format!(" with hash {}", pkg_source.version()),
                            _ => "".to_string(),
                        }
                    ));
                }
            } else if args.flat {
                // Print one line per package.
                for pkg_name in pkg_names {
                    let _ = writeln!(std::io::stdout(), "{}", pkg_name);
                }
            } else {
                // Print all packages per rank on one line.
                for (i, pkg_name) in pkg_names.enumerate() {
                    if i > 0 {
                        let _ = write!(std::io::stdout(), " {}", pkg_name);
                    } else {
                        let _ = write!(std::io::stdout(), "{}", pkg_name);
                    }
                }
                let _ = writeln!(std::io::stdout(),);
            }
        }
        if args.version {
            let mut tw = TabWriter::new(vec![]);
            write!(&mut tw, "{}", version_str).unwrap();
            tw.flush().unwrap();
            let _ = write!(
                std::io::stdout(),
                "{}",
                String::from_utf8(tw.into_inner().unwrap()).unwrap()
            );
        }
    }
    Ok(())
}
