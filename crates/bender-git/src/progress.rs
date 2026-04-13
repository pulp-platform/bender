use std::sync::OnceLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// Public API — trait + types
// ---------------------------------------------------------------------------

/// The kind of git network operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOp {
    /// A `git fetch` on a database.
    Fetch,
    /// A `git submodule update --init --recursive` on a checkout.
    SubmoduleUpdate,
}

/// Progress events emitted by git network operations.
#[derive(Debug, Clone)]
pub enum GitProgressEvent<'a> {
    /// The operation has started.
    Started { op: GitOp, label: &'a str },
    /// Progress percentage (0–100) for a plain git operation.
    ///
    /// For fetch: receiving objects maps to 0–70%, resolving deltas to 70–100%.
    Progress { percent: u8 },
    /// Progress percentage (0–100) within the currently active submodule.
    SubmoduleProgress { name: &'a str, percent: u8 },
    /// The operation has completed.
    Finished,
}

/// A receiver for progress updates from git network operations.
///
/// The crate parses git's stderr internally and maps the individual phases
/// (receiving objects, resolving deltas, checking out files) and submodule
/// lifecycle messages into a stream of high-level events.
pub trait GitProgress: Send + 'static {
    /// Called for each progress event. The default implementation ignores all
    /// events, making [`NoProgress`] zero-cost.
    fn event(&mut self, _event: GitProgressEvent<'_>) {}
}

/// A no-op progress sink for callers that don't need progress reporting.
pub struct NoProgress;

impl GitProgress for NoProgress {}

// ---------------------------------------------------------------------------
// Internal — stderr line parsing
// ---------------------------------------------------------------------------

static RE_GIT: OnceLock<Regex> = OnceLock::new();

/// Parsed representation of a single git stderr line.
#[derive(Debug)]
pub(crate) enum GitStderrLine {
    CloningInto { name: String },
    Receiving { percent: u8 },
    Resolving { percent: u8 },
    Checkout { percent: u8 },
    Error,
    Other,
}

/// Parse a single line from git's stderr into a [`GitStderrLine`].
pub(crate) fn parse_git_line(line: &str) -> GitStderrLine {
    let line = line.trim();
    let re = RE_GIT.get_or_init(|| {
        Regex::new(
            r"(?x)
            ^ # Start
            (?:
                # 1. Cloning: Capture the path
                Cloning\ into\ '(?P<clone_path>[^']+)'\.\.\. |

                # 2. Progress
                (?P<phase>Receiving\ objects|Resolving\ deltas|Checking\ out\ files):\s+(?P<percent>\d+)% |

                # 3. Errors
                (?P<error>fatal:.*|error:.*|remote:\ aborting.*)
            )
        ",
        )
        .expect("Invalid regex")
    });

    if let Some(caps) = re.captures(line) {
        if let Some(path) = caps.name("clone_path") {
            return GitStderrLine::CloningInto {
                name: path_to_name(path.as_str()),
            };
        }
        if caps.name("error").is_some() {
            return GitStderrLine::Error;
        }
        if let Some(phase) = caps.name("phase") {
            let percent = caps.name("percent").unwrap().as_str().parse().unwrap_or(0);
            return match phase.as_str() {
                "Receiving objects" => GitStderrLine::Receiving { percent },
                "Resolving deltas" => GitStderrLine::Resolving { percent },
                "Checking out files" => GitStderrLine::Checkout { percent },
                _ => GitStderrLine::Other,
            };
        }
    }
    GitStderrLine::Other
}

