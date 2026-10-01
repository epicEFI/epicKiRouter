//! M9-T5: the overlay DATA faces' engine pins:
//!
//! 1. **the tee empty-default pin** — a tee-driven (pass-granular)
//!    snapshot carries the EMPTY [`OverlayData`] default (the purity
//!    law: the pure `board_snapshot(&Board)` builder cannot run the
//!    `&mut` DRC faces — the documented honest cadence);
//! 2. **the Parse-boundary attach faces** — the drc craft's
//!    `snapshot_with_overlays` pre-route: markers present, ALL
//!    tagged `Parse`, airlines present, tuning/congestion `None`
//!    (constraint-free + never-routed), determinism (two calls,
//!    byte-equal JSON);
//! 3. **the PostRoute faces + the statistics cross-check** — bm08
//!    routed at defaults: every marker `PostRoute`, the airline
//!    count equals the `BoardStatistics` incomplete face at the same
//!    boundary, congestion/tuning `None` (default settings);
//! 4. **the congestion Some-iff-engaged pin** — the probe's reachable
//!    ON path (`SessionLayer.congestion_global = Some(true)`) yields
//!    `Some` with a non-empty grid, the default-settings session
//!    yields `None` (the contrast in one pin);
//! 5. **the tuning face** — the crafted tuning fixture resolves
//!    `Some` (constraints declared), one info per constrained net
//!    with a positive actual length; the constraint-free craft stays
//!    `None` (the contrast).
//!
//! All runs are IN-PROCESS (no spawned binaries).

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use epic_drc::incompletes::all_incompletes;
use epic_engine::events::{EngineEvent, TeeDriverSink};
use epic_engine::session::Session;
use epic_engine::settings::{CliLayer, SessionLayer};
use epic_engine::snapshot::OverlayData;
use epic_router::pipeline::event_sink::{CaptureDriverSink, DriverSink};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("rust/ is two parents up from crates/epic-engine")
        .to_path_buf()
}

const DRC_CRAFT: &str = "harness/corpus/craft/drc-main.dsn";
const BM08: &str = "../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn";
const TUNING: &str = "harness/fixtures/tuning/min_stair_tuning.dsn";

fn read_fixture(rel: &str) -> Vec<u8> {
    fs::read(repo_root().join(rel)).unwrap_or_else(|error| panic!("{rel} is readable: {error}"))
}

/// A silent sink (the pins never assert on parity rows).
struct SilentSink;

impl DriverSink for SilentSink {}

// --- PIN 1 — the tee empty-default ------------------------------------

/// The tee's `board_snapshot` hook ships the PURE builder's snapshot:
/// `overlays == OverlayData::default()` (the purity law, pinned — a
/// tee that starts attaching `&mut`-computed overlays would flood
/// every pass boundary with DRC walks AND break the parity posture's
/// honest cadence).
#[test]
fn tee_snapshots_carry_the_empty_overlay_default() {
    let bytes = read_fixture(BM08);
    let session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("bm08 loads: {error:?}"),
    };
    let (sender, receiver) = mpsc::channel();
    let mut capture = CaptureDriverSink::default();
    {
        let mut tee = TeeDriverSink::new(&mut capture, sender);
        tee.board_snapshot(session.board());
    }
    // The tee ships LAZILY (StreamStarted first — the documented
    // lazy-ship guard); drain past it.
    let mut snapshot = None;
    for _ in 0..2 {
        let event = receiver.recv().expect("events arrive");
        if let EngineEvent::Snapshot {
            snapshot: inner, ..
        } = event
        {
            snapshot = Some(inner);
            break;
        }
    }
    let snapshot = snapshot.expect("the Snapshot event arrives");
    assert_eq!(
        snapshot.overlays,
        OverlayData::default(),
        "the tee-driven snapshot carries the EMPTY overlay default"
    );
}

// --- PIN 2 — the Parse-boundary attach faces --------------------------

