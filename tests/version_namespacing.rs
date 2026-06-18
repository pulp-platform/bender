// Copyright (c) 2026 ETH Zurich

//! Integration tests for namespaced (prefixed) version resolution.
//!
//! These build throwaway git repositories with both default `v*` tags and
//! `companyX-v*` tags, then drive the real `bender` binary to check that
//! resolution stays within the requested namespace, persists it, and refuses
//! to mix namespaces.

// `file://` URLs with absolute paths are awkward on Windows; the git-based
// fixtures here mirror the bash regression scripts, which are Unix-only.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::cargo;

/// Run a git command in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@localhost")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@localhost")
        .output()
        .expect("failed to run git");
    assert!(
        status.status.success(),
        "git {:?} failed:\n{}",
        args,
        String::from_utf8_lossy(&status.stderr)
    );
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).expect("failed to write file");
}

/// Create a fresh, empty directory under the test binary's tmp dir.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create a `foo` dependency repo carrying both `v*` and `companyX-v*` tags.
///
/// Tags: `v1.0.0`, `v1.1.0`, `companyX-v1.0.0`, `companyX-v2.0.0`.
fn setup_foo(base: &Path) -> String {
    let repo = base.join("foo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    write(&repo.join("Bender.yml"), "package:\n  name: foo\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["tag", "v1.0.0"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "c2"]);
    git(&repo, &["tag", "v1.1.0"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "c3"]);
    git(&repo, &["tag", "companyX-v1.0.0"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "c4"]);
    git(&repo, &["tag", "companyX-v2.0.0"]);
    format!("file://{}", repo.display())
}

/// Create a project directory with the given `Bender.yml` body.
fn setup_project(base: &Path, name: &str, manifest: &str) -> PathBuf {
    let dir = base.join(name);
    fs::create_dir_all(&dir).unwrap();
    write(&dir.join("Bender.yml"), manifest);
    dir
}

fn bender_update(root: &Path) -> Output {
    cargo::cargo_bin_cmd!()
        .arg("update")
        .current_dir(root)
        .output()
        .expect("failed to run bender")
}

#[test]
fn resolves_default_v_namespace() {
    let base = fresh_dir("default_v");
    let foo_url = setup_foo(&base);
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1\" }}\n"
        ),
    );

    let out = bender_update(&app);
    assert!(
        out.status.success(),
        "update failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lock = fs::read_to_string(app.join("Bender.lock")).unwrap();
    // Highest `v1.x` tag wins, and the default prefix is not recorded.
    assert!(lock.contains("version: 1.1.0"), "lockfile:\n{lock}");
    assert!(
        !lock.contains("version_prefix"),
        "default prefix must not be persisted:\n{lock}"
    );
}

