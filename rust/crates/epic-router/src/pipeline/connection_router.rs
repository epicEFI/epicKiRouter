//! Java `autoroute/pipeline/AutorouteConnectionRouter.java` — the
//! per-connection routing face the pass runner calls once per
//! (item, net) attempt: net derivation is the CALLER's job (Java
//! `route(item, routeNetNo, …)` — the pass runner iterates
//! `currentItem.getNetNumber(i)`), the engine is a LOCAL built per
//! call (`init_autoroute`; the OBS-R2 constraint — Java's held-engine
//! reuse face is [`crate::engine::AutorouteEngine::is_reusable_for`]),
//! with the deterministic budget, the necked retry, and the strict-DRC
//! enforcement.
//!
//! ## Banks (SEAM carries the dossier)
//!
//! * **`setAirLine` is banked** — the GUI airline field has no
//!   headless consumer; Java's calculator call (`:70`) is dropped.
//! * **The blanket `catch (Exception)` (`:162-165`) does not exist
//!   here.** Rust has no checked exceptions; a panic propagates and
//!   fails the run loudly instead of folding into an empty-details
//!   FAILED row (the IllegalArgumentException → `Result` face of the
//!   batch loop, applied consistently down the stack).
//! * **`opt_changed_area` clip sentinel.** Java passes
//!   `clipShape = null` and the operations gate
//!   (`clipShape != IntOctagon.EMPTY`) therefore RUNS the tightener
//!   unclipped; the port's `NoPullTight` seam makes that tightener a
//!   no-op, so passing the [`IntOctagon::EMPTY`] sentinel (skip the
//!   tightener, still tear down the marking session) is observably
//!   identical today. M4's pull-tight seam widening revisits.
//! * **Multi-arg trace rows are folded** into
//!   `sink.trace("<operation> <message>")` — the Java
//!   `FRLogger.trace(method, operation, message, impactedItems,
//!   impactedPoints)` method tag and impact payloads are log-only
//!   (bug-118 no-digest convention).
//! * **The `TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP` wall-clock guard**
//!   (Java passes 1000 ms into `optChangedArea`) is banked with the
//!   wall-clock family: the port's pull-tight seam is a no-op today.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use epic_board::board::Board;
use epic_board::contacts::{item_connected_set, item_unconnected_set};
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::routing_board_insert::{NoPullTight, opt_changed_area};
use epic_board::time_limit::TimeLimit;
use epic_board::trace_ops::remove_item_through_repository;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::item_clearance_violations;
use epic_dsn::state::Unit;
use epic_geometry::int_octagon::IntOctagon;

use crate::control::AutorouteControl;
use crate::engine::{
    AutorouteAttemptResult, AutorouteAttemptState, RippedItemSeed, RouteBudget, init_autoroute,
};
use crate::path::inserter::InserterEventSink;
use crate::pipeline::batch::BatchSettings;
use crate::pipeline::board_history::restore_from_snapshot;
use crate::pipeline::event_sink::DriverSink;

/// The [`InserterEventSink`] bridge: the engine's insertion rows flow
/// into the driver's sink trace channel (Java logs both through the
/// one global `FRLogger`).
struct SinkBridge<'a>(&'a mut dyn DriverSink);

impl InserterEventSink for SinkBridge<'_> {
    fn trace(&mut self, message: &str) {
        self.0.trace(message);
    }

    fn debug(&mut self, message: &str) {
        self.0.debug(message);
    }

    fn warn(&mut self, message: &str) {
        self.0.debug(message);
    }

    fn is_trace_enabled(&self) -> bool {
        self.0.is_trace_enabled()
    }
}

