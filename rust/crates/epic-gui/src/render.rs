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
//! append at the single extension site, after all geometry, in the
//! FIXED family order ratsnest → drc → congestion → tuning (each
//! family in wire order — see the extension-site doc).
//!
//! # The overlay op families (M9-T5; each ON flag emits exactly one)
//!
//! | family (flag) | ops | constants (DNR-19 derivations) |
//! |---|---|---|
//! | ratsnest (`overlays.ratsnest`) | family header `SetColor(ratsnest)` + `SetWidth(1.0)` once when ≥ 1 airline survives the cull, then per airline a DASHED `MoveTo`/`LineTo` walk | dash 8 px on / 6 px off — NO Java oracle: Java draws the ratsnest SOLID at draw-width 1 (`NetIncompletesGraphics.java:34`, `int drawWidth = 1`, second instance `:70`; `:28` the incomplete color), so the dash is the charter's legibility requirement, constants documented here: 8 px on keeps a dash readable at the 1:1 test zoom (a 2-DBU half-width trace renders ≈ 4 px — dashes shorter than 8 would merge with copper), 6 px off is 3/4 of the on-length so gaps stay visibly shorter than dashes; width 1.0 px = Java's own drawWidth. Dashes walk the SCREEN segment in f64 px, endpoints rounded with f64 `round` (deterministic; a segment ≤ 8 px renders SOLID — a dash shorter than the on-length carries no information) |
//! | drc (`overlays.drc`) | per marker `SetColor(violation)` + `Circle{center, radius}` | radius = `max(4, min(4 + depth × scale, 64))` px — base 4 px: twice the airline width family (a marker must out-stand a 1-px airline at the same zoom) and ≈ the drill-radius family (`min(w,h)/4`); the `depth × scale` term grows the circle with the violation's DBU depth (`expected − actual`); the 64 px cap keeps one huge-clearance violation under ~1/10 of a 640-px viewport. The `phase` (Parse vs PostRoute) is DATA ONLY — the op set has no stroke/fill semantics for it (the charter's documented limitation), both phases render identically |
//! | congestion (`overlays.congestion`) | per congested cell `SetColor(ramp)` + `FillPolygon(cell rect)` | ramp = a 16-level quantized interpolation (level `= floor(ratio × 16).min(15)`, `t = level / 15`) between `CONGESTION_RAMP_LOW` `[0,150,0]` (Java's conduction green, `ItemColorTableModel.java:34` — the T4-recorded-not-adopted color, adopted HERE for the ramp's cool end) and `CONGESTION_RAMP_HIGH` `[255,0,255]` (Java's violation magenta, `OtherColorTableModel.java:22`); `ratio = overflow / max(1, capacity)` (the guard: the wire's capacity is >= 1 by the map's build clamp, the guard is the wire's documented face). Cells emit in wire order; the rect is the cell's world span transformed at the corners |
//! | tuning (`overlays.tuning`) | per info `SetColor(band)` + `MoveTo` + 4×`LineTo` (a closed square BRACKET outline, half-side 12 px, centered on the net's anchor) | anchor = the net's FIRST airline `from` point (wire order), else the first trace of that net's first polyline corner (wire order); a tuning entry with neither anchor contributes NOTHING (data without geometry — documented; a bracket floating at (0,0) would lie). Half-side 12 px: 1.5× the ratsnest dash-on length, keeping the bracket outside the dash pattern at the same zoom. Band colors: in-band `[0,150,0]` (the conduction green citation above) when `(min == 0 || actual >= min) && (max == 0 || actual <= max)` (the 0.0-absent convention the wire documents; NOTE: this is BAND MEMBERSHIP, not Java's `calcLengthViolation` DRC gate — that gate additionally requires incompletes for the under-min arm), out-of-band = the violation color |
//!
//! Overlay primitives participate in viewport culling like geometry:
//! an airline culls on its segment bbox, a marker on its center
//! POINT, a cell on its rect, a tuning bracket on its anchor point —
//! all INCLUSIVE (touching = kept, the DNR-16 face), all counted in
//! `culled`.

