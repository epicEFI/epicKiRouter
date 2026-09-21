//! Port of Java `app.freerouting.geometry.planar.LineSegment`.
//!
//! A LineSegment is stored as a triple of Lines: the start closing line,
//! the middle line, and the end closing line; the segment runs from
//! `middle ∩ start` to `middle ∩ end`. The Java transient
//! `precalculatedStartPoint` / `precalculatedEndPoint` memo fields are
//! dropped: corner points are recomputed on demand (T14 — memoized pure
//! functions may be re-computed instead of cached).
//!
//! Java returns `Line[]` of length 0, 1 or 2 from `intersection`; this
//! port returns a `Vec<Line>` with the same 0/1/2 contract, so
//! `overlaps` (more than one intersection line) keeps its meaning.
//!
//! `startPoint` / `endPoint` mirror Java exactly (trap T22): when the
//! respective closing line is parallel to the middle line they return an
//! infinite RationalPoint (z = 0) via [`Line::intersection_point`], and
//! every downstream consumer (`sortEndpointsInXY`, `intersection`,
//! `contains`, `toSimplex`) keeps computing through it with exact
//! rational arithmetic. The class contract *prefers* non-parallel
//! closing lines, but the code degrades gracefully — corpus cases
//! seg-000020/93/125/151 pin that behavior.
//!
//! Deferral ledger — CLOSED in Task 8:
//! - `LineSegment(Polyline, int)` landed as [`LineSegment::from_polyline`]
//!   and `toPolyline()` as [`LineSegment::to_polyline`].
//! - Landed with Task 7: `LineSegment(PolylineShape, int)` (via
//!   `TileShape::border_line`), `toSimplex()` and
//!   `borderIntersections(TileShape)`. The oracle pin `PB3` covers the
//!   0/1-entry outcomes of `borderIntersections`; a 2-entry pin (the
//!   result reordering branch) is still open — noted for the M1a
//!   differential corpus.

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::line::Line;
use crate::point::Point;
use crate::polyline::Polyline;
use crate::rounding::java_round;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Implements functionality for line segments: a Line is infinite, a
/// LineSegment has a start and an endpoint.
#[derive(Debug, Clone)]
pub struct LineSegment {
    start: Line,
    middle: Line,
    end: Line,
}

impl LineSegment {
    /// Creates a line segment from the 3 input lines. It starts at the
    /// intersection of startLine and middleLine and ends at the
    /// intersection of middleLine and endLine. startLine and endLine must
    /// not be parallel to middleLine.
    pub fn new(start: Line, middle: Line, end: Line) -> LineSegment {
        LineSegment { start, middle, end }
    }

    /// Returns the intersection of the first 2 lines of this segment
    /// (Java `startPoint`): an infinite RationalPoint when the start
    /// closing line is parallel to the middle line — Java computes on
    /// through it (trap T22, pins SORT-INF1/2/3).
    pub fn start_point(&self) -> Point {
        self.middle.intersection_point(&self.start)
    }

    /// Returns the intersection of the last 2 lines of this segment
    /// (Java `endPoint`); see [`LineSegment::start_point`] for the
    /// parallel-pair behavior.
    pub fn end_point(&self) -> Point {
        self.middle.intersection_point(&self.end)
    }

    /// Returns an approximation of the intersection of the first 2 lines
    /// of this segment.
    pub fn start_point_approx(&self) -> FloatPoint {
        self.start.intersection_approx(&self.middle)
    }

    /// Returns an approximation of the intersection of the last 2 lines
    /// of this segment.
    pub fn end_point_approx(&self) -> FloatPoint {
        self.end.intersection_approx(&self.middle)
    }

    /// Returns the (infinite) line of this segment.
    pub fn get_line(&self) -> &Line {
        &self.middle
    }

    /// Returns the start closing line of this segment.
    pub fn get_start_closing_line(&self) -> &Line {
        &self.start
    }

    /// Returns the end closing line of this segment.
    pub fn get_end_closing_line(&self) -> &Line {
        &self.end
    }

    /// Returns the line segment with the opposite direction.
    pub fn opposite(&self) -> LineSegment {
        LineSegment::new(
            self.end.opposite(),
            self.middle.opposite(),
            self.start.opposite(),
        )
    }

    /// Checks if point is contained in this line segment. Java warns and
    /// returns false for non-IntPoint input; the non-Int branch returns
    /// false here.
    pub fn contains(&self, point: &Point) -> bool {
        if !matches!(point, Point::Int(_)) {
            return false;
        }
        if self.middle.side_of(point) != Side::Collinear {
            return false;
        }
        // create a perpendicular line at point and check, that the two
        // endpoints of this segment are on different sides of that line.
        let perpendicular_direction = self.middle.direction().turn_45_degree(2);
        let perpendicular_line = Line::new_with_direction(point.clone(), perpendicular_direction);
        let start_point_side = perpendicular_line.side_of(&self.start_point());
        let end_point_side = perpendicular_line.side_of(&self.end_point());
        start_point_side != end_point_side || start_point_side == Side::Collinear
    }

    /// Calculates the smallest surrounding box of this line segment.
    pub fn bounding_box(&self) -> IntBox {
        let start_corner = self.start_point_approx();
        let end_corner = self.end_point_approx();
        // Inputs of intersectionApprox are finite doubles, so Rust's
        // NaN-absorbing f64::min/max matches Java's Math.min/max here.
        let llx = start_corner.x.min(end_corner.x);
        let lly = start_corner.y.min(end_corner.y);
        let urx = start_corner.x.max(end_corner.x);
        let ury = start_corner.y.max(end_corner.y);
        let lower_left = IntPoint::new(llx.floor() as i32, lly.floor() as i32);
        let upper_right = IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
        IntBox::new(lower_left, upper_right)
    }

