//! Port of Java `app.freerouting.geometry.planar.Line`.
//!
//! Bit-parity transliteration of the oracle at baseline `e7f9bdf1`. The
//! `dir` field is a memoized pure function of `(a, b)` (Java transient
//! field written by `getDirection()`); the [`OnceLock`] cache is allowed
//! by trap T14 (memoize pure functions only).
//!
//! Trap notes preserved here:
//! - `fastEquals` performs the determinant test **in doubles** on
//!   i32-wrapped delta components; products of coordinates near ±2^31
//!   round, so two lines whose exact cross product is ±2 can compare
//!   equal (pinned in tests, pin `F4`).
//! - `intersection` classifies 16 orientation pairs (delta1 × delta2
//!   for the four 45-degree orientations) in int arithmetic — 10 of
//!   them resolve there, 6 fall through to the BigInteger general
//!   formula (T6); the full table is on [`Line::intersection_point`].
//! - `equals` (the [`PartialEq`] impl) requires a POSITIVE projection,
//!   so opposite directions are UNEQUAL; `is_equal_or_opposite` only
//!   requires collinearity (T9 asymmetry).
//! - `sideOfIntersection` runs the float tolerance-1.0 ladder FIRST and
//!   re-checks exactly only on COLLINEAR (T8, Line.java:152-164).
//!
//! Java logs `FRLogger.warn` in the constructors for non-IntPoint input
//! and later throws `ClassCastException` in the int-arithmetic methods;
//! this port keeps the warning-free surface and panics deliberately at
//! the cast sites.
//!
//! Deferral ledger (Task 8 = Polyline):
//! - `isOnTheLeft` / `isOnTheRight(TileShape)` landed with Task 7.
//! - `middleApprox` / `changeLengthApprox` are **not present** on
//!   `Line.java` at the baseline; `changeLengthApprox` is a LineSegment
//!   method and is ported there.

use std::cmp::Ordering;
use std::sync::OnceLock;

use num_bigint::{BigInt, Sign};

use crate::big_int_aux::{big_abs, big_integer_int_value};
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::limits::CRIT_INT_BIG;
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::rounding::java_round;
use crate::side::Side;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Implements functionality for lines in the plane.
#[derive(Debug, Clone)]
pub struct Line {
    /// The first point of the directed line (Java public final field).
    /// Do not reassign after construction: `dir` caches the direction
    /// (mirrors the Java `final` modifier).
    pub a: Point,
    /// The second point of the directed line (Java public final field).
    /// Do not reassign after construction: `dir` caches the direction
    /// (mirrors the Java `final` modifier).
    pub b: Point,
    dir: OnceLock<Direction>,
}

/// The `(IntPoint)` casts Java performs freely; this port panics with the
/// Java intent instead of throwing `ClassCastException`.
fn int_pair(line: &Line) -> (&IntPoint, &IntPoint) {
    match (&line.a, &line.b) {
        (Point::Int(a), Point::Int(b)) => (a, b),
        _ => panic!("Line only implemented for IntPoints till now"),
    }
}

fn int_ref(point: &Point) -> &IntPoint {
    match point {
        Point::Int(p) => p,
        _ => panic!("Line only implemented for IntPoints till now"),
    }
}

impl Line {
    /// Creates a directed Line from two points (Java constructor).
    pub fn new(a: Point, b: Point) -> Line {
        Line {
            a,
            b,
            dir: OnceLock::new(),
        }
    }

    /// Creates a directed Line from four integer coordinates.
    pub fn from_int_coords(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::new(
            Point::int(IntPoint::new(ax, ay)),
            Point::int(IntPoint::new(bx, by)),
        )
    }

    /// Creates a directed Line from a point and a direction (Java
    /// constructor `Line(Point, Direction)`; it pre-seeds the direction
    /// cache, unlike [`Line::get_instance`]).
    pub fn new_with_direction(a: Point, dir: Direction) -> Line {
        let b = a.translate_by(&dir.get_vector());
        let line = Line {
            a,
            b,
            dir: OnceLock::new(),
        };
        let _ = line.dir.set(dir);
        line
    }

    /// Creates a directed line from a Point and a Direction (Java static
    /// `getInstance`; the direction cache is NOT pre-seeded).
    pub fn get_instance(a: Point, dir: Direction) -> Line {
        let b = a.translate_by(&dir.get_vector());
        Line::new(a, b)
    }

    /// Returns true if this and ob define the same line (Java `getId`).
    pub fn get_id(&self) -> i32 {
        31i32
            .wrapping_mul(self.a.get_id())
            .wrapping_add(self.b.get_id())
    }

    /// Returns true, if this and other define the same line. Designed for
    /// good performance, but works only for lines consisting of IntPoints
    /// (Java `fastEquals`: double determinant on i32-wrapped deltas).
    pub fn fast_equals(&self, other: &Line) -> bool {
        let (this_a, this_b) = int_pair(self);
        let other_a = int_ref(&other.a);
        // Java widens the (wrapping) int differences to double.
        let dx1 = other_a.x.wrapping_sub(this_a.x) as f64;
        let dy1 = other_a.y.wrapping_sub(this_a.y) as f64;
        let dx2 = this_b.x.wrapping_sub(this_a.x) as f64;
        let dy2 = this_b.y.wrapping_sub(this_a.y) as f64;
        let det = dx1 * dy2 - dx2 * dy1;
        if det != 0.0 {
            return false;
        }
        self.direction() == other.direction()
    }

    /// Gets the direction of this directed line (memoized pure function).
    pub fn direction(&self) -> &Direction {
        self.dir
            .get_or_init(|| Direction::get_instance(&self.b.difference_by(&self.a)))
    }

    /// The function returns Side::Positive (Java ON_THE_LEFT), if this
    /// Line is on the left of point, Side::Negative if on the right, and
    /// Side::Collinear, if this Line contains point.
    pub fn side_of(&self, point: &Point) -> Side {
        // Java: point.sideOf(this).negate().
        point.side_of(&self.a, &self.b).negate()
    }

