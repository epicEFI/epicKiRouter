//! Port of Java `app.freerouting.geometry.planar.RationalPoint`.
//!
//! A point in the projective plane represented by 3 integer coordinates
//! `x, y, z`; the affine point with rational coordinates is `(x/z, y/z)`.
//! Two triples on the same line through the origin are equal (cross
//! multiplication). `z == 0` are the points at infinity; unlike the Java
//! factories (`Point.getInstance` throws on `z == 0`), the constructor
//! accepts `z == 0`, mirroring the package-private Java constructor.
//!
//! The constructor does NOT normalize the triple (no gcd reduction, no
//! sign flip of negative z) — it only rejects `z < 0`, exactly like the
//! Java original.
//!
//! The Java double-dispatch overloads (`translateBy`, `differenceBy`,
//! `compareX`, `compareY`) live on the [`crate::point::Point`] enum as
//! `pub(crate)` methods here.
//!
//! Deferred: `sideOf(Line)` and `perpendicularProjection(Line)` (Task 6,
//! Line). `surroundingBox` / `isContainedIn` landed with Task 4 (IntBox);
//! `surroundingOctagon` with Task 5 (IntOctagon).

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use crate::big_int_aux::{
    add_rational_coordinates, big_abs, big_gcd, big_integer_hash_code, determinant,
};
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::point::Point;
use crate::rational_vector::RationalVector;
use crate::vector::Vector;
use num_bigint::{BigInt, Sign};

