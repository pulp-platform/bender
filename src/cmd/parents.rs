// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `parents` subcommand.

use std::io::Write;

use crate::diagnostic::Warnings;
use clap::Args;
use indexmap::IndexMap;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::config::Dependency;
use crate::error::*;
use crate::sess::{DependencyConstraint, DependencySource};
use crate::sess::{Session, SessionIo};
use crate::{fmt_path, fmt_version};

/// List packages calling this dependency
#[derive(Args, Debug)]
#[command(alias = "parent")]
pub struct ParentsArgs {
    /// Package name to get the parents for
    pub name: String,

    /// Print the passed targets to the dependency
    #[arg(long)]
    pub targets: bool,
}

/// Execute the `parents` subcommand.
pub fn run(sess: &Session, args: &ParentsArgs) -> Result<()> {
    let dep = &args.name.to_lowercase();
    let mydep = sess.dependency_with_name(dep)?;
    let rt = Runtime::new()?;
    let io = SessionIo::new(sess);

    let parent_array = get_parent_array(sess, &rt, &io, dep, args.targets)?;

    if args.targets {
        let mut res = String::from("");
        for (k, v) in parent_array.iter() {
            res.push_str(
                &format!("    {}\tfilters: {}\tpasses: {:?}\n", k, &v[0], &v[1..]).to_string(),
            );
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", res).unwrap();
        tw.flush().unwrap();
        print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());
        return Ok(());
    }

    if parent_array.is_empty() {
        let _ = writeln!(std::io::stdout(), "No parents found for {}.", dep);
    } else {
        let _ = writeln!(std::io::stdout(), "Parents found:");
        let source = &parent_array.values().next().unwrap()[1];
        let mut constant_source = true;
        for (_, v) in parent_array.iter() {
            if &v[1] != source {
                constant_source = false;
                break;
            }
        }
        let mut res = String::from("");
        if constant_source {
            for (k, v) in parent_array.iter() {
                res.push_str(&format!("    {}\trequires: {}\n", k, v[0]).to_string());
            }
        } else {
            for (k, v) in parent_array.iter() {
                res.push_str(&format!("    {}\trequires: {}\tat {}\n", k, v[0], v[1]).to_string());
            }
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", res).unwrap();
        tw.flush().unwrap();
        let _ = write!(
            std::io::stdout(),
            "{}",
            String::from_utf8(tw.into_inner().unwrap()).unwrap()
        );
    }

    let _ = writeln!(
        std::io::stdout(),
        "{} used version: {} at {}{}",
        sess.dependency(mydep).name,
        match sess.dependency(mydep).version.clone() {
            Some(ver) => ver.to_string(),
            None => "".to_string(),
        },
        sess.dependency(mydep).source,
        match sess.dependency(mydep).source {
            DependencySource::Path { .. } => " as path".to_string(),
            DependencySource::Git(_) => format!(" with hash {}", sess.dependency(mydep).version()),
            _ => "".to_string(),
        }
    );

    if sess.config.overrides.contains_key(dep) {
        Warnings::DepOverride {
            pkg: dep.to_string(),
            pkg_override: match sess.config.overrides[dep] {
                Dependency::Version(ref v, _) => format!("version {}", fmt_version!(v)),
                Dependency::Path(ref path, _) => format!("path {}", fmt_path!(path.display())),
                Dependency::GitRevision(ref url, ref rev, _) => {
                    format!("git {} at revision {}", fmt_path!(url), fmt_version!(rev))
                }
                Dependency::GitVersion(ref url, ref version, _) => {
                    format!(
                        "git {} with version {}",
                        fmt_path!(url),
                        fmt_version!(version)
                    )
                }
            },
        }
        .emit();
    }

    Ok(())
}

/// Get parents array
pub fn get_parent_array(
    sess: &Session,
    rt: &Runtime,
    io: &SessionIo,
    dep: &str,
    targets: bool,
) -> Result<IndexMap<String, Vec<String>>> {
    let mut map = IndexMap::<String, Vec<String>>::new();
    if sess.manifest.dependencies.contains_key(dep) {
        if targets {
            map.insert(
                sess.manifest.package.name.clone(),
                match sess.manifest.dependencies.get(dep).unwrap() {
                    Dependency::Version(targetspec, _, tgts)
                    | Dependency::Path(targetspec, _, tgts)
                    | Dependency::GitRevision(targetspec, _, _, tgts)
                    | Dependency::GitVersion(targetspec, _, _, tgts) => {
                        let mut tgts = tgts.clone();
                        tgts.insert(0, targetspec.to_string());
                        tgts
                    }
                },
            );
        } else {
            let dep_str = format!(
                "{}",
                DependencyConstraint::from(&sess.manifest.dependencies[dep])
            );
            let dep_source = format!(
                "{}",
                DependencySource::from(&sess.manifest.dependencies[dep])
            );
            map.insert(
                sess.manifest.package.name.clone(),
                vec![dep_str, dep_source],
            );
        }
    }
    for (&pkg, deps) in sess.graph().iter() {
        let pkg_name = sess.dependency_name(pkg);
        let all_deps = deps.iter().map(|&id| sess.dependency(id));
        for current_dep in all_deps {
            if dep == current_dep.name.as_str() {
                let dep_manifest = rt.block_on(io.dependency_manifest(pkg, false, &[]))?;
                // Filter out dependencies without a manifest
                if dep_manifest.is_none() {
                    Warnings::IncludeDepManifestMismatch {
                        pkg: pkg_name.to_string(),
                    }
                    .emit();
                    continue;
                }
                let dep_manifest = dep_manifest.unwrap();
                if dep_manifest.dependencies.contains_key(dep) {
                    if targets {
                        map.insert(
                            pkg_name.to_string(),
                            match dep_manifest.dependencies.get(dep).unwrap() {
                                Dependency::Version(targetspec, _, tgts)
                                | Dependency::Path(targetspec, _, tgts)
                                | Dependency::GitRevision(targetspec, _, _, tgts)
                                | Dependency::GitVersion(targetspec, _, _, tgts) => {
                                    let mut tgts = tgts.clone();
                                    tgts.insert(0, targetspec.to_string());
                                    tgts
                                }
                            },
                        );
                    } else {
                        map.insert(
                            pkg_name.to_string(),
                            vec![
                                format!(
                                    "{}",
                                    DependencyConstraint::from(&dep_manifest.dependencies[dep])
                                ),
                                format!(
                                    "{}",
                                    DependencySource::from(&dep_manifest.dependencies[dep])
                                ),
                            ],
                        );
                    }
                } else {
                    Warnings::IncludeDepManifestMismatch {
                        pkg: pkg_name.to_string(),
                    }
                    .emit();
                }
            }
        }
    }
    Ok(map)
}
