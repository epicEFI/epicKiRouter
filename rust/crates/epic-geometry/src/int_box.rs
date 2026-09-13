//! Port of Java `app.freerouting.geometry.planar.IntBox`.
//!
//! Orthogonal rectangles in the plane with integer coordinates.
//!
//! Empty semantics (plan note T3): [`IntBox::is_empty`] is **value-based**
//! — any box with `ll.x > ur.x || ll.y > ur.y` is empty, so a
//! hand-constructed inverted box is empty. IntOctagon (Task 5) instead
//! uses identity-based emptiness against its `EMPTY` sentinel object;
//! the asymmetry is faithful to Java — do not "harmonize" the two.
//!
//! Wrapping: the coordinate subtractions in [`IntBox::width`],
//! [`IntBox::height`], [`IntBox::area`], [`IntBox::circumference`],
//! [`IntBox::max_width`] and [`IntBox::min_width`] use Java `int`
//! arithmetic and may wrap; `area`/`circumference` widen to `f64` only
//! after the int arithmetic, exactly like the Java casts. The offset
//! methods round via [`java_round`] (JDK 7+ ties toward +infinity) and
//! narrow to `i32` with `as` (Java's `(int)` long cast — both truncate
//! mod 2^32, they do not saturate). Direct `f64`→`i32` `as` sites (the
//! `floor`/`ceil` casts in [`IntBox::divide_into_sections`] and the
//! point-family bounding methods) likewise match Java: JLS 5.1.3
//! double→int narrowing saturates (NaN→0), as does Rust `as`; only the
//! i64→i32 narrowing sites wrap mod 2^32, identically to Java.
//!
//! Deferrals (methods whose parameter/return types belong to later M1a
//! tasks; signatures as in the Java original):
//! - Landed with Task 5 (IntOctagon): `boundingOctagon`, `toIntOctagon`,
//!   `enlarge`, `union(IntOctagon)`, `intersection(IntOctagon)`,
//!   `intersects(IntOctagon)`, `compare(IntOctagon, int)`,
//!   `isContainedIn(IntOctagon)`, `cutoutFrom(IntOctagon)`.
//! - Landed with Task 7 (TileShape/Simplex): `borderLine`,
//!   `borderLineIndex` (warn-and-(-1)), `toSimplex` (raw 4-line
//!   constructor, no redundant-line removal — pin PB1),
//!   `intersection(Simplex)`, `intersects(Simplex)`,
//!   `cutoutFrom(Simplex)`. The TileShape/trait-dispatch surface
//!   (`simplify`, `intersection(TileShape)`, `intersects(Shape)`,
//!   `contains(RegularTileShape)`, `union(RegularTileShape)`,
//!   `boundingShape`, `compare(RegularTileShape, int)`,
//!   `cutout(TileShape)`) lives on the `RegularTileShape` / `TileShape`
//!   enums.
//! - Deferred to Task 8: `boolean intersects(Circle)`.

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::limits::CRIT_INT;
use crate::line::Line;
use crate::point::Point;
use crate::rounding::java_round;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::vector::Vector;

/// Implementation of functionality of orthogonal rectangles in the plane
/// with integer coordinates (Java `IntBox`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntBox {
    /// Stores the coordinates of the lower-left corner.
    pub ll: IntPoint,
    /// Stores the coordinates of the upper-right corner.
    pub ur: IntPoint,
}

impl IntBox {
    /// Standard implementation of an empty box: the inverted
    /// `CRIT_INT` box (Java `IntBox.EMPTY`).
    pub const EMPTY: IntBox = IntBox {
        ll: IntPoint::new(CRIT_INT, CRIT_INT),
        ur: IntPoint::new(-CRIT_INT, -CRIT_INT),
    };

    /// Creates an IntBox from its lower left and upper right corners.
    pub const fn new(ll: IntPoint, ur: IntPoint) -> IntBox {
        IntBox { ll, ur }
    }

    /// Creates an IntBox from the coordinates of its lower-left and
    /// upper-right corners (Java constructor
    /// `IntBox(int, int, int, int)`).
    pub const fn from_corners(
        lower_left_x: i32,
        lower_left_y: i32,
        upper_right_x: i32,
        upper_right_y: i32,
    ) -> IntBox {
        IntBox::new(
            IntPoint::new(lower_left_x, lower_left_y),
            IntPoint::new(upper_right_x, upper_right_y),
        )
    }

    /// Java `isIntOctagon`: every IntBox is an IntOctagon.
    pub const fn is_int_octagon(&self) -> bool {
        true
    }

    /// Returns true, if the box is empty. Value-based: a hand-constructed
    /// inverted box (ll > ur componentwise) is empty — unlike IntOctagon,
    /// whose emptiness is identity-based against its EMPTY sentinel.
    pub const fn is_empty(&self) -> bool {
        self.ll.x > self.ur.x || self.ll.y > self.ur.y
    }

    /// Java `borderLineCount`.
    pub const fn border_line_count(&self) -> i32 {
        4
    }

    /// Returns the horizontal extension of the box (may wrap, like the
    /// Java int subtraction).
    pub const fn width(&self) -> i32 {
        self.ur.x.wrapping_sub(self.ll.x)
    }

    /// Returns the vertical extension of the box (may wrap, like the
    /// Java int subtraction).
    pub const fn height(&self) -> i32 {
        self.ur.y.wrapping_sub(self.ll.y)
    }

    /// Java `maxWidth`: the int extensions are computed with wrapping and
    /// widened afterwards.
    pub fn max_width(&self) -> f64 {
        f64::from(self.width().max(self.height()))
    }

    /// Java `minWidth`: the int extensions are computed with wrapping and
    /// widened afterwards.
    pub fn min_width(&self) -> f64 {
        f64::from(self.width().min(self.height()))
    }

    /// Java `area`: each subtraction wraps in int before the `(double)`
    /// cast; the product is a double multiplication.
    pub fn area(&self) -> f64 {
        f64::from(self.width()) * f64::from(self.height())
    }

    /// Java `circumference`: `2 * (w + h)` is computed entirely in int
    /// (may wrap) and widened to double only by the return type.
    pub fn circumference(&self) -> f64 {
        f64::from(2i32.wrapping_mul(self.width().wrapping_add(self.height())))
    }

    /// Returns the corner with index `no` in the order lower-left,
    /// lower-right, upper-right, upper-left. Panics (Java:
    /// `IllegalArgumentException`) for `no` out of range.
    pub fn corner(&self, no: i32) -> IntPoint {
        match no {
            0 => self.ll,
            1 => IntPoint::new(self.ur.x, self.ll.y),
            2 => self.ur,
            3 => IntPoint::new(self.ll.x, self.ur.y),
            _ => panic!("IntBox.corner: no out of range"),
        }
    }

    /// Java `dimension`: -1 for the empty box, 0 for a point, 1 for a
    /// horizontal or vertical segment, 2 for a proper box.
    pub const fn dimension(&self) -> i32 {
        if self.is_empty() {
            return -1;
        }
        if self.ll.x == self.ur.x && self.ll.y == self.ur.y {
            return 0;
        }
        if self.ur.x == self.ll.x || self.ll.y == self.ur.y {
            return 1;
        }
        2
    }

    /// Checks, if point is located in the interior of this box.
    pub fn contains_inside(&self, point: &IntPoint) -> bool {
        point.x > self.ll.x && point.x < self.ur.x && point.y > self.ll.y && point.y < self.ur.y
    }

    /// Java `isIntBox`.
    pub const fn is_int_box(&self) -> bool {
        true
    }

