//! Port of Java `app.freerouting.geometry.planar.PolygonShape` — a shape
//! described by a closed polygon of corner points, ordered in
//! counterclock sense, normalized so that the corner with the lowest
//! y-value (then lowest x-value) comes first.
//!
//! Bit-parity notes:
//! - Traps D6/D7: `splitToConvex` RESEEDS the shared `Random(99)`
//!   immediately before every top-level split and the recursive scan
//!   keeps drawing from the SAME LCG stream. The decomposition ORDER is
//!   parity-observable: the port seeds
//!   [`crate::java_random::JavaRandom::new(99)`] at the start of each
//!   top-level [`PolygonShape::split_to_convex`] call and passes
//!   `&mut JavaRandom` down through the recursion, reproducing the exact
//!   `nextInt(corners.length)` sequence.
//! - T14: Java memoizes `precalculatedBoundingBox` /
//!   `precalculatedBoundingOctagon` / `precalculatedConvexPieces`; the
//!   port recomputes fresh (pure functions).
//! - Java returns `null` from `splitToConvex` when the split fails
//!   (self-intersections); the port returns
//!   [`Option::None`] and the callers panic where Java would
//!   dereference the null (NPE parity), or propagate `None`.
//! - Java `PolygonShape` is the ONLY subclass of the abstract
//!   `PolylineShape`; the shared `PolylineShape` methods are flattened
//!   onto `PolygonShape` in [`crate::polyline_shape`] (documented there).

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_random::JavaRandom;
use crate::line::Line;
use crate::point::Point;
use crate::polygon::Polygon;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::ShapeBoundingDirections;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Java `PolygonShape.seed`.
const SEED: i64 = 99;

/// Java `PolygonShape`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolygonShape {
    /// The normalized corners (Java public final field).
    pub corners: Vec<Point>,
}

impl PolygonShape {
    /// Creates a new instance of PolygonShape (Java
    /// `PolygonShape(Polygon)`): reverts clockwise input, drops a closing
    /// duplicate, drops a trailing/leading collinear corner and rotates
    /// the corner sequence to start at the lowest-y (then lowest-x) corner.
    pub fn from_polygon(polygon: &Polygon) -> PolygonShape {
        PolygonShape::build(polygon)
    }

    /// Creates a polygon shape from an array of corner points (Java
    /// `PolygonShape(Point[])`).
    pub fn new(corners: &[Point]) -> PolygonShape {
        PolygonShape::from_polygon(&Polygon::new(corners))
    }

    /// The shared constructor body after the clockwise revert (Java ctor
    /// lines 35-92).
    fn build(polygon: &Polygon) -> PolygonShape {
        let reverted;
        let current_polygon: &Polygon = if polygon.winding_number_after_closing() < 0 {
            // the corners of the polygon are in clockwise sense
            reverted = polygon.revert_corners();
            &reverted
        } else {
            polygon
        };
        let current_corners = current_polygon.corner_array();
        let mut last_corner_no = current_corners.len() as i32 - 1;

        if last_corner_no > 0 && current_corners[0] == current_corners[last_corner_no as usize] {
            // skip last point
            last_corner_no -= 1;
        }

        let mut last_point_collinear = false;
        if last_corner_no >= 2 {
            last_point_collinear = current_corners[last_corner_no as usize].side_of(
                &current_corners[last_corner_no as usize - 1],
                &current_corners[0],
            ) == Side::Collinear;
        }
        if last_point_collinear {
            // skip last point
            last_corner_no -= 1;
        }

        let mut first_corner_no: i32 = 0;
        let mut first_point_collinear = false;
        if last_corner_no - first_corner_no >= 2 {
            first_point_collinear = current_corners[0].side_of(
                &current_corners[1],
                &current_corners[last_corner_no as usize],
            ) == Side::Collinear;
        }
        if first_point_collinear {
            // skip first point
            first_corner_no += 1;
        }

        // search the point with the lowest y and then with the lowest x
        let mut start_corner_no = first_corner_no;
        let mut start_corner = current_corners[start_corner_no as usize].to_float();
        for i in start_corner_no + 1..=last_corner_no {
            let current_corner = current_corners[i as usize].to_float();
            if current_corner.y < start_corner.y
                || (current_corner.y == start_corner.y && current_corner.x < start_corner.x)
            {
                start_corner_no = i;
                start_corner = current_corner;
            }
        }
        let new_corner_count = (last_corner_no - first_corner_no + 1) as usize;
        let mut result: Vec<Point> = Vec::with_capacity(new_corner_count);
        for i in start_corner_no..=last_corner_no {
            result.push(current_corners[i as usize].clone());
        }
        for i in first_corner_no..start_corner_no {
            result.push(current_corners[i as usize].clone());
        }
        PolygonShape { corners: result }
    }

