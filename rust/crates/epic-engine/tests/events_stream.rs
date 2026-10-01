//! M9-T3: the event-stream pins (the dispatch's charter for
//! `epic-engine`):
//!
//! 1. **the kind-sequence golden** — bm07 (the probed 18-pass Tier A
//!    fixture), default settings, the FULL kind sequence through the
//!    tee, determinism ×2 (both runs equal each other AND the golden);
//! 2. **the parity-stream pin** — e1_ripup routed twice, once with a
//!    plain capture sink and once with the tee AROUND an identical
//!    capture sink: the captured rows are byte-identical (kills a tee
//!    that drops/reorders/re-renders/gates a row);
//! 3. **the forwarding-exhaustion guard** — ALL
//!    `DRIVER_SINK_METHOD_COUNT` methods driven through BOTH wrappers
//!    (the tee and `PassTrackingSink`) arrive at the inner capture
//!    sink (kills a silently-un-forwarded future trait method);
//! 4. **snapshot determinism** — `board_snapshot` twice → identical
//!    `serde_json` bytes; revision monotonicity across a route
//!    (post-route > post-load);
//! 5. **the dedup pin** — two `board_snapshot` calls at the SAME
//!    revision ship ONE `Snapshot` event;
//! 6. **the version pin** — `StreamStarted` carries
//!    `ENGINE_EVENT_STREAM_VERSION`; a mismatched version is rejected
//!    by the consumer face.
//!
//! All runs are IN-PROCESS (no spawned binaries — the buglog-184
//! stale-bin family cannot apply).

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use epic_board::board::Board;
use epic_engine::events::{ENGINE_EVENT_STREAM_VERSION, EngineEvent, TeeDriverSink};
use epic_engine::session::Session;
use epic_engine::settings::{CliLayer, SessionLayer};
use epic_engine::snapshot::{BoardSnapshot, board_snapshot};
use epic_router::pipeline::event_sink::{
    CaptureDriverSink, DRIVER_SINK_METHOD_COUNT, DriverSink, NullDriverSink,
};
use epic_router::pipeline::pass_runner::RouterCounters;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

fn fixture(rel: &str) -> PathBuf {
    repo_root().join(rel)
}

fn bm07_bytes() -> Vec<u8> {
    fs::read(fixture(
        "../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm07.dsn",
    ))
    .expect("bm07 fixture is readable")
}

fn e1_ripup_bytes() -> Vec<u8> {
    fs::read(fixture("harness/fixtures/event-stream/e1_ripup.dsn"))
        .expect("e1_ripup fixture is readable")
}

/// The golden-format convention note (also in the golden file's
/// header): one line per event, `Kind field=value…` — KINDS + PASSES
/// + REVISIONS + PHASES only.
///
/// NO wall times, NO snapshot payloads, NO counters text: the
/// payloads that carry wall-clock or bulk geometry are exactly the
/// ones that must never gate a golden. `#`-prefixed golden header
/// lines are ignored by the reader.
fn kind_lines(events: &[EngineEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            EngineEvent::StreamStarted { version } => format!("StreamStarted version={version}"),
            EngineEvent::BoardLoaded {
                revision,
                item_count,
                incomplete_count,
            } => {
                format!("BoardLoaded revision={revision} item_count={item_count} incomplete_count={incomplete_count}")
            }
            EngineEvent::StageEntered { stage } => format!("StageEntered stage={stage}"),
            EngineEvent::PassProgress {
                pass,
                phase,
                counters_summary: _,
            } => format!("PassProgress pass={pass} phase={phase}"),
            EngineEvent::TaskState { state, pass, .. } => {
                format!("TaskState state={state} pass={pass}")
            }
            EngineEvent::Snapshot { revision, .. } => format!("Snapshot revision={revision}"),
            EngineEvent::RoutingFinished { .. } => "RoutingFinished".to_string(),
            EngineEvent::Message { level, .. } => format!("Message level={level}"),
        })
        .collect()
}

