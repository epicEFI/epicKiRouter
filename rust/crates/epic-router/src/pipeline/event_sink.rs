//! The driver's log-only output seam: Java scatters `FRLogger.info` /
//! `FRLogger.debug` / `FRLogger.trace` rows, `job.logInfo`,
//! `router.fireTaskStateChangedEvent` and
//! `router.fireBoardUpdatedEvent` across the batch driver
//! (`BatchAutorouter`, `AutoroutePassRunner`, `AutorouteBatchLoop`).
//! None of them affect board state, so the port funnels them through
//! one sink trait — the CLI/GUI wiring (`FRLogger`'s backend, the job
//! event bus) is the host's business, and tests capture the rows
//! verbatim.

use epic_board::board::Board;

use crate::pipeline::pass_runner::{RouterCounters, render_counters};

/// The number of methods on [`DriverSink`] (the
/// forwarding-exhaustion guard's constant, M9-T3): every sink WRAPPER
/// (the tee, the session's pass tracker, the replay buffer) must
/// forward EVERY method — a future 9th method added here without
/// bumping this constant and updating the wrappers' guard pin
/// (`epic-engine` `events_stream.rs`) would be silently un-forwarded.
/// Bump this when the trait grows; the guard pin fails otherwise.
pub const DRIVER_SINK_METHOD_COUNT: usize = 8;

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
    /// progress event, carrying the TYPED [`RouterCounters`] (Java
    /// ships the object through the event too). Rendering is the
    /// sink's business: the capture sink emits the canonical
    /// [`render_counters`] row, the silent sink renders nothing
    /// (logging off). T13's manifest reads the fields without
    /// re-parsing row text (quality MINOR-2).
    fn board_updated(&mut self, _counters: &RouterCounters) {}
    /// The M9 pass-boundary snapshot hook (the tee sink's face — the
    /// GUI consumes [`BoardSnapshot`]s at pass granularity): the
    /// pipeline calls this IMMEDIATELY AFTER every
    /// [`DriverSink::board_updated`] fire, with the SAME live board
    /// the counters describe — no new boundary semantics, the hook
    /// mirrors `board_updated` exactly, and the order (counters row
    /// first, snapshot second) is pinned by the tee's golden.
    ///
    /// The default is the NO-OP every existing sink keeps: no parity
    /// stream ever grows a snapshot row, so captured/replayed streams
    /// (`BufferedSinkRow`) need no new variant and every existing
    /// capture-based pin is byte-stable across this trait extension.
    /// Only a sink that overrides this (the M9 tee) sees the calls —
    /// and the CLI never constructs one (pinned in epic-cli).
    fn board_snapshot(&mut self, _board: &Board) {}
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

    fn board_updated(&mut self, counters: &RouterCounters) {
        // The canonical row text, byte-identical to the pre-reshape
        // emissions (the counter-row pins assert these literals).
        self.rows.push(("board_updated", render_counters(counters)));
    }
}

/// One buffered sink row (the M8-T7 replay envelope): the TYPED faces
/// (`task_state` args, `board_updated` counters) are kept structured so
/// a flush can re-emit them through the real sink's typed methods — a
/// rendered-string buffer (the [`CaptureDriverSink`] shape) cannot
/// replay the typed `board_updated` face (the manifest's per-stage
/// pass counts no longer ride the counters at all — they read the
/// pipeline outcome's stage faces).
#[derive(Debug, Clone)]
pub enum BufferedSinkRow {
    /// [`DriverSink::info`].
    Info(String),
    /// [`DriverSink::warn`].
    Warn(String),
    /// [`DriverSink::debug`].
    Debug(String),
    /// [`DriverSink::trace`].
    Trace(String),
    /// [`DriverSink::task_state`].
    TaskState(String, i32, String),
    /// [`DriverSink::board_updated`].
    BoardUpdated(RouterCounters),
}

/// The per-candidate replay sink (M8-T7): records rows TYPED so the
/// coordinator can flush them to the real sink in candidate order
/// AFTER the parallel evaluation, reproducing the sequential stream
/// byte-for-byte (the reduction discards the buffers of candidates
/// beyond an early break — the sequential face never emitted them).
/// The trace gate mirrors the real sink's `is_trace_enabled`, captured
/// at pass start, so the gated emission faces are identical.
#[derive(Debug)]
pub struct BufferedDriverSink {
    /// The real sink's trace gate (captured pre-pass).
    trace_enabled: bool,
    /// Rows in emission order.
    pub rows: Vec<BufferedSinkRow>,
}

impl BufferedDriverSink {
    /// A buffer mirroring `trace_enabled`.
    #[must_use]
    pub fn new(trace_enabled: bool) -> Self {
        Self {
            trace_enabled,
            rows: Vec::new(),
        }
    }
}

impl BufferedSinkRow {
    /// Replays rows through `sink` in emission order (the M8-T7
    /// reduction's flush face, shared with [`BufferedDriverSink`]).
    pub fn flush_rows(rows: &[BufferedSinkRow], sink: &mut dyn DriverSink) {
        for row in rows {
            match row {
                BufferedSinkRow::Info(message) => sink.info(message),
                BufferedSinkRow::Warn(message) => sink.warn(message),
                BufferedSinkRow::Debug(message) => sink.debug(message),
                BufferedSinkRow::Trace(message) => sink.trace(message),
                BufferedSinkRow::TaskState(state, pass, hash) => {
                    sink.task_state(state, *pass, hash)
                }
                BufferedSinkRow::BoardUpdated(counters) => sink.board_updated(counters),
            }
        }
    }
}

impl DriverSink for BufferedDriverSink {
    fn is_trace_enabled(&self) -> bool {
        self.trace_enabled
    }

    fn info(&mut self, message: &str) {
        self.rows.push(BufferedSinkRow::Info(message.to_string()));
    }

    fn warn(&mut self, message: &str) {
        self.rows.push(BufferedSinkRow::Warn(message.to_string()));
    }

    fn debug(&mut self, message: &str) {
        self.rows.push(BufferedSinkRow::Debug(message.to_string()));
    }

    fn trace(&mut self, message: &str) {
        if self.trace_enabled {
            self.rows.push(BufferedSinkRow::Trace(message.to_string()));
        }
    }

    fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
        self.rows.push(BufferedSinkRow::TaskState(
            state.to_string(),
            pass,
            hash.to_string(),
        ));
    }

    fn board_updated(&mut self, counters: &RouterCounters) {
        self.rows
            .push(BufferedSinkRow::BoardUpdated(counters.clone()));
    }
}
