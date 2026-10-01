//! M4-T3 capture pins for the tightener seam (jar oracle:
//! `rust/harness/oracle/TraceTightenerProbe.java` on the 90° fixture
//! `rust/harness/fixtures/trace-tightener/tightener90.dsn`; capture
//! `logs/M4-T3/evidence/tightener90_capture_run3.jsonl`, double-run
//! byte-identical, plus `tightener90_regionscan.jsonl` for the reach
//! boundary). Every literal below is read off the capture rows.
//!
//! Fixture geometry (DSN units x10 = DBU): rule width 200 / clearance
//! 250 -> halfwidth 1000, maxClearanceL0 2500, maxTraceHalfWidth 1000
//! (jar-diagnosed on the world rows), so the changed-area offset is
//! `1.5 * (2500 + 2 * 1000) = 6750` DBU. Nets: N1..N7 -> numbers 1..6
//! in network order (N7=6, N6=5). Trace ids 3..8 in wiring order; the
//! blocker A (net 6) is wired FIFTH (id 7) and the blocked B (net 5)
//! SIXTH (id 8), so descending-id processing runs B first.
//!
//! The probe call is the production-consumption shape
//! `optChangedArea(new int[0], null, 100, null, null, 0)`: empty nets,
//! null clip (unbounded), accuracy 100 (the clamp face), no trace
//! costs (no vias on the fixture — the via arm is dead), no stopper,
//! timeLimit 0 (no budget — deterministic witness). The budget /
//! stopper pins (t3_budget_*, t3_stoppable_*) have NO jar counterpart
//! (Java's budget is wall clock): they pin the Rust-deterministic
//! tick face and are disclosed as such in the task report.

use super::{TraceTightener, TraceTightenerSeam, active_for};
use crate::id::ItemId;
use crate::items::ItemData;
use crate::routing_board_insert::{
    join_changed_area, opt_changed_area, start_marking_changed_area,
};
use crate::rules_surf::AngleRestriction;
use crate::test_util::parse_board_from_path;
use crate::tree_manager::SearchTreeManager;
use epic_geometry::direction::Direction;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_point::IntPoint;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A fresh fixture parse + tree reinsert + the wiring-scope-tail
/// normalize (the probe's `parse` = Java `DsnReader.readBoard` — which
/// fires `board.normalizeAllTraces()` at the `(wiring ...)` scope tail,
/// `Wiring.java:347` — plus `reinsertTreeItems()`). The epic-dsn reader
/// deliberately omits the normalize pass (the D11 deferral,
/// `epic-dsn/src/reader.rs`): every consumer MUST run
/// [`crate::normalize_all::normalize_all_traces`] after building its
/// `SearchTreeManager` (the production recipe,
/// `epic-cli/src/route.rs:905-919` — refreshed at M10-T2; the prior
/// cite `:486-490` was stale at HEAD, pointing at `java_two_decimal`).
/// Omitting it is invisible on
/// disjoint-wire fixtures (normalize is a no-op) but diverges the
/// moment same-net wires touch — the smooth fixture's T junctions.
fn parse_fixture() -> (SearchTreeManager, crate::board::Board) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rust/harness/fixtures/trace-tightener/tightener90.dsn");
    let mut board = parse_board_from_path(&path.to_string_lossy());
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    crate::normalize_all::normalize_all_traces(&mut manager, &mut board);
    (manager, board)
}

/// The single-layer-0 trace carrying `net` (the probe identifies traces
/// by net; ids additionally pinned by `trace_id`).
fn trace_by_net(board: &crate::board::Board, net: i32, trace_id: i32) -> ItemId {
    let found = board
        .iter_descending()
        .find(|entry| {
            entry.nets == vec![net] && matches!(entry.data, ItemData::Trace { layer: 0, .. })
        })
        .map(|entry| entry.id)
        .expect("fixture trace on net");
    assert_eq!(
        found.get() as i32,
        trace_id,
        "capture trace id for net {net}"
    );
    found
}

/// The corner chain of a live trace as i32 pairs (the capture's
/// cornerApprox values; all fixture geometry is integral).
#[track_caller]
fn assert_corners(board: &crate::board::Board, id: ItemId, expected: &[(i32, i32)]) {
    let lines = board.trace_polyline(id).expect("live trace polyline");
    assert_eq!(
        lines.corner_count(),
        expected.len(),
        "corner count of trace {id:?}"
    );
    for (i, (x, y)) in expected.iter().enumerate() {
        let c = lines.corner_approx(i as i32);
        assert_eq!(
            (c.x, c.y),
            (f64::from(*x), f64::from(*y)),
            "corner {i} of trace {id:?}"
        );
    }
}

/// One probe world: fresh parse, changed-area session, the join plan on
/// layer 0, then the free `opt_changed_area` behind the PRODUCTION seam
/// with the probe's arguments (`only_nets = []`, clip `None`,
/// accuracy 100, no keep point, `trace_costs`, `stoppable`,
/// `time_limit_millis`, `deterministic_budgets`).
#[allow(clippy::too_many_arguments)] // mirrors the seam tail
fn run_world(
    joins: &[(i32, i32)],
    trace_costs: Option<&[super::TraceCostFactor]>,
    stoppable: Option<Arc<AtomicBool>>,
    time_limit_millis: i32,
    deterministic_budgets: bool,
) -> (SearchTreeManager, crate::board::Board) {
    let (mut manager, mut board) = parse_fixture();
    start_marking_changed_area(&mut board);
    for (x, y) in joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        trace_costs,
        stoppable.as_ref(),
        time_limit_millis,
        deterministic_budgets,
    );
    (manager, board)
}

/// The capture's BEFORE literals per net (trace id in the capture).
mod before {
    pub const STAIR: [(i32, i32); 6] = [
        (200000, 400000),
        (360000, 400000),
        (360000, 340000),
        (240000, 340000),
        (240000, 280000),
        (160000, 280000),
    ];
    pub const ACID: [(i32, i32); 2] = [(200000, 120000), (360000, 120000)];
    pub const CROSSER: [(i32, i32); 2] = [(280000, 114000), (280000, 126000)];
    pub const CUP: [(i32, i32); 4] = [
        (1000000, 200000),
        (1060000, 200000),
        (1060000, 180000),
        (1000000, 180000),
    ];
    pub const BLOCKER_A: [(i32, i32); 4] = [
        (450000, 390000),
        (480000, 390000),
        (480000, 410000),
        (450000, 410000),
    ];
    pub const BLOCKED_B: [(i32, i32); 4] = [
        (480000, 360000),
        (540000, 360000),
        (540000, 420000),
        (480000, 420000),
    ];
}

/// The 45°-LIVENESS pin (the T3 dormancy pin
/// `t3_seam_inert_on_fortyfive_degree_boards`, converted per the T4
/// contract): the crafted 45° board `tightener45.dsn` (one 6-corner
/// staircase, net N1) joined over its own corners must converge to
/// the JAR's fixpoint literal — the cornerworlds capture row
/// (`logs/M4-T4/evidence/tightener45_cornerworlds_run3.jsonl`,
/// double-run byte-identical): before
/// `100000:100000;200000:100000;300000:200000;400000:200000;500000:300000;600000:300000`
/// → after `100000:100000;400000:100000;600000:300000` (the
/// reposition ladder folds the three staircase steps and the
/// skipSegmentsOfLength0 arm removes the zero-length tails). This is
/// also the T3-M6 composite mis-dispatch killer: dispatching the 45°
/// arm to `pull_tight_90` produces a DIFFERENT final
/// `(100000:100000;100000:-100000;500000:300000;600000:300000)`
/// shape — the literal dies. Non-termination of the fixpoint hangs
/// the test (the termination face).
#[test]
fn t4_staircase_liveness_jar_final() {
    let joins = [
        (100000, 100000),
        (200000, 100000),
        (300000, 200000),
        (400000, 200000),
        (500000, 300000),
        (600000, 300000),
    ];
    let (mut manager, mut board) =
        parse_fixture_rel("../../../rust/harness/fixtures/trace-tightener/tightener45.dsn");
    let before = trace_fingerprint(&board);
    assert_eq!(
        before,
        vec![
            "1|100000:100000;200000:100000;300000:200000;400000:200000;500000:300000;600000:300000"
        ],
        "staircase parse shape"
    );
    start_marking_changed_area(&mut board);
    for (x, y) in &joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        None,
        None,
        0,
        false,
    );
    let after = trace_fingerprint(&board);
    assert_eq!(
        after,
        vec!["1|100000:100000;400000:100000;600000:300000"],
        "staircase jar fixpoint (the 90°-dispatch composite makes a different final)"
    );
}

/// The `stair` world (capture world 1): the 6-corner N1 staircase
/// joins its own six corners; the reposition + skipSegmentsOfLength0
/// ladder collapses it to the capture's 3-corner L — and every OTHER
/// trace stays byte-exact at its before state (region selectivity).
/// Termination is pinned by the run CONVERGING to the jar's fixpoint
/// (a non-terminating fixpoint hangs the test).
#[test]
fn t3_stair_jar_geometry_and_fixpoint_termination() {
    let joins: Vec<(i32, i32)> = before::STAIR.to_vec();
    let (_manager, board) = run_world(&joins, None, None, 0, false);
    // Capture `stair` after row: trace 3 collapsed to the L.
    assert_corners(
        &board,
        trace_by_net(&board, 1, 3),
        &[(200000, 400000), (200000, 280000), (160000, 280000)],
    );
    // The untouched witnesses (capture before rows).
    assert_corners(&board, trace_by_net(&board, 2, 4), &before::ACID);
    assert_corners(&board, trace_by_net(&board, 3, 5), &before::CROSSER);
    assert_corners(&board, trace_by_net(&board, 4, 6), &before::CUP);
    assert_corners(&board, trace_by_net(&board, 6, 7), &before::BLOCKER_A);
    assert_corners(&board, trace_by_net(&board, 5, 8), &before::BLOCKED_B);
}

/// The `acid` world (capture world 2): the straight N3 trace FROM its
/// own-net pin, crossed by the foreign N4 trace, joins its own two
/// corners — and NOTHING moves. Java `TraceTightener.avoidAcidTraps`
/// is `if (true) return polyline;` (dead body): the mutant activating
/// it would wrap the acid trace around the crosser (the pin's corner
/// literals die). The pin AT the trace's end corner is the acid-trap
/// arm (own-net pin contact), the crosser the wrap obstacle.
#[test]
fn t3_acid_trap_dead_body_identity() {
    let joins = [(200000, 120000), (360000, 120000)];
    let (_manager, board) = run_world(&joins, None, None, 0, false);
    assert_corners(&board, trace_by_net(&board, 2, 4), &before::ACID);
    assert_corners(&board, trace_by_net(&board, 3, 5), &before::CROSSER);
    assert_corners(&board, trace_by_net(&board, 1, 3), &before::STAIR);
}

