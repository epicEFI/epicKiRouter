//! Port of Java `app.freerouting.geometry.planar.Vector`.
//!
//! Java models vectors as an abstract class with two concrete
//! implementations (`IntVector`, `RationalVector`) and resolves the
//! `add`/`sideOf`/`projection`/`scalarProduct` overloads via double
//! dispatch. This port seals the hierarchy as a 2-variant enum and
//! implements every dispatch as a 2x2 `match`, preserving exactly which
//! concrete overload runs (including the asymmetric quirks — e.g. the
//! rational `sideOf` uses the raw numerators without z cross
//! multiplication).
//!
//! `projection` returns [`Side`] used as the port of Java `Signum`
//! (Positive = POSITIVE, Collinear = ZERO, Negative = NEGATIVE); see
//! [`crate::side`].
//!
//! Java `Vector.ZERO` is an `IntVector`; it is [`Vector::ZERO`] here.
//! Like the Java originals, the concrete vector classes override `equals`
//! but not `hashCode` (identity hash in Java), so the enum implements no
//! `Hash`.

use std::cmp::Ordering;

use crate::big_int_aux::{big_abs, big_integer_int_value, determinant};
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::limits::{CRIT_INT, CRIT_INT_BIG};
use crate::point::Point;
use crate::rational_point::RationalPoint;
use crate::rational_vector::RationalVector;
use crate::side::Side;
use num_bigint::{BigInt, Sign};

/// Abstract class describing functionality of Vectors, used for
/// translating points in the plane.
#[derive(Debug, Clone)]
pub enum Vector {
    /// Integer coordinates, valid and exact up to `|x|, |y| = CRIT_INT`.
    Int(IntVector),
    /// Rational coordinates `(x / z, y / z)` with unbounded BigIntegers.
    Rational(Box<RationalVector>),
}

impl Vector {
    /// Java `Vector.ZERO`.
    pub const ZERO: Vector = Vector::Int(IntVector::ZERO);

    /// Wraps an IntVector.
    pub const fn int(vector: IntVector) -> Vector {
        Vector::Int(vector)
    }

    /// Wraps a RationalVector.
    pub fn rational(vector: RationalVector) -> Vector {
        Vector::Rational(Box::new(vector))
    }

    /// Creates a Vector (x, y). If |x| or |y| exceeds CRIT_INT (with Java
    /// `Math.abs` wraparound at i32::MIN), a RationalVector is created.
    pub fn get_instance(x: i32, y: i32) -> Vector {
        let result = IntVector::new(x, y);
        if x.wrapping_abs() > CRIT_INT || y.wrapping_abs() > CRIT_INT {
            return Vector::Rational(Box::new(RationalVector::from_int_vector(&result)));
        }
        Vector::Int(result)
    }

    /// Creates a 2-dimensional Vector from the 3 input values. If z != 0
    /// it corresponds to the Vector with rational coordinates (x / z,
    /// y / z). Mirrors `Vector.getInstance(BigInteger x, BigInteger y,
    /// BigInteger z)`: the triple is sign-normalized (z >= 0); if z
    /// divides x, BOTH x and y are divided (y by truncation, even when
    /// not divisible — Java quirk); an integral result within CRIT_INT
    /// down-converts to an IntVector. Panics on z == 0 like Java's
    /// `ArithmeticException` (modulus not positive).
    pub fn get_instance_big(x: BigInt, y: BigInt, z: BigInt) -> Vector {
        let (x, y, z) = if z.sign() == Sign::Minus {
            // the denominator z of a RationalVector is expected to be positive
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
            // the Vector fits into an IntVector
            return Vector::Int(IntVector::new(
                big_integer_int_value(&x),
                big_integer_int_value(&y),
            ));
        }
        Vector::Rational(Box::new(RationalVector::new(x, y, z)))
    }

