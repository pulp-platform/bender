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

use tokio::process::{ChildStderr, Command};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{GitError, Result};

static GIT_BIN: OnceLock<PathBuf> = OnceLock::new();

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
        .set(PathBuf::from(path.as_ref()))
        .map_err(|_| GitError::GitBinAlreadySet)
}

pub(crate) struct SubprocessRunner {
    pub work_dir: PathBuf,
    pub throttle: Arc<Semaphore>,
}

impl SubprocessRunner {
    pub fn new(work_dir: PathBuf, throttle: Arc<Semaphore>) -> Result<Self> {
        if GIT_BIN.get().is_none() {
            let path = which::which("git").map_err(|e| GitError::GitBinNotFound(e.to_string()))?;
            let _ = GIT_BIN.set(path);
        }
        Ok(Self { work_dir, throttle })
    }

    /// Begin configuring a git command to run in this runner's working directory.
    pub fn cmd<'a>(&'a self, args: &'a [&'a str]) -> GitCommand<'a> {
        GitCommand {
            runner: self,
            args,
            envs: &[],
        }
    }

    /// Build a pre-configured git command.
    ///
    /// The caller acquires the semaphore and runs it.
    fn build_cmd(&self, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new(GIT_BIN.get().expect("git binary resolved in new()"));
        cmd.args(args);
        cmd.current_dir(&self.work_dir);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        log::debug!("git {:?} in {:?}", args, self.work_dir);
        cmd
    }
}

pub(crate) struct GitCommand<'a> {
    runner: &'a SubprocessRunner,
    args: &'a [&'a str],
    envs: &'a [(&'a str, &'a str)],
}

impl<'a> GitCommand<'a> {
    /// Add extra environment variables for this command.
    pub fn envs(mut self, envs: &'a [(&'a str, &'a str)]) -> Self {
        self.envs = envs;
        self
    }

    /// Run a git command and capture stdout.
    ///
    /// Uses `.output()` which concurrently collects stdout and stderr,
    /// avoiding pipe deadlocks without a background task.
    /// If `check` is `false`, stdout is returned even on non-zero exit.
    pub async fn run_stdout(self, check: bool) -> Result<Vec<u8>> {
        let mut cmd = self.runner.build_cmd(self.args, self.envs);
        let permit = self
            .runner
            .throttle
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| GitError::SemaphoreClosed)?;
        let output = cmd.output().await.map_err(GitError::SpawnFailed)?;
        drop(permit);

        if output.status.success() || !check {
            Ok(output.stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            match output.status.code() {
                Some(code) => Err(GitError::CommandFailed { code, stderr }),
                None => Err(GitError::CommandKilled { stderr }),
            }
        }
    }

    /// Run a git command and interpret stdout as UTF-8.
    pub async fn run_string(self, check: bool) -> Result<String> {
        let context = format!("git {}", self.args.join(" "));
        let bytes = self.run_stdout(check).await?;
        String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 { context })
    }

    /// Run a git command and discard stdout.
    pub async fn run_discard(self) -> Result<()> {
        self.run_stdout(true).await.map(|_| ())
    }

    /// Start a git command with piped stderr for streaming progress parsing.
    ///
    /// Stdout is discarded. Stderr is returned separately so callers can
    /// drain and parse it before calling [`GitChild::finish`].
    ///
    /// The throttle semaphore is held until [`GitChild::finish`] is called.
    pub async fn start_stderr(self) -> Result<GitChild> {
        let mut cmd = self.runner.build_cmd(self.args, self.envs);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());
        let permit = self
            .runner
            .throttle
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| GitError::SemaphoreClosed)?;
        let mut child = cmd.spawn().map_err(GitError::SpawnFailed)?;
        let stderr = child.stderr.take().expect("stderr was piped");
        Ok(GitChild {
            child,
            stderr,
            _permit: permit,
        })
    }
}

/// A running git subprocess.
///
/// Created by [`SubprocessRunner::spawn`]. Callers borrow [`stderr`](GitChild::stderr)
/// to stream and parse it, then call [`finish`](GitChild::finish) with the
/// collected raw stderr to wait for the process and check the exit code.
pub(crate) struct GitChild {
    child: tokio::process::Child,
    pub stderr: ChildStderr,
    _permit: OwnedSemaphorePermit,
}

impl GitChild {
    /// Wait for the process to exit and check its status.
    ///
    /// `raw_stderr` is the already-drained stderr content used in error messages.
    pub async fn finish(mut self, raw_stderr: String) -> Result<()> {
        let status = self.child.wait().await.map_err(GitError::SpawnFailed)?;
        drop(self._permit);
        if status.success() {
            Ok(())
        } else {
            match status.code() {
                Some(code) => Err(GitError::CommandFailed {
                    code,
                    stderr: raw_stderr,
                }),
                None => Err(GitError::CommandKilled { stderr: raw_stderr }),
            }
        }
    }
}
