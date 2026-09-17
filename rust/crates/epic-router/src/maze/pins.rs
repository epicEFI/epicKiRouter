//! The T6 jar-capture pins — the maze search CORE replayed against the
//! probe oracle `rust/harness/oracle/MazeSpike.java`.
//!
//! Capture: `/tmp/maze_capture_{1,2}.rows`, 182 JSON rows, run TWICE on
//! the jar with byte-identical output (`cmp` clean). Fixture:
//! `rust/harness/fixtures/maze-spike/t6_maze.dsn` (2 signal layers, a
//! keepout wall pair with a 4000-wide gap, SMD pins KA/PA and KB/PB on
//! NET_A, via VT). Build/run header lives in the oracle source.
//!
//! Protocol P (both sides reproduce it EXACTLY): a phase opens a FRESH
//! board (re-parse; a maze run mutates item start infos and the shared
//! tree — a second engine on the same board fails init), runs
//! `init`, then drains `while front non-empty && k < cap`: read the
//! front head (RAW element fields), call `occupyNextElement` ONCE,
//! record (before, after, cont); on `cont == false` record the
//! destination and BREAK (Java `findConnection` is the same loop —
//! popping past the destination re-marks it ~93 times, drain noise).
//! The Rust replay runs the same P, so head-vs-pop divergence cancels.
//!
//! The pinned rows cover:
//! * the totally ordered front — 28 MAIN pops (vias off) and 40 DRILL
//!   pops (vias on, drill pages + drills interleaved with room doors),
//!   each head a full RAW row (kind, id, section, values, backtrack,
//!   next room, shape entry) and each pop the front-size delta;
//! * the cost model — weighted distance + bend penalty (the BEND
//!   straight/diag contrast, bendCosts[0]=47) + the destination
//!   distance of the `sortingValue`;
//! * the occupancy marking — the front-size drops when an occupied
//!   section is skipped, plus a direct `is_occupied` state pin;
//! * `doorIsSmall` — the strict `<` boundary at exactly the door
//!   length (38964.0: false, 38965.0: true);
//! * the comparator tie chain — sortingValue → expansionValue → door
//!   id → section, the full-tie set dedup, the NaN fall-through and
//!   the -0.0 ≡ 0.0 tie (T1-T7).
//!
//! `TargetDistance` below is the capture-replay mirror of Java
//! `DestinationDistance` (the REAL lower-bound heuristic, not the T5
//! targetless stub) — the T8 production port owns the module and can
//! absorb it; until then these pins verify the maze wiring AND this
//! port against the jar. `LegalChecker` answers DRILLABLE because
//! Java's real `ForcedViaInserter.checkLayer` is vacuously legal on
//! this empty fixture; a mutant answering NotDrillable flips the DRILL
//! head rows (the DRILL-phase pins are its mutation coverage).

use std::collections::BTreeSet;

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::tile_shape::TileShape;

use super::list_element::{ExpandableObject, FrontOrder, MazeListElement, compare};
use super::search_engine::MazeSearchEngine;
use crate::control::{AngleRestriction, AutorouteControl, ExpansionCostFactor, RouterSettingsIr};
use crate::drill::pins::Harness;
use crate::drill::{
    Adjustment, CheckDrillResult, DestinationDistance, DrillEngine, DrillPageArray,
    ViaLayerChecker, max_drill_page_width,
};
use crate::expansion::{ExpansionDoor, NeighbourEngine};

/// The T6 spike fixture (a committed deliverable of this task).
const FIXTURE_DSN: &str = include_str!("../../../../harness/fixtures/maze-spike/t6_maze.dsn");

/// NET_A (capture `ctrl` row `netNumber:1`).
const NET_A: i32 = 1;

/// The pin item keys (capture `pins` row: startId 4, destId 5 — the
/// lower id starts).
const START_PIN: u64 = 4;
const DEST_PIN: u64 = 5;

/// bendCosts[0] forced for the run (the capture's bend probes).
const BEND_COST: f64 = 47.0;

// ---- the T8-seam replay distance (Java `DestinationDistance`) ----

/// Java `autoroute/maze/DestinationDistance.java` verbatim — the
/// lower-bound distance from a front element to the joined
/// destination boxes. Test-side: T8 owns the production module.
struct TargetDistance {
    trace_costs: Vec<ExpansionCostFactor>,
    layer_active: Vec<bool>,
    layer_count: usize,
    active_layer_count: usize,
    min_normal_via_cost: f64,
    min_component_side_trace_cost: f64,
    max_component_side_trace_cost: f64,
    min_solder_side_trace_cost: f64,
    max_solder_side_trace_cost: f64,
    max_inner_side_trace_cost: f64,
    min_component_inner_trace_cost: f64,
    min_solder_inner_trace_cost: f64,
    min_component_solder_inner_trace_cost: f64,
    component_side_box: IntBox,
    solder_side_box: IntBox,
    inner_side_box: IntBox,
    box_is_empty: bool,
    component_side_box_is_empty: bool,
    solder_side_box_is_empty: bool,
    inner_side_box_is_empty: bool,
}

