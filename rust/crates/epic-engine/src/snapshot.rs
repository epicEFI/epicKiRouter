//! The M9-T3 `BoardSnapshot`: the plain-`Send` render projection of a
//! live board at one revision (design §7 — "the GUI renders, never
//! mutates"; the GUI holds snapshots, never boards). The builder
//! [`board_snapshot`] takes `&Board` ONLY (the renders-never-mutates
//! law, structural) and reads ONLY the verified board read faces:
//!
//! * `get` (`board.rs:426`) via the iteration faces,
//! * `iter_ascending` (`:444`) — the ONE ascending-id walk
//!   (creation order), the snapshot's deterministic item order,
//! * `revision` (`:466`) — the u64 dirty tick the event stream
//!   dedups on,
//! * `bounding_box` (`:693`),
//! * `drill_shape` (`:908`) — the per-index padstack shape of a
//!   pin/via,
//! * `drill_tile_shape_count` — the padstack span
//!   (`DrillItem.java:202-208`, `toLayer - fromLayer + 1`),
//! * `trace_polyline` (`:982`) / `trace_layer` (`:992`) /
//!   `trace_half_width` (`:1002`),
//! * `area_layer` (`:1087`),
//! * `item_shape_layer_read` (`:1154`) — the `&self` drill-layer
//!   face (the memoizing `drill_first_layer` twin needs `&mut`, which
//!   the snapshot builder must never take),
//! * `conduction_area` (`:1257`) — the STORED area (parse-time
//!   conduction areas carry identity placement, the face's own docs),
//! * `outline_shapes` (`:1268`) — the board outline's shapes
//!   (`BoardOutline.getShape(i)` backing),
//! * `rules` (`:472`) — the net table walk (`rules().nets.iter()`,
//!   the `NetInfo` source),
//! * `obstacle_area` (`items/obstacle.rs:143`) — the T49
//!   absolute-area chain for keepout areas (the placement transform
//!   is applied lazily on the stored area, the snapshot ships the
//!   TRANSFORMED outline).
//!
//! # The overlay slots (M9-T5) and the purity law
//!
//! [`BoardSnapshot::overlays`] carries the four §7 overlay DATA faces
//! ([`OverlayData`]). THE PURITY LAW: [`board_snapshot`] takes
//! `&Board` ONLY and fills the slots EMPTY by construction — both DRC
//! overlay faces (`all_incompletes`, `all_clearance_violation_depths`)
//! take `&mut`, which the pure builder must never take. The pass-
//! granular `EngineEvent::Snapshot`s the tee ships therefore carry
//! the EMPTY default (the documented honest cadence; pinned) — the
//! attach step is [`crate::session::Session::snapshot_with_overlays`].
//!
//! Coordinates are DBU i64 ([`PointPrimitive`]); shape outlines are
//! polygons via the geometry crate's existing faces — NO new
//! geometry: [`TileShape::border_line`] /
//! [`PolygonShape::border_line`] (the directed border lines' start
//! corners, in shape order) and, for a `Circle` (which has no
//! corners — `Circle::corner_approx_arr` is empty by port docs), the
//! four corners of its own `Circle::bounding_box` — an
//! APPROXIMATION (quality Q6): every round via/pad shape ships as a
//! 4-corner rectangle on the wire; determinism is the only contract,
//! and T4's renderer must know it draws squares for discs unless it
//! consumes a circle primitive. Determinism: same
//! board state → byte-identical [`serde_json`] output (pinned in
//! `events_stream.rs`).

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::{Area, BoardItemType, BoardShape, ItemData, ObstacleKind};
use epic_drc::clearance::DepthRow;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use serde::Serialize;

/// A DBU point as plain i64 coordinates (the snapshot wire shape —
/// `epic_geometry::IntPoint` is i32 and carries no `Serialize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PointPrimitive {
    /// The x coordinate (DBU).
    pub x: i64,
    /// The y coordinate (DBU).
    pub y: i64,
}

impl PointPrimitive {
    fn from_int_point(point: &IntPoint) -> Self {
        Self {
            x: i64::from(point.x),
            y: i64::from(point.y),
        }
    }

