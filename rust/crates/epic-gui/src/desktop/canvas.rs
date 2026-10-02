//! The M9-T6 canvas (desktop-gated): the egui painter over the pure
//! `project` face — the T4 op mapping, per the charter:
//!
//! * `SetColor`/`SetWidth` set painter state (the stroke color /
//!   width, px);
//! * `MoveTo`/`LineTo` runs batched into path segments per state (a
//!   walk run flushes as ONE stroked path shape on the next state
//!   change);
//! * `FillPolygon` -> filled polygon shape;
//! * `Circle` -> stroked circle (the radius is already screen px
//!   i64).
//!
//! The viewport passed to `project` is the canvas rect in world
//! coordinates (the cull precheck's face): `screen_to_world` of the
//! rect corners. The ops come back in canvas-RELATIVE screen px (the
//! transform includes pan), so painter positions add the rect
//! origin. The shell must NOT assume lossless world->screen at
//! fractional zoom (the AM4 exactness contract): the ops are already
//! screen-space — the transform's documented truncation IS the face.
//!
//! Documented thin-shell limitations:
//!
//! * `FillPolygon` renders via egui's `convex_polygon` + a closed
//!   same-color outline stroke (the standard egui idiom: the 1-px
//!   outline covers the diagonal seam of a slightly concave
//!   polygon). A strongly concave polygon can show fill seams — a
//!   presentation face only (the ops stream is the pinned face).
//! * The zoom RE-ANCHORS the pan at the cursor across a wheel step
//!   (G1: `shell::zoom_about_point` — the world point under the
//!   pointer stays under the pointer; the pre-G1 origin-anchored
//!   face lost a centered board one octave per step). The residual
//!   sub-lattice anchor error at coarse zooms is bounded by one
//!   world lattice step on screen (the pinned drift face).

use crate::render::{RenderList, RenderOp, project};
use crate::view::{ScreenTransform, ViewModel};
use eframe::egui;
use epic_engine::snapshot::BoardSnapshot;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;

/// Paints the snapshot into the canvas `rect`; returns the op count
/// (the status line's face).
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    snapshot: &BoardSnapshot,
    view: &ViewModel,
) -> usize {
    // The canvas backdrop (the color table's background).
    painter.rect_filled(rect, 0.0, color(view.colors.background));
    paint_ops(painter, rect, snapshot, view)
}

/// The viewport face: the canvas rect in world coordinates
/// (INCLUSIVE overlap in the cull precheck — `screen_to_world` of
/// the rect corners).
fn world_viewport(transform: &ScreenTransform, rect: egui::Rect) -> IntBox {
    let ll = transform.screen_to_world(IntPoint::new(rect.min.x as i32, rect.min.y as i32));
    let ur = transform.screen_to_world(IntPoint::new(rect.max.x as i32, rect.max.y as i32));
    IntBox {
        ll: IntPoint::new(saturate_i32(ll.x), saturate_i32(ll.y)),
        ur: IntPoint::new(saturate_i32(ur.x), saturate_i32(ur.y)),
    }
}

fn paint_ops(
    painter: &egui::Painter,
    rect: egui::Rect,
    snapshot: &BoardSnapshot,
    view: &ViewModel,
) -> usize {
    let transform = &view.transform;
    let viewport = world_viewport(transform, rect);
    let list: RenderList = project(snapshot, view, viewport);
    let op_count = list.ops.len();
    let mut state = StrokeState {
        color: egui::Color32::WHITE,
        width: 1.0,
    };
    let mut path: Vec<egui::Pos2> = Vec::new();
    for op in list.ops {
        match op {
            RenderOp::SetColor(c) => {
                flush_path(painter, &mut path, &state);
                state.color = color(c);
            }
            RenderOp::SetWidth(w) => {
                flush_path(painter, &mut path, &state);
                state.width = w.max(0.1);
            }
            RenderOp::MoveTo(p) => {
                flush_path(painter, &mut path, &state);
                path.push(point(rect, p));
            }
            RenderOp::LineTo(p) => {
                path.push(point(rect, p));
            }
            RenderOp::FillPolygon(points) => {
                flush_path(painter, &mut path, &state);
                fill_polygon(painter, rect, &points, &state);
            }
            RenderOp::Circle { center, radius } => {
                flush_path(painter, &mut path, &state);
                // Q8 clamp-asymmetry note: the Circle stroke floors at
                // 1.0 px (a drill hole thinner than 1 px would vanish
                // entirely — a 1-px ring is the minimum VISIBLE
                // circle), while path strokes floor at 0.1 px (traces
                // legitimately render sub-pixel at fit zoom, where a
                // 1-px floor would turn every hairline trace into a
                // visible 1-px line and drown the geometry).
                painter.circle_stroke(
                    point(rect, center),
                    radius.max(1) as f32,
                    egui::Stroke::new(state.width.max(1.0), state.color),
                );
            }
        }
    }
    flush_path(painter, &mut path, &state);
    op_count
}

/// Flushes the accumulated `MoveTo`/`LineTo` run as ONE stroked path
/// shape (the charter's batched-path face); a run of fewer than two
/// points flushes as nothing.
fn flush_path(painter: &egui::Painter, path: &mut Vec<egui::Pos2>, state: &StrokeState) {
    if path.len() >= 2 {
        let stroke = egui::Stroke::new(state.width.max(0.1), state.color);
        painter.add(egui::Shape::line(std::mem::take(path), stroke));
    } else {
        path.clear();
    }
}

/// The `FillPolygon` face: egui `convex_polygon` fill + the closed
/// same-color outline stroke (the module doc's concavity note).
fn fill_polygon(
    painter: &egui::Painter,
    rect: egui::Rect,
    points: &[IntPoint],
    state: &StrokeState,
) {
    if points.len() < 3 {
        return;
    }
    let pts: Vec<egui::Pos2> = points.iter().map(|p| point(rect, *p)).collect();
    let fill = state.color;
    let stroke = egui::Stroke::new(1.0, fill);
    painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
}

fn color(rgba: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn point(rect: egui::Rect, p: IntPoint) -> egui::Pos2 {
    egui::Pos2::new(rect.min.x + p.x as f32, rect.min.y + p.y as f32)
}

/// The canvas corner as a WORLD point pre-conversion (the transform
/// input face).
/// The world i64 -> screen-rect i32 narrowing (saturating — the
/// same documented face as the transform's own narrowings; a board
/// corner beyond the i32 world-viewport range culls by saturation,
/// never panics).
fn saturate_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

struct StrokeState {
    color: egui::Color32,
    width: f32,
}
