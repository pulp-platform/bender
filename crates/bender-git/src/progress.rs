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

/// A receiver for progress updates from git network operations.
///
/// The crate parses git's stderr internally and reports progress via opaque
/// sink-owned ids. A sink can use those ids to map one git operation to one
/// progress bar, spinner, log record, or any other UI element.
pub trait GitProgress: Send + 'static {
    /// Opaque progress handle returned by [`GitProgress::started`].
    type Id: Copy + Eq + Send + 'static;

    /// Create a new progress stream for `op`.
    fn started(&mut self, _op: GitOp, _label: &str) -> Self::Id;

    /// Update the progress percentage for an existing progress stream.
    fn progress(&mut self, _id: Self::Id, _percent: u8) {}

    /// Mark a progress stream as finished.
    fn finished(&mut self, _id: Self::Id) {}
}

/// A no-op progress sink for callers that don't need progress reporting.
pub struct NoProgress;

impl GitProgress for NoProgress {
    type Id = ();

    fn started(&mut self, _op: GitOp, _label: &str) -> Self::Id {}
}

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
pub(crate) struct SubmoduleTracker<Id> {
    active: Option<Id>,
}

impl<Id> SubmoduleTracker<Id> {
    pub fn new() -> Self {
        Self { active: None }
    }
}

impl<Id: Copy + Eq> SubmoduleTracker<Id> {
    /// Process a parsed stderr line and emit the appropriate progress updates
    /// through the given sink.
    pub fn apply<P: GitProgress<Id = Id>>(&mut self, line: &GitStderrLine, progress: &mut P) {
        match line {
            GitStderrLine::CloningInto { name } => {
                if let Some(prev) = self.active.take() {
                    progress.finished(prev);
                }
                self.active = Some(progress.started(GitOp::SubmoduleUpdate, name));
            }
            GitStderrLine::Receiving { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Receiving, *percent);
                if let Some(id) = self.active {
                    progress.progress(id, local);
                }
            }
            GitStderrLine::Resolving { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Resolving, *percent);
                if let Some(id) = self.active {
                    progress.progress(id, local);
                }
            }
            GitStderrLine::Checkout { percent } => {
                let local = map_progress(ProgressMode::Submodule, Phase::Checkout, *percent);
                if let Some(id) = self.active {
                    progress.progress(id, local);
                }
            }
            GitStderrLine::Error | GitStderrLine::Other => {}
        }
    }

    pub fn finish<P: GitProgress<Id = Id>>(&mut self, progress: &mut P) {
        if let Some(id) = self.active.take() {
            progress.finished(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Stderr handlers and draining
// ---------------------------------------------------------------------------

/// Build a line handler for plain fetch progress.
pub(crate) fn on_fetch_progress<'a, P>(progress: &'a mut P, id: P::Id) -> impl FnMut(&str) + 'a
where
    P: GitProgress,
{
    move |line| match parse_git_line(line) {
        GitStderrLine::Receiving { percent } => {
            progress.progress(
                id,
                map_progress(ProgressMode::Fetch, Phase::Receiving, percent),
            );
        }
        GitStderrLine::Resolving { percent } => {
            progress.progress(
                id,
                map_progress(ProgressMode::Fetch, Phase::Resolving, percent),
            );
        }
        GitStderrLine::Checkout { percent } => {
            progress.progress(
                id,
                map_progress(ProgressMode::Fetch, Phase::Checkout, percent),
            );
        }
        _ => {}
    }
}

/// Build a line handler for recursive submodule update progress.
pub(crate) fn on_submodule_progress<'a, P>(
    tracker: &'a mut SubmoduleTracker<P::Id>,
    progress: &'a mut P,
) -> impl FnMut(&str) + 'a
where
    P: GitProgress,
{
    move |line| tracker.apply(&parse_git_line(line), progress)
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
        #[derive(Default)]
        struct Recorder {
            next_id: usize,
            events: Vec<(usize, Option<u8>, bool)>,
        }

        impl GitProgress for Recorder {
            type Id = usize;

            fn started(&mut self, _op: GitOp, _label: &str) -> Self::Id {
                let id = self.next_id;
                self.next_id += 1;
                self.events.push((id, None, false));
                id
            }

            fn progress(&mut self, id: Self::Id, percent: u8) {
                self.events.push((id, Some(percent), false));
            }

            fn finished(&mut self, id: Self::Id) {
                self.events.push((id, None, true));
            }
        }

        let mut tracker = SubmoduleTracker::new();
        let mut rec = Recorder::default();

        tracker.apply(&GitStderrLine::CloningInto { name: "a".into() }, &mut rec);
        tracker.apply(&GitStderrLine::Receiving { percent: 100 }, &mut rec);

        assert_eq!(rec.events[0], (0, None, false));
        assert_eq!(rec.events[1], (0, Some(50), false));
    }

    #[test]
    fn submodule_tracker_finishes_previous_on_next_clone() {
        #[derive(Default)]
        struct Recorder {
            next_id: usize,
            started: Vec<(usize, String)>,
            finished: Vec<usize>,
        }

        impl GitProgress for Recorder {
            type Id = usize;

            fn started(&mut self, _op: GitOp, label: &str) -> Self::Id {
                let id = self.next_id;
                self.next_id += 1;
                self.started.push((id, label.to_owned()));
                id
            }

            fn finished(&mut self, id: Self::Id) {
                self.finished.push(id);
            }
        }

        let mut tracker = SubmoduleTracker::new();
        let mut rec = Recorder::default();

        tracker.apply(&GitStderrLine::CloningInto { name: "a".into() }, &mut rec);
        tracker.apply(&GitStderrLine::CloningInto { name: "b".into() }, &mut rec);
        tracker.finish(&mut rec);

        assert_eq!(rec.started, vec![(0, "a".into()), (1, "b".into())]);
        assert_eq!(rec.finished, vec![0, 1]);
    }
}
