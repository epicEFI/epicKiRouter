//! The M9-T3 versioned [`EngineEvent`] stream + the [`TeeDriverSink`]
//! (design §7: "board snapshots arrive via a versioned engine event
//! stream"). The ONE stream law (the M9 plan's opening rule): the tee
//! forwards every [`DriverSink`] row to the inner sink UNCHANGED —
//! `is_trace_enabled` mirrors the inner sink exactly (the
//! `BufferedDriverSink` gate-mirroring precedent,
//! `event_sink.rs:140-141`/`:164-177`), and no event emission ever
//! reorders, re-renders, or gates a parity row. The proof faces are
//! the standing events golden (3,520 rows), the seven compares, and
//! the `events_stream.rs` parity pin (the same route run with and
//! without the tee → byte-identical capture rows).
//!
//! ## The host layering (documented per the T3 charter)
//!
//! The SESSION stays untouched (except its sink wrapper's snapshot
//! forwarding): the HOST composes the stream. The host builds the
//! channel and the tee around its own inner sink, hands the tee to
//! [`crate::session::Session::route`] as the `sink` argument, and
//! emits the two events the pipeline cannot see — the load event
//! AFTER [`crate::session::Session::load_dsn`] and the finish event
//! from the returned [`crate::session::RouteSummary`] + statistics:
//!
//! ```text
//! let (sender, receiver) = std::sync::mpsc::channel();
//! // The host keeps a CLONE for its own events (BoardLoaded /
//! // RoutingFinished are typed faces the tee cannot see):
//! let host_sender = sender.clone();
//! let mut capture = CaptureDriverSink::default();          // or any DriverSink
//! let mut tee = TeeDriverSink::new(&mut capture, sender);
//! // pre-route, on host_sender: BoardLoaded { revision, item_count, incomplete_count }
//! let summary = session.route(&CliLayer::default(), &mut tee).expect("route succeeds");
//! // post-route, on host_sender: RoutingFinished { final_state, incomplete_count, violations }
//! // then drain receiver → the GUI's event queue (T6's worker is the real consumer).
//! ```
//!
//! `StreamStarted` is emitted LAZILY on the first shipped event (the
//! documented choice): it needs no host discipline to stay first —
//! every ship path funnels through one guard — and it makes the
//! golden deterministic without a separate start call. The golden
//! pins that it IS the first line. SCOPE (quality Q2): the guard
//! orders the TEE'S OWN events — a host event emitted on a pre-move
//! CLONE (e.g. `BoardLoaded` before `route()`) legitimately precedes
//! it in the channel; the enum doc's first-ness claim is
//! tee-relative, not channel-relative. The T6 ordering decision (a
//! public `TeeDriverSink::start()` the host calls before its own
//! emissions — forcing header-first across the whole channel) is
//! BANKED as a T6 charter input; no API change this round.
//!
//! ## The message-coverage note (documented per the charter)
//!
//! `info`/`warn` rows ship as [`EngineEvent::Message`]; `debug` and
//! `trace` rows are FORWARDED but never become events: they are the
//! voluminous parity-compare rows (the 3,520-row stream is mostly
//! trace), not GUI-consumed content — shipping them would flood the
//! channel with rows the view layer never reads.

use std::collections::BTreeSet;
use std::sync::mpsc::Sender;

use epic_board::board::Board;
use epic_router::pipeline::event_sink::DriverSink;
use epic_router::pipeline::pass_runner::RouterCounters;
use serde::Serialize;

use crate::snapshot::{BoardSnapshot, board_snapshot};

/// The engine event stream's schema version (DNR-19 derivation note,
/// IN CODE per the charter): schema-semver-style MAJOR, bumped on any
/// BREAKING shape change of [`EngineEvent`] (a field rename, a
/// variant removal, a payload type change); additive variants bump
/// nothing. Carried by [`EngineEvent::StreamStarted`]; consumers
/// reject mismatches (the T6 worker asserts it — pinned by
/// `events_stream.rs`'s version pin).
pub const ENGINE_EVENT_STREAM_VERSION: u32 = 1;