    /// Returns Side::Collinear, if point is on the line with tolerance
    /// `tolerance`; otherwise the side of the line relative to the point
    /// (Java `sideOf(FloatPoint, double)`).
    pub fn side_of_float(&self, point: &FloatPoint, tolerance: f64) -> Side {
        let (this_a, this_b) = int_pair(self);
        let det = (this_b.y.wrapping_sub(this_a.y)) as f64 * (point.x - this_a.x as f64)
            - (this_b.x.wrapping_sub(this_a.x)) as f64 * (point.y - this_a.y as f64);
        if det - tolerance > 0.0 {
            Side::Positive
        } else if det + tolerance < 0.0 {
            Side::Negative
        } else {
            Side::Collinear
        }
    }

    /// Java `sideOf(FloatPoint)` — the tolerance-0 overload.
    pub fn side_of_float_zero(&self, point: &FloatPoint) -> Side {
        self.side_of_float(point, 0.0)
    }

    /// Returns the side of this line relative to the intersection of p1
    /// and p2: Side::Collinear only if all 3 lines intersect in exactly 1
    /// point. The float tolerance-1.0 check runs first; the exact check
    /// runs only on COLLINEAR (T8). Java feeds `p1.intersection(p2)`
    /// onward verbatim — for a parallel pair that is an infinite
    /// RationalPoint, whose side is computed by the exact rational
    /// arithmetic of [`Point::side_of`]; the port does the same via
    /// [`Line::intersection_point`].
    pub fn side_of_intersection(&self, p1: &Line, p2: &Line) -> Side {
        let intersection_approx = p1.intersection_approx(p2);
        let result = self.side_of_float(&intersection_approx, 1.0);
        if result == Side::Collinear {
            // Previous calculation was with FloatPoints and a tolerance
            // for performance reasons. Make an exact check for
            // collinearity now with class Point instead of FloatPoint.
            self.side_of(&p1.intersection_point(p2))
        } else {
            result
        }
    }

    /// Returns the signed distance of this line from point. Positive, if
    /// the line is on the left of the point, else negative.
    pub fn signed_distance(&self, point: &FloatPoint) -> f64 {
        let (this_a, this_b) = int_pair(self);
        let dx = this_b.x.wrapping_sub(this_a.x) as f64;
        let dy = this_b.y.wrapping_sub(this_a.y) as f64;
        let det = dy * (point.x - this_a.x as f64) - dx * (point.y - this_a.y as f64);
        // area of the parallelogramm spanned by the 3 points
        let length = (dx * dx + dy * dy).sqrt();
        det / length
    }

    /// Returns true if the two lines define the same set of points, but
    /// may have opposite directions (Java `overlaps`).
    pub fn overlaps(&self, other: &Line) -> bool {
        self.side_of(&other.a) == Side::Collinear && self.side_of(&other.b) == Side::Collinear
    }

    /// Returns true if this and other define the same set of points, but
    /// may have opposite directions (Java `isEqualOrOpposite`; same
    /// predicate as [`Line::overlaps`], kept as a separate ported name).
    pub fn is_equal_or_opposite(&self, other: &Line) -> bool {
        self.side_of(&other.a) == Side::Collinear && self.side_of(&other.b) == Side::Collinear
    }

    /// Returns the line defining the same set of points, but with
    /// opposite direction.
    pub fn opposite(&self) -> Line {
        Line::new(self.b.clone(), self.a.clone())
    }

    /// Returns the intersection point of the 2 lines, or `None` if the
    /// lines are parallel (Java returns an infinite RationalPoint; the
    /// corpus convention serializes both as `{"kind":"Infinity"}`).
    /// Java-verbatim behavior lives on [`Line::intersection_point`].
    pub fn intersection(&self, other: &Line) -> Option<Point> {
        let p = self.intersection_point(other);
        if p.is_infinite() { None } else { Some(p) }
    }

