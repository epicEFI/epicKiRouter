//! Java `core/scoring/BoardStatistics.java` — the board counting walk
//! and the router-score faces the batch driver consumes (T12 build
//! item 7): the live-item counting ladder, the bends/via/violation
//! statistics (with the V2 `totalViolationUm` depth), the difficulty
//! scale, and the three score formulas — the legacy
//! [`Self::calculate_score`]/[`Self::get_maximum_score`]/[`Self::get_legacy_normalized_score`]
//! trio and the [`RouterScoringVersion::V2Continuous`] score.
//!
//! ## Walk parity
//!
//! Every Java collection this module reads
//! (`startReadObject`/`readObject`, `getTraces`, `getVias`, `getPins`,
//! …) funnels through `UndoableObjects.readObject`, which iterates the
//! `objects` ConcurrentSkipListMap (declared `UndoableObjects.java:21`,
//! constructed `:37`) keyed by `Item.compareTo = other.id - this.id`
//! (`Item.java:95-102`) — the DESCENDING-id order — and skips only
//! redo-only nodes; `UndoableObjects.delete` REMOVES the node from the
//! map (`objects.remove`, `UndoableObjects.java:126`), so the walks are
//! LIVE-ONLY. The Rust
//! arena keeps off-board tombstones behind `on_the_board`; the walks in
//! THIS module use [`epic_board::board::Board::iter_ascending`], which
//! yields the same LIVE elements in the OPPOSITE order — observably
//! identical here because every face is an order-independent COUNT
//! (the one f64 length SUM rounds to the same f32; the manifest
//! canaries have pinned it since M3). Order-SENSITIVE consumers must
//! walk descending like Java: see
//! [`crate::pipeline::board_statistics_bounds`] (spec-review T8
//! observation fix; the pre-T8 text wrongly claimed a TreeMap and
//! ascending order).
//!
//! ## The float/double discipline
//!
//! The legacy path is JAVA-FLOAT arithmetic end to end (f32 in Rust);
//! the V2 path widens every input to f64 and computes in f64. The V2
//! weight defaults `1000.0f/3.0f` and `2000.0f/3.0f` are DIVIDED IN
//! f32 first and widened after (Java evaluates the `float` default
//! argument, then widens on assignment to `double`) — see
//! [`Self::get_v2_router_score`].
//!
//! ## Banks (SEAM carries the dossier)
//!
//! * Skipped diagnostics: `host`, board bounding boxes, `areaCm2`,
//!   fanout, per-segment H/V/A lengths and the weighted trace length
//!   — output-serialization fields no M3 surface reads (the score
//!   consumes counts, `totalLength(Mm)`, bends, vias, connections,
//!   violations and difficulty only). The `bounds` capture and the
//!   optimizer score live in
//!   [`crate::pipeline::board_statistics_bounds`] and the
//!   [`Self::get_optimizer_score`]/[`Self::get_v2_optimizer_score`]
//!   pair below (M4-T8).
//! * The `transient double[]` preferred/undesired per-layer cost
//!   tables of `RoutingCostSettings` are not part of the score face.
//! * The V2 ROUTER score's `ensureDifficulty()` call (`:689`) is a
//!   no-op through the counting ctor (it always writes `pinCount` /
//!   `signalLayerCount` / `complexityC` / `difficultyD`); the method
//!   itself is now ported as [`Self::ensure_difficulty`] — its
//!   GSON-defensive fallback arms stay reachable only through
//!   [`BoardStatistics::new_empty`] (the port has no
//!   JSON-deserialized statistics).
//! * `BasicBoard.preExistingClearanceViolationsCount` is set once at
//!   load by Java's `HeadlessBoardManager` (`:789-793`); the load-side
//!   write is the headless-manager port's business (M5+), the field
//!   itself lives on [`epic_board::board::Board`] since this module is
//!   its first reader.

use crate::pipeline::board_statistics_bounds::BoardStatisticsBounds;
use epic_board::board::Board;
use epic_board::items::ItemData;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::all_clearance_violation_depths;
use epic_drc::incompletes::all_incompletes;
use epic_dsn::state::Unit;

// ---------------------------------------------------------------------------
// the scoring settings types (Java's nullable settings boxes)
// ---------------------------------------------------------------------------

/// Java `settings.RouterScoringVersion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterScoringVersion {
    /// Java `V1_LEGACY` — the combined completion score.
    V1Legacy,
    /// Java `V2_CONTINUOUS` — completion plus continuous DRC penalties.
    V2Continuous,
}

/// Java `settings.RoutingCostSettings` — the scoring scalars (the
/// transient per-layer cost arrays are not part of this face).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingCostSettings {
    /// Java `defaultPreferredDirectionTraceCost`.
    pub default_preferred_direction_trace_cost: Option<f64>,
    /// Java `defaultUndesiredDirectionTraceCost`.
    pub default_undesired_direction_trace_cost: Option<f64>,
    /// Java `viaCosts`.
    pub via_costs: Option<i32>,
    /// Java `planeViaCosts`.
    pub plane_via_costs: Option<i32>,
    /// Java `startRipupCosts`.
    pub start_ripup_costs: Option<i32>,
    /// Java `defaultBendCost`.
    pub default_bend_cost: Option<f64>,
    /// Java `unroutedNetPenalty`.
    pub unrouted_net_penalty: Option<f32>,
    /// Java `clearanceViolationPenalty`.
    pub clearance_violation_penalty: Option<f32>,
    /// Java `bendPenalty`.
    pub bend_penalty: Option<f32>,
}

/// Java `settings.RouterScoreSettings` — the V2_CONTINUOUS weights
/// (the fields V2 reads; `unroutedConnectionWeight` is V1-only).
/// No `Default`: the version selector is a required field (Java's
/// deserialized box always carries it or the gate falls back to the
/// legacy face).
#[derive(Clone, Debug, PartialEq)]
pub struct RouterScoreSettings {
    /// Java `RouterScoreSettings.version` — the formula selector the
    /// [`BoardStatistics::get_router_score`] gate reads.
    pub version: RouterScoringVersion,
    /// Java `unroutedFreeFraction` — the split between the first-half
    /// and second-half unrouted penalties.
    pub unrouted_free_fraction: Option<f32>,
    /// Java `unroutedFirstHalfWeight`.
    pub unrouted_first_half_weight: Option<f32>,
    /// Java `unroutedSecondHalfWeight`.
    pub unrouted_second_half_weight: Option<f32>,
    /// Java `clearanceViolationCountWeight`.
    pub clearance_violation_count_weight: Option<f32>,
    /// Java `clearanceViolationDepthWeight`.
    pub clearance_violation_depth_weight: Option<f32>,
    /// Java `clearanceViolationDepthScale`.
    pub clearance_violation_depth_scale: Option<f32>,
}

/// The slice of Java `RouterSettings` the score faces read:
/// `scoring` (the legacy cost box) and `routerScoring` (the V2 box).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouterSettingsScoring {
    /// Java `RouterSettings.scoring`.
    pub scoring: Option<RoutingCostSettings>,
    /// Java `RouterSettings.routerScoring`.
    pub router_scoring: Option<RouterScoreSettings>,
    /// Java `RouterSettings.optimizerScoring` (`@SerializedName
    /// "optimizer_scoring"`) — the optimizer-score box. `None` mirrors
    /// Java's null group (the V2 gate fails and the legacy face runs);
    /// the CLI resolver materializes the group over the DefaultSettings
    /// seeds, so production carries `Some` with V2_LOWER_BOUND.
    pub optimizer_scoring: Option<OptimizerScoreSettings>,
}

/// Java `DefaultSettings().getSettings().scoring`
/// (`sources/DefaultSettings.java:29-77, 192-204`) — the fallback the
/// legacy score reads when the caller carries no scoring box.
#[must_use]
pub fn default_routing_cost_settings() -> RoutingCostSettings {
    RoutingCostSettings {
        default_preferred_direction_trace_cost: Some(1.0),
        default_undesired_direction_trace_cost: Some(1.0),
        via_costs: Some(50),
        plane_via_costs: Some(5),
        start_ripup_costs: Some(100),
        default_bend_cost: Some(0.0),
        unrouted_net_penalty: Some(5_000_000.0),
        clearance_violation_penalty: Some(1_000_000.0),
        bend_penalty: Some(10.0),
    }
}

/// Java `BoardStatistics.valueOrDefault(Float, float)`
/// (`BoardStatistics.java:787-789`).
fn value_or_default(value: Option<f32>, default_value: f32) -> f32 {
    value.unwrap_or(default_value)
}

// ---------------------------------------------------------------------------
// the optimizer-score settings types (Java's nullable settings boxes)
// ---------------------------------------------------------------------------

/// Java `settings.OptimizerScoringVersion` — the optimizer-board score
/// formula selector. Exactly `V1_LEGACY` and `V2_LOWER_BOUND` (no
/// V2_CONTINUOUS on this box — the fact that makes the CLI `v2` alias
/// asymmetric across the two scoring boxes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizerScoringVersion {
    /// Java `V1_LEGACY`.
    V1Legacy,
    /// Java `V2_LOWER_BOUND`.
    V2LowerBound,
}

/// Java `settings.OptimizerScoreSettings` — the nullable settings box
/// the V2 optimizer score reads. Every weight/floor is Java `Float`
/// (null → the inline default at the read site); the version is the
/// formula selector the [`BoardStatistics::get_optimizer_score`] gate
/// reads. No `Default` (a second defaults source would duplicate the
/// CLI resolver's `default_optimizer_score_settings` — the
/// settings-merger trap; CLAUDE.md).
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizerScoreSettings {
    /// Java `OptimizerScoreSettings.version`.
    pub version: OptimizerScoringVersion,
    /// Java `excessWireLengthWeight`.
    pub excess_wire_length_weight: Option<f32>,
    /// Java `excessViaWeight`.
    pub excess_via_weight: Option<f32>,
    /// Java `excessBendWeight`.
    pub excess_bend_weight: Option<f32>,
    /// Java `lengthFloor`.
    pub length_floor: Option<f32>,
    /// Java `difficultyScaleFloor`.
    pub difficulty_scale_floor: Option<f32>,
}