/// Java `AutorouteConnectionRouter.route` (`:30-161`). `route_net_no`
/// arrives from the caller (the pass runner's per-net loop); the
/// ripped-list maps are the caller-owned accumulators the locator
/// fills and the ripup block drains.
#[allow(clippy::too_many_lines)] // the Java body is one flat walk
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn route(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    item_id: ItemId,
    route_net_no: i32,
    ripped_item_list: &mut BTreeMap<i32, RippedItemSeed>,
    ripup_costs: &mut HashMap<u64, i32>,
    ripup_pass_no: i32,
    stoppable_flag: Option<&Arc<AtomicBool>>,
    sink: &mut dyn DriverSink,
) -> AutorouteAttemptResult {
    let contains_plane = board
        .rules()
        .nets
        .get(route_net_no)
        .is_some_and(|net| net.contains_plane);
    let current_via_costs = if contains_plane {
        settings.plane_via_costs
    } else {
        settings.via_costs
    };

    let mut ctrl = AutorouteControl::new_with_costs(
        board,
        route_net_no,
        &settings.router_settings,
        current_via_costs,
        &settings.trace_costs,
    );
    ctrl.ripup_allowed = true;
    // Java `router.getStartRipupCosts() * ripupPassNo` — int
    // arithmetic; `wrapping_mul` keeps the overflow face exact.
    ctrl.ripup_costs = settings.start_ripup_costs.wrapping_mul(ripup_pass_no);
    ctrl.remove_unconnected_vias = settings.remove_unconnected_vias;

    let unconnected_set = item_unconnected_set(manager, board, item_id, route_net_no);
    if unconnected_set.is_empty() {
        return AutorouteAttemptResult::new(AutorouteAttemptState::NoUnconnectedNets);
    }

    let connected_set = item_connected_set(manager, board, item_id, route_net_no);
    if contains_plane {
        // Java `:57-63`: any ConductionArea in the connected set
        // answers CONNECTED_TO_PLANE before any engine work.
        let plane_in_connected = connected_set.iter().any(|id| {
            board
                .get(id.0)
                .is_some_and(|entry| matches!(entry.data, ItemData::ConductionArea { .. }))
        });
        if plane_in_connected {
            return AutorouteAttemptResult::new(AutorouteAttemptState::ConnectedToPlane);
        }
    }
    let (start_set, dest_set): (BTreeSet<_>, BTreeSet<_>) = if contains_plane {
        (connected_set, unconnected_set)
    } else {
        (unconnected_set, connected_set)
    };
    // Java `router.setAirLine(AutorouteAirlineCalculator.calculateAirline(…))`
    // (`:70`) — banked GUI seam, no headless consumer.
    let start_items: Vec<u64> = start_set.iter().map(|id| u64::from(id.0.get())).collect();
    let dest_items: Vec<u64> = dest_set.iter().map(|id| u64::from(id.0.get())).collect();

    // Java `:72-74`: the wall-clock `TimeLimit` carries the ladder
    // value `min(100000 * 2^(pass-1), Integer.MAX_VALUE)`; the
    // deterministic profile spends the SAME value as call ticks
    // (SEAM: units deviation).
    let budget_limit = deterministic_limit_for_pass(ripup_pass_no);
    let budget = Some(if settings.deterministic_budgets {
        RouteBudget::Deterministic {
            limit: budget_limit,
            spent: Cell::new(0),
        }
    } else {
        RouteBudget::Wall(TimeLimit::new(i64::from(
            i32::try_from(budget_limit).unwrap_or(i32::MAX),
        )))
    });

    // Java reads `maxItemIdBeforeRoute` (`:83`) and takes the strict
    // snapshot (`:88-89`) AFTER `initAutoroute`; the port's engine
    // mutably borrows the board for its lifetime, so both reads move
    // ahead of construction. `init_autoroute` allocates registry keys
    // (>= 2^40), never item ids, and touches no routed geometry — the
    // values are identical (documented reorder).
    let max_item_id_before_route = board.max_generated_id();
    let snapshot = if settings.strict_drc {
        Some(board.clone())
    } else {
        None
    };

    let mut engine = init_autoroute(
        manager,
        board,
        route_net_no,
        ctrl.trace_clearance_class_index,
        settings.retain_autoroute_database,
        stoppable_flag.cloned(),
        budget,
    );
    let autoroute_result = engine.autoroute_connection(
        &start_items,
        &dest_items,
        &mut ctrl,
        ripped_item_list,
        ripup_costs,
        &mut NoPullTight,
        &mut SinkBridge(sink),
    );
    // The necked retry hands the SAME budget to its engine (Java
    // passes the same `timeLimit` instance — `:210-216`); the spent
    // ticks carry across both attempts.
    let retry_budget = engine.take_budget();
    drop(engine);

    if autoroute_result.state == AutorouteAttemptState::Routed {
        // Java `:99-125` — the id-watermark rows around the
        // (today no-op) pull-tight consumption.
        let max_item_id_before_opt = board.max_generated_id();
        sink.trace(&format!(
            "compare_trace_opt_changed_area_before net={route_net_no}, maxItemId={max_item_id_before_opt}"
        ));
        opt_changed_area(
            manager,
            board,
            &mut NoPullTight,
            &[],
            &IntOctagon::EMPTY,
            settings.pull_tight_accuracy,
            None,
            0,
        );
        let max_item_id_after_opt = board.max_generated_id();
        sink.trace(&format!(
            "compare_trace_opt_changed_area_after net={route_net_no}, maxItemId={max_item_id_after_opt}, delta={}",
            max_item_id_after_opt.wrapping_sub(max_item_id_before_opt)
        ));
    }

    if (autoroute_result.state == AutorouteAttemptState::Failed
        || autoroute_result.state == AutorouteAttemptState::InsertError)
        && settings.neck_width_um > 0.0
    {
        let necked_result = retry_connection_necked(
            manager,
            board,
            settings,
            route_net_no,
            &ctrl,
            current_via_costs,
            &start_items,
            &dest_items,
            ripped_item_list,
            ripup_costs,
            ripup_pass_no,
            retry_budget,
            stoppable_flag.cloned(),
            sink,
        );
        if let Some(necked_result) = necked_result {
            if let Some(strict_result) = apply_strict_drc_after_route(
                manager,
                board,
                settings,
                route_net_no,
                max_item_id_before_route,
                &snapshot,
                ripup_pass_no,
                sink,
            ) {
                return strict_result;
            }
            return necked_result;
        }
    }

    if autoroute_result.state == AutorouteAttemptState::Routed
        && let Some(strict_result) = apply_strict_drc_after_route(
            manager,
            board,
            settings,
            route_net_no,
            max_item_id_before_route,
            &snapshot,
            ripup_pass_no,
            sink,
        )
    {
        return strict_result;
    }

    autoroute_result
}