impl TargetDistance {
    /// Java ctor (`:45-99`) — the trace-cost extremes per side, inner
    /// layers assumed cost 1 in the preferred direction.
    fn new(ctrl: &AutorouteControl) -> Self {
        let trace_costs = ctrl.trace_costs.clone();
        let layer_active = ctrl.layer_active.clone();
        let layer_count = layer_active.len();
        let active_layer_count = layer_active.iter().filter(|&&a| a).count();
        let mut this = TargetDistance {
            trace_costs,
            layer_active,
            layer_count,
            active_layer_count,
            min_normal_via_cost: ctrl.min_normal_via_cost,
            min_component_side_trace_cost: 0.0,
            max_component_side_trace_cost: 0.0,
            min_solder_side_trace_cost: 0.0,
            max_solder_side_trace_cost: 0.0,
            max_inner_side_trace_cost: 0.0,
            min_component_inner_trace_cost: 0.0,
            min_solder_inner_trace_cost: 0.0,
            min_component_solder_inner_trace_cost: 0.0,
            component_side_box: IntBox::EMPTY,
            solder_side_box: IntBox::EMPTY,
            inner_side_box: IntBox::EMPTY,
            box_is_empty: true,
            component_side_box_is_empty: true,
            solder_side_box_is_empty: true,
            inner_side_box_is_empty: true,
        };
        if this.layer_active[0] {
            if this.trace_costs[0].horizontal < this.trace_costs[0].vertical {
                this.min_component_side_trace_cost = this.trace_costs[0].horizontal;
                this.max_component_side_trace_cost = this.trace_costs[0].vertical;
            } else {
                this.min_component_side_trace_cost = this.trace_costs[0].vertical;
                this.max_component_side_trace_cost = this.trace_costs[0].horizontal;
            }
        }
        let last = this.layer_count - 1;
        if this.layer_active[last] {
            if this.trace_costs[last].horizontal < this.trace_costs[last].vertical {
                this.min_solder_side_trace_cost = this.trace_costs[last].horizontal;
                this.max_solder_side_trace_cost = this.trace_costs[last].vertical;
            } else {
                this.min_solder_side_trace_cost = this.trace_costs[last].vertical;
                this.max_solder_side_trace_cost = this.trace_costs[last].horizontal;
            }
        }
        // Note: for inner layers we assume cost 1 in the preferred
        // direction — the max inner cost is the min over the sides.
        this.max_inner_side_trace_cost = this
            .max_component_side_trace_cost
            .min(this.max_solder_side_trace_cost);
        for ind2 in 1..last {
            if !this.layer_active[ind2] {
                continue;
            }
            let current_max = this.trace_costs[ind2]
                .horizontal
                .max(this.trace_costs[ind2].vertical);
            this.max_inner_side_trace_cost = this.max_inner_side_trace_cost.min(current_max);
        }
        this.min_component_inner_trace_cost = this
            .min_component_side_trace_cost
            .min(this.max_inner_side_trace_cost);
        this.min_solder_inner_trace_cost = this
            .min_solder_side_trace_cost
            .min(this.max_inner_side_trace_cost);
        this.min_component_solder_inner_trace_cost = this
            .min_component_inner_trace_cost
            .min(this.min_solder_inner_trace_cost);
        this
    }

    /// Java `join(IntBox, int)` (`:102-114`).
    fn join_box(&mut self, shape: &IntBox, layer: i32) {
        if layer == 0 {
            self.component_side_box = self.component_side_box.union(shape);
            self.component_side_box_is_empty = false;
        } else if layer == self.layer_count as i32 - 1 {
            self.solder_side_box = self.solder_side_box.union(shape);
            self.solder_side_box_is_empty = false;
        } else {
            self.inner_side_box = self.inner_side_box.union(shape);
            self.inner_side_box_is_empty = false;
        }
        self.box_is_empty = false;
    }

    /// Java `calculate(IntBox, int)` (`:122-379`) — the layered
    /// lower-bound minimum. The port keeps Java's structure verbatim
    /// (all inputs finite; no NaN arms).
    fn calculate_box(&self, shape: &IntBox, layer: i32) -> f64 {
        if self.box_is_empty {
            return 2147483647.0;
        }
        let delta = |side_box: &IntBox| -> (f64, f64) {
            let dx = if shape.ll.x > side_box.ur.x {
                f64::from(shape.ll.x - side_box.ur.x)
            } else if shape.ur.x < side_box.ll.x {
                f64::from(side_box.ll.x - shape.ur.x)
            } else {
                0.0
            };
            let dy = if shape.ll.y > side_box.ur.y {
                f64::from(shape.ll.y - side_box.ur.y)
            } else if shape.ur.y < side_box.ll.y {
                f64::from(side_box.ll.y - shape.ur.y)
            } else {
                0.0
            };
            (dx, dy)
        };
        let (component_dx, component_dy) = delta(&self.component_side_box);
        let (solder_dx, solder_dy) = delta(&self.solder_side_box);
        let (inner_dx, inner_dy) = delta(&self.inner_side_box);
        let split = |dx: f64, dy: f64| -> (f64, f64) { if dx > dy { (dx, dy) } else { (dy, dx) } };
        let (component_max, component_min) = split(component_dx, component_dy);
        let (solder_max, solder_min) = split(solder_dx, solder_dy);
        let (inner_max, inner_min) = split(inner_dx, inner_dy);

        let mut result = 2147483647.0;
        let min_normal = self.min_normal_via_cost;
        if layer == 0 {
            if !self.component_side_box_is_empty {
                result = shape.weighted_distance(
                    &self.component_side_box,
                    self.trace_costs[0].horizontal,
                    self.trace_costs[0].vertical,
                );
            }
            if self.active_layer_count <= 1 {
                return result;
            }
            let tmp = if self.min_solder_side_trace_cost < self.min_component_side_trace_cost {
                self.min_solder_side_trace_cost * solder_max
                    + self.min_component_side_trace_cost * solder_min
                    + min_normal
            } else {
                self.min_component_side_trace_cost * solder_max
                    + self.min_solder_side_trace_cost * solder_min
                    + min_normal
            };
            result = result.min(tmp);
            let tmp = component_max
                + component_min * self.min_component_inner_trace_cost
                + 2.0 * min_normal;
            result = result.min(tmp);
            if self.active_layer_count == 2 {
                return result;
            }
            let tmp = inner_max + inner_min * self.min_component_inner_trace_cost + min_normal;
            result = result.min(tmp);
            let tmp = solder_max
                + self.min_component_solder_inner_trace_cost * solder_min
                + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = component_max + component_min + 2.0 * min_normal;
            result = result.min(tmp);
            if self.active_layer_count == 3 {
                return result;
            }
            let tmp = inner_max + inner_min + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = solder_max + solder_min + 3.0 * min_normal;
            return result.min(tmp);
        }
        let last = self.layer_count as i32 - 1;
        if layer == last {
            if !self.solder_side_box_is_empty {
                result = shape.weighted_distance(
                    &self.solder_side_box,
                    self.trace_costs[last as usize].horizontal,
                    self.trace_costs[last as usize].vertical,
                );
            }
            let tmp = if self.min_component_side_trace_cost < self.min_solder_side_trace_cost {
                self.min_component_side_trace_cost * component_max
                    + self.min_solder_side_trace_cost * component_min
                    + min_normal
            } else {
                self.min_solder_side_trace_cost * component_max
                    + self.min_component_side_trace_cost * component_min
                    + min_normal
            };
            result = result.min(tmp);
            let tmp = solder_max + solder_min * self.min_solder_inner_trace_cost + 2.0 * min_normal;
            result = result.min(tmp);
            if self.active_layer_count <= 2 {
                return result;
            }
            let tmp = inner_min * self.min_solder_inner_trace_cost + inner_max + min_normal;
            result = result.min(tmp);
            let tmp = component_max
                + self.min_component_solder_inner_trace_cost * component_min
                + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = solder_max + solder_min + 2.0 * min_normal;
            result = result.min(tmp);
            if self.active_layer_count == 3 {
                return result;
            }
            let tmp = inner_max + inner_min + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = component_max + component_min + 3.0 * min_normal;
            return result.min(tmp);
        }
        // an inner layer
        if !self.inner_side_box_is_empty {
            result = shape.weighted_distance(
                &self.inner_side_box,
                self.trace_costs[layer as usize].horizontal,
                self.trace_costs[layer as usize].vertical,
            );
        }
        let tmp = inner_max + inner_min + min_normal;
        result = result.min(tmp);
        let tmp = component_max + component_min * self.min_component_inner_trace_cost + min_normal;
        result = result.min(tmp);
        let tmp = solder_max + solder_min * self.min_solder_inner_trace_cost + min_normal;
        result = result.min(tmp);
        let tmp = component_max + component_min + 2.0 * min_normal;
        result = result.min(tmp);
        let tmp = solder_max + solder_min + 2.0 * min_normal;
        result.min(tmp)
    }
}