    /// Java `corner(int)`: the no-th corner; panics on out-of-range where
    /// Java warns and returns null (the caller would NPE immediately).
    pub fn corner(&self, no: i32) -> Point {
        if no < 0 || no >= self.corners.len() as i32 {
            panic!("PolygonShape.corner: no out of range (Java returns null)");
        }
        self.corners[no as usize].clone()
    }

    /// Java `borderLineCount()`.
    pub fn border_line_count(&self) -> usize {
        self.corners.len()
    }

    /// Java `cornerIsBounded(int)`: always true.
    pub fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    /// Java `borderLine(int)`.
    pub fn border_line(&self, no: i32) -> Line {
        if no < 0 || no >= self.corners.len() as i32 {
            panic!("PolygonShape.borderLine: no out of range (Java returns null)");
        }
        let next_corner = if no == self.corners.len() as i32 - 1 {
            &self.corners[0]
        } else {
            &self.corners[no as usize + 1]
        };
        Line::new(self.corners[no as usize].clone(), next_corner.clone())
    }

    /// Java `intersects(Shape)` over the convex split: true if any convex
    /// piece intersects the given tile. Panics where Java would NPE
    /// (failed split).
    pub fn intersects_tile(&self, shape: &TileShape) -> bool {
        for piece in self.expect_split_pieces() {
            if piece.intersects(shape) {
                return true;
            }
        }
        false
    }

    /// Java `intersects(Circle)`.
    pub fn intersects_circle(&self, circle: &crate::circle::Circle) -> bool {
        for piece in self.expect_split_pieces() {
            if circle.intersects_tile(&crate::shape::ShapeRef::from(&piece)) {
                return true;
            }
        }
        false
    }

    /// Java `isConvex()`.
    pub fn is_convex(&self) -> bool {
        let n = self.corners.len();
        if n <= 2 {
            return true;
        }
        let mut prev_point = self.corners[n - 1].clone();
        let mut current_point = self.corners[0].clone();
        let mut next_point = self.corners[1].clone();

        for ind in 0..n {
            if next_point.side_of(&prev_point, &current_point) == Side::Negative {
                return false;
            }
            prev_point = current_point;
            current_point = next_point;
            if ind == n - 2 {
                next_point = self.corners[0].clone();
            } else if ind == n - 1 {
                next_point = self.corners[1].clone();
            } else {
                next_point = self.corners[ind + 2].clone();
            }
        }
        // check, if the sum of the interior angles is at most 2 * pi
        let first_line = Line::new(self.corners[n - 1].clone(), self.corners[0].clone());
        let mut current_line = Line::new(self.corners[0].clone(), self.corners[1].clone());
        let first_direction = int_direction_of(first_line.direction());
        let mut current_direction = int_direction_of(current_line.direction());
        let mut last_det = first_direction.determinant(&current_direction);

        for ind2 in 2..n {
            current_line = Line::new(current_line.b.clone(), self.corners[ind2].clone());
            current_direction = int_direction_of(current_line.direction());
            let current_det = first_direction.determinant(&current_direction);
            if last_det <= 0.0 && current_det > 0.0 {
                return false;
            }
            last_det = current_det;
        }
        true
    }