    /// Calculates the smallest surrounding octagon of this line segment.
    pub fn bounding_octagon(&self) -> IntOctagon {
        let start_corner = self.start_point_approx();
        let end_corner = self.end_point_approx();
        let lx = start_corner.x.min(end_corner.x).floor();
        let ly = start_corner.y.min(end_corner.y).floor();
        let rx = start_corner.x.max(end_corner.x).ceil();
        let uy = start_corner.y.max(end_corner.y).ceil();
        let start_x_minus_y = start_corner.x - start_corner.y;
        let end_x_minus_y = end_corner.x - end_corner.y;
        let ulx = start_x_minus_y.min(end_x_minus_y).floor();
        let lrx = start_x_minus_y.max(end_x_minus_y).ceil();
        let start_x_plus_y = start_corner.x + start_corner.y;
        let end_x_plus_y = end_corner.x + end_corner.y;
        let llx = start_x_plus_y.min(end_x_plus_y).floor();
        let urx = start_x_plus_y.max(end_x_plus_y).ceil();
        IntOctagon::new(
            lx as i32, ly as i32, rx as i32, uy as i32, ulx as i32, lrx as i32, llx as i32,
            urx as i32,
        )
        .normalize()
    }

    /// Creates a new line segment with the same start and middle line and
    /// an end line, so that the length of the new line segment is about
    /// newLength.
    pub fn change_length_approx(&self, new_length: f64) -> LineSegment {
        let new_end_point = self
            .start_point_approx()
            .change_length(&self.end_point_approx(), new_length);
        let perpendicular_direction = self.middle.direction().turn_45_degree(2);
        let new_end_line =
            Line::new_with_direction(Point::int(new_end_point.round()), perpendicular_direction);
        LineSegment::new(self.start.clone(), self.middle.clone(), new_end_line)
    }

    /// Looks up the intersections of this line segment with other. The
    /// result vector has length 0 (no intersection), 1 (unique
    /// intersection or touching point) or 2 (overlap; the intersection
    /// points are the first and the last overlap point). The result lines
    /// are so that the intersections of the result lines with this line
    /// segment deliver the intersection points. The result is not
    /// symmetric in this and other, because intersecting lines and not
    /// the intersection points are returned (Java `intersection`).
    pub fn intersection(&self, other: &LineSegment) -> Vec<Line> {
        if !self.bounding_box().intersects(&other.bounding_box()) {
            return Vec::new();
        }
        let start_point_side = self.start_point().side_of(&other.middle.a, &other.middle.b);
        let end_point_side = self.end_point().side_of(&other.middle.a, &other.middle.b);
        if start_point_side == Side::Collinear && end_point_side == Side::Collinear {
            // there may be an overlap
            let this_sorted = self.sort_endpoints_in_xy();
            let other_sorted = other.sort_endpoints_in_xy();
            let (left_line, right_line) = if this_sorted
                .start_point()
                .compare_xy(&other_sorted.start_point())
                <= 0
            {
                (&this_sorted, &other_sorted)
            } else {
                (&other_sorted, &this_sorted)
            };
            let cmp = left_line.end_point().compare_xy(&right_line.start_point());
            if cmp < 0 {
                // end point of the left line is to the left of the start
                // point of the right line
                return Vec::new();
            }
            if cmp == 0 {
                // end point of the left line is equal to the start point
                // of the right line
                return vec![left_line.get_end_closing_line().clone()];
            }
            // now there is a real overlap
            let second = if right_line.end_point().compare_xy(&left_line.end_point()) >= 0 {
                left_line.get_end_closing_line().clone()
            } else {
                right_line.get_end_closing_line().clone()
            };
            return vec![right_line.get_start_closing_line().clone(), second];
        }
        if start_point_side == end_point_side
            || other.start_point().side_of(&self.middle.a, &self.middle.b)
                == other.end_point().side_of(&self.middle.a, &self.middle.b)
        {
            return Vec::new(); // no intersection possible
        }
        // now both start points and both end points are on different
        // sides of the middle line of the other segment.
        vec![other.middle.clone()]
    }

    /// Checks if this LineSegment and other contain a common point.
    pub fn intersects(&self, other: &LineSegment) -> bool {
        !self.intersection(other).is_empty()
    }

    /// Checks if this LineSegment and other contain a common LineSegment,
    /// which is not reduced to a point.
    pub fn overlaps(&self, other: &LineSegment) -> bool {
        self.intersection(other).len() > 1
    }

