//! Port of Java `app.freerouting.geometry.planar.PolylineArea` — an Area
//! where the outside border curve and the hole borders consist of straight
//! lines.
//!
//! Java takes `PolylineShape` arguments; the only subclass in the oracle is
//! `PolygonShape`, so the port uses it concretely. The Java
//! `StoppableThread.isStopRequested()` hook becomes an
//! [`AtomicBool`] "stop requested" flag (Java `splitToConvex(Stoppable)`
//! -> [`PolylineArea::split_to_convex_stoppable`]).
//!
//! T14: the `precalculatedConvexPieces` transient cache is dropped; the
//! split is recomputed per call.

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_point::IntPoint;
use crate::polygon_shape::PolygonShape;
use crate::tile_shape::TileShape;
use crate::vector::Vector;
use std::sync::atomic::{AtomicBool, Ordering};

/// Java `PolylineArea`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolylineArea {
    /// Java `borderShape` (package-private final).
    pub border_shape: PolygonShape,
    /// Java `holeArr` (package-private final).
    pub hole_arr: Vec<PolygonShape>,
}

impl PolylineArea {
    /// Creates a new instance (Java `PolylineArea(PolylineShape,
    /// PolylineShape[])`).
    pub fn new(border_shape: PolygonShape, hole_arr: Vec<PolygonShape>) -> PolylineArea {
        PolylineArea {
            border_shape,
            hole_arr,
        }
    }

    /// Java `cutoutHolePiece`: cuts hole_piece out of divide_piece and adds
    /// the 2-dimensional result pieces.
    fn cutout_hole_piece(
        divide_piece: &TileShape,
        hole_piece: &TileShape,
        pieces: &mut Vec<TileShape>,
    ) {
        let result_pieces = divide_piece.cutout(hole_piece);
        for current_piece in result_pieces {
            if current_piece.dimension() == 2 {
                pieces.push(current_piece);
            }
        }
    }

    /// Java `dimension()`.
    pub fn dimension(&self) -> i32 {
        self.border_shape.dimension()
    }

    /// Java `isBounded()`.
    pub fn is_bounded(&self) -> bool {
        self.border_shape.is_bounded()
    }