    /// Returns the convex hull of this polygon shape (Java `convexHull()`).
    pub fn convex_hull(&self) -> PolygonShape {
        let n = self.corners.len();
        if n <= 2 {
            return self.clone();
        }
        let mut prev_point = self.corners[n - 1].clone();
        let mut current_point = self.corners[0].clone();
        for ind in 0..n {
            let next_point = if ind == n - 1 {
                &self.corners[0]
            } else {
                &self.corners[ind + 1]
            };
            if next_point.side_of(&prev_point, &current_point) != Side::Positive {
                // skip currentPoint;
                let mut new_corners: Vec<Point> = Vec::with_capacity(n - 1);
                new_corners.extend_from_slice(&self.corners[0..ind]);
                if ind < n - 1 {
                    // copy remaining elements if present
                    new_corners.extend_from_slice(&self.corners[ind + 1..]);
                }
                let result = PolygonShape::new(&new_corners);
                return result.convex_hull();
            }
            prev_point = current_point;
            current_point = next_point.clone();
        }
        self.clone()
    }

    /// Java `boundingTile()`: the tile through the convex hull corners.
    pub fn bounding_tile(&self) -> TileShape {
        let hull = self.convex_hull();
        let hull_len = hull.corners.len();
        let mut bounding_lines: Vec<Line> = Vec::with_capacity(hull_len);
        for i in 0..hull_len - 1 {
            bounding_lines.push(Line::new(
                hull.corners[i].clone(),
                hull.corners[i + 1].clone(),
            ));
        }
        if hull_len > 0 {
            bounding_lines.push(Line::new(
                hull.corners[hull_len - 1].clone(),
                hull.corners[0].clone(),
            ));
        }
        TileShape::get_instance(&bounding_lines)
    }

    /// Java `area()`: half of the absolute shoelace sum over the float
    /// corner coordinates.
    pub fn area(&self) -> f64 {
        if self.dimension() <= 2 {
            return 0.0;
        }
        let n = self.corners.len();
        let mut result = 0.0f64;
        let mut prev_corner = self.corners[n - 2].to_float();
        let mut current_corner = self.corners[n - 1].to_float();
        for i in 0..n {
            let next_corner = self.corners[i].to_float();
            result += current_corner.x * (next_corner.y - prev_corner.y);
            prev_corner = current_corner;
            current_corner = next_corner;
        }
        0.5 * result.abs()
    }

    /// Java `dimension()`.
    pub fn dimension(&self) -> i32 {
        match self.corners.len() {
            0 => -1,
            1 => 0,
            2 => 1,
            _ => 2,
        }
    }

    /// Java `isBounded()`: always true.
    pub fn is_bounded(&self) -> bool {
        true
    }

