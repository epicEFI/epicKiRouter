//! The M9-T4 projection: [`project`] — a PURE function of
//! `(snapshot, view, viewport)`: same inputs, byte-identical
//! [`RenderList`], always. No HashMap anywhere (BTreeMap/BTreeSet
//! only); iterate layers ascending, items in snapshot order, overlays
//! last.
//!
//! # The primitive-kind -> op-sequence mapping (the doc table)
//!
//! | primitive kind | visibility face | op sequence (fixed order) |
//! |---|---|---|
//! | board outline | layer-less — always renders | `SetColor(outline)` → `FillPolygon(points)` |
//! | conduction/keepout area | its layer visible | `SetColor(pad)` → `FillPolygon(points)` |
//! | pad | its layer visible | `SetColor(pad)` → `FillPolygon(points)` |
//! | via | at least one padstack shape's layer visible | `SetColor(via)` → `FillPolygon(pad rectangle)` per visible shape (padstack order) → `Circle{center, radius}` LAST (the drill on top of the pad) |
//! | trace | its layer visible | `SetColor(trace_color(layer))` → `SetWidth(half-width x scale)` → `MoveTo`/`LineTo` walk |
//!
//! The via pad rectangle: the snapshot's shape wire ships every round
//! pad/via shape as a 4-corner RECTANGLE approximation (AM3/T3
//! quality Q6 — `snapshot.rs`'s own doc); T4 renders them as the
//! rectangles they are — a round pad never renders as a circle
//! primitive. The via's trailing `Circle` is the DRILL: its radius
//! derives from the rendered pad rectangles' union bbox —
//! `min(w, h) / 4` DBU (a square pad of side `min(w, h)` approximates
//! a pad disc of that diameter, pad radius `min/2`; the drill is
//! documented at half the pad radius), floored at 1 px so a drill is
//! never sub-pixel.
//!
//! `SetWidth` carries the trace's DBU HALF-width x the transform
//! scale as an f32 (the charter letter; the consumer may double it
//! for full copper width). Rounding face: the scale is f32
//! (`ScreenTransform::scale_f32`), the product is one f32 multiply,
//! NO px rounding — a sub-pixel width stays sub-pixel; the fixed test
//! transform is 1:1, where the product is an exact integer-valued
//! f32.
//!
//! # Culling (the `culled` count)
//!
//! Per-primitive bbox-overlap precheck against the viewport (a WORLD
//! `IntBox`), INCLUSIVE: a primitive exactly ON the boundary is KEPT
//! (the DNR-16 face, pinned both directions in `render_goldens.rs` —
//! 1 DBU inside renders, 1 DBU outside is culled, ON the edge
//! renders). Primitive bboxes: trace = polyline corner bbox expanded
//! by the half-width (saturating); via = union bbox of the VISIBLE
//! shapes; pad/area/outline = outline-point bbox. `culled` counts
//! ONLY primitives that passed the visibility filter and were then
//! rejected by the viewport precheck — a visibility-skipped primitive
//! (hidden layer, copper-less via) never reaches the precheck and is
//! not counted.
//!
//! # Determinism law (pinned by the goldens)
//!
//! Layers ascending: the single-layer categories (areas, pads,
//! traces) iterate via a `BTreeMap<i32, _>` so a layer's items emit
//! in snapshot order and layers emit in ascending order. The via pass
//! keeps the wire's own order (padstack index order per via,
//! snapshot order across vias) — documented. Emission order of
//! categories: outline, areas, pads, vias, traces, overlays (traces
//! on top — the viewer's copper-over-pads convention; the order is a
//! fixed documented choice, byte-pinned by the goldens). Overlays
//! append at the single T5 extension site, after all geometry.

use epic_engine::snapshot::{
    AreaPrimitive, BoardSnapshot, PadPrimitive, PointPrimitive, TracePrimitive,
};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use serde::Serialize;
use serde::ser::{SerializeStruct, SerializeStructVariant, Serializer};
use std::collections::BTreeMap;

use crate::view::{ScreenTransform, ViewModel};

