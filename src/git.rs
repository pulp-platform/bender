// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A git repository and context for command execution.

#![deny(missing_docs)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use futures::TryFutureExt;
use miette::{Context as _, Diagnostic, IntoDiagnostic as _};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;
use walkdir::WalkDir;

use crate::progress::{ProgressHandler, monitor_stderr};

use crate::debugln;
use crate::err;
use crate::{Error, Result};

#[derive(Debug, Error, Diagnostic)]
enum GitSpawnError {
    #[error("Failed to spawn git command `{}` in directory {:?}.", .0, .1)]
    Spawn(String, PathBuf, #[source] std::io::Error),
    #[error("Failed to spawn git command `{}` in directory {:?}.", .0, .1)]
    #[diagnostic(help("Please consider increasing your `ulimit -n`."))]
    TooManyOpenFiles(String, PathBuf, #[source] std::io::Error),
}

fn format_command(cmd: &Command) -> String {
    let std_cmd = cmd.as_std();
    let program = std_cmd.get_program().to_string_lossy();
    let args = std_cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// A git repository.
///
/// This struct is used to interact with git repositories on disk. It makes
/// heavy use of futures to execute the different tasks.
#[derive(Clone)]
pub struct Git<'ctx> {
    /// The path to the repository.
    pub path: &'ctx Path,
    /// The session within which commands will be executed.
    pub git: &'ctx String,
    /// Reference to the throttle object.
    pub throttle: Arc<Semaphore>,
}

impl<'ctx> Git<'ctx> {
    /// Create a new git context.
    pub fn new(path: &'ctx Path, git: &'ctx String, throttle: Arc<Semaphore>) -> Git<'ctx> {
        Git {
            path,
            git,
            throttle,
        }
    }

    /// Create a new git command.
    ///
    /// The command will have the form `git <subcommand>` and be pre-configured
    /// to operate in the repository's path.
    pub fn command(self, subcommand: &str) -> Command {
        let mut cmd = Command::new(self.git);
        cmd.arg(subcommand);
        cmd.current_dir(self.path);
        cmd
    }

    /// Schedule a command for execution.
    ///
    /// Configures the command's stdout and stderr to be captured and wires up
    /// appropriate error handling. In case the command fails, the exact
    /// arguments to the command are emitted together with the captured output.
    /// The command is spawned asynchronously on the session's reactor core.
    /// Returns a future that will resolve to the command's stdout.
    ///
    /// If `check` is false, the stdout will be returned regardless of the
    /// command's exit code.
    pub async fn spawn(
        self,
        mut cmd: Command,
        check: bool,
        pb: Option<ProgressHandler>,
    ) -> Result<String> {
        // Acquire the throttle semaphore
        let permit = self
            .throttle
            .clone()
            .acquire_owned()
            .await
            .into_diagnostic()?;

        // Configure pipes for streaming
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Disable interactive terminal prompts.
        // This ensures git fails immediately with a specific error message
        // instead of hanging indefinitely if auth is missing.
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        let command = format_command(&cmd);

        // Spawn the child process
        let mut child = cmd.spawn().map_err(|cause| {
            if cause
                .to_string()
                .to_lowercase()
                .contains("too many open files")
            {
                Error::from(GitSpawnError::TooManyOpenFiles(
                    command.clone(),
                    self.path.to_path_buf(),
                    cause,
                ))
            } else {
                Error::from(GitSpawnError::Spawn(
                    command.clone(),
                    self.path.to_path_buf(),
                    cause,
                ))
            }
        })?;

        debugln!("git: {:?} in {:?}", cmd, self.path);

        // Setup Streaming for Stderr (Progress + Error Collection)
        // We need to capture stderr in case the command fails, so we collect it while parsing.
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| err!("Failed to capture git stderr",))?;

        // Spawn a background task to handle stderr so it doesn't block
        let stderr_handle = tokio::spawn(async move {
            // We pass the handler clone into the async task
            monitor_stderr(stderr, pb).await
        });

        // Read Stdout (for the success return value)
        let mut stdout_buffer = Vec::new();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| err!("Failed to capture git stdout.",))?;
        stdout
            .read_to_end(&mut stdout_buffer)
            .await
            .into_diagnostic()?;

