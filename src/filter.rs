// Copyright (c) 2025 ETH Zurich
// Alessandro Ottaviano <aottaviano@iis.ee.ethz.ch>

//! Heuristic (regex) filtering of unused sources from a DUT and (optionally) TB
//! top levels by tree traversal.

use crate::error::{Error, Result};
use crate::sess::{Session, SessionIo};

use common_path::common_path_all;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// Options controlling SystemVerilog dependency filtering.
#[derive(Debug, Clone)]
pub struct FilterOptions {
    /// RTL top module stems (case-insensitive; `[_-.]` ignored)
    pub rtl_tops: Vec<String>,
    /// Testbench top stems or glob-style patterns
    pub tb_tops: Vec<String>,
    /// Print a compact summary about the used files
    pub show_tree: bool,
}

/// Filter out unused SystemVerilog files from a flattened Bender source list.
/// `srcs_flat_files` must contain `(package_name, absolute_path)` entries.
pub fn filter_unused(
    sess: &Session,
    srcs_flat_files: &[(String, PathBuf)],
    opts: &FilterOptions,
) -> Result<BTreeSet<PathBuf>> {
    // package discovery (roots/manifests)
    let pkgs = collect_packages_from_flat(sess, srcs_flat_files)?;
    let mut pkg_by_file: BTreeMap<PathBuf, String> = BTreeMap::new();
    let roots_by_pkg: BTreeMap<String, PathBuf> =
        pkgs.iter().map(|p| (p.name.clone(), p.root.clone())).collect();

    // superset of existing files, normalized once
    let mut all_files = Vec::<FileInfo>::new();
    for (pkg, abs) in srcs_flat_files {
        if abs.is_file() {
            let k = norm(abs);
            pkg_by_file.insert(k.clone(), pkg.clone());
            all_files.push(FileInfo {
                abs: k,
                pkg: pkg.clone(),
                is_tb: is_tb(abs),
                level: 0,
            });
        }
    }

    // parse level hints (`# Level N`) from each Bender.yml
    let mut level_map_abs: BTreeMap<PathBuf, u32> = BTreeMap::new();
    for p in &pkgs {
        let raw = fs::read_to_string(&p.manifest)
            .map_err(|e| Error::new(format!("Failed to read {}: {e}", p.manifest.display())))?;
        for (rel, lvl) in scan_levels_from_bender_yaml_text(&raw) {
            let abs = p.root.join(&rel);
            if abs.exists() {
                level_map_abs.insert(norm(&abs), lvl);
            }
        }
    }
    for f in &mut all_files {
        f.level = *level_map_abs.get(&f.abs).unwrap_or(&0);
    }

    // indices for traversal
    let mut pkg_defs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut mod_defs_per_pkg: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let abs2lvl: BTreeMap<PathBuf, u32> =
        all_files.iter().map(|f| (f.abs.clone(), f.level)).collect();
    let text_cache = TextCache::default();

    for f in &all_files {
        let txt = text_cache.read(&f.abs);

        // packages
        for cap in RE_SV_PACKAGE_DEF.captures_iter(&txt) {
            pkg_defs.entry(cap[1].to_string()).or_insert(f.abs.clone());
        }

        // modules (only sv units, skip TB)
        if is_sv_unit(&f.abs) && !f.is_tb {
            for cap in RE_SV_MODULE_DEF.captures_iter(&txt) {
                mod_defs_per_pkg
                    .entry(f.pkg.clone())
                    .or_default()
                    .entry(cap[1].to_lowercase())
                    .or_insert(f.abs.clone());
            }
        }
    }

    // seeds
    let main_pkg = sess.manifest.package.name.clone();
    let rtl_stems: BTreeSet<String> = opts.rtl_tops.iter().map(normalize_stem).collect();
    let tb_patterns = opts
        .tb_tops
        .iter()
        .filter_map(|p| glob::Pattern::new(&p.to_lowercase()).ok())
        .collect::<Vec<_>>();

    // pick lowest-level match per RTL stem in main pkg
    let mut starts: Vec<(String, PathBuf, u32)> = Vec::new();
    if !rtl_stems.is_empty() {
        let mut best_by_stem: BTreeMap<String, (PathBuf, u32)> = BTreeMap::new();
        for f in &all_files {
            if f.pkg != main_pkg || !is_sv_unit(&f.abs) || f.is_tb {
                continue;
            }
            let st = normalize_stem(&path_stem(&f.abs));
            if rtl_stems.contains(&st) {
                best_by_stem
                    .entry(st)
                    .and_modify(|e| {
                        if f.level < e.1 {
                            *e = (f.abs.clone(), f.level);
                        }
                    })
                    .or_insert((f.abs.clone(), f.level));
            }
        }
        for (abs, lvl) in best_by_stem.values() {
            starts.push((main_pkg.clone(), abs.clone(), *lvl));
        }
    }

    // add TB seeds matching patterns
    if !tb_patterns.is_empty() {
        for f in &all_files {
            if f.is_tb && is_sv_unit(&f.abs) {
                let stem = path_stem(&f.abs).to_lowercase();
                if tb_patterns.iter().any(|p| p.matches(&stem)) {
                    starts.push((f.pkg.clone(), f.abs.clone(), f.level));
                }
            }
        }
    }

    if starts.is_empty() {
        return Err(Error::new(
            "No BFS start points: pass --rtl-top and/or --tb-top with --filter-unused",
        ));
    }

    // set of DUT absolute paths (for TB-only allowlist)
    let dut_abs_paths: BTreeSet<PathBuf> = starts
        .iter()
        .filter(|(p, _, _)| p == &main_pkg)
        .map(|(_, a, _)| a.clone())
        .collect();

    // traversal
    let allow_rtl = !rtl_stems.is_empty();
    let mut used: BTreeSet<PathBuf> = starts.iter().map(|(_, a, _)| a.clone()).collect();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut q: VecDeque<(String, PathBuf, u32)> = starts.into();

    // helper to push edges if allowed
    let mut push_if = |pkg_hint: &str, abs: &Path| -> Option<(String, PathBuf, u32)> {
        let abs = norm(abs);
        if !allow_file(allow_rtl, &abs, &pkg_by_file, &dut_abs_paths, &main_pkg) {
            return None;
        }
        if !used.insert(abs.clone()) {
            return None;
        }
        let pkg = pkg_by_file
            .get(&abs)
            .cloned()
            .unwrap_or_else(|| pkg_hint.to_string());
        let lvl = *abs2lvl.get(&abs).unwrap_or(&0);
        Some((pkg, abs, lvl))
    };

    while let Some((pkg, cur, lvl)) = q.pop_front() {
        if !seen.insert(cur.clone()) {
            continue;
        }
        let txt = text_cache.read(&cur);

        // `include "..."` edges
        for inc in scan_includes(&txt) {
            if let Some(abs) = resolve_include(&cur, roots_by_pkg.get(&pkg), &inc) {
                if let Some(next) = push_if(&pkg, &abs) {
                    q.push_back(next);
                }
            }
        }

        // package uses ? package definition file
        for p_used in scan_pkg_uses(&txt) {
            if let Some(abs) = pkg_defs.get(&p_used) {
                if let Some(next) = push_if(&pkg, abs) {
                    q.push_back(next);
                }
            }
        }

        // module instantiations
        let code = strip_sv_comments(&txt);
        let mut insts: BTreeSet<String> = BTreeSet::new();
        for caps in RE_SV_INSTANTIATION.captures_iter(&code) {
            let m = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !RE_EXCLUDE_NAME.is_match(m) {
                insts.insert(m.to_string());
            }
        }

        if insts.is_empty() {
            continue;
        }

        // prefer same-package definitions
        if let Some(map) = mod_defs_per_pkg.get(&pkg) {
            for name in &insts {
                if file_defines_module(&txt, name) {
                    continue;
                }
                if let Some(abs) = map.get(&name.to_lowercase()) {
                    if let Some(next) = push_if(&pkg, abs) {
                        q.push_back(next);
                    }
                }
            }
        }

        // cross-package fallback (by level)
        let mut cross: Vec<(u32, PathBuf, String)> = Vec::new();
        for (pname, mods) in &mod_defs_per_pkg {
            if pname == &pkg {
                continue;
            }
            for name in &insts {
                if let Some(abs) = mods.get(&name.to_lowercase()) {
                    cross.push((*abs2lvl.get(abs).unwrap_or(&0), abs.clone(), pname.clone()));
                }
            }
        }
        cross.sort_by_key(|t| t.0);
        for (_, abs, p_pkg) in cross {
            if let Some(next) = push_if(&p_pkg, &abs) {
                q.push_back(next);
            }
        }

        // (unused) lvl is kept for potential future heuristics
        let _ = lvl;
    }

    if opts.show_tree {
        eprintln!("[filter] used files: {}", used.len());
    }

    Ok(used)
}

