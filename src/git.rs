// Copyright (c) 2017-2018 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

//! A git repository and context for command execution.

#![deny(missing_docs)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use futures::future;
use futures::Future;
use tokio_process::CommandExt;

use crate::error::*;
use crate::sess::SessionIo;

/// A git repository.
///
/// This struct is used to interact with git repositories on disk. It makes
/// heavy use of futures to execute the different tasks.
#[derive(Copy, Clone)]
pub struct Git<'io, 'sess: 'io, 'ctx: 'sess> {
    /// The path to the repository.
    pub path: &'ctx Path,
    /// The session within which commands will be executed.
    pub sess: &'io SessionIo<'sess, 'ctx>,
}

impl<'git, 'io, 'sess: 'io, 'ctx: 'sess> Git<'io, 'sess, 'ctx> {
    /// Create a new git context.
    pub fn new(path: &'ctx Path, sess: &'io SessionIo<'sess, 'ctx>) -> Git<'io, 'sess, 'ctx> {
        Git {
            path: path,
            sess: sess,
        }
    }

    /// Create a new git command.
    ///
    /// The command will have the form `git <subcommand>` and be pre-configured
    /// to operate in the repository's path.
    pub fn command(self, subcommand: &str) -> Command {
        let mut cmd = Command::new(&self.sess.sess.config.git);
        cmd.arg(subcommand);
        cmd.current_dir(&self.path);
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
    pub fn spawn(self, mut cmd: Command, check: bool) -> GitFuture<'io, String> {
        let output = cmd.output_async(&self.sess.handle).map_err(|cause| {
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
        let result = output.and_then(move |output| {
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
                    Some(code) => msg.push_str(&format!(" failed with exit code {}", code)),
                    None => msg.push_str(" failed"),
                };
                match String::from_utf8(output.stderr) {
                    Ok(txt) => {
                        msg.push_str(":\n\n");
                        msg.push_str(&txt);
                    }
                    Err(err) => msg.push_str(&format!(". Stderr is not valid UTF-8, {}.", err)),
                };
                Err(Error::new(msg))
            }
        });
        Box::new(result)
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is a convenience function that creates a command, passses it to the
    /// closure `f` for configuration, then passes it to the `spawn` function
    /// and returns the future.
    pub fn spawn_with<F>(self, f: F) -> GitFuture<'io, String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(&self.sess.sess.config.git);
        cmd.current_dir(&self.path);
        f(&mut cmd);
        self.spawn(cmd, true)
    }

    /// Assemble a command and schedule it for execution.
    ///
    /// This is the same as `spawn_with()`, but returns the stdout regardless of
    /// whether the command failed or not.
    pub fn spawn_unchecked_with<F>(self, f: F) -> GitFuture<'io, String>
    where
        F: FnOnce(&mut Command) -> &mut Command,
    {
        let mut cmd = Command::new(&self.sess.sess.config.git);
        cmd.current_dir(&self.path);
        f(&mut cmd);
        self.spawn(cmd, false)
    }

    /// Fetch the tags and refs of a remote.
    pub fn fetch(self, remote: &str) -> GitFuture<'io, ()> {
        let r1 = String::from(remote);
        let r2 = String::from(remote);
        Box::new(
            self.spawn_with(move |c| c.arg("fetch").arg("--prune").arg(r1))
                .and_then(move |_| {
                    self.spawn_with(|c| c.arg("fetch").arg("--tags").arg("--prune").arg(r2))
                })
                .map(|_| ()),
        )
    }

    /// List all refs and their hashes.
    pub fn list_refs(self) -> GitFuture<'io, Vec<(String, String)>> {
        Box::new(
            self.spawn_unchecked_with(|c| c.arg("show-ref"))
                .and_then(move |raw| {
                    future::join_all(
                        raw.lines()
                            .map(|line| {
                                // Parse the line.
                                let mut fields = line.split_whitespace().map(String::from);
                                // TODO: Handle the case where the line might not contain enough
                                // information or is missing some fields.
                                let mut rev = fields.next().unwrap();
                                let rf = fields.next().unwrap();
                                rev.push_str("^{commit}");

                                // Parse the ref. This is needed since the ref for an annotated
                                // tag points to the hash of the tag itself, rather than the
                                // underlying commit. By calling `git rev-parse` with the ref
                                // augmented with `^{commit}`, we can ensure that we always end
                                // up with a commit hash.
                                self.spawn_with(|c| c.arg("rev-parse").arg("--verify").arg(rev))
                                    .map(|rev| (rev.trim().into(), rf))
                            })
                            .collect::<Vec<_>>(),
                    )
                }),
        )
    }

    /// List all revisions.
    pub fn list_revs(self) -> GitFuture<'io, Vec<String>> {
        Box::new(
            self.spawn_with(|c| c.arg("rev-list").arg("--all").arg("--date-order"))
                .map(|raw| raw.lines().map(String::from).collect()),
        )
    }

    /// Determine the currently checked out revision.
    pub fn current_checkout(self) -> GitFuture<'io, Option<String>> {
        Box::new(
            self.spawn_with(|c| c.arg("rev-parse").arg("--revs-only").arg("HEAD^{commit}"))
                .map(|raw| raw.lines().take(1).map(String::from).next()),
        )
    }

    /// List files in the directory.
    ///
    /// Calls `git ls-tree` under the hood.
    pub fn list_files<R: AsRef<OsStr>, P: AsRef<OsStr>>(
        self,
        rev: R,
        path: Option<P>,
    ) -> GitFuture<'io, Vec<TreeEntry>> {
        Box::new(
            self.spawn_with(|c| {
                c.arg("ls-tree").arg(rev);
                if let Some(p) = path {
                    c.arg(p);
                }
                c
            })
            .map(|raw| raw.lines().map(TreeEntry::parse).collect()),
        )
    }

    /// Read the content of a file.
    pub fn cat_file<O: AsRef<OsStr>>(self, hash: O) -> GitFuture<'io, String> {
        Box::new(self.spawn_with(|c| c.arg("cat-file").arg("blob").arg(hash)))
    }
}

/// A future returned from any of the git functions.
pub type GitFuture<'io, T> = Box<dyn Future<Item = T, Error = Error> + 'io>;

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
