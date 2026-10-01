//! Port of Java `app.freerouting.geometry.planar.RegularTileShape` — the
//! sealed `TileShape` subtype whose border directions come from a fixed
//! set: [`IntBox`] and [`IntOctagon`].
//!
//! Java models the cross-type operations (`compare`, `union`,
//! `contains`, `isContainedIn`) as abstract overloads declared twice:
//! once with `RegularTileShape` and once with the concrete leaf type. The
//! `RegularTileShape`-typed overrides *negate the swapped call*, and Java
//! overload resolution picks the concrete overload because the swapped
//! argument is `this` (statically typed as the leaf). The Rust enum arms
//! below reproduce those resolved call chains exactly:
//!
//! - `compare(a, b, e)` = `b.compare_typed(a, e).negate()` where the typed
//!   call is the (Box,Box)/(Oct,Oct)/(Oct-as-receiver,Box)/(Box-as-receiver,Oct)
//!   overload of `b`.
//! - `union(a, b)` = `b.union_typed(a)`.
//! - `contains_regular(a, b)` = `b.isContainedIn(a)` with the typed
//!   `isContainedIn` overload of `b`.
//!
//! Deferral ledger (Task 7 closure): everything below is ported. Nothing
//! in `RegularTileShape.java` is deferred; it has no Task-8-typed members.

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::line::Line;
use crate::point::Point;
use crate::shape::ShapeBoundingDirections;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// TileShapes whose border lines have directions out of a fixed set
/// (Java `RegularTileShape`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegularTileShape {
    IntBox(IntBox),
    IntOctagon(IntOctagon),
}

impl RegularTileShape {
    /// The box variant, if this is a box.
    pub fn as_int_box(&self) -> Option<&IntBox> {
        match self {
            RegularTileShape::IntBox(b) => Some(b),
            RegularTileShape::IntOctagon(_) => None,
        }
    }

    /// The octagon variant, if this is an octagon.
    pub fn as_int_octagon(&self) -> Option<&IntOctagon> {
        match self {
            RegularTileShape::IntBox(_) => None,
            RegularTileShape::IntOctagon(o) => Some(o),
        }
    }

