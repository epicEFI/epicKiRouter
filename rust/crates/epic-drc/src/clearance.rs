//! The clearance-violation walk — the port of Java
//! `Item.clearanceViolations()` (`Item.java:367-495`), the per-kind
//! `isObstacle(Item)` matrix, and the board-wide
//! `DesignRulesChecker.getAllClearanceViolations()` (`:56-87`), frozen
//! at `e7f9bdf1` and pinned record-for-record by the drc corpus.
//!
//! ## The per-item walk (`Item.java:367-495`)
//!
//! For every tree shape `i` of the walking item: the
//! `overlappingTreeEntriesWithClearance` query on the DEFAULT tree
//! (query shape, `shapeLayer(i)`, no ignore nets, THIS item's
//! clearance class); per surviving entry, `currentItem.isObstacle(this)`
//! — note the ORIENTATION: the matrix dispatches on the OTHER item as
//! receiver — then the two exemptions, then the enlarged-intersection
//! gate:
//!
//! 1. **Tie-pin exemption** (`:383-413`): two Traces are allowed to
//!    overlap without sharing a net when both contact the same tie
//!    pin — the WALKING trace's endpoint contacts under
//!    `ignoreNet = true` are searched for the other trace, and any
//!    Pin contact sharing nets with BOTH clears the flag.
//! 2. **Outline exemption** (`:415-430`): a Pin against the
//!    BoardOutline is not an obstacle when EVERY corner of the pin's
//!    actual tile shape lies inside the outline (edge connectors and
//!    castellated pads whose pads protrude stay obstacles).
//! 3. **The gate** (`:432-491`): `minimumClearance` is
//!    `getValue(currentItem.class, this.class, layer)` — the OTHER
//!    item's class is the ROW (asymmetric matrix, T54);
//!    `clComp` is the tree's compensation values when compensation is
//!    used, else the `(int) Math.round(0.5 * min)` /
//!    `Math.round(min - clComp1)` split; each shape is enlarged ONLY
//!    when its compensation is `> 0` (the conditional is exact —
//!    `enlarge(0)` is not applied); the violation exists iff the
//!    intersection has dimension 2.
//!
//!    **The #925a shortfall-tolerance gate** (upstream `14b28b6ff`,
//!    post-`e7f9bdf1` — deliberate divergence 1 from the frozen
//!    walk): a violation whose shortfall `minimumClearance −
//!    actualClearance` converts to ≤ the board's clearance tolerance
//!    in MICROMETRES (default 1.0 — floating-point discretization and
//!    imperial↔metric rounding noise, not an electrical defect) is
//!    DROPPED, STRICT `>` at the boundary. The tolerance is
//!    `BoardRules::clearance_tolerance_um` (seeded 1.0 at both
//!    construction faces; the settings override writes it through
//!    `epic_engine::drc_tolerance`); non-finite/negative CLAMP to 0.0
//!    in the measure loop (defense in depth — the settings face
//!    rejects them with a warn). Upstream's guard also wraps the
//!    `smallestClearance` bookkeeping; that face stays unported, so
//!    the port reduces to the record push. Tolerance 0.0 reproduces
//!    the pre-#925a behavior for every STRICTLY-POSITIVE shortfall —
//!    but a shortfall of EXACTLY 0.0 (a zero-rule, zero-measured
//!    pair, e.g. drc-main's outline vs its edge-crossing pin after
//!    the copper-to-edge override leaves the pin's class cell at 0)
//!    is dropped at EVERY tolerance, 0.0 included (`0.0 > 0.0` is
//!    false) — upstream's gate behaves identically, so this is
//!    upstream-faithful, not a divergence.
//!
//! ## The Pin-Pin exemptions (upstream `14b28b6ff`, #925b — deliberate
//! divergence 2)
//!
//! The `Pin` row of the `isObstacle` matrix gains TWO same-component
//! exemptions (the composite-pad half of #925; both live inside
//! [`is_obstacle`], so every walk face and every router consumer of
//! the matrix sees them):
//!
//! * **Same-net pins** (after the Trace check): two pins that share
//!   a net are never obstacles — composite pads, thermal vias in
//!   pad, stitching vias, internally connected pins across
//!   components (#925b scoped this to ONE component;
//!   `this.getComponentId() > 0 && equal`; #931 cluster G dropped
//!   the guard — same-net pins are electrically connected
//!   regardless of footprint).
//! * **Netless sub-pads of one logical pad** (inside the
//!   no-shared-net arm): BOTH pins netless, same component, and
//!   [`base_pin_name`] equal and non-empty (`PAD@1`/`PAD@2`,
//!   `pad_1_1`/`pad_1_2` — one logical pad split into sub-pads that
//!   the netlist leaves unconnected).
//!
//! Upstream ships NO knob for either exemption (unlike #925a's
//! tolerance) — but the frozen corpus DOES carry such pairs (drc-0003
//! DAC2020_bm06: ALL 8 golden rows; drc-0009 ecc83-pp_v2: 2 of 16 —
//! real reference boards place same-component pins inside their
//! clearance rules). `BoardRules::same_component_pin_exemptions`
//! (seeded `true` = upstream HEAD) exists solely so the corpus walk
//! can read the frozen pre-#925b face — the P4 corpus law, the exact
//! `clearance_tolerance_um` precedent. The gate POLICY is pinned by
//! the epic-drc unit pins, not by the corpus.
//!
//! `smallestClearance` stays unported (label-only in every surface).
//! `actualClearance` was label-only until T12: the V2 router score's
//! `clearanceViolations.totalViolationUm` (BoardStatistics) consumes
//! the per-violation shortfall, so [`ViolationRec`] and
//! [`all_clearance_violation_depths`] now carry it — the 16-iteration
//! bisection port ([`calculate_clearance_between_two_shapes`],
//! `Item.java:497-519`). The (a, b, layer) rows are unchanged.
//!
//! ## The dedup (`DesignRulesChecker.java:56-87`)
//!
//! The board walk (insertion order = ascending id for a parse board)
//! dedups A-B vs B-A by the sorted-id pair + layer key, keeping the
//! FIRST occurrence; the corpus rows are then canonicalized by
//! (a, b, layer) — exactly the DrcOracle emission. NOTE: Java's
//! `getAllClearanceViolations` itself returns the collection in WALK
//! order (no sort); the depth sum in `BoardStatistics` iterates THAT
//! order, so [`all_clearance_violation_depths`] is walk-ordered and
//! UNSORTED — float addition order matters for the parity sum.

use std::collections::HashSet;

use epic_board::board::Board;
use epic_board::components::{base_pin_name, pin_name};
use epic_board::contacts::{items_share_net, normal_contacts};
use epic_board::id::ItemId;
use epic_board::items::{BoardItemType, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_board::tree_shapes::clearance_compensation_value;
use epic_geometry::rounding::java_round;
use epic_geometry::tile_shape::TileShape;

/// One deduped violation: `(min(id1, id2), max(id1, id2), layer)` —
/// the record shape of the canonical (a, b, layer) sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViolationRow {
    /// `min(id1, id2)`.
    pub a: i64,
    /// `max(id1, id2)`.
    pub b: i64,
    /// The 0-based layer of the walking item's shape.
    pub layer: i64,
}