    /// Java `Line.intersection` verbatim: returns the intersection point,
    /// an **infinite RationalPoint (z = 0)** for parallel/collinear pairs.
    /// Java never returns null here, and its callers (`LineSegment`
    /// corner points, `sideOfIntersection`) keep computing through the
    /// infinity with exact RationalPoint cross products (trap T22, corpus
    /// cases seg-000020/93/125/151).
    ///
    /// The 16 fast paths cover every pair of 45-degree orientations
    /// (delta1 = b - a of this, delta2 = b - a of other), classified in
    /// Java's exact branch order (Line.java:225-275): 10 pairs resolve
    /// in int arithmetic, 6 fall through to the BigInteger general
    /// formula (T6); the full table is on [`Line::intersection_point`].
    ///
    /// | #  | delta1      | delta2      | result                              |
    /// |----|-------------|-------------|-------------------------------------|
    /// | 1  | vertical    | horizontal  | (a.x, oa.y)                         |
    /// | 2  | vertical    | right diag  | (a.x, oa.y + a.x - oa.x)            |
    /// | 3  | vertical    | left diag   | (a.x, oa.y + oa.x - a.x)            |
    /// | 4  | horizontal  | vertical    | (oa.x, a.y)                         |
    /// | 5  | horizontal  | right diag  | (oa.x + a.y - oa.y, a.y)            |
    /// | 6  | horizontal  | left diag   | (oa.x + oa.y - a.y, a.y)            |
    /// | 7  | right diag  | vertical    | (oa.x, a.y + oa.x - a.x)            |
    /// | 8  | right diag  | horizontal  | (a.x + oa.y - a.y, oa.y)            |
    /// | 9  | left diag   | vertical    | (oa.x, a.y + a.x - oa.x)            |
    /// | 10 | left diag   | horizontal  | (a.x + a.y - oa.y, oa.y)            |
    /// | 11 | vertical    | vertical    | general formula (parallel)          |
    /// | 12 | horizontal  | horizontal  | general formula (parallel)          |
    /// | 13 | right diag  | right diag  | general formula (parallel)          |
    /// | 14 | right diag  | left diag   | general formula (exact division)    |
    /// | 15 | left diag   | left diag   | general formula (parallel)          |
    /// | 16 | left diag   | right diag  | general formula (exact division)    |
    ///
    /// General-slope lines always take the general formula.
    pub fn intersection_point(&self, other: &Line) -> Point {
        let (this_a, this_b) = int_pair(self);
        let (other_a, other_b) = int_pair(other);
        let delta1 = IntVector::new(
            this_b.x.wrapping_sub(this_a.x),
            this_b.y.wrapping_sub(this_a.y),
        );
        let delta2 = IntVector::new(
            other_b.x.wrapping_sub(other_a.x),
            other_b.y.wrapping_sub(other_a.y),
        );
        // Separate handling for orthogonal and 45 degree lines for better
        // performance
        if delta1.x == 0 {
            // this line is vertical
            if delta2.y == 0 {
                // other line is horizontal
                return Point::int(IntPoint::new(this_a.x, other_a.y));
            }
            if delta2.x == delta2.y {
                // other line is right diagonal
                let this_x = this_a.x;
                return Point::int(IntPoint::new(
                    this_x,
                    other_a.y.wrapping_add(this_x).wrapping_sub(other_a.x),
                ));
            }
            if delta2.x == delta2.y.wrapping_neg() {
                // other line is left diagonal
                let this_x = this_a.x;
                return Point::int(IntPoint::new(
                    this_x,
                    other_a.y.wrapping_add(other_a.x).wrapping_sub(this_x),
                ));
            }
        } else if delta1.y == 0 {
            // this line is horizontal
            if delta2.x == 0 {
                // other line is vertical
                return Point::int(IntPoint::new(other_a.x, this_a.y));
            }
            if delta2.x == delta2.y {
                // other line is right diagonal
                let this_y = this_a.y;
                return Point::int(IntPoint::new(
                    other_a.x.wrapping_add(this_y).wrapping_sub(other_a.y),
                    this_y,
                ));
            }
            if delta2.x == delta2.y.wrapping_neg() {
                // other line is left diagonal
                let this_y = this_a.y;
                return Point::int(IntPoint::new(
                    other_a.x.wrapping_add(other_a.y).wrapping_sub(this_y),
                    this_y,
                ));
            }
        } else if delta1.x == delta1.y {
            // this line is right diagonal
            if delta2.x == 0 {
                // other line is vertical
                let other_x = other_a.x;
                return Point::int(IntPoint::new(
                    other_x,
                    this_a.y.wrapping_add(other_x).wrapping_sub(this_a.x),
                ));
            }
            if delta2.y == 0 {
                // other line is horizontal
                let other_y = other_a.y;
                return Point::int(IntPoint::new(
                    this_a.x.wrapping_add(other_y).wrapping_sub(this_a.y),
                    other_y,
                ));
            }
        } else if delta1.x == delta1.y.wrapping_neg() {
            // this line is left diagonal
            if delta2.x == 0 {
                // other line is vertical
                let other_x = other_a.x;
                return Point::int(IntPoint::new(
                    other_x,
                    this_a.y.wrapping_add(this_a.x).wrapping_sub(other_x),
                ));
            }
            if delta2.y == 0 {
                // other line is horizontal
                let other_y = other_a.y;
                return Point::int(IntPoint::new(
                    this_a.x.wrapping_add(this_a.y).wrapping_sub(other_y),
                    other_y,
                ));
            }
        }

        let det1 = BigInt::from(this_a.determinant(this_b));
        let det2 = BigInt::from(other_a.determinant(other_b));
        let mut det = BigInt::from(delta2.determinant(&delta1));
        let mut tmp1 = &det1 * BigInt::from(delta2.x);
        let mut tmp2 = &det2 * BigInt::from(delta1.x);
        let mut is_x = tmp1 - tmp2;
        tmp1 = &det1 * BigInt::from(delta2.y);
        tmp2 = &det2 * BigInt::from(delta1.y);
        let mut is_y = tmp1 - tmp2;
        // Java shape kept for diffing: the signum gate is only ever false
        // for parallel pairs, which return RationalPoint(isX, isY, 0)
        // (never null — trap T22).
        if det.sign() != Sign::NoSign {
            if det.sign() == Sign::Minus {
                det = -det;
                is_x = -is_x;
                is_y = -is_y;
            }
            if (&is_x % &det).sign() == Sign::NoSign && (&is_y % &det).sign() == Sign::NoSign {
                is_x = &is_x / &det;
                is_y = &is_y / &det;
                // Java tests Math.abs(isX.doubleValue()) <= CRIT_INT; for
                // integer BigIntegers the double conversion is exact in
                // the decisive range, so the BigInt comparison is
                // bit-identical. Above 2^53 the conversion rounds with
                // relative error <= 2^-53, so any |v| >= 2^53 lands on a
                // magnitude >= 2^52 — still far above CRIT_INT — and
                // values beyond double range become +-Infinity, which
                // also compares Greater; the exact BigInt comparison
                // classifies every such value identically.
                if big_abs(&is_x).cmp(&*CRIT_INT_BIG) != Ordering::Greater
                    && big_abs(&is_y).cmp(&*CRIT_INT_BIG) != Ordering::Greater
                {
                    return Point::int(IntPoint::new(
                        big_integer_int_value(&is_x),
                        big_integer_int_value(&is_y),
                    ));
                }
                det = BigInt::ONE;
            }
        }
        Point::rational(RationalPoint::new(is_x, is_y, det))
    }

    /// Returns an approximation of the intersection of the 2 lines by a
    /// FloatPoint. If the lines are parallel the result coordinates are
    /// Integer.MAX_VALUE.
    pub fn intersection_approx(&self, other: &Line) -> FloatPoint {
        let (this_a, this_b) = int_pair(self);
        let (other_a, other_b) = int_pair(other);
        let d1x = this_b.x.wrapping_sub(this_a.x) as f64;
        let d1y = this_b.y.wrapping_sub(this_a.y) as f64;
        let d2x = other_b.x.wrapping_sub(other_a.x) as f64;
        let d2y = other_b.y.wrapping_sub(other_a.y) as f64;
        let det1 = this_a.x as f64 * this_b.y as f64 - this_a.y as f64 * this_b.x as f64;
        let det2 = other_a.x as f64 * other_b.y as f64 - other_a.y as f64 * other_b.x as f64;
        let det = d2x * d1y - d2y * d1x;
        if det == 0.0 {
            return FloatPoint::new(i32::MAX as f64, i32::MAX as f64);
        }
        FloatPoint::new(
            (d2x * det1 - d1x * det2) / det,
            (d2y * det1 - d1y * det2) / det,
        )
    }

    /// Returns the perpendicular projection of point onto this line
    /// (Java double dispatch `point.perpendicularProjection(this)`).
    pub fn perpendicular_projection(&self, point: &Point) -> Point {
        match point {
            Point::Int(p) => self.perpendicular_projection_int(p),
            Point::Rational(p) => self.perpendicular_projection_rational(p),
        }
    }

