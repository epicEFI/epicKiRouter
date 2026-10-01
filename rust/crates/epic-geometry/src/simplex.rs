//! Port of Java `app.freerouting.geometry.planar.Simplex` — a convex shape
//! defined as the intersection of half-planes, one per directed border line
//! (`lines[i]` and its positive side). The border lines are kept sorted by
//! ascending direction (Java `Comparable<Line>`), which makes corners
//! `intersection(lines[i - 1], lines[i])` in counterclock order.
//!
//! Bit-parity notes:
//! - Java `Arrays.sort(Object[])` is **stable**; the port uses
//!   [`slice::sort_by`] (also stable, T13). Observable: after the sort,
//!   `remove_redundant_lines` deduplicates with `fastEquals` and thereby
//!   keeps the **first** of equal-direction lines (pinned in tests).
//! - Java memoizes `precalculatedCorners` / `precalculatedBoundingBox` /
//!   `precalculatedBoundingOctagon` in transient fields. Those caches are
//!   pure functions of the lines (T14 allows them), but the port recomputes
//!   fresh on every call instead — no observable difference, no interior
//!   mutability.
//! - Java's `Simplex.EMPTY` sentinel is a shared instance; the port has
//!   [`Simplex::empty`] (a fresh zero-line Simplex), which is
//!   indistinguishable because emptiness is `lines.is_empty()`.
//! - Index clamping in `corner` / `corner_approx` / `border_line` mirrors
//!   Java, which logs a warning and clamps; the port clamps silently.
//! - Where Java returns `null` the port returns `Option`
//!   (`corner_approx` on the empty simplex, `to_int_octagon`,
//!   `bounding_octagon` for an unbounded simplex) or panics where Java
//!   would immediately dereference the null (`corner`, `border_line`).
//!
//! Trap T18 (oracle bug kept for parity): in Java `cutoutFrom(Simplex)` the
//! loop ends with `nextDivisionLine = prevDivisionLine;` — a dead store.
//! `prevDivisionLine` is **never updated**, so every
//! `if (prevDivisionLine != null)` merge branch in that method is dead code
//! and never fires. The port reproduces this: `prev_division_line` stays
//! `None` for the whole loop.

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_direction::IntDirection;
use crate::int_octagon::IntOctagon;
use crate::limits::CRIT_INT;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::ShapeBoundingDirections;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Java `(IntDirection) line.direction()` cast; the port panics with the
/// Java intent (a later `ClassCastException`) instead.
fn int_dir(direction: &Direction) -> IntDirection {
    match direction {
        Direction::Int(d) => *d,
        Direction::BigInt(_) => {
            panic!("Simplex: only IntDirections are supported (Java ClassCastException)")
        }
    }
}

/// Java `a.getVector().scalarProduct(b.getVector())` restricted to
/// IntDirections (a non-int vector would ClassCastException in Java).
fn dir_scalar_product(a: &IntDirection, b: &IntDirection) -> f64 {
    match (a.get_vector(), b.get_vector()) {
        (Vector::Int(x), Vector::Int(y)) => x.scalar_product(&y),
        _ => panic!("Simplex: only IntDirections are supported (Java ClassCastException)"),
    }
}

/// `Integer.MAX_VALUE` / `Integer.MIN_VALUE` widened to double, the initial
/// values Java uses in the distance scans below.
const INT_MAX_F: f64 = 2147483647.0;
const INT_MIN_F: f64 = -2147483648.0;

/// Convex shape defined as intersection of half-planes (Java `Simplex`).
/// A half-plane is defined as the positive side of a directed line.
#[derive(Debug, Clone)]
pub struct Simplex {
    /// The directed border lines, sorted by ascending direction (only
    /// guaranteed for results of [`Simplex::get_instance`] and the
    /// intersection/offset pipelines; a raw [`Simplex::new`] keeps the
    /// caller's order like the Java constructor).
    pub(crate) lines: Vec<Line>,
}

impl PartialEq for Simplex {
    /// Structural equality over the border lines. Java `Simplex` inherits
    /// identity `equals`; the structural form exists for tests and Rust
    /// ergonomics only.
    fn eq(&self, other: &Self) -> bool {
        self.lines.len() == other.lines.len()
            && self
                .lines
                .iter()
                .zip(other.lines.iter())
                .all(|(a, b)| a == b)
    }
}

impl Simplex {
    /// Standard implementation for an empty Simplex (Java `Simplex.EMPTY`).
    pub fn empty() -> Simplex {
        Simplex { lines: Vec::new() }
    }

    /// Constructs a Simplex from the directed lines in `lines` **without**
    /// normalizing it (Java public constructor). To get a normalized simplex
    /// use [`Simplex::get_instance`].
    pub fn new(lines: Vec<Line>) -> Simplex {
        Simplex { lines }
    }

    /// Creates a Simplex as intersection of the half-planes defined by
    /// directed lines: the lines are copied, sorted by ascending direction
    /// (stable, like Java `Arrays.sort`) and redundant lines are removed.
    pub fn get_instance(lines: &[Line]) -> Simplex {
        if lines.is_empty() {
            return Simplex::empty();
        }
        let mut current_arr = lines.to_vec();
        // Sort the lines in ascending direction. Java's Arrays.sort on
        // objects is stable; sort_by is stable too (T13).
        current_arr.sort_by(|a, b| a.compare_to(b));
        Simplex::new(current_arr).remove_redundant_lines()
    }