/// The Java `:72-74` ladder value in isolation:
/// `(long) min(100000 * 2^(ripupPassNo - 1), Integer.MAX_VALUE)`.
#[must_use]
pub fn deterministic_limit_for_pass(ripup_pass_no: i32) -> u64 {
    RouteBudget::deterministic_for_pass(ripup_pass_no).limit_value()
}

/// Java `retryConnectionNecked` (`:168-247`): retry the connection at
/// the neck width (`settings.neck_width_um`), keeping the SAME budget.
/// Returns `None` where Java returns null (no narrower layer allowed,
/// or the retry did not route).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn retry_connection_necked(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    route_net_no: i32,
    original_ctrl: &AutorouteControl,
    via_costs: i32,
    start_items: &[u64],
    dest_items: &[u64],
    ripped_item_list: &mut BTreeMap<i32, RippedItemSeed>,
    ripup_costs: &mut HashMap<u64, i32>,
    ripup_pass_no: i32,
    budget: Option<RouteBudget>,
    stoppable_flag: Option<Arc<AtomicBool>>,
    sink: &mut dyn DriverSink,
) -> Option<AutorouteAttemptResult> {
    let communication = board.communication();
    let board_resolution = communication.resolution.max(1);
    let neck_width = Unit::scale(
        settings.neck_width_um * f64::from(board_resolution),
        Unit::Um,
        communication.unit,
    )
    .round() as i32;
    let neck_half_width = (neck_width / 2).max(1);
    let mut narrower_somewhere = false;
    for i in 0..original_ctrl.layer_count {
        if original_ctrl.layer_active[i] && original_ctrl.trace_half_width[i] > neck_half_width {
            narrower_somewhere = true;
            break;
        }
    }
    if !narrower_somewhere {
        return None;
    }

    let mut neck_ctrl = AutorouteControl::new_with_costs(
        board,
        route_net_no,
        &settings.router_settings,
        via_costs,
        &settings.trace_costs,
    );
    neck_ctrl.ripup_allowed = true;
    neck_ctrl.ripup_costs = settings.start_ripup_costs.wrapping_mul(ripup_pass_no);
    neck_ctrl.remove_unconnected_vias = settings.remove_unconnected_vias;
    for i in 0..neck_ctrl.layer_count {
        // Java int arithmetic — `wrapping_*` keeps the overflow face.
        let compensation =
            neck_ctrl.compensated_trace_half_width[i].wrapping_sub(neck_ctrl.trace_half_width[i]);
        neck_ctrl.trace_half_width[i] = neck_ctrl.trace_half_width[i].min(neck_half_width);
        neck_ctrl.compensated_trace_half_width[i] =
            neck_ctrl.trace_half_width[i].wrapping_add(compensation);
    }

    let mut neck_engine = init_autoroute(
        manager,
        board,
        route_net_no,
        neck_ctrl.trace_clearance_class_index,
        settings.retain_autoroute_database,
        stoppable_flag,
        budget,
    );
    let neck_result = neck_engine.autoroute_connection(
        start_items,
        dest_items,
        &mut neck_ctrl,
        ripped_item_list,
        ripup_costs,
        &mut NoPullTight,
        &mut SinkBridge(sink),
    );
    drop(neck_engine);

    if neck_result.state != AutorouteAttemptState::Routed {
        return None;
    }

    // Java `:228-238` — the same no-op-today pull-tight consumption
    // as the primary arm (clip sentinel banked).
    opt_changed_area(
        manager,
        board,
        &mut NoPullTight,
        &[],
        &IntOctagon::EMPTY,
        settings.pull_tight_accuracy,
        None,
        0,
    );
    let net_name = board
        .rules()
        .nets
        .get(route_net_no)
        .map_or_else(|| format!("#{route_net_no}"), |net| net.name.clone());
    sink.info(&format!(
        "Necked retry routed net '{net_name}' at {} um trace width.",
        settings.neck_width_um
    ));
    Some(neck_result)
}