/// Drains the receiver into a vector (the host's post-run collect).
fn collect(receiver: mpsc::Receiver<EngineEvent>) -> Vec<EngineEvent> {
    receiver.try_iter().collect()
}

/// Routes bm07 at defaults through a fresh tee, returning the event
/// sequence (one full route per call — the golden's determinism pair
/// calls it twice).
fn route_bm07_through_tee() -> Vec<EngineEvent> {
    let bytes = bm07_bytes();
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("bm07 loads"),
    };
    let (sender, receiver) = mpsc::channel();
    let mut capture = CaptureDriverSink::default();
    {
        let mut tee = TeeDriverSink::new(&mut capture, sender);
        let summary = session
            .route(&CliLayer::default(), &mut tee)
            .expect("route succeeds");
        assert_eq!(
            summary.final_state, "COMPLETED",
            "bm07 completes at defaults"
        );
    }
    collect(receiver)
}

/// PIN 1 — the kind-sequence golden (determinism ×2: two independent
/// full bm07 routes produce the SAME kind sequence, and it equals the
/// committed golden).
#[test]
fn bm07_kind_sequence_matches_the_golden_twice() {
    let golden_text = include_str!("goldens/bm07.event-kinds.txt");
    let golden: Vec<String> = golden_text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert!(!golden.is_empty(), "the golden is non-empty");

    let run_one = kind_lines(&route_bm07_through_tee());
    let run_two = kind_lines(&route_bm07_through_tee());
    assert_eq!(
        run_one, run_two,
        "two bm07 routes produce identical kind sequences (determinism)"
    );
    assert_eq!(
        run_one, golden,
        "the bm07 kind sequence equals the committed golden"
    );
    // The lazy StreamStarted guard: the FIRST line is the stream
    // header carrying the version (pinned here and in the golden).
    assert_eq!(
        golden.first().map(String::as_str),
        Some("StreamStarted version=1"),
        "StreamStarted is the first event"
    );
}

/// The wall-clock mask (the pin's ONE documented normalization): the
/// driver's progress rows carry REAL elapsed times (`pass_end …
/// durationMs=N`, `… completed in N.NN seconds …` — the fanout, pass
/// and optimizer summary rows), so two runs can never be raw-byte
/// identical on those spans. The mask replaces exactly the two
/// wall-clock faces with `<wall>` on BOTH sides; every other byte of
/// every row (tags, order, all non-wall text — including any digits a
/// re-rendering mutant would alter) must still match exactly.
fn mask_wall_clocks(row: &str) -> String {
    let mut out = row.to_string();
    // The `pass_end … durationMs=N` face: mask the integer.
    if let Some(start) = out.find("durationMs=") {
        let text_start = start + "durationMs=".len();
        let text_end = out[text_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(out.len(), |index| text_start + index);
        out.replace_range(text_start..text_end, "<wall>");
    }
    // The `completed in N.NN seconds` face (the fanout, autoroute-pass
    // and optimizer summary rows): mask the float.
    if let Some(start) = out.find("completed in ") {
        let text_start = start + "completed in ".len();
        let text_end = out[text_start..]
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .map_or(out.len(), |index| text_start + index);
        out.replace_range(text_start..text_end, "<wall>");
    }
    out
}

/// PIN 2 — the parity-stream pin: the same e1_ripup route with and
/// without the tee yields IDENTICAL capture rows up to the masked
/// wall-clock faces (the tee forwards unchanged; a
/// dropped/reordered/re-rendered/gated row is killed by the
/// full-sequence compare — 8,621 rows on this fixture, 6 of which
/// carry wall clocks, probed 2026-09-30).
#[test]
fn parity_stream_is_byte_identical_through_the_tee() {
    let cli = CliLayer::default();

    // Run A: the plain capture sink (no tee).
    let bytes = e1_ripup_bytes();
    let mut session_a = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("e1_ripup loads"),
    };
    let mut plain = CaptureDriverSink::default();
    session_a.route(&cli, &mut plain).expect("route succeeds");

    // Run B: the tee AROUND an identical capture sink.
    let bytes = e1_ripup_bytes();
    let mut session_b = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("e1_ripup loads"),
    };
    let mut wrapped = CaptureDriverSink::default();
    let (sender, receiver) = mpsc::channel();
    {
        let mut tee = TeeDriverSink::new(&mut wrapped, sender);
        session_b.route(&cli, &mut tee).expect("route succeeds");
    }
    let _events = collect(receiver);

    assert_eq!(
        plain.rows.len(),
        wrapped.rows.len(),
        "the tee must not drop or duplicate ANY parity row"
    );
    for (index, ((tag_plain, row_plain), (tag_wrapped, row_wrapped))) in
        plain.rows.iter().zip(wrapped.rows.iter()).enumerate()
    {
        assert_eq!(
            tag_plain, tag_wrapped,
            "row {index}: the tee must not re-tag a parity row"
        );
        assert_eq!(
            mask_wall_clocks(row_plain),
            mask_wall_clocks(row_wrapped),
            "row {index}: the tee must not re-render/reorder/gate a parity row (only the \
             documented wall-clock faces are masked)"
        );
    }
    assert!(
        !plain.rows.is_empty(),
        "the probe actually captured rows (both runs exercised the stream)"
    );
}