/// The `region_in` / `region_out` worlds (capture worlds 3+4): the
/// SAME 2x2 cell pair that pins the changed-area offset arithmetic.
/// The offset is `1.5 * (maxClearance + 2 * maxTraceHalfWidth) =
/// 6750` DBU (regionscan evidence: reach boundary x* = 992250 =
/// the bottom arm's segment-box west edge 999000 - 6750). The in-point
/// 994000 reaches (the cup collapses to the west bar); the out-point
/// 985000 does not (byte-exact before). The mutant dropping the
/// `2 * maxTraceHalfWidth` term (offset 3750) flips the in-cell to a
/// miss (999000 - 3750 = 995250 > 994000) and dies.
#[test]
fn t3_region_enlargement_jar_in_out() {
    // in x moved: the cup collapses to the capture's west bar.
    let (_manager, board) = run_world(&[(994000, 180000)], None, None, 0, false);
    assert_corners(
        &board,
        trace_by_net(&board, 4, 6),
        &[(1000000, 200000), (1000000, 180000)],
    );
    // out x untouched: the SAME cup byte-exact at its before state.
    let (_manager, board) = run_world(&[(985000, 180000)], None, None, 0, false);
    assert_corners(&board, trace_by_net(&board, 4, 6), &before::CUP);
}

/// The offset-boundary pin (spec-review MINOR-2 / reviewer T3-S4): the
/// measured reach boundary is asserted EXACTLY, not just bracketed.
/// Closed form `x + offset >= 999000` (the bottom arm's west segment-box
/// edge) with offset `1.5 * (maxClearanceL0 2500 + 2 * maxTraceHalfWidth
/// 1000) = 6750` gives x* = 992250 — the regionscan's bracket
/// (992000 miss / 992250 hit) plus the arithmetic. One DBU OUTSIDE
/// (992249) must NOT move; the boundary point itself (992250) and one
/// DBU inside (992251) must collapse to the same west bar. A ±1-DBU
/// drift of the offset arithmetic flips an arm: offset 6751 reaches
/// 992249 (the out-arm dies), offset 6749 stops short of 992250 (the
/// boundary arm dies) — self-verified both ways as mutant T3-M5.
#[test]
fn t3_region_offset_boundary_exact_pin() {
    // 992249 + 6750 = 998999 < 999000: the cup stays byte-exact.
    let (_manager, board) = run_world(&[(992249, 180000)], None, None, 0, false);
    assert_corners(&board, trace_by_net(&board, 4, 6), &before::CUP);
    // 992250 + 6750 = 999000: the inclusive boundary hit — the
    // regionscan's first-move row collapses to the west bar.
    let (_manager, board) = run_world(&[(992250, 180000)], None, None, 0, false);
    assert_corners(
        &board,
        trace_by_net(&board, 4, 6),
        &[(1000000, 200000), (1000000, 180000)],
    );
    // 992251: one DBU inside — same west bar.
    let (_manager, board) = run_world(&[(992251, 180000)], None, None, 0, false);
    assert_corners(
        &board,
        trace_by_net(&board, 4, 6),
        &[(1000000, 200000), (1000000, 180000)],
    );
}

/// The `block` world (capture world 5): two nested C-cups, blocker A
/// (id 7 < blocked B's id 8 — the wiring order witness) sitting with
/// its middle vertical exactly on B's collapse target. Descending-id
/// processing runs B FIRST: its full slide fails the check and the
/// bisection strands it east of A; then A collapses; the SECOND
/// fixpoint sweep re-marks B (which moved) and collapses it onto its
/// target. Both after states are the capture's. The early-exit
/// mutant (one sweep only) leaves B stranded — the B literal dies.
#[test]
fn t3_block_two_sweep_fixpoint_jar() {
    let joins = [
        (450000, 390000),
        (480000, 390000),
        (480000, 410000),
        (450000, 410000),
        (480000, 360000),
        (540000, 360000),
        (540000, 420000),
        (480000, 420000),
    ];
    let (_manager, board) = run_world(&joins, None, None, 0, false);
    let a = trace_by_net(&board, 6, 7);
    let b = trace_by_net(&board, 5, 8);
    assert!(
        a.get() < b.get(),
        "wiring order: the blocker is wired (and id'd) FIRST"
    );
    // Capture `block` after rows.
    assert_corners(&board, a, &[(450000, 390000), (450000, 410000)]);
    assert_corners(&board, b, &[(480000, 360000), (480000, 420000)]);
}

/// The dispatch pin (M4-T3 census, LIVE contract as of T4 — the T3
/// dormancy form asserted `FortyfiveDegree => false`, superseded with
/// disclosure): `active_for` is the getInstance class-selection face
/// — TRUE for `NinetyDegree` and (since T4) `FortyfiveDegree`, FALSE
/// for `None`. Parsed-fixture arms: the 90° and 45° fixtures
/// activate; the any-angle locator (the only fixture carrying an
/// explicit `(snap_angle none)`) stays a no-op — that arm is the
/// any-angle DEFERRAL stub (no tier fixture selects any-angle: no
/// tier fixture carries a `snap_angle` keyword at all beyond the
/// locators — `ReadScopeParameter.snapAngle`,
/// `ReadScopeParameter.java:59`, defaults to FORTYFIVE_DEGREE;
/// spec-review whole-root sweep: 0 hits across all 2334 fixture
/// DSNs — machine-tripwired by the dsn-compare goldens'
/// Java-captured `snap_angle:"45"` field, 1331/1331 parseable
/// fixtures); the SEAM's any-angle boundary opens with the
/// any-angle port, not before.
#[test]
fn t3_dispatch_active_for_census() {
    assert!(active_for(AngleRestriction::NinetyDegree));
    assert!(active_for(AngleRestriction::FortyfiveDegree));
    assert!(!active_for(AngleRestriction::None));

    for (rel, expected) in [
        (
            "../../../rust/harness/fixtures/trace-tightener/tightener90.dsn",
            true,
        ),
        (
            "../../../rust/harness/fixtures/trace-tightener/tightener45.dsn",
            true,
        ),
        (
            "../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn",
            true,
        ),
        (
            "../../../rust/harness/fixtures/locator-spike/t9_locator_any.dsn",
            false,
        ),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let board = parse_board_from_path(&path.to_string_lossy());
        assert_eq!(
            active_for(board.rules().trace_angle_restriction),
            expected,
            "active_for on {rel}"
        );
    }
}

/// The deterministic tick-budget face (NO jar counterpart — Java's
/// budget is wall clock; this pins the Rust `RouteBudget`-pattern
/// face). Block world, `deterministic_budgets = true`,
/// `time_limit_millis = 2`: consult 1 (outer loop, before B) passes,
/// B's first 90°-loop consult passes and its FIRST reposition is
/// productive (the bisection strands B east of A), the next consult
/// (spent 3 > 2) stops the run. Expected: B moved but did NOT reach
/// its collapse target, A untouched. A `>=` flip stops one consult
/// earlier (B untouched); a budget that never fires tightens the
/// whole world — all literals die.
#[test]
fn t3_deterministic_tick_budget_partial_face() {
    let joins = [
        (450000, 390000),
        (480000, 390000),
        (480000, 410000),
        (450000, 410000),
        (480000, 360000),
        (540000, 360000),
        (540000, 420000),
        (480000, 420000),
    ];
    let (_manager, board) = run_world(&joins, None, None, 2, true);
    // A untouched (its consult never passed).
    assert_corners(&board, trace_by_net(&board, 6, 7), &before::BLOCKER_A);
    // B moved off its before state but did NOT converge to the jar's
    // fixpoint bar.
    let b = trace_by_net(&board, 5, 8);
    let lines = board.trace_polyline(b).expect("live B");
    let corners: Vec<(i32, i32)> = (0..lines.corner_count())
        .map(|i| {
            let c = lines.corner_approx(i as i32);
            (c.x as i32, c.y as i32)
        })
        .collect();
    let before_b: Vec<(i32, i32)> = before::BLOCKED_B.to_vec();
    assert_ne!(corners, before_b, "B moved under the budget");
    assert_ne!(
        corners,
        vec![(480000, 360000), (480000, 420000)],
        "B did NOT reach the collapse target under the budget"
    );
}

/// The stoppable-flag face (NO jar counterpart). Block world, flag
/// preset, no budget: the FIRST outer-loop consult observes the flag
/// and the sweep returns before touching anything — the whole world
/// byte-exact at its before state. A seam that drops the stoppable
/// tail tightens everything and dies.
#[test]
fn t3_stoppable_flag_face() {
    let joins = [
        (450000, 390000),
        (480000, 390000),
        (480000, 410000),
        (450000, 410000),
        (480000, 360000),
        (540000, 360000),
        (540000, 420000),
        (480000, 420000),
    ];
    let flag = Arc::new(AtomicBool::new(true));
    let (_manager, board) = run_world(&joins, None, Some(flag), 0, false);
    assert_corners(&board, trace_by_net(&board, 6, 7), &before::BLOCKER_A);
    assert_corners(&board, trace_by_net(&board, 5, 8), &before::BLOCKED_B);
    assert_corners(&board, trace_by_net(&board, 4, 6), &before::CUP);
}