    /// Returns true, if this vector is equal to the zero vector.
    pub fn is_zero(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_zero(),
            Vector::Rational(v) => v.is_zero(),
        }
    }

    /// Returns the Vector such that this plus `self.negate()` is zero.
    /// The int negation wraps like Java's unary minus.
    pub fn negate(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(IntVector::new(v.x.wrapping_neg(), v.y.wrapping_neg())),
            Vector::Rational(v) => {
                Vector::rational(RationalVector::new(-v.x.clone(), -v.y.clone(), v.z.clone()))
            }
        }
    }

    /// Returns true, if the vector is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_orthogonal(),
            Vector::Rational(v) => v.x.sign() == Sign::NoSign || v.y.sign() == Sign::NoSign,
        }
    }

    /// Returns true, if the vector is diagonal. For rational vectors the
    /// comparison is on the absolute raw numerators (Java
    /// `x.abs().equals(y.abs())`).
    pub fn is_diagonal(&self) -> bool {
        match self {
            Vector::Int(v) => v.is_diagonal(),
            Vector::Rational(v) => big_abs(&v.x) == big_abs(&v.y),
        }
    }

    /// Returns true, if the vector is orthogonal or diagonal.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Adds other to this vector (Java `add(Vector)` double dispatch).
    pub fn add(&self, other: &Vector) -> Vector {
        match (self, other) {
            (Vector::Int(s), Vector::Int(o)) => {
                Vector::Int(IntVector::new(s.x.wrapping_add(o.x), s.y.wrapping_add(o.y)))
            }
            (Vector::Int(s), Vector::Rational(o)) => o.add_int_vector(s),
            (Vector::Rational(s), Vector::Int(o)) => s.add_int_vector(o),
            (Vector::Rational(s), Vector::Rational(o)) => s.add_rational(o),
        }
    }

    /// Let L be the line from the zero vector to other. Returns
    /// Side::Positive if this vector is on the left of L, Side::Negative
    /// if on the right, Side::Collinear if collinear. All four dispatch
    /// pairs reduce to the sign of `y_self * x_other - x_self * y_other`:
    /// for int pairs in double arithmetic, otherwise in raw rational
    /// numerators (Java divides nothing by z here — see RationalVector
    /// sideOf).
    pub fn side_of(&self, other: &Vector) -> Side {
        match (self, other) {
            (Vector::Int(s), Vector::Int(o)) => {
                // Java IntVector.sideOf(IntVector):
                // Side.of((double) other.x * y - (double) other.y * x)
                Side::of(o.x as f64 * s.y as f64 - o.y as f64 * s.x as f64)
            }
            (Vector::Int(s), Vector::Rational(o)) => side_from_sign(&determinant(
                &BigInt::from(s.y),
                &o.y,
                &BigInt::from(s.x),
                &o.x,
            )),
            (Vector::Rational(s), Vector::Int(o)) => side_from_sign(&determinant(
                &s.y,
                &BigInt::from(o.y),
                &s.x,
                &BigInt::from(o.x),
            )),
            (Vector::Rational(s), Vector::Rational(o)) => {
                // Java RationalVector.sideOf(RationalVector): raw numerators.
                side_from_sign(&determinant(&s.y, &o.y, &s.x, &o.x))
            }
        }
    }

    /// The sign of the scalar product of this vector and other (Java
    /// `Signum projection`, ported as [`Side`]). Int pairs multiply in
    /// double, pairs involving a RationalVector multiply the raw
    /// numerators as BigIntegers (the z denominators are ignored, exactly
    /// as in Java).
    pub fn projection(&self, other: &Vector) -> Side {
        match (self, other) {
            (Vector::Int(s), Vector::Int(o)) => {
                Side::of(s.x as f64 * o.x as f64 + s.y as f64 * o.y as f64)
            }
            (Vector::Int(s), Vector::Rational(o)) => side_from_sign(&rational_dot(
                &BigInt::from(s.x),
                &BigInt::from(s.y),
                &o.x,
                &o.y,
            )),
            (Vector::Rational(s), Vector::Int(o)) => side_from_sign(&rational_dot(
                &s.x,
                &s.y,
                &BigInt::from(o.x),
                &BigInt::from(o.y),
            )),
            (Vector::Rational(s), Vector::Rational(o)) => {
                side_from_sign(&rational_dot(&s.x, &s.y, &o.x, &o.y))
            }
        }
    }

    /// Returns an approximation of the scalar product of this vector with
    /// other by a double. Int pairs multiply exactly in double; any
    /// rational pair first converts both vectors to floats.
    pub fn scalar_product(&self, other: &Vector) -> f64 {
        match (self, other) {
            (Vector::Int(s), Vector::Int(o)) => s.x as f64 * o.x as f64 + s.y as f64 * o.y as f64,
            _ => {
                let v1 = self.to_float();
                let v2 = other.to_float();
                v1.x * v2.x + v1.y * v2.y
            }
        }
    }

    /// Approximates the coordinates of this vector by float coordinates.
    pub fn to_float(&self) -> FloatPoint {
        match self {
            Vector::Int(v) => v.to_float(),
            Vector::Rational(v) => v.to_float(),
        }
    }

    /// Turns this vector by factor times 90 degree (negative factors and
    /// values >= 4 wrap exactly like the Java while loops).
    pub fn turn_90_degree(&self, factor: i32) -> Vector {
        let mut n = factor;
        while n < 0 {
            n += 4;
        }
        while n >= 4 {
            n -= 4;
        }
        match (self, n) {
            (Vector::Int(v), 1) => Vector::Int(IntVector::new(v.y.wrapping_neg(), v.x)),
            (Vector::Int(v), 2) => {
                Vector::Int(IntVector::new(v.x.wrapping_neg(), v.y.wrapping_neg()))
            }
            (Vector::Int(v), 3) => Vector::Int(IntVector::new(v.y, v.x.wrapping_neg())),
            (Vector::Rational(v), 1) => {
                Vector::rational(RationalVector::new(-v.y.clone(), v.x.clone(), v.z.clone()))
            }
            (Vector::Rational(v), 2) => {
                Vector::rational(RationalVector::new(-v.x.clone(), -v.y.clone(), v.z.clone()))
            }
            (Vector::Rational(v), 3) => {
                Vector::rational(RationalVector::new(v.y.clone(), -v.x.clone(), v.z.clone()))
            }
            // 0 (and the unreachable default) return the vector unchanged.
            _ => self.clone(),
        }
    }

    /// Mirrors this vector at the x axis.
    pub fn mirror_at_x_axis(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(IntVector::new(v.x, v.y.wrapping_neg())),
            Vector::Rational(v) => {
                Vector::rational(RationalVector::new(v.x.clone(), -v.y.clone(), v.z.clone()))
            }
        }
    }

    /// Mirrors this vector at the y axis.
    pub fn mirror_at_y_axis(&self) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(IntVector::new(v.x.wrapping_neg(), v.y)),
            Vector::Rational(v) => {
                Vector::rational(RationalVector::new(-v.x.clone(), v.y.clone(), v.z.clone()))
            }
        }
    }

    /// Returns an approximation of the Euclidean length of this vector.
    pub fn length_approx(&self) -> f64 {
        self.to_float().size()
    }

    /// Returns an approximation of the cosine of the angle between this
    /// vector and other.
    pub fn cos_angle(&self, other: &Vector) -> f64 {
        let mut result = self.scalar_product(other);
        result /= self.to_float().size() * other.to_float().size();
        result
    }

    /// Returns an approximation of the signed angle between this vector
    /// and other (positive when other is on the left of this vector).
    pub fn angle_approx(&self, other: &Vector) -> f64 {
        let mut result = self.cos_angle(other).acos();
        if self.side_of(other) == Side::Positive {
            result = -result;
        }
        result
    }

    /// Returns an approximation of the signed angle between this vector
    /// and the x axis.
    pub fn angle_approx_x_axis(&self) -> f64 {
        Vector::Int(IntVector::new(1, 0)).angle_approx(self)
    }

    /// Returns an approximation vector of this vector with the same
    /// direction and the given length. RationalVectors are returned
    /// unchanged (Java logs "not yet implemented" there and returns this).
    pub fn change_length_approx(&self, length: f64) -> Vector {
        match self {
            Vector::Int(v) => Vector::Int(
                v.to_float()
                    .change_size(length)
                    .round()
                    .difference_by(&IntPoint::ZERO),
            ),
            Vector::Rational(_) => self.clone(),
        }
    }

    /// Java abstract `toNormalizedDirection()`: the gcd-reduced direction
    /// of this vector (double dispatch to the concrete vector classes).
    pub fn to_normalized_direction(&self) -> Direction {
        match self {
            Vector::Int(v) => v.to_normalized_direction(),
            Vector::Rational(v) => v.to_normalized_direction(),
        }
    }

    /// Java package-private `addTo(IntPoint)`: the translated point. The
    /// rational case builds the raw triple `(z * point.x + x, z * point.y
    /// + y, z)` without down-conversion, exactly as in Java.
    pub(crate) fn add_to_int_point(&self, point: &IntPoint) -> Point {
        match self {
            Vector::Int(v) => Point::int(IntPoint::new(
                point.x.wrapping_add(v.x),
                point.y.wrapping_add(v.y),
            )),
            Vector::Rational(v) => {
                let new_x = &v.z * BigInt::from(point.x) + &v.x;
                let new_y = &v.z * BigInt::from(point.y) + &v.y;
                Point::rational(RationalPoint::new(new_x, new_y, v.z.clone()))
            }
        }
    }

    /// Java package-private `addTo(RationalPoint)`: sums the `[x, y, z]`
    /// triples via `BigIntAux.addRationalCoordinates`.
    pub(crate) fn add_to_rational_point(&self, point: &RationalPoint) -> Point {
        match self {
            Vector::Int(v) => point.translate_by_int_vector(v),
            Vector::Rational(v) => {
                let v1 = [v.x.clone(), v.y.clone(), v.z.clone()];
                let v2 = [point.x.clone(), point.y.clone(), point.z.clone()];
                let result = crate::big_int_aux::add_rational_coordinates(&v1, &v2);
                Point::rational(RationalPoint::new(
                    result[0].clone(),
                    result[1].clone(),
                    result[2].clone(),
                ))
            }
        }
    }
}