    /// The snapshot's one Point → i64 narrowing. The `Int` arm is
    /// exact; the `Rational` arm (a projection artifact — parsed
    /// shapes are int-cornered, so this is defensive) rounds the
    /// rational's `to_float` face to the NEAREST DBU (f64 `round`,
    /// half away from zero) — deterministic on a fixed board state,
    /// which is the only contract the snapshot carries.
    fn from_point(point: &Point) -> Self {
        match point {
            Point::Int(int_point) => Self::from_int_point(int_point),
            Point::Rational(rational) => {
                let float = rational.to_float();
                Self {
                    x: float.x.round() as i64,
                    y: float.y.round() as i64,
                }
            }
        }
    }
}

/// The snapshot's axis-aligned bound ([`epic_geometry::IntBox`] as
/// plain i64 fields — the wire shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BoxPrimitive {
    /// The lower-left x (DBU).
    pub ll_x: i64,
    /// The lower-left y (DBU).
    pub ll_y: i64,
    /// The upper-right x (DBU).
    pub ur_x: i64,
    /// The upper-right y (DBU).
    pub ur_y: i64,
}

impl BoxPrimitive {
    fn from_int_box(r#box: &epic_geometry::int_box::IntBox) -> Self {
        Self {
            ll_x: i64::from(r#box.ll.x),
            ll_y: i64::from(r#box.ll.y),
            ur_x: i64::from(r#box.ur.x),
            ur_y: i64::from(r#box.ur.y),
        }
    }
}

/// One routed trace ([`Board::trace_polyline`] corner chain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TracePrimitive {
    /// The stored corner polyline, corner order preserved.
    pub polyline_points: Vec<PointPrimitive>,
    /// `Trace.get_layer()` — the 0-based layer (quality Q1: every
    /// other primitive carries placement — `ViaPrimitive.layers`,
    /// `PadPrimitive.layer`, `AreaPrimitive.layer` — and T4's
    /// `project()` needs layer-filtered views; the polyline geometry
    /// is pure 2D, so this is genuinely absent from the wire without
    /// the field).
    pub layer: i32,
    /// `Trace.get_half_width()` (DBU).
    pub half_width: i32,
    /// The carrying net (see [`BoardSnapshot`]'s net-field note).
    pub net: i32,
}

/// One via: the center plus the padstack's per-copper-layer shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViaPrimitive {
    /// `DrillItem.center` (DBU).
    pub center: PointPrimitive,
    /// The per-layer outline polygons, PARALLEL to [`Self::layers`]
    /// (one entry per copper-bearing padstack layer — a copper-less
    /// layer, Java's null `getShape`, contributes NO entry).
    pub drill_shapes: Vec<Vec<PointPrimitive>>,
    /// The copper layers (0-based), in padstack index order.
    pub layers: Vec<i32>,
    /// The carrying net (see [`BoardSnapshot`]'s net-field note).
    pub net: i32,
}

/// One component pin: the front-layer pad outline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PadPrimitive {
    /// The padstack's first-layer shape outline (DBU).
    pub outline_points: Vec<PointPrimitive>,
    /// The first padstack layer (`item_shape_layer_read(id, 0)` —
    /// Java `DrillItem.firstLayer()`); a back-side pin still reports
    /// its padstack's own first layer (the span is mirrored per the
    /// `drill_tile_shape_count` docs — the snapshot pins the geometry
    /// face, not a placement opinion).
    pub layer: i32,
    /// The carrying net (see [`BoardSnapshot`]'s net-field note).
    pub net: i32,
}

/// One keepout or conduction area.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AreaPrimitive {
    /// The TRANSFORMED border outline (DBU) — keepouts go through
    /// the T49 absolute-area chain, conduction areas ship the stored
    /// area verbatim (identity placement, the `conduction_area`
    /// face's docs).
    pub outline_points: Vec<PointPrimitive>,
    /// The stored single layer (`Board::area_layer`).
    pub layer: i32,
    /// `"conduction"` or the obstacle kind's DSN spelling
    /// (`keepout` / `via_keepout` / `place_keepout`).
    pub kind: String,
}