/// One violation WITH the clearance depth fields (Java
/// `ClearanceViolation.expectedClearance` / `.actualClearance`, board
/// units) — the T12 face consumed by the V2 router score's
/// `totalViolationUm`. Java `ClearanceViolation.java`: the fields are
/// plain doubles in BOARD UNITS; the um scaling happens in
/// `BoardStatistics` via `boardUnitToUmFactor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViolationRec {
    /// The OTHER item of the pair (Java `secondItem`; the walker is
    /// `firstItem`).
    pub other: ItemId,
    /// The walking item's shape layer.
    pub layer: i32,
    /// Java `expectedClearance` = the matrix value
    /// (current class, walker class, layer) in board units.
    pub expected_clearance: f64,
    /// Java `actualClearance` = the 16-iteration bisection result in
    /// board units (0.0 when the raw shapes already overlap in 2D).
    pub actual_clearance: f64,
}

/// A walk-ordered deduped violation with its depth fields, the ids
/// canonicalized exactly like [`ViolationRow`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthRow {
    pub a: i64,
    pub b: i64,
    pub layer: i64,
    pub expected_clearance: f64,
    pub actual_clearance: f64,
}

/// Java `Item.calculateClearanceBetweenTwoShapes`
/// (`Item.java:497-521`): the raw shapes ALREADY overlapping in 2D
/// answer 0.0; otherwise a 16-iteration bisection over
/// `[0, minimumClearance]` — each mid enlarges shape1 by
/// `mid * clComp1 / (clComp1 + clComp2)` and shape2 by the
/// complementary fraction (0.5/0.5 when the compensations sum to 0),
/// and the bracket shrinks toward the exact contact distance; `low`
/// (the largest non-overlapping enlargement) is the answer.
#[must_use]
fn calculate_clearance_between_two_shapes(
    raw_shape1: &TileShape,
    raw_shape2: &TileShape,
    minimum_clearance: f64,
    cl_comp1: i32,
    cl_comp2: i32,
) -> f64 {
    if raw_shape1.intersection(raw_shape2).dimension() == 2 {
        return 0.0;
    }
    let mut low = 0.0f64;
    let mut high = minimum_clearance;
    let sum_comp = f64::from(cl_comp1) + f64::from(cl_comp2);
    let factor1 = if sum_comp > 0.0 {
        f64::from(cl_comp1) / sum_comp
    } else {
        0.5
    };
    let factor2 = if sum_comp > 0.0 {
        f64::from(cl_comp2) / sum_comp
    } else {
        0.5
    };
    for _ in 0..16 {
        let mid = (low + high) * 0.5;
        let s1 = raw_shape1.enlarge(mid * factor1);
        let s2 = raw_shape2.enlarge(mid * factor2);
        if s1.intersection(&s2).dimension() == 2 {
            high = mid;
        } else {
            low = mid;
        }
    }
    low
}

#[must_use]
fn kind_of(board: &Board, id: ItemId) -> Option<BoardItemType> {
    board.get(id).map(|entry| entry.board_item_type())
}

/// Any `ObstacleArea`-instanceof: the BASE class and its subclasses —
/// including [`BoardItemType::ConductionArea`], which EXTENDS
/// `ObstacleArea` (`ConductionArea.java:25`), so every base-class
/// check (`Pin.isObstacle:354`, `BoardOutline.isObstacle:86`) catches
/// conduction areas regardless of their own `isObstacle` flag.
#[must_use]
fn is_any_obstacle_area(kind: Option<BoardItemType>) -> bool {
    matches!(
        kind,
        Some(
            BoardItemType::ObstacleArea
                | BoardItemType::ViaObstacleArea
                | BoardItemType::ComponentObstacleArea
                | BoardItemType::ConductionArea
        )
    )
}

/// Java `Pin.drillAllowed()` (`Pin.java:348-350`): vias may drill
/// through this pin's pads iff the pin spans a single layer.
#[must_use]
fn pin_drill_allowed(board: &mut Board, pin: ItemId) -> bool {
    board.item_first_layer(pin) == board.item_last_layer(pin)
}

/// Upstream `14b28b6ff` (#925b), the shared guard of both Pin-Pin
/// exemptions: `this.getComponentId() > 0 &&
/// this.getComponentId() == otherPin.getComponentId()` — both items
/// belong to the SAME placed component (id 0 belongs to no
/// component).
fn same_component_pins(board: &Board, a: ItemId, b: ItemId) -> bool {
    board.get(a).is_some_and(|ea| {
        board
            .get(b)
            .is_some_and(|eb| ea.component_id > 0 && ea.component_id == eb.component_id)
    })
}

/// Upstream `14b28b6ff`, the netless-sub-pad guard:
/// `this.netCount() == 0 && otherPin.netCount() == 0`.
fn both_netless(board: &Board, a: ItemId, b: ItemId) -> bool {
    board.get(a).is_some_and(|ea| {
        board
            .get(b)
            .is_some_and(|eb| ea.nets.is_empty() && eb.nets.is_empty())
    })
}

/// Upstream `Pin.isSameLogicalPad` (`14b28b6ff`): both pins resolve a
/// package pin NAME from the shared component's package, and the
/// composite-sub-pad base names ([`base_pin_name`]) are equal and
/// non-empty. Java's null guards (`board`, `components`, `package`,
/// null pin names) collapse to `Option` here; the pin-index bounds
/// check is epic-board's `resolve_package_pin`/`get_pin`.
fn same_logical_pad(board: &Board, a: ItemId, b: ItemId) -> bool {
    let Some(ea) = board.get(a) else {
        return false;
    };
    let ItemData::Pin {
        pin_index: index_a, ..
    } = ea.data
    else {
        return false;
    };
    let Some(eb) = board.get(b) else {
        return false;
    };
    let ItemData::Pin {
        pin_index: index_b, ..
    } = eb.data
    else {
        return false;
    };
    // The callers guarantee `ea.component_id == eb.component_id`, so
    // BOTH names resolve from pin a's component — upstream reads
    // `pin1.board.components.get(pin1.getComponentId())` for both.
    let Some(component_id) = u32::try_from(ea.component_id).ok() else {
        return false;
    };
    let Some(name_a) = pin_name(board.components(), board.library(), component_id, index_a) else {
        return false;
    };
    let Some(name_b) = pin_name(board.components(), board.library(), component_id, index_b) else {
        return false;
    };
    let (base_a, base_b) = (base_pin_name(name_a), base_pin_name(name_b));
    !base_a.is_empty() && base_a == base_b
}

