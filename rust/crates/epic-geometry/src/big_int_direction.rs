//! Port of Java `app.freerouting.geometry.planar.BigIntDirection`.

use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};

use crate::big_int_aux::big_abs;
use crate::direction::Direction;
use crate::int_direction::IntDirection;
use crate::rational_vector::RationalVector;
use crate::vector::Vector;

/// Implements the abstract class Direction as a tuple of infinite
/// precision integers.
///
/// Like the Java original, this class overrides no `equals` of its own
/// (the inherited `Direction.equals` lives on the
/// [`crate::direction::Direction`] enum), so this struct derives no
/// `PartialEq` either.
#[derive(Debug, Clone)]
pub struct BigIntDirection {
    /// The x coordinate of this direction.
    pub x: BigInt,
    /// The y coordinate of this direction.
    pub y: BigInt,
}

impl BigIntDirection {
    /// Java package-private constructor `BigIntDirection(BigInteger,
    /// BigInteger)`.
    pub fn new(x: BigInt, y: BigInt) -> BigIntDirection {
        BigIntDirection { x, y }
    }

    /// Creates a BigIntDirection from an IntDirection.
    pub fn from_int_direction(dir: &IntDirection) -> BigIntDirection {
        BigIntDirection::new(BigInt::from(dir.x), BigInt::from(dir.y))
    }

    /// Returns true, if the direction is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        self.x.sign() == Sign::NoSign || self.y.sign() == Sign::NoSign
    }

    /// Returns true, if the direction is diagonal.
    pub fn is_diagonal(&self) -> bool {
        big_abs(&self.x) == big_abs(&self.y)
    }

    /// Returns any Vector pointing into this direction (a RationalVector
    /// with denominator 1).
    pub fn get_vector(&self) -> Vector {
        Vector::rational(RationalVector::new(
            self.x.clone(),
            self.y.clone(),
            BigInt::ONE,
        ))
    }

    /// Java logs "BigIntDirection: turn_45_degree not yet implemented" and
    /// returns `this`. The geometry kernel has no logging infrastructure,
    /// so only the return value is reproduced: the direction is returned
    /// unchanged for every factor.
    pub fn turn_45_degree(&self, _factor: i32) -> Direction {
        Direction::BigInt(Box::new(self.clone()))
    }

    /// Returns the opposite direction of this direction.
    pub fn opposite(&self) -> BigIntDirection {
        BigIntDirection::new(-self.x.clone(), -self.y.clone())
    }
}

/// The signum of a BigInteger as an int (`BigInteger.signum()`).
fn signum(value: &BigInt) -> i32 {
    match value.sign() {
        Sign::Plus => 1,
        Sign::NoSign => 0,
        Sign::Minus => -1,
    }
}

