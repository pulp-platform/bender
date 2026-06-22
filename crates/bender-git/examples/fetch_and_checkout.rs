use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bender_git::database::GitDatabase;
use bender_git::error::Result;
use bender_git::progress::GitProgress;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tempfile::{Builder, TempDir};

const REMOTE_NAME: &str = "origin";
const REPOS: &[RepoSpec] = &[
    RepoSpec {
        name: "FlooNoC",
        url: "https://github.com/pulp-platform/FlooNoC",
        target: "v0.8.0",
    },
    RepoSpec {
        name: "snitch_cluster",
        url: "https://github.com/pulp-platform/snitch_cluster",
        target: "1defbef780cd8453068dd4cb366ce8ea68dd92d7",
    },
    RepoSpec {
        name: "cheshire",
        url: "https://github.com/pulp-platform/cheshire",
        target: "v0.3.0",
    },
];

#[derive(Clone, Copy)]
struct RepoSpec {
    name: &'static str,
    url: &'static str,
    target: &'static str,
}

#[tokio::main]
async fn main() -> Result<()> {
    let base_dir = std::env::current_dir()?;
    let multi = Arc::new(MultiProgress::new());
    let fetch_style = ProgressStyle::with_template("{spinner} {msg:28} [{wide_bar}] {pos:>3}%")
        .expect("valid progress template")
        .progress_chars("=> ");
    let [repo_a, repo_b, repo_c] = REPOS else {
        panic!("expected exactly three remotes");
    };

    let (summary_a, summary_b, summary_c) = tokio::join!(
        fetch_repo(
            base_dir.clone(),
            *repo_a,
            multi.clone(),
            ProgressBars::new(multi.clone(), fetch_style.clone(), repo_a.name.to_owned()),
        ),
        fetch_repo(
            base_dir.clone(),
            *repo_b,
            multi.clone(),
            ProgressBars::new(multi.clone(), fetch_style.clone(), repo_b.name.to_owned()),
        ),
        fetch_repo(
            base_dir,
            *repo_c,
            multi.clone(),
            ProgressBars::new(multi.clone(), fetch_style, repo_c.name.to_owned()),
        ),
    );

    let mut summaries = vec![summary_a?, summary_b?, summary_c?];

    println!();
    println!("fetch summaries:");
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    for summary in summaries {
        println!(
            "{}: target={} tags={} branches={} revisions={} rev={} db={} checkout={}",
            summary.name,
            summary.checkout_target,
            summary.tags,
            summary.branches,
            summary.revisions,
            summary.checkout_rev,
            summary.db_path.display(),
            summary.checkout_path.display(),
        );
    }

    Ok(())
}

async fn fetch_repo(
    base_dir: PathBuf,
    repo: RepoSpec,
    multi: Arc<MultiProgress>,
    fetch_progress: ProgressBars,
) -> Result<FetchSummary> {
    let temp = Builder::new()
        .prefix("tmp-bender-git-db-")
        .tempdir_in(base_dir)?;
    let db_path = temp.path().join("db.git");
    fs::create_dir(&db_path)?;

    let db = GitDatabase::init_bare(&db_path)?;
    db.add_remote(REMOTE_NAME, repo.url)?;
    db.fetch(REMOTE_NAME, fetch_progress).await?;
    let revs = db.list_revs()?;
    let checkout_rev = db.resolve(repo.target)?;
    let checkout_tag = format!("bender-example-{}", checkout_rev.short(8));
    db.tag_commit(&checkout_tag, &checkout_rev)?;

    let checkout_path = temp.path().join("checkout");
    let checkout = db.clone_into(&checkout_path, &checkout_tag).await?;
    let current_checkout = checkout.current_checkout()?;
    assert_eq!(current_checkout.to_string(), checkout_rev.to_string());
    checkout.update_submodules().await?;
    multi.println(format!(
        "{}: checkout ready at {}",
        repo.name,
        checkout_path.display()
    ))?;

    Ok(FetchSummary {
        name: repo.name.to_owned(),
        checkout_target: repo.target.to_owned(),
        tags: db.list_tags()?.len(),
        branches: db.list_branches()?.len(),
        revisions: revs.len(),
        checkout_rev: current_checkout.to_string(),
        _temp: temp,
        db_path,
        checkout_path,
    })
}

struct FetchSummary {
    name: String,
    checkout_target: String,
    tags: usize,
    branches: usize,
    revisions: usize,
    checkout_rev: String,
    _temp: TempDir,
    db_path: PathBuf,
    checkout_path: PathBuf,
}

struct ProgressEntry {
    label: String,
    bar: ProgressBar,
    last_percent: Option<u8>,
}

struct ProgressBars {
    multi: Arc<MultiProgress>,
    repo_name: String,
    style: ProgressStyle,
    entry: Option<ProgressEntry>,
}

impl ProgressBars {
    fn new(multi: Arc<MultiProgress>, style: ProgressStyle, repo_name: String) -> Self {
        Self {
            multi,
            repo_name,
            style,
            entry: None,
        }
    }

    fn make_bar(&self, label: &str) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new(100));
        bar.set_style(self.style.clone());
        bar.set_message(label.to_owned());
        bar.enable_steady_tick(Duration::from_millis(100));
        bar
    }
}

impl GitProgress for ProgressBars {
    fn started(&mut self, label: &str) {
        let full_label = format!("{}: fetch {}", self.repo_name, label);
        let bar = self.make_bar(&full_label);
        self.multi
            .println(format!("{}: started fetch ({label})", self.repo_name))
            .ok();
        self.entry = Some(ProgressEntry {
            label: full_label,
            bar,
            last_percent: None,
        });
    }

    fn progress(&mut self, percent: u8) {
        if let Some(entry) = &mut self.entry
            && entry.last_percent != Some(percent)
        {
            entry.last_percent = Some(percent);
            entry.bar.set_position(u64::from(percent));
        }
    }

    fn finished(&mut self) {
        if let Some(entry) = self.entry.take() {
            entry.bar.set_position(100);
            entry
                .bar
                .finish_with_message(format!("{}: finished", entry.label));
        }
    }
}
