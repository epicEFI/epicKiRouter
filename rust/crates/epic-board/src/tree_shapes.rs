//! The per-item tree shapes and the clearance arithmetic behind them
//! (M2 Tasks 6-7): the T54 compensation formula, the T55 drill-hole
//! inflation, the three-variant drill dispatch, and the Task 7
//! per-kind construction for traces, obstacle/conduction areas, board
//! outlines and component outlines.
//!
//! Java anchors: `board/searchtree/ShapeSearchTree.java` —
//! `clearanceCompensationValue` `:104-114` (T54),
//! `calculateTreeShapes(DrillItem)` `:871-906` (the BASE dispatch —
//! the angle restriction is read at CALL time),
//! `drillHoleObstacle` `:1012-1028` and `drillHoleClearanceDelta`
//! `:1035-1074` (T55), `DRILL_HOLE_CLEARANCE_MARGIN = 10` `:52` —
//! the Task 7 family `calculateTreeShapes(ObstacleArea)` `:908-938`
//! (convex division + `enlarge` + `divideIntoSections` under the
//! `maxTreeShapeWidth` RESOLUTION CLAMP `:916-920`),
//! `calculateTreeShapes(BoardOutline)` `:940-990` (line branch +
//! keepout-area branch), `calculateTreeShapes(PolylineTrace)`
//! `:992-1004` (`offsetShape` at `halfWidth + compensation`) —
//! plus the subclass overrides
//! `ShapeSearchTree45Degree.java:488-519` (`isIntBox` swap + `offset`)
//! and `ShapeSearchTree90Degree.java:434-462` (box + `offset`), and
//! the 45°/90° OBSTACLE and OUTLINE post-processing
//! (`ShapeSearchTree45Degree.java:521-541`,
//! `ShapeSearchTree90Degree.java:464-484`) plus the 90° trace
//! `offsetBox` swap (`:486-490`).
//!
//! ## Why this lives in epic-board, not epic-index (D17)
//!
//! The formula reads [`BoardRules`] (the clearance matrix and the hole
//! clearance) and the item surface — both epic-board concepts. The
//! GENERIC index crate receives the finished shapes as opaque
//! `Option<TileShape>` lists; it never sees a rule.
//!
//! ## Ground truth (jar spikes)
//!
//! `rust/harness/oracle/TreeShapesSpike.java` drove the frozen jar
//! over `fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn`
//! (capture `/tmp/epic-t6-treeshapes.out`), `PCBench/1Bitsy_1bitsy`
//! (capture `/tmp/epic-t6-treeshapes-1bitsy.out`, 4 layers, classes
//! 0/1/2 with `v1.1 = v2.1 = 1490`, `cc(1) = 745`, and — like every
//! capture fixture — an ALL-ZERO row 0: `getValue(0, j, l) = 0`), and a
//! SYNTHETIC hole padstack (`spike_hole_600:300`: copper circles only
//! on layers 0 and 3 of the 4-layer board) inserted through the real
//! Java board API, which drives the null-shape and drill-hole-obstacle
//! branches no corpus fixture reaches (DSN-exported padstacks define
//! every layer). The tests quote those lines.
//!
//! Task 7 added the per-kind dumps (`/tmp/epic-t7-shapes.out`): trace,
//! obstacle, conduction, component-outline and board-outline items
//! across the five tree configurations on Issue575 +
//! Issue054-tairakb, the SECTIONING fixture (a keepout larger than
//! the 50000 threshold) and the RESOLUTION-CLAMP fixture
//! (`(resolution um 1)` → threshold 12700) — see the tests.

use epic_geometry::circle::Circle;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use epic_index::{SearchTree, SearchTreeVariant};

use crate::board::Board;
use crate::components::BoardPadstack;
use crate::id::ItemId;
use crate::items::{Area, BoardShape, ItemData};
use crate::rules_surf::{AngleRestriction, BoardRules, DEFAULT_CLEARANCE_CLASS};

/// Java `ShapeSearchTree.DRILL_HOLE_CLEARANCE_MARGIN` (`:52`).
pub const DRILL_HOLE_CLEARANCE_MARGIN: i32 = 10;

/// Java `ShapeSearchTree.clearanceCompensationValue(clearanceClassIndex,
/// layer)` (`:104-114`) — **T54**:
///
/// ```text
/// if item_class <= 0 { 0 } else {
///     max(0, getValue(item_class, tree_class, layer, false)
///            - matrix.clearance_compensation_value(tree_class, layer))
/// }
/// ```
///
/// The matrix read is ASYMMETRIC-position: the ITEM class is the row
/// argument `i`, the TREE's compensated class the column `j` (storage
/// `values[layer][j][i]`, pinned in T3) — for a symmetric matrix the
/// order is unobservable, so the pins below use a hand-built
/// ASYMMETRIC matrix where swapping them changes the answer.
///
/// The `item_class <= 0` guard is unobservable through EVERY capture
/// fixture (row 0 is all zero on both Issue575 and 1Bitsy — the
/// negative clamp subsumes it); it is pinned on a hand-built matrix
/// where the unguarded formula is POSITIVE (`getValue(0, 1, l) = 2000`
/// against `cc(1) = 745`: unguarded 1255, guarded 0).
#[must_use]
pub fn clearance_compensation_value(
    rules: &BoardRules,
    item_class: i32,
    tree_class: i32,
    layer: i32,
) -> i32 {
    if item_class <= 0 {
        return 0;
    }
    let raw = rules
        .clearance
        .get_value_opt(item_class, tree_class, layer, false)
        - rules
            .clearance
            .clearance_compensation_value(tree_class, layer);
    raw.max(0)
}

/// Java `Trace.getCompensatedHalfWidth(searchTree)` (`Trace.java:84-89`):
/// the trace's half width plus the tree's clearance compensation value
/// for the trace's class and layer — equal to the plain half width when
/// the tree does not compensate (`compensatedClearanceClassNo == 0`
/// makes [`clearance_compensation_value`] answer 0 through the guard).
/// The shove substrate reads it for the single-step offset shapes, the
/// end-corner contact analysis and the dog-ear cutlines.
///
/// The `unwrap_or` defaults are dead at every call site (the trace id
/// is always a live board item); they keep the function total where
/// Java would NPE on a null board read.
#[must_use]
pub fn trace_compensated_half_width(
    board: &Board,
    search_tree: &SearchTree,
    trace_id: ItemId,
) -> i32 {
    let half_width = board.trace_half_width(trace_id).unwrap_or(0);
    let class = board.item_clearance_class(trace_id).unwrap_or(0);
    let layer = board.trace_layer(trace_id).unwrap_or(0);
    half_width
        + clearance_compensation_value(
            board.rules(),
            class,
            search_tree.compensated_clearance_class,
            layer,
        )
}

/// Java `ShapeSearchTree.drillHoleObstacle(drillItem)` (`:1012-1028`):
/// the synthesized obstacle for a copper-less layer of a drilled item
/// — `Circle(rounded center, ceil(drillRadius))`. `None` when the
/// hole-clearance rule is off (`holeClearance <= 0`) or no drill
/// radius is known (`drillRadius <= 0`); the caller's null-padstack
/// guard (`drillItem.getPadstack() == null`, `:1015-1017`) is the
/// `Option` the caller resolves before calling.
///
/// The radius is the CEIL of the double drill radius cast to int
/// (`(int) Math.ceil(drillRadius)`, `:1027`) — captured:
/// `spike_hole_600:300` (drillRadius 1500.0) -> the `r=1500.0` circles
/// of the 1Bitsy hole layers.
#[must_use]
pub fn drill_hole_obstacle(
    hole_clearance: i32,
    padstack: &BoardPadstack,
    center: &Point,
) -> Option<BoardShape> {
    if hole_clearance <= 0 {
        return None;
    }
    let drill_radius = padstack.drill_radius();
    if drill_radius <= 0.0 {
        return None;
    }
    // `if (!(center instanceof IntPoint)) center = center.toFloat().round();`
    // (:1023-1026) — a RationalPoint center rounds; pin/via centers on
    // a parsed board are integral already.
    let center = match center {
        Point::Int(point) => *point,
        // FloatPoint::round returns the rounded IntPoint directly.
        Point::Rational(_) => center.to_float().round(),
    };
    Some(BoardShape::Circle(Circle::new(
        center,
        drill_radius.ceil() as i32,
    )))
}