/// Extract the leaf directory name from a git path.
fn path_to_name(path: &str) -> String {
    path.trim_end_matches('/')
        .split('/')
        .next_back()
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------------
// Internal — phase-to-percentage mapping
// ---------------------------------------------------------------------------

/// Progress mapping policy for a git operation.
pub(crate) enum ProgressMode {
    Fetch,
    Submodule,
}

/// Individual git transfer phase.
pub(crate) enum Phase {
    Receiving,
    Resolving,
    Checkout,
}

/// Convert a phase-local percentage (0–100) to an operation-local percentage.
pub(crate) fn map_progress(mode: ProgressMode, phase: Phase, percent: u8) -> u8 {
    let percent = percent as u16;
    match (mode, phase) {
        (ProgressMode::Fetch, Phase::Receiving) => (percent * 70 / 100) as u8,
        (ProgressMode::Fetch, Phase::Resolving) => (70 + percent * 30 / 100) as u8,
        (ProgressMode::Fetch, Phase::Checkout) => 100,
        (ProgressMode::Submodule, Phase::Receiving) => (percent * 50 / 100) as u8,
        (ProgressMode::Submodule, Phase::Resolving) => (50 + percent * 30 / 100) as u8,
        (ProgressMode::Submodule, Phase::Checkout) => (80 + percent * 20 / 100) as u8,
    }
}

// ---------------------------------------------------------------------------
// Internal — submodule progress tracking
// ---------------------------------------------------------------------------

/// Tracks the currently active submodule for [`GitOp::SubmoduleUpdate`].
pub(crate) struct SubmoduleTracker {
    active: Option<String>,
}

impl SubmoduleTracker {
    pub fn new() -> Self {
        Self { active: None }
    }

    /// Process a parsed stderr line and emit the appropriate progress/submodule
    /// events through the given sink.
    pub fn apply(&mut self, line: &GitStderrLine, progress: &mut impl GitProgress) {
        match line {
            GitStderrLine::CloningInto { name } => {
                if let Some(prev) = self.active.replace(name.clone()) {
                    progress.event(GitProgressEvent::SubmoduleProgress {
                        name: &prev,
                        percent: 100,
                    });
                }
                progress.event(GitProgressEvent::SubmoduleProgress { name, percent: 0 });
            }
            GitStderrLine::Receiving { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Receiving, *percent);
                if let Some(name) = self.active.as_deref() {
                    progress.event(GitProgressEvent::SubmoduleProgress {
                        name,
                        percent: local,
                    });
                }
            }
            GitStderrLine::Resolving { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Resolving, *percent);
                if let Some(name) = self.active.as_deref() {
                    progress.event(GitProgressEvent::SubmoduleProgress {
                        name,
                        percent: local,
                    });
                }
            }
            GitStderrLine::Checkout { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Checkout, *percent);
                if let Some(name) = self.active.as_deref() {
                    progress.event(GitProgressEvent::SubmoduleProgress {
                        name,
                        percent: local,
                    });
                }
            }
            GitStderrLine::Error | GitStderrLine::Other => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Stderr handlers and draining
// ---------------------------------------------------------------------------

/// Build a line handler for plain fetch progress.
pub(crate) fn on_fetch_progress<'a>(
    progress: &'a mut impl GitProgress,
) -> impl FnMut(GitStderrLine) + 'a {
    move |line| match line {
        GitStderrLine::Receiving { percent } => {
            progress.event(GitProgressEvent::Progress {
                percent: map_progress(ProgressMode::Fetch, Phase::Receiving, percent),
            });
        }
        GitStderrLine::Resolving { percent } => {
            progress.event(GitProgressEvent::Progress {
                percent: map_progress(ProgressMode::Fetch, Phase::Resolving, percent),
            });
        }
        GitStderrLine::Checkout { percent } => {
            progress.event(GitProgressEvent::Progress {
                percent: map_progress(ProgressMode::Fetch, Phase::Checkout, percent),
            });
        }
        _ => {}
    }
}

/// Build a line handler for recursive submodule update progress.
pub(crate) fn on_submodule_progress<'a>(
    tracker: &'a mut SubmoduleTracker,
    progress: &'a mut impl GitProgress,
) -> impl FnMut(GitStderrLine) + 'a {
    move |line| tracker.apply(&line, progress)
}

