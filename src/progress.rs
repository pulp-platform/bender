// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use std::sync::OnceLock;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use tokio::io::{AsyncReadExt, BufReader};

/// Parses a line of git output.
/// (Put your `GitProgress` enum and `parse_git_line` function here)
#[derive(Debug, PartialEq, Clone)]
pub enum GitProgress {
    CloningInto {
        path: String,
    },
    SubmoduleEnd {
        path: String,
    },
    Receiving {
        percent: u8,
        current: usize,
        total: usize,
    },
    Resolving {
        percent: u8,
        current: usize,
        total: usize,
    },
    Checkout {
        percent: u8,
        current: usize,
        total: usize,
    },
    Other,
}

/// The git operation types that currently support progress reporting.
#[derive(Debug, PartialEq, Clone)]
pub enum GitProgressOps {
    Checkout,
    Clone,
    Fetch,
    Submodule,
}

static RE_GIT: OnceLock<Regex> = OnceLock::new();

pub fn parse_git_line(line: &str) -> GitProgress {
    let line = line.trim();
    let re = RE_GIT.get_or_init(|| {
        Regex::new(r"(?x)
            ^ # Start
            (?:
                Cloning\ into\ '(?P<clone_path>[^']+)'\.\.\. |
                Submodule\ path\ '(?P<sub_end_path>[^']+)':\ checked\ out\ '.* |
                (?P<phase>Receiving\ objects|Resolving\ deltas|Checking\ out\ files):\s+(?P<percent>\d+)%
                (?: \s+ \( (?P<current>\d+) / (?P<total>\d+) \) )?
            )
        ").expect("Invalid Regex")
    });

    if let Some(caps) = re.captures(line) {
        // Case 1: Cloning into...
        if let Some(path) = caps.name("clone_path") {
            return GitProgress::CloningInto {
                path: path.as_str().to_string(),
            };
        }

        // Case 2: Submodule finished
        if let Some(path) = caps.name("sub_end_path") {
            return GitProgress::SubmoduleEnd {
                path: path.as_str().to_string(),
            };
        }

        // Case 3: Progress
        if let Some(phase) = caps.name("phase") {
            let percent = caps.name("percent").unwrap().as_str().parse().unwrap_or(0);
            let current = caps
                .name("current")
                .map(|m| m.as_str().parse().unwrap_or(0))
                .unwrap_or(0);
            let total = caps
                .name("total")
                .map(|m| m.as_str().parse().unwrap_or(0))
                .unwrap_or(0);

            return match phase.as_str() {
                "Receiving objects" => GitProgress::Receiving {
                    percent,
                    current,
                    total,
                },
                "Resolving deltas" => GitProgress::Resolving {
                    percent,
                    current,
                    total,
                },
                "Checking out files" => GitProgress::Checkout {
                    percent,
                    current,
                    total,
                },
                _ => GitProgress::Other,
            };
        }
    }
    // Otherwise, we don't care
    GitProgress::Other
}

/// This struct captures (dynamic) state information for a git operation's progress.
/// for instance, the actuall progress bars to update.
pub struct ProgressState {
    /// The progress bar of the current package.
    pb: ProgressBar,
    /// The progress bar for submodules, if any.
    sub_pb: Option<ProgressBar>,
    /// The start time of the operation.
    start_time: std::time::Instant,
}

/// This struct captures (static) information neeed to handle progress updates for a git operation.
pub struct ProgressHandler {
    /// Reference to the multi-progress bar, which can manage multiple progress bars.
    mpb: MultiProgress,
    /// The type of git operation being performed.
    git_op: GitProgressOps,
    /// The name of the repository being processed.
    name: String,
}

impl ProgressHandler {
    /// Create a new progress handler for a git operation.
    pub fn new(mpb: MultiProgress, git_op: GitProgressOps, name: &str) -> Self {
        Self {
            mpb,
            git_op,
            name: name.to_string(),
        }
    }

    pub fn start(&self) -> ProgressState {
        // Create and configure the main progress bar
        let style = ProgressStyle::with_template(
            "{spinner:.green} {prefix:<32!} {bar:40.cyan/blue} {percent:>3}% {msg}",
        )
        .unwrap()
        .progress_chars("-- ")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);

        let pb = self.mpb.add(ProgressBar::new(100).with_style(style));

        let prefix = match self.git_op {
            GitProgressOps::Clone => "Cloning",
            GitProgressOps::Fetch => "Fetching",
            GitProgressOps::Checkout => "Checkout",
            GitProgressOps::Submodule => "Update Submodules",
        };
        let prefix = format!("{} {}", green_bold!(prefix), bold!(&self.name));
        pb.set_prefix(prefix);
        // Configure the spinners to automatically tick every 100ms
        pb.enable_steady_tick(Duration::from_millis(100));