// ---------------------------------------------------------------------------
// the counting structs (Java's nested BoardStatistics* classes)
// ---------------------------------------------------------------------------

/// Java `BoardStatisticsItems`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemsCounts {
    /// Java `totalCount`.
    pub total_count: i32,
    /// Java `traceCount`.
    pub trace_count: i32,
    /// Java `viaCount`.
    pub via_count: i32,
    /// Java `conductionAreaCount`.
    pub conduction_area_count: i32,
    /// Java `drillItemCount` (Via and Pin are caught earlier in the
    /// ladder, so the ctor never increments it — ported verbatim).
    pub drill_item_count: i32,
    /// Java `pinCount`.
    pub pin_count: i32,
    /// Java `componentOutlineCount`.
    pub component_outline_count: i32,
    /// Java `otherCount` (BoardOutline and the non-routed residue).
    pub other_count: i32,
}

/// Java `BoardStatisticsConnections`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionsCounts {
    /// Java `maximumCount` — the endpoint-sum lower bound
    /// (`DesignRulesChecker.maxConnections`). `None` mirrors the
    /// skipped-connections face (Java's null).
    pub maximum_count: Option<i32>,
    /// Java `incompleteCount` — the airline total
    /// (`DesignRulesChecker.getIncompleteCount`).
    pub incomplete_count: Option<i32>,
}

/// Java `BoardStatisticsTraces` (the consumed subset; the per-segment
/// H/V/A lengths and the weighted length are banked diagnostics).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracesCounts {
    /// Java `totalCount`.
    pub total_count: i32,
    /// Java `totalLength` — the f64 length sum, rounded to f32.
    pub total_length: f32,
    /// Java `totalLengthMm` — `(float) (totalLength * unitToMmFactor)`.
    pub total_length_mm: Option<f32>,
    /// Java `averageLength` — `totalLength / totalCount` (0 when
    /// empty).
    pub average_length: f32,
}

/// Java `BoardStatisticsBends`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BendsCounts {
    /// Java `totalCount` — `cornerCount - 2` per trace.
    pub total_count: i32,
    /// Java `ninetyDegreeCount` (|angle − 90| < 1).
    pub ninety_degree_count: i32,
    /// Java `fortyFiveDegreeCount` (|angle − 45| < 1 or
    /// |angle − 135| < 1).
    pub forty_five_degree_count: i32,
    /// Java `otherAngleCount`.
    pub other_angle_count: i32,
}

/// Java `BoardStatisticsVias`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViasCounts {
    /// Java `totalCount`.
    pub total_count: i32,
    /// Java `throughHoleCount` (first == 0 && last == layerCount-1).
    pub through_hole_count: i32,
    /// Java `blindCount` (exactly one of the two ends).
    pub blind_count: i32,
    /// Java `buriedCount` (neither end exposed).
    pub buried_count: i32,
}

/// Java `BoardStatisticsClearanceViolations`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClearanceViolationsStats {
    /// Java `totalCount`.
    pub total_count: Option<i32>,
    /// Java `preExistingCount`.
    pub pre_existing_count: i32,
    /// Java `routerIntroducedCount` — `max(0, total - preExisting)`.
    pub router_introduced_count: i32,
    /// Java `totalViolationUm` — the shortfall sum in micrometres
    /// (the V2 depth input).
    pub total_violation_um: Option<f64>,
    /// Java `minViolationUm`.
    pub min_violation_um: Option<f64>,
    /// Java `maxViolationUm`.
    pub max_violation_um: Option<f64>,
    /// Java `avgViolationUm`.
    pub avg_violation_um: Option<f64>,
}

/// Java `BoardStatisticsDifficulty`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DifficultyStats {
    /// Java `pinCount`.
    pub pin_count: i32,
    /// Java `signalLayerCount`.
    pub signal_layer_count: i32,
    /// Java `complexityC` — `max(1, pinCount * signalLayerCount)`.
    pub complexity_c: i32,
    /// Java `difficultyD` — `(float) complexityC` (the V2 scale input).
    pub difficulty_d: Option<f32>,
}

/// Java `BoardStatistics` — the counting + scoring aggregate. Construct
/// through [`Self::with_options`] (the counting walk); [`Self::new_empty`]
/// is the GSON-empty analog for score-unit tests only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatistics {
    /// Java `layers.totalCount`.
    pub layers_total_count: i32,
    /// Java `layers.signalCount`.
    pub layers_signal_count: i32,
    /// Java `items`.
    pub items: ItemsCounts,
    /// Java `components.totalCount`.
    pub components_total_count: i32,
    /// Java `pads.totalCount`.
    pub pads_total_count: i32,
    /// Java `nets.totalCount` (`rules.nets.maxNetNumber()`).
    pub nets_total_count: i32,
    /// Java `nets.classCount`.
    pub nets_class_count: i32,
    /// Java `connections`.
    pub connections: ConnectionsCounts,
    /// Java `traces`.
    pub traces: TracesCounts,
    /// Java `bends`.
    pub bends: BendsCounts,
    /// Java `vias`.
    pub vias: ViasCounts,
    /// Java `clearanceViolations`.
    pub clearance_violations: ClearanceViolationsStats,
    /// Java `difficulty`.
    pub difficulty: DifficultyStats,
    /// Java `bounds` (`@SerializedName("bounds")`) — the V2 optimizer
    /// score's board-only lower bounds, captured by the ctor from
    /// [`crate::pipeline::board_statistics_bounds::calculate`]. Java
    /// memoizes that capture per board (`WeakHashMap`), so the value is
    /// the load-time face even on a routed board; the port's ctor call
    /// reproduces it (routing never moves terminals).
    pub bounds: BoardStatisticsBounds,
}

impl BoardStatistics {
    /// Java `new BoardStatistics(board)` (`:91-93`) — the FULL
    /// counting walk including the clearance-violation pass.
    pub fn new(manager: &mut SearchTreeManager, board: &mut Board) -> Self {
        Self::with_options(manager, board, true, true)
    }