use epic_engine::snapshot::{
    AirLinePrimitive, AreaPrimitive, BoardSnapshot, CongestionHeatmap, NetTuningInfo, PadPrimitive,
    PointPrimitive, TracePrimitive, ViolationMarker,
};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use serde::Serialize;
use serde::ser::{SerializeStruct, SerializeStructVariant, Serializer}; // codespell:ignore (the serde::ser module path)
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
/// Circle note (i128 intermediates; the i64 px narrowing reuses the
/// crate's ONE saturating implementation, quality-review Q7 — the
/// negative branch is unreachable past the `max(1)`).
fn drill_radius_px(bbox: &WorldBBox, transform: &ScreenTransform) -> i64 {
    let w = bbox.ur_x.saturating_sub(bbox.ll_x);
    let h = bbox.ur_y.saturating_sub(bbox.ll_y);
    let r_dbu = w.min(h) / 4;
    let scaled =
        i128::from(r_dbu) * i128::from(transform.zoom_num) / i128::from(transform.zoom_den);
    crate::view::narrow_i128_to_i64(scaled.max(1))
}

// ---------------------------------------------------------------------------
// The M9-T5 overlay op families (constants carry their DNR-19
// derivations in the module doc's family table)
// ---------------------------------------------------------------------------

/// Ratsnest dash: px ON per dash (see the module-doc family table).
const RATSNEST_DASH_ON_PX: f64 = 8.0;
/// Ratsnest dash: px OFF between dashes (3/4 of the on-length — the
/// derivation note is the module doc's family table).
const RATSNEST_DASH_OFF_PX: f64 = 6.0;
/// Ratsnest stroke width px (Java's own drawWidth,
/// `NetIncompletesGraphics.java:34` (second instance `:70`, third `:85`).
const RATSNEST_WIDTH_PX: f32 = 1.0;
/// Violation marker radius: base px (derivation: the module doc's
/// family table).
const MARKER_RADIUS_BASE_PX: f64 = 4.0;
/// Violation marker radius: the saturating cap px (same derivation).
const MARKER_RADIUS_MAX_PX: f64 = 64.0;
/// The congestion ramp's quantization steps (16 levels — see the
/// family table).
const CONGESTION_RAMP_STEPS: u32 = 16;
/// The congestion ramp's cool end (Java's conduction green,
/// `ItemColorTableModel.java:34`).
const CONGESTION_RAMP_LOW: [u8; 4] = [0, 150, 0, 255];
/// The congestion ramp's hot end (Java's violation magenta,
/// `OtherColorTableModel.java:22`).
const CONGESTION_RAMP_HIGH: [u8; 4] = [255, 0, 255, 255];
/// The tuning bracket's half-side px (derivation: the family table;
/// i32 — the screen side of the bracket arithmetic).
const TUNING_BRACKET_HALF_PX: i32 = 12;
/// The tuning in-band color (the conduction green citation).
const TUNING_IN_BAND: [u8; 4] = [0, 150, 0, 255];