/// Java `isObstacle(Item other)` — the per-kind override matrix, with
/// the WALK-DAY orientation of the call site: `receiver` is
/// `currentItem` (the tree entry), `other` is the walking item
/// (`Item.java:382`). Every branch mirrors its Java override
/// verbatim; the override set is complete (the method is abstract on
/// `Item`), so the catch-all is unreachable for board items.
pub fn is_obstacle(board: &mut Board, receiver: ItemId, other: ItemId) -> bool {
    let Some(receiver_kind) = kind_of(board, receiver) else {
        return false;
    };
    let other_kind = kind_of(board, other);
    let shares_net = items_share_net(board, receiver, other);
    let other_conduction_open = matches!(other_kind, Some(BoardItemType::ConductionArea))
        && !board.get(other).is_some_and(|entry| match &entry.data {
            ItemData::ConductionArea { is_obstacle, .. } => *is_obstacle,
            _ => false,
        });
    match receiver_kind {
        // Trace.java:92-102.
        BoardItemType::Trace => {
            if receiver == other
                || matches!(
                    other_kind,
                    Some(BoardItemType::ViaObstacleArea | BoardItemType::ComponentObstacleArea)
                )
            {
                return false;
            }
            if other_conduction_open {
                return false;
            }
            !shares_net
        }
        // Via.java:152-166.
        BoardItemType::Via => {
            if receiver == other || matches!(other_kind, Some(BoardItemType::ComponentObstacleArea))
            {
                return false;
            }
            if other_conduction_open {
                return false;
            }
            if !shares_net {
                return true;
            }
            if matches!(other_kind, Some(BoardItemType::Trace)) {
                return false;
            }
            let attach_allowed = board.get(receiver).is_some_and(|entry| match &entry.data {
                ItemData::Via {
                    attach_smd_allowed, ..
                } => *attach_smd_allowed,
                _ => false,
            });
            !attach_allowed
                || !matches!(other_kind, Some(BoardItemType::Pin))
                || !pin_drill_allowed(board, other)
        }
        // Pin.java:353-380 + the #925b exemptions (upstream
        // 14b28b6ff) widened by #931 cluster G — the ObstacleArea
        // BASE class exemption (all three keepout kinds AND the
        // conduction areas — ConductionArea extends ObstacleArea),
        // then TWO Pin-Pin exemptions (gated on
        // BoardRules::same_component_pin_exemptions, the P4 corpus
        // law): netless sub-pads of one logical pad inside the
        // no-shared-net arm, and — post-#931, guardless — ANY
        // same-net pin pair after the Trace check (#931 dropped the
        // same-component conjunct: stitching vias and internally
        // connected pins across components are electrically
        // connected too).
        BoardItemType::Pin => {
            if receiver == other || is_any_obstacle_area(other_kind) {
                return false;
            }
            let pin_exemptions_on = board.rules().same_component_pin_exemptions;
            if !shares_net {
                if pin_exemptions_on
                    && matches!(other_kind, Some(BoardItemType::Pin))
                    && same_component_pins(board, receiver, other)
                    && both_netless(board, receiver, other)
                    && same_logical_pad(board, receiver, other)
                {
                    return false;
                }
                return true;
            }
            if matches!(other_kind, Some(BoardItemType::Trace)) {
                return false;
            }
            if pin_exemptions_on && matches!(other_kind, Some(BoardItemType::Pin)) {
                return false;
            }
            !pin_drill_allowed(board, receiver) || !matches!(other_kind, Some(BoardItemType::Via))
        }
        // ConductionArea.java:380-385 — gated on the area's own
        // isObstacle flag, then the ObstacleArea rule.
        BoardItemType::ConductionArea => {
            let flag = board.get(receiver).is_some_and(|entry| match &entry.data {
                ItemData::ConductionArea { is_obstacle, .. } => *is_obstacle,
                _ => false,
            });
            if !flag {
                return false;
            }
            if shares_net {
                return false;
            }
            matches!(other_kind, Some(BoardItemType::Trace | BoardItemType::Via))
        }
        // ObstacleArea.java:175-180.
        BoardItemType::ObstacleArea => {
            if shares_net {
                return false;
            }
            matches!(other_kind, Some(BoardItemType::Trace | BoardItemType::Via))
        }
        // ViaObstacleArea.java:92-97.
        BoardItemType::ViaObstacleArea => {
            if shares_net {
                return false;
            }
            matches!(other_kind, Some(BoardItemType::Via))
        }
        // ComponentObstacleArea.java:64-68.
        BoardItemType::ComponentObstacleArea => {
            receiver != other
                && matches!(other_kind, Some(BoardItemType::ComponentObstacleArea))
                && board.get(other).map(|entry| entry.component_id)
                    != board.get(receiver).map(|entry| entry.component_id)
        }
        // ComponentOutline.java:120-122.
        BoardItemType::ComponentOutline => false,
        // BoardOutline.java:162-171 + M11-T6 (upstream #931): the
        // outline/ObstacleArea base exemptions (the ObstacleArea base
        // class includes the conduction areas), then the Trace arm —
        // a trace whose net list is ALL edge-pin nets is not an
        // obstacle for the outline (`!blocksNets`).
        BoardItemType::BoardOutline => {
            if matches!(other_kind, Some(BoardItemType::BoardOutline))
                || is_any_obstacle_area(other_kind)
            {
                return false;
            }
            if matches!(other_kind, Some(BoardItemType::Trace)) {
                let nets: Vec<i32> = board
                    .get(other)
                    .map(|entry| entry.nets.clone())
                    .unwrap_or_default();
                return board.outline_blocks_nets(receiver, &nets);
            }
            true
        }
        // BoardItemType.OTHER has no Java item class (no parse-time
        // item maps to it) and `isObstacle` is abstract on Item, so
        // the arm is unreachable for board items; conservative false.
        BoardItemType::Other => false,
    }
}

/// Java `outlineContainsTileShape` (`Item.java:528-537`) over
/// `BoardOutline.contains` (`BoardOutline.java:266-276`, any shape):
/// every corner of the tile shape must lie inside SOME outline shape.
fn outline_contains_tile_shape(board: &Board, outline: ItemId, shape: &TileShape) -> bool {
    let Some(shapes) = board.outline_shapes(outline) else {
        return false;
    };
    let contains = |corner: &epic_geometry::point::Point| {
        shapes.iter().any(|board_shape| match board_shape {
            epic_board::items::BoardShape::Tile(tile) => tile.contains_point(corner),
            epic_board::items::BoardShape::PolygonShape(polygon) => polygon.contains_point(corner),
            epic_board::items::BoardShape::Circle(circle) => circle.contains_point(corner),
        })
    };
    (0..shape.border_line_count()).all(|ci| contains(&shape.corner(ci as i32)))
}

/// The tie-pin exemption (`Item.java:383-413`): the WALKING trace's
/// endpoint contacts under `ignore_net = true` are searched for
/// `other`; a Pin contact sharing nets with BOTH clears the obstacle
/// flag. `contacts` mirrors Java's `currentContacts` variable — the
/// first corner's set, REASSIGNED to the last corner's set only when
/// the first did not contain the other trace — and the clear loop
/// runs only when the other trace was found.
fn tie_pin_exemption(
    manager: &SearchTreeManager,
    board: &mut Board,
    walker: ItemId,
    other: ItemId,
    first_corner: &epic_geometry::point::Point,
    last_corner: &epic_geometry::point::Point,
) -> bool {
    let mut contacts = normal_contacts(manager, board, walker, first_corner, true);
    if !contacts.contains(&other) {
        contacts = normal_contacts(manager, board, walker, last_corner, true);
    }
    if !contacts.contains(&other) {
        return true; // contact not found — the flag stands
    }
    for contact in contacts {
        if matches!(kind_of(board, contact), Some(BoardItemType::Pin))
            && items_share_net(board, contact, walker)
            && items_share_net(board, contact, other)
        {
            return false; // cleared
        }
    }
    true
}

