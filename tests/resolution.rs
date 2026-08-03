// Copyright (c) 2026 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! End-to-end tests for dependency resolution and the resulting source tree.
//!
//! Manifest parsing and validation is covered by the unit tests in
//! `src/config.rs`; the tests here run the actual binary and assert on the
//! resolved graph, the lockfile and the generated file lists.
//!
//! Fixtures are built from scratch under `tests/tmp/` and only use path
//! dependencies, so no network access is required and each case gets a clean
//! directory.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo;

/// Write `contents` to `path`, creating the parent directories as needed.
fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).expect("Failed to create directory");
    fs::write(path, contents).expect("Failed to write file");
}

/// Create a package with a single source file at `root/name`.
fn write_package(root: &Path, name: &str, manifest: &str) {
    write_file(&root.join(name).join("Bender.yml"), manifest);
    write_file(
        &root.join(name).join("src").join(format!("{}.sv", name)),
        &format!("module {};\nendmodule\n", name),
    );
}

/// Create an empty fixture directory for `case`.
fn fixture(case: &str) -> PathBuf {
    let root = Path::new("tests/tmp").join(case);
    if root.exists() {
        fs::remove_dir_all(&root).expect("Failed to clean fixture directory");
    }
    fs::create_dir_all(&root).expect("Failed to create fixture directory");
    root
}

/// Run bender in the fixture directory, expecting success; returns stdout.
fn run_bender(root: &Path, args: &[&str]) -> String {
    let mut full_args = vec!["-d", root.to_str().unwrap()];
    full_args.extend(args);

    let out = cargo::cargo_bin_cmd!()
        .args(&full_args)
        .output()
        .expect("Failed to execute bender binary");

    assert!(
        out.status.success(),
        "bender {:?} failed.\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout must be utf-8")
}

/// Build the standard dev-dependency fixture and return its root directory.
///
/// The dependency structure is:
///
/// - `root_pkg` depends on `lib` and dev-depends on `vip`
/// - `lib` dev-depends on `lib_only`, which must not reach `root_pkg`
fn dev_dep_fixture(case: &str, root_manifest: &str) -> PathBuf {
    let root = fixture(case);

    write_package(
        &root,
        "lib",
        "package:\n  name: lib\n\
         dev_dependencies:\n  lib_only: { path: ../lib_only }\n\
         sources:\n  - src/lib.sv\n",
    );
    write_package(
        &root,
        "vip",
        "package:\n  name: vip\nsources:\n  - src/vip.sv\n",
    );
    write_package(
        &root,
        "lib_only",
        "package:\n  name: lib_only\nsources:\n  - src/lib_only.sv\n",
    );

    write_file(&root.join("Bender.yml"), root_manifest);
    write_file(&root.join("src").join("top.sv"), "module top;\nendmodule\n");
    write_file(
        &root.join("tb").join("tb_top.sv"),
        "module tb_top;\nendmodule\n",
    );

    root
}

/// The standard root manifest: a regular dependency plus a dev-dependency.
const ROOT_MANIFEST: &str = "package:\n  name: root_pkg\n\
     dependencies:\n  lib: { path: ./lib }\n\
     dev_dependencies:\n  vip: { path: ./vip }\n\
     sources:\n  - src/top.sv\n  - target: test\n    files:\n      - tb/tb_top.sv\n";

/// The dev-dependencies of the root package are resolved, the dev-dependencies
/// of a dependency are not.
#[test]
fn dev_deps_are_root_only() {
    let root = dev_dep_fixture("dev_deps_root_only", ROOT_MANIFEST);

    // Packages of the same rank are printed on a single line.
    let packages: Vec<String> = run_bender(&root, &["packages"])
        .split_whitespace()
        .map(str::to_string)
        .collect();

    assert!(
        packages.iter().any(|p| p == "lib"),
        "regular dependency `lib` missing from {:?}",
        packages
    );
    assert!(
        packages.iter().any(|p| p == "vip"),
        "dev-dependency `vip` of the root package missing from {:?}",
        packages
    );
    assert!(
        !packages.iter().any(|p| p == "lib_only"),
        "dev-dependency `lib_only` of `lib` was propagated into {:?}",
        packages
    );
}

