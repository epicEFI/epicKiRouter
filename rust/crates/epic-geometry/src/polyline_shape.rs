//! Java abstract class `app.freerouting.geometry.planar.PolylineShape` —
//! functions for shapes whose borders consist of straight lines.
//!
//! Java `PolygonShape` is the ONLY subclass of `PolylineShape`; the port
//! flattens the abstract class onto [`crate::polygon_shape::PolygonShape`]
//! as an inherent impl block (Rust trait objects would not support the
//! `Self`-returning transforms, and a second implementor never exists in
//! the oracle). Every method here mirrors the Java `PolylineShape` body.

use crate::float_line::FloatLine;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::line::Line;
use crate::point::Point;
use crate::polygon_shape::PolygonShape;
use crate::side::Side;

impl PolygonShape {
    /// Java `PolylineShape.boundedCorners()`: all (necessarily bounded)
    /// corners; kept for API parity.
    pub fn bounded_corners(&self) -> Vec<Point> {
        (0..self.border_line_count() as i32)
            .filter(|&i| self.corner_is_bounded(i))
            .map(|i| self.corner(i))
            .collect()
    }

    /// Java `PolylineShape.cornerApprox(int)`.
    pub fn corner_approx(&self, no: i32) -> FloatPoint {
        self.corner(no).to_float()
    }