/// Java `Item.clearanceViolations()` with the depth fields filled in —
/// the records face of the walk. See the module docs for the walk.
/// `&mut` on both arguments: the entry query fills the shape-precalc
/// caches as it runs.
pub fn item_clearance_violation_records(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    id: ItemId,
) -> Vec<ViolationRec> {
    let Some(this_class) = board.item_clearance_class(id) else {
        return Vec::new();
    };
    let (tree_oid, variant, tree_class) = {
        let tree = manager.default_tree();
        (
            tree.object_id(),
            tree.variant,
            tree.compensated_clearance_class,
        )
    };
    let compensation_used = manager.is_clearance_compensation_used();
    let shapes = board.tree_shape_precalc(id, tree_oid, variant, tree_class);

    let walker_kind = kind_of(board, id);
    let endpoints: Option<(epic_geometry::point::Point, epic_geometry::point::Point)> =
        if walker_kind == Some(BoardItemType::Trace) {
            let lines = board.trace_polyline(id);
            match lines {
                Some(lines) => match (
                    epic_board::items::trace::first_corner(lines),
                    epic_board::items::trace::last_corner(lines),
                ) {
                    (Some(first), Some(last)) => Some((first, last)),
                    _ => None,
                },
                None => None,
            }
        } else {
            None
        };

    // The #925a gate inputs (module docs, item 3): the tolerance in
    // MICROMETRES with the measure-loop clamp (non-finite/negative →
    // 0.0 — the settings face warns+rejects, this is defense in
    // depth), and the board-unit→µm factor so the tolerance is
    // resolution- and unit-independent (a 1-DBU shortfall is 2.54 µm
    // on a resolution-mil-10 board but 0.1 µm on resolution-um-10).
    let tolerance_um = {
        let value = board.rules().clearance_tolerance_um;
        if value.is_finite() && value >= 0.0 {
            value
        } else {
            0.0
        }
    };
    let um_factor = epic_board::aesthetics::board_unit_to_um_factor(board);

    let mut result = Vec::new();
    for (i, shape_slot) in shapes.iter().enumerate() {
        // A None slot holds no leaf (Java's precalculated array slot
        // would be null — unreachable on a static parse board; the
        // walk simply cannot query without a shape).
        let Some(shape1) = shape_slot.clone() else {
            continue;
        };
        let Some(layer) = board.item_shape_layer(id, i as i32) else {
            continue;
        };
        let entries = manager.overlapping_tree_entries_with_clearance(
            board,
            SearchTreeManager::DEFAULT_TREE_INDEX,
            &shape1,
            layer,
            &[],
            this_class,
        );
        for entry in entries {
            let Some(current) = SearchTreeManager::item_of_entry_key(entry.object_key) else {
                continue;
            };
            if current == id {
                continue;
            }
            let current_kind = kind_of(board, current);
            let Some(current_class) = board.item_clearance_class(current) else {
                continue;
            };

            let mut obstacle = is_obstacle(board, current, id);

            // The tie-pin exemption (walker must be a Trace, the other
            // item a Trace).
            if obstacle
                && walker_kind == Some(BoardItemType::Trace)
                && current_kind == Some(BoardItemType::Trace)
                && let Some((first, last)) = &endpoints
            {
                obstacle = tie_pin_exemption(manager, board, id, current, first, last);
            }

            // The outline exemption for TRACES (M11-T6, upstream #931,
            // `Item.java:419-427` — both walker directions): an
            // outline does not violate against a trace whose net list
            // is ALL edge-pin nets. (The walker-is-trace direction is
            // also decided by the outline-receiver arm of `is_obstacle`
            // — Java carries the same redundancy in the override and
            // the explicit arm.)
            if obstacle
                && ((walker_kind == Some(BoardItemType::BoardOutline)
                    && current_kind == Some(BoardItemType::Trace))
                    || (walker_kind == Some(BoardItemType::Trace)
                        && current_kind == Some(BoardItemType::BoardOutline)))
            {
                let (outline_id, trace_id) = if walker_kind == Some(BoardItemType::BoardOutline) {
                    (id, current)
                } else {
                    (current, id)
                };
                let trace_nets: Vec<i32> = board
                    .get(trace_id)
                    .map(|entry| entry.nets.clone())
                    .unwrap_or_default();
                if !board.outline_blocks_nets(outline_id, &trace_nets) {
                    obstacle = false;
                }
            }

            // The outline exemption (Item.java:415-430).
            if obstacle
                && ((walker_kind == Some(BoardItemType::BoardOutline)
                    && current_kind == Some(BoardItemType::Pin))
                    || (walker_kind == Some(BoardItemType::Pin)
                        && current_kind == Some(BoardItemType::BoardOutline)))
            {
                let outline_id = if walker_kind == Some(BoardItemType::BoardOutline) {
                    id
                } else {
                    current
                };
                let pin_shape = if walker_kind == Some(BoardItemType::Pin) {
                    Some(shape1.clone())
                } else {
                    board
                        .tree_shape_precalc(current, tree_oid, variant, tree_class)
                        .get(entry.shape_index_in_object as usize)
                        .and_then(|slot| slot.clone())
                };
                if let Some(pin_shape) = pin_shape
                    && outline_contains_tile_shape(board, outline_id, &pin_shape)
                {
                    obstacle = false;
                }
            }

            if !obstacle {
                continue;
            }

            // The gate (Item.java:432-491).
            let Some(shape2) = board
                .tree_shape_precalc(current, tree_oid, variant, tree_class)
                .get(entry.shape_index_in_object as usize)
                .and_then(|slot| slot.clone())
            else {
                // Java warns "unexpected null shape" and skips.
                continue;
            };
            // The OTHER item's class is the ROW (asymmetric matrix).
            let minimum_clearance =
                board
                    .rules()
                    .clearance
                    .get_value(current_class, this_class, layer);
            let (cl_comp1, cl_comp2) = if compensation_used {
                (
                    clearance_compensation_value(board.rules(), this_class, tree_class, layer),
                    clearance_compensation_value(board.rules(), current_class, tree_class, layer),
                )
            } else {
                // (int) Math.round(...): Math.round first (java_round),
                // then the narrowing cast (clearance values are small).
                let cl_comp1 = java_round(0.5 * f64::from(minimum_clearance)) as i32;
                let cl_comp2 =
                    java_round(f64::from(minimum_clearance) - f64::from(cl_comp1)) as i32;
                (cl_comp1, cl_comp2)
            };
            let enlarged1 = if cl_comp1 > 0 {
                shape1.enlarge(f64::from(cl_comp1))
            } else {
                shape1.clone()
            };
            let enlarged2 = if cl_comp2 > 0 {
                shape2.enlarge(f64::from(cl_comp2))
            } else {
                shape2.clone()
            };
            let intersection = enlarged1.intersection(&enlarged2);
            if intersection.dimension() == 2 {
                // Java measures actualClearance from the RAW shapes at
                // exactly this site (`Item.java:462-467`).
                let actual_clearance = calculate_clearance_between_two_shapes(
                    &shape1,
                    &shape2,
                    f64::from(minimum_clearance),
                    cl_comp1,
                    cl_comp2,
                );
                // The #925a gate (module docs, item 3): a shortfall
                // within the µm tolerance is discretization/rounding
                // noise — dropped, STRICT `>` (exactly-at-tolerance
                // drops). Upstream wraps the smallestClearance
                // bookkeeping too; that face is unported, so the port
                // reduces to this push.
                let shortfall_um = (f64::from(minimum_clearance) - actual_clearance) * um_factor;
                if shortfall_um > tolerance_um {
                    result.push(ViolationRec {
                        other: current,
                        layer,
                        expected_clearance: f64::from(minimum_clearance),
                        actual_clearance,
                    });
                }
            }
        }
    }
    result
}

/// Java `Item.clearanceViolations()` minus the label-only fields: the
/// `(other item, layer)` pairs this item violates against — the
/// historical (a, b, layer) face over [`item_clearance_violation_records`].
pub fn item_clearance_violations(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    id: ItemId,
) -> Vec<(ItemId, i32)> {
    item_clearance_violation_records(manager, board, id)
        .into_iter()
        .map(|rec| (rec.other, rec.layer))
        .collect()
}