#[test]
fn resolves_custom_namespace() {
    let base = fresh_dir("custom_ns");
    let foo_url = setup_foo(&base);
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  \
             foo: {{ git: \"{foo_url}\", version: \"*\", version_prefix: \"companyX-v\" }}\n"
        ),
    );

    let out = bender_update(&app);
    assert!(
        out.status.success(),
        "update failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lock = fs::read_to_string(app.join("Bender.lock")).unwrap();
    // Resolution stays in the `companyX-v` namespace: highest there is 2.0.0,
    // and the `v1.x` tags are ignored entirely.
    assert!(lock.contains("version: 2.0.0"), "lockfile:\n{lock}");
    assert!(
        !lock.contains("version: 1.1.0"),
        "must not leak the default `v` namespace:\n{lock}"
    );
    assert!(
        lock.contains("version_prefix: companyX-v"),
        "custom prefix must be persisted:\n{lock}"
    );
}

#[test]
fn conflicting_namespaces_fail() {
    let base = fresh_dir("conflict");
    let foo_url = setup_foo(&base);

    // `bar` requires foo from the `companyX-v` namespace.
    let bar_dir = setup_project(
        &base,
        "bar",
        &format!(
            "package:\n  name: bar\ndependencies:\n  \
             foo: {{ git: \"{foo_url}\", version: \"*\", version_prefix: \"companyX-v\" }}\n"
        ),
    );
    git(&bar_dir, &["init", "-q"]);
    git(&bar_dir, &["add", "."]);
    git(&bar_dir, &["commit", "-q", "-m", "init"]);
    git(&bar_dir, &["tag", "v1.0.0"]);
    let bar_url = format!("file://{}", bar_dir.display());

    // The app requires foo from the default `v` namespace and bar (which pulls
    // foo from `companyX-v`): two distinct prefixes, hence no resolution.
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  \
             foo: {{ git: \"{foo_url}\", version: \"1\" }}\n  \
             bar: {{ git: \"{bar_url}\", version: \"1\" }}\n"
        ),
    );

    let out = bender_update(&app);
    assert!(
        !out.status.success(),
        "update unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Reported through the normal conflict path, which lists each requirement. Without a TTY
    // there is nobody to ask, so it stays a hard error -- CI still fails loudly.
    assert!(
        stderr.contains("conflict with each other"),
        "expected a requirements conflict, got:\n{stderr}"
    );
    assert!(
        stderr.contains("prefix `companyX-v`"),
        "the conflicting namespace must be visible in the requirements:\n{stderr}"
    );
}

/// A `bar` repo depending on `foo` under the default `v` namespace, tagged `v0.1.0`.
fn setup_bar(base: &Path, foo_url: &str) -> String {
    let repo = base.join("bar");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    write(
        &repo.join("Bender.yml"),
        &format!(
            "package:\n  name: bar\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\" }}\n"
        ),
    );
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["tag", "v0.1.0"]);
    format!("file://{}", repo.display())
}

/// The namespace recorded in the lockfile is honoured on a re-resolve, rather than the run
/// silently drifting back to the default `v` namespace.
#[test]
fn lockfile_namespace_round_trips() {
    let base = fresh_dir("lock_round_trip");
    let foo_url = setup_foo(&base);
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\", version_prefix: \"companyX-v\" }}\n"
        ),
    );

    assert!(bender_update(&app).status.success());
    let first = fs::read_to_string(app.join("Bender.lock")).unwrap();
    assert!(first.contains("version_prefix: companyX-v"), "{first}");

    // Re-resolving against the existing lockfile must keep both version and namespace.
    let out = bender_update(&app);
    assert!(
        out.status.success(),
        "second update failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let second = fs::read_to_string(app.join("Bender.lock")).unwrap();
    assert!(second.contains("version: 1.0.0"), "{second}");
    assert!(second.contains("version_prefix: companyX-v"), "{second}");
}

/// The manifest, not a stale lockfile, decides the namespace: dropping `version_prefix` must move
/// the dependency back into the default `v` namespace on the next update.
#[test]
fn manifest_change_overrides_locked_namespace() {
    let base = fresh_dir("lock_stale");
    let foo_url = setup_foo(&base);
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\", version_prefix: \"companyX-v\" }}\n"
        ),
    );
    assert!(bender_update(&app).status.success());
    assert!(
        fs::read_to_string(app.join("Bender.lock"))
            .unwrap()
            .contains("version_prefix: companyX-v")
    );

    write(
        &app.join("Bender.yml"),
        &format!(
            "package:\n  name: app\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\" }}\n"
        ),
    );
    let out = bender_update(&app);
    assert!(
        out.status.success(),
        "update failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lock = fs::read_to_string(app.join("Bender.lock")).unwrap();
    // Back to the highest `v1.x` tag, with no namespace recorded.
    assert!(lock.contains("version: 1.1.0"), "{lock}");
    assert!(
        !lock.contains("version_prefix"),
        "stale namespace must not survive a manifest change:\n{lock}"
    );
}

/// The remedy the conflict error points at actually works: an `overrides` entry in `.bender.yml`
/// collapses two competing namespaces onto one.
#[test]
fn override_resolves_namespace_conflict() {
    let base = fresh_dir("override_conflict");
    let foo_url = setup_foo(&base);
    let bar_url = setup_bar(&base, &foo_url);
    let app = setup_project(
        &base,
        "app",
        &format!(
            "package:\n  name: app\ndependencies:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\", version_prefix: \"companyX-v\" }}\n  bar: {{ git: \"{bar_url}\", version: \"0.1.0\" }}\n"
        ),
    );

    // Without the override the two namespaces collide.
    let out = bender_update(&app);
    assert!(!out.status.success(), "conflicting namespaces must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("conflict with each other"), "{stderr}");

    write(
        &app.join(".bender.yml"),
        &format!(
            "overrides:\n  foo: {{ git: \"{foo_url}\", version: \"1.0.0\", version_prefix: \"companyX-v\" }}\n"
        ),
    );
    let out = bender_update(&app);
    assert!(
        out.status.success(),
        "override must resolve the conflict:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lock = fs::read_to_string(app.join("Bender.lock")).unwrap();
    assert!(lock.contains("version_prefix: companyX-v"), "{lock}");
}
