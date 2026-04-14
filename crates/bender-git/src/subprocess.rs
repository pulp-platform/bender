/// Internal subprocess runner — the only place `tokio::process::Command` is used
/// in this crate.
///
/// All network operations (fetch, clone, push) go through here. The semaphore
/// is acquired for every subprocess invocation so that the total number of
/// concurrent git processes is bounded. Local/read-only operations implemented
/// via `gix` do NOT go through here and therefore do not acquire the semaphore.
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock};

use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
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

    /// Build a pre-configured git command.
    ///
    /// The caller acquires the semaphore and runs it.
    fn build_cmd(&self, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new(GIT_BIN.get().expect("git binary resolved in new()"));
        cmd.args(args);
        cmd.current_dir(&self.work_dir);
        cmd.envs(envs.iter().copied());
        log::debug!("git {:?} in {:?}", args, self.work_dir);
        cmd
    }

    async fn acquire_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.throttle
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| GitError::SemaphoreClosed)
    }

    /// Run a git command and capture both stdout and stderr.
    ///
    /// Uses `.output()` which concurrently collects stdout and stderr,
    /// avoiding pipe deadlocks without a background task.
    /// If `check` is `false`, output is returned even on non-zero exit.
    pub async fn run(&self, args: &[&str], envs: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut cmd = self.build_cmd(args, envs);
        let permit = self.acquire_permit().await?;
        let output = cmd.output().await.map_err(GitError::SpawnFailed)?;
        drop(permit);

        if output.status.success() {
            Ok(output.stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            match output.status.code() {
                Some(code) => Err(GitError::CommandFailed { code, stderr }),
                None => Err(GitError::CommandKilled { stderr }),
            }
        }
    }

    /// Run a git command and discard stdout.
    pub async fn run_discard(&self, args: &[&str], envs: &[(&str, &str)]) -> Result<()> {
        self.run(args, envs).await.map(|_| ())
    }

    /// Run a git command, draining stderr line-by-line while it executes.
    ///
    /// The full raw stderr is preserved for error reporting, while each
    /// completed UTF-8 line is forwarded to `on_line`.
    pub async fn run_drain(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        on_line: impl FnMut(&str),
    ) -> Result<()> {
        let mut cmd = self.build_cmd(args, envs);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());
        let permit = self.acquire_permit().await?;
        let mut child = cmd.spawn().map_err(GitError::SpawnFailed)?;
        let mut stderr = child.stderr.take().expect("stderr was piped");
        let raw_stderr = drain_stderr_lines(&mut stderr, on_line).await;

        let status = child.wait().await.map_err(GitError::SpawnFailed)?;
        drop(permit);
        let raw_stderr = String::from_utf8_lossy(&raw_stderr).into_owned();
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

async fn drain_stderr_lines(
    stderr: &mut (impl tokio::io::AsyncRead + Unpin),
    mut on_line: impl FnMut(&str),
) -> Vec<u8> {
    let mut line_buf = Vec::new();
    let mut raw_stderr = Vec::new();

    while let Ok(byte) = stderr.read_u8().await {
        raw_stderr.push(byte);
        if byte != b'\r' && byte != b'\n' {
            line_buf.push(byte);
            continue;
        }
        if let Ok(line) = std::str::from_utf8(&line_buf) {
            on_line(line);
        }
        line_buf.clear();
    }

    raw_stderr
}