/// The last-corner-skip corner-worlds (jar capture via the probe's
/// `cornerworlds` mode on `last-corner-skip.dsn`; each trace gets its
/// own fresh-parse world joining its own corners). Three of the five
/// crafted end-geometries (the P-tab, the Z-stair, the end hook — all
/// 6-line polylines) REACH the `i == len` skip arm whose
/// `second_last_corner_skipped` tail fires (instrumented-reach
/// evidence in logs/M4-T3/evidence/); the final geometry is the jar's
/// fixpoint literal per trace. The two L-shaped traces pin the
/// middle-corner skip path. Caveat (the T4-revisit condition, TWO
/// banked mutants): (1) T3-M2 — killing the
/// `second_last_corner_skipped` flag arm heals to the SAME finals on
/// every world/face probed; (2) quality-review T3-Q4 — removing
/// `reposition_line`'s `first_time` biggest-change break (Java
/// `:313-315`) also heals: it fires 8x across the pin run yet
/// converges to the same finals. Both are reposition faces the outer
/// fixpoint heals; T4 (the 45° port + pin set) RE-PROBED BOTH as
/// required — still 14/14 green with each arm killed
/// (`reprobe_T3-M2_second_last_corner_killed_on_45.log`,
/// `reprobe_T3-Q4_first_time_break_removed.log`), so BOTH STAY BANKED,
/// now with 45° evidence. Witnessing the break needs a world where
/// first-accepted ≠ fixpoint (a clearance-borderline bisection). The
/// T4 mutant round added the same-class healers T4-M1 (reposition
/// nearer-corner tie flip, fires 11x on the 45° pins) and T4-M2
/// (smoothen tie flip, fires once). These literals pin the
/// middle-corner skip path. LEDGER DISPOSITION (T6 re-triage — all
/// four banked healers now PERMANENTLY CLOSED, evidence under
/// logs/M4-T6/evidence/): (1) T3-M2 healed again on the 33/35-pin
/// suites (`mut_T3-M2_second_last_corner_t6tailpins_33green.log`) and
/// is closed structurally — the bogus 3-line tail's extra corner is
/// re-skippable by the same clear direct connection within the same
/// sweep unless another trace moves in between (a cross-trace
/// interleaving no single-fixture world sustains). (2) T4-M1 healed
/// (`mut_T4-M1_tie_flip_t6tailpins_33green.log`); closed structurally
/// — at an exact tie the two candidates are equidistant same-side, so
/// their chord is parallel to the line and the snap line through
/// either corner is the IDENTICAL line; the residual Int/Rational
/// mixed-tie surface is unconstructible within float error.
/// (3) T4-M2 healed (`mut_T4-M2_smoothen_tie_t6tailpins_33green.log`);
/// closed structurally — at a tie both candidates lie on the shared
/// outer side of the bisector with equal magnitude, so the flip is a
/// semantic no-op (the NEAR-tie magnitude choice stays pinned by the
/// S1b/S1c worlds). (4) T3-Q4 healed on every world including the two
/// T6 ledger worlds below
/// (`mut_T3-Q4_first_time_break_t6ledger_35green.log`); closed
/// structurally — for any f64-exact nearest corner the mutant's
/// post-acceptance search provably re-lands on the identical
/// through-corner line (the overshoot guard crawls back in exact
/// 0.5-steps), mid-search dyadic acceptances are transient (any change
/// re-triggers the fixpoint, whose terminals are integer-lattice —
/// the [`t6_dyadic45_integer_terminal_jar_literal`] world), and the
/// skip arms preempt the translate loop in worlds that do not block
/// every skip corridor individually
/// ([`t6_bisect45_skip_preempt_jar_literal`]); the only residual is a
/// float-epsilon straddle of the side-check at a diagonal-direction
/// corner — the same uncraftable JVM-arithmetic-luck class the T4-M1
/// close excludes. The literals below still pin the arm-execution
/// path downstream so a heal-break surfaces here.
#[test]
fn t3_last_corner_skip_corner_worlds_jar() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rust/harness/fixtures/trace-tightener/last-corner-skip.dsn");
    let after_by_net: [(i32, &[(i32, i32)]); 5] = [
        (1, &[(100000, 100000), (600000, 100000)]),
        (2, &[(100000, 250000), (100000, 270000), (600000, 270000)]),
        (3, &[(100000, 400000), (100000, 420000), (700000, 420000)]),
        (4, &[(700000, 100000), (1150000, 100000)]),
        (5, &[(700000, 450000), (1100000, 450000)]),
    ];
    for (net, expected) in after_by_net {
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        crate::normalize_all::normalize_all_traces(&mut manager, &mut board);
        let id = board
            .iter_descending()
            .find(|entry| {
                entry.nets == vec![net] && matches!(entry.data, ItemData::Trace { layer: 0, .. })
            })
            .map(|entry| entry.id)
            .expect("candidate trace");
        let joins: Vec<(i32, i32)> = {
            let lines = board.trace_polyline(id).expect("live");
            (0..lines.corner_count())
                .map(|i| {
                    let c = lines.corner_approx(i as i32);
                    (c.x as i32, c.y as i32)
                })
                .collect()
        };
        start_marking_changed_area(&mut board);
        for (x, y) in &joins {
            join_changed_area(
                &mut board,
                &FloatPoint::new(f64::from(*x), f64::from(*y)),
                0,
            );
        }
        opt_changed_area(
            &mut manager,
            &mut board,
            &mut TraceTightenerSeam,
            &[],
            None,
            100,
            None,
            0,
            None,
            None,
            0,
            false,
        );
        assert_corners(&board, id, expected);
    }
}

// ======================================================================
// M4-T4 — the 45° tightener LIVE. Jar oracle: the `cornerworlds` mode
// of `rust/harness/oracle/TraceTightenerProbe.java` (one world per
// trace of the fixture, fresh parse each, joining the focal trace's
// own corners, the production-consumption call
// `optChangedArea(new int[0], null, 100, null, null, 0)`) on the three
// crafted 45° fixtures. Captures:
// `logs/M4-T4/evidence/tightener45*_cornerworlds_run3.jsonl`, each
// captured TWICE and `cmp`-identical (run3b twins). Every literal
// below is read off the run3 after-inventories; the fingerprint
// format is one sorted string per trace `net1,net2|x:y;x:y;...` with
// ids dropped — the smoothen ladder removes/reinserts moved traces
// under FRESH ids, so only geometry is stable across the ladder.
//
// The fixtures (DSN units x10 = DBU, all corners integral):
// - tightener45_diag.dsn: four INDEPENDENT single-trace nets. N1/N3
//   are the reposition NEAR-TIE worlds (+9990/+10010 DBU offsets):
//   the fixpoint final has 3 corners. (Mutation verdict T4-M1: the
//   `<` → `<=` flip of repositionLine's nearer-corner pick fires 11x
//   on these worlds and HEALS to the same finals — banked healer, see
//   the DIAG_AFTER_N1 note.) N2 is the EXACT tie (symmetric wire):
//   strict `<` picks NEXT on the tie; the pinned final is the shared
//   one (both picks converge to it). N4 is the reduceCorners dog-ear
//   Z (5 → 3 corners).
// - tightener45_smooth.dsn: the smoothen/at-trace cluster. S1a/S1b/
//   S1c chamfer worlds (S1b/S1c pin the smoothenCorner `prevDist <=
//   nextDist` PREV-tie at a 10-DBU jog — both flip directions die);
//   S2+B2 the greedy-null → smoothenSharpCorner fallback shave
//   (fail band covers every bisection probe; shaved corners
//   179416/20584); S3 the acute splice at a mid-trace T junction
//   (the parse recipe's wiring-scope-tail normalizeAllTraces splits
//   the crossbar at the touch point, the splice consolidates the net
//   to one trace); S4 the bend splice (the jog
//   slides flush to y=55000); S5 a self-overlapping diagonal crossbar
//   whose world consolidates the whole net into ONE straight trace
//   (the deepest fixpoint cascade in the set); S6 the pin-kill
//   control (pin at the junction → whole world byte-exact).
// - tightener45_tail.dsn: the pin-connection tail face (DEFERRED in
//   the port — the counter-witness pin).
// ======================================================================

/// Fresh parse of a trace-tightener fixture (relative to the repo
/// root via `CARGO_MANIFEST_DIR`) + tree reinsert + the
/// wiring-scope-tail normalize (the full consumer recipe — see
/// [`parse_fixture`]).
fn parse_fixture_rel(rel: &str) -> (SearchTreeManager, crate::board::Board) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let mut board = parse_board_from_path(&path.to_string_lossy());
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    crate::normalize_all::normalize_all_traces(&mut manager, &mut board);
    (manager, board)
}

/// The whole-board trace geometry fingerprint: one sorted string per
/// trace, `net1,net2|x:y;x:y;...`, ids dropped. All fixture corners
/// are integral DBU (the jar rows are Double.toString of integral
/// values); the casts assert exactness.
#[track_caller]
fn trace_fingerprint(board: &crate::board::Board) -> Vec<String> {
    let mut out: Vec<String> = board
        .iter_descending()
        .filter(|entry| matches!(entry.data, ItemData::Trace { .. }))
        .map(|entry| {
            let lines = board.trace_polyline(entry.id).expect("live trace");
            let mut nets = entry.nets.clone();
            nets.sort_unstable();
            let nets = nets
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let chain = (0..lines.corner_count())
                .map(|i| {
                    let c = lines.corner_approx(i as i32);
                    assert_eq!(c.x, c.x.round(), "integral corner x");
                    assert_eq!(c.y, c.y.round(), "integral corner y");
                    format!("{}:{}", c.x as i64, c.y as i64)
                })
                .collect::<Vec<_>>()
                .join(";");
            format!("{nets}|{chain}")
        })
        .collect();
    out.sort();
    out
}

/// One 45° corner world: fresh parse of `rel`, join the focal trace's
/// own corners (identified by the jar's trace id — the Rust parse
/// assigns the same ids, asserted by the before fingerprints), then
/// the probe's production-consumption opt call.
#[track_caller]
fn run_45_corner_world(rel: &str, focal_id: i32) -> (SearchTreeManager, crate::board::Board) {
    let (mut manager, mut board) = parse_fixture_rel(rel);
    let joins: Vec<(i32, i32)> = {
        let entry = board
            .iter_descending()
            .find(|entry| entry.id.get() as i32 == focal_id)
            .expect("focal id present (parse id parity with the jar)");
        let lines = board.trace_polyline(entry.id).expect("focal polyline");
        (0..lines.corner_count())
            .map(|i| {
                let c = lines.corner_approx(i as i32);
                (c.x as i32, c.y as i32)
            })
            .collect()
    };
    start_marking_changed_area(&mut board);
    for (x, y) in &joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        None,
        None,
        0,
        false,
    );
    (manager, board)
}

