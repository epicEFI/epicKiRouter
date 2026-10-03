//! Java `autoroute/pipeline/BatchFanout.java` — the fanout stage: the
//! SMD-pin escape loop ([`fanout_board`]/[`fanout_pass`]) and the
//! per-pin escape attempt (Java `RoutingBoard.fanout`,
//! `RoutingBoard.java:978-1110`, ported as [`fanout_pin`]).
//!
//! ## Banks (SEAM carries the dossier — the "T7 fanout stage" section)
//!
//! * **Multi-arg trace rows are folded** into
//!   `sink.trace("<operation> <message>")` — the Java
//!   `FRLogger.trace(method, operation, message, impactedItems,
//!   impactedPoints)` method tag and impact payloads are log-only
//!   (bug-118 no-digest convention). The folded rows: `pass_start`,
//!   `pin_start`, `pin_routed`, `pin_already_connected`,
//!   `pin_failed`, `pin_insert_error`, `pin_no_unconnected_nets`,
//!   `pin_other_state`, `fanout_via_reverted`, `pass_end`.
//! * **The `ProgressThrottler` mid-pass ticks are banked** — Java's
//!   1-second GUI progress updates ride the listener only; the port
//!   publishes exactly the Java-unconditional sites (the pass-start
//!   publish, the pass-end publish, the per-pin timeout publish and
//!   the thread-stop publish). `maybePublishProgress` with
//!   `passCompleted=false` has no port call site.
//! * **The listener status is a projection.** Java's
//!   `FanoutPassStatus` carries the whole `BoardStatistics` object and
//!   the batch-loop listener reads `board.getHash()` itself; the
//!   ported status carries the only consumed faces —
//!   `incomplete_count` (Java
//!   `boardStatistics().connections.incompleteCount`) and
//!   `board_hash` (the live hash at publish time) — both computed at
//!   the publish site, the same points Java computes the stats. The
//!   Java `progressStats.vias/traces.totalCount` overwrites
//!   (`:545-546`) feed GUI panel fields with no headless consumer —
//!   banked. `BoardStatistics` construction is read-only, so gating
//!   the pass-start stats computation on listener presence deviates
//!   only in CPU.
//! * **`startMarkingChangedArea` is banked** (`BatchFanout.java:280`)
//!   — the changed-area observer seam; the port has no observer
//!   system (the engine.rs observer bank).
//! * **The stage deadline is wall-profile-only.** Java parses
//!   `timeoutString` into a `System.currentTimeMillis` deadline;
//!   behind `deterministic_budgets` the port leaves the deadline
//!   unset (no wall to time out — the banked job-timeout seam). The
//!   per-pin budget spends Java's exact `(int) (baseMillisPerPin *
//!   (passNo + 1))` value as [`RouteBudget`] ticks in the
//!   deterministic profile.
//! * **`TextManager.parseTimespanString` delegation is banked** —
//!   Java converts `H:MM:SS` to a `Duration.parse` literal and
//!   answers null on any parse failure; [`parse_timespan_string`]
//!   splits on `:` directly (1/2/3 arms, per-part signed i64 parse,
//!   null on failure, more than 3 parts, or no parts left after
//!   Java's trailing-empty split drop — JDK-verified faces documented
//!   on the fn).
//! * **`ViaRule.contains` is via-info-index containment.** Java
//!   compares `ViaInfo` objects; the port compares the 0-based
//!   `via_infos` indexes (one index = one padstack+class+attach row).
//!   The combined fallback rule carries `id: 0` — an unregistered
//!   table entry; `rebuild_via_info` reads only `via_infos`.
//! * **The ripup-costs map is a fresh empty map per engine call** —
//!   Java passes `null` ("costs not needed here"), i.e. no cost
//!   history reaches the resolver; a fresh empty map is that
//!   no-history behavior exactly.
//! * **Durations ride `Instant`/`SystemTime`** where Java uses
//!   `System.currentTimeMillis` — duration/log faces only, no
//!   routing decision consumes them outside the wall-profile
//!   deadline.
//! * **The pass summary row's `%.1f`** renders with the dot decimal
//!   separator; Java's `String.format("%.1f", …)` in the
//!   listener-less pass row is DEFAULT-LOCALE (a decimal-comma
//!   locale would print `33,3` — unrepresentable in the port's
//!   fixed logging locale).
//! * **The escape scan lives here** (Java `BoardStatistics` fanout
//!   block `:442-462` + `isPinEscaped` `:590-608`); the Rust
//!   [`crate::pipeline::board_statistics::BoardStatistics`] face has
//!   no fanout group yet and this module is its only consumer.
//! * **A pin whose package row is missing** renders `null` in the
//!   full name (Java string-concats the null `Pin.name()` as
//!   `"null"`).

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use epic_board::board::Board;
use epic_board::contacts::{item_connected_set, item_normal_contacts, item_unconnected_set};
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::routing_board_insert::opt_changed_area;
use epic_board::rules_surf::ViaRule;
use epic_board::time_limit::TimeLimit;
use epic_board::trace_tightener::{TraceCostFactor, TraceTightenerSeam};
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::item_clearance_violations;
use epic_dsn::state::Unit;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;

use crate::control::AutorouteControl;
use crate::engine::{
    AutorouteAttemptResult, AutorouteAttemptState, RouteBudget, init_autoroute, item_to_string,
};
use crate::pipeline::batch::BatchSettings;
use crate::pipeline::connection_router::{
    SinkBridge, TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP, enforce_strict_drc,
};
use crate::pipeline::event_sink::DriverSink;

// ---------------------------------------------------------------------------
// escape statistics (Java BatchFanout.EscapeStatistics)
// ---------------------------------------------------------------------------

/// Java `BatchFanout.EscapeStatistics` — the escaped-SMD-pin count and
/// its percentage of the net-connected SMD pins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EscapeStatistics {
    pub total_smd_pins: i32,
    pub escaped_count: i32,
    pub escaped_percentage: f64,
}

impl EscapeStatistics {
    /// The interim placeholder (`new EscapeStatistics(total, 0, 0.0)`).
    #[must_use]
    pub fn placeholder(total_smd_pins: i32) -> Self {
        Self {
            total_smd_pins,
            escaped_count: 0,
            escaped_percentage: 0.0,
        }
    }

    /// Java `toString` — `"%d/%d (%.1f%%)"` (dot decimal separator;
    /// see the module banks).
    #[must_use]
    pub fn to_display(&self) -> String {
        format!(
            "{}/{} ({:.1}%)",
            self.escaped_count, self.total_smd_pins, self.escaped_percentage
        )
    }
}

/// Java `BoardStatistics` fanout block (`:442-462`): walk the live SMD
/// pins, count the net-connected ones and the escaped ones
/// ([`is_pin_escaped`]).
#[must_use]
pub fn escape_statistics_from_board(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> EscapeStatistics {
    let smd_pin_ids = board.smd_pin_ids();
    let mut total = 0_i32;
    let mut escaped = 0_i32;
    for pin_id in &smd_pin_ids {
        let Some(entry) = board.get(*pin_id) else {
            continue;
        };
        if entry.nets.is_empty() {
            continue;
        }
        total += 1;
        if is_pin_escaped(manager, board, *pin_id) {
            escaped += 1;
        }
    }
    let escaped_percentage = if total > 0 {
        f64::from(escaped) * 100.0 / f64::from(total)
    } else {
        0.0
    };
    EscapeStatistics {
        total_smd_pins: total,
        escaped_count: escaped,
        escaped_percentage,
    }
}

/// Java `BoardStatistics.isPinEscaped` (`:590-608`): a pin is escaped
/// when it directly touches a violation-free Trace, a violation-free
/// Via whose own normal contacts include a Trace or a ConductionArea,
/// or any ConductionArea.
fn is_pin_escaped(manager: &mut SearchTreeManager, board: &mut Board, pin_id: ItemId) -> bool {
    for contact in item_normal_contacts(manager, board, pin_id) {
        let Some(entry) = board.get(contact) else {
            continue;
        };
        match &entry.data {
            ItemData::Trace { .. } => {
                if item_clearance_violations(manager, board, contact).is_empty() {
                    return true;
                }
            }
            ItemData::Via { .. } => {
                if item_clearance_violations(manager, board, contact).is_empty() {
                    for via_contact in item_normal_contacts(manager, board, contact) {
                        if board.get(via_contact).is_some_and(|via_entry| {
                            matches!(
                                via_entry.data,
                                ItemData::Trace { .. } | ItemData::ConductionArea { .. }
                            )
                        }) {
                            return true;
                        }
                    }
                }
            }
            ItemData::ConductionArea { .. } => return true,
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------------------
// the ordering model (Java BatchFanout.Component / Component.Pin)
// ---------------------------------------------------------------------------

/// Java dispatches `pinSortingOrder` on EXACT string matches, any
/// other value falling through to the `pinIndex` tie-break
/// (`BatchFanout.java:764-792`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PinSortingOrder {
    InnerFirst,
    OuterFirst,
    DistanceToClosestOnNet,
    SurroundingsDensity,
    Other,
}

impl PinSortingOrder {
    fn parse(raw: &str) -> Self {
        match raw {
            "inner_first" => Self::InnerFirst,
            "outer_first" => Self::OuterFirst,
            "distanceToClosestOnNet" => Self::DistanceToClosestOnNet,
            "surroundingsDensity" => Self::SurroundingsDensity,
            _ => Self::Other,
        }
    }
}

/// Java's double sign test (`delta > 0 → +1, delta < 0 → −1, else 0`)
/// — NaN folds to Equal exactly as in Java (both comparisons false).
fn cmp_sign_f64(a: f64, b: f64) -> Ordering {
    let delta = a - b;
    if delta > 0.0 {
        Ordering::Greater
    } else if delta < 0.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// Java `BatchFanout.Component.Pin` — one netted SMD pin with its
/// ctor-computed sort keys.
#[derive(Clone, Debug)]
pub(crate) struct SmdPinRow {
    pub item_id: ItemId,
    /// Java `boardPin.pinIndex` — the tie-break (unique within a
    /// component).
    pub pin_index: i32,
    /// Java `component.name + "-" + pin.name()` precomputed
    /// (`fullPinName`, `BatchFanout.java:233-234`).
    pub full_name: String,
    /// Java `boardPin.getNetNumber(0)` — the pins carry exactly one
    /// net (the ctor's net filter).
    pub net_number: i32,
    pub distance_to_component_center: f64,
    /// Java `Double.MAX_VALUE` when the pin has no net (unreachable —
    /// the rows are netted) or no other pin shares the net.
    pub distance_to_closest_on_net: f64,
    pub surroundings_density: i32,
}

impl SmdPinRow {
    /// Java `Component.Pin.compareTo` (`:762-797`): the sorting-order
    /// arm, then the `pinIndex` tie-break.
    fn compare(&self, other: &Self, sorting_order: PinSortingOrder) -> Ordering {
        let mut result = match sorting_order {
            PinSortingOrder::InnerFirst => cmp_sign_f64(
                self.distance_to_component_center,
                other.distance_to_component_center,
            ),
            PinSortingOrder::OuterFirst => cmp_sign_f64(
                other.distance_to_component_center,
                self.distance_to_component_center,
            ),
            PinSortingOrder::DistanceToClosestOnNet => cmp_sign_f64(
                self.distance_to_closest_on_net,
                other.distance_to_closest_on_net,
            ),
            PinSortingOrder::SurroundingsDensity => {
                // `delta = other - this` — densest first.
                other.surroundings_density.cmp(&self.surroundings_density)
            }
            PinSortingOrder::Other => Ordering::Equal,
        };
        if result == Ordering::Equal {
            result = self.pin_index.cmp(&other.pin_index);
        }
        result
    }
}

/// Java `BatchFanout.Component` — one component's netted SMD pins,
/// pin-sorted, with the gravity center of the pin set.
#[derive(Clone, Debug)]
pub(crate) struct ComponentFanout {
    pub component_id: i32,
    pub smd_pin_count: i32,
    /// Java `smdPins` TreeSet — the same total order as this sorted
    /// vector (the comparator keys are unique: `pinIndex` within a
    /// component).
    pub pins: Vec<SmdPinRow>,
}

/// Java `BatchFanout` ctor ordering: `smdPinCount` DESCENDING, tie
/// `component.id` ASCENDING (`:702-713`).
fn compare_components(a: &ComponentFanout, b: &ComponentFanout) -> Ordering {
    b.smd_pin_count
        .cmp(&a.smd_pin_count)
        .then(a.component_id.cmp(&b.component_id))
}

// ---------------------------------------------------------------------------
// the board faces the ctor needs
// ---------------------------------------------------------------------------

/// Java `routingBoard.getPins()` — the live pins, itemList order
/// (= ascending id; the engine.rs insertion-order convention).
fn all_pin_ids(board: &Board) -> Vec<ItemId> {
    board
        .iter_ascending()
        .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Pin { .. }))
        .map(|entry| entry.id)
        .collect()
}

/// Java `board.getVias().size()` — the live via count.
fn via_count(board: &Board) -> i32 {
    board
        .iter_ascending()
        .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Via { .. }))
        .count() as i32
}

