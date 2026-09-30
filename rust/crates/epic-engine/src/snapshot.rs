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
use epic_board::items::{Area, BoardShape, ItemData, ObstacleKind};
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

/// The render projection of a board at one revision. Net fields are
/// the FIRST net of the item (`ItemEntry.nets[0]`, 0 when the item
/// carries none) — router copper is single-net by construction; a
/// multi-net item is a parse shape the GUI does not consume
/// per-net.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    }
}