    /// Constructs an approximation of this line segment by orthogonal
    /// stairs with integer coordinates. The length of the stairs will be
    /// at most stairWidth. If toTheRight, the stairs will be to the right
    /// of this line segment, else to the left.
    pub fn stair_approximation(&self, width: f64, to_the_right: bool) -> Vec<IntPoint> {
        let start_point = self.start_point().to_float().round();
        let end_point = self.end_point().to_float().round();
        if start_point == end_point {
            return Vec::new();
        }

        if start_point.x == end_point.x || start_point.y == end_point.y {
            return vec![start_point, end_point];
        }

        let dx = end_point.x.wrapping_sub(start_point.x);
        let dy = end_point.y.wrapping_sub(start_point.y);
        let abs_dx = dx.wrapping_abs();
        let abs_dy = dy.wrapping_abs();
        let function_of_x = abs_dx >= abs_dy;
        // use otherwise function of y for better numerical  stability

        let stair_width;
        let stair_count;
        if function_of_x {
            let w = java_round(width * abs_dx as f64 / abs_dy as f64) as i32;
            let c = (abs_dx - 1) / w + 1;
            stair_count = c;
            stair_width = if end_point.x < start_point.x {
                w.wrapping_neg()
            } else {
                w
            };
        } else {
            let w = java_round(width * abs_dy as f64 / abs_dx as f64) as i32;
            let c = (abs_dy - 1) / w + 1;
            stair_count = c;
            stair_width = if end_point.y < start_point.y {
                w.wrapping_neg()
            } else {
                w
            };
        }
        let mut result = Vec::with_capacity((2 * stair_count + 1) as usize);

        result.push(start_point);
        let det = dx as f64 * dy as f64;
        let change_x_first = to_the_right && det > 0.0 || !to_the_right && det < 0.0;

        let mut prev_line_point_x = start_point.x;
        let mut prev_line_point_y = start_point.y;
        for i in 1..stair_count {
            let current_line_point_x;
            let current_line_point_y;
            if function_of_x {
                current_line_point_x = start_point.x.wrapping_add(i.wrapping_mul(stair_width));
                current_line_point_y = java_round(
                    self.get_line()
                        .function_value_approx(current_line_point_x as f64),
                ) as i32;
            } else {
                current_line_point_y = start_point.y.wrapping_add(i.wrapping_mul(stair_width));
                current_line_point_x = java_round(
                    self.get_line()
                        .function_in_y_value_approx(current_line_point_y as f64),
                ) as i32;
            }
            if change_x_first {
                result.push(IntPoint::new(current_line_point_x, prev_line_point_y));
            } else {
                result.push(IntPoint::new(prev_line_point_x, current_line_point_y));
            }
            result.push(IntPoint::new(current_line_point_x, current_line_point_y));
            prev_line_point_x = current_line_point_x;
            prev_line_point_y = current_line_point_y;
        }
        if change_x_first {
            result.push(IntPoint::new(end_point.x, prev_line_point_y));
        } else {
            result.push(IntPoint::new(prev_line_point_x, end_point.y));
        }
        result.push(end_point);
        result
    }

    /// Constructs an approximation of this line segment by 45 degree
    /// stairs with integer coordinates. The length of the stairs will be
    /// at most stairWidth. If toTheRight, the stairs will be to the right
    /// of this line segment, else to the left.
    pub fn stair_approximation_45(&self, width: f64, to_the_right: bool) -> Vec<IntPoint> {
        let start_point = self.start_point().to_float().round();
        let end_point = self.end_point().to_float().round();
        if start_point == end_point {
            return Vec::new();
        }
        let delta = IntVector::new(
            end_point.x.wrapping_sub(start_point.x),
            end_point.y.wrapping_sub(start_point.y),
        );
        // Java IntVector inherits isMultipleOf45Degree from Vector.
        if Vector::Int(delta).is_multiple_of_45_degree() {
            return vec![start_point, end_point];
        }
        let abs_delta = IntVector::new(delta.x.wrapping_abs(), delta.y.wrapping_abs());
        let function_of_x = abs_delta.x >= abs_delta.y;
        // use otherwise function of y for better numerical  stability
        let det = delta.x as f64 * delta.y as f64;
        let stair_width;
        let stair_count;
        if function_of_x {
            let w = java_round(width * abs_delta.x as f64 / abs_delta.y as f64) as i32;
            let c = (abs_delta.x - 1) / w + 1;
            stair_count = c;
            stair_width = if end_point.x < start_point.x {
                w.wrapping_neg()
            } else {
                w
            };
        } else {
            let w = java_round(width * abs_delta.y as f64 / abs_delta.x as f64) as i32;
            let c = (abs_delta.y - 1) / w + 1;
            stair_count = c;
            stair_width = if end_point.y < start_point.y {
                w.wrapping_neg()
            } else {
                w
            };
        }
        let mut result = Vec::with_capacity((2 * stair_count + 1) as usize);
        result.push(start_point);
        let mut prev_line_point = start_point;
        for i in 1..=stair_count {
            let current_line_point = if i == stair_count {
                end_point
            } else if function_of_x {
                let current_x = start_point.x.wrapping_add(i.wrapping_mul(stair_width));
                let current_y =
                    java_round(self.get_line().function_value_approx(current_x as f64)) as i32;
                IntPoint::new(current_x, current_y)
            } else {
                let current_y = start_point.y.wrapping_add(i.wrapping_mul(stair_width));
                // Java (LineSegment.java:432) calls functionValueApprox
                // here — NOT functionInYValueApprox. Bug-compatible.
                let current_x =
                    java_round(self.get_line().function_value_approx(current_y as f64)) as i32;
                IntPoint::new(current_x, current_y)
            };
            let current_x;
            let current_y;
            if function_of_x {
                let diagonal_first = to_the_right && det < 0.0 || !to_the_right && det > 0.0;
                if diagonal_first {
                    current_x = prev_line_point.x.wrapping_add(
                        Side::as_int(stair_width as f64).wrapping_mul(
                            current_line_point
                                .y
                                .wrapping_sub(prev_line_point.y)
                                .wrapping_abs(),
                        ),
                    );
                    current_y = current_line_point.y;
                } else {
                    // horizontal first
                    current_x = current_line_point.x.wrapping_sub(
                        Side::as_int(stair_width as f64).wrapping_mul(
                            current_line_point
                                .y
                                .wrapping_sub(prev_line_point.y)
                                .wrapping_abs(),
                        ),
                    );
                    current_y = prev_line_point.y;
                }
            } else {
                // function of y
                let diagonal_first = to_the_right && det > 0.0 || !to_the_right && det < 0.0;
                if diagonal_first {
                    current_x = current_line_point.x;
                    current_y = prev_line_point.y.wrapping_add(
                        Side::as_int(stair_width as f64).wrapping_mul(
                            current_line_point
                                .x
                                .wrapping_sub(prev_line_point.x)
                                .wrapping_abs(),
                        ),
                    );
                } else {
                    current_x = prev_line_point.x;
                    current_y = current_line_point.y.wrapping_sub(
                        Side::as_int(stair_width as f64).wrapping_mul(
                            current_line_point
                                .x
                                .wrapping_sub(prev_line_point.x)
                                .wrapping_abs(),
                        ),
                    );
                }
            }
            result.push(IntPoint::new(current_x, current_y));
            result.push(current_line_point);
            prev_line_point = current_line_point;
        }
        result
    }