/// The drc craft (a probed >= 1 violation bearer): markers present and
/// ALL `Parse` pre-route; airlines present; tuning `None` (the craft
/// declares no length constraints) and congestion `None` (never
/// routed); two calls byte-equal (determinism).
#[test]
fn parse_boundary_overlays_on_the_drc_craft() {
    let bytes = read_fixture(DRC_CRAFT);
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("the drc craft loads: {error:?}"),
    };
    let snapshot = session.snapshot_with_overlays();
    assert!(
        !snapshot.overlays.violation_markers.is_empty(),
        "the craft carries >= 1 violation marker at the load boundary"
    );
    assert!(
        snapshot
            .overlays
            .violation_markers
            .iter()
            .all(|marker| marker.phase == epic_engine::snapshot::MarkerPhase::Parse),
        "every pre-route marker carries Parse"
    );
    // The wire distinguishes the phases (the phase-tag pin's
    // distinguishability face).
    assert_ne!(
        serde_json::to_string(&epic_engine::snapshot::MarkerPhase::Parse)
            .expect("MarkerPhase serializes"),
        serde_json::to_string(&epic_engine::snapshot::MarkerPhase::PostRoute)
            .expect("MarkerPhase serializes"),
        "the two phases serialize differently"
    );
    assert!(
        !snapshot.overlays.airlines.is_empty(),
        "the unrouted craft carries airlines"
    );
    assert_eq!(
        snapshot.overlays.tuning, None,
        "the constraint-free craft carries tuning None"
    );
    assert_eq!(
        snapshot.overlays.congestion, None,
        "a never-routed session carries congestion None"
    );
    // Determinism: two attach calls over the unchanged board are
    // byte-equal.
    let again = session.snapshot_with_overlays();
    assert_eq!(
        serde_json::to_string(&snapshot).expect("the snapshot serializes"),
        serde_json::to_string(&again).expect("the snapshot serializes"),
        "two attach calls on the unchanged board are byte-equal"
    );
}

// --- PIN 3 — PostRoute + the statistics cross-check -------------------

/// bm08 routed at defaults: every marker `PostRoute`, the airline
/// count equals the `BoardStatistics` incomplete face at the same
/// boundary (the count cross-check), congestion/tuning `None`.
#[test]
fn post_route_faces_and_the_statistics_cross_check() {
    let bytes = read_fixture(BM08);
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("bm08 loads: {error:?}"),
    };
    let summary = session
        .route(&CliLayer::default(), &mut SilentSink)
        .expect("route succeeds");
    let snapshot = session.snapshot_with_overlays();
    assert!(
        snapshot
            .overlays
            .violation_markers
            .iter()
            .all(|marker| marker.phase == epic_engine::snapshot::MarkerPhase::PostRoute),
        "every post-route marker carries PostRoute"
    );
    assert_eq!(
        snapshot.overlays.airlines.len() as i64,
        summary.incomplete_count,
        "the airline count equals the BoardStatistics incomplete face at the same boundary"
    );
    let stats = session.statistics();
    assert_eq!(
        snapshot.overlays.airlines.len() as i64,
        stats.incomplete_count,
        "the airline count equals the stored statistics face"
    );
    assert_eq!(
        snapshot.overlays.congestion, None,
        "default settings => None"
    );
    assert_eq!(
        snapshot.overlays.tuning, None,
        "bm08 declares no constraints"
    );
    let _ = summary.final_state;
}

// --- PIN 4 — congestion Some-iff-engaged ------------------------------

