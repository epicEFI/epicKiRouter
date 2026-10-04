//! Java `autoroute/pipeline/AutoroutePassRunner.java` — the
//! single-threaded pass walk (`runSingleThread`, `:150-335`): the item
//! queue walk with the per-(item, net) route attempts, the
//! `compare_trace_*` rows, the failure log, and the per-pass tail
//! removal. The multi-thread variant (`runMultiThread`) — board
//! deep-copies, the seeded shuffle, and the cross-thread
//! `BoardHistory` reduction — is the parallelism milestone's seam.
//!
//! ## Banks (SEAM carries the dossier)
//!
//! * **`catch (Exception) → no Rust catch`** (`:330-334`): a panic
//!   propagates (the batch-loop IAE→`Result` face, applied down the
//!   stack).
//! * **The benchmark profile + `PerformanceProfiler` faces** are
//!   profile-only instrumentation (`isBenchmarkProfileEnabled()` gates
//!   every one of them); not ported. `recordPass`'s wall-clock pass
//!   duration and the `currentRipupCost = startRipupCosts * passNo`
//!   render belong to that family.
//! * **The GUI progress event is a sink call.** Java fires
//!   `fireBoardUpdatedEvent(progressStatistics, counters, board)` —
//!   the `progressStatistics` snapshot and the
//!   `shouldFireBoardUpdate()` throttle are event-bus faces; the port
//!   fires the TYPED [`RouterCounters`] through
//!   [`DriverSink::board_updated`](crate::pipeline::event_sink::DriverSink::board_updated)
//!   at the pre-pass fire (`:195`) and the post-pass fire (`:320`) —
//!   the sink impls render (the capture sink via [`render_counters`],
//!   the null sink not at all). The per-item `updateProgress` keeps
//!   the LIVE interval counter (`progressItemsSinceStatistics`,
//!   pinnable) and drops only the snapshot + throttle around it.
//! * **`router.airLine = null`** — the GUI airline seam, no-op here.
//! * **The failure log's `firstNet`** is read at record creation on
//!   the port (`epic_board::failure_log`); Java reads
//!   `currentItem`'s nets internally the same way.

use std::collections::{BTreeMap, HashMap};

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::trace_ops::StopConnectionOption;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::incompletes::{NetIncompleteRow, all_incompletes};
use epic_geometry::point::Point;

use crate::engine::{AutorouteAttemptResult, AutorouteAttemptState, RippedItemSeed};
use crate::pipeline::batch::{
    BatchSettings, PROGRESS_STATISTICS_ITEM_INTERVAL, StopFace, TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
    calculate_incomplete_count, get_autoroute_items, remove_tails,
};
use crate::pipeline::connection_router::route as route_connection;
use crate::pipeline::event_sink::DriverSink;

// ---------------------------------------------------------------------------
// the counters (Java core.RouterCounters)
// ---------------------------------------------------------------------------

/// Java `core.RouterCounters` — the per-pass counters the progress
/// event carries. They flow TYPED through
/// [`DriverSink::board_updated`](crate::pipeline::event_sink::DriverSink::board_updated);
/// [`render_counters`] is the canonical row text the capture sink
/// emits (the GUI consumer is the host's business).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouterCounters {
    /// Java `phase`.
    pub phase: String,
    /// Java `passCount`.
    pub pass_count: i32,
    /// Java `queuedToBeRoutedCount`.
    pub queued_to_be_routed_count: i32,
    /// Java `skippedCount`.
    pub skipped_count: i32,
    /// Java `rippedCount`.
    pub ripped_count: i32,
    /// Java `failedToBeRoutedCount`.
    pub failed_to_be_routed_count: i32,
    /// Java `routedCount`.
    pub routed_count: i32,
    /// Java `incompleteCount`.
    pub incomplete_count: i32,
    /// Java `fanoutExtraViasCount` — the M4 fanout stage's face;
    /// stays 0 until that stage lands.
    pub fanout_extra_vias_count: i32,
}

/// The canonical rendering of the counters row (Java ships the OBJECT
/// through the event; the row is the port-defined observability face).
/// Consumed by the capture sink's `board_updated` impl — the
/// production null sink renders nothing (logging off).
pub(crate) fn render_counters(counters: &RouterCounters) -> String {
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
        counters.fanout_extra_vias_count,
    )
}

// ---------------------------------------------------------------------------
// row helpers
// ---------------------------------------------------------------------------

/// Java `IntPoint.toString()` — `"(" + x + "," + y + ")"`, no space.
/// The `Rational` arm renders the float approximation in the same
/// shape (Java's `RationalPoint.toString` is a numeric form too;
/// rational corners never occur on parsed-board items reaching these
/// rows).
fn point_to_string(point: &Point) -> String {
    match point {
        Point::Int(p) => format!("({},{})", p.x, p.y),
        Point::Rational(_) => {
            let f = point.to_float();
            format!("({},{})", f.x, f.y)
        }
    }
}

/// Java `net != null ? net.name : "net#" + netNo`.
fn net_name_or(board: &Board, net_no: i32) -> String {
    board
        .rules()
        .nets
        .get(net_no)
        .map(|net| net.name.clone())
        .unwrap_or_else(|| format!("net#{net_no}"))
}

/// Java `logIncompleteDetails` (`:337-358`): the pass header row and
/// the per-net breakdown. `rows` arrives net-ascending from
/// [`all_incompletes`] (Java loops `1..=maxNetNumber`).
fn log_incomplete_details(
    board: &Board,
    sink: &mut dyn DriverSink,
    pass_no: i32,
    items_to_go_count: i32,
    incomplete_count: i32,
    rows: &[NetIncompleteRow],
) {
    if incomplete_count <= 0 {
        return;
    }
    sink.debug(&format!(
        "Pass #{pass_no}: {incomplete_count} incompletes across {items_to_go_count} \
         items to route"
    ));
    for row in rows {
        if row.incomplete_count > 0 {
            let name = net_name_or(board, row.net_no);
            sink.debug(&format!(
                "  Net '{name}' has {} incomplete(s)",
                row.incomplete_count
            ));
        }
    }
}

/// Java `logRippedItems` (`:360-394`): one row per harvested seed.
/// Java iterates the `TreeSet<Item>` in DESCENDING id order; the map
/// is ascending-keyed, so the iteration is reversed. The `ripped_*`
/// faces come from the seed (Java reads them off the live object after
/// removal — the seed is the port's snapshot of exactly those faces,
/// captured pre-removal; see [`RippedItemSeed`]).
fn log_ripped_items(
    sink: &mut dyn DriverSink,
    source_item_id: i32,
    source_net: i32,
    ripped_item_list: &BTreeMap<i32, RippedItemSeed>,
    ripped_item_costs: &HashMap<u64, i32>,
) {
    for (item_id, seed) in ripped_item_list.iter().rev() {
        let ripped_nets = seed
            .nets
            .iter()
            .map(|net| net.to_string())
            .collect::<Vec<_>>()
            .join("|");
        // Java `rippedItemCosts.getOrDefault(rippedItem, -1)`.
        let ripup_cost = ripped_item_costs.get(&seed.key).copied().unwrap_or(-1);
        sink.trace(&format!(
            "compare_trace_ripped_item source_item={source_item_id}, source_net={source_net}, \
             ripped_id={item_id}, ripped_type={}, ripped_net_count={}, \
             ripped_nets={ripped_nets}, ripupCost={ripup_cost}",
            seed.simple_name,
            seed.nets.len()
        ));
    }
}

/// Java `logTraceRouteComparison` (`:396-441`): the fresh per-item DRC
/// walk (`innerDrc`) plus the net-population and id-watermark reads.
/// Trace-gated by the caller (Java `FRLogger.isTraceEnabled()`).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn log_trace_route_comparison(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    sink: &mut dyn DriverSink,
    simple_name: &str,
    net_no: i32,
    result: &AutorouteAttemptResult,
    ripped_count: usize,
    net_items_before: usize,
) {
    // Java `:400-405`: `innerDrc.getIncompleteCount()` (the SUM over
    // nets) + `getIncompleteCount(net)` (this net's own count).
    let (_max_connections, rows) = all_incompletes(manager, board);
    let temp_incomp: usize = rows.iter().map(|row| row.incomplete_count).sum();
    let temp_net_incomp = rows
        .iter()
        .find(|row| row.net_no == net_no)
        .map_or(0, |row| row.incomplete_count);
    let net_items_after = board.get_connectable_items(net_no).len();
    let max_item_id = board.max_generated_id();
    sink.trace(&format!(
        "compare_trace_route_item Routing {simple_name} -> result={}, details={}, \
         incompletes={temp_incomp}, netIncomplete={temp_net_incomp}, ripped={ripped_count}, \
         netItems={net_items_before}->{net_items_after}, maxItemId={max_item_id}",
        result.state.as_str(),
        result.details
    ));
}

