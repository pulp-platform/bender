// Copyright (c) 2021 ETH Zurich
// Michael Rogenmoser <michaero@iis.ee.ethz.ch>

//! The `parents` subcommand.

use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::fs::canonicalize;

#[cfg(windows)]
use dunce::canonicalize;

use crate::diagnostic::Warnings;
use clap::Args;
use indexmap::IndexMap;
use miette::IntoDiagnostic as _;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

use crate::Result;
use crate::config::Dependency;
use crate::sess::{DependencyConstraint, DependencySource};
use crate::sess::{Session, SessionIo};
use crate::{fmt_path, fmt_version};

/// Format a `DependencySource` as a string, canonicalizing path dependencies
/// relative to the given base directory. This ensures that different relative
/// paths pointing to the same directory (e.g., `ip/common` vs `../common`)
/// produce the same string representation.
fn format_dep_source(source: &DependencySource, base_dir: &Path) -> String {
    match source {
        DependencySource::Path(rel_path) => {
            let abs_path = base_dir.join(rel_path);
            let canonical = canonicalize(&abs_path).unwrap_or(abs_path);
            format!("{:?}", canonical)
        }
        other => format!("{}", other),
    }
}

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
    let rt = Runtime::new().into_diagnostic()?;
    let io = SessionIo::new(sess);

    if args.targets {
        let parent_targets = get_parent_targets(sess, &rt, &io, dep)?;
        let mut res = String::from("");
        for (k, v) in parent_targets.iter() {
            res.push_str(&format!(
                "    {}\tfilters: {}\tpasses: {:?}\n",
                k,
                v[0],
                &v[1..]
            ));
        }
        let mut tw = TabWriter::new(vec![]);
        write!(&mut tw, "{}", res).unwrap();
        tw.flush().unwrap();
        print!("{}", String::from_utf8(tw.into_inner().unwrap()).unwrap());
        return Ok(());
    }

    let parents = get_parent_requirements(sess, &rt, &io, dep)?;

    if parents.is_empty() {
        let _ = writeln!(std::io::stdout(), "No parents found for {}.", dep);
    } else {
        let _ = writeln!(std::io::stdout(), "Parents found:");
        let source = &parents.values().next().unwrap().source;
        let constant_source = parents.values().all(|p| &p.source == source);
        let mut res = String::from("");
        if constant_source {
            for (k, p) in parents.iter() {
                res.push_str(&format!("    {}\trequires: {}\n", k, p.constraint));
            }
        } else {
            for (k, p) in parents.iter() {
                res.push_str(&format!(
                    "    {}\trequires: {}\tat {}\n",
                    k, p.constraint, p.source
                ));
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
        match sess.dependency(mydep).version {
            // The default `v` namespace is left implicit, as it always has been; only a custom
            // one needs spelling out.
            Some(ref ver) => match sess.dependency(mydep).version_prefix.as_deref() {
                Some(prefix) => format!("{}{}", prefix, ver),
                None => ver.to_string(),
            },
            None => String::new(),
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
                Dependency::Version { ref version, .. } => {
                    format!("version {}", fmt_version!(version))
                }
                Dependency::Path { ref path, .. } => format!("path {}", fmt_path!(path.display())),
                Dependency::GitRevision {
                    ref url, ref rev, ..
                } => {
                    format!("git {} at revision {}", fmt_path!(url), fmt_version!(rev))
                }
                Dependency::GitVersion {
                    ref url,
                    ref version,
                    ref version_prefix,
                    ..
                } => {
                    format!(
                        "git {} with version {}{}",
                        fmt_path!(url),
                        fmt_version!(version),
                        match version_prefix {
                            Some(prefix) => format!(" (prefix `{}`)", prefix),
                            None => String::new(),
                        }
                    )
                }
            },
        }
        .emit();
    }

    Ok(())
}

/// Get parents array
/// What one parent requires of a dependency.
pub struct ParentRequirement {
    /// The constraint the parent imposes.
    pub constraint: DependencyConstraint,
    /// Where the parent pulls the dependency from, formatted for display.
    pub source: String,
}

/// Map every parent that depends on `dep` through `f`, which receives the dependency entry the
/// parent declares and the directory that entry's paths are relative to.
fn map_parents<T>(
    sess: &Session,
    rt: &Runtime,
    io: &SessionIo,
    dep: &str,
    mut f: impl FnMut(&Dependency, &Path) -> T,
) -> Result<IndexMap<String, T>> {
    let mut map = IndexMap::new();
    if let Some(entry) = sess.manifest.dependencies.get(dep) {
        map.insert(sess.manifest.package.name.clone(), f(entry, sess.root));
    }
    for (&pkg, deps) in sess.graph().iter() {
        let pkg_name = sess.dependency_name(pkg);
        for current_dep in deps.iter().map(|&id| sess.dependency(id)) {
            if dep != current_dep.name.as_str() {
                continue;
            }
            let mismatch = || {
                Warnings::IncludeDepManifestMismatch {
                    pkg: pkg_name.to_string(),
                }
                .emit()
            };
            // Filter out dependencies without a manifest.
            let Some(dep_manifest) = rt.block_on(io.dependency_manifest(pkg, false, &[]))? else {
                mismatch();
                continue;
            };
            match dep_manifest.dependencies.get(dep) {
                Some(entry) => {
                    let pkg_path = sess.get_package_path(pkg);
                    map.insert(pkg_name.to_string(), f(entry, &pkg_path));
                }
                None => mismatch(),
            }
        }
    }
    Ok(map)
}

/// The constraint every parent of `dep` imposes on it.
pub fn get_parent_requirements(
    sess: &Session,
    rt: &Runtime,
    io: &SessionIo,
    dep: &str,
) -> Result<IndexMap<String, ParentRequirement>> {
    map_parents(sess, rt, io, dep, |entry, base| ParentRequirement {
        constraint: DependencyConstraint::from(entry),
        source: format_dep_source(&DependencySource::from(entry), base),
    })
}

/// The target filter each parent of `dep` applies, followed by the targets it passes down.
pub fn get_parent_targets(
    sess: &Session,
    rt: &Runtime,
    io: &SessionIo,
    dep: &str,
) -> Result<IndexMap<String, Vec<String>>> {
    map_parents(sess, rt, io, dep, |entry, _| {
        let (targetspec, tgts) = match entry {
            Dependency::Version {
                target,
                pass_targets,
                ..
            }
            | Dependency::Path {
                target,
                pass_targets,
                ..
            }
            | Dependency::GitRevision {
                target,
                pass_targets,
                ..
            }
            | Dependency::GitVersion {
                target,
                pass_targets,
                ..
            } => (target, pass_targets),
        };
        let mut tgts = tgts.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        tgts.insert(0, targetspec.to_string());
        tgts
    })
}