/// Converts a `BigInteger.compareTo` result to Java's documented -1/0/1.
pub(crate) fn ordering_to_int(ordering: Ordering) -> i32 {
    match ordering {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Implementation of the abstract class Point with rational coordinates
/// `(x / z, y / z)` in the projective plane.
#[derive(Debug, Clone)]
pub struct RationalPoint {
    /// The x numerator.
    pub(crate) x: BigInt,
    /// The y numerator.
    pub(crate) y: BigInt,
    /// The denominator; `z == 0` marks a point at infinity.
    pub(crate) z: BigInt,
}

impl RationalPoint {
    /// Creates a RationalPoint from 3 BigIntegers x, y and z. Panics
    /// (Java: `IllegalArgumentException`) if the denominator z is
    /// negative. The triple is not normalized. `pub(crate)` because the
    /// Java constructor is package-private.
    pub(crate) fn new(x: BigInt, y: BigInt, z: BigInt) -> RationalPoint {
        assert!(
            z.sign() != Sign::Minus,
            "RationalPoint: z is expected to be >= 0"
        );
        RationalPoint { x, y, z }
    }

    /// Creates a RationalPoint from an IntPoint (denominator 1).
    pub(crate) fn from_int_point(point: &IntPoint) -> RationalPoint {
        RationalPoint {
            x: BigInt::from(point.x),
            y: BigInt::from(point.y),
            z: BigInt::ONE,
        }
    }

    /// Approximates the coordinates of this point by float coordinates.
    /// Points at infinity use `Float.MAX_VALUE` (the f32 maximum widened
    /// to f64), exactly as in Java (RationalPoint.java:50-63).
    pub fn to_float(&self) -> FloatPoint {
        let xd = crate::big_int_aux::big_integer_double_value(&self.x);
        let yd = crate::big_int_aux::big_integer_double_value(&self.y);
        let zd = crate::big_int_aux::big_integer_double_value(&self.z);
        if zd == 0.0 {
            FloatPoint::new(f32::MAX as f64, f32::MAX as f64)
        } else {
            FloatPoint::new(xd / zd, yd / zd)
        }
    }

    /// Returns the smallest IntBox containing only this point (Java
    /// `surroundingBox`, landed with Task 4): floors/ceils the float
    /// approximation.
    pub fn surrounding_box(&self) -> IntBox {
        let fp = self.to_float();
        let llx = fp.x.floor() as i32;
        let lly = fp.y.floor() as i32;
        let urx = fp.x.ceil() as i32;
        let ury = fp.y.ceil() as i32;
        IntBox::from_corners(llx, lly, urx, ury)
    }

    /// Returns the smallest IntOctagon containing only this point (Java
    /// `surroundingOctagon`, landed with Task 5): floors/ceils the float
    /// approximation and the two diagonal coordinates separately. The
    /// result is NOT normalized, like the Java original.
    pub fn surrounding_octagon(&self) -> IntOctagon {
        let fp = self.to_float();
        let lx = fp.x.floor() as i32;
        let ly = fp.y.floor() as i32;
        let rx = fp.x.ceil() as i32;
        let uy = fp.y.ceil() as i32;
        let diag1 = fp.x - fp.y;
        let diag2 = fp.x + fp.y;
        let ulx = diag1.floor() as i32;
        let lrx = diag1.ceil() as i32;
        let llx = diag2.floor() as i32;
        let urx = diag2.ceil() as i32;
        IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx)
    }

    /// Returns true if this exact rational point lies inside `int_box`
    /// or on its border (Java `isContainedIn(IntBox)`, landed with
    /// Task 4): compares each numerator against `corner * z` with
    /// BigIntegers.
    pub fn is_contained_in(&self, int_box: &IntBox) -> bool {
        let mut tmp = BigInt::from(int_box.ll.x) * &self.z;
        if self.x < tmp {
            return false;
        }
        tmp = BigInt::from(int_box.ll.y) * &self.z;
        if self.y < tmp {
            return false;
        }
        tmp = BigInt::from(int_box.ur.x) * &self.z;
        if self.x > tmp {
            return false;
        }
        tmp = BigInt::from(int_box.ur.y) * &self.z;
        if self.y > tmp {
            return false;
        }
        true
    }

    /// Returns a unique ID for deterministic tie-breaking (the raw,
    /// unreduced hash composition, RationalPoint.java:66-70).
    pub fn get_id(&self) -> i32 {
        self.raw_hash()
    }

    /// Java `getId()`/composition: `r = x.hashCode(); r = 31*r +
    /// y.hashCode(); r = 31*r + z.hashCode()` with int wraparound, WITHOUT
    /// the gcd reduction of `hashCode()`.
    fn raw_hash(&self) -> i32 {
        let mut result = big_integer_hash_code(&self.x);
        result = result
            .wrapping_mul(31)
            .wrapping_add(big_integer_hash_code(&self.y));
        result
            .wrapping_mul(31)
            .wrapping_add(big_integer_hash_code(&self.z))
    }

    /// Returns true if this point has denominator z = 0 (lies on the line
    /// at infinity).
    pub fn is_infinite(&self) -> bool {
        self.z.sign() == Sign::NoSign
    }

    /// Java `compareX(RationalPoint)`: cross multiplication, no division
    /// (RationalPoint.java:290-294).
    pub(crate) fn compare_x_rational_point(&self, other: &RationalPoint) -> i32 {
        let tmp1 = &self.x * &other.z;
        let tmp2 = &other.x * &self.z;
        ordering_to_int(tmp1.cmp(&tmp2))
    }

    /// Java `compareX(IntPoint)`: this.x compared with z * other.x
    /// (RationalPoint.java:297-300).
    pub(crate) fn compare_x_int_point(&self, other: &IntPoint) -> i32 {
        let tmp1 = &self.z * BigInt::from(other.x);
        ordering_to_int(self.x.cmp(&tmp1))
    }

    /// Java `compareY(RationalPoint)` (RationalPoint.java:308-312).
    pub(crate) fn compare_y_rational_point(&self, other: &RationalPoint) -> i32 {
        let tmp1 = &self.y * &other.z;
        let tmp2 = &other.y * &self.z;
        ordering_to_int(tmp1.cmp(&tmp2))
    }

    /// Java `compareY(IntPoint)` (RationalPoint.java:315-318).
    pub(crate) fn compare_y_int_point(&self, other: &IntPoint) -> i32 {
        let tmp1 = &self.z * BigInt::from(other.y);
        ordering_to_int(self.y.cmp(&tmp1))
    }

    /// Java package-private `translateBy(IntVector)`: wraps the IntVector
    /// as a RationalVector, then performs the rational translation.
    pub(crate) fn translate_by_int_vector(&self, vector: &IntVector) -> Point {
        self.translate_by_rational_vector(&RationalVector::from_int_vector(vector))
    }

    /// Java package-private `translateBy(RationalVector)`: sums the
    /// `[x, y, z]` triples via `BigIntAux.addRationalCoordinates`. The
    /// result is constructed raw (no down-conversion to IntPoint), exactly
    /// as in Java.
    pub(crate) fn translate_by_rational_vector(&self, vector: &RationalVector) -> Point {
        let v1 = [self.x.clone(), self.y.clone(), self.z.clone()];
        let v2 = [vector.x.clone(), vector.y.clone(), vector.z.clone()];
        let result = add_rational_coordinates(&v1, &v2);
        Point::rational(RationalPoint::new(
            result[0].clone(),
            result[1].clone(),
            result[2].clone(),
        ))
    }

    /// Java package-private `differenceBy(IntPoint)`: wraps other as a
    /// RationalPoint, then subtracts rationally.
    pub(crate) fn difference_by_int_point(&self, other: &IntPoint) -> Vector {
        self.difference_by_rational_point(&RationalPoint::from_int_point(other))
    }

    /// Java package-private `differenceBy(RationalPoint)`: adds the
    /// negated `[other.x, other.y, other.z]` triple via
    /// `BigIntAux.addRationalCoordinates` (RationalPoint.java:206-218).
    pub(crate) fn difference_by_rational_point(&self, other: &RationalPoint) -> Vector {
        let v1 = [self.x.clone(), self.y.clone(), self.z.clone()];
        let v2 = [-other.x.clone(), -other.y.clone(), other.z.clone()];
        let result = add_rational_coordinates(&v1, &v2);
        Vector::rational(RationalVector::new(
            result[0].clone(),
            result[1].clone(),
            result[2].clone(),
        ))
    }
}

/// Java `equals`: cross multiplication via `BigIntAux.determinant` —
/// `x1*z2 == x2*z1 && y1*z2 == y2*z1`. Two points on the same line through
/// the origin are equal, in particular all points with z = 0.
impl PartialEq for RationalPoint {
    fn eq(&self, other: &Self) -> bool {
        let det = determinant(&self.x, &other.x, &self.z, &other.z);
        if det.sign() != Sign::NoSign {
            return false;
        }
        let det = determinant(&self.y, &other.y, &self.z, &other.z);
        det.sign() == Sign::NoSign
    }
}

impl Eq for RationalPoint {}

/// Java `hashCode` (RationalPoint.java:93-109): 0 for points at infinity,
/// otherwise the triple reduced by the gcd of the absolute values, then the
/// JDK `BigInteger.hashCode` composition with int wraparound. The gcd
/// reduction never flips a sign, and cross multiplication ignores one, but
/// the composition itself negates for a sign-flipped triple: (1,2,3) and
/// (-1,-2,-3) are equal yet hash to 1026 and -1026 (states the constructor
/// rejects; pinned on a synthetic field state below).
impl Hash for RationalPoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut code = 0;
        if self.z.sign() != Sign::NoSign {
            let gcd = big_gcd(&big_gcd(&big_abs(&self.x), &big_abs(&self.y)), &self.z);
            let (rx, ry, rz);
            if gcd > BigInt::ONE {
                rx = &self.x / &gcd;
                ry = &self.y / &gcd;
                rz = &self.z / &gcd;
            } else {
                rx = self.x.clone();
                ry = self.y.clone();
                rz = self.z.clone();
            }
            let mut result = big_integer_hash_code(&rx);
            result = result
                .wrapping_mul(31)
                .wrapping_add(big_integer_hash_code(&ry));
            code = result
                .wrapping_mul(31)
                .wrapping_add(big_integer_hash_code(&rz));
        }
        state.write_i32(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    /// A `Hasher` that records the single `write_i32` payload, so tests can
    /// pin the exact `hashCode` value Rust writes.
    struct Capture(i32);

    impl Hasher for Capture {
        fn finish(&self) -> u64 {
            self.0 as u64
        }
        fn write(&mut self, _bytes: &[u8]) {}
        fn write_i32(&mut self, v: i32) {
            self.0 = v;
        }
    }

    fn hash_code_of(p: &RationalPoint) -> i32 {
        let mut c = Capture(0);
        p.hash(&mut c);
        c.0
    }

    /// jshell pins `P8`/`P9`: `Point.getInstance(6,10,4)` has hashCode 3040
    /// (triple reduced to (3,5,2) before the JDK BigInteger composition)
    /// and getId 6080 (raw, unreduced composition).
    #[test]
    fn hash_code_reduces_get_id_does_not() {
        let p = RationalPoint::new(b(6), b(10), b(4));
        assert_eq!(p.get_id(), 6080);
        assert_eq!(hash_code_of(&p), 3040);
        // (3, 5, 2) is already reduced: hashCode == its raw composition.
        let reduced = RationalPoint::new(b(3), b(5), b(2));
        assert_eq!(hash_code_of(&reduced), 3040);
        assert_eq!(reduced.get_id(), 3040);
    }

    /// jshell pins `Q1`/`Q2`: two points at infinity with different
    /// numerators are equal and hash to 0.
    #[test]
    fn points_at_infinity_are_equal_with_hash_zero() {
        let inf1 = RationalPoint::new(b(10), b(20), b(0));
        let inf2 = RationalPoint::new(b(30), b(40), b(0));
        assert_eq!(inf1, inf2);
        assert_eq!(hash_code_of(&inf1), 0);
        assert!(inf1.is_infinite());
        assert!(!RationalPoint::new(b(1), b(2), b(3)).is_infinite());
    }

    /// jshell pins `P11`/`P12`: (6,10,4) and (3,5,2) both represent
    /// (1.5, 2.5), so they are equal with equal hashes; unequal values are
    /// rejected by the cross multiplication.
    #[test]
    fn equality_is_cross_multiplication() {
        assert_eq!(
            RationalPoint::new(b(6), b(10), b(4)),
            RationalPoint::new(b(3), b(5), b(2))
        );
        assert_eq!(
            RationalPoint::new(b(12), b(20), b(10)),
            RationalPoint::new(b(6), b(10), b(5))
        );
        assert_eq!(
            RationalPoint::new(b(4), b(8), b(2)),
            RationalPoint::new(b(2), b(4), b(1))
        );
        assert_ne!(
            RationalPoint::new(b(1), b(2), b(3)),
            RationalPoint::new(b(1), b(3), b(3))
        );
        assert_ne!(
            RationalPoint::new(b(6), b(10), b(4)),
            RationalPoint::new(b(6), b(10), b(5))
        );
    }

    /// jshell pin `Q3`: a point at infinity approximates to
    /// (Float.MAX_VALUE, Float.MAX_VALUE).
    #[test]
    fn to_float_uses_float_max_for_infinity() {
        let f = RationalPoint::new(b(10), b(20), b(0)).to_float();
        assert_eq!(f.x, f32::MAX as f64);
        assert_eq!(f.y, f32::MAX as f64);
        // jshell pin `P57`: (3, 4, 2) approximates to (1.5, 2).
        let f = RationalPoint::new(b(3), b(4), b(2)).to_float();
        assert_eq!(f.x, 1.5);
        assert_eq!(f.y, 2.0);
    }

    /// jshell pins `R5`-`R12` for the compare overloads.
    #[test]
    fn compare_overloads_are_cross_multiplications() {
        let rp = RationalPoint::new(b(3), b(4), b(2));
        // compareX(IntPoint(2,4)): 3 vs z*2 = 4.
        assert_eq!(rp.compare_x_int_point(&IntPoint::new(2, 4)), -1);
        // compareX(RationalPoint(6,8,4)): 3*4 vs 6*2.
        assert_eq!(
            rp.compare_x_rational_point(&RationalPoint::new(b(6), b(8), b(4))),
            0
        );
        // compareX(IntPoint(5,0)): 3 vs 10.
        assert_eq!(rp.compare_x_int_point(&IntPoint::new(5, 0)), -1);
        // compareY(IntPoint(0,2)): 4 vs 4.
        assert_eq!(rp.compare_y_int_point(&IntPoint::new(0, 2)), 0);
        // compareY(RationalPoint(0,9,1)): 4*1 vs 9*2.
        assert_eq!(
            rp.compare_y_rational_point(&RationalPoint::new(b(0), b(9), b(1))),
            -1
        );
        // compareY(RationalPoint(6,10,4)) vs rp: 10*2 vs 4*4.
        let rp2 = RationalPoint::new(b(6), b(10), b(4));
        assert_eq!(rp2.compare_y_rational_point(&rp), 1);
        assert_eq!(rp2.compare_x_rational_point(&rp), 0);
    }

    /// jshell pin `Q13`: (3,4,2) translated by the RationalVector (1,0,3)
    /// gives the raw triple (11,12,6) — no down-conversion.
    #[test]
    fn translate_by_rational_vector_keeps_raw_triple() {
        let p = RationalPoint::new(b(3), b(4), b(2));
        let v = RationalVector::new(b(1), b(0), b(3));
        let Point::Rational(result) = p.translate_by_rational_vector(&v) else {
            panic!("expected a RationalPoint");
        };
        assert_eq!(
            (result.x.clone(), result.y.clone(), result.z.clone()),
            (b(11), b(12), b(6))
        );
    }

    /// Translating by an IntVector wraps it with denominator 1 first.
    #[test]
    fn translate_by_int_vector_wraps_the_vector() {
        let p = RationalPoint::new(b(3), b(4), b(2));
        let Point::Rational(result) = p.translate_by_int_vector(&IntVector::new(1, 0)) else {
            panic!("expected a RationalPoint");
        };
        assert_eq!(
            (result.x.clone(), result.y.clone(), result.z.clone()),
            (b(5), b(4), b(2))
        );
    }

    /// jshell pins `P37`/`P38` and `Q24`: difference results keep the raw
    /// rational representation.
    #[test]
    fn difference_by_keeps_raw_triples() {
        let rp = RationalPoint::new(b(3), b(4), b(2));
        // P37: rp - (1,1) = (1,2,2).
        let Vector::Rational(v1) = rp.difference_by_int_point(&IntPoint::new(1, 1)) else {
            panic!("expected a RationalVector");
        };
        assert_eq!(
            (v1.x.clone(), v1.y.clone(), v1.z.clone()),
            (b(1), b(2), b(2))
        );
        // Q24: rp - RationalPoint(1,1,1) = (1,2,2) as well.
        let Vector::Rational(v2) =
            rp.difference_by_rational_point(&RationalPoint::new(b(1), b(1), b(1)))
        else {
            panic!("expected a RationalVector");
        };
        assert_eq!(
            (v2.x.clone(), v2.y.clone(), v2.z.clone()),
            (b(1), b(2), b(2))
        );
    }

    /// The constructor rejects a negative denominator like Java's
    /// IllegalArgumentException.
    #[test]
    #[should_panic(expected = "RationalPoint: z is expected to be >= 0")]
    fn constructor_rejects_negative_z() {
        RationalPoint::new(b(1), b(2), b(-3));
    }

    /// The triple is stored as given: no gcd reduction, no z sign flip.
    #[test]
    fn constructor_does_not_normalize() {
        let p = RationalPoint::new(b(6), b(10), b(4));
        assert_eq!(p.get_id(), 6080);
        assert_eq!((p.x.clone(), p.y.clone(), p.z.clone()), (b(6), b(10), b(4)));
    }

    /// jshell pin `I8`: (-4,6,2) and (-2,3,1) are cross-mult equal and
    /// share the gcd-reduced hash -1828 (the reduction divides by 2 while
    /// keeping the numerator signs, and both then fold identically).
    #[test]
    fn negative_numerators_share_the_reduced_hash() {
        let p1 = RationalPoint::new(b(-4), b(6), b(2));
        let p2 = RationalPoint::new(b(-2), b(3), b(1));
        assert_eq!(p1, p2);
        assert_eq!(hash_code_of(&p1), -1828);
        assert_eq!(hash_code_of(&p2), -1828);
    }

    /// Sign-flip asymmetry of the hash contract: (1,2,3) and (-1,-2,-3)
    /// are cross-mult equal but hash to 1026 and -1026 (jshell pin `I3`
    /// composition; Java cannot construct the negative-z point — the
    /// constructor throws even via reflection — so the flipped state is
    /// built directly from the `pub(crate)` fields to pin the formula).
    #[test]
    fn sign_flip_negates_the_hash_composition() {
        let p = RationalPoint::new(b(1), b(2), b(3));
        assert_eq!(hash_code_of(&p), 1026);
        let flipped = RationalPoint {
            x: b(-1),
            y: b(-2),
            z: b(-3),
        };
        assert_eq!(p, flipped);
        assert_eq!(hash_code_of(&flipped), -1026);
    }

    /// Hand-derived pin: RationalPoint(1,1,2).toFloat() = (0.5, 0.5)
    /// (pinned in the Task 2 jshell battery), so surroundingOctagon
    /// floors/ceils to (0,0,1,1) with both diagonals through (0.5, 0.5).
    #[test]
    fn surrounding_octagon_pin() {
        let half = RationalPoint::new(b(1), b(1), b(2));
        assert_eq!(
            half.surrounding_octagon(),
            IntOctagon::new(0, 0, 1, 1, 0, 0, 1, 1)
        );
    }
}