/// Java `ShapeSearchTree.drillHoleClearanceDelta(drillItem, shape,
/// layer)` (`:1035-1074`) — **T55**: the extra inflation keeping other
/// nets' copper `holeClearance` away from the DRILL HOLE (not just the
/// copper pad):
///
/// ```text
/// max(0, ceil(drillRadius + holeClearance + 10
///             - copperRadius - copperClearance))
/// ```
///
/// with
/// * `copperClearance = getValue(item_class, clearance_class, layer,
///   false)` where `clearance_class = tree_class > 0 ? tree_class :
///   defaultClearanceClass() (= 1)` — the TREE's class on the J side
///   of the asymmetric read, not the item's,
/// * `copperRadius = drillRadius` for a `holeOnly` padstack, else the
///   current shape's `borderDistance(item center)`; a `<= 0` result
///   re-reads the RAW padstack shape's `borderDistance(ZERO)`
///   (`:1053-1055`), falling back to `drillRadius` when that slot is
///   null.
///
/// The int additions promote to double before `ceil` (Java `:1068` —
/// the whole sum is a double expression); `(int)` casts the ceil.
#[must_use]
pub fn drill_hole_clearance_delta(
    rules: &BoardRules,
    padstack: &BoardPadstack,
    center: &Point,
    shape: Option<&BoardShape>,
    item_class: i32,
    tree_class: i32,
    layer: i32,
) -> i32 {
    let hole_clearance = rules.hole_clearance;
    if hole_clearance <= 0 {
        return 0;
    }
    let Some(shape) = shape else {
        // Java's `shape == null` guard (:1040).
        return 0;
    };
    let drill_radius = padstack.drill_radius();
    if drill_radius <= 0.0 {
        return 0;
    }
    let copper_radius = if padstack.hole_only {
        drill_radius
    } else {
        let distance = shape.border_distance(&center.to_float());
        if distance > 0.0 {
            distance
        } else {
            match padstack.get_shape(usize::try_from(layer).ok().unwrap_or(usize::MAX)) {
                Some(pad_shape) => pad_shape.border_distance(&FloatPoint::ZERO),
                None => drill_radius,
            }
        }
    };
    let clearance_class = if tree_class > 0 {
        tree_class
    } else {
        DEFAULT_CLEARANCE_CLASS
    };
    let copper_clearance = rules
        .clearance
        .get_value_opt(item_class, clearance_class, layer, false);
    let total = drill_radius + f64::from(hole_clearance) + f64::from(DRILL_HOLE_CLEARANCE_MARGIN)
        - copper_radius
        - f64::from(copper_clearance);
    (total.ceil() as i32).max(0)
}

/// Java `ShapeSearchTree.calculateTreeShapes(DrillItem)` `:871-906`
/// and its two subclass overrides — the tree shapes of a pin or via
/// item, one `Option` per padstack-shape index (`None` = Java's null
/// entry: a copper-less layer whose drill-hole obstacle is off).
///
/// The dispatch per [`SearchTreeVariant`]:
///
/// * **Generic** (the base tree, `:885-893`): the angle restriction is
///   read at CALL time — `NINETY_DEGREE` -> `boundingBox`, else
///   `FORTYFIVE_DEGREE` -> `boundingOctagon`, else `boundingTile` —
///   and the hull is grown with **`enlarge(offset_width)`** (an
///   `IntBox` hull becomes `toIntOctagon().offset(w)` — an octagon
///   with diagonals at `w·√2`; a genuine octagon's `enlarge` moves the
///   diagonals by `w·√2`, the same as `offset` there — which is why
///   CIRCLE pads agree between the base and 45° trees while BOX pads
///   do not: captured `...1284433 1865267...` base vs
///   `...1171180 1978520...` 45° at width 193335).
/// * **FortyfiveDegree** (`ShapeSearchTree45Degree.java:496-517`):
///   `boundingOctagon`, the `isIntBox` swap back to `boundingBox`
///   ("to avoid small corner cutoffs", `:501-506`), **`offset(w)`**
///   (sides by `w` — a box stays a box), then a final
///   `boundingOctagon()` (cut corners at `w`).
/// * **NinetyDegree** (`ShapeSearchTree90Degree.java:436-459`):
///   `boundingBox` + **`offset(w)`** — an `IntBox` stays an `IntBox`.
///
/// `offset_width` is the T54 compensation plus the T55 delta
/// (`:893-896` / the subclass equivalents) — both keyed by
/// `drillItem.shapeLayer(i)`, the ITEM-span-clamped layer
/// ([`Board::drill_shape_layer`]).
///
/// Returns an empty list for a non-drill id (Java's
/// `DrillItem`-typed parameter cannot express that; the id simply
/// names no drill item on the board) or a drill id whose CENTER does
/// not resolve (an unplaced pin — a Java `Pin` always carries its
/// component, so `getCenter()` never fails there; the port refuses to
/// fabricate origin geometry, unlike the self-neutralizing
/// [`EMPTY_PADSTACK`] below).
#[must_use]
pub fn drill_tree_shapes(
    board: &mut Board,
    variant: SearchTreeVariant,
    compensated_class: i32,
    id: ItemId,
) -> Vec<Option<TileShape>> {
    let Some(count) = board.drill_tile_shape_count(id) else {
        return Vec::new();
    };
    // Resolve the center ONCE up front (T6 quality review MINOR-2): a
    // drill id whose center does not resolve yields NO shapes rather
    // than silently offsetting at the origin.
    let Some(center) = board.drill_center(id) else {
        return Vec::new();
    };
    let count = count.max(0) as usize;
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let index = i32::try_from(index).unwrap_or(i32::MAX);
        let mut current = board.drill_shape(id, index);
        if current.is_none() {
            // The synthesized drill-hole obstacle (:878-880) — for a
            // copper-less layer when the hole-clearance rule is on.
            if let Some(padstack) = board.drill_padstack(id) {
                current = drill_hole_obstacle(board.rules().hole_clearance, padstack, &center);
            }
        }
        let Some(current) = current else {
            // Java result[i] = null (:881-882) — a null SLOT, not a
            // skipped index.
            result.push(None);
            continue;
        };
        // Structurally guaranteed: shapes resolved for this index means
        // the drill span and the arena entry exist (T7 quality review
        // NIT-3) — expect, never a silent 0.
        let layer = board
            .drill_shape_layer(id, index)
            .expect("a drill item with shapes has a layer span");
        let item_class = board
            .item_clearance_class(id)
            .expect("an arena entry always carries its clearance class");
        let offset_width = {
            let rules = board.rules();
            clearance_compensation_value(rules, item_class, compensated_class, layer)
                + drill_hole_clearance_delta(
                    rules,
                    board.drill_padstack(id).unwrap_or(&EMPTY_PADSTACK),
                    &center,
                    Some(&current),
                    item_class,
                    compensated_class,
                    layer,
                )
        };
        let width = f64::from(offset_width);
        let tile = match variant {
            SearchTreeVariant::Generic => {
                // The base dispatch: the restriction at CALL time
                // (:885-893).
                let hull = match board.rules().trace_angle_restriction {
                    AngleRestriction::NinetyDegree => Some(box_tile(current.bounding_box())),
                    AngleRestriction::FortyfiveDegree => current.bounding_octagon(),
                    AngleRestriction::None => Some(current.bounding_tile()),
                };
                // Java warns and stores null for a null hull (:897-901)
                // — reachable only for an unbounded Simplex shape,
                // which a padstack shape never is.
                hull.map(|hull| hull.enlarge(width))
            }
            SearchTreeVariant::FortyfiveDegree => {
                // ShapeSearchTree45Degree.java:496-517.
                let mut hull = current.bounding_octagon();
                if hull.as_ref().is_some_and(TileShape::is_int_box) {
                    // swap back to the box "to avoid small corner
                    // cutoffs" (:501-506)
                    hull = Some(box_tile(current.bounding_box()));
                }
                hull.and_then(|hull| {
                    hull.offset(width)
                        .bounding_octagon()
                        .map(RegularTileShape::IntOctagon)
                        .map(TileShape::RegularTileShape)
                })
            }
            SearchTreeVariant::NinetyDegree => {
                // ShapeSearchTree90Degree.java:443-459: box + offset;
                // an IntBox stays an IntBox (IntBox.offset).
                Some(box_tile(current.bounding_box()).offset(width))
            }
        };
        result.push(tile);
    }
    result
}

/// The per-item tree-shape entry point the tree manager broadcasts
/// (Java `ShapeTree.insert(Storable)` reaching
/// `item.treeShapeCount(tree)` and the per-kind `calculateTreeShapes`
/// — `Item.getPrecalculatedTreeShapes`'s lazy compute, `Item.java:228-238`).
///
/// The dispatch (Java's virtual `calculateTreeShapes`):
///
/// * pins and vias -> [`drill_tree_shapes`] (the `DrillItem` entry),
/// * traces -> [`trace_tree_shapes`] (`ShapeSearchTree.java:992-1004`),
/// * obstacle areas — plain, via and component keepouts alike — and
///   CONDUCTION areas -> [`obstacle_tree_shapes`] (`:908-938`; Java's
///   `ConductionArea` extends `ObstacleArea` and overrides NOTHING on
///   this path, and a parse-time conduction area's placement is the
///   identity so its stored area is `getArea()` verbatim),
/// * board outlines -> [`outline_tree_shapes`] (`:940-990`),
/// * COMPONENT outlines -> an EMPTY list (Java
///   `ComponentOutline.calculateTreeShapes` returns `new
///   TileShape[0]`, `ComponentOutline.java:135-137` — no tree entries,
///   Java's `shapeCount <= 0` early-out),
/// * a foreign id -> an empty list (no item, no shapes).
#[must_use]
pub fn item_tree_shapes(
    board: &mut Board,
    variant: SearchTreeVariant,
    compensated_class: i32,
    id: ItemId,
) -> Vec<Option<TileShape>> {
    match board.get(id).map(|entry| &entry.data) {
        Some(ItemData::Pin { .. }) | Some(ItemData::Via { .. }) => {
            drill_tree_shapes(board, variant, compensated_class, id)
        }
        Some(ItemData::Trace { .. }) => trace_tree_shapes(board, variant, compensated_class, id),
        Some(ItemData::ObstacleArea { .. }) | Some(ItemData::ConductionArea { .. }) => {
            obstacle_tree_shapes(board, variant, compensated_class, id)
        }
        Some(ItemData::BoardOutline { .. }) => {
            outline_tree_shapes(board, variant, compensated_class, id)
        }
        // ComponentOutline: Java returns new TileShape[0].
        Some(ItemData::ComponentOutline { .. }) | None => Vec::new(),
        // The unclassified fallback kind never reaches the tree.
        Some(ItemData::Other) => Vec::new(),
    }
}