impl DestinationDistance for TargetDistance {
    fn calculate(&self, middle: &FloatPoint, layer: i32) -> f64 {
        self.calculate_box(&middle.bounding_box(), layer)
    }

    /// The REAL join (the trait default is the T8 no-op slot).
    fn join(&mut self, shape: &epic_geometry::int_box::IntBox, layer: i32) {
        self.join_box(shape, layer);
    }
}

/// Java `ForcedViaInserter.checkLayer` for this capture: the fixture
/// is empty board space, so every candidate via position is legal.
/// The DRILL-phase rows are the mutation coverage — a NotDrillable
/// answer reorders/drops them.
struct LegalChecker;

impl ViaLayerChecker for LegalChecker {
    fn check_layer(
        &mut self,
        _required_radius: f64,
        _clearance_class: i32,
        _attach_smd_allowed: bool,
        _room_shape: &TileShape,
        _location: &epic_geometry::point::Point,
        _layer: i32,
        _net_number: i32,
    ) -> CheckDrillResult {
        CheckDrillResult::Drillable
    }
}

// ---- the replay rig ----

/// Java `new RouterSettings(board)` projected to the IR — the capture
/// `ctrl` row's resolved scoring tables (2 layers, default via costs,
/// all layers active, no neckdown).
fn maze_settings_ir(vias_allowed: bool) -> RouterSettingsIr {
    RouterSettingsIr {
        trace_costs: vec![
            ExpansionCostFactor {
                horizontal: 1.0,
                vertical: 2.7,
            },
            ExpansionCostFactor {
                horizontal: 1.6,
                vertical: 1.0,
            },
        ],
        via_costs: 1,
        vias_allowed,
        bend_costs: vec![BEND_COST, 0.0],
        layer_active: vec![true, true],
        automatic_neckdown: false,
    }
}

/// One phase rig: a FRESH board (the capture re-parses per phase — a
/// maze run pollutes item start infos and the shared tree) with its
/// ctrl, distance, checker and page array.
fn make_run(
    vias_allowed: bool,
) -> (
    Harness,
    AutorouteControl,
    TargetDistance,
    LegalChecker,
    DrillPageArray,
) {
    let mut harness = Harness::build_with(FIXTURE_DSN, NET_A);
    let ctrl = AutorouteControl::new(harness.board_mut(), NET_A, &maze_settings_ir(vias_allowed));
    let distance = TargetDistance::new(&ctrl);
    let checker = LegalChecker;
    let max_page_width = max_drill_page_width(harness.default_via_diameter());
    let pages = DrillPageArray::new(&harness, max_page_width);
    (harness, ctrl, distance, checker, pages)
}

/// The door polymorphism of a capture row (`doorKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    RoomDoor,
    TargetDoor,
    DrillPage,
    Drill,
}

fn kind_of(door: &ExpandableObject) -> Kind {
    match door {
        ExpandableObject::RoomDoor(_) => Kind::RoomDoor,
        ExpandableObject::TargetDoor(_) => Kind::TargetDoor,
        ExpandableObject::DrillPage { .. } => Kind::DrillPage,
        ExpandableObject::Drill { .. } => Kind::Drill,
    }
}

// ---- literal tables (generated from /tmp/maze_capture_1.rows) ----

/// One RAW `head` row: the front element fields of the capture.
///
/// Every capture row carries `roomRipped:false, ripupCost:0,
/// alreadyChecked:false` (the no-ripup T6 world); the fields stay
/// in [`assert_head`] so the constants are pinned too.
struct RawHead {
    kind: Kind,
    door_id: i32,
    section: i32,
    exp: f64,
    sort: f64,
    back_kind: Option<Kind>,
    back_id: i32,
    back_section: i32,
    next_room_id: i32,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
}

/// Row literal constructor — the float literals ARE the EXACT
/// `Double.toString` captures (both parses are correctly rounded, so
/// the literal is the captured f64 bit pattern).
#[allow(clippy::too_many_arguments)]
const fn h(
    kind: Kind,
    door_id: i32,
    section: i32,
    exp: f64,
    sort: f64,
    back_kind: Option<Kind>,
    back_id: i32,
    back_section: i32,
    next_room_id: i32,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
) -> RawHead {
    // The literals ARE the capture strings: both Java's
    // Double.parseDouble and Rust's literal parse are correctly
    // rounded, so `445513.2277891202` is the exact captured f64.
    RawHead {
        kind,
        door_id,
        section,
        exp,
        sort,
        back_kind,
        back_id,
        back_section,
        next_room_id,
        ax,
        ay,
        bx,
        by,
    }
}