    /// Returns true, if this shape is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.is_empty(),
            // IntOctagon.isEmpty is the EMPTY-sentinel identity test (T11).
            RegularTileShape::IntOctagon(o) => o.is_empty(),
        }
    }

    /// Returns true, if this shape is bounded.
    pub fn is_bounded(&self) -> bool {
        // Both regulars return true unconditionally in Java.
        true
    }

    /// Returns true if this regular tile shape is a box or can be
    /// converted into a box.
    pub fn is_int_box(&self) -> bool {
        match self {
            RegularTileShape::IntBox(_) => true,
            RegularTileShape::IntOctagon(o) => o.is_int_box(),
        }
    }

    /// Returns true if this regular tile shape is an octagon or can be
    /// converted into an octagon.
    pub fn is_int_octagon(&self) -> bool {
        // Both IntBox and IntOctagon return true in Java
        // (IntBox.isIntOctagon is true by definition).
        true
    }

    /// Returns the dimension of this shape.
    pub fn dimension(&self) -> i32 {
        match self {
            RegularTileShape::IntBox(b) => b.dimension(),
            RegularTileShape::IntOctagon(o) => o.dimension(),
        }
    }

    /// Returns the number of border lines of this shape.
    pub fn border_line_count(&self) -> usize {
        match self {
            // IntBox.borderLineCount() == 4, IntOctagon.borderLineCount() == 8.
            RegularTileShape::IntBox(_) => 4,
            RegularTileShape::IntOctagon(_) => 8,
        }
    }

    /// Returns the no-th border line of this shape.
    ///
    /// Panics for an out-of-range index (Java throws).
    pub fn border_line(&self, no: i32) -> Line {
        match self {
            RegularTileShape::IntBox(b) => b.border_line(no),
            RegularTileShape::IntOctagon(o) => o.border_line(no),
        }
    }

    /// Returns the edge number if line is a border line of this shape,
    /// otherwise -1. Both leaves log a warning and return -1 (identity
    /// comparison is deliberately not implemented, bug-compatible with the
    /// oracle), so the port always returns -1.
    pub fn border_line_index(&self, line: &Line) -> i32 {
        match self {
            RegularTileShape::IntBox(b) => b.border_line_index(line),
            RegularTileShape::IntOctagon(o) => o.border_line_index(line),
        }
    }

    /// Returns the no-th corner of this shape. The corners are sorted
    /// counterclock starting with the corner of smallest y (smallest x on
    /// ties). Consecutive corners may be equal.
    pub fn corner(&self, no: i32) -> Point {
        match self {
            RegularTileShape::IntBox(b) => Point::int(b.corner(no)),
            RegularTileShape::IntOctagon(o) => Point::int(o.corner(no)),
        }
    }

    /// Returns an approximation of the no-th corner of this shape.
    pub fn corner_approx(&self, no: i32) -> FloatPoint {
        match self {
            RegularTileShape::IntBox(b) => b.corner(no).to_float(),
            RegularTileShape::IntOctagon(o) => o.corner(no).to_float(),
        }
    }

    /// Returns approximations of all corners of this shape.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..self.border_line_count() as i32)
            .map(|i| self.corner_approx(i))
            .collect()
    }

    /// Returns true if the shape has no infinite part at this corner.
    /// Both regular shapes are bounded at every corner.
    pub fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    /// Returns the content of the area of the shape.
    pub fn area(&self) -> f64 {
        match self {
            RegularTileShape::IntBox(b) => b.area(),
            RegularTileShape::IntOctagon(o) => o.area(),
        }
    }

    /// Returns the cumulative border length of the shape. IntBox overrides
    /// with the exact `2 * (w + h)` formula; IntOctagon uses the generic
    /// corner-distance sum of PolylineShape (both regulars are bounded, so
    /// the unbounded branch is dead here but kept for parity).
    pub fn circumference(&self) -> f64 {
        match self {
            RegularTileShape::IntBox(b) => b.circumference(),
            RegularTileShape::IntOctagon(o) => {
                let corner_count = 8;
                let mut result = 0.0f64;
                let mut prev_corner = o.corner(corner_count - 1).to_float();
                for i in 0..corner_count {
                    let current_corner = o.corner(i).to_float();
                    result += current_corner.distance(&prev_corner);
                    prev_corner = current_corner;
                }
                result
            }
        }
    }

    /// Returns the arithmetic middle of the corners (PolylineShape).
    pub fn centre_of_gravity(&self) -> FloatPoint {
        crate::shape::corner_average(self.border_line_count(), |i| self.corner_approx(i))
    }

    /// Returns the maximum diameter of the shape.
    pub fn max_width(&self) -> f64 {
        match self {
            RegularTileShape::IntBox(b) => b.max_width(),
            RegularTileShape::IntOctagon(o) => o.max_width(),
        }
    }

    /// Returns the minimum diameter of the shape.
    pub fn min_width(&self) -> f64 {
        match self {
            RegularTileShape::IntBox(b) => b.min_width(),
            RegularTileShape::IntOctagon(o) => o.min_width(),
        }
    }

    /// Returns the smallest bounding box of this shape.
    pub fn bounding_box(&self) -> IntBox {
        match self {
            RegularTileShape::IntBox(b) => *b,
            // IntOctagon.boundingBox() == new IntBox(leftX, bottomY,
            // rightX, topY); the Rust IntOctagon::bounding_box is that
            // constructor (appended with the Task 7 closures).
            RegularTileShape::IntOctagon(o) => o.bounding_box(),
        }
    }

    /// Returns the smallest bounding octagon of this shape.
    pub fn bounding_octagon(&self) -> IntOctagon {
        match self {
            RegularTileShape::IntBox(b) => b.to_int_octagon(),
            RegularTileShape::IntOctagon(o) => *o,
        }
    }

    /// The bounding tile of a regular tile shape is itself (Java
    /// `boundingTile` leaf overrides).
    pub fn bounding_tile(&self) -> TileShape {
        TileShape::RegularTileShape(self.clone())
    }

    /// Returns the bounding RegularTileShape with the fixed directions
    /// `dirs`. Infallible for regular shapes (Java never returns null
    /// here).
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => dirs.bounds_box(b),
            RegularTileShape::IntOctagon(o) => dirs.bounds_octagon(o),
        }
    }

    /// Converts this shape into an octagon (identity for the octagon
    /// variant).
    pub fn to_int_octagon(&self) -> IntOctagon {
        match self {
            RegularTileShape::IntBox(b) => b.to_int_octagon(),
            RegularTileShape::IntOctagon(o) => *o,
        }
    }

    /// Converts the internal representation of this shape to a Simplex.
    pub fn to_simplex(&self) -> Simplex {
        match self {
            RegularTileShape::IntBox(b) => b.to_simplex(),
            RegularTileShape::IntOctagon(o) => o.to_simplex(),
        }
    }

    /// Converts the physical instance of this shape to a simpler physical
    /// instance, if possible (a box-like octagon to a box; identity
    /// otherwise).
    pub fn simplify(&self) -> TileShape {
        match self {
            RegularTileShape::IntBox(b) => {
                TileShape::RegularTileShape(RegularTileShape::IntBox(*b))
            }
            RegularTileShape::IntOctagon(o) => o.simplify(),
        }
    }

    /// Returns a unique ID for this shape for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        match self {
            RegularTileShape::IntBox(b) => b.get_id(),
            RegularTileShape::IntOctagon(o) => o.get_id(),
        }
    }

    /// Compares the edge lines of index `edge_index` of this shape and
    /// `other`: `Side::Positive` if the edge line of this shape is to the
    /// left of the edge line of other, `Side::Collinear` if they are
    /// equal, `Side::Negative` if it is to the right.
    ///
    /// Faithful to the Java chain: the leaf override returns
    /// `other.compare(this, e).negate()`, and overload resolution picks
    /// the concrete overload of `other` (see module docs).
    pub fn compare(&self, other: &RegularTileShape, edge_index: i32) -> Side {
        match (self, other) {
            // IntBox.compare(RegularTileShape) -> IntBox.compare(IntBox).
            (RegularTileShape::IntBox(a), RegularTileShape::IntBox(b)) => {
                b.compare(a, edge_index).negate()
            }
            // IntBox.compare(RegularTileShape) -> IntOctagon.compare(IntBox).
            (RegularTileShape::IntBox(a), RegularTileShape::IntOctagon(b)) => {
                b.compare_box(a, edge_index).negate()
            }
            // IntOctagon.compare(RegularTileShape) -> IntBox.compare(IntOctagon)
            // == toIntOctagon().compare(other, e); the conversion is on the
            // BOX argument.
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntBox(b)) => {
                b.to_int_octagon().compare(a, edge_index).negate()
            }
            // IntOctagon.compare(RegularTileShape) -> IntOctagon.compare(IntOctagon).
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntOctagon(b)) => {
                b.compare(a, edge_index).negate()
            }
        }
    }

    /// Calculates the smallest RegularTileShape containing this shape and
    /// other (Java `union`; same resolution chain as [`Self::compare`]).
    pub fn union(&self, other: &RegularTileShape) -> RegularTileShape {
        match (self, other) {
            // IntBox.union(RegularTileShape) -> IntBox.union(IntBox).
            (RegularTileShape::IntBox(a), RegularTileShape::IntBox(b)) => {
                RegularTileShape::IntBox(b.union(a))
            }
            // IntBox.union(RegularTileShape) -> IntOctagon.union(IntBox).
            (RegularTileShape::IntBox(a), RegularTileShape::IntOctagon(b)) => {
                RegularTileShape::IntOctagon(b.union_box(a))
            }
            // IntOctagon.union(RegularTileShape) -> IntBox.union(IntOctagon)
            // == other.union(toIntOctagon()).
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntBox(b)) => {
                RegularTileShape::IntOctagon(b.union_octagon(a))
            }
            // IntOctagon.union(RegularTileShape) -> IntOctagon.union(IntOctagon).
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntOctagon(b)) => {
                RegularTileShape::IntOctagon(b.union(a))
            }
        }
    }

    /// Returns true, if this shape contains other completely (Java
    /// `contains(RegularTileShape)` == `other.isContainedIn(this)`).
    pub fn contains_regular(&self, other: &RegularTileShape) -> bool {
        match (self, other) {
            (RegularTileShape::IntBox(a), RegularTileShape::IntBox(b)) => b.is_contained_in(a),
            (RegularTileShape::IntBox(a), RegularTileShape::IntOctagon(b)) => {
                b.is_contained_in_box(a)
            }
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntBox(b)) => {
                b.is_contained_in_octagon(a)
            }
            (RegularTileShape::IntOctagon(a), RegularTileShape::IntOctagon(b)) => {
                b.is_contained_in(a)
            }
        }
    }

    /// Returns true, if this shape is completely contained in box.
    pub fn is_contained_in_box(&self, r#box: &IntBox) -> bool {
        match self {
            RegularTileShape::IntBox(b) => b.is_contained_in(r#box),
            // IntOctagon.isContainedIn(IntBox) == box.contains(toIntOctagon()).
            RegularTileShape::IntOctagon(o) => o.is_contained_in_box(r#box),
        }
    }

    /// Returns true, if this shape is completely contained in octagon.
    pub fn is_contained_in_octagon(&self, oct: &IntOctagon) -> bool {
        match self {
            // IntBox.isContainedIn(IntOctagon) == oct.contains(toIntOctagon()).
            RegularTileShape::IntBox(b) => b.is_contained_in_octagon(oct),
            RegularTileShape::IntOctagon(o) => o.is_contained_in(oct),
        }
    }

    /// Returns the affine translation of this shape by vector.
    pub fn translate_by(&self, vector: &Vector) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => RegularTileShape::IntBox(b.translate_by(vector)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.translate_by(vector)),
        }
    }

    /// Returns this shape offsetted by width. If width > 0, the offset is
    /// to the outside, else to the inside.
    pub fn offset(&self, width: f64) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => RegularTileShape::IntBox(b.offset(width)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.offset(width)),
        }
    }

    /// Returns this shape enlarged by offset (Java `enlarge` leaf
    /// overrides: the box result is an octagon).
    pub fn enlarge(&self, offset: f64) -> RegularTileShape {
        match self {
            RegularTileShape::IntBox(b) => RegularTileShape::IntOctagon(b.enlarge(offset)),
            RegularTileShape::IntOctagon(o) => RegularTileShape::IntOctagon(o.enlarge(offset)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// jshell pins `P8`: `boxT.compare(octT, e)` with
    /// boxT = getInstance(0,0,10,10) (an IntOctagon) and
    /// octT = getInstance(2,-1,12,8,2,12,0,8): the shared lower edge is
    /// on the left (0 > -1) and the right, upper-left-diagonal and
    /// lower-left-diagonal edges are on the right.
    #[test]
    fn p8_compare_edge_order_pins() {
        let box_t =
            RegularTileShape::IntOctagon(IntBox::from_corners(0, 0, 10, 10).to_int_octagon());
        let oct_t = RegularTileShape::IntOctagon(TileShape::from_8_ints(2, -1, 12, 8, 2, 12, 0, 8));
        assert_eq!(box_t.compare(&oct_t, 0), Side::Positive);
        assert_eq!(box_t.compare(&oct_t, 2), Side::Negative);
        assert_eq!(box_t.compare(&oct_t, 5), Side::Negative);
        assert_eq!(box_t.compare(&oct_t, 7), Side::Negative);
        // The same geometry with an IntBox receiver exercises the
        // (Box, Octagon) arm: octT.compare(box, e).negate().
        let box_variant = RegularTileShape::IntBox(IntBox::from_corners(0, 0, 10, 10));
        assert_eq!(box_variant.compare(&oct_t, 0), Side::Positive);
        assert_eq!(box_variant.compare(&oct_t, 2), Side::Negative);
    }
}
