// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use crate::util::fmt_duration;

use indexmap::IndexMap;
use std::sync::OnceLock;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use tokio::io::{AsyncReadExt, BufReader};

/// Parses a line of git output.
/// (Put your `GitProgress` enum and `parse_git_line` function here)
#[derive(Debug, PartialEq, Clone)]
pub enum GitProgress {
    SubmoduleRegistered { name: String },
    CloningInto { name: String },
    SubmoduleEnd { name: String },
    Receiving { percent: u8 },
    Resolving { percent: u8 },
    Checkout { percent: u8 },
    Error(String),
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

/// Helper to extract the name from a git path.
fn path_to_name(path: &str) -> String {
    path.trim_end_matches('/')
        .split('/')
        .last()
        .unwrap_or(path)
        .to_string()
}

pub fn parse_git_line(line: &str) -> GitProgress {
    let line = line.trim();
    let re = RE_GIT.get_or_init(|| {
        Regex::new(r"(?x)
            ^ # Start
            (?:
                # 1. Registration: Capture the path, ignore the descriptive name
                Submodule\ '[^']+'\ .*\ registered\ for\ path\ '(?P<reg_path>[^']+)' |

                # 2. Cloning: Capture the path
                Cloning\ into\ '(?P<clone_path>[^']+)'\.\.\. |

                # 3. Completion: Capture the name
                Submodule\ path\ '(?P<sub_end_name>[^']+)':\ checked\ out\ '.* |

                # 4. Progress
                (?P<phase>Receiving\ objects|Resolving\ deltas|Checking\ out\ files):\s+(?P<percent>\d+)% |

                # 5. Errors
                (?P<error>fatal:.*|error:.*|remote:\ aborting.*)
            )
        ").expect("Invalid Regex")
    });

    if let Some(caps) = re.captures(line) {
        if let Some(path) = caps.name("reg_path") {
            return GitProgress::SubmoduleRegistered {
                name: path_to_name(path.as_str()),
            };
        }
        if let Some(path) = caps.name("clone_path") {
            return GitProgress::CloningInto {
                name: path_to_name(path.as_str()),
            };
        }
        if let Some(path) = caps.name("sub_end_name") {
            return GitProgress::SubmoduleEnd {
                name: path_to_name(path.as_str()),
            };
        }
        if let Some(err) = caps.name("error") {
            return GitProgress::Error(err.as_str().to_string());
        }
        if let Some(phase) = caps.name("phase") {
            let percent = caps.name("percent").unwrap().as_str().parse().unwrap_or(0);
            return match phase.as_str() {
                "Receiving objects" => GitProgress::Receiving { percent },
                "Resolving deltas" => GitProgress::Resolving { percent },
                "Checking out files" => GitProgress::Checkout { percent },
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
    /// The sub-progress bar (for submodules), if any.
    pub sub_bars: IndexMap<String, ProgressBar>,
    // The currently active submodule, if any.
    pub active_sub: Option<String>,
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
            sub_bars: IndexMap::new(),
            active_sub: None,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn update_pb(&self, line: &str, state: &mut ProgressState) {
        let progress = parse_git_line(line);

        // Target the active submodule if one exists, otherwise the main bar
        let target_pb = if let Some(name) = &state.active_sub {
            state.sub_bars.get(name).unwrap_or(&state.pb)
        } else {
            &state.pb
        };

        match progress {
            GitProgress::SubmoduleRegistered { name } => {
                if self.git_op == GitProgressOps::Submodule {
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

                    // Tree Logic
                    let ref_bar = match state.sub_bars.last() {
                        Some((last_name, last_pb)) => {
                            // Update the previous last bar to have a "T" connector (├─)
                            // because it is no longer the last one.
                            let prev_prefix = format!("{} {}", dim!("├─ "), dim!(last_name));
                            last_pb.set_prefix(prev_prefix);
                            last_pb // Insert the new one after this one
                        }
                        None => &state.pb, // Insert the first one after the main bar
                    };

                    // Create bar immediately
                    let sub_pb = self
                        .mpb
                        .insert_after(ref_bar, ProgressBar::new(100).with_style(style));

                    let sub_prefix = format!("{} {}", dim!("└─ "), dim!(&name));
                    sub_pb.set_prefix(sub_prefix);
                    sub_pb.set_message(dim!("Waiting...").to_string());

                    state.sub_bars.insert(name, sub_pb);
                }
            }
            GitProgress::CloningInto { name } => {
                if self.git_op == GitProgressOps::Submodule {
                    // Logic to handle missing 'checked out' lines:
                    // If we are activating 'bar', but 'foo' was active, assume 'foo' is done.
                    if let Some(prev) = &state.active_sub {
                        if prev != &name {
                            if let Some(b) = state.sub_bars.get(prev) {
                                b.finish_and_clear();
                            }
                        }
                    }
                    // Activate the new bar
                    if let Some(bar) = state.sub_bars.get(&name) {
                        // Switch style to the active progress bar style
                        bar.set_message(dim!("Cloning...").to_string());
                    }
                    state.active_sub = Some(name);
                }
            }
            GitProgress::SubmoduleEnd { name } => {
                if let Some(bar) = state.sub_bars.get(&name) {
                    bar.finish_and_clear();
                }
                if state.active_sub.as_ref() == Some(&name) {
                    state.active_sub = None;
                }
            }
            GitProgress::Receiving { percent, .. } => {
                target_pb.set_message(dim!("Receiving objects").to_string());
                target_pb.set_position(percent as u64);
            }
            GitProgress::Resolving { percent, .. } => {
                target_pb.set_message(dim!("Resolving deltas").to_string());
                target_pb.set_position(percent as u64);
            }
            GitProgress::Checkout { percent, .. } => {
                target_pb.set_message(dim!("Checking out").to_string());
                target_pb.set_position(percent as u64);
            }
            GitProgress::Error(err_msg) => {
                target_pb.finish_and_clear();
                // TODO(fischeti): Consider enumerating error
                errorln!(
                    "{} {}: {}",
                    "Error during git operation of",
                    bold!(&self.name),
                    err_msg
                );
            }
            _ => {}
        }
    }

    pub fn finish(self, state: &mut ProgressState) {
        // Clear all sub bars that might be lingering
        for pb in state.sub_bars.values() {
            pb.finish_and_clear();
        }
        state.pb.finish_and_clear();

        // Print a final message indicating completion
        let op_str = match self.git_op {
            GitProgressOps::Clone => "Cloned",
            GitProgressOps::Fetch => "Fetched",
            GitProgressOps::Checkout => "Checked out",
            GitProgressOps::Submodule => "Updated Submodules",
        };

        self.mpb
            .println(format!(
                "  {} {} {}",
                green_bold!(op_str),
                bold!(&self.name),
                dim!(fmt_duration(state.start_time.elapsed()))
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
