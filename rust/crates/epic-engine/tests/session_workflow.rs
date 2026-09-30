//! M9-T2: the `Session` workflow pins (the dispatch's charter for
//! `epic-engine` integration tests):
//!
//! (a) the `LoadError` arms — a parse-broken DSN (hard `Parse`), an
//!     outline-missing DSN (warn-and-continue: the session is handed
//!     BACK via `LoadError::OutlineMissing`), and the load SUCCESS
//!     face. The `Io` arm is pinned by direct construction in
//!     `session.rs`'s unit tests — the port's reader reads in-memory
//!     bytes and NEVER constructs `DsnReadResult::IoError` (epic-dsn
//!     reader.rs module docs), so no byte input can reach it (the
//!     dispatch's "bad path is CLI-only" note).
//! (b) cancel: `request_cancel` before pass 1 ⇒ final_state
//!     CANCELLED, and `export_ses` refuses (the route.rs:1385-1399
//!     gate — only COMPLETED/TIMED_OUT produce a file).
//! (c) session-layer override precedence: CLI layer default (all
//!     None) + `SessionLayer { max_passes: Some(1) }` ⇒
//!     `RouteSummary.passes == 1` on a MULTI-PASS fixture, vs the
//!     same fixture at defaults ⇒ passes > 1 (probed 2026-09-30 at
//!     `4a0d7b3f5` + T2: bm07 routes 18 passes at defaults, 1
//!     incomplete, COMPLETED — the cheapest Tier A multi-pass face;
//!     bm08/e1_ripup/ecc83-pp all complete in ONE pass). Plus the
//!     runtime precedence face: a CLI `max_passes` opinion LOSES to
//!     the session layer's (the T1 slot merges above the CLI).
//! (d) a load/statistics shape pin: revision > 0 post-load, the
//!     read-only `board()` face works, the pre-route statistics are
//!     the documented empty face.
//!
//! All runs are IN-PROCESS (no spawned binaries — the buglog-184
//! stale-bin family cannot apply; the SPAWNED parity face lives in
//! `rust/harness/tests/session_parity_pin.rs`).

use std::fs;
use std::path::PathBuf;

use epic_engine::session::{LoadError, Session};
use epic_engine::settings::{CliLayer, SessionLayer};
use epic_router::pipeline::event_sink::DriverSink;

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

/// A tiny recorder sink (assertion-free; the pins read the summaries).
#[derive(Default)]
struct SilentSink;

impl DriverSink for SilentSink {}

/// (a) The parse arm: a header-broken DSN is a hard `LoadError::Parse`
/// with the CLI's error text shape — never a panic, never a partial
/// session.
#[test]
fn load_parse_error_is_a_hard_error() {
    // A session-file header is NOT a DSN header: the `(pcb` dispatch
    // fails at keyword 2 (the ses_lexer_no_panic face — an `.ses` is
    // not a DSN, ParseError). A bare `(pcb` is NOT broken enough: the
    // header scan tolerates a missing name token and the empty scope
    // walk SUCCEEDS (an empty board) — probed live 2026-09-30.
    let loaded = Session::load_dsn(b"(session \"probe\" (place))", SessionLayer::default());
    match loaded {
        Err(LoadError::Parse(detail)) => {
            assert!(
                detail.contains("parse error"),
                "the CLI-shaped parse text, got: {detail}"
            );
        }
        Err(LoadError::OutlineMissing(_)) => {
            panic!("a parse-broken DSN must not load, even warn-and-continue");
        }
        Err(LoadError::Io(detail)) => panic!("unexpected Io arm: {detail}"),
        Ok(_) => panic!("a parse-broken DSN must not load"),
    }
}