/// Java `ShapeSearchTree.calculateTreeShapes(ObstacleArea)`
/// `:916-920` — the sectioning threshold of the obstacle path:
///
/// ```text
/// 50000, clamped by min(500 * getResolution(MIL), 50000)
///          but ONLY when a host CAD was recorded
/// ```
///
/// `getResolution(MIL)` converts the board resolution into mils
/// (`Unit.scale(resolution, MIL, unit)`), so the clamp BITES exactly
/// when `500 * resolution-in-mils < 50000`:
///
/// * the corpus `(resolution um 10)` boards: 500 × 25.4 = 127000 ->
///   the CAP wins, threshold 50000 (jar-verified,
///   `/tmp/epic-t7-probe.out`),
/// * a `(resolution um 1)` board: 500 × 25.4 = 12700 -> the CLAMP
///   wins, threshold 12700 — an 8× finer sectioning grid than the
///   naive constant,
/// * the DSN default `(resolution mil 100)`: 500 × 100 = 50000 —
///   both agree,
/// * a board with no host CAD (`hostCadExists()` false): the
///   constant 50000 regardless of resolution.
pub fn max_tree_shape_width(board: &Board) -> f64 {
    let mut width = 50_000.0;
    if board.communication().host_cad_exists() {
        width = (500.0 * board.communication().resolution_mil()).min(width);
    }
    width
}

/// The per-shape arm dispatch of `Area.splitToConvex()` over
/// epic-board's [`Area`] — Java's border impls:
/// `TileShape.splitToConvex` = `[this]` (convex by construction),
/// `PolygonShape.splitToConvex` = the seeded recursive split (null on
/// failure), `Circle.splitToConvex` = `[boundingTile]`, and a
/// holed area = Java `PolylineArea.splitToConvex` (`PolylineArea.java:150-181`):
/// border pieces, then every convex HOLE piece cut out of every
/// divide piece, keeping only dimension-2 results.
///
/// `None` mirrors Java's null (the polygon split failed) — the
/// caller's `new TileShape[0]` turns it into "no tree entries".
pub(crate) fn area_split_to_convex(area: &Area) -> Option<Vec<TileShape>> {
    let shape_split = |shape: &BoardShape| -> Option<Vec<TileShape>> {
        match shape {
            BoardShape::Tile(tile) => Some(tile.split_to_convex()),
            BoardShape::PolygonShape(polygon) => polygon.split_to_convex(),
            BoardShape::Circle(circle) => Some(circle.split_to_convex()),
        }
    };
    if area.holes.is_empty() {
        return shape_split(&area.border);
    }
    let mut pieces = shape_split(&area.border)?;
    for hole in &area.holes {
        let hole_dimension = match hole {
            BoardShape::Tile(tile) => tile.dimension(),
            BoardShape::PolygonShape(polygon) => polygon.dimension(),
            BoardShape::Circle(_) => 2,
        };
        if hole_dimension < 2 {
            // Java: FRLogger.warn("dimension 2 for hole expected") and
            // skips the hole (log-only, D12).
            continue;
        }
        for hole_piece in shape_split(hole)? {
            let mut next_pieces = Vec::new();
            for divide_piece in &pieces {
                for cut in divide_piece.cutout(&hole_piece) {
                    if cut.dimension() == 2 {
                        next_pieces.push(cut);
                    }
                }
            }
            pieces = next_pieces;
        }
    }
    Some(pieces)
}

/// The obstacle post-processing of the 45°/90° subclass overrides:
/// every shape through `boundingOctagon()` (`ShapeSearchTree45Degree
/// .java:521-530`) / `boundingBox()` (`ShapeSearchTree90Degree.java:
/// 464-484`) — applied to BOTH the obstacle and the outline results
/// (`result[i] != null` guard: a `None` slot stays `None`).
fn bounding_post_process(
    shapes: Vec<Option<TileShape>>,
    variant: SearchTreeVariant,
) -> Vec<Option<TileShape>> {
    match variant {
        SearchTreeVariant::Generic => shapes,
        SearchTreeVariant::FortyfiveDegree => shapes
            .into_iter()
            .map(|shape| {
                shape.and_then(|shape| {
                    shape.bounding_octagon().map(|octagon| {
                        TileShape::RegularTileShape(RegularTileShape::IntOctagon(octagon))
                    })
                })
            })
            .collect(),
        SearchTreeVariant::NinetyDegree => shapes
            .into_iter()
            .map(|shape| shape.map(|shape| box_tile(shape.bounding_box())))
            .collect(),
    }
}

/// Java `ShapeSearchTree.calculateTreeShapes(ObstacleArea)`
/// `:908-938` (+ the 45°/90° overrides): convex division, per-piece
/// `enlarge(offsetWidth)`, `divideIntoSections(maxTreeShapeWidth)`,
/// and the subclass bounding pass.
///
/// `offsetWidth` is [`clearance_compensation_value`] at the AREA's
/// single layer (`obstacleArea.getLayer()`); the section threshold is
/// [`max_tree_shape_width`] — the resolution clamp the corpus um-10
/// boards do NOT reach (127000 > 50000) but a um-1 board does.
fn obstacle_tree_shapes(
    board: &mut Board,
    variant: SearchTreeVariant,
    compensated_class: i32,
    id: ItemId,
) -> Vec<Option<TileShape>> {
    // Conduction areas read their STORED area (identity placement —
    // the Board::obstacle_area docs); obstacle areas go through the
    // placement-resolved getArea() chain.
    let area = match board.get(id).map(|entry| &entry.data) {
        Some(ItemData::ConductionArea { area, .. }) => Some(area.clone()),
        _ => board.obstacle_area(id),
    };
    let Some(area) = area else {
        return Vec::new();
    };
    let Some(convex_shapes) = area_split_to_convex(&area) else {
        // Java :913-915 — a null split yields new TileShape[0].
        return Vec::new();
    };
    // Structurally guaranteed: the convex split resolved means the
    // arena entry (and its area layer) exists (T7 quality review
    // NIT-3).
    let layer = board
        .area_layer(id)
        .expect("an area item with a convex split has a layer");
    let item_class = board
        .item_clearance_class(id)
        .expect("an arena entry always carries its clearance class");
    let max_width = max_tree_shape_width(board);
    let mut tree_shapes: Vec<Option<TileShape>> = Vec::new();
    for convex in convex_shapes {
        let offset_width =
            clearance_compensation_value(board.rules(), item_class, compensated_class, layer);
        let enlarged = convex.enlarge(f64::from(offset_width));
        tree_shapes.extend(
            enlarged
                .divide_into_sections(max_width)
                .into_iter()
                .map(Some),
        );
    }
    bounding_post_process(tree_shapes, variant)
}

/// Java `ShapeSearchTree.calculateTreeShapes(BoardOutline)`
/// `:940-990` (+ the 45°/90° overrides), both branches:
///
/// * **flag set** (`generateKeepoutOutside(true)`): the convex
///   division of the outside-the-outline [`keepout
///   area`](`crate::items::outline::keepout_area`), each piece
///   enlarged by the LAYER's compensation — LAYER-major, one pass per
///   layer, NO sectioning (`:945-964`);
/// * **flag clear** (every parse): the LINE keepout —
///   [`crate::items::outline::line_keepout_tiles`] with the
///   rules-driven compensation closure (`:965-987`; the geometry is
///   Task 4's, pinned there against the bm08 capture; the closure is
///   the Task 7 half).
fn outline_tree_shapes(
    board: &mut Board,
    variant: SearchTreeVariant,
    compensated_class: i32,
    id: ItemId,
) -> Vec<Option<TileShape>> {
    let Some(keepout_outside) = board.outline_keepout_outside_generated(id) else {
        return Vec::new();
    };
    let item_class = board
        .item_clearance_class(id)
        .expect("an arena entry always carries its clearance class");
    let layer_count = board.layers().layers.len();
    if keepout_outside {
        let Some(area) = board.outline_keepout_area(id) else {
            return Vec::new();
        };
        let Some(convex_shapes) = area_split_to_convex(&area) else {
            return Vec::new();
        };
        let mut tree_shapes = Vec::new();
        for layer_index in 0..layer_count {
            let offset_width = clearance_compensation_value(
                board.rules(),
                item_class,
                compensated_class,
                layer_index as i32,
            );
            for convex in &convex_shapes {
                tree_shapes.push(Some(convex.enlarge(f64::from(offset_width))));
            }
        }
        bounding_post_process(tree_shapes, variant)
    } else {
        let Some(shapes) = board.outline_shapes(id) else {
            return Vec::new();
        };
        let shapes = shapes.to_vec();
        let cmp = |layer: i32| {
            clearance_compensation_value(board.rules(), item_class, compensated_class, layer)
        };
        let tiles = crate::items::outline::line_keepout_tiles(
            &shapes,
            layer_count,
            crate::items::outline::half_width(),
            cmp,
        );
        bounding_post_process(tiles, variant)
    }
}

