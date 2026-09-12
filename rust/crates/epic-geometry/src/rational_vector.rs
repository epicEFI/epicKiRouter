//! Port of Java `app.freerouting.geometry.planar.RationalVector`.
//!
//! The Java double-dispatch overloads (`add`, `sideOf`, `projection`,
//! `scalarProduct`, `addTo`) live on the [`crate::vector::Vector`] enum.
//! Like the Java original, `RationalVector` overrides `equals` (cross
//! multiplication) but NOT `hashCode`, so there is no `Hash` impl, and it
//! has no `toString`, so there is no `Display` impl.

use std::cmp::Ordering;

use crate::big_int_aux::{
    big_abs, big_gcd, big_integer_double_value, big_integer_int_value, determinant,
};
use crate::big_int_direction::BigIntDirection;
use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_direction::IntDirection;
use crate::int_vector::IntVector;
use crate::limits::CRIT_INT_BIG;
use num_bigint::{BigInt, Sign};

/// Analog RationalPoint, but implementing the functionality of a Vector
/// instead of the functionality of a Point.
#[derive(Debug, Clone)]
pub struct RationalVector {
    /// The x numerator; the 2-dimensional vector is `(x / z, y / z)`.
    pub x: BigInt,
    /// The y numerator.
    pub y: BigInt,
    /// The denominator; never negative after construction.
    pub z: BigInt,
}

impl RationalVector {
    /// Creates a RationalVector from 3 BigIntegers x, y and z. The sign of
    /// z is normalized to non-negative (the only constructor normalization
    /// the Java original performs — no gcd reduction).
    pub fn new(x: BigInt, y: BigInt, z: BigInt) -> RationalVector {
        if z.sign() == Sign::Minus {
            RationalVector {
                x: -x,
                y: -y,
                z: -z,
            }
        } else {
            RationalVector { x, y, z }
        }
    }

    /// Creates a RationalVector from an IntVector (Java package-private
    /// constructor, z = 1).
    pub(crate) fn from_int_vector(vector: &IntVector) -> RationalVector {
        RationalVector {
            x: BigInt::from(vector.x),
            y: BigInt::from(vector.y),
            z: BigInt::ONE,
        }
    }

    /// Returns true, if the x and y coordinates of this vector are 0.
    pub fn is_zero(&self) -> bool {
        self.x.sign() == Sign::NoSign && self.y.sign() == Sign::NoSign
    }

    /// Approximates the coordinates of this vector by float coordinates
    /// (numerators and denominator are converted to double first, then
    /// divided, exactly as in Java).
    pub fn to_float(&self) -> FloatPoint {
        let xd = big_integer_double_value(&self.x);
        let yd = big_integer_double_value(&self.y);
        let zd = big_integer_double_value(&self.z);
        FloatPoint::new(xd / zd, yd / zd)
    }

    /// Java package-private `toNormalizedDirection()`: gcd-reduces the raw
    /// numerators (Java `dx.gcd(y)`), down-converts to an IntDirection
    /// when both reduced numerators fit within CRIT_INT_BIG, and otherwise
    /// builds a BigIntDirection. Dividing by the zero gcd (both numerators
    /// zero) panics exactly like Java's `BigInteger.divide(0)`
    /// ArithmeticException.
    pub fn to_normalized_direction(&self) -> Direction {
        let dx = self.x.clone();
        let gcd = big_gcd(&dx, &self.y);
        let dx = dx / &gcd;
        let dy = &self.y / &gcd;
        if big_abs(&dx).cmp(&*CRIT_INT_BIG) != Ordering::Greater
            && big_abs(&dy).cmp(&*CRIT_INT_BIG) != Ordering::Greater
        {
            Direction::Int(IntDirection::new(
                big_integer_int_value(&dx),
                big_integer_int_value(&dy),
            ))
        } else {
            Direction::BigInt(Box::new(BigIntDirection::new(dx, dy)))
        }
    }

    /// Java package-private `add(IntVector)`: wraps the IntVector, then
    /// performs the rational addition.
    pub(crate) fn add_int_vector(&self, other: &IntVector) -> crate::vector::Vector {
        let vector = RationalVector::from_int_vector(other);
        self.add_rational(&vector)
    }