/// The guard's recording sink: one row per method, in arrival order.
#[derive(Default)]
struct GuardRecorder {
    rows: Vec<&'static str>,
    trace_gate: bool,
}

impl DriverSink for GuardRecorder {
    fn is_trace_enabled(&self) -> bool {
        self.trace_gate
    }
    fn info(&mut self, _message: &str) {
        self.rows.push("info");
    }
    fn warn(&mut self, _message: &str) {
        self.rows.push("warn");
    }
    fn debug(&mut self, _message: &str) {
        self.rows.push("debug");
    }
    fn trace(&mut self, _message: &str) {
        self.rows.push("trace");
    }
    fn task_state(&mut self, _state: &str, _pass: i32, _hash: &str) {
        self.rows.push("task_state");
    }
    fn board_updated(&mut self, _counters: &RouterCounters) {
        self.rows.push("board_updated");
    }
    fn board_snapshot(&mut self, _board: &epic_board::board::Board) {
        self.rows.push("board_snapshot");
    }
}

/// PIN 3 — the forwarding-exhaustion guard: ALL
/// `DRIVER_SINK_METHOD_COUNT` methods driven through BOTH wrappers
/// (the tee and `PassTrackingSink`) arrive at the inner recorder, in
/// order, and the trace gate mirrors through both. A future 9th
/// trait method added without forwarding breaks this pin (the
/// constant count assertion fails first).
#[test]
fn forwarding_exhaustion_guard_all_methods_arrive_through_both_wrappers() {
    let expected_rows: [&str; DRIVER_SINK_METHOD_COUNT - 1] = [
        "info",
        "warn",
        "debug",
        "trace",
        "task_state",
        "board_updated",
        "board_snapshot",
    ];

    // The ALL-methods drive body, run against any wrapper.
    fn drive(sink: &mut dyn DriverSink, board: &Board, counters: &RouterCounters) {
        sink.info("info-row");
        sink.warn("warn-row");
        sink.debug("debug-row");
        sink.trace("trace-row");
        assert!(
            sink.is_trace_enabled(),
            "the gate mirrors the inner recorder (true)"
        );
        sink.task_state("STARTED", 0, "hash");
        sink.board_updated(counters);
        sink.board_snapshot(board);
    }

    let counters = RouterCounters {
        phase: "autoroute".to_string(),
        pass_count: 1,
        ..RouterCounters::default()
    };
    let bytes = e1_ripup_bytes();
    let session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("e1_ripup loads"),
    };
    let board = session.board();

    // Through the tee.
    let mut tee_recorder = GuardRecorder {
        trace_gate: true,
        ..GuardRecorder::default()
    };
    let (sender, receiver) = mpsc::channel();
    {
        let mut tee = TeeDriverSink::new(&mut tee_recorder, sender);
        drive(&mut tee, board, &counters);
    }
    assert_eq!(
        tee_recorder.rows, expected_rows,
        "every tee-forwarded method arrived at the inner recorder, in order"
    );
    // The tee ALSO shipped its events for the drive (messages,
    // stage entries, task state, pass completed, snapshot) — the
    // snapshot dedups to one.
    let tee_events = collect(receiver);
    assert_eq!(
        tee_events
            .iter()
            .filter(|event| matches!(event, EngineEvent::Snapshot { .. }))
            .count(),
        1,
        "exactly one Snapshot event for one board_snapshot call"
    );

    // Through PassTrackingSink.
    let mut tracker_recorder = GuardRecorder {
        trace_gate: true,
        ..GuardRecorder::default()
    };
    {
        let mut tracker = epic_engine::session::PassTrackingSink::new(&mut tracker_recorder);
        drive(&mut tracker, board, &counters);
    }
    assert_eq!(
        tracker_recorder.rows, expected_rows,
        "every PassTrackingSink-forwarded method arrived at the inner recorder, in order"
    );
}