/// The reachable ON path (`SessionLayer.congestion_global`) yields
/// `Some` with a non-empty grid; the default session (the contrast,
/// loaded in the same test) yields `None`.
#[test]
fn congestion_engaged_yields_some_and_default_yields_none() {
    let bytes = read_fixture(BM08);
    let on_layer = SessionLayer {
        congestion_global: Some(true),
        ..SessionLayer::default()
    };
    let mut on_session = match Session::load_dsn(&bytes, on_layer) {
        Ok(session) => session,
        Err(error) => panic!("bm08 loads: {error:?}"),
    };
    on_session
        .route(&CliLayer::default(), &mut SilentSink)
        .expect("route succeeds");
    let snapshot = on_session.snapshot_with_overlays();
    let heatmap = snapshot
        .overlays
        .congestion
        .as_ref()
        .expect("the congestion_global-ON route engages the heatmap");
    assert!(heatmap.dims.0 > 0 && heatmap.dims.1 > 0, "a non-empty grid");
    assert!(
        heatmap.cells.iter().all(|cell| cell.overflow > 0),
        "only congested cells are recorded"
    );

    let mut off_session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("bm08 loads: {error:?}"),
    };
    off_session
        .route(&CliLayer::default(), &mut SilentSink)
        .expect("route succeeds");
    assert_eq!(
        off_session.snapshot_with_overlays().overlays.congestion,
        None,
        "the default-settings route leaves the heatmap None"
    );

    // Fix-round 2 (quality Q2, mutant Q-M4) — REWORKED at M10-T2: the
    // Q6 one-route law (RouteError::AlreadyRouted) removed the
    // SAME-session re-route face this leg used (`=` vs `|=` is
    // indistinguishable on a session that routes exactly once —
    // `congestion_engaged` is written exactly once, so the sticky
    // mutant is now structurally equivalent). The engagement face
    // stands on ONE session below; the reset-to-None face is pinned
    // on the fresh default-CLI session (`off_session`) directly
    // above, loaded from the SAME bytes.
    let bytes = read_fixture(BM08);
    let mut one_session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("bm08 loads: {error:?}"),
    };
    let cli_on = CliLayer {
        congestion_global: Some(true),
        ..CliLayer::default()
    };
    one_session
        .route(&cli_on, &mut SilentSink)
        .expect("route succeeds");
    assert!(
        one_session
            .snapshot_with_overlays()
            .overlays
            .congestion
            .is_some(),
        "the CLI-ON route engages the heatmap"
    );
}

// --- PIN 3b — the F1 fix-round face: PostRoute on REAL content -------

/// The violation-retaining post-route face (spec-review F1, mutant
/// M3): the drc craft ROUTED at defaults RETAINS violations (the
/// non-vacuousness assert comes FIRST — a phase forced to Parse now
/// fails here on real content, where the vacuous routed-bm08 face
/// could not catch it). Also pins the marker count == the depth-walk
/// total at the SAME post-route boundary (via
/// [`Session::clearance_violation_depth_total`], the same walk the
/// attach step runs).
#[test]
fn post_route_markers_survive_on_the_violation_retaining_craft() {
    let bytes = read_fixture(DRC_CRAFT);
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("the drc craft loads: {error:?}"),
    };
    session
        .route(&CliLayer::default(), &mut SilentSink)
        .expect("route succeeds");
    let snapshot = session.snapshot_with_overlays();
    // NON-VACUOUS FIRST (the M3 closure's whole point).
    assert!(
        !snapshot.overlays.violation_markers.is_empty(),
        "the routed craft RETAINS >= 1 clearance violation (the non-vacuousness face)"
    );
    assert!(
        snapshot
            .overlays
            .violation_markers
            .iter()
            .all(|marker| marker.phase == epic_engine::snapshot::MarkerPhase::PostRoute),
        "every retained post-route marker carries PostRoute on REAL content"
    );
    assert_eq!(
        snapshot.overlays.violation_markers.len(),
        session.clearance_violation_depth_total() as usize,
        "the marker count equals the depth-walk total at the same post-route boundary"
    );
}

// --- PIN 5 — the tuning face ------------------------------------------

/// The crafted tuning fixture (a probed constraint declaration):
/// `tuning = Some`, one info per constrained net with a positive
/// actual length; the constraint-free drc craft stays `None` (the
/// contrast).
#[test]
fn tuning_face_on_the_tuning_fixture() {
    let bytes = read_fixture(TUNING);
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("the tuning fixture loads: {error:?}"),
    };
    let snapshot = session.snapshot_with_overlays();
    let infos = snapshot.overlays.tuning.as_ref().expect("Some");
    assert!(!infos.is_empty(), "the fixture declares constraints");
    assert!(infos.windows(2).all(|w| w[0].net < w[1].net), "nets ascend");
    for info in infos {
        assert!(info.min > 0.0 || info.max > 0.0, "a non-zero bound");
        assert!(info.actual > 0.0, "the routed length is positive");
    }

    let craft_bytes = read_fixture(DRC_CRAFT);
    let mut craft = match Session::load_dsn(&craft_bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("the craft loads: {error:?}"),
    };
    assert_eq!(
        craft.snapshot_with_overlays().overlays.tuning,
        None,
        "the constraint-free craft stays None"
    );
}