    /// Shared setup of the two projection algorithms: (v, vxvx, vyvy,
    /// vxvy, denominator, det) with `det = a.determinant(b)`.
    fn projection_terms(&self) -> (IntVector, BigInt, BigInt, BigInt, BigInt, BigInt) {
        let (line_a, line_b) = int_pair(self);
        let v = IntVector::new(
            line_b.x.wrapping_sub(line_a.x),
            line_b.y.wrapping_sub(line_a.y),
        );
        let vxvx = BigInt::from((v.x as i64) * (v.x as i64));
        let vyvy = BigInt::from((v.y as i64) * (v.y as i64));
        let vxvy = BigInt::from((v.x as i64) * (v.y as i64));
        let denominator = &vxvx + &vyvy;
        let det = BigInt::from(line_a.determinant(line_b));
        (v, vxvx, vyvy, vxvy, denominator, det)
    }

    /// `IntPoint.perpendicularProjection(Line)`. NOTE (bug-compatible):
    /// the projY numerator SUBTRACTS `det * vx`, and the integral result
    /// down-conversion has **no** CRIT_INT check (the `intValue()` cast
    /// wraps silently), exactly as in Java.
    fn perpendicular_projection_int(&self, point: &IntPoint) -> Point {
        let (v, vxvx, vyvy, vxvy, denominator, det) = self.projection_terms();
        let point_x = BigInt::from(point.x);
        let point_y = BigInt::from(point.y);

        let mut tmp1 = &vxvx * &point_x;
        let mut tmp2 = &vxvy * &point_y;
        tmp1 += tmp2;
        tmp2 = &det * BigInt::from(v.y);
        let mut proj_x = tmp1 + tmp2;

        tmp1 = &vxvy * &point_x;
        tmp2 = &vyvy * &point_y;
        tmp1 += tmp2;
        tmp2 = &det * BigInt::from(v.x);
        let mut proj_y = tmp1 - tmp2;

        let mut denominator = denominator;
        let signum = denominator.sign();
        if signum != Sign::NoSign {
            if signum == Sign::Minus {
                denominator = -denominator;
                proj_x = -proj_x;
                proj_y = -proj_y;
            }
            if (&proj_x % &denominator).sign() == Sign::NoSign
                && (&proj_y % &denominator).sign() == Sign::NoSign
            {
                proj_x = &proj_x / &denominator;
                proj_y = &proj_y / &denominator;
                return Point::int(IntPoint::new(
                    big_integer_int_value(&proj_x),
                    big_integer_int_value(&proj_y),
                ));
            }
        }
        Point::rational(RationalPoint::new(proj_x, proj_y, denominator))
    }

    /// `RationalPoint.perpendicularProjection(Line)`. NOTE (bug-compatible
    /// with the oracle): the projY numerator ADDS `det * vx * z`, while
    /// the IntPoint variant SUBTRACTS `det * vx` — the two Java
    /// algorithms disagree on y for the same affine input (pinned in
    /// tests, pins `PP1`/`PP2r`); the rational result additionally applies
    /// a CRIT_INT down-conversion with `denominator = 1` fallback.
    fn perpendicular_projection_rational(&self, point: &RationalPoint) -> Point {
        let (v, vxvx, vyvy, vxvy, denominator, det) = self.projection_terms();

        let mut tmp1 = &vxvx * &point.x;
        let mut tmp2 = &vxvy * &point.y;
        tmp1 += tmp2;
        tmp2 = &det * BigInt::from(v.y);
        tmp2 *= &point.z;
        let mut proj_x = tmp1 + tmp2;

        tmp1 = &vxvy * &point.x;
        tmp2 = &vyvy * &point.y;
        tmp1 += tmp2;
        tmp2 = &det * BigInt::from(v.x);
        tmp2 *= &point.z;
        let mut proj_y = tmp1 + tmp2;

        let mut denominator = denominator;
        let signum = denominator.sign();
        if signum != Sign::NoSign {
            if signum == Sign::Minus {
                denominator = -denominator;
                proj_x = -proj_x;
                proj_y = -proj_y;
            }
            if (&proj_x % &denominator).sign() == Sign::NoSign
                && (&proj_y % &denominator).sign() == Sign::NoSign
            {
                proj_x = &proj_x / &denominator;
                proj_y = &proj_y / &denominator;
                if big_abs(&proj_x).cmp(&*CRIT_INT_BIG) != Ordering::Greater
                    && big_abs(&proj_y).cmp(&*CRIT_INT_BIG) != Ordering::Greater
                {
                    return Point::int(IntPoint::new(
                        big_integer_int_value(&proj_x),
                        big_integer_int_value(&proj_y),
                    ));
                }
                denominator = BigInt::ONE;
            }
        }
        Point::rational(RationalPoint::new(proj_x, proj_y, denominator))
    }

    /// Translates the line perpendicular by dist. If dist > 0, the line
    /// is translated to the left; otherwise to the right.
    pub fn translate(&self, dist: f64) -> Line {
        let (ai, _) = int_pair(self);
        let v = match self.direction().get_vector() {
            Vector::Int(v) => v,
            _ => panic!("Line.translate only implemented for IntPoints till now"),
        };
        let vxvx = v.x as f64 * v.x as f64;
        let vyvy = v.y as f64 * v.y as f64;
        let length = (vxvx + vyvy).sqrt();
        let new_a = if vxvx <= vyvy {
            // translate along the x axis
            let rel_x = java_round(dist * length / v.y as f64) as i32;
            IntPoint::new(ai.x.wrapping_sub(rel_x), ai.y)
        } else {
            // translate along the y axis
            let rel_y = java_round(dist * length / v.x as f64) as i32;
            IntPoint::new(ai.x, ai.y.wrapping_add(rel_y))
        };
        Line::get_instance(Point::int(new_a), self.direction().clone())
    }

    /// Translates the line by vector (Java `translateBy`).
    pub fn translate_by(&self, vector: &Vector) -> Line {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        Line::new(self.a.translate_by(vector), self.b.translate_by(vector))
    }

    /// Returns true if the line is axis-parallel.
    pub fn is_orthogonal(&self) -> bool {
        self.direction().is_orthogonal()
    }

    /// Returns true if this line is diagonal.
    pub fn is_diagonal(&self) -> bool {
        self.direction().is_diagonal()
    }

