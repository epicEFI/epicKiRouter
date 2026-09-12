//! Port of Java `app.freerouting.geometry.planar.Point`.
//!
//! Java models points as an abstract sealed class with the two concrete
//! implementations [`IntPoint`] and [`RationalPoint`]; every polymorphic
//! operation resolves through double dispatch. This port seals the
//! hierarchy as a 2-variant enum, and each operation is a 2x2 `match`
//! covering the same concrete overload pairs as the Java dispatch —
//! including the asymmetries (e.g. an IntPoint translated by a
//! RationalVector uses `RationalVector.addTo(IntPoint)`, while a
//! RationalPoint translated by an IntVector goes through
//! `BigIntAux.addRationalCoordinates`).
//!
//! The zero-vector short-circuit of `translateBy` is parity-exact:
//! `vector.equals(Vector.ZERO)` is false for every RationalVector (Java
//! `getClass()` check), so only an IntVector(0, 0) short-circuits — and a
//! rational-typed zero vector DOES change the representation of the
//! result triple.
//!
//! Java `Point.ZERO` is an IntPoint; it is available both as
//! [`IntPoint::ZERO`] and [`Point::ZERO`].
//!
//! Landed with Task 7: `sideOf(Line)` / `perpendicularDirection`
//! (`sideOf(Line)` is consumed by `LineSegment::to_simplex`);
//! `perpendicularProjection` landed with Task 6 on the `Line` side (the
//! single-sided dispatch port). `roundToTheRight`/
//! `roundToTheLeft` landed with Task 3 (Direction);
//! `surroundingBox`/`isContainedIn` with Task 4 (IntBox);
//! `surroundingOctagon` with Task 5 (IntOctagon).

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use crate::big_int_aux::{big_abs, big_integer_int_value};
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::limits::{CRIT_INT, CRIT_INT_BIG};
use crate::line::Line;
use crate::rational_point::{RationalPoint, ordering_to_int};
use crate::side::Side;
use crate::vector::Vector;
use num_bigint::{BigInt, Sign};

/// Abstract class describing functionality for points in the plane.
#[derive(Debug, Clone)]
pub enum Point {
    /// Integer coordinates, exact within the int range.
    Int(IntPoint),
    /// Projective rational coordinates `(x / z, y / z)`.
    Rational(Box<RationalPoint>),
}

impl Point {
    /// Java `Point.ZERO` (an IntPoint).
    pub const ZERO: Point = Point::Int(IntPoint::ZERO);

    /// Wraps an IntPoint.
    pub const fn int(point: IntPoint) -> Point {
        Point::Int(point)
    }

    /// Wraps a RationalPoint.
    pub fn rational(point: RationalPoint) -> Point {
        Point::Rational(Box::new(point))
    }

    /// Creates an IntPoint from x and y. If |x| or |y| is too big for an
    /// IntPoint (beyond CRIT_INT, with Java's `Math.abs` wraparound), a
    /// RationalPoint is created.
    pub fn get_instance(x: i32, y: i32) -> Point {
        let result = IntPoint::new(x, y);
        if x.wrapping_abs() > CRIT_INT || y.wrapping_abs() > CRIT_INT {
            return Point::rational(RationalPoint::from_int_point(&result));
        }
        Point::Int(result)
    }

    /// Factory method for creating a Point from 3 BigIntegers. Mirrors
    /// `Point.getInstance(BigInteger, BigInteger, BigInteger)`: the triple
    /// is sign-normalized (z >= 0); if z divides x, BOTH x and y are
    /// divided (y by truncation toward zero, even when not divisible —
    /// Java quirk); an integral result within CRIT_INT down-converts to an
    /// IntPoint. Panics on z == 0 like Java's `ArithmeticException`
    /// (BigInteger: modulus not positive).
    pub fn get_instance_big(x: BigInt, y: BigInt, z: BigInt) -> Point {
        let (x, y, z) = if z.sign() == Sign::Minus {
            // the denominator z of a RationalPoint is expected to be positive
            (-x, -y, -z)
        } else {
            (x, y, z)
        };
        let (x, y, z) = if (&x % &z).sign() == Sign::NoSign {
            // x and y can be divided by z (y is divided regardless)
            (&x / &z, &y / &z, BigInt::ONE)
        } else {
            (x, y, z)
        };
        if z == BigInt::ONE
            && big_abs(&x).cmp(&*CRIT_INT_BIG) != Ordering::Greater
            && big_abs(&y).cmp(&*CRIT_INT_BIG) != Ordering::Greater
        {
            // the Point fits into an IntPoint
            return Point::Int(IntPoint::new(
                big_integer_int_value(&x),
                big_integer_int_value(&y),
            ));
        }
        Point::rational(RationalPoint::new(x, y, z))
    }

