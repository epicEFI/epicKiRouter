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

use super::{TraceTightenerSeam, active_for};
use crate::id::ItemId;
use crate::items::ItemData;
use crate::routing_board_insert::{
    join_changed_area, opt_changed_area, start_marking_changed_area,
};
use crate::rules_surf::AngleRestriction;
use crate::test_util::parse_board_from_path;
use crate::tree_manager::SearchTreeManager;
use epic_geometry::float_point::FloatPoint;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A fresh fixture parse + tree reinsert (the probe's `parse`).
fn parse_fixture() -> (SearchTreeManager, crate::board::Board) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rust/harness/fixtures/trace-tightener/tightener90.dsn");
    let mut board = parse_board_from_path(&path.to_string_lossy());
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
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

/// Every trace's corner chain, id-tagged, descending-id order — the
/// whole-board geometry fingerprint for the inertness pin.
fn trace_chains(board: &crate::board::Board) -> Vec<(i32, Vec<(i32, i32)>)> {
    board
        .iter_descending()
        .filter(|entry| matches!(entry.data, ItemData::Trace { .. }))
        .map(|entry| {
            let lines = board.trace_polyline(entry.id).expect("live trace");
            (
                entry.id.get() as i32,
                (0..lines.corner_count())
                    .map(|i| {
                        let c = lines.corner_approx(i as i32);
                        (c.x as i32, c.y as i32)
                    })
                    .collect(),
            )
        })
        .collect()
}

/// The 45°-inertness pin (quality-review MINOR-1, reviewer mutant
/// T3-Q1): the seam's 45° face is asserted BYTE-EXACT end to end, not
/// just at the gate value. Two arms:
/// (a) the production-adjacent locator fixture `t9_locator45.dsn` —
/// it carries NO routed traces (pins + keepouts only), so its
/// inertness is STRUCTURAL; this arm pins the gate face on the shared
/// fixture and survives T3-M6 by construction (disclosed, not the
/// killer);
/// (b) the killer: the crafted 45° board `tightener45.dsn` (one
/// 6-corner 45° staircase) with the region joined over the trace's
/// own corners — under the dormancy contract nothing moves. NOT
/// vacuous: mutant T3-M6 (the T4 mis-integration shape — `active_for`
/// wired open AND the 45° arm of `pull_tight_polyline` dispatched to
/// the 90° tightener) collapses the staircase and dies at the
/// byte-exact assert. Gate-open ALONE stays green by design (T3-Q1:
/// the T4 fallback is identity) — the pin dies only when inertness is
/// actually broken.
#[test]
fn t3_seam_inert_on_fortyfive_degree_boards() {
    for (rel, joins) in [
        (
            "../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn",
            vec![(200000, 300000), (800000, 300000), (150000, 450000)],
        ),
        (
            "../../../rust/harness/fixtures/trace-tightener/tightener45.dsn",
            vec![
                (100000, 100000),
                (200000, 100000),
                (300000, 200000),
                (400000, 200000),
                (500000, 300000),
                (600000, 300000),
            ],
        ),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let before = trace_chains(&board);
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
        let after = trace_chains(&board);
        assert_eq!(before, after, "45° board byte-exact under the seam: {rel}");
    }
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

/// The dispatch pin (M4-T3 census): `active_for` is the getInstance
/// class-selection face — TRUE only for `NinetyDegree`. Parsed-fixture
/// arms: the 90° fixture (this one) activates; the 45° locator and the
/// any-angle locator stay no-ops. The full 23-tier-fixture census
/// (0 ninety / 23 fortyfive / 0 any-angle) rests on the Java parser
/// DEFAULT — no tier fixture carries a `snap_angle` keyword at all
/// (`ReadScopeParameter.snapAngle`, `ReadScopeParameter.java:59`,
/// defaults to FORTYFIVE_DEGREE; spec-review whole-root sweep: 0 hits
/// across all 2334 fixture DSNs) — and is machine-tripwired by the
/// dsn-compare goldens' Java-captured `snap_angle:"45"` field
/// (1331/1331 parseable fixtures); the ENUM mapping is pinned here.
#[test]
fn t3_dispatch_active_for_census() {
    assert!(active_for(AngleRestriction::NinetyDegree));
    assert!(!active_for(AngleRestriction::FortyfiveDegree));
    assert!(!active_for(AngleRestriction::None));

    for (rel, expected) in [
        (
            "../../../rust/harness/fixtures/trace-tightener/tightener90.dsn",
            true,
        ),
        (
            "../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn",
            false,
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
/// fixpoint heals; if T4's 45° port changes `reposition_lines` or
/// Polyline construction, RE-PROBE BOTH — witnessing the break needs
/// a world where first-accepted ≠ fixpoint (a clearance-borderline
/// bisection). These literals pin the arm-execution path downstream
/// so a heal-break surfaces here.
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