    /// Return true, if this simplex is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Converts the physical instance of this shape to a simpler physical
    /// instance, if possible (for example a Simplex to an IntOctagon).
    pub fn simplify(&self) -> TileShape {
        if self.is_empty() {
            TileShape::Simplex(Box::new(Simplex::empty()))
        } else if self.is_int_box() {
            TileShape::RegularTileShape(RegularTileShape::IntBox(self.bounding_box()))
        } else if self.is_int_octagon() {
            let oct = self
                .to_int_octagon()
                .expect("is_int_octagon() implies to_int_octagon() succeeds");
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct))
        } else {
            TileShape::Simplex(Box::new(self.clone()))
        }
    }

    /// Returns a unique ID for this shape for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        let mut result = 0;
        for line in &self.lines {
            result = 31i32.wrapping_mul(result).wrapping_add(line.get_id());
        }
        result
    }

    /// Returns true, if the shape of this simplex is contained in a
    /// sufficiently large box.
    pub fn is_bounded(&self) -> bool {
        if self.lines.is_empty() {
            return true;
        }
        if self.lines.len() < 3 {
            return false;
        }
        (0..self.lines.len() as i32).all(|i| self.corner_is_bounded(i))
    }

    /// Returns the number of edge lines defining this simplex.
    pub fn border_line_count(&self) -> usize {
        self.lines.len()
    }

    /// Returns the intersection of the `corner_index - 1`-th with the
    /// `corner_index`-th line of this simplex. If the simplex is not bounded
    /// at this corner, the coordinates of the result will be set to
    /// `i32::MAX`-scale values.
    ///
    /// Panics if the two border lines are parallel (Java caches a `null`
    /// here, which surfaces as a `NullPointerException` at the call site).
    pub fn corner(&self, corner_index: i32) -> Point {
        let no = self.clamp_index(corner_index);
        let prev = if no == 0 {
            &self.lines[self.lines.len() - 1]
        } else {
            &self.lines[no - 1]
        };
        match self.lines[no].intersection(prev) {
            Some(p) => p,
            None => panic!("Simplex.corner: parallel border lines (Java: null corner)"),
        }
    }

    /// Returns an approximation of the intersection of the `corner_index -
    /// 1`-th with the `corner_index`-th line of this simplex by a FloatPoint.
    /// `None` for the empty simplex (Java returns `null`).
    pub fn corner_approx(&self, corner_index: i32) -> Option<FloatPoint> {
        if self.lines.is_empty() {
            return None;
        }
        let no = self.clamp_index(corner_index);
        let prev = if no == 0 {
            &self.lines[self.lines.len() - 1]
        } else {
            &self.lines[no - 1]
        };
        Some(self.lines[no].intersection_approx(prev))
    }

    /// Returns an approximation of all corners of this simplex.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..self.lines.len() as i32)
            .filter_map(|i| self.corner_approx(i))
            .collect()
    }

    /// Java index clamping with the warning paths removed: negative indices
    /// clamp to 0, indices past the last line clamp to the last line.
    fn clamp_index(&self, corner_index: i32) -> usize {
        if corner_index < 0 {
            0
        } else if corner_index as usize >= self.lines.len() {
            self.lines.len() - 1
        } else {
            corner_index as usize
        }
    }

    /// Returns the `edge_index`-th edge line of this simplex (sorted in
    /// ascending direction). Panics on the empty simplex (Java returns
    /// `null` and the caller dereferences it).
    pub fn border_line(&self, edge_index: i32) -> Line {
        if self.lines.is_empty() {
            panic!("Simplex.borderLine : simplex is empty");
        }
        let no = self.clamp_index(edge_index);
        self.lines[no].clone()
    }

    /// Returns the dimension of this simplex: 2, 1, 0, or -1 (empty).
    pub fn dimension(&self) -> i32 {
        let len = self.lines.len();
        if len == 0 {
            return -1;
        }
        if len > 4 {
            return 2;
        }
        if len == 1 {
            // we have a half plane
            return 2;
        }
        if len == 2 {
            if self.lines[0].overlaps(&self.lines[1]) {
                return 1;
            }
            return 2;
        }
        if len == 3 {
            if self.lines[0].overlaps(&self.lines[1])
                || self.lines[0].overlaps(&self.lines[2])
                || self.lines[1].overlaps(&self.lines[2])
            {
                // simplex is 1 dimensional and unbounded at one side
                return 1;
            }
            // Java: intersection of lines[1] and lines[2]; parallel lines
            // cannot occur here because overlaps() already caught them.
            let intersection = match self.lines[1].intersection(&self.lines[2]) {
                Some(p) => p,
                None => panic!("Simplex.dimension: parallel border lines after overlaps check"),
            };
            let side_of_line0 = self.lines[0].side_of(&intersection);
            if side_of_line0 == Side::Negative {
                return 2;
            }
            if side_of_line0 == Side::Positive {
                // Java logs "empty Simplex not normalized"
                return -1;
            }
            // now the 3 lines intersect in the same point
            return 0;
        }
        // now the simplex has 4 edge lines
        // check if opposing lines are collinear
        let collinear02 = self.lines[0].overlaps(&self.lines[2]);
        let collinear13 = self.lines[1].overlaps(&self.lines[3]);
        if collinear02 && collinear13 {
            return 0;
        }
        if collinear02 || collinear13 {
            return 1;
        }
        2
    }

    /// Returns the maximum diameter of the shape.
    pub fn max_width(&self) -> f64 {
        if !self.is_bounded() {
            return 2147483647.0;
        }
        let mut max_distance = INT_MIN_F;
        let mut max_distance2 = INT_MIN_F;
        let gravity_point = self.centre_of_gravity();
        for line in &self.lines {
            let current_distance = line.signed_distance(&gravity_point).abs();
            if current_distance > max_distance {
                max_distance2 = max_distance;
                max_distance = current_distance;
            } else if current_distance > max_distance2 {
                max_distance2 = current_distance;
            }
        }
        max_distance + max_distance2
    }

    /// Returns the minimum diameter of the shape.
    pub fn min_width(&self) -> f64 {
        if !self.is_bounded() {
            return 2147483647.0;
        }
        let mut min_distance = INT_MAX_F;
        let mut min_distance2 = INT_MAX_F;
        let gravity_point = self.centre_of_gravity();
        for line in &self.lines {
            let current_distance = line.signed_distance(&gravity_point).abs();
            if current_distance < min_distance {
                min_distance2 = min_distance;
                min_distance = current_distance;
            } else if current_distance < min_distance2 {
                min_distance2 = current_distance;
            }
        }
        min_distance + min_distance2
    }

    /// Checks if this simplex can be converted into an IntBox.
    pub fn is_int_box(&self) -> bool {
        for line in &self.lines {
            if !matches!(&line.a, Point::Int(_)) || !matches!(&line.b, Point::Int(_)) {
                return false;
            }
            if !line.is_orthogonal() {
                return false;
            }
            if !self.corner_is_bounded_for_line(line) {
                return false;
            }
        }
        true
    }

    /// Checks if this simplex can be converted into an IntOctagon.
    pub fn is_int_octagon(&self) -> bool {
        for line in &self.lines {
            if !matches!(&line.a, Point::Int(_)) || !matches!(&line.b, Point::Int(_)) {
                return false;
            }
            if !line.is_multiple_of_45_degree() {
                return false;
            }
            if !self.corner_is_bounded_for_line(line) {
                return false;
            }
        }
        true
    }

    /// `cornerIsBounded(i)` restricted to the corner in front of `line`
    /// (helper so `is_int_box`/`is_int_octagon` can reuse the loop index of
    /// the line they are inspecting).
    fn corner_is_bounded_for_line(&self, line: &Line) -> bool {
        let index = self
            .lines
            .iter()
            .position(|l| l.a == line.a && l.b == line.b)
            .expect("line is a border line of this simplex");
        self.corner_is_bounded(index as i32)
    }

    /// Returns true, if the determinant of the direction of index no - 1 and
    /// the direction of index no is > 0 (Java `cornerIsBounded`).
    pub fn corner_is_bounded(&self, corner_index: i32) -> bool {
        let no = self.clamp_index(corner_index);
        if self.lines.len() == 1 {
            return false;
        }
        let prev_no = if no == 0 {
            self.lines.len() - 1
        } else {
            no - 1
        };
        let prev_dir = int_dir(self.lines[prev_no].direction());
        let current_direction = int_dir(self.lines[no].direction());
        prev_dir.determinant(&current_direction) > 0.0
    }

    /// Converts this simplex to an IntOctagon. `None` if that is not
    /// possible, because not all lines of this simplex are multiples of 45
    /// degrees (Java returns `null`).
    pub fn to_int_octagon(&self) -> Option<IntOctagon> {
        // this function is at the moment only implemented for lines
        // consisting of IntPoints.
        if !self.is_int_octagon() {
            return None;
        }
        if self.is_empty() {
            return Some(IntOctagon::EMPTY);
        }

        // initialise to the biggest octagon values
        let mut rx = CRIT_INT;
        let mut uy = CRIT_INT;
        let mut lrx = CRIT_INT;
        let mut urx = CRIT_INT;
        let mut lx = -CRIT_INT;
        let mut ly = -CRIT_INT;
        let mut llx = -CRIT_INT;
        let mut ulx = -CRIT_INT;
        for line in &self.lines {
            let (a, b) = match (&line.a, &line.b) {
                (Point::Int(a), Point::Int(b)) => (a, b),
                _ => panic!("Simplex.toIntOctagon: non-IntPoint line (Java ClassCastException)"),
            };
            if a.y == b.y {
                if b.x >= a.x {
                    // lower boundary line
                    ly = a.y;
                }
                if b.x <= a.x {
                    // upper boundary line
                    uy = a.y;
                }
            }
            if a.x == b.x {
                if b.y >= a.y {
                    // right boundary line
                    rx = a.x;
                }
                if b.y <= a.y {
                    // left boundary line
                    lx = a.x;
                }
            }
            if a.y < b.y {
                if a.x < b.x {
                    // lower right boundary line
                    lrx = a.x.wrapping_sub(a.y);
                } else if a.x > b.x {
                    // upper right boundary line
                    urx = a.x.wrapping_add(a.y);
                }
            } else if a.y > b.y {
                if a.x < b.x {
                    // lower left boundary line
                    llx = a.x.wrapping_add(a.y);
                } else if a.x > b.x {
                    // upper left boundary line
                    ulx = a.x.wrapping_sub(a.y);
                }
            }
        }
        let result = IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx);
        Some(result.normalize())
    }

    /// Returns the simplex that results from translating its lines by
    /// vector.
    pub fn translate_by(&self, vector: &Vector) -> Simplex {
        if vector.is_zero() {
            return self.clone();
        }
        let new_array = self.lines.iter().map(|l| l.translate_by(vector)).collect();
        Simplex::new(new_array)
    }

    /// Returns the smallest box with int coordinates containing all corners
    /// of this simplex. The coordinates of the result will be `i32::MAX`
    /// scale if the simplex is not bounded.
    pub fn bounding_box(&self) -> IntBox {
        if self.lines.is_empty() {
            return IntBox::EMPTY;
        }
        let mut llx = 2147483647.0f64;
        let mut lly = 2147483647.0f64;
        let mut urx = -2147483648.0f64;
        let mut ury = -2147483648.0f64;
        for i in 0..self.lines.len() as i32 {
            // Java dereferences corner_approx unconditionally; the empty case
            // returned above, so Some is guaranteed here.
            let current = self.corner_approx(i).expect("non-empty simplex");
            llx = llx.min(current.x);
            lly = lly.min(current.y);
            urx = urx.max(current.x);
            ury = ury.max(current.y);
        }
        // (int) Math.floor / (int) Math.ceil narrow with saturation (T15),
        // exactly like Rust's float `as i32` cast.
        let lower_left = crate::int_point::IntPoint::new(llx.floor() as i32, lly.floor() as i32);
        let upper_right = crate::int_point::IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
        IntBox::new(lower_left, upper_right)
    }

    /// Calculates a bounding octagon of the Simplex. `None` if the Simplex
    /// is not bounded enough for an octagon (Java returns `null`).
    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        let mut lx = 2147483647.0f64;
        let mut ly = 2147483647.0f64;
        let mut rx = -2147483648.0f64;
        let mut uy = -2147483648.0f64;
        let mut ulx = 2147483647.0f64;
        let mut lrx = -2147483648.0f64;
        let mut llx = 2147483647.0f64;
        let mut urx = -2147483648.0f64;
        for i in 0..self.lines.len() as i32 {
            let current = self.corner_approx(i)?;
            lx = lx.min(current.x);
            ly = ly.min(current.y);
            rx = rx.max(current.x);
            uy = uy.max(current.y);

            let tmp = current.x - current.y;
            ulx = ulx.min(tmp);
            lrx = lrx.max(tmp);

            let tmp = current.x + current.y;
            llx = llx.min(tmp);
            urx = urx.max(tmp);
        }
        if lx.min(ly) < -(CRIT_INT as f64)
            || rx.max(uy) > CRIT_INT as f64
            || ulx.min(llx) < -(CRIT_INT as f64)
            || lrx.max(urx) > CRIT_INT as f64
        {
            // result is not bounded
            return None;
        }
        Some(
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
            .normalize(),
        )
    }

    /// Java `boundingTile`: the simplex is its own bounding tile.
    pub fn bounding_tile(&self) -> TileShape {
        TileShape::Simplex(Box::new(self.clone()))
    }

    /// Returns the bounding RegularTileShape with the fixed directions
    /// `dirs`. `None` where Java returns `null` (unbounded simplex with the
    /// 45-degree directions).
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        dirs.bounds_simplex(self)
    }

    /// Returns the simplex offsetted by width. If width > 0, the offset is
    /// to the outer, else to the inner.
    pub fn offset(&self, width: f64) -> Simplex {
        if width == 0.0 {
            return self.clone();
        }
        let new_array = self.lines.iter().map(|l| l.translate(-width)).collect();
        let mut offset_simplex = Simplex::new(new_array);
        if width < 0.0 {
            offset_simplex = offset_simplex.remove_redundant_lines();
        }
        offset_simplex
    }

    /// Returns this simplex enlarged by offset. The result simplex is
    /// intersected with the by offset enlarged bounding octagon of this
    /// simplex.
    pub fn enlarge(&self, offset: f64) -> Simplex {
        if offset == 0.0 {
            return self.clone();
        }
        let offset_simplex = self.offset(offset);
        let bounding_oct = match self.bounding_octagon() {
            Some(oct) => oct,
            None => return Simplex::empty(),
        };
        let offset_oct = bounding_oct.offset(offset);
        // Java: offsetSimplex.intersection(offsetOct.toSimplex()); the
        // octagon's toSimplex is a fresh computation (T14).
        offset_simplex.intersection_octagon(&offset_oct)
    }

    /// Returns the number of the rightmost corner seen from fromPoint. No
    /// other point of this simplex may be to the right of the line from
    /// fromPoint to the result corner.
    pub fn index_of_right_most_corner(&self, from_point: &Point) -> usize {
        let pole = from_point;
        let mut right_most_corner = self.corner(0);
        let mut result = 0usize;
        for i in 1..self.lines.len() {
            let current_corner = self.corner(i as i32);
            if current_corner.side_of(pole, &right_most_corner) == Side::Negative {
                right_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Returns the intersection of box with this simplex.
    pub fn intersection_box(&self, r#box: &IntBox) -> Simplex {
        // Java: intersection(box.toSimplex())
        self.intersection_simplex(&r#box.to_simplex())
    }

    /// Returns the intersection of this simplex with an octagon.
    pub fn intersection_octagon(&self, other: &IntOctagon) -> Simplex {
        // Java: intersection(other.toSimplex()); fresh computation (T14).
        self.intersection_simplex(&other.to_simplex())
    }

    /// Returns the intersection of this simplex and other.
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        if self.is_empty() || other.is_empty() {
            return Simplex::empty();
        }
        let mut new_array = self.lines.clone();
        new_array.extend(other.lines.iter().cloned());
        new_array.sort_by(|a, b| a.compare_to(b));
        Simplex::new(new_array).remove_redundant_lines()
    }

    /// Returns true, if this simplex and other have a nonempty intersection.
    pub fn intersects_box(&self, r#box: &IntBox) -> bool {
        self.intersects_simplex(&r#box.to_simplex())
    }

    /// Returns true, if this simplex and the octagon have a nonempty
    /// intersection.
    pub fn intersects_octagon(&self, octagon: &IntOctagon) -> bool {
        self.intersects_simplex(&octagon.to_simplex())
    }

    /// Returns true, if this simplex and other have a nonempty intersection.
    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        !self.intersection_simplex(other).is_empty()
    }

    /// Returns the edge number if line is a border line of this simplex,
    /// otherwise -1.
    pub fn border_line_index(&self, line: &Line) -> i32 {
        for (i, l) in self.lines.iter().enumerate() {
            if line == l {
                return i as i32;
            }
        }
        -1
    }

    /// Enlarges the simplex by removing the edge line with index no. The
    /// result simplex may get unbounded.
    pub fn remove_border_line(&self, no: usize) -> Simplex {
        if no >= self.lines.len() {
            return self.clone();
        }
        let mut new_array = self.lines.clone();
        new_array.remove(no);
        Simplex::new(new_array)
    }

    /// Converts the internal representation of this TileShape to a Simplex:
    /// the simplex itself (Java `toSimplex`).
    pub fn to_simplex(&self) -> Simplex {
        self.clone()
    }

    /// Returns up to `count` nearest border points of this simplex to
    /// `from_point` (Java `TileShape.nearestBorderPointsApprox(FloatPoint,
    /// int)` resolved on the simplex arm). Callers that Java routes
    /// through a `toSimplex()` conversion (e.g. the 45-degree locator's
    /// door-point check, FoundConnectionLocator45Degree.java:256-259) MUST
    /// call this method and not the [`TileShape`] one: the regular-tile
    /// border lines include zero-length edges, which changes the candidate
    /// set.
    pub fn nearest_border_points_approx(
        &self,
        from_point: &FloatPoint,
        count: i32,
    ) -> Vec<FloatPoint> {
        crate::shape::shape_nearest_border_points_approx(
            &crate::shape::ShapeRef::Simplex(self),
            from_point,
            count,
        )
    }

    /// Cuts this simplex out of `shape` (Java `cutout(TileShape)`):
    /// dispatches to `shape.cutoutFrom(this)`. See
    /// [`TileShape::cutout_from`].
    pub fn cutout(&self, shape: &TileShape) -> Vec<TileShape> {
        shape.cutout_from(&TileShape::Simplex(Box::new(self.clone())))
    }

    /// Cuts this simplex out of outer_simplex. Divides the resulting shape
    /// into simplices along the minimal distance lines from the vertices of
    /// the inner simplex to the outer simplex; returns the convex pieces
    /// constructed by this division.
    ///
    /// Panics for a degenerate (dimension < 2) inner simplex, where Java
    /// logs a warning and returns `null` (dereferenced as NPE by callers).
    /// See the T18 note in the module docs: `prev_division_line` never
    /// becomes non-`None`, exactly like the oracle.
    pub fn cutout_from_simplex(&self, outer_simplex: &Simplex) -> Vec<Simplex> {
        if self.dimension() < 2 {
            panic!("Simplex.cutout_from only implemented for 2-dim simplex (Java returns null)");
        }
        let inner_simplex = self.intersection_simplex(outer_simplex);
        if inner_simplex.dimension() < 2 {
            // nothing to cutout from outerSimplex
            return vec![outer_simplex.clone()];
        }
        let inner_corner_count = inner_simplex.lines.len();
        let mut division_line_arr: Vec<Vec<Line>> = Vec::with_capacity(inner_corner_count);
        for inner_corner_no in 0..inner_corner_count {
            match inner_simplex.calc_division_lines(inner_corner_no, outer_simplex) {
                Some(lines) => division_line_arr.push(lines),
                // Java: warning + return [outerSimplex]
                None => return vec![outer_simplex.clone()],
            }
        }
        let mut check_cross_first_line = false;
        // Java initializes prevDivisionLine = null and (T18) never assigns
        // it again; the port mirrors that with an always-None Option.
        let prev_division_line: Option<Line> = None;
        let first_division_line = division_line_arr[0][0].clone();
        let first_direction = int_dir(first_division_line.direction());
        let mut result_list: Vec<Simplex> = Vec::new();

        for inner_corner_no in 0..inner_corner_count {
            let next_corner_no = (inner_corner_no + 1) % inner_corner_count;
            let next_division_line = division_line_arr[next_corner_no][0].clone();
            let current_division_lines = &division_line_arr[inner_corner_no];
            if current_division_lines.len() == 2 {
                // 2 division lines are necessary (sharp corner).
                let current_direction = int_dir(current_division_lines[0].direction());
                let mut merge_prev_division_line = false;
                let mut merge_first_division_line = false;
                if let Some(prev) = &prev_division_line {
                    let prev_dir = int_dir(prev.direction());
                    if current_direction.determinant(&prev_dir) > 0.0 {
                        // the previous division line may intersect
                        // currentDivisionLines[0] inside the divided simplex
                        merge_prev_division_line = true;
                    }
                }
                if !check_cross_first_line {
                    check_cross_first_line = inner_corner_no > 0
                        && current_direction.determinant(&first_direction) > 0.0;
                }
                if check_cross_first_line {
                    let current_dir2 = int_dir(current_division_lines[1].direction());
                    if current_dir2.determinant(&first_direction) < 0.0 {
                        // The current piece has an intersection area with the
                        // first piece. Add a line to prevent this.
                        merge_first_division_line = true;
                    }
                }
                let mut piece_lines: Vec<Line> = Vec::with_capacity(4);
                piece_lines.push(Line::new(
                    current_division_lines[1].b.clone(),
                    current_division_lines[1].a.clone(),
                ));
                piece_lines.push(current_division_lines[0].clone());
                if merge_prev_division_line {
                    piece_lines.push(prev_division_line.as_ref().expect("checked above").clone());
                }
                if merge_first_division_line {
                    piece_lines.push(Line::new(
                        first_division_line.b.clone(),
                        first_division_line.a.clone(),
                    ));
                }
                let current_piece = Simplex::new(piece_lines);
                result_list.push(current_piece.intersection_simplex(outer_simplex));
            }
            // construct an unbounded simplex from nextDivisionLine,
            // innerSimplex.line[innerCornerNo] and the last current division
            // line, intersected with the outer simplex
            let merge_next_division_line = next_division_line.b != next_division_line.a;
            let last_curr_division_line =
                current_division_lines[current_division_lines.len() - 1].clone();
            let last_curr_dir = int_dir(last_curr_division_line.direction());
            let merge_last_curr_division_line =
                last_curr_division_line.b != last_curr_division_line.a;
            let mut merge_prev_division_line = false;
            let mut merge_first_division_line = false;
            if let Some(prev) = &prev_division_line {
                let prev_dir = int_dir(prev.direction());
                if last_curr_dir.determinant(&prev_dir) > 0.0 {
                    merge_prev_division_line = true;
                }
            }
            if !check_cross_first_line {
                check_cross_first_line = inner_corner_no > 0
                    && last_curr_dir.determinant(&first_direction) > 0.0
                    && dir_scalar_product(&last_curr_dir, &first_direction) < 0.0;
                // scalarProduct checked to ignore backcrossing at small
                // innerCornerNo
            }
            if check_cross_first_line {
                let next_dir = int_dir(next_division_line.direction());
                if next_dir.determinant(&first_direction) < 0.0 {
                    merge_first_division_line = true;
                }
            }
            let mut piece_lines: Vec<Line> = Vec::with_capacity(5);
            let current_line = inner_simplex.lines[inner_corner_no].clone();
            piece_lines.push(Line::new(current_line.b.clone(), current_line.a.clone()));
            if merge_next_division_line {
                piece_lines.push(Line::new(
                    next_division_line.b.clone(),
                    next_division_line.a.clone(),
                ));
            }
            if merge_last_curr_division_line {
                piece_lines.push(last_curr_division_line.clone());
            }
            if merge_prev_division_line {
                piece_lines.push(prev_division_line.as_ref().expect("checked above").clone());
            }
            if merge_first_division_line {
                piece_lines.push(Line::new(
                    first_division_line.b.clone(),
                    first_division_line.a.clone(),
                ));
            }
            let current_piece = Simplex::new(piece_lines);
            result_list.push(current_piece.intersection_simplex(outer_simplex));
            // Java 860: `nextDivisionLine = prevDivisionLine;` — dead store
            // (T18); prevDivisionLine is never updated.
        }
        result_list
    }

    /// Cuts this simplex out of the box `r#box` (Java `cutoutFrom(IntBox)`).
    pub fn cutout_from_box(&self, r#box: &IntBox) -> Vec<Simplex> {
        self.cutout_from_simplex(&r#box.to_simplex())
    }

    /// Cuts this simplex out of the octagon `oct` (Java
    /// `cutoutFrom(IntOctagon)`).
    pub fn cutout_from_octagon(&self, oct: &IntOctagon) -> Vec<Simplex> {
        self.cutout_from_simplex(&oct.to_simplex())
    }

    /// Removes lines which are redundant in the definition of the shape of
    /// this simplex. Assumes that the lines of this simplex are sorted.
    /// (Java package-private; public here for the sibling ports.)
    ///
    /// The Java `intersectionSides` cache is kept verbatim: same
    /// `Option<Side>` slots, same lazy fill on first use, same two
    /// `= None` resets after a line removal (Java Simplex.java:971-978;
    /// T14-compliant: the cache preserves the oracle's evaluation order
    /// instead of changing it).
    pub fn remove_redundant_lines(&self) -> Simplex {
        assert!(
            !self.lines.is_empty(),
            "removeRedundantLines on empty simplex: Java throws ArrayIndexOutOfBounds"
        );
        let mut lines: Vec<Line> = Vec::with_capacity(self.lines.len());
        // copy the sorted lines while skipping multiple lines
        let mut new_length: usize = 1;
        lines.push(self.lines[0].clone());
        let mut prev = self.lines[0].clone();
        for line in &self.lines[1..] {
            if !line.fast_equals(&prev) {
                lines.push(line.clone());
                prev = line.clone();
                new_length += 1;
            }
        }

        let mut intersection_sides: Vec<Option<Side>> = vec![None; new_length];
        // precalculated array: on which side of this line the previous and
        // the next line do intersect

        let mut try_again = new_length > 2;
        // Java uses an int that can reach -1 via `--ind; indexOfLastRemovedLine = ind;`
        let mut index_of_last_removed_line: i64 = new_length as i64;
        while try_again {
            try_again = false;
            let mut prev_ind: i64 = new_length as i64 - 1;
            let mut prev_line = lines[prev_ind as usize].clone();
            let mut current_line = lines[0].clone();
            let mut ind: i64 = 0;
            while ind < new_length as i64 {
                let next_ind: i64 = if ind == new_length as i64 - 1 {
                    0
                } else {
                    ind + 1
                };
                let next_line = lines[next_ind as usize].clone();

                let mut remove_line = false;
                let prev_dir = int_dir(prev_line.direction());
                let next_dir = int_dir(next_line.direction());
                let det = prev_dir.determinant(&next_dir);
                if det != 0.0 {
                    // prevLine and nextLine are not parallel
                    if intersection_sides[ind as usize].is_none() {
                        // intersectionSides[ind] not precalculated
                        intersection_sides[ind as usize] =
                            Some(current_line.side_of_intersection(&prev_line, &next_line));
                    }
                    if det > 0.0 {
                        // direction of nextLine is bigger than direction of
                        // prevLine: if the intersection of prevLine and
                        // nextLine is not on the left of currentLine,
                        // currentLine does not contribute to the shape
                        remove_line = intersection_sides[ind as usize] != Some(Side::Positive);
                    } else {
                        // direction of nextLine is smaller than direction of
                        // prevLine
                        if intersection_sides[ind as usize] == Some(Side::Positive) {
                            let current_direction = int_dir(current_line.direction());
                            if prev_dir.determinant(&current_direction) > 0.0 {
                                // direction of currentLine is bigger than
                                // direction of prevLine: the halfplane defined
                                // by currentLine does not intersect with the
                                // simplex defined by prevLine and nextLine,
                                // hence this simplex must be empty
                                new_length = 0;
                                try_again = false;
                                break;
                            }
                        }
                    }
                } else {
                    // prevLine and nextLine are parallel
                    if prev_line.side_of(&next_line.a) == Side::Positive {
                        // prevLine is to the left of nextLine; the half-planes
                        // defined by prevLine and nextLine do not intersect.
                        new_length = 0;
                        try_again = false;
                        break;
                    }
                }
                if remove_line {
                    try_again = true;
                    new_length -= 1;
                    let mut i = ind;
                    while i < new_length as i64 {
                        lines[i as usize] = lines[(i + 1) as usize].clone();
                        intersection_sides[i as usize] = intersection_sides[(i + 1) as usize];
                        i += 1;
                    }

                    if new_length < 3 {
                        try_again = false;
                        break;
                    }
                    // reset 3 precalculated intersectionSides
                    if ind == 0 {
                        prev_ind = new_length as i64 - 1;
                    }
                    intersection_sides[prev_ind as usize] = None;
                    let reset_ind = if ind >= new_length as i64 { 0 } else { ind };
                    intersection_sides[reset_ind as usize] = None;
                    ind -= 1;
                    index_of_last_removed_line = ind;
                } else {
                    prev_line = current_line.clone();
                    prev_ind = ind;
                }
                current_line = next_line.clone();
                if !try_again && ind >= index_of_last_removed_line {
                    // tried all lines without removing one
                    break;
                }
                ind += 1;
            }
        }

        if new_length == 2 && lines[0].is_parallel(&lines[1]) {
            if lines[0].direction() == lines[1].direction() {
                // one of the two remaining lines is redundant
                if lines[1].side_of(&lines[0].a) == Side::Positive {
                    lines[0] = lines[1].clone();
                }
                new_length -= 1;
            } else {
                // the two remaining lines have opposite direction; the
                // simplex may be empty.
                if lines[1].side_of(&lines[0].a) == Side::Positive {
                    new_length = 0;
                }
            }
        }
        if new_length == self.lines.len() {
            return self.clone(); // nothing removed
        }
        if new_length == 0 {
            return Simplex::empty();
        }
        lines.truncate(new_length);
        Simplex::new(lines)
    }

    /// For each corner of this inner simplex 1 or 2 perpendicular
    /// projections onto lines of the outer simplex are constructed, so that
    /// the resulting pieces after cutting out the inner simplex are convex.
    /// 2 projections may be necessary at sharp angle corners. Used in
    /// [`Simplex::cutout_from_simplex`].
    fn calc_division_lines(
        &self,
        inner_corner_no: usize,
        outer_simplex: &Simplex,
    ) -> Option<Vec<Line>> {
        let current_inner_line = &self.lines[inner_corner_no];
        let prev_inner_line = if inner_corner_no != 0 {
            &self.lines[inner_corner_no - 1]
        } else {
            &self.lines[self.lines.len() - 1]
        };
        let intersection = current_inner_line.intersection_approx(prev_inner_line);
        if intersection.x >= 2147483647.0 {
            // Java: warning "intersection expected" + null
            return None;
        }
        let inner_corner = intersection.round();
        let ctolerance = 0.0001;
        // Java: isExact = abs(...) < tolerance for both coordinates
        // (Simplex.java:1043-1045, strict <).
        let is_exact = (f64::from(inner_corner.x) - intersection.x).abs() < ctolerance
            && (f64::from(inner_corner.y) - intersection.y).abs() < ctolerance;

        if !is_exact {
            // it is assumed, that the corners of the original inner simplex
            // are exact and the not exact corners come from the intersection
            // of the inner simplex with the outer simplex. Because these
            // corners lie on the border of the outer simplex, no division is
            // necessary
            return Some(vec![prev_inner_line.clone()]);
        }
        let mut first_projection_dir = IntDirection::NULL;
        let mut second_projection_dir = IntDirection::NULL;
        let prev_inner_dir = int_dir(prev_inner_line.direction()).opposite();
        let next_inner_dir = int_dir(current_inner_line.direction());
        let mut outer_line_no: usize = 0;
        let corner_point = Point::Int(inner_corner);

        // search the first outer line, so that the perpendicular projection
        // of the inner corner onto this line is visible from innerCorner to
        // the left of prevInnerLine.
        let mut min_distance = INT_MAX_F;

        for _ in 0..outer_simplex.lines.len() {
            let outer_line = &outer_simplex.lines[outer_line_no];
            let current_projection_dir = match corner_point.perpendicular_direction(outer_line) {
                Direction::Int(d) => d,
                Direction::BigInt(_) => {
                    panic!("Simplex: only IntDirections supported (Java ClassCastException)")
                }
            };
            // Java compares `currentProjectionDir == Direction.NULL` by
            // reference: TRUE exactly when perpendicularDirection handed
            // back the shared NULL constant, i.e. the corner lies on
            // outerLine. `is_null()` reproduces this; `==` (Java equals)
            // would not — distinct NULL instances are unequal.
            if current_projection_dir.is_null() {
                // inner corner is on outerLine
                return Some(vec![Line::new(corner_point.clone(), corner_point.clone())]);
            }
            let projection_visible = prev_inner_dir.determinant(&current_projection_dir) >= 0.0;
            if projection_visible {
                let mut current_distance =
                    outer_line.signed_distance(&inner_corner.to_float()).abs();
                let second_division_necessary =
                    current_projection_dir.determinant(&next_inner_dir) < 0.0;
                // may occur at a sharp angle
                let mut current_second_projection_dir = current_projection_dir;

                if second_division_necessary {
                    // search the first projection dir between
                    // currentProjectionDir and nextInnerDir that is visible
                    // from next_inner_line
                    let mut second_projection_visible = false;
                    let mut tmp_outer_line_no = outer_line_no;
                    while !second_projection_visible {
                        tmp_outer_line_no = if tmp_outer_line_no == outer_simplex.lines.len() - 1 {
                            0
                        } else {
                            tmp_outer_line_no + 1
                        };
                        current_second_projection_dir = match corner_point
                            .perpendicular_direction(&outer_simplex.lines[tmp_outer_line_no])
                        {
                            Direction::Int(d) => d,
                            Direction::BigInt(_) => {
                                panic!("Simplex: only IntDirections supported")
                            }
                        };

                        // Same reference-equality semantics as above.
                        if current_second_projection_dir.is_null() {
                            // inner corner is on outerLine
                            return Some(vec![Line::new(
                                corner_point.clone(),
                                corner_point.clone(),
                            )]);
                        }
                        if current_projection_dir.determinant(&current_second_projection_dir) < 0.0
                        {
                            // currentSecondProjectionDir not found; the angle
                            // between currentProjectionDir and
                            // currentSecondProjectionDir would be already
                            // bigger than 180 degree
                            current_distance = INT_MAX_F;
                            break;
                        }

                        second_projection_visible =
                            current_second_projection_dir.determinant(&next_inner_dir) >= 0.0;
                    }
                    current_distance += outer_simplex.lines[tmp_outer_line_no]
                        .signed_distance(&inner_corner.to_float())
                        .abs();
                }
                if current_distance < min_distance {
                    min_distance = current_distance;
                    first_projection_dir = current_projection_dir;
                    second_projection_dir = current_second_projection_dir;
                }
            }
            outer_line_no = if outer_line_no == outer_simplex.lines.len() - 1 {
                0
            } else {
                outer_line_no + 1
            };
        }
        if min_distance == INT_MAX_F {
            // Java: warning "division not found" + null
            return None;
        }
        // Java (Simplex.java:1137): `firstProjectionDir.equals(
        // secondProjectionDir)` — VALUE equality (collinear side plus
        // positive projection, the T9 contract). Rust `==` on
        // IntDirection IS that contract; the `==`/reference flavor of
        // Java appears only at the NULL guards above.
        if first_projection_dir == second_projection_dir {
            Some(vec![Line::get_instance(
                corner_point.clone(),
                Direction::Int(first_projection_dir),
            )])
        } else {
            Some(vec![
                Line::get_instance(corner_point.clone(), Direction::Int(first_projection_dir)),
                Line::get_instance(corner_point.clone(), Direction::Int(second_projection_dir)),
            ])
        }
    }
}

impl Simplex {
    /// Java `TileShape.centreOfGravity` (PolylineShape formula, shared by
    /// all TileShapes): the arithmetic middle of the corner approximations.
    /// On the empty simplex this divides 0 by 0 — Java produces the same
    /// NaN garbage through its own corner-average loop.
    fn centre_of_gravity(&self) -> FloatPoint {
        crate::shape::corner_average(self.lines.len(), |i| {
            // the loop is empty for count == 0, so corner_approx is Some
            self.corner_approx(i).expect("non-empty in loop")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_point::IntPoint;

    fn line(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::new(
            Point::int(IntPoint::new(ax, ay)),
            Point::int(IntPoint::new(bx, by)),
        )
    }

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// The square (0,-5)-(6,6) of jshell pins P5/P9/P10/P12, through the
    /// four border lines E/N/W/S.
    fn square_lines() -> Vec<Line> {
        vec![
            line(0, -5, 6, -5),
            line(6, 0, 6, 6),
            line(6, 6, 0, 6),
            line(0, 6, 0, -5),
        ]
    }

    /// Invariant (Task 10): the direction sort inside
    /// `Simplex.getInstance` is STABLE for compareTo == 0 lines (Java
    /// `Arrays.sort` on objects is TimSort and Java's spec guarantees
    /// stability; the port uses the stable `slice::sort_by`). Checked on
    /// deliberately duplicate-direction fans:
    /// (a) the output is non-decreasing by `compareTo`;
    /// (b) every surviving border line IS one of the input lines,
    /// endpoint-exact (T20 rule — `Line` equality is anchor-blind, so
    /// anchors are compared pairwise): `getInstance` (Simplex.java:46-51)
    /// copies references, and `removeRedundantLines` (:884) only skips
    /// and shifts references — it never derives new lines;
    /// (c) STABILITY: survivors whose `compareTo` ties keep their INPUT
    /// relative order. The input fans are deliberately NOT sorted (the
    /// stride set wraps mod 8), so survivors do NOT appear in input
    /// order in general — stability is a property of ties only, and it
    /// survives pruning because that only deletes.
    /// 10k seeded fans.
    #[test]
    fn get_instance_sort_is_stable_for_equal_compare_to() {
        use std::cmp::Ordering;

        use crate::java_random::JavaRandom;

        /// The 8 canonical 45-degree unit vectors.
        const TEST_DIRS: [(i32, i32); 8] = [
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
        ];
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_567D);
        for _ in 0..10_000 {
            let base_no = 3 + rng.next_int_bound(2) as usize;
            let dup_no = 1 + rng.next_int_bound(3) as usize;
            let mut input: Vec<Line> = Vec::with_capacity(base_no + dup_no);
            // Base fan with pairwise different directions (stride set
            // from the corpus generator).
            let start = rng.next_int_bound(8) as usize;
            let strides = [0usize, 1, 3, 2];
            for i in 0..base_no {
                let (dx, dy) = TEST_DIRS[(start + strides[i % strides.len()]) % 8];
                let ax = rng.next_int();
                let ay = rng.next_int();
                let k = rng.next_int_bound(4000) + 1;
                input.push(line(
                    ax,
                    ay,
                    ax.wrapping_add(dx * k),
                    ay.wrapping_add(dy * k),
                ));
            }
            // Deliberate duplicates: re-emit an existing direction from a
            // fresh anchor (compareTo == 0 class with different corners).
            for _ in 0..dup_no {
                let src = input[rng.next_int_bound(input.len() as u32) as usize].clone();
                let (sa, sb) = match (&src.a, &src.b) {
                    (Point::Int(sa), Point::Int(sb)) => (*sa, *sb),
                    _ => unreachable!("fans are IntPoint-only"),
                };
                let ax = rng.next_int();
                let ay = rng.next_int();
                input.push(line(
                    ax,
                    ay,
                    ax.wrapping_add(sb.x.wrapping_sub(sa.x)),
                    ay.wrapping_add(sb.y.wrapping_sub(sa.y)),
                ));
            }
            let simplex = Simplex::get_instance(&input);
            // (a) non-decreasing by compareTo.
            for w in 1..simplex.border_line_count() as i32 {
                assert_ne!(
                    simplex
                        .border_line(w - 1)
                        .compare_to(&simplex.border_line(w)),
                    Ordering::Greater,
                    "output not sorted at {w}"
                );
            }
            // (b) survivors ARE input lines, endpoint-exact.
            // `position()` maps each survivor to its FIRST
            // endpoint-identical input line: the fresh-anchor generator
            // makes an exact (a, b) pair repeat with probability ~2^-64,
            // and a genuine repeat would mean get_instance emitted the
            // same line twice, which dedup removes (pin P1).
            // (c) tie stability: compareTo == Equal survivors keep their
            // input relative order (TimSort stability; pruning only
            // deletes). Debug dump of the input fires on mismatch.
            let mut first_pos: Vec<usize> = Vec::new();
            for w in 0..simplex.border_line_count() as i32 {
                let border = simplex.border_line(w);
                let pos = input
                    .iter()
                    .position(|l| l.a == border.a && l.b == border.b)
                    .unwrap_or_else(|| {
                        for (idx, l) in input.iter().enumerate() {
                            eprintln!("input[{idx}] = {:?}", l);
                        }
                        panic!("survivor {w} not found in input: {border:?}")
                    });
                first_pos.push(pos);
            }
            for j in 0..first_pos.len() {
                for i in 0..j {
                    if simplex
                        .border_line(i as i32)
                        .compare_to(&simplex.border_line(j as i32))
                        == Ordering::Equal
                    {
                        assert!(
                            first_pos[i] < first_pos[j],
                            "tie order inverted: survivors {i},{j} at input positions {},{}",
                            first_pos[i],
                            first_pos[j]
                        );
                    }
                }
            }
        }
    }

    /// jshell pin `P1`: two equal-direction lines on the same supporting
    /// line dedup to one and the FIRST one survives.
    #[test]
    fn p1_dedup_keeps_first() {
        let s = Simplex::get_instance(&[line(0, 0, 5, 0), line(2, 0, 7, 0)]);
        assert_eq!(s.border_line_count(), 1);
        assert_eq!(s.border_line(0).a, p(0, 0));
    }

    /// jshell pins `P5`/`P5b`/`P5c`/`P5d`: the square has 4 border lines;
    /// a redundant parallel line does not change the count; the opposite
    /// of the bottom line empties the simplex; an exact duplicate keeps
    /// the first.
    #[test]
    fn p5_square_construction_pins() {
        let s = Simplex::get_instance(&square_lines());
        assert_eq!(s.border_line_count(), 4);
        assert_eq!(s.border_line(0), line(0, -5, 6, -5));

        // P5b: an extra parallel line at y=-4 keeps the count at 4.
        let mut with_redundant = square_lines();
        with_redundant.push(line(2, -4, 8, -4));
        let s_b = Simplex::get_instance(&with_redundant);
        assert_eq!(s_b.border_line_count(), 4);

        // P5c: the opposite of the bottom line makes the simplex empty.
        let mut with_opposite = square_lines();
        with_opposite.push(line(0, -7, 6, -7).opposite());
        let s_c = Simplex::get_instance(&with_opposite);
        assert_eq!(s_c.border_line_count(), 0);

        // P5d: an exact duplicate keeps the first.
        let mut with_dup = square_lines();
        with_dup.insert(0, line(0, -5, 6, -5));
        let s_d = Simplex::get_instance(&with_dup);
        assert_eq!(s_d.border_line_count(), 4);
        assert_eq!(s_d.border_line(0), line(0, -5, 6, -5));
    }

    /// jshell pin `P10`: the square simplex converts to the exact box
    /// octagon (0,-5,6,6,-6,11,-5,12), has dimension 2 and bounding box
    /// (0,-5)-(6,6).
    #[test]
    fn p10_to_int_octagon_pin() {
        let s = Simplex::get_instance(&square_lines());
        assert_eq!(
            s.to_int_octagon(),
            Some(IntOctagon::new(0, -5, 6, 6, -6, 11, -5, 12))
        );
        assert_eq!(s.dimension(), 2);
        assert_eq!(
            s.bounding_box(),
            IntBox::new(IntPoint::new(0, -5), IntPoint::new(6, 6))
        );
    }

    /// jshell pin `P12`: `sqx.cutout(octT)` yields 5 simplex pieces with
    /// these exact border lines (piece 3 has 5 lines; the merge branches
    /// of `cutout_from_simplex` stay dead there, T18). Values re-captured
    /// live from the jar (P12b) and compared ANCHOR-STRICT (T20): `Line`
    /// equality cannot see wall anchors, and the right-wall lines here
    /// were previously mis-pinned as (6,-5)/(6,-5) anchors instead of the
    /// oracle's (6,0)/(6,0) — the square's original border-line anchors,
    /// carried through verbatim by both pipelines.
    #[test]
    fn p12_cutout_octagon_pieces_pin() {
        let sqx = Simplex::get_instance(&square_lines());
        let oct_t = TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            TileShape::from_8_ints(2, -1, 12, 8, 2, 12, 0, 8),
        ));
        let pieces = sqx.cutout(&oct_t);
        assert_eq!(pieces.len(), 5);
        let expected: [Vec<Line>; 5] = [
            vec![
                line(0, -5, 6, -5),
                line(6, 0, 6, 6),
                line(1, -1, 0, -1),
                line(0, 6, 0, -5),
            ],
            vec![
                line(0, -5, 6, -5),
                line(6, 0, 6, 6),
                line(6, 6, 0, 6),
                line(6, 6, 6, 0),
            ],
            vec![
                line(6, 0, 6, 6),
                line(6, 6, 0, 6),
                line(5, 4, 5, 3),
                line(7, 1, 8, 0),
            ],
            vec![
                line(1, 0, 2, 0),
                line(1, -1, 2, 0),
                line(5, 3, 5, 4),
                line(6, 6, 0, 6),
                line(0, 6, 0, -5),
            ],
            vec![
                line(1, -1, 2, -1),
                line(2, -1, 2, 0),
                line(2, 0, 1, 0),
                line(0, 6, 0, -5),
            ],
        ];
        for (piece, expected_lines) in pieces.iter().zip(expected.iter()) {
            let s = match piece {
                TileShape::Simplex(s) => s,
                TileShape::RegularTileShape(_) => panic!("expected a simplex piece"),
            };
            assert_eq!(s.border_line_count(), expected_lines.len());
            for (i, expected_line) in expected_lines.iter().enumerate() {
                let actual = s.border_line(i as i32);
                // T20: anchor-strict — `Line` equality (in both languages)
                // ignores where a line starts and ends on its supporting
                // line, so anchor identity is only observable through the
                // endpoint pair.
                assert_eq!((&actual.a, &actual.b), (&expected_line.a, &expected_line.b));
            }
        }
    }

    /// jshell pin `PB9` (the T19 two-flavors pin): a RAW outer simplex built
    /// with the public constructor keeps a duplicated same-direction bottom
    /// line (indices 0 and 4) — Java `==` on directions would still treat
    /// the twin perps as distinct objects. At the sharp inner corner (4,0)
    /// the bottom perpendicular DOWN is a necessary+visible first candidate
    /// whose second-division scan exits on the NW45 border (perp NE45), so
    /// `calcDivisionLines` returns TWO lines there: the
    /// `firstProjectionDir.equals(secondProjectionDir)` branch of
    /// Simplex.java:1137 runs and takes the else arm (the directions are
    /// value-unequal, so Rust `==` matches the oracle). Oracle values
    /// captured verbatim from the jar (per-corner results via reflection).
    #[test]
    fn pb9_raw_parallel_pair_second_division_pin() {
        let outer = Simplex::new(vec![
            line(0, -4, 9, -4), // e1, duplicated at index 4 below
            line(9, -4, 3, 2),  // NW45 border
            line(3, 2, 0, 2),   // top
            line(0, 2, 0, -4),  // left wall
            line(0, -4, 9, -4), // e1 again — the parallel same-direction pair
        ]);
        let inner =
            Simplex::get_instance(&[line(2, -1, 4, 0), line(4, 0, 2, 1), line(2, 1, 2, -1)]);
        assert_eq!(inner.border_line_count(), 3);
        assert_eq!(inner.border_line(0), line(2, -1, 4, 0));
        assert_eq!(inner.dimension(), 2);

        // Per-corner calcDivisionLines (package-private in Java, driven via
        // reflection there; the tests module reaches it through super::*).
        assert_eq!(
            inner.calc_division_lines(0, &outer),
            Some(vec![line(2, -1, 2, -2)])
        );
        assert_eq!(
            inner.calc_division_lines(1, &outer),
            Some(vec![line(4, 0, 4, -1), line(4, 0, 5, 1)])
        );
        assert_eq!(
            inner.calc_division_lines(2, &outer),
            Some(vec![line(2, 1, 2, 2)])
        );

        // The enclosing cutout: 3 pieces with these exact borders.
        let pieces = inner.cutout(&TileShape::Simplex(Box::new(outer)));
        assert_eq!(pieces.len(), 3);
        let expected: [Vec<Line>; 3] = [
            vec![
                line(2, -1, 4, 0),
                line(4, 0, 2, 1),
                line(4, 0, 2, -1),
                line(2, 1, 2, -1),
            ],
            vec![
                line(2, -1, 4, 0),
                line(4, 0, 2, 1),
                line(2, 1, 2, -1),
                line(2, 1, 4, 0),
            ],
            vec![
                line(2, -1, 4, 0),
                line(2, -1, 2, 1),
                line(4, 0, 2, 1),
                line(2, 1, 2, -1),
            ],
        ];
        for (piece, expected_lines) in pieces.iter().zip(expected.iter()) {
            let s = match piece {
                TileShape::Simplex(s) => s,
                TileShape::RegularTileShape(_) => panic!("expected a simplex piece"),
            };
            assert_eq!(s.border_line_count(), expected_lines.len());
            for (i, expected_line) in expected_lines.iter().enumerate() {
                let actual = s.border_line(i as i32);
                // T20: anchor-strict, as in the P12 pin.
                assert_eq!((&actual.a, &actual.b), (&expected_line.a, &expected_line.b));
            }
        }
    }
}
