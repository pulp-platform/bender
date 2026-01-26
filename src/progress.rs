// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use crate::util::fmt_duration;

use indexmap::IndexMap;
use std::sync::OnceLock;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use tokio::io::{AsyncReadExt, BufReader};

use crate::{fmt_completed, fmt_dim, fmt_pkg, fmt_stage};

static RE_GIT: OnceLock<Regex> = OnceLock::new();

// The alignment of the operation strings
const OP_ALIGN: usize = 12;

/// The result of parsing a git progress line.
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

impl GitProgressOps {
    /// Returns the present-tense name (for active bars) and the padding needed for the spinner.
    fn active_fmt(&self) -> (&'static str, usize) {
        let name = match self {
            Self::Clone => "Cloning",
            Self::Fetch => "Fetching",
            Self::Checkout => "Checking out",
            Self::Submodule => "Submodules",
        };
        (name, (OP_ALIGN - name.len()) + 1)
    }

    /// Returns the past-tense name (for finished lines).
    fn past_fmt(&self) -> &'static str {
        match self {
            Self::Clone => "Cloned",
            Self::Fetch => "Fetched",
            Self::Checkout => "Checked out",
            Self::Submodule => "Submodules",
        }
    }
}

/// Captures (dynamic) state information for a git operation's progress.
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

/// Captures (static) information neeed to handle progress updates for a git operation.
pub struct ProgressHandler {
    /// Reference to the multi-progress bar, which can manage multiple progress bars.
    multiprogress: Option<MultiProgress>,
    /// The type of git operation being performed.
    git_op: GitProgressOps,
    /// The name of the repository being processed.
    name: String,
}

/// The git operation types that currently support progress reporting.
#[derive(PartialEq)]
pub enum GitProgressOps {
    Checkout,
    Clone,
    Fetch,
    Submodule,
}

/// Monitor the stderr stream of a git process and update progress bars
/// of a given handler accordingly.
pub async fn monitor_stderr(
    stream: impl tokio::io::AsyncRead + Unpin,
    handler: Option<ProgressHandler>,
) -> String {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new(); // Buffer for accumulating bytes of a line
    let mut raw_log = Vec::new(); // The full raw log output

    // Add a new progress bar and state if we have a handler
    let mut state = handler.as_ref().map(|h| h.start());

    // We loop over the stream reading byte by byte
    // and process lines as they are completed.
    while let Ok(byte) = reader.read_u8().await {
        raw_log.push(byte);

        // We push bytes into the buffer until we hit a delimiter
        if byte != b'\r' && byte != b'\n' {
            buffer.push(byte);
            continue;
        }

        // Process the line, if:
        if let (
            // 1. it is valid UTF-8
            Ok(line),
            // 2. we have a progress handler
            Some(
                h @ ProgressHandler {
                    // 3. we have a multi-progress bar
                    multiprogress: Some(_),
                    ..
                },
            ),
        ) = (std::str::from_utf8(&buffer), &handler)
        {
            // Parse the line and update the progress bar accordingly
            let progress = parse_git_line(line);
            h.update_pb(progress, state.as_mut().unwrap());
        }

        // Always clear buffer after a delimiter
        buffer.clear();
    }

    // Finalize the progress bar if we have a handler
    if let Some(handler) = handler {
        handler.finish(&mut state.unwrap());
    }

    // Return the full raw log as a string
    String::from_utf8_lossy(&raw_log).to_string()
}

impl ProgressHandler {
    /// Create a new progress handler for a git operation.
    pub fn new(multiprogress: Option<MultiProgress>, git_op: GitProgressOps, name: &str) -> Self {
        Self {
            multiprogress,
            git_op,
            name: name.to_string(),
        }
    }

    /// Adds a new progress bar to the multi-progress and returns the initial state
    /// that is needed to track progress updates.
    pub fn start(&self) -> ProgressState {
        // In case there is no multi-progress progress, we just create a hidden progress bar.
        if self.multiprogress.is_none() {
            return ProgressState {
                pb: ProgressBar::hidden(),
                sub_bars: IndexMap::new(),
                active_sub: None,
                start_time: std::time::Instant::now(),
            };
        }

        let (op_name, spinner_pad) = self.git_op.active_fmt();

        // Set the prefix based on the git operation
        let template = format!(
            "{{spinner:>{spinner_pad}.cyan}} {{prefix:<32}} {{bar:40.cyan/blue}} {{percent:>3}}% {{msg}}"
        );

        let style = ProgressStyle::with_template(template.as_str())
            .unwrap()
            .progress_chars("-- ")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);