/// One render operation (the charter's fixed op surface). Screen
/// coordinates are px (`IntPoint`, i32); the `Circle` radius carries
/// its OWN face: SCREEN px as i64 (i32 px would overflow at extreme
/// zooms; i64 cannot).
///
/// `Serialize` is a MANUAL impl: `epic_geometry::IntPoint` carries no
/// `Serialize` (the T3 erratum — the snapshot wire uses
/// `PointPrimitive`), so the golden format serializes a point as
/// `{"x": i32, "y": i32}` — the externally-tagged default shape with
/// Rust variant spelling (the golden-format convention, documented in
/// each golden header).
#[derive(Debug, Clone, PartialEq)]
pub enum RenderOp {
    /// Set the current color (u8 RGBA).
    SetColor([u8; 4]),
    /// Set the stroke width in px (f32; the rounding face is the
    /// module doc's `SetWidth` note).
    SetWidth(f32),
    /// Begin a polyline walk (screen px).
    MoveTo(IntPoint),
    /// Continue a polyline walk (screen px).
    LineTo(IntPoint),
    /// Fill a polygon (screen px; corner order preserved from the
    /// wire).
    FillPolygon(Vec<IntPoint>),
    /// The via drill (screen px center; i64 px radius — its own
    /// face). The pad copper renders as `FillPolygon` rectangles
    /// BEFORE it (see the mapping table).
    Circle {
        /// The drill center (screen px).
        center: IntPoint,
        /// The drill radius (screen px, i64; >= 1).
        radius: i64,
    },
}

/// The wire shape of a point in the golden format (`{"x","y"}`) — the
/// `IntPoint` lacks `Serialize`, so the manual `RenderOp` impl wraps.
struct PointWire {
    x: i32,
    y: i32,
}

impl From<&IntPoint> for PointWire {
    fn from(p: &IntPoint) -> Self {
        Self { x: p.x, y: p.y }
    }
}

impl Serialize for PointWire {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("PointWire", 2)?;
        state.serialize_field("x", &self.x)?;
        state.serialize_field("y", &self.y)?;
        state.end()
    }
}

/// Variant indices for the manual `Serialize` impl (stable wire
/// order; part of the golden-format convention).
const SET_COLOR: u32 = 0;
const SET_WIDTH: u32 = 1;
const MOVE_TO: u32 = 2;
const LINE_TO: u32 = 3;
const FILL_POLYGON: u32 = 4;
const CIRCLE: u32 = 5;

impl Serialize for RenderOp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RenderOp::SetColor(color) => {
                serializer.serialize_newtype_variant("RenderOp", SET_COLOR, "SetColor", color)
            }
            RenderOp::SetWidth(w) => {
                serializer.serialize_newtype_variant("RenderOp", SET_WIDTH, "SetWidth", w)
            }
            RenderOp::MoveTo(p) => serializer.serialize_newtype_variant(
                "RenderOp",
                MOVE_TO,
                "MoveTo",
                &PointWire::from(p),
            ),
            RenderOp::LineTo(p) => serializer.serialize_newtype_variant(
                "RenderOp",
                LINE_TO,
                "LineTo",
                &PointWire::from(p),
            ),
            RenderOp::FillPolygon(points) => {
                let wire: Vec<PointWire> = points.iter().map(PointWire::from).collect();
                serializer.serialize_newtype_variant("RenderOp", FILL_POLYGON, "FillPolygon", &wire)
            }
            RenderOp::Circle { center, radius } => {
                let mut state =
                    serializer.serialize_struct_variant("RenderOp", CIRCLE, "Circle", 2)?;
                state.serialize_field("center", &PointWire::from(center))?;
                state.serialize_field("radius", radius)?;
                state.end()
            }
        }
    }
}

/// The projection result: the op list (deterministic order — the
/// module doc's determinism law) + the cull count (viewport rejects
/// only, the module doc's culling note).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderList {
    /// The ops in emission order.
    pub ops: Vec<RenderOp>,
    /// Primitives that passed the visibility filter and were rejected
    /// by the viewport precheck (inclusive boundary — kept ON the
    /// edge).
    pub culled: usize,
}

/// A world-space i64 bbox (the cull precheck's primitive face).
struct WorldBBox {
    ll_x: i64,
    ll_y: i64,
    ur_x: i64,
    ur_y: i64,
}