/// The run3 jar literals (see the module docs for the capture).
mod jar45 {
    /// tightener45_diag.dsn before (also the parse-parity pin).
    pub(super) const DIAG_BEFORE: &[&str] = &[
        "1|10000:30000;20000:40000;30000:40000;39990:30010;39990:20010",
        "2|60000:30000;70000:40000;80000:40000;90000:30000;90000:20000",
        "3|110000:30000;120000:40000;130000:40000;140010:29990;140010:19990",
        "4|310000:200000;320000:210000;330000:210000;340000:220000;350000:220000",
    ];
    /// World focal 3 (N1, near-tie +9990): final 3 corners. NOTE
    /// (mutation-verdict correction): an earlier working note claimed a
    /// `<` → `<=` flip of `reposition_line`'s nearer-corner pick dies
    /// here — FALSE. The flip fires 11x across the T4 pin suite
    /// (instrumented reach,
    /// `logs/M4-T4/evidence/mut_T4-M1_tie_reach_t4pins_11hits.log`) and
    /// heals to the SAME finals everywhere (mutant run
    /// `mut_T4-M1_reposition_tie_flip.log`, 14/14 green) — BANKED
    /// HEALER, same class as T3-M2/T3-Q4. The near-tie literals pin
    /// the arm-execution path so a heal-break surfaces here.
    pub(super) const DIAG_AFTER_N1: &[&str] = &[
        "1|10000:30000;30000:30000;39990:20010",
        "2|60000:30000;70000:40000;80000:40000;90000:30000;90000:20000",
        "3|110000:30000;120000:40000;130000:40000;140010:29990;140010:19990",
        "4|310000:200000;320000:210000;330000:210000;340000:220000;350000:220000",
    ];
    /// World focal 4 (N2, exact tie): the shared 3-corner final.
    pub(super) const DIAG_AFTER_N2: &[&str] = &[
        "1|10000:30000;20000:40000;30000:40000;39990:30010;39990:20010",
        "2|60000:30000;80000:30000;90000:20000",
        "3|110000:30000;120000:40000;130000:40000;140010:29990;140010:19990",
        "4|310000:200000;320000:210000;330000:210000;340000:220000;350000:220000",
    ];
    /// World focal 5 (N3, near-tie +10010, the N1 mirror).
    pub(super) const DIAG_AFTER_N3: &[&str] = &[
        "1|10000:30000;20000:40000;30000:40000;39990:30010;39990:20010",
        "2|60000:30000;70000:40000;80000:40000;90000:30000;90000:20000",
        "3|110000:30000;130000:30000;140010:19990",
        "4|310000:200000;320000:210000;330000:210000;340000:220000;350000:220000",
    ];
    /// World focal 6 (N4 dog-ear Z): 5 → 3 corners.
    pub(super) const DIAG_AFTER_Z: &[&str] = &[
        "1|10000:30000;20000:40000;30000:40000;39990:30010;39990:20010",
        "2|60000:30000;70000:40000;80000:40000;90000:30000;90000:20000",
        "3|110000:30000;120000:40000;130000:40000;140010:29990;140010:19990",
        "4|310000:200000;330000:220000;350000:220000",
    ];

    /// tightener45_smooth.dsn before (also pins the parse faces: the
    /// wiring-scope-tail `normalizeAllTraces` (`Wiring.java:347`) —
    /// run by the parse recipe, see [`parse_fixture`] — SPLITS the
    /// S3/S4 crossbars at the T touch points (`PolylineTrace.split`:
    /// intersecting same-net traces split at contact points) and
    /// MERGES the S5 overlap (T's end segment lies exactly on the
    /// diagonal crossbar, same-net, split at first/last common point +
    /// `combine`), leaving
    /// `8|420000:30000;430000:40000` / `8|420000:40000;430000:40000` /
    /// `8|430000:40000;440000:50000;460000:30000`).
    pub(super) const SMOOTH_BEFORE: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// World focal 3 (S1a): chamfer worlds collapse to the diagonal
    /// (the tie-flip mutant converges to the SAME shape here — the
    /// discriminators are S1b/S1c).
    pub(super) const SMOOTH_AFTER_S1A: &[&str] = &[
        "1|10000:10000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// World focal 4 (S1b): the 10-DBU jog survives at (79990,29990)
    /// — the smoothenCorner `<=` PREV-tie face.
    pub(super) const SMOOTH_AFTER_S1B: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;79990:29990;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// World focal 5 (S1c): the mirrored 10-DBU jog at (130000,30000).
    pub(super) const SMOOTH_AFTER_S1C: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;130000:30000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// Worlds focal 6+7 (S2 + blocker B2): the fallback shave fired on
    /// S2 (corners 179416/20584), B2 byte-exact throughout.
    pub(super) const SMOOTH_AFTER_S2_B2: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;179416:20000;180000:20584;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// Worlds focal 8/16/17 (S3 acute-splice cluster): the net
    /// consolidates to ONE trace (20000,40000)→(30000,50000)→(60000,
    /// 50000) — the splice + normalize-merge + fixpoint cascade.
    pub(super) const SMOOTH_AFTER_S3: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// Worlds focal 10/18/19 (S4 bend-splice cluster): the SE+E jog
    /// slides flush to y=55000, the crossbar re-splits there.
    pub(super) const SMOOTH_AFTER_S4: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;235000:55000",
        "7|235000:40000;235000:55000",
        "7|235000:55000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// Worlds focal 14+15 (S6 pin-kill control): the pin at the
    /// junction makes every at-trace arm return null — the WHOLE world
    /// byte-exact at its before state. (Equals `SMOOTH_BEFORE`.)
    pub(super) const SMOOTH_AFTER_S6_INERT: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;430000:40000",
        "8|420000:40000;430000:40000",
        "8|430000:40000;440000:50000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
    /// Worlds focal 20/22/24 (S5 self-overlap cascade): the whole net
    /// consolidates into ONE straight trace (420000,30000)→(460000,
    /// 30000).
    pub(super) const SMOOTH_AFTER_S5: &[&str] = &[
        "1|10000:10000;20000:20000;30000:20000;30000:30000",
        "2|60000:10000;70000:20000;80000:20000;80000:29990",
        "3|110000:10000;120000:20000;130000:20000;130000:30010",
        "4|160000:10000;170000:20000;180000:20000;180000:40000",
        "5|170600:22300;177700:22300",
        "6|20000:40000;30000:40000;40000:50000",
        "6|20000:50000;40000:50000",
        "6|40000:50000;60000:50000",
        "7|220000:55000;225000:50000;235000:50000",
        "7|235000:40000;235000:50000",
        "7|235000:50000;235000:65000",
        "8|420000:30000;460000:30000",
        "9|520000:40000;530000:40000;540000:50000",
        "9|540000:50000;520000:50000",
    ];
}

/// The four diagonal-fixture corner worlds (jar run3, byte-identical
/// double run). The before literal pins the Rust PARSE of the fixture;
/// each after literal is a whole-board fixpoint final.
#[test]
fn t4_diag_corner_worlds_jar() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_diag.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(trace_fingerprint(&board), jar45::DIAG_BEFORE, "diag parse");
    for (focal, expected) in [
        (3, &jar45::DIAG_AFTER_N1),
        (4, &jar45::DIAG_AFTER_N2),
        (5, &jar45::DIAG_AFTER_N3),
        (6, &jar45::DIAG_AFTER_Z),
    ] {
        let (_manager, board) = run_45_corner_world(rel, focal);
        assert_eq!(
            trace_fingerprint(&board),
            *expected,
            "diag world focal {focal}"
        );
    }
}

/// The sixteen smooth-fixture corner worlds (jar run3, byte-identical
/// double run), keyed by focal id into the eight distinct jar finals.
#[test]
fn t4_smooth_corner_worlds_jar() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_smooth.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(
        trace_fingerprint(&board),
        jar45::SMOOTH_BEFORE,
        "smooth parse (splits + self-overlap merge)"
    );
    let worlds: [(i32, &[&str]); 16] = [
        (3, jar45::SMOOTH_AFTER_S1A),
        (4, jar45::SMOOTH_AFTER_S1B),
        (5, jar45::SMOOTH_AFTER_S1C),
        (6, jar45::SMOOTH_AFTER_S2_B2),
        (7, jar45::SMOOTH_AFTER_S2_B2),
        (8, jar45::SMOOTH_AFTER_S3),
        (10, jar45::SMOOTH_AFTER_S4),
        (14, jar45::SMOOTH_AFTER_S6_INERT),
        (15, jar45::SMOOTH_AFTER_S6_INERT),
        (16, jar45::SMOOTH_AFTER_S3),
        (17, jar45::SMOOTH_AFTER_S3),
        (18, jar45::SMOOTH_AFTER_S4),
        (19, jar45::SMOOTH_AFTER_S4),
        (20, jar45::SMOOTH_AFTER_S5),
        (22, jar45::SMOOTH_AFTER_S5),
        (24, jar45::SMOOTH_AFTER_S5),
    ];
    for (focal, expected) in worlds {
        let (_manager, board) = run_45_corner_world(rel, focal);
        assert_eq!(
            trace_fingerprint(&board),
            expected,
            "smooth world focal {focal}"
        );
    }
}

/// The pin-connection tail face, LANDED in T6 (was the T4 deferral —
/// this pin began life as `t4_tail_deferral_counter_witness`, asserting
/// the RUST world stayed byte-exact while the jar split it; the deferral
/// contract died the day the tail landed). The jar's tail
/// (swap/correctConnectionToPin inside `PolylineTrace.pullTight`)
/// splits the diagonal into an L-stub plus a pad-side stub —
/// jar after (run3 capture): `1|150000:150000;58517:58517;58517:50000`
/// and `1|50000:50000;58517:50000`. The port now reproduces the jar
/// literal byte-exactly (correct arm: the diagonal's SW end direction
/// matches no cardinal exit restriction of the square pad, so
/// `correctConnectionToPin(false)` rebuilds the end along the LEFT
/// exit and inserts the shove-fixed stub; the swap arms are inert —
/// the single end contact is the PIN, not a shove-fixed trace).
#[test]
fn t6_tail_correct_arm_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    let before = trace_fingerprint(&board);
    assert_eq!(before, vec!["1|150000:150000;50000:50000"], "tail parse");
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|150000:150000;58517:58517;58517:50000",
            "1|50000:50000;58517:50000",
        ],
        "tail landed: jar run3 counter-witness literal is now the Rust result"
    );
}