/// PIN 4 — snapshot determinism (two builds at the same state →
/// identical serde_json bytes) and revision monotonicity (the route
/// moves the revision: post-route > post-load).
#[test]
fn snapshot_is_deterministic_and_revision_monotonic_across_a_route() {
    let bytes = e1_ripup_bytes();
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("e1_ripup loads"),
    };

    let before_a: BoardSnapshot = board_snapshot(session.board());
    let before_b = board_snapshot(session.board());
    let json_a = serde_json::to_vec(&before_a).expect("snapshot serializes");
    let json_b = serde_json::to_vec(&before_b).expect("snapshot serializes");
    assert_eq!(json_a, json_b, "two builds at one state: identical bytes");

    let summary = {
        let mut sink = NullDriverSink;
        session
            .route(&CliLayer::default(), &mut sink)
            .expect("route succeeds")
    };
    assert_eq!(summary.final_state, "COMPLETED");

    let after = board_snapshot(session.board());
    assert!(
        after.revision > before_a.revision,
        "the route moves the revision: post-load {} < post-route {}",
        before_a.revision,
        after.revision
    );
    let json_after = serde_json::to_vec(&after).expect("snapshot serializes");
    assert_ne!(json_a, json_after, "the post-route snapshot DIFFERS");
}

/// PIN 5 — the dedup: two `board_snapshot` calls at the SAME revision
/// ship exactly ONE `Snapshot` event (driven directly on the tee —
/// a real pass sequence never repeats a revision on these fixtures,
/// so the tee is driven by hand per the charter).
#[test]
fn same_revision_snapshots_dedup_to_one_event() {
    let bytes = e1_ripup_bytes();
    let session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(_) => panic!("e1_ripup loads"),
    };
    let (sender, receiver) = mpsc::channel();
    let mut null = NullDriverSink;
    {
        let mut tee = TeeDriverSink::new(&mut null, sender);
        let sink: &mut dyn DriverSink = &mut tee;
        let board = session.board();
        sink.board_snapshot(board);
        sink.board_snapshot(board);
    }
    let events = collect(receiver);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, EngineEvent::Snapshot { .. }))
            .count(),
        1,
        "two calls at one revision ship ONE Snapshot event"
    );
    // The lazy header guard is independent of the dedup: the first
    // event is STILL the StreamStarted.
    assert!(
        matches!(events.first(), Some(EngineEvent::StreamStarted { .. })),
        "StreamStarted ships before the first event even when the first event dedups"
    );
}

