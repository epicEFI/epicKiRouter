//! Port of Java `app.freerouting.geometry.planar.IntVector`.
//!
//! The Java double-dispatch overloads (`add`, `sideOf`, `projection`,
//! `scalarProduct`, `addTo`) live on the [`crate::vector::Vector`] enum; the
//! concrete `IntVector` surface is mirrored here. Java declares
//! `public static final IntVector ZERO = new IntVector(0, 0)` on `Vector`;
//! it is [`IntVector::ZERO`] here. `IntVector` overrides neither
//! `hashCode()` nor `toString()` in Java, so no `Hash`/`Display` impls.
//!
//! Deferred to Task 3 (Directions): `toNormalizedDirection()`.

use crate::float_point::FloatPoint;

/// Implementation of the interface Vector via a tuple of integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntVector {
    /// The x coordinate of this vector.
    pub x: i32,
    /// The y coordinate of this vector.
    pub y: i32,
}

impl IntVector {
    /// Java `Vector.ZERO`.
    pub const ZERO: IntVector = IntVector { x: 0, y: 0 };

    /// Creates an IntVector from two integer coordinates. Java comment:
    /// "range check omitted for performance reasons" — so there is no check
    /// here either.
    pub const fn new(x: i32, y: i32) -> IntVector {
        IntVector { x, y }
    }

    /// Returns true, if both coordinates of this vector are 0.
    pub fn is_zero(&self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// Returns true, if the vector is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        self.x == 0 || self.y == 0
    }

    /// Returns true, if the vector is diagonal. `Math.abs` wraps at
    /// `i32::MIN` in Java, hence `wrapping_abs`.
    pub fn is_diagonal(&self) -> bool {
        self.x.wrapping_abs() == self.y.wrapping_abs()
    }

    /// Calculates the determinant of the matrix consisting of this Vector
    /// and other, in 64-bit arithmetic (IntVector.java:64-66).
    pub fn determinant(&self, other: &IntVector) -> i64 {
        (self.x as i64) * (other.y as i64) - (self.y as i64) * (other.x as i64)
    }

    /// Converts this vector to a FloatPoint.
    pub fn to_float(&self) -> FloatPoint {
        FloatPoint::new(self.x as f64, self.y as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned against JDK 25 jshell (`P3`, `P71`).
    #[test]
    fn zero_vector_predicates() {
        assert!(IntVector::ZERO.is_zero());
        assert!(!IntVector::new(2, 0).is_zero());
        assert!(IntVector::new(1, 0).is_orthogonal());
        assert!(!IntVector::new(1, 1).is_orthogonal());
        assert!(!IntVector::new(0, 1).is_diagonal());
        assert!(IntVector::new(1, 1).is_diagonal());
        // Math.abs wraps, so |i32::MIN| == |i32::MIN|.
        assert!(IntVector::new(i32::MIN, i32::MIN).is_diagonal());
    }

    /// jshell pins `P3`: `new IntVector(33554432,0).determinant(new
    /// IntVector(0,33554432)) == 1125899906842624` — an i32 product would
    /// wrap here; the Java computation is `(long) x * other.y - ...`.
    #[test]
    fn determinant_is_64_bit_at_the_crit_int_boundary() {
        let a = IntVector::new(33_554_432, 0);
        let b = IntVector::new(0, 33_554_432);
        assert_eq!(a.determinant(&b), 1_125_899_906_842_624);
        assert_eq!(b.determinant(&a), -1_125_899_906_842_624);
    }

    #[test]
    fn to_float_widens_coordinates() {
        let v = IntVector::new(3, -4);
        let f = v.to_float();
        assert_eq!(f.x, 3.0);
        assert_eq!(f.y, -4.0);
    }

    #[test]
    fn equality_is_componentwise() {
        assert_eq!(IntVector::new(1, 2), IntVector::new(1, 2));
        assert_ne!(IntVector::new(1, 2), IntVector::new(2, 1));
    }
}