/// A net-table row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetInfo {
    /// The 1-based net number (the `Nets` table position).
    pub id: i32,
    /// Java `Net.name`.
    pub name: String,
}

/// One ratsnest airline on the snapshot wire (world DBU; the
/// projection of `epic_drc::incompletes::AirLineSegment` — the
/// Kruskal-ACCEPTED airlines, NOT the full Delaunay edge set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AirLinePrimitive {
    /// The from ratsnest corner (DBU).
    pub from: PointPrimitive,
    /// The to ratsnest corner (DBU).
    pub to: PointPrimitive,
    /// The 1-based net number.
    pub net: i32,
}

/// Which boundary a violation marker was computed at (the T5
/// charter's parse-time vs post-route tag; DNR-19: the two faces
/// exist because the marker walk is boundary-dependent — the LOAD
/// seed can differ from the post-route state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerPhase {
    /// The load boundary (the pre-route seed — the board state the
    /// parse produced).
    Parse,
    /// The post-route boundary (the live board after a route run).
    PostRoute,
}

/// One DRC clearance-violation marker. NO center point ships from
/// the DRC walk itself (the `DepthRow` carries the item-id pair +
/// layer + both clearances only) — the center derives here, from the
/// two items' geometry (see [`item_center`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ViolationMarker {
    /// The marker center (DBU): the midpoint of the two violated
    /// items' bbox centers (each bbox center at integer floor
    /// division; the midpoint likewise — deterministic by
    /// construction, documented approximation of "between the two
    /// violating shapes").
    pub center: PointPrimitive,
    /// `expected_clearance - actual_clearance`, BOARD UNITS (DBU) —
    /// how much closer the pair sits than the matrix demands (>= 0
    /// for a genuine violation; a 0 means the shapes already
    /// overlap).
    pub depth: f64,
    /// The two items' `BoardItemType` discriminants, the Java enum's
    /// own declaration order (`items/mod.rs:51-72` — Trace=0, Pin=1,
    /// Via=2, ObstacleArea=3, ViaObstacleArea=4, ConductionArea=5,
    /// ComponentObstacleArea=6, BoardOutline=7, ComponentOutline=8,
    /// Other=9; pair order follows the DepthRow's canonical
    /// `(min id, max id)` order). Consumers distinguish, say, a
    /// trace-via violation from a pad-outline one without a board
    /// lookup.
    pub pair_kind: (u8, u8),
    /// The boundary the walk ran at ([`MarkerPhase`]).
    pub phase: MarkerPhase,
}

/// One congested cell of the heatmap (a `(cell, signal layer)` pair
/// with overflow > 0 — the recorded subset; clear cells ship
/// nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CongestionCell {
    /// The cell's column (x index).
    pub ix: u64,
    /// The cell's row (y index).
    pub iy: u64,
    /// The signal-layer ordinal (the map's own indexing).
    pub signal_layer: u64,
    /// `max(0, occupancy - capacity)` with NO net excluded (the raw
    /// read — the boundary projection, not a routing-net read).
    pub overflow: i64,
    /// The signal layer's cell capacity (tracks per cell; >= 1 by
    /// the map's build clamp — the render ramp's ratio denominator
    /// guard uses `max(1, capacity)` regardless).
    pub capacity: i64,
}

/// The congestion heatmap projection ([`BoardSnapshot::overlays`]'s
/// `congestion` slot): a RECOMPUTED `CongestionMap::build` over the
/// boundary board state — the same pure builder the global stage
/// uses, NOT the mid-run artifact (no live map survives a planning
/// face; the instance is a local in `GlobalPlan::build`). Grid
/// geometry: cell `(ix, iy)` spans world
/// `x ∈ [origin.x + ix * cell_size, origin.x + (ix + 1) * cell_size)`
/// (same per axis y) — origin = the board bbox lower-left, cell_size
/// in DBU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CongestionHeatmap {
    /// The square cell side (DBU).
    pub cell_size: i64,
    /// The grid origin (the board bbox lower-left).
    pub origin: PointPrimitive,
    /// `(nx, ny)` grid dimensions.
    pub dims: (u64, u64),
    /// The congested cells, in signal-layer-ascending then row-major
    /// `(iy, ix)` order — ONLY cells with `overflow > 0` (clear
    /// cells ship nothing; the wire stays small on quiet boards).
    pub cells: Vec<CongestionCell>,
}