        // Wait for child process to finish
        let status = child.wait().await.into_diagnostic()?;

        // Join the stderr task to get the error log
        let collected_stderr = stderr_handle.await.into_diagnostic()?;

        // We can release the throttle here since we're done with the process
        drop(permit);

        // Process the output based on success and check flag
        if status.success() || !check {
            String::from_utf8(stdout_buffer)
                .into_diagnostic()
                .wrap_err("Output of git command is not valid UTF-8.")
        } else {
            let exit = match status.code() {
                Some(code) => format!("exit code {}", code),
                None => String::from("unknown exit status"),
            };
            Err(err!(
                help = format!("git failed with stderr output:\n{}", collected_stderr),
                "Git command `{}` failed in directory {:?} with {}.",
                command,
                self.path,
                exit
            ))
        }
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is a convenience function that creates a command, passes it to the
    /// closure `f` for configuration, then passes it to the `spawn` function
    /// and returns the future.
    pub async fn spawn_with<F>(self, f: F, pb: Option<ProgressHandler>) -> Result<String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(self.git);
        cmd.current_dir(self.path);
        f(&mut cmd);
        self.spawn(cmd, true, pb).await
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is the same as `spawn_with()`, but returns the stdout regardless of
    /// whether the command failed or not.
    pub async fn spawn_unchecked_with<F>(self, f: F, pb: Option<ProgressHandler>) -> Result<String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(self.git);
        cmd.current_dir(self.path);
        f(&mut cmd);
        self.spawn(cmd, false, pb).await
    }

    /// Assemble a command and execute it interactively.
    ///
    /// This is the same as `spawn_with()`, but inherits stdin, stdout, and stderr
    /// from the caller.
    pub async fn spawn_interactive_with<F>(self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(self.git);
        cmd.current_dir(self.path);
        f(&mut cmd);
        cmd.spawn()
            .into_diagnostic()?
            .wait()
            .await
            .into_diagnostic()?;
        Ok(())
    }

    /// Check if the repository uses LFS.
    pub async fn uses_lfs(self) -> Result<bool> {
        let output = self
            .spawn_with(|c| c.arg("lfs").arg("ls-files"), None)
            .await?;
        Ok(!output.trim().is_empty())
    }

    /// Check if the repository has LFS attributes configured.
    pub async fn uses_lfs_attributes(self) -> Result<bool> {
        // We use tokio::task::spawn_blocking because walkdir is synchronous
        // and file I/O should not block the async runtime.
        let path = self.path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            Ok(WalkDir::new(&path).into_iter().flatten().any(|entry| {
                if entry.file_type().is_file() && entry.file_name() == ".gitattributes" {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        content.contains("filter=lfs")
                    } else {
                        false
                    }
                } else {
                    false
                }
            }))
        })
        .await
        .into_diagnostic()?
    }

    /// Fetch the tags and refs of a remote.
    pub async fn fetch(self, remote: &str, pb: Option<ProgressHandler>) -> Result<()> {
        self.clone()
            .spawn_with(
                |c| {
                    c.arg("fetch")
                        .arg("--tags")
                        .arg("--prune")
                        .arg(remote)
                        .arg("--progress")
                },
                pb,
            )
            .await
            .map(|_| ())
    }

    /// Fetch the specified ref of a remote.
    pub async fn fetch_ref(
        self,
        remote: &str,
        reference: &str,
        pb: Option<ProgressHandler>,
    ) -> Result<()> {
        self.spawn_with(
            |c| c.arg("fetch").arg(remote).arg(reference).arg("--progress"),
            pb,
        )
        .await
        .map(|_| ())
    }

    /// Stage all local changes.
    pub async fn add_all(self) -> Result<()> {
        self.spawn_with(|c| c.arg("add").arg("--all"), None)
            .await
            .map(|_| ())
    }

    /// Commit the staged changes.
    ///
    /// If message is None, this starts an interactive commit session.
    pub async fn commit(self, message: Option<&String>) -> Result<()> {
        match message {
            Some(msg) => self
                .spawn_with(
                    |c| {
                        c.arg("-c")
                            .arg("commit.gpgsign=false")
                            .arg("commit")
                            .arg("-m")
                            .arg(msg)
                    },
                    None,
                )
                .await
                .map(|_| ()),

            None => self
                .spawn_interactive_with(|c| c.arg("-c").arg("commit.gpgsign=false").arg("commit"))
                .await
                .map(|_| ()),
        }
    }

    /// List all refs and their hashes.
    pub async fn list_refs(self) -> Result<Vec<(String, String)>> {
        self.spawn_unchecked_with(|c| c.arg("show-ref").arg("--dereference"), None)
            .and_then(|raw| async move {
                let mut all_revs = raw
                    .lines()
                    .map(|line| {
                        // Parse the line
                        let mut fields = line.split_whitespace().map(String::from);
                        let rev = fields.next().unwrap();
                        let rf = fields.next().unwrap();
                        (rev, rf)
                    })
                    .collect::<Vec<_>>();
                // Ensure only commit hashes are returned by using dereferenced values in case they exist
                let deref_revs = all_revs
                    .clone()
                    .into_iter()
                    .filter(|tup| tup.1.ends_with("^{}"));
                for item in deref_revs {
                    let index = all_revs
                        .iter()
                        .position(|x| *x.1 == item.1.replace("^{}", ""))
                        .unwrap();
                    all_revs.remove(index);
                    let index = all_revs.iter().position(|x| *x.1 == item.1).unwrap();
                    all_revs.remove(index);
                    all_revs.push((item.0, item.1.replace("^{}", "")));
                }
                // Return future
                Ok(all_revs)
            })
            .await
    }

    /// List all revisions.
    pub async fn list_revs(self) -> Result<Vec<String>> {
        self.spawn_with(|c| c.arg("rev-list").arg("--all").arg("--date-order"), None)
            .await
            .map(|raw| raw.lines().map(String::from).collect())
    }

    /// Determine the currently checked out revision.
    pub async fn current_checkout(self) -> Result<Option<String>> {
        self.spawn_with(
            |c| c.arg("rev-parse").arg("--revs-only").arg("HEAD^{commit}"),
            None,
        )
        .await
        .map(|raw| raw.lines().take(1).map(String::from).next())
    }

    /// Determine the url of a remote.
    pub async fn remote_url(self, remote: &str) -> Result<String> {
        self.spawn_with(|c| c.arg("remote").arg("get-url").arg(remote), None)
            .await
            .map(|raw| raw.lines().take(1).map(String::from).next().unwrap())
    }

    /// List files in the directory.
    ///
    /// Calls `git ls-tree` under the hood.
    pub async fn list_files<R: AsRef<OsStr>, P: AsRef<OsStr>>(
        self,
        rev: R,
        path: Option<P>,
    ) -> Result<Vec<TreeEntry>> {
        self.spawn_with(
            |c| {
                c.arg("ls-tree").arg(rev);
                if let Some(p) = path {
                    c.arg(p);
                }
                c
            },
            None,
        )
        .await
        .map(|raw| raw.lines().map(TreeEntry::parse).collect())
    }

    /// Read the content of a file.
    pub async fn cat_file<O: AsRef<OsStr>>(self, hash: O) -> Result<String> {
        self.spawn_with(|c| c.arg("cat-file").arg("blob").arg(hash), None)
            .await
    }
}

/// A single entry in a git tree.
///
/// The `list_files` command returns a vector of these.
pub struct TreeEntry {
    /// The name of the file.
    pub name: String,
    /// The hash of the entry.
    pub hash: String,
    /// The kind of the entry. Usually `blob` or `tree`.
    pub kind: String,
    /// The mode of the entry, i.e. its permissions.
    pub mode: String,
}

impl TreeEntry {
    /// Parse a single line of output of `git ls-tree`.
    pub fn parse(input: &str) -> TreeEntry {
        let tab = input.find('\t').unwrap();
        let (metadata, name) = input.split_at(tab);
        let mut iter = metadata.split(' ');
        let mode = iter.next().unwrap();
        let kind = iter.next().unwrap();
        let hash = iter.next().unwrap();
        TreeEntry {
            name: name.into(),
            hash: hash.into(),
            kind: kind.into(),
            mode: mode.into(),
        }
    }
}
