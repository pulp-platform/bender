// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A git repository and context for command execution.

#![deny(missing_docs)]

use std::ffi::OsStr;
use std::path::Path;

use futures::TryFutureExt;
use tokio::process::Command;

use crate::error::*;

/// A git repository.
///
/// This struct is used to interact with git repositories on disk. It makes
/// heavy use of futures to execute the different tasks.
#[derive(Copy, Clone)]
pub struct Git<'ctx> {
    /// The path to the repository.
    pub path: &'ctx Path,
    /// The session within which commands will be executed.
    pub git: &'ctx String,
}

impl<'git, 'ctx> Git<'ctx> {
    /// Create a new git context.
    pub fn new(path: &'ctx Path, git: &'ctx String) -> Git<'ctx> {
        Git { path, git }
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
    #[allow(clippy::format_push_string)]
    pub async fn spawn(self, mut cmd: Command, check: bool) -> Result<String> {
        let output = cmd.output().map_err(|cause| {
            if cause
                .to_string()
                .to_lowercase()
                .contains("too many open files")
            {
                println!(
                    "Please consider increasing your `ulimit -n`, e.g. by running `ulimit -n 4096`"
                );
                println!("This is a known issue (#52).");
                Error::chain("Failed to spawn child process.", cause)
            } else {
                Error::chain("Failed to spawn child process.", cause)
            }
        });
        let result = output.and_then(|output| async move {
            debugln!("git: {:?} in {:?}", cmd, self.path);
            if output.status.success() || !check {
                String::from_utf8(output.stdout).map_err(|cause| {
                    Error::chain(
                        format!(
                            "Output of git command ({:?}) in directory {:?} is not valid UTF-8.",
                            cmd, self.path
                        ),
                        cause,
                    )
                })
            } else {
                let mut msg = format!("Git command ({:?}) in directory {:?}", cmd, self.path);
                match output.status.code() {
                    Some(code) => msg.push_str(&format!(" failed with exit code {code}")),
                    None => msg.push_str(" failed"),
                };
                match String::from_utf8(output.stderr) {
                    Ok(txt) => {
                        msg.push_str(":\n\n");
                        msg.push_str(&txt);
                    }
                    Err(err) => msg.push_str(&format!(". Stderr is not valid UTF-8, {err}.")),
                };
                Err(Error::new(msg))
            }
        });
        result.await
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is a convenience function that creates a command, passes it to the
    /// closure `f` for configuration, then passes it to the `spawn` function
    /// and returns the future.
    pub async fn spawn_with<F>(self, f: F) -> Result<String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(self.git);
        cmd.current_dir(self.path);
        f(&mut cmd);
        self.spawn(cmd, true).await
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is the same as `spawn_with()`, but returns the stdout regardless of
    /// whether the command failed or not.
    pub async fn spawn_unchecked_with<F>(self, f: F) -> Result<String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(self.git);
        cmd.current_dir(self.path);
        f(&mut cmd);
        self.spawn(cmd, false).await
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
        cmd.spawn()?.wait().await?;
        Ok(())
    }

    /// Fetch the tags and refs of a remote.
    pub async fn fetch(self, remote: &str) -> Result<()> {
        let r1 = String::from(remote);
        let r2 = String::from(remote);
        self.spawn_with(|c| c.arg("fetch").arg("--prune").arg(r1))
            .and_then(|_| self.spawn_with(|c| c.arg("fetch").arg("--tags").arg("--prune").arg(r2)))
            .await
            .map(|_| ())
    }

    /// Stage all local changes.
    pub async fn add_all(self) -> Result<()> {
        self.spawn_with(|c| c.arg("add").arg("--all"))
            .await
            .map(|_| ())
    }

    /// Commit the staged changes.
    ///
    /// If message is None, this starts an interactive commit session.
    pub async fn commit(self, message: Option<&String>) -> Result<()> {
        match message {
            Some(msg) => self
                .spawn_with(|c| {
                    c.arg("-c")
                        .arg("commit.gpgsign=false")
                        .arg("commit")
                        .arg("-m")
                        .arg(msg)
                })
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
        self.spawn_unchecked_with(|c| c.arg("show-ref").arg("--dereference"))
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
        self.spawn_with(|c| c.arg("rev-list").arg("--all").arg("--date-order"))
            .await
            .map(|raw| raw.lines().map(String::from).collect())
    }

    /// Determine the currently checked out revision.
    pub async fn current_checkout(self) -> Result<Option<String>> {
        self.spawn_with(|c| c.arg("rev-parse").arg("--revs-only").arg("HEAD^{commit}"))
            .await
            .map(|raw| raw.lines().take(1).map(String::from).next())
    }

    /// List files in the directory.
    ///
    /// Calls `git ls-tree` under the hood.
    pub async fn list_files<R: AsRef<OsStr>, P: AsRef<OsStr>>(
        self,
        rev: R,
        path: Option<P>,
    ) -> Result<Vec<TreeEntry>> {
        self.spawn_with(|c| {
            c.arg("ls-tree").arg(rev);
            if let Some(p) = path {
                c.arg(p);
            }
            c
        })
        .await
        .map(|raw| raw.lines().map(TreeEntry::parse).collect())
    }

    /// Read the content of a file.
    pub async fn cat_file<O: AsRef<OsStr>>(self, hash: O) -> Result<String> {
        self.spawn_with(|c| c.arg("cat-file").arg("blob").arg(hash))
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