/// The board resolution in UM (Java
/// `communication.getResolution(Unit.UM)`), the same face the maze
/// fanout gate reads.
fn resolution_um(board: &Board) -> f64 {
    let comm = board.communication();
    Unit::scale(f64::from(comm.resolution), Unit::Um, comm.unit)
}

// ---------------------------------------------------------------------------
// the BatchFanout instance state
// ---------------------------------------------------------------------------

/// Java `BatchFanout` — the ctor-built ordering plus the loop-carried
/// counters.
pub struct FanoutState {
    pub(crate) components: Vec<ComponentFanout>,
    /// Java `totalSmdPinCount` — the NETTED SMD pin count.
    pub total_smd_pin_count: i32,
    /// Java `alreadyConnectedPinCount`.
    pub already_connected_pin_count: i32,
    /// Java `totalItemsFanouted` — ROUTED + FAILED + INSERT_ERROR pins
    /// across passes.
    pub total_items_fanouted: i32,
    /// Java `extraViasTotal` — accumulated only on a pass that runs to
    /// its end (the timeout/thread-stop early returns skip the
    /// accumulation, exactly as in Java).
    pub extra_vias_total: i32,
    /// Java `lastNotRoutedCount` — set only at a pass end (the early
    /// returns skip it).
    pub last_not_routed_count: i32,
    /// Java `isTimedOut`.
    pub is_timed_out: bool,
    /// Java `deadlineMs` — wall-profile only (see the module banks).
    pub(crate) deadline_ms: Option<i64>,
    /// Java `failedPinGeneration` (#933, upstream 339e8bb50) — pin item
    /// id to the component generation observed when that pin last
    /// failed (recorded in the FAILED and INSERT_ERROR arms). Lives for
    /// the whole batch fanout, across passes, exactly the Java field.
    pub(crate) failed_pin_generation: HashMap<ItemId, i32>,
    /// Java `componentGeneration` (#933) — successful escapes per
    /// component id, bumped only on ROUTED. A change allows a
    /// previously failed pin to be retried.
    pub(crate) component_generation: HashMap<i32, i32>,
}

/// M6-T8 ordering face / M6-T9 bank (T8 quality Q6): the
/// congestion-rank re-sort of a component's pin rows — STABLE BY
/// CONTRACT: equal ranks (including the `usize::MAX` absent-plan tie)
/// keep the Java order the rows already carry from the
/// `sort_by(compare)` pass at the call site; the rank alone reorders.
/// Pinned by `fanout_congestion_rank_sort_keeps_java_order_on_ties`
/// (`global/tests.rs`) with a past-the-insertion-sort-floor row count,
/// where an `sort_unstable_by_key` mutant genuinely reorders ties.
pub(crate) fn sort_by_congestion_rank<T, F>(rows: &mut [T], rank_of: F)
where
    F: Fn(&T) -> usize,
{
    rows.sort_by_key(rank_of);
}

