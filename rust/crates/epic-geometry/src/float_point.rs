//! Port of Java `app.freerouting.geometry.planar.FloatPoint`.
//!
//! A point in the plane as a tuple of `double`s. Java deliberately does
//! NOT derive FloatPoint from the abstract Point class ("because
//! arithmetic calculations with doubles are in general not exact").
//!
//! Equality note: Java overrides neither `equals` nor `hashCode`
//! (reference identity). Rust has no object identity in the same sense,
//! so this port derives value equality — a documented deviation that is
//! unobservable in the engine, which never stores FloatPoints in
//! hash-based containers. There is no `Eq`/`Hash` (NaN coordinates).
//!
//! Rounding: `round` uses `Math.round` (JDK 7+ ties toward +infinity,
//! then the `(int)` cast), `roundToGrid` uses `Math.rint` (ties to even)
//! — see [`crate::rounding`].
//!
//! `projectionApprox` landed with Task 7 (it only needs `FloatLine`).
//! Deferred: the locale-dependent `toString(Locale, ...)` variants (locale
//! formatting is out of scope for the geometry kernel). `roundToTheRight` /
//! `roundToTheLeft` landed with Task 3 (Direction); `boundingBox` with
//! Task 4 (IntBox); static `boundingOctagon` with Task 5 (IntOctagon).

use std::fmt;

use crate::direction::Direction;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::rounding::{java_rint, java_round};
use crate::side::Side;

/// Implements a point in the plane as a tuple of doubles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatPoint {
    /// The x coordinate of this point.
    pub x: f64,
    /// The y coordinate of this point.
    pub y: f64,
}

impl FloatPoint {
    /// Java `FloatPoint.ZERO`.
    pub const ZERO: FloatPoint = FloatPoint { x: 0.0, y: 0.0 };

    /// Creates an instance of FloatPoint from two doubles.
    pub const fn new(x: f64, y: f64) -> FloatPoint {
        FloatPoint { x, y }
    }

    /// Creates the smallest IntBox containing this point (Java
    /// `boundingBox`, landed with Task 4).
    pub fn bounding_box(&self) -> IntBox {
        let lower_left = IntPoint::new(self.x.floor() as i32, self.y.floor() as i32);
        let upper_right = IntPoint::new(self.x.ceil() as i32, self.y.ceil() as i32);
        IntBox::new(lower_left, upper_right)
    }

    /// Creates the smallest IntOctagon containing all points of `points`
    /// (Java static `boundingOctagon`, landed with Task 5). The minima
    /// start at `Integer.MAX_VALUE` (+2147483647.0) and the maxima at
    /// `Integer.MIN_VALUE` (-2147483648.0), exactly as seeded in
    /// FloatPoint.java:38-45; for an empty slice the result degenerates to
    /// those seeds after the floor/ceil casts.
    pub fn bounding_octagon(points: &[FloatPoint]) -> IntOctagon {
        let mut min_x = f64::from(i32::MAX);
        let mut min_y = f64::from(i32::MAX);
        let mut max_x = f64::from(i32::MIN);
        let mut max_y = f64::from(i32::MIN);
        let mut min_ulx = f64::from(i32::MAX);
        let mut max_lrx = f64::from(i32::MIN);
        let mut min_llx = f64::from(i32::MAX);
        let mut max_urx = f64::from(i32::MIN);
        for point in points {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
            min_ulx = min_ulx.min(point.x - point.y);
            max_lrx = max_lrx.max(point.x - point.y);
            min_llx = min_llx.min(point.x + point.y);
            max_urx = max_urx.max(point.x + point.y);
        }
        IntOctagon::new(
            min_x.floor() as i32,
            min_y.floor() as i32,
            max_x.ceil() as i32,
            max_y.ceil() as i32,
            min_ulx.floor() as i32,
            max_lrx.ceil() as i32,
            min_llx.floor() as i32,
            max_urx.ceil() as i32,
        )
    }