    /// Returns true if the direction of this line is a multiple of 45
    /// degrees.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.direction().is_multiple_of_45_degree()
    }

    /// Checks if this line and other are parallel.
    pub fn is_parallel(&self, other: &Line) -> bool {
        self.direction().side_of(other.direction()) == Side::Collinear
    }

    /// Checks if this line and other are perpendicular.
    pub fn is_perpendicular(&self, other: &Line) -> bool {
        let v1 = self.direction().get_vector();
        let v2 = other.direction().get_vector();
        v1.projection(&v2) == Side::Collinear
    }

    /// Calculates the cosine of the angle between this line and other.
    pub fn cos_angle(&self, other: &Line) -> f64 {
        self.b
            .difference_by(&self.a)
            .cos_angle(&other.b.difference_by(&other.a))
    }

    /// A line l1 is defined bigger than a line l2, if the direction of l1
    /// is bigger than the direction of l2 (Java `compareTo`; fast
    /// implementation for IntPoint lines).
    pub fn compare_to(&self, other: &Line) -> Ordering {
        let (this_a, this_b) = int_pair(self);
        let (other_a, other_b) = int_pair(other);
        let dx1 = this_b.x.wrapping_sub(this_a.x);
        let dy1 = this_b.y.wrapping_sub(this_a.y);
        let dx2 = other_b.x.wrapping_sub(other_a.x);
        let dy2 = other_b.y.wrapping_sub(other_a.y);
        if dy1 > 0 {
            if dy2 < 0 {
                return Ordering::Less;
            }
            if dy2 == 0 {
                if dx2 > 0 {
                    return Ordering::Greater;
                }
                return Ordering::Less;
            }
        } else if dy1 < 0 {
            if dy2 >= 0 {
                return Ordering::Greater;
            }
        } else {
            // dy1 == 0
            if dx1 > 0 {
                if dy2 != 0 || dx2 < 0 {
                    return Ordering::Less;
                }
                return Ordering::Equal;
            }
            // dx1 < 0
            if dy2 > 0 || (dy2 == 0 && dx2 > 0) {
                return Ordering::Greater;
            }
            if dy2 < 0 {
                return Ordering::Less;
            }
            return Ordering::Equal;
        }

        // now this direction and other are located in the same
        // open horizontal half plane

        let determinant = dx2 as f64 * dy1 as f64 - dy2 as f64 * dx1 as f64;
        match Side::as_int(determinant) {
            1 => Ordering::Greater,
            -1 => Ordering::Less,
            _ => Ordering::Equal,
        }
    }

    /// Calculates an approximation of the function value of this line at
    /// x, if the line is not vertical.
    pub fn function_value_approx(&self, x: f64) -> f64 {
        let p1 = self.a.to_float();
        let p2 = self.b.to_float();
        let dx = p2.x - p1.x;
        if dx == 0.0 {
            // Java logs "function_value_approx: line is vertical".
            return 0.0;
        }
        let dy = p2.y - p1.y;
        let det = p1.x * p2.y - p2.x * p1.y;
        (dy * x - det) / dx
    }

    /// Calculates an approximation of the function value in x of this
    /// line at y, if the line is not horizontal (Java
    /// `functionInYValueApprox`).
    pub fn function_in_y_value_approx(&self, y: f64) -> f64 {
        let p1 = self.a.to_float();
        let p2 = self.b.to_float();
        let dy = p2.y - p1.y;
        if dy == 0.0 {
            // Java logs "function_in_y_value_approx: line is horizontal".
            return 0.0;
        }
        let dx = p2.x - p1.x;
        let det = p1.x * p2.y - p2.x * p1.y;
        (dx * y + det) / dy
    }

    /// Calculates the direction from fromPoint to the nearest point on
    /// this line; `None`, if fromPoint is contained in this line (Java
    /// returns null).
    pub fn perpendicular_direction(&self, from_point: &Point) -> Option<Direction> {
        let line_side = self.side_of(from_point);
        if line_side == Side::Collinear {
            return None;
        }
        let dir1 = self.direction().turn_45_degree(2);
        let dir2 = self.direction().turn_45_degree(6);

        let check_point1 = from_point.translate_by(&dir1.get_vector());
        if self.side_of(&check_point1) != line_side {
            return Some(dir1);
        }
        let check_point2 = from_point.translate_by(&dir2.get_vector());
        if self.side_of(&check_point2) != line_side {
            return Some(dir2);
        }
        let nearest_line_point = from_point.to_float().projection_approx(self);
        if nearest_line_point.distance_square(&check_point1.to_float())
            <= nearest_line_point.distance_square(&check_point2.to_float())
        {
            Some(dir1)
        } else {
            Some(dir2)
        }
    }

    /// Turns this line by factor times 90 degree around pole.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Line {
        let pole = Point::int(*pole);
        Line::new(
            self.a.turn_90_degree(factor, &pole),
            self.b.turn_90_degree(factor, &pole),
        )
    }

    /// Mirrors this line at the vertical line through pole (note the a/b
    /// swap, as in Java).
    pub fn mirror_vertical(&self, pole: &IntPoint) -> Line {
        let pole = Point::int(*pole);
        Line::new(self.b.mirror_vertical(&pole), self.a.mirror_vertical(&pole))
    }

    /// Mirrors this line at the horizontal line through pole (note the
    /// a/b swap, as in Java).
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Line {
        let pole = Point::int(*pole);
        Line::new(
            self.b.mirror_horizontal(&pole),
            self.a.mirror_horizontal(&pole),
        )
    }

    /// Returns the Euclidean length of this line: the dx*dx + dy*dy sum
    /// is computed in **wrapping i32** exactly as in Java
    /// (Line.java:561-566), then widened to f64 for the sqrt and narrowed
    /// to f32 for the return.
    pub fn length(&self) -> f32 {
        let (ipa, ipb) = int_pair(self);
        let dx = ipb.x.wrapping_sub(ipa.x);
        let dy = ipb.y.wrapping_sub(ipa.y);
        let sum = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        (sum as f64).sqrt() as f32
    }
}

