//! The driver's log-only output seam: Java scatters `FRLogger.info` /
//! `FRLogger.debug` / `FRLogger.trace` rows, `job.logInfo`,
//! `router.fireTaskStateChangedEvent` and
//! `router.fireBoardUpdatedEvent` across the batch driver
//! (`BatchAutorouter`, `AutoroutePassRunner`, `AutorouteBatchLoop`).
//! None of them affect board state, so the port funnels them through
//! one sink trait — the CLI/GUI wiring (`FRLogger`'s backend, the job
//! event bus) is the host's business, and tests capture the rows
//! verbatim.

/// The batch driver's event sink (Java's log + event surfaces).
pub trait DriverSink {
    /// `job.logInfo` / `FRLogger.info`.
    fn info(&mut self, _message: &str) {}
    /// `FRLogger.warn` — the driver's one warning face (the
    /// disabled-layers abort, `AutorouteBatchLoop.java:53`).
    fn warn(&mut self, _message: &str) {}
    /// `job.logDebug` / `FRLogger.debug`.
    fn debug(&mut self, _message: &str) {}
    /// `FRLogger.trace(method, operation, message, …)` — the
    /// `compare_trace_*` / `compare_unrouted_*` rows.
    fn trace(&mut self, _message: &str) {}
    /// `FRLogger.isTraceEnabled()` — gates the pass runner's expensive
    /// per-item comparison rows (Java `AutoroutePassRunner:251`); the
    /// default (silent sink) keeps the headless path free of the
    /// per-item DRC walks the gated rows carry.
    fn is_trace_enabled(&self) -> bool {
        false
    }
    /// `router.fireTaskStateChangedEvent(new TaskStateChangedEvent(
    /// router, taskState, currentPass, hash))`.
    fn task_state(&mut self, _state: &str, _pass: i32, _hash: &str) {}
    /// `router.fireBoardUpdatedEvent(stats, counters, board)` — the
    /// progress event; the counters are rendered into one row.
    fn board_updated(&mut self, _message: &str) {}
}

/// The silent sink (the headless default — Java with logging off).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullDriverSink;

impl DriverSink for NullDriverSink {}

/// The capture sink: every row is stored with its level tag, in
/// emission order — the pin battery's observer.
#[derive(Debug, Default)]
pub struct CaptureDriverSink {
    /// `("level", row)` pairs in emission order.
    pub rows: Vec<(&'static str, String)>,
}

impl CaptureDriverSink {
    /// The rows of one level, joined with newlines.
    #[must_use]
    pub fn joined(&self, level: &'static str) -> String {
        self.rows
            .iter()
            .filter(|(tag, _)| *tag == level)
            .map(|(_, row)| row.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether any row of any level contains `needle`.
    #[must_use]
    pub fn any_contains(&self, needle: &str) -> bool {
        self.rows.iter().any(|(_, row)| row.contains(needle))
    }
}

impl DriverSink for CaptureDriverSink {
    fn is_trace_enabled(&self) -> bool {
        true
    }

    fn info(&mut self, message: &str) {
        self.rows.push(("info", message.to_string()));
    }

    fn warn(&mut self, message: &str) {
        self.rows.push(("warn", message.to_string()));
    }

    fn debug(&mut self, message: &str) {
        self.rows.push(("debug", message.to_string()));
    }

    fn trace(&mut self, message: &str) {
        self.rows.push(("trace", message.to_string()));
    }

    fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
        self.rows.push((
            "task_state",
            format!("task_state state={state} pass={pass} hash={hash}"),
        ));
    }

    fn board_updated(&mut self, message: &str) {
        self.rows.push(("board_updated", message.to_string()));
    }
}