/// Java `ShapeSearchTree.calculateTreeShapes(PolylineTrace)`
/// `:992-1004`: one shape per interior line window at
/// `halfWidth + compensation(layer)` — `Polyline.offsetShape` for the
/// base and 45-degree trees (the 45° subclass does NOT override
/// `offsetShape`, `ShapeSearchTree.java:1079-1081`), `Polyline.
/// offsetBox` for the 90-degree tree (`ShapeSearchTree90Degree.java:
/// 488-490`).
fn trace_tree_shapes(
    board: &mut Board,
    variant: SearchTreeVariant,
    compensated_class: i32,
    id: ItemId,
) -> Vec<Option<TileShape>> {
    let Some(lines) = board.trace_polyline(id) else {
        return Vec::new();
    };
    let lines = lines.clone();
    let Some(layer) = board.trace_layer(id) else {
        return Vec::new();
    };
    let Some(half_width) = board.trace_half_width(id) else {
        return Vec::new();
    };
    let item_class = board.item_clearance_class(id).unwrap_or(0);
    let offset_width = half_width
        + clearance_compensation_value(board.rules(), item_class, compensated_class, layer);
    let count = crate::items::trace::tile_shape_count(&lines);
    (0..count)
        .map(|index| {
            let index = i32::try_from(index).unwrap_or(i32::MAX);
            match variant {
                SearchTreeVariant::NinetyDegree => {
                    lines.offset_box(offset_width, index).map(box_tile)
                }
                // The base tree's offsetShape == the 45-degree
                // subclass's (no override).
                SearchTreeVariant::Generic | SearchTreeVariant::FortyfiveDegree => {
                    lines.offset_shape(offset_width, index)
                }
            }
        })
        .collect()
}

/// A never-mutated empty padstack standing in for Java's null
/// `getPadstack()` in the (structurally unreachable) delta call of an
/// item whose padstack vanished — the empty shape list makes the
/// drill radius 0, which the delta's own `drillRadius <= 0` guard
/// turns into 0, exactly like Java's `drillItem.getPadstack() == null`
/// early-out (`:1040`).
static EMPTY_PADSTACK: BoardPadstack = BoardPadstack {
    name: String::new(),
    shapes: Vec::new(),
    drillable: false,
    placed_absolute: false,
    hole_only: false,
};

