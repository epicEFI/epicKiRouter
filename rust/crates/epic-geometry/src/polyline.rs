//! Port of Java `app.freerouting.geometry.planar.Polyline` — a sequence of
//! lines, where no 2 consecutive lines may be parallel. A Polyline of n
//! lines defines a Polygon of n-1 intersection points of consecutive lines.
//!
//! Bit-parity notes:
//! - Trap T10 (constructor normalization order is observable): the
//!   `Line[]` constructor first filters consecutive parallels, then
//!   removes overlaps, and only THEN flips line directions — the flipping
//!   decision uses `side_of` of the FLOAT corner approximations
//!   (`intersection_approx`) and can disagree with the exact rational
//!   side. The port keeps this order and the float predicate.
//! - Trap T14: Java memoizes `precalculatedCorners` /
//!   `precalculatedFloatCorners` / `precalculatedBoundingBox` in transient
//!   fields; the port recomputes fresh on every call (pure functions).
//! - `bounding_box` / `bounding_octagon` use Java `(int)` narrowing casts
//!   of `Math.floor` / `Math.ceil` results (saturating like Java).
//! - Java `Polyline` has no `equals` / `fastEquals` override in the v2.3
//!   oracle; the Rust [`Polyline::fast_equals`] / [`Polyline::equals`]
//!   helpers below are additive element-wise comparisons over
//!   [`Line::fast_equals`] / line equality (used by tests and later
//!   ports; they do not change any oracle behavior).
//!
//! Deferral ledger (Task 8): `combine`, `split`, `skipLines`,
//! `offsetBox`, `nearestPointApprox`, `distance`, `projectionLine` and
//! `shorten` are not ported yet — they belong to the M2 autoroute
//! surface and are re-ledgered there. `offsetShapes` /
//! `offsetShape(int, int)` are ported (Task 8 scope).

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::line::Line;
use crate::line_segment::LineSegment;
use crate::point::Point;
use crate::polygon::Polygon;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Java `USE_BOUNDING_OCTAGON_FOR_OFFSET_SHAPES` (compile-time constant).
const USE_BOUNDING_OCTAGON_FOR_OFFSET_SHAPES: bool = true;

/// Java `Polyline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Polyline {
    /// The lines of this polyline (Java public final field).
    pub lines: Vec<Line>,
}

impl Polyline {
    /// Creates a polyline of length `polygon.corner_count() + 1` from
    /// polygon, so that the i-th corner of polygon is the intersection of
    /// the i-th and the (i+1)-th lines (Java `Polyline(Polygon)`).
    /// `polygon` must have at least 2 corners; with fewer corners the
    /// result is the empty polyline (Java logs a warning).
    pub fn from_polygon(polygon: &Polygon) -> Polyline {
        let points = polygon.corner_array();
        if points.len() < 2 {
            // Java: FRLogger.warn("Polyline: must contain at least 2
            // different points");
            return Polyline { lines: Vec::new() };
        }
        let mut lines: Vec<Line> = Vec::with_capacity(points.len() + 1);
        lines.push(Line::new(points[0].clone(), points[0].clone())); // placeholder, filled below
        for i in 1..points.len() {
            lines.push(Line::new(points[i - 1].clone(), points[i].clone()));
        }
        // construct perpendicular lines at the start and at the end to
        // represent the first and the last point of points as intersection
        // of lines.
        let dir = Direction::get_instance_from_points(&points[0], &points[1])
            .expect("distinct points have a direction");
        let start_line = Line::get_instance(points[0].clone(), dir.turn_45_degree(2));

        let dir = Direction::get_instance_from_points(
            &points[points.len() - 1],
            &points[points.len() - 2],
        )
        .expect("distinct points have a direction");
        let end_line = Line::get_instance(points[points.len() - 1].clone(), dir.turn_45_degree(2));
        lines[0] = start_line;
        lines.push(end_line);
        Polyline { lines }
    }

    /// Creates a polyline from an array of points (Java `Polyline(Point[])`).
    pub fn from_points(points: &[Point]) -> Polyline {
        Polyline::from_polygon(&Polygon::new(points))
    }