    /// Returns the square of the distance from this point to the zero
    /// point.
    pub fn size_square(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Returns the distance from this point to the zero point.
    pub fn size(&self) -> f64 {
        self.size_square().sqrt()
    }

    /// Returns the square of the distance from this point to other.
    pub fn distance_square(&self, other: &FloatPoint) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        dx * dx + dy * dy
    }

    /// Returns the distance from this point to other.
    pub fn distance(&self, other: &FloatPoint) -> f64 {
        self.distance_square(other).sqrt()
    }

    /// Computes the weighted distance to other.
    pub fn weighted_distance(
        &self,
        other: &FloatPoint,
        horizontal_weight: f64,
        vertical_weight: f64,
    ) -> f64 {
        let mut delta_x = self.x - other.x;
        let mut delta_y = self.y - other.y;
        delta_x *= horizontal_weight;
        delta_y *= vertical_weight;
        (delta_x * delta_x + delta_y * delta_y).sqrt()
    }

    /// Rounds the coordinates to an IntPoint via `Math.round` (ties toward
    /// +infinity) followed by Java's `(int)` cast, which keeps only the
    /// low 32 bits of the long result.
    pub fn round(&self) -> IntPoint {
        IntPoint::new(java_round(self.x) as i32, java_round(self.y) as i32)
    }

    /// Rounds this point, so that if this point is on the right side of
    /// any directed line with direction dir, the result point will also be
    /// on the right side. The coordinate opposite to the direction is
    /// rounded outward (ceil/floor), the coordinate along it is rounded
    /// inward (floor/ceil), and zero direction components fall back to
    /// `Math.round`.
    pub fn round_to_the_right(&self, dir: &Direction) -> IntPoint {
        let direction_vector = dir.get_vector().to_float();
        let rounded_x = if direction_vector.y > 0.0 {
            self.x.ceil() as i32
        } else if direction_vector.y < 0.0 {
            self.x.floor() as i32
        } else {
            java_round(self.x) as i32
        };

        let rounded_y = if direction_vector.x > 0.0 {
            self.y.floor() as i32
        } else if direction_vector.x < 0.0 {
            self.y.ceil() as i32
        } else {
            java_round(self.y) as i32
        };
        IntPoint::new(rounded_x, rounded_y)
    }

    /// Rounds this point, so that if this point is on the left side of any
    /// directed line with direction dir, the result point will also be on
    /// the left side.
    pub fn round_to_the_left(&self, dir: &Direction) -> IntPoint {
        let direction_vector = dir.get_vector().to_float();
        let rounded_x = if direction_vector.y > 0.0 {
            self.x.floor() as i32
        } else if direction_vector.y < 0.0 {
            self.x.ceil() as i32
        } else {
            java_round(self.x) as i32
        };

        let rounded_y = if direction_vector.x > 0.0 {
            self.y.ceil() as i32
        } else if direction_vector.x < 0.0 {
            self.y.floor() as i32
        } else {
            java_round(self.y) as i32
        };
        IntPoint::new(rounded_x, rounded_y)
    }

    /// Rounds this point, so that the x coordinate of the result is a
    /// multiple of horizontal_grid and the y coordinate a multiple of
    /// vertical_grid. Uses `Math.rint` (ties to even); non-positive grids
    /// leave the coordinate unchanged.
    pub fn round_to_grid(&self, horizontal_grid: i32, vertical_grid: i32) -> IntPoint {
        let rounded_x = if horizontal_grid > 0 {
            java_rint(self.x / horizontal_grid as f64) * horizontal_grid as f64
        } else {
            self.x
        };
        let rounded_y = if vertical_grid > 0 {
            java_rint(self.y / vertical_grid as f64) * vertical_grid as f64
        } else {
            self.y
        };
        IntPoint::new(rounded_x as i32, rounded_y as i32)
    }

    /// Adds the coordinates of this FloatPoint and other.
    pub fn add(&self, other: &FloatPoint) -> FloatPoint {
        FloatPoint::new(self.x + other.x, self.y + other.y)
    }