impl FanoutState {
    /// Java `BatchFanout(board, settings, thread)` ctor (`:35-78`):
    /// the sorting order, the netted SMD pin rows, the component
    /// ordering and the already-connected count.
    #[must_use]
    pub fn new(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        settings: &BatchSettings,
    ) -> FanoutState {
        let fanout = &settings.router_settings.fanout;
        let sorting_order = PinSortingOrder::parse(&fanout.pin_sorting_order);

        // M6-T8 (design :69): the congestion-aware pin order — the map
        // feeds fanout's pin order when the NEGOTIATED stage is on
        // (`congestion_global` + `congestion_global_pathfinder`,
        // default OFF). Gating note: the plan row says the order "rides
        // this setting"; it rides the negotiated STAGE's flag, not the
        // master alone — a master-only gate would rotate the committed
        // T7 master-face golden (`t7_ripup.global-golden.json`,
        // immutable), which is the binding constraint. A pin's PRIMARY
        // key is its net's planned rank (most-congested guide first,
        // the GlobalPlan order); ties keep the Java order below (the
        // sort is stable over the already-Java-sorted rows). This is
        // the adjacent-QFP mutual-blocking fix: pins of contested nets
        // get their escapes BEFORE the easy nets monopolize the
        // channels.
        let congestion_plan = if settings.congestion_global && settings.congestion_global_pathfinder
        {
            Some(crate::global::plan::GlobalPlan::build(board))
        } else {
            None
        };

        // Java getSmdPins() then the netCount() > 0 filter — both in
        // itemList order (= ascending id).
        let netted_pin_ids: Vec<ItemId> = board
            .smd_pin_ids()
            .into_iter()
            .filter(|&id| board.get(id).is_some_and(|entry| !entry.nets.is_empty()))
            .collect();

        let component_count = board.components().count();
        let mut components: Vec<ComponentFanout> = Vec::new();
        for component_no in 1..=component_count {
            let component = board
                .components()
                .get(component_no)
                .unwrap_or_else(|| panic!("component ids are 1..=count ({component_no})"));
            let component_id = i32::try_from(component_no).expect("component id fits i32");
            // Java filters the netted list by componentId — preserving
            // the list order (the gravity-center f64 sum order).
            let component_pin_ids: Vec<ItemId> = netted_pin_ids
                .iter()
                .copied()
                .filter(|&id| {
                    board
                        .get(id)
                        .is_some_and(|entry| entry.component_id == component_id)
                })
                .collect();
            if component_pin_ids.is_empty() {
                continue;
            }
            // The gravity center of the pin set (mean of the centers;
            // Java `:670-690` — the (0, 0) arm is unreachable here
            // because the list is non-empty).
            let mut gravity_x = 0.0_f64;
            let mut gravity_y = 0.0_f64;
            for &pin_id in &component_pin_ids {
                let center = board
                    .pin_center(pin_id)
                    .expect("a live SMD pin carries a center")
                    .to_float();
                gravity_x += center.x;
                gravity_y += center.y;
            }
            let pin_count = i32::try_from(component_pin_ids.len()).expect("pin count fits i32");
            gravity_x /= f64::from(pin_count);
            gravity_y /= f64::from(pin_count);
            let gravity_center = FloatPoint::new(gravity_x, gravity_y);

            // The per-pin sort keys (Java `Pin` ctor `:722-759`).
            let resolution = resolution_um(board);
            let max_density_dist = 20_000.0 * resolution;
            let board_pins = all_pin_ids(board);
            let mut pins: Vec<SmdPinRow> = component_pin_ids
                .iter()
                .map(|&pin_id| {
                    let entry = board.get(pin_id).expect("live pin");
                    let ItemData::Pin { pin_index, .. } = &entry.data else {
                        panic!("row is a pin");
                    };
                    let net_number = entry.nets[0];
                    let pin_location = board
                        .pin_center(pin_id)
                        .expect("a live SMD pin carries a center")
                        .to_float();
                    let distance_to_component_center = pin_location.distance(&gravity_center);
                    // distanceToClosestOnNet: the nearest OTHER live pin
                    // sharing the net (ALL board pins, not just SMD).
                    let mut min_distance = f64::MAX;
                    if net_number > 0 {
                        for &other_id in &board_pins {
                            if other_id == pin_id {
                                continue;
                            }
                            let shares_net = board
                                .get(other_id)
                                .is_some_and(|other| other.nets.contains(&net_number));
                            if !shares_net {
                                continue;
                            }
                            let other_center = board
                                .pin_center(other_id)
                                .expect("a live pin carries a center")
                                .to_float();
                            let dist = pin_location.distance(&other_center);
                            if dist < min_distance {
                                min_distance = dist;
                            }
                        }
                    }
                    // surroundingsDensity: netted SMD pins within
                    // 20 mm (coordinate units) of the pin.
                    let mut density = 0_i32;
                    for &other_id in &netted_pin_ids {
                        if other_id == pin_id {
                            continue;
                        }
                        let other_center = board
                            .pin_center(other_id)
                            .expect("a live SMD pin carries a center")
                            .to_float();
                        if pin_location.distance(&other_center) <= max_density_dist {
                            density += 1;
                        }
                    }
                    // Java `component.name + "-" + pin.name()`; a
                    // missing package row stringifies as "null".
                    let package_pin_name = board
                        .components()
                        .get(component_no)
                        .and_then(|component| board.library().package(component.package_no()))
                        .and_then(|package| package.get_pin(*pin_index))
                        .map(|package_pin| package_pin.name.clone())
                        .unwrap_or_else(|| "null".to_string());
                    SmdPinRow {
                        item_id: pin_id,
                        pin_index: *pin_index,
                        full_name: format!("{}-{}", component.name, package_pin_name),
                        net_number,
                        distance_to_component_center,
                        distance_to_closest_on_net: min_distance,
                        surroundings_density: density,
                    }
                })
                .collect();
            pins.sort_by(|a, b| a.compare(b, sorting_order));
            if let Some(plan) = &congestion_plan {
                sort_by_congestion_rank(&mut pins, |row| {
                    plan.rank_of(row.net_number).unwrap_or(usize::MAX)
                });
            }
            components.push(ComponentFanout {
                component_id,
                smd_pin_count: pin_count,
                pins,
            });
        }
        components.sort_by(compare_components);

        // The already-connected count (Java `:62-75`): a netted SMD
        // pin with an empty unconnected set.
        let mut already_connected = 0_i32;
        let mut total = 0_i32;
        for component in &components {
            total += component.smd_pin_count;
            for pin in &component.pins {
                if item_unconnected_set(manager, board, pin.item_id, pin.net_number).is_empty() {
                    already_connected += 1;
                }
            }
        }
        FanoutState {
            components,
            total_smd_pin_count: total,
            already_connected_pin_count: already_connected,
            total_items_fanouted: 0,
            extra_vias_total: 0,
            last_not_routed_count: 0,
            is_timed_out: false,
            deadline_ms: None,
            failed_pin_generation: HashMap::new(),
            component_generation: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// the progress listener (Java FanoutProgressListener / FanoutPassStatus)
// ---------------------------------------------------------------------------

/// Java `FanoutPassStatus` — the projection carries the consumed faces
/// only (see the module banks).
#[derive(Clone, Debug)]
pub struct FanoutPassStatus {
    /// Java `passNo + 1` — 1-BASED.
    pub pass_no: i32,
    /// Java `ripupCosts` — the RAW pass costs (not the effective
    /// no-ripup `-1`).
    pub ripup_costs: i32,
    pub total_pins: i32,
    pub pins_to_go: i32,
    pub routed_count: i32,
    pub not_routed_count: i32,
    pub insert_error_count: i32,
    pub extra_vias_this_pass: i32,
    pub extra_vias_total: i32,
    pub pass_duration_millis: i64,
    /// Java: the listener reads `board.getHash()` itself.
    pub board_hash: String,
    /// Java `boardStatistics().connections.incompleteCount`.
    pub incomplete_count: i32,
    pub pass_completed: bool,
    pub escape_statistics: EscapeStatistics,
}

/// Java `FanoutProgressListener`. The port threads the driver sink
/// through the callback: Java splits the logging faces (BatchFanout's
/// own rows ride the global `FRLogger`, the listener's ride `job`), a
/// split a `&mut` sink cannot serve twice — the port's
/// [`DriverSink`](crate::pipeline::event_sink::DriverSink) is the
/// unified backend (info = `job.logInfo` / `FRLogger.info`).
pub trait FanoutProgressListener {
    /// M9-T3 note: the `board` parameter was ADDED for the snapshot
    /// hook — the board is in scope in [`publish_progress`] one frame
    /// up, not in the old `on_progress` body, so it is threaded in
    /// here. The single implementor ([`crate::pipeline::batch::BatchFanoutListener`])
    /// only forwards it to [`DriverSink::board_snapshot`] (the default
    /// no-op for every pre-existing sink); no parity row changes.
    fn on_progress(&mut self, status: &FanoutPassStatus, board: &Board, sink: &mut dyn DriverSink);
}

/// Java `FanoutRunSummary`.
#[derive(Clone, Debug)]
pub struct FanoutRunSummary {
    pub completed_pass_count: i32,
    pub total_duration_millis: i64,
    pub escape_statistics: EscapeStatistics,
    pub is_timed_out: bool,
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| i64::try_from(delta.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Java `TextManager.parseTimespanString` (`:83-113`) — the
/// `Duration.parse` delegation banked to a direct split (see the
/// module banks). JDK-verified faces (2026-09-23 single-file probe
/// against the oracle JDK): `String.split` drops TRAILING empty parts
/// (`"1:2:"` → `PT1M2S` = 62; `":"` → no parts → null) while a MIDDLE
/// empty fails the parse (`"1::30"` → null); signs are legal PER
/// COMPONENT (`"PT-5S"` → −5, `"PT1M-2S"` → 58) so each part is a
/// signed i64; the split runs on the RAW string (java.time rejects
/// whitespace: `"1:30 "` → null); overflow throws (huge digits →
/// null, mirroring the i64 parse failure).
#[must_use]
pub fn parse_timespan_string(value: &str) -> Option<i64> {
    if value.trim().is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = value.split(':').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut seconds = [0_i64; 3];
    for (index, part) in parts.iter().enumerate() {
        seconds[index] = part.parse::<i64>().ok()?;
    }
    Some(match parts.len() {
        3 => seconds[0].wrapping_mul(3600) + seconds[1].wrapping_mul(60) + seconds[2],
        2 => seconds[0].wrapping_mul(60) + seconds[1],
        _ => seconds[0],
    })
}

/// Java `publishProgress` (`:564-596`): the listener publish with the
/// duration and the fresh hash/incomplete faces. The stats
/// computations are listener-gated (read-only; see the module banks).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn publish_progress(
    state: &FanoutState,
    listener: &mut Option<&mut dyn FanoutProgressListener>,
    sink: &mut dyn DriverSink,
    pass_no: i32,
    ripup_costs: i32,
    pins_to_go: i32,
    routed_count: i32,
    not_routed_count: i32,
    insert_error_count: i32,
    extra_vias_this_pass: i32,
    escape_statistics: EscapeStatistics,
    pass_completed: bool,
    pass_start_millis: i64,
    manager: &mut SearchTreeManager,
    board: &mut Board,
) {
    let Some(listener) = listener.as_deref_mut() else {
        return;
    };
    let duration = (now_millis() - pass_start_millis).max(0);
    let incomplete_count = crate::pipeline::board_statistics::BoardStatistics::new(manager, board)
        .connections
        .incomplete_count
        .unwrap_or(0);
    listener.on_progress(
        &FanoutPassStatus {
            pass_no: pass_no + 1,
            ripup_costs,
            total_pins: state.total_smd_pin_count,
            pins_to_go,
            routed_count,
            not_routed_count,
            insert_error_count,
            extra_vias_this_pass,
            extra_vias_total: state.extra_vias_total + extra_vias_this_pass,
            pass_duration_millis: duration,
            board_hash: crate::pipeline::board_hash::board_hash(board),
            incomplete_count,
            pass_completed,
            escape_statistics,
        },
        board,
        sink,
    );
}

// ---------------------------------------------------------------------------
// the stage loop (Java fanoutBoard / fanoutPass)
// ---------------------------------------------------------------------------

/// Java `AutorouteBatchLoop.java:113-129` — the stage-start walk: a
/// net-connected SMD pin counts, and an empty unconnected set counts it
/// as already connected. Returns `(net_connected_smd_pins,
/// already_connected)`.
pub(crate) fn smd_pin_connection_counts(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> (i32, i32) {
    let mut net_connected = 0_i32;
    let mut already_connected = 0_i32;
    for pin_id in board.smd_pin_ids() {
        let Some(entry) = board.get(pin_id) else {
            continue;
        };
        if entry.nets.is_empty() {
            continue;
        }
        net_connected += 1;
        if item_unconnected_set(manager, board, pin_id, entry.nets[0]).is_empty() {
            already_connected += 1;
        }
    }
    (net_connected, already_connected)
}

/// Java `BatchFanout.java:128-148` — one step of the oscillation
/// detector, extracted for exact ±1 pinning (cerebrum 16): the pass
/// state is `(routedCount << 32) ^ viaCount`; a REPEAT of the previous
/// state bumps the identical-run counter, any different state resets
/// it to 0 and becomes the new previous state. Returns the updated
/// `(identical_passes, previous_board_state, stop)`; `stop` is the
/// `identical_passes >= stagnation_pass_limit` face — with the Java
/// limit 3, the THIRD consecutive REPEAT stops (2 repeats continue).
fn oscillation_step(
    routed_count: i32,
    via_total: i32,
    previous_board_state: i64,
    identical_passes: i32,
    stagnation_pass_limit: i32,
) -> (i32, i64, bool) {
    let board_state = (i64::from(routed_count) << 32) ^ i64::from(via_total);
    if board_state == previous_board_state {
        let identical_passes = identical_passes + 1;
        (
            identical_passes,
            previous_board_state,
            identical_passes >= stagnation_pass_limit,
        )
    } else {
        (0, board_state, false)
    }
}

/// Java `BatchFanout.stagnationPassLimit` (`:105`) — the oscillation
/// breaker arms at the THIRD consecutive identical-state pass.
const STAGNATION_PASS_LIMIT: i32 = 3;

/// Java `BatchFanout.fanoutBoard` (`:81-163`) — the pass loop with the
/// deadline, max-items, routed-count, oscillation and board-hash
/// breakers.
pub fn fanout_board(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    stoppable_flag: Option<&Arc<AtomicBool>>,
    mut listener: Option<&mut dyn FanoutProgressListener>,
    sink: &mut dyn DriverSink,
) -> FanoutRunSummary {
    #[allow(clippy::type_complexity)] // the Java listener type, re-borrowed per publish
    let listener: &mut Option<&mut dyn FanoutProgressListener> = &mut listener;
    let fanout = &settings.router_settings.fanout;
    let mut state = FanoutState::new(manager, board, settings);
    let fanout_start_millis = now_millis();
    // The stage deadline is wall-profile-only (see the module banks).
    if !settings.deterministic_budgets
        && let Some(timeout_string) = &fanout.timeout_string
        && let Some(timeout_seconds) = parse_timespan_string(timeout_string)
    {
        state.deadline_ms = Some(fanout_start_millis + timeout_seconds * 1000);
    }
    let max_passes = fanout.max_passes;
    let stagnation_pass_limit = STAGNATION_PASS_LIMIT;
    let mut completed_passes = 0_i32;
    let mut previous_board_state = i64::MIN;
    let mut identical_passes = 0_i32;
    let mut last_board_hash = crate::pipeline::board_hash::board_hash(board);
    for pass_no in 0..max_passes {
        if let Some(deadline) = state.deadline_ms
            && now_millis() >= deadline
        {
            state.is_timed_out = true;
            sink.info(&format!(
                "Fanout stage timed out before starting pass #{}",
                pass_no + 1
            ));
            break;
        }
        if fanout.max_items > 0 && state.total_items_fanouted >= fanout.max_items {
            break;
        }
        let routed_count = fanout_pass(
            &mut state,
            manager,
            board,
            settings,
            pass_no,
            listener,
            stoppable_flag,
            sink,
        );
        completed_passes += 1;
        if routed_count == 0 {
            break;
        }
        // The oscillation detector (`:128-148`) — the pure step above;
        // the stop carries Java's info row verbatim.
        let (new_identical, new_previous, stop) = oscillation_step(
            routed_count,
            via_count(board),
            previous_board_state,
            identical_passes,
            stagnation_pass_limit,
        );
        identical_passes = new_identical;
        previous_board_state = new_previous;
        if stop {
            sink.info(&format!(
                "Fanout stopped after {completed_passes} passes: no progress for \
                 {stagnation_pass_limit} consecutive passes."
            ));
            break;
        }
        if state.is_timed_out {
            break;
        }
        let current_board_hash = crate::pipeline::board_hash::board_hash(board);
        if current_board_hash == last_board_hash {
            break;
        }
        last_board_hash = current_board_hash;
    }
    let escape_statistics = escape_statistics_from_board(manager, board);
    let total_duration_millis = (now_millis() - fanout_start_millis).max(0);
    FanoutRunSummary {
        completed_pass_count: completed_passes,
        total_duration_millis,
        escape_statistics,
        is_timed_out: state.is_timed_out,
    }
}

/// Java `BatchFanout.fanoutPass` (`:166-526`) — one pass over the
/// sorted SMD pins; returns the newly fanouted pin count.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
#[allow(clippy::too_many_lines)] // the Java body is one flat walk
fn fanout_pass(
    state: &mut FanoutState,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    pass_no: i32,
    listener: &mut Option<&mut dyn FanoutProgressListener>,
    stoppable_flag: Option<&Arc<AtomicBool>>,
    sink: &mut dyn DriverSink,
) -> i32 {
    let fanout = &settings.router_settings.fanout;
    let pass_start_millis = now_millis();
    let mut pins_to_go = state.total_smd_pin_count;
    let mut routed_count = 0_i32;
    let mut not_routed_count = 0_i32;
    let mut insert_error_count = 0_i32;
    let mut already_connected_count = 0_i32;
    let vias_before_pass = via_count(board);
    // Java `getStartRipupCosts() * (passNo + 1)` — int arithmetic.
    let ripup_costs = settings.start_ripup_costs.wrapping_mul(pass_no + 1);
    let base_millis_per_pin = fanout.max_milliseconds_per_pin;
    let ripup_allowed = fanout.ripup_allowed;
    // Negative ripup costs signal "no ripup" to fanout_pin.
    let effective_ripup_costs = if ripup_allowed { ripup_costs } else { -1 };

    sink.trace(&format!(
        "pass_start pass={}, totalPins={}, alreadyConnected={}, pinsToFanout={}, \
         ripupCosts={effective_ripup_costs}, baseMillisPerPin={base_millis_per_pin}",
        pass_no + 1,
        state.total_smd_pin_count,
        state.already_connected_pin_count,
        state.total_smd_pin_count - state.already_connected_pin_count,
    ));

    // The unconditional pass-start publish (the placeholder escape
    // stats; the stats scan is listener-gated — read-only either way).
    publish_progress(
        state,
        listener,
        sink,
        pass_no,
        ripup_costs,
        pins_to_go,
        routed_count,
        not_routed_count,
        insert_error_count,
        0,
        EscapeStatistics::placeholder(state.total_smd_pin_count),
        false,
        pass_start_millis,
        manager,
        board,
    );

    let mut max_limit_reached = false;
    // Disjoint field borrows: the component rows stay shared while the
    // loop-carried counters mutate.
    let state = &mut *state;
    let components = &state.components;
    'component_loop: for component in components {
        for pin in &component.pins {
            if fanout.max_items > 0 && state.total_items_fanouted >= fanout.max_items {
                sink.info(&format!(
                    "Max items limit reached ({}). Stopping fanout.",
                    fanout.max_items
                ));
                max_limit_reached = true;
                break;
            }
            let net_number = pin.net_number;
            let target_count = item_unconnected_set(manager, board, pin.item_id, net_number).len();

            // The via gate (`:238-259`): the RAW net-class rule, the
            // board-first-rule fallback.
            let rules = board.rules();
            let can_use_vias = if let Some(net) = rules.nets.get(net_number) {
                let net_class = rules.net_class(net.net_class);
                let via_rule = net_class
                    .and_then(|class| class.via_rule)
                    .and_then(|rule_id| rules.via_rule_by_id(rule_id));
                let has_board_vias = rules
                    .via_rules
                    .first()
                    .is_some_and(|rule| !rule.via_infos.is_empty());
                let fallback_allowed = fanout.fallback_to_board_vias && has_board_vias;
                via_rule.is_some_and(|rule| !rule.via_infos.is_empty()) || fallback_allowed
            } else {
                // Java: a null net skips the gate entirely.
                true
            };
            if !can_use_vias {
                sink.debug(&format!(
                    "BatchFanout: skipping pin {} because its net class has no vias defined \
                     and fallback is disabled/unavailable.",
                    pin.full_name
                ));
                pins_to_go -= 1;
                continue;
            }

            // Java `:269-274` (#933, upstream 339e8bb50): a pin that
            // failed is retried only after its OWN component escaped
            // another pin since — silently skipped otherwise (no trace
            // row, exactly the Java face).
            if !retry_fanout(
                state.failed_pin_generation.get(&pin.item_id).copied(),
                state
                    .component_generation
                    .get(&component.component_id)
                    .copied()
                    .unwrap_or(0),
            ) {
                pins_to_go -= 1;
                continue;
            }

            sink.trace(&format!(
                "pin_start pin={}, net={net_number}, targetCount={target_count}, pass={}",
                pin.full_name,
                pass_no + 1
            ));

            let max_item_id_before_fanout = board.max_generated_id();
            // Java `baseMillisPerPin * (passNo + 1)` is a LONG multiply
            // widened to double, then `(int)`-truncated.
            let max_millis_i64 = base_millis_per_pin.wrapping_mul(i64::from(pass_no + 1));
            let limit_i32 = (max_millis_i64 as f64) as i32; // Java (int) double: saturating, NaN → 0
            let budget = if settings.deterministic_budgets {
                RouteBudget::Deterministic {
                    limit: u64::try_from(i64::from(limit_i32)).unwrap_or(0),
                    spent: Cell::new(0),
                }
            } else {
                RouteBudget::Wall(TimeLimit::new(i64::from(limit_i32)))
            };
            let mut current_result = fanout_pin(
                manager,
                board,
                settings,
                pin.item_id,
                effective_ripup_costs,
                stoppable_flag,
                budget,
                sink,
            );

            if current_result.state == AutorouteAttemptState::Routed
                && let Some(rejection) =
                    enforce_strict_drc(manager, board, net_number, max_item_id_before_fanout)
            {
                sink.trace(&format!(
                    "fanout_via_reverted pin={}, net={net_number}, reason={}",
                    pin.full_name, rejection.details
                ));
                current_result = rejection;
            }

            match current_result.state {
                AutorouteAttemptState::Routed => {
                    routed_count += 1;
                    state.total_items_fanouted += 1;
                    // Java `componentGeneration.merge(id, 1, sum)` (#933)
                    // — the only bump site (successful escapes only).
                    *state
                        .component_generation
                        .entry(component.component_id)
                        .or_insert(0) += 1;
                    sink.trace(&format!(
                        "pin_routed pin={}, net={net_number}, targetCount={target_count}",
                        pin.full_name
                    ));
                }
                AutorouteAttemptState::AlreadyConnected => {
                    already_connected_count += 1;
                    sink.trace(&format!(
                        "pin_already_connected pin={}, net={net_number}, \
                         targetCount={target_count}, detail={}",
                        pin.full_name, current_result.details
                    ));
                }
                AutorouteAttemptState::Failed => {
                    not_routed_count += 1;
                    state.total_items_fanouted += 1;
                    // Java `failedPinGeneration.put(pinId, gen)` (#933) —
                    // the component's generation AT FAILURE TIME.
                    state.failed_pin_generation.insert(
                        pin.item_id,
                        state
                            .component_generation
                            .get(&component.component_id)
                            .copied()
                            .unwrap_or(0),
                    );
                    let detail = if current_result.details.is_empty() {
                        "no detail"
                    } else {
                        current_result.details.as_str()
                    };
                    sink.trace(&format!(
                        "pin_failed pin={}, net={net_number}, targetCount={target_count}, \
                         detail={detail}",
                        pin.full_name
                    ));
                }
                AutorouteAttemptState::InsertError => {
                    insert_error_count += 1;
                    state.total_items_fanouted += 1;
                    // Java `failedPinGeneration.put(pinId, gen)` (#933) —
                    // INSERT_ERROR failures gate retries the same way.
                    state.failed_pin_generation.insert(
                        pin.item_id,
                        state
                            .component_generation
                            .get(&component.component_id)
                            .copied()
                            .unwrap_or(0),
                    );
                    let detail = if current_result.details.is_empty() {
                        "no detail"
                    } else {
                        current_result.details.as_str()
                    };
                    sink.trace(&format!(
                        "pin_insert_error pin={}, net={net_number}, detail={detail}",
                        pin.full_name
                    ));
                }
                AutorouteAttemptState::NoUnconnectedNets => {
                    sink.trace(&format!(
                        "pin_no_unconnected_nets pin={}, net={net_number}, detail={}",
                        pin.full_name, current_result.details
                    ));
                }
                other => {
                    sink.trace(&format!(
                        "pin_other_state pin={}, net={net_number}, state={}, detail={}",
                        pin.full_name,
                        other.as_str(),
                        current_result.details
                    ));
                }
            }
            pins_to_go -= 1;
            let extra_vias_this_pass = (via_count(board) - vias_before_pass).max(0);
            // Java's throttled mid-pass publish (passCompleted=false)
            // is banked — no port call site.

            if let Some(deadline) = state.deadline_ms
                && now_millis() >= deadline
            {
                sink.info("Fanout stage timed out.");
                state.is_timed_out = true;
                let escape_statistics = escape_statistics_from_board(manager, board);
                publish_progress(
                    state,
                    listener,
                    sink,
                    pass_no,
                    ripup_costs,
                    pins_to_go,
                    routed_count,
                    not_routed_count,
                    insert_error_count,
                    extra_vias_this_pass,
                    escape_statistics,
                    true,
                    pass_start_millis,
                    manager,
                    board,
                );
                return routed_count;
            }
            if stoppable_flag.is_some_and(|flag| flag.load(AtomicOrdering::Relaxed)) {
                let escape_statistics = escape_statistics_from_board(manager, board);
                publish_progress(
                    state,
                    listener,
                    sink,
                    pass_no,
                    ripup_costs,
                    pins_to_go,
                    routed_count,
                    not_routed_count,
                    insert_error_count,
                    extra_vias_this_pass,
                    escape_statistics,
                    true,
                    pass_start_millis,
                    manager,
                    board,
                );
                return routed_count;
            }
        }
        if max_limit_reached {
            break 'component_loop;
        }
    }
    let extra_vias_this_pass = (via_count(board) - vias_before_pass).max(0);
    state.extra_vias_total += extra_vias_this_pass;
    let escape_statistics = escape_statistics_from_board(manager, board);
    // Java `pass_end` (`:465-488`) — the 5-arg trace row folded.
    sink.trace(&format!(
        "pass_end pass={}, durationMs={}, routed={routed_count}, notRouted={not_routed_count}, \
         insertErrors={insert_error_count}, alreadyConnected={already_connected_count}, \
         escaped={}",
        pass_no + 1,
        (now_millis() - pass_start_millis).max(0),
        escape_statistics.to_display(),
    ));
    if listener.is_none() {
        sink.info(&format!(
            "fanout pass: {}, routed: {routed_count}, not routed: {not_routed_count}, \
             errors: {insert_error_count}, extra vias: +{extra_vias_this_pass}, \
             escaped SMD pins: {}",
            pass_no + 1,
            escape_statistics.to_display(),
        ));
    }
    state.last_not_routed_count = not_routed_count;
    publish_progress(
        state,
        listener,
        sink,
        pass_no,
        ripup_costs,
        pins_to_go,
        routed_count,
        not_routed_count,
        insert_error_count,
        extra_vias_this_pass,
        escape_statistics,
        true,
        pass_start_millis,
        manager,
        board,
    );
    routed_count
}

/// Java `BatchFanout.retryFanout` (`:550-557`, #933 upstream 339e8bb50):
/// a pin is retried when it has not failed, or when its component has
/// escaped another pin since that failure. One thread and many threads
/// take the same decision because the generation is updated only after
/// a successful escape on the calling thread.
fn retry_fanout(failed_generation: Option<i32>, current_generation: i32) -> bool {
    match failed_generation {
        Some(failed) => failed != current_generation,
        None => true,
    }
}

// ---------------------------------------------------------------------------
// the per-pin escape attempt (Java RoutingBoard.fanout)
// ---------------------------------------------------------------------------

/// Java `Item.boundingBox` for the target-sort distance: DrillItem
/// (Pin, Via) unions its tile shapes; PolylineTrace is the polyline
/// box offset by the half width; the area kinds answer their border
/// box (the holes are inside it — Java `getArea().boundingBox()`).
fn item_bounding_box(board: &Board, id: ItemId) -> IntBox {
    let entry = board.get(id).expect("a fanout target is a live item");
    match &entry.data {
        ItemData::Pin { .. } | ItemData::Via { .. } => {
            // Java DrillItem.boundingBox: the union over every tile
            // shape of the drill span.
            let shape_count = board
                .drill_tile_shape_count(id)
                .expect("a live drill item carries its tile shape count");
            let mut result = board
                .drill_shape(id, 0)
                .expect("a live drill item carries its first shape")
                .bounding_box();
            for index in 1..shape_count {
                let shape = board
                    .drill_shape(id, index)
                    .expect("a live drill item carries every shape of its span");
                result = result.union(&shape.bounding_box());
            }
            result
        }
        ItemData::Trace {
            lines, half_width, ..
        } => epic_board::items::trace::bounding_box(lines, *half_width),
        ItemData::ConductionArea { area, .. } | ItemData::ObstacleArea { area, .. } => {
            area.border.bounding_box()
        }
        other => panic!("unexpected fanout target kind: {other:?}"),
    }
}

/// The target-sort key: the squared distance from the pin center to
/// the item's bounding-box center (`RoutingBoard.java:1004-1020`).
fn bounding_box_center_dist_sq(board: &Board, id: ItemId, pin_center: &FloatPoint) -> f64 {
    let bounds = item_bounding_box(board, id);
    let center_x = f64::from(bounds.ll.x.wrapping_add(bounds.ur.x)) / 2.0;
    let center_y = f64::from(bounds.ll.y.wrapping_add(bounds.ur.y)) / 2.0;
    let dx = center_x - pin_center.x;
    let dy = center_y - pin_center.y;
    dx * dx + dy * dy
}

/// Java `RoutingBoard.fanout` (`:978-1110`) — one SMD pin's escape
/// attempt: the single-layer/single-net guard, the same-layer
/// connected-set guard, the closest-first target sort, the combined
/// fallback via rule, and the ≤4-targets two-stage (or whole-set)
/// engine calls, closed by the pull-tight over the pin's net.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
#[allow(clippy::too_many_lines)] // the Java body is one flat walk
pub fn fanout_pin(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    pin_id: ItemId,
    ripup_costs: i32,
    stoppable_flag: Option<&Arc<AtomicBool>>,
    budget: RouteBudget,
    sink: &mut dyn DriverSink,
) -> AutorouteAttemptResult {
    let already_connected = |board: &Board| {
        AutorouteAttemptResult::with_details(
            AutorouteAttemptState::AlreadyConnected,
            format!(
                "The pin '{}' is already connected.",
                item_to_string(board, u64::from(pin_id.get()))
            ),
        )
    };
    let Some(entry) = board.get(pin_id) else {
        panic!("fanout target pin is live");
    };
    assert!(
        matches!(entry.data, ItemData::Pin { .. }),
        "fanout target is a pin"
    );
    let net_count = entry.nets.len();
    let pin_net_no = entry.nets[0];
    let pin_first_layer = board
        .item_first_layer(pin_id)
        .expect("a live pin carries its first layer");
    let pin_last_layer = board
        .item_last_layer(pin_id)
        .expect("a live pin carries its last layer");
    if pin_first_layer != pin_last_layer || net_count != 1 {
        return already_connected(board);
    }
    let pin_layer = pin_first_layer;

    let connected_set = item_connected_set(manager, board, pin_id, pin_net_no);
    for contact in &connected_set {
        let contact_first = board
            .item_first_layer(contact.0)
            .expect("a live contact carries its first layer");
        let contact_last = board
            .item_last_layer(contact.0)
            .expect("a live contact carries its last layer");
        if contact_first != pin_layer || contact_last != pin_layer {
            return already_connected(board);
        }
    }

    let unconnected_set = item_unconnected_set(manager, board, pin_id, pin_net_no);
    if unconnected_set.is_empty() {
        // Java reuses the "already connected" MESSAGE literal here but
        // answers the NO_UNCONNECTED_NETS state (RoutingBoard.java:999-1001)
        // — the consumer renders pin_no_unconnected_nets and bumps NO
        // counter, not the already-connected one.
        return AutorouteAttemptResult::with_details(
            AutorouteAttemptState::NoUnconnectedNets,
            format!(
                "The pin '{}' is already connected.",
                item_to_string(board, u64::from(pin_id.get()))
            ),
        );
    }

    let pin_center = board
        .pin_center(pin_id)
        .expect("a live pin carries a center");
    let pin_center_float = pin_center.to_float();
    // Java: `new ArrayList<>(unconnectedSet)` starts in the TreeSet's
    // DESCENDING-id order; the stable sort keeps that order for ties.
    let mut sorted_targets: Vec<ItemId> = unconnected_set.iter().map(|reverse| reverse.0).collect();
    sorted_targets.sort_by(|&a, &b| {
        bounding_box_center_dist_sq(board, a, &pin_center_float)
            .total_cmp(&bounding_box_center_dist_sq(board, b, &pin_center_float))
    });

    let mut ctrl = AutorouteControl::new_with_costs(
        board,
        pin_net_no,
        &settings.router_settings,
        settings.via_costs,
        &settings.trace_costs,
    );
    ctrl.is_fanout = true;
    let fallback_to_board_vias = settings.router_settings.fanout.fallback_to_board_vias;
    if fallback_to_board_vias && let Some(net_rule) = ctrl.via_rule.as_ref() {
        let mut combined = ViaRule {
            id: 0, // unregistered table entry — rebuild_via_info reads via_infos only
            name: format!("{}_fallback", net_rule.name),
            via_infos: net_rule.via_infos.clone(),
        };
        if let Some(default_rule) = board.rules().via_rules.first() {
            for &via_index in &default_rule.via_infos {
                if !combined.via_infos.contains(&via_index) {
                    combined.via_infos.push(via_index);
                }
            }
        }
        ctrl.via_rule = Some(combined);
        ctrl.rebuild_via_info(board, settings.via_costs, pin_net_no);
    }
    let pin_component_name = board
        .get(pin_id)
        .map(|entry| entry.component_id)
        .and_then(|component_id| board.components().get(u32::try_from(component_id).ok()?))
        .map(|component| component.name.clone());
    ctrl.fanout_start_pin_name = Some(match pin_component_name {
        Some(component_name) => {
            // Java: `pinComponent != null && pin.name() != null` — the
            // name face is null exactly when the component is.
            let pin_name = board
                .get(pin_id)
                .and_then(|entry| match &entry.data {
                    ItemData::Pin { pin_index, .. } => Some(*pin_index),
                    _ => None,
                })
                .and_then(|pin_index| {
                    let component_id = board
                        .get(pin_id)
                        .map(|entry| entry.component_id)
                        .and_then(|id| u32::try_from(id).ok())?;
                    let component = board.components().get(component_id)?;
                    let package = board.library().package(component.package_no())?;
                    package.get_pin(pin_index).map(|pin| pin.name.clone())
                })
                .unwrap_or_else(|| "null".to_string());
            format!("{component_name}-{pin_name}")
        }
        None => item_to_string(board, u64::from(pin_id.get())),
    });
    ctrl.fanout_start_pin_center = Some(pin_center);
    ctrl.fanout_start_pin_layer = pin_layer;
    ctrl.remove_unconnected_vias = false;
    if ripup_costs >= 0 {
        ctrl.ripup_allowed = true;
        ctrl.ripup_costs = ripup_costs;
    }

    let start_items: Vec<u64> = connected_set
        .iter()
        .map(|reverse| u64::from(reverse.0.get()))
        .collect();
    let dest_items: Vec<u64> = sorted_targets
        .iter()
        .map(|id| u64::from(id.get()))
        .collect();

    // Java: `rippedItemList` is a fresh TreeSet per pin, SHARED across
    // both engine calls; the ripup-costs map is null (see the module
    // banks).
    let mut ripped_item_list: BTreeMap<i32, crate::engine::RippedItemSeed> = BTreeMap::new();
    let mut engine = init_autoroute(
        manager,
        board,
        pin_net_no,
        ctrl.trace_clearance_class_index,
        false, // Java initAutoroute(..., false) — retainDatabase off
        stoppable_flag.cloned(),
        Some(budget),
    );

    let result: Option<AutorouteAttemptResult> = if sorted_targets.len() <= 4 {
        if let Some(&closest_target) = sorted_targets.first() {
            // 1. Try to route to the closest target first.
            let mut result = engine.autoroute_connection(
                &start_items,
                &[u64::from(closest_target.get())],
                &mut ctrl,
                &mut ripped_item_list,
                &mut HashMap::new(),
                &mut TraceTightenerSeam,
                &mut SinkBridge(sink),
            );
            // 2. Fall back to searching the entire unconnected set at
            // once.
            if result.state != AutorouteAttemptState::Routed
                && result.state != AutorouteAttemptState::AlreadyConnected
                && sorted_targets.len() > 1
            {
                result = engine.autoroute_connection(
                    &start_items,
                    &dest_items,
                    &mut ctrl,
                    &mut ripped_item_list,
                    &mut HashMap::new(),
                    &mut TraceTightenerSeam,
                    &mut SinkBridge(sink),
                );
            }
            Some(result)
        } else {
            // Unreachable: the empty unconnected set returned
            // NO_UNCONNECTED_NETS above (Java leaves result null and
            // answers FAILED below — the same dead-defensive arm).
            None
        }
    } else {
        // Large nets: route to the entire unconnected set at once.
        Some(engine.autoroute_connection(
            &start_items,
            &dest_items,
            &mut ctrl,
            &mut ripped_item_list,
            &mut HashMap::new(),
            &mut TraceTightenerSeam,
            &mut SinkBridge(sink),
        ))
    };
    drop(engine);

    let result = result.unwrap_or_else(|| {
        AutorouteAttemptResult::with_details(
            AutorouteAttemptState::Failed,
            "No target items to route connection.",
        )
    });

    if result.state == AutorouteAttemptState::Routed {
        let tightener_trace_costs: Vec<TraceCostFactor> =
            ctrl.trace_costs.iter().map(Into::into).collect();
        opt_changed_area(
            manager,
            board,
            &mut TraceTightenerSeam,
            &[pin_net_no],
            None,
            settings.pull_tight_accuracy,
            None,
            0,
            Some(&tightener_trace_costs),
            stoppable_flag,
            TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
            settings.deterministic_budgets,
        );
    }
    result
}

// ---------------------------------------------------------------------------
// pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::test_util::{net_no, parse};
    use epic_board::id::ItemId;

    /// The fanout ordering world (2 signal layers, `unit um`,
    /// resolution 1 → coordinates are um·1000). Components in
    /// placement order (ids 1..): CMP_BIG (3 SMD pins), CMP_SMALL
    /// (2), CMP_TIE2 (1), CMP_TIE1 (1), CMP_TH (through-hole),
    /// CMP_ORPHAN (unnetted SMD). Absolute pin centers: BIG
    /// P1/P2/P3 = (20000,30000)/(35000,30000)/(35000,48000); SMALL
    /// P1/P2 = (90000,30000)/(100000,30000); TIE2 (60000,55000);
    /// TIE1 (62000,55000); TH (20000,50000); orphan (110000,50000).
    ///
    /// The wiring section routes NET1 from BIG-P1 to TH with one
    /// clean vertical trace, so BIG-P1 touches a violation-free
    /// trace (escaped) while SMALL-P1 stays unconnected.
    const FANOUT_DSN: &str = r#"(pcb epic-router-fanout.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_BIG
      (place CMP_BIG 20000 30000 front 0)
    )
    (component CMP_SMALL
      (place CMP_SMALL 90000 30000 front 0)
    )
    (component CMP_TIE2
      (place CMP_TIE2 60000 55000 front 0)
    )
    (component CMP_TIE1
      (place CMP_TIE1 62000 55000 front 0)
    )
    (component CMP_TH
      (place CMP_TH 20000 50000 front 0)
    )
    (component CMP_ORPHAN
      (place CMP_ORPHAN 110000 50000 front 0)
    )
  )
  (library
    (padstack PAD_SMD
      (shape (circle F.Cu 600 0 0))
    )
    (padstack PAD_TH
      (shape (circle F.Cu 500 0 0))
      (shape (circle B.Cu 500 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_BIG
      (pin PAD_SMD P1 0 0)
      (pin PAD_SMD P2 15000 0)
      (pin PAD_SMD P3 15000 18000)
    )
    (image CMP_SMALL
      (pin PAD_SMD P1 0 0)
      (pin PAD_SMD P2 10000 0)
    )
    (image CMP_TIE2
      (pin PAD_SMD P1 0 0)
    )
    (image CMP_TIE1
      (pin PAD_SMD P1 0 0)
    )
    (image CMP_TH
      (pin PAD_TH P1 0 0)
    )
    (image CMP_ORPHAN
      (pin PAD_SMD P1 0 0)
    )
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_BIG-P1 CMP_SMALL-P1 CMP_TH-P1))
    (net NET2 (pins CMP_BIG-P2))
    (net NET3 (pins CMP_BIG-P3 CMP_SMALL-P2))
    (net NET4 (pins CMP_TIE2-P1))
    (net NET5 (pins CMP_TIE1-P1))
  )
  (wiring
    (wire (path F.Cu 200  20000 30000  20000 50000) (net NET1))
  )
)
"#;