/// The GUI-consumable engine event (the plan's shape, verbatim).
///
/// `#[allow(clippy::large_enum_variant)]` (M9-T5): the T5 overlay
/// field grew `BoardSnapshot` past the variant-size threshold vs
/// `PassProgress`. Boxing `Snapshot`'s payload would keep the lint
/// quiet at zero wire cost (serde erases the box) but changes the
/// T3-landed match surface for every consumer; the size is ACCEPTED,
/// documented — the tee already funnels whole-board snapshots by
/// design (the AM3 T6 mpsc-drain-policy note owns the memory side).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    /// The stream header — the FIRST event the tee itself ships (the
    /// lazy-ship guard; quality Q2: a host event sent on a pre-move
    /// clone may legitimately precede it in the channel — the T6
    /// ordering decision is banked, see the module docs), carrying
    /// [`ENGINE_EVENT_STREAM_VERSION`].
    StreamStarted {
        /// The schema version (see the constant's derivation note).
        version: u32,
    },
    /// The host's post-load event (the layering note in the module
    /// docs — the tee cannot see the load face).
    BoardLoaded {
        /// `board.revision()` after the load prelude.
        revision: u64,
        /// `board.item_count()`.
        item_count: usize,
        /// The pre-existing violation seed.
        incomplete_count: i64,
    },
    /// Derived: the FIRST sighting of a counters phase (`fanout` /
    /// `autoroute` / `optimizer`) or a task-state string. Ordering:
    /// the stage-entered event precedes the event that first sighted
    /// the stage (pinned by the golden).
    StageEntered {
        /// The phase/state string.
        stage: String,
    },
    /// Typed from every `board_updated` fire (all six pipeline
    /// sites — the pass-begin, pass-finish, fanout and optimizer
    /// rows alike; spec-review D4 renamed this from `PassCompleted`:
    /// the fire happens at pass STARTS too (the golden's doubled
    /// `pass=1 phase=fanout` rows are begin + finish), so the old
    /// name overstated the semantics; a boundary field is not viable
    /// (the tee sees only `&RouterCounters` — no boundary tag exists
    /// to carry), and version 1 has zero consumers, so the rename is
    /// free now and costly at T6).
    PassProgress {
        /// `counters.pass_count`.
        pass: i32,
        /// `counters.phase`.
        phase: String,
        /// The typed counters rendered to a deterministic compact
        /// row (the tee's own rendering — `render_counters` is
        /// `pub(crate)` to epic-router; the golden excludes this
        /// text).
        counters_summary: String,
    },
    /// Typed from every `task_state` fire.
    TaskState {
        /// The state string (`STARTED`, …).
        state: String,
        /// The pass number.
        pass: i32,
        /// The board hash at the fire.
        hash: String,
    },
    /// The pass-boundary snapshot (worker-built here in the tee —
    /// the design's "snapshots are built on the worker side and
    /// travel in events"). Shipped ONLY when the board revision
    /// changed since the last shipped snapshot (the dedup, pinned).
    Snapshot {
        /// The snapshot's revision.
        revision: u64,
        /// The snapshot itself.
        snapshot: BoardSnapshot,
    },
    /// The host's post-route event (the layering note).
    RoutingFinished {
        /// `RouteSummary::final_state`.
        final_state: String,
        /// `RouteSummary::incomplete_count`.
        incomplete_count: i64,
        /// `RouteSummary::violations_total`.
        violations: i64,
    },
    /// An `info`/`warn` row (the coverage note — debug/trace stay
    /// parity-only).
    Message {
        /// `"info"` or `"warn"`.
        level: String,
        /// The row text, verbatim.
        text: String,
    },
}

// The T6 worker ships events across threads (the charter's compile
// assert): an `EngineEvent` must stay `Send`.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<EngineEvent>();
};

/// The counters rendering of [`EngineEvent::PassProgress`] — the
/// tee's own deterministic compact row (`render_counters` is
/// `pub(crate)` to epic-router, so the tee renders the same fields
/// in the same order itself; the golden format excludes this text).
fn counters_summary(counters: &RouterCounters) -> String {
    format!(
        "phase={} pass={} queued={} skipped={} ripped={} failed={} routed={} \
         incomplete={} fanout_extra_vias={}",
        counters.phase,
        counters.pass_count,
        counters.queued_to_be_routed_count,
        counters.skipped_count,
        counters.ripped_count,
        counters.failed_to_be_routed_count,
        counters.routed_count,
        counters.incomplete_count,
        counters.fanout_extra_vias_count
    )
}

