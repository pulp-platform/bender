/// The kind of network operation being performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressOp {
    Fetch,
    Clone,
    Checkout,
}

/// A receiver for incremental progress updates from a git operation.
///
/// All methods have default no-op implementations. Consumers that don't need
/// progress can pass [`NoProgress`] to any function that accepts this trait.
///
/// # Integration note
///
/// When integrating `bender-git` into bender, the existing `ProgressHandler`
/// in `src/progress.rs` should implement this trait so that progress bars
/// continue to work without changes to the UI layer.
///
/// Full progress parsing from git's stderr output is intentionally deferred
/// for the initial implementation. The trait boundary is established here so
/// that the API does not need to change later.
pub trait GitProgressSink: Send + 'static {
    /// Called once when the operation begins.
    fn operation_started(&mut self, _op: ProgressOp, _label: &str) {}

    /// Called repeatedly during object transfer (0–100).
    fn receiving_objects(&mut self, _percent: u8) {}

    /// Called during delta resolution (0–100).
    fn resolving_deltas(&mut self, _percent: u8) {}

    /// Called during working tree checkout (0–100).
    fn checking_out_files(&mut self, _percent: u8) {}

    /// Called once when the operation completes successfully.
    fn operation_finished(&mut self) {}

    /// Called if the operation fails, before the error is propagated.
    fn operation_failed(&mut self, _reason: &str) {}
}

/// A no-op progress sink for callers that don't need progress reporting.
pub struct NoProgress;

impl GitProgressSink for NoProgress {}