    /// Java `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.corners.is_empty()
    }

    /// Java `translateBy(Vector)`.
    pub fn translate_by(&self, vector: &Vector) -> PolygonShape {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.translate_by(vector))
            .collect();
        PolygonShape::new(&new_corners)
    }

    /// Java `boundingShape(ShapeBoundingDirections)`.
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_polygon(self)
    }

    /// Java `boundingBox()` (uncached, T14).
    pub fn bounding_box(&self) -> IntBox {
        let mut llx = 2147483647.0f64;
        let mut lly = 2147483647.0f64;
        let mut urx = -2147483648.0f64;
        let mut ury = -2147483648.0f64;
        for corner in &self.corners {
            let current = corner.to_float();
            llx = llx.min(current.x);
            lly = lly.min(current.y);
            urx = urx.max(current.x);
            ury = ury.max(current.y);
        }
        IntBox::new(
            IntPoint::new(llx.floor() as i32, lly.floor() as i32),
            IntPoint::new(urx.ceil() as i32, ury.ceil() as i32),
        )
    }

    /// Java `boundingOctagon()` (uncached, T14).
    pub fn bounding_octagon(&self) -> IntOctagon {
        let mut lx = 2147483647.0f64;
        let mut ly = 2147483647.0f64;
        let mut rx = -2147483648.0f64;
        let mut uy = -2147483648.0f64;
        let mut ulx = 2147483647.0f64;
        let mut lrx = -2147483648.0f64;
        let mut llx = 2147483647.0f64;
        let mut urx = -2147483648.0f64;
        for corner in &self.corners {
            let current = corner.to_float();
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

    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        for piece in self.expect_split_pieces() {
            if piece.contains_float(point) {
                return true;
            }
        }
        false
    }

    /// Java `contains(Point)`.
    pub fn contains_point(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    /// Java `containsInside(Point)`.
    pub fn contains_inside(&self, point: &Point) -> bool {
        if self.contains_on_border(point) {
            return false;
        }
        !self.is_outside(point)
    }

    /// Java `isOutside(Point)`.
    pub fn is_outside(&self, point: &Point) -> bool {
        for piece in self.expect_split_pieces() {
            if !piece.is_outside(point) {
                return false;
            }
        }
        true
    }

    /// Java `containsOnBorder(Point)`: Java stubs this to `false`
    /// (the real check is commented out in the oracle) — bug-compat.
    pub fn contains_on_border(&self, _point: &Point) -> bool {
        false
    }

    /// Java `nearestPointApprox(FloatPoint)`; panics where Java would NPE
    /// (empty shape or failed split).
    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        crate::shape::nearest_point_approx_over_pieces(&self.expect_split_pieces(), from_point)
    }

    /// Java `turn90Degree(int, IntPoint)`.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> PolygonShape {
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.turn_90_degree(factor, &Point::Int(*pole)))
            .collect();
        PolygonShape::new(&new_corners)
    }

    /// Java `rotateApprox(double, FloatPoint)`.
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> PolygonShape {
        if angle == 0.0 {
            return self.clone();
        }
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| Point::Int(c.to_float().rotate(angle, pole).round()))
            .collect();
        PolygonShape::new(&new_corners)
    }

    /// Java `mirrorVertical(IntPoint)`.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> PolygonShape {
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.mirror_vertical(&Point::Int(*pole)))
            .collect();
        PolygonShape::new(&new_corners)
    }

    /// Java `mirrorHorizontal(IntPoint)`.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> PolygonShape {
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.mirror_horizontal(&Point::Int(*pole)))
            .collect();
        PolygonShape::new(&new_corners)
    }

    /// Java `enlarge(double)`: only the 0-offset fast path exists (Java
    /// warns and returns null otherwise) — `None` is that null.
    pub fn enlarge(&self, offset: f64) -> Option<PolygonShape> {
        if offset == 0.0 {
            return Some(self.clone());
        }
        None
    }

    /// Java `splitToConvex()`; `None` where Java returns null (split
    /// failed, maybe self-intersections). The `Random(99)` LCG is reseeded
    /// on EVERY call (D6/D7).
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        // use a fixed seed to get reproducible result
        let mut random_generator = JavaRandom::new(SEED);
        let convex_pieces = self.split_to_convex_recu(&mut random_generator)?;
        Some(
            convex_pieces
                .iter()
                .map(|piece| TileShape::from_points(&piece.corners))
                .collect(),
        )
    }

    /// The split pieces or the Java NPE at the dereference site.
    fn expect_split_pieces(&self) -> Vec<TileShape> {
        crate::shape::expect_pieces(self.split_to_convex())
    }

    /// Private recursive part of split_to_convex (Java
    /// `splitToConvexRecu()`); returns `None` where Java returns null.
    fn split_to_convex_recu(&self, random_generator: &mut JavaRandom) -> Option<Vec<PolygonShape>> {
        let n = self.corners.len();
        // start with a hashed corner and search the first concave corner
        let mut start_corner_no = random_generator.next_int_bound(n as u32);
        let mut current_corner = self.corners[start_corner_no as usize].clone();
        let mut prev_corner = if start_corner_no != 0 {
            self.corners[start_corner_no as usize - 1].clone()
        } else {
            self.corners[n - 1].clone()
        };

        // search for the next concave corner from here
        let mut concave_corner_no: i32 = -1;
        for _ in 0..n {
            let next_corner = if start_corner_no < n as i32 - 1 {
                self.corners[start_corner_no as usize + 1].clone()
            } else {
                self.corners[0].clone()
            };
            if next_corner.side_of(&prev_corner, &current_corner) == Side::Negative {
                // concave corner found
                concave_corner_no = start_corner_no;
                break;
            }
            prev_corner = current_corner;
            current_corner = next_corner;
            start_corner_no = (start_corner_no + 1) % n as i32;
        }
        let mut result: Vec<PolygonShape> = Vec::new();
        if concave_corner_no < 0 {
            // no concave corner found, this shape is already convex
            result.push(self.clone());
            return Some(result);
        }
        let d = DivisionPoint::new(self, concave_corner_no);
        let projection = d.projection?;

        // construct the result pieces from polygon and the division point
        let mut corner_count = d.corner_no_after_projection - concave_corner_no;
        if corner_count < 0 {
            corner_count += n as i32;
        }
        corner_count += 1;
        let mut first_arr: Vec<Point> = Vec::with_capacity(corner_count as usize);
        let mut corner_ind = concave_corner_no;
        for _ in 0..corner_count - 1 {
            first_arr.push(self.corners[corner_ind as usize].clone());
            corner_ind = (corner_ind + 1) % n as i32;
        }
        first_arr.push(Point::Int(projection.round()));
        let mut corner_count = concave_corner_no - d.corner_no_after_projection;
        if corner_count < 0 {
            corner_count += n as i32;
        }
        corner_count += 2;
        let mut last_arr: Vec<Point> = Vec::with_capacity(corner_count as usize);
        last_arr.push(Point::Int(projection.round()));
        let mut corner_ind = d.corner_no_after_projection;
        for _ in 1..corner_count {
            last_arr.push(self.corners[corner_ind as usize].clone());
            corner_ind = (corner_ind + 1) % n as i32;
        }
        let last_piece = PolygonShape::new(&last_arr);
        let first_piece = PolygonShape::new(&first_arr);
        let c1 = first_piece.split_to_convex_recu(random_generator)?;
        let c2 = last_piece.split_to_convex_recu(random_generator)?;
        result.extend(c1);
        result.extend(c2);
        Some(result)
    }
}

