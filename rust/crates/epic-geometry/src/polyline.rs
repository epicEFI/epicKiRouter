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
//! Deferral ledger (Task 8): `split` and `combine` landed with the M2
//! trace-surgery tasks and the M3 shove substrate respectively;
//! `skipLines`, `projectionLine` and `shorten` landed with the M3-T10c
//! forced-insertion surface (the sampling retry and the connectToTrace
//! ladder). Still not ported: `nearestPointApprox` and `distance`
//! (M4 pull-tight surface). `offsetShapes` /
//! `offsetShape(int, int)` are ported (Task 8 scope), and `offsetBox`
//! lands with the M2 Task 7 search-tree shapes (the 90-degree tree's
//! trace path, `ShapeSearchTree90Degree.offsetShape`).

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

    /// Returns the number of lines minus 1 (Java `cornerCount()`); the
    /// empty polyline wraps to `usize::MAX` via `wrapping_sub`, mirroring
    /// Java's int result of -1.
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

    /// Combines this polyline with `other` at a common end corner, if
    /// possible, so that the shape of the combined polyline is the
    /// union of both (Java `Polyline.combine(Polyline)`,
    /// `Polyline.java:700-756`). If there is something to combine at
    /// the START of this polyline, `other` is inserted in front of it;
    /// at the END, this polyline is inserted in front of `other`.
    /// `None` for `other` (Java null), a sub-3-line polyline on either
    /// side, or no common end corner yields `self` unchanged (Java
    /// returns `this`). The result re-normalizes through the
    /// canonicalizing constructor exactly like every Java `Polyline`
    /// construction.
    #[must_use]
    pub fn combine(&self, other: Option<&Polyline>) -> Polyline {
        let Some(other) = other else {
            return self.clone();
        };
        if self.lines.len() < 3 || other.lines.len() < 3 {
            return self.clone();
        }
        let (combine_at_start, combine_other_at_start) =
            if self.first_corner() == other.first_corner() {
                (true, true)
            } else if self.first_corner() == other.last_corner() {
                (true, false)
            } else if self.last_corner() == other.first_corner() {
                (false, true)
            } else if self.last_corner() == other.last_corner() {
                (false, false)
            } else {
                // no common endpoint
                return self.clone();
            };
        let mut new_lines: Vec<Line> = Vec::with_capacity(self.lines.len() + other.lines.len() - 2);
        if combine_at_start {
            // insert the lines of other in front
            if combine_other_at_start {
                // insert in reverse order, skip the first line of other
                for i in 0..other.lines.len() - 1 {
                    new_lines.push(other.lines[other.lines.len() - i - 1].opposite());
                }
            } else {
                // skip the last line of other
                new_lines.extend_from_slice(&other.lines[..other.lines.len() - 1]);
            }
            // append the lines of this polyline, skip the first line
            new_lines.extend_from_slice(&self.lines[1..]);
        } else {
            // insert the lines of this polyline in front, skip the last line
            new_lines.extend_from_slice(&self.lines[..self.lines.len() - 1]);
            if combine_other_at_start {
                // skip the first line of other
                new_lines.extend_from_slice(&other.lines[1..]);
            } else {
                // insert in reverse order, skip the last line of other
                for i in 1..other.lines.len() {
                    new_lines.push(other.lines[other.lines.len() - i - 1].opposite());
                }
            }
        }
        Polyline::new(new_lines)
    }

    /// Splits this polyline at the line with index `line_index` into
    /// two by inserting `end_line` as the concluding line of the first
    /// split piece and as the start line of the second split piece
    /// (Java `Polyline.split(int, Line)`, `Polyline.java:758-835`).
    /// `end_line` and the line with index `line_index` must not be
    /// parallel. The order of the lines in the two result pieces is
    /// preserved. `1 <= line_index <= lines.len() - 2`. `None` where
    /// Java returns null: out-of-range index, parallel end line, an
    /// end line that only TOUCHES a polyline end point, or a piece
    /// that collapses in the canonicalizing constructor (`isPoint`).
    ///
    /// The zero-length skips: when the new end corner equals
    /// `corner(line_index - 1)` the first piece omits `end_line` (its
    /// last segment would have length 0), and when it equals
    /// `corner(line_index)` the second piece omits it. The touch-only
    /// guard uses the JAVA operator precedence verbatim —
    /// `(lineIndex == 1 && first) || (lineIndex >= length - 2 && last)`
    /// — the two conjuncts bind tighter than the disjunction.
    pub fn split(&self, line_index: i32, end_line: &Line) -> Option<[Polyline; 2]> {
        let length = self.lines.len() as i32;
        if line_index < 1 || line_index > length - 2 {
            // Java: FRLogger.warn("Polyline.split: lineIndex out of range")
            return None;
        }
        let index = line_index as usize;
        if self.lines[index].is_parallel(end_line) {
            return None;
        }
        // Java's `Line.intersection` never returns null (parallel pairs
        // yield an infinite RationalPoint, excluded by the guard above);
        // the Rust finite-point form maps the unreachable parallel case
        // to `None`.
        let new_end_corner = self.lines[index].intersection(end_line)?;
        // No split, if endLine does not intersect, but touches
        // only this Polyline at an end point.
        if (line_index == 1 && self.first_corner().as_ref() == Some(&new_end_corner))
            || (line_index >= length - 2 && self.last_corner().as_ref() == Some(&new_end_corner))
        {
            return None;
        }
        let mut first_piece: Vec<Line> = self.lines[..index + 1].to_vec();
        if self.corner(line_index - 1).as_ref() != Some(&new_end_corner) {
            // skip line segment of length 0 at the end of the first piece
            first_piece.push(end_line.clone());
        }
        let mut second_piece: Vec<Line> = Vec::with_capacity(self.lines.len() - index + 1);
        if self.corner(line_index).as_ref() == Some(&new_end_corner) {
            // skip line segment of length 0 at the beginning of the
            // second piece
        } else {
            second_piece.push(end_line.clone());
        }
        second_piece.extend_from_slice(&self.lines[index..]);
        let result0 = Polyline::new(first_piece);
        let result1 = Polyline::new(second_piece);
        if result0.is_point() || result1.is_point() {
            return None;
        }
        Some([result0, result1])
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

    /// Calculates for the `no`-th line segment a BOX shape around
    /// this line where the border lines have the distance
    /// `half_width` from the center line (Java `offsetBox(int, int)`
    /// `Polyline.java:531-534`):
    /// `new LineSegment(this, no + 1).boundingBox().offset(halfWidth)`
    /// — the 3-line window `(lines[no], lines[no+1], lines[no+2])`,
    /// its float-intersection floor/ceil bounding box, inflated by
    /// `half_width` on all four sides. `None` where Java's
    /// from-polyline `LineSegment` construction warns (and would
    /// NPE): `no` out of the `0..=lines.len() - 3` range.
    ///
    /// Unlike [`Polyline::offset_shape`] the box is NOT cut at the
    /// diagonal end lines — a 45-degree trace segment gets the
    /// FULL box under the 90-degree tree (jar-pinned in the M2 Task 7
    /// tree-shape captures).
    pub fn offset_box(&self, half_width: i32, no: i32) -> Option<IntBox> {
        if no < 0 || no > self.lines.len() as i32 - 3 {
            // Java: FRLogger.warn("LineSegment from Polyline: no out of
            // range") — then boundingBox() of the null-line segment NPEs;
            // the guard here is the caller-facing null.
            return None;
        }
        let segment = LineSegment::new(
            self.lines[no as usize].clone(),
            self.lines[no as usize + 1].clone(),
            self.lines[no as usize + 2].clone(),
        );
        Some(segment.bounding_box().offset(f64::from(half_width)))
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

    /// Returns this polyline without the lines from `from_no` to `to_no`
    /// (inclusive). The input polyline is returned unchanged when the
    /// range is empty or out of bounds (Java `skipLines`,
    /// `Polyline.java:838-846`). The remaining lines go through the
    /// normalizing constructor — skipping a line can make previously
    /// non-adjacent lines consecutive parallels.
    pub fn skip_lines(&self, from_no: i32, to_no: i32) -> Polyline {
        if from_no < 0 || to_no > self.lines.len() as i32 - 1 || from_no > to_no {
            return self.clone();
        }
        let mut new_lines: Vec<Line> = self.lines[..from_no as usize].to_vec();
        new_lines.extend_from_slice(&self.lines[to_no as usize + 1..]);
        Polyline::new(new_lines)
    }

    /// Creates a perpendicular line segment from `point` onto the nearest
    /// line segment of this polyline to `point`. Returns `None` if the
    /// perpendicular line does not intersect the nearest line segment
    /// inside its segment bounds or if `point` is contained in this
    /// polyline (Java `projectionLine`, `Polyline.java:871-910`; the
    /// dispatch named it `LineSegment::projection_line`, but Java puts it
    /// on `Polyline` — the Java placement wins).
    ///
    /// The candidate walk is STRICTLY closer (`<`): equal distances never
    /// replace the incumbent, so the FIRST minimal candidate wins ties.
    /// The containment filter is the exact `sideOf` pair — both corners of
    /// segment `i` on the same (non-collinear) side of the projection
    /// line disqualify the candidate.
    pub fn projection_line(&self, point: &Point) -> Option<LineSegment> {
        let from_point = point.to_float();
        let mut min_distance = f64::MAX;
        let mut result_line: Option<Line> = None;
        let mut nearest_line: Option<&Line> = None;
        for i in 1..self.lines.len() as i32 - 1 {
            let projection = from_point.projection_approx(&self.lines[i as usize]);
            let current_distance = projection.distance(&from_point);
            if current_distance < min_distance {
                let Some(direction_towards_line) =
                    self.lines[i as usize].perpendicular_direction(point)
                else {
                    continue;
                };
                let current_result_line =
                    Line::new_with_direction(point.clone(), direction_towards_line);
                let Some(prev_corner) = self.corner(i - 1) else {
                    continue;
                };
                let Some(next_corner) = self.corner(i) else {
                    continue;
                };
                let prev_corner_side = current_result_line.side_of(&prev_corner);
                let next_corner_side = current_result_line.side_of(&next_corner);
                if prev_corner_side == next_corner_side && prev_corner_side != Side::Collinear {
                    // the projection point is outside the line segment
                    continue;
                }
                nearest_line = Some(&self.lines[i as usize]);
                min_distance = current_distance;
                result_line = Some(current_result_line);
            }
        }
        let nearest_line = nearest_line?;
        let result_line = result_line?;
        let start_line = Line::new_with_direction(point.clone(), nearest_line.direction().clone());
        Some(LineSegment::new(
            start_line,
            result_line,
            nearest_line.clone(),
        ))
    }

    /// Shortens this polyline to `new_line_count` lines. Additionally, the
    /// last line segment will be approximately shortened to
    /// `last_segment_length`. The last corner of the new polyline will be
    /// an `IntPoint` (Java `shorten`, `Polyline.java:916-936`).
    ///
    /// When the rounded new last corner coincides with the ORIGINAL
    /// second-to-last corner, the last line is skipped instead (the
    /// `skipLines(newLineCount - 1, newLineCount - 1)` arm) — the result
    /// then goes through the normalizing constructor and can degenerate.
    pub fn shorten(&self, new_line_count: i32, last_segment_length: f64) -> Polyline {
        let last_corner = self.corner_approx(new_line_count - 2);
        let prev_last_corner = self.corner_approx(new_line_count - 3);
        let new_last_corner = prev_last_corner.change_length(&last_corner, last_segment_length);
        let new_last_corner = new_last_corner.round();
        let original_second_to_last = self.corner(self.corner_count() as i32 - 2);
        if Some(Point::Int(new_last_corner)) == original_second_to_last {
            // skip the last line
            return self.skip_lines(new_line_count - 1, new_line_count - 1);
        }
        let mut new_lines: Vec<Line> = self.lines[..(new_line_count - 2) as usize].to_vec();
        // create the last 2 lines of the new polyline
        let mut first_line_point = self.lines[(new_line_count - 2) as usize].a.clone();
        if first_line_point == Point::Int(new_last_corner) {
            first_line_point = self.lines[(new_line_count - 2) as usize].b.clone();
        }
        let new_prev_last_line = Line::new(first_line_point, Point::Int(new_last_corner));
        new_lines.push(new_prev_last_line.clone());
        new_lines.push(Line::get_instance(
            Point::Int(new_last_corner),
            new_prev_last_line.direction().turn_45_degree(6),
        ));
        Polyline::new(new_lines)
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
    // Java dereferences tmpArr[newLength - 2] unconditionally here; when
    // the loop above decremented newLength below 2 the index is negative
    // and Java crashes with ArrayIndexOutOfBoundsException (POVL3).
    if new_length < 2 {
        panic!(
            "Polyline.removeOverlaps: tmpArr[newLength - 2] with newLength = {new_length} (Java ArrayIndexOutOfBoundsException)"
        );
    }
    if !lines[lines.len() - 1].is_equal_or_opposite(&tmp_arr[new_length - 2]) {
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

    /// `offsetBox` (`Polyline.java:531-534`): the 3-line window's
    /// floor/ceil bounding box inflated by `half_width` on all four
    /// sides — the 90-degree tree's trace shape. For the FLAT
    /// segments of the L-shape the box is the exact segment box
    /// inflated; the range guard mirrors `offsetShape`'s
    /// (`0..=lines.len() - 3`, the Java warn-and-NPE window).
    #[test]
    fn offset_box_is_the_window_box_inflated() {
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        // Window 0: segment (0,0)-(10,0), offset 2 on every side.
        assert_eq!(
            pl.offset_box(2, 0),
            Some(IntBox::new(IntPoint::new(-2, -2), IntPoint::new(12, 2)))
        );
        // Window 1: segment (10,0)-(10,10).
        assert_eq!(
            pl.offset_box(2, 1),
            Some(IntBox::new(IntPoint::new(8, -2), IntPoint::new(12, 12)))
        );
        // Zero width: the raw window box; out of range: None.
        assert_eq!(
            pl.offset_box(0, 0),
            Some(IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 0)))
        );
        assert_eq!(pl.offset_box(1, 2), None);
        assert_eq!(pl.offset_box(1, -1), None);
    }

    /// `offsetBox` vs `offsetShape` on a 45-degree segment — the
    /// discriminating form: the BOX keeps the full corner squares the
    /// octagon-cut `offsetShape` shears off, so the two must DIFFER
    /// (a port that routes the 90-degree tree through `offsetShape`
    /// fails here even though both share the same bounding box).
    #[test]
    fn offset_box_differs_from_offset_shape_on_diagonals() {
        let pl = from_int_corners(&[(0, 0), (7, 7)]);
        let box_shape = pl.offset_box(1, 0).expect("the single window");
        assert_eq!(
            box_shape,
            IntBox::new(IntPoint::new(-1, -1), IntPoint::new(8, 8)),
            "the exact diagonal segment box inflated by 1"
        );
        let oct_shape = pl
            .offset_shape(1, 0)
            .expect("offsetShape keeps the same window");
        // Same bounding box, DIFFERENT shape: the octagon shears the
        // corner triangles the box keeps.
        assert_eq!(
            oct_shape.bounding_box(),
            TileShape::RegularTileShape(crate::regular_tile_shape::RegularTileShape::IntBox(
                box_shape
            ))
            .bounding_box(),
            "control: both share the bounding box"
        );
        assert!(
            !oct_shape.contains_point(&p(7, -1)) || !oct_shape.contains_point(&p(-1, 7)),
            "the octagon must shear at least one corner the box keeps — \\
            a shape-equal port fails this anchor-blind form"
        );
    }

    /// Invariant (Task 10): the normalizing Polyline constructor is
    /// IDEMPOTENT — re-wrapping a constructed polyline's lines reproduces
    /// the same polyline (the ctor output is a fixed point of the
    /// dog-ear/flip normalization). Fans mirror the corpus generator:
    /// 2-4 lines mixing 45-degree continuations and free general-slope
    /// lines at full-range magnitudes (wrapping bands included).
    ///
    /// Mutation-proven (Task 10 review F1): re-anchoring one rewrapped
    /// line one direction-step along its own supporting line (anchor-blind
    /// equal under `Line`'s PartialEq) passes the naive
    /// `assert_eq!(lines, lines)` and FAILS the endpoint-exact comparison
    /// below.
    #[test]
    fn ctor_is_idempotent_on_random_fans() {
        use crate::java_random::JavaRandom;
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_567C);
        let line_of = |ax: i32, ay: i32, bx: i32, by: i32| {
            Line::new(
                Point::int(IntPoint::new(ax, ay)),
                Point::int(IntPoint::new(bx, by)),
            )
        };
        for _ in 0..10_000 {
            let n = 2 + rng.next_int_bound(3) as usize;
            let mut lines = Vec::with_capacity(n);
            let mut ax = rng.next_int();
            let mut ay = rng.next_int();
            for _ in 0..n {
                let (bx, by) = if rng.next_int_bound(2) == 0 {
                    // 45-degree continuation from the previous anchor
                    let k = rng.next_int_bound(4000) + 1;
                    match rng.next_int_bound(4) {
                        0 => (ax.wrapping_add(k), ay),
                        1 => (ax, ay.wrapping_add(k)),
                        2 => (ax.wrapping_add(k), ay.wrapping_add(k)),
                        _ => (ax.wrapping_sub(k), ay.wrapping_add(k)),
                    }
                } else {
                    (rng.next_int(), rng.next_int())
                };
                lines.push(line_of(ax, ay, bx, by));
                ax = bx;
                ay = by;
            }
            let pl = Polyline::new(lines);
            let rewrapped = Polyline::new(pl.lines.clone());
            // Endpoint-exact element-wise comparison (trap T20): `Line`'s
            // PartialEq is ANCHOR-BLIND (collinearity + positive direction
            // projection), so a plain `rewrapped.lines == pl.lines` would
            // let a ctor mutation that re-anchors lines along their own
            // direction pass silently.
            assert!(
                rewrapped.lines.len() == pl.lines.len()
                    && rewrapped
                        .lines
                        .iter()
                        .zip(&pl.lines)
                        .all(|(r, pv)| r.a == pv.a && r.b == pv.b),
                "ctor not idempotent for {pl:?}"
            );
        }
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
    /// the corner APPROXIMATION changes the ctor outcome. Lines: L0
    /// through the origin with direction (2^27, 2^27-1). The exact corner
    /// of L1 and L2 is (2^27+1, 2^27); geometrically it lies a hair's
    /// breadth off L0 (det = +-1), but every Java side predicate for int
    /// points multiplies in f64 (Line.sideOf -> IntVector.sideOf computes
    /// `(double) other.x * y - (double) other.y * x`, IntVector.java:136),
    /// where (2^27-1)(2^27+1) = 2^54-1 tie-rounds to 2^54 and cancels —
    /// jar capture: `L0.sideOf(new IntPoint(134217729, 134217728))`
    /// prints collinear. So at the EXACT corner no flip decision would be
    /// taken. Java's `intersectionApprox` instead returns (2^27, 2^27) at
    /// this magnitude — a DIFFERENT point whose float side on L0 is
    /// onTheRight — and that manufactured side flips the middle line. The
    /// pin freezes the observed ctor result.
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

    /// jshell pin `POVL2` (trap T10 ORDER — the filter/overlap swap
    /// mutant): the input contains a consecutive-parallel pair (A, P), so
    /// the parallel filter — which runs FIRST — drops both P and the
    /// repeated A' before removeOverlaps ever runs; the overlap scan then
    /// sees only [A, C, D] and is a no-op. Under the swapped order,
    /// removeOverlaps sees all 5 lines, anchor-blind Line equality
    /// matches A == A' and skips the first line, yielding [P, A', C, D];
    /// the filter then drops A' (parallel to P), leaving [P, C, D] — a
    /// DIFFERENT first line. Jar transcript:
    /// ```text
    /// jshell> new Polyline(new Line[]{A, P, A2, C, D}) with
    ///     A=(0,0)-(10,0), P=(2,3)-(12,3), A2=(0,0)-(10,0),
    ///     C=(0,0)-(7,7), D=(0,0)-(0,10)
    /// surviving lines: 3
    /// (0,0) -> (10,0)
    /// (0,0) -> (7,7)
    /// (0,0) -> (0,10)
    /// ```
    #[test]
    fn povl2_filter_before_overlap_order_pin() {
        let pl = Polyline::new(vec![
            Line::new(p(0, 0), p(10, 0)),
            Line::new(p(2, 3), p(12, 3)),
            Line::new(p(0, 0), p(10, 0)),
            Line::new(p(0, 0), p(7, 7)),
            Line::new(p(0, 0), p(0, 10)),
        ]);
        let end_points: Vec<(Point, Point)> = pl
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            end_points,
            vec![(p(0, 0), p(10, 0)), (p(0, 0), p(7, 7)), (p(0, 0), p(0, 10)),]
        );
    }

    /// Pairwise cancellation drives `newLength` down to 1 before the final
    /// `tmpArr[newLength - 2]` dereference: the index is negative and Java
    /// throws ArrayIndexOutOfBoundsException; the port panics with the
    /// same meaning (POVL3). Trace: the filter keeps all 5 lines (no
    /// consecutive parallels); removeOverlaps skips the first line
    /// (lines[0] and lines[2] share supporting line y = 0), tmp[0] =
    /// lines[1]; the loop then sees tmp[0] == lines[3] (same x = 0 line)
    /// and skips 2 (newLength = 0), then tmp[0] = lines[3] (newLength =
    /// 1) — the guard fires.
    #[test]
    #[should_panic(expected = "ArrayIndexOutOfBoundsException")]
    fn povl3_remove_overlaps_index_underflow_is_a_java_crash() {
        let _ = Polyline::new(vec![
            Line::new(p(0, 0), p(10, 0)),
            Line::new(p(0, 0), p(0, 10)),
            Line::new(p(5, 0), p(15, 0)),
            Line::new(p(0, 5), p(0, 20)),
            Line::new(p(1, 1), p(5, 5)),
        ]);
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

    // -----------------------------------------------------------------
    // Polyline.split(int, Line) — the Java trap surface
    // (Polyline.java:758-835). Jar-verified against SplitSpike's X1
    // rows: an interior split receives the OTHER segment's middle line
    // (receiver-asymmetric LineSegment.intersection), never a line
    // parallel to the perpendicular end phantoms.
    // -----------------------------------------------------------------

    /// The X1 geometry: A = (3,2)-(5,2), split by B's vertical line
    /// x=4 at line index 1. Both pieces keep their perpendicular
    /// phantoms (3 lines each) — capture rows
    /// `X1_RESULT size=2 [14:[30000,20000 40000,20000]
    /// 15:[40000,20000 50000,20000]]`, scaled by 10.
    #[test]
    fn interior_split_keeps_both_pieces() {
        let pl = from_int_corners(&[(30000, 20000), (50000, 20000)]);
        let end_line = Line::new(p(40000, 10000), p(40000, 30000));
        let [first, second] = pl.split(1, &end_line).expect("the X1 split succeeds");
        assert_eq!(first.corners(), vec![p(30000, 20000), p(40000, 20000)]);
        assert_eq!(second.corners(), vec![p(40000, 20000), p(50000, 20000)]);
        assert_eq!(first.lines.len(), 3);
        assert_eq!(second.lines.len(), 3);
    }

    /// The guard ladder: out-of-range indices (`:759-762`), a parallel
    /// end line (`:763-765`), and the touch-only rejects at both
    /// polyline end points (`:800-805`, precedence
    /// `(li == 1 && first) || (li >= len - 2 && last)`).
    #[test]
    fn split_guards_reject() {
        let pl = from_int_corners(&[(30000, 20000), (50000, 20000)]);
        let vert = Line::new(p(40000, 10000), p(40000, 30000));
        let horiz = Line::new(p(10000, 20000), p(60000, 20000));
        // Out of range: 0 and lines.length - 1.
        assert!(pl.split(0, &vert).is_none(), "lineIndex < 1");
        assert!(pl.split(2, &vert).is_none(), "lineIndex > length - 2");
        // Parallel end line.
        assert!(pl.split(1, &horiz).is_none());
        // Touch-only at the first corner: lineIndex == 1 and the
        // intersection equals corner 0.
        let at_first = Line::new(p(30000, 10000), p(30000, 30000));
        assert!(
            pl.split(1, &at_first).is_none(),
            "touch at the first corner"
        );
        // Touch-only at the last corner: lineIndex >= length - 2 and
        // the intersection equals the last corner.
        let at_last = Line::new(p(50000, 10000), p(50000, 30000));
        assert!(pl.split(1, &at_last).is_none(), "touch at the last corner");
    }

    /// The zero-length skips (`:806-827`): an end line through
    /// `corner(lineIndex - 1)` omits it from the first piece (which
    /// then collapses and rejects the split), an end line through
    /// `corner(lineIndex)` omits it from the second piece — the
    /// split-at-existing-corner case yields an UNCHANGED tail piece.
    #[test]
    fn zero_length_skips() {
        let pl = from_int_corners(&[(0, 0), (10000, 0), (10000, 5000), (20000, 5000)]);
        // End line through corner(0) = (0,0) at lineIndex 1: the first
        // piece skips the end line and collapses to 2 lines -> the
        // whole split is None (the isPoint guard).
        let through_first = Line::new(p(0, -1000), p(0, 5000));
        assert!(
            pl.split(1, &through_first).is_none(),
            "corner(lineIndex - 1) skip collapses the first piece"
        );
        // End line through corner(1) = (10000,0) at lineIndex 1: the
        // second piece is the unchanged tail lines[1..].
        let through_corner1 = Line::new(p(10000, -1000), p(10000, 1000));
        let [first, second] = pl
            .split(1, &through_corner1)
            .expect("corner split succeeds");
        assert_eq!(first.corners(), vec![p(0, 0), p(10000, 0)]);
        assert_eq!(
            second.corners(),
            vec![p(10000, 0), p(10000, 5000), p(20000, 5000)]
        );
    }

    /// An interior perpendicular split of the L's vertical leg (not at
    /// a corner): both pieces survive the canonicalizing constructor.
    #[test]
    fn interior_split_of_an_l_leg() {
        let pl = from_int_corners(&[(0, 0), (10000, 0), (10000, 10000)]);
        let mid = Line::new(p(0, 5000), p(20000, 5000));
        let [first, second] = pl.split(2, &mid).expect("the leg split succeeds");
        assert_eq!(first.corners(), vec![p(0, 0), p(10000, 0), p(10000, 5000)]);
        assert_eq!(second.corners(), vec![p(10000, 5000), p(10000, 10000)]);
    }

    /// `combine` arms, captured from the jar via jshell
    /// (`logs/M3-T10a/captures/shape_entry_side_caps.txt`): end-start
    /// append, start-start prepend-with-flip, no-common-corner
    /// unchanged, null other unchanged, and the end-end arm where the
    /// OTHER polyline's tail continues from this one's end.
    #[test]
    fn combine_arms_match_the_jar() {
        let pa = from_int_corners(&[(-500, 500), (500, 500), (500, 1500)]);
        let corners_of = |poly: &Polyline| -> Vec<(f64, f64)> {
            poly.corner_approx_arr()
                .iter()
                .map(|c| (c.x, c.y))
                .collect()
        };
        let assert_corners = |poly: &Polyline, want: &[(f64, f64)], label: &str| {
            let got = corners_of(poly);
            assert_eq!(got.len(), want.len(), "{label} corner count");
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9,
                    "{label} corner {g:?} vs {w:?}"
                );
            }
        };

        // end-start: other starts at pa's end (500,1500).
        let other = from_int_corners(&[(500, 1500), (1500, 1500), (1500, 2000)]);
        assert_corners(
            &pa.combine(Some(&other)),
            &[
                (-500.0, 500.0),
                (500.0, 500.0),
                (500.0, 1500.0),
                (1500.0, 1500.0),
                (1500.0, 2000.0),
            ],
            "end-start",
        );

        // start-start: other's END is pa's start (-500,500) — the
        // flipped prepend arm.
        let other2 = from_int_corners(&[(-500, 500), (-500, 1500), (-1000, 1500)]);
        assert_corners(
            &pa.combine(Some(&other2)),
            &[
                (-1000.0, 1500.0),
                (-500.0, 1500.0),
                (-500.0, 500.0),
                (500.0, 500.0),
                (500.0, 1500.0),
            ],
            "start-start",
        );

        // no common corner: unchanged.
        let other3 = from_int_corners(&[(5000, 5000), (6000, 5000), (6000, 6000)]);
        assert_corners(
            &pa.combine(Some(&other3)),
            &[(-500.0, 500.0), (500.0, 500.0), (500.0, 1500.0)],
            "no-common",
        );

        // null other: unchanged.
        assert_corners(
            &pa.combine(None),
            &[(-500.0, 500.0), (500.0, 500.0), (500.0, 1500.0)],
            "null",
        );

        // end-end: other's LAST corner equals pa's end — the combined
        // tail runs through other's interior corner (2000,500) BACK to
        // its end (500,1500).
        let other4 = from_int_corners(&[(1500, 500), (2000, 500), (500, 1500)]);
        assert_corners(
            &pa.combine(Some(&other4)),
            &[
                (-500.0, 500.0),
                (500.0, 500.0),
                (500.0, 1500.0),
                (2000.0, 500.0),
                (1500.0, 500.0),
            ],
            "end-end",
        );
    }

    /// An open L plus a 45-degree tail, built from EXPLICIT lines so the
    /// test controls every anchor (the polygon ctor would append a
    /// closing line). Corner approximations: (100,0), (100,100),
    /// (200,100).
    fn open_l() -> Polyline {
        Polyline::new(vec![
            Line::new(p(0, 0), p(100, 0)),
            Line::new(p(100, 0), p(100, 100)),
            Line::new(p(100, 100), p(200, 100)),
        ])
    }

    /// `skipLines` (`Polyline.java:838-846`): the inclusive range is
    /// removed and the remainder re-wrapped; the out-of-range and empty
    /// ranges return the input UNCHANGED (Java `return this`); a
    /// remainder shorter than 3 lines is EMPTIED by the normalizing
    /// ctor.
    #[test]
    fn skip_lines_range_and_guards() {
        // Connected 4-line input: (0,0),(100,0),(100,100),(200,100)
        // open, plus the (200,200) riser.
        let four = Polyline::new(vec![
            Line::new(p(0, 0), p(100, 0)),
            Line::new(p(100, 0), p(100, 100)),
            Line::new(p(100, 100), p(200, 100)),
            Line::new(p(200, 100), p(200, 200)),
        ]);
        // Guards: fromNo < 0, toNo > len-1, fromNo > toNo → clone.
        for (from, to) in [(-1, 0), (0, 4), (2, 1)] {
            let skipped = four.skip_lines(from, to);
            assert!(
                skipped.equals(&four),
                "guard ({from},{to}) must return the input"
            );
        }
        // Dropping the first line leaves 3 CONNECTED lines that the
        // ctor keeps verbatim (no parallels, flip pass finds no
        // dog-ear).
        let skipped = four.skip_lines(0, 0);
        let ends: Vec<(Point, Point)> = skipped
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            ends,
            vec![
                (p(100, 0), p(100, 100)),
                (p(100, 100), p(200, 100)),
                (p(200, 100), p(200, 200)),
            ],
            "lines 0..=0 inclusive are gone"
        );
        // The T10c sampling-retry form skipLines(newLineCount-1,
        // newLineCount-1) drops exactly the last line; on open_l the
        // two survivors are only 2 lines — the ctor EMPTIES those.
        let pl = open_l();
        assert!(pl.skip_lines(2, 2).is_empty(), "ctor: < 3 lines → empty");
    }

    /// `projectionLine` (`Polyline.java:871-910`), happy face: the
    /// perpendicular from (50,50) onto the interior vertical x=100.
    /// The result line is the horizontal through the point; start/end
    /// are its intersections with the point-parallel start line and the
    /// nearest polyline line.
    #[test]
    fn projection_line_interior_hit() {
        let pl = open_l();
        let seg = pl.projection_line(&p(50, 50)).expect("interior hit");
        // line = the perpendicular through the point (anchored at it).
        assert_eq!(seg.get_line().a, p(50, 50));
        assert_eq!(seg.get_line().direction(), &Direction::RIGHT);
        // end = line ∩ nearest line = (100,50); the nearest line is
        // the interior polyline line itself.
        assert_eq!(seg.end_point(), p(100, 50));
        assert_eq!(seg.get_end_closing_line(), &pl.lines[1]);
    }

    /// The strictly-closer walk (`currentDistance < minDistance`): a
    /// THREE-way equidistant tie — (100,50) is 50 away from the x=50,
    /// y=100 and x=150 interior lines, and ALL THREE projections land
    /// inside their segments (the side filter accepts every candidate)
    /// — must keep the FIRST. Mutant `<=` lets the later x=150
    /// candidate replace it (last replacement wins), observable in the
    /// returned nearest line.
    #[test]
    fn projection_line_tie_keeps_first_candidate() {
        let pl = Polyline::new(vec![
            Line::new(p(0, 0), p(50, 0)),
            Line::new(p(50, 0), p(50, 100)),
            Line::new(p(50, 100), p(150, 100)),
            Line::new(p(150, 100), p(150, 0)),
            Line::new(p(150, 0), p(200, 0)),
        ]);
        let seg = pl.projection_line(&p(100, 50)).expect("tie hit");
        assert_eq!(
            seg.get_end_closing_line(),
            &pl.lines[1],
            "the first (x=50) line wins"
        );
        assert_eq!(seg.end_point(), p(50, 50));
    }

    /// The segment-bounds filter and the contained-point bail: a point
    /// whose perpendicular lands BEYOND the interior segment (both
    /// corners on the same side of the projection line) yields None;
    /// a point ON the polyline yields None (perpendicularDirection is
    /// null there).
    #[test]
    fn projection_line_misses() {
        let pl = open_l();
        // (150,150) projects to (100,150) — above the segment end.
        assert!(pl.projection_line(&p(150, 150)).is_none(), "outside bounds");
        // on the middle line: collinear, no perpendicular direction.
        assert!(pl.projection_line(&p(100, 50)).is_none(), "contained point");
    }

    /// `shorten` (`Polyline.java:916-936`), rebuild face: the open L
    /// shortened to 3 lines with a 50-long last segment keeps line 0,
    /// replaces lines 1-2 with the (100,0)-(100,50) stub and the 45/90
    /// turn continuation `turn45Degree(6)` (UP turned 270° → RIGHT),
    /// so the new last corner is the IntPoint (100,50).
    #[test]
    fn shorten_rebuilds_the_tail() {
        let pl = open_l();
        let short = pl.shorten(3, 50.0);
        let ends: Vec<(Point, Point)> = short
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(short.lines.len(), 3, "rebuild face keeps newLineCount");
        assert_eq!(
            ends,
            vec![
                (p(0, 0), p(100, 0)),
                (p(100, 0), p(100, 50)),
                // Line.getInstance unit line: (100,50) plus one RIGHT step
                (p(100, 50), p(101, 50)),
            ],
            "tail rebuilt at 50 length with the turn45Degree(6) end line"
        );
        assert_eq!(
            short.last_corner(),
            Some(p(100, 50)),
            "IntPoint last corner"
        );
    }

    /// The skip-line arm: when the rounded new last corner equals the
    /// ORIGINAL `corner(cornerCount - 2)`, the last line is skipped
    /// instead (zero-length last segment → newLastCorner == prevLast).
    /// Mutants: comparing against corner(cornerCount-1) or skipping the
    /// wrong range both flip this pin. The survivor set of the closed L
    /// is [perp-start x=0, RIGHT, UP] — fromPolygon's first line is the
    /// perpendicular START line, the dropped line 3 the perpendicular
    /// END line.
    #[test]
    fn shorten_zero_length_skips_the_last_line() {
        // closed L: corners (0,0),(10,0),(10,10), 4 lines,
        // corner(1) = (10,0) = corner(cornerCount-2).
        let pl = from_int_corners(&[(0, 0), (10, 0), (10, 10)]);
        assert_eq!(pl.lines.len(), 4);
        let short = pl.shorten(4, 0.0);
        let ends: Vec<(Point, Point)> = short
            .lines
            .iter()
            .map(|l| (l.a.clone(), l.b.clone()))
            .collect();
        assert_eq!(
            ends,
            vec![
                (p(0, 0), p(0, 1)),
                (p(0, 0), p(10, 0)),
                (p(10, 0), p(10, 10)),
            ],
            "perpendicular end line 3 is skipped"
        );
    }

    /// The degenerate newLineCount=2 face the sampling retry can reach
    /// (shapeIndex = len-3 arithmetic): corner_approx(-1) clamps onto
    /// corner_approx(0), changeLength on equal points returns the point
    /// unchanged, and the rounded corner equals corner(cornerCount-2)
    /// → skip arm → skipLines(1,1) → the 2-line remainder is EMPTIED
    /// by the normalizing ctor (Java `new Polyline` < 3 lines).
    #[test]
    fn shorten_two_lines_degenerate() {
        let pl = open_l();
        let short = pl.shorten(2, 50.0);
        assert!(
            short.is_empty(),
            "skip arm fired; the 2-line remainder emptied"
        );
    }
}