/// OUTCOME parity pin for the swap-CANDIDATE geometry (jar
/// `tailworlds` witness, `tail45_swap_worlds.jsonl`): a SHOVE-FIXED
/// pad stub met by a free loop that wraps back into the offset pad.
/// Mechanism note (instrumented debug build, since removed): the tail
/// is NOT the producer here — the main 45-degree tightener slides the
/// loop onto the stub's line (a legal same-net overlap), and the
/// overlap consolidation then deletes the SHOVE-FIXED stub, leaving
/// ONE unfixed pad stub. The same cascade runs in the jar (the
/// pull-tight tail only runs when the tightener returns the input
/// unchanged, so the preemption is Java-faithful). The pin anchors
/// the jar OUTCOME byte-exactly; the swap arm's own FIRE face is
/// pinned separately by [`t6_tail_swap_fire_jar_literal`], whose
/// world denies the tightener any improving move first.
#[test]
fn t6_tail_swap_world_outcome_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|150000:50000;120000:80000;120000:180000;20000:180000;20000:50000;46500:50000",
            "1|50000:50000;150000:50000",
        ],
        "swap world parse: shove-fixed pad stub + free loop"
    );
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|50000:50000;46500:50000"],
        "jar outcome: one pad stub (slide + overlap consolidation)"
    );
}

/// The OUTCOME control twin (jar `tail45_swap_notshove_worlds.jsonl`):
/// the same geometry with the stub trace USER_FIXED (`(type protect)`
/// — the lexer's catch-all fixed arm) instead of SHOVE-FIXED. The
/// overlap consolidation refuses to absorb a USER_FIXED trace, so the
/// stub SURVIVES beside the collapsed loop (jar after: `{3 USER_FIXED
/// unchanged, 6 UNFIXED 50000:50000;46500:50000}`). The fixed-state
/// sensitivity of the consolidation is the face this twin pins; the
/// swap arm's STRICT SHOVE_FIXED-equality gate is pinned by
/// [`t6_tail_swap_fire_not_shove_control`].
#[test]
fn t6_tail_swap_world_notshove_outcome_control() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap_notshove.dsn";
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|50000:50000;150000:50000", "1|50000:50000;46500:50000",],
        "USER_FIXED stub survives the consolidation"
    );
}

/// The swap arm's FIRE face (jar `tail45_swap_fire_worlds.jsonl`,
/// BOTH focal joins reaching the identical final): S is a SHOVE-FIXED
/// diagonal exiting the pad NE; F is a free W-then-S wrap whose end
/// re-enters the offset pad box from the north. F's tightening is
/// clearance-blocked by S into a local optimum, so the pull-tight
/// TAIL runs — `swapConnectionToPin(at_start)` finds S as F's single
/// start contact (exact SHOVE_FIXED equality), the sharp-angle
/// projection (NE vs W) is NEGATIVE, and the combined polyline's last
/// entry into the offset pad shape (the north border) picks the exit
/// UP, different from S's own NE — all five Java `:1252-1313`
/// conditions hold, so the swap fires: `setFixedState(S := UNFIXED)`
/// plus `combine()`, and the fixpoint cascade collapses the merged path
/// to the jar's final `50000:50000;50000:51000;45000:56000`. Only the
/// swap deletes a SHOVE-FIXED trace (the correct arm only ever
/// INSERTS shove-fixed stubs), so S's vanishing is the fire's
/// signature. (Two earlier world designs for this face were killed by
/// preempting faces, both jar-witnessed in evidence/: the loop world
/// — tightener slide onto the stub's line,
/// `tail45_swap_worlds.jsonl` — and the anti-parallel retrace world —
/// the parse itself consolidates the SHOVE-FIXED overlap away,
/// `tail45_swap_diag_worlds.jsonl`, pinned by
/// [`t6_tail_swap_diag_parse_consolidation_jar_literal`].)
#[test]
fn t6_tail_swap_fire_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap_fire.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|50000:50000;94000:94000",
            "1|94000:94000;45000:94000;45000:56000",
        ],
        "swap-fire parse: shove-fixed NE diagonal + free W-S wrap"
    );
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|50000:50000;50000:51000;45000:56000"],
        "swap fired: the shove-fixed diagonal is gone (combined away)"
    );
}

/// The swap gate's STRICT fixed-state equality control
/// (`currentContact.getFixedState() == FixedState.SHOVE_FIXED`,
/// `:1265`; jar `tail45_swap_fire_notshove_worlds.jsonl`): the same
/// fire world with S USER_FIXED must REFUSE the swap — S SURVIVES
/// unchanged and F stops at its S-blocked tightener optimum
/// (`94000:94000;83000:94000;45000:56000`). USER_FIXED passes the
/// ordinal `isShoveFixed()` face (USER_FIXED >= SHOVE_FIXED), so a
/// relaxation of the equality would combine here and drop S — the
/// surviving S is the discriminator (mutant T6-M1 kills exactly
/// here).
#[test]
fn t6_tail_swap_fire_not_shove_control() {
    let rel =
        "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap_fire_notshove.dsn";
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|50000:50000;94000:94000",
            "1|94000:94000;83000:94000;45000:56000",
        ],
        "swap refused for the USER_FIXED contact: S survives, F stops blocked"
    );
}

/// The parse-consolidation face that the SHOVE-FIXED overlap worlds
/// run into (jar `tail45_swap_diag_worlds.jsonl`): an anti-parallel
/// free wire fully covering a SHOVE-FIXED wire is consolidated AT
/// PARSE into ONE UNFIXED trace spanning the uncovered remainder —
/// the SHOVE-FIXED state does not survive the consolidation (the
/// `(type protect)` twin keeps BOTH traces, jar
/// `tail45_swap_diag_notshove_worlds.jsonl`). The jar's single parse
/// trace carries id 6 (the SECOND wire's insert id); the id-parity
/// assert pins the Rust id assignment too. The subsequent corner
/// world is the correct arm on the SW diagonal (the 41483 pad-exit
/// rebuild + SHOVE-FIXED stub, the mirror of
/// [`t6_tail_correct_arm_jar_literal`]'s 58517 literal).
#[test]
fn t6_tail_swap_diag_parse_consolidation_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap_diag.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|50000:50000;38000:38000"],
        "parse consolidation: the shove-fixed overlap is eaten to one unfixed trace"
    );
    let id = board
        .iter_descending()
        .find(|entry| matches!(entry.data, ItemData::Trace { .. }))
        .map(|entry| entry.id.get() as i32)
        .expect("single trace");
    assert_eq!(id, 6, "the surviving trace carries the SECOND wire's id");
    let (_manager, board) = run_45_corner_world(rel, 6);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|41483:50000;41483:41483;38000:38000",
            "1|50000:50000;41483:50000",
        ],
        "correct arm on the SW diagonal: 41483 exit rebuild + shove-fixed stub"
    );
}

/// The `(type protect)` NOT-SHOVE control of the diag-parse-consolidation
/// world (jar `tail45_swap_diag_notshove_worlds.jsonl`, re-derived fresh
/// from the committed unshadowed jar in the M4-T6 quality round —
/// byte-identical to the T6 capture): with the covering wire protected,
/// NOTHING consolidates at parse — BOTH wires survive (trace 3
/// USER_FIXED `50000:50000;94000:94000`, trace 6 UNFIXED
/// `50000:50000;38000:38000`) — and the corner world keeps BOTH traces
/// after the run: the 41483 exit rebuild + the SHOVE_FIXED stub on
/// trace 6's slot (new id 8) beside the untouched USER_FIXED wire. This
/// control makes [`t6_tail_swap_diag_parse_consolidation_jar_literal`] a
/// 2x2 discriminator (world x protect-face): the consolidation face
/// exists only when the covering wire is UNPROTECTED.
#[test]
fn t6_tail_swap_diag_notshove_outcome_jar_literal() {
    let rel =
        "../../../rust/harness/fixtures/trace-tightener/tightener45_tail_swap_diag_notshove.dsn";
    let (_manager, board) = parse_fixture_rel(rel);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|50000:50000;38000:38000", "1|50000:50000;94000:94000",],
        "protect twin parse face: nothing consolidates — both wires survive"
    );
    let (_manager, board) = run_45_corner_world(rel, 6);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|41483:50000;41483:41483;38000:38000",
            "1|50000:50000;41483:50000",
            "1|50000:50000;94000:94000",
        ],
        "protect twin outcome: BOTH traces survive the corner world — the 41483 \
         rebuild + SHOVE_FIXED stub beside the untouched USER_FIXED wire"
    );
}

/// The `pinEdgeToTurnDist > 0` gate arm (Java `:842`, the second
/// conjunct of the tail gate; `BoardRules.setPinEdgeToTurnDist` doc:
/// "If the value is <= 0, there are no exit restrictions"). The SAME
/// tail world with the distance forced to 0 before the opt call must
/// leave the trace UNCHANGED — the default-distance run of the same
/// fixture is the positive control ([`t6_tail_correct_arm_jar_literal`]).
/// Jar witness (`tail45_gate0_worlds.jsonl`, probe mode `tailgate0`
/// calling `board.rules.setPinEdgeToTurnDist(0)`): after == before.
#[test]
fn t6_tail_gate_zero_edge_dist_negative() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_tail.dsn";
    let (mut manager, mut board) = parse_fixture_rel(rel);
    board.rules_mut().pin_edge_to_turn_dist = 0.0;
    let joins: Vec<(i32, i32)> = {
        let entry = board
            .iter_descending()
            .find(|entry| entry.id.get() as i32 == 3)
            .expect("focal id present");
        let lines = board.trace_polyline(entry.id).expect("focal polyline");
        (0..lines.corner_count())
            .map(|i| {
                let c = lines.corner_approx(i as i32);
                (c.x as i32, c.y as i32)
            })
            .collect()
    };
    start_marking_changed_area(&mut board);
    for (x, y) in &joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        None,
        None,
        0,
        false,
    );
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|150000:150000;50000:50000"],
        "gate0: the tail never fires with pinEdgeToTurnDist == 0"
    );
}

/// The `angleRestriction != NINETY_DEGREE` gate arm (Java `:842`,
/// first conjunct): the SAME tail geometry on a ninety-degree board
/// must stay UNCHANGED — the 90-degree tightener cannot improve the
/// cornerless diagonal and the whole tail block is skipped. Jar
/// witness (`tail90_worlds.jsonl` on `tightener90_tail.dsn`): after
/// == before (the diagonal also survives the 90-degree parse
/// untouched, so the world isolates the gate, not a parse face).
#[test]
fn t6_tail_gate_ninety_degree_negative() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener90_tail.dsn";
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec!["1|150000:150000;50000:50000"],
        "gate90: the tail never fires on a ninety-degree board"
    );
}

