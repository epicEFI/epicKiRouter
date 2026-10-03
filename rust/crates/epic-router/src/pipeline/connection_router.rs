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
//! * **`opt_changed_area` clip.** Java passes `clipShape = null` and
//!   the operations gate (`clipShape != IntOctagon.EMPTY`) therefore
//!   RUNS the tightener unclipped; the port passes `None` (unbounded)
//!   into the live [`epic_board::trace_tightener::TraceTightenerSeam`]
//!   (M4-T3) — same gate semantics, the tightener now actually runs
//!   on 90° boards.
//! * **Multi-arg trace rows are folded** into
//!   `sink.trace("<operation> <message>")` — the Java
//!   `FRLogger.trace(method, operation, message, impactedItems,
//!   impactedPoints)` method tag and impact payloads are log-only
//!   (bug-118 no-digest convention).
//! * **The `TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP` guard** (Java `:22`,
//!   1000 ms into `optChangedArea`) feeds the tightener through the
//!   wall→tick budget face since M4-T3: behind
//!   `deterministic_budgets` the same limit is spent as consultation
//!   ticks (the `RouteBudget` pattern; see the
//!   [`epic_board::trace_tightener`] module docs).

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use epic_board::board::Board;
use epic_board::contacts::{item_connected_set, item_unconnected_set};
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::routing_board_insert::opt_changed_area;
use epic_board::time_limit::TimeLimit;
use epic_board::trace_ops::remove_item_through_repository;
use epic_board::trace_tightener::{TraceCostFactor, TraceTightenerSeam};
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::item_clearance_violations;
use epic_dsn::state::Unit;

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
pub(crate) struct SinkBridge<'a>(pub(crate) &'a mut dyn DriverSink);

/// Java `AutorouteConnectionRouter.TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP`
/// (`:22`): the pull-tight time limit handed to `optChangedArea`.
/// Wall clock in Java; behind `deterministic_budgets` the port spends
/// the same value as `is_stop_requested` consultation ticks (the
/// `RouteBudget` pattern). `BatchAutorouter` declares its own twin
/// (`:43`, same value) — `batch.rs` carries that duplicate.
pub(crate) const TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP: i32 = 1000;

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

/// Java `:57-63` — the CONNECTED_TO_PLANE gate: any ConductionArea in
/// the connected set answers CONNECTED_TO_PLANE before any engine
/// work. The M6-T6 clamp (`router.plane_island_clamp`, default OFF —
/// dead code at defaults), REGION-LEVEL as of M6-T8: a pour answers the
/// gate only through a REGION carrying the net's seed copper — the
/// connected set's own pins/vias/traces overlapping uncarved pour metal
/// (`epic_board::islands::pour_region_seeded_by`). With the flag OFF
/// this is the Java face verbatim (the detector never runs).
fn plane_connected_gate(
    board: &Board,
    connected_set: &BTreeSet<Reverse<ItemId>>,
    island_clamp: bool,
) -> bool {
    let mut plane_in_connected = connected_set.iter().any(|id| {
        board
            .get(id.0)
            .is_some_and(|entry| matches!(entry.data, ItemData::ConductionArea { .. }))
    });
    if plane_in_connected && island_clamp {
        // The connected set's own seed copper (every non-pour member:
        // pins/vias/traces of the net).
        let seed_items: Vec<ItemId> = connected_set
            .iter()
            .map(|id| id.0)
            .filter(|id| {
                board
                    .get(*id)
                    .is_some_and(|entry| !matches!(entry.data, ItemData::ConductionArea { .. }))
            })
            .collect();
        plane_in_connected = connected_set.iter().any(|id| {
            board.get(id.0).is_some_and(|entry| {
                matches!(entry.data, ItemData::ConductionArea { .. })
                    && epic_board::islands::pour_region_seeded_by(board, id.0, &seed_items)
            })
        });
    }
    plane_in_connected
}