    /// Creates a polyline consisting of three lines (Java
    /// `Polyline(Point, Point)`). The empty polyline if the corners match.
    pub fn from_two_corners(from_corner: &Point, to_corner: &Point) -> Polyline {
        if from_corner == to_corner {
            return Polyline { lines: Vec::new() };
        }
        let dir = Direction::get_instance_from_points(from_corner, to_corner)
            .expect("distinct points have a direction");
        let lines = vec![
            Line::get_instance(from_corner.clone(), dir.turn_45_degree(2)),
            Line::new(from_corner.clone(), to_corner.clone()),
            Line::get_instance(
                to_corner.clone(),
                Direction::get_instance_from_points(from_corner, to_corner)
                    .expect("distinct points have a direction")
                    .turn_45_degree(2),
            ),
        ];
        Polyline { lines }
    }

    /// Creates a polyline from an array of lines. Lines which are parallel
    /// to the previous line are skipped. The directed lines are normalized,
    /// so that they intersect the previous line before the next line (Java
    /// `Polyline(Line[])` — trap T10, see the module docs).
    pub fn new(input_lines: Vec<Line>) -> Polyline {
        let mut filtered_lines = remove_consecutive_parallel_lines(input_lines);
        filtered_lines = remove_overlaps(filtered_lines);
        if filtered_lines.len() < 3 {
            return Polyline { lines: Vec::new() };
        }

        // turn evtl the direction of the lines that they point always
        // from the previous corner to the next corner. The decision is
        // taken on the FLOAT corner approximations (T10).
        let mut filtered_lines = filtered_lines;
        for i in 1..filtered_lines.len() - 1 {
            let precalculated_float_corner =
                filtered_lines[i].intersection_approx(&filtered_lines[i + 1]);
            let side_of_line =
                filtered_lines[i - 1].side_of_float_zero(&precalculated_float_corner);
            if side_of_line != Side::Collinear {
                let d0 = filtered_lines[i - 1].direction();
                let d1 = filtered_lines[i].direction();
                let side1 = d0.side_of(d1);
                if side1 != side_of_line {
                    filtered_lines[i] = filtered_lines[i].opposite();
                }
            }
        }
        Polyline {
            lines: filtered_lines,
        }
    }

    /// Returns the number of lines minus 1 (Java `cornerCount()`).
    pub fn corner_count(&self) -> usize {
        self.lines.len().wrapping_sub(1)
    }