    /// Returns the smallest IntBox containing only this point (Java
    /// `surroundingBox`, landed with Task 4).
    pub fn surrounding_box(&self) -> IntBox {
        match self {
            Point::Int(p) => p.surrounding_box(),
            Point::Rational(p) => p.surrounding_box(),
        }
    }

    /// Returns the smallest IntOctagon containing only this point (Java
    /// `surroundingOctagon` double dispatch, landed with Task 5).
    pub fn surrounding_octagon(&self) -> IntOctagon {
        match self {
            Point::Int(p) => p.surrounding_octagon(),
            Point::Rational(p) => p.surrounding_octagon(),
        }
    }

    /// Returns true if this point lies inside `int_box` or on its border
    /// (Java `isContainedIn(IntBox)` double dispatch, landed with Task 4).
    pub fn is_contained_in(&self, int_box: &IntBox) -> bool {
        match self {
            Point::Int(p) => p.is_contained_in(int_box),
            Point::Rational(p) => p.is_contained_in(int_box),
        }
    }

    /// Returns the translation of this point by vector (Java
    /// `translateBy(Vector)` double dispatch: `vector.addTo(this)`).
    pub fn translate_by(&self, vector: &Vector) -> Point {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        match self {
            Point::Int(p) => vector.add_to_int_point(p),
            Point::Rational(p) => vector.add_to_rational_point(p),
        }
    }

    /// Returns the difference vector of this point and other (Java
    /// `differenceBy(Point)` double dispatch): the vector leading from
    /// other to this point.
    pub fn difference_by(&self, other: &Point) -> crate::vector::Vector {
        match (self, other) {
            (Point::Int(s), Point::Int(o)) => crate::vector::Vector::Int(s.difference_by(o)),
            (Point::Int(s), Point::Rational(o)) => o.difference_by_int_point(s).negate(),
            (Point::Rational(s), Point::Int(o)) => s.difference_by_int_point(o),
            (Point::Rational(s), Point::Rational(o)) => s.difference_by_rational_point(o),
        }
    }

    /// Returns the side of the directed line from p1 to p2 on which this
    /// point lies.
    pub fn side_of(&self, p1: &Point, p2: &Point) -> Side {
        let v1 = self.difference_by(p1);
        let v2 = p2.difference_by(p1);
        v1.side_of(&v2)
    }

    /// Returns 1, if this point has a strict bigger x coordinate than
    /// other, 0, if the x coordinates are equal, and -1 otherwise.
    pub fn compare_x(&self, other: &Point) -> i32 {
        match (self, other) {
            (Point::Int(s), Point::Int(o)) => ordering_to_int(s.x.cmp(&o.x)),
            (Point::Int(s), Point::Rational(o)) => -o.compare_x_int_point(s),
            (Point::Rational(s), Point::Int(o)) => s.compare_x_int_point(o),
            (Point::Rational(s), Point::Rational(o)) => s.compare_x_rational_point(o),
        }
    }

    /// Returns 1, if this point has a strict bigger y coordinate than
    /// other, 0, if the y coordinates are equal, and -1 otherwise.
    pub fn compare_y(&self, other: &Point) -> i32 {
        match (self, other) {
            (Point::Int(s), Point::Int(o)) => ordering_to_int(s.y.cmp(&o.y)),
            (Point::Int(s), Point::Rational(o)) => -o.compare_y_int_point(s),
            (Point::Rational(s), Point::Int(o)) => s.compare_y_int_point(o),
            (Point::Rational(s), Point::Rational(o)) => s.compare_y_rational_point(o),
        }
    }

    /// Returns compare_x(other), or compare_y(other) if the x coordinates
    /// are equal.
    pub fn compare_xy(&self, other: &Point) -> i32 {
        let result = self.compare_x(other);
        if result == 0 {
            return self.compare_y(other);
        }
        result
    }

    /// Turns this point by factor times 90 degrees around pole.
    pub fn turn_90_degree(&self, factor: i32, pole: &Point) -> Point {
        let v = self.difference_by(pole);
        let v = v.turn_90_degree(factor);
        pole.translate_by(&v)
    }

    /// Mirrors this point at the vertical line through pole.
    pub fn mirror_vertical(&self, pole: &Point) -> Point {
        let v = self.difference_by(pole);
        let v = v.mirror_at_y_axis();
        pole.translate_by(&v)
    }

