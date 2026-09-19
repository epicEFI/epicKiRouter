//! Port of Java `app.freerouting.geometry.planar.TileShape` — the sealed
//! abstract class above the router's tile shapes.
//!
//! Java `TileShape` has exactly two direct subclasses: `RegularTileShape`
//! (implemented by [`IntBox`] and [`IntOctagon`]) and [`Simplex`]; this
//! port is the corresponding two-variant enum. Java resolves every
//! operation through virtual dispatch plus overload selection on the
//! static receiver types; the port spells those resolved call chains out:
//!
//! - [`TileShape::intersection`] / [`TileShape::intersects`] /
//!   [`TileShape::cutout_from`] are 3x3 dispatch tables in which each arm
//!   is the SAME Java overload body the Java dispatch would select. The
//!   reverse-dispatch mirrors are kept verbatim:
//!   `IntBox.intersection(Simplex)` is
//!   `other.intersection(this.toSimplex())` (IntBox.java:324) and
//!   `IntOctagon.intersection(Simplex)` is `other.intersection(this)`
//!   (IntOctagon.java:373).
//! - Leaf specializations (e.g. the `IntBox.turn90Degree` coordinate
//!   swap, the specialized octagon/box `area`) stay on the arms; the
//!   concrete `TileShape` bodies (e.g. the border-line rotation of
//!   `turn90Degree`, TileShape.java:669) serve the arms without
//!   overrides.
//!
//! Bit-parity notes:
//! - [`TileShape::length`] keeps Java's `Integer.MAX_VALUE`
//!   (`2147483647.0`) result for unbounded shapes and seeds its
//!   two-maxima scan with `-1.0`.
//! - [`TileShape::intersecting_border_line_no`] seeds its minimum with
//!   Java's `Float.MAX_VALUE` widened to double (`f64::from(f32::MAX)`)
//!   and skips border lines whose approximate intersection has
//!   `x >= Integer.MAX_VALUE` (the parallel marker of
//!   `Line.intersectionApprox`).
//! - [`TileShape::contains_float`] is the VIRTUAL one-arg Java dispatch
//!   (pin P14B): `IntOctagon` OVERRIDES `contains(FloatPoint)` with
//!   border-INCLUDED `<=` arithmetic (`IntOctagon.java:332-344`), while
//!   `IntBox`/`Simplex` fall through to the strict two-arg border loop
//!   (P14). The int-coordinate [`TileShape::contains_point`]
//!   (`!isOutside`) accepts border points for every variant.
//!
//! Factories (Java `getInstance`): [`TileShape::get_instance`] (line
//! array, through `Simplex::get_instance(..).simplify()` — so a square
//! yields the `IntBox` arm, pin P9/P13), [`TileShape::from_line`]
//! (single line, NO simplify — Java returns the Simplex directly),
//! [`TileShape::from_points`] (point array), [`TileShape::from_8_ints`] /
//! [`TileShape::from_4_ints`] (return the normalized `IntOctagon`, like
//! Java), [`TileShape::surrounding_point`].
//!
//! Deferral ledger — CLOSED in Task 8: `rotateApprox` ([`Self::rotate_approx`]
//! with the Polygon rebuild), `cutout(Polyline)` / `entrancePoints`
//! ([`Self::cutout_polyline`] / [`Self::entrance_points`]),
//! `divideIntoSections` ([`Self::divide_into_sections`]), `touchingSides`
//! ([`Self::touching_sides`]), `distanceToTheLeft`
//! ([`Self::distance_to_the_left`]), `nearestRelativeOutsideLocations`
//! ([`Self::nearest_relative_outside_locations`]),
//! `diagonalCornerSegment` ([`Self::diagonal_corner_segment`]),
//! `isIntersectedInteriorBy` ([`Self::is_intersected_interior_by`]), and
//! `indexOfNearestCorner` ([`Self::index_of_nearest_corner`]; whose Java
//! body seeds `Double.MIN_VALUE` — the positive 4.9e-324 — as a
//! "distance" minimum, an oracle bug that pins the result to index 0;
//! ported bug-compat as trap T21 with a pin proving the bug).

use crate::direction::Direction;
use crate::float_line::FloatLine;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::{
    Shape, ShapeBoundingDirections, ShapeRef, mirror_via_border_lines, shape_centre_of_gravity,
    shape_nearest_border_point_approx, shape_nearest_border_points_approx, tile_area,
    tile_contains_float, tile_contains_inside, tile_contains_on_border,
    tile_contains_on_border_line_no, tile_is_outside, tile_nearest_point_approx, tile_offset,
    tile_shrink, turn_90_degree_via_border_lines,
};
use crate::side::Side;
use crate::simplex::Simplex;
use crate::vector::Vector;

/// The sealed tile hierarchy (Java `TileShape`): a
/// [`RegularTileShape`](`IntBox`/`IntOctagon`) or a [`Simplex`].
#[derive(Debug, Clone, PartialEq)]
pub enum TileShape {
    /// The regular tile arm (Java `RegularTileShape`).
    RegularTileShape(RegularTileShape),
    /// The half-plane intersection arm (Java `Simplex`).
    Simplex(Box<Simplex>),
}

impl TileShape {
    // -----------------------------------------------------------------
    // Factories (Java `getInstance`)
    // -----------------------------------------------------------------

    /// TileShape from border lines: `Simplex.getInstance(lines)` followed
    /// by `simplify()` — a square of 4 lines yields the
    /// [`RegularTileShape`] box arm (Java `TileShape.getInstance(Line[])`).
    pub fn get_instance(lines: &[Line]) -> TileShape {
        Simplex::get_instance(lines).simplify()
    }

    /// TileShape from a single line: the Simplex directly, WITHOUT the
    /// simplify step (Java `TileShape.getInstance(Line)`).
    pub fn from_line(line: &Line) -> TileShape {
        TileShape::Simplex(Box::new(Simplex::get_instance(std::slice::from_ref(line))))
    }

    /// TileShape through the corner points: consecutive points are joined
    /// by lines, the last point is joined back to the first. Panics on an
    /// empty slice (Java throws `ArrayIndexOutOfBoundsException`).
    pub fn from_points(points: &[Point]) -> TileShape {
        if points.is_empty() {
            panic!("TileShape.getInstance: empty point array");
        }
        let mut lines = Vec::with_capacity(points.len());
        for j in 0..points.len() - 1 {
            lines.push(Line::new(points[j].clone(), points[j + 1].clone()));
        }
        let last = points.len() - 1;
        lines.push(Line::new(points[last].clone(), points[0].clone()));
        TileShape::get_instance(&lines)
    }

    /// IntOctagon from 8 integer boundary values, normalized (Java
    /// `TileShape.getInstance(int...)`; returns the octagon, not the
    /// TileShape, exactly like Java).
    #[allow(clippy::too_many_arguments)]
    pub fn from_8_ints(
        left_x: i32,
        bottom_y: i32,
        right_x: i32,
        top_y: i32,
        upper_left_diagonal_x: i32,
        lower_right_diagonal_x: i32,
        lower_left_diagonal_x: i32,
        upper_right_diagonal_x: i32,
    ) -> IntOctagon {
        IntOctagon::new(
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        )
        .normalize()
    }

    /// IntOctagon from the axis bounds of a box, normalized (Java
    /// `TileShape.getInstance(int, int, int, int)`).
    pub fn from_4_ints(
        lower_left_x: i32,
        lower_left_y: i32,
        upper_right_x: i32,
        upper_right_y: i32,
    ) -> IntOctagon {
        IntBox::from_corners(lower_left_x, lower_left_y, upper_right_x, upper_right_y)
            .to_int_octagon()
    }

    /// The smallest box containing only this point (Java
    /// `TileShape.getInstance(Point)`).
    pub fn surrounding_point(point: &Point) -> IntBox {
        point.surrounding_box()
    }

    // -----------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------