    /// Inverts the direction of this.middle, if startPoint() has a bigger
    /// x coordinate than endPoint(), or an equal x coordinate and a bigger
    /// y coordinate (Java `sortEndpointsInXY`).
    pub fn sort_endpoints_in_xy(&self) -> LineSegment {
        if self.start_point().compare_xy(&self.end_point()) > 0 {
            LineSegment::new(self.end.clone(), self.middle.clone(), self.start.clone())
        } else {
            self.clone()
        }
    }

    /// Converts this line segment to a simplex (Java `toSimplex`): the
    /// (possibly flipped) start line, the middle line and its opposite,
    /// and the (possibly flipped) end line. The flips put the segment
    /// interior on the left of every border line (jshell pin `PB4`).
    pub fn to_simplex(&self) -> Simplex {
        let first_line = if self.end_point().side_of_line(&self.start) == Side::Negative {
            self.start.opposite()
        } else {
            self.start.clone()
        };
        let third_line = self.middle.opposite();
        let last_line = if self.start_point().side_of_line(&self.end) == Side::Negative {
            self.end.opposite()
        } else {
            self.end.clone()
        };
        Simplex::get_instance(&[first_line, self.middle.clone(), third_line, last_line])
    }

    /// Returns the numbers of the border lines of `shape` intersected by
    /// this line segment, at most 2 entries ordered by distance from the
    /// segment start (Java `borderIntersections(TileShape)`; jshell pins
    /// `PB3`). The middle-line/border-line intersection is computed
    /// unconditionally as in Java — a parallel pair yields an infinite
    /// RationalPoint whose side checks run on exact rational arithmetic
    /// (trap T22; the former Task-6 skip divergence is closed).
    pub fn border_intersections(&self, shape: &TileShape) -> Vec<i32> {
        if !self.bounding_box().intersects(&shape.bounding_box()) {
            return Vec::new();
        }

        let edge_count = shape.border_line_count() as i32;
        let line_start = self.start_point();
        let line_end = self.end_point();

        let mut prev_line = shape.border_line(edge_count - 1);
        let mut current_line = shape.border_line(0);
        let mut result = [0i32; 2];
        let mut intersection: Vec<Point> = Vec::with_capacity(2);

        for edge_line_no in 0..edge_count {
            let next_line = if edge_line_no == edge_count - 1 {
                shape.border_line(0)
            } else {
                shape.border_line(edge_line_no + 1)
            };

            let start_point_side = current_line.side_of(&line_start);
            let end_point_side = current_line.side_of(&line_end);
            if start_point_side == Side::Positive && end_point_side == Side::Positive {
                // both endpoints are outside the border line,
                // no intersection possible
                return Vec::new();
            }

            if start_point_side == Side::Collinear && end_point_side != Side::Negative {
                // the start is on current_line; touches count only if the
                // interior is entered
                return Vec::new();
            }

            if end_point_side == Side::Collinear && start_point_side != Side::Negative {
                // the end is on current_line; touches count only if the
                // interior is entered
                return Vec::new();
            }

            if start_point_side != Side::Negative || end_point_side != Side::Negative {
                // not both points are inside the halfplane defined by
                // current_line
                // Java computes `get_line().intersection(current_line)`
                // unconditionally; a parallel pair yields an infinite
                // RationalPoint and the side checks below decide on it
                // exactly (trap T22).
                let is = self.middle.intersection_point(&current_line);
                let prev_line_side_of_is = prev_line.side_of(&is);
                let next_line_side_of_is = next_line.side_of(&is);
                if prev_line_side_of_is != Side::Positive && next_line_side_of_is != Side::Positive
                {
                    // this line segment intersects current_line between
                    // the previous and the next corner of the shape

                    if prev_line_side_of_is == Side::Collinear {
                        // this line segment goes through the previous corner
                        // of the shape; check that the intersection is not
                        // merely a touch.
                        let prev_prev_corner = if edge_line_no == 0 {
                            shape.corner(edge_count - 1)
                        } else {
                            shape.corner(edge_line_no - 1)
                        };
                        let next_corner = if edge_line_no == edge_count - 1 {
                            shape.corner(0)
                        } else {
                            shape.corner(edge_line_no + 1)
                        };
                        // check that prev_prev_corner and next_corner are on
                        // different sides of this line segment.
                        let prev_prev_corner_side = self.middle.side_of(&prev_prev_corner);
                        let next_corner_side = self.middle.side_of(&next_corner);
                        if prev_prev_corner_side == Side::Collinear
                            || next_corner_side == Side::Collinear
                            || prev_prev_corner_side == next_corner_side
                        {
                            return Vec::new();
                        }
                    }
                    if next_line_side_of_is == Side::Collinear {
                        // this line segment goes through the next corner of
                        // the shape; check that the intersection is not
                        // merely a touch.
                        let prev_corner = shape.corner(edge_line_no);
                        let next_next_corner = if edge_line_no == edge_count - 2 {
                            shape.corner(0)
                        } else if edge_line_no == edge_count - 1 {
                            shape.corner(1)
                        } else {
                            shape.corner(edge_line_no + 2)
                        };
                        // check that prev_corner and next_next_corner are on
                        // different sides of this line segment.
                        let prev_corner_side = self.middle.side_of(&prev_corner);
                        let next_next_corner_side = self.middle.side_of(&next_next_corner);
                        if prev_corner_side == Side::Collinear
                            || next_next_corner_side == Side::Collinear
                            || prev_corner_side == next_next_corner_side
                        {
                            return Vec::new();
                        }
                    }
                    let mut intersection_already_handled = false;
                    for found in &intersection {
                        if *found == is {
                            intersection_already_handled = true;
                            break;
                        }
                    }
                    if !intersection_already_handled && intersection.len() < 2 {
                        // a new intersection is found; else Java logs
                        // "intersection_count too big" and keeps the first
                        // two intersections.
                        result[intersection.len()] = edge_line_no;
                        intersection.push(is);
                    }
                }
            }

            prev_line = current_line.clone();
            current_line = next_line;
        }

        if intersection.is_empty() {
            return Vec::new();
        }

        if intersection.len() == 2 {
            // assure the correct order
            let current_start = line_start.to_float();
            if current_start.distance_square(&intersection[1].to_float())
                < current_start.distance_square(&intersection[0].to_float())
            {
                result.swap(0, 1);
            }
            return result.to_vec();
        }

        // Java warns when intersectionCount != 1 here; the observable
        // result is the first entry.
        vec![result[0]]
    }