/// The `c_min_corner_dist_square` boundary (Java
/// `TraceTightener.java:36`, consumed by `skipSegmentsOfLength0`
/// `:352`): middle corners (NOT the exact-retention i==1 / i==len-2
/// faces — those are pinned by the T3 last-corner-skip worlds) closer
/// than sqrt(0.9) DBU collapse. Direct pin through the production
/// `skip_segments_of_length_0` on hand-built 3-corner polylines whose
/// middle-corner gap squared distance brackets the constant: 0.5
/// (skip) and 1.0 (keep) — the two NEAREST points of the exactly
/// constructible 45° lattice (integer anchors + 45°-multiple lines
/// only admit middle gaps d² ∈ {k²/2, k², …}; no constructible world
/// lands nearer 0.9, so the pinned envelope is (0.5, 1.0] — a mutant
/// constant drifting INSIDE it survives, disclosed here; both coarse
/// directions die, mutation-verified: c ≤ 0.5 keeps the skip world
/// (`mut_T4-M5b_cmin_0p4.log`), c > 1.0 skips the keep world
/// (`mut_T4-M5a_cmin_1p5.log` — whose first run SURVIVED an E/N/E
/// keep world whose parallel-collapse veto, not the threshold, did
/// the rejecting; the keep world was redesigned E/N/SE so the
/// threshold alone vetoes). All-Int construction: the Rational route
/// through
/// `Polyline::from_points` panics (line.rs implements IntPoint lines
/// only); the cap lines mirror `Polyline::from_two_corners`
/// (`Line::get_instance(corner, dir.turn_45_degree(2))` — the same
/// caps `from_polygon` builds). Each tiny middle segment is
/// 45°-multiple, so `skipSegmentsOfLength0` runs its
/// no-clearance-check branch for it (Java :355-357) — the face under
/// test is purely the distance threshold.
#[test]
fn t4_skip_length0_min_corner_dist_boundary() {
    let (manager, board) = parse_fixture();
    let mut state =
        TraceTightener::get_instance(&manager, &board, &[], None, 100, None, 0, false, None, 0);
    let mut manager = manager;
    let mut board = board;
    let pt = |x: i32, y: i32| Point::int(IntPoint::new(x, y));
    let dir_e = Direction::get_instance_from_points(&pt(0, 0), &pt(1, 0))
        .expect("distinct points have a direction");

    // Skip world: E → NE → SE. corner(1) = (10000, 0) (E∩NE),
    // corner(2) = (10000.5, 0.5) (NE∩SE): d² = 0.5 < 0.9 → the NE
    // line collapses (no consecutive parallels remain).
    let skip_l2 = Line::new(pt(10000, 0), pt(10001, 1));
    let skip_l3 = Line::new(pt(10001, 0), pt(10002, -1));
    let skip_cap4 = skip_l3.direction().clone().turn_45_degree(2);
    let skip_world = Polyline::new(vec![
        Line::get_instance(pt(0, 0), dir_e.turn_45_degree(2)),
        Line::new(pt(0, 0), pt(10000, 0)),
        skip_l2,
        skip_l3,
        Line::get_instance(pt(10002, -1), skip_cap4),
    ]);
    assert_eq!(skip_world.lines.len(), 5, "skip world: 3 corners + 2 caps");
    let d2_skip = skip_world
        .corner_approx(2)
        .distance_square(&skip_world.corner_approx(1));
    assert_eq!(d2_skip, 0.5, "skip world middle gap (the pin's premise)");

    // Keep world: E → N → SE. corner(1) = (10000, 0), corner(2) =
    // (10000, 1): d² = 1.0 ≥ 0.9 → no skip (5 lines in, 5 out). The
    // middle N line is deliberately flanked by E and SE (NOT E and E):
    // an E/N/E world's skip attempt is vetoed by the
    // consecutive-parallel collapse of `Polyline::new`, not by the
    // threshold — the veto must not do the constant's job (mutation
    // round T4-M5a: constant → 1.5 survived the E/N/E world before
    // this redesign; it kills this one, since removal here is
    // otherwise valid and the 45° fast path accepts it).
    let keep_l2 = Line::new(pt(10000, 0), pt(10000, 1));
    let keep_l3 = Line::new(pt(10001, 0), pt(10002, -1));
    let keep_cap4 = keep_l3.direction().clone().turn_45_degree(2);
    let keep_world = Polyline::new(vec![
        Line::get_instance(pt(0, 0), dir_e.turn_45_degree(2)),
        Line::new(pt(0, 0), pt(10000, 0)),
        keep_l2,
        keep_l3,
        Line::get_instance(pt(10002, -1), keep_cap4),
    ]);
    assert_eq!(keep_world.lines.len(), 5, "keep world: 3 corners + 2 caps");
    let d2_keep = keep_world
        .corner_approx(2)
        .distance_square(&keep_world.corner_approx(1));
    assert_eq!(d2_keep, 1.0, "keep world middle gap (the pin's premise)");

    let shortened = state.skip_segments_of_length_0(&mut manager, &mut board, &skip_world);
    assert_eq!(
        shortened.lines.len(),
        4,
        "d² = 0.5 < 0.9: the middle corner collapses"
    );
    let kept = state.skip_segments_of_length_0(&mut manager, &mut board, &keep_world);
    assert_eq!(
        kept.lines.len(),
        5,
        "d² = 1.0 >= 0.9: the middle corner stays"
    );
}

// ---------------------------------------------------------------------------
// M4-T5 via-relocation pins (jar oracle: `TraceTightenerProbe.java
// viaworlds` on via_optimizer45.dsn; capture
// `logs/M4-T5/evidence/via45_viaworlds_run4.jsonl` + run4b — double-run
// byte-identical, and the first 35 lines byte-identical to the run3
// matrix). Every literal below is read off the capture rows; the
// one-call intermediate of t5_depth_gate is jar-witnessed by the
// probe's `viadepth` mode (logs/M4-T5/evidence/via45_costs_diag.txt,
// double-run byte-identical: depth 0 refuses; depth 1 lands the N6
// via on (790000,450000); a single depth-10 call lands on
// (660000,390000) — the same final as the run4/run5 seam rows).
//
// Nets in network order: N1=1 N2=2 N3=3 N4=4 N6=5 N8=6 N9=7 N5=8.
// Via ids 5,9,11,13,16,19,23 (the capture's inventory rows); the N5
// via (id 23, net 8) carries (type shoveFixed).
// ---------------------------------------------------------------------------

/// The costs tables of the probe's `runViaWorlds`: 0 = uniform (1,1),
/// 1 = F.Cu (1,3) / B.Cu (3,1) (layer index = cost index).
fn costs_uniform() -> [super::TraceCostFactor; 2] {
    [
        super::TraceCostFactor {
            horizontal: 1.0,
            vertical: 1.0,
        },
        super::TraceCostFactor {
            horizontal: 1.0,
            vertical: 1.0,
        },
    ]
}

fn costs_f13_b31() -> [super::TraceCostFactor; 2] {
    [
        super::TraceCostFactor {
            horizontal: 1.0,
            vertical: 3.0,
        },
        super::TraceCostFactor {
            horizontal: 3.0,
            vertical: 1.0,
        },
    ]
}

/// The via carrying `net` (the fixture has exactly one per net).
#[track_caller]
fn via_by_net(board: &crate::board::Board, net: i32) -> ItemId {
    board
        .iter_descending()
        .find(|entry| entry.nets == vec![net] && matches!(entry.data, ItemData::Via { .. }))
        .map(|entry| entry.id)
        .expect("fixture via on net")
}

/// The center of a live via as integral DBU (all fixture geometry is
/// integral; the capture rows are Double.toString of integral values).
#[track_caller]
fn via_center(board: &crate::board::Board, id: ItemId) -> (i32, i32) {
    match board.drill_center(id).expect("live via center") {
        Point::Int(p) => (p.x, p.y),
        Point::Rational(_) => panic!("integral DBU domain"),
    }
}

/// All 7 via centers in ASCENDING id order (the probe's `viaCenters`).
#[track_caller]
fn assert_via_centers(board: &crate::board::Board, expected_ascending_id: [(i32, i32); 7]) {
    let mut ids: Vec<ItemId> = board
        .iter_descending()
        .filter(|entry| matches!(entry.data, ItemData::Via { .. }))
        .map(|entry| entry.id)
        .collect();
    ids.reverse();
    assert_eq!(ids.len(), 7, "7 fixture vias");
    for (id, expected) in ids.iter().zip(expected_ascending_id.iter()) {
        assert_eq!(via_center(board, *id), *expected, "via {id:?} center");
    }
}

/// The capture's before centers (ids 5,9,11,13,16,19,23 ascending).
const BEFORE_CENTERS: [(i32, i32); 7] = [
    (600000, 380000), // N1 collinear
    (200000, 450000), // N2 gate3
    (900000, 150000), // N3 fanout
    (520000, 330000), // N4 projection
    (720000, 450000), // N6 costs
    (200000, 150000), // N8 acute
    (450000, 600000), // N5 shove-fixed
];

/// One probe via world: fresh parse of via_optimizer45.dsn, changed-area
/// session + the join plan on layer 0, then the free `opt_changed_area`
/// behind the PRODUCTION seam with the probe's arguments
/// (`only_nets = []`, clip `None`, accuracy 100, no keep point, the
/// world's `trace_costs`, no stopper, no budget).
#[track_caller]
fn run_via_world(
    joins: &[(i32, i32)],
    trace_costs: Option<&[super::TraceCostFactor]>,
) -> (SearchTreeManager, crate::board::Board) {
    let (mut manager, mut board) =
        parse_fixture_rel("../../../rust/harness/fixtures/trace-tightener/via_optimizer45.dsn");
    start_marking_changed_area(&mut board);
    for (x, y) in joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        trace_costs,
        None,
        0,
        false,
    );
    (manager, board)
}

/// The capture's before rows: parse id parity (5,9,11,13,16,19,23),
/// center parity, and exactly ONE shove-fixed via (N5, the P8 gate
/// premise).
#[test]
fn t5_via_fixture_parse_parity() {
    let (_manager, board) = run_via_world(&[], Some(&costs_uniform()));
    let ids: Vec<ItemId> = board
        .iter_descending()
        .filter(|entry| matches!(entry.data, ItemData::Via { .. }))
        .map(|entry| entry.id)
        .collect();
    let mut ids = ids;
    ids.reverse();
    let ids: Vec<i32> = ids.iter().map(|id| id.get() as i32).collect();
    assert_eq!(ids, vec![5, 9, 11, 13, 16, 19, 23], "via id parity");
    assert_via_centers(&board, BEFORE_CENTERS);
    for (net, expected_fixed) in [(1, false), (8, true)] {
        let id = via_by_net(&board, net);
        assert_eq!(
            crate::trace_ops::is_shove_fixed(&board, id),
            expected_fixed,
            "shoveFixed premise for net {net}"
        );
    }
}