impl WorldBBox {
    /// Inclusive overlap against the viewport (widened to i64):
    /// touching counts (the DNR-16 exact-boundary face).
    fn overlaps_viewport(&self, viewport: &IntBox) -> bool {
        self.ll_x <= i64::from(viewport.ur.x)
            && self.ur_x >= i64::from(viewport.ll.x)
            && self.ll_y <= i64::from(viewport.ur.y)
            && self.ur_y >= i64::from(viewport.ll.y)
    }
}

/// The inclusive bbox of `points` (`None` when empty — nothing
/// renderable).
fn points_bbox(points: &[PointPrimitive]) -> Option<WorldBBox> {
    let first = points.first()?;
    let mut bbox = WorldBBox {
        ll_x: first.x,
        ll_y: first.y,
        ur_x: first.x,
        ur_y: first.y,
    };
    for p in &points[1..] {
        bbox.ll_x = bbox.ll_x.min(p.x);
        bbox.ll_y = bbox.ll_y.min(p.y);
        bbox.ur_x = bbox.ur_x.max(p.x);
        bbox.ur_y = bbox.ur_y.max(p.y);
    }
    Some(bbox)
}

/// Saturating expansion of `bbox` by `d` on every side (the trace
/// half-width face; saturation keeps the precheck total).
fn expand(bbox: &WorldBBox, d: i64) -> WorldBBox {
    WorldBBox {
        ll_x: bbox.ll_x.saturating_sub(d),
        ll_y: bbox.ll_y.saturating_sub(d),
        ur_x: bbox.ur_x.saturating_add(d),
        ur_y: bbox.ur_y.saturating_add(d),
    }
}

/// Transforms a world point slice to screen px (wire order
/// preserved).
fn screen_points(points: &[PointPrimitive], transform: &ScreenTransform) -> Vec<IntPoint> {
    points
        .iter()
        .map(|p| transform.world_to_screen(*p))
        .collect()
}

/// Pushes one polygon primitive (area or pad): cull precheck, then
/// `SetColor` + `FillPolygon`.
fn push_polygon(
    ops: &mut Vec<RenderOp>,
    culled: &mut usize,
    color: [u8; 4],
    points: &[PointPrimitive],
    transform: &ScreenTransform,
    viewport: &IntBox,
) {
    let Some(bbox) = points_bbox(points) else {
        return;
    };
    if !bbox.overlaps_viewport(viewport) {
        *culled += 1;
        return;
    }
    ops.push(RenderOp::SetColor(color));
    ops.push(RenderOp::FillPolygon(screen_points(points, transform)));
}

/// The drill radius in screen px from the rendered shapes' union
/// bbox: `max(1, min(w, h) / 4 DBU x scale)` — the module doc's
/// Circle note (i128 intermediates; the i64 px narrowing saturates).
fn drill_radius_px(bbox: &WorldBBox, transform: &ScreenTransform) -> i64 {
    let w = bbox.ur_x.saturating_sub(bbox.ll_x);
    let h = bbox.ur_y.saturating_sub(bbox.ll_y);
    let r_dbu = w.min(h) / 4;
    let scaled =
        i128::from(r_dbu) * i128::from(transform.zoom_num) / i128::from(transform.zoom_den);
    let clamped = scaled.max(1);
    if clamped > i128::from(i64::MAX) {
        i64::MAX
    } else {
        clamped as i64
    }
}