/// The negotiated-or-linear rip-up base (the M6-T8 scheduler seam; the
/// M6-T9 Q5 dedup — this expression was previously duplicated verbatim
/// at the primary [`route`] seam and the necked-retry seam): the
/// pass's per-net negotiated base when the PathFinder stage is on and
/// the net carries a guide entry, else Java's linear
/// `start_ripup_costs * pass` ladder.
///
/// M11-T4 fix round (2026-10-03): the negotiated base now FLOORS the
/// linear ladder (`max(negotiated, start * pass)`) instead of
/// replacing it. The replace composition (M6-T8) capped every hot
/// net at `2 * start` for the whole run, which removed Java's ladder
/// as a de-facto oscillation dampener — under #931 cluster F's
/// near-tie corner-door routes the maze kept pricing contested ripups
/// at `2 * start` forever and two fighting nets never resolved (the
/// gv-iu limit cycle, buglog 256: scores cycling 927-967 with 5-11
/// unrouted, best 966.83/5 vs the golden's 980.10/3, while the SAME
/// detail router at default settings — Java's ladder intact — routes
/// the board 1000/0 in 18 passes). With the floor, negotiation can
/// only ESCALATE beyond Java's own per-pass pricing, never undercut
/// it; the negotiated component keeps its relative `2 * start` cap
/// and the ladder keeps its Java semantics (uncapped below the
/// absolute `RIPUP_CAP`, and disarming the maze's
/// `start * 2` fanout-protection threshold from pass 3 on — exactly
/// Java's late-pass behavior). Measured on gv-iu: the floor world
/// converges (the wander is gone); see the M11-T4 fix-round report.
fn negotiated_or_linear_ripup_costs(
    pathfinder: Option<&crate::global::history::PathfinderPass>,
    route_net_no: i32,
    settings: &BatchSettings,
    ripup_pass_no: i32,
) -> i32 {
    let linear = settings
        .start_ripup_costs
        .saturating_mul(ripup_pass_no.max(1))
        .min(
            crate::global::history::RIPUP_CAP
                .try_into()
                .unwrap_or(i32::MAX),
        );
    match pathfinder.and_then(|pass| pass.bases.get(&route_net_no).copied()) {
        Some(negotiated) => negotiated.max(linear),
        None => linear,
    }
}