/// The MAIN drain `head` rows, k = 0..28 (capture order).
const MAIN_HEADS: [RawHead; 28] = [
    h(
        Kind::TargetDoor,
        125,
        0,
        0.0,
        578750.0,
        None,
        -1,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        0,
        279663.4894122756,
        579663.4894122756,
        Some(Kind::TargetDoor),
        125,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        279663.4894122756,
        579663.4894122756,
        Some(Kind::TargetDoor),
        125,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::RoomDoor,
        65,
        0,
        280940.2711977969,
        580208.2711977969,
        Some(Kind::RoomDoor),
        33,
        0,
        3,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        65,
        1,
        280940.2711977969,
        580208.2711977969,
        Some(Kind::RoomDoor),
        33,
        1,
        3,
        479482.0,
        300000.0,
        479482.0,
        315998.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        9,
        575651.0899450427,
        581688.0899450427,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        9,
        575651.0899450427,
        581688.0899450427,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        759584.0,
        318750.0,
        785841.1111111111,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        8,
        549534.1524044513,
        581828.1524044513,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        733326.8888888889,
        281250.0,
        759584.0,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        8,
        549534.1524044513,
        581828.1524044513,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        733326.8888888889,
        318750.0,
        759584.0,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        7,
        523447.4957723329,
        581998.4957723329,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        707069.7777777778,
        281250.0,
        733326.8888888889,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        7,
        523447.4957723329,
        581998.4957723329,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        707069.7777777778,
        318750.0,
        733326.8888888889,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        6,
        497402.0705148735,
        582210.0705148735,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        680812.6666666666,
        281250.0,
        707069.7777777778,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        6,
        497402.0705148735,
        582210.0705148735,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        680812.6666666666,
        318750.0,
        707069.7777777778,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        5,
        471414.7954530079,
        582479.7954530079,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        654555.5555555555,
        281250.0,
        680812.6666666666,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        5,
        471414.7954530079,
        582479.7954530079,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        654555.5555555555,
        318750.0,
        680812.6666666666,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        4,
        445513.2277891202,
        582836.2277891203,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        628298.4444444445,
        281250.0,
        654555.5555555555,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        4,
        445513.2277891202,
        582836.2277891203,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        628298.4444444445,
        318750.0,
        654555.5555555555,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        3,
        419745.37207271333,
        583325.3720727133,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        602041.3333333334,
        281250.0,
        628298.4444444445,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        3,
        419745.37207271333,
        583325.3720727133,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        602041.3333333334,
        318750.0,
        628298.4444444445,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        2,
        394202.5638589007,
        584039.5638589007,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        575784.2222222222,
        281250.0,
        602041.3333333334,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        2,
        394202.5638589007,
        584039.5638589007,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        575784.2222222222,
        318750.0,
        602041.3333333334,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        1,
        369080.779438006,
        585174.779438006,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        549527.1111111111,
        281250.0,
        575784.2222222222,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        1,
        369080.779438006,
        585174.779438006,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        549527.1111111111,
        318750.0,
        575784.2222222222,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        0,
        344878.59818040574,
        587229.5981804058,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        523270.0,
        281250.0,
        549527.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        0,
        344878.59818040574,
        587229.5981804058,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        523270.0,
        318750.0,
        549527.1111111111,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        9,
        582981.0117433746,
        589018.0117433746,
        Some(Kind::RoomDoor),
        65,
        1,
        4,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        10,
        601790.912884423,
        601790.912884423,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        785841.1111111111,
        318750.0,
        812098.2222222222,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        2,
        412102.3189319887,
        601939.3189319887,
        Some(Kind::RoomDoor),
        65,
        1,
        4,
        575784.2222222222,
        281250.0,
        602041.3333333334,
        281250.0,
    ),
];

/// The MAIN drain `popMeta` rows: (before, after, cont).
const MAIN_POPS: [(i32, i32, bool); 28] = [
    (1, 2, true),
    (2, 3, true),
    (3, 4, true),
    (4, 40, true),
    (40, 75, true),
    (75, 89, true),
    (89, 103, true),
    (103, 117, true),
    (117, 131, true),
    (131, 145, true),
    (145, 159, true),
    (159, 173, true),
    (173, 187, true),
    (187, 201, true),
    (201, 215, true),
    (215, 229, true),
    (229, 243, true),
    (243, 257, true),
    (257, 271, true),
    (271, 285, true),
    (285, 299, true),
    (299, 313, true),
    (313, 327, true),
    (327, 341, true),
    (341, 355, true),
    (355, 355, true),
    (355, 369, true),
    (369, 366, false),
];