    /// Java `BoardStatistics(board, unit, includeClearanceViolations,
    /// includeConnections)` (`:109-115`) — the counting walk with the
    /// two expensive passes switchable. The `unit` parameter of the
    /// Java ctor only steers the serialization string; the mm/um
    /// factors are board-unit factors read from
    /// `board.communication` regardless (see the ctor bodies).
    pub fn with_options(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        include_clearance_violations: bool,
        include_connections: bool,
    ) -> Self {
        let mut stats = BoardStatistics::new_empty();

        // Layers.
        stats.layers_total_count =
            i32::try_from(board.layers().layers.len()).expect("layer count fits i32");
        stats.layers_signal_count = i32::try_from(
            board
                .layers()
                .layers
                .iter()
                .filter(|layer| layer.is_signal)
                .count(),
        )
        .expect("signal layer count fits i32");

        // Communication-derived unit factors. Java:
        // `Unit.scale(1.0, unit, MM|UM) / (resolution > 0 ? resolution : 1)`.
        let communication = board.communication();
        let resolution = i64::from(communication.resolution);
        let resolution = if resolution > 0 { resolution } else { 1 };
        let board_unit_to_mm_factor =
            Unit::scale(1.0, communication.unit, Unit::Mm) / resolution as f64;
        let board_unit_to_um_factor =
            Unit::scale(1.0, communication.unit, Unit::Um) / resolution as f64;

        // The live-item walk (Java `itemList.startReadObject`): every
        // readObject collection is live-only and ascending-id ordered —
        // see the module docs. The instanceof ladder order is
        // load-bearing: Trace, Via, ConductionArea, Pin, DrillItem,
        // ComponentOutline, else.
        let mut live_traces: Vec<epic_board::id::ItemId> = Vec::new();
        let mut live_vias: Vec<epic_board::id::ItemId> = Vec::new();
        for entry in board.iter_ascending() {
            if !entry.on_the_board {
                // Java's `UndoableObjects.delete` removed the node from
                // the map; the arena keeps the tombstone invisible.
                continue;
            }
            stats.items.total_count += 1;
            match &entry.data {
                ItemData::Trace { .. } => {
                    stats.items.trace_count += 1;
                    live_traces.push(entry.id);
                }
                ItemData::Via { .. } => {
                    stats.items.via_count += 1;
                    live_vias.push(entry.id);
                }
                ItemData::ConductionArea { .. } => {
                    stats.items.conduction_area_count += 1;
                }
                ItemData::Pin { .. } => {
                    stats.items.pin_count += 1;
                }
                // The DrillItem rung is shadowed by Via/Pin above —
                // Java's ladder has the same property.
                ItemData::ObstacleArea { .. } | ItemData::BoardOutline { .. } | ItemData::Other => {
                    stats.items.other_count += 1;
                }
                ItemData::ComponentOutline { .. } => {
                    stats.items.component_outline_count += 1;
                }
            }
        }

        stats.components_total_count =
            i32::try_from(board.components().count()).expect("component count fits i32");
        stats.pads_total_count = stats.items.pin_count;
        stats.nets_total_count = board.rules().nets.max_net_number();
        stats.nets_class_count =
            i32::try_from(board.rules().net_classes.len()).expect("class count fits i32");

        // Traces: the f64 length sum (ascending id), rounded to f32;
        // the mm normalization multiplies the ROUNDED f32 value
        // (Java widens the Float field, not the raw f64 sum).
        let total_length_f64: f64 = live_traces
            .iter()
            .filter_map(|&id| board.trace_polyline(id))
            .map(epic_geometry::polyline::Polyline::length_approx_total)
            .sum();
        stats.traces.total_count = i32::try_from(live_traces.len()).expect("trace count fits i32");
        stats.traces.total_length = total_length_f64 as f32;
        stats.traces.total_length_mm =
            Some(((f64::from(stats.traces.total_length)) * board_unit_to_mm_factor) as f32);
        stats.traces.average_length = if stats.traces.total_count > 0 {
            stats.traces.total_length / stats.traces.total_count as f32
        } else {
            0.0
        };

        // Difficulty.
        stats.difficulty.pin_count = stats.items.pin_count;
        stats.difficulty.signal_layer_count = stats.layers_signal_count;
        stats.difficulty.complexity_c = 1.max(
            stats
                .difficulty
                .pin_count
                .saturating_mul(stats.difficulty.signal_layer_count),
        );
        stats.difficulty.difficulty_d = Some(stats.difficulty.complexity_c as f32);

        // Bounds (Java: `this.bounds = BoardStatisticsBoundsCalculator
        // .calculate(board)`) — the load-time capture (see the bounds
        // module docs for the memoization note).
        stats.bounds = crate::pipeline::board_statistics_bounds::calculate(board);

        // Connections (the incompletes pass).
        if include_connections {
            let (max_connections, rows) = all_incompletes(manager, board);
            stats.connections.maximum_count =
                Some(i32::try_from(max_connections).expect("max connections fits i32"));
            let incomplete: usize = rows.iter().map(|row| row.incomplete_count).sum();
            stats.connections.incomplete_count =
                Some(i32::try_from(incomplete).expect("incomplete count fits i32"));
        }

        // Bends: `cornerCount >= 3` → `cornerCount - 2` bends,
        // classified by the interior-corner turn angle.
        for &id in &live_traces {
            let Some(polyline) = board.trace_polyline(id) else {
                continue;
            };
            let corner_count = polyline.corner_count();
            if corner_count < 3 {
                continue;
            }
            let bends_in_trace = i64::try_from(corner_count).expect("corner count fits i64") - 2;
            stats.bends.total_count = stats
                .bends
                .total_count
                .saturating_add(i32::try_from(bends_in_trace).expect("bends fit i32"));
            let corners = polyline.corners();
            for i in 1..(corner_count - 1) {
                let prev = corners[i - 1].to_float();
                let current = corners[i].to_float();
                let next = corners[i + 1].to_float();
                // Java `double dx1 = current.x - prev.x` — the atan2
                // runs in f64 (Math.atan2); the port's corners are
                // already f64 after `to_float()`.
                let dx1 = current.x - prev.x;
                let dy1 = current.y - prev.y;
                let dx2 = next.x - current.x;
                let dy2 = next.y - current.y;
                let mut angle = (dy2.atan2(dx2) - dy1.atan2(dx1)).to_degrees().abs();
                angle = angle.min(360.0 - angle);
                if angle > 180.0 {
                    angle = 360.0 - angle;
                }
                if (angle - 90.0).abs() < 1.0 {
                    stats.bends.ninety_degree_count += 1;
                } else if (angle - 45.0).abs() < 1.0 || (angle - 135.0).abs() < 1.0 {
                    stats.bends.forty_five_degree_count += 1;
                } else {
                    stats.bends.other_angle_count += 1;
                }
            }
        }

        // Vias: the through/blind/buried ladder over the drill-layer
        // span (Java `DrillItem.firstLayer/lastLayer`).
        stats.vias.total_count = i32::try_from(live_vias.len()).expect("via count fits i32");
        let last_layer_index = stats.layers_total_count - 1;
        for &id in &live_vias {
            let first = board
                .drill_first_layer(id)
                .expect("a live via carries a padstack");
            let last = board
                .drill_last_layer(id)
                .expect("a live via carries a padstack");
            if first == 0 && last == last_layer_index {
                stats.vias.through_hole_count += 1;
            } else if first == 0 || last == last_layer_index {
                stats.vias.blind_count += 1;
            } else {
                stats.vias.buried_count += 1;
            }
        }

        // Clearance violations: the walk-ordered depth rows; the
        // min/max/sum run in Java's collection order (walk order, NOT
        // the canonical sort).
        if include_clearance_violations {
            let (total, rows) = all_clearance_violation_depths(manager, board);
            stats.clearance_violations.total_count =
                Some(i32::try_from(total).expect("violation count fits i32"));
            if !rows.is_empty() {
                let mut min_violation = f64::MAX;
                let mut max_violation = 0.0f64;
                let mut sum_violation = 0.0;
                for row in &rows {
                    let shortfall = (row.expected_clearance - row.actual_clearance).max(0.0);
                    let shortfall_um = shortfall * board_unit_to_um_factor;
                    min_violation = min_violation.min(shortfall_um);
                    max_violation = max_violation.max(shortfall_um);
                    sum_violation += shortfall_um;
                }
                stats.clearance_violations.total_violation_um = Some(sum_violation);
                stats.clearance_violations.min_violation_um = Some(min_violation);
                stats.clearance_violations.max_violation_um = Some(max_violation);
                stats.clearance_violations.avg_violation_um =
                    Some(sum_violation / rows.len() as f64);
            } else {
                stats.clearance_violations.total_violation_um = Some(0.0);
                stats.clearance_violations.min_violation_um = Some(0.0);
                stats.clearance_violations.max_violation_um = Some(0.0);
                stats.clearance_violations.avg_violation_um = Some(0.0);
            }
            stats.clearance_violations.pre_existing_count =
                board.pre_existing_clearance_violations_count;
            stats.clearance_violations.router_introduced_count = 0.max(
                stats.clearance_violations.total_count.unwrap_or(0)
                    - board.pre_existing_clearance_violations_count,
            );
        } else {
            stats.clearance_violations.total_count = Some(0);
            stats.clearance_violations.pre_existing_count = 0;
            stats.clearance_violations.router_introduced_count = 0;
            stats.clearance_violations.total_violation_um = Some(0.0);
            stats.clearance_violations.min_violation_um = Some(0.0);
            stats.clearance_violations.max_violation_um = Some(0.0);
            stats.clearance_violations.avg_violation_um = Some(0.0);
        }

        stats
    }

    /// Java `new BoardStatistics()` (`:89-90`) — the all-empty
    /// aggregate. Production code constructs through
    /// [`Self::with_options`]; this face exists for the score-arm unit
    /// tests (and mirrors Java's GSON-empty nullability, where the V2
    /// null-guard arms are actually reachable).
    #[must_use]
    pub fn new_empty() -> Self {
        BoardStatistics {
            layers_total_count: 0,
            layers_signal_count: 0,
            items: ItemsCounts::default(),
            components_total_count: 0,
            pads_total_count: 0,
            nets_total_count: 0,
            nets_class_count: 0,
            connections: ConnectionsCounts::default(),
            traces: TracesCounts::default(),
            bends: BendsCounts::default(),
            vias: ViasCounts::default(),
            clearance_violations: ClearanceViolationsStats {
                total_count: None,
                pre_existing_count: 0,
                router_introduced_count: 0,
                total_violation_um: None,
                min_violation_um: None,
                max_violation_um: None,
                avg_violation_um: None,
            },
            difficulty: DifficultyStats {
                pin_count: 0,
                signal_layer_count: 0,
                complexity_c: 0,
                difficulty_d: None,
            },
            bounds: BoardStatisticsBounds::empty(),
        }
    }

    /// Java `calculateScore(RoutingCostSettings)` (`:621-655`) — the
    /// legacy raw score; HIGHER is better. The null unboxes are Java
    /// NPEs (unreachable through the counting ctor, which sets every
    /// field).
    #[must_use]
    pub fn calculate_score(&self, scoring: &RoutingCostSettings) -> f32 {
        let maximum_score = self.get_maximum_score(scoring);
        let penalties = self
            .connections
            .incomplete_count
            .expect("Java NPE parity: incompleteCount") as f32
            * scoring
                .unrouted_net_penalty
                .expect("Java NPE parity: unroutedNetPenalty")
            + self
                .clearance_violations
                .total_count
                .expect("Java NPE parity: totalCount") as f32
                * scoring
                    .clearance_violation_penalty
                    .expect("Java NPE parity: clearanceViolationPenalty")
            + self.bends.total_count as f32
                * scoring.bend_penalty.expect("Java NPE parity: bendPenalty");
        // The mm-normalized length keeps the trace-cost term comparable
        // to the net penalty across DSN resolutions (the Java comment).
        let trace_length_for_cost = self
            .traces
            .total_length_mm
            .unwrap_or(self.traces.total_length);
        // Java's second term is an INT product (totalCount * viaCosts)
        // promoted on the addition — not a double product.
        let costs = (f64::from(trace_length_for_cost)
            * scoring
                .default_preferred_direction_trace_cost
                .expect("Java NPE parity: defaultPreferredDirectionTraceCost")
            + f64::from(
                self.vias.total_count * scoring.via_costs.expect("Java NPE parity: viaCosts"),
            )) as f32;
        maximum_score - penalties - costs
    }

    /// Java `getMaximumScore(RoutingCostSettings)` (`:660-663`) —
    /// int * float, evaluated in f32.
    #[must_use]
    pub fn get_maximum_score(&self, scoring: &RoutingCostSettings) -> f32 {
        self.connections
            .maximum_count
            .expect("Java NPE parity: maximumCount") as f32
            * scoring
                .unrouted_net_penalty
                .expect("Java NPE parity: unroutedNetPenalty")
    }

    /// Java `getLegacyNormalizedScore(RoutingCostSettings)`
    /// (`:668-681`) — `max(0, calculate/maximum) * 1000`, with the
    /// `maximum <= 0` guard (no connections / negative maximum).
    #[must_use]
    pub fn get_legacy_normalized_score(&self, scoring: &RoutingCostSettings) -> f32 {
        let maximum_score = self.get_maximum_score(scoring);
        if maximum_score <= 0.0 {
            return 0.0;
        }
        (self.calculate_score(scoring) / maximum_score).max(0.0) * 1000.0
    }