    /// Creates a line segment from `polyline`, which starts at the line
    /// with number `no - 1`, continues along the line with number `no`
    /// and ends at the line with number `no + 1` (Java
    /// `LineSegment(Polyline, int)`). Java indexes the array directly;
    /// an out-of-range `no` leaves null fields that NPE at first use, so
    /// the port panics eagerly with the same meaning.
    pub fn from_polyline(polyline: &Polyline, no: i32) -> LineSegment {
        if no < 1 || no + 1 >= polyline.lines.len() as i32 {
            panic!("LineSegment(Polyline, int): line no out of range (Java null-field NPE)");
        }
        LineSegment::new(
            polyline.lines[(no - 1) as usize].clone(),
            polyline.lines[no as usize].clone(),
            polyline.lines[(no + 1) as usize].clone(),
        )
    }

    /// Returns this segment as a `Polyline` of the 3 lines
    /// `[start, middle, end]` through the normalizing Java constructor
    /// (`Polyline(Line[])`; dog-ear / overlap / direction-flip
    /// normalization applies, trap T10).
    pub fn to_polyline(&self) -> Polyline {
        Polyline::new(vec![
            self.start.clone(),
            self.middle.clone(),
            self.end.clone(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regular_tile_shape::RegularTileShape;

    fn line(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::from_int_coords(ax, ay, bx, by)
    }

    fn seg(
        start: (i32, i32, i32, i32),
        middle: (i32, i32, i32, i32),
        end: (i32, i32, i32, i32),
    ) -> LineSegment {
        LineSegment::new(
            line(start.0, start.1, start.2, start.3),
            line(middle.0, middle.1, middle.2, middle.3),
            line(end.0, end.1, end.2, end.3),
        )
    }

    /// jshell pins `SP1`/`SPA1`/`BB1`/`BO1`/`OP1` on
    /// s1 = LineSegment(Line(-5,-10,-5,10), Line(-10,0,10,0),
    /// Line(5,-10,5,10)): corner points (exact and approx), bounding box,
    /// bounding octagon and the closing-line rotation of `opposite`.
    #[test]
    fn s1_geometry_fundamentals() {
        let s1 = seg((-5, -10, -5, 10), (-10, 0, 10, 0), (5, -10, 5, 10));
        // SP1: corners are (-5, 0) and (5, 0).
        assert_eq!(s1.start_point(), Point::int(IntPoint::new(-5, 0)));
        assert_eq!(s1.end_point(), Point::int(IntPoint::new(5, 0)));
        // SPA1: startPointApprox = (-0x1.4p2, 0x0.0p0) = (-5.0, 0.0).
        assert_eq!(s1.start_point_approx(), FloatPoint::new(-5.0, 0.0));
        // BB1: bounding box (-5, 0) .. (5, 0).
        assert_eq!(
            s1.bounding_box(),
            IntBox::new(IntPoint::new(-5, 0), IntPoint::new(5, 0))
        );
        // BO1: bounding octagon fields leftX..upperRightDiagonalX =
        // -5,0,5,0,-5,5,-5,5.
        assert_eq!(
            s1.bounding_octagon(),
            IntOctagon::new(-5, 0, 5, 0, -5, 5, -5, 5)
        );
        // OP1: opposite() start closing line 5,10->5,-10 and middle line
        // 10,0->-10,0 (the end/middle/start rotation).
        let opp = s1.opposite();
        assert_eq!(opp.get_start_closing_line(), &line(5, 10, 5, -10));
        assert_eq!(opp.get_line(), &line(10, 0, -10, 0));
        // Accessors hand back the stored triple.
        assert_eq!(s1.get_start_closing_line(), &line(-5, -10, -5, 10));
        assert_eq!(s1.get_end_closing_line(), &line(5, -10, 5, 10));
    }

    /// jshell pins `I1`-`I6`: the three outcomes of `intersection`
    /// (single crossing line / two overlap closing lines / empty), the
    /// intersects/overlaps predicates and their asymmetry (o1 does not
    /// overlap s2 even though both cross).
    ///
    /// jshell transcript:
    /// ```text
    /// I1 count 1   line -2,0->8,10
    /// I2 count 2   lines 5,-5->5,5 and 10,-5->10,5
    /// I3 0  I4 true  I5 true  I6 false
    /// ```
    #[test]
    fn intersection_crossing_overlap_and_disjoint() {
        let s1 = seg((-5, -10, -5, 10), (-10, 0, 10, 0), (5, -10, 5, 10));
        let s2 = seg((-5, -10, -5, 10), (-2, 0, 8, 10), (5, -10, 5, 10));
        let o1 = seg((0, -5, 0, 5), (-10, 0, 10, 0), (10, -5, 10, 5));
        let o2 = seg((5, -5, 5, 5), (-10, 0, 20, 0), (15, -5, 15, 5));
        let far = seg((100, -5, 100, 5), (90, 0, 110, 0), (110, -5, 110, 5));
        // I1: crossing -> the middle line of s2.
        let x1 = s1.intersection(&s2);
        assert_eq!(x1.len(), 1);
        assert_eq!(x1[0], line(-2, 0, 8, 10));
        // I2: collinear overlap -> the start closing line of the right
        // segment and the end closing line of the left segment.
        let x2 = o1.intersection(&o2);
        assert_eq!(x2.len(), 2);
        assert_eq!(x2[0], line(5, -5, 5, 5));
        assert_eq!(x2[1], line(10, -5, 10, 5));
        // I3: disjoint -> empty.
        assert!(o1.intersection(&far).is_empty());
        // I4/I5: o1 vs o2 intersect and overlap.
        assert!(o1.intersects(&o2));
        assert!(o1.overlaps(&o2));
        // I6: o1 and s2 do not overlap; they do not even cross, because
        // s2's middle line (y = x + 2) meets y = 0 at x = -2, outside
        // o1's span [0, 10].
        assert!(!o1.intersects(&s2));
        assert!(!o1.overlaps(&s2));
    }

    /// jshell pins `CL1`/`SO`/`CT1`/`CT2`: changeLengthApprox shortens
    /// s1 to length 4 behind a vertical closing line through (-1, 0);
    /// sortEndpointsInXY swaps the closing lines when the start corner
    /// sorts after the end corner; contains distinguishes (10, 5) on the
    /// st middle line from (10, 6) off it.
    #[test]
    fn change_length_sort_and_contains() {
        let s1 = seg((-5, -10, -5, 10), (-10, 0, 10, 0), (5, -10, 5, 10));
        // CL1: end closing line -1,0->-1,1.
        assert_eq!(
            s1.change_length_approx(4.0).get_end_closing_line(),
            &line(-1, 0, -1, 1)
        );
        // SO: closing lines -5,1->5,1|-5,-1->5,-1 swap to
        // -5,-1->5,-1|-5,1->5,1.
        let so = seg((-5, 1, 5, 1), (0, -10, 0, 10), (-5, -1, 5, -1));
        let sorted = so.sort_endpoints_in_xy();
        assert_eq!(sorted.get_start_closing_line(), &line(-5, -1, 5, -1));
        assert_eq!(sorted.get_end_closing_line(), &line(-5, 1, 5, 1));
        // A already-sorted segment is returned unchanged.
        let sorted_twice = sorted.sort_endpoints_in_xy();
        assert_eq!(sorted_twice.get_start_closing_line(), &line(-5, -1, 5, -1));
        assert_eq!(sorted_twice.get_end_closing_line(), &line(-5, 1, 5, 1));
        // CT1/CT2.
        let st = seg((0, -2, 0, 2), (0, 0, 20, 10), (20, 18, 20, 22));
        assert!(st.contains(&Point::int(IntPoint::new(10, 5))));
        assert!(!st.contains(&Point::int(IntPoint::new(10, 6))));
        // Non-IntPoint input is rejected with false (Java logs a warning).
        assert!(
            !st.contains(&Point::rational(crate::rational_point::RationalPoint::new(
                num_bigint::BigInt::from(10),
                num_bigint::BigInt::from(5),
                num_bigint::BigInt::from(2),
            )))
        );
    }

    /// jshell pins `ST1`/`ST2`: orthogonal and 45-degree stair
    /// approximations of st = LineSegment(Line(0,-2,0,2), Line(0,0,20,10),
    /// Line(20,18,20,22)) with stair width 3 to the right.
    #[test]
    fn stair_approximations() {
        let st = seg((0, -2, 0, 2), (0, 0, 20, 10), (20, 18, 20, 22));
        let expected: Vec<IntPoint> = [
            (0, 0),
            (6, 0),
            (6, 3),
            (12, 3),
            (12, 6),
            (18, 6),
            (18, 9),
            (20, 9),
            (20, 10),
        ]
        .iter()
        .map(|(x, y)| IntPoint::new(*x, *y))
        .collect();
        assert_eq!(st.stair_approximation(3.0, true), expected); // ST1
        let expected45: Vec<IntPoint> = [
            (0, 0),
            (3, 0),
            (6, 3),
            (9, 3),
            (12, 6),
            (15, 6),
            (18, 9),
            (19, 9),
            (20, 10),
        ]
        .iter()
        .map(|(x, y)| IntPoint::new(*x, *y))
        .collect();
        assert_eq!(st.stair_approximation_45(3.0, true), expected45); // ST2
        // The 45-degree path short-circuits to [start, end] when the
        // delta is already a multiple of 45 degrees.
        let straight = seg((0, -2, 0, 2), (0, 0, 20, 0), (20, -2, 20, 2));
        assert_eq!(
            straight.stair_approximation_45(3.0, true),
            vec![IntPoint::new(0, 0), IntPoint::new(20, 0)]
        );
        // Degenerate zero-length segments produce no stairs.
        let zero = seg((0, 0, 1, 0), (0, 0, 0, 1), (0, 0, -1, 0));
        assert!(zero.stair_approximation(3.0, true).is_empty());
    }

    /// jshell pins `S45`/`SOR` on the vertical-ish segment
    /// vst = LineSegment(Line(-1,0,1,0), Line(0,0,10,20), Line(9,20,11,20))
    /// (corners (0,0)-(10,20), |dy| = 20 > |dx| = 10, so BOTH
    /// function-of-y branches execute — pin ST2 never reaches them):
    /// S45 freezes the LineSegment.java:432 defect (trap T17), where y is
    /// fed into functionValueApprox and produces the garbage x columns
    /// 12/24/36 and the overshooting (10, 44); SOR pins the orthogonal
    /// stair's function-of-y path (stair width 6, risers at y = 6/12/18).
    #[test]
    fn stair_function_of_y_branches() {
        let vst = seg((-1, 0, 1, 0), (0, 0, 10, 20), (9, 20, 11, 20));
        let expected45: Vec<IntPoint> = [
            (0, 0),
            (12, 12),
            (12, 6),
            (24, 18),
            (24, 12),
            (36, 24),
            (36, 18),
            (10, 44),
            (10, 20),
        ]
        .iter()
        .map(|(x, y)| IntPoint::new(*x, *y))
        .collect();
        assert_eq!(vst.stair_approximation_45(3.0, true), expected45); // S45
        let expected_orth: Vec<IntPoint> = [
            (0, 0),
            (3, 0),
            (3, 6),
            (6, 6),
            (6, 12),
            (9, 12),
            (9, 18),
            (10, 18),
            (10, 20),
        ]
        .iter()
        .map(|(x, y)| IntPoint::new(*x, *y))
        .collect();
        assert_eq!(vst.stair_approximation(3.0, true), expected_orth); // SOR
    }

    /// jshell pin `PB4`: `seg1.toSimplex()` — border lines in the
    /// Simplex's canonical direction-sorted order (E, UP, W, DOWN); the
    /// flips put the segment interior on the left of every border line.
    #[test]
    fn pb4_to_simplex_pin() {
        let seg1 = seg((3, -7, 4, -7), (3, -7, 3, -6), (3, 8, 4, 8));
        let simplex = seg1.to_simplex();
        assert_eq!(simplex.border_line_count(), 4);
        assert_eq!(simplex.border_line(0), line(3, -7, 4, -7));
        assert_eq!(simplex.border_line(1), line(3, -7, 3, -6));
        assert_eq!(simplex.border_line(2), line(4, 8, 3, 8));
        assert_eq!(simplex.border_line(3), line(3, -6, 3, -7));
    }

    /// jshell pins `PB3`: `borderIntersections` against the box
    /// (0,0,10,10): seg1 crosses the bottom border -> [0]; seg2 (fully
    /// inside) finds nothing (both endpoints on the left of the bottom
    /// border line). The oracle's seg3 is degenerate — all three lines
    /// horizontal, so Java's `startPoint()` is an infinite RationalPoint
    /// (trap T22, now built identically here); two non-degenerate
    /// substitutes hit the same branches: seg3a below the box (parallel
    /// middle line — the candidate is computed through the infinite
    /// point exactly, as Java does) and seg3b with its end point exactly
    /// on the bottom border (the collinear early return).
    #[test]
    fn pb3_border_intersections_pins() {
        let box_tile = TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            TileShape::from_4_ints(0, 0, 10, 10),
        ));
        let seg1 = seg((3, -7, 4, -7), (3, -7, 3, -6), (3, 8, 4, 8));
        assert_eq!(seg1.border_intersections(&box_tile), vec![0]);
        let seg2 = seg((3, 1, 4, 1), (3, 1, 3, 2), (3, 9, 4, 9));
        assert!(seg2.border_intersections(&box_tile).is_empty());
        // seg3a: fully below the box; the middle line is parallel to the
        // bottom border line (the skip branch) and the right-wall
        // candidate is rejected by the next-line check.
        let seg3a = seg((3, -5, 3, -6), (3, -5, 4, -5), (5, -5, 5, -4));
        assert!(seg3a.border_intersections(&box_tile).is_empty());
        // seg3b: the end point lies exactly on the bottom border line
        // while the start is outside the halfplane: collinear early
        // return.
        let seg3b = seg((3, 1, 3, 0), (3, 1, 4, 1), (5, 0, 5, -1));
        assert!(seg3b.border_intersections(&box_tile).is_empty());
    }

    fn rat(x: i64, y: i64, z: i64) -> Point {
        Point::rational(crate::rational_point::RationalPoint::new(
            num_bigint::BigInt::from(x),
            num_bigint::BigInt::from(y),
            num_bigint::BigInt::from(z),
        ))
    }

    /// jshell pins `SORT-INF1`/`SORT-INF2`/`SORT-INF3` and `CMP-INF1`/`CMP-INF2`
    /// (corpus cases seg-000020/93/125, trap T22): corner points of segments
    /// with a closing line PARALLEL to the middle line are infinite
    /// RationalPoints (z = 0); `compareXY` decides on them exactly, and
    /// `sortEndpointsInXY` swaps or keeps the closing lines accordingly —
    /// bit-identical to the Java oracle, which never panics here.
    #[test]
    fn sort_endpoints_with_parallel_closing_lines() {
        // seg-000020: sp = Rat(822986573811360, 822986573811360, 0),
        // ep = Int(-119856304, 9102); sp sorts AFTER ep, so the closing
        // lines swap (start <- e, middle <- m, end <- s).
        let s20 = seg(
            (29065462, -5356, 29067219, -3599),
            (-119870338, -4932, -119873483, -8077),
            (-5751, 9102, -4566, 9102),
        );
        assert_eq!(s20.start_point(), rat(822986573811360, 822986573811360, 0)); // SORT-INF1
        assert_eq!(s20.end_point(), Point::int(IntPoint::new(-119856304, 9102)));
        assert_eq!(s20.start_point().compare_xy(&s20.end_point()), 1); // CMP-INF1
        let sorted20 = s20.sort_endpoints_in_xy();
        assert_eq!(
            sorted20.get_start_closing_line(),
            &line(-5751, 9102, -4566, 9102)
        );
        assert_eq!(
            sorted20.get_line(),
            &line(-119870338, -4932, -119873483, -8077)
        );
        assert_eq!(
            sorted20.get_end_closing_line(),
            &line(29065462, -5356, 29067219, -3599)
        );

        // seg-000093: sp = Rat(0, -34839094706, 0), ep = Int(2146,
        // 235305820); sp sorts BEFORE ep, so the triple is kept as-is.
        let s93 = seg(
            (8237, 2759, 8237, 432),
            (2146, 14305889, 2146, 14303431),
            (-2517, 235305820, 341, 235305820),
        );
        assert_eq!(s93.start_point(), rat(0, -34839094706, 0)); // SORT-INF2
        assert_eq!(s93.end_point(), Point::int(IntPoint::new(2146, 235305820)));
        assert_eq!(s93.start_point().compare_xy(&s93.end_point()), -1);
        let sorted93 = s93.sort_endpoints_in_xy();
        assert_eq!(
            sorted93.get_start_closing_line(),
            &line(8237, 2759, 8237, 432)
        );
        assert_eq!(
            sorted93.get_end_closing_line(),
            &line(-2517, 235305820, 341, 235305820)
        );

        // seg-000125: sp = Int(26732269, 567239809), the END corner is the
        // infinite one here: Rat(0, 1791925913988, 0); still sp before ep.
        let s125 = seg(
            (-29884187, 623856265, -29887963, 623860041),
            (26732269, -7725, 26732269, -7534),
            (5458321, 21233336, 5458321, 21233777),
        );
        assert_eq!(
            s125.start_point(),
            Point::int(IntPoint::new(26732269, 567239809))
        );
        assert_eq!(s125.end_point(), rat(0, 1791925913988, 0)); // SORT-INF3
        assert_eq!(s125.start_point().compare_xy(&s125.end_point()), -1); // CMP-INF2
        let sorted125 = s125.sort_endpoints_in_xy();
        assert_eq!(
            sorted125.get_start_closing_line(),
            &line(-29884187, 623856265, -29887963, 623860041)
        );
        assert_eq!(
            sorted125.get_end_closing_line(),
            &line(5458321, 21233336, 5458321, 21233777)
        );
    }

    /// jshell pin `I-INF` (corpus case seg-000151, trap T22): both
    /// segments have a closing line parallel to their middle line, yet
    /// Java's `intersection` degrades gracefully — the infinite corner
    /// points are collinear with / on definite sides of the other middle
    /// line, and the single crossing line (other's middle) is returned.
    #[test]
    fn intersection_through_infinite_corner_points() {
        let a = seg(
            (-4478, -1894, -7376, 1004),
            (8326, -8949, 8948, -9571),
            (5386, -846, 5386, 1172),
        );
        let b = seg(
            (-3193, -5007, -3193, -6455),
            (14717832, -17304286, 14720647, -17307101),
            (6198, -9568, 7570, -10940),
        );
        assert_eq!(
            a.intersection(&b),
            vec![line(14717832, -17304286, 14720647, -17307101)] // I-INF
        );
    }

    /// THE buglog-169 construction-face pin (T17a): a segment's end
    /// corner is `middle.intersection(end)` — the EXACT rational of
    /// Java `Line.intersection` (Line.java:277-303 returns
    /// `new RationalPoint(isX, isY, det)`), never a rounded or
    /// int-truncated value. The end corner of this corridor segment
    /// is (9_799_970_000, 6_000_000_000) / 20_000 = (489998.5,
    /// 300000): the exact triple survives only when the BigInteger
    /// path runs unrounded — a float-rounding or int-truncating
    /// mutant answers `Int(489998)` / a doubled 489998.0 and dies
    /// here. The float face of the SAME corner (the Java
    /// `intersectionApprox` domain) is exactly representable: 0.5.
    #[test]
    fn t17_end_point_is_the_exact_rational_corner() {
        let start = line(480000, 300000, 480000, 301000);
        let middle = line(480000, 300000, 490000, 300000);
        let end = line(489999, 300001, 490000, 300003);
        let segment = LineSegment::new(start, middle, end);
        assert_eq!(
            segment.end_point(),
            Point::rational(crate::rational_point::RationalPoint::new(
                num_bigint::BigInt::from(9_799_970_000_i64),
                num_bigint::BigInt::from(6_000_000_000_i64),
                num_bigint::BigInt::from(20_000_i64),
            ))
        );
        assert_eq!(
            segment.end_point_approx(),
            FloatPoint::new(489998.5, 300000.0)
        );
    }
}