// --- the count cross-check against all_incompletes (pin 7's engine
//     face; the drc crate pins the crafted-board face) ---------------

/// The airline count equals Σ per-net `incomplete_count` from an
/// INDEPENDENT `all_incompletes` walk (a separately parsed board +
/// fresh manager, the exact load prelude replicated), and the marker
/// count equals the depth walk's total (the `(i64, Vec<DepthRow>)`
/// first element).
#[test]
fn attach_counts_match_the_independent_drc_walks() {
    use epic_board::board::Board;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_dsn::reader::read_board;
    use epic_dsn::ses_board::SesBoard;

    let bytes = read_fixture(DRC_CRAFT);
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("the craft loads: {error:?}"),
    };
    let snapshot = session.snapshot_with_overlays();

    // The independent walk: parse fresh, replicate the load prelude
    // (board build + tree fill + normalization; the copper-to-edge
    // override touches only the outline's clearance class — no
    // effect on the incompletes walk, which reads connectivity — so
    // the replicated prelude ends at normalize_all_traces).
    let mut ses = SesBoard::new();
    let result = read_board(&bytes, &mut ses);
    assert!(
        matches!(result, epic_dsn::reader::DsnReadResult::Success { .. }),
        "the craft parses"
    );
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
    let (_max_conn, rows) = all_incompletes(&manager, &mut board);
    let sum: usize = rows.iter().map(|row| row.incomplete_count).sum();
    assert_eq!(
        snapshot.overlays.airlines.len(),
        sum,
        "airlines == Σ per-net incomplete_count (the independent walk)"
    );

    // The marker count == the depth walk's total.
    let (total, rows) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    assert_eq!(
        snapshot.overlays.violation_markers.len() as i64,
        total,
        "markers == the depth walk total"
    );
    assert_eq!(
        rows.len() as i64,
        total,
        "rows.len == total (the walk contract)"
    );

    // F2 (spec-review fix round): the pair_kind DERIVED OUTPUT, from
    // real fixture content. The markers derive IN ROW ORDER
    // (filter_map preserves order; the count equality above is the
    // pairing guard on this fixture), so markers[i] corresponds to
    // rows[i]. The expected u8s come from a TEST-LOCAL table (the
    // documented SPEC — BoardItemType's Java declaration order,
    // Trace=0 … Other=9), NOT from the impl's own
    // `kind_discriminant` — an ordinal-permutation mutant in the
    // impl now disagrees with the spec table and dies.
    fn spec_discriminant(kind: epic_board::items::BoardItemType) -> u8 {
        use epic_board::items::BoardItemType as K;
        match kind {
            K::Trace => 0,
            K::Pin => 1,
            K::Via => 2,
            K::ObstacleArea => 3,
            K::ViaObstacleArea => 4,
            K::ConductionArea => 5,
            K::ComponentObstacleArea => 6,
            K::BoardOutline => 7,
            K::ComponentOutline => 8,
            K::Other => 9,
        }
    }
    assert_eq!(
        snapshot.overlays.violation_markers.len(),
        rows.len(),
        "index pairing is sound on this fixture (counts equal)"
    );
    for (marker, row) in snapshot.overlays.violation_markers.iter().zip(rows.iter()) {
        let kind = |id: i64| {
            let id = epic_board::id::ItemId::new(u32::try_from(id).expect("craft id fits u32"));
            board
                .get(id)
                .map(|entry| spec_discriminant(entry.board_item_type()))
                .expect("the violating item exists on the board")
        };
        let expected = (kind(row.a), kind(row.b));
        assert_eq!(
            marker.pair_kind, expected,
            "pair_kind == the declaration-order discriminants of the violated pair \
             (row ({}, {}))",
            row.a, row.b
        );
    }
}