    /// Java `getRouterScore(RouterSettings)` (`:666-686`): the V2
    /// formula when the settings carry a `V2_CONTINUOUS`
    /// `routerScoring` box, the legacy normalized score over
    /// `scoring`-or-`DefaultSettings` otherwise.
    #[must_use]
    pub fn get_router_score(&self, router_settings: Option<&RouterSettingsScoring>) -> f32 {
        let v2 = router_settings
            .and_then(|settings| settings.router_scoring.as_ref())
            .filter(|score_settings| score_settings.version == RouterScoringVersion::V2Continuous);
        if let Some(score_settings) = v2 {
            return self.get_v2_router_score(score_settings);
        }
        match router_settings.and_then(|settings| settings.scoring.as_ref()) {
            Some(scoring) => self.get_legacy_normalized_score(scoring),
            None => self.get_legacy_normalized_score(&default_routing_cost_settings()),
        }
    }

    /// Java `getV2RouterScore(RouterScoreSettings)` (`:688-737`) —
    /// completion split over the free-fraction plus the continuous
    /// clearance count/depth penalties, all f64, clamped at 0.
    #[must_use]
    pub fn get_v2_router_score(&self, settings: &RouterScoreSettings) -> f32 {
        // ensureDifficulty() is a no-op here: the counting ctor always
        // set difficultyD (module docs).
        let difficulty = match self.difficulty.difficulty_d {
            Some(difficulty_d) => 1.0f64.max(f64::from(difficulty_d)),
            None => 1.0,
        };
        let connections = f64::from(self.connections.maximum_count.map_or(0, |v| v.max(0)));
        let incomplete = f64::from(self.connections.incomplete_count.map_or(0, |v| v.max(0)));
        let violation_count = f64::from(
            self.clearance_violations
                .total_count
                .map_or(0, |v| v.max(0)),
        );
        let violation_depth = self
            .clearance_violations
            .total_violation_um
            .map_or(0.0, |v| v.max(0.0));
        let split =
            f64::from(value_or_default(settings.unrouted_free_fraction, 0.5)).clamp(0.0, 1.0);
        let first_half_weight = f64::from(value_or_default(
            settings.unrouted_first_half_weight,
            1000.0f32 / 3.0f32,
        ));
        let second_half_weight = f64::from(value_or_default(
            settings.unrouted_second_half_weight,
            2000.0f32 / 3.0f32,
        ));
        let open_fraction = if connections > 0.0 {
            incomplete / connections
        } else {
            0.0
        };
        let first_half_open;
        let second_half_open;
        if connections <= 0.0 {
            first_half_open = 0.0;
            second_half_open = 0.0;
        } else if split <= 0.0 {
            first_half_open = 0.0;
            second_half_open = open_fraction;
        } else if split >= 1.0 {
            first_half_open = open_fraction;
            second_half_open = 0.0;
        } else {
            first_half_open = ((open_fraction - split) / (1.0 - split)).clamp(0.0, 1.0);
            second_half_open = (open_fraction / split).min(1.0);
        }
        let unrouted_penalty =
            first_half_weight * first_half_open + second_half_weight * second_half_open;
        let mut drc_penalty = f64::from(value_or_default(
            settings.clearance_violation_count_weight,
            25.0,
        )) * violation_count
            / difficulty;
        let depth_scale = 1.0f64.max(f64::from(value_or_default(
            settings.clearance_violation_depth_scale,
            1000.0,
        )));
        drc_penalty += f64::from(value_or_default(
            settings.clearance_violation_depth_weight,
            300.0,
        )) * violation_depth
            / depth_scale
            / difficulty;
        (1000.0 - unrouted_penalty - drc_penalty).max(0.0) as f32
    }
}

// ---------------------------------------------------------------------------
// the optimizer score (Java `getOptimizerScore`/`getV2OptimizerScore`)
// ---------------------------------------------------------------------------

impl BoardStatistics {
    /// Java `ensureDifficulty` (`BoardStatistics.java:743-778`) — fills
    /// the difficulty ladder over the (possibly GSON-empty) aggregate:
    /// pin count ← items.pinCount, else pads.totalCount, else 0;
    /// signal layer count ← layers.signalCount, else layers.totalCount,
    /// else 0; complexity C ← max(1, pins·layers) when ≤ 0; D ← C when
    /// D is null. The port spells the Java null faces of the three int
    /// fields as `<= 0` (the counting ctor always writes them, and the
    /// empty aggregate's 0 plays the null role).
    pub fn ensure_difficulty(&mut self) {
        // Java `:747-749` — the early return: a stats that already
        // carries D is a FULL no-op (Java keeps the FIRST computation;
        // spec-review MINOR-3: load-bearing for T9's repeated calls on
        // evolving boards — a recompute-on-every-call port diverges).
        // The `difficulty == null` guard above it is structural here
        // (the aggregate is never null).
        if self.difficulty.difficulty_d.is_some() {
            return;
        }
        if self.difficulty.pin_count <= 0 {
            if self.items.pin_count > 0 {
                self.difficulty.pin_count = self.items.pin_count;
            } else if self.pads_total_count > 0 {
                self.difficulty.pin_count = self.pads_total_count;
            } else {
                self.difficulty.pin_count = 0;
            }
        }
        if self.difficulty.signal_layer_count <= 0 {
            if self.layers_signal_count > 0 {
                self.difficulty.signal_layer_count = self.layers_signal_count;
            } else if self.layers_total_count > 0 {
                self.difficulty.signal_layer_count = self.layers_total_count;
            } else {
                self.difficulty.signal_layer_count = 0;
            }
        }
        if self.difficulty.complexity_c <= 0 {
            let pins = self.difficulty.pin_count;
            let layers = self.difficulty.signal_layer_count;
            // House convention (matches the M3 counting ctor above):
            // saturating_mul where Java's int `pins * layers` wraps.
            // Unreachable on any real board (overflow needs
            // pins*layers > 2^31; layers <= a few dozen, so pins
            // > 33M); if ever reached, Java's face is
            // `max(1, wrapped-negative) = 1` — materially different,
            // documented rather than "fixed" (quality-review NIT-3).
            self.difficulty.complexity_c = 1.max(pins.saturating_mul(layers));
        }
        if self.difficulty.difficulty_d.is_none() {
            self.difficulty.difficulty_d = Some(self.difficulty.complexity_c as f32);
        }
    }

    /// Java `getOptimizerScore(RouterSettings)` (`:795-806`): the V2
    /// formula when the settings carry a `V2_LOWER_BOUND`
    /// `optimizerScoring` box, the legacy normalized score over
    /// `scoring`-or-`DefaultSettings.scoring` otherwise
    /// (`legacyScoringOrDefault`, `:780-785`).
    #[must_use]
    pub fn get_optimizer_score(&self, router_settings: Option<&RouterSettingsScoring>) -> f32 {
        let v2 = router_settings
            .and_then(|settings| settings.optimizer_scoring.as_ref())
            .filter(|score_settings| {
                score_settings.version == OptimizerScoringVersion::V2LowerBound
            });
        if let Some(score_settings) = v2 {
            return self.get_v2_optimizer_score(score_settings);
        }
        match router_settings.and_then(|s| s.scoring.as_ref()) {
            Some(scoring) => self.get_legacy_normalized_score(scoring),
            None => self.get_legacy_normalized_score(&default_routing_cost_settings()),
        }
    }

    /// Java `getV2OptimizerScore(OptimizerScoreSettings)` (`:808-835`)
    /// — the three excess penalties over the stored lower bounds, all
    /// f64, clamped at 0. Weight defaults widen AFTER the f32 default
    /// argument is evaluated (Java `valueOrDefault(..., 1000.0f)`);
    /// `1000.0 - lp - vp - bp` associates LEFT (Java evaluation order);
    /// the clamp runs on the f64 sum BEFORE the f32 cast.
    ///
    /// # Non-finite inputs (documented divergence, quality-review
    /// MINOR-1 — do not "fix" silently either way)
    ///
    /// Rust `f64::max` returns the non-NaN operand; Java
    /// `Math.max(double,double)` propagates NaN. Both parsers ACCEPT
    /// NaN coordinate text (Rust `text.parse::<f64>()`,
    /// `epic-dsn/src/lexer.rs:395`; Java `Double.valueOf(yytext())`,
    /// `io.specctra.parser.SpecctraDsnStreamReader.java:1487`), so a
    /// crafted DSN with `nan` coordinates poisons the bounds
    /// accumulator on BOTH sides identically (`<`/`+=`/`abs` are
    /// NaN-identical — the bounds module itself has no `max` and is
    /// NOT affected) — after which Java's score is NaN end-to-end
    /// (e.g. jar `optimizer_score: NaN`) while this port clamps each
    /// NaN term to the finite side and returns a finite number.
    /// Unreachable from real boards; NaN-propagating arms would buy
    /// strict parity on crafted worlds only, if ever wanted.
    #[must_use]
    pub fn get_v2_optimizer_score(&self, settings: &OptimizerScoreSettings) -> f32 {
        let difficulty = match self.difficulty.difficulty_d {
            Some(difficulty_d) => 1.0f64.max(f64::from(difficulty_d)),
            None => 1.0,
        };
        let min_trace_length = self
            .bounds
            .min_trace_length_mm
            .map_or(0.0, |v| 0.0f64.max(f64::from(v)));
        let min_via_count = f64::from(self.bounds.min_via_count.map_or(0, |v| v.max(0)));
        let min_bend_count = f64::from(self.bounds.min_bend_count.map_or(0, |v| v.max(0)));
        let actual_trace_length = self
            .traces
            .total_length_mm
            .map_or(0.0, |v| 0.0f64.max(f64::from(v)));
        let actual_via_count = f64::from(self.vias.total_count.max(0));
        let actual_bend_count = f64::from(self.bends.total_count.max(0));
        let length_floor = 0.0f64.max(f64::from(value_or_default(settings.length_floor, 1.0)));
        let difficulty_floor = 1.0f64.max(f64::from(value_or_default(
            settings.difficulty_scale_floor,
            1.0,
        )));
        let length_penalty =
            f64::from(value_or_default(settings.excess_wire_length_weight, 1000.0))
                * (actual_trace_length - min_trace_length).max(0.0)
                / min_trace_length.max(length_floor);
        let via_penalty = f64::from(value_or_default(settings.excess_via_weight, 2000.0))
            * (actual_via_count - min_via_count).max(0.0)
            / difficulty.max(difficulty_floor);
        let bend_penalty = f64::from(value_or_default(settings.excess_bend_weight, 500.0))
            * (actual_bend_count - min_bend_count).max(0.0)
            / difficulty.max(difficulty_floor);
        (1000.0 - length_penalty - via_penalty - bend_penalty).max(0.0) as f32
    }
}