/// The DRILL drain `head` rows, k = 0..40 (capture order).
const DRILL_HEADS: [RawHead; 40] = [
    h(
        Kind::TargetDoor,
        125,
        0,
        0.0,
        578750.0,
        None,
        -1,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813583,
        0,
        400.0,
        579150.0,
        Some(Kind::TargetDoor),
        125,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1913489583,
        0,
        400.0,
        579150.0,
        Some(Kind::TargetDoor),
        125,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        -1701202883,
        0,
        29573.69021909981,
        458472.6902190998,
        Some(Kind::TargetDoor),
        125,
        0,
        -1,
        349851.0,
        304000.0,
        349851.0,
        304000.0,
    ),
    h(
        Kind::Drill,
        -1701202883,
        1,
        29573.69021909981,
        458872.6902190998,
        Some(Kind::Drill),
        -1701202883,
        0,
        9,
        349851.0,
        304000.0,
        349851.0,
        304000.0,
    ),
    h(
        Kind::RoomDoor,
        102,
        13,
        38012.63861428629,
        462666.6386142863,
        Some(Kind::Drill),
        -1701202883,
        1,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813581,
        1,
        38412.63861428629,
        463066.6386142863,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        102,
        14,
        78695.64212314785,
        477394.64212314785,
        Some(Kind::Drill),
        -1701202883,
        1,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813581,
        1,
        79095.64212314785,
        477794.64212314785,
        Some(Kind::RoomDoor),
        102,
        14,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -765662255,
        1,
        79095.64212314785,
        480159.2210705162,
        Some(Kind::RoomDoor),
        102,
        14,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -765662253,
        1,
        38412.63861428629,
        481004.30177218106,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        1,
        92414.08490269308,
        483174.08490269305,
        Some(Kind::RoomDoor),
        102,
        14,
        -1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        0,
        92414.08490269308,
        482774.08490269305,
        Some(Kind::Drill),
        -589245159,
        1,
        1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        1,
        92791.61698710383,
        483551.61698710383,
        Some(Kind::RoomDoor),
        102,
        13,
        -1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::DrillPage,
        1604640401,
        0,
        183986.42501316947,
        483986.4250131695,
        Some(Kind::RoomDoor),
        33,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        73,
        0,
        184863.20679869078,
        484131.2067986908,
        Some(Kind::RoomDoor),
        33,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1799050863,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1505175567,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1059537551,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -320024239,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        125613777,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        865127089,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1310765105,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1604640403,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        1716054496,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        479482.0,
        292001.0,
        479482.0,
        292001.0,
    ),
    h(
        Kind::DrillPage,
        2050278417,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        -1430797451,
        0,
        209788.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        504007.0,
        292001.0,
        504007.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1122813288,
        0,
        230299.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        524518.0,
        292001.0,
        524518.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        1716054496,
        1,
        185263.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        1716054496,
        0,
        3,
        479482.0,
        292001.0,
        479482.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1430797451,
        1,
        209788.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        -1430797451,
        0,
        5,
        504007.0,
        292001.0,
        504007.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1122813288,
        1,
        230299.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        -1122813288,
        0,
        5,
        524518.0,
        292001.0,
        524518.0,
        292001.0,
    ),
    h(
        Kind::DrillPage,
        -1761726863,
        0,
        185263.20679869078,
        485333.48534766433,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1348089105,
        0,
        185263.20679869078,
        485455.87751820916,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1159002385,
        1,
        38412.63861428629,
        485542.37545639154,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        9,
        479574.02554593654,
        485611.02554593654,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        162937777,
        0,
        185263.20679869078,
        485622.22323809366,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        8,
        453457.0880053451,
        485751.0880053451,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        733326.8888888889,
        281250.0,
        759584.0,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        -1022213551,
        0,
        185263.20679869078,
        485861.2721562131,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        7,
        427370.4313732267,
        485921.4313732267,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        707069.7777777778,
        281250.0,
        733326.8888888889,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        -1799050861,
        0,
        479974.02554593654,
        486011.02554593654,
        Some(Kind::RoomDoor),
        353,
        9,
        12,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
];

/// The DRILL drain `popMeta` rows: (before, after, cont).
const DRILL_POPS: [(i32, i32, bool); 40] = [
    (1, 210, true),
    (210, 209, true),
    (209, 209, true),
    (209, 209, true),
    (209, 243, true),
    (243, 357, true),
    (357, 356, true),
    (356, 470, true),
    (470, 469, true),
    (469, 469, true),
    (469, 469, true),
    (469, 469, true),
    (469, 470, true),
    (470, 472, true),
    (472, 471, true),
    (471, 535, true),
    (535, 534, true),
    (534, 535, true),
    (535, 534, true),
    (534, 533, true),
    (533, 532, true),
    (532, 531, true),
    (531, 530, true),
    (530, 530, true),
    (530, 530, true),
    (530, 529, true),
    (529, 529, true),
    (529, 529, true),
    (529, 556, true),
    (556, 599, true),
    (599, 642, true),
    (642, 641, true),
    (641, 640, true),
    (640, 640, true),
    (640, 736, true),
    (736, 735, true),
    (735, 831, true),
    (831, 830, true),
    (830, 926, true),
    (926, 926, true),
];

/// One pinned `bendElem` row (the BEND probes' front dump).
const BEND_STRAIGHT_HEADS: [RawHead; 3] = [
    h(
        Kind::RoomDoor,
        33,
        0,
        22754.62725829493,
        322754.62725829496,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        22754.62725829493,
        322754.62725829496,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::TargetDoor,
        125,
        0,
        277850.0,
        856600.0,
        Some(Kind::RoomDoor),
        232558643,
        0,
        -1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
];

/// The BEND_DIAG front dump — SAME seed, DIAGONAL entry chord: the
/// two sections split apart (20057.6 vs 25452.2) where the straight
/// entry gave equal values, and the target row shifts by the bend.
const BEND_DIAG_HEADS: [RawHead; 3] = [
    h(
        Kind::RoomDoor,
        33,
        0,
        20057.628072715335,
        320057.62807271536,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        25452.266452855223,
        325452.2664528552,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::TargetDoor,
        125,
        0,
        277863.1230023165,
        856613.1230023166,
        Some(Kind::RoomDoor),
        232558643,
        0,
        -1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
];

/// The capture `ctrl` row's derived-control assertions (shared by
/// both phase rigs).
fn assert_ctrl(ctrl: &AutorouteControl, harness: &Harness, vias_allowed: bool) {
    assert_eq!(ctrl.net_number, 1, "ctrl netNumber");
    assert_eq!(ctrl.layer_count, 2, "ctrl layerCount");
    assert_eq!(ctrl.vias_allowed, vias_allowed, "ctrl viasAllowed");
    assert_eq!(ctrl.bend_costs, vec![BEND_COST, 0.0], "ctrl bendCosts");
    assert!(!ctrl.with_neckdown, "ctrl withNeckdown");
    assert!(!ctrl.is_fanout, "ctrl isFanout");
    assert!(!ctrl.ripup_allowed, "ctrl ripupAllowed");
    assert_eq!(ctrl.max_shove_trace_recursion_depth, 20, "ctrl maxShove");
    assert_eq!(
        ctrl.compensated_trace_half_width,
        [2750, 2750],
        "ctrl compensatedTraceHalfWidth"
    );
    assert_eq!(
        ctrl.trace_costs,
        vec![
            ExpansionCostFactor {
                horizontal: 1.0,
                vertical: 2.7
            },
            ExpansionCostFactor {
                horizontal: 1.6,
                vertical: 1.0
            },
        ],
        "ctrl traceCosts"
    );
    assert_eq!(ctrl.layer_active, [true, true], "ctrl layerActive");
    assert_eq!(ctrl.min_normal_via_cost, 400.0, "ctrl minNormalViaCost");
    assert_eq!(ctrl.min_cheap_via_cost, 320.0, "ctrl minCheapViaCost");
    assert_eq!(
        harness.trace_angle_restriction(),
        AngleRestriction::FortyfiveDegree,
        "ctrl angleRestriction"
    );
}

/// Asserts one front element against its capture `head` row. `what`
/// names the row for the failure message. `live_door_id` is the
/// Java-faithful id read: Java's capture prints `element.door.getId()`
/// on the LIVE door object, and `DrillPage.getId()` = `31 *
/// shape.getId() + netNumber` embeds the page's CURRENT drill-net
/// state (-1 before its first `getDrills`, the net afterwards). The
/// frozen id stored in the [`ExpandableObject::DrillPage`] variant is
/// the construction-time value — equal to the live read unless the
/// page was drilled between the element's insertion and its head read
/// (DRILL head k=10: frozen -765662255 vs live -765662253, delta =
/// +2·net). Ordering is unaffected: live-vs-frozen can only disagree
/// for |31·Δs| ≤ 2, i.e. the SAME page, where both reads are equal and
/// the comparator falls through to the section anyway.
fn assert_head(
    ctx: &Harness,
    element: &MazeListElement,
    expected: &RawHead,
    what: &str,
    live_door_id: i32,
) {
    assert_eq!(kind_of(&element.door), expected.kind, "{what} doorKind");
    assert_eq!(live_door_id, expected.door_id, "{what} doorId");
    assert_eq!(
        element.section_no_of_door, expected.section,
        "{what} section"
    );
    assert_eq!(
        element.expansion_value, expected.exp,
        "{what} expansionValue"
    );
    assert_eq!(element.sorting_value, expected.sort, "{what} sortingValue");
    let (back_kind, back_id) = match &element.backtrack_door {
        None => (None, -1),
        Some(door) => (Some(kind_of(door)), door.id()),
    };
    assert_eq!(back_kind, expected.back_kind, "{what} backKind");
    assert_eq!(back_id, expected.back_id, "{what} backId");
    assert_eq!(
        element.section_no_of_backtrack_door, expected.back_section,
        "{what} backSection"
    );
    let next_room_id = element
        .next_room_key
        .map(|key| ctx.room_id(key))
        .unwrap_or(-1);
    assert_eq!(next_room_id, expected.next_room_id, "{what} nextRoomId");
    assert_eq!(element.shape_entry.a.x, expected.ax, "{what} ax");
    assert_eq!(element.shape_entry.a.y, expected.ay, "{what} ay");
    assert_eq!(element.shape_entry.b.x, expected.bx, "{what} bx");
    assert_eq!(element.shape_entry.b.y, expected.by, "{what} by");
    // The constants every capture row carries.
    assert!(!element.room_ripped, "{what} roomRipped");
    assert_eq!(element.ripup_cost, 0, "{what} ripupCost");
    assert!(!element.already_checked, "{what} alreadyChecked");
}

/// The drain under capture protocol P: head row k, one
/// `occupy_next_element`, the (before, after, cont) literal. Returns
/// the reached destination (the `cont == false` row), if any.
fn drain(
    engine: &mut MazeSearchEngine<'_, Harness, TargetDistance, LegalChecker>,
    heads: &[RawHead],
    pops: &[(i32, i32, bool)],
    phase: &str,
) -> Option<(ExpandableObject, i32)> {
    let mut dest = None;
    for (k, expected) in heads.iter().enumerate() {
        let head = engine
            .front
            .iter()
            .next()
            .unwrap_or_else(|| panic!("{phase}: front empty at pop {k}"))
            .clone();
        // The Java-faithful id read (see assert_head): DrillPage ids
        // resolve through the LIVE page (its net state may have flipped
        // since the element was inserted).
        let live_door_id = match &head.door {
            ExpandableObject::DrillPage { row, column, .. } => {
                engine.pages.page(*row, *column).get_id()
            }
            _ => head.door.id(),
        };
        assert_head(
            &*engine.ctx,
            &head,
            expected,
            &format!("{phase} head {k}"),
            live_door_id,
        );
        let before = i32::try_from(engine.front.len()).expect("front size");
        let cont = engine.occupy_next_element();
        let after = i32::try_from(engine.front.len()).expect("front size");
        let (eb, ea, ec) = pops[k];
        assert_eq!((before, after, cont), (eb, ea, ec), "{phase} popMeta {k}");
        if !cont {
            let reached = engine
                .destination
                .clone()
                .unwrap_or_else(|| panic!("{phase}: cont=false without a destination"));
            dest = Some(reached);
            break;
        }
    }
    dest
}

/// The MAIN phase: ctrl row, INIT row, the pre-completion door dump,
/// the destination-distance probes, the `doorIsSmall` boundary and the
/// full 28-pop drain to the destination.
#[test]
fn main_phase_protocol_pins() {
    let (mut harness, ctrl, mut distance, mut checker, mut pages) = make_run(false);
    assert_ctrl(&ctrl, &harness, false);

    let mut engine =
        MazeSearchEngine::new(&mut harness, &ctrl, &mut distance, &mut checker, &mut pages);
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "MAIN init ok");

    // The INIT row: a single seed, the RAW row equal to head k=0.
    assert_eq!(engine.front.len(), 1, "init frontSize");
    let seed = engine.front.iter().next().expect("the init seed").clone();
    assert_head(
        &*engine.ctx,
        &seed,
        &MAIN_HEADS[0],
        "INIT head",
        seed.door.id(),
    );

    // The door dump (BEFORE any pop): the start room holds exactly one
    // door pre-completion; `completeNeighbourRooms` adds door 33 only
    // when the drain expands the room — the ordering is pinned.
    let start_room = seed.next_room_key.expect("the seed's room");
    let start_doors = engine.ctx.room_doors(start_room);
    assert_eq!(start_doors.len(), 1, "door dump: exactly one door");
    let start_door = &start_doors[0];
    assert_eq!(start_door.id(), 232558643, "door i=0 id");
    assert_eq!(start_door.dimension, 1, "door i=0 dim");
    let door_shape = engine.ctx.room_shape(start_room); // placeholder read to settle borrows
    let _ = door_shape;
    {
        let (first_key, second_key, first_shape, second_shape) = {
            let ctx = &*engine.ctx;
            let fk = ctx.room_key_of_id(start_door.first_room_id);
            let sk = ctx.room_key_of_id(start_door.second_room_id);
            let fs = ctx.room_shape(fk.expect("live first room"));
            let ss = ctx.room_shape(sk.expect("live second room"));
            (fk, sk, fs, ss)
        };
        let _ = (first_key, second_key);
        let shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
        assert_eq!(
            shape.bounding_box().max_width(),
            38964.0,
            "door i=0 len (IntBox maxWidth)"
        );
    }

    // The destination-distance probes (`dist` rows) — the REAL
    // heuristic, not the T5 targetless stub.
    {
        let dist = &engine.destination_distance;
        let probes = [
            (200000.0, 300000.0, 0, 578750.0),
            (200000.0, 300000.0, 1, 579150.0),
            (500000.0, 300000.0, 0, 278750.0),
            (10000.0, 10000.0, 1, 1037900.0),
        ];
        for (x, y, layer, expected) in probes {
            let value = dist.calculate(&FloatPoint::new(x, y), layer);
            assert_eq!(value, expected, "dist ({x},{y},{layer})");
        }
    }

    // The `doorIsSmall` boundary: strict `<` at exactly the door
    // length (38964.0 → NOT small; 38965.0 → small).
    {
        for (w, expected_small) in [(38963.0, false), (38964.0, false), (38965.0, true)] {
            let small = engine.door_is_small(start_door, w);
            assert_eq!(small, expected_small, "doorIsSmall door 232558643 w={w}");
        }
    }

    // The 28-pop drain to the destination (head rows + popMeta rows).
    let dest = drain(&mut engine, &MAIN_HEADS, &MAIN_POPS, "MAIN");
    let (dest_door, dest_section) = dest.expect("MAIN reaches the destination");
    assert_eq!(dest_door.id(), 158, "dest doorId");
    assert_eq!(dest_section, 0, "dest section");
    assert!(!engine.front.is_empty(), "drainSummary frontEmpty false");

    // The direct occupancy state pin: the door-33 section-0 element
    // (head k=1) popped at k=1 — its section is marked. The popMeta
    // front-size rows are the behavioral mirror of this state.
    let door33 = ExpandableObject::RoomDoor(ExpansionDoor {
        first_room_id: 1,
        second_room_id: 2,
        dimension: 1,
        section_count: 2,
    });
    assert!(
        engine.maze_element(&door33, 0).is_occupied,
        "door 33 section 0 occupied after the drain"
    );
    assert!(
        engine.maze_element(&door33, 1).is_occupied,
        "door 33 section 1 occupied after the drain"
    );
}

/// The DRILL phase: vias ON — the page grid row, then the 40-pop drain
/// interleaving DrillPage, ExpansionDrill and room-door rows (the
/// drill dispatch).
#[test]
fn drill_phase_protocol_pins() {
    let (mut harness, ctrl, mut distance, mut checker, mut pages) = make_run(true);
    assert_ctrl(&ctrl, &harness, true);

    // The pageMeta row: the grid frame the DrillPage ids hang on.
    assert_eq!(harness.default_via_diameter(), 8000.0, "pageMeta viaDiam");
    let max_page_width = max_drill_page_width(harness.default_via_diameter());
    assert_eq!(max_page_width, 40000, "pageMeta maxPageWidth");
    assert_eq!(pages.column_count, 26, "pageMeta columnCount");
    assert_eq!(pages.row_count, 16, "pageMeta rowCount");
    assert_eq!(pages.page_width, 38539, "pageMeta pageWidth");
    assert_eq!(pages.page_height, 37625, "pageMeta pageHeight");

    let mut engine =
        MazeSearchEngine::new(&mut harness, &ctrl, &mut distance, &mut checker, &mut pages);
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "DRILL init ok");

    // The 40-pop drain: cap reached, destination NOT reached (no
    // cont=false row in the capture).
    let dest = drain(&mut engine, &DRILL_HEADS, &DRILL_POPS, "DRILL");
    assert!(dest.is_none(), "DRILL drain hits the cap, not the dest");
    assert!(!engine.front.is_empty(), "drainSummary frontEmpty false");
    assert!(
        engine.destination.is_none(),
        "DRILL: no destination row in the capture"
    );
}

/// The BEND probes: a seeded door element with a live backtrack door
/// and a straight vs diagonal entry chord — the bend penalty +
/// weighted-distance contrast (bendCosts[0] = 47).
#[test]
fn bend_cost_contrast_pins() {
    for (diag, expected) in [(false, &BEND_STRAIGHT_HEADS), (true, &BEND_DIAG_HEADS)] {
        let (mut harness, ctrl, mut distance, mut checker, mut pages) = make_run(false);
        let mut engine =
            MazeSearchEngine::new(&mut harness, &ctrl, &mut distance, &mut checker, &mut pages);
        assert!(
            engine.init(&[START_PIN], &[DEST_PIN]),
            "diag={diag} init ok"
        );

        // The init seed donates the backtrack door and the room.
        let seed_target = engine.front.iter().next().expect("the init seed").clone();
        let room_key = seed_target.next_room_key.expect("the seed's room");
        let doors = engine.ctx.room_doors(room_key);
        assert_eq!(doors.len(), 1, "bendSeed: the pre-pop door set");
        let door = &doors[0];
        assert_eq!(door.id(), 232558643, "bendSeed doorId");

        // allocateSections fires inside getSectionSegments — the
        // production precondition for reading sectionArr
        // (MazeSearchEngine.expandToDoor:715).
        let (first_key, second_key, first_shape, second_shape) = {
            let ctx = &*engine.ctx;
            let fk = ctx.room_key_of_id(door.first_room_id);
            let sk = ctx.room_key_of_id(door.second_room_id);
            let fs = ctx.room_shape(fk.expect("live first room"));
            let ss = ctx.room_shape(sk.expect("live second room"));
            (fk, sk, fs, ss)
        };
        let both_complete_free_space = first_key
            .is_some_and(|k| engine.ctx.room_is_complete_free_space(k))
            && second_key.is_some_and(|k| engine.ctx.room_is_complete_free_space(k));
        let door_shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
        let seed_layer = engine.ctx.room_layer(room_key);
        let seed_half_width = f64::from(ctrl.compensated_trace_half_width[seed_layer as usize]);
        let (section_count, _) = door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            seed_half_width,
        );
        assert_eq!(section_count, 2, "bendSeed sectionCount");

        // The entry chord: straight = axis-parallel, diag = 45
        // degrees, both through the door center.
        let c = door_shape.centre_of_gravity();
        let entry = if diag {
            FloatLine::new(FloatPoint::new(c.x - 2000.0, c.y - 2000.0), c)
        } else {
            FloatLine::new(FloatPoint::new(c.x - 2000.0, c.y), c)
        };
        let seed_elem = MazeListElement::new(
            ExpandableObject::RoomDoor(door.clone()),
            0,
            Some(seed_target.door.clone()),
            seed_target.section_no_of_door,
            100.0,
            100.0,
            Some(room_key),
            entry,
            false,
            Adjustment::None,
            false,
        );
        engine.front.clear();
        assert!(engine.front.add(seed_elem), "bendSeed accepted");
        let before = engine.front.len();
        let _ = engine.occupy_next_element();
        let after = engine.front.len();
        assert_eq!(
            (before, after),
            (1, 3),
            "bendSeed before/after (diag={diag})"
        );

        // The front dump (bendElem rows).
        let elems: Vec<MazeListElement> = engine.front.iter().cloned().collect();
        assert_eq!(elems.len(), expected.len(), "bendElem count (diag={diag})");
        for (element, want) in elems.iter().zip(expected.iter()) {
            assert_head(&*engine.ctx, element, want, "bendElem", element.door.id());
        }
    }
}