#[derive(Debug, Clone)]
struct FileInfo {
    abs: PathBuf,
    pkg: String,
    is_tb: bool,
    level: u32,
}

#[derive(Default)]
struct TextCache {
    map: parking_lot::Mutex<BTreeMap<PathBuf, String>>,
}
impl TextCache {
    fn read(&self, abs: &Path) -> String {
        let k = norm(abs);
        if let Some(s) = self.map.lock().get(&k).cloned() {
            return s;
        }
        let s = fs::read_to_string(&k).unwrap_or_default();
        self.map.lock().insert(k.clone(), s.clone());
        s
    }
}

static RE_FILES_KEY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*files\s*:\s*$").unwrap());
static RE_LEVEL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)#\s*Level\s+(\d+)\b"#).unwrap());
static RE_FILE_ITEM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*-\s+([^\s#]+)\s*$"#).unwrap());

static RE_SV_PACKAGE_DEF: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*package\s+([A-Za-z_]\w*)\b").unwrap());
static RE_SV_PACKAGE_USE_IMPORT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bimport\s+([A-Za-z_]\w*)\s*::\s*\*").unwrap());
static RE_SV_PACKAGE_SCOPED_USE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([A-Za-z_]\w*)\s*::\s*[A-Za-z_]\w*").unwrap());
static RE_SV_INCLUDE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^[ \t]*`include[ \t]+"([^"]+)""#).unwrap());
static RE_SV_MODULE_DEF: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*module\s+([A-Za-z_]\w*)\b").unwrap());
static RE_SV_INSTANTIATION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?mx)
        ^\s*([A-Za-z_]\w*)\s*
        (?:\#\s*\((?:[^()]|\([^()]*\))*\)\s*)?
        ([A-Za-z_]\w*)\s*
        (?:\[[^\]]+\]\s*)?
        \(
        "#
    ).unwrap()
});
static RE_EXCLUDE_NAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^(?:module|macromodule|primitive|interface|class|package|program|checker|function|task|typedef|struct|union|enum)$"#).unwrap()
});
static RE_LINE_COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)//.*?$").unwrap());
static RE_BLOCK_COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());

fn strip_sv_comments(t: &str) -> String {
    let no_block = RE_BLOCK_COMMENT.replace_all(t, "");
    let no_line = RE_LINE_COMMENT.replace_all(&no_block, "");
    no_line.into_owned()
}

fn scan_includes(t: &str) -> Vec<String> {
    RE_SV_INCLUDE
        .captures_iter(t)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn scan_pkg_uses(t: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in RE_SV_PACKAGE_USE_IMPORT.captures_iter(t) {
        out.insert(c[1].to_string());
    }
    for c in RE_SV_PACKAGE_SCOPED_USE.captures_iter(t) {
        out.insert(c[1].to_string());
    }
    out
}

fn file_defines_module(text: &str, mod_name: &str) -> bool {
    RE_SV_MODULE_DEF
        .captures_iter(text)
        .any(|c| c[1].eq(mod_name))
}

fn is_sv_unit(p: &Path) -> bool {
    matches!(p.extension().and_then(|s| s.to_str()), Some("sv" | "v"))
}

static RE_TB: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(^tb_|_tb$|_tb_|tb$)").unwrap());

fn is_tb(p: &Path) -> bool {
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    matches!(p.extension().and_then(|e| e.to_str()), Some("sv" | "v")) && RE_TB.is_match(&stem)
}

fn path_stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

fn normalize_stem<S: AsRef<str>>(s: S) -> String {
    s.as_ref()
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .collect()
}

/// Canonicalize a path but fall back gracefully if canonicalization fails.
fn norm(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Resolve an `` `include "..." `` path relative to the includer, then package root.
fn resolve_include(includer_abs: &Path, pkg_root: Option<&PathBuf>, inc: &str) -> Option<PathBuf> {
    let inc_path = Path::new(inc);
    if inc_path.is_absolute() && inc_path.exists() {
        return Some(norm(inc_path));
    }
    includer_abs
        .parent()
        .and_then(|dir| {
            let cand = dir.join(inc_path);
            cand.exists().then(|| norm(&cand))
        })
        .or_else(|| {
            pkg_root.and_then(|root| {
                let cand = root.join(inc_path);
                cand.exists().then(|| norm(&cand))
            })
        })
}

/// Extract `# Level N` hints from textual `Bender.yml`, scanning only `sources: ... files:`.
fn scan_levels_from_bender_yaml_text(text: &str) -> BTreeMap<PathBuf, u32> {
    let mut levels = BTreeMap::new();
    let mut in_sources = false;
    let mut current_level: u32 = 0;

    for line in text.lines() {
        let t = line.trim_start();

        if !in_sources {
            if t.starts_with("sources:") {
                in_sources = true;
            }
            continue;
        }

        // end of sources section heuristics
        if !line.starts_with(' ') && line.trim_end().ends_with(':') && !t.starts_with("files:") {
            break;
        }

        if RE_FILES_KEY.is_match(line) {
            current_level = 0;
            continue;
        }
        if let Some(c) = RE_LEVEL.captures(line) {
            current_level = c[1].parse::<u32>().unwrap_or(0);
            continue;
        }
        if let Some(c) = RE_FILE_ITEM.captures(line) {
            let p = c[1].trim();
            if p.ends_with(".sv") || p.ends_with(".v") || p.ends_with(".svh") {
                levels.insert(PathBuf::from(p), current_level);
            }
        }
    }
    levels
}

/// Allowlist policy for TB-only traversals:
/// - always allow when RTL traversal is enabled;
/// - otherwise, exclude explicit DUT files;
/// - always allow any testbench files;
/// - allow non-test files only when they are outside the main package.
fn allow_file(
    allow_rtl: bool,
    abs: &Path,
    pkg_by_file: &BTreeMap<PathBuf, String>,
    dut_abs_paths: &BTreeSet<PathBuf>,
    main_pkg: &str,
) -> bool {
    if allow_rtl {
        return true;
    }
    if dut_abs_paths.contains(&norm(abs)) {
        return false;
    }
    if is_tb(abs) {
        return true;
    }
    matches!(pkg_by_file.get(&norm(abs)), Some(p) if p != main_pkg)
}

#[derive(Debug, Clone)]
struct PackageCtx {
    name: String,
    root: PathBuf,
    manifest: PathBuf,
}

fn collect_packages_from_flat(sess: &Session, flat: &[(String, PathBuf)]) -> Result<Vec<PackageCtx>> {
    let mut by_pkg: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (pkg, abs) in flat {
        if let Some(dir) = abs.parent() {
            by_pkg.entry(pkg.clone()).or_default().push(dir.to_path_buf());
        }
    }

    let io = SessionIo::new(sess);
    let main_pkg = &sess.manifest.package.name;

    let mut out = Vec::new();
    for (pkg, dirs) in by_pkg {
        let root = if &pkg == main_pkg {
            sess.root.to_path_buf()
        } else if let Ok(dep_id) = sess.dependency_with_name(&pkg) {
            io.get_package_path(dep_id)
        } else {
            common_path_all(dirs.iter().map(|p| p.as_path()))
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| dirs[0].clone())
        };

        out.push(PackageCtx {
            name: pkg,
            manifest: root.join("Bender.yml"),
            root,
        });
    }
    Ok(out)
}