        ProgressState {
            pb,
            sub_pb: None,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn update_pb(&self, line: &str, state: &mut ProgressState) {
        let progress = parse_git_line(line);
        let target_pb = state.sub_pb.as_ref().unwrap_or(&state.pb);

        match progress {
            GitProgress::CloningInto { path } => {
                // Only spawn a sub-bar if we are explicitly running the 'Submodule' op.
                // For normal Clone/Checkout, 'Cloning into' is just the main repo header, which we ignore.
                if self.git_op == GitProgressOps::Submodule {
                    if let Some(sub) = state.sub_pb.take() {
                        sub.finish_and_clear();
                    }
                    // The main simply becomes a spinner since the sub-bar will show progress
                    // on the subsequent line.
                    state.pb.set_style(
                        ProgressStyle::with_template("{spinner:.green} {prefix:<32!}").unwrap(),
                    );

                    // The submodule style is similar to the main bar, but indented and without spinner
                    let style = ProgressStyle::with_template(
                        "    {prefix:<32!} {bar:40.cyan/blue} {percent:>3}% {msg}",
                    )
                    .unwrap()
                    .progress_chars("-- ");

                    // Create the submodule progress bar below the main one
                    let sub_pb = self
                        .mpb
                        .insert_after(&state.pb, ProgressBar::new(100).with_style(style));

                    // Set the submodule prefix to the submodule name
                    let sub_name = path.split('/').last().unwrap_or(&path);
                    let sub_prefix = format!("{} {}", dim!("└─ "), dim!(sub_name));
                    sub_pb.set_prefix(sub_prefix);
                    state.sub_pb = Some(sub_pb);
                }
            }
            GitProgress::SubmoduleEnd { .. } => {
                if let Some(sub) = state.sub_pb.take() {
                    sub.finish_and_clear();
                }
            }
            GitProgress::Receiving { current, .. } => {
                target_pb.set_message(dim!("Receiving objects").to_string());
                target_pb.set_position(current as u64);
            }
            GitProgress::Resolving { percent, .. } => {
                target_pb.set_message(dim!("Resolving deltas").to_string());
                target_pb.set_position(percent as u64);
            }
            GitProgress::Checkout { percent, .. } => {
                target_pb.set_message(dim!("Checking out").to_string());
                target_pb.set_position(percent as u64);
            }
            _ => {}
        }
    }

    pub fn finish(self, state: &mut ProgressState) {
        if let Some(sub) = state.sub_pb.take() {
            sub.finish_and_clear();
        }
        state.pb.finish_and_clear();

        // Print a final message indicating completion
        let op_str = match self.git_op {
            GitProgressOps::Clone => "Cloned",
            GitProgressOps::Fetch => "Fetched",
            GitProgressOps::Checkout => "Checked out",
            GitProgressOps::Submodule => "Updated Submodules",
        };

        // Format the duration nicely based on its length
        let duration_str = match state.start_time.elapsed().as_millis() {
            ms if ms < 1000 => format!("in {}ms", ms),
            ms => format!("in {:.1}s", ms as f64 / 1000.0),
        };
        self.mpb
            .println(format!(
                "  {} {} {}",
                green_bold!(op_str),
                bold!(&self.name),
                dim!(duration_str)
            ))
            .unwrap();
    }
}

pub async fn monitor_stderr(
    stream: impl tokio::io::AsyncRead + Unpin,
    handler: Option<ProgressHandler>,
) -> String {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    let mut collected_stderr = String::new();

    // Add a new progress bar and state if we have a handler
    let mut state = handler.as_ref().map(|h| h.start());

    loop {
        match reader.read_u8().await {
            Ok(byte) => {
                // Collect raw error output (simplified for brevity)
                if byte.is_ascii() {
                    collected_stderr.push(byte as char);
                }

                if byte == b'\r' || byte == b'\n' {
                    if !buffer.is_empty() {
                        if let Ok(line) = std::str::from_utf8(&buffer) {
                            // Update UI if we have a handler
                            if let Some(h) = &handler {
                                h.update_pb(line, &mut state.as_mut().unwrap());
                            }
                        }
                        buffer.clear();
                    }
                } else {
                    buffer.push(byte);
                }
            }
            Err(_) => break,
        }
    }

    handler.map(|h| h.finish(&mut state.unwrap()));

    collected_stderr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_logic() {
        // Copy your existing unit tests here
        let p = parse_git_line("Receiving objects: 34% (123/456)");
        match p {
            GitProgress::Receiving { percent, .. } => assert_eq!(percent, 34),
            _ => panic!("Failed to parse receiving"),
        }
    }
}