/// Java `getAllClearanceViolations` (`DesignRulesChecker.java:56-87`)
/// WITH depths: the board walk (ascending id = insertion order), the
/// A-B/B-A dedup keyed by the sorted pair + layer keeping the FIRST
/// occurrence, returned in WALK ORDER (Java's collection order; the
/// V2 score's float sum depends on it — see the module docs).
/// Returns the deduped total and the rows.
pub fn all_clearance_violation_depths(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> (i64, Vec<DepthRow>) {
    // Collect the walk ids up front: the iterator borrows the board,
    // the per-item query needs it mutably (Java's getItems() snapshot
    // has the same snapshot semantics on a static board).
    let walkers: Vec<ItemId> = board.iter_ascending().map(|entry| entry.id).collect();
    let mut seen: HashSet<(i64, i64, i64)> = HashSet::new();
    let mut rows = Vec::new();
    for walker in walkers {
        for rec in item_clearance_violation_records(manager, board, walker) {
            let a = i64::from(walker.get().min(rec.other.get()));
            let b = i64::from(walker.get().max(rec.other.get()));
            let layer = i64::from(rec.layer);
            if seen.insert((a, b, layer)) {
                rows.push(DepthRow {
                    a,
                    b,
                    layer,
                    expected_clearance: rec.expected_clearance,
                    actual_clearance: rec.actual_clearance,
                });
            }
        }
    }
    // Deliberately UNSORTED: Java returns the walk-ordered ArrayList.
    let total = rows.len() as i64;
    (total, rows)
}

/// Java `getAllClearanceViolations` (`DesignRulesChecker.java:56-87`):
/// the board walk (ascending id = insertion order), the A-B/B-A dedup
/// keyed by the sorted pair + layer keeping the FIRST occurrence, and
/// the canonical (a, b, layer) sort. Returns the deduped total and the
/// rows (the M3 gate consumes the total; the corpus pins the rows).
pub fn all_clearance_violations(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> (i64, Vec<ViolationRow>) {
    let (total, depths) = all_clearance_violation_depths(manager, board);
    let mut rows: Vec<ViolationRow> = depths
        .into_iter()
        .map(|row| ViolationRow {
            a: row.a,
            b: row.b,
            layer: row.layer,
        })
        .collect();
    rows.sort_unstable_by_key(|row| (row.a, row.b, row.layer));
    (total, rows)
}

#[cfg(test)]
mod pins {
    use epic_board::board::Board;
    use epic_board::id::ItemId;
    use epic_board::items::BoardItemType;

    use super::{
        ViolationRow, all_clearance_violations, both_netless, is_obstacle, same_component_pins,
        same_logical_pad,
    };
    use crate::test_util::{
        DSN_MAIN, DSN_P4, DSN_TIE, DSN_TIE_CONTRAST, ca_is_open, id_at_corner, ids_of_kind,
        net_list, nets_of, parse,
    };

    fn kind_of(board: &Board, id: ItemId) -> Option<BoardItemType> {
        board.get(id).map(|entry| entry.board_item_type())
    }

    fn sorted_pair(a: ItemId, b: ItemId) -> (i64, i64) {
        let (x, y) = (i64::from(a.get()), i64::from(b.get()));
        (x.min(y), x.max(y))
    }

    fn rows_contain(rows: &[ViolationRow], a: ItemId, b: ItemId) -> usize {
        let key = sorted_pair(a, b);
        // Layer literals are left to the corpus pins; the unit pins
        // target the MATRIX and DEDUP behavior, identified by pair.
        rows.iter().filter(|row| (row.a, row.b) == key).count()
    }

    /// THE TRAP (this round's parity bug, jar-probed): ConductionArea
    /// EXTENDS ObstacleArea (`ConductionArea.java:25`), so
    /// Pin.isObstacle's base-class instanceof check catches a CA
    /// regardless of flags or nets — false BOTH ways for a foreign,
    /// open wiring rect. The two cells ride DIFFERENT arms (CA
    /// receiver: the open flag; Pin receiver: the base-class catch),
    /// so neither is a tautology for the other.
    #[test]
    fn the_ca_base_class_trap_cells() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nq = net_list(&board, "NQ");
        let ca = net_list(&board, "NC")[0];
        let p1 = id_at_corner(&manager, &mut board, &nq, 10000.0, 10000.0);
        assert!(
            ca_is_open(&board, ca),
            "wiring rects parse OPEN (reader fact)"
        );
        assert!(!is_obstacle(&mut board, ca, p1), "open CA receiver → false");
        assert!(
            !is_obstacle(&mut board, p1, ca),
            "Pin receiver: the ObstacleArea base-class instanceof catches the CA"
        );
    }

    /// BoardOutline + closed keepout cells. The outline exempts its
    /// own kind AND the whole ObstacleArea base class (CAs included)
    /// but NOT pins; a closed keepout area is an obstacle against
    /// foreign traces and never against pins (the ObstacleArea rule
    /// only fires on Trace|Via) — the contrast pair.
    #[test]
    fn outline_and_keepout_cells() {
        let (manager, mut board) = parse(DSN_MAIN);
        let outline = ids_of_kind(&board, BoardItemType::BoardOutline)[0];
        let keepout = ids_of_kind(&board, BoardItemType::ObstacleArea)[0];
        let ca = net_list(&board, "NC")[0];
        let nx = net_list(&board, "NX")[0];
        let nq = net_list(&board, "NQ");
        let p1 = id_at_corner(&manager, &mut board, &nq, 10000.0, 10000.0);
        assert!(
            !is_obstacle(&mut board, outline, ca),
            "outline vs CA (base class)"
        );
        assert!(
            !is_obstacle(&mut board, outline, keepout),
            "outline vs keepout"
        );
        assert!(
            !is_obstacle(&mut board, outline, outline),
            "outline self-kind exempt"
        );
        assert!(
            is_obstacle(&mut board, outline, p1),
            "pins are NOT in the base class"
        );
        assert!(
            is_obstacle(&mut board, p1, outline),
            "Pin receiver: no shared net, no catch"
        );
        assert!(
            is_obstacle(&mut board, keepout, nx),
            "closed area vs foreign trace"
        );
        assert!(
            is_obstacle(&mut board, nx, keepout),
            "foreign trace vs closed area"
        );
        assert!(
            !is_obstacle(&mut board, keepout, p1),
            "contrast: the ObstacleArea rule never fires against a Pin"
        );
        assert!(
            !is_obstacle(&mut board, p1, keepout),
            "Pin receiver: base-class catch"
        );
    }

    /// Via/trace cells plus the SAME-net pin cell. PARITY DECISION
    /// (P4, upstream `14b28b6ff`): this pin originally asserted the
    /// pre-#925b verbatim quirk — "SAME-net pins are still obstacles
    /// (Pin.java:364 verbatim)" — because at the frozen baseline
    /// `e7f9bdf1` the tail `!drillAllowed || !(other instanceof Via)`
    /// made ANY same-net Pin-Pin pair an obstacle. Upstream then
    /// added the same-component exemption for exactly this shape
    /// (composite pads / thermals in pad / internally connected
    /// footprint pins), and P1/P2 both sit on CMP1 — so the assert
    /// now pins the EXEMPT verdict, deliberately diverging from the
    /// frozen walk (the drc-main golden rows are unaffected: the
    /// craft's pin gaps are 20000 µm, far outside the 2000 rule, so
    /// no golden row ever encoded the old matrix cell). The
    /// different-component same-net widening witness lives on the
    /// P4 craft ([`same_component_pin_exemptions`]).
    #[test]
    fn via_trace_cells_and_same_net_pin_quirk() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nx = net_list(&board, "NX")[0];
        let ny1 = net_list(&board, "NY")[0];
        let via = net_list(&board, "NV")
            .into_iter()
            .find(|&id| kind_of(&board, id) == Some(BoardItemType::Via))
            .expect("the NV craft carries vias");
        let nq = net_list(&board, "NQ");
        let p1 = id_at_corner(&manager, &mut board, &nq, 10000.0, 10000.0);
        let p2 = id_at_corner(&manager, &mut board, &nq, 30000.0, 10000.0);
        assert!(
            is_obstacle(&mut board, via, nx),
            "via: !sharesNet fires first"
        );
        assert!(is_obstacle(&mut board, nx, via), "trace: !sharesNet");
        assert!(is_obstacle(&mut board, nx, ny1), "foreign traces");
        assert!(is_obstacle(&mut board, via, p1), "via vs foreign pin");
        assert!(
            !is_obstacle(&mut board, p1, p2),
            "SAME-net pins are exempt (upstream 14b28b6ff #925b; #931 cluster G dropped the component guard)"
        );
    }

    /// #925b (upstream `14b28b6ff`) + the #931 cluster-G widening:
    /// the Pin-Pin exemptions on the P4 craft — every close pair
    /// sits 3000 µm apart under the 2000 µm rule, so the MATRIX
    /// verdicts and the walk rows agree pair-for-pair. The NS2 pair
    /// (same net, DIFFERENT components) WITNESSES the #931 guard
    /// drop — a guard-restoring mutant re-blocks it and fails the
    /// 0-row assert; the knob-off control below kills a knob-blind
    /// port (every exemption flips back together). The contrasts
    /// each kill their own mutant: a netless-blind port exempts the
    /// CMPE pair (netted vs netless, same base), a name-blind port
    /// exempts the CMPD pair (netless, different bases), and a
    /// deletion mutant of either exemption fails its own 0-row
    /// assert.
    #[test]
    fn same_component_pin_exemptions() {
        use epic_board::components::pin_name;
        use epic_board::items::ItemData;

        let (mut manager, mut board) = parse(DSN_P4);
        let pins = ids_of_kind(&board, BoardItemType::Pin);
        let at = |x: f64, board: &mut Board| id_at_corner(&manager, board, &pins, x, 10000.0);
        let (ps1, ps2) = (at(10000.0, &mut board), at(13000.0, &mut board));
        let (pb_b, pb_b2) = (at(40000.0, &mut board), at(43000.0, &mut board));
        let (px1, px2) = (at(70000.0, &mut board), at(73000.0, &mut board));
        let (py1, pz1) = (at(100000.0, &mut board), at(103000.0, &mut board));
        let (pw1, pw2) = (at(130000.0, &mut board), at(133000.0, &mut board));

        // The craft parses as intended: sub-pads netless where the
        // netlist omits them, netted where it names them.
        assert!(
            nets_of(&board, px1).is_empty() && nets_of(&board, px2).is_empty(),
            "PX@1/PX@2 are netless"
        );
        assert!(nets_of(&board, pw2).is_empty(), "PW@2 is netless");
        assert_eq!(nets_of(&board, pw1).len(), 1, "PW@1 is on NS3");
        assert_eq!(nets_of(&board, ps1), nets_of(&board, ps2), "PS1/PS2 on NS");

        // The name-resolution chain (isSameLogicalPad's lookup) through
        // the REAL parse: entry → component → package → pin name.
        let entry = board.get(px1).expect("px1 exists");
        let ItemData::Pin {
            pin_index: px1_index,
            ..
        } = entry.data
        else {
            panic!("px1 is a pin");
        };
        let component_id = u32::try_from(entry.component_id).expect("component id");
        assert_eq!(
            pin_name(board.components(), board.library(), component_id, px1_index),
            Some("PX@1"),
            "the package pin name resolves through the parse"
        );

        // Exemption 2: same component, same net — exempt both ways.
        assert!(!is_obstacle(&mut board, ps1, ps2));
        assert!(!is_obstacle(&mut board, ps2, ps1), "symmetric");
        // #931 cluster G: same net ACROSS components — exempt (the
        // guard drop; stitching vias, internally connected pins).
        assert!(!is_obstacle(&mut board, pb_b, pb_b2));
        assert!(!is_obstacle(&mut board, pb_b2, pb_b), "symmetric");
        // Exemption 1: netless sub-pads of one logical pad — exempt.
        assert!(!is_obstacle(&mut board, px1, px2));
        assert!(!is_obstacle(&mut board, px2, px1), "symmetric");
        // Contrast: netless, different bases — still obstacles.
        assert!(is_obstacle(&mut board, py1, pz1));
        // Contrast: netted vs netless, same base — still obstacles.
        assert!(is_obstacle(&mut board, pw1, pw2));
        assert!(is_obstacle(&mut board, pw2, pw1));

        // Helper-level faces (the shared guards).
        assert!(same_component_pins(&board, ps1, ps2));
        assert!(!same_component_pins(&board, pb_b, pb_b2));
        assert!(both_netless(&board, px1, px2));
        assert!(!both_netless(&board, pw1, pw2));
        assert!(same_logical_pad(&board, px1, px2));
        assert!(!same_logical_pad(&board, py1, pz1));
        // PS1/PS2: same component + same net, but the logical names
        // DIFFER — insertion 2 exempts without ever asking.
        assert!(!same_logical_pad(&board, ps1, ps2));

        // The walk agrees row-for-row: exactly the two contrasting
        // pairs (shortfall 1000 µm each, far above the 1.0 tolerance)
        // — the NS2 pair dropped with the #931 guard.
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 2, "exactly the two contrasts: {rows:?}");
        assert_eq!(rows_contain(&rows, pb_b, pb_b2), 0, "NS2 pair (#931)");
        assert_eq!(rows_contain(&rows, py1, pz1), 1, "CMPD pair");
        assert_eq!(rows_contain(&rows, pw1, pw2), 1, "CMPE pair");
        assert_eq!(rows_contain(&rows, ps1, ps2), 0, "exemption 2 drops NS");
        assert_eq!(rows_contain(&rows, px1, px2), 0, "exemption 1 drops CMPC");

        // The knob-off CONTROL: the frozen pre-#925b face (the corpus
        // law's read) — every exemption verdict flips back, and the
        // walk gains exactly the two exempt pairs (5 rows). A
        // knob-blind port cannot produce this split.
        board.rules_mut().same_component_pin_exemptions = false;
        assert!(
            is_obstacle(&mut board, ps1, ps2),
            "frozen face: NS pair is an obstacle"
        );
        assert!(
            is_obstacle(&mut board, px1, px2),
            "frozen face: CMPC pair is an obstacle"
        );
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(
            total, 5,
            "the frozen face records both exempt pairs too: {rows:?}"
        );
        assert_eq!(rows_contain(&rows, ps1, ps2), 1);
        assert_eq!(rows_contain(&rows, px1, px2), 1);
    }

    /// The board walk on DSN_MAIN: exactly THREE deduped rows — the
    /// NX/NY trace pair (emitted by BOTH walks; a dedup-removed
    /// mutant yields it twice), the NA/NB close pair (600 < 2000),
    /// and the outline against the EDGE STRADDLING pin P10.
    ///
    /// Outline-exemption contrast, both arms (review round 2):
    /// P11's pad is fully interior — it never queries the outline
    /// (the outline precalc tree holds only the 100-unit border-band
    /// tiles, which an interior pad's bbox never reaches), so its
    /// missing row is BAND GEOMETRY, not the exemption. P13 overlaps
    /// the band while ALL its corners stay inside the outline → the
    /// exemption arm actually fires and clears; an exemption-disabled
    /// mutant turns P13 into a violation row and fails the pin.
    #[test]
    fn walk_rows_dedup_and_outline_exemption() {
        let (mut manager, mut board) = parse(DSN_MAIN);
        let outline = ids_of_kind(&board, BoardItemType::BoardOutline)[0];
        let nx = net_list(&board, "NX")[0];
        let ny = net_list(&board, "NY")[0];
        let np = net_list(&board, "NP");
        let na = net_list(&board, "NA")[0];
        let nb = net_list(&board, "NB")[0];
        let np2 = net_list(&board, "NP2");
        let p10 = id_at_corner(&manager, &mut board, &np, 250000.0, 70000.0);
        let p11 = id_at_corner(&manager, &mut board, &np, 240000.0, 70000.0);
        let p13 = id_at_corner(&manager, &mut board, &np2, 248999.0, 20000.0);
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 3, "exactly the three crafted violations: {rows:?}");
        assert_eq!(
            rows_contain(&rows, nx, ny),
            1,
            "the A-B/B-A dedup keeps ONE row for the pair"
        );
        assert_eq!(
            rows_contain(&rows, na, nb),
            1,
            "center distance 600 < default 2000: the close pair violates"
        );
        assert_eq!(
            rows_contain(&rows, p10, outline),
            1,
            "straddling pad violates"
        );
        assert_eq!(
            rows_contain(&rows, p11, outline),
            0,
            "interior pad: BAND GEOMETRY — its bbox never reaches the outline precalc tree"
        );
        assert_eq!(
            rows_contain(&rows, p13, outline),
            0,
            "band-overlapping INSIDE pad: the exemption arm clears it (the contrast to P11)"
        );
        assert_eq!(
            rows_contain(&rows, p10, p11),
            0,
            "same-net pins never violate"
        );
    }

    /// THE asymmetric-matrix pin (T54, review round 2). The gate
    /// (Item.java:452-453) reads `getValue(current.clClass,
    /// this.clClass, layer)` — the RECEIVER's class is the FIRST
    /// argument. No corpus fixture carries an asymmetric cell, so the
    /// swapped-args mutant (`get_value(this, current)`) survives
    /// every golden compare; this pin builds the asymmetry on
    /// DSN_MAIN and pins the verdict through the FULL walk.
    ///
    /// Setup: NA/NB, parallel F.Cu traces at center distance 600
    /// (half width 125 → bbox gap 350); classes NA → 1, NB → 2;
    /// matrix (both layers): V(0, *) = 2000 (default world kept),
    /// V(1,1) = V(1,2) = V(2,2) = 200, V(2,1) = 20000.
    ///
    /// Why exactly ONE gate execution decides the verdict (a pair
    /// found by both walks would take the max over the two cells and
    /// be orientation-blind):
    ///  * NA's walk: `max_value(1)` = max_i V(i, 1) = 20000 enlarges
    ///    the query (NB IS a leaf candidate), but the exact
    ///    per-candidate filter reads V(1, 2) = 200 (+16 safety
    ///    margin) < gap 350 → NB is dropped BEFORE the gate.
    ///  * NB's walk: `max_value(2)` = max_i V(i, 2) = 2000 → NA is a
    ///    candidate; the filter reads V(2, 1) = 20000 → kept; the
    ///    gate reads get_value(current_class = 1, this_class = 2) =
    ///    V(1, 2) = 200 → 600 > 250 + 200 → NO violation.
    ///
    /// The swapped-args mutant makes that single surviving gate read
    /// V(2, 1) = 20000 → violation → the (NA, NB) row appears and
    /// both asserts below fail (mutation-verified, review round 2).
    #[test]
    fn asymmetric_matrix_gate_reads_current_class_as_row() {
        let (mut manager, mut board) = parse(DSN_MAIN);
        let na = net_list(&board, "NA")[0];
        let nb = net_list(&board, "NB")[0];
        board.set_item_clearance_class(na, 1);
        board.set_item_clearance_class(nb, 2);
        // The parsed craft matrix carries ONE class and an
        // out-of-range set_value is a silent no-op (T54), so the pin
        // REPLACES the matrix with a 3-class one. With the
        // compensation flag OFF the leaves store raw shapes and the
        // walk reads classes and cells live — no tree rebuild needed.
        let mut matrix = epic_board::rules_surf::ClearanceMatrix::new(
            3,
            board.rules().clearance.layer_count(),
            vec![
                "class_1".to_string(),
                "class_2".to_string(),
                "class_3".to_string(),
            ],
        );
        for (i, j, value) in [
            (0, 0, 2000),
            (0, 1, 2000),
            (0, 2, 2000),
            (1, 0, 2000),
            (1, 1, 200),
            (1, 2, 200),
            (2, 0, 2000),
            (2, 1, 20000),
            (2, 2, 200),
        ] {
            matrix.set_value_all_layers(i, j, value);
        }
        board.rules_mut().clearance = matrix;
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(
            total, 2,
            "the default-matrix NA/NB row is gone; NX/NY + P10/outline remain: {rows:?}"
        );
        assert_eq!(
            rows_contain(&rows, na, nb),
            0,
            "the single surviving gate read V(1, 2) = 200: receiver's class is the row argument"
        );
    }

    /// The tie-pin exemption, CONTRAST PAIR (cerebrum mode 7): on
    /// DSN_TIE the overlapping foreign-net traces share tie pin P5 →
    /// ZERO rows (an exemption-removed mutant fails here); on
    /// DSN_TIE_CONTRAST no trace corner touches the other and no tie
    /// pin bridges them → EXACTLY ONE row (an always-clear mutant
    /// fails here).
    #[test]
    fn tie_pin_exemption_contrast_pair() {
        let (mut manager, mut board) = parse(DSN_TIE);
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!((total, rows.len()), (0, 0), "the exemption clears the pair");

        let (mut manager, mut board) = parse(DSN_TIE_CONTRAST);
        let tw1 = net_list(&board, "TA");
        let tw2 = net_list(&board, "TB");
        let tw1 = tw1
            .into_iter()
            .find(|&id| kind_of(&board, id) == Some(BoardItemType::Trace))
            .expect("TA's trace");
        let tw2 = tw2
            .into_iter()
            .find(|&id| kind_of(&board, id) == Some(BoardItemType::Trace))
            .expect("TB's trace");
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 1, "the flag stands without the tie pin: {rows:?}");
        assert_eq!(rows_contain(&rows, tw1, tw2), 1);
        assert_eq!(rows_contain(&rows, tw1, tw1), 0);
    }

    /// The #925a comparison world — unit um, resolution 5 (1 DBU =
    /// 0.2 µm). CRAFT LAW (probe-verified): the parser scales every
    /// file value ×resolution into DBU (`resolution.rs` t47 pin:
    /// "um 10: DSN 2000 -> board 20000"), so a shortfall's µm depth
    /// always equals its value in the DECLARED unit — integer file
    /// gaps give integer-µm shortfalls, which cannot straddle the
    /// 1.0 default. The ODD-clearance lever breaks the quantization:
    /// clearance 201 file → 1005 DBU → `set_value` rounds ODD UP to
    /// 1006 (T54), leaving a 1-DBU = 0.2 µm reachable shortfall.
    /// Two parallel pairs (octagon flats facing: gap = center dy −
    /// 2×half width, exact): SA/SB gap 1005 DBU → shortfall 1 DBU =
    /// 0.2 µm (rounding noise); SC/SD gap 975 → shortfall 31 DBU =
    /// 6.2 µm (a real violation). Width 50 file (half 125 DBU).
    const DSN_TOL_UM5: &str = "\
(pcb p3-tol-um.dsn\n\
  (parser\n\
    (string_quote \")\n\
    (space_in_quoted_tokens on)\n\
  )\n\
  (resolution um 5)\n\
  (unit um)\n\
  (structure\n\
    (layer F.Cu (type signal))\n\
    (layer B.Cu (type signal))\n\
    (boundary (rect pcb 0 0 250000 400000))\n\
    (rule (width 50) (clearance 201))\n\
  )\n\
  (placement\n\
    (component CMP1\n\
      (place CMP1 20000 20000 front 0)\n\
    )\n\
  )\n\
  (library\n\
    (padstack PAD_C600\n\
      (shape (circle F.Cu 600 0 0))\n\
    )\n\
    (image CMP1\n\
      (pin PAD_C600 P1 0 0)\n\
    )\n\
  )\n\
  (network\n\
    (net PU (pins CMP1-P1))\n\
    (net SA)\n\
    (net SB)\n\
    (net SC)\n\
    (net SD)\n\
  )\n\
  (wiring\n\
    (wire (path F.Cu 50  50000 100000  150000 100000) (net SA))\n\
    (wire (path F.Cu 50  50000 100251  150000 100251) (net SB))\n\
    (wire (path F.Cu 50  50000 300000  150000 300000) (net SC))\n\
    (wire (path F.Cu 50  50000 300245  150000 300245) (net SD))\n\
  )\n\
)\n";

    /// The #925a FACTOR world — unit mil, resolution 5 (factor
    /// 25.4/5 = 5.08 µm/DBU). Same odd-clearance lever: clearance 81
    /// file → 405 DBU → rounds to 406; the pair's gap 405 leaves a
    /// 1-DBU shortfall = 5.08 µm. Width 10 file (half 25 DBU).
    const DSN_TOL_MIL5: &str = "\
(pcb p3-tol-mil.dsn\n\
  (parser\n\
    (string_quote \")\n\
    (space_in_quoted_tokens on)\n\
  )\n\
  (resolution mil 5)\n\
  (unit mil)\n\
  (structure\n\
    (layer F.Cu (type signal))\n\
    (layer B.Cu (type signal))\n\
    (boundary (rect pcb 0 0 250000 140000))\n\
    (rule (width 10) (clearance 81))\n\
  )\n\
  (placement\n\
    (component CMP1\n\
      (place CMP1 20000 20000 front 0)\n\
    )\n\
  )\n\
  (library\n\
    (padstack PAD_C600\n\
      (shape (circle F.Cu 600 0 0))\n\
    )\n\
    (image CMP1\n\
      (pin PAD_C600 P1 0 0)\n\
    )\n\
  )\n\
  (network\n\
    (net PU (pins CMP1-P1))\n\
    (net TA)\n\
    (net TB)\n\
  )\n\
  (wiring\n\
    (wire (path F.Cu 10  50000 100000  150000 100000) (net TA))\n\
    (wire (path F.Cu 10  50000 100091  150000 100091) (net TB))\n\
  )\n\
)\n";

    /// #925a (upstream `14b28b6ff`): the µm shortfall-tolerance gate.
    /// Two worlds, margins ≥ 0.3 µm everywhere (far beyond the
    /// bisection face's ≤0.003 µm error at these bracket sizes):
    ///
    /// * COMPARISON (um-5 world): the 0.2 µm pair drops at the 1.0
    ///   default, the 6.2 µm pair records. The tolerance-0.0 control
    ///   reproduces the pre-#925a face exactly (BOTH record);
    ///   tolerance 0.5 keeps the split (a raw-DBU compare or a
    ///   factor-blind port records the 1-DBU pair — both mutants
    ///   die); 7.0 drops both; negative/NaN CLAMP to 0.0 in the
    ///   measure loop (the settings face warns+ignores them — this
    ///   is the defense in depth).
    /// * FACTOR (mil-5 world): the 1-DBU shortfall is 5.08 µm —
    ///   RECORDED at the default. A raw-DBU compare (1 ≤ 1.0 drops
    ///   it), a resolution-blind factor (0.2 µm drops it), and a
    ///   unit-blind factor (um assumed: 0.2 µm drops it) all die on
    ///   this one row.
    #[test]
    fn clearance_tolerance_um_gate_drops_rounding_noise() {
        // World 1 (um-5): the parsed board carries the from_ir seed.
        let (mut manager, mut board) = parse(DSN_TOL_UM5);
        assert_eq!(
            board.rules().clearance_tolerance_um,
            1.0,
            "the from_ir seed"
        );
        // The odd-clearance bump: matrix 1006, not 1005 (T54 round-up).
        assert_eq!(board.rules().clearance.get_value(1, 1, 0), 1006);
        let sa = net_list(&board, "SA")[0];
        let sb = net_list(&board, "SB")[0];
        let sc = net_list(&board, "SC")[0];
        let sd = net_list(&board, "SD")[0];

        // Default 1.0: only the 6.2 µm pair records.
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 1, "only the 6.2um pair: {rows:?}");
        assert_eq!(rows_contain(&rows, sa, sb), 0, "0.2um shortfall dropped");
        assert_eq!(rows_contain(&rows, sc, sd), 1, "6.2um shortfall recorded");

        // Tolerance 0.0 — the pre-#925a face exactly.
        board.rules_mut().clearance_tolerance_um = 0.0;
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 2, "both pairs at tolerance 0: {rows:?}");
        assert_eq!(rows_contain(&rows, sa, sb), 1);
        assert_eq!(rows_contain(&rows, sc, sd), 1);

        // 0.5 keeps the split — the mutant-killing control (see docs).
        board.rules_mut().clearance_tolerance_um = 0.5;
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 1, "0.2um still dropped at 0.5: {rows:?}");
        assert_eq!(rows_contain(&rows, sa, sb), 0);
        assert_eq!(rows_contain(&rows, sc, sd), 1, "6.2um still records");

        // 7.0 drops both (6.2 ≤ 7.0 — the STRICT > boundary family).
        board.rules_mut().clearance_tolerance_um = 7.0;
        let (total, _) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 0, "7.0um tolerance drops both");

        // The measure-loop clamp: negative and NaN read as 0.0.
        for bad in [-3.0, f64::NAN] {
            board.rules_mut().clearance_tolerance_um = bad;
            let (total, _) = all_clearance_violations(&mut manager, &mut board);
            assert_eq!(total, 2, "{bad} clamps to 0.0 in the measure loop");
        }

        // World 2 (mil-5): 1 DBU = 5.08 µm — the factor pin.
        let (mut manager, mut board) = parse(DSN_TOL_MIL5);
        assert_eq!(board.rules().clearance.get_value(1, 1, 0), 406);
        let ta = net_list(&board, "TA")[0];
        let tb = net_list(&board, "TB")[0];
        let (total, rows) = all_clearance_violations(&mut manager, &mut board);
        assert_eq!(total, 1, "1-DBU shortfall = 5.08um > 1.0: {rows:?}");
        assert_eq!(rows_contain(&rows, ta, tb), 1);
    }
}
