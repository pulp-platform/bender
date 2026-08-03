// Copyright (c) 2026 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

//! Tests for the `dev_dependencies` manifest section.
//!
//! The fixture is built from scratch in a temporary directory and only uses
//! path dependencies, so the tests need no network access.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

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

/// Build the fixture and return its root directory.
///
/// The dependency structure is:
///
/// - `root_pkg` depends on `lib` and dev-depends on `vip`
/// - `lib` dev-depends on `lib_only`, which must not reach `root_pkg`
fn setup(case: &str) -> PathBuf {
    let root = Path::new("tests/tmp/dev_deps").join(case);
    if root.exists() {
        fs::remove_dir_all(&root).expect("Failed to clean fixture directory");
    }
    fs::create_dir_all(&root).expect("Failed to create fixture directory");

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

    root
}

/// Write the root manifest of the fixture.
fn write_root_manifest(root: &Path, manifest: &str) {
    write_file(&root.join("Bender.yml"), manifest);
    write_file(&root.join("src").join("top.sv"), "module top;\nendmodule\n");
    write_file(
        &root.join("tb").join("tb_top.sv"),
        "module tb_top;\nendmodule\n",
    );
}

/// The default root manifest: a regular dependency plus a dev-dependency.
const ROOT_MANIFEST: &str = "package:\n  name: root_pkg\n\
     dependencies:\n  lib: { path: ./lib }\n\
     dev_dependencies:\n  vip: { path: ./vip }\n\
     sources:\n  - src/top.sv\n  - target: test\n    files:\n      - tb/tb_top.sv\n";

/// Run bender in the fixture directory.
fn run_bender(root: &Path, args: &[&str]) -> Output {
    let mut full_args = vec!["-d", root.to_str().unwrap()];
    full_args.extend(args);

    cargo::cargo_bin_cmd!()
        .args(&full_args)
        .output()
        .expect("Failed to execute bender binary")
}

/// Run bender expecting success; returns stdout.
fn run_bender_ok(root: &Path, args: &[&str]) -> String {
    let out = run_bender(root, args);
    assert!(
        out.status.success(),
        "bender {:?} failed.\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout must be utf-8")
}

/// Run bender expecting failure; returns stderr.
fn run_bender_failing(root: &Path, args: &[&str]) -> String {
    let out = run_bender(root, args);
    assert!(
        !out.status.success(),
        "bender {:?} unexpectedly succeeded.\nstdout:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).expect("stderr must be utf-8")
}

/// The dev-dependencies of the root package are resolved, the dev-dependencies
/// of a dependency are not.
#[test]
fn dev_deps_are_root_only() {
    let root = setup("root_only");
    write_root_manifest(&root, ROOT_MANIFEST);

    // Packages of the same rank are printed on a single line.
    let packages: Vec<String> = run_bender_ok(&root, &["packages"])
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
    let root = setup("lockfile");
    write_root_manifest(&root, ROOT_MANIFEST);

    run_bender_ok(&root, &["update"]);
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
    let root = setup("targets");
    write_root_manifest(
        &root,
        "package:\n  name: root_pkg\n\
         dependencies:\n  lib: { path: ./lib }\n\
         dev_dependencies:\n  vip: { path: ./vip, target: test }\n\
         sources:\n  - src/top.sv\n  - target: test\n    files:\n      - tb/tb_top.sv\n",
    );

    let without_test = run_bender_ok(&root, &["script", "flist"]);
    assert!(
        !without_test.contains("vip.sv"),
        "target-gated dev-dependency leaked without `-t test`:\n{}",
        without_test
    );

    let with_test = run_bender_ok(&root, &["script", "flist", "-t", "test"]);
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

/// Listing the same package as both a dependency and a dev-dependency is an
/// error.
#[test]
fn duplicate_dep_and_dev_dep_is_rejected() {
    let root = setup("duplicate");
    write_root_manifest(
        &root,
        "package:\n  name: root_pkg\n\
         dependencies:\n  lib: { path: ./lib }\n\
         dev_dependencies:\n  lib: { path: ./lib }\n\
         sources:\n  - src/top.sv\n",
    );

    let stderr = run_bender_failing(&root, &["packages"]);
    assert!(
        stderr.contains("both a dependency and a dev-dependency"),
        "unexpected error message:\n{}",
        stderr
    );
}

/// The `dev-dependencies` spelling is accepted as an alias.
#[test]
fn dashed_alias_is_accepted() {
    let root = setup("alias");
    write_root_manifest(
        &root,
        "package:\n  name: root_pkg\n\
         dependencies:\n  lib: { path: ./lib }\n\
         dev-dependencies:\n  vip: { path: ./vip }\n\
         sources:\n  - src/top.sv\n",
    );

    let packages = run_bender_ok(&root, &["packages"]);
    assert!(
        packages.split_whitespace().any(|p| p == "vip"),
        "`dev-dependencies` alias not honoured:\n{}",
        packages
    );
}

/// A package that is both a dev-dependency of the root package and a regular
/// transitive dependency resolves to a single entry.
#[test]
fn dev_dep_overlapping_transitive_dep() {
    let root = setup("overlap");
    // Make `lib` depend on `vip` regularly, while the root dev-depends on it.
    write_package(
        &root,
        "lib",
        "package:\n  name: lib\n\
         dependencies:\n  vip: { path: ../vip }\n\
         sources:\n  - src/lib.sv\n",
    );
    write_root_manifest(&root, ROOT_MANIFEST);

    run_bender_ok(&root, &["update"]);
    let lock = fs::read_to_string(root.join("Bender.lock")).expect("Failed to read Bender.lock");
    assert_eq!(
        lock.matches("\n  vip:").count(),
        1,
        "`vip` should appear exactly once in the lockfile:\n{}",
        lock
    );

    // `vip` must be ranked below `lib`, since `lib` depends on it.
    let flist = run_bender_ok(&root, &["script", "flist"]);
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
    let root = setup("parents");
    write_root_manifest(&root, ROOT_MANIFEST);

    let parents = run_bender_ok(&root, &["parents", "vip"]);
    assert!(
        parents.contains("root_pkg"),
        "root package missing from parents of dev-dependency:\n{}",
        parents
    );
}