    /// Subtracts the coordinates of other from this FloatPoint.
    pub fn subtract(&self, other: &FloatPoint) -> FloatPoint {
        FloatPoint::new(self.x - other.x, self.y - other.y)
    }

    /// Calculates the scalar product of (p1 - this) with (p2 - this).
    /// (The Java null-parameter guard is unrepresentable with references.)
    pub fn scalar_product(&self, p1: &FloatPoint, p2: &FloatPoint) -> f64 {
        let dx1 = p1.x - self.x;
        let dx2 = p2.x - self.x;
        let dy1 = p1.y - self.y;
        let dy2 = p2.y - self.y;
        dx1 * dx2 + dy1 * dy2
    }

    /// Approximates a FloatPoint on the line from zero to this point with
    /// distance new_size from zero. The zero point is returned unchanged.
    pub fn change_size(&self, new_size: f64) -> FloatPoint {
        if self.x == 0.0 && self.y == 0.0 {
            // the size of the zero point cannot be changed
            return *self;
        }
        let length = (self.x * self.x + self.y * self.y).sqrt();
        FloatPoint::new((self.x * new_size) / length, (self.y * new_size) / length)
    }

    /// Approximates a FloatPoint on the line from this point to to_point
    /// with distance new_length from this point. Returns to_point
    /// unchanged if both points are equal (Java logs a warning there).
    pub fn change_length(&self, to_point: &FloatPoint, new_length: f64) -> FloatPoint {
        let dx = to_point.x - self.x;
        let dy = to_point.y - self.y;
        if dx == 0.0 && dy == 0.0 {
            return *to_point;
        }
        let length = (dx * dx + dy * dy).sqrt();
        FloatPoint::new(
            self.x + (dx * new_length) / length,
            self.y + (dy * new_length) / length,
        )
    }

    /// Returns the middle point between this point and to_point. Java
    /// short-circuits on reference identity (`toPoint == this`); this port
    /// uses value equality, which only diverges when equal coordinates
    /// overflow on doubling (|coord| beyond about 8.99e307) — Java itself
    /// then computes differently depending on whether the caller passed
    /// the same reference.
    pub fn middle_point(&self, to_point: &FloatPoint) -> FloatPoint {
        if self == to_point {
            return *self;
        }
        FloatPoint::new(0.5 * (self.x + to_point.x), 0.5 * (self.y + to_point.y))
    }

    /// Returns Side::Positive if this point is on the left of the line
    /// from p1 to p2, Side::Negative if on the right, and Side::Collinear
    /// for the (numerically fragile) collinear case.
    pub fn side_of(&self, p1: &FloatPoint, p2: &FloatPoint) -> Side {
        let d21_x = p2.x - p1.x;
        let d21_y = p2.y - p1.y;
        let d01_x = self.x - p1.x;
        let d01_y = self.y - p1.y;
        let determinant = d21_x * d01_y - d21_y * d01_x;
        Side::of(determinant)
    }

    /// Rotates this FloatPoint by angle (in radians) around the pole.
    /// An angle of exactly 0 returns the point unchanged.
    ///
    /// Parity note: Java `Math.sin`/`Math.cos` and Rust `f64::sin`/`cos`
    /// are both within 1 ulp of correctly rounded but not guaranteed
    /// bit-identical across libm implementations; for the exactly
    /// representable special angles used by the engine they agree.
    pub fn rotate(&self, angle: f64, pole: &FloatPoint) -> FloatPoint {
        if angle == 0.0 {
            return *self;
        }
        let dx = self.x - pole.x;
        let dy = self.y - pole.y;
        let sin_angle = angle.sin();
        let cos_angle = angle.cos();
        let new_dx = dx * cos_angle - dy * sin_angle;
        let new_dy = dx * sin_angle + dy * cos_angle;
        FloatPoint::new(pole.x + new_dx, pole.y + new_dy)
    }