/// One net's tuning target band + the net's actual routed length
/// (world DBU; `min == 0.0` = no lower bound, `max == 0.0` = no
/// upper bound — the `has_length_constraints`/`net_class_length_bounds`
/// convention: 0.0 is the absence sentinel).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NetTuningInfo {
    /// The 1-based net number.
    pub net: i32,
    /// The class-resolved minimum trace length (DBU; 0.0 = absent).
    pub min: f64,
    /// The class-resolved maximum trace length (DBU; 0.0 = absent).
    pub max: f64,
    /// `Board::net_trace_length(net)` at the snapshot boundary.
    pub actual: f64,
}

/// The overlay DATA faces (M9-T5): the four §7 views as plain wire
/// data. The PURE snapshot builder fills this EMPTY by construction
/// (the purity law: `board_snapshot(&Board)` cannot run the `&mut`
/// DRC faces, so pass-granular tee snapshots carry the empty default
/// — pinned; the honest cadence). The ATTACH step
/// (`Session::snapshot_with_overlays`) fills all four slots at a
/// session boundary.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct OverlayData {
    /// The ratsnest airlines (empty until the attach step).
    pub airlines: Vec<AirLinePrimitive>,
    /// The DRC violation markers (empty until the attach step).
    pub violation_markers: Vec<ViolationMarker>,
    /// The congestion heatmap — `Some` IFF the session's last-resolved
    /// route settings engaged the `congestion_global` family (the
    /// probe's reachable ON path: the `SessionLayer`/`CliLayer`
    /// `router.congestion_global` flag) AND the recomputed map is
    /// non-empty (`project_heatmap` maps an empty map to `None`),
    /// `None` otherwise.
    pub congestion: Option<CongestionHeatmap>,
    /// The tuning target bands — `Some` IFF
    /// `BoardRules::has_length_constraints()` (one
    /// [`NetTuningInfo`] per net with a non-zero resolved bound,
    /// nets ascending), `None` otherwise.
    pub tuning: Option<Vec<NetTuningInfo>>,
}

/// The render projection of a board at one revision. Net fields are
/// the FIRST net of the item (`ItemEntry.nets[0]`, 0 when the item
/// carries none) — router copper is single-net by construction; a
/// multi-net item is a parse shape the GUI does not consume
/// per-net.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardSnapshot {
    /// Traces in `iter_ascending` (creation) order.
    pub traces: Vec<TracePrimitive>,
    /// Vias in `iter_ascending` (creation) order.
    pub vias: Vec<ViaPrimitive>,
    /// Pins in `iter_ascending` (creation) order.
    pub pads: Vec<PadPrimitive>,
    /// Keepout + conduction areas in `iter_ascending` (creation)
    /// order. Component outlines are NOT areas (design plan: areas
    /// are conduction/keepout) and ship nowhere.
    pub areas: Vec<AreaPrimitive>,
    /// The board outline's shapes, concatenated in shape order (one
    /// outline item in practice; multiple would concatenate in
    /// ascending-id order).
    pub outline: Vec<PointPrimitive>,
    /// The net table in number order (1..=max).
    pub nets: Vec<NetInfo>,
    /// `Board::bounding_box` — the parse always carries it (the
    /// face's docs); a `None` (pre-`create_board`, unreachable from a
    /// parsed session) maps to the all-zero box, documented.
    pub bounds: BoxPrimitive,
    /// `Board::revision` — the dirty tick the event stream dedups on.
    pub revision: u64,
    /// The overlay data faces (M9-T5). The PURE builder fills this
    /// EMPTY (the purity law — see [`OverlayData`]); `serde(default)`
    /// keeps the wire additive (a stream written before T5 deserializes
    /// into the empty default).
    #[serde(default)]
    pub overlays: OverlayData,
}