/// The 16-level quantized ramp color at `ratio = overflow/capacity`
/// (clamped to [0, 1]; `t = level / 15`; channel lerp with u8
/// rounding). NO floats reach the op stream — the ramp resolves to a
/// `[u8; 4]` BEFORE the ops are built.
fn ramp_color(ratio: f64) -> [u8; 4] {
    let clamped = ratio.clamp(0.0, 1.0);
    let level = ((clamped * f64::from(CONGESTION_RAMP_STEPS)) as usize).min(15);
    let t = f64::from(level as u32) / f64::from(CONGESTION_RAMP_STEPS - 1);
    let lerp =
        |a: u8, b: u8| -> u8 { (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8 };
    [
        lerp(CONGESTION_RAMP_LOW[0], CONGESTION_RAMP_HIGH[0]),
        lerp(CONGESTION_RAMP_LOW[1], CONGESTION_RAMP_HIGH[1]),
        lerp(CONGESTION_RAMP_LOW[2], CONGESTION_RAMP_HIGH[2]),
        255,
    ]
}
/// Pushes one DASHED screen segment (the ratsnest family's walk):
/// 8 px on / 6 px off in f64 screen px, endpoints rounded with f64
/// `round` (deterministic). A segment at most 8 px long renders
/// SOLID (a dash shorter than the on-length carries no information —
/// the family table's note).
fn push_dashed_segment(ops: &mut Vec<RenderOp>, from: IntPoint, to: IntPoint) {
    let (x0, y0) = (f64::from(from.x), f64::from(from.y));
    let (dx, dy) = (f64::from(to.x) - x0, f64::from(to.y) - y0);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= RATSNEST_DASH_ON_PX {
        ops.push(RenderOp::MoveTo(from));
        ops.push(RenderOp::LineTo(to));
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    let mut d = 0.0;
    let mut on = true;
    while d < length {
        let dash = if on {
            RATSNEST_DASH_ON_PX
        } else {
            RATSNEST_DASH_OFF_PX
        };
        let end = (d + dash).min(length);
        if on {
            ops.push(RenderOp::MoveTo(IntPoint::new(
                (x0 + ux * d).round() as i32,
                (y0 + uy * d).round() as i32,
            )));
            ops.push(RenderOp::LineTo(IntPoint::new(
                (x0 + ux * end).round() as i32,
                (y0 + uy * end).round() as i32,
            )));
        }
        d = end;
        on = !on;
    }
}

/// The marker radius in screen px: `max(4, min(4 + depth x scale,
/// 64))` (the family table's derivation).
fn marker_radius_px(depth: f64, scale: f32) -> i64 {
    let radius = (MARKER_RADIUS_BASE_PX + depth * f64::from(scale))
        .clamp(MARKER_RADIUS_BASE_PX, MARKER_RADIUS_MAX_PX);
    radius.round() as i64
}

/// The inclusive bbox of a world segment (the airline cull face).
fn segment_bbox(a: &PointPrimitive, b: &PointPrimitive) -> WorldBBox {
    WorldBBox {
        ll_x: a.x.min(b.x),
        ll_y: a.y.min(b.y),
        ur_x: a.x.max(b.x),
        ur_y: a.y.max(b.y),
    }
}

/// The cull face of a world POINT (markers, tuning anchors): the
/// degenerate bbox, inclusive.
fn point_box(p: &PointPrimitive) -> WorldBBox {
    WorldBBox {
        ll_x: p.x,
        ll_y: p.y,
        ur_x: p.x,
        ur_y: p.y,
    }
}

/// Emits the ratsnest family (flag ON): the family header
/// (`SetColor(ratsnest)` + `SetWidth(1.0)`) once when at least one
/// airline survives the cull, then the dashed walks. Cull: the
/// segment bbox, inclusive, counted.
fn emit_ratsnest(
    ops: &mut Vec<RenderOp>,
    culled: &mut usize,
    airlines: &[AirLinePrimitive],
    view: &ViewModel,
    viewport: &IntBox,
) {
    let mut surviving: Vec<&AirLinePrimitive> = Vec::new();
    for airline in airlines {
        if segment_bbox(&airline.from, &airline.to).overlaps_viewport(viewport) {
            surviving.push(airline);
        } else {
            *culled += 1;
        }
    }
    if surviving.is_empty() {
        return;
    }
    let transform = &view.transform;
    ops.push(RenderOp::SetColor(view.colors.ratsnest));
    ops.push(RenderOp::SetWidth(RATSNEST_WIDTH_PX));
    for airline in &surviving {
        push_dashed_segment(
            ops,
            transform.world_to_screen(airline.from),
            transform.world_to_screen(airline.to),
        );
    }
}

/// Emits the DRC family (flag ON): per marker `SetColor(violation)` +
/// `Circle{center, radius}`; cull on the center point (inclusive),
/// counted. Both phases render identically (the `phase` is DATA —
/// the family table's note).
fn emit_drc(
    ops: &mut Vec<RenderOp>,
    culled: &mut usize,
    markers: &[ViolationMarker],
    view: &ViewModel,
    viewport: &IntBox,
) {
    let transform = &view.transform;
    for marker in markers {
        if !point_box(&marker.center).overlaps_viewport(viewport) {
            *culled += 1;
            continue;
        }
        ops.push(RenderOp::SetColor(view.colors.violation));
        ops.push(RenderOp::Circle {
            center: transform.world_to_screen(marker.center),
            radius: marker_radius_px(marker.depth, transform.scale_f32()),
        });
    }
}

/// Emits the congestion family (flag ON): per congested cell
/// `SetColor(ramp(overflow / max(1, capacity)))` +
/// `FillPolygon(cell rect)`; cull on the cell rect (inclusive),
/// counted. Cells in wire order; the rect is the cell's world span
/// transformed at its four corners (corner order: ll, lr, ur, ul).
fn emit_congestion(
    ops: &mut Vec<RenderOp>,
    culled: &mut usize,
    heatmap: &CongestionHeatmap,
    view: &ViewModel,
    viewport: &IntBox,
) {
    let transform = &view.transform;
    for cell in &heatmap.cells {
        let ll_x = heatmap
            .origin
            .x
            .saturating_add((cell.ix as i64).saturating_mul(heatmap.cell_size));
        let ll_y = heatmap
            .origin
            .y
            .saturating_add((cell.iy as i64).saturating_mul(heatmap.cell_size));
        let ur_x = ll_x.saturating_add(heatmap.cell_size);
        let ur_y = ll_y.saturating_add(heatmap.cell_size);
        let cell_box = WorldBBox {
            ll_x,
            ll_y,
            ur_x,
            ur_y,
        };
        if !cell_box.overlaps_viewport(viewport) {
            *culled += 1;
            continue;
        }
        let ratio = cell.overflow as f64 / i64::max(cell.capacity, 1) as f64;
        ops.push(RenderOp::SetColor(ramp_color(ratio)));
        ops.push(RenderOp::FillPolygon(vec![
            transform.world_to_screen(PointPrimitive { x: ll_x, y: ll_y }),
            transform.world_to_screen(PointPrimitive { x: ur_x, y: ll_y }),
            transform.world_to_screen(PointPrimitive { x: ur_x, y: ur_y }),
            transform.world_to_screen(PointPrimitive { x: ll_x, y: ur_y }),
        ]));
    }
}

/// Emits the tuning family (flag ON): per tuning entry a closed
/// square BRACKET outline (MoveTo + 4 LineTo, half-side 12 px) at
/// the net's anchor; cull on the anchor point (inclusive), counted.
/// Anchor rule (the family table): the net's FIRST airline `from`
/// point (wire order), else the first trace of that net's first
/// polyline corner (wire order); an entry with neither anchor
/// contributes NOTHING (data without geometry). Band colors:
/// in-band = the documented green, out-of-band = the violation
/// color, predicate = band membership with the 0.0-absent
/// convention (NOT Java's DRC gate — the family table's note).
fn emit_tuning(
    ops: &mut Vec<RenderOp>,
    culled: &mut usize,
    infos: &[NetTuningInfo],
    airlines: &[AirLinePrimitive],
    traces: &[TracePrimitive],
    view: &ViewModel,
    viewport: &IntBox,
) {
    let transform = &view.transform;
    for info in infos {
        let anchor = airlines
            .iter()
            .find(|airline| airline.net == info.net)
            .map(|airline| airline.from)
            .or_else(|| {
                traces
                    .iter()
                    .find(|trace| trace.net == info.net)
                    .and_then(|trace| trace.polyline_points.first().copied())
            });
        let Some(anchor) = anchor else {
            continue;
        };
        if !point_box(&anchor).overlaps_viewport(viewport) {
            *culled += 1;
            continue;
        }
        let in_band = (info.min == 0.0 || info.actual >= info.min)
            && (info.max == 0.0 || info.actual <= info.max);
        ops.push(RenderOp::SetColor(if in_band {
            TUNING_IN_BAND
        } else {
            view.colors.violation
        }));
        let center = transform.world_to_screen(anchor);
        let (lx, ly) = (
            center.x - TUNING_BRACKET_HALF_PX,
            center.y - TUNING_BRACKET_HALF_PX,
        );
        let (ux, uy) = (
            center.x + TUNING_BRACKET_HALF_PX,
            center.y + TUNING_BRACKET_HALF_PX,
        );
        ops.push(RenderOp::MoveTo(IntPoint::new(lx, ly)));
        ops.push(RenderOp::LineTo(IntPoint::new(ux, ly)));
        ops.push(RenderOp::LineTo(IntPoint::new(ux, uy)));
        ops.push(RenderOp::LineTo(IntPoint::new(lx, uy)));
        ops.push(RenderOp::LineTo(IntPoint::new(lx, ly)));
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

    // --- THE OVERLAY EXTENSION SITE — still the ONLY read of
    // `view.overlays` in the projection (the T4 single-site law, kept
    // at T5). Overlay ops append HERE, after all board geometry (the
    // determinism law: overlays last), in the FIXED family order
    // ratsnest -> drc -> congestion -> tuning; each family in its
    // wire order (the module doc's family table carries every
    // constant's derivation).
    if view.overlays.ratsnest {
        emit_ratsnest(
            &mut ops,
            &mut culled,
            &snapshot.overlays.airlines,
            view,
            &viewport,
        );
    }
    if view.overlays.drc {
        emit_drc(
            &mut ops,
            &mut culled,
            &snapshot.overlays.violation_markers,
            view,
            &viewport,
        );
    }
    if view.overlays.congestion
        && let Some(heatmap) = &snapshot.overlays.congestion
    {
        emit_congestion(&mut ops, &mut culled, heatmap, view, &viewport);
    }
    if view.overlays.tuning
        && let Some(infos) = &snapshot.overlays.tuning
    {
        emit_tuning(
            &mut ops,
            &mut culled,
            infos,
            &snapshot.overlays.airlines,
            &snapshot.traces,
            view,
            &viewport,
        );
    }

    RenderList { ops, culled }
}