    /// Turns this FloatPoint by factor times 90 degrees around ZERO.
    pub fn turn_90_degree(&self, factor: i32) -> FloatPoint {
        let mut n = factor;
        while n < 0 {
            n += 4;
        }
        while n >= 4 {
            n -= 4;
        }
        match n {
            0 => FloatPoint::new(self.x, self.y),   // 0 degrees
            1 => FloatPoint::new(-self.y, self.x),  // 90 degrees
            2 => FloatPoint::new(-self.x, -self.y), // 180 degrees
            _ => FloatPoint::new(self.y, -self.x),  // 270 degrees
        }
    }

    /// Turns this FloatPoint by factor times 90 degrees around pole.
    pub fn turn_90_degree_around(&self, factor: i32, pole: &FloatPoint) -> FloatPoint {
        let v = self.subtract(pole);
        let v = v.turn_90_degree(factor);
        pole.add(&v)
    }

    /// Checks if this point is contained in the box spanned by p1 and p2
    /// with the input tolerance.
    pub fn is_contained_in_box(&self, p1: &FloatPoint, p2: &FloatPoint, tolerance: f64) -> bool {
        let (min_x, max_x) = if p1.x < p2.x {
            (p1.x, p2.x)
        } else {
            (p2.x, p1.x)
        };
        if self.x < min_x - tolerance || self.x > max_x + tolerance {
            return false;
        }
        let (min_y, max_y) = if p1.y < p2.y {
            (p1.y, p2.y)
        } else {
            (p2.y, p1.y)
        };
        self.y >= min_y - tolerance && self.y <= max_y + tolerance
    }

    /// Calculates the touching points of the tangents from this point to a
    /// circle around to_point with radius distance, by solving the
    /// quadratic equation resulting from the polar line of the circle.
    /// Returns an empty vector if this point is inside the circle. The
    /// situation is turned by 90 degrees when the y difference is smaller
    /// than the x difference, for numerical stability.
    pub fn tangential_points(&self, to_point: &FloatPoint, distance: f64) -> Vec<FloatPoint> {
        // turn the situation 90 degree if the x difference is smaller
        // than the y difference for better numerical stability
        let dx = (self.x - to_point.x).abs();
        let dy = (self.y - to_point.y).abs();
        let situation_turned = dy > dx;
        let (pole, circle_center) = if situation_turned {
            // turn the situation by 90 degree
            (
                FloatPoint::new(-self.y, self.x),
                FloatPoint::new(-to_point.y, to_point.x),
            )
        } else {
            (*self, *to_point)
        };

        let dx = pole.x - circle_center.x;
        let dy = pole.y - circle_center.y;
        let dx_square = dx * dx;
        let dy_square = dy * dy;
        let dist_square = dx_square + dy_square;
        let radius_square = distance * distance;
        let discriminant = radius_square * dy_square - (radius_square - dx_square) * dist_square;

        if discriminant <= 0.0 {
            // pole is inside the circle.
            return Vec::new();
        }
        let square_root = discriminant.sqrt();

        let a1 = radius_square * dy;
        let dy1 = (a1 + distance * square_root) / dist_square;
        let dy2 = (a1 - distance * square_root) / dist_square;

        let first_point_y = dy1 + circle_center.y;
        let first_point_x = (radius_square - dy * dy1) / dx + circle_center.x;
        let second_point_y = dy2 + circle_center.y;
        let second_point_x = (radius_square - dy * dy2) / dx + circle_center.x;

        if situation_turned {
            // turn the result by 270 degree
            vec![
                FloatPoint::new(first_point_y, -first_point_x),
                FloatPoint::new(second_point_y, -second_point_x),
            ]
        } else {
            vec![
                FloatPoint::new(first_point_x, first_point_y),
                FloatPoint::new(second_point_x, second_point_y),
            ]
        }
    }

    /// Calculates the left tangential point of the line from this point to
    /// a circle around to_point with radius distance. Returns None if this
    /// point is inside the circle.
    pub fn left_tangential_point(
        &self,
        to_point: &FloatPoint,
        distance: f64,
    ) -> Option<FloatPoint> {
        let tangent_points = self.tangential_points(to_point, distance);
        let first = *tangent_points.first()?;
        Some(if to_point.side_of(self, &first) == Side::Negative {
            tangent_points[0]
        } else {
            tangent_points[1]
        })
    }