/// The tee sink: forwards EVERY [`DriverSink`] method to the inner
/// sink UNCHANGED (the stream law) and additionally ships the typed
/// [`EngineEvent`]s into the channel. Constructed ONLY by the host
/// (the CLI never builds one — pinned in epic-cli's inertia test).
pub struct TeeDriverSink<'a> {
    /// The parity sink (every row lands here first, byte-verbatim).
    inner: &'a mut dyn DriverSink,
    /// The event channel's producer half.
    sender: Sender<EngineEvent>,
    /// The lazy [`EngineEvent::StreamStarted`] guard.
    started: bool,
    /// The seen stages ([`EngineEvent::StageEntered`] derivation) —
    /// a `BTreeSet` so the derivation is order-stable in source.
    seen_stages: BTreeSet<String>,
    /// The revision of the last SHIPPED snapshot (the dedup key).
    last_snapshot_revision: Option<u64>,
}

impl<'a> TeeDriverSink<'a> {
    /// Wraps `inner` (the parity sink stays in charge of every row)
    /// and `sender` (the GUI's channel producer half).
    pub fn new(inner: &'a mut dyn DriverSink, sender: Sender<EngineEvent>) -> Self {
        Self {
            inner,
            sender,
            started: false,
            seen_stages: BTreeSet::new(),
            last_snapshot_revision: None,
        }
    }

    /// The one ship funnel: the lazy `StreamStarted` guard, then the
    /// event. A closed receiver (the host dropped it) drops the
    /// event silently — the stream is best-effort by contract, the
    /// parity rows are the guaranteed face.
    fn ship(&mut self, event: EngineEvent) {
        if !self.started {
            self.started = true;
            let _ = self.sender.send(EngineEvent::StreamStarted {
                version: ENGINE_EVENT_STREAM_VERSION,
            });
        }
        let _ = self.sender.send(event);
    }
}

impl DriverSink for TeeDriverSink<'_> {
    fn info(&mut self, message: &str) {
        self.inner.info(message);
        self.ship(EngineEvent::Message {
            level: "info".to_string(),
            text: message.to_string(),
        });
    }

    fn warn(&mut self, message: &str) {
        self.inner.warn(message);
        self.ship(EngineEvent::Message {
            level: "warn".to_string(),
            text: message.to_string(),
        });
    }

    fn debug(&mut self, message: &str) {
        // Forwarded unchanged; NOT an event (the coverage note).
        self.inner.debug(message);
    }

    fn trace(&mut self, message: &str) {
        // Forwarded unchanged; NOT an event (the coverage note —
        // the voluminous compare rows stay parity-only).
        self.inner.trace(message);
    }

    fn is_trace_enabled(&self) -> bool {
        // The gate-mirroring law (event_sink.rs:140-141 precedent):
        // the inner sink's gate IS the tee's gate.
        self.inner.is_trace_enabled()
    }

    fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
        self.inner.task_state(state, pass, hash);
        if self.seen_stages.insert(state.to_string()) {
            self.ship(EngineEvent::StageEntered {
                stage: state.to_string(),
            });
        }
        self.ship(EngineEvent::TaskState {
            state: state.to_string(),
            pass,
            hash: hash.to_string(),
        });
    }

    fn board_updated(&mut self, counters: &RouterCounters) {
        self.inner.board_updated(counters);
        if self.seen_stages.insert(counters.phase.clone()) {
            self.ship(EngineEvent::StageEntered {
                stage: counters.phase.clone(),
            });
        }
        self.ship(EngineEvent::PassProgress {
            pass: counters.pass_count,
            phase: counters.phase.clone(),
            counters_summary: counters_summary(counters),
        });
    }

    fn board_snapshot(&mut self, board: &Board) {
        // Forward FIRST (the counters-row-then-snapshot order is the
        // hook's contract; the inner sink sees exactly what a
        // tee-less run would see), then the deduped event.
        self.inner.board_snapshot(board);
        let revision = board.revision();
        if self.last_snapshot_revision == Some(revision) {
            return; // the dedup: same revision → no second snapshot.
        }
        self.last_snapshot_revision = Some(revision);
        self.ship(EngineEvent::Snapshot {
            revision,
            snapshot: board_snapshot(board),
        });
    }
}