/// (a) The outline-missing arm: the session is handed BACK
/// (warn-and-continue — the CLI proceeds with the default boundary),
/// and the loaded board is live (revision ticks, the read-only face).
#[test]
fn load_outline_missing_returns_the_session() {
    // The crafted world MINUS its boundary scope (the reader classifies
    // `boardOutlineOk == false` -> OutlineMissing).
    let bytes = fs::read(fixture("harness/fixtures/event-stream/e1_ripup.dsn"))
        .expect("the ripup fixture reads");
    let text = String::from_utf8(bytes).expect("the fixture is utf-8");
    let boundary =
        "    (boundary\n      (path pcb 0  0 0  120000 0  120000 64000  0 64000  0 0)\n    )\n";
    assert!(
        text.contains(boundary),
        "the boundary block anchor must exist (fixture changed?)"
    );
    let outline_missing = text.replace(boundary, "");
    let loaded = Session::load_dsn(outline_missing.as_bytes(), SessionLayer::default());
    let session = match loaded {
        Err(LoadError::OutlineMissing(session)) => *session,
        Err(LoadError::Parse(detail)) => panic!("outline-missing is NOT a parse error: {detail}"),
        Err(LoadError::Io(detail)) => panic!("unexpected Io arm: {detail}"),
        Ok(_) => panic!("a boundary-less DSN must land the OutlineMissing arm"),
    };
    // The arm's contract is the HAND-BACK (warn-and-continue at LOAD,
    // the route.rs:1137-1143 face). The board carries NO outline item
    // (the relocated override's own no-outline skip arm fired at load
    // — its warning row is on stderr) and no built boundary, so the
    // board's mutation tick still sits at ZERO; routing it is NOT
    // pinned here because the port's post-load flow cannot run it:
    // the geometry pass's bounding-box invariant
    // (settings.rs `a built board always has a bounding box`) PANICS
    // on a never-create_board'd board — the CLI does EXACTLY the same
    // (probed live 2026-09-30: `epic-cli route` on this input exits
    // 101 at the same line), so the session mirrors the CLI face by
    // NOT special-casing it.
    let _board = session.board();
}

/// (a/d) The success arm + the load/statistics shape: revision > 0
/// post-load, `board()` readable, the pre-route statistics are the
/// documented empty face, warnings empty on a clean load, and the
/// export gate refuses BEFORE any route run (no final state yet).
#[test]
fn load_success_shape_and_pre_route_faces() {
    let bytes = fs::read(fixture("harness/fixtures/event-stream/e1_ripup.dsn"))
        .expect("the ripup fixture reads");
    let mut session =
        Session::load_dsn(&bytes, SessionLayer::default()).expect("the fixture loads clean");
    assert!(session.board_revision() > 0, "revision ticks at load");
    assert_eq!(
        session.board_revision(),
        session.board().revision(),
        "the session tick mirrors the board's"
    );
    assert!(
        session.warnings().is_empty(),
        "a clean load carries no warnings: {:?}",
        session.warnings()
    );
    // The pre-route statistics: the documented EMPTY face (the walk
    // needs &mut; it runs at route time).
    let stats = session.statistics();
    assert_eq!(stats.incomplete_count, 0);
    assert_eq!(stats.violations_total, 0);
    assert_eq!(stats.stats.layers_total_count, 0, "new_empty before a run");
    // The export gate with NO route run: refused, nothing on disk.
    let scratch =
        std::env::temp_dir().join(format!("session_workflow_no_route_{}", std::process::id()));
    let out = scratch.join("never.ses");
    let refused = session.export_ses(&out);
    assert!(refused.is_err(), "no route run -> no session file");
    assert!(!out.exists(), "the refused export writes NOTHING");
    let _ = fs::remove_dir_all(&scratch);
}

/// (b) Cancel: the shared flag raised MID-RUN (the T12
/// `StopOnPassOneFillSink` pattern — a sink that stores `true` on the
/// FIRST `board_updated` row, before the autoroute pass finishes) ⇒
/// CANCELLED, and `export_ses` refuses with the state named. The
/// pre-route raise is a DIFFERENT face (the pipeline's pre-stage
/// check skips the routing stage: pass-through COMPLETED — pinned in
/// `pre_route_cancel_passes_through_completed` below).
#[test]
fn cancel_mid_run_cancels_and_refuses_export() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use epic_router::pipeline::pass_runner::RouterCounters;

    /// Raises the session's shared flag on the first counters row —
    /// the host-side cancel button's exact shape (the flag Arc is the
    /// one the pipeline's StopFace polls).
    struct CancelOnFirstRowSink {
        flag: Arc<AtomicBool>,
        fired: bool,
    }
    impl DriverSink for CancelOnFirstRowSink {
        fn board_updated(&mut self, _counters: &RouterCounters) {
            if !self.fired {
                self.fired = true;
                self.flag.store(true, Ordering::Relaxed);
            }
        }
    }

    let bytes = fs::read(fixture("harness/fixtures/event-stream/e1_ripup.dsn"))
        .expect("the ripup fixture reads");
    let mut session =
        Session::load_dsn(&bytes, SessionLayer::default()).expect("the fixture loads clean");
    let mut sink = CancelOnFirstRowSink {
        flag: session.stop_flag(),
        fired: false,
    };
    let summary = session.route(&CliLayer::default(), &mut sink);
    assert_eq!(
        summary.final_state, "CANCELLED",
        "a MID-RUN external stop is the CANCELLED face"
    );
    assert_eq!(summary.passes, 0, "the stop fired before pass 1 finished");
    // The export gate: CANCELLED produces NO file (route.rs:1385-1399).
    let scratch =
        std::env::temp_dir().join(format!("session_workflow_cancel_{}", std::process::id()));
    let out = scratch.join("cancelled.ses");
    let refused = session.export_ses(&out);
    match refused {
        Err(message) => assert!(
            message.contains("CANCELLED"),
            "the refusal names the final state: {message}"
        ),
        Ok(()) => panic!("CANCELLED must not produce a session file"),
    }
    assert!(!out.exists(), "the refused export writes NOTHING");
    let _ = fs::remove_dir_all(&scratch);
}