    /// Mirrors this point at the horizontal line through pole.
    pub fn mirror_horizontal(&self, pole: &Point) -> Point {
        let v = self.difference_by(pole);
        let v = v.mirror_at_x_axis();
        pole.translate_by(&v)
    }

    /// Returns a unique ID for this point for deterministic tie-breaking.
    pub fn get_id(&self) -> i32 {
        match self {
            Point::Int(p) => p.get_id(),
            Point::Rational(p) => p.get_id(),
        }
    }

    /// Returns true if this point is a RationalPoint with denominator
    /// z = 0.
    pub fn is_infinite(&self) -> bool {
        match self {
            Point::Int(_) => false,
            Point::Rational(p) => p.is_infinite(),
        }
    }

    /// Approximates the coordinates of this point by float coordinates.
    pub fn to_float(&self) -> FloatPoint {
        match self {
            Point::Int(p) => p.to_float(),
            Point::Rational(p) => p.to_float(),
        }
    }
}

/// Java `equals`: IntPoint pairs are componentwise equal, RationalPoint
/// pairs compare by cross multiplication, different classes are never
/// equal (`getClass()` check).
impl PartialEq for Point {
    fn eq(&self, other: &Point) -> bool {
        match (self, other) {
            (Point::Int(s), Point::Int(o)) => s == o,
            (Point::Rational(s), Point::Rational(o)) => s == o,
            _ => false,
        }
    }
}

impl Eq for Point {}

/// Delegates to the concrete Java hashCodes (31*x + y, and the
/// gcd-reduced BigInteger composition respectively).
impl Hash for Point {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Point::Int(p) => p.hash(state),
            Point::Rational(p) => p.hash(state),
        }
    }
}

// ---------------------------------------------------------------------------
// Task 7 closures (Line family)
// ---------------------------------------------------------------------------

impl Point {
    /// Returns on which side of the line this point is (Java
    /// `Point.sideOf(Line)` == `sideOf(line.a, line.b)`).
    pub fn side_of_line(&self, line: &Line) -> Side {
        self.side_of(&line.a, &line.b)
    }

