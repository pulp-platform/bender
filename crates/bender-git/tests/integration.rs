/// Integration tests for bender-git.
///
/// These tests exercise the full stack (gix local reads + subprocess writes)
/// against real git repositories created in temporary directories.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bender_git::database::GitDatabase;
use bender_git::progress::NoProgress;
use tokio::sync::Semaphore;

const ORIGIN: &str = "origin";
const BENDER_FILE: &str = "Bender.yml";

/// Create a small local git repository with a few commits and a tag, then
/// return its path. Used as the "remote" that the database fetches from.
fn create_local_repo(dir: &Path) -> PathBuf {
    let repo_path = dir.join("source_repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let run = |args: &[&str]| run_git(&repo_path, args);

    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);

    // First commit with a Bender.yml file
    write_bender_manifest(&repo_path, None);
    std::fs::write(repo_path.join("README.md"), "# Test repo\n").unwrap();
    run(&["add", "."]);
    run(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "Initial commit",
    ]);
    run(&["tag", "v0.1.0"]);

    // Second commit
    write_bender_manifest(&repo_path, Some("0.2.0"));
    run(&["add", "."]);
    run(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "Bump to v0.2.0",
    ]);
    run(&["tag", "v0.2.0"]);

    repo_path
}

fn write_bender_manifest(repo_path: &Path, version: Option<&str>) {
    let manifest = match version {
        Some(version) => format!("package:\n  name: test\n  version: {version}\n"),
        None => "package:\n  name: test\n".to_owned(),
    };
    std::fs::write(repo_path.join(BENDER_FILE), manifest).unwrap();
}

fn run_git(repo_path: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

fn add_repo_commit(repo_path: &Path, version: &str, tag: &str) {
    write_bender_manifest(repo_path, Some(version));
    run_git(repo_path, &["add", "."]);
    run_git(
        repo_path,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            &format!("Bump to {tag}"),
        ],
    );
    run_git(repo_path, &["tag", tag]);
}

struct TestContext {
    _tmp: tempfile::TempDir,
    source: PathBuf,
    checkout_path: PathBuf,
    db: GitDatabase,
}

impl TestContext {
    async fn init() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let source = create_local_repo(tmp.path());

        let db_path = tmp.path().join("db");
        std::fs::create_dir(&db_path).unwrap();

        let db = GitDatabase::init_bare(&db_path, Arc::new(Semaphore::new(4))).unwrap();
        db.add_remote(ORIGIN, source.to_str().unwrap())
            .await
            .unwrap();
        db.fetch(ORIGIN, NoProgress).await.unwrap();

        Self {
            checkout_path: tmp.path().join("checkout"),
            _tmp: tmp,
            source,
            db,
        }
    }
}

#[tokio::test]
// Verifies that a bare database can connect to a local remote, fetch objects,
// and expose fetched tags through the read API.
async fn test_database_init_and_fetch() {
    let ctx = TestContext::init().await;

    // list_tags should return at least the two tags
    let tags = ctx.db.list_tags().unwrap();
    assert!(
        tags.iter().any(|(name, _)| name.contains("v0.1.0")),
        "should have v0.1.0 tag, got: {:?}",
        tags.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    assert!(
        tags.iter().any(|(name, _)| name.contains("v0.2.0")),
        "should have v0.2.0 tag"
    );
}

#[tokio::test]
// Verifies that revision enumeration sees the full fetched commit history from
// the remote repository.
async fn test_list_revs() {
    let ctx = TestContext::init().await;

    let revs = ctx.db.list_revs().unwrap();
    // Two commits were made
    assert_eq!(revs.len(), 2, "expected 2 commits, got: {:?}", revs);
}

#[tokio::test]
// Verifies that a fetched tag can be resolved to an object ID and used to read
// file contents directly from the object database.
async fn test_resolve_and_cat_file() {
    let ctx = TestContext::init().await;

    // Resolve v0.1.0 tag to a commit
    let rev = ctx.db.resolve("refs/tags/v0.1.0").unwrap();
    assert_eq!(rev.to_string().len(), 40);

    // read_file should return the correct content
    let content = ctx
        .db
        .read_file(&rev, Path::new(BENDER_FILE))
        .unwrap()
        .expect("Bender manifest not found in root");
    assert!(
        content.contains("name: test"),
        "unexpected content: {}",
        content
    );
}

#[tokio::test]
// Verifies that cloning a checkout from a tagged commit materializes the
// expected working tree and records the checked-out revision correctly.
async fn test_checkout() {
    let ctx = TestContext::init().await;

    // Get the commit for v0.1.0
    let rev = ctx.db.resolve("refs/tags/v0.1.0").unwrap();

    // Create a bender-tmp tag so git clone --branch can reference it
    let tag = format!("bender-tmp-{}", rev.short(8));
    ctx.db.tag_commit(&tag, &rev).unwrap();

    // Clone the checkout
    let checkout = ctx.db.clone_into(&ctx.checkout_path, &tag).await.unwrap();

    // Verify the checkout is at the right commit
    let head = checkout.current_checkout().unwrap();
    assert_eq!(head.to_string(), rev.to_string());

    // Verify the file exists in the checkout
    assert!(ctx.checkout_path.join(BENDER_FILE).exists());
    let content = std::fs::read_to_string(ctx.checkout_path.join(BENDER_FILE)).unwrap();
    assert!(content.contains("name: test"));
}

#[tokio::test]
// Verifies that an existing checkout can switch to a newly fetched commit
// using objects from the shared database even if the checkout's own remote is
// unusable, proving `switch` does not fetch on its own.
async fn test_checkout_switch_uses_objects_from_shared_database() {
    let ctx = TestContext::init().await;

    let rev_v1 = ctx.db.resolve("refs/tags/v0.1.0").unwrap();
    let tag_v1 = format!("bender-tmp-{}", rev_v1.short(8));
    ctx.db.tag_commit(&tag_v1, &rev_v1).unwrap();

    let checkout = ctx
        .db
        .clone_into(&ctx.checkout_path, &tag_v1)
        .await
        .unwrap();
    assert_eq!(
        checkout.current_checkout().unwrap().to_string(),
        rev_v1.to_string()
    );

    add_repo_commit(&ctx.source, "0.3.0", "v0.3.0");
    ctx.db.fetch(ORIGIN, NoProgress).await.unwrap();

    // Break the checkout's remote after the database fetch. If `switch()`
    // attempted its own fetch, this would fail.
    let missing_remote = ctx.checkout_path.join("missing-database");
    run_git(
        &ctx.checkout_path,
        &[
            "remote",
            "set-url",
            ORIGIN,
            missing_remote.to_str().unwrap(),
        ],
    );

    let rev_v3 = ctx.db.resolve("refs/tags/v0.3.0").unwrap();
    checkout.switch(&rev_v3).await.unwrap();

    assert_eq!(
        checkout.current_checkout().unwrap().to_string(),
        rev_v3.to_string()
    );
    let content = std::fs::read_to_string(ctx.checkout_path.join(BENDER_FILE)).unwrap();
    assert!(content.contains("version: 0.3.0"));
}

#[tokio::test]
// Verifies that the configured remote URL is preserved and can be queried
// without performing additional network operations.
async fn test_remote_url() {
    let ctx = TestContext::init().await;

    let url = ctx.source.to_str().unwrap();
    let remote = ctx.db.remote_url(ORIGIN).unwrap();
    assert_eq!(remote.trim(), url);
}