    /// Java `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.lines.len() < 3
    }

    /// Checks if this polyline is empty or if all corner points are equal
    /// (Java `isPoint()`).
    pub fn is_point(&self) -> bool {
        if self.lines.len() < 3 {
            return true;
        }
        let first_corner = self
            .corner(0)
            .expect("polyline with >= 3 lines has corners");
        for i in 1..self.lines.len() - 1 {
            if self.corner(i as i32).expect("corner in range") != first_corner {
                return false;
            }
        }
        true
    }

    /// Checks if all lines of this polyline are orthogonal.
    pub fn is_orthogonal(&self) -> bool {
        self.lines.iter().all(|line| line.is_orthogonal())
    }

    /// Checks if all lines of this polyline are multiples of 45 degrees.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.lines
            .iter()
            .all(|line| line.is_multiple_of_45_degree())
    }

    /// Returns the intersection of the first line with the second line.
    pub fn first_corner(&self) -> Option<Point> {
        self.corner(0)
    }

    /// Returns the intersection of the last line with the line before the
    /// last line.
    pub fn last_corner(&self) -> Option<Point> {
        self.corner(self.lines.len() as i32 - 2)
    }

    /// Returns the array of the intersections of two consecutive lines as
    /// exact `Point`s (Java `corners()`; uncached recompute, T14).
    pub fn corners(&self) -> Vec<Point> {
        if self.lines.len() < 2 {
            return Vec::new();
        }
        (0..self.lines.len() - 1)
            .map(|i| {
                self.lines[i]
                    .intersection(&self.lines[i + 1])
                    .expect("consecutive polyline lines intersect")
            })
            .collect()
    }

    /// Returns the array of intersections of consecutive lines, approximated
    /// by `FloatPoint` values (Java `cornerApproxArr()`).
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        if self.lines.len() < 2 {
            return Vec::new();
        }
        (0..self.lines.len() - 1)
            .map(|i| self.lines[i].intersection_approx(&self.lines[i + 1]))
            .collect()
    }

    /// Returns an approximation of the intersection of the `corner_index`-th
    /// with the (`corner_index + 1`)-th line (Java `cornerApprox(int)`).
    /// Out-of-range indices clamp like Java (which warns).
    pub fn corner_approx(&self, corner_index: i32) -> FloatPoint {
        let no = self.clamp_corner_no(corner_index);
        self.lines[no as usize].intersection_approx(&self.lines[no as usize + 1])
    }

    /// Returns the intersection of the `corner_index`-th with the
    /// (`corner_index + 1`)-th edge line (Java `corner(int)`).
    /// `None` where Java returns null (`lines.length < 2`).
    pub fn corner(&self, corner_index: i32) -> Option<Point> {
        if self.lines.len() < 2 {
            return None;
        }
        let no = self.clamp_corner_no(corner_index);
        Some(
            self.lines[no as usize]
                .intersection(&self.lines[no as usize + 1])
                .expect("consecutive polyline lines intersect"),
        )
    }

    /// Java's index clamp shared by `corner` / `cornerApprox`
    /// (Java warns; the port clamps silently).
    fn clamp_corner_no(&self, corner_index: i32) -> i32 {
        if corner_index < 0 {
            0
        } else if corner_index >= self.lines.len() as i32 - 1 {
            self.lines.len() as i32 - 2
        } else {
            corner_index
        }
    }

    /// Returns the polyline with the reversed order of lines (Java
    /// `reverse()`; re-normalized through the constructor).
    pub fn reverse(&self) -> Polyline {
        let reversed_lines: Vec<Line> = (0..self.lines.len())
            .map(|i| self.lines[self.lines.len() - i - 1].opposite())
            .collect();
        Polyline::new(reversed_lines)
    }

    /// Calculates the length of this polyline from `requested_from_corner`
    /// to `requested_to_corner` (Java `lengthApprox(int, int)`).
    pub fn length_approx(&self, requested_from_corner: i32, requested_to_corner: i32) -> f64 {
        let from_corner = requested_from_corner.max(0);
        let to_corner = requested_to_corner.min(self.lines.len() as i32 - 2);
        let mut result = 0.0f64;
        for i in from_corner..to_corner {
            result += self.corner_approx(i + 1).distance(&self.corner_approx(i));
        }
        result
    }

    /// Calculates the cumulative distance between consecutive corners (Java
    /// `lengthApprox()`).
    pub fn length_approx_total(&self) -> f64 {
        self.length_approx(0, self.lines.len() as i32 - 2)
    }

    /// Calculates for each line between `from_no` and `to_no` a shape around
    /// this line where the right and left edge lines have the distance
    /// `half_width` from the center line (Java `offsetShapes(int, int, int)`).
    pub fn offset_shapes(
        &self,
        half_width: i32,
        requested_from_no: i32,
        requested_to_no: i32,
    ) -> Vec<TileShape> {
        let from_no = requested_from_no.max(0);
        let to_no = requested_to_no.min(self.lines.len() as i32 - 1);
        let shape_count = (to_no - from_no - 1).max(0);
        let mut shapes: Vec<TileShape> = Vec::new();
        if shape_count == 0 {
            return shapes;
        }
        shapes.reserve(shape_count as usize);
        let mut prev_dir = self.lines[from_no as usize].direction().get_vector();
        let mut current_direction = self.lines[from_no as usize + 1].direction().get_vector();
        for i in from_no + 1..to_no {
            let next_dir = self.lines[i as usize + 1].direction().get_vector();

            let mut offset_lines: Vec<Line> =
                vec![Line::new(Point::Int(IntPoint::ZERO), Point::Int(IntPoint::ZERO)); 4];

            offset_lines[0] = self.lines[i as usize].translate(-(half_width as f64));
            // current center line translated to the right

            // create the front line of the offset shape
            let next_dir_from_curr_dir = next_dir.side_of(&current_direction);
            // left turn from currentLine to nextLine
            offset_lines[1] = if next_dir_from_curr_dir == Side::Positive {
                self.lines[i as usize + 1].translate(-(half_width as f64))
                // next right line
            } else {
                self.lines[i as usize + 1]
                    .opposite()
                    .translate(-(half_width as f64))
                // next left line in opposite direction
            };

            offset_lines[2] = self.lines[i as usize]
                .opposite()
                .translate(-(half_width as f64));
            // current left line in opposite direction

            // create the back line of the offset shape
            let current_dir_from_prev_dir = current_direction.side_of(&prev_dir);
            // left turn from prevLine to currentLine
            offset_lines[3] = if current_dir_from_prev_dir == Side::Positive {
                self.lines[i as usize - 1].translate(-(half_width as f64))
                // previous line translated to the right
            } else {
                self.lines[i as usize - 1]
                    .opposite()
                    .translate(-(half_width as f64))
                // previous left line in opposite direction
            };

            // cut off outstanding corners with following shapes
            let mut corner_to_check = FloatPoint::new(0.0, 0.0);
            let mut current_line = offset_lines[1].clone();
            let check_line: Line = if next_dir_from_curr_dir == Side::Positive {
                offset_lines[2].clone()
            } else {
                offset_lines[0].clone()
            };
            let check_distance_corner = self.corner_approx(i);
            let check_dist_square = 2.0 * half_width as f64 * half_width as f64;
            let mut cut_dog_ear_lines: Vec<Line> = Vec::new();
            let mut tmp_curr_dir = next_dir.clone();
            let mut direction_changed = false;
            let mut j = i + 2;
            while j < self.lines.len() as i32 - 1 {
                if self
                    .corner_approx(j - 1)
                    .distance_square(&check_distance_corner)
                    > check_dist_square
                {
                    break;
                }
                if !direction_changed {
                    corner_to_check = current_line.intersection_approx(&check_line);
                }
                let tmp_next_dir = self.lines[j as usize].direction().get_vector();
                let tmp_next_dir_from_tmp_curr_dir = tmp_next_dir.side_of(&tmp_curr_dir);
                direction_changed = tmp_next_dir_from_tmp_curr_dir != next_dir_from_curr_dir;
                if !direction_changed {
                    let next_border_line = if tmp_next_dir_from_tmp_curr_dir == Side::Positive {
                        self.lines[j as usize].translate(-(half_width as f64))
                    } else {
                        self.lines[j as usize]
                            .opposite()
                            .translate(-(half_width as f64))
                    };

                    if next_border_line.side_of_float_zero(&corner_to_check) == Side::Positive
                        && next_border_line.side_of(&self.corner(i).expect("corner in range"))
                            == Side::Negative
                        && next_border_line.side_of(&self.corner(i - 1).expect("corner in range"))
                            == Side::Negative
                    {
                        // an outstanding corner
                        cut_dog_ear_lines.push(next_border_line.clone());
                    }
                    tmp_curr_dir = tmp_next_dir;
                    current_line = next_border_line;
                }
                j += 1;
            }

            // cut off outstanding corners with previous shapes
            let check_distance_corner = self.corner_approx(i - 1);
            let check_line: Line = if current_dir_from_prev_dir == Side::Positive {
                offset_lines[2].clone()
            } else {
                offset_lines[0].clone()
            };
            let mut current_line = offset_lines[3].clone();
            let mut tmp_curr_dir = prev_dir.clone();
            let mut direction_changed = false;
            let mut j = i - 2;
            while j >= 1 {
                if self
                    .corner_approx(j)
                    .distance_square(&check_distance_corner)
                    > check_dist_square
                {
                    break;
                }
                if !direction_changed {
                    corner_to_check = current_line.intersection_approx(&check_line);
                }
                let tmp_prev_dir = self.lines[j as usize].direction().get_vector();
                let tmp_curr_dir_from_tmp_prev_dir = tmp_curr_dir.side_of(&tmp_prev_dir);
                direction_changed = tmp_curr_dir_from_tmp_prev_dir != current_dir_from_prev_dir;
                if !direction_changed {
                    let prev_border_line = if tmp_curr_dir.side_of(&tmp_prev_dir) == Side::Positive
                    {
                        self.lines[j as usize].translate(-(half_width as f64))
                    } else {
                        self.lines[j as usize]
                            .opposite()
                            .translate(-(half_width as f64))
                    };
                    if prev_border_line.side_of_float_zero(&corner_to_check) == Side::Positive
                        && prev_border_line.side_of(&self.corner(i).expect("corner in range"))
                            == Side::Negative
                        && prev_border_line.side_of(&self.corner(i - 1).expect("corner in range"))
                            == Side::Negative
                    {
                        // an outstanding corner
                        cut_dog_ear_lines.push(prev_border_line.clone());
                    }
                    tmp_curr_dir = tmp_prev_dir;
                    current_line = prev_border_line;
                }
                j -= 1;
            }
            let mut s1 = TileShape::get_instance(&offset_lines);
            let cut_line_count = cut_dog_ear_lines.len();
            if cut_line_count > 0 {
                s1 = s1.intersection(&TileShape::get_instance(&cut_dog_ear_lines));
            }
            // Java computes current_shape_no here only for a debug
            // FRLogger call; the value is unused in the observable result.
            let _current_shape_no = i - from_no - 1;
            let bounding_shape: TileShape = if USE_BOUNDING_OCTAGON_FOR_OFFSET_SHAPES {
                // intersect with the bounding octagon
                let surr_oct = self.bounding_octagon(i - 1, i);
                TileShape::RegularTileShape(
                    crate::regular_tile_shape::RegularTileShape::IntOctagon(
                        surr_oct.offset(half_width as f64),
                    ),
                )
            } else {
                // Dead branch in Java (`USE_BOUNDING_OCTAGON_...` is a
                // constant true): Java computes
                // `offsetBox.toSimplex()` here. The port keeps the live
                // octagon branch only.
                let surr_box = self.bounding_box(i - 1, i);
                let offset_box = surr_box.offset(half_width as f64);
                TileShape::RegularTileShape(crate::regular_tile_shape::RegularTileShape::IntBox(
                    offset_box,
                ))
            };
            shapes.push(bounding_shape.intersection_with_simplify(&s1));
            // Java: FRLogger.warn("offset_shapes: shape is empty") when the
            // pushed shape is empty; the value is still pushed.

            prev_dir = current_direction;
            current_direction = next_dir;
        }
        shapes
    }

    /// Calculates for the `no`-th line segment a shape around this line
    /// where the right and left edge lines have the distance `half_width`
    /// from the center line (Java `offsetShape(int, int)`).
    /// `None` where Java returns null (`no` out of range).
    pub fn offset_shape(&self, half_width: i32, no: i32) -> Option<TileShape> {
        if no < 0 || no > self.lines.len() as i32 - 3 {
            // Java: FRLogger.warn("Polyline.offsetShape: no out of range")
            return None;
        }
        self.offset_shapes(half_width, no, no + 2)
            .into_iter()
            .next()
    }

    /// Returns the polyline translated by vector (Java `translateBy`).
    pub fn translate_by(&self, vector: &Vector) -> Polyline {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        let new_array: Vec<Line> = self.lines.iter().map(|l| l.translate_by(vector)).collect();
        Polyline::new(new_array)
    }

    /// Returns the polyline turned by factor times 90 degrees around pole
    /// (Java `turn90Degree`).
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Polyline {
        let new_array: Vec<Line> = self
            .lines
            .iter()
            .map(|l| l.turn_90_degree(factor, pole))
            .collect();
        Polyline::new(new_array)
    }

    /// Returns an approximation of this polyline rotated around pole (Java
    /// `rotateApprox`).
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Polyline {
        if angle == 0.0 {
            return self.clone();
        }
        let new_corners: Vec<Point> = (0..self.corner_count())
            .map(|i| Point::Int(self.corner_approx(i as i32).rotate(angle, pole).round()))
            .collect();
        Polyline::from_points(&new_corners)
    }

    /// Mirrors this polyline at the vertical line through pole.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> Polyline {
        let new_array: Vec<Line> = self.lines.iter().map(|l| l.mirror_vertical(pole)).collect();
        Polyline::new(new_array)
    }

    /// Mirrors this polyline at the horizontal line through pole.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Polyline {
        let new_array: Vec<Line> = self
            .lines
            .iter()
            .map(|l| l.mirror_horizontal(pole))
            .collect();
        Polyline::new(new_array)
    }

    /// Returns the smallest box containing the intersection points from
    /// index `requested_from_corner_no` to index `requested_to_corner_no`
    /// of the lines of this polyline (Java `boundingBox(int, int)`).
    pub fn bounding_box(
        &self,
        requested_from_corner_no: i32,
        requested_to_corner_no: i32,
    ) -> IntBox {
        let from_corner_no = requested_from_corner_no.max(0);
        let to_corner_no = requested_to_corner_no.min(self.lines.len() as i32 - 2);
        let mut llx = 2147483647.0f64;
        let mut lly = llx;
        let mut urx = -2147483648.0f64;
        let mut ury = urx;
        for i in from_corner_no..=to_corner_no {
            let current_corner = self.corner_approx(i);
            llx = llx.min(current_corner.x);
            lly = lly.min(current_corner.y);
            urx = urx.max(current_corner.x);
            ury = ury.max(current_corner.y);
        }
        let lower_left = IntPoint::new(llx.floor() as i32, lly.floor() as i32);
        let upper_right = IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
        IntBox::new(lower_left, upper_right)
    }

    /// Returns the smallest box containing the intersection points of the
    /// lines of this polyline (Java `boundingBox()`; uncached, T14).
    pub fn bounding_box_total(&self) -> IntBox {
        self.bounding_box(0, self.corner_count() as i32 - 1)
    }

    /// Returns the smallest octagon containing the intersection points from
    /// index `requested_from_corner_no` to index `requested_to_corner_no`
    /// (Java `boundingOctagon(int, int)`).
    pub fn bounding_octagon(
        &self,
        requested_from_corner_no: i32,
        requested_to_corner_no: i32,
    ) -> IntOctagon {
        let from_corner_no = requested_from_corner_no.max(0);
        let to_corner_no = requested_to_corner_no.min(self.lines.len() as i32 - 2);
        let mut lx = 2147483647.0f64;
        let mut ly = 2147483647.0f64;
        let mut rx = -2147483648.0f64;
        let mut uy = -2147483648.0f64;
        let mut ulx = 2147483647.0f64;
        let mut lrx = -2147483648.0f64;
        let mut llx = 2147483647.0f64;
        let mut urx = -2147483648.0f64;
        for i in from_corner_no..=to_corner_no {
            let current = self.corner_approx(i);
            lx = lx.min(current.x);
            ly = ly.min(current.y);
            rx = rx.max(current.x);
            uy = uy.max(current.y);
            let mut tmp = current.x - current.y;
            ulx = ulx.min(tmp);
            lrx = lrx.max(tmp);
            tmp = current.x + current.y;
            llx = llx.min(tmp);
            urx = urx.max(tmp);
        }
        IntOctagon::new(
            lx.floor() as i32,
            ly.floor() as i32,
            rx.ceil() as i32,
            uy.ceil() as i32,
            ulx.floor() as i32,
            lrx.ceil() as i32,
            llx.floor() as i32,
            urx.ceil() as i32,
        )
    }

    /// Returns whether this polyline contains the given point (Java
    /// `contains(Point)`).
    pub fn contains(&self, point: &Point) -> bool {
        for i in 1..self.lines.len() as i32 - 1 {
            let current_segment = LineSegment::from_polyline(self, i);
            if current_segment.contains(point) {
                return true;
            }
        }
        false
    }

    /// Element-wise [`Line::fast_equals`] comparison (additive helper; see
    /// the module docs — Java `Polyline` has no `fastEquals`).
    pub fn fast_equals(&self, other: &Polyline) -> bool {
        self.lines.len() == other.lines.len()
            && self
                .lines
                .iter()
                .zip(other.lines.iter())
                .all(|(a, b)| a.fast_equals(b))
    }

    /// Element-wise line equality (additive helper; see the module docs —
    /// Java `Polyline` inherits identity `equals`).
    pub fn equals(&self, other: &Polyline) -> bool {
        self.lines == other.lines
    }
}