    /// Calculates the right tangential point of the line from this point
    /// to a circle around to_point with radius distance. Returns None if
    /// this point is inside the circle.
    pub fn right_tangential_point(
        &self,
        to_point: &FloatPoint,
        distance: f64,
    ) -> Option<FloatPoint> {
        let tangent_points = self.tangential_points(to_point, distance);
        let first = *tangent_points.first()?;
        Some(if to_point.side_of(self, &first) == Side::Positive {
            tangent_points[0]
        } else {
            tangent_points[1]
        })
    }

    /// Calculates the center of the circle through this point, p1 and p2
    /// by intersecting the perpendicular bisectors of (this, p1) and
    /// (p1, p2).
    pub fn circle_center(&self, p1: &FloatPoint, p2: &FloatPoint) -> FloatPoint {
        let slope1 = (p1.y - self.y) / (p1.x - self.x);
        let slope2 = (p2.y - p1.y) / (p2.x - p1.x);
        let center_x = (slope1 * slope2 * (self.y - p2.y) + slope2 * (self.x + p1.x)
            - slope1 * (p1.x + p2.x))
            / (2.0 * (slope2 - slope1));
        let center_y = (0.5 * (self.x + p1.x) - center_x) / slope1 + 0.5 * (self.y + p1.y);
        FloatPoint::new(center_x, center_y)
    }

    /// Returns true, if this point is contained in the circle through p1,
    /// p2 and p3. The `- 1` on the radius square is a tolerance for
    /// numerical stability (FloatPoint.java:464-465).
    pub fn inside_circle(&self, p1: &FloatPoint, p2: &FloatPoint, p3: &FloatPoint) -> bool {
        let center = p1.circle_center(p2, p3);
        let radius_square = center.distance_square(p1);
        self.distance_square(&center) < radius_square - 1.0
    }
}

/// Java `FloatPoint(IntPoint)` constructor: exact widening.
impl From<IntPoint> for FloatPoint {
    fn from(pt: IntPoint) -> FloatPoint {
        FloatPoint::new(pt.x as f64, pt.y as f64)
    }
}

/// Java `toString()`: `"(" + nf.format(x) + " , " + nf.format(y) + ")"`
/// (FloatPoint.java:469-473) with at most 4 fraction digits (English
/// locale). Documented deviations: this Display prints the coordinates
/// comma-separated (Java uses `" , "`) with full precision instead of
/// locale-aware rounding.
impl fmt::Display for FloatPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.x, self.y)
    }
}

