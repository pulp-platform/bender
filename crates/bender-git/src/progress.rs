use std::sync::OnceLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// Public API — trait + types
// ---------------------------------------------------------------------------

/// A receiver for progress updates from a git fetch.
///
/// The crate parses git's stderr internally and reports progress through this
/// sink. A sink handles exactly one fetch lifecycle: [`started`](GitProgress::started),
/// then zero or more [`progress`](GitProgress::progress) updates, then
/// [`finished`](GitProgress::finished). It can drive a progress bar, spinner,
/// log record, or any other UI element.
pub trait GitProgress: Send + 'static {
    /// Begin progress reporting for a fetch labelled `label`.
    fn started(&mut self, _label: &str) {}

    /// Update the progress percentage (0–100).
    fn progress(&mut self, _percent: u8) {}

    /// Mark the fetch as finished.
    fn finished(&mut self) {}
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
                # 1. Progress
                (?P<phase>Receiving\ objects|Resolving\ deltas|Checking\ out\ files):\s+(?P<percent>\d+)% |

                # 2. Errors
                (?P<error>fatal:.*|error:.*|remote:\ aborting.*)
            )
        ",
        )
        .expect("Invalid regex")
    });

    if let Some(caps) = re.captures(line) {
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

// ---------------------------------------------------------------------------
// Internal — phase-to-percentage mapping
// ---------------------------------------------------------------------------

/// Individual git transfer phase.
pub(crate) enum Phase {
    Receiving,
    Resolving,
    Checkout,
}

/// Convert a phase-local percentage (0–100) to a fetch-local percentage.
pub(crate) fn map_progress(phase: Phase, percent: u8) -> u8 {
    let percent = percent as u16;
    match phase {
        Phase::Receiving => (percent * 70 / 100) as u8,
        Phase::Resolving => (70 + percent * 30 / 100) as u8,
        Phase::Checkout => 100,
    }
}

// ---------------------------------------------------------------------------
// Stderr handlers and draining
// ---------------------------------------------------------------------------

/// Build a line handler for fetch progress.
pub(crate) fn on_fetch_progress<P>(progress: &mut P) -> impl FnMut(&str) + '_
where
    P: GitProgress,
{
    move |line| match parse_git_line(line) {
        GitStderrLine::Receiving { percent } => {
            progress.progress(map_progress(Phase::Receiving, percent));
        }
        GitStderrLine::Resolving { percent } => {
            progress.progress(map_progress(Phase::Resolving, percent));
        }
        GitStderrLine::Checkout { percent } => {
            progress.progress(map_progress(Phase::Checkout, percent));
        }
        _ => {}
    }
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
        assert_eq!(map_progress(Phase::Receiving, 0), 0);
        assert_eq!(map_progress(Phase::Receiving, 50), 35);
        assert_eq!(map_progress(Phase::Receiving, 100), 70);
        assert_eq!(map_progress(Phase::Resolving, 0), 70);
        assert_eq!(map_progress(Phase::Resolving, 100), 100);
    }
}