/// Maps a BigInteger determinant (or scalar product) sign to a side:
/// Java `Side.of(int signum)` and `Signum.of(int)`.
fn side_from_sign(det: &BigInt) -> Side {
    match det.sign() {
        Sign::Plus => Side::Positive,
        Sign::NoSign => Side::Collinear,
        Sign::Minus => Side::Negative,
    }
}

/// The raw rational scalar product `x1*x2 + y1*y2` (no denominators),
/// Java `RationalVector.projection(RationalVector)`.
fn rational_dot(x1: &BigInt, y1: &BigInt, x2: &BigInt, y2: &BigInt) -> BigInt {
    x1 * x2 + y1 * y2
}

/// Java `equals`: IntVector pairs are componentwise equal, RationalVector
/// pairs compare by cross multiplication, different classes are never
/// equal (`getClass()` check).
impl PartialEq for Vector {
    fn eq(&self, other: &Vector) -> bool {
        match (self, other) {
            (Vector::Int(s), Vector::Int(o)) => s == o,
            (Vector::Rational(s), Vector::Rational(o)) => s == o,
            _ => false,
        }
    }
}

impl Eq for Vector {}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    fn rat(x: i64, y: i64, z: i64) -> Vector {
        Vector::rational(RationalVector::new(b(x), b(y), b(z)))
    }

    fn int(x: i32, y: i32) -> Vector {
        Vector::Int(IntVector::new(x, y))
    }

    /// Invariant (Task 10): the Int-Int `sideOf` is EXACTLY antisymmetric
    /// under operand swap even though it computes in doubles. Confirmed
    /// against the oracle jar (seed 20260912, 100k full-range
    /// i32 pairs: zero asymmetric pairs; rounding-magnitude and
    /// signed-zero probes included). Why it is no accident: the two
    /// products fl(oy*x) / fl(ox*y) are IEEE round-to-nearest and
    /// commutative, so both swap directions round the SAME two doubles,
    /// and fl(q - p) == -fl(p - q) holds for every finite double pair
    /// (RN negation symmetry; the signed-zero corners -0.0 - 0.0 vs
    /// 0.0 - -0.0 both classify as zero). The Rational-Rational and
    /// mixed branches use exact BigInteger arithmetic — antisymmetric
    /// trivially. This test asserts WHAT JAVA DOES; do not "fix" the
    /// double rounding to exact arithmetic here.
    #[test]
    fn side_of_is_antisymmetric_under_swap() {
        use crate::java_random::JavaRandom;
        let mut rng = JavaRandom::new(0x5EED_C0DE_1234_567B);
        let coord = |rng: &mut JavaRandom| {
            if rng.next_int_bound(4) == 0 {
                rng.next_int()
            } else {
                rng.next_int_bound(20001) - 10_000
            }
        };
        for _ in 0..10_000 {
            let va = int(coord(&mut rng), coord(&mut rng));
            let vb = int(coord(&mut rng), coord(&mut rng));
            assert_eq!(
                va.side_of(&vb),
                vb.side_of(&va).negate(),
                "sideOf not antisymmetric for {va:?} x {vb:?}"
            );
        }
    }

    /// jshell pins `P54`/`P55`: getInstance(4,3,2) down-converts to the
    /// IntVector (2, 1) — x divisible by z, and y divided anyway (3/2
    /// truncates to 1), the Java T13 quirk.
    #[test]
    fn get_instance_big_divides_both_coordinates() {
        let v = Vector::get_instance_big(b(4), b(3), b(2));
        assert_eq!(v, int(2, 1));
    }

    /// jshell pins `P17`/`P18` analog: CRIT_INT stays an IntVector,
    /// CRIT_INT + 1 promotes. `Math.abs(i32::MIN)` wraps to the negative
    /// i32::MIN, so Java does NOT promote an IntVector(i32::MIN, 0) — the
    /// `wrapping_abs` port reproduces that exactly.
    #[test]
    fn get_instance_promotes_beyond_crit_int() {
        assert_eq!(Vector::get_instance(CRIT_INT, 0), int(CRIT_INT, 0));
        assert_eq!(
            Vector::get_instance(CRIT_INT + 1, 0),
            rat(CRIT_INT as i64 + 1, 0, 1)
        );
        // Math.abs(i32::MIN) wraps to i32::MIN, which is NOT > CRIT_INT.
        assert_eq!(Vector::get_instance(i32::MIN, 0), int(i32::MIN, 0));
    }

    /// jshell pins `P32`/`P33`/`P34`/`P35`: sideOf semantics, including
    /// the raw-numerator rational quirk.
    #[test]
    fn side_of_matches_java() {
        assert_eq!(int(0, 1).side_of(&int(1, 0)), Side::Positive);
        assert_eq!(int(1, 0).side_of(&int(0, 1)), Side::Negative);
        // P34: (2,0,4) vs (0,2,8) -> onTheRight despite the parallel
        // doubles (2/0 not involved): raw det = 0*0 - 2*2 = -4.
        assert_eq!(rat(2, 0, 4).side_of(&rat(0, 2, 8)), Side::Negative);
        // P35: det = 67108865^2 is exact in f64.
        assert_eq!(
            int(67_108_865, 0).side_of(&int(0, -67_108_865)),
            Side::Positive
        );
        // Mixed pairs wrap the IntVector first.
        assert_eq!(int(0, 1).side_of(&rat(1, 0, 1)), Side::Positive);
        assert_eq!(rat(0, 1, 3).side_of(&int(1, 0)), Side::Positive);
    }

    /// jshell pins `Q16`/`Q8`/`P66`: addition keeps the raw rational
    /// representation.
    #[test]
    fn add_keeps_raw_rational_representation() {
        // Q16: (1,2,3) + (1,1,5) = (1*5+1*3, 2*5+1*3, 15) = (8,13,15).
        assert_eq!(rat(1, 2, 3).add(&rat(1, 1, 5)), rat(8, 13, 15));
        // Q8: (1,1,2) + (1,1,3) = (5,5,6).
        assert_eq!(rat(1, 1, 2).add(&rat(1, 1, 3)), rat(5, 5, 6));
        // P66: (1,2) + (1,1,2) = (3,5,2) as a RationalVector.
        assert_eq!(int(1, 2).add(&rat(1, 1, 2)), rat(3, 5, 2));
        assert_eq!(rat(1, 1, 2).add(&int(1, 2)), rat(3, 5, 2));
        // Int + Int wraps like Java int addition.
        assert_eq!(
            int(i32::MAX, 0).add(&int(1, 0)),
            int(i32::MAX.wrapping_add(1), 0)
        );
    }

    /// jshell pins `P49`/`P50`/`Q35`-`Q38`/`R13`.
    #[test]
    fn projection_and_scalar_product() {
        // P49: (1,2) projected on (3,4) is positive; P50: product 11.0.
        assert_eq!(int(1, 2).projection(&int(3, 4)), Side::Positive);
        assert_eq!(int(1, 2).scalar_product(&int(3, 4)), 11.0);
        // Q35: rational scalar product via floats: 0.7333...
        assert_eq!(
            rat(1, 2, 3).scalar_product(&rat(3, 4, 5)),
            0.733_333_333_333_333_3
        );
        // Q36: int * rational: (1,2) . (3/5, 4/5) = 2.2.
        assert_eq!(int(1, 2).scalar_product(&rat(3, 4, 5)), 2.2);
        // Q37: rational projection uses raw numerators: 1*3 + 2*4 = 11.
        assert_eq!(rat(1, 2, 3).projection(&int(3, 4)), Side::Positive);
        // Q38: 67108865^2 is exact in double: positive.
        assert_eq!(
            int(67_108_865, 0).projection(&int(67_108_865, 0)),
            Side::Positive
        );
        // R13: zero projection.
        assert_eq!(rat(0, 0, 1).projection(&int(0, 0)), Side::Collinear);
        // jshell pins `I2a`/`I2b`: the (Rational, Rational) arm multiplies
        // the RAW numerators (1*3 + 2*4 = 11 > 0), no z division anywhere;
        // deleting the BigInt arms previously passed the suite (mutation
        // trap).
        assert_eq!(rat(1, 2, 3).projection(&rat(3, 4, 5)), Side::Positive);
        assert_eq!(rat(-1, -2, 3).projection(&rat(3, 4, 5)), Side::Negative);
        // Negative projection.
        assert_eq!(int(1, 0).projection(&int(-3, 0)), Side::Negative);
    }

    /// jshell pins `P51`-`P53`, `Q9`-`Q11`, `Q31`, `Q14`: 90 degree turns
    /// and mirrors for both variants.
    #[test]
    fn turns_and_mirrors() {
        assert_eq!(int(1, 2).turn_90_degree(1), int(-2, 1));
        assert_eq!(int(1, 2).mirror_at_y_axis(), int(-1, 2));
        assert_eq!(int(1, 2).mirror_at_x_axis(), int(1, -2));
        // Q31: factor -1 wraps to 270 degrees: (y, -x).
        assert_eq!(int(-1, 2).turn_90_degree(-1), int(2, 1));
        assert_eq!(int(1, 2).turn_90_degree(4), int(1, 2));
        // Q14: rational turns keep the denominator.
        assert_eq!(rat(1, 2, 3).turn_90_degree(1), rat(-2, 1, 3));
        assert_eq!(rat(1, 2, 3).turn_90_degree(2), rat(-1, -2, 3));
        assert_eq!(rat(1, 2, 3).mirror_at_y_axis(), rat(-1, 2, 3));
        assert_eq!(rat(1, 2, 3).mirror_at_x_axis(), rat(1, -2, 3));
        // Negation wraps for ints and keeps z for rationals.
        assert_eq!(int(i32::MIN, 5).negate(), int(i32::MIN, -5));
        assert_eq!(rat(1, 2, 3).negate(), rat(-1, -2, 3));
    }

    /// jshell pins `P60`/`Q12`: (3,4) with length 6 -> (3.6, 4.8) ->
    /// Math.round -> (4, 5).
    #[test]
    fn change_length_approx_rounds_int_vectors() {
        assert_eq!(int(3, 4).change_length_approx(6.0), int(4, 5));
        // RationalVectors are returned unchanged (Java: not implemented).
        assert_eq!(rat(1, 2, 3).change_length_approx(6.0), rat(1, 2, 3));
    }

    /// jshell pins `P67`/`P68`/`P61`: length, cosine and angle pins.
    #[test]
    fn length_cosine_and_angle() {
        assert_eq!(int(3, 4).length_approx(), 5.0);
        assert_eq!(int(1, 0).cos_angle(&int(0, 1)), 0.0);
        // jshell pin P61 = 1.5707963267948966 = FRAC_PI_2.
        assert_eq!(
            int(1, 0).angle_approx(&int(0, 1)),
            std::f64::consts::FRAC_PI_2
        );
        // Signed: (0,1) relative to (1,0) is on the right: -PI/2.
        assert_eq!(
            int(0, 1).angle_approx(&int(1, 0)),
            -std::f64::consts::FRAC_PI_2
        );
        // angle_approx() against the x axis.
        assert_eq!(int(0, 1).angle_approx_x_axis(), std::f64::consts::FRAC_PI_2);
    }

    /// jshell pins `Q19`/`P73`: a RationalVector is never equal to an
    /// IntVector, not even the zero vector (Java getClass check).
    #[test]
    fn cross_class_equality_is_false() {
        assert_ne!(rat(1, 2, 3), int(1, 2));
        assert_ne!(rat(0, 0, 1), Vector::ZERO);
        assert_eq!(int(1, 2), int(1, 2));
        assert_eq!(rat(1, 2, 3), rat(2, 4, 6));
        assert_ne!(rat(1, 2, 3), rat(2, 4, 5));
    }

    /// jshell pin `Q22` and the toFloat dispatch.
    #[test]
    fn to_float_divides_doubles_for_rationals() {
        assert_eq!(rat(2, 4, 8).to_float(), FloatPoint::new(0.25, 0.5));
        assert_eq!(int(3, -4).to_float(), FloatPoint::new(3.0, -4.0));
        // doubleValue() saturates like the JDK.
        let big = BigInt::from(10u32).pow(400);
        let v = Vector::Rational(Box::new(RationalVector::new(
            big.clone(),
            big.clone(),
            b(1),
        )));
        assert_eq!(v.to_float().x, f64::INFINITY);
        assert_eq!(v.to_float().y, f64::INFINITY);
    }

    /// Predicates on both variants.
    #[test]
    fn predicates() {
        assert!(Vector::ZERO.is_zero());
        assert!(!rat(2, 0, 4).is_zero());
        assert!(rat(2, 0, 4).is_orthogonal());
        assert!(rat(2, 2, 4).is_diagonal());
        assert!(!rat(2, 3, 4).is_multiple_of_45_degree());
        assert!(int(0, 5).is_multiple_of_45_degree());
        assert!(int(5, 5).is_multiple_of_45_degree());
    }

    /// addTo dispatch: int vectors produce IntPoints (wrapping), rational
    /// vectors build the raw triple without down-conversion.
    #[test]
    fn add_to_points() {
        // Int + IntPoint stays an IntPoint and wraps.
        assert_eq!(
            int(i32::MAX, 0).add_to_int_point(&IntPoint::new(1, 0)),
            Point::int(IntPoint::new(i32::MIN, 0))
        );
        // Rational addTo builds (z * p + v, z): (2*1+1, 2*0+0, 2).
        assert_eq!(
            rat(1, 0, 2).add_to_int_point(&IntPoint::new(1, 0)),
            Point::rational(RationalPoint::new(b(3), b(0), b(2)))
        );
        // Rational addTo(RationalPoint) sums the triples:
        // (1,0,3) + (3,4,2) -> (1*2+3*3, 0*2+4*3, 6) = (11, 12, 6).
        assert_eq!(
            rat(1, 0, 3).add_to_rational_point(&RationalPoint::new(b(3), b(4), b(2))),
            Point::rational(RationalPoint::new(b(11), b(12), b(6)))
        );
        // Int addTo(RationalPoint) delegates to the rational translation.
        assert_eq!(
            int(1, 0).add_to_rational_point(&RationalPoint::new(b(3), b(4), b(2))),
            Point::rational(RationalPoint::new(b(5), b(4), b(2)))
        );
    }

    /// The BigInt factory panics on z == 0 like Java's
    /// ArithmeticException (BigInteger: modulus not positive).
    #[test]
    #[should_panic]
    fn get_instance_big_panics_on_zero_z() {
        Vector::get_instance_big(b(1), b(2), b(0));
    }

    /// jshell pin `P72` analog with the java_round dependency: the
    /// factory down-conversion covers negative values ((-2,-4,-2)
    /// normalizes to (1,2), pin `P19` at Point level).
    #[test]
    fn get_instance_big_normalizes_negative_z() {
        assert_eq!(Vector::get_instance_big(b(-2), b(-4), b(-2)), int(1, 2));
    }
}