        // Create and attach the progress bar to the multi-progress bar.
        let pb = self
            .multiprogress
            .as_ref()
            .unwrap()
            .add(ProgressBar::new(100).with_style(style));

        let prefix = format!("{} {}", fmt_stage!(op_name), fmt_pkg!(&self.name));
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

    /// Update the progress bar(s) based on a parsed git progress line.
    fn update_pb(&self, progress: GitProgress, state: &mut ProgressState) {
        // Target the active submodule if one exists, otherwise the main bar
        let target_pb = if let Some(name) = &state.active_sub {
            state.sub_bars.get(name).unwrap_or(&state.pb)
        } else {
            &state.pb
        };

        match progress {
            // This case is only relevant for submodule operations i.e. `git submodule update`
            // It indicates that a new submodule has been registered, and we create a new progress bar for it.
            GitProgress::SubmoduleRegistered { name } => {
                if self.git_op == GitProgressOps::Submodule {
                    // The main bar simply becomes a spinner since the sub-bar will show progress
                    // on the subsequent line.
                    state.pb.set_style(
                        ProgressStyle::with_template("{spinner:>3.cyan} {prefix:<40!}").unwrap(),
                    );

                    // The submodule style is similar to the main bar, but indented and without spinner
                    let style = ProgressStyle::with_template(
                        "     {prefix:<24!} {bar:40.cyan/blue} {percent:>3}% {msg}",
                    )
                    .unwrap()
                    .progress_chars("-- ");

                    // We can have multiple sub-bars, and we insert them after the last one.
                    // In order to maintain proper tree-like structure, we need to update the previous last bar
                    // to have a "T" connector (├─) instead of an "L"
                    let prev_bar = match state.sub_bars.last() {
                        Some((last_name, last_pb)) => {
                            let prev_prefix = format!("{} {}", fmt_dim!("├─"), last_name);
                            last_pb.set_prefix(prev_prefix);
                            last_pb // Insert the new one after this one
                        }
                        None => &state.pb, // Insert the first one after the main bar
                    };

                    // Create the new sub-bar and insert it in the multi-progress *after* the previous sub-bar
                    let sub_pb = self
                        .multiprogress
                        .as_ref()
                        .unwrap()
                        .insert_after(prev_bar, ProgressBar::new(100).with_style(style));
                    // Set the prefix and initial message
                    let sub_prefix = format!("{} {}", fmt_dim!("╰─"), &name);
                    sub_pb.set_prefix(sub_prefix);
                    sub_pb.set_message(format!("{}", fmt_dim!("Waiting...")));

                    // Store the sub-bar in the state for later updates
                    state.sub_bars.insert(name, sub_pb);
                }
            }
            // This indicates that we are starting to clone a submodule.
            // Again, it is only relevant for submodule operations. For normal
            // clones, we just update the main bar.
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
                    // Set the new bar to active
                    if let Some(bar) = state.sub_bars.get(&name) {
                        // Switch style to the active progress bar style
                        bar.set_message(format!("{}", fmt_dim!("Cloning...")));
                    }
                    state.active_sub = Some(name);
                }
            }
            // Indicates that we have finished processing a submodule.
            GitProgress::SubmoduleEnd { name } => {
                // We finish and clear the sub-bar
                if let Some(bar) = state.sub_bars.get(&name) {
                    bar.finish_and_clear();
                }
                // If this was the active submodule, we clear the active state
                if state.active_sub.as_ref() == Some(&name) {
                    state.active_sub = None;
                }
            }
            // Update the progress percentage for receiving objects
            GitProgress::Receiving { percent, .. } => {
                target_pb.set_message(format!("{}", fmt_dim!("Receiving objects")));
                target_pb.set_position(percent as u64);
            }
            // Update the progress percentage for resolving deltas
            GitProgress::Resolving { percent, .. } => {
                target_pb.set_message(format!("{}", fmt_dim!("Resolving deltas")));
                target_pb.set_position(percent as u64);
            }
            // Update the progress percentage for checking out files
            GitProgress::Checkout { percent, .. } => {
                target_pb.set_message(format!("{}", fmt_dim!("Checking out")));
                target_pb.set_position(percent as u64);
            }
            // Handle errors by finishing and clearing the target bar, then logging the error
            GitProgress::Error(err_msg) => {
                target_pb.finish_and_clear();
                errorln!(
                    "{} {}: {}",
                    "Error during git operation of",
                    fmt_pkg!(&self.name),
                    err_msg
                );
            }
            _ => {}
        }
    }

    // Finalize the progress bars and print a completion message.
    pub fn finish(self, state: &mut ProgressState) {
        // Clear all sub bars that might be lingering
        for pb in state.sub_bars.values() {
            pb.finish_and_clear();
        }
        state.pb.finish_and_clear();

        let finish_msg = format!(
            "{:>14} {} {}",
            fmt_completed!(self.git_op.past_fmt()),
            fmt_pkg!(&self.name),
            fmt_dim!(fmt_duration(state.start_time.elapsed()))
        );

        // We print on top of the progress bars, if they exist.
        // Otherwise, we just print to stderr.
        if let Some(multi) = self.multiprogress {
            // Print a completion message on top of active progress bars
            multi.println(finish_msg).unwrap();
        } else {
            eprintln!("{}", finish_msg);
        }
    }
}