    /// The jar world's cost table (the batch.rs `settings_ir` shape).
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

    fn fanout_settings(sorting_order: &str) -> BatchSettings {
        let mut settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        settings.router_settings.fanout.pin_sorting_order = sorting_order.to_string();
        settings
    }

    fn row(pin_index: i32, dist_center: f64, dist_net: f64, density: i32) -> SmdPinRow {
        SmdPinRow {
            item_id: ItemId::new(u32::try_from(pin_index).unwrap_or(0) + 1),
            pin_index,
            full_name: format!("C-P{pin_index}"),
            net_number: 1,
            distance_to_component_center: dist_center,
            distance_to_closest_on_net: dist_net,
            surroundings_density: density,
        }
    }

    /// The comparator discriminator (Java `Component.Pin.compareTo`,
    /// `:762-797`): for EVERY sorting-order arm, the strict less /
    /// greater faces with that arm's key differing by EXACTLY 1.0 and
    /// the other keys equal, plus the equal-key face falling through
    /// to the `pinIndex` tie-break (a `<=`/`>=` mutant flips an equal
    /// face and dies here; a swapped-direction mutant flips a strict
    /// face). The `Other` fallback arm compares EQUAL on every key.
    #[test]
    fn ordering_compare_matrix() {
        use PinSortingOrder::{
            DistanceToClosestOnNet, InnerFirst, Other, OuterFirst, SurroundingsDensity,
        };
        // The center keys differ by exactly 1.0; net/density equal.
        let a = row(0, 100.0, 7.0, 1);
        let b = row(1, 101.0, 7.0, 1);
        assert_eq!(a.compare(&b, InnerFirst), Ordering::Less);
        assert_eq!(b.compare(&a, InnerFirst), Ordering::Greater);
        assert_eq!(a.compare(&b, OuterFirst), Ordering::Greater);
        assert_eq!(b.compare(&a, OuterFirst), Ordering::Less);
        // GENUINE tie on the center key (both rows 100.0): BOTH center
        // arms fall through to the pinIndex tie-break, arm-independently
        // — the crossing cell this block claims. (The strict faces
        // above used centers 100 vs 101; these rows do not.)
        let tie_a = row(0, 100.0, 7.0, 1);
        let tie_b = row(1, 100.0, 7.0, 1);
        assert_eq!(tie_a.compare(&tie_a, InnerFirst), Ordering::Equal);
        assert_eq!(
            tie_a.compare(&tie_b, InnerFirst),
            Ordering::Less,
            "genuine tie → pinIndex"
        );
        assert_eq!(
            tie_b.compare(&tie_a, InnerFirst),
            Ordering::Greater,
            "genuine tie → pinIndex"
        );
        assert_eq!(
            tie_a.compare(&tie_b, OuterFirst),
            Ordering::Less,
            "genuine tie → pinIndex"
        );
        assert_eq!(
            tie_b.compare(&tie_a, OuterFirst),
            Ordering::Greater,
            "genuine tie → pinIndex"
        );

        // The net keys differ by exactly 1.0; center/density equal.
        let a = row(0, 100.0, 7.0, 1);
        let b = row(1, 100.0, 8.0, 1);
        assert_eq!(a.compare(&b, DistanceToClosestOnNet), Ordering::Less);
        assert_eq!(b.compare(&a, DistanceToClosestOnNet), Ordering::Greater);
        assert_eq!(a.compare(&b, InnerFirst), Ordering::Less, "tie → pinIndex");
        assert_eq!(a.compare(&b, OuterFirst), Ordering::Less, "tie → pinIndex");

        // The density keys differ by 1; SurroundingsDensity is
        // `other - this` (densest first).
        let a = row(0, 100.0, 7.0, 2);
        let b = row(1, 100.0, 7.0, 1);
        assert_eq!(a.compare(&b, SurroundingsDensity), Ordering::Less);
        assert_eq!(b.compare(&a, SurroundingsDensity), Ordering::Greater);
        assert_eq!(a.compare(&b, InnerFirst), Ordering::Less, "tie → pinIndex");

        // The Other arm: equal on every key, pinIndex decides.
        let a = row(0, 100.0, 7.0, 1);
        let b = row(1, 200.0, 9.0, 5);
        assert_eq!(a.compare(&b, Other), Ordering::Less);
        assert_eq!(b.compare(&a, Other), Ordering::Greater);
        assert_eq!(a.compare(&row(0, 1.0, 2.0, 3), Other), Ordering::Equal);
    }

