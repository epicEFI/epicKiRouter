//! Port of Java `app.freerouting.geometry.planar.IntOctagon` — the router's
//! hottest shape. An octagon with integer coordinates whose four diagonal
//! borders sit at exactly ±45°; each diagonal is stored as the x-axis
//! intercept of its border line.
//!
//! Field order follows the Java constructor signature
//! `(leftX, bottomY, rightX, topY, upperLeftDiagonalX, lowerRightDiagonalX,
//! lowerLeftDiagonalX, upperRightDiagonalX)`; the eight borders are numbered
//! counterclockwise: 0 lower, 1 lower-right diagonal, 2 right, 3 upper-right
//! diagonal, 4 upper, 5 upper-left diagonal, 6 left, 7 lower-left diagonal.
//!
//! Empty semantics (plan note T11): Java's [`IntOctagon::is_empty`] is
//! `this == EMPTY` — **reference identity** against one shared sentinel
//! object produced by `normalize()`. Rust has no reference identity here, so
//! the port uses **field equality with the sentinel tuple**. This is
//! equivalent for every value Java can produce: `normalize()` returns the
//! shared `EMPTY` constant on emptiness, so all canonical empty octagons are
//! reference-identical in Java and sentinel-equal here. The one behavioral
//! difference: a hand-constructed octagon that happens to equal the sentinel
//! fields is empty in Rust but would not be in Java. Do not "fix" this by
//! adding structural emptiness checks (e.g. `left_x > right_x`) — an
//! inverted, non-sentinel octagon is **not** empty, unlike [`IntBox`].
//!
//! `normalize()` (plan note T7) is the single most order-sensitive function
//! in the kernel: twelve sequential tightening steps whose later steps read
//! results of earlier ones, followed by four ceil/floor diagonal-merge
//! steps. It is ported step-for-step in Java order; do not reorder, merge,
//! or "fix" redundant-looking conditions.
//!
//! Wrapping: every `i32` subexpression uses Java `int` arithmetic via
//! `wrapping_*` and may wrap (sentinel-scale coordinates are legal inputs).
//! The `f64` divisions by `2.0` in `normalize`/`border_point` are Java's
//! int-to-double promotions; `.ceil() as i32` / `.floor() as i32` saturate
//! exactly like JLS 5.1.3 double→int narrowing (plan note T15). The
//! `java_round(...) as i32` sites are i64→i32 narrowings and wrap mod 2^32
//! like Java's `(int)` long cast.
//!
//! Deferral ledger closure (Task 7): `borderLine`, `borderLineIndex`
//! (warn-and-(-1)), `toSimplex` (fresh computation, T14 — Java memoizes
//! `precalculatedToSimplex`; pin PB2), `intersects(Simplex)` ==
//! `other.intersects(this)`, `cutoutFrom(Simplex)` ==
//! `toSimplex().cutoutFrom(simplex)`, `simplify`, and `turn90Degree`
//! (inherited TileShape.java:669 border-line rotation) all landed. The
//! TileShape/trait-dispatch surface (`intersection(TileShape)`,
//! `intersects(Shape)`, `contains(RegularTileShape)`, `union`,
//! `boundingShape`, `compare(RegularTileShape, int)`,
//! `cutout(TileShape)`) lives on the `RegularTileShape` / `TileShape`
//! enums. `intersection(Simplex)` is the IntOctagon.java:373 mirror
//! `other.intersection(this)`.
//! - Closed in Task 8: `boolean intersects(Circle)` == `intersects_circle`
//!   (concrete `TileShape.distance` centre check against the radius).

use crate::circle::Circle;
use crate::float_point::FloatPoint;
use crate::fortyfive_degree_direction::FortyfiveDegreeDirection;
use crate::int_box::IntBox;
use crate::int_point::IntPoint;
use crate::limits::{CRIT_INT, SQRT2};
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::rounding::java_round;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Implementation of an octagon in the plane with 45 degree angle
/// constraints and integer coordinates (Java `IntOctagon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntOctagon {
    /// X-coordinate of the left vertical border.
    pub left_x: i32,
    /// Y-coordinate of the bottom horizontal border.
    pub bottom_y: i32,
    /// X-coordinate of the right vertical border.
    pub right_x: i32,
    /// Y-coordinate of the top horizontal border.
    pub top_y: i32,
    /// X-axis intersection of the upper-left diagonal border (-45°).
    pub upper_left_diagonal_x: i32,
    /// X-axis intersection of the lower-right diagonal border (-45°).
    pub lower_right_diagonal_x: i32,
    /// X-axis intersection of the lower-left diagonal border (+45°).
    pub lower_left_diagonal_x: i32,
    /// X-axis intersection of the upper-right diagonal border (+45°).
    pub upper_right_diagonal_x: i32,
}

impl IntOctagon {
    /// Reusable instance of an empty octagon (Java `IntOctagon.EMPTY`).
    /// See the module docs for the identity-vs-value emptiness subtlety.
    pub const EMPTY: IntOctagon = IntOctagon {
        left_x: CRIT_INT,
        bottom_y: CRIT_INT,
        right_x: -CRIT_INT,
        top_y: -CRIT_INT,
        upper_left_diagonal_x: CRIT_INT,
        lower_right_diagonal_x: -CRIT_INT,
        lower_left_diagonal_x: CRIT_INT,
        upper_right_diagonal_x: -CRIT_INT,
    };