/// Java `ExpansionDoor.allocateSections` (`:193-203`) — the
/// re-segmentation contract: re-allocating with the SAME section count
/// is idempotent (marks survive), re-allocating with a DIFFERENT count
/// RESETS the section state. The T6 capture contains no
/// re-segmentation event (no door changes its section count mid-drain),
/// so the drain pins cannot discriminate the reset arm — this direct
/// contract pin closes that coverage boundary (cerebrum mode 10).
#[test]
fn allocate_sections_resegmentation_contract() {
    let (mut harness, ctrl, mut distance, mut checker, mut pages) = make_run(false);
    let mut engine =
        MazeSearchEngine::new(&mut harness, &ctrl, &mut distance, &mut checker, &mut pages);
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "init ok");
    let room_key = engine
        .front
        .iter()
        .next()
        .expect("the init seed")
        .next_room_key
        .expect("the seed's room");
    let doors = engine.ctx.room_doors(room_key);
    let door = ExpandableObject::RoomDoor(doors[0].clone());

    engine.allocate_sections(&door, 2);
    engine.maze_element_mut(&door, 1).is_occupied = true;

    // Same count: idempotent, the mark SURVIVES.
    engine.allocate_sections(&door, 2);
    assert!(
        engine.maze_element(&door, 1).is_occupied,
        "same-count re-allocation keeps the section mark"
    );

    // Different count: re-segmentation RESETS the state.
    engine.allocate_sections(&door, 3);
    assert!(
        !engine.maze_element(&door, 1).is_occupied,
        "re-segmentation resets the section state"
    );
}