/// Java `applyStrictDrcAfterRoute` (`:249-269`): the ENFORCEMENT gate
/// is `isStrictDrc() || ripupPassNo >= 3` (the SNAPSHOT gate — taken
/// in [`route`] — is `isStrictDrc()` only). A rejection is traced,
/// then the board is rolled back to the snapshot when one exists
/// (Java's deserialize; [`restore_from_snapshot`] here). Returns the
/// rejection, `None` = Java null (clean, or the gate closed).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn apply_strict_drc_after_route(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    route_net_no: i32,
    max_item_id_before: u32,
    snapshot: &Option<Board>,
    ripup_pass_no: i32,
    sink: &mut dyn DriverSink,
) -> Option<AutorouteAttemptResult> {
    let is_strict_pass = settings.strict_drc || ripup_pass_no >= 3;
    if !is_strict_pass {
        return None;
    }
    let rejection = enforce_strict_drc(manager, board, route_net_no, max_item_id_before);
    if let Some(rejection) = &rejection {
        // Java `FRLogger.trace(method, operation, message)` — the
        // three fields folded into one row (banked convention).
        sink.trace(&format!(
            "compare_trace_strict_drc_rejection pass={ripup_pass_no}, net={route_net_no}, reason={}",
            rejection.details
        ));
        if let Some(snapshot) = snapshot {
            restore_from_snapshot(manager, board, snapshot);
        }
    }
    rejection
}

/// Java `BatchAutorouter.enforceStrictDrc` (`:305-329`): if any
/// trace/via inserted by the connection that just routed (item id
/// above `max_item_id_before`) carries a clearance violation, rip the
/// whole set of new items and report the connection FAILED, so the
/// pass counts it as not routed and later passes (higher ripup
/// costs) retry it. `None` = clean.
pub fn enforce_strict_drc(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    route_net_no: i32,
    max_item_id_before: u32,
) -> Option<AutorouteAttemptResult> {
    let mut new_items: Vec<ItemId> = Vec::new();
    let mut has_violation = false;
    for id in board.get_connectable_items(route_net_no) {
        let is_new_trace_or_via = id.get() > max_item_id_before
            && board.get(id).is_some_and(|entry| {
                matches!(entry.data, ItemData::Trace { .. } | ItemData::Via { .. })
            });
        if !is_new_trace_or_via {
            continue;
        }
        new_items.push(id);
        // Java collects ALL new items even after the first violation
        // (no short-circuit — the count lands in the details row).
        if !has_violation && !item_clearance_violations(manager, board, id).is_empty() {
            has_violation = true;
        }
    }
    if !has_violation {
        return None;
    }
    for id in &new_items {
        remove_item_through_repository(manager, board, *id);
    }
    Some(AutorouteAttemptResult::with_details(
        AutorouteAttemptState::Failed,
        format!(
            "strict_drc: connection ripped because {} new item(s) included clearance violations",
            new_items.len()
        ),
    ))
}