/// The projection: snapshot + view + viewport -> the deterministic
/// `RenderList` (pure — the module doc's determinism law).
#[must_use]
pub fn project(snapshot: &BoardSnapshot, view: &ViewModel, viewport: IntBox) -> RenderList {
    let mut ops: Vec<RenderOp> = Vec::new();
    let mut culled: usize = 0;
    let transform = &view.transform;
    let scale = transform.scale_f32();
    let colors = &view.colors;
    let visible = &view.visible_layers;

    // --- 1. The board outline (layer-less — always rendered; culled
    //        as ONE primitive on its point bbox).
    if let Some(bbox) = points_bbox(&snapshot.outline) {
        if bbox.overlaps_viewport(&viewport) {
            ops.push(RenderOp::SetColor(colors.outline));
            ops.push(RenderOp::FillPolygon(screen_points(
                &snapshot.outline,
                transform,
            )));
        } else {
            culled += 1;
        }
    }

    // --- 2. Areas: layers ascending (BTreeMap), snapshot order
    //        within a layer.
    let mut areas_by_layer: BTreeMap<i32, Vec<&AreaPrimitive>> = BTreeMap::new();
    for area in &snapshot.areas {
        areas_by_layer.entry(area.layer).or_default().push(area);
    }
    for (layer, layer_areas) in &areas_by_layer {
        if !visible.contains(layer) {
            continue;
        }
        for area in layer_areas {
            push_polygon(
                &mut ops,
                &mut culled,
                colors.pad,
                &area.outline_points,
                transform,
                &viewport,
            );
        }
    }

    // --- 3. Pads: layers ascending, snapshot order within a layer.
    let mut pads_by_layer: BTreeMap<i32, Vec<&PadPrimitive>> = BTreeMap::new();
    for pad in &snapshot.pads {
        pads_by_layer.entry(pad.layer).or_default().push(pad);
    }
    for (layer, layer_pads) in &pads_by_layer {
        if !visible.contains(layer) {
            continue;
        }
        for pad in layer_pads {
            push_polygon(
                &mut ops,
                &mut culled,
                colors.pad,
                &pad.outline_points,
                transform,
                &viewport,
            );
        }
    }

    // --- 4. Vias: the wire's own order (padstack index order per
    //        via, snapshot order across vias — the module doc's
    //        mapping note). Visibility FIRST (per-shape; a via with
    //        no visible shape contributes nothing and is not
    //        counted), then the viewport precheck on the visible
    //        shapes' union bbox.
    for via in &snapshot.vias {
        let visible_shapes: Vec<&Vec<PointPrimitive>> = via
            .layers
            .iter()
            .zip(via.drill_shapes.iter())
            .filter(|(layer, _)| visible.contains(layer))
            .map(|(_, shape)| shape)
            .collect();
        if visible_shapes.is_empty() {
            continue;
        }
        let Some(union_bbox) = visible_shapes
            .iter()
            .filter_map(|shape| points_bbox(shape))
            .reduce(|a, b| WorldBBox {
                ll_x: a.ll_x.min(b.ll_x),
                ll_y: a.ll_y.min(b.ll_y),
                ur_x: a.ur_x.max(b.ur_x),
                ur_y: a.ur_y.max(b.ur_y),
            })
        else {
            continue;
        };
        if !union_bbox.overlaps_viewport(&viewport) {
            culled += 1;
            continue;
        }
        ops.push(RenderOp::SetColor(colors.via));
        for shape in &visible_shapes {
            ops.push(RenderOp::FillPolygon(screen_points(shape, transform)));
        }
        ops.push(RenderOp::Circle {
            center: transform.world_to_screen(via.center),
            radius: drill_radius_px(&union_bbox, transform),
        });
    }

    // --- 5. Traces: layers ascending, snapshot order within a
    //        layer.
    let mut traces_by_layer: BTreeMap<i32, Vec<&TracePrimitive>> = BTreeMap::new();
    for trace in &snapshot.traces {
        traces_by_layer.entry(trace.layer).or_default().push(trace);
    }
    for (layer, layer_traces) in &traces_by_layer {
        if !visible.contains(layer) {
            continue;
        }
        for trace in layer_traces {
            let Some(corner_bbox) = points_bbox(&trace.polyline_points) else {
                continue;
            };
            let bbox = expand(&corner_bbox, i64::from(trace.half_width));
            if !bbox.overlaps_viewport(&viewport) {
                culled += 1;
                continue;
            }
            ops.push(RenderOp::SetColor(colors.trace_color(*layer)));
            ops.push(RenderOp::SetWidth(
                i64::from(trace.half_width) as f32 * scale,
            ));
            let points = &trace.polyline_points;
            if let Some(first) = points.first() {
                ops.push(RenderOp::MoveTo(transform.world_to_screen(*first)));
                for p in &points[1..] {
                    ops.push(RenderOp::LineTo(transform.world_to_screen(*p)));
                }
            }
        }
    }

    // --- T5 EXTENSION SITE — the ONLY read of `view.overlays` in the
    // T4 projection. Overlay ops (ratsnest airlines, DRC markers,
    // congestion heatmap, tuning bands) append HERE, after all board
    // geometry (the determinism law: overlays last). T4 ships the
    // flags + this plumbing only; toggling changes nothing in T4
    // output (pinned).
    let _overlays = &view.overlays;

    RenderList { ops, culled }
}