/// Collinear world (uniform costs): the 11-arg COLLINEAR arm walks N1
/// west along the y=380000 line to (400000,380000); everything else
/// stays. (Capture `collinear` after row.)
///
/// BANKED HEALER (T5-M1, `mut_T5-M1_collinear_arm_dropped.log`):
/// disabling the COLLINEAR arm leaves this pin GREEN — on collinear
/// geometry the acute arm's else branch (uniform-cost tie → toPoint2
/// first) issues the IDENTICAL `repositionVia(secondCorner, FIRST
/// params)` call, so the one-call result and the fixpoint final
/// coincide by construction (same class as T3-M2/T3-Q4/T4-M1).
/// Witnessing the break needs a world where the arms' first moves
/// diverge (collinear pair + non-uniform layer costs where the acute
/// wd comparison fires toPoint1 first) — a new fixture + capture; the
/// arm order itself is jar-trace-pinned (the jar's collinear world
/// opens with the COLLINEAR arm).
#[test]
fn t5_collinear_world_moves_to_the_nearer_bend() {
    let (_manager, board) = run_via_world(&[(600000, 380000)], Some(&costs_uniform()));
    assert_via_centers(
        &board,
        [
            (400000, 380000),
            (200000, 450000),
            (900000, 150000),
            (520000, 330000),
            (720000, 450000),
            (200000, 150000),
            (450000, 600000),
        ],
    );
}

/// Gate3 world (uniform costs): a via with THREE trace contacts is
/// untouched — the <=2 gate. (Capture `gate3` after row.)
///
/// PIN LIMIT (T5-S1b, bankable survivor): this pins the OUTCOME
/// "N2 stays", not the predicate — in this world the downstream scan
/// keeps N2 put even un-gated. Witnessing the gate itself needs a
/// 3-contact via whose first two descending-id contacts are clean
/// movable traces (spec-review ledger, `spec-review-t5-1.md`).
#[test]
fn t5_gate3_three_contacts_stay() {
    let (_manager, board) = run_via_world(&[(200000, 450000)], Some(&costs_uniform()));
    assert_via_centers(&board, BEFORE_CENTERS);
}

/// Fanout world (uniform costs): the single-contact face walks N3 along
/// its own trace to the far end (960000,150000). (Capture `fanout`.)
#[test]
fn t5_fanout_world_walks_along_its_trace() {
    let (_manager, board) = run_via_world(&[(900000, 150000)], Some(&costs_uniform()));
    assert_via_centers(
        &board,
        [
            (600000, 380000),
            (200000, 450000),
            (960000, 150000),
            (520000, 330000),
            (720000, 450000),
            (200000, 150000),
            (450000, 600000),
        ],
    );
}

/// Projection world (uniform costs): the direct SW move of N4 is
/// mover-blocked by the N9 B.Cu wire, the 6-arg bisection exhausts, and
/// the projection face drops the via onto the prev line; the fixpoint
/// dance lands at (400000,290000). (Capture `projection`.)
#[test]
fn t5_projection_world_dances_to_the_prev_line() {
    let (_manager, board) = run_via_world(&[(520000, 330000)], Some(&costs_uniform()));
    assert_via_centers(
        &board,
        [
            (600000, 380000),
            (200000, 450000),
            (900000, 150000),
            (400000, 290000),
            (720000, 450000),
            (200000, 150000),
            (450000, 600000),
        ],
    );
}

/// Costs world (F.Cu (1,3) / B.Cu (3,1)): the weighted-distance arm 1
/// fires — the depth-1 one-call relocation lands the via east on
/// (790000,450000) — and the recursion's fanout face then walks it
/// back; the fixpoint final is the F corner (660000,390000). (Capture
/// `costs` after row; the one-call and production-depth endpoints are
/// jar-witnessed per-depth in evidence/via45_costs_diag.txt.)
#[test]
fn t5_costs_world_relocates_to_the_f_corner() {
    let (_manager, board) = run_via_world(&[(720000, 450000)], Some(&costs_f13_b31()));
    assert_via_centers(
        &board,
        [
            (600000, 380000),
            (200000, 450000),
            (900000, 150000),
            (520000, 330000),
            (660000, 390000),
            (200000, 150000),
            (450000, 600000),
        ],
    );
}

/// The reject cell of the 2x2 (costs x movement): the SAME N6 geometry
/// under uniform (1,1) costs ties every weighted-distance comparison
/// and the via STAYS. (Capture `costs_uniform` after row.)
#[test]
fn t5_costs_uniform_reject_cell_stays() {
    let (_manager, board) = run_via_world(&[(720000, 450000)], Some(&costs_uniform()));
    assert_via_centers(&board, BEFORE_CENTERS);
}

/// Acute world (uniform costs): scalarProduct > 0, non-collinear — the
/// acute-angle arm fires and the dance lands N8 at (140000,90000).
/// (Capture `acute` after row.)
#[test]
fn t5_acute_world_moves_through_the_acute_arm() {
    let (_manager, board) = run_via_world(&[(200000, 150000)], Some(&costs_uniform()));
    assert_via_centers(
        &board,
        [
            (600000, 380000),
            (200000, 450000),
            (900000, 150000),
            (520000, 330000),
            (720000, 450000),
            (140000, 90000),
            (450000, 600000),
        ],
    );
}

/// The shove-fixed gate's crossing cell: N5 carries the same movable
/// 2-trace westward geometry as N1 AND the join is AT its center (the
/// opt pass processes it) — but `(type shoveFixed)` must hold it while
/// the unfixed N1 twin moves. (Capture `shove_fixed` after row.)
///
/// DEFENSE IN DEPTH (T5-M4/M4b, `mut_T5-M4*.log`): Java guards the
/// behavior TWICE — `optViaLocation`'s head gate AND the same
/// `isShoveFixed` head check in `DrillItemMover.check`/`insert` — and
/// the port mirrors both. Dropping the optimizer gate alone (T5-M4)
/// survives: the mover's guard refuses every move. Dropping BOTH
/// (T5-M4b, two-site mutant) kills this pin — the via walks west. The
/// behavior is pinned end-to-end; the single-site survivor is a
/// redundant-guard artifact, not a pin vacuity.
#[test]
fn t5_shove_fixed_gate_holds_a_joined_movable_via() {
    let (_manager, board) = run_via_world(&[(450000, 600000)], Some(&costs_uniform()));
    assert_via_centers(&board, BEFORE_CENTERS);
}

/// The consumption gate's crossing cell: N1's movable geometry joined
/// at its center with `traceCosts == null` — the whole via arm is dead
/// and N1 STAYS (while P1, costs present, moves it). (Capture
/// `costs_null` after row.)
#[test]
fn t5_costs_null_consumption_gate_keeps_the_arm_dead() {
    let (_manager, board) = run_via_world(&[(600000, 380000)], None);
    assert_via_centers(&board, BEFORE_CENTERS);
}

/// The T6 ledger world for the T5-M1 banked healer: the SAME N1
/// collinear geometry joined at its center, but under the
/// NON-UNIFORM F.Cu (1,3) / B.Cu (3,1) cost table (jar
/// `viaworlds` world `collinear_costs`,
/// `via45_viaworlds_t6_ledger.jsonl`): the collinear arm still walks
/// N1 west to the nearer bend (400000,380000). This is the world
/// where the collinear arm's absence could first diverge — with
/// non-uniform costs the acute arm's wd comparison need not tie, so
/// the T5-M1 heal (the acute arm issuing the identical
/// `repositionVia(secondCorner, FIRST params)` call) is no longer
/// forced by symmetry. (Mutant re-run verdict recorded in the task
/// report; the uniform-cost twin is
/// [`t5_collinear_world_moves_to_the_nearer_bend`].)
#[test]
fn t6_collinear_costs_ledger_world_jar_literal() {
    let (_manager, board) = run_via_world(&[(600000, 380000)], Some(&costs_f13_b31()));
    assert_via_centers(
        &board,
        [
            (400000, 380000),
            (200000, 450000),
            (900000, 150000),
            (520000, 330000),
            (720000, 450000),
            (200000, 150000),
            (450000, 600000),
        ],
    );
}

/// The recursion-depth gate boundary (production budget 10 through the
/// seam is exercised by every other pin): driven DIRECTLY on the
/// production `opt_via_location` — depth 0 answers false and leaves the
/// board untouched; depth 1 performs exactly the one-call arm-1
/// relocation (jar-witnessed intermediate (790000,450000),
/// evidence/via45_costs_diag.txt depth-1 row) without the fanout walk.
#[test]
fn t5_depth_gate_boundary() {
    let costs = costs_f13_b31();
    // Depth 0: the gate arm.
    {
        let (mut manager, mut board) =
            parse_fixture_rel("../../../rust/harness/fixtures/trace-tightener/via_optimizer45.dsn");
        let mut state =
            TraceTightener::get_instance(&manager, &board, &[], None, 100, None, 0, false, None, 0);
        let via = via_by_net(&board, 5);
        let moved = super::via_optimizer::opt_via_location(
            &mut state,
            &mut manager,
            &mut board,
            via,
            Some(&costs),
            100,
            0,
        );
        assert!(!moved, "depth 0: the gate refuses");
        assert_eq!(
            via_center(&board, via),
            (720000, 450000),
            "depth 0: no move"
        );
    }
    // Depth 1: exactly one relocation, no recursion walk.
    {
        let (mut manager, mut board) =
            parse_fixture_rel("../../../rust/harness/fixtures/trace-tightener/via_optimizer45.dsn");
        let mut state =
            TraceTightener::get_instance(&manager, &board, &[], None, 100, None, 0, false, None, 0);
        let via = via_by_net(&board, 5);
        let moved = super::via_optimizer::opt_via_location(
            &mut state,
            &mut manager,
            &mut board,
            via,
            Some(&costs),
            100,
            1,
        );
        assert!(moved, "depth 1: the cost arm relocates");
        assert_eq!(
            via_center(&board, via),
            (790000, 450000),
            "depth 1: the jar's one-call arm-1 intermediate"
        );
    }
}

// ======================================================================
// T6 ledger-consolidation worlds (the banked-mutant re-triage). Both
// pins are jar-literal finals of worlds crafted to isolate the
// `repositionLine` translate loop; both document WHY the banked
// healers (T3-M2/T3-Q4/T4-M1/T4-M2) heal in every constructed world,
// the T3-Q4 permanent-close evidence (see the module-head ledger note
// and logs/M4-T6/evidence/).
// ======================================================================