#[allow(clippy::too_many_arguments)] // the Java seam's face
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
    pathfinder: Option<&crate::global::history::PathfinderPass>,
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
    // The scheduler seam (design :70): Java's linear ladder
    // `start * pass` is replaced WHEN the negotiated stage is on — the
    // pass's per-net negotiated base (present + history mix over the
    // net's guide region, `global/history.rs` docs). Absent nets (no
    // guide) fall back to the linear face.
    ctrl.ripup_costs =
        negotiated_or_linear_ripup_costs(pathfinder, route_net_no, settings, ripup_pass_no);
    ctrl.remove_unconnected_vias = settings.remove_unconnected_vias;
    // M6-T9: the push-and-shove insertion flag (default OFF) — the
    // maze's shove-before-rip waiver (`maze/ripup.rs`).
    ctrl.push_shove = settings.push_shove;
    // The guides consumption (M6-T8, the T7 preferred-layers
    // residual): the net's guide preference order biases the maze's
    // per-layer trace costs (a cost bias, not a geometric clamp — the
    // containment relaxation documented in the module docs and the
    // task report). `None` pathfinder = the Java cost face verbatim.
    if let Some(layers) = pathfinder.and_then(|pass| pass.preferred_layers.get(&route_net_no)) {
        crate::global::history::apply_preferred_layer_bias(&mut ctrl.trace_costs, layers);
    }
    // M7-T6: the pair COUPLING preference (default inert — an empty
    // declaration list, or this net is no declared pair's follower,
    // or the leader has no routed copper yet). The follower's maze
    // steps toward the leader's corridor then cost half (`pairs.rs`
    // constants); every other net's cost face is bit-identical.
    ctrl.coupling = crate::pipeline::pairs::coupling_preference(board, settings, route_net_no);

    let unconnected_set = item_unconnected_set(manager, board, item_id, route_net_no);
    if unconnected_set.is_empty() {
        return AutorouteAttemptResult::new(AutorouteAttemptState::NoUnconnectedNets);
    }

    let connected_set = item_connected_set(manager, board, item_id, route_net_no);
    if contains_plane && plane_connected_gate(board, &connected_set, settings.plane_island_clamp) {
        return AutorouteAttemptResult::new(AutorouteAttemptState::ConnectedToPlane);
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

    // M6-T7 (default OFF): the pattern-routing fast path — easy nets
    // (two single-layer pins on one active signal layer) attempt an
    // L/Z candidate BEFORE the maze search; success returns through
    // the SAME Routed postlude (the pull-tight + strict-DRC faces).
    if let Some(pattern_result) = crate::global::pattern::try_pattern_route(
        manager,
        board,
        settings,
        &ctrl,
        &start_items,
        &dest_items,
        sink,
    ) {
        routed_postlude(
            manager,
            board,
            settings,
            route_net_no,
            &ctrl,
            stoppable_flag,
            sink,
        );
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
        return pattern_result;
    }

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
        &mut TraceTightenerSeam,
        &mut SinkBridge(sink),
    );
    // The necked retry hands the SAME budget to its engine (Java
    // passes the same `timeLimit` instance — `:210-216`); the spent
    // ticks carry across both attempts.
    let retry_budget = engine.take_budget();
    drop(engine);

    if autoroute_result.state == AutorouteAttemptState::Routed {
        // Java `:99-125` — the id-watermark rows + the pull-tight
        // consumption (the shared postlude; the pattern arm returns
        // through the same helper — C1-1).
        routed_postlude(
            manager,
            board,
            settings,
            route_net_no,
            &ctrl,
            stoppable_flag,
            sink,
        );
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
            pathfinder,
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

/// The Routed postlude (C1-1): the id-watermark rows + the pull-tight
/// consumption the PATTERN arm and the MAZE arm both run after a
/// successful route. Extracted verbatim from the maze arm's inline
/// block (Java `:99-125` — the cite the maze arm carried): the tightener
/// may move corners, split traces and relocate vias, so the id delta is
/// no longer structurally 0. The pattern arm previously carried a
/// verbatim hand-copy of this block minus the provenance (the C1-1
/// bank) — one helper now serves both arms; the default-settings gates
/// are the byte-identity proof.
#[allow(clippy::too_many_arguments)] // the extracted block's face
fn routed_postlude(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    route_net_no: i32,
    ctrl: &AutorouteControl,
    stoppable_flag: Option<&Arc<AtomicBool>>,
    sink: &mut dyn DriverSink,
) {
    let max_item_id_before_opt = board.max_generated_id();
    sink.trace(&format!(
        "compare_trace_opt_changed_area_before net={route_net_no}, maxItemId={max_item_id_before_opt}"
    ));
    // Java `:107-113`: `optChangedArea(new int[0], null,
    // tracePullTightAccuracy, autorouteControl.traceCosts,
    // router.thread, TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP)` — the
    // clip is null (unbounded), the nets empty (all).
    let tightener_trace_costs: Vec<TraceCostFactor> =
        ctrl.trace_costs.iter().map(Into::into).collect();
    opt_changed_area(
        manager,
        board,
        &mut TraceTightenerSeam,
        &[],
        None,
        settings.pull_tight_accuracy,
        None,
        0,
        Some(&tightener_trace_costs),
        stoppable_flag,
        TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
        settings.deterministic_budgets,
    );
    let max_item_id_after_opt = board.max_generated_id();
    sink.trace(&format!(
        "compare_trace_opt_changed_area_after net={route_net_no}, maxItemId={max_item_id_after_opt}, delta={}",
        max_item_id_after_opt.wrapping_sub(max_item_id_before_opt)
    ));
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
    pathfinder: Option<&crate::global::history::PathfinderPass>,
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
    // The scheduler seam (the primary arm's face, mirrored through the
    // shared helper — the M6-T9 Q5 dedup).
    neck_ctrl.ripup_costs =
        negotiated_or_linear_ripup_costs(pathfinder, route_net_no, settings, ripup_pass_no);
    neck_ctrl.remove_unconnected_vias = settings.remove_unconnected_vias;
    // M6-T9: the primary arm's face, mirrored.
    neck_ctrl.push_shove = settings.push_shove;
    // M7-T6: the pair coupling preference, mirrored (the necked retry
    // routes the same follower against the same leader corridors).
    neck_ctrl.coupling = crate::pipeline::pairs::coupling_preference(board, settings, route_net_no);
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
        // Java hands the SAME `router.thread` reference to the engine
        // and the pull-tight call; the Arc clone is that shared
        // reference.
        stoppable_flag.clone(),
        budget,
    );
    let neck_result = neck_engine.autoroute_connection(
        start_items,
        dest_items,
        &mut neck_ctrl,
        ripped_item_list,
        ripup_costs,
        &mut TraceTightenerSeam,
        &mut SinkBridge(sink),
    );
    drop(neck_engine);

    if neck_result.state != AutorouteAttemptState::Routed {
        return None;
    }

    // Java `:229-235` — the same pull-tight consumption as the
    // primary arm (`neckControl.traceCosts`, clip null, the same
    // 1000 ms limit).
    let tightener_trace_costs: Vec<TraceCostFactor> =
        neck_ctrl.trace_costs.iter().map(Into::into).collect();
    opt_changed_area(
        manager,
        board,
        &mut TraceTightenerSeam,
        &[],
        None,
        settings.pull_tight_accuracy,
        None,
        0,
        Some(&tightener_trace_costs),
        stoppable_flag.as_ref(),
        TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
        settings.deterministic_budgets,
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
                fanout: Default::default(),
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

    /// M11-T4 fix round (2026-10-03): the negotiated base FLOORS
    /// Java's linear ladder at the scheduler seam —
    /// `max(negotiated, start * pass)`. Four arms, each killing its
    /// own mutant: (1) a base ABOVE the ladder (early pass, high
    /// pressure) is consumed as-is — the max must not collapse to the
    /// linear face; (2) a base BELOW the ladder (the capped hot net at
    /// a late pass — the OLD replace composition that sustained the
    /// #931 cluster-F ripup limit cycle on gv-iu, buglog 256) lets the
    /// LADDER win; (3) a net with no guide entry falls to the linear
    /// face; (4) no scheduler at all = the linear face alone.
    #[test]
    fn negotiated_base_floors_the_linear_ladder() {
        let settings = settings(); // start_ripup_costs = 1
        let mut pass = crate::global::history::PathfinderPass::default();
        pass.bases.insert(7, 30); // above the pass-1 ladder (1)
        pass.bases.insert(9, 2); // the relative cap, below the pass-12 ladder (12)
        assert_eq!(
            negotiated_or_linear_ripup_costs(Some(&pass), 7, &settings, 1),
            30,
            "negotiated above the ladder wins"
        );
        assert_eq!(
            negotiated_or_linear_ripup_costs(Some(&pass), 9, &settings, 12),
            12,
            "the ladder floors the capped base (replace would answer 2)"
        );
        assert_eq!(
            negotiated_or_linear_ripup_costs(Some(&pass), 11, &settings, 12),
            12,
            "no guide entry: the linear face"
        );
        assert_eq!(
            negotiated_or_linear_ripup_costs(None, 7, &settings, 12),
            12,
            "no scheduler: the linear face"
        );
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
            None,
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
    /// M6-T6 settings-ON smoke (interpretation 2): the clamp gate
    /// helper, both arms. The `t11_island_floating` crafted world's
    /// pour is fully floating (its only pin sits OUTSIDE the pour, so
    /// no region carries a same-net seed). Clamp OFF = the Java face
    /// verbatim (any pour in the set answers); clamp ON = the gate
    /// stays shut. The connected set is constructed directly — the
    /// helper is a pure function of (board, set, flag); the T8/T9
    /// wiring replaces the coarse pour-level verdict with region-level
    /// target filtering.
    #[test]
    fn t11_island_clamp_gate_smoke_off_and_on() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/island-spike/t11_island_floating.dsn");
        let text = std::fs::read_to_string(&path).expect("floating world present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        assert!(matches!(
            epic_dsn::reader::read_board(text.as_bytes(), &mut ses),
            epic_dsn::reader::DsnReadResult::Success { .. }
        ));
        let board = epic_board::board::Board::from_ses_board(&ses);
        let (pour_id, pin_id) =
            board
                .iter_ascending()
                .fold((None, None), |acc, entry| match (&entry.data, acc) {
                    (ItemData::ConductionArea { .. }, (_, pin)) => (Some(entry.id), pin),
                    (ItemData::Pin { .. }, (pour, _)) => (pour, Some(entry.id)),
                    _ => acc,
                });
        let pour_id = pour_id.expect("floating world has a pour");
        let pin_id = pin_id.expect("floating world has a pin");
        assert!(
            epic_board::islands::pour_fully_floating(&board, pour_id),
            "rig precondition: the floating world's pour has zero seeded regions"
        );
        let mut connected = BTreeSet::new();
        connected.insert(Reverse(pour_id));
        connected.insert(Reverse(pin_id));

        // Clamp OFF: the Java face — the pour answers the gate.
        assert!(
            plane_connected_gate(&board, &connected, false),
            "clamp OFF: any pour answers CONNECTED_TO_PLANE (Java verbatim)"
        );
        // Clamp ON: the pour is fully floating -> the gate stays shut.
        assert!(
            !plane_connected_gate(&board, &connected, true),
            "clamp ON: a fully-floating pour cannot answer the gate"
        );
    }
}