    /// Returns the direction which leads from this point to the line in
    /// an angle of 90 degree (Java `Point.perpendicularDirection`):
    /// collinear points yield the null direction, points on the right
    /// side of the line yield the line direction turned by 2 x 45
    /// degrees, points on the left side the direction turned by
    /// 6 x 45 degrees.
    pub fn perpendicular_direction(&self, line: &Line) -> Direction {
        let side = self.side_of_line(line);
        if side == Side::Collinear {
            return Direction::NULL;
        }
        if side == Side::Negative {
            line.direction().turn_45_degree(2)
        } else {
            line.direction().turn_45_degree(6)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_vector::IntVector;
    use crate::rational_vector::RationalVector;
    use num_bigint::BigInt;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    fn int(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    fn rat(x: i64, y: i64, z: i64) -> Point {
        Point::rational(RationalPoint::new(b(x), b(y), b(z)))
    }

    /// jshell pins `P17`/`P18`: CRIT_INT fits into an IntPoint, CRIT_INT +
    /// 1 promotes to a RationalPoint.
    #[test]
    fn get_instance_promotes_beyond_crit_int() {
        assert_eq!(Point::get_instance(CRIT_INT, 0), int(CRIT_INT, 0));
        assert_eq!(
            Point::get_instance(CRIT_INT + 1, 0),
            rat(CRIT_INT as i64 + 1, 0, 1)
        );
    }

    /// jshell pins `P15`/`P16`/`P19` and `Q23`: the factory divides both
    /// coordinates when z divides x (y by truncation), normalizes a
    /// negative z, and down-converts integral results.
    #[test]
    fn get_instance_big_down_conversion_quirks() {
        // (4, 3, 2): x divisible, y divided anyway (3/2 truncates to 1).
        assert_eq!(Point::get_instance_big(b(4), b(3), b(2)), int(2, 1));
        // (100, 200, 50) -> (2, 4).
        assert_eq!(Point::get_instance_big(b(100), b(200), b(50)), int(2, 4));
        // (-2, -4, -2) normalizes to (2, 4, 2) -> (1, 2).
        assert_eq!(Point::get_instance_big(b(-2), b(-4), b(-2)), int(1, 2));
        // 10^18 exceeds CRIT_INT: stays rational.
        let big = BigInt::from(10u32).pow(18);
        assert_eq!(
            Point::get_instance_big(big.clone(), big, b(1)),
            rat(1_000_000_000_000_000_000, 1_000_000_000_000_000_000, 1)
        );
        // z == 0 panics like Java's ArithmeticException (modulus not
        // positive).
        let result = std::panic::catch_unwind(|| {
            Point::get_instance_big(b(10), b(20), b(0));
        });
        assert!(result.is_err());
    }

    /// jshell pin `P72`: an IntPoint equals the factory result built from
    /// the same integral triple; cross-class comparison stays false
    /// (jshell pins `Q5`/`Q6`).
    #[test]
    fn equality_is_class_sensitive() {
        assert_eq!(
            Point::get_instance(2, 4),
            Point::get_instance_big(b(2), b(4), b(1))
        );
        // A RationalPoint representing the same value (2, 4) built through
        // the direct constructor (the factory would down-convert it, as
        // the Q5/Q6 jshell pins did via reflection).
        let rp = rat(4, 8, 2);
        assert_eq!(rp, rat(2, 4, 1));
        assert_ne!(rp, int(2, 4));
        assert_ne!(int(2, 4), rp);
    }

    /// jshell pin `P20`: int translation wraps; pins `Q7`/`Q13`: rational
    /// translations keep raw triples. The zero-vector short-circuit fires
    /// only for the IntVector zero (Q19/P73 class-sensitivity).
    #[test]
    fn translate_by_dispatch_and_zero_shortcut() {
        // P20: (i32::MAX, 0) + (1, 0) wraps to (i32::MIN, 0).
        assert_eq!(
            int(i32::MAX, 0).translate_by(&Vector::Int(IntVector::new(1, 0))),
            int(i32::MIN, 0)
        );
        // Q7: (3, 4, 2) + (1, 0) = (5, 4, 2) as a RationalPoint.
        let rp = Point::get_instance_big(b(3), b(4), b(2));
        assert_eq!(
            rp.translate_by(&Vector::Int(IntVector::new(1, 0))),
            rat(5, 4, 2)
        );
        // Q13: (3, 4, 2) + (1, 0, 3) = (11, 12, 6).
        assert_eq!(
            rp.translate_by(&Vector::rational(RationalVector::new(b(1), b(0), b(3)))),
            rat(11, 12, 6)
        );
        // IntVector(0, 0) short-circuits.
        assert_eq!(rp.translate_by(&Vector::ZERO), rp);
        assert_eq!(int(5, 5).translate_by(&Vector::ZERO), int(5, 5));
        // A rational-typed zero vector does NOT short-circuit: the triples
        // are combined via addRationalCoordinates ((3,4,2) + (0,0,3) =
        // (3*3+0*2, 4*3+0*2, 6) = (9,12,6)), same value, new triple.
        let rebased = rp.translate_by(&Vector::rational(RationalVector::new(b(0), b(0), b(3))));
        assert_eq!(rebased, rat(9, 12, 6));
        assert_eq!(rebased, rp); // same affine value
        assert_ne!(rebased.get_id(), rp.get_id()); // ... but a fresh triple
    }

    /// jshell pins `P37`/`P38`/`Q24` and the int-int case.
    #[test]
    fn difference_by_dispatch() {
        let rp = Point::get_instance_big(b(3), b(4), b(2));
        // P37: rp - (1, 1) = (1, 2, 2).
        assert_eq!(
            rp.difference_by(&int(1, 1)),
            Vector::rational(RationalVector::new(b(1), b(2), b(2)))
        );
        // P38: (1, 1) - rp = (-1, -2, 2).
        assert_eq!(
            int(1, 1).difference_by(&rp),
            Vector::rational(RationalVector::new(b(-1), b(-2), b(2)))
        );
        // Q24: rp - RationalPoint(1, 1, 1) = (1, 2, 2).
        assert_eq!(
            rp.difference_by(&rat(1, 1, 1)),
            Vector::rational(RationalVector::new(b(1), b(2), b(2)))
        );
        // Int - Int stays an IntVector.
        assert_eq!(
            int(3, 4).difference_by(&int(1, 1)),
            Vector::Int(IntVector::new(2, 3))
        );
    }

    /// jshell pins `P56`/`P77` and a rational-point case.
    #[test]
    fn side_of_two_points() {
        assert_eq!(int(0, 1).side_of(&int(0, 0), &int(1, 0)), Side::Positive);
        assert_eq!(int(0, 0).side_of(&int(1, 0), &int(2, 0)), Side::Collinear);
        // (3/2, 2) is above the x axis: on the left of (0,0) -> (1,0).
        assert_eq!(
            Point::get_instance_big(b(3), b(4), b(2)).side_of(&int(0, 0), &int(1, 0)),
            Side::Positive
        );
    }

    /// jshell pins `R5`-`R12` for the compare dispatch.
    #[test]
    fn compare_dispatch_matches_java() {
        let rp = Point::get_instance_big(b(3), b(4), b(2));
        // R5: rp.compareX(IntPoint(2, 4)) = 3 vs z*2 = 4.
        assert_eq!(rp.compare_x(&int(2, 4)), -1);
        // R6: rp.compareX((6, 8, 4)): 3*4 vs 6*2.
        assert_eq!(rp.compare_x(&Point::get_instance_big(b(6), b(8), b(4))), 0);
        // R7: 3 vs 5*2.
        assert_eq!(rp.compare_x(&int(5, 0)), -1);
        // R8: 4 vs 2*2.
        assert_eq!(rp.compare_y(&int(0, 2)), 0);
        // R9: 4*1 vs 9*2.
        assert_eq!(rp.compare_y(&Point::get_instance(0, 9)), -1);
        // R10/R11: compareXY falls back to y.
        assert_eq!(rp.compare_xy(&int(5, 0)), -1);
        assert_eq!(rp.compare_xy(&int(3, 9)), -1);
        // R12: (6, 10, 4) vs (3, 4, 2): x equal, y bigger.
        let rp2 = Point::get_instance_big(b(6), b(10), b(4));
        assert_eq!(rp2.compare_x(&rp), 0);
        assert_eq!(rp2.compare_y(&rp), 1);
        // jshell pins `I1a`-`I1d`: the (Int, Rational) dispatch arms NEGATE
        // the rational comparison; dropping the negation would flip all
        // four of these (mutation trap).
        assert_eq!(int(2, 4).compare_x(&rp), 1); // 2 > 1.5 (un-negated: -1)
        assert_eq!(int(1, 0).compare_x(&rp), -1); // 1 < 1.5 (un-negated: 1)
        assert_eq!(int(0, 9).compare_y(&rp), 1); // 9 > 2 (un-negated: -1)
        assert_eq!(int(0, 1).compare_y(&rp), -1); // 1 < 2 (un-negated: 1)
        // Int-int compares.
        assert_eq!(int(3, 1).compare_x(&int(5, 1)), -1);
        assert_eq!(int(3, 1).compare_y(&int(3, 1)), 0);
        assert_eq!(int(7, 1).compare_xy(&int(7, 9)), -1);
    }

    /// jshell pins `P28`-`P31` and a rational turn.
    #[test]
    fn turns_and_mirrors_around_poles() {
        // P28: (3, 1) turned 90 degrees around (1, 0) -> (0, 2).
        assert_eq!(int(3, 1).turn_90_degree(1, &int(1, 0)), int(0, 2));
        // P29: (1, 0) turned -90 degrees around (0, 0) -> (0, -1).
        assert_eq!(int(1, 0).turn_90_degree(-1, &int(0, 0)), int(0, -1));
        // P30/P31: mirrors at the vertical/horizontal line through (1, 0).
        assert_eq!(int(3, 1).mirror_vertical(&int(1, 0)), int(-1, 1));
        assert_eq!(int(3, 1).mirror_horizontal(&int(1, 0)), int(3, -1));
        // Rational point turned around the origin: (3, 4, 2) -> (-4, 3, 2)
        // (the translation re-bases the triple to denominator 2).
        assert_eq!(
            Point::get_instance_big(b(3), b(4), b(2)).turn_90_degree(1, &int(0, 0)),
            rat(-4, 3, 2)
        );
    }

    /// jshell pins `P4`/`P9`: getId pins; `P8` is covered by the
    /// RationalPoint hash tests.
    #[test]
    fn get_id_and_is_infinite() {
        assert_eq!(int(100, 200).get_id(), 3300);
        assert_eq!(rat(6, 10, 4).get_id(), 6080);
        assert!(!int(0, 0).is_infinite());
        assert!(rat(10, 20, 0).is_infinite());
        assert_eq!(rat(3, 4, 2).to_float(), FloatPoint::new(1.5, 2.0));
        assert_eq!(int(2, 3).to_float(), FloatPoint::new(2.0, 3.0));
    }

    /// jshell pin: Point dispatch of surroundingOctagon matches the
    /// concrete implementations (int point (3,-4) and the rational half).
    #[test]
    fn surrounding_octagon_dispatches() {
        assert_eq!(
            int(3, -4).surrounding_octagon(),
            IntOctagon::new(3, -4, 3, -4, 7, 7, -1, -1)
        );
        assert_eq!(
            rat(1, 1, 2).surrounding_octagon(),
            IntOctagon::new(0, 0, 1, 1, 0, 0, 1, 1)
        );
    }
}