/// The blocked S-bend world (`tightener45_bisect.dsn`; jar witness
/// `evidence/tightener45_bisect_cornerworlds.jsonl`, probe mode
/// `cornerworlds`, focal trace 3): N1 carries a 6-corner S-bend whose
/// every corner-direct connection is non-45 (so the skip/reduce arms
/// are lattice-blocked EXCEPT the tail merge at corner 4, whose direct
/// NE line `y = x - 600000` misses both foreign-net blockers B1/N2 and
/// B2/N3) — the jar shows the fixpoint exits through exactly that
/// tail merge (6 corners → 3), so `repositionLines` (the ≥5-line
/// translate loop hosting the T3-Q4 `first_time` break and the T4-M1
/// nearest-corner tie face) NEVER FIRES: the two translate-loop
/// banked mutants are preempted by the skip arm in every world whose
/// skip corridors are not individually blocked. This pin holds that
/// preemption shape as a regression fact.
#[test]
fn t6_bisect45_skip_preempt_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_bisect.dsn";
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|600000:100000;700000:100000;900000:300000",
            "2|630000:200000;670000:200000",
            "3|830000:150000;870000:150000",
            "4|100000:100000;200000:100000;300000:200000;300000:300000;400000:300000",
        ],
        "bisect world (focal N1): the fixpoint exits through the tail-corner \
         merge; the reposition translate loop never fires; the non-focal \
         traces stay untouched in this world"
    );
}

/// The dyadic-probe world (`tightener45_dyadic.dsn`; jar witness
/// `evidence/tightener45_dyadic_cornerworlds.jsonl`, focal trace 3):
/// N1's S-bend has ALL corner-directs non-45 (no skip arm can fire)
/// and the N2 blocker sits ON the `repositionLine` first-probe
/// (snap-through-nearest-corner) corridor `y = 250000`, forcing the
/// translate loop into its binary-search body — the only constructed
/// route to a half-DBU (dyadic) accepted line, whose next-sweep
/// corners would be `Point::Rational` and thus dodge the snap face
/// (the T3-Q4 residual surface). The jar's terminal is ALL-INTEGER
/// (`245459` band with 45-degree stubs — the engine's fixpoint
/// re-integerizes through the interplay of its arms), empirically
/// confirming the structural close: every terminal reachable from
/// integer DSN geometry is a through-corner or boundary line in the
/// integer lattice, so the break-vs-continue difference of the
/// `first_time` acceptance is unobservable in ANY final (the residual
/// would need a float-epsilon straddle of the side-check at a
/// diagonal-direction corner — the same JVM-arithmetic-luck class the
/// T4-M1 close already excludes as uncraftable). Pin: the jar's
/// integer terminal, both traces.
#[test]
fn t6_dyadic45_integer_terminal_jar_literal() {
    let rel = "../../../rust/harness/fixtures/trace-tightener/tightener45_dyadic.dsn";
    let (_manager, board) = run_45_corner_world(rel, 3);
    assert_eq!(
        trace_fingerprint(&board),
        vec![
            "1|0:250000;100000:250000;104541:245459;645459:245459;750000:350000",
            "2|300000:250000;350000:250000",
        ],
        "dyadic world: the fixpoint terminal is integer-lattice; \
         no Rational corner survives to the final"
    );
}

// ------------------------------------------------------------------
// M7-T3 - the min-length honoring pins (beyond-Java contract)
// ------------------------------------------------------------------

/// Inline DSN for the honoring worlds: the 6-corner 90-degree
/// staircase on N1 (length 160000+60000+120000+60000+80000 =
/// 480000 board DBU) under class `cmin` with a `(circuit (length
/// -1 MIN))` declaration, plus the unconstrained N2 cup witness
/// (class kicad_default). Unconstrained, the tightener folds the
/// staircase to the L `(200000,400000),(200000,280000),
/// (160000,280000)` in ONE candidate (length exactly 160000 - the
/// t3 stair pin's final), which makes the min boundary
/// hand-checkable at 160000.
fn m7t3_min_stair_dsn(min_dsn: &str) -> String {
    format!(
        r#"(PCB min_stair.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 120000 60000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (library
    (padstack "VIA_PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
  )
  (network
    (via VT VIA_PAD kicad_default)
    (net "N1")
    (net "N2")
    (class cmin "N1"
      (circuit (length {min_dsn}))
      (rule (clearance 200)))
    (class kicad_default "N2" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 20000 40000 36000 40000 36000 34000 24000 34000 24000 28000 16000 28000) (net N1))
    (wire (path F.Cu 200 100000 20000 106000 20000 106000 18000 100000 18000) (net N2))
  )
)"#
    )
}

/// One honoring world: parse (the `(circuit (length ...))`
/// declaration rides the T1 read/deliver chain), tree + normalize,
/// the tuning flag, then the changed-area tightener run over
/// `joins` behind the PRODUCTION seam.
fn m7t3_min_stair_world(
    min_dsn: &str,
    tuning_active: bool,
    joins: &[(i32, i32)],
) -> crate::board::Board {
    let mut board = crate::test_util::parse_board_from_text(&m7t3_min_stair_dsn(min_dsn));
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    crate::normalize_all::normalize_all_traces(&mut manager, &mut board);
    board.set_tuning_active(tuning_active);
    start_marking_changed_area(&mut board);
    for (x, y) in joins {
        join_changed_area(
            &mut board,
            &FloatPoint::new(f64::from(*x), f64::from(*y)),
            0,
        );
    }
    opt_changed_area(
        &mut manager,
        &mut board,
        &mut TraceTightenerSeam,
        &[],
        None,
        100,
        None,
        0,
        None,
        None,
        0,
        false,
    );
    board
}

/// The N1 staircase's corner chain.
fn m7t3_stair_corners(board: &crate::board::Board) -> Vec<(i64, i64)> {
    for entry in board.iter_descending() {
        let ItemData::Trace {
            layer: 0, lines, ..
        } = &entry.data
        else {
            continue;
        };
        if !entry.nets.contains(&1) {
            continue;
        }
        return (0..lines.corner_count())
            .map(|i| {
                let c = lines.corner_approx(i as i32);
                (c.x as i64, c.y as i64)
            })
            .collect();
    }
    Vec::new()
}

const M7T3_STAIR_JOINS: [(i32, i32); 6] = [
    (200000, 400000),
    (360000, 400000),
    (360000, 340000),
    (240000, 340000),
    (240000, 280000),
    (160000, 280000),
];
const M7T3_STAIR_BEFORE: [(i64, i64); 6] = [
    (200000, 400000),
    (360000, 400000),
    (360000, 340000),
    (240000, 340000),
    (240000, 280000),
    (160000, 280000),
];
/// The unconstrained fold terminal (the t3 stair pin's final): the
/// staircase collapses to this L, length exactly 160000.
const M7T3_STAIR_FOLDED: [(i64, i64); 3] = [(200000, 400000), (200000, 280000), (160000, 280000)];

/// A constrained net at EXACTLY its `min` (= current trace length
/// 480000): the tightener leaves it byte-stable - every fold
/// candidate would go below `min`, so the gate rejects WHOLE.
/// Kills the gate-removed mutant (the trace would fold to the L).
#[test]
fn m7t3_min_honored_at_exact_length_tightener_leaves_it() {
    // DSN 48000 x10 = 480000 board DBU = the trace's own length.
    let board = m7t3_min_stair_world("-1 48000", true, M7T3_STAIR_JOINS.as_slice());
    assert_eq!(
        m7t3_stair_corners(&board),
        M7T3_STAIR_BEFORE.to_vec(),
        "trace at exactly min: no optimization pass may shorten it"
    );
}

/// The EXACT boundary row (DNR-16, both directions): the fold
/// candidate is length 160000, so
/// * min = 160000 - the candidate lands AT min exactly (allowed:
///   the contract is "never below"), final = the folded L;
/// * min = 160001 (one DBU above) - the candidate is rejected
///   WHOLE, final = the untouched staircase.
///
/// The `<=` mutant (reject at exactly min) dies on the first arm, the
/// `< min - 1` (accept below min) mutant on the second.
#[test]
fn m7t3_min_boundary_exact_tightens_to_min_both_directions() {
    // DSN 16000 x10 = 160000 = the candidate length exactly.
    let at_min = m7t3_min_stair_world("-1 16000", true, M7T3_STAIR_JOINS.as_slice());
    assert_eq!(
        m7t3_stair_corners(&at_min),
        M7T3_STAIR_FOLDED.to_vec(),
        "tightens TO min exactly (160000), never below"
    );
    // DSN 16000.1 x10 = 160001: one DBU above the boundary.
    let above = m7t3_min_stair_world("-1 16000.1", true, M7T3_STAIR_JOINS.as_slice());
    assert_eq!(
        m7t3_stair_corners(&above),
        M7T3_STAIR_BEFORE.to_vec(),
        "one DBU above min: the fold is forbidden whole"
    );
}

/// The isolation pin: on the SAME board, the unconstrained
/// neighbor (N2 cup) still tightens normally while the constrained
/// staircase is protected.
#[test]
fn m7t3_unconstrained_neighbor_still_tightens() {
    let board = m7t3_min_stair_world(
        "-1 48000",
        true,
        &[
            (200000, 400000),
            (360000, 400000),
            (360000, 340000),
            (240000, 340000),
            (240000, 280000),
            (160000, 280000),
            (994000, 180000),
        ],
    );
    assert_eq!(
        m7t3_stair_corners(&board),
        M7T3_STAIR_BEFORE.to_vec(),
        "constrained N1 protected"
    );
    // The cup folds to the west bar exactly as in the unconstrained
    // t3 region pin (join (994000,180000)).
    assert_corners(
        &board,
        trace_by_net(&board, 2, 3),
        &[(1000000, 200000), (1000000, 180000)],
    );
}

/// The INERT face (the two-regime rule): with the `(length)`
/// declaration PRESENT but the tuning flag OFF, the gate is inert
/// and the staircase folds normally. Kills the always-gate mutant.
#[test]
fn m7t3_gate_inert_without_tuning_flag() {
    let board = m7t3_min_stair_world("-1 48000", false, M7T3_STAIR_JOINS.as_slice());
    assert_eq!(
        m7t3_stair_corners(&board),
        M7T3_STAIR_FOLDED.to_vec(),
        "flag off = parity regime: the gate never fires"
    );
}