    /// Java dispatches the RAW string with four exact matches; any
    /// other value (wrong case, junk, empty) is the tie-break arm.
    #[test]
    fn pin_sorting_order_parse_table() {
        assert_eq!(
            PinSortingOrder::parse("inner_first"),
            PinSortingOrder::InnerFirst
        );
        assert_eq!(
            PinSortingOrder::parse("outer_first"),
            PinSortingOrder::OuterFirst
        );
        assert_eq!(
            PinSortingOrder::parse("distanceToClosestOnNet"),
            PinSortingOrder::DistanceToClosestOnNet
        );
        assert_eq!(
            PinSortingOrder::parse("surroundingsDensity"),
            PinSortingOrder::SurroundingsDensity
        );
        for junk in [
            "INNER_FIRST",
            "outerFirst",
            "",
            "distance_to_closest_on_net",
        ] {
            assert_eq!(
                PinSortingOrder::parse(junk),
                PinSortingOrder::Other,
                "{junk} must be the tie-break arm"
            );
        }
    }

    /// Java `:702-713`: `smdPinCount` DESCENDING, tie `component.id`
    /// ASCENDING.
    #[test]
    fn component_order_count_desc_id_asc() {
        let mk = |count: i32, id: i32| ComponentFanout {
            component_id: id,
            smd_pin_count: count,
            pins: Vec::new(),
        };
        assert_eq!(compare_components(&mk(3, 2), &mk(2, 9)), Ordering::Less);
        assert_eq!(compare_components(&mk(2, 9), &mk(3, 2)), Ordering::Greater);
        assert_eq!(
            compare_components(&mk(1, 3), &mk(1, 4)),
            Ordering::Less,
            "equal count → id ASC"
        );
        assert_eq!(compare_components(&mk(1, 4), &mk(1, 3)), Ordering::Greater);
        assert_eq!(compare_components(&mk(1, 3), &mk(1, 3)), Ordering::Equal);
    }