/// Java `logNet94Items` (`:444-487`): the net-94 debug dump. The pin
/// faces read through the component/package chain (`Pin.name()` —
/// Java's missing-name arms render as the literal `"null"` in the
/// string concat; mirrored, though unreachable on a live board).
fn log_net94_items(board: &mut Board, sink: &mut dyn DriverSink) {
    sink.trace("compare_trace_dump_net_items Dump net 94 items");
    for id in board.get_connectable_items(94) {
        let Some(entry) = board.get(id) else {
            continue;
        };
        match &entry.data {
            ItemData::Trace { layer, lines, .. } => {
                let first = lines
                    .first_corner()
                    .map(|p| point_to_string(&p))
                    .unwrap_or_else(|| "null".to_string());
                let last = lines
                    .last_corner()
                    .map(|p| point_to_string(&p))
                    .unwrap_or_else(|| "null".to_string());
                sink.trace(&format!(
                    "compare_trace_dump_net_item Trace layer={layer} corners={first} to {last}"
                ));
            }
            ItemData::Via { center, .. } => {
                let point = Point::Int(*center);
                sink.trace(&format!(
                    "compare_trace_dump_net_item Via center={}",
                    point_to_string(&point)
                ));
            }
            ItemData::Pin { pin_index, .. } => {
                let center = board
                    .pin_center(id)
                    .map(|p| point_to_string(&p))
                    .unwrap_or_else(|| "null".to_string());
                let component = u32::try_from(entry.component_id)
                    .ok()
                    .and_then(|cid| board.components().get(cid));
                let name = component
                    .and_then(|component| board.library().package(component.package_no()))
                    .and_then(|package| package.get_pin(*pin_index))
                    .map(|pin| pin.name.clone())
                    .unwrap_or_else(|| "null".to_string());
                let comp_name = component
                    .map(|component| component.name.clone())
                    .unwrap_or_else(|| "null".to_string());
                sink.trace(&format!(
                    "compare_trace_dump_net_item Pin center={center} name={name} comp={comp_name}"
                ));
            }
            _ => {
                let simple = entry.data.java_simple_name();
                sink.trace(&format!("compare_trace_dump_net_item Item {simple}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// the pass walk (Java runSingleThread, :150-335) + the T7 partitioned
// executor
// ---------------------------------------------------------------------------

/// One (item, net) work unit of the golden walk — the atom both
/// drivers execute. `nets`/`simple_name` are the item's faces read
/// from the live board at walk time (the sequential read semantics,
/// [`crate::pipeline::pass_runner`] doc).
struct PassUnit {
    item_id: ItemId,
    net: i32,
    /// The item's first net — the only face `run_attempt` reads from
    /// the item's net list (the failure log's `first_net`); the full
    /// `Vec<i32>` clone the first draft carried was dead weight on the
    /// golden path (quality round N6).
    first_net: i32,
    simple_name: &'static str,
    /// The walk's `items_to_go_count` at this unit (pre-decrement) —
    /// the failure row's "items remaining" face.
    items_remaining: i32,
}

/// The unit's `first_net` face: the item's first net, or the inert
/// `-1` sentinel when the item carries no nets. One named helper for
/// BOTH construction sites (the serial walk and the partitioned
/// walk — the M6-T1 N3 dedup of the duplicated `unwrap_or(-1)`
/// fallbacks); behavior byte-identical.
fn unit_first_net(nets: &[i32]) -> i32 {
    nets.first().copied().unwrap_or(-1)
}

/// The counter deltas one attempt contributes (its sink rows are
/// emitted inside [`run_attempt`]).
struct AttemptTally {
    routed: bool,
    skipped: bool,
    ripped_count: usize,
}

/// The per-attempt body, extracted verbatim from the original
/// `run_single_thread` loop (M5-T7 pure motion): the sequential driver
/// and the partitioned executor share THIS body, so the golden bytes
/// cannot drift between `--threads 1` and `--threads N`.
#[allow(clippy::too_many_arguments)] // the extracted loop body's face
fn run_attempt(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    sink: &mut dyn DriverSink,
    stop_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pass_no: i32,
    unit: &PassUnit,
    pathfinder: Option<&crate::global::history::PathfinderPass>,
) -> AttemptTally {
    #[cfg(test)]
    if FORCE_ATTEMPT_PANIC.load(std::sync::atomic::Ordering::SeqCst) && pass_no == 777 {
        panic!("t7-f1 forced attempt panic on net {}", unit.net);
    }
    board.start_marking_changed_area();

    // Fresh per-attempt accumulators (Java `:224-225`).
    let mut ripped_item_list: BTreeMap<i32, RippedItemSeed> = BTreeMap::new();
    let mut ripped_item_costs: HashMap<u64, i32> = HashMap::new();
    let net_items_before = board.get_connectable_items(unit.net).len();

    let result = route_connection(
        manager,
        board,
        settings,
        unit.item_id,
        unit.net,
        &mut ripped_item_list,
        &mut ripped_item_costs,
        pass_no,
        stop_flag,
        sink,
        pathfinder,
    );

    log_ripped_items(
        sink,
        i32::try_from(unit.item_id.get()).unwrap_or(i32::MAX),
        unit.net,
        &ripped_item_list,
        &ripped_item_costs,
    );
    if sink.is_trace_enabled() {
        log_trace_route_comparison(
            manager,
            board,
            sink,
            unit.simple_name,
            unit.net,
            &result,
            ripped_item_list.len(),
            net_items_before,
        );
    }
    if unit.net == 94 {
        log_net94_items(board, sink);
    }

    let mut tally = AttemptTally {
        routed: false,
        skipped: false,
        ripped_count: ripped_item_list.len(),
    };
    match result.state {
        AutorouteAttemptState::Routed => {
            tally.routed = true;
        }
        AutorouteAttemptState::AlreadyConnected
        | AutorouteAttemptState::NoUnconnectedNets
        | AutorouteAttemptState::ConnectedToPlane => {
            tally.skipped = true;
        }
        _ => {
            // Java records the failure against the ITEM with the
            // item's first net read at creation.
            let first_net = unit.first_net;
            board.failure_log.record_failure(
                u64::from(unit.item_id.get()),
                first_net,
                i64::from(pass_no),
                result.state.as_str(),
                Some(&result.details),
            );
            sink.debug(&format!("Autorouter {}", result.details));
            let failure_count = board
                .failure_log
                .failure_count(u64::from(unit.item_id.get()));
            if unit.items_remaining <= 5 || failure_count >= 3 {
                let name = net_name_or(board, unit.net);
                sink.debug(&format!(
                    "Pass #{pass_no}: Failed to route {} on net '{name}' \
                     ({} items remaining, {failure_count} failures). State: {}",
                    unit.simple_name,
                    unit.items_remaining,
                    result.state.as_str()
                ));
            }
        }
    }
    tally
}

/// T7-F1 pin hook (test-only): when set, `run_attempt` panics on
/// entry FOR PASS NUMBER 777 ONLY (the gate keeps concurrent tests in
/// the same binary from ever seeing the hook — no other test uses a
/// pass outside the production ladder 1..=20) — the forced-panic face
/// the partitioned fail-fast pin drives.
#[cfg(test)]
static FORCE_ATTEMPT_PANIC: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The maxItems face (Java `AutoroutePassRunner.java:211-218`),
/// extracted verbatim: returns false when the walk must stop (the info
/// row, the FULL stop raise and the CAUSE mark are exactly the
/// original inline block).
fn max_items_gate(
    settings: &BatchSettings,
    total_items_routed: i32,
    stop: &mut StopFace,
    sink: &mut dyn DriverSink,
) -> bool {
    if let Some(max_items) = settings.max_items
        && max_items > 0
        && total_items_routed >= max_items
    {
        sink.info(&format!(
            "Max items limit reached ({max_items}). Stopping auto-router."
        ));
        // Java `AutoroutePassRunner.java:211-218`: the maxItems
        // face raises the FULL stop (`router.thread.requestStop()`),
        // not the auto-router-only face — the optimizer-stage
        // gate (`RoutingPipeline.java:122`) must see it.
        stop.request_full();
        // Mark the CAUSE (quality MINOR-3): the driver
        // attributes `StopReason::MaxItemsReached` only off
        // this mark, so an external shared-flag stop during a
        // max-items-bounded run is never misclassified.
        stop.mark_max_items_faced();
        return false;
    }
    true
}

/// The pass prelude shared by both drivers (the queue walk + the
/// opening counters event). Empty queue → the caller returns Java's
/// `anyProgress = false` WITHOUT touching the progress counter.
fn begin_pass(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    sink: &mut dyn DriverSink,
    pass_no: i32,
) -> Vec<ItemId> {
    let mut autoroute_item_list = get_autoroute_items(manager, board, sink);
    if autoroute_item_list.is_empty() {
        // Java `router.airLine = null` — banked GUI seam.
        return Vec::new();
    }
    // M6-T7 (default OFF): the congestion-aware planned order — the
    // queue sorted by the plan rank of the item's most urgent net
    // (stable sort; equal ranks keep the DSN/item-id order).
    if settings.congestion_global {
        let plan = crate::global::plan::GlobalPlan::build(board);
        let mut keyed: Vec<(usize, ItemId)> = autoroute_item_list
            .iter()
            .map(|item_id| {
                let rank = board
                    .get(*item_id)
                    .map(|entry| {
                        entry
                            .nets
                            .iter()
                            .filter_map(|net| plan.rank_of(*net))
                            .min()
                            .unwrap_or(usize::MAX)
                    })
                    .unwrap_or(usize::MAX);
                (rank, *item_id)
            })
            .collect();
        keyed.sort_by_key(|(rank, _item_id)| *rank);
        autoroute_item_list = keyed.into_iter().map(|(_rank, item_id)| item_id).collect();
    }
    // M7-T6 (default OFF — an empty declaration list): the pair
    // LEADER-FIRST order. The declared pairs' leader items move to the
    // FRONT of the pass queue (stable — the leader items keep their
    // relative order, and every undeclared net keeps its place after
    // them), so the leader routes first and the follower's coupling
    // preference always finds leader copper on the pass (the
    // deterministic lead rule: the lower net number leads). Gated on
    // `settings.pairs` being non-empty: the default queue is
    // untouched, byte-for-byte.
    if !settings.pairs.is_empty() {
        let leader_nets: std::collections::BTreeSet<i32> =
            settings.pairs.iter().map(|pair| pair.leader).collect();
        let mut keyed: Vec<(bool, ItemId)> = autoroute_item_list
            .iter()
            .map(|item_id| {
                let is_leader_item = board
                    .get(*item_id)
                    .is_some_and(|entry| entry.nets.iter().any(|net| leader_nets.contains(net)));
                (is_leader_item, *item_id)
            })
            .collect();
        keyed.sort_by_key(|(is_leader_item, _item_id)| std::cmp::Reverse(*is_leader_item));
        autoroute_item_list = keyed.into_iter().map(|(_flag, item_id)| item_id).collect();
    }
    let items_to_go_count = i32::try_from(autoroute_item_list.len()).unwrap_or(i32::MAX);
    let mut counters = RouterCounters {
        phase: "autoroute".to_string(),
        pass_count: pass_no,
        queued_to_be_routed_count: items_to_go_count,
        ..RouterCounters::default()
    };
    // Java `:185-189`: `tempDrc.getIncompleteCount()` — the SUM of the
    // per-net airline counts (NOT `maxConnections`; see
    // [`crate::pipeline::batch::calculate_incomplete_count`]).
    let (_max_connections, incomplete_rows) = all_incompletes(manager, board);
    let total_incompletes: usize = incomplete_rows.iter().map(|row| row.incomplete_count).sum();
    counters.incomplete_count = i32::try_from(total_incompletes).unwrap_or(i32::MAX);
    log_incomplete_details(
        board,
        sink,
        pass_no,
        items_to_go_count,
        counters.incomplete_count,
        &incomplete_rows,
    );
    sink.board_updated(&counters);
    // The M9 snapshot hook (event_sink.rs): mirrors the
    // `board_updated` fire immediately — the default sink is a no-op,
    // so the parity stream is byte-stable.
    sink.board_snapshot(board);
    autoroute_item_list
}

/// The pass tail shared by both drivers (Java `:296-329` verbatim):
/// the tail removal with its before/after rows, the final counters
/// fill + event, and the `anyProgress` answer.
#[allow(clippy::too_many_arguments)] // the extracted tail's face
fn finish_pass(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    stop: &StopFace,
    sink: &mut dyn DriverSink,
    pass_no: i32,
    items_to_go_count: i32,
    skipped: i32,
    ripped_item_count: i32,
    not_routed: i32,
    routed: i32,
) -> bool {
    let incompletes_before = calculate_incomplete_count(manager, board);
    sink.trace(&format!(
        "compare_trace_remove_tails Incompletes before remove_tails={incompletes_before}"
    ));
    let stop_connection_option = if settings.remove_unconnected_vias {
        StopConnectionOption::None
    } else {
        StopConnectionOption::FanoutVia
    };
    remove_tails(
        manager,
        board,
        stop_connection_option,
        settings.pull_tight_accuracy,
        &settings.trace_costs,
        stop.flag(),
        TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
        settings.deterministic_budgets,
    );
    let incompletes_after = calculate_incomplete_count(manager, board);
    sink.trace(&format!(
        "compare_trace_remove_tails Incompletes after remove_tails={incompletes_after}"
    ));

    // Java `:308-320` — the final statistics + counters fill and the
    // (ungated) board-update fire.
    let mut counters = RouterCounters {
        phase: "autoroute".to_string(),
        pass_count: pass_no,
        queued_to_be_routed_count: items_to_go_count,
        skipped_count: skipped,
        ripped_count: ripped_item_count,
        failed_to_be_routed_count: not_routed,
        routed_count: routed,
        incomplete_count: calculate_incomplete_count(manager, board),
        ..RouterCounters::default()
    };
    counters.pass_count = pass_no;
    sink.board_updated(&counters);
    // The M9 snapshot hook (event_sink.rs): mirrors the
    // `board_updated` fire immediately — the default sink is a no-op,
    // so the parity stream is byte-stable.
    sink.board_snapshot(board);

    // Java `:322-329` — PerformanceProfiler.recordPass + the benchmark
    // profile render are banked; `router.airLine = null` is the GUI
    // seam.
    routed > 0 || not_routed > 0
}

/// Java `AutoroutePassRunner.runSingleThread(passNo)` (`:150-335`):
/// one pass over the item queue. Returns Java's `anyProgress` —
/// `routed > 0 || not_routed > 0` (`:329`); an empty queue returns
/// `false` (`:162-165`).
///
/// THE GOLDEN PATH (M5-T7): this is the `--threads 1` face — byte-for-
/// byte the pre-T7 walk (the attempt body/tail now live in the shared
/// [`run_attempt`]/[`finish_pass`] helpers, a pure code motion).
#[allow(clippy::too_many_lines)] // the Java body is one flat walk
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn run_single_thread(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    pass_no: i32,
    total_items_routed: &mut i32,
    progress_items_since_statistics: &mut i32,
    stop: &mut StopFace,
    sink: &mut dyn DriverSink,
    pathfinder: Option<&crate::global::history::PathfinderPass>,
) -> bool {
    let autoroute_item_list = begin_pass(manager, board, settings, sink, pass_no);
    if autoroute_item_list.is_empty() {
        return false;
    }
    // Java `router.progressStatistics = new BoardStatistics(...)` —
    // the snapshot is the GUI event (banked); the counter reset is the
    // live residue.
    *progress_items_since_statistics = 0;

    let mut items_to_go_count = i32::try_from(autoroute_item_list.len()).unwrap_or(i32::MAX);
    let mut ripped_item_count: i32 = 0;
    let mut not_routed: i32 = 0;
    let mut routed: i32 = 0;
    let mut skipped: i32 = 0;
    for item_id in &autoroute_item_list {
        if stop.is_requested() {
            break;
        }
        // The current item's faces, read BEFORE any attempt can touch
        // the board (Java reads the live object throughout; the queued
        // items are non-routable connectables, so the read is stable —
        // capturing early keeps the borrows flat).
        let (nets, simple_name): (Vec<i32>, &'static str) = match board.get(*item_id) {
            Some(entry) => (entry.nets.clone(), entry.data.java_simple_name()),
            None => (Vec::new(), "Item"),
        };
        for net in &nets {
            if stop.is_requested() {
                break;
            }
            if !max_items_gate(settings, *total_items_routed, stop, sink) {
                break;
            }
            *total_items_routed += 1;
            let unit = PassUnit {
                item_id: *item_id,
                net: *net,
                first_net: unit_first_net(&nets),
                simple_name,
                items_remaining: items_to_go_count,
            };
            let tally = run_attempt(
                manager,
                board,
                settings,
                sink,
                stop.flag(),
                pass_no,
                &unit,
                pathfinder,
            );
            match tally {
                AttemptTally { routed: true, .. } => routed += 1,
                AttemptTally { skipped: true, .. } => skipped += 1,
                _ => not_routed += 1,
            }
            // Java decrements AFTER the failure row — the row's
            // "items remaining" still counts the current item.
            items_to_go_count -= 1;
            ripped_item_count += i32::try_from(tally.ripped_count).unwrap_or(i32::MAX);
            update_progress(progress_items_since_statistics);
        }
    }
    finish_pass(
        manager,
        board,
        settings,
        stop,
        sink,
        pass_no,
        items_to_go_count,
        skipped,
        ripped_item_count,
        not_routed,
        routed,
    )
}

// ---------------------------------------------------------------------------
// the deterministic partitioned executor (M5-T7, design :260/:261)
// ---------------------------------------------------------------------------

/// Fixed net-id partitioning (design :260): the net ids `1..=max_net`
/// are mapped onto `threads` CONTIGUOUS owner partitions by
/// `((net - 1) * threads) / max_net` — a pure function of
/// `(net, threads, max_net)`, monotone in `net`, balanced, and
/// identical for every pass and every run. Out-of-domain ids are
/// total: `net < 1` and `max_net <= 0` answer partition 0; `net >
/// max_net` answers the last partition (cannot occur on parsed
/// boards — every item net exists in the rules table).
///
/// The partition decides WHICH worker executes a unit, never WHEN:
/// the deterministic reduction is the golden walk order itself (see
/// [`run_partitioned`]).
fn net_partition(net: i32, threads: usize, max_net: i32) -> usize {
    if threads <= 1 || max_net <= 0 || net < 1 {
        return 0;
    }
    if net > max_net {
        return threads - 1;
    }
    let idx = (net - 1) as usize;
    let max = max_net as usize;
    idx * threads / max
}

/// The board-side environment one partition worker holds while it
/// executes a unit — handed off unit-by-unit in the golden order.
struct UnitEnv<'a> {
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    sink: &'a mut dyn DriverSink,
    /// The M6-T8 negotiated pass state (immutable during the pass; a
    /// shared read, never mutated by any worker — the scheduler reads
    /// are pure).
    pathfinder: Option<&'a crate::global::history::PathfinderPass>,
}

/// The [`Send`] wrapper for the hand-off.
///
/// SAFETY: the coordinator dispatches ONE unit at a time and BLOCKS on
/// the worker's reply before the next dispatch — at most one worker
/// can touch the environment at any instant, so exclusive access is
/// guaranteed by construction (this total order IS the deterministic
/// reduction: with ripup enabled every search may rip any item, so
/// every unit is a conflicted point and the byte contract demands the
/// golden interleaving; the executor therefore serializes all of it
/// and parallelizes nothing that could change a byte). No worker
/// retains a reference after replying — the env moves back inside the
/// reply. The referenced types carry no thread-affine state: the only
/// production thread-locals are the epic-index scratch pools (owned
/// `Cell<Option<Vec<_>>>` buffers, take/put within a single call —
/// `epic-index/src/scratch.rs`), and the per-engine `row_buf`/
/// `key_index` fields are created within one attempt.
struct SendUnitEnv<'a>(UnitEnv<'a>);

// The workspace's single deliberate unsafe opt-in outside the SIMD
// charter: the Send assertion is load-bearing for the hand-off
// executor, and its soundness contract is the struct doc above (the
// coordinator's one-in-flight dispatch makes exclusive access true by
// construction).
#[allow(unsafe_code)]
unsafe impl Send for SendUnitEnv<'_> {}

/// One dispatch envelope (the worker owns the unit through the call).
struct UnitJob<'a> {
    env: SendUnitEnv<'a>,
    unit: PassUnit,
}

enum WorkerMessage<'a> {
    Run(UnitJob<'a>),
    Quit,
}

/// The deterministic partitioned executor (M5-T7): the pass walk over
/// the SAME golden queue with `threads > 1`. Units are assigned to
/// FIXED net-id partitions ([`net_partition`]); each partition owns an
/// OS worker thread and every unit runs FULLY (search + insert + the
/// log rows) on its owning worker. The board/manager/sink environment
/// is handed off per unit in the golden order — the conflicted points
/// are serialized exactly, so the output is byte-identical to
/// [`run_single_thread`] at every `threads` (the threads-invariance
/// gate pins this). What the executor buys at N>1 today is
/// partition-affine worker residency (per-thread scratch pools and
/// caches warm up per partition); it does NOT buy search parallelism,
/// because with net-agnostic ripup any search may rip any item, so
/// the parallel order could not reproduce the sequential
/// interleaving the byte contract demands — the charter's
/// "serialize the conflicted points" arm, taken to its sound
/// conclusion. Wall delta is recorded honestly in SEAM/report.
#[allow(clippy::too_many_lines)] // the hand-off walk
#[allow(clippy::too_many_arguments)] // mirrors run_single_thread
pub fn run_partitioned<'a>(
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    settings: &BatchSettings,
    pass_no: i32,
    total_items_routed: &mut i32,
    progress_items_since_statistics: &mut i32,
    stop: &mut StopFace,
    sink: &'a mut (dyn DriverSink + 'a),
    threads: usize,
    pathfinder: Option<&'a crate::global::history::PathfinderPass>,
) -> bool {
    let autoroute_item_list = begin_pass(manager, board, settings, sink, pass_no);
    if autoroute_item_list.is_empty() {
        return false;
    }
    *progress_items_since_statistics = 0;

    let mut items_to_go_count = i32::try_from(autoroute_item_list.len()).unwrap_or(i32::MAX);
    let max_net = board.rules().nets.max_net_number();

    let mut routed: i32 = 0;
    let mut skipped: i32 = 0;
    let mut not_routed: i32 = 0;
    let mut ripped_item_count: i32 = 0;

    // The environment lives in an Option OUTSIDE the scope closure so
    // the coordinator can check it out per unit and the borrow returns
    // to the function body afterwards (closures capture by move; the
    // Option indirection keeps the &mut reassignable across hand-offs).
    let mut env: Option<UnitEnv<'a>> = Some(UnitEnv {
        manager,
        board,
        sink,
        pathfinder,
    });
    let stop_flag = stop.flag().cloned();
    std::thread::scope(|scope| {
        // PER-WORKER reply channels (T7 spec round F1): the coordinator
        // recv()s from EXACTLY the worker it dispatched to, so a
        // panicking worker's sender drop turns that recv into Err BY
        // CONSTRUCTION — the pass fail-fasts instead of hanging on the
        // other workers' alive clones (the shared-channel defect the
        // spec review found). The panic then propagates out of the
        // scope closure; unwinding drops the job senders, the blocked
        // workers drain (`recv() → Err`) and the scope joins before the
        // panic re-raises — the run aborts, mirroring the
        // `catch (Exception) → no Rust catch` bank.
        let mut reply_rxs: Vec<std::sync::mpsc::Receiver<(SendUnitEnv<'a>, AttemptTally)>> =
            Vec::with_capacity(threads);
        let mut job_txs: Vec<std::sync::mpsc::Sender<WorkerMessage<'a>>> =
            Vec::with_capacity(threads);
        let mut handles = Vec::with_capacity(threads);
        for _ in 0..threads {
            let (job_tx, job_rx) = std::sync::mpsc::channel::<WorkerMessage<'a>>();
            let (reply_tx, reply_rx) =
                std::sync::mpsc::channel::<(SendUnitEnv<'a>, AttemptTally)>();
            let stop_flag = stop_flag.clone();
            handles.push(scope.spawn(move || {
                let stop_flag = stop_flag.as_ref();
                while let Ok(message) = job_rx.recv() {
                    match message {
                        WorkerMessage::Quit => break,
                        WorkerMessage::Run(job) => {
                            let UnitJob { env, unit } = job;
                            let SendUnitEnv(UnitEnv {
                                manager,
                                board,
                                sink,
                                pathfinder,
                            }) = env;
                            let tally = run_attempt(
                                manager, board, settings, sink, stop_flag, pass_no, &unit,
                                pathfinder,
                            );
                            let reply = (
                                SendUnitEnv(UnitEnv {
                                    manager,
                                    board,
                                    sink,
                                    pathfinder,
                                }),
                                tally,
                            );
                            if reply_tx.send(reply).is_err() {
                                break;
                            }
                        }
                    }
                }
            }));
            job_txs.push(job_tx);
            reply_rxs.push(reply_rx);
        }

        // The coordinator walk: the golden order, one unit in flight.
        'items: for item_id in &autoroute_item_list {
            if stop.is_requested() {
                break;
            }
            // The item's faces read at walk time from the live board —
            // the same read semantics as the sequential driver.
            let (nets, simple_name): (Vec<i32>, &'static str) =
                match env.as_ref().expect("env free").board.get(*item_id) {
                    Some(entry) => (entry.nets.clone(), entry.data.java_simple_name()),
                    None => (Vec::new(), "Item"),
                };
            for net in &nets {
                if stop.is_requested() {
                    break 'items;
                }
                if !max_items_gate(
                    settings,
                    *total_items_routed,
                    stop,
                    env.as_mut().expect("env free").sink,
                ) {
                    break 'items;
                }
                *total_items_routed += 1;
                let unit = PassUnit {
                    item_id: *item_id,
                    net: *net,
                    first_net: unit_first_net(&nets),
                    simple_name,
                    items_remaining: items_to_go_count,
                };
                // The unit identity outlives the move into the job (the
                // F1 panic path names it after the worker dies).
                let unit_net = unit.net;
                let unit_item_id = unit.item_id.get();
                let part = net_partition(*net, threads, max_net);
                let job_tx = &job_txs[part];
                let checked_out = env.take().expect("env free between units");
                job_tx
                    .send(WorkerMessage::Run(UnitJob {
                        env: SendUnitEnv(checked_out),
                        unit,
                    }))
                    .expect("partition worker alive");
                // Fail-fast (F1): this recv reads EXACTLY the worker
                // the job was dispatched to, so `Err` means that worker
                // PANICKED mid-attempt — propagate with the unit
                // identity (the original panic's message already went
                // to stderr through the default hook; the payload
                // cannot cross the channel in general — a documented
                // residual vs Java's exception object).
                let (env_back, tally) = match reply_rxs[part].recv() {
                    Ok(reply) => reply,
                    Err(_) => panic!(
                        "partition worker died executing unit net={unit_net} \
                         item={unit_item_id} (pass {pass_no}); the worker's own panic message \
                         precedes this on stderr"
                    ),
                };
                let SendUnitEnv(restored) = env_back;
                env = Some(restored);
                match tally {
                    AttemptTally { routed: true, .. } => routed += 1,
                    AttemptTally { skipped: true, .. } => skipped += 1,
                    _ => not_routed += 1,
                }
                items_to_go_count -= 1;
                ripped_item_count += i32::try_from(tally.ripped_count).unwrap_or(i32::MAX);
                update_progress(progress_items_since_statistics);
            }
        }

        for job_tx in &job_txs {
            let _ = job_tx.send(WorkerMessage::Quit);
        }
        for handle in handles {
            let _ = handle.join();
        }
    });

    let UnitEnv {
        manager,
        board,
        sink,
        ..
    } = env
        .take()
        .expect("env returned after the worker fleet drains");

    finish_pass(
        manager,
        board,
        settings,
        stop,
        sink,
        pass_no,
        items_to_go_count,
        skipped,
        ripped_item_count,
        not_routed,
        routed,
    )
}

/// The batch stage's pass entry (M5-T7): `settings.max_threads`
/// 0/1 engages the golden sequential face, `>= 2` the partitioned
/// executor. Byte-identical at every N by construction. The OPTIMIZER
/// stage's inline reroute passes call [`run_single_thread`] directly
/// (sequential only at this tree — the scope comment at that call
/// site); they do NOT route through here.
#[allow(clippy::too_many_arguments)] // the shared entry
pub fn run_pass(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    pass_no: i32,
    total_items_routed: &mut i32,
    progress_items_since_statistics: &mut i32,
    stop: &mut StopFace,
    sink: &mut dyn DriverSink,
    pathfinder: Option<&crate::global::history::PathfinderPass>,
) -> bool {
    let threads = usize::try_from(settings.max_threads.max(0)).unwrap_or(0);
    if threads <= 1 {
        run_single_thread(
            manager,
            board,
            settings,
            pass_no,
            total_items_routed,
            progress_items_since_statistics,
            stop,
            sink,
            pathfinder,
        )
    } else {
        run_partitioned(
            manager,
            board,
            settings,
            pass_no,
            total_items_routed,
            progress_items_since_statistics,
            stop,
            sink,
            threads,
            pathfinder,
        )
    }
}

/// Java `updateProgress` (`:489-513`): bump the interval counter and
/// reset it at the interval (the snapshot + `shouldFireBoardUpdate()`
/// throttle around it are the banked GUI event faces).
fn update_progress(progress_items_since_statistics: &mut i32) {
    *progress_items_since_statistics += 1;
    if *progress_items_since_statistics >= PROGRESS_STATISTICS_ITEM_INTERVAL {
        *progress_items_since_statistics = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::pipeline::batch::{BatchSettings, StopFace};
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::test_util::parse;
    use epic_board::board::ItemEntry;
    use epic_board::id::ItemId;
    use epic_board::items::{Area, BoardShape, FixedState};
    use epic_board::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`,
    /// resolution 10; ALL nets single-pin; the net-94 pin D093 sits at
    /// DB (663500, 20000)).
    pub(crate) fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    /// The jar world's cost table (the t11 capture row `ctrl_costs`).
    fn settings_ir() -> RouterSettingsIr {
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
        }
    }

    /// The starved batch settings: only the fixture's UNUSED layer (1)
    /// active — Java's maze reads `ctrl.layerActive` at every
    /// expansion (`MazeSearchEngine.java:396-397/478/493`), so every
    /// attempt on this layer-0 world fails deterministically with zero
    /// expansion work (the t11 `layer_active` world A, driver-shaped).
    pub(crate) fn starved_settings() -> BatchSettings {
        let mut settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        settings.router_settings.layer_active = vec![false, true];
        settings
    }

    fn corner(x: i32, y: i32) -> IntPoint {
        IntPoint::new(x, y)
    }

    /// The t11 anchor trace: net 94's dangling stub in the y=300000
    /// corridor (UNFIXED → `is_routable` true → never queued itself,
    /// but it doubles net 94's connectable population so the pin
    /// queues).
    fn insert_anchor(manager: &mut SearchTreeManager, board: &mut Board) -> ItemId {
        insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(
                &Point::Int(corner(600_000, 300_000)),
                &Point::Int(corner(620_000, 300_000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace")
    }

    /// A hand-inserted net-49 conduction area (the port's second
    /// endpoint kind — Java `NetIncompletes` counts Pin/CA endpoints;
    /// traces are never endpoints). Component-attached so the
    /// connected-set walk enters it (Java `Item.java:666-669` skips
    /// free pours).
    fn insert_ca(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        net: i32,
        component_id: i32,
        lo: (i32, i32),
        hi: (i32, i32),
    ) -> ItemId {
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::ConductionArea {
                layer: 0,
                area: Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
                    RegularTileShape::IntBox(IntBox::new(
                        IntPoint::new(lo.0, lo.1),
                        IntPoint::new(hi.0, hi.1),
                    )),
                ))),
                is_obstacle: false,
                is_filled: true,
            },
            nets: vec![net],
            clearance_class: 1,
            component_id,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(board, id);
        id
    }

    /// The fixture's net-94 pin's component id (the CA arms reuse it —
    /// the pin rides that component in the fixture).
    fn pin94_component_id(board: &Board) -> i32 {
        board
            .iter_ascending()
            .find(|entry| entry.nets.contains(&94) && matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.component_id)
            .expect("net 94 has a pin")
    }

    /// The starved three-attempt world: net 94's pin (population
    /// doubled by the dangling anchor) plus net 49's pin + dangling CA
    /// (one PERMANENT incomplete — 2 endpoints that never connect).
    /// Queue order is seed order (descending id): the CA (newest) is
    /// queued first.
    fn starved_world() -> (SearchTreeManager, Board, BatchSettings) {
        let (mut manager, mut board) = parse_fixture();
        let _anchor = insert_anchor(&mut manager, &mut board);
        let comp = pin94_component_id(&board);
        let _ca = insert_ca(
            &mut manager,
            &mut board,
            49,
            comp,
            (640_000, 10_000),
            (650_000, 20_000),
        );
        let settings = starved_settings();
        (manager, board, settings)
    }

    /// Fixture pins ride components (`component_id > 0`), so
    /// `remove_item_through_repository`'s `is_deletion_forbidden` guard
    /// refuses them (Java `Item.isDeletionForbidden` parity). The
    /// empty-queue world removes them directly — same tree+arena
    /// removal pair, guard skipped (synthetic test world only).
    fn remove_item_unguarded(manager: &mut SearchTreeManager, board: &mut Board, id: ItemId) {
        manager.remove(board, id);
        board.remove_item(id);
    }

    /// The empty-queue face: with nets 33/98's four pins removed, every
    /// remaining net is a lone pin (its connected set covers the net's
    /// population), so the queue is empty and the pass returns Java's
    /// `anyProgress` = false WITHOUT touching the caller's progress
    /// counter (the reset sits behind the empty gate).
    #[test]
    fn pr_empty_queue_returns_false() {
        let (mut manager, mut board) = parse_fixture();
        // The fixture's only under-connected nets are 33 and 98 (one
        // pin pair each) — remove all four pins and the queue empties.
        let pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| entry.nets.contains(&33) || entry.nets.contains(&98))
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        assert_eq!(pins.len(), 4, "two pin pairs");
        for id in pins {
            remove_item_unguarded(&mut manager, &mut board, id);
        }
        let settings = starved_settings();
        let mut total = 0;
        let mut progress = 7;
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let any_progress = run_single_thread(
            &mut manager,
            &mut board,
            &settings,
            1,
            &mut total,
            &mut progress,
            &mut stop,
            &mut sink,
            None,
        );
        assert!(!any_progress, "an empty queue is no progress");
        assert_eq!(total, 0, "no attempts");
        assert_eq!(progress, 7, "the counter reset sits behind the empty gate");
        assert!(!stop.is_requested(), "no stop");
        assert!(
            !sink.any_contains("Queuing item"),
            "nothing queued after the pairs are gone"
        );
    }

    /// The counters row render: Java ships the `RouterCounters` OBJECT
    /// through the event; the row is the port-defined rendering and is
    /// pinned verbatim (a dropped field dies here).
    #[test]
    fn pr_render_counters_exact() {
        let counters = RouterCounters {
            phase: "autoroute".to_string(),
            pass_count: 3,
            queued_to_be_routed_count: 12,
            skipped_count: 4,
            ripped_count: 5,
            failed_to_be_routed_count: 2,
            routed_count: 6,
            incomplete_count: 7,
            fanout_extra_vias_count: 1,
        };
        assert_eq!(
            render_counters(&counters),
            "phase=autoroute pass=3 queued=12 skipped=4 ripped=5 failed=2 routed=6 \
             incomplete=7 fanout_extra_vias=1"
        );
    }

    /// Java `logRippedItems` (`:360-394`): the TreeSet iterates
    /// DESCENDING id, the net list joins with `|`, and the cost lookup
    /// defaults to -1 (`getOrDefault`). The exact rows, in order, kill
    /// the `.rev()` drop, the default-cost change, and a type-name
    /// read from the wrong source.
    #[test]
    fn pr_log_ripped_items_descending_join_default() {
        let mut sink = CaptureDriverSink::default();
        let mut ripped = BTreeMap::new();
        ripped.insert(
            5,
            RippedItemSeed {
                key: 105,
                simple_name: "PolylineTrace",
                nets: vec![2, 7],
            },
        );
        ripped.insert(
            3,
            RippedItemSeed {
                key: 103,
                simple_name: "Via",
                nets: vec![9],
            },
        );
        let mut costs = HashMap::new();
        costs.insert(105_u64, 120_i32);
        log_ripped_items(&mut sink, 11, 94, &ripped, &costs);
        let trace = sink.joined("trace");
        let rows: Vec<&str> = trace.lines().collect();
        assert_eq!(rows.len(), 2, "one row per seed: {trace}");
        assert_eq!(
            rows[0],
            "compare_trace_ripped_item source_item=11, source_net=94, ripped_id=5, \
             ripped_type=PolylineTrace, ripped_net_count=2, ripped_nets=2|7, ripupCost=120",
            "the HIGHER id first (descending TreeSet)"
        );
        assert_eq!(
            rows[1],
            "compare_trace_ripped_item source_item=11, source_net=94, ripped_id=3, \
             ripped_type=Via, ripped_net_count=1, ripped_nets=9, ripupCost=-1",
            "a seed missing from the cost map renders the -1 default"
        );
    }

    /// Java `logIncompleteDetails` (`:337-358`): the gate (no rows at
    /// zero incompletes), the pass header with the item count, and the
    /// per-net rows for exactly the non-empty nets, in net-ascending
    /// order (the `all_incompletes` row order).
    #[test]
    fn pr_log_incomplete_details_rows_and_gate() {
        let (manager, mut board, _settings) = starved_world();
        let (total, rows) = all_incompletes(&manager, &mut board);
        assert_eq!(
            total, 3,
            "NET_33 + NET_49 (the hand-inserted pair) + NET_98"
        );
        let mut sink = CaptureDriverSink::default();
        log_incomplete_details(
            &board,
            &mut sink,
            3,
            7,
            i32::try_from(total).expect("fits"),
            &rows,
        );
        let debug = sink.joined("debug");
        let lines: Vec<&str> = debug.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("Pass #3: 3 incompletes across 7 items to route"),
            "the pass header: {debug}"
        );
        assert_eq!(lines.len(), 4, "header + three non-empty nets: {debug}");
        assert!(
            lines[1].contains("Net 'NET_33' has 1 incomplete(s)"),
            "net-ascending order: {debug}"
        );
        assert!(
            lines[2].contains("Net 'N048' has 1 incomplete(s)"),
            "net 49's generated name (N + zero-padded index-1): {debug}"
        );
        assert!(
            lines[3].contains("Net 'NET_98' has 1 incomplete(s)"),
            "net-ascending order: {debug}"
        );

        // The gate: zero incompletes → NO rows at all (the bare
        // fixture minus its pairs has none).
        let (mut manager, mut board) = parse_fixture();
        let pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| entry.nets.contains(&33) || entry.nets.contains(&98))
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        for id in pins {
            remove_item_unguarded(&mut manager, &mut board, id);
        }
        let (total, rows) = all_incompletes(&manager, &mut board);
        assert_eq!(total, 0, "the pair-less fixture has no incompletes");
        let mut sink = CaptureDriverSink::default();
        log_incomplete_details(
            &board,
            &mut sink,
            1,
            0,
            i32::try_from(total).expect("fits"),
            &rows,
        );
        assert!(
            sink.rows.is_empty(),
            "the zero gate emits nothing: {:?}",
            sink.rows
        );
    }

    /// Java `logTraceRouteComparison` (`:396-441`): the row renders
    /// the attempt state, details, the FRESH incompletes total, the
    /// net's own incomplete count, the ripped count, the net
    /// population before→after, and the id watermark (the world's
    /// stable high-water: anchor 103 + CA 104 → allocation head 106
    /// after the registry's internal keys... the parse+2-inserts
    /// high-water is deterministic and pinned).
    #[test]
    fn pr_route_comparison_row() {
        let (mut manager, mut board, _settings) = starved_world();
        let mut sink = CaptureDriverSink::default();
        let result = AutorouteAttemptResult::with_details(
            AutorouteAttemptState::Failed,
            "unit failure".to_string(),
        );
        log_trace_route_comparison(
            &mut manager,
            &mut board,
            &mut sink,
            "Pin",
            49,
            &result,
            0,
            2,
        );
        let trace = sink.joined("trace");
        let rows: Vec<&str> = trace.lines().collect();
        assert_eq!(rows.len(), 1, "one row: {trace}");
        assert_eq!(
            rows[0],
            "compare_trace_route_item Routing Pin -> result=FAILED, details=unit failure, \
             incompletes=3, netIncomplete=1, ripped=0, netItems=2->2, maxItemId=106",
            "the full row, including the fresh walk's totals"
        );
    }

    /// Java `logNet94Items` (`:444-487`): the header, the trace row
    /// with `IntPoint.toString` corners (no space), and the pin row
    /// resolved through the component/package chain (`Pin.name()` =
    /// the package pad name PD; the component refdes D093 renders in
    /// the comp field).
    #[test]
    fn pr_net94_dump_rows() {
        let (mut manager, mut board) = parse_fixture();
        let _anchor = insert_anchor(&mut manager, &mut board);
        let mut sink = CaptureDriverSink::default();
        log_net94_items(&mut board, &mut sink);
        let trace = sink.joined("trace");
        let rows: Vec<&str> = trace.lines().collect();
        assert_eq!(rows.len(), 3, "header + anchor trace + pin: {trace}");
        assert_eq!(rows[0], "compare_trace_dump_net_items Dump net 94 items");
        assert_eq!(
            rows[1],
            "compare_trace_dump_net_item Trace layer=0 corners=(600000,300000) to (620000,300000)",
            "the anchor trace row, Java IntPoint.toString corners"
        );
        assert_eq!(
            rows[2], "compare_trace_dump_net_item Pin center=(663500,20000) name=PD comp=D093",
            "the pin row: package pin name + component refdes"
        );
    }

    /// The full pass walk on the starved world: 7 queued attempts (the
    /// fixture's own NET_33/NET_98 pin pairs plus the hand-inserted
    /// NET_49 pair and the anchor-inflated net 94), all FAILED, the
    /// queue-row ORDER (CA first — descending seed ids), the pre-pass
    /// and final counters rows, the per-attempt failure rows with the
    /// not-yet-decremented counter, and the tail-removal rows. Java
    /// `runSingleThread` (`:150-335`) end to end.
    #[test]
    fn pr_pass_walk_rows_and_tally() {
        let (mut manager, mut board, settings) = starved_world();
        let anchor = board
            .iter_descending()
            .find(|entry| entry.nets.contains(&94) && matches!(entry.data, ItemData::Trace { .. }))
            .map(|entry| entry.id)
            .expect("anchor present");
        let mut total = 0;
        let mut progress = 0;
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let any_progress = run_single_thread(
            &mut manager,
            &mut board,
            &settings,
            1,
            &mut total,
            &mut progress,
            &mut stop,
            &mut sink,
            None,
        );
        assert!(any_progress, "failures ARE progress (Java :329)");
        assert_eq!(total, 7, "one attempt per queued (item, net) pair");
        assert!(!stop.is_requested(), "no stop source fired");
        assert_eq!(progress, 7, "the interval counter counts the 7 attempts");

        // Queue rows: descending seed id — the CA (newest) first, then
        // the fixture pins; each carries the connected/population pair.
        let debug = sink.joined("debug");
        let queue: Vec<&str> = debug
            .lines()
            .filter(|row| row.contains("Queuing item for routing:"))
            .collect();
        assert_eq!(queue.len(), 7, "seven queue rows: {debug}");
        assert!(
            queue[0].contains("Queuing item for routing: ConductionArea")
                // #152 commit B rotation (2026-10-03, upstream
                // d0d876e30): the row gained the `plane: false` field.
                && queue[0].contains("(connected: 1/2, plane: false)"),
            "the newest id seeds first, lone CA set vs net population: {queue:?}"
        );
        assert_eq!(
            queue
                .iter()
                .filter(|row| row.contains("ConductionArea"))
                .count(),
            1,
            "only the hand-inserted CA queues"
        );

        // The pass header + the pre-pass counters row.
        assert!(
            sink.any_contains("Pass #1: 3 incompletes across 7 items to route"),
            "the pass header"
        );
        let updates_text = sink.joined("board_updated");
        let updates: Vec<&str> = updates_text.lines().collect();
        assert_eq!(updates.len(), 2, "pre-pass + final fire: {updates_text}");
        assert_eq!(
            updates[0],
            "phase=autoroute pass=1 queued=7 skipped=0 ripped=0 failed=0 routed=0 \
             incomplete=3 fanout_extra_vias=0",
        );
        assert_eq!(
            updates[1],
            "phase=autoroute pass=1 queued=0 skipped=0 ripped=0 failed=7 routed=0 \
             incomplete=3 fanout_extra_vias=0",
            "the final fill: queue drained, 7 failures, incompletes unchanged"
        );

        // The engine's failure-detail debug row fires for EVERY
        // attempt (7), but the pass-runner failure ROW is gated at
        // `items_to_go <= 5` — the first two attempts (the CA at 7
        // remaining, the first NET_98 pin at 6) are suppressed.
        let details_rows: Vec<&str> = debug
            .lines()
            .filter(|row| row.starts_with("Autorouter Failed to route connection"))
            .collect();
        assert_eq!(details_rows.len(), 7, "one engine detail per attempt");
        let failures: Vec<&str> = debug
            .lines()
            .filter(|row| row.contains("State: FAILED"))
            .collect();
        assert_eq!(
            failures.len(),
            5,
            "the gate suppresses rows above 5: {debug}"
        );
        assert!(
            failures
                .iter()
                .all(|row| !row.contains("(7 items remaining")
                    && !row.contains("(6 items remaining")),
            "the >5 gate really suppressed the first two: {failures:?}"
        );
        assert!(
            failures[0].contains("Pin on net 'NET_98' (5 items remaining, 1 failures)")
                && failures[0].ends_with("State: FAILED"),
            "the first VISIBLE failure row: {failures:?}"
        );
        assert!(
            failures[1].contains("(4 items remaining, 1 failures)"),
            "the row's counter still includes the CURRENT item: {failures:?}"
        );
        assert!(
            failures[4].contains("Pin on net 'N048' (1 items remaining, 1 failures)"),
            "the last item's row: {failures:?}"
        );

        // The tail-removal rows + the sweep itself: the dangling
        // anchor is a free-floating trace → swept (Java's
        // removeTails runs at the end of every pass).
        assert!(
            sink.any_contains("compare_trace_remove_tails Incompletes before remove_tails=3"),
            "the before row"
        );
        assert!(
            sink.any_contains("compare_trace_remove_tails Incompletes after remove_tails=3"),
            "the after row (the anchor is not an endpoint — incompletes stay 3)"
        );
        assert!(
            !board.get(anchor).is_some_and(|entry| entry.on_the_board),
            "the dangling anchor was swept"
        );
    }

    // -----------------------------------------------------------------
    // M5-T7 — deterministic partitioned executor pins
    // -----------------------------------------------------------------

    /// A capture sink that also witnesses the thread each row was
    /// emitted on — the executor-engagement observer.
    struct ThreadWitnessSink {
        inner: CaptureDriverSink,
        thread_ids: Vec<std::thread::ThreadId>,
    }

    impl ThreadWitnessSink {
        fn note(&mut self) {
            self.thread_ids.push(std::thread::current().id());
        }
    }

    impl DriverSink for ThreadWitnessSink {
        fn info(&mut self, message: &str) {
            self.note();
            self.inner.info(message);
        }
        fn warn(&mut self, message: &str) {
            self.note();
            self.inner.warn(message);
        }
        fn debug(&mut self, message: &str) {
            self.note();
            self.inner.debug(message);
        }
        fn trace(&mut self, message: &str) {
            self.note();
            self.inner.trace(message);
        }
        fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
            self.note();
            self.inner.task_state(state, pass, hash);
        }
        fn board_updated(&mut self, counters: &RouterCounters) {
            self.note();
            self.inner.board_updated(counters);
        }
    }

    /// M5-T7 PIN (T7-P1) — the fixed net-id partition function
    /// (design :260), pinned at its EXACT boundaries: monotone,
    /// total over `1..=max_net`, balanced contiguous ranges with the
    /// exact `floor(idx * threads / max)` edges, and a pure function
    /// of `(net, threads, max_net)` — the same net lands on the same
    /// partition for every call and every pass.
    #[test]
    fn t7_net_partition_exact_boundaries() {
        // threads=4 over nets 1..=10: floor(idx*4/10) with
        // idx = net-1 gives ranges [1..3]→0, [4..5]→1, [6..8]→2,
        // [9..10]→3. Pinned exactly below.
        let expected_4_10 = [
            (1usize, 0usize),
            (2, 0),
            (3, 0),
            (4, 1),
            (5, 1),
            (6, 2),
            (7, 2),
            (8, 2),
            (9, 3),
            (10, 3),
        ];
        for (net, part) in expected_4_10 {
            assert_eq!(
                net_partition(net as i32, 4, 10),
                part,
                "net {net} must own partition {part} at threads=4, max_net=10"
            );
        }
        // The exact ±1 boundary faces: floor(idx*4/10) steps at
        // idx2→0, idx3→1, idx7→2, idx8→3 — net 3 is the LAST of
        // range 0, net 4 the FIRST of range 1, net 8 the LAST of
        // range 2, net 9 the FIRST of range 3.
        assert_eq!(net_partition(3, 4, 10), 0);
        assert_eq!(net_partition(4, 4, 10), 1);
        assert_eq!(net_partition(8, 4, 10), 2);
        assert_eq!(net_partition(9, 4, 10), 3);

        // The odd N: threads=3 over nets 1..=7 → floor(idx*3/7):
        // ranges [1..3]→0, [4..5]→1, [6..7]→2.
        let expected_3_7 = [
            (1usize, 0usize),
            (2, 0),
            (3, 0),
            (4, 1),
            (5, 1),
            (6, 2),
            (7, 2),
        ];
        for (net, part) in expected_3_7 {
            assert_eq!(
                net_partition(net as i32, 3, 7),
                part,
                "net {net} at threads=3"
            );
        }

        // Totality + monotonicity over a bigger sweep.
        let mut last = 0usize;
        for net in 1..=160i32 {
            let part = net_partition(net, 4, 160);
            assert!(part < 4, "partition index in range");
            assert!(part >= last, "monotone in net");
            last = part;
        }
        // Degenerate domains answer total values (never panic).
        assert_eq!(net_partition(0, 4, 10), 0, "net<1 → 0");
        assert_eq!(net_partition(5, 4, 0), 0, "max_net<=0 → 0");
        assert_eq!(net_partition(5, 0, 10), 0, "threads<=1 → 0");
        assert_eq!(net_partition(11, 4, 10), 3, "net>max_net → last partition");
        assert_eq!(net_partition(5, 1, 10), 0, "single thread → 0");
    }

    /// M5-T7 PIN (T7-P2) — the executor is byte-identical to the
    /// golden sequential face: the SAME starved world routed through
    /// `run_pass` at threads 1 / 3 / 4 produces the identical sink row
    /// stream (every level, in emission order), identical attempt
    /// counts, and an identical board hash afterwards.
    #[test]
    fn t7_executor_rows_and_hash_identical_across_threads() {
        let faces = [1usize, 3, 4];
        type BaselineRecord = (Vec<(&'static str, String)>, bool, i32, String);
        let mut baseline: Option<BaselineRecord> = None;
        for threads in faces {
            let (mut manager, mut board, mut settings) = starved_world();
            settings.max_threads = threads as i32;
            let mut total = 0;
            let mut progress = 0;
            let mut stop = StopFace::default();
            let mut sink = ThreadWitnessSink {
                inner: CaptureDriverSink::default(),
                thread_ids: Vec::new(),
            };
            let any_progress = run_pass(
                &mut manager,
                &mut board,
                &settings,
                1,
                &mut total,
                &mut progress,
                &mut stop,
                &mut sink,
                None,
            );
            let hash = crate::pipeline::board_hash::board_hash(&board);
            let rows = sink.inner.rows.clone();
            match &baseline {
                None => baseline = Some((rows, any_progress, total, hash)),
                Some((base_rows, base_progress, base_total, base_hash)) => {
                    assert_eq!(rows, *base_rows, "sink rows at threads={threads}");
                    assert_eq!(any_progress, *base_progress);
                    assert_eq!(total, *base_total, "attempt count at threads={threads}");
                    assert_eq!(hash, *base_hash, "board hash at threads={threads}");
                }
            }
        }
        let (_, _, total, _) = baseline.expect("at least one face ran");
        assert_eq!(
            total, 7,
            "the starved world's seven attempts ran at every face"
        );
    }

    /// M5-T7 PIN (T7-P3) — the partition assignment is LOAD-BEARING:
    /// at threads=3 the starved world's units execute on at least two
    /// DISTINCT worker threads (the world's nets span ≥ 2 partitions),
    /// while the golden sequential face stays on the single calling
    /// thread. Kills the always-partition-0 wiring mutant that the
    /// byte-invariance pin (T7-P2) cannot see — any partition map is
    /// byte-identical by construction; only the thread witness
    /// observes WHICH worker ran.
    #[test]
    fn t7_executor_engages_distinct_partition_workers() {
        // The world's attempt nets under threads=3, max_net=98:
        // 33→0, 94→2, 98→2 (floor(93·3/98)=2, floor(97·3/98)=2) —
        // at least two partitions own units.
        assert_eq!(net_partition(33, 3, 98), 0);
        assert_eq!(net_partition(94, 3, 98), 2);
        assert_eq!(net_partition(98, 3, 98), 2);

        let (mut manager, mut board, mut settings) = starved_world();
        settings.max_threads = 3;
        let mut total = 0;
        let mut progress = 0;
        let mut stop = StopFace::default();
        let mut sink = ThreadWitnessSink {
            inner: CaptureDriverSink::default(),
            thread_ids: Vec::new(),
        };
        let any_progress = run_pass(
            &mut manager,
            &mut board,
            &settings,
            1,
            &mut total,
            &mut progress,
            &mut stop,
            &mut sink,
            None,
        );
        assert!(any_progress);
        // The world's attempt nets under threads=3, max_net=98 span
        // THREE partitions: 33→0, 49→1, 94/98→2 — plus the coordinator
        // thread, so the row witness must see >= 4 distinct threads.
        // (The coordinator's prelude/tail rows always count as one; a
        // lower threshold would survive an always-partition-0 mutant.)
        assert_eq!(net_partition(49, 3, 98), 1);
        let distinct: std::collections::HashSet<std::thread::ThreadId> =
            sink.thread_ids.iter().copied().collect();
        assert!(
            distinct.len() >= 4,
            "units must execute on the >= 3 distinct partition owners + coordinator, saw {distinct:?}"
        );

        // Contrast face: threads=1 stays on the calling thread.
        let (mut manager, mut board, mut settings) = starved_world();
        settings.max_threads = 1;
        let mut total = 0;
        let mut progress = 0;
        let mut stop = StopFace::default();
        let mut sink = ThreadWitnessSink {
            inner: CaptureDriverSink::default(),
            thread_ids: Vec::new(),
        };
        run_pass(
            &mut manager,
            &mut board,
            &settings,
            1,
            &mut total,
            &mut progress,
            &mut stop,
            &mut sink,
            None,
        );
        let distinct_1: std::collections::HashSet<std::thread::ThreadId> =
            sink.thread_ids.iter().copied().collect();
        assert_eq!(
            distinct_1.len(),
            1,
            "the sequential face emits every row from one thread"
        );
    }

    /// M5-T7 PIN (T7-B1) — deterministic budgets are thread-count
    /// invariant: the call-tick ladder (`deterministic_limit_for_pass`,
    /// Java `:72-74`) is a pure function of the pass number and the
    /// budget instance is created per attempt inside the unit body
    /// (`route` → `init_autoroute`), so no thread state can move a
    /// limit. Pinned at the ladder's faces: doubling passes, the
    /// last uncapped rung, and the cap boundary — amended M11-T4
    /// (buglog 256): the ladder now min's
    /// `RouteBudget::SEARCH_TICK_CEILING` (4,194,304 = 95x the
    /// measured healthy search max) ABOVE Java's own i32::MAX clamp,
    /// so the first capped rung is pass 7 (100000·2^6 = 6,400,000 >
    /// the ceiling; the old i32::MAX saturation at pass 16 is
    /// unreachable through the ceiling).
    #[test]
    fn t7_deterministic_budget_ladder_thread_invariant() {
        use crate::pipeline::connection_router::deterministic_limit_for_pass;
        assert_eq!(deterministic_limit_for_pass(1), 100_000);
        assert_eq!(deterministic_limit_for_pass(2), 200_000);
        assert_eq!(
            deterministic_limit_for_pass(6),
            3_200_000,
            "the last uncapped rung (100000·2^5)"
        );
        assert_eq!(
            deterministic_limit_for_pass(7),
            4_194_304,
            "the first ceiling rung (100000·2^6 = 6,400,000 > the ceiling)"
        );
        assert_eq!(
            deterministic_limit_for_pass(11),
            4_194_304,
            "the ceiling face (100000·2^10 = 102,400,000 clamped)"
        );
        assert_eq!(
            deterministic_limit_for_pass(32),
            4_194_304,
            "the ceiling holds for deep passes (i32::MAX only beneath it)"
        );
    }

    // -----------------------------------------------------------------
    // M5-T7 F1 — the partitioned face fail-fast pin
    // -----------------------------------------------------------------

    /// RAII reset so a mid-test panic cannot leak the hook into other
    /// tests (belt-and-braces: the pass-777 gate already isolates).
    struct ResetForcePanic;
    impl Drop for ResetForcePanic {
        fn drop(&mut self) {
            FORCE_ATTEMPT_PANIC.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// M5-T7 F1 PIN — a panicking partition worker FAILS FAST with the
    /// unit identity instead of hanging the pass (the shared-reply-
    /// channel defect the spec review found, closed with per-worker
    /// reply channels: the coordinator recv()s from exactly the
    /// dispatched worker, so the dead worker's sender drop is the
    /// propagation trigger BY CONSTRUCTION). The forced panic fires on
    /// pass 777 only (cross-test isolation).
    ///
    /// MUTATION DISCLOSURE (cerebrum-11): the natural no-propagation
    /// mutant (a revert to the shared reply channel) manifests as a
    /// HANG — unpinnable-without-timeout, disclosed per the fix brief.
    /// The FAIL-FAST mutants ARE killed: (a) a propagation-message
    /// mutant (identity stripped / generic text) dies on the expected-
    /// prefix mismatch; (b) any swallow-mutant loses the env, and the
    /// next unit's `env.take()` panics with "env free between units" —
    /// also a fast expected-prefix mismatch.
    #[test]
    #[should_panic(expected = "partition worker died executing unit net=")]
    fn t7_f1_panicking_worker_fails_fast_with_unit_identity() {
        FORCE_ATTEMPT_PANIC.store(true, std::sync::atomic::Ordering::SeqCst);
        let _reset = ResetForcePanic;
        let (mut manager, mut board, mut settings) = starved_world();
        settings.max_threads = 3;
        let mut total = 0;
        let mut progress = 0;
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        run_pass(
            &mut manager,
            &mut board,
            &settings,
            777,
            &mut total,
            &mut progress,
            &mut stop,
            &mut sink,
            None,
        );
        panic!("the forced-panic attempt did not propagate");
    }
}