/// Wraps an [`IntBox`] as the [`TileShape`] the Java `TileShape`
/// locals carry (`boundingBox()` assigns an IntBox into a TileShape).
fn box_tile(box_shape: epic_geometry::int_box::IntBox) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(box_shape))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::ItemEntry;
    use crate::components::BoardLibrary;
    use crate::items::FixedState;
    use crate::rules_surf::ClearanceMatrix;
    use crate::test_util::parse_board_from_path as parse_board;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;
    use epic_geometry::int_point::IntPoint;

    // -----------------------------------------------------------------
    // Capture-driven pins (TreeShapesSpike, the frozen jar)
    // -----------------------------------------------------------------

    /// The Issue575 fixture (2 layers, FORTYFIVE restriction, 4
    /// classes: `v1.1 = v2.1 = v3.1 = 2000`, `cc(1) = 1000`,
    /// `v2.2 = 500`, `cc(2) = 250`, row 0 all zero — capture header
    /// `BOARD items=815 layerCount=2 restriction=FORTYFIVE_DEGREE
    /// holeClearance=0 classCount=4 defaultClass=1`).
    const ISSUE575: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn"
    );

    /// The 1Bitsy fixture (4 layers, `v1.1 = v2.1 = 1490`, `cc(1) =
    /// 745`, row 0 = 2000 against class 1).
    const ONEBITSY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/PCBench/1Bitsy_1bitsy/unrouted.dsn"
    );

    /// Renders a tile in the spike's capture format (`oct[...]` /
    /// `box[...]`, the exact `fmt` helper of
    /// `rust/harness/oracle/TreeShapesSpike.java`) so assertions quote
    /// the capture lines verbatim. A Simplex shape renders through its
    /// bounding octagon, exactly like the spike's `fmt`.
    fn capture_string(shape: &TileShape) -> String {
        match shape {
            TileShape::RegularTileShape(regular) => epic_index::format_bounds(regular),
            TileShape::Simplex(_) => {
                let octagon = shape
                    .bounding_octagon()
                    .expect("a bounded simplex has a bounding octagon");
                format!(
                    "tile[{}]",
                    epic_index::format_bounds(&RegularTileShape::IntOctagon(octagon))
                )
            }
        }
    }

    /// The captured Issue575 lines — VIA 815 (class 1,
    /// `Via[0-1]_600:300_um`, drillRadius 1500.0, circle r=3000 at
    /// 1312750,-728125) and the box PIN 350 (class 1, drillRadius
    /// 3825.0, box[1134500 -440350 1151500 -423350]) — across the
    /// three variants and the compensation classes. Every expectation
    /// is a quoted `treeShape=` capture field
    /// (/tmp/epic-t6-treeshapes.out).
    #[test]
    fn issue575_drill_shapes_match_the_jar_capture() {
        let mut board = parse_board(ISSUE575);
        let via = ItemId::new(815);
        let pin = ItemId::new(350);

        // Sanity: the items the capture selected.
        assert_eq!(board.item_clearance_class(via), Some(1));
        assert_eq!(board.drill_tile_shape_count(via), Some(2));
        assert_eq!(
            board.drill_padstack(via).expect("via padstack").name,
            "Via[0-1]_600:300_um"
        );

        // hc=0, cc0 base tree: octagon hull of the circle, no offset.
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[1309750 -731125 1315750 -725125 2036633 2045118 580383 588868]",
            "hc=0 base0 via (capture)"
        );
        // hc=0, cc0, 90-degree tree: the BOX hull.
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "box[1309750 -731125 1315750 -725125]",
            "hc=0 deg900 via (capture)"
        );

        // Hole clearance 200000: the delta fires (196510 for the via).
        board.rules_mut().set_hole_clearance(200_000);
        // base0: comp 0 + delta 196510, enlarge on the octagon.
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[1113240 -927635 1512260 -528615 1758726 2323025 302476 866775]",
            "hc=200000 base0 via — 45deg restriction -> octagon + enlarge"
        );
        // deg451: comp 1000 + delta 196510, offset.
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 1, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[1112240 -928635 1513260 -527615 1757312 2324439 301062 868189]",
            "hc=200000 deg451 via (capture)"
        );

        // THE BOX-PIN ASYMMETRY — the same width 193335, three
        // different results. base0 (enlarge of the octagon hull):
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[941165 -633685 1344835 -230015 1284433 1865267 420733 1001567]",
            "hc=200000 base0 box pin — enlarge moves the diagonals by w*sqrt(2)"
        );
        // deg450 (isIntBox swap -> box offset -> cut corners at w):
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 0, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[941165 -633685 1344835 -230015 1171180 1978520 307480 1114820]",
            "hc=200000 deg450 box pin — offset moves the sides by w"
        );
        // deg900 (box stays a box):
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "box[941165 -633685 1344835 -230015]",
            "hc=200000 deg900 box pin (capture)"
        );
        // deg451 (comp 1000 + delta 193335, offset path):
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 1, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[940165 -634685 1345835 -229015 1169180 1980520 305480 1116820]",
            "hc=200000 deg451 box pin (capture)"
        );
        // base1: the SAME total width through enlarge — the diagonals
        // differ from deg451 (1283019 vs 1169180).
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 1, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[940165 -634685 1345835 -229015 1283019 1866681 419319 1002981]",
            "hc=200000 base1 box pin (capture)"
        );

        // hc=2500: the NEGATIVE CLAMP — the via's raw delta is
        // 1500+2500+10-3000-2000 = -990 -> 0 (no inflation at all).
        board.rules_mut().set_hole_clearance(2500);
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape 0")),
            "oct[1309750 -731125 1315750 -725125 2036633 2045118 580383 588868]",
            "hc=2500 delta clamps to 0 — identical to hc=0 (capture)"
        );
    }

    /// The 1Bitsy capture — 4 layers and the SYNTHETIC hole padstack
    /// (copper circles on layers 0 and 3 only), inserted here exactly
    /// as the spike inserted it through the Java board API: padstack
    /// 19 (`spike_hole_600:300`), vias 804 (class 1, 700000,-300000)
    /// and 805 (class 0, 800000,-400000). Pins the null-shape slots
    /// and the synthesized drill-hole obstacle
    /// (/tmp/epic-t6-treeshapes-1bitsy.out).
    #[test]
    fn onebitsy_synthetic_hole_padstack_matches_the_jar_capture() {
        let mut board = parse_board(ONEBITSY);
        // The synthetic padstack: circle r=3000 on layers 0 and 3.
        let hole_circle = || Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 3000)));
        board.library_mut().padstacks.push(BoardPadstack {
            name: "spike_hole_600:300".to_string(),
            shapes: vec![hole_circle(), None, None, hole_circle()],
            drillable: true,
            placed_absolute: false,
            hole_only: false,
        });
        assert_eq!(
            board.library_mut().padstacks.len(),
            19,
            "SYNTH padstack id=19 (capture)"
        );
        let insert_hole_via = |board: &mut Board, center: IntPoint, class: i32| {
            let id = board.alloc_id();
            board.insert_item(ItemEntry {
                id,
                data: ItemData::Via {
                    center,
                    padstack_no: 19,
                    attach_smd_allowed: false,
                },
                nets: Vec::new(),
                clearance_class: class,
                component_id: 0,
                fixed: FixedState::SystemFixed,
                on_the_board: false,
            });
            id
        };
        // The unrouted parse consumes exactly 476 ids (T61: the digest's
        // items=476, the committed golden dsn-0020; the fresh re-run
        // capture /tmp/epic-t6-1bitsy-rerun.out inserts holeVia with
        // id=477, holeViaC0 with id=478 — byte-identical shape lines to
        // the original capture, whose ids 804/805 came from the
        // ROUTED variant of the fixture, reference-routed.dsn).
        assert_eq!(board.item_count(), 476, "golden dsn-0020 items=476");
        let hole_via = insert_hole_via(&mut board, IntPoint::new(700_000, -300_000), 1);
        assert_eq!(
            hole_via.get(),
            477,
            "capture: holeVia id=477 — the generator continues the parse"
        );
        let hole_via_c0 = insert_hole_via(&mut board, IntPoint::new(800_000, -400_000), 0);
        assert_eq!(hole_via_c0.get(), 478, "capture: holeViaC0 id=478");

        // hc=0, cc1: the hole layers are NULL slots (the obstacle rule
        // is off), the copper layers carry comp=745.
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 1, hole_via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("layer 0")),
            "oct[696255 -303745 703745 -296255 994704 1005297 394704 405297]",
            "hc=0 base1 holeVia layer 0 (capture)"
        );
        assert_eq!(shapes[1], None, "hc=0 hole layer 1 — null slot");
        assert_eq!(shapes[2], None, "hc=0 hole layer 2 — null slot");
        assert_eq!(shapes.len(), 4, "the padstack span");

        // hc=200000, cc1: the hole layers synthesize Circle r=1500
        // (ceil of drillRadius 1500.0) and inflate by comp 745 + delta
        // 198520; the copper layers take delta 197020 — and the
        // resulting octagons COINCIDE (3000-1500 cancels the delta
        // difference exactly), captured identical on all four layers.
        board.rules_mut().set_hole_clearance(200_000);
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 1, hole_via);
        for (index, shape) in shapes.iter().enumerate() {
            assert_eq!(
                capture_string(shape.as_ref().expect("all four layers live")),
                "oct[499235 -500765 900765 -99235 716076 1283925 116076 683925]",
                "hc=200000 base1 holeVia layer {index} (capture — the cancellation)"
            );
        }

        // hc=200000, cc0, CLASS-0 ITEM: comp is 0 (the T54 guard) and
        // copperClearance reads getValue(0, 1, l) = 0 (row 0 is all
        // zero on 1Bitsy) — copper delta 198510, hole delta 200010
        // (the same octagon extent by the same cancellation).
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, hole_via_c0);
        for (index, shape) in shapes.iter().enumerate() {
            assert_eq!(
                capture_string(shape.as_ref().expect("all four layers live")),
                "oct[598490 -601510 1001510 -198490 915022 1484979 115022 684979]",
                "hc=200000 base0 holeViaC0 layer {index} (capture)"
            );
        }

        // hc=2500: drillRadius 1250? — no: this padstack's drillRadius
        // is 1500; the class-1 via takes delta 0 (copper:
        // 1500+2510-3000-1490 = -480 -> 0) / 1020 (hole:
        // 1500+2510-1500-1490); the class-0 via — with
        // copperClearance 0 (row 0 all zero) — takes 1010 / 2510, and
        // the two deltas round the octagon diagonals DIFFERENTLY
        // (1194330 vs 1194329), captured.
        board.rules_mut().set_hole_clearance(2500);
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, hole_via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("layer 0")),
            "oct[697000 -303000 703000 -297000 995758 1004243 395758 404243]",
            "hc=2500 base0 holeVia layer 0 — delta 0 (capture)"
        );
        assert_eq!(
            capture_string(shapes[1].as_ref().expect("hole layer")),
            "oct[697480 -302520 702520 -297480 996437 1003564 396437 403564]",
            "hc=2500 base0 holeVia hole layer — delta 1020 (capture)"
        );
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, hole_via_c0);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("layer 0")),
            "oct[795990 -404010 804010 -395990 1194330 1205671 394330 405671]",
            "hc=2500 base0 holeViaC0 layer 0 — delta 1010 (capture)"
        );
        assert_eq!(
            capture_string(shapes[1].as_ref().expect("hole layer")),
            "oct[795990 -404010 804010 -395990 1194329 1205672 394329 405672]",
            "hc=2500 base0 holeViaC0 hole layer — delta 2510, the 1-off diagonal (capture)"
        );
    }

    // -----------------------------------------------------------------
    // Task 7 capture pins — the per-kind construction
    // (/tmp/epic-t7-shapes.out, the KIND_SHAPES lines)
    // -----------------------------------------------------------------

    /// The Issue575 TRACE and OBSTACLE captures: trace id 790 (class 1,
    /// single window) and keepout id 285 (class 1, LAYER 1 — a
    /// 180000x59500 box that sections into a 4x2 grid).
    #[test]
    fn issue575_trace_and_obstacle_shapes_match_the_jar_capture() {
        let mut board = parse_board(ISSUE575);
        let trace = ItemId::new(790);
        let obstacle = ItemId::new(285);

        // The trace: offsetShape at halfWidth + comp — the base and
        // 45-degree trees share the path (no override), the 90-degree
        // tree takes offsetBox.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, trace);
        assert_eq!(shapes.len(), 1, "a single-segment trace has one window");
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("window")),
            "oct[1336840 -430900 1358500 -428900 1766326 1788814 906526 929014]",
            "base0 trace 790 (capture)"
        );
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, trace);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("window")),
            "box[1336840 -430900 1358500 -428900]",
            "deg900 trace 790 — offsetBox, the three-line window's bounding box (capture)"
        );
        // comp(1, 1, 0) = 1000: the base and 45-degree trees agree.
        for variant in [
            SearchTreeVariant::Generic,
            SearchTreeVariant::FortyfiveDegree,
        ] {
            let shapes = item_tree_shapes(&mut board, variant, 1, trace);
            assert_eq!(
                capture_string(shapes[0].as_ref().expect("window")),
                "oct[1335840 -431900 1359500 -427900 1764912 1790228 905112 930428]",
                "cc1 {variant:?} trace 790 (capture)"
            );
        }

        // The obstacle: convex division ([this] for the tile border),
        // enlarge(0), divideIntoSections(50000) — the 4x2 grid.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, obstacle);
        let expected_sections = [
            "box[1180000 -414500 1225000 -384750]",
            "box[1225000 -414500 1270000 -384750]",
            "box[1270000 -414500 1315000 -384750]",
            "box[1315000 -414500 1360000 -384750]",
            "box[1180000 -384750 1225000 -355000]",
            "box[1225000 -384750 1270000 -355000]",
            "box[1270000 -384750 1315000 -355000]",
            "box[1315000 -384750 1360000 -355000]",
        ];
        assert_eq!(shapes.len(), expected_sections.len(), "the 4x2 grid");
        for (shape, expected) in shapes.iter().zip(expected_sections) {
            assert_eq!(
                capture_string(shape.as_ref().expect("section")),
                expected,
                "base0 obstacle 285 section (capture)"
            );
        }
        // The 90-degree post-process keeps the boxes verbatim.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, obstacle);
        for (shape, expected) in shapes.iter().zip(expected_sections) {
            assert_eq!(
                capture_string(shape.as_ref().expect("section")),
                expected,
                "deg900 obstacle 285 — boundingBox of a box (capture)"
            );
        }
        // The 45-degree post-process: boundingOctagon of each section
        // (the box hull octagon, diagonals at the corners).
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 0, obstacle);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "oct[1180000 -414500 1225000 -384750 1564750 1639500 765500 840250]",
            "deg450 obstacle 285 section 0 (capture)"
        );
        assert_eq!(
            capture_string(shapes[7].as_ref().expect("section")),
            "oct[1315000 -384750 1360000 -355000 1670000 1744750 930250 1005000]",
            "deg450 obstacle 285 section 7 (capture)"
        );
        // comp(1, 1, 1) = 1000 on the keepout's layer: enlarge BEFORE
        // the post-process — the enlarged box's octagon diagonals
        // (1563750/764086...) are NOT the box-then-octagon's
        // (1564750/765500...): the order is the discriminator.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 1, obstacle);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "oct[1179000 -415500 1224500 -384750 1563750 1640000 764086 839750]",
            "deg451 obstacle 285 section 0 — enlarge(1000) then boundingOctagon (capture)"
        );
        assert_eq!(
            capture_string(shapes[7].as_ref().expect("section")),
            "oct[1315500 -384750 1361000 -354000 1669500 1745750 930750 1006414]",
            "deg451 obstacle 285 section 7 (capture)"
        );
    }

    /// The Issue575 BOARD-OUTLINE line-branch capture: lineCount 4 x 2
    /// layers, the Task 4 window geometry at halfWidth 100 + the
    /// compensation closure (base0 comp 0, deg451 comp 1000 per layer).
    #[test]
    fn issue575_outline_line_shapes_match_the_jar_capture() {
        let mut board = parse_board(ISSUE575);
        let outline = ItemId::new(1);

        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, outline);
        let base0 = [
            "oct[1124900 -933100 1415100 -932900 2057859 2348141 191859 482141]",
            "oct[1414900 -933100 1415100 -413900 1828859 2348141 481859 1001141]",
            "oct[1124900 -414100 1415100 -413900 1538859 1829141 710859 1001141]",
            "oct[1124900 -933100 1125100 -413900 1538859 2058141 191859 711141]",
        ];
        assert_eq!(shapes.len(), 8, "lineCount 4 x layerCount 2");
        for (layer, window) in shapes.chunks(4).enumerate() {
            for (shape, expected) in window.iter().zip(base0) {
                assert_eq!(
                    capture_string(shape.as_ref().expect("window")),
                    expected,
                    "base0 outline layer {layer} (capture)"
                );
            }
        }
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 1, outline);
        let deg451 = [
            "oct[1123900 -934100 1416100 -931900 2056444 2349556 190444 483556]",
            "oct[1413900 -934100 1416100 -412900 1827444 2349556 480444 1002556]",
            "oct[1123900 -415100 1416100 -412900 1537444 1830556 709444 1002556]",
            "oct[1123900 -934100 1126100 -412900 1537444 2059556 190444 712556]",
        ];
        for (shape, expected) in shapes.iter().take(4).zip(deg451) {
            assert_eq!(
                capture_string(shape.as_ref().expect("window")),
                expected,
                "deg451 outline layer 0 — comp 1000 into the closure (capture)"
            );
        }
    }

    /// The Issue054-tairakb fixture (the CONDUCTION-area board).
    const ISSUE054: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/Issue054-tairakb.dsn"
    );

    /// The Issue054 CONDUCTION capture (plane id 3, class 1, layer 1 —
    /// a big polygon border that convex-divides and sections into
    /// 1046 SIMPLEX pieces) and the small polygon keepout 5847 (one
    /// convex piece, no sectioning). The plane drives the whole
    /// obstacle path — polygon split, enlarge, sectioning, and all
    /// three variant post-processes — at scale.
    #[test]
    fn issue054_conduction_and_obstacle_shapes_match_the_jar_capture() {
        let mut board = parse_board(ISSUE054);
        let conduction = ItemId::new(3);
        let obstacle = ItemId::new(5847);

        // The conduction plane: 1046 sections in every variant (the
        // sectioning never changes the COUNT; the post-process only
        // rewraps each shape).
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, conduction);
        assert_eq!(shapes.len(), 1046, "capture: n=1046");
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "tile[oct[1514877 -1671208 1516460 -1670897 3185774 3187668 -156020 -154437]]",
            "base0 conduction 3 section 0 — a Simplex rendered by its bounding octagon (capture)"
        );
        assert_eq!(
            capture_string(shapes[1].as_ref().expect("section")),
            "tile[oct[1516460 -1680910 1565938 -1670897 3187357 3246848 -154748 -104959]]",
            "base0 conduction 3 section 1 (capture)"
        );
        // The 45-degree post-process: the Simplex BECOMES its bounding
        // octagon (same ints as the tile[...] rendering).
        let shapes = item_tree_shapes(
            &mut board,
            SearchTreeVariant::FortyfiveDegree,
            0,
            conduction,
        );
        assert_eq!(shapes.len(), 1046);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "oct[1514877 -1671208 1516460 -1670897 3185774 3187668 -156020 -154437]",
            "deg450 conduction 3 section 0 — boundingOctagon of the Simplex (capture)"
        );
        // The 90-degree post-process: the bounding box.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, conduction);
        assert_eq!(shapes.len(), 1046);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "box[1514877 -1671208 1516460 -1670897]",
            "deg900 conduction 3 section 0 — boundingBox of the Simplex (capture)"
        );
        // comp(1, 1, 1) = 1001: every section enlarges by 1001 before
        // the post-process.
        let shapes = item_tree_shapes(
            &mut board,
            SearchTreeVariant::FortyfiveDegree,
            1,
            conduction,
        );
        assert_eq!(shapes.len(), 1046);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("section")),
            "oct[1514454 -1672215 1516395 -1671834 3186288 3188610 -157380 -155439]",
            "deg451 conduction 3 section 0 — enlarge(1001) + octagon (capture)"
        );
        assert_eq!(
            capture_string(shapes[1].as_ref().expect("section")),
            "oct[1516395 -1681927 1565925 -1671834 3188229 3247852 -155820 -105909]",
            "deg451 conduction 3 section 1 (capture)"
        );

        // The polygon keepout 5847: ONE convex piece below the
        // sectioning threshold.
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, obstacle);
        assert_eq!(shapes.len(), 1, "capture: n=1");
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("piece")),
            "oct[915950 -588750 933950 -570750 1491973 1517428 332473 357928]",
            "base0 obstacle 5847 — the polygon's convex piece (capture)"
        );
    }

    // -----------------------------------------------------------------
    // Task 7 capture pins — the sectioning threshold
    // (/tmp/epic-t7-shapes.out, the three crafted COMM boards)
    // -----------------------------------------------------------------

    /// `(resolution um 10)` + host CAD — the corpus situation: 500 x
    /// 254 = 127000, the CAP wins (threshold 50000).
    const T7_SECTION_DSN: &str = r#"(pcb t7-section.dsn
  (parser
    (host_cad KICAD)
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 14000 8000))
    (keepout (rect F.Cu 0 0 12000 6000))
    (rule (width 250) (clearance 200))
  )
  (placement)
  (library)
  (network
    (net T7NET)
  )
)
"#;

    /// `(resolution um 1)` + host CAD — the CLAMP: 500 x 25.4 =
    /// 12700 < 50000, the clamp wins (threshold 12700).
    const T7_CLAMP_DSN: &str = r#"(pcb t7-clamp.dsn
  (parser
    (host_cad KICAD)
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 140000 80000))
    (keepout (rect F.Cu 0 0 120000 60000))
    (rule (width 250) (clearance 200))
  )
  (placement)
  (library)
  (network
    (net T7NET)
  )
)
"#;

    /// `(resolution um 1)` with NO host CAD — the GUARD: the min is
    /// skipped entirely, threshold 50000 even at um 1.
    const T7_NOHOST_DSN: &str = r#"(pcb t7-nohost.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 140000 80000))
    (keepout (rect F.Cu 0 0 120000 60000))
    (rule (width 250) (clearance 200))
  )
  (placement)
  (library)
  (network
    (net T7NET)
  )
)
"#;

    /// Parses crafted DSN text through the epic-dsn reader (the same
    /// strings the Java spike parsed — both readers must agree).
    fn parse_dsn_text(dsn: &str) -> Board {
        let mut ses = SesBoard::new();
        match read_board(dsn.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "crafted DSN: {warnings:?}");
            }
            other => panic!("expected Success for the crafted DSN, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// **The resolution clamp of the sectioning threshold**
    /// (`ShapeSearchTree.java:916-920`), jar-captured on three crafted
    /// boards whose keepout INTERNAL box is identical (120000x60000)
    /// and whose only differences are the clamp inputs:
    ///
    /// * SECTION (um 10 + host CAD): threshold 50000 -> a 3x2 grid
    ///   (sections 40000x30000),
    /// * CLAMP (um 1 + host CAD): threshold 12700 -> a 10x5 grid of
    ///   12000x12000 sections — 50 shapes,
    /// * NOHOST (um 1, NO host CAD): the guard skips the min ->
    ///   threshold 50000 -> the SAME 3x2 grid as SECTION.
    ///
    /// CLAMP-vs-NOHOST isolates the `hostCadExists()` guard;
    /// SECTION-vs-CLAMP isolates the resolution arithmetic — the forms
    /// where a wrong threshold, a dropped guard, or a hardcoded 50000
    /// all differ from the capture.
    #[test]
    fn sectioning_threshold_follows_the_resolution_clamp() {
        let section = parse_dsn_text(T7_SECTION_DSN);
        let clamp = parse_dsn_text(T7_CLAMP_DSN);
        let nohost = parse_dsn_text(T7_NOHOST_DSN);

        // The COMM facts (capture: resMil 254.0 / 25.4 / 25.4; the
        // thresholds 50000.0 / 12700.0 / 50000.0 — the spike's
        // threshold line prints the FORMULA, the section grid shows
        // the APPLIED value, which is what these assertions pin).
        assert!(section.communication().host_cad_exists());
        assert!(clamp.communication().host_cad_exists());
        assert!(!nohost.communication().host_cad_exists());
        assert_eq!(section.communication().resolution_mil(), 254.0);
        assert_eq!(clamp.communication().resolution_mil(), 25.4);
        assert_eq!(nohost.communication().resolution_mil(), 25.4);
        assert_eq!(max_tree_shape_width(&section), 50_000.0);
        assert_eq!(max_tree_shape_width(&clamp), 12_700.0);
        assert_eq!(
            max_tree_shape_width(&nohost),
            50_000.0,
            "no host CAD -> the constant, the min never applies"
        );

        // The keepout is item id 2 on every crafted board (outline 1,
        // keepout 2 — items=2 in the capture).
        let keepout = ItemId::new(2);
        let grid = |board: &mut Board| {
            item_tree_shapes(board, SearchTreeVariant::Generic, 0, keepout)
                .into_iter()
                .map(|shape| capture_string(shape.as_ref().expect("section")))
                .collect::<Vec<_>>()
        };

        let section_grid = grid(&mut { section.clone() });
        let expected_grid = [
            "box[0 0 40000 30000]",
            "box[40000 0 80000 30000]",
            "box[80000 0 120000 30000]",
            "box[0 30000 40000 60000]",
            "box[40000 30000 80000 60000]",
            "box[80000 30000 120000 60000]",
        ];
        assert_eq!(
            section_grid, expected_grid,
            "SECTION: the 3x2 grid (capture)"
        );

        let clamp_grid = grid(&mut { clamp.clone() });
        assert_eq!(clamp_grid.len(), 50, "CLAMP: the 10x5 grid (capture n=50)");
        assert_eq!(
            clamp_grid[0], "box[0 0 12000 12000]",
            "CLAMP first (capture)"
        );
        assert_eq!(
            clamp_grid[1], "box[12000 0 24000 12000]",
            "CLAMP second (capture)"
        );
        assert_eq!(
            clamp_grid[49], "box[108000 48000 120000 60000]",
            "CLAMP last (capture)"
        );

        let nohost_grid = grid(&mut { nohost.clone() });
        assert_eq!(
            nohost_grid, expected_grid,
            "NOHOST: the same 3x2 grid as SECTION — only the host-CAD guard differs"
        );
    }

    // -----------------------------------------------------------------
    // T54 — the compensation formula, on a hand-built ASYMMETRIC
    // matrix (the fixture matrices are symmetric: the row/column
    // order is unobservable there)
    // -----------------------------------------------------------------

    /// An asymmetric 3-class, 2-layer matrix:
    /// `getValue(1, 2, 0) = 100` but `getValue(2, 1, 0) = 5000`;
    /// diagonal `(1,1,l0) = 2000` -> `cc(1) = 1000`, `(2,2,l0) = 600`
    /// -> `cc(2) = 300`.
    fn asymmetric_rules() -> BoardRules {
        let mut rules = BoardRules::new();
        rules.clearance = ClearanceMatrix::new(
            3,
            2,
            vec![
                "null".to_string(),
                "default".to_string(),
                "power".to_string(),
            ],
        );
        rules.clearance.set_value(1, 2, 0, 100);
        rules.clearance.set_value(2, 1, 0, 5000);
        rules.clearance.set_value(1, 1, 0, 2000);
        rules.clearance.set_value(2, 2, 0, 600);
        rules
    }

    /// **T54, the asymmetric read.** In a class-1 tree a class-2 item
    /// reads `getValue(2, 1, 0) = 5000 - cc(1)=1000 -> 4000`; a port
    /// that swaps the arguments reads `getValue(1, 2, 0) = 100 - 1000
    /// -> clamped 0`. Both directions are asserted — this is the form
    /// where wrong and right differ.
    #[test]
    fn compensation_reads_the_matrix_asymmetrically() {
        let rules = asymmetric_rules();
        assert_eq!(
            clearance_compensation_value(&rules, 2, 1, 0),
            4000,
            "the item class is the ROW argument: 5000 - 1000"
        );
        // The mirrored query (a class-1 item in a class-2 tree) reads
        // 100 - cc(2)=300 -> negative -> the CLAMP.
        assert_eq!(
            clearance_compensation_value(&rules, 1, 2, 0),
            0,
            "100 - 300 clamps to 0 — the swapped-argument port returns 4000's mirror, not 0"
        );
    }

    /// **T54, the negative clamp** (`:113`) — without `max(result, 0)`
    /// the class-1-in-class-2 read returns -200 and POISONUSLY
    /// shrinks the tree shape on `enlarge(-200)`. The layer-1 read
    /// below (an all-zero layer) is the control where clamp and
    /// no-clamp agree — layer 0 is the discriminating form.
    #[test]
    fn compensation_clamps_negatives_to_zero() {
        let rules = asymmetric_rules();
        assert_eq!(clearance_compensation_value(&rules, 1, 2, 0), 0);
        assert_eq!(
            clearance_compensation_value(&rules, 1, 2, 1),
            0,
            "all-zero layer: the control form (0 either way)"
        );
    }

    /// **T54, the class-0 guard** (`:105-107`): a class-0 (null-class)
    /// item compensates 0 EVEN where the unguarded formula would NOT
    /// clamp — a HAND-BUILT matrix (`getValue(0, 1, l) = 2000` against
    /// `cc(1) = 745`, unguarded 1255; row 0 is all zero on every
    /// capture fixture, so no jar capture can discriminate). A port
    /// without the guard inflates class-0 items in compensated trees;
    /// the captures alone could never tell the difference.
    #[test]
    fn compensation_guard_zeroes_the_null_class() {
        let mut rules = BoardRules::new();
        rules.clearance =
            ClearanceMatrix::new(2, 1, vec!["null".to_string(), "default".to_string()]);
        rules.clearance.set_value(0, 1, 0, 2000);
        rules.clearance.set_value(1, 1, 0, 1490);
        assert_eq!(
            clearance_compensation_value(&rules, 0, 1, 0),
            0,
            "the guard fires before the matrix read"
        );
        // The unguarded arithmetic is POSITIVE (2000 - 745 = 1255):
        // only the guard — not the clamp — produces 0.
        assert_eq!(
            rules.clearance.get_value_opt(0, 1, 0, false)
                - rules.clearance.clearance_compensation_value(1, 0),
            1255,
            "the unguarded value is positive: wrong and right differ"
        );
    }

    // -----------------------------------------------------------------
    // T55 — the delta's class choice and the holeOnly arm
    // -----------------------------------------------------------------

    /// A 2000-radius circle padstack named `ps_200:100` (smallestRadius
    /// 2000, ratio 100/200) -> drillRadius exactly 1000.
    fn circle_padstack() -> BoardPadstack {
        BoardPadstack {
            name: "ps_200:100".to_string(),
            shapes: vec![Some(BoardShape::Circle(Circle::new(
                IntPoint::new(0, 0),
                2000,
            )))],
            drillable: true,
            placed_absolute: false,
            hole_only: false,
        }
    }

    /// **T55 — the delta's clearance class is the TREE's, defaulting
    /// to 1** (`:1058-1061`): in a class-0 tree the copper clearance
    /// reads `getValue(item, 1, l)` (the DEFAULT class), not
    /// `getValue(item, 0, l)`. Hand-built matrix where the two differ
    /// by 500 — the captured 1Bitsy class-0 deltas (198510/200010)
    /// exercise the same path end-to-end above, but under an all-zero
    /// row 0 (`getValue(0,1,l) = getValue(0,0,l) = 0`) they cannot
    /// DISCRIMINATE the default; this matrix does.
    #[test]
    fn delta_uses_the_tree_class_defaulting_to_one() {
        let mut rules = BoardRules::new();
        rules.set_hole_clearance(10_000);
        rules.clearance =
            ClearanceMatrix::new(2, 1, vec!["null".to_string(), "default".to_string()]);
        rules.clearance.set_value(1, 0, 0, 100);
        rules.clearance.set_value(1, 1, 0, 600);
        let padstack = circle_padstack();
        let center = Point::Int(IntPoint::new(0, 0));
        // drillRadius 1000, copperRadius 2000 (the centered circle).
        // class-0 tree: clearance class 1 -> 600:
        // ceil(1000 + 10000 + 10 - 2000 - 600) = 8410.
        // A port reading class 0 (100) instead: 8910.
        assert_eq!(
            drill_hole_clearance_delta(
                &rules,
                &padstack,
                &center,
                padstack.shapes[0].as_ref(),
                1,
                0,
                0
            ),
            8410,
            "a class-0 tree reads the DEFAULT class (1)"
        );
        // A class-2 tree on this 2-class matrix: class 2 is out of
        // bounds, the read is 0 -> ceil(1000 + 10010 - 2000) = 9010
        // (the OOB-early-return-0 of ClearanceMatrix.getValue).
        assert_eq!(
            drill_hole_clearance_delta(
                &rules,
                &padstack,
                &center,
                padstack.shapes[0].as_ref(),
                1,
                2,
                0
            ),
            9010,
            "an out-of-matrix tree class reads 0"
        );
    }

    /// **T55 — `holeOnly` short-circuits the copper radius to the
    /// drill radius** (`:1049-1050`): the delta then measures from the
    /// HOLE, not the copper. Java never writes `holeOnly` on a parsed
    /// padstack (grep-verified — no writer in the main tree), so this
    /// pins the field's one read site on a hand-built padstack.
    #[test]
    fn delta_hole_only_uses_the_drill_radius_as_copper_radius() {
        let mut rules = BoardRules::new();
        rules.set_hole_clearance(10_000);
        rules.clearance = ClearanceMatrix::new(2, 1, vec!["a".to_string(), "b".to_string()]);
        rules.clearance.set_value(1, 1, 0, 600);
        let mut padstack = circle_padstack();
        let center = Point::Int(IntPoint::new(0, 0));
        // Not hole-only: copperRadius 2000 -> ceil(1000+10010-2000-600) = 8410.
        assert_eq!(
            drill_hole_clearance_delta(
                &rules,
                &padstack,
                &center,
                padstack.shapes[0].as_ref(),
                1,
                1,
                0
            ),
            8410
        );
        // Hole-only: copperRadius 1000 -> ceil(1000+10010-1000-600) = 9410.
        padstack.hole_only = true;
        assert_eq!(
            drill_hole_clearance_delta(
                &rules,
                &padstack,
                &center,
                padstack.shapes[0].as_ref(),
                1,
                1,
                0
            ),
            9410,
            "holeOnly measures from the drill hole"
        );
    }

    /// **T55 — the obstacle's guards and the ceil** (`:1012-1028`):
    /// hole clearance off -> None; a zero drill radius -> None;
    /// otherwise the CEIL of the drill radius at the (rounded) center.
    /// The radius value comes from the colon-less 0.45 fallback
    /// (`2001 * 0.45 = 900.45` -> 901) — captured via Double.toString,
    /// not a formatting toString.
    #[test]
    fn drill_hole_obstacle_guards_and_ceil() {
        let fallback = BoardPadstack {
            name: "no_colon".to_string(),
            shapes: vec![Some(BoardShape::Circle(Circle::new(
                IntPoint::new(0, 0),
                2001,
            )))],
            drillable: true,
            placed_absolute: false,
            hole_only: false,
        };
        assert!(
            (fallback.drill_radius() - 900.45).abs() < 1e-9,
            "0.45 fallback: {}",
            fallback.drill_radius()
        );
        let center = Point::Int(IntPoint::new(700_000, -300_000));
        match drill_hole_obstacle(200_000, &fallback, &center) {
            Some(BoardShape::Circle(circle)) => {
                assert_eq!(circle.center, IntPoint::new(700_000, -300_000));
                assert_eq!(circle.radius, 901, "ceil(900.45...)");
            }
            other => panic!("expected a circle, got {other:?}"),
        }
        // The rule off -> None.
        assert_eq!(drill_hole_obstacle(0, &fallback, &center), None);
        // A shape-less padstack: drillRadius 0 -> None.
        let zero = BoardPadstack {
            name: "z".to_string(),
            shapes: Vec::new(),
            drillable: true,
            placed_absolute: false,
            hole_only: false,
        };
        assert_eq!(zero.drill_radius(), 0.0);
        assert_eq!(drill_hole_obstacle(100, &zero, &center), None);
    }

    /// The dispatcher (Java's virtual `calculateTreeShapes`): every
    /// kind routes to its per-kind construction — drill (the via's
    /// padstack span), trace (one shape per interior line window),
    /// obstacle area, conduction area, board outline (lineCount x
    /// layerCount line-keepout tiles) — EXCEPT the component outline,
    /// where Java returns `new TileShape[0]`
    /// (`ComponentOutline.java:135-137`): no shapes, hence no tree
    /// entries, while the item still goes on the board.
    #[test]
    fn item_tree_shapes_dispatches_per_kind() {
        let mut board = parse_board(ISSUE575);
        // Drill: the via's padstack span (2 layers of copper).
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, ItemId::new(815));
        assert_eq!(shapes.len(), 2, "the via's padstack span");
        // The trace (id 790 on the parse): one tile per interior line
        // window (>= 1 for any real trace — a plain segment has one).
        let trace_lines = board.trace_polyline(ItemId::new(790)).expect("trace");
        let expected = crate::items::trace::tile_shape_count(trace_lines);
        assert!(expected >= 1, "a trace has at least one line window");
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, ItemId::new(790));
        assert_eq!(shapes.len(), expected, "trace: one shape per window");
        // The obstacle area (first keepout id 285): convex division +
        // sectioning — non-empty; the exact section count is
        // threshold-dependent (the capture pins below quote it from
        // the jar).
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, ItemId::new(285));
        assert!(
            !shapes.is_empty(),
            "obstacle: the keepout carries convex-section shapes"
        );
        // The board outline (id 1 on every parse): lineCount x
        // layerCount line-keepout tiles.
        let line_count = crate::items::outline::line_count(
            board
                .outline_shapes(ItemId::new(1))
                .expect("outline shapes"),
        );
        assert!(line_count >= 4, "a closed polygon outline");
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, ItemId::new(1));
        assert_eq!(
            shapes.len(),
            line_count * board.layers().layers.len(),
            "outline: the line keepout, layer-major"
        );
        // The component outline (first id 384): EMPTY — Java's
        // new TileShape[0].
        let shapes = item_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, ItemId::new(384));
        assert!(
            shapes.is_empty(),
            "ComponentOutline.calculateTreeShapes returns new TileShape[0]"
        );
        assert_eq!(
            board
                .get(ItemId::new(384))
                .expect("component outline 384")
                .board_item_type(),
            crate::items::BoardItemType::ComponentOutline,
            "sanity: 384 is a component outline"
        );
        // A foreign id: empty.
        let shapes = item_tree_shapes(
            &mut board,
            SearchTreeVariant::Generic,
            0,
            ItemId::new(999_999),
        );
        assert!(shapes.is_empty());
    }

    /// The base dispatch reads the restriction at CALL time
    /// (`:885-893`) — the spike's RESTRICTION_SWEEP phase captured all
    /// three arms on Issue575 (no corpus fixture parses with anything
    /// but FORTYFIVE_DEGREE). The captures discriminate the arms:
    /// under NINETY the box hull is still ENLARGED — and
    /// `IntBox.enlarge` returns the box's bounding OCTAGON (diagonals
    /// at the box corners: `...2034875 2046875...`), NOT the
    /// subclass's `box[...]`; the FORTYFIVE arm's circle octagon pulls
    /// the diagonals in to contain the circle (`...2036633
    /// 2045118...`), and at hc=200000 the two arms' diagonals still
    /// differ (`1756968` vs `1758726`) — the form where a port that
    /// dispatches by variant instead of by call-time restriction, or
    /// that returns the box, fails.
    #[test]
    fn generic_dispatch_follows_the_call_time_restriction() {
        let mut board = parse_board(ISSUE575);
        let via = ItemId::new(815);
        let pin = ItemId::new(350);

        // NINETY: boundingBox + enlarge -> the box's OCTAGON.
        board.rules_mut().trace_angle_restriction = AngleRestriction::NinetyDegree;
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1309750 -731125 1315750 -725125 2034875 2046875 578625 590625]",
            "NINETY sweep hc=0 via (capture) — the box hull enlarged to its octagon"
        );
        board.rules_mut().set_hole_clearance(200_000);
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1113240 -927635 1512260 -528615 1756968 2324782 300718 868532]",
            "NINETY sweep hc=200000 via (capture) — diagonals at the BOX corners"
        );
        assert_ne!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1113240 -927635 1512260 -528615 1758726 2323025 302476 866775]",
            "the FORTYFIVE arm's hc=200000 diagonals (circle octagon) differ"
        );
        // The BOX PIN under NINETY: box + enlarge (identical here to
        // the FORTYFIVE arm's octagon-enlarge, both 1284433 — pinned so
        // the agreement itself is guarded).
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[941165 -633685 1344835 -230015 1284433 1865267 420733 1001567]",
            "NINETY sweep hc=200000 box pin (capture) — == the FORTYFIVE arm"
        );

        // NONE: boundingTile — the circle's own octagon tile (== the
        // FORTYFIVE arm), the box stays a box pre-enlarge.
        board.rules_mut().set_hole_clearance(0);
        board.rules_mut().trace_angle_restriction = AngleRestriction::None;
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1309750 -731125 1315750 -725125 2036633 2045118 580383 588868]",
            "NONE sweep hc=0 via (capture) — boundingTile == the FORTYFIVE arm"
        );
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, pin);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1134500 -440350 1151500 -423350 1557850 1591850 694150 728150]",
            "NONE sweep hc=0 box pin (capture) — the box tile enlarged"
        );
        board.rules_mut().set_hole_clearance(200_000);
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::Generic, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1113240 -927635 1512260 -528615 1758726 2323025 302476 866775]",
            "NONE sweep hc=200000 via (capture) — == the FORTYFIVE arm"
        );

        // The subclass variants IGNORE the board restriction: under a
        // NINETY restriction the 90-degree subclass still stores the
        // BOX, the 45-degree subclass the offset octagon.
        board.rules_mut().trace_angle_restriction = AngleRestriction::NinetyDegree;
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::NinetyDegree, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "box[1113240 -927635 1512260 -528615]",
            "the 90-degree subclass under a NINETY restriction (== its FORTYFIVE-run capture)"
        );
        let shapes = drill_tree_shapes(&mut board, SearchTreeVariant::FortyfiveDegree, 0, via);
        assert_eq!(
            capture_string(shapes[0].as_ref().expect("shape")),
            "oct[1113240 -927635 1512260 -528615 1758726 2323025 302476 866775]",
            "the 45-degree subclass ignores the NINETY restriction (capture; == base0 — a circle pad)"
        );
    }

    /// The unused-import guard: [`BoardLibrary`] is referenced only by
    /// the library_mut seam assertion below.
    #[allow(dead_code)]
    fn _library_type_witness(_: &BoardLibrary) {}
}