    /// The ctor end-to-end on the crafted world (cerebrum 13: every
    /// ordering arm observed through the production
    /// [`FanoutState::new`] walk, not just the comparator): the
    /// component order, the per-order BIG/SMALL pin sequences, the
    /// exact sort keys and the counts.
    ///
    /// Key math (um·1000 units; gravity centers BIG (30000,36000),
    /// SMALL (95000,30000)):
    /// * BIG center distances: P3 = √(5000²+12000²) = 13000.0
    ///   EXACT, P1 ≈ 11661.5, P2 ≈ 7810.3.
    /// * Closest-on-net over ALL live pins (the TH pad is not SMD
    ///   but IS on NET1): P1 → TH = 20000.0 EXACT (an SMD-only scan
    ///   mutant answers 70000.0 = SMALL-P1), P3 → SMALL-P2 ≈
    ///   67446.3, P2 → f64::MAX (sole NET2 pin), SMALL-P1 → BIG-P1
    ///   = 70000.0, SMALL-P2 → BIG-P3 ≈ 67446.3.
    /// * Density (netted SMD pins within 20000.0): P2 sees P1+P3 =
    ///   2; P1 sees P2 = 1; P3 sees P2 = 1; each SMALL pin sees its
    ///   partner = 1; TIE2/TIE1 see each other = 1.
    #[test]
    fn fanout_state_new_orders_keys_and_counts() {
        for (sorting_order, big_sequence, small_sequence) in [
            ("outer_first", vec![2, 0, 1], vec![0, 1]),
            ("inner_first", vec![1, 0, 2], vec![0, 1]),
            ("distanceToClosestOnNet", vec![0, 2, 1], vec![1, 0]),
            ("surroundingsDensity", vec![1, 0, 2], vec![0, 1]),
            ("junk_falls_to_pin_index", vec![0, 1, 2], vec![0, 1]),
        ] {
            let (mut manager, mut board) = parse(FANOUT_DSN);
            let state = FanoutState::new(&mut manager, &mut board, &fanout_settings(sorting_order));
            assert_eq!(
                state.total_smd_pin_count, 7,
                "{sorting_order}: 8 SMD pins on the world minus the unnetted orphan"
            );
            assert_eq!(
                state.already_connected_pin_count, 3,
                "{sorting_order}: the three single-pin nets (BIG-P2, TIE2-P1, TIE1-P1)"
            );
            // Component order: count DESC, then id ASC (CMP_TIE2 id 3
            // before CMP_TIE1 id 4).
            let component_ids: Vec<i32> = state.components.iter().map(|c| c.component_id).collect();
            assert_eq!(component_ids, vec![1, 2, 3, 4], "{sorting_order}");
            assert_eq!(state.components[0].smd_pin_count, 3);
            assert_eq!(state.components[1].smd_pin_count, 2);
            let big: Vec<i32> = state.components[0]
                .pins
                .iter()
                .map(|p| p.pin_index)
                .collect();
            assert_eq!(big, big_sequence, "{sorting_order}: BIG sequence");
            let small: Vec<i32> = state.components[1]
                .pins
                .iter()
                .map(|p| p.pin_index)
                .collect();
            assert_eq!(small, small_sequence, "{sorting_order}: SMALL sequence");

            if sorting_order == "outer_first" {
                // The exact key faces (only asserted once — the rows
                // are order-independent).
                let pins = &state.components[0].pins;
                let by_index = |index: i32| {
                    pins.iter()
                        .find(|p| p.pin_index == index)
                        .unwrap_or_else(|| panic!("pin {index}"))
                };
                let p1 = by_index(0);
                let p2 = by_index(1);
                let p3 = by_index(2);
                assert_eq!(
                    p3.distance_to_component_center, 13000.0,
                    "exact gravity distance"
                );
                assert_eq!(
                    p1.distance_to_component_center,
                    FloatPoint::new(20000.0, 30000.0).distance(&FloatPoint::new(30000.0, 36000.0))
                );
                assert_eq!(
                    p1.distance_to_closest_on_net, 20000.0,
                    "the TH pad (non-SMD, same net) is the closest NET1 pin"
                );
                assert_eq!(p2.distance_to_closest_on_net, f64::MAX, "sole NET2 pin");
                assert_eq!(
                    p3.distance_to_closest_on_net,
                    FloatPoint::new(35000.0, 48000.0).distance(&FloatPoint::new(100000.0, 30000.0))
                );
                assert_eq!(p1.surroundings_density, 1);
                assert_eq!(p2.surroundings_density, 2);
                assert_eq!(p3.surroundings_density, 1);
                assert_eq!(p3.full_name, "CMP_BIG-P3");
                assert_eq!(p1.net_number, net_no(&board, "NET1"));
                assert_eq!(p2.net_number, net_no(&board, "NET2"));
                assert_eq!(p3.net_number, net_no(&board, "NET3"));
                let small_p1 = &state.components[1].pins[0];
                assert_eq!(small_p1.distance_to_component_center, 5000.0);
                assert_eq!(small_p1.full_name, "CMP_SMALL-P1");
            }
        }
    }

    /// The stage loop end-to-end on the starved world (only layer 1
    /// active, pins live on layer 0): every attempt fails with zero
    /// expansion work, so `routed == 0` breaks after ONE pass. The
    /// pass rows carry the exact counters. Note the ctor/pass split
    /// on the already-connected face: the ctor counts 3 (the
    /// single-pin nets — BIG-P1 still has an unconnected NET1
    /// partner), and only BIG-P1 answers AlreadyConnected in the pass
    /// (its connected set contains the through-hole pin, tripping the
    /// same-layer contact guard). The three single-pin nets answer
    /// the empty-unconnected arm — Java's NO_UNCONNECTED_NETS state
    /// carrying the reused "already connected" MESSAGE literal
    /// (`RoutingBoard.java:999-1001`) — so they render
    /// `pin_no_unconnected_nets` rows and bump NO counter.
    #[test]
    fn fanout_board_starved_one_pass_rows() {
        let (mut manager, mut board) = parse(FANOUT_DSN);
        let mut settings = fanout_settings("outer_first");
        settings.router_settings.layer_active = vec![false, true];
        let mut sink = CaptureDriverSink::default();
        let summary = fanout_board(&mut manager, &mut board, &settings, None, None, &mut sink);
        assert_eq!(
            summary.completed_pass_count, 1,
            "routed==0 breaks after pass 1"
        );
        assert!(!summary.is_timed_out);
        assert_eq!(summary.escape_statistics.total_smd_pins, 7);
        assert_eq!(summary.escape_statistics.escaped_count, 1);
        assert_eq!(summary.escape_statistics.to_display(), "1/7 (14.3%)");

        let trace = sink.joined("trace");
        assert!(
            trace.contains(
                "pass_start pass=1, totalPins=7, alreadyConnected=3, \
                 pinsToFanout=4, ripupCosts=1, baseMillisPerPin=10000"
            ),
            "pass_start row: {trace}"
        );
        assert_eq!(
            trace.matches("pin_start pin=").count(),
            7,
            "one row per pin: {trace}"
        );
        assert_eq!(
            trace.matches("pin_already_connected pin=").count(),
            1,
            "only BIG-P1 (the TH contact guard): {trace}"
        );
        assert_eq!(
            trace.matches("pin_no_unconnected_nets pin=").count(),
            3,
            "the single-pin nets, at Java's NO_UNCONNECTED_NETS state: {trace}"
        );
        assert_eq!(trace.matches("pin_failed pin=").count(), 3, "{trace}");
        assert!(
            trace.contains(
                "routed=0, notRouted=3, insertErrors=0, alreadyConnected=1, \
                            escaped=1/7 (14.3%)"
            ),
            "pass_end row: {trace}"
        );
        assert!(
            !sink.any_contains("Fanout stopped"),
            "no oscillation stop: {trace:?}"
        );
        assert!(!sink.any_contains("Max items limit"), "{trace:?}");
        // The listener-less pass info row (Java `:490-508` shape).
        assert!(
            sink.joined("info").contains(
                "fanout pass: 1, routed: 0, not routed: 3, errors: 0, \
                 extra vias: +0, escaped SMD pins: 1/7 (14.3%)"
            ),
            "info rows: {:?}",
            sink.joined("info")
        );
    }

    /// The extracted oscillation detector at its exact boundary
    /// (cerebrum 16): with Java's limit 3, two identical-state
    /// REPEATS continue and the THIRD repeat stops; any different
    /// state (routed OR via count moved) resets the run to 0.
    #[test]
    fn oscillation_step_repeat_boundary() {
        // First sight: the i64::MIN seed never matches; state stored.
        let (run, previous, stop) = oscillation_step(2, 5, i64::MIN, 0, 3);
        assert!(!stop);
        assert_eq!((run, previous), (0, (2 << 32) ^ 5));
        let (run, previous, stop) = oscillation_step(2, 5, previous, run, 3);
        assert!(!stop);
        assert_eq!(run, 1, "first REPEAT");
        let (run, previous, stop) = oscillation_step(2, 5, previous, run, 3);
        assert!(!stop, "2 identical REPEATS continue");
        assert_eq!(run, 2);
        let (_, _, stop) = oscillation_step(2, 5, previous, run, 3);
        assert!(stop, "the 3rd repeat stops");

        // A state change resets: A,A,B,A,A never stops.
        let (_, previous, _) = oscillation_step(2, 5, i64::MIN, 0, 3);
        let (run, previous, _) = oscillation_step(2, 5, previous, 0, 3);
        assert_eq!(run, 1);
        let (run, previous, _) = oscillation_step(3, 5, previous, run, 3);
        assert_eq!(run, 0, "a routed-count change resets the run");
        let (run, previous, _) = oscillation_step(3, 6, previous, run, 3);
        assert_eq!(run, 0, "a via-count change resets the run");
        let (run, previous, _) = oscillation_step(3, 6, previous, run, 3);
        assert_eq!(run, 1);
        let (run, _, stop) = oscillation_step(3, 6, previous, run, 3);
        assert_eq!(run, 2);
        assert!(!stop, "two repeats after the reset still continue");
    }

    /// The PRODUCTION stagnation wiring (cerebrum 16): the boundary
    /// pin above feeds the pure fn its own literal 3s, so a flip of
    /// the call-site constant survived it (reviewer mutant T7-S1).
    /// The constant is the Java face (`BatchFanout.java:105`
    /// `stagnationPassLimit = 3`), `fanout_board` feeds exactly this
    /// constant to the detector, and the third consecutive REPEAT
    /// stops exactly AT it. The call site itself is pinned END-TO-END
    /// by [`fanout_board_stops_on_three_identical_pass_states`] (the
    /// quality-review MINOR-3 fix): the substitution mutant now dies
    /// there.
    #[test]
    fn stagnation_limit_production_constant_is_java_face() {
        assert_eq!(STAGNATION_PASS_LIMIT, 3, "Java BatchFanout.java:105");
        // The seed sight stores the state without arming the run.
        let (_, previous, _) = oscillation_step(2, 5, i64::MIN, 0, STAGNATION_PASS_LIMIT);
        let (run, previous, stop) = oscillation_step(2, 5, previous, 0, STAGNATION_PASS_LIMIT);
        assert_eq!(run, 1);
        assert!(!stop, "the first REPEAT continues");
        let (run, previous, stop) = oscillation_step(2, 5, previous, run, STAGNATION_PASS_LIMIT);
        assert_eq!(run, 2);
        assert!(!stop, "the second REPEAT continues");
        let (run, _, stop) = oscillation_step(2, 5, previous, run, STAGNATION_PASS_LIMIT);
        assert!(stop, "the third REPEAT stops");
        assert_eq!(run, STAGNATION_PASS_LIMIT, "the stop lands ON the limit");
    }

