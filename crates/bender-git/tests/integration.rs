/// Integration tests for bender-git.
///
/// These tests exercise the full stack (gix local reads + subprocess writes)
/// against real git repositories created in temporary directories.
use std::sync::Arc;

use bender_git::database::GitDatabase;
use bender_git::progress::NoProgress;
use tokio::sync::Semaphore;

/// Create a small local git repository with a few commits and a tag, then
/// return its path. Used as the "remote" that the database fetches from.
fn create_local_repo(dir: &std::path::Path) -> std::path::PathBuf {
    let repo_path = dir.join("source_repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    };

    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);

    // First commit with a Bender.yml file
    std::fs::write(repo_path.join("Bender.yml"), "package:\n  name: test\n").unwrap();
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
    std::fs::write(
        repo_path.join("Bender.yml"),
        "package:\n  name: test\n  version: 0.2.0\n",
    )
    .unwrap();
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

#[tokio::test]
async fn test_database_init_and_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let source = create_local_repo(tmp.path());

    let db_path = tmp.path().join("db");
    std::fs::create_dir(&db_path).unwrap();

    let throttle = Arc::new(Semaphore::new(4));
    let db = GitDatabase::init_bare(&db_path, throttle).unwrap();

    // Add remote and fetch
    db.add_remote("origin", source.to_str().unwrap())
        .await
        .unwrap();
    db.fetch("origin", NoProgress).await.unwrap();

    // list_tags should return at least the two tags
    let tags = db.list_tags().unwrap();
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
async fn test_list_revs() {
    let tmp = tempfile::tempdir().unwrap();
    let source = create_local_repo(tmp.path());

    let db_path = tmp.path().join("db");
    std::fs::create_dir(&db_path).unwrap();

    let throttle = Arc::new(Semaphore::new(4));
    let db = GitDatabase::init_bare(&db_path, throttle).unwrap();

    db.add_remote("origin", source.to_str().unwrap())
        .await
        .unwrap();
    db.fetch("origin", NoProgress).await.unwrap();

    let revs = db.list_revs().unwrap();
    // Two commits were made
    assert_eq!(revs.len(), 2, "expected 2 commits, got: {:?}", revs);
}

#[tokio::test]
async fn test_resolve_and_cat_file() {
    let tmp = tempfile::tempdir().unwrap();
    let source = create_local_repo(tmp.path());

    let db_path = tmp.path().join("db");
    std::fs::create_dir(&db_path).unwrap();

    let throttle = Arc::new(Semaphore::new(4));
    let db = GitDatabase::init_bare(&db_path, throttle).unwrap();

    db.add_remote("origin", source.to_str().unwrap())
        .await
        .unwrap();
    db.fetch("origin", NoProgress).await.unwrap();

    // Resolve v0.1.0 tag to a commit
    let rev = db.resolve("refs/tags/v0.1.0").unwrap();
    assert_eq!(rev.to_string().len(), 40);

    // read_file should return the correct content
    let content = db
        .read_file(&rev, std::path::Path::new("Bender.yml"))
        .unwrap()
        .expect("Bender.yml not found in root");
    assert!(
        content.contains("name: test"),
        "unexpected content: {}",
        content
    );
}

#[tokio::test]
async fn test_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let source = create_local_repo(tmp.path());

    let db_path = tmp.path().join("db");
    std::fs::create_dir(&db_path).unwrap();

    let db = GitDatabase::init_bare(&db_path, Arc::new(Semaphore::new(4))).unwrap();

    db.add_remote("origin", source.to_str().unwrap())
        .await
        .unwrap();
    db.fetch("origin", NoProgress).await.unwrap();

    // Get the commit for v0.1.0
    let rev = db.resolve("refs/tags/v0.1.0").unwrap();

    // Create a bender-tmp tag so git clone --branch can reference it
    let tag = format!("bender-tmp-{}", rev.short(8));
    db.tag_commit(&tag, &rev).unwrap();

    // Clone the checkout
    let checkout_path = tmp.path().join("checkout");
    let checkout = db
        .clone_into(&checkout_path, &tag, NoProgress)
        .await
        .unwrap();

    // Verify the checkout is at the right commit
    let head = checkout.current_checkout().unwrap();
    assert_eq!(head.to_string(), rev.to_string());

    // Verify the file exists in the checkout
    assert!(checkout_path.join("Bender.yml").exists());
    let content = std::fs::read_to_string(checkout_path.join("Bender.yml")).unwrap();
    assert!(content.contains("name: test"));
}

#[tokio::test]
async fn test_remote_url() {
    let tmp = tempfile::tempdir().unwrap();
    let source = create_local_repo(tmp.path());

    let db_path = tmp.path().join("db");
    std::fs::create_dir(&db_path).unwrap();

    let throttle = Arc::new(Semaphore::new(4));
    let db = GitDatabase::init_bare(&db_path, throttle).unwrap();

    let url = source.to_str().unwrap();
    db.add_remote("origin", url).await.unwrap();

    let remote = db.remote_url("origin").unwrap();
    assert_eq!(remote.trim(), url);
}