    /// Java package-private `add(RationalVector)`: sums the `[x, y, z]`
    /// triples via `BigIntAux.addRationalCoordinates` (never gcd-reduced).
    pub(crate) fn add_rational(&self, other: &RationalVector) -> crate::vector::Vector {
        let v1 = [self.x.clone(), self.y.clone(), self.z.clone()];
        let v2 = [other.x.clone(), other.y.clone(), other.z.clone()];
        let result = crate::big_int_aux::add_rational_coordinates(&v1, &v2);
        crate::vector::Vector::Rational(Box::new(RationalVector::new(
            result[0].clone(),
            result[1].clone(),
            result[2].clone(),
        )))
    }
}

/// Java `equals`: cross multiplication via `BigIntAux.determinant` —
/// `x1*z2 == x2*z1 && y1*z2 == y2*z1`. Two vectors with different
/// denominators can therefore be equal.
impl PartialEq for RationalVector {
    fn eq(&self, other: &Self) -> bool {
        let det = determinant(&self.x, &other.x, &self.z, &other.z);
        if det.sign() != Sign::NoSign {
            return false;
        }
        let det = determinant(&self.y, &other.y, &self.z, &other.z);
        det.sign() == Sign::NoSign
    }
}

impl Eq for RationalVector {}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    /// jshell pin `Q15`: `new RationalVector(-1, -2, -3)` stores
    /// `(1, 2, 3)` — the constructor normalizes the sign of z only.
    #[test]
    fn constructor_normalizes_sign_of_z() {
        let v = RationalVector::new(b(-1), b(-2), b(-3));
        assert_eq!(v.x, b(1));
        assert_eq!(v.y, b(2));
        assert_eq!(v.z, b(3));
    }

    #[test]
    fn from_int_vector_uses_denominator_one() {
        let v = RationalVector::from_int_vector(&IntVector::new(3, -4));
        assert_eq!(v.x, b(3));
        assert_eq!(v.y, b(-4));
        assert_eq!(v.z, b(1));
    }

    /// jshell pins `Q17`–`Q19`: `(1,2,3) == (2,4,6)`, `(1,2,3) != (2,4,5)`,
    /// and a RationalVector is never equal to the IntVector zero (Java's
    /// `getClass()` check — that half lives in the `Vector` enum tests).
    #[test]
    fn equals_is_cross_multiplication() {
        assert_eq!(
            RationalVector::new(b(1), b(2), b(3)),
            RationalVector::new(b(2), b(4), b(6))
        );
        assert_ne!(
            RationalVector::new(b(1), b(2), b(3)),
            RationalVector::new(b(2), b(4), b(5))
        );
        // The all-zero triple equals itself (determinants all 0).
        assert_eq!(
            RationalVector::new(b(0), b(0), b(0)),
            RationalVector::new(b(0), b(0), b(0))
        );
    }

    #[test]
    fn is_zero_requires_both_numerators_zero() {
        assert!(RationalVector::new(b(0), b(0), b(7)).is_zero());
        assert!(!RationalVector::new(b(2), b(0), b(4)).is_zero());
    }

    /// jshell pin `Q22`: `(2, 4, 8).toFloat() == (0.25, 0.5)`.
    #[test]
    fn to_float_divides_doubles() {
        let f = RationalVector::new(b(2), b(4), b(8)).to_float();
        assert_eq!(f.x, 0.25);
        assert_eq!(f.y, 0.5);
    }

    /// jshell pins `J`/`K`: a coprime pair beyond CRIT_INT_BIG promotes to
    /// a BigIntDirection — gcd(2^40, 2^40+1) = 1 (consecutive integers),
    /// so nothing reduces, and both numerators exceed the 2^25
    /// down-conversion bound.
    #[test]
    fn to_normalized_direction_promotes_to_big_int_direction() {
        let huge = BigInt::from(1u32) << 40u32;
        let expected_y = huge.clone() + b(1);
        let d =
            RationalVector::new(huge.clone(), expected_y.clone(), b(1)).to_normalized_direction();
        match d {
            Direction::BigInt(dir) => {
                assert_eq!(dir.x, huge);
                assert_eq!(dir.y, expected_y);
            }
            Direction::Int(_) => panic!("expected promotion to a BigIntDirection"),
        }
    }
}