    /// JDK-verified faces (see [`parse_timespan_string`]): the
    /// trailing-empty split drop, the lone-colon no-parts arm, the
    /// middle-empty failure, the >3-parts arm, per-component signs,
    /// whitespace rejection, overflow, and the 1/2/3-part arithmetic.
    #[test]
    fn parse_timespan_faces() {
        assert_eq!(parse_timespan_string("90"), Some(90));
        assert_eq!(parse_timespan_string("2:30"), Some(150));
        assert_eq!(parse_timespan_string("1:02:03"), Some(3723));
        assert_eq!(
            parse_timespan_string("1:60"),
            Some(120),
            "unit overflow rides raw math"
        );
        assert_eq!(parse_timespan_string("007"), Some(7));
        assert_eq!(
            parse_timespan_string("1:2:"),
            Some(62),
            "trailing empty part dropped"
        );
        assert_eq!(parse_timespan_string(":"), None, "no parts left");
        assert_eq!(parse_timespan_string("::"), None);
        assert_eq!(parse_timespan_string("1::30"), None, "middle empty fails");
        assert_eq!(parse_timespan_string("1:2:3:4"), None);
        assert_eq!(
            parse_timespan_string("1:-2"),
            Some(58),
            "per-component sign"
        );
        assert_eq!(parse_timespan_string("-1:30"), Some(-30));
        assert_eq!(
            parse_timespan_string("1:30 "),
            None,
            "whitespace fails the parse"
        );
        assert_eq!(parse_timespan_string(" 90"), None);
        assert_eq!(parse_timespan_string(""), None);
        assert_eq!(parse_timespan_string("   "), None);
        assert_eq!(parse_timespan_string("abc"), None);
        assert_eq!(
            parse_timespan_string("99999999999999999999"),
            None,
            "overflow"
        );
    }

    /// The escape scan (`BoardStatistics` fanout block `:442-462`):
    /// exactly the trace-touching BIG-P1 counts on the crafted world.
    #[test]
    fn escape_statistics_faces() {
        let (mut manager, mut board) = parse(FANOUT_DSN);
        let stats = escape_statistics_from_board(&mut manager, &mut board);
        assert_eq!(stats.total_smd_pins, 7);
        assert_eq!(
            stats.escaped_count, 1,
            "BIG-P1 touches the clean NET1 trace"
        );
        assert_eq!(stats.to_display(), "1/7 (14.3%)");
        assert!((stats.escaped_percentage - 100.0 / 7.0).abs() < 1e-9);
        assert_eq!(EscapeStatistics::placeholder(4).to_display(), "0/4 (0.0%)");
    }