/// Parse a git progress line and return the corresponding `GitProgress` enum.
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

/// Helper to extract the name from a git path.
fn path_to_name(path: &str) -> String {
    path.trim_end_matches('/')
        .split('/')
        .next_back()
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_receiving() {
        // Copy your existing unit tests here
        let p = parse_git_line("Receiving objects: 34% (123/456)");
        match p {
            GitProgress::Receiving { percent, .. } => assert_eq!(percent, 34),
            _ => panic!("Failed to parse receiving"),
        }
    }
    #[test]
    fn test_parsing_receiving_done() {
        // Copy your existing unit tests here
        let p =
            parse_git_line("Receiving objects: 100% (1955/1955), 1.51 MiB | 45.53 MiB/s, done.");
        match p {
            GitProgress::Receiving { percent, .. } => assert_eq!(percent, 100),
            _ => panic!("Failed to parse receiving"),
        }
    }
    #[test]
    fn test_parsing_resolving() {
        // Copy your existing unit tests here
        let p = parse_git_line("Resolving deltas: 56% (789/1400)");
        match p {
            GitProgress::Resolving { percent, .. } => assert_eq!(percent, 56),
            _ => panic!("Failed to parse receiving"),
        }
    }
    #[test]
    fn test_parsing_resolving_deltas_done() {
        // Copy your existing unit tests here
        let p = parse_git_line("Resolving deltas: 100% (1122/1122), done.");
        match p {
            GitProgress::Resolving { percent, .. } => assert_eq!(percent, 100),
            _ => panic!("Failed to parse receiving"),
        }
    }
    #[test]
    fn test_parsing_cloning_into() {
        let p = parse_git_line("Cloning into 'myrepo'...");
        match p {
            GitProgress::CloningInto { name } => assert_eq!(name, "myrepo"),
            _ => panic!("Failed to parse cloning into"),
        }
    }
    #[test]
    fn test_parsing_submodule_registered() {
        let p = parse_git_line("Submodule 'libs/mylib' ... registered for path 'libs/mylib'");
        match p {
            GitProgress::SubmoduleRegistered { name } => assert_eq!(name, "mylib"),
            _ => panic!("Failed to parse submodule registered"),
        }
    }
    #[test]
    fn test_parsing_submodule_end() {
        let p = parse_git_line("Submodule path 'libs/mylib': checked out 'abc1234'");
        match p {
            GitProgress::SubmoduleEnd { name } => assert_eq!(name, "mylib"),
            _ => panic!("Failed to parse submodule end"),
        }
    }
    #[test]
    fn test_parsing_error() {
        let p = parse_git_line("fatal: unable to access 'https://example.com/repo.git/': Could not resolve host: example.com");
        match p {
            GitProgress::Error(msg) => assert!(msg.contains("fatal: unable to access")),
            _ => panic!("Failed to parse error"),
        }
    }
}