    /// Returns true, if this shape is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            TileShape::RegularTileShape(rs) => rs.is_empty(),
            TileShape::Simplex(s) => s.is_empty(),
        }
    }

    /// Returns true, if this shape has finite extension. Regular tiles are
    /// always bounded.
    pub fn is_bounded(&self) -> bool {
        match self {
            TileShape::RegularTileShape(rs) => rs.is_bounded(),
            TileShape::Simplex(s) => s.is_bounded(),
        }
    }

    /// Returns the dimension: -1 (empty), 0 (point), 1 (segment),
    /// 2 (area).
    pub fn dimension(&self) -> i32 {
        match self {
            TileShape::RegularTileShape(rs) => rs.dimension(),
            TileShape::Simplex(s) => s.dimension(),
        }
    }

    /// The number of border lines.
    pub fn border_line_count(&self) -> usize {
        match self {
            TileShape::RegularTileShape(rs) => rs.border_line_count(),
            TileShape::Simplex(s) => s.border_line_count(),
        }
    }

    /// Creates the no-th border line. Panics out of range (Java throws
    /// `IllegalArgumentException` / NPE).
    pub fn border_line(&self, no: i32) -> Line {
        match self {
            TileShape::RegularTileShape(rs) => rs.border_line(no),
            TileShape::Simplex(s) => s.border_line(no),
        }
    }

    /// Returns the index of the border line with the same a and b points,
    /// or -1 (IntBox/IntOctagon warn and yield -1; only the Simplex arm
    /// searches).
    pub fn border_line_index(&self, line: &Line) -> i32 {
        match self {
            TileShape::RegularTileShape(rs) => rs.border_line_index(line),
            TileShape::Simplex(s) => s.border_line_index(line),
        }
    }

    /// Returns the no-th corner exactly. Panics out of range.
    pub fn corner(&self, no: i32) -> Point {
        match self {
            TileShape::RegularTileShape(rs) => rs.corner(no),
            TileShape::Simplex(s) => s.corner(no),
        }
    }

    /// Returns the no-th corner as float coordinates, or `None` where
    /// Java returns null (the unbounded simplex arm).
    pub fn corner_approx(&self, no: i32) -> Option<FloatPoint> {
        match self {
            TileShape::RegularTileShape(rs) => Some(rs.corner_approx(no)),
            TileShape::Simplex(s) => s.corner_approx(no),
        }
    }

    /// Returns the approximations of all corners.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        match self {
            TileShape::RegularTileShape(rs) => rs.corner_approx_arr(),
            TileShape::Simplex(s) => s.corner_approx_arr(),
        }
    }

    /// Returns true, if the no-th corner is finite (regular tiles:
    /// always).
    pub fn corner_is_bounded(&self, no: i32) -> bool {
        match self {
            TileShape::RegularTileShape(rs) => rs.corner_is_bounded(no),
            TileShape::Simplex(s) => s.corner_is_bounded(no),
        }
    }

    /// Returns true if the regular arm is a box (Java `isIntBox`).
    pub fn is_int_box(&self) -> bool {
        match self {
            TileShape::RegularTileShape(rs) => rs.is_int_box(),
            TileShape::Simplex(s) => s.is_int_box(),
        }
    }

    /// Returns true if the regular arm is an octagon (Java
    /// `isIntOctagon`).
    pub fn is_int_octagon(&self) -> bool {
        match self {
            TileShape::RegularTileShape(rs) => rs.is_int_octagon(),
            TileShape::Simplex(s) => s.is_int_octagon(),
        }
    }

    /// Returns a unique ID of this shape for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        match self {
            TileShape::RegularTileShape(rs) => rs.get_id(),
            TileShape::Simplex(s) => s.get_id(),
        }
    }

    /// Returns the smallest surrounding box (Java `boundingBox`).
    pub fn bounding_box(&self) -> IntBox {
        match self {
            TileShape::RegularTileShape(rs) => rs.bounding_box(),
            TileShape::Simplex(s) => s.bounding_box(),
        }
    }

    /// True if this shape is contained in the box (Java
    /// `PolylineShape.isContainedIn(IntBox)`, `PolylineShape.java:136-139`):
    /// that overload is just `box.contains(boundingBox())`, and
    /// `IntBox.contains(RegularTileShape)` (`IntBox.java:377-379`) is
    /// reverse dispatch — `boundingBox().isContainedIn(box)`. An empty
    /// bounding box counts as contained (`IntBox::is_contained_in` keeps
    /// Java's empty-shape shortcut).
    pub fn is_contained_in_int_box(&self, board_box: &IntBox) -> bool {
        self.bounding_box().is_contained_in(board_box)
    }

    /// Returns the smallest surrounding octagon, or `None` where Java
    /// returns null (unbounded simplex).
    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        match self {
            TileShape::RegularTileShape(rs) => Some(rs.bounding_octagon()),
            TileShape::Simplex(s) => s.bounding_octagon(),
        }
    }

    /// Returns the smallest surrounding tile shape.
    pub fn bounding_tile(&self) -> TileShape {
        match self {
            TileShape::RegularTileShape(rs) => rs.bounding_tile(),
            TileShape::Simplex(s) => s.bounding_tile(),
        }
    }

    /// Returns the smallest surrounding regular tile in the given bounding
    /// directions, or `None` where Java returns null (unbounded simplex).
    pub fn bounding_shape(&self, directions: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        match self {
            TileShape::RegularTileShape(rs) => Some(rs.bounding_shape(directions)),
            TileShape::Simplex(s) => s.bounding_shape(directions),
        }
    }

    /// Converts the physical instance of this shape to a simpler physical
    /// instance: a box-like shape to its `IntBox`, identity otherwise
    /// (Java `simplify`).
    pub fn simplify(&self) -> TileShape {
        match self {
            TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => {
                // Java returns this.
                TileShape::RegularTileShape(RegularTileShape::IntBox(*b))
            }
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(o)) => o.simplify(),
            TileShape::Simplex(s) => s.simplify(),
        }
    }

    /// Converts this shape to a simplex (regular tiles through their
    /// border lines, the simplex arm by clone).
    pub fn to_simplex(&self) -> Simplex {
        match self {
            TileShape::RegularTileShape(rs) => rs.to_simplex(),
            TileShape::Simplex(s) => s.to_simplex(),
        }
    }

    /// Returns the area of this shape (Java `TileShape.area`): the leaf
    /// specializations for the regular arm, the rolling shoelace sum for
    /// the simplex arm. Unbounded shapes yield `Double.MAX_VALUE` and
    /// shapes of dimension < 2 yield 0.
    pub fn area(&self) -> f64 {
        match self {
            TileShape::RegularTileShape(rs) => rs.area(),
            TileShape::Simplex(s) => tile_area(&ShapeRef::Simplex(s)),
        }
    }

    /// Returns the circumference of the shape border (Java
    /// `PolylineShape.circumference`): `Integer.MAX_VALUE` =
    /// `2147483647.0` when the shape is unbounded, otherwise the sum of
    /// the corner-to-corner edge lengths.
    pub fn circumference(&self) -> f64 {
        match self {
            TileShape::RegularTileShape(rs) => rs.circumference(),
            TileShape::Simplex(s) => s.circumference(),
        }
    }

    /// Returns the maximum of the edge widths of the shape (Java
    /// `TileShape.length`): `Integer.MAX_VALUE` = `2147483647.0` when
    /// unbounded, 0.0 for dimension < 1, `circumference() / 2` for
    /// dimension 1, and the sum of the two biggest border-line distances
    /// from the gravity point for dimension 2 (the diagonal edges of an
    /// octagon dominate, pin PW3).
    pub fn length(&self) -> f64 {
        if !self.is_bounded() {
            return 2147483647.0;
        }
        let dimension = self.dimension();
        if dimension <= 0 {
            return 0.0;
        }
        if dimension == 1 {
            return self.circumference() / 2.0;
        }
        // now the shape is 2-dimensional
        let mut max_distance = -1.0f64;
        let mut max_distance2 = -1.0f64;
        let gravity_point = self.centre_of_gravity();
        for i in 0..self.border_line_count() as i32 {
            let current_distance = self.border_line(i).signed_distance(&gravity_point).abs();
            if current_distance > max_distance {
                max_distance2 = max_distance;
                max_distance = current_distance;
            } else if current_distance > max_distance2 {
                max_distance2 = current_distance;
            }
        }
        max_distance + max_distance2
    }

    /// Returns the gravity point of this shape (Java `centreOfGravity`).
    pub fn centre_of_gravity(&self) -> FloatPoint {
        match self {
            TileShape::RegularTileShape(rs) => rs.centre_of_gravity(),
            TileShape::Simplex(s) => shape_centre_of_gravity(&ShapeRef::Simplex(s)),
        }
    }

    /// Returns the maximal extension of this shape (Java `maxWidth`).
    pub fn max_width(&self) -> f64 {
        match self {
            TileShape::RegularTileShape(rs) => rs.max_width(),
            TileShape::Simplex(s) => s.max_width(),
        }
    }

    /// Returns the minimal extension of this shape (Java `minWidth`).
    pub fn min_width(&self) -> f64 {
        match self {
            TileShape::RegularTileShape(rs) => rs.min_width(),
            TileShape::Simplex(s) => s.min_width(),
        }
    }

    /// Returns the radius of the biggest circle with center
    /// `centre_of_gravity()` contained in this shape (Java
    /// `smallestRadius`).
    pub fn smallest_radius(&self) -> f64 {
        self.border_distance(&self.centre_of_gravity())
    }

    // -----------------------------------------------------------------
    // Containment
    // -----------------------------------------------------------------

    /// Returns true, if this shape is completely outside of the half
    /// plane defined by `point` (Java `isOutside`).
    pub fn is_outside(&self, point: &Point) -> bool {
        tile_is_outside(&ShapeRef::from(self), point)
    }

    /// Returns true, if `point` is contained in this shape or on its
    /// border (Java `contains(Point)` == `!isOutside`).
    pub fn contains_point(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    /// Returns true, if the float point is contained in this shape
    /// (Java `contains(FloatPoint)`; border points do NOT count, pin
    /// P14 — EXCEPT for octagons: Java `IntOctagon` OVERRIDES the
    /// one-arg `contains(FloatPoint)` (`IntOctagon.java:332-344`) with
    /// direct `<=` arithmetic that INCLUDES the border, and the
    /// locator's `shrinkedRoomShape.contains(addCorner)` call is a
    /// virtual one-arg dispatch that selects that override. IntBox and
    /// Simplex do not override, so the strict `TileShape
    /// .contains(FloatPoint, 0)` border loop applies there (P14 pins a
    /// box).
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        match self {
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct)) => oct.contains(point),
            _ => tile_contains_float(&ShapeRef::from(self), point, 0.0),
        }
    }

    /// Returns true, if the float point is contained in this shape with
    /// the given tolerance (Java `contains(FloatPoint, double)`).
    pub fn contains_float_tolerance(&self, point: &FloatPoint, tolerance: f64) -> bool {
        tile_contains_float(&ShapeRef::from(self), point, tolerance)
    }

    /// Returns true, if all corners of `other` are contained in this
    /// shape (Java `contains(TileShape)`).
    pub fn contains_tile(&self, other: &TileShape) -> bool {
        for i in 0..other.border_line_count() as i32 {
            if !self.contains_point(&other.corner(i)) {
                return false;
            }
        }
        true
    }

    /// Returns true, if all approximate corners of `other` are contained
    /// in this shape (Java `containsApprox`).
    pub fn contains_approx(&self, other: &TileShape) -> bool {
        for corner in other.corner_approx_arr() {
            if !self.contains_float(&corner) {
                return false;
            }
        }
        true
    }

    /// Returns true, if `point` is contained in the interior of this
    /// shape, that is the border does not count (Java `containsInside`).
    pub fn contains_inside(&self, point: &Point) -> bool {
        tile_contains_inside(&ShapeRef::from(self), point)
    }

    /// If `point` lies on the border of this shape, the number of the
    /// edge line containing `point` is returned, otherwise -1 (Java
    /// `containsOnBorderLineNo`).
    pub fn contains_on_border_line_no(&self, point: &Point) -> i32 {
        tile_contains_on_border_line_no(&ShapeRef::from(self), point)
    }

    /// Returns true, if `point` lies on the border of this shape (Java
    /// `containsOnBorder`).
    pub fn contains_on_border(&self, point: &Point) -> bool {
        tile_contains_on_border(&ShapeRef::from(self), point)
    }

    /// Returns Side::Negative if `point` is inside this shape,
    /// Side::Collinear if it lies on the border (within `tolerance`) and
    /// Side::Positive if it is outside (Java `sideOfBorder`).
    pub fn side_of_border(&self, point: &FloatPoint, tolerance: f64) -> Side {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return Side::Collinear;
        }
        let mut result = Side::Negative; // point is inside
        for i in 0..line_count as i32 {
            let current_side = self.border_line(i).side_of_float(point, tolerance);
            if current_side == Side::Positive {
                return Side::Positive; // point is outside
            } else if current_side == Side::Collinear {
                result = current_side;
            }
        }
        result
    }

    /// Returns Side::Positive (Java `ON_THE_LEFT`) if this shape is
    /// completely on the left of `line`, Side::Negative if completely on
    /// the right, and Side::Collinear if the line cuts this shape
    /// (Java `sideOf(Line)`, TileShape.java:645).
    pub fn side_of_line(&self, line: &Line) -> Side {
        let mut on_the_left = false;
        let mut on_the_right = false;
        for i in 0..self.border_line_count() as i32 {
            let current_side = line.side_of(&self.corner(i));
            if current_side == Side::Positive {
                on_the_right = true;
            } else if current_side == Side::Negative {
                on_the_left = true;
            }
            if on_the_left && on_the_right {
                return Side::Collinear;
            }
        }
        if on_the_left {
            Side::Positive
        } else {
            Side::Negative
        }
    }

    // -----------------------------------------------------------------
    // Distances and nearest points
    // -----------------------------------------------------------------

    /// Returns the nearest point of this shape to `from_point`
    /// (Java `nearestPoint`): `from_point` itself if contained, else the
    /// nearest border point. `None` for the empty shape (Java returns
    /// null).
    pub fn nearest_point(&self, from_point: &Point) -> Option<Point> {
        if !self.is_outside(from_point) {
            return Some(from_point.clone());
        }
        self.nearest_border_point(from_point)
    }

    /// Returns an approximation of the nearest point of this shape to
    /// `from_point` (Java `nearestPointApprox`).
    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        tile_nearest_point_approx(&ShapeRef::from(self), from_point)
    }

    /// Returns the nearest point of the border of this shape to
    /// `from_point` (Java `nearestBorderPoint`); `None` for the empty
    /// shape (Java returns null). Panics on unbounded simplex corners
    /// (Java NPE).
    pub fn nearest_border_point(&self, from_point: &Point) -> Option<Point> {
        let line_count = self.border_line_count();
        if line_count == 0 {
            return None;
        }
        let from_point_f = from_point.to_float();
        if line_count == 1 {
            return Some(self.border_line(0).perpendicular_projection(from_point));
        }
        // Check the corners first.
        let mut min_dist = f64::MAX;
        let mut min_dist_ind = 0i32;
        for i in 0..line_count as i32 {
            let current_corner_f = self
                .corner_approx(i)
                .expect("nearestBorderPoint: Java NPE on the null cornerApprox");
            let current_distance = current_corner_f.distance_square(&from_point_f);
            if current_distance < min_dist {
                min_dist = current_distance;
                min_dist_ind = i;
            }
        }
        let mut nearest_point = self.corner(min_dist_ind);
        // Now check, if a point on a border line is nearer.
        let mut prev_ind = line_count as i32 - 2;
        let mut current_ind = line_count as i32 - 1;
        for next_ind in 0..line_count as i32 {
            let projection = self
                .border_line(current_ind)
                .perpendicular_projection(from_point);
            if (!self.corner_is_bounded(current_ind)
                || self.border_line(prev_ind).side_of(&projection) == Side::Negative)
                && (!self.corner_is_bounded(next_ind)
                    || self.border_line(next_ind).side_of(&projection) == Side::Negative)
            {
                let current_distance = projection.to_float().distance_square(&from_point_f);
                if current_distance < min_dist {
                    min_dist = current_distance;
                    nearest_point = projection;
                }
            }
            prev_ind = current_ind;
            current_ind = next_ind;
        }
        Some(nearest_point)
    }

    /// Returns an approximation of the nearest point of the border of
    /// this shape (Java `nearestBorderPointApprox`).
    pub fn nearest_border_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        shape_nearest_border_point_approx(&ShapeRef::from(self), from_point)
    }

    /// Returns up to `count` nearest border points of this shape to
    /// `from_point` (Java `nearestBorderPointsApprox`); ties keep the
    /// earlier candidate (strict `<` insertion, pin PW10).
    pub fn nearest_border_points_approx(
        &self,
        from_point: &FloatPoint,
        count: i32,
    ) -> Vec<FloatPoint> {
        shape_nearest_border_points_approx(&ShapeRef::from(self), from_point, count)
    }

    /// Returns the smallest distance from `point` to this shape (Java
    /// `distance`).
    pub fn distance(&self, point: &FloatPoint) -> f64 {
        self.nearest_point_approx(point).distance(point)
    }

    /// Returns the smallest distance from `point` to the border of this
    /// shape (Java `borderDistance`).
    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        self.nearest_border_point_approx(point).distance(point)
    }

    // -----------------------------------------------------------------
    // Construction / transformation
    // -----------------------------------------------------------------

    /// Returns this shape offsetted by `width`; a positive offset grows,
    /// a negative shrinks (Java `offset`). `tile_shrink` keeps the
    /// empty-result fallback to the gravity-point box (TileShape.shrink).
    pub fn offset(&self, width: f64) -> TileShape {
        tile_offset(&ShapeRef::from(self), width)
    }

    /// Returns this shape enlarged by `width` (Java `enlarge`).
    pub fn enlarge(&self, width: f64) -> TileShape {
        match self {
            TileShape::RegularTileShape(rs) => TileShape::RegularTileShape(rs.enlarge(width)),
            TileShape::Simplex(s) => TileShape::Simplex(Box::new(s.enlarge(width))),
        }
    }

    /// Returns this shape shrunk by `width`; an empty result falls back
    /// to the box around the gravity point (Java `shrink`).
    pub fn shrink(&self, offset: f64) -> TileShape {
        tile_shrink(&ShapeRef::from(self), offset)
    }

    /// Turns this shape by `factor` times 90 degrees around `pole`
    /// (Java `turn90Degree`, TileShape.java:669: rotate every border line
    /// and rebuild with `getInstance`, i.e. simplify). The `IntBox` arm
    /// keeps its coordinate-swap specialization.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape {
        match self {
            TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => {
                TileShape::RegularTileShape(RegularTileShape::IntBox(
                    b.turn_90_degree(factor, pole),
                ))
            }
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(o)) => {
                turn_90_degree_via_border_lines(
                    o.border_line_count() as usize,
                    |i| o.border_line(i),
                    factor,
                    pole,
                )
            }
            TileShape::Simplex(s) => turn_90_degree_via_border_lines(
                s.border_line_count(),
                |i| s.border_line(i),
                factor,
                pole,
            ),
        }
    }

    /// Mirrors this shape at the vertical line through `pole` (Java
    /// `mirrorVertical`).
    pub fn mirror_vertical(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(
            self.border_line_count(),
            |i| self.border_line(i),
            pole,
            true,
        )
    }

    /// Mirrors this shape at the horizontal line through `pole` (Java
    /// `mirrorHorizontal`).
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(
            self.border_line_count(),
            |i| self.border_line(i),
            pole,
            false,
        )
    }

    /// Returns the translation of this shape by `vector` (Java
    /// `translateBy`).
    pub fn translate_by(&self, vector: &Vector) -> TileShape {
        match self {
            TileShape::RegularTileShape(rs) => TileShape::RegularTileShape(rs.translate_by(vector)),
            TileShape::Simplex(s) => TileShape::Simplex(Box::new(s.translate_by(vector))),
        }
    }

    /// Tile shapes are convex: splitting yields the shape itself (Java
    /// `splitToConvex`).
    pub fn split_to_convex(&self) -> Vec<TileShape> {
        vec![self.clone()]
    }

    /// Returns the intersecting border line number when following
    /// `p_direction` from `point` inside this shape, or -1 (Java
    /// `intersectingBorderLineNo`). The minimum-distance seed is Java's
    /// `Float.MAX_VALUE` widened to double.
    pub fn intersecting_border_line_no(&self, point: &Point, direction: &Direction) -> i32 {
        if !self.contains_point(point) {
            return -1;
        }
        let from_point = point.to_float();
        let intersection_line = Line::new_with_direction(point.clone(), direction.clone());
        let second_line_point = intersection_line.b.to_float();
        let mut result = -1;
        let mut min_distance = f64::from(f32::MAX); // Java Float.MAX_VALUE
        for i in 0..self.border_line_count() as i32 {
            let current_border_line = self.border_line(i);
            let current_intersection = current_border_line.intersection_approx(&intersection_line);
            if current_intersection.x >= f64::from(i32::MAX) {
                // the lines are parallel
                continue;
            }
            let current_distance = current_intersection.distance_square(&from_point);
            if current_distance < min_distance {
                let direction_ok = current_border_line.side_of_float_zero(&second_line_point)
                    == Side::Positive
                    || second_line_point.distance_square(&current_intersection) < current_distance;
                if direction_ok {
                    result = i;
                    min_distance = current_distance;
                }
            }
        }
        result
    }

    // -----------------------------------------------------------------
    // Set operations (3x3 dispatch tables)
    // -----------------------------------------------------------------

    /// Returns the intersection of this shape and `other`. Every arm is
    /// the Java overload the dispatch selects; the cross-type arms keep
    /// the reverse-dispatch mirrors verbatim (IntBox.java:324,
    /// IntOctagon.java:373).
    pub fn intersection(&self, other: &TileShape) -> TileShape {
        match (self, other) {
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => TileShape::RegularTileShape(RegularTileShape::IntBox(a.intersection(b))),
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => {
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a.intersection_octagon(b)))
            }
            (TileShape::RegularTileShape(RegularTileShape::IntBox(a)), TileShape::Simplex(s)) => {
                // Java IntBox.intersection(Simplex):
                // other.intersection(this.toSimplex()).
                TileShape::Simplex(Box::new(a.intersection_simplex(s)))
            }
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => TileShape::RegularTileShape(RegularTileShape::IntOctagon(a.intersection_box(b))),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => TileShape::RegularTileShape(RegularTileShape::IntOctagon(a.intersection(b))),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::Simplex(s),
            ) => {
                // Java IntOctagon.intersection(Simplex):
                // other.intersection(this).
                TileShape::Simplex(Box::new(a.intersection_simplex(s)))
            }
            (TileShape::Simplex(s), TileShape::RegularTileShape(RegularTileShape::IntBox(b))) => {
                TileShape::Simplex(Box::new(s.intersection_box(b)))
            }
            (
                TileShape::Simplex(s),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => TileShape::Simplex(Box::new(s.intersection_octagon(b))),
            (TileShape::Simplex(s), TileShape::Simplex(o)) => {
                TileShape::Simplex(Box::new(s.intersection_simplex(o)))
            }
        }
    }

    /// Returns true, if this shape and `other` have a nonempty
    /// intersection (Java `intersects`). Every arm is the Java overload
    /// the `other.intersects(this)` reverse dispatch selects.
    pub fn intersects(&self, other: &TileShape) -> bool {
        match (self, other) {
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => a.intersects(b),
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => a.intersects_octagon(b),
            (TileShape::RegularTileShape(RegularTileShape::IntBox(a)), TileShape::Simplex(s)) => {
                s.intersects_box(a)
            }
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => a.intersects_box(b),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => a.intersects(b),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::Simplex(s),
            ) => s.intersects_octagon(a),
            (TileShape::Simplex(s), TileShape::RegularTileShape(RegularTileShape::IntBox(b))) => {
                s.intersects_box(b)
            }
            (
                TileShape::Simplex(s),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => s.intersects_octagon(b),
            (TileShape::Simplex(s), TileShape::Simplex(o)) => s.intersects_simplex(o),
        }
    }

    /// Returns the intersection of this shape and `other`, simplified
    /// (Java `intersectionWithSimplify`).
    pub fn intersection_with_simplify(&self, other: &TileShape) -> TileShape {
        self.intersection(other).simplify()
    }

    /// Returns the pieces of `outer` which result from cutting this shape
    /// out of it (Java `cutoutFrom`; the receiver is the cutter, the
    /// argument is the shape the pieces belong to). Every arm is the Java
    /// overload the dispatch selects.
    pub fn cutout_from(&self, outer: &TileShape) -> Vec<TileShape> {
        match (self, outer) {
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => a
                .cutout_from(b)
                .into_iter()
                .map(|p| TileShape::RegularTileShape(RegularTileShape::IntBox(p)))
                .collect(),
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => a
                .cutout_from_octagon(b)
                .into_iter()
                .map(|p| TileShape::RegularTileShape(RegularTileShape::IntOctagon(p)))
                .collect(),
            (TileShape::RegularTileShape(RegularTileShape::IntBox(a)), TileShape::Simplex(s)) => a
                .cutout_from_simplex(s)
                .into_iter()
                .map(|p| TileShape::Simplex(Box::new(p)))
                .collect(),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)),
            ) => a
                .cutout_from_box(b)
                .into_iter()
                .map(|p| TileShape::RegularTileShape(RegularTileShape::IntOctagon(p)))
                .collect(),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => a
                .cutout_from(b)
                .into_iter()
                .map(|p| TileShape::RegularTileShape(RegularTileShape::IntOctagon(p)))
                .collect(),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(a)),
                TileShape::Simplex(s),
            ) => a
                .cutout_from_simplex(s)
                .into_iter()
                .map(|p| TileShape::Simplex(Box::new(p)))
                .collect(),
            (TileShape::Simplex(s), TileShape::RegularTileShape(RegularTileShape::IntBox(b))) => s
                .cutout_from_box(b)
                .into_iter()
                .map(|p| TileShape::Simplex(Box::new(p)))
                .collect(),
            (
                TileShape::Simplex(s),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(b)),
            ) => s
                .cutout_from_octagon(b)
                .into_iter()
                .map(|p| TileShape::Simplex(Box::new(p)))
                .collect(),
            (TileShape::Simplex(s), TileShape::Simplex(o)) => s
                .cutout_from_simplex(o)
                .into_iter()
                .map(|p| TileShape::Simplex(Box::new(p)))
                .collect(),
        }
    }

    /// Cuts this shape out of `shape`, i.e. returns the pieces of `shape`
    /// outside this shape. Java `TileShape.cutout` is abstract with
    /// per-leaf bodies, and the leaves are NOT uniform: `IntBox.cutout`
    /// (`IntBox.java:688-695`) returns `shape.cutoutFrom(this)` with each
    /// result SIMPLIFIED, while `IntOctagon.cutout`
    /// (`IntOctagon.java:1059-1061`) and `Simplex.cutout`
    /// (`Simplex.java:695-697`) return the pieces UNSIMPLIFIED. The
    /// simplify belongs to the receiver's dynamic type — simplifying
    /// unconditionally would re-type a box-like `IntOctagon` result to
    /// `IntBox` and flip the NEXT round's `cutoutFrom` dispatch arm (a
    /// different decomposition, e.g. the box 4-piece vs octagon 8-piece
    /// split).
    pub fn cutout(&self, shape: &TileShape) -> Vec<TileShape> {
        let pieces = shape.cutout_from(self);
        if matches!(
            self,
            TileShape::RegularTileShape(RegularTileShape::IntBox(_))
        ) {
            pieces.into_iter().map(|piece| piece.simplify()).collect()
        } else {
            pieces
        }
    }

    // -------------------------------------------------------------------
    // Task 8 ledger closures (TileShape.java methods that needed the
    // Polyline / Polygon / FloatLine family from Tasks 2-8).
    // -------------------------------------------------------------------

    /// Returns the number of the nearest corner of the shape to from_point
    /// (Java `indexOfNearestCorner`).
    ///
    /// Trap T21 (oracle bug kept for parity): Java seeds the running
    /// minimum with `Double.MIN_VALUE` — the smallest POSITIVE double
    /// (4.9e-324), not the most negative one. A distance can only beat
    /// that seed when it is exactly 0, so for any from_point that does not
    /// coincide with a corner the result is pinned to index 0. The port
    /// reproduces the seed with `f64::from_bits(1)`.
    pub fn index_of_nearest_corner(&self, from_point: &Point) -> i32 {
        let from_point_f = from_point.to_float();
        let mut result = 0;
        let corner_count = self.border_line_count();
        let mut min_dist = f64::from_bits(1); // Java Double.MIN_VALUE bug-compat
        for i in 0..corner_count as i32 {
            let current_distance = self
                .corner_approx(i)
                .expect("corner loop of a non-empty shape")
                .distance(&from_point_f);
            if current_distance < min_dist {
                min_dist = current_distance;
                result = i;
            }
        }
        result
    }

    /// Returns the index of the corner that is the left most one when
    /// this shape is viewed from `from_point` (Java
    /// `PolylineShape.indexOfLeftMostCorner`, inherited by TileShape —
    /// no TileShape override). Ties keep the earlier corner (strict
    /// `Positive` update), exactly like the PolylineShape loop.
    pub fn index_of_left_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut left_most_corner = self
            .corner_approx(0)
            .expect("corner loop of a non-empty shape");
        let corner_count = self.border_line_count();
        let mut result = 0;
        for i in 1..corner_count as i32 {
            let current_corner = self
                .corner_approx(i)
                .expect("corner loop of a non-empty shape");
            if current_corner.side_of(from_point, &left_most_corner) == Side::Positive {
                left_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Returns the index of the corner that is the right most one when
    /// this shape is viewed from `from_point` (Java
    /// `PolylineShape.indexOfRightMostCorner`, inherited by TileShape —
    /// no TileShape override). Ties keep the earlier corner (strict
    /// `Negative` update).
    pub fn index_of_right_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut right_most_corner = self
            .corner_approx(0)
            .expect("corner loop of a non-empty shape");
        let corner_count = self.border_line_count();
        let mut result = 0;
        for i in 1..corner_count as i32 {
            let current_corner = self
                .corner_approx(i)
                .expect("corner loop of a non-empty shape");
            if current_corner.side_of(from_point, &right_most_corner) == Side::Negative {
                right_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Returns a line segment consisting of approximations of the corners
    /// with index 0 and cornerCount / 2 (Java `diagonalCornerSegment`);
    /// `None` where Java returns null (empty shape).
    pub fn diagonal_corner_segment(&self) -> Option<FloatLine> {
        if self.is_empty() {
            return None;
        }
        let first_corner = self.corner_approx(0)?;
        let last_corner = self.corner_approx(self.border_line_count() as i32 / 2)?;
        Some(FloatLine::new(first_corner, last_corner))
    }

    /// Returns a [`FloatLine`] whose `.a` approximates the left most
    /// corner of this shape when viewed from `from_point`, and whose
    /// `.b` approximates the right most corner (Java
    /// `PolylineShape.polarLineSegment`, `PolylineShape.java:182-200`);
    /// `None` where Java returns null (empty shape).
    pub fn polar_line_segment(&self, from_point: &FloatPoint) -> Option<FloatLine> {
        if self.is_empty() {
            return None;
        }
        let mut left_most_corner = self.corner_approx(0)?;
        let mut right_most_corner = self.corner_approx(0)?;
        let corner_count = self.border_line_count();
        for i in 1..corner_count {
            let current_corner = self.corner_approx(i as i32)?;
            if current_corner.side_of(from_point, &right_most_corner) == Side::Negative {
                right_most_corner = current_corner;
            }
            if current_corner.side_of(from_point, &left_most_corner) == Side::Positive {
                left_most_corner = current_corner;
            }
        }
        Some(FloatLine::new(left_most_corner, right_most_corner))
    }

    /// Returns an approximation of the count nearest relative outside
    /// locations of shape in the direction of different border lines of
    /// this shape, sorted ascending (Java
    /// `nearestRelativeOutsideLocations`).
    pub fn nearest_relative_outside_locations(
        &self,
        shape: &TileShape,
        count: i32,
    ) -> Vec<FloatPoint> {
        let line_count = self.border_line_count() as i32;
        if count <= 0 || line_count < 3 || !self.intersects(shape) {
            return Vec::new();
        }
        let result_count = count.min(line_count) as usize;
        let mut translate_coors = vec![FloatPoint::ZERO; result_count];
        let mut min_dists = vec![f64::MAX; result_count];

        let mut current_ind = line_count - 1;
        let other_line_count = shape.border_line_count() as i32;

        for next_ind in 0..line_count {
            let mut current_max_dist = 0.0f64;
            let mut current_translate_coor = FloatPoint::ZERO;
            for corner_index in 0..other_line_count {
                let current_corner = shape
                    .corner_approx(corner_index)
                    .expect("non-empty other shape");
                if self
                    .border_line(current_ind)
                    .side_of_float_zero(&current_corner)
                    == Side::Negative
                {
                    let projection =
                        current_corner.projection_approx(&self.border_line(current_ind));
                    let current_distance = projection.distance_square(&current_corner);
                    if current_distance > current_max_dist {
                        current_max_dist = current_distance;
                        current_translate_coor = projection.subtract(&current_corner);
                    }
                }
            }
            for j in 0..result_count {
                if current_max_dist < min_dists[j] {
                    let mut k = j + 1;
                    while k < result_count {
                        min_dists[k] = min_dists[k - 1];
                        translate_coors[k] = translate_coors[k - 1];
                        k += 1;
                    }
                    min_dists[j] = current_max_dist;
                    translate_coors[j] = current_translate_coor;
                    break;
                }
            }
            current_ind = next_ind;
        }
        translate_coors
    }

    /// Calculates whether this shape and other have a common border piece
    /// and returns the indices of the touching edge lines (Java
    /// `touchingSides`); empty when they do not touch.
    pub fn touching_sides(&self, other: &TileShape) -> Vec<i32> {
        use std::cmp::Ordering as Ord;
        // search the first edge line of other with reverse direction >= right
        let mut side_no2: i32 = -1;
        let mut dir2: Option<Direction> = None;
        for i in 0..other.border_line_count() as i32 {
            let current_direction = other.border_line(i).direction().clone();
            if current_direction.compare_to(&Direction::LEFT) != Ord::Less {
                side_no2 = i;
                dir2 = Some(current_direction.opposite());
                break;
            }
        }
        let mut dir2 = match dir2 {
            Some(d) => d,
            // Java: FRLogger.warn("touching_side : dir2 not found")
            None => return Vec::new(),
        };
        let mut side_no1: i32 = 0;
        let mut dir1 = self.border_line(0).direction().clone();
        let max_ind = self.border_line_count() as i32 + other.border_line_count() as i32;

        for _ in 0..max_ind {
            let compare = dir2.compare_to(&dir1);
            if compare == Ord::Equal
                && self
                    .border_line(side_no1)
                    .is_equal_or_opposite(&other.border_line(side_no2))
            {
                return vec![side_no1, side_no2];
            }
            if compare != Ord::Less {
                // dir2 is bigger than dir1
                side_no1 = (side_no1 + 1) % self.border_line_count() as i32;
                dir1 = self.border_line(side_no1).direction().clone();
            } else {
                // dir1 is bigger than dir2
                side_no2 = (side_no2 + 1) % other.border_line_count() as i32;
                dir2 = other.border_line(side_no2).direction().opposite();
            }
        }
        Vec::new()
    }

    /// Calculates the minimal distance of line to this shape, assuming
    /// that line is on the left of this shape. Returns -1 if line is on
    /// the right of this shape or intersects with its interior (Java
    /// `distanceToTheLeft`).
    pub fn distance_to_the_left(&self, line: &Line) -> f64 {
        let mut result = 2147483647.0f64; // Java Integer.MAX_VALUE
        for i in 0..self.border_line_count() as i32 {
            let current_corner = self
                .corner_approx(i)
                .expect("corner loop of a non-empty shape");
            let mut line_side = line.side_of_float(&current_corner, 1.0);
            if line_side == Side::Collinear {
                line_side = line.side_of(&self.corner(i));
            }
            if line_side == Side::Negative {
                // currentPoint would be outside the result shape
                return -1.0;
            }
            result = result.min(line.signed_distance(&current_corner));
        }
        result
    }

    /// Returns an approximation of this shape rotated around pole (Java
    /// `TileShape.rotateApprox`): the rounded corners rebuild a polygon
    /// whose instance shape depends on the remaining corner count.
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> TileShape {
        if angle == 0.0 {
            return self.clone();
        }
        let new_corners: Vec<Point> = (0..self.border_line_count() as i32)
            .map(|i| {
                Point::Int(
                    self.corner_approx(i)
                        .expect("corner loop of a non-empty shape")
                        .rotate(angle, pole)
                        .round(),
                )
            })
            .collect();
        let corner_polygon = crate::polygon::Polygon::new(&new_corners);
        let polygon_corners = corner_polygon.corner_array();
        if polygon_corners.len() >= 3 {
            TileShape::from_points(polygon_corners)
        } else if polygon_corners.len() == 2 {
            let current_polyline = crate::polyline::Polyline::from_points(polygon_corners);
            let current_segment =
                crate::line_segment::LineSegment::from_polyline(&current_polyline, 0);
            TileShape::Simplex(Box::new(current_segment.to_simplex()))
        } else if polygon_corners.len() == 1 {
            TileShape::RegularTileShape(RegularTileShape::IntBox(TileShape::surrounding_point(
                &polygon_corners[0],
            )))
        } else {
            TileShape::Simplex(Box::new(Simplex::empty()))
        }
    }

    /// Returns tuples (polyline line no, border line no) of the points
    /// where polyline enters or leaves the interior of this shape (Java
    /// `entrancePoints`).
    pub fn entrance_points(&self, polyline: &crate::polyline::Polyline) -> Vec<(i32, i32)> {
        let mut result: Vec<(i32, i32)> = Vec::new();
        let mut prev_intersection: Option<(i32, i32)> = None;
        // Java TileShape.java:872:
        // lineIndex < polyline.lines.length - 1.
        for line_index in 1..(polyline.lines.len() as i32).wrapping_sub(1) {
            let current_line_seg =
                crate::line_segment::LineSegment::from_polyline(polyline, line_index);
            let current_intersections = current_line_seg.border_intersections(self);
            for edge_index in current_intersections {
                if prev_intersection != Some((line_index, edge_index)) {
                    result.push((line_index, edge_index));
                    prev_intersection = Some((line_index, edge_index));
                }
            }
        }
        result
    }

    /// Cuts out the parts of polyline in the interior of this shape and
    /// returns the remaining pieces of polyline (Java
    /// `TileShape.cutout(Polyline)`). Pieces completely contained in the
    /// border are not returned.
    pub fn cutout_polyline(
        &self,
        polyline: &crate::polyline::Polyline,
    ) -> Vec<crate::polyline::Polyline> {
        let intersection_no = self.entrance_points(polyline);
        let first_corner = polyline
            .first_corner()
            .expect("Java NPE: polyline without corners");
        let first_corner_is_inside = self.contains_inside(&first_corner);
        let mut pieces: Vec<crate::polyline::Polyline> = Vec::new();
        if intersection_no.is_empty() {
            // no intersections
            if first_corner_is_inside {
                // polyline is contained completely in this shape
                return pieces;
            }
            // polyline is completely outside
            pieces.push(polyline.clone());
            return pieces;
        }
        let mut current_intersection_no: usize = 0;
        let current_intersection_tuple = intersection_no[current_intersection_no];
        let first_intersection = polyline.lines[current_intersection_tuple.0 as usize]
            .intersection(&self.border_line(current_intersection_tuple.1))
            .expect("entrance lines intersect");
        if !first_corner_is_inside {
            // calculate outside piece at start
            if first_corner != first_intersection {
                // otherwise skip 1 point outside polyline at the start
                let current_polyline_intersection_no = current_intersection_tuple.0;
                let mut current_lines: Vec<Line> =
                    polyline.lines[0..=(current_polyline_intersection_no) as usize].to_vec();
                // close the polyline piece with the intersected edge line.
                current_lines.push(self.border_line(current_intersection_tuple.1));
                let current_piece = crate::polyline::Polyline::new(current_lines);
                if !current_piece.is_empty() {
                    pieces.push(current_piece);
                }
            }
            current_intersection_no += 1;
        }
        while current_intersection_no < intersection_no.len().wrapping_sub(1) {
            // calculate the next outside polyline piece
            let current_intersection_tuple = intersection_no[current_intersection_no];
            let next_intersection_tuple = intersection_no[current_intersection_no + 1];
            let current_intersection_no_of_polyline = current_intersection_tuple.0;
            let next_intersection_no_of_polyline = next_intersection_tuple.0;
            // check that at least 1 corner of polyline with number between
            // the two intersections is not contained in this shape.
            // Otherwise the part between the intersections is completely
            // contained in the border and can be ignored
            let mut insert_piece = false;
            for i in current_intersection_no_of_polyline + 1..next_intersection_no_of_polyline {
                if self.is_outside(&polyline.corner(i).expect("corner between intersections")) {
                    insert_piece = true;
                    break;
                }
            }
            if insert_piece {
                let mut current_lines: Vec<Line> = Vec::with_capacity(
                    (next_intersection_no_of_polyline - current_intersection_no_of_polyline + 3)
                        as usize,
                );
                current_lines.push(self.border_line(current_intersection_tuple.1));
                current_lines.extend(
                    polyline.lines[current_intersection_no_of_polyline as usize
                        ..(current_intersection_no_of_polyline + next_intersection_no_of_polyline
                            - current_intersection_no_of_polyline
                            + 1) as usize]
                        .iter()
                        .cloned(),
                );
                current_lines.push(self.border_line(next_intersection_tuple.1));
                let current_piece = crate::polyline::Polyline::new(current_lines);
                if !current_piece.is_empty() {
                    pieces.push(current_piece);
                }
            }
            current_intersection_no += 2;
        }
        if current_intersection_no <= intersection_no.len().wrapping_sub(1) {
            // calculate outside piece at end
            let current_intersection_tuple = intersection_no[current_intersection_no];
            let current_polyline_intersection_no = current_intersection_tuple.0;
            let mut current_lines: Vec<Line> = Vec::with_capacity(
                (polyline.lines.len() as i32 - current_polyline_intersection_no + 1) as usize,
            );
            current_lines.push(self.border_line(current_intersection_tuple.1));
            current_lines.extend(
                polyline.lines[current_polyline_intersection_no as usize..]
                    .iter()
                    .cloned(),
            );
            let current_piece = crate::polyline::Polyline::new(current_lines);
            if !current_piece.is_empty() {
                pieces.push(current_piece);
            }
        }
        pieces
    }

    /// Divides this shape into sections with width and height at most
    /// max_section_width of about equal size (Java `divideIntoSections`).
    pub fn divide_into_sections(&self, max_section_width: f64) -> Vec<TileShape> {
        if self.is_empty() {
            return vec![self.clone()];
        }
        let section_boxes = self.bounding_box().divide_into_sections(max_section_width);
        let mut section_list: Vec<TileShape> = Vec::new();
        for section_box in section_boxes {
            let current_section = self.intersection_with_simplify(&TileShape::RegularTileShape(
                RegularTileShape::IntBox(section_box),
            ));
            if current_section.dimension() == 2 {
                section_list.push(current_section);
            }
        }
        section_list
    }

    /// Checks if line_segment has a common point with the interior of this
    /// shape (Java `isIntersectedInteriorBy(LineSegment)`).
    pub fn is_intersected_interior_by(
        &self,
        line_segment: &crate::line_segment::LineSegment,
    ) -> bool {
        self.is_intersected_interior_by_points(
            &line_segment.start_point(),
            &line_segment.end_point(),
            line_segment.get_line(),
        )
    }

    /// Checks if the line segment defined by start_point, end_point and
    /// line has a common point with the interior of this shape (Java
    /// `isIntersectedInteriorBy(Point, Point, Line)`).
    pub fn is_intersected_interior_by_points(
        &self,
        start_point: &Point,
        end_point: &Point,
        line: &Line,
    ) -> bool {
        let float_start_point = start_point.to_float();
        let float_end_point = end_point.to_float();

        let border_line_count = self.border_line_count() as i32;
        let mut border_line_side_of_start_point_arr: Vec<Side> =
            Vec::with_capacity(border_line_count as usize);
        let mut border_line_side_of_end_point_arr: Vec<Side> =
            Vec::with_capacity(border_line_count as usize);
        for i in 0..border_line_count {
            let current_border_line = self.border_line(i);
            let mut border_line_side_of_start_point =
                current_border_line.side_of_float(&float_start_point, 1.0);
            if border_line_side_of_start_point == Side::Collinear {
                border_line_side_of_start_point = current_border_line.side_of(start_point);
            }
            let mut border_line_side_of_end_point =
                current_border_line.side_of_float(&float_end_point, 1.0);
            if border_line_side_of_end_point == Side::Collinear {
                border_line_side_of_end_point = current_border_line.side_of(end_point);
            }
            if border_line_side_of_start_point != Side::Negative
                && border_line_side_of_end_point != Side::Negative
            {
                // both endpoints are outside the borderLine,
                // no intersection possible
                return false;
            }
            border_line_side_of_start_point_arr.push(border_line_side_of_start_point);
            border_line_side_of_end_point_arr.push(border_line_side_of_end_point);
        }
        let mut start_point_is_inside = true;
        for &side in &border_line_side_of_start_point_arr {
            if side != Side::Negative {
                start_point_is_inside = false;
                break;
            }
        }
        if start_point_is_inside {
            return true;
        }
        let mut end_point_is_inside = true;
        for &side in &border_line_side_of_end_point_arr {
            if side != Side::Negative {
                end_point_is_inside = false;
                break;
            }
        }
        if end_point_is_inside {
            return true;
        }
        let segment_line = line;
        // Check, if this line segment intersects a border line of shape.
        for i in 0..border_line_count {
            let border_line_side_of_start_point = border_line_side_of_start_point_arr[i as usize];
            let border_line_side_of_end_point = border_line_side_of_end_point_arr[i as usize];
            if border_line_side_of_start_point != border_line_side_of_end_point {
                if (border_line_side_of_start_point == Side::Collinear
                    && border_line_side_of_end_point == Side::Positive)
                    || (border_line_side_of_end_point == Side::Collinear
                        && border_line_side_of_start_point == Side::Positive)
                {
                    // the interior of shape is not intersected.
                    continue;
                }
                let mut prev_corner_side = segment_line.side_of_float(
                    &self
                        .corner_approx(i)
                        .expect("corner loop of a non-empty shape"),
                    1.0,
                );
                if prev_corner_side == Side::Collinear {
                    prev_corner_side = segment_line.side_of(&self.corner(i));
                }
                let next_corner_index = if i == border_line_count - 1 { 0 } else { i + 1 };
                let mut next_corner_side = segment_line.side_of_float(
                    &self
                        .corner_approx(next_corner_index)
                        .expect("corner loop of a non-empty shape"),
                    1.0,
                );
                if next_corner_side == Side::Collinear {
                    next_corner_side = segment_line.side_of(&self.corner(next_corner_index));
                }
                if (prev_corner_side == Side::Positive && next_corner_side == Side::Negative)
                    || (prev_corner_side == Side::Negative && next_corner_side == Side::Positive)
                {
                    // this line segment crosses a border line of shape
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_direction::IntDirection;

    /// Constructor-order shorthand for point tiles.
    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// Wraps an octagon in the regular arm.
    fn oct_tile(o: IntOctagon) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(o))
    }

    /// Java `TileShape.getInstance(llx, lly, urx, ury)`.
    fn box_tile(llx: i32, lly: i32, urx: i32, ury: i32) -> TileShape {
        oct_tile(TileShape::from_4_ints(llx, lly, urx, ury))
    }

    /// Java `boxT = TileShape.getInstance(0, 0, 10, 10)`.
    fn box_t() -> TileShape {
        box_tile(0, 0, 10, 10)
    }

    /// Java `octT = TileShape.getInstance(2, -1, 12, 8, 2, 12, 0, 8)`.
    fn oct_t() -> TileShape {
        oct_tile(TileShape::from_8_ints(2, -1, 12, 8, 2, 12, 0, 8))
    }

    /// Java `o8 = TileShape.getInstance(-2, 0, 10, 10, -2, 10, 0, 8)`.
    fn o8() -> TileShape {
        oct_tile(TileShape::from_8_ints(-2, 0, 10, 10, -2, 10, 0, 8))
    }

    fn fl(x: f64, y: f64) -> FloatPoint {
        FloatPoint::new(x, y)
    }

    /// jshell pin `P2`: `getInstance(-1,0,8,5,-2,8,0,8)` is the
    /// normalized octagon with the same fields; `getInstance(0,0,10,10)`
    /// yields the box octagon (0,0,10,10,-10,10,0,20).
    #[test]
    fn p2_from_ints_factories_normalize() {
        assert_eq!(
            TileShape::from_8_ints(-1, 0, 8, 5, -2, 8, 0, 8),
            IntOctagon::new(-1, 0, 8, 5, -2, 8, 0, 8)
        );
        assert_eq!(
            TileShape::from_4_ints(0, 0, 10, 10),
            IntOctagon::new(0, 0, 10, 10, -10, 10, 0, 20)
        );
    }

    /// jshell pin `P1`: `Simplex.getInstance` keeps the FIRST of two
    /// equal-direction lines; through `TileShape::get_instance` a 1-line
    /// simplex stays in the Simplex arm (no box simplification).
    #[test]
    fn p1_dedup_keeps_first_and_stays_simplex() {
        let tile =
            TileShape::get_instance(&[Line::new(p(0, 0), p(5, 0)), Line::new(p(2, 0), p(7, 0))]);
        assert!(matches!(tile, TileShape::Simplex(_)));
        assert_eq!(tile.border_line_count(), 1);
        assert_eq!(tile.border_line(0).a, p(0, 0));
    }

    /// jshell pins `P5`/`P9`: the 4 border lines E/N/W/S of the square
    /// (0,-5)-(6,6) simplify to the `IntBox` arm through `get_instance`.
    #[test]
    fn p5_p9_get_instance_simplifies_square_to_box() {
        let lb = Line::new(p(0, -5), p(6, -5));
        let lr = Line::new(p(6, 0), p(6, 6));
        let lt = Line::new(p(6, 6), p(0, 6));
        let ll = Line::new(p(0, 6), p(0, -5));
        let sq = TileShape::get_instance(&[lb, lr, lt, ll]);
        assert_eq!(
            sq,
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(0, -5),
                IntPoint::new(6, 6)
            )))
        );
        // P9: boundingShape over the 45-degree directions is the octagon,
        // over the orthogonal directions the box itself.
        assert_eq!(
            sq.bounding_shape(&ShapeBoundingDirections::FortyfiveDegree),
            Some(RegularTileShape::IntOctagon(IntOctagon::new(
                0, -5, 6, 6, -6, 11, -5, 12
            )))
        );
        assert_eq!(
            sq.bounding_shape(&ShapeBoundingDirections::Orthogonal),
            Some(RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(0, -5),
                IntPoint::new(6, 6)
            )))
        );
    }

    /// jshell pin: `TileShape.getInstance(Line)` does NOT simplify — the
    /// single line stays a 1-line Simplex.
    #[test]
    fn from_line_stays_simplex() {
        let tile = TileShape::from_line(&Line::new(p(0, 0), p(5, 0)));
        assert!(matches!(tile, TileShape::Simplex(_)));
        assert_eq!(tile.border_line_count(), 1);
        assert_eq!(tile.border_line(0), Line::new(p(0, 0), p(5, 0)));
    }

    /// jshell pins `P3`/`P11`: `boxT.intersection(octT)` and the reverse
    /// both yield the regular IntOctagon arm
    /// (2,0,8,3,2,8,2,8).
    #[test]
    fn p3_p11_intersection_box_octagon_dispatch() {
        let expected = oct_tile(IntOctagon::new(2, 0, 8, 3, 2, 8, 2, 8));
        assert_eq!(box_t().intersection(&oct_t()), expected);
        assert_eq!(oct_t().intersection(&box_t()), expected);
    }

    /// jshell pin `P4`: `outer.cutout(inner)` for the boxes
    /// (0,0,20,20) and (5,5,15,15) — 8 regular IntOctagon pieces in the
    /// Java order.
    #[test]
    fn p4_cutout_eight_pieces() {
        let outer = box_tile(0, 0, 20, 20);
        let inner = box_tile(5, 5, 15, 15);
        let pieces = outer.cutout(&inner);
        let expected = [
            IntOctagon::new(0, 5, 5, 15, -15, 0, 5, 20),
            IntOctagon::new(0, 0, 5, 5, -5, 5, 0, 10),
            IntOctagon::new(5, 0, 15, 5, 0, 15, 5, 20),
            IntOctagon::new(15, 0, 20, 5, 10, 20, 15, 25),
            IntOctagon::new(15, 5, 20, 15, 0, 15, 20, 35),
            IntOctagon::new(15, 15, 20, 20, -5, 5, 30, 40),
            IntOctagon::new(5, 15, 15, 20, -15, 0, 20, 35),
            IntOctagon::new(0, 15, 5, 20, -20, -10, 15, 25),
        ];
        assert_eq!(pieces.len(), 8);
        for (piece, oct) in pieces.iter().zip(expected.iter()) {
            assert_eq!(piece, &oct_tile(*oct));
        }
    }

    /// jshell pins `P13`/`P15`: the triangle (0,0),(10,0),(10,10)
    /// simplifies to the regular IntOctagon arm with 8 border lines, and
    /// the first three border lines are the triangle's edges.
    #[test]
    fn p13_p15_triangle_simplifies_to_octagon() {
        let tri = TileShape::from_points(&[p(0, 0), p(10, 0), p(10, 10)]);
        assert!(matches!(
            tri,
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
        ));
        assert_eq!(tri.dimension(), 2);
        assert_eq!(tri.border_line_count(), 8);
        assert_eq!(tri.border_line(0), Line::new(p(0, 0), p(1, 0)));
        assert_eq!(tri.border_line(1), Line::new(p(10, 0), p(11, 1)));
        assert_eq!(tri.border_line(2), Line::new(p(10, 0), p(10, 1)));
        // The exact same points still yield a nonempty shape and the
        // point factory panics on an empty slice (Java
        // ArrayIndexOutOfBoundsException).
        let result = std::panic::catch_unwind(|| TileShape::from_points(&[]));
        assert!(result.is_err());
    }

    /// jshell pin `P14`/`P14B`: the TWO-ARG `contains(FloatPoint, 0.0)`
    /// accepts the interior point (5,5) but NOT the border point (0,5)
    /// (strict loop, no octagon override). The ONE-ARG
    /// `contains(FloatPoint)` is a VIRTUAL dispatch: on the
    /// octagon-typed `boxOct` Java selects the border-INCLUDED
    /// `IntOctagon.contains` override — (0,5) is accepted, (15,5) not —
    /// while on a genuine IntBox the strict `TileShape` loop applies
    /// (0,5) rejected. The locator's `shrinkedRoomShape.contains
    /// (addCorner)` call is this one-arg dispatch.
    #[test]
    fn p14_contains_float_border_exclusion() {
        // Two-arg: strict for every variant.
        assert!(box_t().contains_float_tolerance(&fl(5.0, 5.0), 0.0));
        assert!(!box_t().contains_float_tolerance(&fl(0.0, 5.0), 0.0));
        assert!(!box_t().contains_float_tolerance(&fl(15.0, 5.0), 0.0));
        // One-arg on the OCTAGON: border-included override.
        assert!(box_t().contains_float(&fl(5.0, 5.0)));
        assert!(box_t().contains_float(&fl(0.0, 5.0)));
        assert!(!box_t().contains_float(&fl(15.0, 5.0)));
        // One-arg on a genuine IntBox: strict (no override).
        let int_box = TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(0, 0),
            IntPoint::new(10, 10),
        )));
        assert!(int_box.contains_float(&fl(5.0, 5.0)));
        assert!(!int_box.contains_float(&fl(0.0, 5.0)));
        // The int-point containment includes the border (!isOutside).
        assert!(box_t().contains_point(&p(5, 5)));
        assert!(box_t().contains_point(&p(0, 5)));
        assert!(!box_t().contains_point(&p(15, 5)));
    }

    /// jshell pin `PW1`: `sideOfBorder` is onTheRight (inside) at (5,5),
    /// collinear at the border point (5,0) and onTheLeft at (15,5).
    #[test]
    fn pw1_side_of_border_pin() {
        assert_eq!(box_t().side_of_border(&fl(5.0, 5.0), 0.0), Side::Negative);
        assert_eq!(box_t().side_of_border(&fl(5.0, 0.0), 0.0), Side::Collinear);
        assert_eq!(box_t().side_of_border(&fl(15.0, 5.0), 0.0), Side::Positive);
    }

    /// jshell pin `PW2`: `sideOf(Line)` for the vertical line x=15 gives
    /// onTheLeft (the whole box is left of it) and for x=-5 onTheRight.
    #[test]
    fn pw2_side_of_line_pin() {
        assert_eq!(
            box_t().side_of_line(&Line::new(p(15, 0), p(15, 1))),
            Side::Positive
        );
        assert_eq!(
            box_t().side_of_line(&Line::new(p(-5, 0), p(-5, 1))),
            Side::Negative
        );
        // A line cutting the box is collinear.
        assert_eq!(
            box_t().side_of_line(&Line::new(p(5, 0), p(5, 1))),
            Side::Collinear
        );
    }

    /// jshell pin `PW3`: `length` is sqrt(200), `circumference` is 40.0
    /// and `o8.smallestRadius()` is 1.5.
    #[test]
    fn pw3_length_circumference_smallest_radius_pins() {
        assert_eq!(box_t().length(), 14.142_135_623_730_95);
        assert_eq!(box_t().circumference(), 40.0);
        assert_eq!(o8().smallest_radius(), 1.5);
    }

    /// jshell pin `PW4`: the distance to (15,5) is 5.0 from the shape and
    /// 5.0 from its border; an interior point has distance 0.
    #[test]
    fn pw4_distance_pins() {
        assert_eq!(box_t().distance(&fl(15.0, 5.0)), 5.0);
        assert_eq!(box_t().border_distance(&fl(15.0, 5.0)), 5.0);
        assert_eq!(box_t().distance(&fl(5.0, 5.0)), 0.0);
    }

    /// jshell pin `PW5`: `boxT.shrink(2)` is the octagon (2,2,8,8,-6,6,4,16).
    #[test]
    fn pw5_shrink_pin() {
        assert_eq!(
            box_t().shrink(2.0),
            oct_tile(IntOctagon::new(2, 2, 8, 8, -6, 6, 4, 16))
        );
    }

    /// jshell pin `PW6`: `nearestBorderPoint((15,5))` is (10,5) and
    /// `nearestPoint` returns interior points unchanged.
    #[test]
    fn pw6_nearest_point_pins() {
        assert_eq!(box_t().nearest_border_point(&p(15, 5)), Some(p(10, 5)));
        assert_eq!(box_t().nearest_point(&p(5, 5)), Some(p(5, 5)));
        assert_eq!(box_t().nearest_point(&p(15, 5)), Some(p(10, 5)));
    }

    /// jshell pin `PW8`: `intersectingBorderLineNo((5,5), UP)` hits the
    /// upper border (4) and LEFT hits the left border (6).
    #[test]
    fn pw8_intersecting_border_line_no_pins() {
        assert_eq!(
            box_t().intersecting_border_line_no(&p(5, 5), &Direction::Int(IntDirection::UP)),
            4
        );
        assert_eq!(
            box_t().intersecting_border_line_no(&p(5, 5), &Direction::Int(IntDirection::LEFT)),
            6
        );
        // Outside starting points yield -1.
        assert_eq!(
            box_t().intersecting_border_line_no(&p(15, 5), &Direction::Int(IntDirection::UP)),
            -1
        );
    }

    /// jshell pin `PW9`: `area` is 100.0, the gravity point is (5,5) and
    /// `o8.area()` is 24.0.
    #[test]
    fn pw9_area_and_centre_of_gravity_pins() {
        assert_eq!(box_t().area(), 100.0);
        assert_eq!(box_t().centre_of_gravity(), fl(5.0, 5.0));
        assert_eq!(o8().area(), 24.0);
    }

    /// jshell pin `PW10`: the 3 nearest border points of o8 to (4,4)
    /// contain the DUPLICATE (3,5) twice — the strict `<` insertion
    /// keeps both ties.
    #[test]
    fn pw10_nearest_border_points_top_k_ties() {
        assert_eq!(
            o8().nearest_border_points_approx(&fl(4.0, 4.0), 3),
            vec![fl(4.0, 4.0), fl(3.0, 5.0), fl(3.0, 5.0)]
        );
    }

    /// jshell pin `PW6b`: `outer.nearestBorderPointsApprox((5,5), 2)`
    /// returns the bottom and the left border points.
    #[test]
    fn pw6b_nearest_border_points_approx_pin() {
        let outer = box_tile(0, 0, 20, 20);
        assert_eq!(
            outer.nearest_border_points_approx(&fl(5.0, 5.0), 2),
            vec![fl(5.0, 0.0), fl(0.0, 5.0)]
        );
    }

    /// jshell pin `PW6c` (jar-captured on a genuine 4-line `IntBox`, both
    /// top-k loops in tie territory): from (1,1) all four corners tie and
    /// all four edge projections tie nearer, so the result order pins the
    /// strict-`<` projection insertion (a `<=` mutant flips the first two
    /// entries). From (3,3) every projection fails the segment-visibility
    /// check, so only CORNERS compete: (2,0) and (0,2) tie at squared
    /// distance 10 and strict `<` keeps the first-encountered (2,0) — a
    /// `<=` corner mutant displaces it with (0,2) and would fail this pin.
    /// The n3 duplicate (2,0) is oracle truth: the shift loop copies
    /// `minDists[k] = minDists[k - 1]` sequentially, so the MAX seeds die
    /// at the first insertion and equal distances propagate duplicates.
    #[test]
    fn pw6c_nearest_border_points_tie_order_pin() {
        let int_box = TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(0, 0),
            IntPoint::new(2, 2),
        )));
        // CTIE/PW6b-tie capture: from (1,1).
        assert_eq!(
            int_box.nearest_border_points_approx(&fl(1.0, 1.0), 2),
            vec![fl(0.0, 1.0), fl(1.0, 0.0)]
        );
        assert_eq!(
            int_box.nearest_border_points_approx(&fl(1.0, 1.0), 3),
            vec![fl(0.0, 1.0), fl(1.0, 0.0), fl(2.0, 1.0)]
        );
        // CTIE capture: from (3,3), corners only.
        assert_eq!(
            int_box.nearest_border_points_approx(&fl(3.0, 3.0), 2),
            vec![fl(2.0, 2.0), fl(2.0, 0.0)]
        );
        assert_eq!(
            int_box.nearest_border_points_approx(&fl(3.0, 3.0), 3),
            vec![fl(2.0, 2.0), fl(2.0, 0.0), fl(2.0, 0.0)]
        );
    }

    /// jshell pin `PW11`: `contains(TileShape)` is true for the inner
    /// box (1,1,9,9) and false in both directions between boxT and octT.
    #[test]
    fn pw11_contains_tile_pins() {
        let inner9 = box_tile(1, 1, 9, 9);
        assert!(box_t().contains_tile(&inner9));
        assert!(!box_t().contains_tile(&oct_t()));
        assert!(!oct_t().contains_tile(&box_t()));
    }

    /// jshell pin `PW12`: `containsApprox` agrees with PW11.
    #[test]
    fn pw12_contains_approx_pins() {
        let inner9 = box_tile(1, 1, 9, 9);
        assert!(box_t().contains_approx(&inner9));
        assert!(!box_t().contains_approx(&oct_t()));
    }

    /// jshell pin `P7`: turning o8 by 90 degrees around the origin goes
    /// through the border-line rotation and simplifies back to the
    /// regular octagon (-5,-1,0,8,-8,0,-2,8).
    #[test]
    fn p7_turn_90_degree_octagon_pin() {
        assert_eq!(
            o8().turn_90_degree(1, &IntPoint::new(0, 0)),
            oct_tile(IntOctagon::new(-5, -1, 0, 8, -8, 0, -2, 8))
        );
    }

    /// The intersects dispatch: the box tile, the simplex of the same box
    /// and octT pairwise.
    #[test]
    fn intersects_dispatch_arms() {
        let box_simplex = TileShape::Simplex(Box::new(
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 10)).to_simplex(),
        ));
        // (Box, Simplex), (Simplex, Box), (Simplex, Simplex),
        // (Octagon, Simplex) and (Box, Box).
        assert!(box_t().intersects(&box_simplex));
        assert!(box_simplex.intersects(&box_t()));
        assert!(box_simplex.intersects(&box_simplex));
        assert!(oct_t().intersects(&box_simplex));
        assert!(box_t().intersects(&box_t()));
        // A far-away simplex intersects nothing.
        let far = TileShape::Simplex(Box::new(
            IntBox::new(IntPoint::new(100, 100), IntPoint::new(110, 110)).to_simplex(),
        ));
        assert!(!box_t().intersects(&far));
        assert!(!far.intersects(&box_t()));
    }

    /// The simplex pieces of the (Simplex, Octagon) cutout dispatch
    /// agree with the P12 pin count (the exact border lines are pinned in
    /// the simplex tests).
    #[test]
    fn cutout_from_simplex_dispatch_piece_count() {
        let sq_simplex = TileShape::Simplex(Box::new(Simplex::get_instance(&[
            Line::new(p(0, -5), p(6, -5)),
            Line::new(p(6, 0), p(6, 6)),
            Line::new(p(6, 6), p(0, 6)),
            Line::new(p(0, 6), p(0, -5)),
        ])));
        let pieces = sq_simplex.cutout(&oct_t());
        assert_eq!(pieces.len(), 5);
        assert!(
            pieces
                .iter()
                .all(|piece| matches!(piece, TileShape::Simplex(_)))
        );
    }

    /// Regular-arm delegations: to_simplex of the box tile is the 4-line
    /// simplex, and intersection_with_simplify returns a regular arm.
    #[test]
    fn to_simplex_and_intersection_with_simplify() {
        let simplex = box_t().to_simplex();
        assert_eq!(simplex.border_line_count(), 4);
        let simplified = box_t().intersection_with_simplify(&oct_t());
        assert!(matches!(
            simplified,
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
        ));
    }

    /// Trivial flags across both arms.
    #[test]
    fn flags_across_arms() {
        assert!(box_t().is_bounded());
        assert!(!box_t().is_empty());
        assert_eq!(box_t().dimension(), 2);
        assert!(box_t().is_int_octagon());
        // jshell pin `PB5`: the box octagon (0,0,10,10) via
        // fromInstance(4 ints) IS convertible — `boxT.isIntBox()` and
        // `isIntOctagon()` are both true.
        assert!(box_t().is_int_box());
        // The square tile IS a box.
        let sq = TileShape::get_instance(&[
            Line::new(p(0, -5), p(6, -5)),
            Line::new(p(6, 0), p(6, 6)),
            Line::new(p(6, 6), p(0, 6)),
            Line::new(p(0, 6), p(0, -5)),
        ]);
        assert!(sq.is_int_box());
        assert_eq!(sq.dimension(), 2);
        // Splitting a convex shape yields the shape itself.
        assert_eq!(sq.split_to_convex(), vec![sq.clone()]);
        assert_eq!(sq.bounding_tile(), sq);
        assert_eq!(
            sq.bounding_box(),
            IntBox::new(IntPoint::new(0, -5), IntPoint::new(6, 6))
        );
        assert_eq!(
            sq.bounding_octagon(),
            Some(IntOctagon::new(0, -5, 6, 6, -6, 11, -5, 12))
        );
    }

    // -------------------------------------------------------------------
    // Task 8 oracle pins (all jshell-captured from the frozen jar).
    // -------------------------------------------------------------------

    /// Java helpers for the Task 8 pins:
    /// `boxOct = TileShape.getInstance(0, 0, 10, 10)`,
    /// `right = TileShape.getInstance(10, 0, 20, 10)`,
    /// `cross = new Polyline(new Point[]{(-5,5), (15,5)})`.
    fn task8_box_oct() -> TileShape {
        oct_tile(TileShape::from_4_ints(0, 0, 10, 10))
    }

    fn task8_right_oct() -> TileShape {
        oct_tile(TileShape::from_4_ints(10, 0, 20, 10))
    }

    fn task8_cross() -> crate::polyline::Polyline {
        crate::polyline::Polyline::from_points(&[p(-5, 5), p(15, 5)])
    }

    /// jshell pins `PINC1`/`PINC2` (trap T21, oracle bug kept for parity):
    /// `boxOct.indexOfNearestCorner((3,7))` is 0 — no corner coincides
    /// with the query point so the `Double.MIN_VALUE` seed is never beaten
    /// and the result is pinned to index 0 even though corner 5 at (0,10)
    /// is strictly nearer. When the query point IS a corner the exact 0
    /// distance wins: `indexOfNearestCorner((10,10))` is 3.
    #[test]
    fn pinc_index_of_nearest_corner_bug_compat() {
        assert_eq!(task8_box_oct().index_of_nearest_corner(&p(3, 7)), 0);
        assert_eq!(task8_box_oct().index_of_nearest_corner(&p(10, 10)), 3);
    }

    /// jshell pin `PENT`: `boxOct.entrancePoints(cross)` is
    /// [(1,6), (1,2)] — the polyline enters through border line 6 (left)
    /// and leaves through border line 2 (right), in that order.
    #[test]
    fn pent_entrance_points_pin() {
        assert_eq!(
            task8_box_oct().entrance_points(&task8_cross()),
            vec![(1, 6), (1, 2)]
        );
    }

    /// jshell pin `PCUT`: `boxOct.cutout(cross)` yields 2 pieces of 3
    /// lines each; piece 0 spans (-5,5)..(0,5), piece 1 (10,5)..(15,5).
    #[test]
    fn pcut_cutout_polyline_pin() {
        let pieces = task8_box_oct().cutout_polyline(&task8_cross());
        assert_eq!(pieces.len(), 2);
        let expected_corners = [
            [fl(-5.0, 5.0), fl(0.0, 5.0)],
            [fl(10.0, 5.0), fl(15.0, 5.0)],
        ];
        for (piece, corners) in pieces.iter().zip(expected_corners.iter()) {
            assert_eq!(piece.lines.len(), 3);
            let approx = piece.corner_approx_arr();
            for (c, e) in approx.iter().zip(corners.iter()) {
                assert_eq!((c.x, c.y), (e.x, e.y));
            }
        }
    }

    /// jshell pin `PTS1`: `boxOct.touchingSides(right)` is [2, 6] —
    /// side 2 of this octagon lies on side 6 of the neighbour.
    #[test]
    fn pts1_touching_sides_pin() {
        assert_eq!(
            task8_box_oct().touching_sides(&task8_right_oct()),
            vec![2, 6]
        );
    }

    /// jshell pin `PROT1`: `boxOct.rotateApprox(PI/2, (0,0))` is the box
    /// (-10,0)-(0,10).
    #[test]
    fn prot1_rotate_approx_pin() {
        let rotated = task8_box_oct().rotate_approx(std::f64::consts::FRAC_PI_2, &FloatPoint::ZERO);
        assert_eq!(
            rotated,
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(-10, 0),
                IntPoint::new(0, 10)
            )))
        );
    }

    /// jshell pin `PDIV1`: `boxOct.divideIntoSections(8)` is 4 boxes in
    /// the order (0,0)-(5,5), (5,0)-(10,5), (0,5)-(5,10), (5,5)-(10,10).
    #[test]
    fn pdiv1_divide_into_sections_pin() {
        let sections = task8_box_oct().divide_into_sections(8.0);
        let expected = [(0, 0, 5, 5), (5, 0, 10, 5), (0, 5, 5, 10), (5, 5, 10, 10)];
        assert_eq!(sections.len(), 4);
        for (section, (llx, lly, urx, ury)) in sections.iter().zip(expected.iter()) {
            match section {
                TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => {
                    assert_eq!((b.ll.x, b.ll.y, b.ur.x, b.ur.y), (*llx, *lly, *urx, *ury));
                }
                other => panic!("expected IntBox section, got {:?}", other),
            }
        }
    }

    /// jshell pins `PDL1`..`PDL3`: `distanceToTheLeft` is -1.0 for the
    /// line x=15 (right of the shape), 5.0 for x=-5 (left) and -1.0 for
    /// the cutting line x=5.
    #[test]
    fn pdl_distance_to_the_left_pins() {
        assert_eq!(
            task8_box_oct().distance_to_the_left(&Line::new(p(15, 0), p(15, 5))),
            -1.0
        );
        assert_eq!(
            task8_box_oct().distance_to_the_left(&Line::new(p(-5, 0), p(-5, 5))),
            5.0
        );
        assert_eq!(
            task8_box_oct().distance_to_the_left(&Line::new(p(5, 0), p(5, 5))),
            -1.0
        );
    }

    /// jshell pins `PIIB1`/`PIIB2`: the interior-crossing segment of
    /// `cross` intersects the interior; the segment of the outside
    /// polyline (15,5)-(25,5) does not.
    #[test]
    fn piib_is_intersected_interior_by_pins() {
        let crossing = crate::line_segment::LineSegment::from_polyline(&task8_cross(), 1);
        assert!(task8_box_oct().is_intersected_interior_by(&crossing));
        let outside = crate::polyline::Polyline::from_points(&[p(15, 5), p(25, 5)]);
        let outside_segment = crate::line_segment::LineSegment::from_polyline(&outside, 1);
        assert!(!task8_box_oct().is_intersected_interior_by(&outside_segment));
    }

    /// jshell pin `PDCS1`: `boxOct.diagonalCornerSegment()` runs from
    /// (0,0) to (10,10).
    #[test]
    fn pdcs1_diagonal_corner_segment_pin() {
        let diagonal = task8_box_oct().diagonal_corner_segment().expect("bounded");
        assert_eq!((diagonal.a.x, diagonal.a.y), (0.0, 0.0));
        assert_eq!((diagonal.b.x, diagonal.b.y), (10.0, 10.0));
    }

    /// jshell pin `PNRL1`: `boxOct.nearestRelativeOutsideLocations(right, 2)`
    /// is [(0,0), (5,-5)].
    #[test]
    fn pnrl1_nearest_relative_outside_locations_pin() {
        assert_eq!(
            task8_box_oct().nearest_relative_outside_locations(&task8_right_oct(), 2),
            vec![fl(0.0, 0.0), fl(5.0, -5.0)]
        );
    }

    /// `isContainedIn(IntBox)`, captured from the jar
    /// (`logs/M3-T10a/captures/shape_entry_side_caps.txt`): the box
    /// cases go through the IntBox reverse dispatch, the simplex cases
    /// through the Simplex face (a simplex is contained via its
    /// bounding box). `inner` is strictly inside, `shifted` shares the
    /// lower-left corner but extends beyond — the discriminating pair.
    #[test]
    fn is_contained_in_int_box_matches_the_jar() {
        let box_shape = IntBox::new(IntPoint::new(0, 0), IntPoint::new(1000, 1000));
        let inner = IntBox::new(IntPoint::new(100, 100), IntPoint::new(900, 900));
        let big = IntBox::new(IntPoint::new(-100, -100), IntPoint::new(1100, 1100));
        let shifted = IntBox::new(IntPoint::new(100, 100), IntPoint::new(2000, 2000));
        let box_tile = TileShape::RegularTileShape(RegularTileShape::IntBox(box_shape));
        assert!(box_tile.is_contained_in_int_box(&big), "box-in-big");
        assert!(
            !box_tile.is_contained_in_int_box(&shifted),
            "box-in-shifted"
        );
        assert!(
            TileShape::RegularTileShape(RegularTileShape::IntBox(inner))
                .is_contained_in_int_box(&box_shape),
            "inner-in-box"
        );

        // the simplex branch: octagon (100,100,900,900,200,800,200,800)
        // reduced to a simplex; contained in big, and ALSO in shifted
        // (the bounding box (100,100,900,900) sits inside both).
        let oct = TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::new(
            100, 100, 900, 900, 200, 800, 200, 800,
        )));
        let simplex = TileShape::Simplex(Box::new(oct.to_simplex()));
        assert!(simplex.is_contained_in_int_box(&big), "simplex-in-big");
        assert!(
            simplex.is_contained_in_int_box(&shifted),
            "simplex-in-shifted"
        );
    }
}