/// The pre-route cancel face, PINNED (spec-review F1+F2): a raise
/// BEFORE `route()` hits the pipeline's pre-stage stop check (full.rs
/// `router_enabled && !stop.is_requested()`) — the routing stage is
/// SKIPPED (pass-through `Ok(true)`, no stop reason), so the run
/// lands COMPLETED with zero passes and the UNROUTED board stands in
/// the statistics. Also `request_cancel`'s exercised-caller pin (its
/// mid-run sibling raises through `stop_flag()` instead).
#[test]
fn pre_route_cancel_passes_through_completed() {
    let bytes = fs::read(fixture("harness/fixtures/event-stream/e1_ripup.dsn"))
        .expect("the ripup fixture reads");
    let mut session =
        Session::load_dsn(&bytes, SessionLayer::default()).expect("the fixture loads clean");
    session.request_cancel();
    let mut sink = SilentSink;
    let summary = session.route(&CliLayer::default(), &mut sink);
    assert_eq!(
        summary.final_state, "COMPLETED",
        "a stop raised BEFORE route() skips the routing stage (pass-through Ok(true), no \
         stop reason) — raise MID-RUN for CANCELLED"
    );
    assert_eq!(
        summary.passes, 0,
        "the routing stage never ran: no board_updated rows reached the sink"
    );
    assert!(
        summary.incomplete_count > 0,
        "the unrouted board stands (the statistics walk still ran)"
    );
}

/// (c) The session-layer override is OUTPUT-OBSERVABLE: on bm07 (the
/// probed multi-pass Tier A face — 18 passes at defaults), the
/// session layer's `max_passes = 1` caps the run at exactly one pass,
/// beating BOTH the default and an explicit CLI opinion (the T1 slot
/// merges above the CLI layer).
#[test]
fn session_layer_max_passes_override_is_output_observable() {
    let dsn = fixture("../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm07.dsn");
    let bytes = fs::read(&dsn).expect("bm07 reads");

    // Defaults: the multi-pass face (probed: 18 passes, 1 incomplete,
    // COMPLETED — the pin only demands > 1 so a future engine change
    // in pass count does not rot the pin).
    let mut session = Session::load_dsn(&bytes, SessionLayer::default()).expect("bm07 loads clean");
    let mut sink = SilentSink;
    let defaults = session.route(&CliLayer::default(), &mut sink);
    assert_eq!(defaults.final_state, "COMPLETED");
    assert!(
        defaults.passes > 1,
        "bm07 at defaults must be MULTI-pass for this pin (got {})",
        defaults.passes
    );
    assert_eq!(defaults.incomplete_count, 1, "the probed bm07 face");
    assert!(
        session.pre_existing_violations() >= 0,
        "the pre-existing seed is exposed"
    );

    // The override: a FRESH session (the first run consumed its
    // board) with max_passes = 1 rides exactly one pass.
    let session_layer = SessionLayer {
        max_passes: Some(1),
        ..SessionLayer::default()
    };
    let mut session = Session::load_dsn(&bytes, session_layer).expect("bm07 loads clean");
    let mut sink = SilentSink;
    let capped = session.route(&CliLayer::default(), &mut sink);
    assert_eq!(capped.passes, 1, "the session layer's cap bit");
    assert_eq!(
        capped.final_state, "COMPLETED",
        "an internal stop is COMPLETED"
    );
    assert!(
        capped.incomplete_count > defaults.incomplete_count,
        "the cap left MORE work undone than the defaults run"
    );

    // The runtime precedence face: session 1 beats CLI 5.
    let session_layer = SessionLayer {
        max_passes: Some(1),
        ..SessionLayer::default()
    };
    let mut session = Session::load_dsn(&bytes, session_layer).expect("bm07 loads clean");
    let cli = CliLayer {
        max_passes: Some(5),
        ..CliLayer::default()
    };
    let mut sink = SilentSink;
    let beats_cli = session.route(&cli, &mut sink);
    assert_eq!(
        beats_cli.passes, 1,
        "the session layer merges ABOVE the CLI"
    );
}