    /// Java `PolylineShape.cornerApproxArr()`.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..self.border_line_count() as i32)
            .map(|i| self.corner_approx(i))
            .collect()
    }

    /// Java `PolylineShape.equalsCorner(Point)`: the number of the corner
    /// equal to point, or -1.
    pub fn equals_corner(&self, point: &Point) -> i32 {
        for i in 0..self.border_line_count() as i32 {
            if &self.corner(i) == point {
                return i;
            }
        }
        -1
    }

    /// Java `PolylineShape.circumference()`: the shapes are always
    /// bounded, so the unbounded `Integer.MAX_VALUE` branch is dead.
    pub fn circumference(&self) -> f64 {
        let corner_count = self.border_line_count();
        let mut result = 0.0f64;
        let mut prev_corner = self.corner_approx(corner_count as i32 - 1);
        for i in 0..corner_count as i32 {
            let current_corner = self.corner_approx(i);
            result += current_corner.distance(&prev_corner);
            prev_corner = current_corner;
        }
        result
    }

    /// Java `PolylineShape.centreOfGravity()`.
    pub fn centre_of_gravity(&self) -> FloatPoint {
        let corner_count = self.border_line_count();
        let mut x = 0.0f64;
        let mut y = 0.0f64;
        for i in 0..corner_count as i32 {
            let current_point = self.corner_approx(i);
            x += current_point.x;
            y += current_point.y;
        }
        x /= corner_count as f64;
        y /= corner_count as f64;
        FloatPoint::new(x, y)
    }

    /// Java `PolylineShape.isContainedIn(IntBox)`.
    pub fn is_contained_in(&self, r#box: &IntBox) -> bool {
        self.bounding_box().is_contained_in(r#box)
    }

    /// Java `PolylineShape.indexOfLeftMostCorner(FloatPoint)`.
    pub fn index_of_left_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut left_most_corner = self.corner_approx(0);
        let corner_count = self.border_line_count();
        let mut result = 0;
        for i in 1..corner_count as i32 {
            let current_corner = self.corner_approx(i);
            if current_corner.side_of(from_point, &left_most_corner) == Side::Positive {
                left_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Java `PolylineShape.indexOfRightMostCorner(FloatPoint)`.
    pub fn index_of_right_most_corner(&self, from_point: &FloatPoint) -> i32 {
        let mut right_most_corner = self.corner_approx(0);
        let corner_count = self.border_line_count();
        let mut result = 0;
        for i in 1..corner_count as i32 {
            let current_corner = self.corner_approx(i);
            if current_corner.side_of(from_point, &right_most_corner) == Side::Negative {
                right_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Java `PolylineShape.polarLineSegment(FloatPoint)`: a FloatLine whose
    /// `a` approximates the left most and `b` the right most corner viewed
    /// from from_point. Panics on the empty shape (Java warns and returns
    /// null).
    pub fn polar_line_segment(&self, from_point: &FloatPoint) -> FloatLine {
        if self.is_empty() {
            panic!("PolylineShape.polarLineSegment: shape is empty (Java returns null)");
        }
        let mut left_most_corner = self.corner_approx(0);
        let mut right_most_corner = self.corner_approx(0);
        let corner_count = self.border_line_count();
        for i in 1..corner_count as i32 {
            let current_corner = self.corner_approx(i);
            if current_corner.side_of(from_point, &right_most_corner) == Side::Negative {
                right_most_corner = current_corner;
            }
            if current_corner.side_of(from_point, &left_most_corner) == Side::Positive {
                left_most_corner = current_corner;
            }
        }
        FloatLine::new(left_most_corner, right_most_corner)
    }

    /// Java `PolylineShape.intersects(Line)`: true if the border corners
    /// are not all on one side of the line.
    pub fn intersects_line(&self, line: &Line) -> bool {
        let side_of_first_corner = line.side_of(&self.corner(0));
        if side_of_first_corner == Side::Collinear {
            return true;
        }
        for i in 1..self.border_line_count() as i32 {
            if line.side_of(&self.corner(i)) != side_of_first_corner {
                return true;
            }
        }
        false
    }

    /// Java `PolylineShape.prevNo(int)`.
    pub fn prev_no(&self, no: i32) -> i32 {
        if no == 0 {
            self.border_line_count() as i32 - 1
        } else {
            no - 1
        }
    }

    /// Java `PolylineShape.nextNo(int)`.
    pub fn next_no(&self, no: i32) -> i32 {
        (no + 1) % self.border_line_count() as i32
    }

    /// Java `PolylineShape.getBorder()`: the shape itself is its border.
    pub fn get_border(&self) -> &PolygonShape {
        self
    }

    /// Java `PolylineShape.getHoles()`: always empty.
    pub fn get_holes(&self) -> Vec<PolygonShape> {
        Vec::new()
    }

    /// Java `PolylineShape.leftMostCorner(Point)`: the empty shape returns
    /// from_point.
    pub fn left_most_corner(&self, from_point: &Point) -> Point {
        if self.is_empty() {
            return from_point.clone();
        }
        let mut result = self.corner(0);
        let corner_count = self.border_line_count();
        for i in 1..corner_count as i32 {
            let current_corner = self.corner(i);
            if current_corner.side_of(from_point, &result) == Side::Positive {
                result = current_corner;
            }
        }
        result
    }

    /// Java `PolylineShape.rightMostCorner(Point)`.
    pub fn right_most_corner(&self, from_point: &Point) -> Point {
        if self.is_empty() {
            return from_point.clone();
        }
        let mut result = self.corner(0);
        let corner_count = self.border_line_count();
        for i in 1..corner_count as i32 {
            let current_corner = self.corner(i);
            if current_corner.side_of(from_point, &result) == Side::Negative {
                result = current_corner;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_point::IntPoint;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// Square (0,0),(10,0),(10,10),(0,10): circumference 40, centre of
    /// gravity (5,5).
    #[test]
    fn measures_of_a_square() {
        let square = PolygonShape::new(&[p(0, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert!((square.circumference() - 40.0).abs() < 1e-9);
        let cog = square.centre_of_gravity();
        assert_eq!((cog.x, cog.y), (5.0, 5.0));
        assert!(square.is_contained_in(&IntBox::new(IntPoint::new(-1, -1), IntPoint::new(11, 11))));
        assert!(!square.is_contained_in(&IntBox::new(IntPoint::new(1, 1), IntPoint::new(11, 11))));
    }

    /// equalsCorner and left/right-most corners from a point (jshell
    /// pins `PLM1`/`PRM1`/`PPOL1`: leftMostCorner((-5,5)) is (0,10),
    /// rightMostCorner((20,5)) is (10,10), polarLineSegment((-5,5)) runs
    /// from (0,10) to (0,0)).
    #[test]
    fn corner_lookup() {
        let square = PolygonShape::new(&[p(0, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(square.equals_corner(&p(10, 10)), 2);
        assert_eq!(square.equals_corner(&p(7, 7)), -1);
        let left = square.left_most_corner(&p(-5, 5));
        assert_eq!(left, p(0, 10));
        let right = square.right_most_corner(&p(20, 5));
        assert_eq!(right, p(10, 10));
        let polar = square.polar_line_segment(&FloatPoint::new(-5.0, 5.0));
        assert_eq!((polar.a.x, polar.a.y), (0.0, 10.0));
        assert_eq!((polar.b.x, polar.b.y), (0.0, 0.0));
    }

    /// The border-line loop: 4 lines, prev/next index wrap.
    #[test]
    fn border_wrap() {
        let square = PolygonShape::new(&[p(0, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(square.prev_no(0), 3);
        assert_eq!(square.next_no(3), 0);
        assert!(square.intersects_line(&Line::new(p(-1, 5), p(11, 5))));
        // a line fully to the left of the square
        assert!(!square.intersects_line(&Line::new(p(-5, 0), p(-5, 10))));
    }
}