/// Java `removeConsecutiveParallelLines`. With fewer than 3 lines the input
/// is returned unchanged (a polyline must have at least 3 lines).
fn remove_consecutive_parallel_lines(lines: Vec<Line>) -> Vec<Line> {
    if lines.len() < 3 {
        return lines;
    }
    let mut tmp_arr: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        match tmp_arr.last() {
            Some(prev) if prev.is_parallel(&line) => {
                // skip multiple lines
            }
            _ => tmp_arr.push(line),
        }
    }
    if tmp_arr.len() < 3 {
        return Vec::new();
    }
    tmp_arr
}

/// Java `removeOverlaps`: checks if previous and next lines are equal or
/// opposite and removes the resulting overlap. The index gymnastics is
/// transliterated 1:1 — note that the Java `else` branch OVERWRITES
/// `tmpArr[0]` with `lines[1]` instead of appending, and that slots of
/// skipped lines keep stale content that later decrements can read again.
fn remove_overlaps(lines: Vec<Line>) -> Vec<Line> {
    if lines.len() < 4 {
        return lines;
    }
    let mut tmp_arr: Vec<Line> = Vec::with_capacity(lines.len());
    tmp_arr.resize(lines.len(), lines[0].clone());
    let mut new_length: usize = 0;
    tmp_arr[0] = lines[0].clone();
    if !lines[0].is_equal_or_opposite(&lines[2]) {
        new_length += 1;
    }
    // else skip the first line
    tmp_arr[new_length] = lines[1].clone();
    new_length += 1;
    for i in 2..lines.len() - 2 {
        if tmp_arr[new_length - 1].is_equal_or_opposite(&lines[i + 1]) {
            // skip 2 lines
            new_length -= 1;
        } else {
            tmp_arr[new_length] = lines[i].clone();
            new_length += 1;
        }
    }
    tmp_arr[new_length] = lines[lines.len() - 2].clone();
    new_length += 1;
    // Guard: newLength must be >= 2 before accessing tmpArr[newLength - 2].
    // If the loop decremented newLength all the way to 0 the index is -1.
    if new_length >= 2 && !lines[lines.len() - 1].is_equal_or_opposite(&tmp_arr[new_length - 2]) {
        tmp_arr[new_length] = lines[lines.len() - 1].clone();
        new_length += 1;
    }
    // else skip the last line
    if new_length == lines.len() {
        // nothing skipped
        return lines;
    }
    // at least 1 line is skipped, adjust the array
    if new_length < 3 {
        return Vec::new();
    }
    tmp_arr.truncate(new_length);
    tmp_arr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_vector::IntVector;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// Java `new Polyline(new Point[]{...})` helper.
    fn from_int_corners(coords: &[(i32, i32)]) -> Polyline {
        let points: Vec<Point> = coords.iter().map(|(x, y)| p(*x, *y)).collect();
        Polyline::from_points(&points)
    }

    /// An L-shaped polyline of 3 corners has 4 lines.
    #[test]
    fn l_shape_structure() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        assert_eq!(pl.lines.len(), 4);
        assert_eq!(pl.corner_count(), 3);
        assert!(!pl.is_empty());
        assert!(!pl.is_point());
        assert!(pl.is_orthogonal());
        assert!(pl.is_multiple_of_45_degree());
    }

    /// Equal endpoints collapse to the empty polyline (Java ctor: at least
    /// 2 DIFFERENT points; duplicates are removed by `Polygon` first).
    #[test]
    fn degenerate_inputs() {
        let pl = Polyline::from_points(&[p(3, 7), p(3, 7)]);
        assert!(pl.is_empty());
        assert_eq!(pl.lines.len(), 0);
        let two = Polyline::from_two_corners(&p(1, 2), &p(1, 2));
        assert!(two.is_empty());
    }

    /// Corner access: corner i lies between line i and line i+1; clamping
    /// mirrors Java (index -1 acts as 0, index >= cornerCount acts as the
    /// last corner).
    #[test]
    fn corner_access_and_clamping() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        assert_eq!(pl.corner(0).expect("exists"), p(0, 0));
        assert_eq!(pl.corner(1).expect("exists"), p(10, 0));
        assert_eq!(pl.corner(2).expect("exists"), p(10, 10));
        assert_eq!(pl.corner(-5).expect("exists"), p(0, 0));
        assert_eq!(pl.corner(99).expect("exists"), p(10, 10));
        assert_eq!(pl.first_corner(), pl.corner(0));
        assert_eq!(pl.last_corner(), pl.corner(2));
        let approx = pl.corner_approx(1);
        assert_eq!((approx.x, approx.y), (10.0, 0.0));
    }

    /// Length of the L-shape (0,0)-(10,0)-(10,10) is 20.
    #[test]
    fn length_approx_total() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        assert_eq!(pl.length_approx_total(), 20.0);
    }

    /// Bounding box / octagon of the L-shape.
    #[test]
    fn bounding_shapes() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        assert_eq!(
            pl.bounding_box_total(),
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 10))
        );
        let oct = pl.bounding_octagon(0, 2);
        assert_eq!(oct, IntOctagon::new(0, 0, 10, 10, 0, 10, 0, 20));
    }

    /// jshell pin `PFL1`: the ctor direction flip turns the middle line of
    /// the small triangle so it points from the previous corner to the
    /// next (the float side-of agrees with the exact side here).
    #[test]
    fn pfl1_ctor_corner_flip_small() {
        let pl = Polyline::new(vec![
            Line::new(p(0, 0), p(4, 3)),
            Line::new(p(5, 4), p(5, 3)),
            Line::new(p(6, 4), p(7, 4)),
        ]);
        let end_points: Vec<(Point, Point)> = pl
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            end_points,
            vec![
                (p(0, 0), p(4, 3)),
                // flipped from the input (5,4)->(5,3)
                (p(5, 3), p(5, 4)),
                (p(6, 4), p(7, 4)),
            ]
        );
    }

    /// jshell pin `PFL2` (trap T10): deliberately constructed input where
    /// the FLOAT side-of of the corner approximation disagrees with the
    /// exact one. Lines: L0 through the origin with direction
    /// (2^27, 2^27-1); the corner approximation of L1 and L2 is
    /// (2^27+1, 2^27) (Java intersectionApprox), where the float cross
    /// dx*(py-ay) - dy*(px-ax) rounds to 0. The oracle output flips the
    /// middle line — the pin freezes the observed ctor result.
    #[test]
    fn pfl2_ctor_corner_flip_float_vs_exact() {
        let big = 134217728i32; // 2^27
        let pl = Polyline::new(vec![
            Line::new(p(0, 0), p(big, big - 1)),
            Line::new(p(big + 1, big + 1), p(big + 1, big)),
            Line::new(p(big - 1, big), p(big - 2, big)),
        ]);
        let end_points: Vec<(Point, Point)> = pl
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            end_points,
            vec![
                (p(0, 0), p(big, big - 1)),
                // oracle flips this line
                (p(big + 1, big), p(big + 1, big + 1)),
                (p(big - 1, big), p(big - 2, big)),
            ]
        );
    }

    /// jshell pin `POVL1` (trap T10 order): the 5-line input survives the
    /// parallel filter untouched and then removeOverlaps skips the first
    /// line (lines[0] is equal-or-opposite to lines[2]) AND the last line
    /// (equal-or-opposite to the kept lines[2]), truncating to
    /// [lines[1], lines[2], lines[3]].
    #[test]
    fn povl1_overlap_removal_order_pin() {
        let pl = Polyline::new(vec![
            Line::new(p(0, 0), p(10, 0)),
            Line::new(p(0, 0), p(0, 10)),
            Line::new(p(5, 0), p(15, 0)),
            Line::new(p(0, 0), p(7, 7)),
            Line::new(p(6, 0), p(20, 0)),
        ]);
        let end_points: Vec<(Point, Point)> = pl
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            end_points,
            vec![(p(0, 0), p(0, 10)), (p(5, 0), p(15, 0)), (p(0, 0), p(7, 7)),]
        );
    }

    /// Reversing the L-shape keeps the same exact corners in reverse order.
    #[test]
    fn reverse_round_trip() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        let rev = pl.reverse();
        assert_eq!(rev.corner(0).expect("exists"), p(10, 10));
        assert_eq!(rev.corner(2).expect("exists"), p(0, 0));
        // reverse is an involution up to the normalization
        assert!(pl.fast_equals(&rev.reverse()));
    }

    /// Translations and 90-degree turns preserve length.
    #[test]
    fn transforms_preserve_length() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        let moved = pl.translate_by(&Vector::Int(IntVector::new(5, -3)));
        assert_eq!(moved.length_approx_total(), 20.0);
        assert_eq!(moved.corner(0).expect("exists"), p(5, -3));
        let turned = pl.turn_90_degree(1, &IntPoint::new(0, 0));
        assert_eq!(turned.length_approx_total(), 20.0);
    }
}