// ---------------------------------------------------------------------------
// Task 7 closures (Line family)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// jshell pins `P43`/`P44`: `Math.round` ties toward +infinity
    /// (JDK-6430675): (-2.5, 3.5) rounds to (-2, 4), (2.5, -2.5) to
    /// (3, -2).
    #[test]
    fn round_ties_toward_positive_infinity() {
        assert_eq!(FloatPoint::new(-2.5, 3.5).round(), IntPoint::new(-2, 4));
        assert_eq!(FloatPoint::new(2.5, -2.5).round(), IntPoint::new(3, -2));
    }

    /// jshell pin `P48`: (5, 7) on grid 2 rounds to (4, 8) — `Math.rint`
    /// ties to even (rint(2.5) = 2, rint(3.5) = 4).
    #[test]
    fn round_to_grid_ties_to_even() {
        assert_eq!(
            FloatPoint::new(5.0, 7.0).round_to_grid(2, 2),
            IntPoint::new(4, 8)
        );
        // Non-positive grids leave the coordinate unchanged (truncated).
        assert_eq!(
            FloatPoint::new(5.7, 7.2).round_to_grid(0, 0),
            IntPoint::new(5, 7)
        );
    }

    /// jshell pins `P74`/`P76`: size, distance and weighted distance.
    #[test]
    fn distances_match_java() {
        assert_eq!(FloatPoint::new(5.0, 0.0).size(), 5.0);
        assert_eq!(FloatPoint::new(3.0, 4.0).distance(&FloatPoint::ZERO), 5.0);
        // sqrt((3*2)^2 + (4*1)^2) = sqrt(52).
        assert_eq!(
            FloatPoint::new(3.0, 4.0).weighted_distance(&FloatPoint::ZERO, 2.0, 1.0),
            7.211_102_550_927_978
        );
        assert_eq!(FloatPoint::new(1.0, 1.0).size_square(), 2.0);
    }

    /// jshell pins `Q25`/`R14`: middle points, including the identity
    /// shortcut.
    #[test]
    fn middle_point() {
        assert_eq!(
            FloatPoint::new(1.0, 1.0).middle_point(&FloatPoint::new(3.0, 5.0)),
            FloatPoint::new(2.0, 3.0)
        );
        assert_eq!(
            FloatPoint::new(1.5, 2.5).middle_point(&FloatPoint::new(1.5, 2.5)),
            FloatPoint::new(1.5, 2.5)
        );
    }

    /// jshell pins `Q26`/`S2`, `Q27`: changeSize and changeLength keep the
    /// Java operation order ((x * newSize) / length).
    #[test]
    fn change_size_and_change_length() {
        let length = 13.0f64.sqrt();
        assert_eq!(
            FloatPoint::new(2.0, 3.0).change_size(5.0),
            FloatPoint::new((2.0 * 5.0) / length, (3.0 * 5.0) / length)
        );
        assert_eq!(
            FloatPoint::ZERO.change_length(&FloatPoint::new(3.0, 4.0), 10.0),
            FloatPoint::new(6.0, 8.0)
        );
        // Equal points return to_point unchanged.
        let p = FloatPoint::new(1.0, 1.0);
        assert_eq!(p.change_length(&p, 3.0), p);
        // The zero point cannot be resized.
        assert_eq!(FloatPoint::ZERO.change_size(7.0), FloatPoint::ZERO);
    }

    /// jshell pin `Q28`: scalar product of (3, 4) with (-1, 2) from the
    /// origin is 5.0.
    #[test]
    fn scalar_product_is_relative_to_this() {
        assert_eq!(
            FloatPoint::ZERO
                .scalar_product(&FloatPoint::new(3.0, 4.0), &FloatPoint::new(-1.0, 2.0)),
            5.0
        );
    }

    /// jshell pins `Q39`/`R4`: coordinate arithmetic.
    #[test]
    fn add_and_subtract() {
        let result = FloatPoint::new(1.0, 2.0)
            .add(&FloatPoint::new(10.0, 20.0))
            .subtract(&FloatPoint::new(0.5, 0.25));
        assert_eq!(result, FloatPoint::new(10.5, 21.75));
    }

    /// jshell pins `P58`/`R15`-`R17`.
    #[test]
    fn side_of_matches_java() {
        assert_eq!(
            FloatPoint::new(0.0, 1.0).side_of(&FloatPoint::ZERO, &FloatPoint::new(1.0, 0.0)),
            Side::Positive
        );
        assert_eq!(
            FloatPoint::new(-1.0, 0.0).side_of(&FloatPoint::ZERO, &FloatPoint::new(1.0, 0.0)),
            Side::Collinear
        );
        assert_eq!(
            FloatPoint::new(0.3, 0.0).side_of(&FloatPoint::ZERO, &FloatPoint::new(1.0, 0.0)),
            Side::Collinear
        );
    }

    /// jshell pins `R2`/`S1` and the angle == 0 shortcut: rotating (3, 4)
    /// by PI/2 gives (-4, 3.0000000000000004) because cos(PI/2) is the
    /// inexact 6.123233995736766e-17.
    #[test]
    fn rotate_and_turn_90_degree() {
        let angle = std::f64::consts::FRAC_PI_2;
        let rotated = FloatPoint::new(3.0, 4.0).rotate(angle, &FloatPoint::ZERO);
        assert_eq!(rotated.x, -4.0);
        assert_eq!(rotated.y, 3.000_000_000_000_000_4);
        // angle == 0 returns the point unchanged.
        assert_eq!(
            FloatPoint::new(1.0, 0.0).rotate(0.0, &FloatPoint::new(5.0, 5.0)),
            FloatPoint::new(1.0, 0.0)
        );
        // Integer quarter turns are exact.
        assert_eq!(
            FloatPoint::new(1.0, 2.0).turn_90_degree(1),
            FloatPoint::new(-2.0, 1.0)
        );
        assert_eq!(
            FloatPoint::new(1.0, 2.0).turn_90_degree(-1),
            FloatPoint::new(2.0, -1.0)
        );
        assert_eq!(
            FloatPoint::new(1.0, 2.0).turn_90_degree(2),
            FloatPoint::new(-1.0, -2.0)
        );
        assert_eq!(
            FloatPoint::new(1.0, 2.0).turn_90_degree(4),
            FloatPoint::new(1.0, 2.0)
        );
        assert_eq!(
            FloatPoint::new(3.0, 1.0).turn_90_degree_around(1, &FloatPoint::new(1.0, 0.0)),
            FloatPoint::new(0.0, 2.0)
        );
    }

    /// jshell pins `Q32`/`Q33`: the tolerance extends the box, but 3.6 is
    /// outside [1, 3] + 0.5.
    #[test]
    fn is_contained_in_box_uses_tolerance() {
        assert!(!FloatPoint::ZERO.is_contained_in_box(
            &FloatPoint::new(1.0, 1.0),
            &FloatPoint::new(3.0, 3.0),
            0.5
        ));
        assert!(!FloatPoint::new(3.6, 3.6).is_contained_in_box(
            &FloatPoint::new(1.0, 1.0),
            &FloatPoint::new(3.0, 3.0),
            0.5
        ));
        assert!(FloatPoint::new(3.4, 1.2).is_contained_in_box(
            &FloatPoint::new(1.0, 1.0),
            &FloatPoint::new(3.0, 3.0),
            0.5
        ));
        // The box is spanned regardless of corner order.
        assert!(FloatPoint::new(2.0, 2.0).is_contained_in_box(
            &FloatPoint::new(3.0, 3.0),
            &FloatPoint::new(1.0, 1.0),
            0.0
        ));
    }

    /// jshell pins `P62`/`P63`/`S3`: tangents from (5, 0) to the circle
    /// around (0, 0) with radius 3 touch at (1.8, 2.4) and (1.8, -2.4).
    #[test]
    fn tangential_points_match_java() {
        let t = FloatPoint::new(5.0, 0.0).tangential_points(&FloatPoint::ZERO, 3.0);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], FloatPoint::new(1.8, 2.4));
        assert_eq!(t[1], FloatPoint::new(1.8, -2.4));
    }

    /// jshell pin `P64`: the left tangential point is (1.8, -2.4) — the
    /// "left" point has the smaller y here; the right one is (1.8, 2.4).
    #[test]
    fn left_and_right_tangential_points() {
        let pole = FloatPoint::new(5.0, 0.0);
        assert_eq!(
            pole.left_tangential_point(&FloatPoint::ZERO, 3.0),
            Some(FloatPoint::new(1.8, -2.4))
        );
        assert_eq!(
            pole.right_tangential_point(&FloatPoint::ZERO, 3.0),
            Some(FloatPoint::new(1.8, 2.4))
        );
        // Inside the circle: no tangents, Java returns null.
        assert_eq!(
            FloatPoint::ZERO
                .tangential_points(&FloatPoint::ZERO, 3.0)
                .len(),
            0
        );
        assert_eq!(
            FloatPoint::ZERO.left_tangential_point(&FloatPoint::ZERO, 3.0),
            None
        );
        assert_eq!(
            FloatPoint::ZERO.right_tangential_point(&FloatPoint::ZERO, 3.0),
            None
        );
    }

    /// jshell pins `P45`/`S5` and `P46`/`P47` with the `- 1` tolerance.
    #[test]
    fn circle_center_and_inside_circle() {
        assert_eq!(
            FloatPoint::new(2.0, 0.0)
                .circle_center(&FloatPoint::new(0.0, 2.0), &FloatPoint::new(-2.0, 0.0)),
            FloatPoint::ZERO
        );
        // The center itself is strictly inside (0 < 4 - 1).
        assert!(FloatPoint::ZERO.inside_circle(
            &FloatPoint::new(2.0, 0.0),
            &FloatPoint::new(0.0, 2.0),
            &FloatPoint::new(-2.0, 0.0)
        ));
        // (1.9, 0) has distance square 3.61 >= 4 - 1: outside.
        assert!(!FloatPoint::new(1.9, 0.0).inside_circle(
            &FloatPoint::new(2.0, 0.0),
            &FloatPoint::new(0.0, 2.0),
            &FloatPoint::new(-2.0, 0.0)
        ));
    }

    /// Exact widening from IntPoint and the Display format.
    #[test]
    fn from_int_point_and_display() {
        let f: FloatPoint = IntPoint::new(3, -4).into();
        assert_eq!(f, FloatPoint::new(3.0, -4.0));
        assert_eq!(FloatPoint::new(1.5, 2.0).to_string(), "(1.5,2)");
    }

    /// jshell pins `A`-`F`: the roundToTheRight/Left arm selection — UP
    /// covers `dir.y > 0` (ceil x) plus `dir.x == 0` (round y), LEFT45
    /// covers the negative arms of both branches (`dir.y < 0` and
    /// `dir.x < 0`), and the NULL direction (zero direction vector) falls
    /// back to `Math.round` on both coordinates (ties toward +infinity:
    /// -1.5 -> -1, 2.5 -> 3).
    #[test]
    fn round_to_the_right_and_left_arms() {
        // A/B: UP.
        assert_eq!(
            FloatPoint::new(1.2, 3.7).round_to_the_right(&Direction::UP),
            IntPoint::new(2, 4)
        );
        assert_eq!(
            FloatPoint::new(1.2, 3.7).round_to_the_left(&Direction::UP),
            IntPoint::new(1, 4)
        );
        // C/D: LEFT45.
        assert_eq!(
            FloatPoint::new(1.2, 3.7).round_to_the_right(&Direction::LEFT45),
            IntPoint::new(1, 4)
        );
        assert_eq!(
            FloatPoint::new(1.2, 3.7).round_to_the_left(&Direction::LEFT45),
            IntPoint::new(2, 3)
        );
        // E/F: NULL direction.
        assert_eq!(
            FloatPoint::new(-1.5, 2.5).round_to_the_right(&Direction::NULL),
            IntPoint::new(-1, 3)
        );
        assert_eq!(
            FloatPoint::new(-1.5, 2.5).round_to_the_left(&Direction::NULL),
            IntPoint::new(-1, 3)
        );
    }

    /// jshell pin: boundingOctagon([(2.5,-3.5),(-0.5,0.5)]) = (lx,ly,rx,uy,
    /// ulx,lrx,llx,urx) = (-1,-4,3,1,-1,6,-1,0).
    #[test]
    fn bounding_octagon_pin() {
        assert_eq!(
            FloatPoint::bounding_octagon(
                &[FloatPoint::new(2.5, -3.5), FloatPoint::new(-0.5, 0.5),]
            ),
            IntOctagon::new(-1, -4, 3, 1, -1, 6, -1, 0)
        );
        // jshell pin: empty slice — the minima keep the Integer.MAX_VALUE
        // seeds and the maxima keep the Integer.MIN_VALUE seeds
        // (-2147483648, not -2147483647) after the floor/ceil casts
        assert_eq!(
            FloatPoint::bounding_octagon(&[]),
            IntOctagon::new(
                i32::MAX,
                i32::MAX,
                i32::MIN,
                i32::MIN,
                i32::MAX,
                i32::MIN,
                i32::MAX,
                i32::MIN,
            )
        );
    }
}