// ---------------------------------------------------------------------------
// structural tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse;
    use epic_board::id::ItemId;
    use epic_board::items::FixedState;
    use epic_board::trace_ops::{insert_trace_without_cleaning, remove_item_through_repository};
    use epic_board::tree_manager::SearchTreeManager;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`,
    /// `resolution um 10` → 1 board unit = 0.1 µm).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn pt(x: i32, y: i32) -> epic_geometry::point::Point {
        epic_geometry::point::Point::int(IntPoint::new(x, y))
    }

    /// A polyline through consecutive corners (Java
    /// `Polyline(Point[])` — the ctor adds the perpendicular end
    /// placeholders; `Polyline::new(Line[])` would NOT).
    fn poly(corners: &[(i32, i32)]) -> Polyline {
        let points: Vec<epic_geometry::point::Point> =
            corners.iter().map(|(x, y)| pt(*x, *y)).collect();
        Polyline::from_points(&points)
    }

    fn insert(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: &[(i32, i32)],
        net: i32,
    ) -> ItemId {
        // Clearance class 1 = the fixture's real trace class
        // (`kicad_default`, matrix names ["null", "default"]; class 0 is
        // the all-zero `"null"` dummy — a class-0 insert would produce
        // zero-clearance "violations" no parsed board carries).
        insert_trace_without_cleaning(
            manager,
            board,
            poly(corners),
            0,
            1500,
            &[net],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds")
    }

    /// The V2 default settings face (the gate picks V2 by version,
    /// every weight unset → the ctor defaults).
    fn v2_default_settings() -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: None,
            router_scoring: Some(RouterScoreSettings {
                version: RouterScoringVersion::V2Continuous,
                unrouted_free_fraction: None,
                unrouted_first_half_weight: None,
                unrouted_second_half_weight: None,
                clearance_violation_count_weight: None,
                clearance_violation_depth_weight: None,
                clearance_violation_depth_scale: None,
            }),
            optimizer_scoring: None,
        }
    }

    /// The inner V2 box of [`Self::v2_default_settings`].
    fn v2_default_box() -> RouterScoreSettings {
        v2_default_settings()
            .router_scoring
            .expect("the v2 box is set")
    }

    /// The V2 split arms (Java `getV2RouterScore`, `:688-737`): the
    /// mid-range open fraction flows only through the second-half
    /// weight, 0.75 through BOTH, 1.0 saturates the clamp, the
    /// connections<=0 arm waives everything, the split 0/1 arms
    /// reroute the whole fraction, negative counts floor at 0, and
    /// the penalty clamp bottoms out at exactly 0.
    #[test]
    fn t12_score_v2_split_arms() {
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(10.0);

        // openFraction 0.25 (1 of 4): firstHalfOpen = max(0, .25-.5)/.5
        // = 0 (clamped), secondHalfOpen = .25/.5 = .5.
        stats.connections.maximum_count = Some(4);
        stats.connections.incomplete_count = Some(1);
        let score = stats.get_v2_router_score(&v2_default_box());
        let fhw = f64::from(1000.0f32 / 3.0f32);
        let shw = f64::from(2000.0f32 / 3.0f32);
        let expected = (1000.0 - shw * 0.5).max(0.0) as f32;
        assert_eq!(score, expected, "open 0.25 flows through the second half");

        // openFraction 0.75: fho = (.75-.5)/.5 = .5, sho = .75/.5 = 1.5
        // → min 1.
        stats.connections.incomplete_count = Some(3);
        let score = stats.get_v2_router_score(&v2_default_box());
        let expected = (1000.0 - fhw * 0.5 - shw * 1.0).max(0.0) as f32;
        assert_eq!(score, expected, "open 0.75 flows through both halves");

        // openFraction 1.0 + a violation: the clamp bottoms at 0.
        stats.connections.incomplete_count = Some(4);
        stats.clearance_violations.total_count = Some(1);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 0.0, "saturated unrouted + violation clamps at 0");

        // connections <= 0: both halves waived (fully unrouted reads
        // 0, fully connected reads 1000 on a clean board).
        stats.clearance_violations.total_count = Some(0);
        stats.connections.maximum_count = Some(0);
        stats.connections.incomplete_count = Some(0);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 1000.0, "no connections: no unrouted penalty");

        // split <= 0: everything through the SECOND half.
        stats.connections.maximum_count = Some(4);
        stats.connections.incomplete_count = Some(1);
        let mut settings = v2_default_box();
        settings.unrouted_free_fraction = Some(0.0);
        let score = stats.get_v2_router_score(&settings);
        let expected = (1000.0 - shw * 0.25).max(0.0) as f32;
        assert_eq!(score, expected, "split 0: whole fraction in half two");

        // split >= 1: everything through the FIRST half.
        settings.unrouted_free_fraction = Some(1.0);
        let score = stats.get_v2_router_score(&settings);
        let expected = (1000.0 - fhw * 0.25).max(0.0) as f32;
        assert_eq!(score, expected, "split 1: whole fraction in half one");

        // Negative counts floor at 0 (Java Math.max(0, intValue)).
        stats.connections.incomplete_count = Some(-7);
        stats.clearance_violations.total_count = Some(-3);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(
            score,
            (1000.0f64).max(0.0) as f32,
            "negative counts read as 0"
        );
    }

    /// The V2 depth and difficulty faces: the count term divides by
    /// the difficulty floor (max(1, difficultyD)), the depth term
    /// divides by the depth scale floor (max(1, scale)) and the
    /// difficulty, negative depth floors at 0, and the f32-default
    /// weights widen AT THE f32 DIVISION (1000f/3f first, then f64).
    #[test]
    fn t12_v2_depth_and_difficulty() {
        let mut stats = BoardStatistics::new_empty();
        stats.connections.maximum_count = Some(10);
        stats.connections.incomplete_count = Some(0);
        stats.clearance_violations.total_count = Some(25);
        stats.clearance_violations.total_violation_um = Some(2000.0);
        // difficulty_d unset → the 1.0 floor: count term 25*25/1 = 625,
        // depth term 300*2000/1000/1 = 600 → 1000 - 1225 → clamped 0.
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 0.0, "difficulty floor 1.0, penalties saturate");

        // Only the depth: count 0, depth 2000 → 600 → score 400.
        stats.clearance_violations.total_count = Some(0);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 400.0, "depth term alone: 300*2000/1000/1");

        // difficulty_d 5.0 → depth/5: 300*2000/1000/5 = 120 → 880.
        stats.difficulty.difficulty_d = Some(5.0);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 880.0, "difficulty divides the depth term");

        // Depth scale 0.0 → the max(1, scale) floor. With depth 3 the
        // floored term is 300*3/1 = 900 → score 100; the UNFLOORED
        // mutant divides by 0.0 (+inf → clamp 0), so this face is
        // discriminating at a depth where the floored score stays
        // positive. Scale 1.0 gives the identical value (the floor is
        // invisible at 1).
        stats.difficulty.difficulty_d = None;
        stats.clearance_violations.total_violation_um = Some(3.0);
        let mut settings = v2_default_box();
        settings.clearance_violation_depth_scale = Some(0.0);
        let score = stats.get_v2_router_score(&settings);
        assert_eq!(score, 100.0, "depth-scale floor kicks in at 0");
        settings.clearance_violation_depth_scale = Some(1.0);
        let score = stats.get_v2_router_score(&settings);
        assert_eq!(score, 100.0, "floored 0 equals explicit scale 1");

        // negative depth floors at 0 (Java max(0.0, depth)).
        stats.clearance_violations.total_violation_um = Some(-50.0);
        let score = stats.get_v2_router_score(&v2_default_box());
        assert_eq!(score, 1000.0, "negative depth reads as 0");
    }

    /// The legacy faces (Java `calculateScore`/`getMaximumScore`/
    /// `getLegacyNormalizedScore`, `:621-681`): half-routed reads
    /// exactly 500, a negative raw score clamps at 0, a maximum <= 0
    /// guards the division, and the V1 version routes to the legacy
    /// face while a missing scoring box falls back to the
    /// DefaultSettings literals.
    #[test]
    fn t12_score_legacy_faces() {
        let mut stats = BoardStatistics::new_empty();
        // A ctor-built aggregate always carries the violations face
        // (the skipped walk writes 0s); only the GSON-empty analog
        // leaves it unset.
        stats.clearance_violations.total_count = Some(0);
        stats.connections.maximum_count = Some(2);
        stats.connections.incomplete_count = Some(1);
        let defaults = default_routing_cost_settings();

        // calculate = 2*5M - 1*5M = 5M; maximum 10M → 0.5 * 1000.
        assert_eq!(
            stats.get_legacy_normalized_score(&defaults),
            500.0,
            "half-routed legacy score"
        );

        // Negative raw score: fully unrouted (incomplete == maximum)
        // so the unrouted penalty already cancels the maximum, and
        // the bend penalty pushes calculate = 10M − 10M − 1000 < 0 →
        // the max(0, ratio) clamp reads 0. (Bends alone on a
        // half-routed board could never flip the sign — the penalty
        // scale is tiny against the net penalty.)
        stats.connections.incomplete_count = Some(2);
        stats.bends.total_count = 100;
        assert_eq!(
            stats.get_legacy_normalized_score(&defaults),
            0.0,
            "negative calculateScore clamps at 0"
        );

        // maximum <= 0 guard: no division-by-zero face even with
        // incompletes counted.
        stats.bends.total_count = 0;
        stats.connections.maximum_count = Some(0);
        stats.connections.incomplete_count = Some(1);
        assert_eq!(
            stats.get_legacy_normalized_score(&defaults),
            0.0,
            "maximum 0 → guard, no division"
        );

        // The gate: a V1 routerScoring box falls through to the legacy
        // face (the version filter rejects non-V2).
        let gate = RouterSettingsScoring {
            scoring: Some(defaults.clone()),
            router_scoring: Some(RouterScoreSettings {
                version: RouterScoringVersion::V1Legacy,
                unrouted_free_fraction: None,
                unrouted_first_half_weight: None,
                unrouted_second_half_weight: None,
                clearance_violation_count_weight: None,
                clearance_violation_depth_weight: None,
                clearance_violation_depth_scale: None,
            }),
            optimizer_scoring: None,
        };
        assert_eq!(
            stats.get_router_score(Some(&gate)),
            0.0,
            "V1 box → legacy face (guarded max)"
        );

        // The None fallback: DefaultSettings literals, same face.
        stats.connections.maximum_count = Some(2);
        stats.connections.incomplete_count = Some(1);
        assert_eq!(
            stats.get_router_score(None),
            500.0,
            "no settings → DefaultSettings legacy"
        );
        assert_eq!(
            stats.get_router_score(Some(&RouterSettingsScoring::default())),
            500.0,
            "empty settings → DefaultSettings legacy"
        );
    }

    /// The counting walk on the fixture cross-checked against
    /// INDEPENDENT walks (mode-11 discipline: the expected values are
    /// derived by direct iteration here, not by re-reading the same
    /// fields), plus the V2 end-to-end score faces: the fully
    /// unrouted fixture reads exactly 0 under V2 defaults, and the
    /// connections-skipped walk reads exactly 1000 on the clean board.
    #[test]
    fn t12_ctor_counts_and_v2_end_to_end() {
        let (mut manager, mut board) = parse_fixture();

        // Independent live-item walks.
        let mut live_pins = 0;
        let mut live_traces = 0;
        let mut live_total = 0;
        for entry in board.iter_ascending() {
            if !entry.on_the_board {
                continue;
            }
            live_total += 1;
            match &entry.data {
                ItemData::Pin { .. } => live_pins += 1,
                ItemData::Trace { .. } => live_traces += 1,
                _ => {}
            }
        }
        assert!(live_pins > 0 && live_total > 50, "fixture is populated");
        let max_net_number = board.rules().nets.max_net_number();
        let class_count = board.rules().net_classes.len();
        let signal_layers = board
            .layers()
            .layers
            .iter()
            .filter(|layer| layer.is_signal)
            .count();

        let stats = BoardStatistics::new(&mut manager, &mut board);
        assert_eq!(stats.layers_total_count, 2, "the fixture is 2-layer");
        assert_eq!(
            stats.layers_signal_count,
            i32::try_from(signal_layers).expect("layer count fits i32")
        );
        assert_eq!(stats.items.total_count, live_total);
        assert_eq!(stats.items.pin_count, live_pins);
        assert_eq!(stats.items.trace_count, live_traces);
        assert_eq!(stats.pads_total_count, live_pins);
        assert_eq!(stats.nets_total_count, max_net_number);
        assert_eq!(
            stats.nets_class_count,
            i32::try_from(class_count).expect("class count fits i32")
        );
        assert_eq!(stats.difficulty.pin_count, live_pins);
        assert_eq!(
            stats.difficulty.signal_layer_count,
            i32::try_from(signal_layers).expect("layer count fits i32")
        );
        assert_eq!(
            stats.difficulty.complexity_c,
            1.max(live_pins * i32::try_from(signal_layers).expect("layer count fits i32"))
        );
        // The unrouted fixture: incompletes counted, and the V2 score
        // of a fully-unrouted clean board is exactly 0 (both halves
        // saturate; the clamp does the rest).
        let incomplete = stats.connections.incomplete_count.expect("counted");
        assert!(incomplete > 0, "the fresh fixture is unrouted");
        let score = stats.get_router_score(Some(&v2_default_settings()));
        assert_eq!(score, 0.0, "fully unrouted → V2 clamp at 0");

        // The connections-and-violations-skipped walk: maximumCount
        // stays None (Java null), both skipped faces read 0 → the V2
        // score is the untouched 1000.
        let partial = BoardStatistics::with_options(&mut manager, &mut board, false, false);
        assert!(partial.connections.maximum_count.is_none());
        assert!(partial.connections.incomplete_count.is_none());
        assert_eq!(partial.clearance_violations.total_count, Some(0));
        let score = partial.get_router_score(Some(&v2_default_settings()));
        assert_eq!(
            score, 1000.0,
            "skipped connections + violations → untouched 1000"
        );
        // The violations-included walk on the BASE fixture: the
        // fixture itself carries pre-existing clearance violations
        // (its DRC is not clean), so the V2 score sits strictly below
        // 1000 by exactly the count+depth terms of those violations.
        let with_violations = BoardStatistics::with_options(&mut manager, &mut board, true, false);
        let count = f64::from(
            with_violations
                .clearance_violations
                .total_count
                .expect("counted"),
        );
        assert!(count > 0.0, "the base fixture carries violations");
        let depth = with_violations
            .clearance_violations
            .total_violation_um
            .expect("depth counted");
        let score = with_violations.get_router_score(Some(&v2_default_settings()));
        assert!(score < 1000.0, "the base violations lower the V2 score");
        // The drc terms divide by the difficulty floor
        // max(1, difficultyD) — on this 50+-pin fixture the divisor is
        // large, which is why the base score stays near 1000 despite
        // the violations. Same evaluation order as the production
        // formula ((w*c)/d, ((w*dep)/scale)/d).
        let difficulty = 1.0f64.max(f64::from(
            with_violations
                .difficulty
                .difficulty_d
                .expect("the ctor sets difficultyD"),
        ));
        let expected = (1000.0
            - f64::from(25.0f32) * count / difficulty
            - f64::from(300.0f32) * depth / f64::from(1000.0f32) / difficulty)
            .max(0.0) as f32;
        assert_eq!(
            score, expected,
            "the V2 drc term consumes the base violations"
        );
    }

    /// The bends classification (Java `:319-344`): cornerCount - 2
    /// bends per trace, the interior angle classified with the ±1°
    /// tolerance — an exact 90/45/135 count into their buckets, an
    /// 89.5° corner still counts as ninety (inside the band), an
    /// 88.5° corner falls to other, a 30° corner falls to other.
    #[test]
    fn t12_bends_classification_and_tolerance_band() {
        let (mut manager, mut board) = parse_fixture();
        let base = BoardStatistics::with_options(&mut manager, &mut board, false, false);

        // Six disjoint L-shapes east of the keepout (x >= 530000),
        // net 49 (empty in the base fixture): 90°, 45°, 135°, 30°,
        // 89.5° (tolerance-in → ninety), 88.5° (tolerance-out → other).
        let ninety = insert(
            &mut manager,
            &mut board,
            &[(530_000, 40_000), (555_000, 40_000), (555_000, 65_000)],
            49,
        );
        let forty_five = insert(
            &mut manager,
            &mut board,
            &[(590_000, 40_000), (615_000, 40_000), (640_000, 65_000)],
            49,
        );
        let one_thirty_five = insert(
            &mut manager,
            &mut board,
            &[(530_000, 100_000), (555_000, 100_000), (530_000, 125_000)],
            49,
        );
        let thirty = insert(
            &mut manager,
            &mut board,
            &[(590_000, 100_000), (615_000, 100_000), (640_000, 114_434)],
            49,
        );
        let tolerance_in = insert(
            &mut manager,
            &mut board,
            &[(530_000, 160_000), (555_000, 160_000), (555_200, 182_860)],
            49,
        );
        let tolerance_out = insert(
            &mut manager,
            &mut board,
            &[(590_000, 160_000), (615_000, 160_000), (615_599, 182_860)],
            49,
        );
        let _ = (
            ninety,
            forty_five,
            one_thirty_five,
            thirty,
            tolerance_in,
            tolerance_out,
        );

        let stats = BoardStatistics::with_options(&mut manager, &mut board, false, false);
        assert_eq!(
            stats.items.trace_count,
            base.items.trace_count + 6,
            "six live traces inserted"
        );
        assert_eq!(
            stats.bends.total_count,
            base.bends.total_count + 6,
            "cornerCount - 2 bends per L-trace"
        );
        assert_eq!(
            stats.bends.ninety_degree_count,
            base.bends.ninety_degree_count + 2,
            "exact 90 plus the 89.5 tolerance-in corner"
        );
        assert_eq!(
            stats.bends.forty_five_degree_count,
            base.bends.forty_five_degree_count + 2,
            "exact 45 and 135"
        );
        assert_eq!(
            stats.bends.other_angle_count,
            base.bends.other_angle_count + 2,
            "30 plus the 88.5 tolerance-out corner"
        );
    }

    /// The clearance-violation world (the V2 depth input): two
    /// right-angle-crossing traces of DIFFERENT nets (49 x 94, both
    /// class 0) add exactly ONE violation (delta over the base
    /// fixture, whose own violations the walk also counts) with
    /// actual clearance 0; the µm shortfall of the crossing is the
    /// class-pair matrix value times the board-unit-to-µm factor
    /// (0.1 for `resolution um 10`), and min == the crossing's own
    /// shortfall only if it is the smallest row — so min/max/avg are
    /// pinned through the exact total instead.
    #[test]
    fn t12_violation_world() {
        let (mut manager, mut board) = parse_fixture();
        let base = BoardStatistics::new(&mut manager, &mut board);
        let base_count = base.clearance_violations.total_count.expect("base counted");
        let base_um = base
            .clearance_violations
            .total_violation_um
            .expect("base depth counted");
        // Disjoint horizontal/vertical corridors, crossing at
        // (560000, 210000); the fixture's only other net-94 item is
        // the pin at (663500, 20000), net 49 is empty.
        let horizontal = insert(
            &mut manager,
            &mut board,
            &[(530_000, 210_000), (600_000, 210_000)],
            49,
        );
        let vertical = insert(
            &mut manager,
            &mut board,
            &[(560_000, 190_000), (560_000, 230_000)],
            94,
        );

        let stats = BoardStatistics::new(&mut manager, &mut board);
        assert_eq!(
            stats.clearance_violations.total_count,
            Some(base_count + 1),
            "exactly ONE crossing violation added"
        );
        // The crossing's own expected clearance, read INDEPENDENTLY:
        // the class of the inserted traces (1, `kicad_default`) in the
        // matrix at layer 0 — the DSN `(clearance 250)` scope lands on
        // the (1,1) cell of names ["null", "default"].
        let class_h = board
            .get(horizontal)
            .expect("horizontal inserted")
            .clearance_class;
        let class_v = board
            .get(vertical)
            .expect("vertical inserted")
            .clearance_class;
        let expected_dbu = f64::from(board.rules().clearance.get_value(class_h, class_v, 0));
        assert_eq!(
            expected_dbu, 2500.0,
            "the (1,1) matrix cell carries the DSN clearance 250 µm = 2500 DBU (resolution um 10)"
        );
        // Overlapping shapes measure actual clearance 0.0 exactly
        // (clearance.rs:126-127), so the crossing's shortfall is the
        // full matrix value: 2500 DBU × 0.1 µm/DBU (`resolution um 10`)
        // = exactly 250.0 µm.
        let total = stats
            .clearance_violations
            .total_violation_um
            .expect("depth counted");
        assert_eq!(
            total - base_um,
            250.0,
            "the crossing adds exactly 250.0 µm (2500 DBU × 0.1)"
        );
        let expected_um = base_um + expected_dbu * 0.1;
        assert!(
            (total - expected_um).abs() < 1e-9,
            "shortfall um {total} vs expected {expected_um}"
        );
        // The min/max/avg faces over the (base + 1)-row aggregate:
        // the average is the exact per-row mean, and the bounds hold.
        assert_eq!(
            stats.clearance_violations.avg_violation_um,
            Some(total / f64::from(base_count + 1)),
            "avg == total / rows"
        );
        let min = stats
            .clearance_violations
            .min_violation_um
            .expect("min counted");
        let max = stats
            .clearance_violations
            .max_violation_um
            .expect("max counted");
        assert!(min <= max && min <= total && max <= total, "bounds hold");
        assert!(
            min <= total / f64::from(base_count + 1) && total / f64::from(base_count + 1) <= max,
            "min <= avg <= max"
        );
        // The pre-existing split: the parsed board carries the load
        // default 0, so everything (base + crossing) is
        // router-introduced.
        assert_eq!(stats.clearance_violations.pre_existing_count, 0);
        assert_eq!(
            stats.clearance_violations.router_introduced_count,
            base_count + 1
        );

        // Adding a violation never raises the V2 score, and the drop
        // is at least the count term of the new row.
        let score = stats.get_router_score(Some(&v2_default_settings()));
        let base_score = base.get_router_score(Some(&v2_default_settings()));
        assert!(
            score <= base_score,
            "adding a violation never raises the V2 score"
        );
    }

    /// The live-only walk (Java `UndoableObjects.delete` REMOVES the
    /// node from the objects map, `:121`): after inserting and then
    /// removing a trace, the counting walk must NOT see it — the Rust
    /// arena keeps the tombstone behind `on_the_board`, so dropping
    /// that filter surfaces here as a +1 phantom.
    #[test]
    fn t12_live_only_walk() {
        let (mut manager, mut board) = parse_fixture();
        let base = BoardStatistics::with_options(&mut manager, &mut board, false, false);
        let lone = insert(
            &mut manager,
            &mut board,
            &[(640_000, 210_000), (660_000, 210_000)],
            49,
        );
        let grown = BoardStatistics::with_options(&mut manager, &mut board, false, false);
        assert_eq!(
            grown.items.trace_count,
            base.items.trace_count + 1,
            "the insert is visible"
        );
        remove_item_through_repository(&mut manager, &mut board, lone);
        let after = BoardStatistics::with_options(&mut manager, &mut board, false, false);
        assert_eq!(
            after.items.trace_count, base.items.trace_count,
            "the removed trace is invisible (live-only walk)"
        );
        assert_eq!(
            after.items.total_count, base.items.total_count,
            "no phantom in the total either"
        );
        assert_eq!(
            after.traces.total_length, base.traces.total_length,
            "the length sum drops back (the removed trace's length gone)"
        );
    }
}

/// The T8 optimizer-score pin suite (Java `getOptimizerScore`/
/// `getV2OptimizerScore`/`ensureDifficulty`, `BoardStatistics.java:
/// 743-835`). Every expected value is a CLOSED-FORM dyadic world — the
/// arithmetic is exact in f64/f32 at these inputs, so the expected
/// literals are derived from the FORMULA, not from a run of the port
/// itself (no tautologies; the jar faces live in the bounds module's
/// probe-anchored pins).
#[cfg(test)]
mod t8_optimizer_score_tests {
    use super::*;