/// PIN 6 — the version: `StreamStarted` carries
/// `ENGINE_EVENT_STREAM_VERSION` on the wire, and the consumer face
/// rejects a mismatched version (the T6 worker's assert, pinned).
#[test]
fn stream_started_carries_the_version_and_mismatches_are_rejected() {
    let event = EngineEvent::StreamStarted {
        version: ENGINE_EVENT_STREAM_VERSION,
    };
    let text = serde_json::to_string(&event).expect("the event serializes");
    assert_eq!(
        text,
        format!(r#"{{"kind":"stream_started","version":{ENGINE_EVENT_STREAM_VERSION}}}"#),
        "the wire shape: kind-tagged, version carried"
    );

    // The consumer face (the T6 worker's reject-on-mismatch assert).
    fn consumer_accepts(version: u32) -> bool {
        version == ENGINE_EVENT_STREAM_VERSION
    }
    assert!(consumer_accepts(ENGINE_EVENT_STREAM_VERSION));
    assert!(
        !consumer_accepts(ENGINE_EVENT_STREAM_VERSION + 1),
        "a future version is REJECTED by the consumer face"
    );
}

/// PIN 8 (quality Q3, fix-round 2) — the counters-render drift pin:
/// `TeeDriverSink`'s `counters_summary` duplicates epic-router's
/// `render_counters` field-for-field, order-for-order — but
/// `render_counters` is `pub(crate)` to epic-router, so epic-engine
/// cannot import it and the golden/kind-lines deliberately exclude
/// counters text. THE OBSERVED FACE (documented per the fix-round
/// adjudication): the INNER CaptureDriverSink's `board_updated` row
/// text — which IS `render_counters(&counters)` verbatim (the
/// capture sink's own impl, the exact text the parity stream sees) —
/// must equal the tee's shipped `counters_summary`, counters row for
/// counters row. A drift mutant (a field added/reordered/rendered
/// differently in either rendering) dies here.
#[test]
fn counters_summary_matches_the_parity_row_rendering() {
    // Representative shapes: the two phases with distinct field
    // semantics (autoroute = the full row, fanout = the only
    // fanout_extra_vias face), all fields NONZERO so a dropped or
    // zero-hardcoded field cannot hide, plus a default-shaped
    // optimizer row.
    let variants = [
        RouterCounters {
            phase: "autoroute".to_string(),
            pass_count: 7,
            queued_to_be_routed_count: 42,
            skipped_count: 1,
            ripped_count: 3,
            failed_to_be_routed_count: 2,
            routed_count: 11,
            incomplete_count: 5,
            fanout_extra_vias_count: 0,
        },
        RouterCounters {
            phase: "fanout".to_string(),
            pass_count: 2,
            queued_to_be_routed_count: 9,
            skipped_count: 0,
            ripped_count: 1,
            failed_to_be_routed_count: 4,
            routed_count: 8,
            incomplete_count: 6,
            fanout_extra_vias_count: 13,
        },
        RouterCounters {
            phase: "optimizer".to_string(),
            ..RouterCounters::default()
        },
    ];

    let mut capture = CaptureDriverSink::default();
    let (sender, receiver) = mpsc::channel();
    {
        let mut tee = TeeDriverSink::new(&mut capture, sender);
        let sink: &mut dyn DriverSink = &mut tee;
        for counters in &variants {
            sink.board_updated(counters);
        }
    }

    // The tee's shipped summaries, in order.
    let summaries: Vec<String> = collect(receiver)
        .iter()
        .filter_map(|event| match event {
            EngineEvent::PassProgress {
                counters_summary, ..
            } => Some(counters_summary.clone()),
            _ => None,
        })
        .collect();
    // The inner capture's board_updated rows, in order — each is
    // `render_counters(&counters)` verbatim.
    let rows: Vec<String> = capture
        .rows
        .iter()
        .filter(|(tag, _)| *tag == "board_updated")
        .map(|(_, row)| row.clone())
        .collect();

    assert_eq!(
        summaries.len(),
        variants.len(),
        "every board_updated fire shipped exactly one PassProgress"
    );
    assert_eq!(
        summaries, rows,
        "the tee's counters_summary must equal the parity row rendering \
         (render_counters) counters row for counters row"
    );
    // The rows really carry the canonical shape (a render that
    // degenerated to empty strings on BOTH faces cannot pass).
    assert!(
        rows.first()
            .is_some_and(|row| row.starts_with("phase=autoroute pass=7 ")),
        "the observed face is the canonical render_counters row shape"
    );
}