    /// Calculates the nearest point of this box to `from_point`.
    pub fn nearest_point(&self, from_point: &FloatPoint) -> FloatPoint {
        let x = if from_point.x <= f64::from(self.ll.x) {
            f64::from(self.ll.x)
        } else if from_point.x >= f64::from(self.ur.x) {
            f64::from(self.ur.x)
        } else {
            from_point.x
        };

        let y = if from_point.y <= f64::from(self.ll.y) {
            f64::from(self.ll.y)
        } else if from_point.y >= f64::from(self.ur.y) {
            f64::from(self.ur.y)
        } else {
            from_point.y
        };

        FloatPoint::new(x, y)
    }

    /// Calculates the sorted `max_result_points` nearest points on the
    /// border of this box. `point` is assumed to be located in the
    /// interior of this box. Only implemented for `max_result_points`
    /// <= 2 (like the Java original).
    pub fn nearest_border_projections(
        &self,
        point: &IntPoint,
        max_result_points: i32,
    ) -> Vec<IntPoint> {
        if max_result_points <= 0 {
            return Vec::new();
        }
        let result_size = max_result_points.min(2);
        let lower_horizontal_difference = point.x.wrapping_sub(self.ll.x);
        let upper_horizontal_difference = self.ur.x.wrapping_sub(point.x);
        let lower_vertical_difference = point.y.wrapping_sub(self.ll.y);
        let upper_vertical_difference = self.ur.y.wrapping_sub(point.y);

        let mut min_diff;
        let mut second_min_diff;

        let mut nearest_projection_x;
        let mut nearest_projection_y = point.y;
        let mut second_nearest_projection_x;
        let mut second_nearest_projection_y = point.y;
        if lower_horizontal_difference <= upper_horizontal_difference {
            min_diff = lower_horizontal_difference;
            second_min_diff = upper_horizontal_difference;
            nearest_projection_x = self.ll.x;
            second_nearest_projection_x = self.ur.x;
        } else {
            min_diff = upper_horizontal_difference;
            second_min_diff = lower_horizontal_difference;
            nearest_projection_x = self.ur.x;
            second_nearest_projection_x = self.ll.x;
        }
        if lower_vertical_difference < min_diff {
            second_min_diff = min_diff;
            min_diff = lower_vertical_difference;
            second_nearest_projection_x = nearest_projection_x;
            second_nearest_projection_y = nearest_projection_y;
            nearest_projection_x = point.x;
            nearest_projection_y = self.ll.y;
        } else if lower_vertical_difference < second_min_diff {
            second_min_diff = lower_vertical_difference;
            second_nearest_projection_x = point.x;
            second_nearest_projection_y = self.ll.y;
        }
        // Java's final minDiff/secondMinDiff updates in the following
        // block (IntBox.java:191-202) are dead stores — nothing reads
        // them afterwards — and are omitted here; behavior is identical.
        if upper_vertical_difference < min_diff {
            second_nearest_projection_x = nearest_projection_x;
            second_nearest_projection_y = nearest_projection_y;
            nearest_projection_x = point.x;
            nearest_projection_y = self.ur.y;
        } else if upper_vertical_difference < second_min_diff {
            second_nearest_projection_x = point.x;
            second_nearest_projection_y = self.ur.y;
        }
        let mut result = Vec::with_capacity(result_size as usize);
        result.push(IntPoint::new(nearest_projection_x, nearest_projection_y));
        if result_size > 1 {
            result.push(IntPoint::new(
                second_nearest_projection_x,
                second_nearest_projection_y,
            ));
        }

        result
    }

    /// Calculates distance of this box to `from_point`.
    pub fn distance(&self, from_point: &FloatPoint) -> f64 {
        from_point.distance(&self.nearest_point(from_point))
    }

    /// Computes the weighted distance to the box `other`.
    pub fn weighted_distance(
        &self,
        other: &IntBox,
        horizontal_weight: f64,
        vertical_weight: f64,
    ) -> f64 {
        let max_ll_x = f64::from(self.ll.x.max(other.ll.x));
        let max_ll_y = f64::from(self.ll.y.max(other.ll.y));
        let min_ur_x = f64::from(self.ur.x.min(other.ur.x));
        let min_ur_y = f64::from(self.ur.y.min(other.ur.y));

        if min_ur_x >= max_ll_x {
            (vertical_weight * (max_ll_y - min_ur_y)).max(0.0)
        } else if min_ur_y >= max_ll_y {
            (horizontal_weight * (max_ll_x - min_ur_x)).max(0.0)
        } else {
            let delta_x = (max_ll_x - min_ur_x) * horizontal_weight;
            let delta_y = (max_ll_y - min_ur_y) * vertical_weight;
            (delta_x * delta_x + delta_y * delta_y).sqrt()
        }
    }

    /// Java `boundingBox`: an IntBox is its own bounding box.
    pub const fn bounding_box(&self) -> IntBox {
        *self
    }

    /// Returns a unique ID for deterministic tie-breaking
    /// (Java `getId`: `31 * ll.getId() + ur.getId()`, int arithmetic).
    pub fn get_id(&self) -> i32 {
        31i32
            .wrapping_mul(self.ll.get_id())
            .wrapping_add(self.ur.get_id())
    }

    /// Java `isBounded`.
    pub const fn is_bounded(&self) -> bool {
        true
    }

    /// Java `boundingTile`: an IntBox is its own bounding tile.
    pub const fn bounding_tile(&self) -> IntBox {
        *self
    }

    /// Java `cornerIsBounded`: true for every index (the Java original
    /// does not validate `no`).
    pub const fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    /// Returns the smallest IntBox containing this box and `other`
    /// (Java `union(IntBox)`).
    pub fn union(&self, other: &IntBox) -> IntBox {
        IntBox::from_corners(
            self.ll.x.min(other.ll.x),
            self.ll.y.min(other.ll.y),
            self.ur.x.max(other.ur.x),
            self.ur.y.max(other.ur.y),
        )
    }

    /// Returns the intersection of this box with an IntBox. Returns the
    /// [`IntBox::EMPTY`] sentinel values exactly (not a fresh inverted
    /// box) when the boxes are disjoint, in the Java check order.
    pub fn intersection(&self, other: &IntBox) -> IntBox {
        if other.ll.x > self.ur.x {
            return IntBox::EMPTY;
        }
        if other.ll.y > self.ur.y {
            return IntBox::EMPTY;
        }
        if self.ll.x > other.ur.x {
            return IntBox::EMPTY;
        }
        if self.ll.y > other.ur.y {
            return IntBox::EMPTY;
        }
        IntBox::from_corners(
            self.ll.x.max(other.ll.x),
            self.ll.y.max(other.ll.y),
            self.ur.x.min(other.ur.x),
            self.ur.y.min(other.ur.y),
        )
    }

    /// Returns true, if this box intersects `other` (Java
    /// `intersects(IntBox)`).
    pub fn intersects(&self, other: &IntBox) -> bool {
        if other.ll.x > self.ur.x {
            return false;
        }
        if other.ll.y > self.ur.y {
            return false;
        }
        if self.ll.x > other.ur.x {
            return false;
        }
        self.ll.y <= other.ur.y
    }

    /// Returns true, if this box intersects `other` and the intersection
    /// is 2-dimensional (Java `overlaps(IntBox)`).
    pub fn overlaps(&self, other: &IntBox) -> bool {
        if other.ll.x >= self.ur.x {
            return false;
        }
        if other.ll.y >= self.ur.y {
            return false;
        }
        if self.ll.x >= other.ur.x {
            return false;
        }
        self.ll.y < other.ur.y
    }

    /// Returns the translation of this box by `rel_coor`. Panics (Java:
    /// `ClassCastException` on the `(IntPoint)` casts) if the vector has
    /// rational coordinates.
    pub fn translate_by(&self, rel_coor: &Vector) -> IntBox {
        // This function is at the moment only implemented for Vectors
        // with integer coordinates. The general implementation is still
        // missing.
        if *rel_coor == Vector::ZERO {
            return *self;
        }
        let new_ll = Point::Int(self.ll).translate_by(rel_coor);
        let new_ur = Point::Int(self.ur).translate_by(rel_coor);
        match (new_ll, new_ur) {
            (Point::Int(ll), Point::Int(ur)) => IntBox::new(ll, ur),
            _ => panic!("IntBox.translateBy: expected an int vector"),
        }
    }

