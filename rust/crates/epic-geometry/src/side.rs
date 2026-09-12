//! Port of Java `app.freerouting.geometry.planar.Side` merged with its
//! sibling sign type `app.freerouting.datastructures.Signum`: both are
//! three-valued (value > 0, value == 0, value < 0) and differ only in names.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Java `Side.ON_THE_LEFT` / `Signum.POSITIVE` (value > 0).
    Positive,
    /// Java `Side.COLLINEAR` / `Signum.ZERO`.
    Collinear,
    /// Java `Side.ON_THE_RIGHT` / `Signum.NEGATIVE` (value < 0).
    Negative,
}

impl Side {
    /// Java `Side.of(double)` / `Signum.of(double)`. NaN falls through both
    /// comparisons to the zero variant, exactly as in Java.
    ///
    /// Java's javadoc on `Side.of` states the inverse mapping; the
    /// implementation is ground truth.
    pub fn of(value: f64) -> Side {
        if value > 0.0 {
            Side::Positive
        } else if value < 0.0 {
            Side::Negative
        } else {
            Side::Collinear
        }
    }

    /// Java `Signum.asInt(double)`: +1, 0, or -1.
    pub fn as_int(value: f64) -> i32 {
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    }

    /// Java `Side.negate()` / `Signum.negate()`: the opposite side; the zero
    /// variant negates to itself.
    pub fn negate(self) -> Side {
        match self {
            Side::Positive => Side::Negative,
            Side::Negative => Side::Positive,
            Side::Collinear => Side::Collinear,
        }
    }
}

/// Java `Side.toString()` names. (`Signum.toString()` uses the variant names
/// "positive"/"zero"/"negative" instead.)
impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Side::Positive => "onTheLeft",
            Side::Collinear => "collinear",
            Side::Negative => "onTheRight",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_matches_java_sign_convention() {
        // Positive is ON_THE_LEFT in Java's Side convention.
        assert_eq!(Side::of(0.5), Side::Positive);
        assert_eq!(Side::of(-0.5), Side::Negative);
        assert_eq!(Side::of(0.0), Side::Collinear);
        assert_eq!(Side::of(-0.0), Side::Collinear); // -0.0 == 0.0
        assert_eq!(Side::of(f64::NAN), Side::Collinear);
    }

    #[test]
    fn as_int_is_signum() {
        assert_eq!(Side::as_int(1.0), 1);
        assert_eq!(Side::as_int(-1.0), -1);
        assert_eq!(Side::as_int(0.0), 0);
        assert_eq!(Side::as_int(f64::NAN), 0);
    }

    #[test]
    fn negate_flips_nonzero_and_fixes_zero() {
        assert_eq!(Side::Positive.negate(), Side::Negative);
        assert_eq!(Side::Negative.negate(), Side::Positive);
        assert_eq!(Side::Collinear.negate(), Side::Collinear);
        assert_eq!(Side::Positive.negate().negate(), Side::Positive);
    }

    #[test]
    fn display_uses_java_side_tostring_names() {
        assert_eq!(Side::Positive.to_string(), "onTheLeft");
        assert_eq!(Side::Negative.to_string(), "onTheRight");
        assert_eq!(Side::Collinear.to_string(), "collinear");
    }
}
