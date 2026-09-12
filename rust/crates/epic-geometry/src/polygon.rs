//! Port of Java `app.freerouting.geometry.planar.Polygon` — a list of
//! points in the plane, where no 2 consecutive points may be equal and no
//! 3 consecutive points collinear.
//!
//! Bit-parity notes:
//! - The constructor normalization loop is transliterated exactly: each
//!   collinear removal `break`s the scan and the whole pass (duplicate
//!   removal first, then collinear removal) restarts from the beginning.
//!   The removal order is observable through the corner indices consumed
//!   by `Polyline` / `PolygonShape` (T10).
//! - `winding_number_after_closing` accumulates
//!   [`Vector::angle_approx`] values in the same order and rounds with
//!   [`java_round`] (Java `Math.round`). The Java warning for
//!   `|angleSum| < 0.5` is dropped (the value is still returned).

use crate::point::Point;
use crate::rounding::java_round;
use crate::side::Side;

/// Java `Polygon`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Polygon {
    corners: Vec<Point>,
}

impl Polygon {
    /// Creates a polygon from points. Multiple points and points which are
    /// collinear with their previous and next point are removed (Java
    /// `Polygon(Point[])`).
    pub fn new(points: &[Point]) -> Polygon {
        let mut corners: Vec<Point> = points.to_vec();
        loop {
            let mut corner_removed = false;

            // remove multiple points
            let mut deduplicated: Vec<Point> = Vec::with_capacity(corners.len());
            for point in &corners {
                if deduplicated.last().map(|last| last == point) == Some(true) {
                    corner_removed = true;
                    // duplicate is skipped
                } else {
                    deduplicated.push(point.clone());
                }
            }
            corners = deduplicated;
            if corners.is_empty() {
                return Polygon { corners };
            }

            // remove points which are collinear with the previous and next
            // point. Java removes at most one point per outer pass and then
            // restarts (the inner loop breaks after the removal).
            let mut remove_index: Option<usize> = None;
            for i in 1..corners.len() - 1 {
                if corners[i].side_of(&corners[i - 1], &corners[i + 1]) == Side::Collinear {
                    remove_index = Some(i);
                    break;
                }
            }
            if let Some(index) = remove_index {
                corners.remove(index);
                corner_removed = true;
            }

            if !corner_removed {
                return Polygon { corners };
            }
        }
    }

    /// Returns the corners of this polygon (Java `cornerArray()`).
    pub fn corner_array(&self) -> &[Point] {
        &self.corners
    }

    /// Reverts the order of the corners of this polygon (Java
    /// `revertCorners()`; re-normalizes like every `Polygon` construction).
    pub fn revert_corners(&self) -> Polygon {
        let reversed: Vec<Point> = self.corners.iter().rev().cloned().collect();
        Polygon::new(&reversed)
    }

    /// Returns the winding number of this polygon, treated as closed (Java
    /// `windingNumberAfterClosing()`). Positive for counterclock sense,
    /// negative for clockwise sense.
    pub fn winding_number_after_closing(&self) -> i32 {
        let corners = &self.corners;
        if corners.len() < 2 {
            return 0;
        }
        let first_side_vector = corners[1].difference_by(&corners[0]);
        let mut prev_side_vector = first_side_vector.clone();
        let mut corner_count = corners.len();
        // Skip the last corner, if it is equal to the first corner.
        if corners[0] == corners[corner_count - 1] {
            corner_count -= 1;
        }
        let mut angle_sum = 0.0f64;
        for i in 1..corner_count - 1 {
            let next_side_vector = corners[i + 1].difference_by(&corners[i]);
            angle_sum += prev_side_vector.angle_approx(&next_side_vector);
            prev_side_vector = next_side_vector;
        }
        if corner_count > 1 {
            let next_side_vector = corners[0].difference_by(&corners[corner_count - 1]);
            angle_sum += prev_side_vector.angle_approx(&next_side_vector);
            prev_side_vector = next_side_vector;
        }
        angle_sum += prev_side_vector.angle_approx(&first_side_vector);
        angle_sum /= 2.0 * std::f64::consts::PI;
        // Java warns if |angle_sum| < 0.5 and returns Math.round(angleSum).
        java_round(angle_sum) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_point::IntPoint;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// A square stays a 4-corner polygon; a repeated corner is dropped and
    /// a collinear middle corner is dropped.
    #[test]
    fn ctor_normalization() {
        let square = Polygon::new(&[p(0, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(square.corner_array().len(), 4);

        let with_dup = Polygon::new(&[p(0, 0), p(10, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(with_dup.corner_array().len(), 4);

        let with_collinear = Polygon::new(&[p(0, 0), p(5, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(with_collinear.corner_array(), square.corner_array());
    }

    /// jshell pin `PG1`: winding number of a counterclock square is +1;
    /// reverting makes it -1 (oracle:
    /// `new Polygon(new Point[]{new IntPoint(0,0), new IntPoint(10,0),
    /// new IntPoint(10,10), new IntPoint(0,10)}).windingNumberAfterClosing()`).
    #[test]
    fn pg1_winding_number() {
        let square = Polygon::new(&[p(0, 0), p(10, 0), p(10, 10), p(0, 10)]);
        assert_eq!(square.winding_number_after_closing(), 1);
        assert_eq!(square.revert_corners().winding_number_after_closing(), -1);
    }
}