    /// The escape scan's Via and ConductionArea ARMS (quality review
    /// NIT-1: every earlier world exercised only the Trace arm). All
    /// geometry is pre-placed, no routing. Drill normal contacts are
    /// CENTER-COINCIDENCE, not shape overlap (`DrillItem.java:273-306`),
    /// so the world's via sits EXACTLY on P1's center — and its
    /// witness, a same-net trace ENDING at that center, necessarily
    /// contacts P1 directly too. Arm purity comes from the violation
    /// gate: the witness deliberately VIOLATES against the foreign-net
    /// pad at its far end, so P1's direct TRACE arm is dead (the
    /// violations-non-empty gate) while the VIA arm still counts the
    /// same trace as its witness (the arm does NOT gate the witness —
    /// `isPinEscaped`, `:590-608`). Deleting the Via arm — or adding a
    /// violation gate on its witness — both flip P1 to not-escaped.
    /// P2's only qualifying contact is the bare `(plane ...)` pour —
    /// the CA arm. P3 touches nothing; the foreign pin's only contact
    /// is the violating trace (the trace arm's violation gate holds
    /// from its side too) — neither escapes.
    #[test]
    fn escape_scan_via_and_conduction_area_arms() {
        const ARMS_DSN: &str = r#"(pcb epic-router-escape-arms.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 120000 80000))
    (rule (width 200) (clearance 200))
    (plane N1 (rect F.Cu 19000 19000 21000 21000))
  )
  (placement
    (component CMP1
      (place CMP1 20000 40000 front 0)
    )
    (component CMP2
      (place CMP2 26000 40000 front 0)
    )
  )
  (library
    (padstack PAD_SMD
      (shape (circle F.Cu 600 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP1
      (pin PAD_SMD P1 0 0)
      (pin PAD_SMD P2 0 -20000)
      (pin PAD_SMD P3 0 20000)
    )
    (image CMP2
      (pin PAD_SMD P1 0 0)
    )
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net N1 (pins CMP1-P1 CMP1-P2 CMP1-P3))
    (net FNET (pins CMP2-P1))
  )
  (wiring
    (via PAD_VIA 20000 40000 (net N1))
    (wire (path F.Cu 200  20000 40000  26000 40000) (net N1))
  )
)
"#;
        let (mut manager, mut board) = parse(ARMS_DSN);
        // Every NETTED SMD pin, ascending id: [P1, P2, P3, foreign].
        let netted_pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }) && !entry.nets.is_empty())
            .map(|entry| entry.id)
            .collect();
        assert_eq!(netted_pins.len(), 4);
        // The arms, individually (P1 = via arm, P2 = CA arm, the rest none).
        assert!(
            is_pin_escaped(&mut manager, &mut board, netted_pins[0]),
            "P1 escaped via the VIA arm (the witness trace is violating, \
             so the direct TRACE arm is dead)"
        );
        assert!(
            is_pin_escaped(&mut manager, &mut board, netted_pins[1]),
            "P2 escaped via the bare CONDUCTION-AREA arm"
        );
        assert!(
            !is_pin_escaped(&mut manager, &mut board, netted_pins[2]),
            "P3 has no contact — not escaped"
        );
        assert!(
            !is_pin_escaped(&mut manager, &mut board, netted_pins[3]),
            "the foreign pin's only contact is the violating trace — \
             the trace arm's violation gate holds"
        );
        let stats = escape_statistics_from_board(&mut manager, &mut board);
        assert_eq!(stats.total_smd_pins, 4);
        assert_eq!(stats.escaped_count, 2);
        assert_eq!(stats.to_display(), "2/4 (50.0%)");
    }

    /// THE OSCILLATION STOP, END-TO-END (quality review MINOR-3): the
    /// production `fanout_board` loop armS the extracted detector on
    /// REAL passes and prints the exact stop row. The world: ONE
    /// netted SMD pin and FOUR floating same-net wire stubs at
    /// strictly increasing bounding-box distances (2000/4500/7000/9500)
    /// — the closest-first target sort makes each pass escape to the
    /// next stub, so passes 1..4 all answer routed=1 with the via
    /// count constant at 0: four IDENTICAL detector states
    /// `(1 << 32) ^ 0` while every board hash differs (each pass adds
    /// one trace). The detector arms at the third repeat and the loop
    /// stops after pass 4. The stubs must be wires, not pads: an SMD
    /// helper pin would itself be fanout-attempted (the net would
    /// self-complete inside pass 1), and any multi-layer item on the
    /// net trips the transitive same-layer contact guard
    /// (Java `RoutingBoard.fanout`, `:991-997`) — a floating wire is
    /// neither. All stubs sit in the full-height slab within 4500 of
    /// the pin, so every room-door crossing passes the frontier gate's
    /// max-escape arm and each escape completes AT the stub's
    /// gate-exempt target door (+0 vias, `vias_allowed = false`).
    #[test]
    fn fanout_board_stops_on_three_identical_pass_states() {
        const OSC_DSN: &str = r#"(pcb epic-router-fanout-osc.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 60000 120000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP1
      (place CMP1 0 0 front 0)
    )
  )
  (library
    (padstack PAD_SMD
      (shape (circle F.Cu 200 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP1
      (pin PAD_SMD P1 22000 60600)
    )
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net N0 (pins CMP1-P1))
  )
  (wiring
    (wire (path F.Cu 200  21200 58600  22800 58600) (net N0))
    (wire (path F.Cu 200  21200 56100  22800 56100) (net N0))
    (wire (path F.Cu 200  21200 53600  22800 53600) (net N0))
    (wire (path F.Cu 200  21200 51100  22800 51100) (net N0))
  )
)
"#;
        let (mut manager, mut board) = parse(OSC_DSN);
        let mut settings = fanout_settings("junk");
        settings.router_settings.fanout.ripup_allowed = false;
        settings.router_settings.vias_allowed = false;
        let traces_before = board
            .iter_ascending()
            .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Trace { .. }))
            .count();
        let vias_before = board
            .iter_ascending()
            .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Via { .. }))
            .count();
        let mut sink = CaptureDriverSink::default();
        let summary = fanout_board(&mut manager, &mut board, &settings, None, None, &mut sink);
        assert_eq!(
            summary.completed_pass_count, 4,
            "the third identical-state REPEAT stops after pass 4"
        );
        assert!(!summary.is_timed_out);
        assert!(
            sink.joined("info")
                .contains("Fanout stopped after 4 passes: no progress for 3 consecutive passes."),
            "the exact oscillation stop row: {:?}",
            sink.joined("info")
        );
        // Every pass routed exactly the one pin; the board grew by one
        // trace per pass and never by a via.
        let trace = sink.joined("trace");
        assert_eq!(trace.matches("pin_routed pin=").count(), 4, "{trace}");
        assert_eq!(trace.matches("pin_already_connected pin=").count(), 0);
        assert_eq!(trace.matches("pin_failed pin=").count(), 0, "{trace}");
        // Each escape merges the reached stub into the connected
        // geometry: the normalizer rebuilds the overlapping same-net
        // polylines, so this world grows by exactly 2 trace ITEMS per
        // pass (measured; the via count stays pinned above).
        assert_eq!(
            board
                .iter_ascending()
                .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Trace { .. }))
                .count(),
            traces_before + 8,
            "two normalized trace items per escape pass"
        );
        assert_eq!(
            board
                .iter_ascending()
                .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Via { .. }))
                .count(),
            vias_before,
            "no vias inserted (completion at the stub target doors)"
        );
        let stats = summary.escape_statistics;
        assert_eq!(stats.total_smd_pins, 1);
        assert_eq!(stats.escaped_count, 1);
        assert_eq!(stats.to_display(), "1/1 (100.0%)");
        // The detector consumed four identical states — the pass_end
        // rows confirm each pass routed exactly 1.
        assert_eq!(trace.matches("routed=1, notRouted=0").count(), 4, "{trace}");
    }

    /// The per-pin escape (Java `RoutingBoard.fanout`) through the
    /// real engine. THE ESCAPE SEMANTICS (Java-verified): the maze's
    /// fanout mode completes at the FIRST drill
    /// (`MazeSearchEngine.java:361-369` — "algorithm completed after
    /// the first drill"), so a successful escape inserts exactly the
    /// start stub + one via — the far pin stays unconnected until
    /// the batch pass. The escaped set therefore counts P1 only.
    /// A repeat answers the already-connected literal through the
    /// multi-layer CONTACT guard (the escape via joins P1's
    /// connected set), and the through-hole pin answers it without
    /// any engine call.
    #[test]
    fn fanout_pin_routed_repeat_and_th_guard() {
        const PIN_DSN: &str = r#"(pcb epic-router-fanout-pin.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 120000 60000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP1
      (place CMP1 20000 40000 front 0)
    )
    (component CMP2
      (place CMP2 70000 40000 front 0)
    )
  )
  (library
    (padstack PAD_SMD
      (shape (circle F.Cu 600 0 0))
    )
    (padstack PAD_TH
      (shape (circle F.Cu 500 0 0))
      (shape (circle B.Cu 500 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP1
      (pin PAD_SMD P1 0 0)
      (pin PAD_SMD P2 20000 0)
    )
    (image CMP2
      (pin PAD_TH P1 0 0)
    )
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net SMDNET (pins CMP1-P1 CMP1-P2))
    (net THNET (pins CMP2-P1))
  )
)
"#;
        let count_kind = |board: &Board, want: &str| {
            board
                .iter_ascending()
                .filter(|entry| {
                    entry.on_the_board && format!("{:?}", entry.board_item_type()) == want
                })
                .count()
        };
        let (mut manager, mut board) = parse(PIN_DSN);
        let settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        let smd_net = net_no(&board, "SMDNET");
        let smd_pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| {
                matches!(entry.data, ItemData::Pin { .. }) && entry.nets.contains(&smd_net)
            })
            .map(|entry| entry.id)
            .collect();
        assert_eq!(smd_pins.len(), 2);

        let traces_before = count_kind(&board, "Trace");
        let vias_before = count_kind(&board, "Via");
        let mut sink = CaptureDriverSink::default();
        let result = fanout_pin(
            &mut manager,
            &mut board,
            &settings,
            smd_pins[0],
            1, // ripup_costs >= 0 → ripup allowed
            None,
            RouteBudget::Deterministic {
                limit: 2_000_000,
                spent: Cell::new(0),
            },
            &mut sink,
        );
        assert_eq!(
            result.state,
            AutorouteAttemptState::Routed,
            "{:?}",
            result.details
        );
        // The first-drill escape geometry: exactly the stub + the via.
        assert_eq!(count_kind(&board, "Trace"), traces_before + 1);
        assert_eq!(
            count_kind(&board, "Via"),
            vias_before + 1,
            "the escape drill"
        );
        // The far pin remains unconnected — the batch pass completes it.
        let unconnected: Vec<u32> =
            item_unconnected_set(&manager, &mut board, smd_pins[0], smd_net)
                .iter()
                .map(|r| r.0.get())
                .collect();
        assert_eq!(unconnected, vec![smd_pins[1].get()], "P2 still unconnected");
        let stats = escape_statistics_from_board(&mut manager, &mut board);
        assert_eq!(stats.total_smd_pins, 2);
        assert_eq!(stats.escaped_count, 1, "only P1 touches the escape stub");
        assert_eq!(stats.to_display(), "1/2 (50.0%)");

        // Repeat: P1's connected set now contains the escape VIA —
        // the same-layer contact guard answers before any engine call.
        let again = fanout_pin(
            &mut manager,
            &mut board,
            &settings,
            smd_pins[0],
            1,
            None,
            RouteBudget::Deterministic {
                limit: 2_000_000,
                spent: Cell::new(0),
            },
            &mut sink,
        );
        assert_eq!(again.state, AutorouteAttemptState::AlreadyConnected);
        assert!(
            again.details.ends_with(" is already connected."),
            "{:?}",
            again.details
        );
        assert_eq!(
            count_kind(&board, "Trace"),
            traces_before + 1,
            "repeat inserted nothing"
        );
        assert_eq!(
            count_kind(&board, "Via"),
            vias_before + 1,
            "repeat inserted nothing"
        );

        // The multi-layer guard: a through-hole pin answers the same
        // literal BEFORE any engine call.
        let th_net = net_no(&board, "THNET");
        let th_pin = board
            .iter_ascending()
            .find(|entry| {
                matches!(entry.data, ItemData::Pin { .. }) && entry.nets.contains(&th_net)
            })
            .map(|entry| entry.id)
            .expect("TH pin present");
        let guard = fanout_pin(
            &mut manager,
            &mut board,
            &settings,
            th_pin,
            1,
            None,
            RouteBudget::Deterministic {
                limit: 2_000_000,
                spent: Cell::new(0),
            },
            &mut sink,
        );
        assert_eq!(guard.state, AutorouteAttemptState::AlreadyConnected);
        assert_eq!(
            count_kind(&board, "Trace"),
            traces_before + 1,
            "guard inserted nothing"
        );
        assert_eq!(
            count_kind(&board, "Via"),
            vias_before + 1,
            "guard inserted nothing"
        );
    }

    /// The fanout strict-DRC revert THROUGH the production stage loop
    /// (spec review REQ-1: the T12 pin drives `enforce_strict_drc`
    /// directly and never reaches the `fanout_pass` call site, Java
    /// `BatchFanout.java:287-304`). The BLOCKNET pincer pair shapes
    /// the world: the south wire crosses CMP1-P1's pad (its start
    /// room dies → the engine answers the no-connection fail, NO
    /// revert row), and the north wire only EDGE-touches the south
    /// one's end (boundary contact is not connectivity), so CMP3-P1
    /// keeps an unconnected target and escapes — but its escape's 3
    /// new items carry clearance violations against CMP1-P1's pad, so
    /// the post-route check rips them: the `fanout_via_reverted` row
    /// fires, the pin answers FAILED (not Routed) and the pass
    /// counters attribute it to notRouted, and the id watermark grew
    /// while the on-board counts did not (the rip). The crossing
    /// cell: CMP1-P2's clean escape in the SAME pass survives
    /// (first-drill semantics insert a lone drill for it — the via
    /// count grows by exactly that one). The escape scan counts only
    /// CMP1-P2: the pads' own touching wire (the south pincer) is
    /// itself violating, so it never counts as an escape contact.
    /// The strict-DRC revert world: CMP1-P1/P2 share SMDNET (P1's
    /// escape crosses its sibling pad and maze-fails; P2's clean drill
    /// routes and survives), CMP3-P1's BLOCKNET escape routes but
    /// violates against the pre-placed pincer wires and is reverted →
    /// FAILED. Hoisted for the #933 retry-skip witness below.
    const DRC_DSN: &str = r#"(pcb epic-router-fanout-drc.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 120000 60000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP1
      (place CMP1 20000 40000 front 0)
    )
    (component CMP3
      (place CMP3 20000 20000 front 0)
    )
  )
  (library
    (padstack PAD_SMD
      (shape (circle F.Cu 600 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP1
      (pin PAD_SMD P1 0 0)
      (pin PAD_SMD P2 20000 0)
    )
    (image CMP3
      (pin PAD_SMD P1 0 0)
    )
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net SMDNET (pins CMP1-P1 CMP1-P2))
    (net BLOCKNET (pins CMP3-P1))
  )
  (wiring
    (wire (path F.Cu 200  20000 41000  20000 40050) (net BLOCKNET))
    (wire (path F.Cu 200  20000 39950  20000 20000) (net BLOCKNET))
  )
)
"#;

    #[test]
    fn fanout_board_reverts_violating_escape() {
        let count_kind = |board: &Board, want: &str| {
            board
                .iter_ascending()
                .filter(|entry| {
                    entry.on_the_board && format!("{:?}", entry.board_item_type()) == want
                })
                .count()
        };
        let (mut manager, mut board) = parse(DRC_DSN);
        let mut settings = fanout_settings("outer_first");
        // One pass keeps the revert rows single-count (post-#933 a
        // second pass would SKIP CMP3-P1 — see
        // [`fanout_skips_failed_pins_until_component_escapes`]; pre-#933
        // it re-attempted and re-reverted every pass).
        settings.router_settings.fanout.max_passes = 1;
        let smd_net = net_no(&board, "SMDNET");
        let block_net = net_no(&board, "BLOCKNET");
        let traces_before = count_kind(&board, "Trace");
        let vias_before = count_kind(&board, "Via");
        let id_watermark = board.max_generated_id();
        let mut sink = CaptureDriverSink::default();
        let summary = fanout_board(&mut manager, &mut board, &settings, None, None, &mut sink);
        assert_eq!(summary.completed_pass_count, 1);
        assert!(!summary.is_timed_out);
        assert_eq!(summary.escape_statistics.total_smd_pins, 3);
        assert_eq!(
            summary.escape_statistics.escaped_count, 1,
            "only P2's clean drill counts; P1's and CMP3-P1's touching \
             wires are themselves violating"
        );
        assert_eq!(summary.escape_statistics.to_display(), "1/3 (33.3%)");

        let trace = sink.joined("trace");
        assert!(
            trace.contains(
                "pass_start pass=1, totalPins=3, alreadyConnected=0, \
                 pinsToFanout=3, ripupCosts=1, baseMillisPerPin=10000"
            ),
            "pass_start row: {trace}"
        );
        // THE REVERT: CMP3-P1's escape routed, then its 3 new items
        // carried clearance violations and the post-route check ripped
        // them (Java `BatchFanout.java:287-304`).
        assert_eq!(
            trace.matches("fanout_via_reverted pin=").count(),
            1,
            "exactly the violating escape: {trace}"
        );
        assert!(
            trace.contains(&format!(
                "fanout_via_reverted pin=CMP3-P1, net={block_net}, \
                 reason=strict_drc: connection ripped because 3 new item(s) \
                 included clearance violations"
            )),
            "revert row: {trace}"
        );
        // The pin answers FAILED and the pass counters attribute it to
        // notRouted — NOT to routed.
        assert_eq!(trace.matches("pin_failed pin=").count(), 2, "{trace}");
        assert!(
            trace.contains(&format!(
                "pin_failed pin=CMP3-P1, net={block_net}, targetCount=1, \
                 detail=strict_drc: connection ripped because 3 new item(s) \
                 included clearance violations"
            )),
            "the reverted pin's FAILED row: {trace}"
        );
        assert!(
            trace.contains("pin_failed pin=CMP1-P1")
                && trace.contains("because no connection was found between their nets."),
            "the pad-crossed pin's maze-fail row (no revert): {trace}"
        );
        // The crossing cell: the clean escape ROUTES and SURVIVES in
        // the same pass.
        assert_eq!(trace.matches("pin_routed pin=").count(), 1, "{trace}");
        assert!(
            trace.contains(&format!("pin_routed pin=CMP1-P2, net={smd_net}")),
            "the clean escape routes: {trace}"
        );
        assert!(
            trace.contains(
                "routed=1, notRouted=2, insertErrors=0, alreadyConnected=0, \
                            escaped=1/3 (33.3%)"
            ),
            "pass_end row: {trace}"
        );
        // The watermark rip: escape items were generated (the id
        // watermark grew) but CMP3-P1's three are gone — the board
        // keeps only P2's lone first-drill via over the two pre-placed
        // pincer wires.
        assert!(
            board.max_generated_id() > id_watermark,
            "escape items were generated"
        );
        assert_eq!(
            count_kind(&board, "Trace"),
            traces_before,
            "the ripped escape's traces are gone"
        );
        assert_eq!(
            count_kind(&board, "Via"),
            vias_before + 1,
            "only P2's drill survives"
        );
        assert!(!sink.any_contains("Fanout stopped"), "{trace:?}");
        assert!(!sink.any_contains("Max items limit"), "{trace:?}");
    }

    /// #933 survivor-1 truth table (Java `BatchFanout.retryFanout`
    /// `:550-557`, upstream 339e8bb50): never-failed pins always retry;
    /// a failed pin retries only when its component's generation
    /// MOVED (another pin of the same component escaped since).
    #[test]
    fn retry_fanout_truth_table() {
        assert!(retry_fanout(None, 0), "never failed -> retry");
        assert!(retry_fanout(None, 7), "never failed at any generation");
        assert!(
            !retry_fanout(Some(0), 0),
            "failed at the unchanged generation -> skip"
        );
        assert!(!retry_fanout(Some(3), 3), "same at a later generation");
        assert!(
            retry_fanout(Some(0), 1),
            "the component escaped since -> retry"
        );
        assert!(retry_fanout(Some(2), 3));
        // The table is plain inequality: a backward generation (which
        // the monotone bump can never produce) still reads as "moved".
        assert!(retry_fanout(Some(3), 2));
    }

    /// #933 survivor-1 END-TO-END witness (upstream 339e8bb50): a pin
    /// whose escape FAILED is skipped in later passes until its own
    /// component escapes another pin. Measured pass-1 visit order:
    /// CMP1-P1 fails FIRST (records generation 0), sibling CMP1-P2's
    /// clean escape then bumps CMP1 to generation 1, CMP3-P1 fails
    /// last (records 0; its escape was strict-DRC-reverted). Pass 2
    /// then shows BOTH arms live: CMP1-P1 RETRIES (failed@0 ≠ current
    /// 1 — its component escaped after its failure) and fails again;
    /// CMP3-P1 is SKIPPED (0 == 0 — one pin, its generation provably
    /// never moves): one pin_failed / one fanout_via_reverted row for
    /// it across BOTH passes, where pre-#933 it re-attempted and
    /// re-reverted every pass.
    #[test]
    fn fanout_skips_failed_pins_until_component_escapes() {
        let (mut manager, mut board) = parse(DRC_DSN);
        let mut settings = fanout_settings("outer_first");
        settings.router_settings.fanout.max_passes = 2;
        let mut sink = CaptureDriverSink::default();
        let summary = fanout_board(&mut manager, &mut board, &settings, None, None, &mut sink);
        let trace = sink.joined("trace");
        assert_eq!(
            summary.completed_pass_count, 2,
            "pass 2 runs; routed=0 ends it"
        );
        assert_eq!(trace.matches("pass_start pass=").count(), 2, "{trace}");
        // Pass 1 is the one-pass face: P2 routes, P1 and CMP3-P1 fail.
        assert_eq!(
            trace.matches("pin_routed pin=CMP1-P2").count(),
            1,
            "{trace}"
        );
        // THE RETRY arm: CMP1's generation moved (P2's escape) after
        // P1's failure — P1 re-attempts in pass 2 and fails again.
        assert_eq!(
            trace.matches("pin_failed pin=CMP1-P1").count(),
            2,
            "failed@0, component escaped to 1 -> retried: {trace}"
        );
        assert_eq!(
            trace.matches("pin_start pin=CMP1-P1").count(),
            2,
            "the retry is a real re-attempt: {trace}"
        );
        // THE SKIP arm: CMP3 never escapes another pin — exactly one
        // attempt + revert across both passes, NO pass-2 pin_start.
        assert_eq!(
            trace.matches("pin_failed pin=CMP3-P1").count(),
            1,
            "{trace}"
        );
        assert_eq!(trace.matches("pin_start pin=CMP3-P1").count(), 1, "{trace}");
        assert_eq!(
            trace.matches("fanout_via_reverted pin=").count(),
            1,
            "{trace}"
        );
        // Pass 2's pass_end row: the retry failed, the skip vanished,
        // P2 answered already-connected.
        assert_eq!(
            trace
                .matches("routed=0, notRouted=1, insertErrors=0, alreadyConnected=1")
                .count(),
            1,
            "the pass-2 face: {trace}"
        );
    }
}