/// Java `equals`: both endpoints collinear and a POSITIVE projection, so
/// opposite directions are unequal (T9).
impl PartialEq for Line {
    fn eq(&self, other: &Line) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        if self.side_of(&other.a) != Side::Collinear {
            return false;
        }
        if self.side_of(&other.b) != Side::Collinear {
            return false;
        }
        let dir1 = self.b.difference_by(&self.a);
        let dir2 = other.b.difference_by(&other.a);
        dir1.projection(&dir2) == Side::Positive
    }
}

impl Eq for Line {}

// ---------------------------------------------------------------------------
// Task 7 closures (TileShape family)
// ---------------------------------------------------------------------------

impl Line {
    /// Returns true if all interior points of the tile are on the left
    /// side of this line (Java `Line.isOnTheLeft(TileShape)`).
    pub fn is_on_the_left(&self, tile: &TileShape) -> bool {
        for i in 0..tile.border_line_count() as i32 {
            if self.side_of(&tile.corner(i)) == Side::Negative {
                return false;
            }
        }
        true
    }

    /// Returns true if all interior points of the tile are on the right
    /// side of this line (Java `Line.isOnTheRight(TileShape)`).
    pub fn is_on_the_right(&self, tile: &TileShape) -> bool {
        for i in 0..tile.border_line_count() as i32 {
            if self.side_of(&tile.corner(i)) == Side::Positive {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::side::Side;
    use num_bigint::BigInt;

    fn int(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    fn line(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::from_int_coords(ax, ay, bx, by)
    }

    fn rat(x: i64, y: i64, z: i64) -> Point {
        Point::rational(RationalPoint::new(
            BigInt::from(x),
            BigInt::from(y),
            BigInt::from(z),
        ))
    }

    /// jshell pins `L1`-`L9`, `L11`, `L13`, `P3`/`P9`: every int-arithmetic
    /// fast path of `intersection` (see the table on
    /// [`Line::intersection_point`]) plus the general-formula integral and
    /// rational outcomes, captured from the oracle jar. P3/P9 exercise
    /// paths 3/9 with a.x != oa.x so the coordinate terms cannot cancel.
    #[test]
    fn intersection_fast_paths_and_general_formula() {
        let lv = line(0, 0, 0, 10); // vertical x = 0
        let lh = line(2, 5, 12, 5); // horizontal y = 5
        let lr = line(2, 0, 12, 10); // right diagonal y = x - 2
        let ll = line(0, 4, 4, 0); // left diagonal x + y = 4
        // Path 1 vertical x horizontal.
        assert_eq!(lv.intersection(&lh), Some(int(0, 5))); // L1
        // Path 2 vertical x right diagonal.
        assert_eq!(lv.intersection(&lr), Some(int(0, -2))); // L2
        // Path 3 vertical x left diagonal.
        assert_eq!(lv.intersection(&ll), Some(int(0, 4))); // L6
        // Path 4 horizontal x vertical.
        assert_eq!(lh.intersection(&lv), Some(int(0, 5))); // L5
        // Path 5 horizontal x right diagonal.
        assert_eq!(lh.intersection(&lr), Some(int(7, 5))); // L3
        // Path 6 horizontal x left diagonal.
        assert_eq!(lh.intersection(&ll), Some(int(-1, 5))); // L9 (symmetry)
        // Path 7 right diagonal x vertical.
        assert_eq!(lr.intersection(&lv), Some(int(0, -2))); // L4
        // Path 8 right diagonal x horizontal.
        assert_eq!(lr.intersection(&lh), Some(int(7, 5))); // L8
        // Path 9 left diagonal x vertical.
        assert_eq!(ll.intersection(&lv), Some(int(0, 4))); // L7
        // Path 10 left diagonal x horizontal.
        assert_eq!(ll.intersection(&lh), Some(int(-1, 5))); // L9
        // Paths 3/9 NON-degenerate (a.x = 5 != oa.x = 0, so the
        // +-oa.x / +-a.x terms matter): pins P3/P9. The degenerate
        // lv x ll cases above cancel those terms; a sign-swapped
        // add/sub there would still give (5, 9) here.
        let lv5 = line(5, 0, 5, 10);
        assert_eq!(lv5.intersection(&ll), Some(int(5, -1))); // P3
        assert_eq!(ll.intersection(&lv5), Some(int(5, -1))); // P9
        // Paths 13/14 fall through to the general formula: 45 x 45.
        assert_eq!(lr.intersection(&ll), Some(int(3, 1))); // L11 (integral)
        // General formula with a non-integral intersection.
        let lg1 = line(0, 0, 4, 2);
        let lg2 = line(0, 3, 4, 0);
        assert_eq!(lg1.intersection(&lg2), Some(rat(48, 24, 20))); // L13
    }

    /// jshell pins `L10`/`L12`: parallel and collinear lines hit the
    /// general formula with det = 0 and return the infinite point
    /// Rat(0, -270, 0) / Rat(0, 0, 0); this port maps them to None.
    #[test]
    fn parallel_and_collinear_intersections_are_none() {
        let lv = line(0, 0, 0, 10);
        assert_eq!(lv.intersection(&line(3, 0, 3, 9)), None); // L10
        assert_eq!(line(0, 0, 2, 2).intersection(&line(4, 4, 8, 8)), None); // L12
    }

    /// jshell pin `INF1` (trap T22, corpus case seg-000020): the general
    /// formula on a parallel pair returns the infinite RationalPoint
    /// Rat(isX, isY, 0) — never null. `intersection_point` hands it on
    /// verbatim (LineSegment corner points compute through it); the
    /// [`Line::intersection`] wrapper maps it to None for the corpus's
    /// Infinity encoding.
    #[test]
    fn parallel_intersection_is_infinite_rational_point() {
        let m = line(-119870338, -4932, -119873483, -8077);
        let s = line(29065462, -5356, 29067219, -3599);
        let expected = Point::rational(RationalPoint::new(
            BigInt::from(822986573811360_i64),
            BigInt::from(822986573811360_i64),
            BigInt::ZERO,
        ));
        assert_eq!(m.intersection_point(&s), expected); // INF1
        assert!(m.intersection_point(&s).is_infinite());
        assert_eq!(m.intersection(&s), None);
    }

    /// jshell pin `L14`: coordinates near +-2^31 force intermediate
    /// products beyond 64 bits (det1 * delta2.x ~ 2.5e27), so the
    /// num-bigint general formula is mandatory. Values verbatim from the
    /// oracle.
    #[test]
    fn intersection_bigint_general_formula() {
        let l1 = line(-2147483647, 0, 1073741823, 1073741823);
        let l2 = line(0, -1073741823, 1073741823, 0);
        let expected = Point::rational(RationalPoint::new(
            BigInt::from(-1237940033520772759381082109_i128),
            BigInt::from(-3713820108632768795358789630_i128),
            BigInt::from(2305843008139952127_i128),
        ));
        assert_eq!(l1.intersection(&l2), Some(expected)); // L14
    }

    /// jshell pins `T1`-`T4`: the tolerance ladder of
    /// `sideOfIntersection`. The float stage (tolerance 1.0) reports
    /// COLLINEAR for the (0.5, 1.5) approximation, then the exact
    /// re-check flips the result to ON_THE_LEFT.
    #[test]
    fn side_of_intersection_tolerance_ladder() {
        let this = line(0, 0, 0, 1);
        let p1 = line(0, 0, 1, 3);
        let p2 = line(0, 3, 1, 0);
        // T1: intersectionApprox = (0x1.0p-1, 0x1.8p0) = (0.5, 1.5).
        let approx = p1.intersection_approx(&p2);
        assert_eq!(approx, FloatPoint::new(0.5, 1.5));
        // T2: the float stage alone is collinear.
        assert_eq!(this.side_of_float(&approx, 1.0), Side::Collinear);
        // T3: the exact intersection is Rat(3, 9, 6).
        assert_eq!(p1.intersection(&p2), Some(rat(3, 9, 6)));
        // T4: the ladder result ON_THE_LEFT.
        assert_eq!(this.side_of_intersection(&p1, &p2), Side::Positive);
    }

    /// jshell pin `LEN` (bits 1119913255 = 96.26006f32): dx = dy = 46341
    /// wraps in the i32 squares (46341^2 = 2147488281 overflows to
    /// -2147479015, the sum wraps to 9266), so the "length" of a
    /// ~65536-long diagonal is sqrt(9266).
    #[test]
    fn length_int_wraparound() {
        let l = line(0, 0, 46341, 46341);
        assert_eq!(l.length().to_bits(), 1119913255);
        // Sanity: without the wrap the length would be ~65536.
        assert!((l.length() - 96.26).abs() < 0.01);
    }

    /// jshell pins `C1`-`C10`: the direction angle order of `compareTo`,
    /// including the open-halfplane double-determinant fallthrough (C7,
    /// C8, C10) and equality (C9).
    #[test]
    fn compare_to_ordering() {
        let up = line(0, 0, 0, 1);
        let right = line(0, 0, 1, 0);
        let down = line(0, 0, 0, -1);
        let left = line(0, 0, -1, 0);
        use Ordering::*;
        assert_eq!(up.compare_to(&right), Greater); // C1
        assert_eq!(right.compare_to(&up), Less); // C2
        assert_eq!(right.compare_to(&down), Less); // C3
        assert_eq!(down.compare_to(&left), Greater); // C4
        assert_eq!(up.compare_to(&down), Less); // C5
        assert_eq!(down.compare_to(&up), Greater); // C6
        let d12 = line(0, 0, 1, 2);
        let d21 = line(0, 0, 2, 1);
        assert_eq!(d12.compare_to(&d21), Greater); // C7
        assert_eq!(d21.compare_to(&d12), Less); // C8
        assert_eq!(d12.compare_to(&d12), Equal); // C9
        assert_eq!(
            line(0, 0, -1, 2).compare_to(&line(0, 0, -2, 1)),
            Less // C10
        );
    }

    /// jshell pins `F4`/`F5exact`: the fastEquals double determinant is
    /// exactly 0 here (both products round to the same double:
    /// fl(2^62 - 3*2^31 + 2) == fl(2^62 - 3*2^31)), so the lines compare
    /// fast-equal although the EXACT cross product is 2 (pin F5exact).
    #[test]
    fn fast_equals_double_determinant_boundary() {
        let fd = line(0, 0, -2147483648, 2147483646);
        let fe = line(2147483647, -2147483645, -1, 1);
        assert!(fd.fast_equals(&fe)); // F4
        // F5exact: the exact determinant of (otherA - a) and (b - a) is 2.
        assert_eq!(
            IntPoint::new(2147483647, -2147483645)
                .determinant(&IntPoint::new(-2147483648, 2147483646)),
            2
        );
        // F1/F2/F3: same-direction collinear lines are fast-equal,
        // opposite directions are not.
        let fa = line(0, 0, 2, 2);
        let fb = line(4, 4, 8, 8);
        let fc = line(8, 8, 4, 4);
        assert!(fa.fast_equals(&fb)); // F1
        assert!(!fb.fast_equals(&fc)); // F2
        assert!(!fa.fast_equals(&fc)); // F3
    }

    /// jshell pins `Q1`-`Q4`: equals requires a POSITIVE projection
    /// (opposite directions unequal) while isEqualOrOpposite/overlaps
    /// only require collinearity (T9 asymmetry).
    #[test]
    fn equals_vs_is_equal_or_opposite_asymmetry() {
        let fa = line(0, 0, 2, 2);
        let fb = line(4, 4, 8, 8);
        assert!(fa == fb); // Q1
        assert!(fb != fb.opposite()); // Q2
        assert!(fb.is_equal_or_opposite(&fb.opposite())); // Q3
        assert!(fb.overlaps(&fb.opposite())); // Q4
    }

    /// jshell pins `S1`-`S3`: sideOf(FloatPoint, double) tolerance
    /// ladder — det = 5 sits between the tolerance bands of 6.0 but
    /// outside those of 1.0 and 0.0.
    #[test]
    fn side_of_float_tolerance_bands() {
        let lv = line(0, 0, 0, 10);
        let p = FloatPoint::new(0.5, 3.0);
        assert_eq!(lv.side_of_float(&p, 1.0), Side::Positive); // S1
        assert_eq!(lv.side_of_float(&p, 6.0), Side::Collinear); // S2
        assert_eq!(lv.side_of_float_zero(&p), Side::Positive); // S3
    }

    /// jshell pins `TR1`, `SD1`, `PD1`, `FVA1`, `FIYV1`, `CA1`, `IP1`:
    /// translate (a java_round site), signedDistance, the
    /// perpendicularDirection heuristic, the two function-value
    /// approximations, cosAngle and the parallel/perpendicular checks.
    #[test]
    fn translate_signed_distance_and_approximations() {
        // TR1: translate(1.0) of the 45-degree line through the origin.
        let translated = line(0, 0, 1, 1).translate(1.0);
        assert_eq!(translated.a, int(-1, 0));
        assert_eq!(translated.b, int(0, 1));
        // SD1: signedDistance = 0x1.0p1 = 2.0.
        assert_eq!(
            line(2, 5, 12, 5).signed_distance(&FloatPoint::new(2.0, 3.0)),
            2.0
        );
        // PD1: UP for a point below a horizontal line.
        assert_eq!(
            line(2, 5, 12, 5).perpendicular_direction(&int(2, 0)),
            Some(Direction::UP)
        );
        // FVA1 = 3.5, FIYV1 = 6.0.
        let slope_half = line(0, 0, 20, 10);
        assert_eq!(slope_half.function_value_approx(7.0), 3.5);
        assert_eq!(slope_half.function_in_y_value_approx(3.0), 6.0);
        // CA1 = 0x1.6a09e667f3bccp-1 = cos(45 degrees).
        assert_eq!(
            line(0, 0, 1, 0).cos_angle(&line(0, 0, 1, 1)).to_bits(),
            0x3FE6_A09E_667F_3BCC
        );
        // IP1: parallel/perpendicular classification.
        let lh = line(2, 5, 12, 5);
        assert!(!lh.is_parallel(&line(2, 0, 12, 10)));
        assert!(lh.is_parallel(&line(0, 0, 3, 0)));
        assert!(lh.is_perpendicular(&line(0, 0, 0, 10)));
        assert!(!lh.is_perpendicular(&line(0, 0, 3, 0)));
    }

    /// jshell pins `M1`-`M3`, `TB1`, `TB2`, `GID`, `GI`: the 90-degree
    /// turn and the (a/b-swapping) mirrors, translateBy (including the
    /// zero-vector identity shortcut), getId and the
    /// point+direction factory.
    #[test]
    fn transforms_and_identity() {
        let diag = line(0, 0, 2, 2);
        let turned = diag.turn_90_degree(1, &IntPoint::new(0, 0));
        assert_eq!(
            (turned.a.clone(), turned.b.clone()),
            (int(0, 0), int(-2, 2))
        ); // M1
        let mv = diag.mirror_vertical(&IntPoint::new(1, 0));
        assert_eq!((mv.a.clone(), mv.b.clone()), (int(0, 2), int(2, 0))); // M2
        let mh = diag.mirror_horizontal(&IntPoint::new(0, 1));
        assert_eq!((mh.a.clone(), mh.b.clone()), (int(2, 0), int(0, 2))); // M3
        let tb = diag.translate_by(&Vector::Int(crate::int_vector::IntVector::new(1, 0)));
        assert_eq!((tb.a.clone(), tb.b.clone()), (int(1, 0), int(3, 2))); // TB1
        assert_eq!(diag.translate_by(&crate::vector::Vector::ZERO), diag); // TB2
        assert_eq!(line(100, 200, 3, 4).get_id(), 102397); // GID
        let gi = Line::get_instance(int(0, 0), Direction::RIGHT45);
        assert_eq!((gi.a.clone(), gi.b.clone()), (int(0, 0), int(1, 1))); // GI
        // The constructor variant pre-seeds the direction cache.
        let gi2 = Line::new_with_direction(int(0, 0), Direction::RIGHT45);
        assert_eq!(gi2, gi);
    }

    /// jshell pins `PP1`/`PP2r`: perpendicular projection double
    /// dispatch. The IntPoint algorithm maps (3, 5) onto y = 1 at
    /// (3, 1); the RationalPoint algorithm — SAME affine input — returns
    /// (3, -1) because its projY numerator ADDS det*vx*z where the
    /// IntPoint variant SUBTRACTS det*vx. Genuine upstream divergence,
    /// reproduced bit-parity (see .wolf/buglog.json).
    #[test]
    fn perpendicular_projection_int_vs_rational_divergence() {
        let hline = line(0, 1, 2, 1);
        assert_eq!(
            hline.perpendicular_projection(&int(3, 5)),
            int(3, 1) // PP1
        );
        let rational_point = Point::rational(RationalPoint::new(
            BigInt::from(3),
            BigInt::from(5),
            BigInt::ONE,
        ));
        assert_eq!(
            hline.perpendicular_projection(&rational_point),
            int(3, -1) // PP2r
        );
    }

    /// A degenerate a == b line caches Direction::NULL; per T9, the null
    /// direction compares UNEQUAL to itself (Java equals demands a
    /// POSITIVE projection, which the zero vector never yields), and the
    /// degenerate line is collinear with every point.
    #[test]
    fn degenerate_line_direction() {
        let degenerate = line(3, 4, 3, 4);
        // Structurally the direction is the null vector ...
        assert_eq!(degenerate.direction().get_vector(), Vector::ZERO); // structurally null
        // ... but the Java-parity equality still reports "not equal".
        assert!(*degenerate.direction() != Direction::NULL);
        let same_point = Point::int(IntPoint::new(3, 4));
        assert!(degenerate.side_of(&same_point) == Side::Collinear);
        // opposite() swaps a and b.
        let normal = line(0, 0, 2, 2);
        assert_eq!(normal.opposite(), line(2, 2, 0, 0));
    }

    /// THE buglog-169 boundary pin (T17a): `intersection_approx`
    /// keeps Java's `(IntPoint)` cast face — a line carrying rational
    /// endpoints makes Java throw ClassCastException
    /// (Line.java:315-318), and the port panics with the same
    /// meaning. The bug-169 fix routes rational corners AWAY from
    /// this face (the engine segment delegation never materializes
    /// them); it must not loosen the cast — a float fallback here
    /// would silently accept what Java rejects.
    #[test]
    #[should_panic(expected = "Line only implemented for IntPoints till now")]
    fn t17_intersection_approx_still_rejects_rational_lines() {
        let rational_line = Line::new(
            Point::rational(RationalPoint::new(
                num_bigint::BigInt::from(1_i32),
                num_bigint::BigInt::from(1_i32),
                num_bigint::BigInt::from(2_i32),
            )),
            Point::int(IntPoint::new(3, 4)),
        );
        let other = line(0, 0, 1, 0);
        let _ = rational_line.intersection_approx(&other);
    }
}