/// A comparator tie element — DIRECT struct literal (not
/// `MazeListElement::new`, whose finiteness debug-asserts would
/// reject the NaN probes).
fn tie_elem(id: i32, section: i32, sort: f64, exp: f64) -> MazeListElement {
    MazeListElement {
        door: ExpandableObject::DrillPage {
            row: 0,
            column: 0,
            id,
        },
        section_no_of_door: section,
        backtrack_door: None,
        section_no_of_backtrack_door: 0,
        expansion_value: exp,
        sorting_value: sort,
        next_room_key: None,
        shape_entry: FloatLine::new(FloatPoint::ZERO, FloatPoint::ZERO),
        room_ripped: false,
        adjustment: Adjustment::None,
        already_checked: false,
        ripup_cost: 0,
    }
}

/// The set order of the probe elements as (id, section) pairs.
/// `mutable_key_type` is sound to allow here: [`FrontOrder`]'s `Ord`
/// reads only plain integers (door id, section, the two values) — the
/// interior mutability clippy sees transitively is `Line`'s atomic id
/// cache inside a `Simplex`-typed `TileShape`, which the comparator
/// never touches.
#[allow(clippy::mutable_key_type)]
fn order_of(set: &BTreeSet<FrontOrder>) -> Vec<(i32, i32)> {
    set.iter()
        .map(|e| (e.0.door.id(), e.0.section_no_of_door))
        .collect()
}