/// Java `(IntDirection) line.direction()` cast.
fn int_direction_of(direction: &crate::direction::Direction) -> crate::int_direction::IntDirection {
    match direction {
        crate::direction::Direction::Int(d) => *d,
        crate::direction::Direction::BigInt(_) => {
            panic!("PolygonShape: only IntDirections are supported (Java ClassCastException)")
        }
    }
}

/// Java inner class `PolygonShape.DivisionPoint`: at a concave corner of
/// the closed polygon, a minimal axis-parallel division line is
/// constructed, to divide the closed polygon into two.
struct DivisionPoint {
    corner_no_after_projection: i32,
    /// `None` where Java stores null (projection not found — maybe
    /// self-intersecting polygon).
    projection: Option<FloatPoint>,
}

impl DivisionPoint {
    fn new(shape: &PolygonShape, concave_corner_no: i32) -> DivisionPoint {
        let corners = &shape.corners;
        let n = corners.len() as i32;
        let concave_corner = corners[concave_corner_no as usize].to_float();
        let before_concave_corner = if concave_corner_no != 0 {
            corners[concave_corner_no as usize - 1].to_float()
        } else {
            corners[n as usize - 1].to_float()
        };
        let after_concave_corner = if concave_corner_no == n - 1 {
            corners[0].to_float()
        } else {
            corners[concave_corner_no as usize + 1].to_float()
        };

        let search_right =
            before_concave_corner.y > concave_corner.y || concave_corner.y > after_concave_corner.y;
        let search_left =
            before_concave_corner.y < concave_corner.y || concave_corner.y < after_concave_corner.y;
        let search_up =
            before_concave_corner.x < concave_corner.x || concave_corner.x < after_concave_corner.x;
        let search_down =
            before_concave_corner.x > concave_corner.x || concave_corner.x > after_concave_corner.x;

        // Java initializes with Integer.MAX_VALUE (2147483647.0 as double)
        let mut min_projection_dist = 2147483647.0f64;
        let mut min_projection: Option<FloatPoint> = None;
        let mut corner_no_after_min_projection: i32 = 0;

        let mut corner_no_after_curr_projection = (concave_corner_no + 2).rem_euclid(n);

        let mut corner_before_curr_projection = if corner_no_after_curr_projection != 0 {
            corners[corner_no_after_curr_projection as usize - 1].clone()
        } else {
            corners[n as usize - 1].clone()
        };
        let mut corner_before_projection_approx = corner_before_curr_projection.to_float();

        let loop_end = n - 2;
        for _ in 0..loop_end {
            let corner_after_curr_projection =
                corners[corner_no_after_curr_projection as usize].clone();
            let corner_after_projection_approx = corner_after_curr_projection.to_float();
            if corner_before_projection_approx.y != corner_after_projection_approx.y {
                // try a horizontal division
                let (min_y, max_y) =
                    if corner_after_projection_approx.y > corner_before_projection_approx.y {
                        (
                            corner_before_projection_approx.y,
                            corner_after_projection_approx.y,
                        )
                    } else {
                        (
                            corner_after_projection_approx.y,
                            corner_before_projection_approx.y,
                        )
                    };

                if concave_corner.y >= min_y && concave_corner.y <= max_y {
                    let current_line = Line::new(
                        corner_before_curr_projection.clone(),
                        corner_after_curr_projection.clone(),
                    );
                    let xintersection = current_line.function_in_y_value_approx(concave_corner.y);
                    let current_distance = (xintersection - concave_corner.x).abs();
                    // Make sure, that the new shape will not be concave at
                    // the projection point. That might happen, if the
                    // boundary curve runs back in itself.
                    let projection_ok = current_distance < min_projection_dist
                        && ((search_right
                            && xintersection > concave_corner.x
                            && concave_corner.y <= corner_after_projection_approx.y)
                            || (search_left
                                && xintersection < concave_corner.x
                                && concave_corner.y >= corner_after_projection_approx.y));
                    if projection_ok {
                        min_projection_dist = current_distance;
                        corner_no_after_min_projection = corner_no_after_curr_projection;
                        min_projection = Some(FloatPoint::new(xintersection, concave_corner.y));
                    }
                }
            }

            if corner_before_projection_approx.x != corner_after_projection_approx.x {
                // try a vertical division
                let (min_x, max_x) =
                    if corner_after_projection_approx.x > corner_before_projection_approx.x {
                        (
                            corner_before_projection_approx.x,
                            corner_after_projection_approx.x,
                        )
                    } else {
                        (
                            corner_after_projection_approx.x,
                            corner_before_projection_approx.x,
                        )
                    };
                if concave_corner.x >= min_x && concave_corner.x <= max_x {
                    let current_line = Line::new(
                        corner_before_curr_projection.clone(),
                        corner_after_curr_projection.clone(),
                    );
                    let yintersection = current_line.function_value_approx(concave_corner.x);
                    let current_distance = (yintersection - concave_corner.y).abs();
                    // make sure, that the new shape will be convex at the
                    // projection point
                    let projection_ok = current_distance < min_projection_dist
                        && ((search_up
                            && yintersection > concave_corner.y
                            && concave_corner.x >= corner_after_projection_approx.x)
                            || (search_down
                                && yintersection < concave_corner.y
                                && concave_corner.x <= corner_after_projection_approx.x));
                    if projection_ok {
                        min_projection_dist = current_distance;
                        corner_no_after_min_projection = corner_no_after_curr_projection;
                        min_projection = Some(FloatPoint::new(concave_corner.x, yintersection));
                    }
                }
            }
            corner_before_curr_projection = corner_after_curr_projection;
            corner_before_projection_approx = corner_before_curr_projection.to_float();
            if corner_no_after_curr_projection == n - 1 {
                corner_no_after_curr_projection = 0;
            } else {
                corner_no_after_curr_projection += 1;
            }
        }
        if min_projection_dist == 2147483647.0 {
            // Java: FRLogger.warn(... projection not found ...)
        }

        DivisionPoint {
            projection: min_projection,
            corner_no_after_projection: corner_no_after_min_projection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_octagon::IntOctagon;
    use crate::int_point::IntPoint;
    use crate::regular_tile_shape::RegularTileShape;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// jshell pin `PSP1` (trap D6/D7 — splitToConvex re-seeds
    /// `new Random(99)` at every top-level call): the L-shape polygon
    /// (0,0),(20,0),(20,10),(10,10),(10,20),(0,20) splits into exactly 2
    /// pieces, both IntBoxes, in the order (0,10)-(10,20) then
    /// (0,0)-(20,10). NOTE: this is deliberately only a RESULT pin — the
    /// L-shape has a single concave corner, so the hashed start corner
    /// cannot influence which concavity splits first and a 99 -> 100 seed
    /// mutant survives here (pin-failure mode: degenerate input where the
    /// seeded term cannot influence the outcome). The seed itself is
    /// pinned by `PSP2` below, whose >= 2-concavity polygon IS
    /// seed-sensitive.
    #[test]
    fn psp1_split_to_convex_l_shape_pin() {
        let l_shape =
            PolygonShape::new(&[p(0, 0), p(20, 0), p(20, 10), p(10, 10), p(10, 20), p(0, 20)]);
        let pieces = l_shape.expect_split_pieces();
        assert_eq!(pieces.len(), 2);
        let expected = [
            IntBox::new(IntPoint::new(0, 10), IntPoint::new(10, 20)),
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(20, 10)),
        ];
        for (piece, e) in pieces.iter().zip(expected.iter()) {
            assert_eq!(
                piece,
                &TileShape::RegularTileShape(RegularTileShape::IntBox(*e))
            );
        }
    }

    /// jshell pin `PSP2` (trap D6/D7 — the seed is OBSERVABLE): the
    /// chamfered plus has 13 corners and 4 concave corners, and its
    /// ambiguous convex decomposition makes the hashed start corner
    /// (`nextIntBound(13)`: draw 1 for seed 99, draw 12 for seed 100)
    /// select a different concavity, changing the whole split tree.
    /// Jar transcript (seed 99):
    /// ```text
    /// pieces: 3
    /// piece 0 class=IntOctagon corners: (30,10)(40,10)(40,27)(37,30)(30,30)
    ///   leftX=30 rightX=40 bottomY=10 topY=30
    ///   lowerLeftDiagonalX=40 upperRightDiagonalX=67
    ///   upperLeftDiagonalX=0 lowerRightDiagonalX=30
    /// piece 1 class=IntBox corners: (0,10)(10,10)(10,30)(0,30)
    /// piece 2 class=IntBox corners: (10,0)(30,0)(30,40)(10,40)
    /// ```
    /// A 99 -> 100 seed mutant produces the different partition
    /// (bottom arm | top arm | horizontal bar) and fails here — proved by
    /// temporarily mutating the `SEED` constant.
    #[test]
    fn psp2_split_to_convex_seed_pin() {
        let shape = PolygonShape::new(&[
            p(10, 0),
            p(30, 0),
            p(30, 10),
            p(40, 10),
            p(40, 27),
            p(37, 30),
            p(30, 30),
            p(30, 40),
            p(10, 40),
            p(10, 30),
            p(0, 30),
            p(0, 10),
            p(10, 10),
        ]);
        let pieces = shape.expect_split_pieces();
        assert_eq!(pieces.len(), 3);
        assert_eq!(
            pieces[0],
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::new(
                30, 10, 40, 30, 0, 30, 40, 67
            )))
        );
        assert_eq!(
            pieces[1],
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(0, 10),
                IntPoint::new(10, 30)
            )))
        );
        assert_eq!(
            pieces[2],
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(10, 0),
                IntPoint::new(30, 40)
            )))
        );
    }
}