/// Drain a stderr stream byte-by-byte, parse completed lines, and forward each
/// parsed stderr line to `on_line`.
///
/// Returns the full raw stderr as a `String` for use in error messages.
pub(crate) async fn drain_stderr(
    stderr: &mut (impl tokio::io::AsyncRead + Unpin),
    mut on_line: impl FnMut(GitStderrLine),
) -> String {
    use tokio::io::AsyncReadExt;
    let mut line_buf = Vec::new();
    let mut raw = Vec::new();

    while let Ok(byte) = stderr.read_u8().await {
        raw.push(byte);
        if byte != b'\r' && byte != b'\n' {
            line_buf.push(byte);
            continue;
        }
        if let Ok(line) = std::str::from_utf8(&line_buf) {
            on_line(parse_git_line(line));
        }
        line_buf.clear();
    }

    String::from_utf8_lossy(&raw).into_owned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_receiving() {
        match parse_git_line("Receiving objects: 34% (123/456)") {
            GitStderrLine::Receiving { percent } => assert_eq!(percent, 34),
            other => panic!("Expected Receiving, got {:?}", other),
        }
    }

    #[test]
    fn parse_receiving_done() {
        match parse_git_line("Receiving objects: 100% (1955/1955), 1.51 MiB | 45.53 MiB/s, done.") {
            GitStderrLine::Receiving { percent } => assert_eq!(percent, 100),
            other => panic!("Expected Receiving, got {:?}", other),
        }
    }

    #[test]
    fn parse_resolving() {
        match parse_git_line("Resolving deltas: 56% (789/1400)") {
            GitStderrLine::Resolving { percent } => assert_eq!(percent, 56),
            other => panic!("Expected Resolving, got {:?}", other),
        }
    }

    #[test]
    fn parse_resolving_done() {
        match parse_git_line("Resolving deltas: 100% (1122/1122), done.") {
            GitStderrLine::Resolving { percent } => assert_eq!(percent, 100),
            other => panic!("Expected Resolving, got {:?}", other),
        }
    }

    #[test]
    fn parse_cloning_into() {
        match parse_git_line("Cloning into 'myrepo'...") {
            GitStderrLine::CloningInto { name } => assert_eq!(name, "myrepo"),
            other => panic!("Expected CloningInto, got {:?}", other),
        }
    }

    #[test]
    fn parse_error() {
        match parse_git_line(
            "fatal: unable to access 'https://example.com/repo.git/': Could not resolve host: example.com",
        ) {
            GitStderrLine::Error => {}
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn map_progress_fetch() {
        assert_eq!(map_progress(ProgressMode::Fetch, Phase::Receiving, 0), 0);
        assert_eq!(map_progress(ProgressMode::Fetch, Phase::Receiving, 50), 35);
        assert_eq!(map_progress(ProgressMode::Fetch, Phase::Receiving, 100), 70);
        assert_eq!(map_progress(ProgressMode::Fetch, Phase::Resolving, 0), 70);
        assert_eq!(
            map_progress(ProgressMode::Fetch, Phase::Resolving, 100),
            100
        );
    }

    #[test]
    fn map_progress_submodule() {
        assert_eq!(
            map_progress(ProgressMode::Submodule, Phase::Receiving, 100),
            50
        );
        assert_eq!(
            map_progress(ProgressMode::Submodule, Phase::Resolving, 100),
            80
        );
        assert_eq!(
            map_progress(ProgressMode::Submodule, Phase::Checkout, 100),
            100
        );
    }

    #[test]
    fn submodule_tracker_current_progress() {
        struct Recorder {
            submodule_progress: Vec<(String, u8)>,
        }
        impl GitProgress for Recorder {
            fn event(&mut self, event: GitProgressEvent<'_>) {
                match event {
                    GitProgressEvent::SubmoduleProgress { name, percent } => {
                        self.submodule_progress.push((name.to_owned(), percent));
                    }
                    GitProgressEvent::Progress { .. } => {}
                    GitProgressEvent::Started { .. } | GitProgressEvent::Finished => {}
                }
            }
        }

        let mut tracker = SubmoduleTracker::new();
        let mut rec = Recorder {
            submodule_progress: vec![],
        };

        tracker.apply(&GitStderrLine::CloningInto { name: "a".into() }, &mut rec);
        assert_eq!(rec.submodule_progress.last(), Some(&("a".into(), 0)));
        tracker.apply(&GitStderrLine::Receiving { percent: 100 }, &mut rec);
        assert_eq!(rec.submodule_progress.last(), Some(&("a".into(), 50)));
    }

    #[test]
    fn submodule_tracker_finishes_previous_on_next_clone() {
        struct Recorder {
            submodule_progress: Vec<(String, u8)>,
        }
        impl GitProgress for Recorder {
            fn event(&mut self, event: GitProgressEvent<'_>) {
                if let GitProgressEvent::SubmoduleProgress { name, percent } = event {
                    self.submodule_progress.push((name.to_owned(), percent));
                }
            }
        }

        let mut tracker = SubmoduleTracker::new();
        let mut rec = Recorder {
            submodule_progress: vec![],
        };

        tracker.apply(&GitStderrLine::CloningInto { name: "a".into() }, &mut rec);
        tracker.apply(&GitStderrLine::CloningInto { name: "b".into() }, &mut rec);

        assert_eq!(rec.submodule_progress[0], ("a".into(), 0));
        assert_eq!(rec.submodule_progress[1], ("a".into(), 100));
        assert_eq!(rec.submodule_progress[2], ("b".into(), 0));
    }
}
