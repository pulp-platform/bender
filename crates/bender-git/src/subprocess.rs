/// Internal subprocess runner — the only place `tokio::process::Command` is used
/// in this crate.
///
/// All network operations (fetch, clone, push) go through here. The semaphore
/// is acquired for every subprocess invocation so that the total number of
/// concurrent git processes is bounded. Local/read-only operations implemented
/// via `gix` do NOT go through here and therefore do not acquire the semaphore.
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, OnceLock};

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::error::{GitError, Result};

static GIT_BIN: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();

/// Lazily-resolved path to the `git-lfs` binary.
///
/// Resolved once on first use by running `git-lfs`. If `git-lfs` is
/// not installed this is `Err` and `lfs_pull` will return an error instead of
/// silently leaving raw LFS pointer files in the working tree.
pub(crate) static GIT_LFS: LazyLock<std::result::Result<PathBuf, String>> =
    LazyLock::new(|| which::which("git-lfs").map_err(|e| e.to_string()));

/// Override the `git` binary used for all subprocess operations.
///
/// If not called, the binary is auto-discovered via `which git` on first use.
/// Must be called before the first git subprocess operation to take effect.
/// Returns an error if the binary has already been set or auto-discovered.
pub fn set_git_bin(path: impl AsRef<OsStr>) -> Result<()> {
    GIT_BIN
        .set(Ok(PathBuf::from(path.as_ref())))
        .map_err(|_| GitError::GitBinAlreadySet)
}

pub(crate) struct SubprocessRunner {
    pub work_dir: PathBuf,
    pub throttle: Arc<Semaphore>,
}

impl SubprocessRunner {
    pub fn new(work_dir: PathBuf, throttle: Arc<Semaphore>) -> Self {
        Self { work_dir, throttle }
    }

    /// Run a git command, capturing stdout.
    ///
    /// - Acquires the throttle semaphore before spawning.
    /// - Sets `GIT_TERMINAL_PROMPT=0` to prevent credential prompts from
    ///   blocking indefinitely.
    /// - Stderr is collected in a background task and included in the error
    ///   message if the command fails.
    /// - If `check` is `false`, stdout is returned even on non-zero exit.
    pub async fn run(&self, args: &[&str], check: bool) -> Result<Vec<u8>> {
        self.run_with_env(args, &[], check).await
    }

    pub async fn run_with_env(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        check: bool,
    ) -> Result<Vec<u8>> {
        let permit = self
            .throttle
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| GitError::SemaphoreClosed)?;

        let git = GIT_BIN
            .get_or_init(|| which::which("git").map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| GitError::Gix(format!("git binary not found: {e}")))?;
        let mut cmd = Command::new(git);
        cmd.args(args);
        cmd.current_dir(&self.work_dir);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        log::debug!("git {:?} in {:?}", args, self.work_dir);

        let mut child = cmd.spawn().map_err(GitError::SpawnFailed)?;

        // Collect stderr in a background task so that a full stderr pipe does
        // not deadlock stdout reads.
        let stderr = child.stderr.take().expect("stderr was piped");
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut stderr = stderr;
            let _ = stderr.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        // Read stdout.
        let mut stdout_buf = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout
                .read_to_end(&mut stdout_buf)
                .await
                .map_err(GitError::SpawnFailed)?;
        }

        let status = child.wait().await.map_err(GitError::SpawnFailed)?;
        let collected_stderr = stderr_task.await.unwrap_or_default();

        // Release semaphore permit once the process has exited.
        drop(permit);

        if status.success() || !check {
            Ok(stdout_buf)
        } else {
            match status.code() {
                Some(code) => Err(GitError::CommandFailed {
                    code,
                    stderr: collected_stderr,
                }),
                None => Err(GitError::CommandKilled {
                    stderr: collected_stderr,
                }),
            }
        }
    }

    /// Run a git command and interpret stdout as UTF-8.
    pub async fn run_str(&self, args: &[&str], check: bool) -> Result<String> {
        let bytes = self.run(args, check).await?;
        String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
            context: format!("git {}", args.join(" ")),
        })
    }

    /// Run a git command and discard stdout.
    pub async fn run_discard(&self, args: &[&str]) -> Result<()> {
        self.run(args, true).await.map(|_| ())
    }

    /// Run a git command with extra environment variables and discard stdout.
    pub async fn run_discard_with_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Result<()> {
        self.run_with_env(args, envs, true).await.map(|_| ())
    }
}