    /// Creates an IntOctagon from 8 integer boundary values, in the Java
    /// constructor order (diagonal values are x-axis intercepts).
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        left_x: i32,
        bottom_y: i32,
        right_x: i32,
        top_y: i32,
        upper_left_diagonal_x: i32,
        lower_right_diagonal_x: i32,
        lower_left_diagonal_x: i32,
        upper_right_diagonal_x: i32,
    ) -> IntOctagon {
        IntOctagon {
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        }
    }

    /// True if this is the empty octagon. Java compares reference identity
    /// (`this == EMPTY`); here, field equality with the sentinel — see the
    /// module docs.
    pub const fn is_empty(&self) -> bool {
        // Field-by-field comparison against the sentinel (the derived
        // PartialEq is not const-callable); see the module docs for the
        // Java reference-identity semantics this value check mirrors.
        self.left_x == CRIT_INT
            && self.bottom_y == CRIT_INT
            && self.right_x == -CRIT_INT
            && self.top_y == -CRIT_INT
            && self.upper_left_diagonal_x == CRIT_INT
            && self.lower_right_diagonal_x == -CRIT_INT
            && self.lower_left_diagonal_x == CRIT_INT
            && self.upper_right_diagonal_x == -CRIT_INT
    }

    /// Java `isIntOctagon`: every IntOctagon is an IntOctagon.
    pub const fn is_int_octagon(&self) -> bool {
        true
    }

    /// Java `isBounded`.
    pub const fn is_bounded(&self) -> bool {
        true
    }

    /// Java `cornerIsBounded`: true for every index (the Java original does
    /// not validate `no`).
    pub const fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    /// Java `boundingBox`: the axis bounds as an [`IntBox`].
    pub const fn bounding_box(&self) -> IntBox {
        IntBox::from_corners(self.left_x, self.bottom_y, self.right_x, self.top_y)
    }

    /// Java `boundingOctagon`: an IntOctagon is its own bounding octagon.
    pub const fn bounding_octagon(&self) -> IntOctagon {
        *self
    }

    /// Java `boundingTile`: an IntOctagon is its own bounding tile.
    pub const fn bounding_tile(&self) -> IntOctagon {
        *self
    }

    /// Java `dimension`: -1 for the empty octagon, 0 for a point, 1 for a
    /// segment, 2 for a proper area.
    pub const fn dimension(&self) -> i32 {
        if self.is_empty() {
            return -1;
        }
        if self.right_x > self.left_x
            && self.top_y > self.bottom_y
            && self.lower_right_diagonal_x > self.upper_left_diagonal_x
            && self.upper_right_diagonal_x > self.lower_left_diagonal_x
        {
            return 2;
        }
        if self.right_x == self.left_x && self.top_y == self.bottom_y {
            return 0;
        }
        1
    }

    /// Returns the corner with index `no` in counterclockwise order starting
    /// from the lower-left corner of the bottom border. Panics (Java:
    /// `IllegalArgumentException`) for `no` out of range.
    pub fn corner(&self, no: i32) -> IntPoint {
        match no {
            0 => IntPoint::new(
                self.lower_left_diagonal_x.wrapping_sub(self.bottom_y),
                self.bottom_y,
            ), // lower-left (bottom horizontal)
            1 => IntPoint::new(
                self.lower_right_diagonal_x.wrapping_add(self.bottom_y),
                self.bottom_y,
            ), // lower-right (bottom horizontal)
            2 => IntPoint::new(
                self.right_x,
                self.right_x.wrapping_sub(self.lower_right_diagonal_x),
            ), // bottom-right vertical
            3 => IntPoint::new(
                self.right_x,
                self.upper_right_diagonal_x.wrapping_sub(self.right_x),
            ), // top-right vertical
            4 => IntPoint::new(
                self.upper_right_diagonal_x.wrapping_sub(self.top_y),
                self.top_y,
            ), // upper-right (top horizontal)
            5 => IntPoint::new(
                self.upper_left_diagonal_x.wrapping_add(self.top_y),
                self.top_y,
            ), // upper-left (top horizontal)
            6 => IntPoint::new(
                self.left_x,
                self.left_x.wrapping_sub(self.upper_left_diagonal_x),
            ), // top-left vertical
            7 => IntPoint::new(
                self.left_x,
                self.lower_left_diagonal_x.wrapping_sub(self.left_x),
            ), // bottom-left vertical
            _ => panic!("IntOctagon.corner: no out of range"),
        }
    }

    /// Java `cornerY`: the y value of corner `no` without allocating an
    /// [`IntPoint`]. Panics (Java: `IllegalArgumentException`) for `no` out
    /// of range.
    pub const fn corner_y(&self, no: i32) -> i32 {
        match no {
            0 | 1 => self.bottom_y,
            2 => self.right_x.wrapping_sub(self.lower_right_diagonal_x),
            3 => self.upper_right_diagonal_x.wrapping_sub(self.right_x),
            4 | 5 => self.top_y,
            6 => self.left_x.wrapping_sub(self.upper_left_diagonal_x),
            7 => self.lower_left_diagonal_x.wrapping_sub(self.left_x),
            _ => panic!("IntOctagon.corner: no out of range"),
        }
    }

    /// Java `cornerX`: the x value of corner `no` without allocating an
    /// [`IntPoint`]. Panics (Java: `IllegalArgumentException`) for `no` out
    /// of range.
    pub const fn corner_x(&self, no: i32) -> i32 {
        match no {
            0 => self.lower_left_diagonal_x.wrapping_sub(self.bottom_y),
            1 => self.lower_right_diagonal_x.wrapping_add(self.bottom_y),
            2 | 3 => self.right_x,
            4 => self.upper_right_diagonal_x.wrapping_sub(self.top_y),
            5 => self.upper_left_diagonal_x.wrapping_add(self.top_y),
            6 | 7 => self.left_x,
            _ => panic!("IntOctagon.corner: no out of range"),
        }
    }

    /// Returns a stable identifier for this octagon (Java `getId`; note the
    /// field order differs from the constructor order).
    pub fn get_id(&self) -> i32 {
        let mut result = self.left_x;
        result = 31i32.wrapping_mul(result).wrapping_add(self.right_x);
        result = 31i32.wrapping_mul(result).wrapping_add(self.bottom_y);
        result = 31i32.wrapping_mul(result).wrapping_add(self.top_y);
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.lower_left_diagonal_x);
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.upper_right_diagonal_x);
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.upper_left_diagonal_x);
        31i32
            .wrapping_mul(result)
            .wrapping_add(self.lower_right_diagonal_x)
    }

    /// Java `area`: half of the absolute shoelace sum, specialized to avoid
    /// point allocation. Every parenthesized subexpression wraps in `i32`
    /// before its `f64` cast, and the six products accumulate in the same
    /// order as the Java `+=` sequence. On the empty sentinel the result is
    /// garbage — reproduced exactly (2^51; see the test pin).
    pub fn area(&self) -> f64 {
        // calculate half of the absolute value of
        // x0 (y1 - y7) + x1 (y2 - y0) + x2 (y3 - y1) + ...+ x7( y0 - y6)
        // where xi, yi are the coordinates of the i-th corner of this
        // Octagon.
        let mut result = f64::from(self.lower_left_diagonal_x.wrapping_sub(self.bottom_y))
            * f64::from(
                self.bottom_y
                    .wrapping_sub(self.lower_left_diagonal_x)
                    .wrapping_add(self.left_x),
            );
        result += f64::from(self.lower_right_diagonal_x.wrapping_add(self.bottom_y))
            * f64::from(
                self.right_x
                    .wrapping_sub(self.lower_right_diagonal_x)
                    .wrapping_sub(self.bottom_y),
            );
        result += f64::from(self.right_x)
            * f64::from(
                self.upper_right_diagonal_x
                    .wrapping_sub(2i32.wrapping_mul(self.right_x))
                    .wrapping_sub(self.bottom_y)
                    .wrapping_add(self.top_y)
                    .wrapping_add(self.lower_right_diagonal_x),
            );
        result += f64::from(self.upper_right_diagonal_x.wrapping_sub(self.top_y))
            * f64::from(
                self.top_y
                    .wrapping_sub(self.upper_right_diagonal_x)
                    .wrapping_add(self.right_x),
            );
        result += f64::from(self.upper_left_diagonal_x.wrapping_add(self.top_y))
            * f64::from(
                self.left_x
                    .wrapping_sub(self.upper_left_diagonal_x)
                    .wrapping_sub(self.top_y),
            );
        result += f64::from(self.left_x)
            * f64::from(
                self.lower_left_diagonal_x
                    .wrapping_sub(2i32.wrapping_mul(self.left_x))
                    .wrapping_sub(self.top_y)
                    .wrapping_add(self.bottom_y)
                    .wrapping_add(self.upper_left_diagonal_x),
            );
        0.5 * result.abs()
    }

    /// Java `borderLineCount`.
    pub const fn border_line_count(&self) -> i32 {
        8
    }

    /// Java `maxWidth`: the int subtractions wrap before the double widening.
    pub fn max_width(&self) -> f64 {
        let width1 = f64::from(self.right_x.wrapping_sub(self.left_x))
            .max(f64::from(self.top_y.wrapping_sub(self.bottom_y)));
        let width2 = f64::from(
            self.upper_right_diagonal_x
                .wrapping_sub(self.lower_left_diagonal_x),
        )
        .max(f64::from(
            self.lower_right_diagonal_x
                .wrapping_sub(self.upper_left_diagonal_x),
        ));
        width1.max(width2 / SQRT2)
    }

    /// Java `minWidth`: the int subtractions wrap before the double widening.
    pub fn min_width(&self) -> f64 {
        let width1 = f64::from(self.right_x.wrapping_sub(self.left_x))
            .min(f64::from(self.top_y.wrapping_sub(self.bottom_y)));
        let width2 = f64::from(
            self.upper_right_diagonal_x
                .wrapping_sub(self.lower_left_diagonal_x),
        )
        .min(f64::from(
            self.lower_right_diagonal_x
                .wrapping_sub(self.upper_left_diagonal_x),
        ));
        width1.min(width2 / SQRT2)
    }

    /// Returns the octagon offsetted by `distance`. If `distance > 0`, the
    /// offset is to the outside, else to the inside. Rounding is Java
    /// `Math.round` (ties toward +infinity) narrowed by the `(int)` long
    /// cast (truncation mod 2^32); the result is normalized.
    pub fn offset(&self, distance: f64) -> IntOctagon {
        let width = java_round(distance) as i32;
        if width == 0 {
            return *self;
        }
        let dia_width = java_round(SQRT2 * distance) as i32;
        IntOctagon::new(
            self.left_x.wrapping_sub(width),
            self.bottom_y.wrapping_sub(width),
            self.right_x.wrapping_add(width),
            self.top_y.wrapping_add(width),
            self.upper_left_diagonal_x.wrapping_sub(dia_width),
            self.lower_right_diagonal_x.wrapping_add(dia_width),
            self.lower_left_diagonal_x.wrapping_sub(dia_width),
            self.upper_right_diagonal_x.wrapping_add(dia_width),
        )
        .normalize()
    }

    /// Java `enlarge`: identical to [`IntOctagon::offset`].
    pub fn enlarge(&self, offset: f64) -> IntOctagon {
        self.offset(offset)
    }

    /// Returns true if `point` is contained in this octagon. Because of the
    /// parameter type [`FloatPoint`], the function may not be exact close to
    /// the border.
    pub fn contains(&self, point: &FloatPoint) -> bool {
        if f64::from(self.left_x) > point.x
            || f64::from(self.bottom_y) > point.y
            || f64::from(self.right_x) < point.x
            || f64::from(self.top_y) < point.y
        {
            return false;
        }
        let tmp1 = point.x - point.y;
        let tmp2 = point.x + point.y;
        f64::from(self.upper_left_diagonal_x) <= tmp1
            && f64::from(self.lower_right_diagonal_x) >= tmp1
            && f64::from(self.lower_left_diagonal_x) <= tmp2
            && f64::from(self.upper_right_diagonal_x) >= tmp2
    }

    /// Returns the smallest octagon containing both octagons (Java
    /// `union(IntOctagon)`); the raw min/max bounds are *not* normalized,
    /// exactly like the Java original.
    pub fn union(&self, other: &IntOctagon) -> IntOctagon {
        IntOctagon::new(
            self.left_x.min(other.left_x),
            self.bottom_y.min(other.bottom_y),
            self.right_x.max(other.right_x),
            self.top_y.max(other.top_y),
            self.upper_left_diagonal_x.min(other.upper_left_diagonal_x),
            self.lower_right_diagonal_x
                .max(other.lower_right_diagonal_x),
            self.lower_left_diagonal_x.min(other.lower_left_diagonal_x),
            self.upper_right_diagonal_x
                .max(other.upper_right_diagonal_x),
        )
    }

    /// Java `union(IntBox)` (package-private `union(IntBox)` on the octagon
    /// side is reached via `IntBox.union(IntOctagon)`).
    pub fn union_box(&self, other: &IntBox) -> IntOctagon {
        self.union(&other.to_int_octagon())
    }

    /// Returns the intersection of two octagons: the raw componentwise
    /// max/min bounds, then normalized — which yields exactly the
    /// [`IntOctagon::EMPTY`] sentinel on disjointness.
    pub fn intersection(&self, other: &IntOctagon) -> IntOctagon {
        IntOctagon::new(
            self.left_x.max(other.left_x),
            self.bottom_y.max(other.bottom_y),
            self.right_x.min(other.right_x),
            self.top_y.min(other.top_y),
            self.upper_left_diagonal_x.max(other.upper_left_diagonal_x),
            self.lower_right_diagonal_x
                .min(other.lower_right_diagonal_x),
            self.lower_left_diagonal_x.max(other.lower_left_diagonal_x),
            self.upper_right_diagonal_x
                .min(other.upper_right_diagonal_x),
        )
        .normalize()
    }

    /// Java package-private `intersection(IntBox)`.
    pub fn intersection_box(&self, other: &IntBox) -> IntOctagon {
        self.intersection(&other.to_int_octagon())
    }

    /// Returns an equivalent octagon with all redundant bounds tightened
    /// (Java `normalize`). Twelve sequential tightening steps — later steps
    /// read earlier results — followed by four ceil/floor diagonal-merge
    /// steps, then the final emptiness check. Ported step-for-step in Java
    /// order (IntOctagon.java:399-541); this is the single most
    /// order-sensitive function in the kernel.
    pub fn normalize(&self) -> IntOctagon {
        if self.left_x > self.right_x
            || self.bottom_y > self.top_y
            || self.lower_left_diagonal_x > self.upper_right_diagonal_x
            || self.upper_left_diagonal_x > self.lower_right_diagonal_x
        {
            return IntOctagon::EMPTY;
        }
        let mut new_lx = self.left_x;
        let mut new_rx = self.right_x;
        let mut new_ly = self.bottom_y;
        let mut new_uy = self.top_y;
        let mut new_llx = self.lower_left_diagonal_x;
        let mut new_ulx = self.upper_left_diagonal_x;
        let mut new_lrx = self.lower_right_diagonal_x;
        let mut new_urx = self.upper_right_diagonal_x;

        if new_lx < new_llx.wrapping_sub(new_uy) {
            // the point newLx, newUy is the lower left border line of
            // this octagon
            // change newLx , that the lower left border line runs through
            // this point
            new_lx = new_llx.wrapping_sub(new_uy);
        }

        if new_lx < new_ulx.wrapping_add(new_ly) {
            // the point newLx, newLy is above the upper left border line of
            // this octagon
            // change newLx , that the upper left border line runs through
            // this point
            new_lx = new_ulx.wrapping_add(new_ly);
        }

        if new_rx > new_urx.wrapping_sub(new_ly) {
            // the point newRx, newLy is above the upper right border line of
            // this octagon
            // change newRx, that the upper right border line runs through
            // this point
            new_rx = new_urx.wrapping_sub(new_ly);
        }

        if new_rx > new_lrx.wrapping_add(new_uy) {
            // the point newRx, newUy is below the lower right border line of
            // this octagon
            // change rx , that the lower right border line runs through
            // this point
            new_rx = new_lrx.wrapping_add(new_uy);
        }

        if new_ly < new_lx.wrapping_sub(new_lrx) {
            // the point lx, ly is below the lower right border line of this
            // octagon
            // change ly, so that the lower right border line runs through
            // this point
            new_ly = new_lx.wrapping_sub(new_lrx);
        }

        if new_ly < new_llx.wrapping_sub(new_rx) {
            // the point rx, ly is below the lower left border line of
            // this octagon.
            // change ly, so that the lower left border line runs through
            // this point
            new_ly = new_llx.wrapping_sub(new_rx);
        }

        if new_uy > new_urx.wrapping_sub(new_lx) {
            // the point lx, uy is above the upper right border line of
            // this octagon.
            // Change the uy, so that the upper right border line runs through
            // this point.
            new_uy = new_urx.wrapping_sub(new_lx);
        }

        if new_uy > new_rx.wrapping_sub(new_ulx) {
            // the point rx, uy is above the upper left border line of
            // this octagon.
            // Change the uy, so that the upper left border line runs through
            // this point.
            new_uy = new_rx.wrapping_sub(new_ulx);
        }

        if new_llx.wrapping_sub(new_lx) < new_ly {
            // The point lx, ly is above the lower left border line of
            // this octagon.
            // Change the lower left line, so that it runs through this point.
            new_llx = new_lx.wrapping_add(new_ly);
        }

        if new_rx.wrapping_sub(new_lrx) < new_ly {
            // the point rx, ly is above the lower right border line of
            // this octagon.
            // Change the lower right line, so that it runs through this point.
            new_lrx = new_rx.wrapping_sub(new_ly);
        }

        if new_urx.wrapping_sub(new_rx) > new_uy {
            // the point rx, uy is below the upper right border line of oct.
            // Change the upper right line, so that it runs through this point.
            new_urx = new_uy.wrapping_add(new_rx);
        }

        if new_lx.wrapping_sub(new_ulx) > new_uy {
            // the point lx, uy is below the upper left border line of
            // this octagon.
            // Change the upper left line, so that it runs through this point.
            new_ulx = new_lx.wrapping_sub(new_uy);
        }

        let diag_upper_y = ((new_urx.wrapping_sub(new_ulx)) as f64 / 2.0).ceil() as i32;

        if new_uy > diag_upper_y {
            // the intersection of the upper right and the upper left border
            // line is below newUy.  Adjust newUy to diagUpperY.
            new_uy = diag_upper_y;
        }

        let diag_lower_y = ((new_llx.wrapping_sub(new_lrx)) as f64 / 2.0).floor() as i32;

        if new_ly < diag_lower_y {
            // the intersection of the lower right and the lower left border
            // line is above newLy.  Adjust newLy to diagLowerY.
            new_ly = diag_lower_y;
        }

        let diag_right_x = ((new_urx.wrapping_add(new_lrx)) as f64 / 2.0).ceil() as i32;

        if new_rx > diag_right_x {
            // the intersection of the upper right and the lower right border
            // line is to the left of  right x.  Adjust newRx to diagRightX.
            new_rx = diag_right_x;
        }

        let diag_left_x = ((new_llx.wrapping_add(new_ulx)) as f64 / 2.0).floor() as i32;

        if new_lx < diag_left_x {
            // the intersection of the lower left and the upper left border
            // line is to the right of left x.  Adjust newLx to diagLeftX.
            new_lx = diag_left_x;
        }
        if new_lx > new_rx || new_ly > new_uy || new_llx > new_urx || new_ulx > new_lrx {
            return IntOctagon::EMPTY;
        }
        IntOctagon::new(
            new_lx, new_ly, new_rx, new_uy, new_ulx, new_lrx, new_llx, new_urx,
        )
    }

    /// Checks, if this IntOctagon is normalized (Java `isNormalized`).
    pub fn is_normalized(&self) -> bool {
        let on = self.normalize();
        self.lower_left_diagonal_x == on.lower_left_diagonal_x
            && self.lower_right_diagonal_x == on.lower_right_diagonal_x
            && self.upper_left_diagonal_x == on.upper_left_diagonal_x
            && self.upper_right_diagonal_x == on.upper_right_diagonal_x
            && self.left_x == on.left_x
            && self.bottom_y == on.bottom_y
            && self.right_x == on.right_x
            && self.top_y == on.top_y
    }

    /// Calculates the side of the point (`x`, `y`) of the border line with
    /// index `border_line_no`. The border lines are located in
    /// counterclock sense around this octagon. An out-of-range index yields
    /// [`Side::Collinear`] — the Java original logs a warning and yields 0.
    pub fn side_of_border_line(&self, x: i32, y: i32, border_line_no: i32) -> Side {
        let tmp = match border_line_no {
            0 => self.bottom_y.wrapping_sub(y), // lower boundary line
            1 => x.wrapping_sub(y).wrapping_sub(self.lower_right_diagonal_x), // lower-right diagonal line
            2 => x.wrapping_sub(self.right_x),                                // right boundary line
            3 => x.wrapping_add(y).wrapping_sub(self.upper_right_diagonal_x), // upper-right diagonal line
            4 => y.wrapping_sub(self.top_y),                                  // upper boundary line
            5 => self.upper_left_diagonal_x.wrapping_add(y).wrapping_sub(x), // upper-left diagonal line
            6 => self.left_x.wrapping_sub(x),                                // left boundary line
            7 => self.lower_left_diagonal_x.wrapping_sub(x).wrapping_sub(y), // lower-left diagonal line
            // FRLogger.warn dropped: the observable value (0 => Collinear)
            // is preserved.
            _ => 0,
        };
        if tmp < 0 {
            Side::Positive
        } else if tmp > 0 {
            Side::Negative
        } else {
            Side::Collinear
        }
    }

    /// Checks if this normalized octagon is contained in `box` (Java
    /// `isContainedIn(IntBox)`).
    pub fn is_contained_in_box(&self, r#box: &IntBox) -> bool {
        self.left_x >= r#box.ll.x
            && self.bottom_y >= r#box.ll.y
            && self.right_x <= r#box.ur.x
            && self.top_y <= r#box.ur.y
    }

    /// Checks if this normalized octagon is contained in `other` (Java
    /// `isContainedIn(IntOctagon)`).
    pub fn is_contained_in(&self, other: &IntOctagon) -> bool {
        self.left_x >= other.left_x
            && self.bottom_y >= other.bottom_y
            && self.right_x <= other.right_x
            && self.top_y <= other.top_y
            && self.lower_left_diagonal_x >= other.lower_left_diagonal_x
            && self.upper_left_diagonal_x >= other.upper_left_diagonal_x
            && self.lower_right_diagonal_x <= other.lower_right_diagonal_x
            && self.upper_right_diagonal_x <= other.upper_right_diagonal_x
    }

    /// Checks if two normalized octagons intersect (Java
    /// `intersects(IntOctagon)`). Touching borders count.
    pub fn intersects(&self, other: &IntOctagon) -> bool {
        let is_lx = other.left_x.max(self.left_x);
        let is_rx = other.right_x.min(self.right_x);
        if is_lx > is_rx {
            return false;
        }

        let is_ly = other.bottom_y.max(self.bottom_y);
        let is_uy = other.top_y.min(self.top_y);
        if is_ly > is_uy {
            return false;
        }

        let is_llx = other.lower_left_diagonal_x.max(self.lower_left_diagonal_x);
        let is_urx = other
            .upper_right_diagonal_x
            .min(self.upper_right_diagonal_x);
        if is_llx > is_urx {
            return false;
        }

        let is_ulx = other.upper_left_diagonal_x.max(self.upper_left_diagonal_x);
        let is_lrx = other
            .lower_right_diagonal_x
            .min(self.lower_right_diagonal_x);
        is_ulx <= is_lrx
    }

    /// Java `intersects(IntBox)`.
    pub fn intersects_box(&self, other: &IntBox) -> bool {
        self.intersects(&other.to_int_octagon())
    }

    /// Returns true, if this octagon intersects with `other` and the
    /// intersection is 2-dimensional (Java `overlaps(IntOctagon)`).
    /// Touching borders do not count.
    pub fn overlaps(&self, other: &IntOctagon) -> bool {
        let is_lx = other.left_x.max(self.left_x);
        let is_rx = other.right_x.min(self.right_x);
        if is_lx >= is_rx {
            return false;
        }

        let is_ly = other.bottom_y.max(self.bottom_y);
        let is_uy = other.top_y.min(self.top_y);
        if is_ly >= is_uy {
            return false;
        }

        let is_llx = other.lower_left_diagonal_x.max(self.lower_left_diagonal_x);
        let is_urx = other
            .upper_right_diagonal_x
            .min(self.upper_right_diagonal_x);
        if is_llx >= is_urx {
            return false;
        }

        let is_ulx = other.upper_left_diagonal_x.max(self.upper_left_diagonal_x);
        let is_lrx = other
            .lower_right_diagonal_x
            .min(self.lower_right_diagonal_x);
        is_ulx < is_lrx
    }

    /// Computes the x value of the left boundary of this octagon at `y`.
    pub fn left_x_value(&self, y: i32) -> i32 {
        let result = self.left_x.max(self.upper_left_diagonal_x.wrapping_add(y));
        result.max(self.lower_left_diagonal_x.wrapping_sub(y))
    }

    /// Computes the x value of the right boundary of this octagon at `y`.
    pub fn right_x_value(&self, y: i32) -> i32 {
        let result = self
            .right_x
            .min(self.upper_right_diagonal_x.wrapping_sub(y));
        result.min(self.lower_right_diagonal_x.wrapping_add(y))
    }

    /// Computes the y value of the lower boundary of this octagon at `x`.
    pub fn lower_y_value(&self, x: i32) -> i32 {
        let result = self
            .bottom_y
            .max(self.lower_left_diagonal_x.wrapping_sub(x));
        result.max(x.wrapping_sub(self.lower_right_diagonal_x))
    }

    /// Computes the y value of the upper boundary of this octagon at `x`.
    pub fn upper_y_value(&self, x: i32) -> i32 {
        let result = self.top_y.min(x.wrapping_sub(self.upper_left_diagonal_x));
        result.min(self.upper_right_diagonal_x.wrapping_sub(x))
    }

    /// Compares the edge line with index `edge_index` of this octagon and
    /// `other` in the RegularTileShape edge order: [`Side::Positive`]
    /// (Java `ON_THE_LEFT`) if this edge is on the left of the other,
    /// [`Side::Negative`] (Java `ON_THE_RIGHT`) if on the right,
    /// [`Side::Collinear`] if equal. Panics (Java:
    /// `IllegalArgumentException` — whose message says "IntBox.compare",
    /// a copy-paste artifact of the Java original) for `edge_index` out of
    /// range.
    pub fn compare(&self, other: &IntOctagon, edge_index: i32) -> Side {
        match edge_index {
            0 => {
                // compare the lower edge line
                if self.bottom_y > other.bottom_y {
                    Side::Positive
                } else if self.bottom_y < other.bottom_y {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            1 => {
                // compare the lower right edge line
                if self.lower_right_diagonal_x < other.lower_right_diagonal_x {
                    Side::Positive
                } else if self.lower_right_diagonal_x > other.lower_right_diagonal_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            2 => {
                // compare the right edge line
                if self.right_x < other.right_x {
                    Side::Positive
                } else if self.right_x > other.right_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            3 => {
                // compare the upper right edge line
                if self.upper_right_diagonal_x < other.upper_right_diagonal_x {
                    Side::Positive
                } else if self.upper_right_diagonal_x > other.upper_right_diagonal_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            4 => {
                // compare the upper edge line
                if self.top_y < other.top_y {
                    Side::Positive
                } else if self.top_y > other.top_y {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            5 => {
                // compare the upper left edge line
                if self.upper_left_diagonal_x > other.upper_left_diagonal_x {
                    Side::Positive
                } else if self.upper_left_diagonal_x < other.upper_left_diagonal_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            6 => {
                // compare the left edge line
                if self.left_x > other.left_x {
                    Side::Positive
                } else if self.left_x < other.left_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            7 => {
                // compare the lower left edge line
                if self.lower_left_diagonal_x > other.lower_left_diagonal_x {
                    Side::Positive
                } else if self.lower_left_diagonal_x < other.lower_left_diagonal_x {
                    Side::Negative
                } else {
                    Side::Collinear
                }
            }
            _ => panic!("IntBox.compare: edgeIndex out of range"),
        }
    }

    /// Java `compare(IntBox, int)`.
    pub fn compare_box(&self, other: &IntBox, edge_index: i32) -> Side {
        self.compare(&other.to_int_octagon(), edge_index)
    }

    /// Calculates the border point of this octagon from `point` into the 45
    /// degree direction `dir`. If this border point is not an [`IntPoint`],
    /// the nearest outside IntPoint of the octagon is returned. The ceil/floor
    /// sites are Java `(int) Math.ceil/floor` narrowings (saturating, like
    /// Rust's `as`); the int sums inside wrap first.
    pub fn border_point(&self, point: &IntPoint, dir: FortyfiveDegreeDirection) -> IntPoint {
        match dir {
            FortyfiveDegreeDirection::RIGHT => {
                let mut result_x = self
                    .right_x
                    .min(self.upper_right_diagonal_x.wrapping_sub(point.y));
                result_x = result_x.min(self.lower_right_diagonal_x.wrapping_add(point.y));
                IntPoint::new(result_x, point.y)
            }
            FortyfiveDegreeDirection::LEFT => {
                let mut result_x = self
                    .left_x
                    .max(self.upper_left_diagonal_x.wrapping_add(point.y));
                result_x = result_x.max(self.lower_left_diagonal_x.wrapping_sub(point.y));
                IntPoint::new(result_x, point.y)
            }
            FortyfiveDegreeDirection::UP => {
                let mut result_y = self
                    .top_y
                    .min(point.x.wrapping_sub(self.upper_left_diagonal_x));
                result_y = result_y.min(self.upper_right_diagonal_x.wrapping_sub(point.x));
                IntPoint::new(point.x, result_y)
            }
            FortyfiveDegreeDirection::DOWN => {
                let mut result_y = self
                    .bottom_y
                    .max(self.lower_left_diagonal_x.wrapping_sub(point.x));
                result_y = result_y.max(point.x.wrapping_sub(self.lower_right_diagonal_x));
                IntPoint::new(point.x, result_y)
            }
            FortyfiveDegreeDirection::RIGHT45 => {
                let mut result_x = (0.5
                    * f64::from(
                        point
                            .x
                            .wrapping_sub(point.y)
                            .wrapping_add(self.upper_right_diagonal_x),
                    ))
                .ceil() as i32;
                result_x = result_x.min(self.right_x);
                result_x = result_x.min(point.x.wrapping_sub(point.y).wrapping_add(self.top_y));
                IntPoint::new(
                    result_x,
                    point.y.wrapping_sub(point.x).wrapping_add(result_x),
                )
            }
            FortyfiveDegreeDirection::UP45 => {
                let mut result_x = (0.5
                    * f64::from(
                        point
                            .x
                            .wrapping_add(point.y)
                            .wrapping_add(self.upper_left_diagonal_x),
                    ))
                .floor() as i32;
                result_x = result_x.max(self.left_x);
                result_x = result_x.max(point.x.wrapping_add(point.y).wrapping_sub(self.top_y));
                IntPoint::new(
                    result_x,
                    point.y.wrapping_add(point.x).wrapping_sub(result_x),
                )
            }
            FortyfiveDegreeDirection::LEFT45 => {
                let mut result_x = (0.5
                    * f64::from(
                        point
                            .x
                            .wrapping_sub(point.y)
                            .wrapping_add(self.lower_left_diagonal_x),
                    ))
                .floor() as i32;
                result_x = result_x.max(self.left_x);
                result_x = result_x.max(point.x.wrapping_sub(point.y).wrapping_add(self.bottom_y));
                IntPoint::new(
                    result_x,
                    point.y.wrapping_sub(point.x).wrapping_add(result_x),
                )
            }
            FortyfiveDegreeDirection::DOWN45 => {
                let mut result_x = (0.5
                    * f64::from(
                        point
                            .x
                            .wrapping_add(point.y)
                            .wrapping_add(self.lower_right_diagonal_x),
                    ))
                .ceil() as i32;
                result_x = result_x.min(self.right_x);
                result_x = result_x.min(point.x.wrapping_add(point.y).wrapping_sub(self.bottom_y));
                IntPoint::new(
                    result_x,
                    point.y.wrapping_add(point.x).wrapping_sub(result_x),
                )
            }
        }
    }

    /// Calculates the sorted `max_result_points` nearest points on the
    /// border of this octagon in the 45-degree directions. `point` is
    /// assumed to be located in the interior of this octagon. Java iterates
    /// `FortyfiveDegreeDirection.values()` in declaration order and inserts
    /// with strict `<` into a fixed-size top-k (ties keep the earlier
    /// direction and push later equals one slot deeper, dropping the tail).
    /// This port reproduces that exactly; the returned vector always holds
    /// `min(max_result_points, 8)` entries (an insertion can only fail on a
    /// non-finite distance square, unreachable for CRIT_INT-bounded
    /// coordinates, where Java would leave a null slot instead).
    pub fn nearest_border_projections(
        &self,
        point: &IntPoint,
        max_result_points: i32,
    ) -> Vec<IntPoint> {
        // Java resolves this.contains(point) to TileShape.contains(Point);
        // for int coordinates this equals the FloatPoint predicate exactly.
        if !self.contains(&point.to_float()) || max_result_points <= 0 {
            return Vec::new();
        }
        let result_len = max_result_points.min(8) as usize;
        let inside_point = point.to_float();
        let mut min_dist: Vec<f64> = Vec::with_capacity(result_len);
        let mut result: Vec<IntPoint> = Vec::with_capacity(result_len);
        for current_direction in FortyfiveDegreeDirection::VALUES {
            let current_border_point = self.border_point(point, current_direction);
            let current_distance = inside_point.distance_square(&current_border_point.to_float());
            let pos = min_dist
                .iter()
                .position(|&d| current_distance < d)
                .unwrap_or(min_dist.len());
            if pos == result_len {
                // farther than every kept candidate and the top-k is full:
                // the Java loop falls through and drops it
                continue;
            }
            if pos == min_dist.len() {
                min_dist.push(current_distance);
                result.push(current_border_point);
            } else {
                min_dist.insert(pos, current_distance);
                result.insert(pos, current_border_point);
                if min_dist.len() > result_len {
                    min_dist.pop();
                    result.pop();
                }
            }
        }
        result
    }

    /// Java package-private `borderLineSideOf(FloatPoint, int, double)`
    /// (IntOctagon.java:942): the side of `point` relative to border line
    /// `line_index` with a tolerance. Dead code in Java (no call sites)
    /// but self-contained, so ported rather than deferred. An out-of-range
    /// index yields [`Side::Collinear`] — the Java original logs a warning
    /// first.
    pub fn border_line_side_of(&self, point: &FloatPoint, line_index: i32, tolerance: f64) -> Side {
        match line_index {
            0 => {
                if point.y > f64::from(self.bottom_y) + tolerance {
                    Side::Negative
                } else if point.y < f64::from(self.bottom_y) - tolerance {
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            2 => {
                if point.x < f64::from(self.right_x) - tolerance {
                    Side::Negative
                } else if point.x > f64::from(self.right_x) + tolerance {
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            4 => {
                if point.y < f64::from(self.top_y) - tolerance {
                    Side::Negative
                } else if point.y > f64::from(self.top_y) + tolerance {
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            6 => {
                if point.x > f64::from(self.left_x) + tolerance {
                    Side::Negative
                } else if point.x < f64::from(self.left_x) - tolerance {
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            1 => {
                let tmp = point.y - point.x + f64::from(self.lower_right_diagonal_x);
                if tmp > tolerance {
                    // the point is above the lower right border line of this octagon
                    Side::Negative
                } else if tmp < -tolerance {
                    // the point is below the lower right border line of this octagon
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            3 => {
                let tmp = point.x + point.y - f64::from(self.upper_right_diagonal_x);
                if tmp < -tolerance {
                    // the point is below the upper right border line of this octagon
                    Side::Negative
                } else if tmp > tolerance {
                    // the point is above the upper right border line of this octagon
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            5 => {
                let tmp = point.y - point.x + f64::from(self.upper_left_diagonal_x);
                if tmp < -tolerance {
                    // the point is below the upper left border line of this octagon
                    Side::Negative
                } else if tmp > tolerance {
                    // the point is above the upper left border line of this octagon
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            7 => {
                let tmp = point.x + point.y - f64::from(self.lower_left_diagonal_x);
                if tmp > tolerance {
                    // the point is above the lower left border line of this octagon
                    Side::Negative
                } else if tmp < -tolerance {
                    // the point is below the lower left border line of this octagon
                    Side::Positive
                } else {
                    Side::Collinear
                }
            }
            // FRLogger.warn dropped: the observable value (Collinear) is
            // preserved.
            _ => Side::Collinear,
        }
    }

    /// Checks, if this octagon can be converted to an [`IntBox`] (Java
    /// `isIntBox`).
    pub const fn is_int_box(&self) -> bool {
        if self.lower_left_diagonal_x != self.left_x.wrapping_add(self.bottom_y) {
            return false;
        }
        if self.lower_right_diagonal_x != self.right_x.wrapping_sub(self.bottom_y) {
            return false;
        }
        if self.upper_right_diagonal_x != self.right_x.wrapping_add(self.top_y) {
            return false;
        }
        self.upper_left_diagonal_x == self.left_x.wrapping_sub(self.top_y)
    }

    /// Java package-private `cutoutFrom(IntBox)`: divide `d` minus this
    /// octagon into 8 convex pieces, from which 4 have cut off a corner.
    pub fn cutout_from_box(&self, d: &IntBox) -> Vec<IntOctagon> {
        let c = self.intersection_box(d);

        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![d.to_int_octagon()];
        }

        let mut r#box = [
            // construct left box
            IntBox::from_corners(
                d.ll.x,
                c.lower_left_diagonal_x.wrapping_sub(c.left_x),
                c.left_x,
                c.left_x.wrapping_sub(c.upper_left_diagonal_x),
            ),
            // construct right box
            IntBox::from_corners(
                c.right_x,
                c.right_x.wrapping_sub(c.lower_right_diagonal_x),
                d.ur.x,
                c.upper_right_diagonal_x.wrapping_sub(c.right_x),
            ),
            // construct lower box
            IntBox::from_corners(
                c.lower_left_diagonal_x.wrapping_sub(c.bottom_y),
                d.ll.y,
                c.lower_right_diagonal_x.wrapping_add(c.bottom_y),
                c.bottom_y,
            ),
            // construct upper box
            IntBox::from_corners(
                c.upper_left_diagonal_x.wrapping_add(c.top_y),
                c.top_y,
                c.upper_right_diagonal_x.wrapping_sub(c.top_y),
                d.ur.y,
            ),
        ];

        let mut octagons = [
            // construct upper left octagon
            IntOctagon::new(
                d.ll.x,
                r#box[0].ur.y,
                r#box[3].ll.x,
                d.ur.y,
                -CRIT_INT,
                c.upper_left_diagonal_x,
                -CRIT_INT,
                CRIT_INT,
            )
            .normalize(),
            // construct lower left octagon
            IntOctagon::new(
                d.ll.x,
                d.ll.y,
                r#box[2].ll.x,
                r#box[0].ll.y,
                -CRIT_INT,
                CRIT_INT,
                -CRIT_INT,
                c.lower_left_diagonal_x,
            )
            .normalize(),
            // construct lower right octagon
            IntOctagon::new(
                r#box[2].ur.x,
                d.ll.y,
                d.ur.x,
                r#box[1].ll.y,
                c.lower_right_diagonal_x,
                CRIT_INT,
                -CRIT_INT,
                CRIT_INT,
            )
            .normalize(),
            // construct upper right octagon
            IntOctagon::new(
                r#box[3].ur.x,
                r#box[1].ur.y,
                d.ur.x,
                d.ur.y,
                -CRIT_INT,
                CRIT_INT,
                c.upper_right_diagonal_x,
                CRIT_INT,
            )
            .normalize(),
        ];

        // optimise the result to minimum cumulative circumference

        let (b, o) = (r#box[0], octagons[0]);
        if b.ur.x.wrapping_sub(b.ll.x) > o.top_y.wrapping_sub(o.bottom_y) {
            // switch the horizontal upper left divide line to vertical
            r#box[0] = IntBox::from_corners(b.ll.x, b.ll.y, b.ur.x, o.top_y);
            octagons[0] = IntOctagon::new(
                b.ur.x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }

        let (b, o) = (r#box[3], octagons[0]);
        if b.ur.y.wrapping_sub(b.ll.y) > o.right_x.wrapping_sub(o.left_x) {
            // switch the vertical upper left divide line to horizontal
            r#box[3] = IntBox::from_corners(o.left_x, b.ll.y, b.ur.x, b.ur.y);
            octagons[0] = IntOctagon::new(
                o.left_x,
                o.bottom_y,
                o.right_x,
                b.ll.y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[3], octagons[3]);
        if b.ur.y.wrapping_sub(b.ll.y) > o.right_x.wrapping_sub(o.left_x) {
            // switch the vertical upper right divide line to horizontal
            r#box[3] = IntBox::from_corners(b.ll.x, b.ll.y, o.right_x, b.ur.y);
            octagons[3] = IntOctagon::new(
                o.left_x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[1], octagons[3]);
        if b.ur.x.wrapping_sub(b.ll.x) > o.top_y.wrapping_sub(o.bottom_y) {
            // switch the horizontal upper right divide line to vertical
            r#box[1] = IntBox::from_corners(b.ll.x, b.ll.y, b.ur.x, o.top_y);
            octagons[3] = IntOctagon::new(
                o.left_x,
                o.bottom_y,
                b.ll.x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[1], octagons[2]);
        if b.ur.x.wrapping_sub(b.ll.x) > o.top_y.wrapping_sub(o.bottom_y) {
            // switch the horizontal lower right divide line to vertical
            r#box[1] = IntBox::from_corners(b.ll.x, o.bottom_y, b.ur.x, b.ur.y);
            octagons[2] = IntOctagon::new(
                o.left_x,
                o.bottom_y,
                b.ll.x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[2], octagons[2]);
        if b.ur.y.wrapping_sub(b.ll.y) > o.right_x.wrapping_sub(o.left_x) {
            // switch the vertical lower right divide line to horizontal
            r#box[2] = IntBox::from_corners(b.ll.x, b.ll.y, o.right_x, b.ur.y);
            octagons[2] = IntOctagon::new(
                o.left_x,
                b.ur.y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[2], octagons[1]);
        if b.ur.y.wrapping_sub(b.ll.y) > o.right_x.wrapping_sub(o.left_x) {
            // switch the vertical lower  left divide line to horizontal
            r#box[2] = IntBox::from_corners(o.left_x, b.ll.y, b.ur.x, b.ur.y);
            octagons[1] = IntOctagon::new(
                o.left_x,
                b.ur.y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }
        let (b, o) = (r#box[0], octagons[1]);
        if b.ur.x.wrapping_sub(b.ll.x) > o.top_y.wrapping_sub(o.bottom_y) {
            // switch the horizontal lower left divide line to vertical
            r#box[0] = IntBox::from_corners(b.ll.x, o.bottom_y, b.ur.x, b.ur.y);
            octagons[1] = IntOctagon::new(
                b.ur.x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x,
            )
            .normalize();
        }

        let mut result = Vec::with_capacity(8);

        // add the 4 boxes to the result
        for piece in r#box {
            result.push(piece.to_int_octagon());
        }

        // add the 4 octagons to the result
        result.extend_from_slice(&octagons);
        result
    }

    /// Java package-private `cutoutFrom(IntOctagon)`: divide `d` minus this
    /// octagon into 8 convex pieces without sharp angles.
    #[allow(clippy::too_many_lines)]
    pub fn cutout_from(&self, d: &IntOctagon) -> Vec<IntOctagon> {
        let c = self.intersection(d);

        if self.is_empty() || c.dimension() < self.dimension() {
            // there is only an overlap at the border
            return vec![*d];
        }

        let mut result = [IntOctagon::EMPTY; 8];

        let mut tmp = c.lower_left_diagonal_x.wrapping_sub(c.left_x);

        result[0] = IntOctagon::new(
            d.left_x,
            tmp,
            c.left_x,
            c.left_x.wrapping_sub(c.upper_left_diagonal_x),
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        let mut tmp2 = c.lower_left_diagonal_x.wrapping_sub(c.bottom_y);

        result[1] = IntOctagon::new(
            d.left_x,
            d.bottom_y,
            tmp2,
            tmp,
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            c.lower_left_diagonal_x,
        );

        tmp = c.lower_right_diagonal_x.wrapping_add(c.bottom_y);

        result[2] = IntOctagon::new(
            tmp2,
            d.bottom_y,
            tmp,
            c.bottom_y,
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        tmp2 = c.right_x.wrapping_sub(c.lower_right_diagonal_x);

        result[3] = IntOctagon::new(
            tmp,
            d.bottom_y,
            d.right_x,
            tmp2,
            c.lower_right_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        tmp = c.upper_right_diagonal_x.wrapping_sub(c.right_x);

        result[4] = IntOctagon::new(
            c.right_x,
            tmp2,
            d.right_x,
            tmp,
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        tmp2 = c.upper_right_diagonal_x.wrapping_sub(c.top_y);

        result[5] = IntOctagon::new(
            tmp2,
            tmp,
            d.right_x,
            d.top_y,
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            c.upper_right_diagonal_x,
            d.upper_right_diagonal_x,
        );

        tmp = c.upper_left_diagonal_x.wrapping_add(c.top_y);

        result[6] = IntOctagon::new(
            tmp,
            c.top_y,
            tmp2,
            d.top_y,
            d.upper_left_diagonal_x,
            d.lower_right_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        tmp2 = c.left_x.wrapping_sub(c.upper_left_diagonal_x);

        result[7] = IntOctagon::new(
            d.left_x,
            tmp2,
            tmp,
            d.top_y,
            d.upper_left_diagonal_x,
            c.upper_left_diagonal_x,
            d.lower_left_diagonal_x,
            d.upper_right_diagonal_x,
        );

        for piece in &mut result {
            *piece = piece.normalize();
        }

        let mut curr1 = result[0];
        let mut curr2 = result[7];

        if !(curr1.is_empty() || curr2.is_empty())
            && curr1.right_x.wrapping_sub(curr1.left_x_value(curr1.top_y))
                > curr2
                    .upper_y_value(curr1.right_x)
                    .wrapping_sub(curr2.bottom_y)
        {
            // switch the horizontal upper left divide line to vertical
            curr1 = IntOctagon::new(
                curr1.left_x.min(curr2.left_x),
                curr1.bottom_y,
                curr1.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            curr2 = IntOctagon::new(
                curr1.right_x,
                curr2.bottom_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            result[0] = curr1.normalize();
            result[7] = curr2.normalize();
        }
        curr1 = result[7];
        curr2 = result[6];
        if !(curr1.is_empty() || curr2.is_empty())
            && curr2
                .upper_y_value(curr1.right_x)
                .wrapping_sub(curr2.bottom_y)
                > curr1
                    .right_x
                    .wrapping_sub(curr1.left_x_value(curr2.bottom_y))
        {
            // switch the vertical upper left divide line to horizontal
            curr2 = IntOctagon::new(
                curr1.left_x,
                curr2.bottom_y,
                curr2.right_x,
                curr2.top_y.max(curr1.top_y),
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr1.right_x,
                curr2.bottom_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            result[7] = curr1.normalize();
            result[6] = curr2.normalize();
        }
        curr1 = result[6];
        curr2 = result[5];
        if !(curr1.is_empty() || curr2.is_empty())
            && curr2
                .upper_y_value(curr1.right_x)
                .wrapping_sub(curr1.bottom_y)
                > curr2
                    .right_x_value(curr1.bottom_y)
                    .wrapping_sub(curr2.left_x)
        {
            // switch the vertical upper right divide line to horizontal
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr2.right_x,
                curr2.top_y.max(curr1.top_y),
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr2.right_x,
                curr1.bottom_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            result[6] = curr1.normalize();
            result[5] = curr2.normalize();
        }
        curr1 = result[5];
        curr2 = result[4];
        if !(curr1.is_empty() || curr2.is_empty())
            && curr2.right_x_value(curr2.top_y).wrapping_sub(curr2.left_x)
                > curr1.upper_y_value(curr2.left_x).wrapping_sub(curr2.top_y)
        {
            // switch the horizontal upper right divide line to vertical
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr2.right_x.max(curr1.right_x),
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            curr1 = IntOctagon::new(
                curr1.left_x,
                curr1.bottom_y,
                curr2.left_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            result[5] = curr1.normalize();
            result[4] = curr2.normalize();
        }
        curr1 = result[4];
        curr2 = result[3];
        if !(curr1.is_empty() || curr2.is_empty())
            && curr1
                .right_x_value(curr1.bottom_y)
                .wrapping_sub(curr1.left_x)
                > curr1
                    .bottom_y
                    .wrapping_sub(curr2.lower_y_value(curr1.left_x))
        {
            // switch the horizontal lower right divide line to vertical
            curr1 = IntOctagon::new(
                curr1.left_x,
                curr2.bottom_y,
                curr2.right_x.max(curr1.right_x),
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            curr2 = IntOctagon::new(
                curr2.left_x,
                curr2.bottom_y,
                curr1.left_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            result[4] = curr1.normalize();
            result[3] = curr2.normalize();
        }

        curr1 = result[3];
        curr2 = result[2];

        if !(curr1.is_empty() || curr2.is_empty())
            && curr2.top_y.wrapping_sub(curr2.lower_y_value(curr2.right_x))
                > curr1.right_x_value(curr2.top_y).wrapping_sub(curr2.right_x)
        {
            // switch the vertical lower right divide line to horizontal
            curr2 = IntOctagon::new(
                curr2.left_x,
                curr1.bottom_y.min(curr2.bottom_y),
                curr1.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            curr1 = IntOctagon::new(
                curr1.left_x,
                curr2.top_y,
                curr1.right_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            result[3] = curr1.normalize();
            result[2] = curr2.normalize();
        }

        curr1 = result[2];
        curr2 = result[1];

        if !(curr1.is_empty() || curr2.is_empty())
            && curr1.top_y.wrapping_sub(curr1.lower_y_value(curr1.left_x))
                > curr1.left_x.wrapping_sub(curr2.left_x_value(curr1.top_y))
        {
            // switch the vertical lower left divide line to horizontal
            curr1 = IntOctagon::new(
                curr2.left_x,
                curr1.bottom_y.min(curr2.bottom_y),
                curr1.right_x,
                curr1.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            curr2 = IntOctagon::new(
                curr2.left_x,
                curr1.top_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr2.lower_right_diagonal_x,
                curr2.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            result[2] = curr1.normalize();
            result[1] = curr2.normalize();
        }

        curr1 = result[1];
        curr2 = result[0];

        if !(curr1.is_empty() || curr2.is_empty())
            && curr2
                .right_x
                .wrapping_sub(curr2.left_x_value(curr2.bottom_y))
                > curr2
                    .bottom_y
                    .wrapping_sub(curr1.lower_y_value(curr2.right_x))
        {
            // switch the horizontal lower left divide line to vertical
            curr2 = IntOctagon::new(
                curr2.left_x.min(curr1.left_x),
                curr1.bottom_y,
                curr2.right_x,
                curr2.top_y,
                curr2.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr2.upper_right_diagonal_x,
            );

            curr1 = IntOctagon::new(
                curr2.right_x,
                curr1.bottom_y,
                curr1.right_x,
                curr1.top_y,
                curr1.upper_left_diagonal_x,
                curr1.lower_right_diagonal_x,
                curr1.lower_left_diagonal_x,
                curr1.upper_right_diagonal_x,
            );

            result[1] = curr1.normalize();
            result[0] = curr2.normalize();
        }

        result.to_vec()
    }

    /// Returns the translation of this octagon by `rel_coor`. Panics (Java:
    /// `ClassCastException` on the `(IntVector)` cast) if the vector has
    /// rational coordinates.
    pub fn translate_by(&self, rel_coor: &Vector) -> IntOctagon {
        // This function is at the moment only implemented for Vectors
        // with integer coordinates.
        // The general implementation is still missing.
        if *rel_coor == Vector::ZERO {
            return *self;
        }
        let relative_coordinate = match rel_coor {
            Vector::Int(v) => *v,
            _ => panic!("IntOctagon.translateBy: expected an int vector"),
        };
        IntOctagon::new(
            self.left_x.wrapping_add(relative_coordinate.x),
            self.bottom_y.wrapping_add(relative_coordinate.y),
            self.right_x.wrapping_add(relative_coordinate.x),
            self.top_y.wrapping_add(relative_coordinate.y),
            self.upper_left_diagonal_x
                .wrapping_add(relative_coordinate.x)
                .wrapping_sub(relative_coordinate.y),
            self.lower_right_diagonal_x
                .wrapping_add(relative_coordinate.x)
                .wrapping_sub(relative_coordinate.y),
            self.lower_left_diagonal_x
                .wrapping_add(relative_coordinate.x)
                .wrapping_add(relative_coordinate.y),
            self.upper_right_diagonal_x
                .wrapping_add(relative_coordinate.x)
                .wrapping_add(relative_coordinate.y),
        )
    }
}

impl core::fmt::Display for IntOctagon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "IntOctagon(leftX={}, bottomY={}, rightX={}, topY={}, upperLeftDiagonalX={}, lowerRightDiagonalX={}, lowerLeftDiagonalX={}, upperRightDiagonalX={})",
            self.left_x,
            self.bottom_y,
            self.right_x,
            self.top_y,
            self.upper_left_diagonal_x,
            self.lower_right_diagonal_x,
            self.lower_left_diagonal_x,
            self.upper_right_diagonal_x
        )
    }
}

// ---------------------------------------------------------------------------
// Task 7 closures (TileShape/Simplex family)
// ---------------------------------------------------------------------------

impl IntOctagon {
    /// The no-th boundary line as an infinite supporting line through the
    /// axis intercepts (Java `IntOctagon.borderLine`): 0 lower, 1
    /// lower-right diagonal, 2 right, 3 upper-right diagonal, 4 upper, 5
    /// upper-left diagonal, 6 left, 7 lower-left diagonal. Panics out of
    /// range (Java throws IllegalArgumentException).
    pub fn border_line(&self, no: i32) -> Line {
        match no {
            0 => Line::new(
                Point::int(IntPoint::new(0, self.bottom_y)),
                Point::int(IntPoint::new(1, self.bottom_y)),
            ),
            1 => Line::new(
                Point::int(IntPoint::new(self.lower_right_diagonal_x, 0)),
                Point::int(IntPoint::new(
                    self.lower_right_diagonal_x.wrapping_add(1),
                    1,
                )),
            ),
            2 => Line::new(
                Point::int(IntPoint::new(self.right_x, 0)),
                Point::int(IntPoint::new(self.right_x, 1)),
            ),
            3 => Line::new(
                Point::int(IntPoint::new(self.upper_right_diagonal_x, 0)),
                Point::int(IntPoint::new(
                    self.upper_right_diagonal_x.wrapping_sub(1),
                    1,
                )),
            ),
            4 => Line::new(
                Point::int(IntPoint::new(0, self.top_y)),
                Point::int(IntPoint::new(-1, self.top_y)),
            ),
            5 => Line::new(
                Point::int(IntPoint::new(self.upper_left_diagonal_x, 0)),
                Point::int(IntPoint::new(
                    self.upper_left_diagonal_x.wrapping_sub(1),
                    -1,
                )),
            ),
            6 => Line::new(
                Point::int(IntPoint::new(self.left_x, 0)),
                Point::int(IntPoint::new(self.left_x, -1)),
            ),
            7 => Line::new(
                Point::int(IntPoint::new(self.lower_left_diagonal_x, 0)),
                Point::int(IntPoint::new(
                    self.lower_left_diagonal_x.wrapping_add(1),
                    -1,
                )),
            ),
            _ => panic!("IntOctagon.borderLine: no out of range"),
        }
    }

    /// Returns -1: Java logs "edge_index_of_line not yet implemented for
    /// octagons" (identity comparison deliberately unimplemented,
    /// bug-compatible with the oracle).
    pub fn border_line_index(&self, _line: &Line) -> i32 {
        -1
    }

    /// Converts this octagon to a Simplex: the eight border lines with the
    /// redundant ones removed. Java memoizes the result in
    /// `precalculatedToSimplex`; per T14 the port computes it fresh on
    /// every call. The border-line directions are cyclically sorted, as
    /// `remove_redundant_lines` requires (jshell pin `PB2`).
    pub fn to_simplex(&self) -> Simplex {
        if self.is_empty() {
            return Simplex::empty();
        }
        let lines: Vec<Line> = (0..8).map(|i| self.border_line(i)).collect();
        Simplex::new(lines).remove_redundant_lines()
    }

    /// Converts the physical instance of this shape to a simpler physical
    /// instance: a box-like octagon to its `IntBox`, identity otherwise
    /// (Java `IntOctagon.simplify`).
    pub fn simplify(&self) -> TileShape {
        if self.is_int_box() {
            TileShape::RegularTileShape(RegularTileShape::IntBox(self.bounding_box()))
        } else {
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(*self))
        }
    }

    /// Returns the intersection of this octagon with the simplex (Java
    /// `IntOctagon.intersection(Simplex)` == `other.intersection(this)`).
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        other.intersection_octagon(self)
    }

    /// Returns true if this octagon and the simplex have a nonempty
    /// intersection (Java `IntOctagon.intersects(Simplex)` ==
    /// `other.intersects(this)`).
    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        other.intersects_octagon(self)
    }

    /// Cuts this octagon out of the simplex (Java
    /// `IntOctagon.cutoutFrom(Simplex)` ==
    /// `this.toSimplex().cutoutFrom(simplex)`); the pieces belong to the
    /// simplex.
    pub fn cutout_from_simplex(&self, simplex: &Simplex) -> Vec<Simplex> {
        self.to_simplex().cutout_from_simplex(simplex)
    }

    /// Returns true, if this octagon and the circle have a nonempty
    /// intersection (Java `IntOctagon.intersects(Circle)`): the centre
    /// distance of the concrete `TileShape.distance` must not exceed the
    /// radius. Touching the border counts (pin IC1).
    pub fn intersects_circle(&self, circle: &Circle) -> bool {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(*self))
            .distance(&circle.center.to_float())
            <= circle.radius as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_vector::IntVector;
    use crate::java_random::JavaRandom;
    use crate::rational_vector::RationalVector;
    use num_bigint::BigInt;

    /// Constructor-order shorthand (Java `new IntOctagon(...)`).
    #[allow(clippy::many_single_char_names)]
    #[allow(clippy::too_many_arguments)]
    fn o(lx: i32, ly: i32, rx: i32, uy: i32, ulx: i32, lrx: i32, llx: i32, urx: i32) -> IntOctagon {
        IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx)
    }

    fn p(x: i32, y: i32) -> IntPoint {
        IntPoint::new(x, y)
    }

    /// The unit square-ish test octagon used throughout: the IntBox
    /// (0,0,10,10) converted via `toIntOctagon`.
    fn oct() -> IntOctagon {
        o(0, 0, 10, 10, -10, 10, 0, 20)
    }

    /// jshell pin: `IntOctagon.EMPTY` toString and `is_empty`.
    #[test]
    fn empty_sentinel_fields_exact() {
        let e = IntOctagon::EMPTY;
        assert_eq!(e.left_x, CRIT_INT);
        assert_eq!(e.bottom_y, CRIT_INT);
        assert_eq!(e.right_x, -CRIT_INT);
        assert_eq!(e.top_y, -CRIT_INT);
        assert_eq!(e.upper_left_diagonal_x, CRIT_INT);
        assert_eq!(e.lower_right_diagonal_x, -CRIT_INT);
        assert_eq!(e.lower_left_diagonal_x, CRIT_INT);
        assert_eq!(e.upper_right_diagonal_x, -CRIT_INT);
        assert!(e.is_empty());
        assert_eq!(
            e.to_string(),
            "IntOctagon(leftX=33554432, bottomY=33554432, rightX=-33554432, topY=-33554432, upperLeftDiagonalX=33554432, lowerRightDiagonalX=-33554432, lowerLeftDiagonalX=33554432, upperRightDiagonalX=-33554432)"
        );
    }

    /// T2: Java emptiness is reference identity on the normalize() output,
    /// so the port's value-equality is equivalent on canonical outputs; a
    /// merely inverted octagon is NOT empty (contrast IntBox).
    #[test]
    fn is_empty_identity_based_not_value_based() {
        // inverted but not the sentinel: not empty (its normalization IS)
        assert!(!o(5, 5, 3, 4, 0, 0, 0, 0).is_empty());
        assert!(o(5, 5, 3, 4, 0, 0, 0, 0).normalize().is_empty());
        // a hand-built sentinel tuple IS empty under value equality
        assert!(
            o(
                CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT
            )
            .is_empty()
        );
    }

    /// jshell pin: `IntOctagon.EMPTY.area()` raw bits = 0x4320000000000000
    /// (= 2^51, the garbage-on-empty artifact, plan note T11);
    /// `oct.area()` = 100.0; the wrapped box octagon area = 4.0e10.
    #[test]
    fn area_pins_including_empty_garbage() {
        assert_eq!(oct().area(), 100.0);
        let bits = IntOctagon::EMPTY.area().to_bits();
        assert_eq!(bits, 0x4320_0000_0000_0000);
        assert_eq!(IntOctagon::EMPTY.area(), 2_251_799_813_685_248.0);
        // IntBox(-2000000000,-5,2000000000,5).toIntOctagon()
        let w = IntBox::from_corners(-2_000_000_000, -5, 2_000_000_000, 5).to_int_octagon();
        assert_eq!(
            w,
            o(
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
        assert_eq!(w.area(), 4.0e10);
    }

    /// jshell pin: `oct.getId()` = 294356348.
    #[test]
    fn get_id_pin() {
        assert_eq!(oct().get_id(), 294_356_348);
    }

    /// jshell pins: corners of `oct` = (0,0) (10,0) (10,0) (10,10) (10,10)
    /// (0,10) (0,10) (0,0) — repeated corners on the diagonal-touching
    /// borders.
    #[test]
    fn corners_pin_and_panic() {
        let expected = [
            p(0, 0),
            p(10, 0),
            p(10, 0),
            p(10, 10),
            p(10, 10),
            p(0, 10),
            p(0, 10),
            p(0, 0),
        ];
        for (i, exp) in expected.iter().enumerate() {
            assert_eq!(oct().corner(i as i32), *exp);
            assert_eq!(oct().corner_x(i as i32), exp.x);
            assert_eq!(oct().corner_y(i as i32), exp.y);
        }
    }

    #[test]
    #[should_panic(expected = "IntOctagon.corner: no out of range")]
    fn corner_panics_on_index_out_of_range() {
        let _ = oct().corner(8);
    }

    #[test]
    #[should_panic(expected = "IntOctagon.corner: no out of range")]
    fn corner_x_panics_on_index_out_of_range() {
        let _ = oct().corner_x(-1);
    }

    #[test]
    #[should_panic(expected = "IntOctagon.corner: no out of range")]
    fn corner_y_panics_on_index_out_of_range() {
        let _ = oct().corner_y(9);
    }

    /// jshell pins: `oct.dimension()` = 2, `IntOctagon.EMPTY.dimension()` =
    /// -1; point octagon (from IntPoint(3,-4).surroundingOctagon()) is 0.
    #[test]
    fn dimension_cases() {
        assert_eq!(oct().dimension(), 2);
        assert_eq!(IntOctagon::EMPTY.dimension(), -1);
        assert_eq!(p(3, -4).surrounding_octagon().dimension(), 0);
        // horizontal segment: topY == bottomY, rightX > leftX
        assert_eq!(o(0, 0, 10, 0, -10, 10, 0, 10).dimension(), 1);
        // vertical segment
        assert_eq!(o(0, 0, 0, 10, -10, 0, 0, 10).dimension(), 1);
    }

    /// Trivial flags.
    #[test]
    fn trivial_flags() {
        let a = oct();
        assert!(a.is_int_octagon());
        assert!(a.is_bounded());
        assert!(a.corner_is_bounded(0));
        // Java returns true without validating the index
        assert!(a.corner_is_bounded(99));
        assert_eq!(a.border_line_count(), 8);
        assert_eq!(a.bounding_box(), IntBox::from_corners(0, 0, 10, 10));
        assert_eq!(a.bounding_octagon(), a);
        assert_eq!(a.bounding_tile(), a);
        assert!(a.is_int_box());
        // inner octagon is not axis-aligned: not convertible to a box
        assert!(!o(2, 1, 8, 7, 1, 9, 3, 15).is_int_box());
    }

    /// jshell pins: `oct.maxWidth()` = 14.14213562373095, `oct.minWidth()` =
    /// 10.0, `inner.maxWidth()` = 8.48528137423857 where
    /// inner = (2,1,8,7,1,9,3,15).
    #[test]
    fn max_min_width_pins() {
        assert_eq!(oct().max_width(), 14.142_135_623_730_95);
        assert_eq!(oct().min_width(), 10.0);
        assert_eq!(o(2, 1, 8, 7, 1, 9, 3, 15).max_width(), 8.485_281_374_238_57);
    }

    /// jshell pin: contains = true,false,false for (5,4), (10,11),
    /// (-0.5,5).
    #[test]
    fn contains_float_point_pin() {
        assert!(oct().contains(&FloatPoint::new(5.0, 4.0)));
        assert!(!oct().contains(&FloatPoint::new(10.0, 11.0)));
        assert!(!oct().contains(&FloatPoint::new(-0.5, 5.0)));
    }

    /// jshell pins: intersect=true,true,true,false and touch/overlap split.
    #[test]
    fn intersects_overlaps_pins() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        let far = o(20, 20, 30, 30, 10, 40, 30, 50);
        assert!(oct().intersects(&inner));
        assert!(oct().overlaps(&inner));
        assert!(oct().overlaps(&oct()));
        assert!(!oct().intersects(&far));
        // touching edge: intersects yes, overlaps no (hand-traced max/min)
        let touch = o(10, 0, 20, 10, 0, 20, 10, 30);
        assert!(oct().intersects(&touch));
        assert!(!oct().overlaps(&touch));
        assert!(!oct().intersects(&IntOctagon::EMPTY));
        assert!(!IntOctagon::EMPTY.intersects(&oct()));
        assert!(!oct().overlaps(&IntOctagon::EMPTY));
    }

    /// jshell pin (sbl lines): `oct.sideOfBorderLine(5,4,i)` is onTheLeft
    /// for all i; `(11,12,i)` is left,left,right,right,right,left,left,left.
    #[test]
    fn side_of_border_line_matrix() {
        for i in 0..8 {
            assert_eq!(oct().side_of_border_line(5, 4, i), Side::Positive);
        }
        let expected = [
            Side::Positive,
            Side::Positive,
            Side::Negative,
            Side::Negative,
            Side::Negative,
            Side::Positive,
            Side::Positive,
            Side::Positive,
        ];
        for (i, exp) in expected.iter().enumerate() {
            assert_eq!(oct().side_of_border_line(11, 12, i as i32), *exp);
        }
        // on-border points are collinear
        assert_eq!(oct().side_of_border_line(5, 0, 0), Side::Collinear);
        assert_eq!(oct().side_of_border_line(10, 5, 2), Side::Collinear);
    }

    /// jshell pins (borderLineSideOf via reflection on the package-private
    /// method): interior point (5,4) is onTheRight on all 8 border lines;
    /// the outside point (11,12) flips to onTheLeft on the right,
    /// upper-right and upper borders; tolerance and the out-of-range index
    /// are collinear.
    #[test]
    fn border_line_side_of_pins() {
        let a = oct();
        for i in 0..8 {
            assert_eq!(
                a.border_line_side_of(&FloatPoint::new(5.0, 4.0), i, 1e-6),
                Side::Negative
            );
        }
        let outside = FloatPoint::new(11.0, 12.0);
        assert_eq!(a.border_line_side_of(&outside, 0, 1e-6), Side::Negative);
        assert_eq!(a.border_line_side_of(&outside, 1, 1e-6), Side::Negative);
        assert_eq!(a.border_line_side_of(&outside, 2, 1e-6), Side::Positive);
        assert_eq!(a.border_line_side_of(&outside, 3, 1e-6), Side::Positive);
        assert_eq!(a.border_line_side_of(&outside, 4, 1e-6), Side::Positive);
        assert_eq!(a.border_line_side_of(&outside, 5, 1e-6), Side::Negative);
        assert_eq!(a.border_line_side_of(&outside, 6, 1e-6), Side::Negative);
        assert_eq!(a.border_line_side_of(&outside, 7, 1e-6), Side::Negative);
        // tolerance turns a near-border point collinear
        assert_eq!(
            a.border_line_side_of(&FloatPoint::new(10.5, 5.0), 2, 1.0),
            Side::Collinear
        );
        // out-of-range index falls back to Collinear (Java logs a warning)
        assert_eq!(
            a.border_line_side_of(&FloatPoint::new(5.0, 4.0), 9, 1e-6),
            Side::Collinear
        );
    }

    /// jshell pin (cmp lines): `oct.compare(inner, i)` is onTheRight for all
    /// eight edges.
    #[test]
    fn compare_octagon_pin_and_discrimination() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        for i in 0..8 {
            assert_eq!(oct().compare(&inner, i), Side::Negative);
            assert_eq!(oct().compare(&oct(), i), Side::Collinear);
        }
        // per-edge discrimination (direct field comparisons)
        assert_eq!(
            o(0, 2, 8, 7, 1, 9, 3, 15).compare(&inner, 0),
            Side::Positive
        );
        assert_eq!(
            o(2, 1, 9, 7, 1, 9, 3, 15).compare(&inner, 2),
            Side::Negative
        );
        assert_eq!(
            o(2, 1, 8, 8, 1, 9, 3, 15).compare(&inner, 4),
            Side::Negative
        );
        assert_eq!(
            o(2, 1, 8, 7, 2, 9, 3, 15).compare(&inner, 5),
            Side::Positive
        );
        assert_eq!(
            o(1, 1, 8, 7, 1, 9, 3, 15).compare(&inner, 6),
            Side::Negative
        );
        assert_eq!(
            o(2, 1, 8, 7, 1, 9, 4, 15).compare(&inner, 7),
            Side::Positive
        );
        assert_eq!(
            o(2, 1, 8, 7, 1, 10, 3, 15).compare(&inner, 1),
            Side::Negative
        );
        assert_eq!(
            o(2, 1, 8, 7, 1, 9, 3, 16).compare(&inner, 3),
            Side::Negative
        );
    }

    #[test]
    #[should_panic(expected = "IntBox.compare: edgeIndex out of range")]
    fn compare_panics_with_java_copy_paste_message() {
        let _ = oct().compare(&oct(), 8);
    }

    /// jshell pin (acc): `oct.leftXValue(3)` = 0, `rightXValue(3)` = 10,
    /// `lowerYValue(4)` = 0, `upperYValue(4)` = 10; diagonal hand-cases on
    /// inner = (2,1,8,7,1,9,3,15) (pure max/min arithmetic).
    #[test]
    fn boundary_value_accessors() {
        assert_eq!(oct().left_x_value(3), 0);
        assert_eq!(oct().right_x_value(3), 10);
        assert_eq!(oct().lower_y_value(4), 0);
        assert_eq!(oct().upper_y_value(4), 10);
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert_eq!(inner.left_x_value(7), 8);
        assert_eq!(inner.right_x_value(7), 8);
        assert_eq!(inner.lower_y_value(8), 1);
        assert_eq!(inner.upper_y_value(2), 1);
    }

    /// jshell pin (bp lines): borderPoint((5,4), dir) for all eight
    /// directions of `oct`.
    #[test]
    fn border_point_all_directions_pin() {
        let point = p(5, 4);
        let expected = [
            (FortyfiveDegreeDirection::RIGHT, p(10, 4)),
            (FortyfiveDegreeDirection::RIGHT45, p(10, 9)),
            (FortyfiveDegreeDirection::UP, p(5, 10)),
            (FortyfiveDegreeDirection::UP45, p(0, 9)),
            (FortyfiveDegreeDirection::LEFT, p(0, 4)),
            (FortyfiveDegreeDirection::LEFT45, p(1, 0)),
            (FortyfiveDegreeDirection::DOWN, p(5, 0)),
            (FortyfiveDegreeDirection::DOWN45, p(9, 0)),
        ];
        for (dir, exp) in expected {
            assert_eq!(oct().border_point(&point, dir), exp);
        }
    }

    /// jshell pins (nbp3/nbp8): nearestBorderProjections((2,2),3) =
    /// [(0,2), (2,0), (0,4)] — note the third slot: UP45 (0,4) displaces
    /// LEFT45 (0,0) at equal distance 8 because of the strict `<`
    /// insertion. With 8 slots every direction survives.
    #[test]
    fn nearest_border_projections_pins() {
        assert_eq!(
            oct().nearest_border_projections(&p(2, 2), 3),
            vec![p(0, 2), p(2, 0), p(0, 4)]
        );
        assert_eq!(
            oct().nearest_border_projections(&p(5, 5), 8),
            vec![
                p(10, 5),
                p(5, 10),
                p(0, 5),
                p(5, 0),
                p(10, 10),
                p(0, 10),
                p(0, 0),
                p(10, 0)
            ]
        );
        // guard branches
        assert!(oct().nearest_border_projections(&p(20, 20), 3).is_empty());
        assert!(oct().nearest_border_projections(&p(2, 2), 0).is_empty());
        assert!(oct().nearest_border_projections(&p(2, 2), -5).is_empty());
    }

    /// jshell pins (sliver): normalize((0,0,10,10,3,9,5,7)) tightens
    /// step 2 (lx 0->3), step 3 (rx 10->7), step 7 (uy 10->4), the
    /// upper diag merge (uy 4->2) and the lower-left diag merge
    /// (lx 3->4) => (4,0,7,2,3,7,5,7).
    #[test]
    fn normalize_sliver_pin() {
        assert_eq!(
            o(0, 0, 10, 10, 3, 9, 5, 7).normalize(),
            o(4, 0, 7, 2, 3, 7, 5, 7)
        );
    }

    /// jshell pin (upperMerge): (0,0,100,100,0,90,10,100) — diagUpperY =
    /// ceil((100-0)/2) = 50 pulls topY down, diagRightX = ceil(190/2) = 95
    /// and diagLeftX = floor(10/2) = 5 tighten the sides.
    #[test]
    fn normalize_upper_diag_merge_pin() {
        assert_eq!(
            o(0, 0, 100, 100, 0, 90, 10, 100).normalize(),
            o(5, 0, 95, 50, 0, 90, 10, 100)
        );
    }

    /// jshell pin (lowerMerge): (-100,-100,100,100,-50,60,10,50) — step 1
    /// (lx -100 -> -90), step 6 (ly -100 -> -90), then all four diag merges
    /// fire (uy -> 50, ly -> -25, rx -> 55, lx -> -20).
    #[test]
    fn normalize_lower_diag_merge_pin() {
        assert_eq!(
            o(-100, -100, 100, 100, -50, 60, 10, 50).normalize(),
            o(-20, -25, 55, 50, -50, 60, 10, 50)
        );
    }

    /// jshell pin (ceilHalf): (-50,-50,50,50,-60,40,-40,60) — only the
    /// lower diag merge (ly -50 -> floor((-40-40)/2) = -40) fires.
    #[test]
    fn normalize_ceil_floor_half_pin() {
        assert_eq!(
            o(-50, -50, 50, 50, -60, 40, -40, 60).normalize(),
            o(-50, -40, 50, 50, -60, 40, -40, 60)
        );
    }

    /// jshell pin (s1): (-20,0,20,10,-10,10,0,40) — step 1 (lx -20 -> -10)
    /// and step 11 (urx 40 -> 30).
    #[test]
    fn normalize_step_1_and_11_pin() {
        assert_eq!(
            o(-20, 0, 20, 10, -10, 10, 0, 40).normalize(),
            o(-5, 0, 20, 10, -10, 10, 0, 30)
        );
    }

    /// jshell pin (s4): (0,0,150,85,-50,10,0,100) — step 4 (rx 150 -> 95)
    /// then diagRightX = ceil((100+10)/2) = 55 pulls rx to 55.
    #[test]
    fn normalize_step_4_pin() {
        assert_eq!(
            o(0, 0, 150, 85, -50, 10, 0, 100).normalize(),
            o(0, 0, 55, 75, -50, 10, 0, 100)
        );
    }

    /// jshell pin (s567): (0,-50,20,50,-20,10,15,30) — steps 5 (ly -50 ->
    /// -10), 6 (ly -> -15), 7 (uy 50 -> 30), the upper merge (uy -> 25) and
    /// lower merge (ly -> 2).
    #[test]
    fn normalize_steps_5_6_7_pin() {
        assert_eq!(
            o(0, -50, 20, 50, -20, 10, 15, 30).normalize(),
            o(0, 2, 20, 25, -20, 10, 15, 30)
        );
    }

    /// jshell pin (s8): (0,0,20,5,18,30,0,25) — step 2 (lx 0 -> 18), step 8
    /// (uy 5 -> 2), step 9 (llx 0 -> 18 reading the ALREADY-mutated lx,
    /// T7), step 10 (lrx 30 -> 20), step 11 (urx 25 -> 22).
    #[test]
    fn normalize_steps_8_9_10_pin() {
        assert_eq!(
            o(0, 0, 20, 5, 18, 30, 0, 25).normalize(),
            o(18, 0, 20, 2, 18, 20, 18, 22)
        );
    }

    /// jshell pin (s9): (0,0,20,20,-10,10,-5,25) — step 9 (llx -5 -> 0) and
    /// diagRightX = ceil((25+10)/2) = 18 (rx 20 -> 18) plus the upper merge
    /// (uy 20 -> 18).
    #[test]
    fn normalize_step_9_pin() {
        assert_eq!(
            o(0, 0, 20, 20, -10, 10, -5, 25).normalize(),
            o(0, 0, 18, 18, -10, 10, 0, 25)
        );
    }

    /// jshell pin (s10): (0,10,20,20,-10,15,0,30) — step 9 (llx 0 -> 10)
    /// and step 10 (lrx 15 -> 10).
    #[test]
    fn normalize_step_10_pin() {
        assert_eq!(
            o(0, 10, 20, 20, -10, 15, 0, 30).normalize(),
            o(0, 10, 20, 20, -10, 10, 10, 30)
        );
    }

    /// jshell pins: an already-normalized octagon is a fixed point
    /// (identity case); `inner.isNormalized()` is false but its
    /// normalization is normalized.
    #[test]
    fn normalize_identity_and_is_normalized() {
        let a = oct();
        assert_eq!(a.normalize(), a);
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert!(!inner.is_normalized());
        let norm = inner.normalize();
        assert_eq!(norm, o(2, 1, 8, 7, 1, 7, 3, 15));
        assert!(norm.is_normalized());
        assert!(a.is_normalized());
        assert!(IntOctagon::EMPTY.is_normalized());
    }

    /// jshell pin (emptyProd): normalize((0,0,10,10,3,9,15,7)) returns THE
    /// EMPTY object in Java (`== IntOctagon.EMPTY` is true) — the pre-check
    /// `lowerLeftDiagonalX > upperRightDiagonalX` (15 > 7) fires.
    #[test]
    fn normalize_empty_producing_pin() {
        let result = o(0, 0, 10, 10, 3, 9, 15, 7).normalize();
        assert_eq!(result, IntOctagon::EMPTY);
        // jshell pin (G): this input is NOT empty — the steps only tighten
        // to (4,0,10,7,3,9,5,17); an earlier "post-merge EMPTY" claim for it
        // was a wrong hand trace
        assert_eq!(
            o(0, 0, 10, 10, 3, 9, 5, 20).normalize(),
            o(4, 0, 10, 7, 3, 9, 5, 17)
        );
        // jshell pin (H): the post-merge check (lx > rx after the diag
        // merges) CAN fire: (0,0,2,10,3,4,0,6) collapses to EMPTY
        assert_eq!(o(0, 0, 2, 10, 3, 4, 0, 6).normalize(), IntOctagon::EMPTY);
    }

    /// jshell pins (bigA/bigIntersect): wrapping int arithmetic in the
    /// normalize steps collapses the huge octagon to EMPTY; the raw
    /// intersection of the two big octagons wraps in lowerRightDiagonalX
    /// (2000000000 and -1999999990 both clamp to min = -1999999990) and the
    /// normalized result is EMPTY.
    #[test]
    fn normalize_and_intersection_wrap_like_java() {
        let big_a = o(
            -2_000_000_000,
            -2_000_000_000,
            2_000_000_000,
            2_000_000_000,
            -1_999_999_990,
            -1_999_999_990,
            1_999_999_990,
            1_999_999_990,
        );
        assert_eq!(big_a.normalize(), IntOctagon::EMPTY);
        let big_b = o(
            1_999_999_995,
            1_999_999_995,
            2_000_000_000,
            2_000_000_000,
            1_999_999_985,
            2_000_000_000,
            2_000_000_000,
            2_000_000_005,
        );
        // raw pre-normalize intersection fields (jshell bigIntersectRaw)
        let raw = o(
            big_a.left_x.max(big_b.left_x),
            big_a.bottom_y.max(big_b.bottom_y),
            big_a.right_x.min(big_b.right_x),
            big_a.top_y.min(big_b.top_y),
            big_a.upper_left_diagonal_x.max(big_b.upper_left_diagonal_x),
            big_a
                .lower_right_diagonal_x
                .min(big_b.lower_right_diagonal_x),
            big_a.lower_left_diagonal_x.max(big_b.lower_left_diagonal_x),
            big_a
                .upper_right_diagonal_x
                .min(big_b.upper_right_diagonal_x),
        );
        assert_eq!(raw.lower_right_diagonal_x, -1_999_999_990);
        assert_eq!(big_a.intersection(&big_b), IntOctagon::EMPTY);
    }

    /// jshell pin (touch): the edge-touching intersection keeps a degenerate
    /// (zero-width) octagon, NOT the sentinel.
    #[test]
    fn intersection_touching_pin() {
        let touch = oct().intersection(&o(10, 0, 20, 10, 0, 20, 10, 30));
        assert_eq!(touch, o(10, 0, 10, 10, 0, 10, 10, 20));
        assert!(!touch.is_empty());
    }

    /// jshell pin: `inner.normalize()` is a fixed point (idempotence), and
    /// the intersection of `oct` with itself is itself.
    #[test]
    fn intersection_fixed_points() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert_eq!(oct().intersection(&oct()), oct());
        assert_eq!(oct().intersection(&inner), inner.normalize());
        assert_eq!(oct().intersection(&IntOctagon::EMPTY), IntOctagon::EMPTY);
        assert_eq!(IntOctagon::EMPTY.intersection(&oct()), IntOctagon::EMPTY);
    }

    /// Union is raw min/max without normalization (Java `union`).
    #[test]
    fn union_raw_min_max() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert_eq!(oct().union(&inner), oct());
        // ulx takes the min: -10 vs 1; lrx the max: 10 vs 9 — an unnormalized
        // result, faithfully kept raw
        let u = inner.union(&o(30, 30, 40, 40, 20, 50, 40, 60));
        assert_eq!(u, o(2, 1, 40, 40, 1, 50, 3, 60));
        assert!(!u.is_normalized());
        // jshell pin (F): the diagonals take the raw min/max against the
        // box's octagon conversion (lrx max(10,30)=30, urx max(20,35)=35)
        assert_eq!(
            oct().union_box(&IntBox::from_corners(20, 0, 30, 5)),
            o(0, 0, 30, 10, -10, 30, 0, 35)
        );
    }

    /// Property: normalize is idempotent (normalize(normalize(x)) ==
    /// normalize(x)) on pseudo-random octagons (seeded via the in-crate
    /// JavaRandom — not the rand crate).
    #[test]
    fn normalize_is_idempotent_on_random_octagons() {
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_5678);
        for _ in 0..10_000 {
            let a = o(
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
                rng.next_int() % 64,
            );
            let once = a.normalize();
            assert_eq!(once.normalize(), once, "not idempotent for {a:?}");
        }
        // also with huge wrapping-prone coordinates
        for _ in 0..10_000 {
            let a = o(
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
                rng.next_int(),
            );
            let once = a.normalize();
            assert_eq!(once.normalize(), once, "not idempotent for {a:?}");
        }
    }

    /// Invariant (Task 10): IntOctagon.intersection is commutative — both
    /// operand orders run the same per-field min/max construction on
    /// normalized octagons, so the results are field-equal. 10k seeded
    /// pairs; every 4th pair uses full-range coordinates where the
    /// i32-wrapping field updates actually bite (small bands would let
    /// the interesting cases cancel — cerebrum pin rule 2).
    #[test]
    fn intersection_is_commutative_on_random_octagons() {
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_5679);
        let coord = |rng: &mut JavaRandom| {
            if rng.next_int_bound(4) == 0 {
                rng.next_int()
            } else {
                rng.next_int_bound(256) - 128
            }
        };
        for _ in 0..10_000 {
            let a = o(
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
            )
            .normalize();
            let b = o(
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
                coord(&mut rng),
            )
            .normalize();
            assert_eq!(
                a.intersection(&b),
                b.intersection(&a),
                "intersection not commutative for {a:?} x {b:?}"
            );
        }
        // Deliberate sentinel probe: the shared EMPTY octagon must
        // behave symmetrically too.
        let nonempty = o(-10, -10, 10, 10, -10, 10, -10, 10).normalize();
        assert_eq!(
            IntOctagon::EMPTY.intersection(&nonempty),
            nonempty.intersection(&IntOctagon::EMPTY),
            "EMPTY-sentinel intersection not commutative"
        );
    }

    /// jshell pins (offset25/offsetM25/offsetBig): offset(2.5) rounds the
    /// width to 3 (Math.round ties toward +infinity); offset(-2.5) to -2;
    /// offset(3.0e9) wraps in the (int) long cast and normalizes to EMPTY.
    #[test]
    fn offset_pins() {
        assert_eq!(oct().offset(2.5), o(-3, -3, 13, 13, -14, 14, -4, 24));
        assert_eq!(oct().offset(-2.5), o(2, 2, 8, 8, -6, 6, 4, 16));
        assert_eq!(oct().offset(2.4), o(-2, -2, 12, 12, -13, 13, -3, 23));
        assert_eq!(oct().offset(0.0), oct());
        assert_eq!(oct().enlarge(2.5), oct().offset(2.5));
        assert_eq!(oct().offset(3.0e9), IntOctagon::EMPTY);
    }

    /// jshell pin: IntBox(0,0,10,10).enlarge(2.5) equals the same octagon.
    #[test]
    fn containment_pins() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert!(oct().is_contained_in(&oct()));
        assert!(inner.normalize().is_contained_in(&oct()));
        assert!(!oct().is_contained_in(&inner));
        assert!(oct().is_contained_in_box(&IntBox::from_corners(0, 0, 10, 10)));
        assert!(!oct().is_contained_in_box(&IntBox::from_corners(0, 0, 9, 10)));
        assert!(IntOctagon::EMPTY.is_contained_in(&oct()));
        assert!(!oct().is_contained_in(&IntOctagon::EMPTY));
    }

    #[test]
    fn translate_by_int_vector() {
        assert_eq!(
            oct().translate_by(&Vector::Int(IntVector::new(1, 2))),
            o(1, 2, 11, 12, -11, 9, 3, 23)
        );
        assert_eq!(oct().translate_by(&Vector::ZERO), oct());
    }

    #[test]
    #[should_panic(expected = "expected an int vector")]
    fn translate_by_panics_on_rational_vector() {
        let rational = Vector::rational(RationalVector::new(
            BigInt::from(1),
            BigInt::from(0),
            BigInt::from(3),
        ));
        let _ = oct().translate_by(&rational);
    }

    /// jshell pins (cutB, via reflection on the package-private
    /// cutoutFrom(IntBox)): pieces of IntBox(0,0,10,10) minus the cutter
    /// inner = (2,1,8,7,1,9,3,15). Result order: 4 boxes (left, right,
    /// lower, upper), then 4 octagons (upper-left, lower-left,
    /// lower-right, upper-right).
    #[test]
    fn cutout_from_box_pin() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        let d = IntBox::from_corners(0, 0, 10, 10);
        assert_eq!(
            inner.cutout_from_box(&d),
            vec![
                o(0, 0, 2, 1, -1, 2, 0, 3),
                o(8, 0, 10, 7, 1, 10, 8, 17),
                o(2, 0, 8, 1, 1, 8, 2, 9),
                o(8, 7, 10, 10, -2, 3, 15, 20),
                o(0, 1, 8, 10, -10, 1, 1, 18),
                o(2, 0, 2, 1, 1, 2, 2, 3),
                o(8, 1, 8, 1, 7, 7, 9, 9),
                o(8, 7, 10, 10, -2, 3, 15, 20),
            ]
        );
        // jshell pin (cutC): when the cutter oct() covers the box exactly,
        // the 8 pieces are the degenerate border strips (NOT a single
        // piece) — an earlier single-piece expectation was wrong.
        assert_eq!(
            oct().cutout_from_box(&d),
            vec![
                o(0, 0, 0, 10, -10, 0, 0, 10),
                o(10, 0, 10, 10, 0, 10, 10, 20),
                o(0, 0, 10, 0, 0, 10, 0, 10),
                o(0, 10, 10, 10, -10, 0, 10, 20),
                o(0, 10, 0, 10, -10, -10, 10, 10),
                o(0, 0, 0, 0, 0, 0, 0, 0),
                o(10, 0, 10, 0, 10, 10, 10, 10),
                o(10, 10, 10, 10, 0, 0, 20, 20),
            ]
        );
        // empty cutter: single piece d.toIntOctagon()
        assert_eq!(
            IntOctagon::EMPTY.cutout_from_box(&d),
            vec![d.to_int_octagon()]
        );
    }

    /// jshell pin (cutO, via reflection): pieces of `oct` minus the cutter
    /// inner = (2,1,8,7,1,9,3,15); the "empty cutter: single piece d"
    /// return keeps `d` itself.
    #[test]
    fn cutout_from_octagon_pin() {
        let inner = o(2, 1, 8, 7, 1, 9, 3, 15);
        assert_eq!(
            oct().cutout_from(&inner),
            vec![
                o(2, 1, 2, 1, 1, 1, 3, 3),
                o(2, 1, 2, 1, 1, 1, 3, 3),
                o(2, 1, 8, 1, 1, 7, 3, 9),
                o(8, 1, 8, 1, 7, 7, 9, 9),
                o(8, 1, 8, 7, 1, 7, 9, 15),
                o(8, 7, 8, 7, 1, 1, 15, 15),
                o(8, 7, 8, 7, 1, 1, 15, 15),
                o(2, 1, 8, 7, 1, 1, 3, 15),
            ]
        );
        // jshell pin (cutO2): the flipped direction — pieces of `inner`
        // minus cutter `oct` — spans the argument d = inner's borders.
        assert_eq!(
            inner.cutout_from(&oct()),
            vec![
                o(0, 0, 2, 1, -1, 2, 0, 3),
                o(2, 0, 2, 1, 1, 2, 2, 3),
                o(2, 0, 8, 1, 1, 8, 2, 9),
                o(8, 1, 8, 1, 7, 7, 9, 9),
                o(8, 0, 10, 7, 1, 10, 8, 17),
                o(8, 7, 8, 7, 1, 1, 15, 15),
                o(8, 7, 10, 10, -2, 3, 15, 20),
                o(0, 1, 8, 10, -10, 1, 1, 18),
            ]
        );
        // empty cutter: single piece d
        assert_eq!(IntOctagon::EMPTY.cutout_from(&oct()), vec![oct()]);
    }

    /// jshell pin: IntPoint(3,-4).surroundingOctagon().
    #[test]
    fn int_point_surrounding_octagon_pin() {
        assert_eq!(
            p(3, -4).surrounding_octagon(),
            o(3, -4, 3, -4, 7, 7, -1, -1)
        );
        // the wrapping sites: extreme coordinates wrap like Java int math
        let extreme = p(i32::MAX, i32::MIN);
        assert_eq!(
            extreme.surrounding_octagon(),
            o(
                i32::MAX,
                i32::MIN,
                i32::MAX,
                i32::MIN,
                i32::MAX.wrapping_sub(i32::MIN),
                i32::MAX.wrapping_sub(i32::MIN),
                i32::MAX.wrapping_add(i32::MIN),
                i32::MAX.wrapping_add(i32::MIN),
            )
        );
    }

    /// jshell pin `PB2`: `octT.toSimplex()` — the 4 surviving border
    /// lines in direction order (E, NE, W, DOWN); the normalize-dropped
    /// edges and the collinear duplicate diagonals disappear.
    #[test]
    fn pb2_to_simplex_border_lines_pin() {
        let l = |ax: i32, ay: i32, bx: i32, by: i32| {
            crate::line::Line::new(
                crate::point::Point::int(crate::int_point::IntPoint::new(ax, ay)),
                crate::point::Point::int(crate::int_point::IntPoint::new(bx, by)),
            )
        };
        let simplex = TileShape::from_8_ints(2, -1, 12, 8, 2, 12, 0, 8).to_simplex();
        assert_eq!(simplex.border_line_count(), 4);
        assert_eq!(simplex.border_line(0), l(0, -1, 1, -1));
        assert_eq!(simplex.border_line(1), l(8, 0, 7, 1));
        assert_eq!(simplex.border_line(2), l(2, 0, 1, -1));
        assert_eq!(simplex.border_line(3), l(2, 0, 2, -1));
    }
}