/// The shape-to-polygon face (NO new geometry): the directed border
/// lines' start corners, in shape order. Tiles and polygons walk
/// their `border_line` faces; a circle has no corners (`corner_approx_arr`
/// is empty by port docs) and ships the four corners of its own
/// `bounding_box` — the deterministic rectangle APPROXIMATION (every
/// round via/pad ships as a 4-corner rectangle; quality Q6).
fn shape_outline_points(shape: &BoardShape) -> Vec<PointPrimitive> {
    let border_of = |border_line_count: usize,
                     border_line: &dyn Fn(i32) -> epic_geometry::line::Line|
     -> Vec<PointPrimitive> {
        (0..border_line_count.min(usize::try_from(i32::MAX).unwrap_or(i32::MAX as usize)))
            .map(|no| PointPrimitive::from_point(&border_line(no as i32).a))
            .collect()
    };
    match shape {
        BoardShape::Tile(tile) => {
            let count = tile.border_line_count();
            border_of(count, &|no| tile.border_line(no))
        }
        BoardShape::PolygonShape(polygon) => {
            let count = polygon.border_line_count();
            border_of(count, &|no| polygon.border_line(no))
        }
        BoardShape::Circle(circle) => {
            let r#box = circle.bounding_box();
            vec![
                PointPrimitive::from_int_point(&r#box.ll),
                PointPrimitive {
                    x: i64::from(r#box.ur.x),
                    y: i64::from(r#box.ll.y),
                },
                PointPrimitive::from_int_point(&r#box.ur),
                PointPrimitive {
                    x: i64::from(r#box.ll.x),
                    y: i64::from(r#box.ur.y),
                },
            ]
        }
    }
}

/// The area outline: the border shape's polygon (the `(window …)`
/// holes are NOT chartered for the M9 view set — the border is the
/// render outline).
fn area_outline_points(area: &Area) -> Vec<PointPrimitive> {
    shape_outline_points(&area.border)
}

/// The obstacle kind's DSN spelling (the `(keepout …)` /
/// `(via_keepout …)` / `(place_keepout …)` keyword family).
fn obstacle_kind_name(kind: ObstacleKind) -> &'static str {
    match kind {
        ObstacleKind::ObstacleArea => "keepout",
        ObstacleKind::ViaObstacleArea => "via_keepout",
        ObstacleKind::ComponentObstacleArea => "place_keepout",
    }
}

/// The first net of an item (0 = carries no net — see
/// [`BoardSnapshot`]'s net-field note).
fn first_net(nets: &[i32]) -> i32 {
    nets.first().copied().unwrap_or(0)
}

/// The `BoardItemType` discriminant mapping for
/// [`ViolationMarker::pair_kind`]: the Java `BoardItemType` enum's own
/// declaration order (the port mirrors it exactly — `items/mod.rs:50`
/// docs "declaration order mirrored exactly"). Documented here so the
/// wire's u8s are readable without a source lookup.
///
/// (DNR-19: the mapping IS the derivation — the values are the
/// declaration ordinals of the mirrored Java enum, not a new
/// convention.)
fn kind_discriminant(kind: BoardItemType) -> u8 {
    match kind {
        BoardItemType::Trace => 0,
        BoardItemType::Pin => 1,
        BoardItemType::Via => 2,
        BoardItemType::ObstacleArea => 3,
        BoardItemType::ViaObstacleArea => 4,
        BoardItemType::ConductionArea => 5,
        BoardItemType::ComponentObstacleArea => 6,
        BoardItemType::BoardOutline => 7,
        BoardItemType::ComponentOutline => 8,
        BoardItemType::Other => 9,
    }
}

/// The bbox center of ONE item's render geometry (the marker-center
/// face): the same faces the snapshot builder reads, bbox-reduced.
/// `None` for kinds with no render geometry (component outlines,
/// `Other`) — the caller skips the marker (documented: a violation
/// pair whose member carries no render geometry produces NO marker;
/// unreachable on parsed boards, where every DRC-relevant kind has
/// geometry).
fn item_center(board: &Board, id: ItemId) -> Option<PointPrimitive> {
    let points: Vec<PointPrimitive> = match &board.get(id)?.data {
        ItemData::Trace { .. } => {
            let polyline = board.trace_polyline(id)?;
            polyline
                .corners()
                .iter()
                .map(PointPrimitive::from_point)
                .collect()
        }
        ItemData::Pin { .. } | ItemData::Via { .. } => {
            let shape_count = board.drill_tile_shape_count(id)?;
            let mut points = Vec::new();
            for index in 0..shape_count {
                if let Some(shape) = board.drill_shape(id, index) {
                    points.extend(shape_outline_points(&shape));
                }
            }
            points
        }
        ItemData::ObstacleArea { .. } => area_outline_points(&board.obstacle_area(id)?),
        ItemData::ConductionArea { .. } => area_outline_points(&board.conduction_area(id)?),
        ItemData::BoardOutline { .. } => {
            let shapes = board.outline_shapes(id)?;
            let mut points = Vec::new();
            for shape in shapes {
                points.extend(shape_outline_points(shape));
            }
            points
        }
        // No render geometry — the caller skips (the fn doc).
        ItemData::ComponentOutline { .. } | ItemData::Other => return None,
    };
    let first = points.first()?;
    let (mut ll_x, mut ll_y, mut ur_x, mut ur_y) = (first.x, first.y, first.x, first.y);
    for p in &points[1..] {
        ll_x = ll_x.min(p.x);
        ll_y = ll_y.min(p.y);
        ur_x = ur_x.max(p.x);
        ur_y = ur_y.max(p.y);
    }
    Some(PointPrimitive {
        x: (ll_x + ur_x) / 2,
        y: (ll_y + ur_y) / 2,
    })
}

/// Builds the markers from the depth rows (the T5 charter's
/// documented choices): center = midpoint of the two items' bbox
/// centers (integer floor division both steps), depth =
/// `expected - actual` in board units, pair_kind = the two kinds'
/// declaration-order discriminants (pair order follows the row's
/// canonical `(min id, max id)` order — the CENTER derives from
/// a→b in that same order, and a midpoint is order-free anyway).
/// `pub(crate)`: the session attach step is the caller.
pub(crate) fn violation_markers_from_rows(
    rows: &[DepthRow],
    board: &Board,
    phase: MarkerPhase,
) -> Vec<ViolationMarker> {
    rows.iter()
        .filter_map(|row| {
            let id_a = u32::try_from(row.a).ok().map(ItemId::new)?;
            let id_b = u32::try_from(row.b).ok().map(ItemId::new)?;
            let center_a = item_center(board, id_a)?;
            let center_b = item_center(board, id_b)?;
            let kind = |id: ItemId| {
                board
                    .get(id)
                    .map(|entry| kind_discriminant(entry.board_item_type()))
                    .unwrap_or(9)
            };
            let (kind_a, kind_b) = (kind(id_a), kind(id_b));
            Some(ViolationMarker {
                center: PointPrimitive {
                    x: (center_a.x.saturating_add(center_b.x)) / 2,
                    y: (center_a.y.saturating_add(center_b.y)) / 2,
                },
                depth: row.expected_clearance - row.actual_clearance,
                pair_kind: (kind_a, kind_b),
                phase,
            })
        })
        .collect()
}

/// Builds the snapshot of `board` at its current revision (the
/// renders-never-mutates law: `&Board` ONLY).
#[must_use]
pub fn board_snapshot(board: &Board) -> BoardSnapshot {
    let mut traces = Vec::new();
    let mut vias = Vec::new();
    let mut pads = Vec::new();
    let mut areas = Vec::new();
    let mut outline = Vec::new();
    // The ONE ascending-id walk (creation order — the snapshot's
    // deterministic order, board.rs:444's documented face).
    for entry in board.iter_ascending() {
        match &entry.data {
            ItemData::Trace { .. } => {
                let Some(polyline) = board.trace_polyline(entry.id) else {
                    continue;
                };
                let Some(layer) = board.trace_layer(entry.id) else {
                    continue;
                };
                let Some(half_width) = board.trace_half_width(entry.id) else {
                    continue;
                };
                traces.push(TracePrimitive {
                    polyline_points: polyline
                        .corners()
                        .iter()
                        .map(PointPrimitive::from_point)
                        .collect(),
                    layer,
                    half_width,
                    net: first_net(&entry.nets),
                });
            }
            ItemData::Via { center, .. } => {
                // The padstack span (DrillItem.java:202-208) via the
                // `&self` faces: layer = `item_shape_layer_read(id, i)`
                // (the span clamp), shape = `drill_shape(id, i)`. A
                // copper-less layer (Java's null shape) contributes no
                // entry — layers/shapes stay parallel.
                let Some(shape_count) = board.drill_tile_shape_count(entry.id) else {
                    continue;
                };
                let mut layers = Vec::new();
                let mut drill_shapes = Vec::new();
                for index in 0..shape_count {
                    let Some(shape) = board.drill_shape(entry.id, index) else {
                        continue;
                    };
                    let Some(layer) = board.item_shape_layer_read(entry.id, index) else {
                        continue;
                    };
                    layers.push(layer);
                    drill_shapes.push(shape_outline_points(&shape));
                }
                vias.push(ViaPrimitive {
                    center: PointPrimitive::from_int_point(center),
                    drill_shapes,
                    layers,
                    net: first_net(&entry.nets),
                });
            }
            ItemData::Pin { .. } => {
                let Some(shape) = board.drill_shape(entry.id, 0) else {
                    continue;
                };
                let Some(layer) = board.item_shape_layer_read(entry.id, 0) else {
                    continue;
                };
                pads.push(PadPrimitive {
                    outline_points: shape_outline_points(&shape),
                    layer,
                    net: first_net(&entry.nets),
                });
            }
            ItemData::ObstacleArea { kind, .. } => {
                let Some(area) = board.obstacle_area(entry.id) else {
                    continue;
                };
                let Some(layer) = board.area_layer(entry.id) else {
                    continue;
                };
                areas.push(AreaPrimitive {
                    outline_points: area_outline_points(&area),
                    layer,
                    kind: obstacle_kind_name(*kind).to_string(),
                });
            }
            ItemData::ConductionArea { .. } => {
                let Some(area) = board.conduction_area(entry.id) else {
                    continue;
                };
                let Some(layer) = board.area_layer(entry.id) else {
                    continue;
                };
                areas.push(AreaPrimitive {
                    outline_points: area_outline_points(&area),
                    layer,
                    kind: "conduction".to_string(),
                });
            }
            ItemData::BoardOutline { .. } => {
                let Some(shapes) = board.outline_shapes(entry.id) else {
                    continue;
                };
                for shape in shapes {
                    outline.extend(shape_outline_points(shape));
                }
            }
            // Component outlines are not conduction/keepout areas
            // (the plan's area kind list); `Other` carries no render
            // geometry.
            ItemData::ComponentOutline { .. } | ItemData::Other => {}
        }
    }
    let nets = board
        .rules()
        .nets
        .iter()
        .map(|(net_number, net)| NetInfo {
            id: net_number,
            name: net.name.clone(),
        })
        .collect();
    let bounds = board
        .bounding_box()
        .as_ref()
        .map(BoxPrimitive::from_int_box)
        // Unreachable from a parsed session (the bounding_box face's
        // docs); the all-zero box keeps the wire shape total.
        .unwrap_or(BoxPrimitive {
            ll_x: 0,
            ll_y: 0,
            ur_x: 0,
            ur_y: 0,
        });
    BoardSnapshot {
        traces,
        vias,
        pads,
        areas,
        outline,
        nets,
        bounds,
        revision: board.revision(),
        // THE PURITY LAW (module docs): the pure builder fills the
        // overlay slots EMPTY — the `&Board` handle cannot run the
        // `&mut` DRC faces; the attach step is Session's.
        overlays: OverlayData::default(),
    }
}