/// Java package-private `BigIntDirection.compareTo(BigIntDirection)`: the
/// angle-order worker with the same half-plane branch structure as the
/// IntDirection worker, but an exact BigInteger determinant
/// (`y*other.x - x*other.y`).
pub(crate) fn compare_big_big(a: &BigIntDirection, b: &BigIntDirection) -> Ordering {
    let x1 = signum(&a.x);
    let y1 = signum(&a.y);
    let x2 = signum(&b.x);
    let y2 = signum(&b.y);
    if y1 > 0 {
        if y2 < 0 {
            return Ordering::Less;
        }
        if y2 == 0 {
            return if x2 > 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
    } else if y1 < 0 {
        if y2 >= 0 {
            return Ordering::Greater;
        }
    } else {
        // y1 == 0
        if x1 > 0 {
            if y2 != 0 || x2 < 0 {
                return Ordering::Less;
            }
            return Ordering::Equal;
        }
        // x1 <= 0
        if y2 > 0 || (y2 == 0 && x2 > 0) {
            return Ordering::Greater;
        }
        if y2 < 0 {
            return Ordering::Less;
        }
        return Ordering::Equal;
    }

    // now this direction and other are located in the same
    // open horizontal half plane

    // Java: tmp1 = y.multiply(other.x); tmp2 = x.multiply(other.y);
    // determinant = tmp1.subtract(tmp2).
    let determinant = &a.y * &b.x - &a.x * &b.y;
    match determinant.sign() {
        Sign::Plus => Ordering::Greater,
        Sign::NoSign => Ordering::Equal,
        Sign::Minus => Ordering::Less,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    fn big(x: i64, y: i64) -> Direction {
        Direction::BigInt(Box::new(BigIntDirection::new(b(x), b(y))))
    }

    /// The BigInt worker agrees with the Int worker on small directions
    /// (ascending order RIGHT < UP < UP45), and collinear same-sign
    /// directions compare equal.
    #[test]
    fn compare_to_matches_the_int_order() {
        assert_eq!(big(0, 1).compare_to(&big(1, 0)), Ordering::Greater);
        assert_eq!(big(1, 0).compare_to(&big(0, 1)), Ordering::Less);
        assert_eq!(big(0, 1).compare_to(&big(-1, 1)), Ordering::Less);
        assert_eq!(big(3, 2).compare_to(&big(6, 4)), Ordering::Equal);
    }

    /// The cross-class double dispatch: BigInt vs Int widens the int side
    /// (Java `BigIntDirection.compareTo(IntDirection)`), and Int vs BigInt
    /// negates the widened worker (Java
    /// `IntDirection.compareTo(BigIntDirection)`).
    #[test]
    fn cross_class_compare_to() {
        let up = Direction::Int(IntDirection::UP);
        let right = Direction::Int(IntDirection::RIGHT);
        assert_eq!(big(0, 1).compare_to(&up), Ordering::Equal);
        assert_eq!(up.compare_to(&big(1, 0)), Ordering::Greater);
        assert_eq!(right.compare_to(&big(0, 1)), Ordering::Less);
        // Exact beyond the double determinant: the angles atan(1/2^60) and
        // atan(1/(2^60+1)) differ below double precision, yet the exact
        // BigInteger worker orders them — the steeper (2^60, 1) is
        // GREATER in the ascending-angle order.
        let a = BigIntDirection::new(BigInt::from(1u32) << 60u32, b(1));
        let b_dir = BigIntDirection::new((BigInt::from(1u32) << 60u32) + b(1), b(1));
        assert_eq!(
            Direction::BigInt(Box::new(a)).compare_to(&Direction::BigInt(Box::new(b_dir))),
            Ordering::Greater
        );
    }

    /// turn45Degree warns and returns this for every factor; opposite
    /// negates both coordinates.
    #[test]
    fn turn_returns_self_and_opposite_negates() {
        let d_dir = big(3, 2);
        assert_eq!(d_dir.turn_45_degree(1), d_dir.clone());
        assert_eq!(d_dir.turn_45_degree(-3), d_dir.clone());
        let opp = BigIntDirection::new(b(3), b(2)).opposite();
        assert_eq!(opp.x, b(-3));
        assert_eq!(opp.y, b(-2));
    }

    /// Predicates on signums and absolute values; the vector round trip
    /// builds a denominator-1 RationalVector.
    #[test]
    fn predicates_and_vector() {
        let diagonal = BigIntDirection::new(b(3), b(-3));
        assert!(!diagonal.is_orthogonal());
        assert!(diagonal.is_diagonal());
        let orthogonal = BigIntDirection::new(b(0), b(7));
        assert!(orthogonal.is_orthogonal());
        assert!(!orthogonal.is_diagonal());
        assert!(Direction::BigInt(Box::new(diagonal.clone())).is_multiple_of_45_degree());
        let v = diagonal.get_vector();
        match v {
            Vector::Rational(r) => {
                assert_eq!(r.x, b(3));
                assert_eq!(r.y, b(-3));
                assert_eq!(r.z, b(1));
            }
            Vector::Int(_) => panic!("expected a rational vector"),
        }
        // from_int_direction widens the int coordinates.
        let widened = BigIntDirection::from_int_direction(&IntDirection::UP45);
        assert_eq!(widened.x, b(-1));
        assert_eq!(widened.y, b(1));
    }
}