// ---------------------------------------------------------------------------
// structural tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::pipeline::batch::get_autoroute_items;
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::test_util::parse;
    use epic_board::board::ItemEntry;
    use epic_board::items::{Area, BoardShape, FixedState};
    use epic_board::rules_surf::Nets;
    use epic_board::trace_ops::insert_trace_without_cleaning;
    use epic_dsn::sink::NetIr;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`,
    /// resolution 10).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn settings() -> BatchSettings {
        BatchSettings::new(
            RouterSettingsIr {
                trace_costs: vec![
                    crate::control::ExpansionCostFactor {
                        horizontal: 1.0,
                        vertical: 2.7,
                    },
                    crate::control::ExpansionCostFactor {
                        horizontal: 1.6,
                        vertical: 1.0,
                    },
                ],
                via_costs: 1,
                vias_allowed: true,
                bend_costs: vec![0.0, 0.0],
                layer_active: vec![true, true],
                automatic_neckdown: false,
                start_ripup_costs: 1,
            },
            RouterSettingsScoring::default(),
        )
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    fn insert_trace(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        from: (i32, i32),
        to: (i32, i32),
        net: i32,
    ) -> ItemId {
        insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(&pt(from.0, from.1), &pt(to.0, to.1)),
            0,
            1500,
            &[net],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds")
    }

    /// Java `BatchAutorouter.enforceStrictDrc` (`:305-329`): the id
    /// watermark partitions "new" items; an overlapping foreign-net
    /// pair makes exactly the NEW side violating. With `max_before` at
    /// the FIRST trace, the second is new + violating → ripped, the
    /// FAILED details row carries the count, and the first trace
    /// survives. With `max_before` already past both, nothing is new →
    /// clean. A far-away third trace is new but non-violating → clean.
    #[test]
    fn t12_enforce_strict_drc_watermark_and_rip() {
        let (mut manager, mut board) = parse_fixture();
        let first = insert_trace(
            &mut manager,
            &mut board,
            (600_000, 300_000),
            (620_000, 300_000),
            49,
        );
        // Overlaps `first` exactly → a real clearance violation (the
        // (1,1) matrix cell is 2500 DBU; overlap measures 0 actual).
        let second = insert_trace(
            &mut manager,
            &mut board,
            (610_000, 300_000),
            (630_000, 300_000),
            94,
        );

        // Arm 1: the second trace is "new" and violating → ripped.
        let rejection = enforce_strict_drc(&mut manager, &mut board, 94, first.get());
        let rejection = rejection.expect("the violating new item is rejected");
        assert_eq!(rejection.state, AutorouteAttemptState::Failed);
        assert_eq!(
            rejection.details,
            "strict_drc: connection ripped because 1 new item(s) included clearance violations"
        );
        assert!(
            !board.get(second).is_some_and(|entry| entry.on_the_board),
            "the violating new trace was ripped"
        );
        assert!(
            board.get(first).is_some_and(|entry| entry.on_the_board),
            "the pre-existing trace survives"
        );

        // Arm 2: no NEW trace/via above the watermark → clean (Java
        // returns null without touching anything).
        let max_so_far = board.max_generated_id();
        let rejection = enforce_strict_drc(&mut manager, &mut board, 94, max_so_far);
        assert!(rejection.is_none(), "nothing new → clean");

        // Arm 3: a NEW but non-violating trace → clean, survives.
        let third = insert_trace(
            &mut manager,
            &mut board,
            (640_000, 10_000),
            (645_000, 10_000),
            49,
        );
        let rejection = enforce_strict_drc(&mut manager, &mut board, 49, third.get() - 1);
        assert!(rejection.is_none(), "new + clean → no rejection");
        assert!(
            board.get(third).is_some_and(|entry| entry.on_the_board),
            "the clean new trace survives"
        );
    }

    /// Java `applyStrictDrcAfterRoute` (`:249-269`): the ENFORCEMENT
    /// gate is `isStrictDrc() || ripupPassNo >= 3` (the SNAPSHOT gate
    /// in `route` is strict-only). A violating "just-routed" trace
    /// survives pass 1–2 without strict, is rejected from pass 3 on,
    /// and a SNAPSHOT rolls the board back to the pre-route state.
    #[test]
    fn t12_strict_gate_pass2_vs_pass3_and_snapshot() {
        let (mut manager, mut board) = parse_fixture();
        let pre = insert_trace(
            &mut manager,
            &mut board,
            (600_000, 300_000),
            (620_000, 300_000),
            49,
        );
        let mut settings = settings();
        let max_before = board.max_generated_id();

        // The snapshot is the PRE-ROUTE state (production `route`
        // clones it before the engine runs) — the violating route
        // product below must NOT be part of it.
        let snapshot = Some(board.clone());
        // The violating "route product" (overlaps `pre`).
        let routed = insert_trace(
            &mut manager,
            &mut board,
            (610_000, 300_000),
            (630_000, 300_000),
            94,
        );

        // Pass 2, strict OFF: the gate is closed → None, trace stays.
        let mut sink = CaptureDriverSink::default();
        let outcome = apply_strict_drc_after_route(
            &mut manager,
            &mut board,
            &settings,
            94,
            max_before,
            &snapshot,
            2,
            &mut sink,
        );
        assert!(outcome.is_none(), "pass 2 + non-strict → no enforcement");
        assert!(
            board.get(routed).is_some_and(|entry| entry.on_the_board),
            "the trace survives the closed gate"
        );
        assert!(
            sink.rows.is_empty(),
            "no rejection row either: {:?}",
            sink.rows
        );

        // Pass 3, strict OFF: `ripupPassNo >= 3` opens the gate.
        let mut sink = CaptureDriverSink::default();
        let outcome = apply_strict_drc_after_route(
            &mut manager,
            &mut board,
            &settings,
            94,
            max_before,
            &snapshot,
            3,
            &mut sink,
        );
        let outcome = outcome.expect("pass 3 enforces strict DRC");
        assert_eq!(outcome.state, AutorouteAttemptState::Failed);
        assert!(
            sink.any_contains(
                "compare_trace_strict_drc_rejection pass=3, net=94, \
                 reason=strict_drc: connection ripped because 1 new item(s) included \
                 clearance violations"
            ),
            "the rejection trace row: {:?}",
            sink.rows
        );
        assert!(
            !board.get(routed).is_some_and(|entry| entry.on_the_board),
            "the SNAPSHOT rolled the board back — the trace is gone"
        );
        // The pre-existing trace is part of the snapshot → restored.
        assert!(
            board.get(pre).is_some_and(|entry| entry.on_the_board),
            "the snapshot keeps the pre-route board"
        );

        // Strict ON opens the gate from pass 1.
        let (mut manager, mut board) = parse_fixture();
        let _pre = insert_trace(
            &mut manager,
            &mut board,
            (600_000, 300_000),
            (620_000, 300_000),
            49,
        );
        let max_before = board.max_generated_id();
        let _routed = insert_trace(
            &mut manager,
            &mut board,
            (610_000, 300_000),
            (630_000, 300_000),
            94,
        );
        settings.strict_drc = true;
        let mut sink = CaptureDriverSink::default();
        let outcome = apply_strict_drc_after_route(
            &mut manager,
            &mut board,
            &settings,
            94,
            max_before,
            &None,
            1,
            &mut sink,
        );
        assert!(
            outcome.is_some(),
            "isStrictDrc enforces from pass 1 even without the pass-3 arm"
        );
    }

    /// The plane world (Java `:57-63` + `BatchAutorouter:383-389`):
    /// net 94 carries a pour; a pin touching the pour CA (plus a
    /// dangling far trace keeping the unconnected set non-empty)
    /// answers CONNECTED_TO_PLANE before ANY engine work — zero
    /// insertion rows — and the QUEUE gate skips pour-connected
    /// candidates entirely (the non-plane contrast queues them).
    #[test]
    fn t12_plane_world_connected_to_plane_and_queue_contrast() {
        let (mut manager, mut board) = parse_fixture();

        // Rebuild the net table with net 94 as a plane net (positions
        // ARE the numbering — preserve every net's identity).
        let max_net = board.rules().nets.max_net_number();
        let ir: Vec<NetIr> = (1..=max_net)
            .map(|no| {
                let net = board.rules().nets.get(no).expect("existing net");
                NetIr {
                    name: net.name.clone(),
                    subnet_number: net.subnet_number,
                    contains_plane: no == 94,
                    net_class: net.net_class,
                }
            })
            .collect();
        board.rules_mut().nets = Nets::from_ir(&ir);
        assert!(
            board
                .rules()
                .nets
                .get(94)
                .is_some_and(|net| net.contains_plane),
            "net 94 is now a plane net"
        );

        // The pour CA on the net-94 pin (component-attached so the
        // connected-set walk enters it), plus a dangling far trace so
        // the pin's unconnected set is non-empty.
        let component_id = board
            .iter_ascending()
            .find(|entry| entry.nets.contains(&94) && matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.component_id)
            .expect("net 94 has a pin");
        let _anchor = insert_trace(
            &mut manager,
            &mut board,
            (600_000, 300_000),
            (620_000, 300_000),
            94,
        );
        let ca_id = board.alloc_id();
        board.insert_item(ItemEntry {
            id: ca_id,
            data: ItemData::ConductionArea {
                layer: 0,
                area: Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
                    RegularTileShape::IntBox(IntBox::new(
                        IntPoint::new(660_000, 15_000),
                        IntPoint::new(668_000, 25_000),
                    )),
                ))),
                is_obstacle: false,
                is_filled: true,
            },
            nets: vec![94],
            clearance_class: 1,
            component_id,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(&mut board, ca_id);

        let pin = board
            .iter_ascending()
            .find(|entry| entry.nets.contains(&94) && matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .expect("net 94 pin");

        // route() answers CONNECTED_TO_PLANE with zero engine work.
        let settings = settings();
        let mut ripped = BTreeMap::new();
        let mut costs = HashMap::new();
        let mut sink = CaptureDriverSink::default();
        let result = route(
            &mut manager,
            &mut board,
            &settings,
            pin,
            94,
            &mut ripped,
            &mut costs,
            1,
            None,
            &mut sink,
        );
        assert_eq!(
            result.state,
            AutorouteAttemptState::ConnectedToPlane,
            "pour-connected candidates skip the engine entirely"
        );
        assert!(
            !sink.any_contains("compare_trace_insert_segment"),
            "no insertion rows — the answer fired before any engine work: {:?}",
            sink.rows
        );

        // The queue contrast: the plane world queues NOTHING (both the
        // CA seed and the handled-marked pin are pour-connected); the
        // SAME board with the plane flag OFF queues the CA.
        let mut sink = CaptureDriverSink::default();
        let plane_queue = get_autoroute_items(&mut manager, &mut board, &mut sink);
        // The fixture's own NET_33/NET_98 pin pairs queue regardless of
        // net 94's plane flag (4 items); the net-94 candidates do NOT:
        // the CA and the pin are pour-connected (the CA is in both
        // connected sets), and the anchor is UNFIXED → routable →
        // never a seed at all.
        assert_eq!(
            plane_queue.len(),
            4,
            "only the fixture's own pin pairs queue: {plane_queue:?}"
        );
        let debug = sink.joined("debug");
        assert!(
            !debug.contains("net 'N093'"),
            "no net-94 item queued in the plane world: {debug}"
        );
        assert!(
            debug.contains("Queuing item for routing: Pin on net 'NET_33'")
                && debug.contains("Queuing item for routing: Pin on net 'NET_98'"),
            "the fixture pairs queue: {debug}"
        );

        board.rules_mut().nets = Nets::from_ir(
            &(1..=max_net)
                .map(|no| {
                    let net = board.rules().nets.get(no).expect("existing net");
                    NetIr {
                        name: net.name.clone(),
                        subnet_number: net.subnet_number,
                        contains_plane: false,
                        net_class: net.net_class,
                    }
                })
                .collect::<Vec<NetIr>>(),
        );
        let mut sink = CaptureDriverSink::default();
        let plain_queue = get_autoroute_items(&mut manager, &mut board, &mut sink);
        // CA + the fixture's 4 pin pairs. The net-94 PIN never seeds:
        // the CA's connected-set walk marked it handled, and the
        // anchor is still routable.
        assert_eq!(
            plain_queue.len(),
            5,
            "without the plane flag the CA qualifies: {plain_queue:?}"
        );
        let debug = sink.joined("debug");
        assert!(
            debug.contains(
                "Queuing item for routing: ConductionArea on net 'N093' (connected: 2/3)"
            ),
            "the CA queue row (CA + pin of 3 connectables): {debug}"
        );
        assert_eq!(
            debug
                .lines()
                .filter(|row| row.contains("net 'N093'"))
                .count(),
            1,
            "the handled-marked net-94 pin never seeds: {debug}"
        );
    }
}