    /// The V2 optimizer box: version V2_LOWER_BOUND, every weight unset
    /// (the inline defaults 1000/2000/500 and floors 1.0/1.0 apply).
    fn v2_optimizer_box() -> OptimizerScoreSettings {
        OptimizerScoreSettings {
            version: OptimizerScoringVersion::V2LowerBound,
            excess_wire_length_weight: None,
            excess_via_weight: None,
            excess_bend_weight: None,
            length_floor: None,
            difficulty_scale_floor: None,
        }
    }

    fn v2_settings(box_: OptimizerScoreSettings) -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: None,
            router_scoring: None,
            optimizer_scoring: Some(box_),
        }
    }

    /// The difficulty ladder (Java `ensureDifficulty`, `:743-778`):
    /// pinCount ← items.pinCount, ELSE pads.totalCount, ELSE 0 — the
    /// first-nonzero ORDER is load-bearing (a swapped-fallback mutant
    /// answers 9 where the oracle answers 7). Same ladder for the
    /// signal layer count. C = max(1, pins·layers) only when C <= 0;
    /// D = C only when D is null; a SET field is never overwritten.
    #[test]
    fn t8_ensure_difficulty_ladder_arms() {
        // items wins over pads (order witness).
        let mut stats = BoardStatistics::new_empty();
        stats.items.pin_count = 7;
        stats.pads_total_count = 9;
        stats.layers_signal_count = 3;
        stats.layers_total_count = 5;
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.pin_count, 7);
        assert_eq!(stats.difficulty.signal_layer_count, 3);
        assert_eq!(stats.difficulty.complexity_c, 21);
        assert_eq!(stats.difficulty.difficulty_d, Some(21.0));

        // pads arm + total-layers arm.
        let mut stats = BoardStatistics::new_empty();
        stats.pads_total_count = 9;
        stats.layers_total_count = 5;
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.pin_count, 9);
        assert_eq!(stats.difficulty.signal_layer_count, 5);
        assert_eq!(stats.difficulty.complexity_c, 45);

        // The zero arms: both sources empty -> 0/0, then max(1, 0) = 1.
        let mut stats = BoardStatistics::new_empty();
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.pin_count, 0);
        assert_eq!(stats.difficulty.signal_layer_count, 0);
        assert_eq!(stats.difficulty.complexity_c, 1);
        assert_eq!(stats.difficulty.difficulty_d, Some(1.0));

        // A SET field survives: complexity_c 10 with pins·layers = 21
        // stays 10; D set stays; D null with C set takes C.
        let mut stats = BoardStatistics::new_empty();
        stats.items.pin_count = 7;
        stats.layers_signal_count = 3;
        stats.difficulty.complexity_c = 10;
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.complexity_c, 10, "C <= 0 gate");
        assert_eq!(
            stats.difficulty.difficulty_d,
            Some(10.0),
            "D takes the SET C"
        );
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.complexity_c = 10;
        stats.difficulty.difficulty_d = Some(77.0);
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.difficulty_d, Some(77.0), "set D survives");

        // The EARLY-RETURN arm (Java `:747-749`; spec-review MINOR-3):
        // with D already set the call is a FULL no-op — the ladder must
        // not run even where it WOULD change fields. World: D set +
        // sources that the ladder would adopt (items.pinCount 7,
        // signalCount 3 -> pinCount 7, C 21 on a recompute). The
        // early-return drop mutant recomputes and answers 7/21; Java
        // (and the port) keep 0/0 — the first computation wins, which
        // is what T9's repeated calls on evolving boards need.
        let mut stats = BoardStatistics::new_empty();
        stats.items.pin_count = 7;
        stats.layers_signal_count = 3;
        stats.difficulty.difficulty_d = Some(5.0);
        stats.ensure_difficulty();
        assert_eq!(stats.difficulty.difficulty_d, Some(5.0), "D kept");
        assert_eq!(
            stats.difficulty.pin_count, 0,
            "early return: the ladder must not fill pinCount"
        );
        assert_eq!(
            stats.difficulty.complexity_c, 0,
            "early return: the ladder must not fill complexityC"
        );
    }

    /// The three penalties, one at a time (the others at zero excess),
    /// at the default weights. Dyadic worlds: excess/denominator is a
    /// power-of-two fraction, so the f64 arithmetic is exact and the
    /// f32 result is exact.
    #[test]
    fn t8_penalty_each_isolated_above_floor() {
        let settings = v2_settings(v2_optimizer_box());
        // Length-only: min 4.0 mm, actual 5.0 mm, vias/bends at min,
        // difficulty 4: 1000 - 1000*1/4 = 750.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(4.0);
        stats.traces.total_length_mm = Some(5.0);
        stats.bounds.min_via_count = Some(1);
        stats.vias.total_count = 1;
        stats.bounds.min_bend_count = Some(2);
        stats.bends.total_count = 2;
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 750.0);

        // Via-only: excess 1 via, difficulty 4: 1000 - 2000/4 = 500.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(4.0);
        stats.traces.total_length_mm = Some(4.0);
        stats.bounds.min_via_count = Some(1);
        stats.vias.total_count = 2;
        stats.bounds.min_bend_count = Some(2);
        stats.bends.total_count = 2;
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 500.0);

        // Bend-only: excess 1 bend, difficulty 4: 1000 - 500/4 = 875.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(4.0);
        stats.traces.total_length_mm = Some(4.0);
        stats.bounds.min_via_count = Some(1);
        stats.vias.total_count = 1;
        stats.bounds.min_bend_count = Some(2);
        stats.bends.total_count = 3;
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 875.0);
    }

    /// The floor boundaries (cerebrum mode 16): the world AT each
    /// floor edge and one step either side, both directions killed.
    ///   * length floor: min BELOW/AT/ABOVE the 1.0 default floor —
    ///     the denominator is max(min, floor), so a floor-formula
    ///     mutant (`min` alone, floor±0.25, or `<` instead of `max`)
    ///     moves at least two of the three worlds.
    ///   * difficultyScaleFloor setting: the denominator is
    ///     max(difficulty, floor) AT the edge and both sides.
    ///   * the `Math.max(0.0, lengthFloor)` setting clamp: a NEGATIVE
    ///     length floor must behave as 0.0 (denominator = min), not
    ///     as the 1.0 default.
    #[test]
    fn t8_penalty_floor_boundaries_both_directions() {
        let settings = v2_settings(v2_optimizer_box());
        // min = 0.75 (below floor): denom 1.0 -> 1000 - 1000*1/1 -> 0.0 (clamp).
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(0.75);
        stats.traces.total_length_mm = Some(1.75);
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 0.0);
        // min = 1.0 (AT the floor edge): denom 1.0 -> 1000*1/1 = 1000
        // -> exactly 0.0 (the clamp boundary, not a negative sum).
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(1.0);
        stats.traces.total_length_mm = Some(2.0);
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 0.0);
        // min = 1.25 (above): denom 1.25 -> 1000 - 1000*1/1.25 = 200.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(1.25);
        stats.traces.total_length_mm = Some(2.25);
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 200.0);

        // difficultyScaleFloor setting at the edge (== difficulty 4)
        // and both sides: bend penalty isolated, weight 500.
        //   floor 3 -> denom 4 -> 125 -> 875; floor 4 -> denom 4 ->
        //   875; floor 5 -> denom 5 -> 100 -> 900.
        let floor_world = |floor: f32| {
            let mut stats = BoardStatistics::new_empty();
            stats.difficulty.difficulty_d = Some(4.0);
            stats.bounds.min_bend_count = Some(1);
            stats.bends.total_count = 2;
            let mut box_ = v2_optimizer_box();
            box_.difficulty_scale_floor = Some(floor);
            stats.get_optimizer_score(Some(&v2_settings(box_)))
        };
        assert_eq!(floor_world(3.0), 875.0);
        assert_eq!(floor_world(4.0), 875.0);
        assert_eq!(floor_world(5.0), 900.0);

        // lengthFloor setting clamp: negative floor behaves as 0.0.
        // min 0.5, actual 0.75, floor -3: denom max(0.5, 0.0) = 0.5
        // -> 1000 - 1000*0.25/0.5 = 500.0 (the 1.0 default would give
        // 750.0 — the two faces differ, the world discriminates).
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(0.5);
        stats.traces.total_length_mm = Some(0.75);
        let mut box_ = v2_optimizer_box();
        box_.length_floor = Some(-3.0);
        assert_eq!(stats.get_optimizer_score(Some(&v2_settings(box_))), 500.0);
    }

    /// The excess clamps and the score clamp: actual BELOW min (a
    /// negative excess) waives the term; a huge excess bottoms the
    /// whole score out at exactly 0.0 (Java `Math.max(0.0, …)` on the
    /// f64 sum BEFORE the f32 cast).
    #[test]
    fn t8_excess_clamps_and_score_clamp() {
        let settings = v2_settings(v2_optimizer_box());
        // actual < min on every axis: score stays 1000.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(50.0);
        stats.traces.total_length_mm = Some(40.0);
        stats.bounds.min_via_count = Some(3);
        stats.vias.total_count = 1;
        stats.bounds.min_bend_count = Some(7);
        stats.bends.total_count = 5;
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 1000.0);

        // The score clamp: excess 900 mm over a 10 mm min -> 1000*90
        // = 90000 -> clamp 0.0.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(10.0);
        stats.traces.total_length_mm = Some(100.0);
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 0.0);
    }

    /// The dispatch (Java `getOptimizerScore(RouterSettings)`,
    /// `:795-806`): V2_LOWER_BOUND takes the V2 formula; a V1_LEGACY
    /// box and a NULL box take the legacy normalized score over
    /// `scoring`-or-DefaultSettings. One world, THREE DISTINCT faces —
    /// an arm-blind port cannot pass all three (cerebrum modes 4/7).
    #[test]
    fn t8_version_dispatch_three_distinct_faces() {
        // World: length excess 4 mm over min 2, difficulty 4. The
        // legacy face unboxes the violation count (Java NPE parity —
        // the counting ctor always writes it), so the world carries the
        // fully-computed shape.
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(2.0);
        stats.traces.total_length_mm = Some(6.0);
        stats.clearance_violations.total_count = Some(0);
        // Legacy needs a nonzero maximum count.
        stats.connections.maximum_count = Some(2);
        stats.connections.incomplete_count = Some(1);

        // V2: 1000 - 1000*4/2 -> clamp 0.0.
        let v2 = v2_settings(v2_optimizer_box());
        assert_eq!(stats.get_optimizer_score(Some(&v2)), 0.0);

        // V1 box: the legacy face over DefaultSettings.scoring:
        // calculateScore = 2*5_000_000 - 1*5_000_000 - (6*1 + 0*50)
        // = 4_999_994; /10_000_000 * 1000 = 499.9994.
        let mut v1_box = v2_optimizer_box();
        v1_box.version = OptimizerScoringVersion::V1Legacy;
        let v1 = v2_settings(v1_box);
        let legacy = stats.get_optimizer_score(Some(&v1));
        assert_ne!(legacy, 0.0, "V1 must NOT take the V2 arm");
        assert_eq!(legacy, (4_999_994.0f64 / 10_000_000.0 * 1000.0) as f32);

        // Null box: same legacy face through the DefaultSettings
        // fallback (a port that always dispatches V2 fails here).
        let none = RouterSettingsScoring {
            scoring: None,
            router_scoring: None,
            optimizer_scoring: None,
        };
        assert_eq!(stats.get_optimizer_score(Some(&none)), legacy);

        // The `scoring` passthrough: a caller-supplied legacy box
        // replaces DefaultSettings in the legacy face.
        let custom_scoring = RoutingCostSettings {
            unrouted_net_penalty: Some(100.0),
            ..default_routing_cost_settings()
        };
        let custom = RouterSettingsScoring {
            scoring: Some(custom_scoring),
            router_scoring: None,
            optimizer_scoring: None,
        };
        // calculateScore = 2*100 - 1*100 - 6 = 94; /200 * 1000 = 470.
        assert_eq!(stats.get_optimizer_score(Some(&custom)), 470.0);
    }

    /// The V2 null-guard arms (the GSON-empty face): null bounds read
    /// as 0, null difficulty as 1.0 — the whole score is then the
    /// excess over a 0 lower bound divided by the length floor.
    #[test]
    fn t8_v2_null_guard_arms() {
        let settings = v2_settings(v2_optimizer_box());
        let stats = BoardStatistics::new_empty();
        // Everything null/zero: excess 0, score exactly 1000.0.
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 1000.0);

        // Null bounds but a real trace length: length penalty over the
        // 1.0 length floor: 1000 - 1000*12/1 -> clamp 0.0; keep it in
        // range with a real bound instead: the DIFFICULTY null arm is
        // the discriminator here — null D reads as 1.0, so via excess
        // 1 costs 2000 (clamp 0) — contrast with D=4 (cost 500).
        let mut stats = BoardStatistics::new_empty();
        stats.bounds.min_trace_length_mm = Some(10.0);
        stats.traces.total_length_mm = Some(10.0);
        stats.bounds.min_via_count = Some(0);
        stats.vias.total_count = 1;
        assert_eq!(
            stats.get_optimizer_score(Some(&settings)),
            0.0,
            "null D = 1.0"
        );
        let mut stats = BoardStatistics::new_empty();
        stats.difficulty.difficulty_d = Some(4.0);
        stats.bounds.min_trace_length_mm = Some(10.0);
        stats.traces.total_length_mm = Some(10.0);
        stats.bounds.min_via_count = Some(0);
        stats.vias.total_count = 1;
        assert_eq!(stats.get_optimizer_score(Some(&settings)), 500.0, "D=4");
    }
}