    /// Turns this box by `factor` times 90 degrees around `pole`
    /// (Java `turn90Degree`).
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> IntBox {
        let p1 = Point::Int(self.ll).turn_90_degree(factor, &Point::Int(*pole));
        let p2 = Point::Int(self.ur).turn_90_degree(factor, &Point::Int(*pole));
        let (p1, p2) = match (p1, p2) {
            (Point::Int(p1), Point::Int(p2)) => (p1, p2),
            _ => panic!("IntBox.turn90Degree: expected int points"),
        };

        IntBox::from_corners(
            p1.x.min(p2.x),
            p1.y.min(p2.y),
            p1.x.max(p2.x),
            p1.y.max(p2.y),
        )
    }

    /// Returns the box offsetted by `dist`. If `dist > 0`, the offset is
    /// to the outside, else to the inside. The rounding is
    /// [`java_round`] (ties toward +infinity), then the Java `(int)`
    /// narrowing cast (truncating mod 2^32).
    pub fn offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let rounded_distance = java_round(dist) as i32;
        IntBox::new(
            IntPoint::new(
                self.ll.x.wrapping_sub(rounded_distance),
                self.ll.y.wrapping_sub(rounded_distance),
            ),
            IntPoint::new(
                self.ur.x.wrapping_add(rounded_distance),
                self.ur.y.wrapping_add(rounded_distance),
            ),
        )
    }

    /// Returns the box, where the horizontal boundary is offsetted by
    /// `dist` (rounding as in [`IntBox::offset`]).
    pub fn horizontal_offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let rounded_distance = java_round(dist) as i32;
        IntBox::new(
            IntPoint::new(self.ll.x.wrapping_sub(rounded_distance), self.ll.y),
            IntPoint::new(self.ur.x.wrapping_add(rounded_distance), self.ur.y),
        )
    }

    /// Returns the box, where the vertical boundary is offsetted by
    /// `dist` (rounding as in [`IntBox::offset`]).
    pub fn vertical_offset(&self, dist: f64) -> IntBox {
        if dist == 0.0 || self.is_empty() {
            return *self;
        }
        let rounded_distance = java_round(dist) as i32;
        IntBox::new(
            IntPoint::new(self.ll.x, self.ll.y.wrapping_sub(rounded_distance)),
            IntPoint::new(self.ur.x, self.ur.y.wrapping_add(rounded_distance)),
        )
    }

    /// Shrinks the width and height of the box by `width`. The box will
    /// not vanish completely (it collapses to its center point instead).
    pub fn shrink(&self, width: i32) -> IntBox {
        let (lower_left_x, upper_right_x) =
            if 2i32.wrapping_mul(width) <= self.ur.x.wrapping_sub(self.ll.x) {
                (self.ll.x.wrapping_add(width), self.ur.x.wrapping_sub(width))
            } else {
                let mid = self.ll.x.wrapping_add(self.ur.x) / 2;
                (mid, mid)
            };
        let (lower_left_y, upper_right_y) =
            if 2i32.wrapping_mul(width) <= self.ur.y.wrapping_sub(self.ll.y) {
                (self.ll.y.wrapping_add(width), self.ur.y.wrapping_sub(width))
            } else {
                let mid = self.ll.y.wrapping_add(self.ur.y) / 2;
                (mid, mid)
            };
        IntBox::from_corners(lower_left_x, lower_left_y, upper_right_x, upper_right_y)
    }

    /// Compares the edge line with index `edge_index` (0 = lower,
    /// 1 = right, 2 = upper, 3 = left) of this box and `other` in the
    /// [`RegularTileShape`] edge order: [`Side::Positive`] (Java
    /// `ON_THE_LEFT`) if this edge is left of the other, [`Side::Negative`]
    /// (Java `ON_THE_RIGHT`) if right, [`Side::Collinear`] if equal.
    /// Panics (Java: `IllegalArgumentException`) if `edge_index` is out
    /// of range. The `RegularTileShape`/`IntOctagon` overloads are
    /// deferred to Tasks 5/7.
    pub fn compare(&self, other: &IntBox, edge_index: i32) -> Side {
        match edge_index {
            0 => {
                // compare the lower edge line
                if self.ll.y > other.ll.y {
                    Side::Positive
                } else if self.ll.y < other.ll.y {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            1 => {
                // compare the right edge line
                if self.ur.x < other.ur.x {
                    Side::Positive
                } else if self.ur.x > other.ur.x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            2 => {
                // compare the upper edge line
                if self.ur.y < other.ur.y {
                    Side::Positive
                } else if self.ur.y > other.ur.y {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            3 => {
                // compare the left edge line
                if self.ll.x > other.ll.x {
                    Side::Positive
                } else if self.ll.x < other.ll.x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            _ => panic!("IntBox.compare: edgeIndex out of range"),
        }
    }

    /// Returns true, if this box is contained in `other` (Java
    /// `isContainedIn(IntBox)`). The Java shortcut compares reference
    /// identity (`this == other`); value equality implies the coordinate
    /// predicate below anyway, so the value-based shortcut here is
    /// equivalent.
    pub fn is_contained_in(&self, other: &IntBox) -> bool {
        if self.is_empty() || self == other {
            return true;
        }
        self.ll.x >= other.ll.x
            && self.ll.y >= other.ll.y
            && self.ur.x <= other.ur.x
            && self.ur.y <= other.ur.y
    }

    /// Return true, if `other` is contained in the interior of this box
    /// (Java `containsInInterior(IntBox)`).
    pub fn contains_in_interior(&self, other: &IntBox) -> bool {
        if other.is_empty() {
            return true;
        }
        other.ll.x > self.ll.x
            && other.ll.y > self.ll.y
            && other.ur.x < self.ur.x
            && other.ur.y < self.ur.y
    }

    /// Calculates the part of `from_box`, which has minimal distance to
    /// this box (Java `nearestPart(IntBox)`).
    pub fn nearest_part(&self, from_box: &IntBox) -> IntBox {
        let ll_x = if from_box.ll.x >= self.ll.x {
            from_box.ll.x
        } else {
            from_box.ur.x.min(self.ll.x)
        };

        let ur_x = if from_box.ur.x <= self.ur.x {
            from_box.ur.x
        } else {
            from_box.ll.x.max(self.ur.x)
        };

        let ll_y = if from_box.ll.y >= self.ll.y {
            from_box.ll.y
        } else {
            from_box.ur.y.min(self.ll.y)
        };

        let ur_y = if from_box.ur.y <= self.ur.y {
            from_box.ur.y
        } else {
            from_box.ll.y.max(self.ur.y)
        };
        IntBox::from_corners(ll_x, ll_y, ur_x, ur_y)
    }

    /// Divides this box into sections with width and height at most
    /// `max_section_width` of about equal size (Java
    /// `divideIntoSections`).
    pub fn divide_into_sections(&self, max_section_width: f64) -> Vec<IntBox> {
        if max_section_width <= 0.0 {
            return Vec::new();
        }
        let length = f64::from(self.ur.x.wrapping_sub(self.ll.x));
        let height = f64::from(self.ur.y.wrapping_sub(self.ll.y));
        let xcount = (length / max_section_width).ceil() as i32;
        let ycount = (height / max_section_width).ceil() as i32;
        // A wrapped (negative) extension makes Java throw
        // NegativeArraySizeException at `new IntBox[xcount * ycount]`
        // (IntBox.java:660); mirror that failure instead of silently
        // returning an empty/partial result.
        assert!(
            xcount >= 0 && ycount >= 0,
            "IntBox.divideIntoSections: negative section count"
        );
        let section_length_x = (length / f64::from(xcount)).ceil() as i32;
        let section_length_y = (height / f64::from(ycount)).ceil() as i32;
        let mut result = Vec::new();
        for j in 0..ycount {
            let current_lower_left_y = self.ll.y.wrapping_add(j.wrapping_mul(section_length_y));
            let current_upper_right_y = if j == ycount - 1 {
                self.ur.y
            } else {
                current_lower_left_y.wrapping_add(section_length_y)
            };
            for i in 0..xcount {
                let current_lower_left_x = self.ll.x.wrapping_add(i.wrapping_mul(section_length_x));
                let current_upper_right_x = if i == xcount - 1 {
                    self.ur.x
                } else {
                    current_lower_left_x.wrapping_add(section_length_x)
                };
                result.push(IntBox::from_corners(
                    current_lower_left_x,
                    current_lower_left_y,
                    current_upper_right_x,
                    current_upper_right_y,
                ));
            }
        }
        result
    }

    /// Calculates the pieces of `d` remaining after cutting out this box
    /// (Java package-private `cutoutFrom(IntBox)`). The four pieces are
    /// rearranged so that the cumulative circumference is minimal.
    pub fn cutout_from(&self, d: &IntBox) -> Vec<IntBox> {
        let c = self.intersection(d);
        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![*d];
        }

        let mut result = [
            IntBox::from_corners(d.ll.x, d.ll.y, c.ur.x, c.ll.y),
            IntBox::from_corners(d.ll.x, c.ll.y, c.ll.x, d.ur.y),
            IntBox::from_corners(c.ur.x, d.ll.y, d.ur.x, c.ur.y),
            IntBox::from_corners(c.ll.x, c.ur.y, d.ur.x, d.ur.y),
        ];

        // now the division will be optimised, so that the cumulative
        // circumference will be minimal.

        if c.ll.x.wrapping_sub(d.ll.x) > c.ll.y.wrapping_sub(d.ll.y) {
            // switch left dividing line to lower
            let b = result[0];
            result[0] = IntBox::from_corners(c.ll.x, b.ll.y, b.ur.x, b.ur.y);
            let b = result[1];
            result[1] = IntBox::from_corners(b.ll.x, d.ll.y, b.ur.x, b.ur.y);
        }
        if d.ur.y.wrapping_sub(c.ur.y) > c.ll.x.wrapping_sub(d.ll.x) {
            // switch upper dividing line to the left
            let b = result[1];
            result[1] = IntBox::from_corners(b.ll.x, b.ll.y, b.ur.x, c.ur.y);
            let b = result[3];
            result[3] = IntBox::from_corners(d.ll.x, b.ll.y, b.ur.x, b.ur.y);
        }
        if d.ur.x.wrapping_sub(c.ur.x) > d.ur.y.wrapping_sub(c.ur.y) {
            // switch right dividing line to upper
            let b = result[2];
            result[2] = IntBox::from_corners(b.ll.x, b.ll.y, b.ur.x, d.ur.y);
            let b = result[3];
            result[3] = IntBox::from_corners(b.ll.x, b.ll.y, c.ur.x, b.ur.y);
        }
        if c.ll.y.wrapping_sub(d.ll.y) > d.ur.x.wrapping_sub(c.ur.x) {
            // switch lower dividing line to the left
            let b = result[0];
            result[0] = IntBox::from_corners(b.ll.x, b.ll.y, d.ur.x, b.ur.y);
            let b = result[2];
            result[2] = IntBox::from_corners(b.ll.x, c.ll.y, b.ur.x, b.ur.y);
        }
        result.to_vec()
    }

    // --- Task 5 closures (IntOctagon) ---

    /// Java `toIntOctagon`: the four axis bounds plus the four diagonal
    /// intercepts through the corners (the int sums wrap like Java).
    pub const fn to_int_octagon(&self) -> IntOctagon {
        IntOctagon::new(
            self.ll.x,
            self.ll.y,
            self.ur.x,
            self.ur.y,
            self.ll.x.wrapping_sub(self.ur.y),
            self.ur.x.wrapping_sub(self.ll.y),
            self.ll.x.wrapping_add(self.ll.y),
            self.ur.x.wrapping_add(self.ur.y),
        )
    }

    /// Java `boundingOctagon`: the box converted to an octagon.
    pub const fn bounding_octagon(&self) -> IntOctagon {
        self.to_int_octagon()
    }

    /// Java `enlarge(double)`: the bounding octagon offsetted by `offset`.
    pub fn enlarge(&self, offset: f64) -> IntOctagon {
        self.bounding_octagon().offset(offset)
    }

    /// Java `union(IntOctagon)`: delegates to the octagon union with this
    /// box as an octagon.
    pub fn union_octagon(&self, other: &IntOctagon) -> IntOctagon {
        other.union(&self.to_int_octagon())
    }

    /// Java package-private `intersection(IntOctagon)`: delegates to the
    /// octagon intersection with this box as an octagon.
    pub fn intersection_octagon(&self, other: &IntOctagon) -> IntOctagon {
        other.intersection(&self.to_int_octagon())
    }

    /// Java `intersects(IntOctagon)`.
    pub fn intersects_octagon(&self, other: &IntOctagon) -> bool {
        other.intersects(&self.to_int_octagon())
    }

    /// Java `compare(IntOctagon, int)`.
    pub fn compare_octagon(&self, other: &IntOctagon, edge_index: i32) -> Side {
        self.to_int_octagon().compare(other, edge_index)
    }

    /// Java `isContainedIn(IntOctagon)`: true if this box is contained in
    /// `other`. The Java dispatch is a round trip —
    /// `other.contains(toIntOctagon())` forwards back to
    /// `toIntOctagon().isContainedIn(other)` — so the net semantics is
    /// box ⊆ other; the delegation below is that final call directly.
    pub fn is_contained_in_octagon(&self, other: &IntOctagon) -> bool {
        self.to_int_octagon().is_contained_in(other)
    }

    /// Java package-private `cutoutFrom(IntOctagon)`: delegates to the
    /// octagon cutout with this box as an octagon.
    pub fn cutout_from_octagon(&self, other: &IntOctagon) -> Vec<IntOctagon> {
        self.to_int_octagon().cutout_from(other)
    }
}

// ---------------------------------------------------------------------------
// Task 7 closures (TileShape/Simplex family)
// ---------------------------------------------------------------------------

impl IntBox {
    /// Converts this box to a Simplex. Faithful to `IntBox.toSimplex()`: the
    /// four boundary lines go to the `Simplex` constructor **without**
    /// sorting or redundant-line removal (empty box -> zero lines); the
    /// construction order is already cyclically direction-sorted (jshell
    /// pin `PB1`).
    pub fn to_simplex(&self) -> Simplex {
        if self.is_empty() {
            return Simplex::empty();
        }
        Simplex::new(vec![
            Line::get_instance(Point::int(self.ll), Direction::Int(IntDirection::RIGHT)),
            Line::get_instance(Point::int(self.ur), Direction::Int(IntDirection::UP)),
            Line::get_instance(Point::int(self.ur), Direction::Int(IntDirection::LEFT)),
            Line::get_instance(Point::int(self.ll), Direction::Int(IntDirection::DOWN)),
        ])
    }

    /// The no-th boundary line as an infinite supporting line (Java
    /// `IntBox.borderLine`): 0 lower, 1 right, 2 upper, 3 left. Panics out
    /// of range (Java throws IllegalArgumentException).
    pub fn border_line(&self, no: i32) -> Line {
        match no {
            0 => Line::new(
                Point::int(IntPoint::new(0, self.ll.y)),
                Point::int(IntPoint::new(1, self.ll.y)),
            ),
            1 => Line::new(
                Point::int(IntPoint::new(self.ur.x, 0)),
                Point::int(IntPoint::new(self.ur.x, 1)),
            ),
            2 => Line::new(
                Point::int(IntPoint::new(0, self.ur.y)),
                Point::int(IntPoint::new(-1, self.ur.y)),
            ),
            3 => Line::new(
                Point::int(IntPoint::new(self.ll.x, 0)),
                Point::int(IntPoint::new(self.ll.x, -1)),
            ),
            _ => panic!("IntBox.borderLine: no out of range"),
        }
    }

    /// Returns -1: Java logs a warning ("not yet implemented for IntBoxes")
    /// because the regular shapes would have to test line IDENTITY instead
    /// of geometry. Bug-compatible with the oracle.
    pub fn border_line_index(&self, _line: &Line) -> i32 {
        -1
    }

    /// Returns the intersection of this box with the simplex (Java
    /// `IntBox.intersection(Simplex)` ==
    /// `other.intersection(this.toSimplex())`).
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        other.intersection_simplex(&self.to_simplex())
    }

    /// Returns true if this box and the simplex have a nonempty
    /// intersection (Java `IntBox.intersects(Simplex)` ==
    /// `other.intersects(this.toSimplex())`).
    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        other.intersects_simplex(&self.to_simplex())
    }

    /// Cuts this box out of the simplex (Java `IntBox.cutoutFrom(Simplex)`
    /// == `this.toSimplex().cutoutFrom(simplex)`); the pieces belong to the
    /// simplex (the argument is the outer shape).
    pub fn cutout_from_simplex(&self, simplex: &Simplex) -> Vec<Simplex> {
        self.to_simplex().cutout_from_simplex(simplex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_vector::IntVector;
    use crate::rational_point::RationalPoint;
    use crate::rational_vector::RationalVector;
    use num_bigint::BigInt;

    fn b(llx: i32, lly: i32, urx: i32, ury: i32) -> IntBox {
        IntBox::from_corners(llx, lly, urx, ury)
    }

    fn p(x: i32, y: i32) -> IntPoint {
        IntPoint::new(x, y)
    }

    /// Invariant (Task 10): IntBox.intersection is commutative — the
    /// result is the per-axis max(ll) / min(ur) construction plus the
    /// shared emptiness handling, both symmetric in the operands. 10k
    /// seeded pairs, mostly small-band boxes with every 8th pair at
    /// full-range coordinates (so the boundary arithmetic cannot
    /// silently cancel).
    #[test]
    fn intersection_is_commutative_on_random_boxes() {
        use crate::java_random::JavaRandom;
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_567A);
        let coord = |rng: &mut JavaRandom| {
            if rng.next_int_bound(8) == 0 {
                rng.next_int()
            } else {
                rng.next_int_bound(4001) - 2000
            }
        };
        for _ in 0..10_000 {
            let a = b(
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
            );
            let c = b(
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
            );
            assert_eq!(
                a.intersection(&c),
                c.intersection(&a),
                "intersection not commutative for {a:?} x {c:?}"
            );
        }
        // Deliberate sentinel probe: the shared EMPTY box (fresh
        // sentinel-valued results on both sides) must behave
        // symmetrically too.
        let nonempty = b(1, 2, 3, 4);
        assert_eq!(
            IntBox::EMPTY.intersection(&nonempty),
            nonempty.intersection(&IntBox::EMPTY),
            "EMPTY-sentinel intersection not commutative"
        );
    }

    /// jshell pin: `IntBox.EMPTY` field values.
    #[test]
    fn empty_sentinel_fields_exact() {
        assert_eq!(IntBox::EMPTY.ll, p(CRIT_INT, CRIT_INT));
        assert_eq!(IntBox::EMPTY.ur, p(-CRIT_INT, -CRIT_INT));
        assert!(IntBox::EMPTY.is_empty());
        assert_eq!(IntBox::EMPTY.dimension(), -1);
    }

    /// Plan note T3: emptiness is value-based — a hand-constructed
    /// inverted box IS empty (contrast with IntOctagon's identity-based
    /// empty in Task 5).
    #[test]
    fn is_empty_is_value_based() {
        assert!(b(5, 5, 3, 4).is_empty());
        assert!(b(0, 0, 10, -1).is_empty());
        assert!(b(0, 0, -1, 10).is_empty());
        assert!(!b(0, 0, 10, 10).is_empty());
        // degenerate boxes are not empty
        assert!(!b(3, 3, 3, 3).is_empty());
        assert!(!b(3, 3, 7, 3).is_empty());
        assert_eq!(b(3, 3, 3, 3).dimension(), 0);
        assert_eq!(b(3, 3, 7, 3).dimension(), 1);
        assert_eq!(b(3, 3, 3, 7).dimension(), 1);
        assert_eq!(b(0, 0, 10, 10).dimension(), 2);
    }

    /// jshell pins: `offset(-2.5)` -> (2,2,8,8) per T1 (java_round ties
    /// toward +inf, so -2.5 rounds to -2); `offset(2.5)` -> (-3,-3,13,13).
    #[test]
    fn offset_rounding_ties_toward_plus_infinity() {
        let box10 = b(0, 0, 10, 10);
        assert_eq!(box10.offset(-2.5), b(2, 2, 8, 8));
        assert_eq!(box10.offset(2.5), b(-3, -3, 13, 13));
        assert_eq!(box10.offset(2.4), b(-2, -2, 12, 12));
        assert_eq!(box10.offset(-2.6), b(3, 3, 7, 7));
        assert_eq!(box10.horizontal_offset(-2.5), b(2, 0, 8, 10));
        assert_eq!(box10.vertical_offset(2.5), b(0, -3, 10, 13));
        // dist == 0 and the empty box are returned unchanged
        assert_eq!(box10.offset(0.0), box10);
        assert_eq!(IntBox::EMPTY.offset(5.0), IntBox::EMPTY);
        assert!(IntBox::EMPTY.offset(5.0).is_empty());
    }

    /// Java `(int) Math.round(dist)` narrows the long by truncation
    /// mod 2^32 (NOT saturation); the resulting inverted box is empty.
    #[test]
    fn offset_narrowing_truncates_like_java() {
        let big = b(0, 0, 10, 10).offset(3.0e9);
        assert_eq!(big.ll, p(1_294_967_296, 1_294_967_296));
        assert_eq!(big.ur, p(-1_294_967_286, -1_294_967_286));
        assert!(big.is_empty());
    }

    /// jshell pin: all four disjoint orientations return the exact EMPTY
    /// sentinel fields.
    #[test]
    fn intersection_of_disjoint_returns_sentinel_exactly() {
        let a = b(0, 0, 10, 10);
        let cases = [
            b(20, 5, 30, 15),   // other.ll.x > ur.x
            b(-5, 20, 5, 30),   // other.ll.y > ur.y
            b(-30, -5, -20, 5), // ll.x > other.ur.x
            b(-5, -30, 5, -20), // ll.y > other.ur.y
        ];
        for other in cases {
            let i = a.intersection(&other);
            assert_eq!(i, IntBox::EMPTY);
            assert_eq!(i.ll.x, CRIT_INT);
            assert_eq!(i.ll.y, CRIT_INT);
            assert_eq!(i.ur.x, -CRIT_INT);
            assert_eq!(i.ur.y, -CRIT_INT);
            assert!(i.is_empty());
            assert!(!a.intersects(&other));
            assert!(other.intersection(&a) == IntBox::EMPTY || other.ll.x <= a.ur.x);
        }
        // touching edges are not disjoint: single point / segment result
        assert_eq!(a.intersection(&b(10, 5, 20, 15)), b(10, 5, 10, 10));
        assert_eq!(a.intersection(&b(-10, 5, 0, 15)), b(0, 5, 0, 10));
        assert_eq!(a.intersection(&b(2, 3, 8, 7)), b(2, 3, 8, 7));
        assert_eq!(a.intersection(&IntBox::EMPTY), IntBox::EMPTY);
        assert_eq!(IntBox::EMPTY.intersection(&a), IntBox::EMPTY);
    }

    #[test]
    fn union_min_max_and_empty_identity() {
        let a = b(0, 0, 10, 10);
        assert_eq!(a.union(&b(20, 5, 30, 15)), b(0, 0, 30, 15));
        assert_eq!(a.union(&b(-5, -5, 5, 5)), b(-5, -5, 10, 10));
        assert_eq!(a.union(&IntBox::EMPTY), a);
        assert_eq!(IntBox::EMPTY.union(&a), a);
    }

    /// jshell pins: the coordinate subtractions wrap in int BEFORE the
    /// double cast; circumference wraps in the int multiply.
    #[test]
    fn area_circumference_and_width_wrap_like_java() {
        let w = b(-2_000_000_000, -5, 2_000_000_000, 5);
        assert_eq!(w.width(), -294_967_296);
        assert_eq!(w.height(), 10);
        assert_eq!(w.area(), -2_949_672_960.0);
        assert_eq!(w.circumference(), -589_934_572.0);
        assert_eq!(w.max_width(), 10.0);
        assert_eq!(w.min_width(), -294_967_296.0);
        // no wrap for sane coordinates
        let n = b(2, 2, 8, 8);
        assert_eq!(n.area(), 36.0);
        assert_eq!(n.circumference(), 24.0);
        assert_eq!(n.width(), 6);
        assert_eq!(n.height(), 6);
    }

    #[test]
    fn intersects_overlaps_matrices() {
        let a = b(0, 0, 10, 10);
        let overlap = b(5, 5, 15, 15);
        let touch = b(10, 0, 20, 10);
        let disjoint = b(11, 0, 20, 10);
        let line = b(10, 0, 10, 10);

        assert!(a.intersects(&overlap));
        assert!(a.intersects(&touch)); // touching counts
        assert!(a.intersects(&a));
        assert!(!a.intersects(&disjoint));
        assert!(!a.intersects(&IntBox::EMPTY));
        assert!(!IntBox::EMPTY.intersects(&a));

        assert!(a.overlaps(&overlap));
        assert!(!a.overlaps(&touch)); // zero-width intersection
        assert!(!a.overlaps(&line));
        assert!(!a.overlaps(&disjoint));
        assert!(!a.overlaps(&IntBox::EMPTY));
    }

    #[test]
    fn containment_matrices() {
        let a = b(0, 0, 10, 10);
        assert!(a.contains_inside(&p(5, 5)));
        assert!(!a.contains_inside(&p(0, 0))); // border does not count
        assert!(!a.contains_inside(&p(10, 10)));

        assert!(a.contains_in_interior(&b(2, 3, 8, 7)));
        assert!(!a.contains_in_interior(&a)); // border does not count
        assert!(a.contains_in_interior(&IntBox::EMPTY));

        assert!(a.is_contained_in(&b(-1, -1, 11, 11)));
        assert!(a.is_contained_in(&a));
        assert!(b(10, 0, 20, 10).is_contained_in(&b(0, 0, 20, 10))); // touching border
        assert!(!a.is_contained_in(&IntBox::EMPTY));
        assert!(IntBox::EMPTY.is_contained_in(&a)); // empty shortcut
    }

    /// jshell pin: A=(0,0,10,10) vs B=(2,3,8,7) is onTheRight on all
    /// four edges; symmetric case is onTheLeft; equal boxes collinear.
    #[test]
    fn compare_edge_order() {
        let a = b(0, 0, 10, 10);
        let inner = b(2, 3, 8, 7);
        for edge in 0..4 {
            assert_eq!(a.compare(&inner, edge), Side::Negative);
            assert_eq!(inner.compare(&a, edge), Side::Positive);
            assert_eq!(a.compare(&a, edge), Side::Collinear);
        }
        // per-edge discrimination
        let lower = b(0, 1, 10, 10);
        assert_eq!(b(0, 2, 10, 10).compare(&lower, 0), Side::Positive);
        let righter = b(0, 0, 11, 10);
        assert_eq!(b(0, 0, 10, 10).compare(&righter, 1), Side::Positive);
        let higher = b(0, 0, 10, 11);
        assert_eq!(b(0, 0, 10, 10).compare(&higher, 2), Side::Positive);
        let lefter = b(-1, 0, 10, 10);
        assert_eq!(b(0, 0, 10, 10).compare(&lefter, 3), Side::Positive);
    }

    #[test]
    #[should_panic(expected = "IntBox.compare: edgeIndex out of range")]
    fn compare_panics_on_edge_out_of_range() {
        let _ = b(0, 0, 1, 1).compare(&b(0, 0, 1, 1), 4);
    }

    #[test]
    fn corners_in_regular_tile_shape_order() {
        let a = b(0, 0, 10, 10);
        assert_eq!(a.corner(0), p(0, 0));
        assert_eq!(a.corner(1), p(10, 0));
        assert_eq!(a.corner(2), p(10, 10));
        assert_eq!(a.corner(3), p(0, 10));
    }

    #[test]
    #[should_panic(expected = "IntBox.corner: no out of range")]
    fn corner_panics_on_index_out_of_range() {
        let _ = b(0, 0, 1, 1).corner(4);
    }

    /// jshell pins: turn90Degree(+1) around (0,0) of (0,0,4,2) gives
    /// (-2,0,0,4); turn90Degree(-1) around (1,1) gives (0,-2,2,2).
    #[test]
    fn turn_90_degree_min_max_normalization() {
        assert_eq!(b(0, 0, 4, 2).turn_90_degree(1, &p(0, 0)), b(-2, 0, 0, 4));
        assert_eq!(b(0, 0, 4, 2).turn_90_degree(-1, &p(1, 1)), b(0, -2, 2, 2));
        assert_eq!(b(0, 0, 4, 2).turn_90_degree(4, &p(3, 7)), b(0, 0, 4, 2));
        assert_eq!(b(1, 2, 3, 4).turn_90_degree(2, &p(0, 0)), b(-3, -4, -1, -2));
    }

    #[test]
    fn translate_by_int_vector() {
        let a = b(0, 0, 4, 2);
        assert_eq!(
            a.translate_by(&Vector::Int(IntVector::new(1, 2))),
            b(1, 2, 5, 4)
        );
        assert_eq!(a.translate_by(&Vector::ZERO), a);
    }

    #[test]
    #[should_panic(expected = "expected an int vector")]
    fn translate_by_panics_on_rational_vector() {
        let rational = Vector::rational(RationalVector::new(
            BigInt::from(1),
            BigInt::from(0),
            BigInt::from(3),
        ));
        let _ = b(0, 0, 4, 2).translate_by(&rational);
    }

    #[test]
    fn nearest_point_and_distance() {
        let a = b(0, 0, 10, 10);
        let nearest = a.nearest_point(&FloatPoint::new(15.0, 3.0));
        assert_eq!(nearest, FloatPoint::new(10.0, 3.0));
        assert_eq!(a.distance(&FloatPoint::new(15.0, 3.0)), 5.0);
        let corner_case = a.nearest_point(&FloatPoint::new(-3.0, -4.0));
        assert_eq!(corner_case, FloatPoint::new(0.0, 0.0));
        assert_eq!(a.distance(&FloatPoint::new(-3.0, -4.0)), 5.0);
        let inside = a.nearest_point(&FloatPoint::new(2.5, 7.25));
        assert_eq!(inside, FloatPoint::new(2.5, 7.25));
        assert_eq!(a.distance(&FloatPoint::new(2.5, 7.25)), 0.0);
    }

    /// jshell pins: (1,1) in (0,0,10,10) -> [(0,1),(1,0)];
    /// (5,5) -> [(0,5),(10,5)].
    #[test]
    fn nearest_border_projections() {
        let a = b(0, 0, 10, 10);
        assert_eq!(
            a.nearest_border_projections(&p(1, 1), 2),
            vec![p(0, 1), p(1, 0)]
        );
        assert_eq!(
            a.nearest_border_projections(&p(5, 5), 5),
            vec![p(0, 5), p(10, 5)]
        );
        // jshell pin: for (9,2) the upper border loses to the lower one
        assert_eq!(
            a.nearest_border_projections(&p(9, 2), 2),
            vec![p(10, 2), p(9, 0)]
        );
        assert_eq!(a.nearest_border_projections(&p(5, 5), 1), vec![p(0, 5)]);
        assert!(a.nearest_border_projections(&p(5, 5), 0).is_empty());
        assert!(a.nearest_border_projections(&p(5, 5), -3).is_empty());
    }

    /// jshell pins: 20.0 and sqrt(1300) = 36.05551275463989.
    #[test]
    fn weighted_distance_branches() {
        let a = b(0, 0, 10, 10);
        assert_eq!(a.weighted_distance(&b(20, 0, 30, 10), 2.0, 3.0), 20.0);
        assert_eq!(
            a.weighted_distance(&b(20, 20, 30, 30), 2.0, 3.0),
            36.055_512_754_639_89
        );
        assert_eq!(
            a.weighted_distance(&b(20, 20, 30, 30), 2.0, 3.0),
            1300f64.sqrt()
        );
        assert_eq!(a.weighted_distance(&b(2, 2, 8, 8), 2.0, 3.0), 0.0);
    }

    /// jshell pins: shrink(3) of (0,0,10,10) -> (3,3,7,7); shrink(6)
    /// collapses to the center (5,5,5,5).
    #[test]
    fn shrink_never_vanishes() {
        let a = b(0, 0, 10, 10);
        assert_eq!(a.shrink(3), b(3, 3, 7, 7));
        assert_eq!(a.shrink(6), b(5, 5, 5, 5));
        assert_eq!(a.shrink(0), a);
    }

    /// jshell pin: nearestPart((20,2,30,8)) from (0,0,10,10) clamps to
    /// the degenerate box (20,2,20,8), which is NOT empty.
    #[test]
    fn nearest_part_clamps() {
        let a = b(0, 0, 10, 10);
        let result = a.nearest_part(&b(20, 2, 30, 8));
        assert_eq!(result, b(20, 2, 20, 8));
        assert!(!result.is_empty());
        assert_eq!(a.nearest_part(&b(5, 2, 15, 8)), b(5, 2, 10, 8));
        assert_eq!(a.nearest_part(&b(1, 1, 2, 2)), b(1, 1, 2, 2));
        assert_eq!(a.nearest_part(&a), a);
    }

    /// jshell pins: divideIntoSections(4) of (0,0,10,10) yields 9
    /// sections of size 4 (last row/column clipped).
    #[test]
    fn divide_into_sections_layout() {
        let sections = b(0, 0, 10, 10).divide_into_sections(4.0);
        assert_eq!(sections.len(), 9);
        assert_eq!(sections[0], b(0, 0, 4, 4));
        assert_eq!(sections[2], b(8, 0, 10, 4));
        assert_eq!(sections[4], b(4, 4, 8, 8));
        assert_eq!(sections[8], b(8, 8, 10, 10));
        // degenerate inputs
        assert!(b(0, 0, 10, 10).divide_into_sections(0.0).is_empty());
        assert!(b(0, 0, 10, 10).divide_into_sections(-1.0).is_empty());
        assert_eq!(
            b(0, 0, 10, 10).divide_into_sections(10.0),
            vec![b(0, 0, 10, 10)]
        );
        assert_eq!(
            b(0, 0, 10, 10).divide_into_sections(100.0),
            vec![b(0, 0, 10, 10)]
        );
    }

    /// A wrapped (negative) extension makes Java throw
    /// NegativeArraySizeException at `new IntBox[xcount * ycount]`
    /// (IntBox.java:660); the port asserts instead of silently
    /// returning an empty/partial result.
    #[test]
    #[should_panic(expected = "IntBox.divideIntoSections: negative section count")]
    fn divide_into_sections_panics_on_wrapped_extension() {
        let _ = b(-2_000_000_000, 0, 2_000_000_000, 10).divide_into_sections(100.0);
    }

    /// jshell pins via the public `cutout` path (all pieces IntBox):
    /// no-switch layout, all four switch branches, the border-overlap
    /// early return and the empty early return.
    #[test]
    fn cutout_from_piece_layout_and_switch_optimization() {
        let d = b(0, 0, 10, 10);
        // no switch: c == d == (4,1,8,8)
        assert_eq!(
            b(0, 0, 10, 10).cutout_from(&b(4, 1, 8, 8)),
            vec![b(4, 1, 8, 1), b(4, 1, 4, 8), b(8, 1, 8, 8), b(4, 8, 8, 8)]
        );
        // switch 1 fires (left margin > lower margin)
        assert_eq!(
            b(4, 1, 8, 8).cutout_from(&d),
            vec![
                b(4, 0, 8, 1),
                b(0, 0, 4, 10),
                b(8, 0, 10, 8),
                b(4, 8, 10, 10)
            ]
        );
        // switch 2 fires (upper margin > left margin)
        assert_eq!(
            b(4, 1, 8, 5).cutout_from(&d),
            vec![
                b(4, 0, 8, 1),
                b(0, 0, 4, 5),
                b(8, 0, 10, 5),
                b(0, 5, 10, 10)
            ]
        );
        // switch 3 fires (right margin > upper margin)
        assert_eq!(
            b(4, 1, 5, 6).cutout_from(&d),
            vec![
                b(4, 0, 5, 1),
                b(0, 0, 4, 10),
                b(5, 0, 10, 10),
                b(4, 6, 5, 10)
            ]
        );
        // switch 4 fires (lower margin > right margin)
        assert_eq!(
            b(4, 4, 8, 8).cutout_from(&d),
            vec![
                b(0, 0, 10, 4),
                b(0, 4, 4, 10),
                b(8, 4, 10, 8),
                b(4, 8, 10, 10)
            ]
        );
        // border overlap only (dimension drop of the receiver's c):
        // jshell pin via cutout: (0,0,20,20).cutoutFrom((0,0,10,0)) is
        // the single piece d = (0,0,10,0)
        assert_eq!(
            b(0, 0, 20, 20).cutout_from(&b(0, 0, 10, 0)),
            vec![b(0, 0, 10, 0)]
        );
        // same shapes, opposite argument order: no dimension drop, so
        // the 4-piece split runs with degenerate pieces (Java-verified
        // by hand-tracing the same arithmetic)
        assert_eq!(
            b(0, 0, 10, 0).cutout_from(&b(0, 0, 20, 20)),
            vec![
                b(0, 0, 10, 0),
                b(0, 0, 0, 0),
                b(10, 0, 20, 0),
                b(0, 0, 20, 20)
            ]
        );
        // empty cutter: single piece d
        assert_eq!(
            IntBox::EMPTY.cutout_from(&b(0, 0, 20, 20)),
            vec![b(0, 0, 20, 20)]
        );
    }

    #[test]
    fn trivial_flags_and_bounding_tiles() {
        let a = b(0, 0, 10, 10);
        assert!(a.is_int_box());
        assert!(a.is_int_octagon());
        assert_eq!(a.border_line_count(), 4);
        assert!(a.is_bounded());
        assert!(a.corner_is_bounded(0));
        // Java returns true without validating the index
        assert!(a.corner_is_bounded(99));
        assert_eq!(a.bounding_box(), a);
        assert_eq!(a.bounding_tile(), a);
    }

    /// jshell pin: getId of (1,2,3,4) is 1120 = 31*(31*1+2) + (31*3+4).
    #[test]
    fn get_id_combines_corners() {
        assert_eq!(b(1, 2, 3, 4).get_id(), 1120);
        assert_eq!(b(0, 0, 0, 0).get_id(), 0);
    }

    // --- Task 4 deferrals landed on the point family ---

    #[test]
    fn int_point_surrounding_box_and_is_contained_in() {
        let point = p(3, -4);
        assert_eq!(point.surrounding_box(), b(3, -4, 3, -4));
        assert!(!point.surrounding_box().is_empty());
        assert!(point.is_contained_in(&b(0, -10, 10, 10)));
        assert!(!point.is_contained_in(&b(0, 0, 10, 10)));
        assert!(point.is_contained_in(&b(3, -4, 3, -4))); // on the border
    }

    #[test]
    fn float_point_bounding_box_floors_and_ceils() {
        assert_eq!(FloatPoint::new(2.5, -3.5).bounding_box(), b(2, -4, 3, -3));
        assert_eq!(FloatPoint::new(-0.5, -0.5).bounding_box(), b(-1, -1, 0, 0));
        assert_eq!(FloatPoint::new(3.0, 7.0).bounding_box(), b(3, 7, 3, 7));
    }

    #[test]
    fn rational_point_surrounding_box_and_is_contained_in() {
        let half = RationalPoint::new(BigInt::from(1), BigInt::from(1), BigInt::from(2));
        assert_eq!(half.surrounding_box(), b(0, 0, 1, 1));
        assert!(half.is_contained_in(&b(0, 0, 1, 1)));
        assert!(half.is_contained_in(&b(-5, -5, 5, 5)));
        assert!(!half.is_contained_in(&b(2, 2, 3, 3)));
    }

    #[test]
    fn point_enum_dispatches_surrounding_box_and_is_contained_in() {
        let box_a = b(0, 0, 10, 10);
        let int_point = Point::Int(p(5, 5));
        let rational_inside = Point::Rational(Box::new(RationalPoint::new(
            BigInt::from(1),
            BigInt::from(1),
            BigInt::from(2),
        )));
        let rational_outside = Point::Rational(Box::new(RationalPoint::new(
            BigInt::from(30),
            BigInt::from(30),
            BigInt::from(2),
        )));
        assert_eq!(int_point.surrounding_box(), b(5, 5, 5, 5));
        assert_eq!(rational_inside.surrounding_box(), b(0, 0, 1, 1));
        assert_eq!(rational_outside.surrounding_box(), b(15, 15, 15, 15));
        assert!(int_point.is_contained_in(&box_a));
        assert!(rational_inside.is_contained_in(&box_a)); // (1/2, 1/2)
        assert!(!rational_outside.is_contained_in(&box_a)); // (15, 15)
    }

    // --- Task 5 closures (IntOctagon) ---

    /// Constructor-order shorthand for the octagon tests.
    #[allow(clippy::too_many_arguments)]
    fn o8(
        lx: i32,
        ly: i32,
        rx: i32,
        uy: i32,
        ulx: i32,
        lrx: i32,
        llx: i32,
        urx: i32,
    ) -> IntOctagon {
        IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx)
    }

    /// jshell pin: IntBox(0,0,10,10).toIntOctagon() = (0,0,10,10,-10,10,0,20);
    /// the wrapped box keeps exact diagonal intercepts.
    #[test]
    fn to_int_octagon_fields_pin() {
        assert_eq!(
            b(0, 0, 10, 10).to_int_octagon(),
            o8(0, 0, 10, 10, -10, 10, 0, 20)
        );
        let w = b(-2_000_000_000, -5, 2_000_000_000, 5);
        assert_eq!(
            w.to_int_octagon(),
            o8(
                -2_000_000_000,
                -5,
                2_000_000_000,
                5,
                -2_000_000_005,
                2_000_000_005,
                -2_000_000_005,
                2_000_000_005
            )
        );
        assert_eq!(w.to_int_octagon().area(), 4.0e10); // jshell pin wrapArea
    }

    /// jshell pin: IntBox(0,0,10,10).enlarge(2.5) = (-3,-3,13,13,-14,14,-4,24).
    #[test]
    fn enlarge_delegates_to_octagon_offset() {
        assert_eq!(
            b(0, 0, 10, 10).enlarge(2.5),
            o8(-3, -3, 13, 13, -14, 14, -4, 24)
        );
        assert_eq!(
            b(0, 0, 10, 10).bounding_octagon(),
            b(0, 0, 10, 10).to_int_octagon()
        );
    }

    /// jshell pins: bx.union(inner) = (0,0,10,10,-10,10,0,20);
    /// compare edges 0/4 onTheRight; isContainedIn(oct) true; intersects.
    #[test]
    fn octagon_delegation_pins() {
        let inner = o8(2, 1, 8, 7, 1, 9, 3, 15);
        let oct = b(0, 0, 10, 10).to_int_octagon();
        assert_eq!(b(0, 0, 10, 10).union_octagon(&inner), oct);
        assert_eq!(
            b(0, 0, 10, 10).intersection_octagon(&inner),
            inner.normalize()
        );
        assert!(b(0, 0, 10, 10).intersects_octagon(&inner));
        assert!(!b(20, 20, 30, 30).intersects_octagon(&oct));
        assert_eq!(b(0, 0, 10, 10).compare_octagon(&inner, 0), Side::Negative);
        assert_eq!(b(0, 0, 10, 10).compare_octagon(&inner, 4), Side::Negative);
        // jshell pins: containment is box ⊆ other; both directions on
        // identical shapes would hide a receiver inversion
        assert!(b(0, 0, 5, 5).is_contained_in_octagon(&oct));
        assert!(!b(0, 0, 10, 10).is_contained_in_octagon(&o8(0, 0, 5, 5, -5, 5, 0, 10)));
        assert!(!oct.is_contained_in(&inner));
    }

    /// jshell pin (cutO): bx.cutoutFrom(oct) equals oct.cutoutFrom(inner)
    /// here, because the box's octagon conversion IS `oct` itself; 8
    /// pieces, last one (2,1,8,7,1,1,3,15).
    #[test]
    fn cutout_from_octagon_delegates() {
        let inner = o8(2, 1, 8, 7, 1, 9, 3, 15);
        let oct = b(0, 0, 10, 10).to_int_octagon();
        let pieces = b(0, 0, 10, 10).cutout_from_octagon(&inner);
        assert_eq!(pieces, oct.cutout_from(&inner));
        assert_eq!(pieces.len(), 8);
        assert_eq!(pieces[7], o8(2, 1, 8, 7, 1, 1, 3, 15));
        // jshell pin: an EMPTY argument is returned as the single piece —
        // cutoutFrom returns pieces OF d, so here [d] = [EMPTY]
        assert_eq!(
            b(0, 0, 10, 10).cutout_from_octagon(&IntOctagon::EMPTY),
            vec![IntOctagon::EMPTY]
        );
    }

    /// jshell pin `PB1`: `IntBox(0,0,10,10).toSimplex()` has the 4 border
    /// lines in box-edge order (bottom E, right N, top W, left S).
    #[test]
    fn pb1_to_simplex_border_lines_pin() {
        let l = |ax: i32, ay: i32, bx: i32, by: i32| {
            crate::line::Line::new(
                crate::point::Point::int(crate::int_point::IntPoint::new(ax, ay)),
                crate::point::Point::int(crate::int_point::IntPoint::new(bx, by)),
            )
        };
        let simplex = b(0, 0, 10, 10).to_simplex();
        assert_eq!(simplex.border_line_count(), 4);
        assert_eq!(simplex.border_line(0), l(0, 0, 1, 0));
        assert_eq!(simplex.border_line(1), l(10, 10, 10, 11));
        assert_eq!(simplex.border_line(2), l(10, 10, 9, 10));
        assert_eq!(simplex.border_line(3), l(0, 0, 0, -1));
    }
}