/// The dev-dependencies of the root package end up in the lockfile, the
/// dev-dependencies of a dependency do not.
#[test]
fn dev_deps_in_lockfile() {
    let root = dev_dep_fixture("dev_deps_lockfile", ROOT_MANIFEST);

    run_bender(&root, &["update"]);
    let lock = fs::read_to_string(root.join("Bender.lock")).expect("Failed to read Bender.lock");

    assert!(
        lock.contains("lib:"),
        "`lib` missing from lockfile:\n{}",
        lock
    );
    assert!(
        lock.contains("vip:"),
        "`vip` missing from lockfile:\n{}",
        lock
    );
    assert!(
        !lock.contains("lib_only"),
        "`lib_only` leaked into lockfile:\n{}",
        lock
    );
}

/// The sources of a dev-dependency are available to the root package, and the
/// `target` field filters them as it does for a regular dependency.
#[test]
fn dev_dep_sources_follow_targets() {
    let root = dev_dep_fixture(
        "dev_deps_targets",
        "package:\n  name: root_pkg\n\
         dependencies:\n  lib: { path: ./lib }\n\
         dev_dependencies:\n  vip: { path: ./vip, target: test }\n\
         sources:\n  - src/top.sv\n  - target: test\n    files:\n      - tb/tb_top.sv\n",
    );

    let without_test = run_bender(&root, &["script", "flist"]);
    assert!(
        !without_test.contains("vip.sv"),
        "target-gated dev-dependency leaked without `-t test`:\n{}",
        without_test
    );

    let with_test = run_bender(&root, &["script", "flist", "-t", "test"]);
    assert!(
        with_test.contains("vip.sv"),
        "dev-dependency sources missing with `-t test`:\n{}",
        with_test
    );
    assert!(
        with_test.contains("tb_top.sv"),
        "root testbench sources missing with `-t test`:\n{}",
        with_test
    );
}

/// A package that is both a dev-dependency of the root package and a regular
/// transitive dependency resolves to a single entry.
#[test]
fn dev_dep_overlapping_transitive_dep() {
    let root = dev_dep_fixture("dev_deps_overlap", ROOT_MANIFEST);
    // Make `lib` depend on `vip` regularly, while the root dev-depends on it.
    write_package(
        &root,
        "lib",
        "package:\n  name: lib\n\
         dependencies:\n  vip: { path: ../vip }\n\
         sources:\n  - src/lib.sv\n",
    );

    run_bender(&root, &["update"]);
    let lock = fs::read_to_string(root.join("Bender.lock")).expect("Failed to read Bender.lock");
    assert_eq!(
        lock.matches("\n  vip:").count(),
        1,
        "`vip` should appear exactly once in the lockfile:\n{}",
        lock
    );

    // `vip` must be ranked below `lib`, since `lib` depends on it.
    let flist = run_bender(&root, &["script", "flist"]);
    let vip_pos = flist.find("vip.sv").expect("vip sources missing");
    let lib_pos = flist.find("lib.sv").expect("lib sources missing");
    assert!(
        vip_pos < lib_pos,
        "`vip` must come before `lib` in the file list:\n{}",
        flist
    );
}

/// `bender parents` reports the root package as a parent of its
/// dev-dependencies.
#[test]
fn parents_reports_root_dev_deps() {
    let root = dev_dep_fixture("dev_deps_parents", ROOT_MANIFEST);

    let parents = run_bender(&root, &["parents", "vip"]);
    assert!(
        parents.contains("root_pkg"),
        "root package missing from parents of dev-dependency:\n{}",
        parents
    );
}