    /// Java `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.border_shape.is_empty()
    }

    /// Java `isContainedIn(IntBox)`.
    pub fn is_contained_in(&self, r#box: &IntBox) -> bool {
        self.border_shape.is_contained_in(r#box)
    }

    /// Java `getBorder()`.
    pub fn get_border(&self) -> &PolygonShape {
        &self.border_shape
    }

    /// Java `getHoles()`.
    pub fn get_holes(&self) -> &[PolygonShape] {
        &self.hole_arr
    }

    /// Java `boundingBox()`.
    pub fn bounding_box(&self) -> IntBox {
        self.border_shape.bounding_box()
    }

    /// Java `boundingOctagon()`.
    pub fn bounding_octagon(&self) -> crate::int_octagon::IntOctagon {
        self.border_shape.bounding_octagon()
    }

    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        if !self.border_shape.contains_float(point) {
            return false;
        }
        self.hole_arr.iter().all(|hole| !hole.contains_float(point))
    }

    /// Java `contains(Point)`.
    pub fn contains_point(&self, point: &crate::point::Point) -> bool {
        if !self.border_shape.contains_point(point) {
            return false;
        }
        self.hole_arr
            .iter()
            .all(|hole| !hole.contains_inside(point))
    }

    /// Java `nearestPointApprox(FloatPoint)`; panics where Java would NPE
    /// (no pieces / failed split).
    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        crate::shape::nearest_point_approx_over_pieces(&self.expect_split_pieces(), from_point)
    }

    /// Java `translateBy(Vector)`.
    pub fn translate_by(&self, vector: &Vector) -> PolylineArea {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        let translated_border = self.border_shape.translate_by(vector);
        let translated_holes: Vec<PolygonShape> = self
            .hole_arr
            .iter()
            .map(|hole| hole.translate_by(vector))
            .collect();
        PolylineArea::new(translated_border, translated_holes)
    }

    /// Java `cornerApproxArr()`: border corners followed by hole corners.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        let mut result = self.border_shape.corner_approx_arr();
        for hole in &self.hole_arr {
            result.extend(hole.corner_approx_arr());
        }
        result
    }

    /// Java `splitToConvex()`: `None` where Java returns null.
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        self.split_to_convex_impl(None)
    }

    /// Java `splitToConvex(Stoppable)`: interruptible through the
    /// stop-requested flag; `None` where Java returns null.
    pub fn split_to_convex_stoppable(&self, stop_requested: &AtomicBool) -> Option<Vec<TileShape>> {
        self.split_to_convex_impl(Some(stop_requested))
    }

    fn split_to_convex_impl(&self, stop_requested: Option<&AtomicBool>) -> Option<Vec<TileShape>> {
        let convex_border_pieces = self.border_shape.split_to_convex()?;
        let mut current_piece_list: Vec<TileShape> = convex_border_pieces;
        for hole in &self.hole_arr {
            if hole.dimension() < 2 {
                // Java: FRLogger.warn("dimension 2 for hole expected")
                continue;
            }
            let convex_hole_pieces = hole.split_to_convex()?;
            for current_hole_piece in convex_hole_pieces {
                let mut new_piece_list: Vec<TileShape> = Vec::new();
                for current_divide_piece in &current_piece_list {
                    if let Some(flag) = stop_requested
                        && flag.load(Ordering::Relaxed)
                    {
                        return None;
                    }
                    Self::cutout_hole_piece(
                        current_divide_piece,
                        &current_hole_piece,
                        &mut new_piece_list,
                    );
                }
                current_piece_list = new_piece_list;
            }
        }
        Some(current_piece_list)
    }

    /// The split pieces or the Java NPE at the dereference site.
    fn expect_split_pieces(&self) -> Vec<TileShape> {
        crate::shape::expect_pieces(self.split_to_convex())
    }

    /// Java `turn90Degree(int, IntPoint)`.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> PolylineArea {
        let new_border = self.border_shape.turn_90_degree(factor, pole);
        let new_hole_arr: Vec<PolygonShape> = self
            .hole_arr
            .iter()
            .map(|hole| hole.turn_90_degree(factor, pole))
            .collect();
        PolylineArea::new(new_border, new_hole_arr)
    }

    /// Java `rotateApprox(double, FloatPoint)`.
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> PolylineArea {
        let new_border = self.border_shape.rotate_approx(angle, pole);
        let new_hole_arr: Vec<PolygonShape> = self
            .hole_arr
            .iter()
            .map(|hole| hole.rotate_approx(angle, pole))
            .collect();
        PolylineArea::new(new_border, new_hole_arr)
    }

    /// Java `mirrorVertical(IntPoint)`.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> PolylineArea {
        let new_border = self.border_shape.mirror_vertical(pole);
        let new_hole_arr: Vec<PolygonShape> = self
            .hole_arr
            .iter()
            .map(|hole| hole.mirror_vertical(pole))
            .collect();
        PolylineArea::new(new_border, new_hole_arr)
    }

    /// Java `mirrorHorizontal(IntPoint)`.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> PolylineArea {
        let new_border = self.border_shape.mirror_horizontal(pole);
        let new_hole_arr: Vec<PolygonShape> = self
            .hole_arr
            .iter()
            .map(|hole| hole.mirror_horizontal(pole))
            .collect();
        PolylineArea::new(new_border, new_hole_arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_point::IntPoint;
    use crate::point::Point;
    use crate::regular_tile_shape::RegularTileShape;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// A square area of 20x20 with a 4x4 hole in the middle; containment
    /// in the hole is false, on the frame true. The jshell-captured split
    /// (pin PPA1) yields 4 boxes.
    fn area_with_hole() -> PolylineArea {
        let border = PolygonShape::new(&[p(0, 0), p(20, 0), p(20, 20), p(0, 20)]);
        let hole = PolygonShape::new(&[p(8, 8), p(12, 8), p(12, 12), p(8, 12)]);
        PolylineArea::new(border, vec![hole])
    }

    #[test]
    fn structure_and_measures() {
        let area = area_with_hole();
        assert_eq!(area.dimension(), 2);
        assert!(area.is_bounded());
        assert!(!area.is_empty());
        assert_eq!(area.get_border().border_line_count(), 4);
        assert_eq!(area.get_holes().len(), 1);
        assert_eq!(
            area.bounding_box(),
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(20, 20))
        );
        // inside the hole: not contained
        assert!(!area.contains_point(&p(10, 10)));
        assert!(area.contains_point(&p(2, 2)));
        assert!(area.contains_float(&FloatPoint::new(2.0, 2.0)));
        assert!(!area.contains_float(&FloatPoint::new(10.0, 10.0)));
    }

    /// jshell pin `PPA1`: the split of the 20x20 frame with a 4x4 hole
    /// yields exactly 4 IntBoxes in the order (0,0)-(12,8), (0,8)-(8,20),
    /// (12,0)-(20,12), (8,12)-(20,20).
    #[test]
    fn split_pieces_are_2d() {
        let area = area_with_hole();
        let pieces = area.split_to_convex().expect("split succeeds");
        let expected = [
            (0, 0, 12, 8),
            (0, 8, 8, 20),
            (12, 0, 20, 12),
            (8, 12, 20, 20),
        ];
        assert_eq!(pieces.len(), 4);
        for (piece, (llx, lly, urx, ury)) in pieces.iter().zip(expected.iter()) {
            match piece {
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => {
                    assert_eq!((b.ll.x, b.ll.y, b.ur.x, b.ur.y), (*llx, *lly, *urx, *ury));
                }
                other => panic!("expected IntBox piece, got {:?}", other),
            }
        }
        // the hole corners are not in any piece
        let flag = AtomicBool::new(false);
        assert!(
            area.split_to_convex_stoppable(&flag)
                .expect("not stopped")
                .len()
                == pieces.len()
        );
    }

    /// The stop flag aborts with `None` (Java returns null).
    #[test]
    fn stop_flag_aborts() {
        let area = area_with_hole();
        let flag = AtomicBool::new(true);
        assert!(area.split_to_convex_stoppable(&flag).is_none());
    }

    /// Transforms are applied to border and holes alike: turning the area
    /// by 90 degrees around (0,0) maps the hole to the mirrored location.
    #[test]
    fn transforms_move_holes() {
        let area = area_with_hole();
        let turned = area.turn_90_degree(1, &IntPoint::new(0, 0));
        assert_eq!(
            turned.get_holes()[0].bounding_box().ll,
            IntPoint::new(-12, 8)
        );
        let moved = area.translate_by(&Vector::Int(crate::int_vector::IntVector::new(1, 0)));
        assert_eq!(moved.get_holes()[0].bounding_box().ll, IntPoint::new(9, 8));
    }
}