/// The comparator's 4-level tie chain + dedup + NaN/-0.0 (probes
/// T1-T7): the BTreeSet front must replicate Java's
/// `<`/`>`-semantics `TreeSet` exactly. (`mutable_key_type` allowed —
/// see `order_of`.)
#[allow(clippy::mutable_key_type)]
#[test]
fn front_tie_chain_pins() {
    // T1: sortingValue decides (order [31, 7]).
    let mut ts: BTreeSet<FrontOrder> = BTreeSet::new();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 20.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(31, 0), (7, 0)],
        "T1 sort 10 vs 20: order by sortingValue"
    );

    // T2: sort equal → expansionValue decides (order [7, 31]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 6.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(7, 0), (31, 0)],
        "T2 sort equal: order by expansionValue"
    );

    // T3: values equal → door id decides (order [7, 31]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(7, 0), (31, 0)],
        "T3 values equal: order by doorId"
    );

    // T4: id equal → section decides (order [(31,1), (31,3)]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 3, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(31, 1, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(31, 1), (31, 3)],
        "T4 id equal: order by section"
    );

    // T5: full tie → the set DEDUPS (size 1, second add rejected).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 2, 10.0, 5.0)));
    let first = ts.insert(FrontOrder(tie_elem(31, 2, 10.0, 5.0)));
    assert_eq!(ts.len(), 1, "T5 full tie: size");
    assert!(!first, "T5 full tie: second add rejected");

    // T6: NaN falls through the value tie-breaks (pairwise verdicts).
    use std::cmp::Ordering;
    let na = tie_elem(3, 0, f64::NAN, 0.0);
    let nb = tie_elem(9, 0, 0.0, 0.0);
    let nc = tie_elem(1, 0, 1.0, 0.0);
    assert_eq!(compare(&na, &nb), Ordering::Less, "T6-na-nb");
    assert_eq!(compare(&nb, &na), Ordering::Greater, "T6-nb-na");
    assert_eq!(compare(&nb, &nc), Ordering::Less, "T6-nb-nc");
    assert_eq!(
        compare(&na, &tie_elem(3, 0, f64::NAN, 0.0)),
        Ordering::Equal,
        "T6-na-na"
    );

    // T7: -0.0 ties 0.0 (full tie with same id/section → dedup).
    let nz = tie_elem(31, 2, -0.0, 5.0);
    let pz = tie_elem(31, 2, 0.0, 5.0);
    assert_eq!(compare(&nz, &pz), Ordering::Equal, "T7-nz-pz");
    ts.clear();
    ts.insert(FrontOrder(nz));
    let second = ts.insert(FrontOrder(pz));
    assert_eq!(ts.len(), 1, "T7 -0.0 vs 0.0: size");
    assert!(!second, "T7 -0.0 vs 0.0: second add rejected");
}
