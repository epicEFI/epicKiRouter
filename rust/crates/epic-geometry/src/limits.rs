//! Port of Java `app.freerouting.geometry.planar.Limits`.

use num_bigint::BigInt;
use std::sync::LazyLock;

/// Java `Limits.CRIT_INT` (2^25): an upper bound so that the product of two
/// integers with absolute value at most `CRIT_INT` is contained in the
/// mantissa of a double with some space left for addition.
pub const CRIT_INT: i32 = 33_554_432;

/// Java `Limits.CRIT_DOUBLE` (2^53): the biggest double value so that all
/// integers smaller than this value are represented exactly as double values.
pub const CRIT_DOUBLE: f64 = 9_007_199_254_740_992.0;

/// Java `Limits.CRIT_INT_BIG`: `CRIT_INT` as an arbitrary-precision value.
pub static CRIT_INT_BIG: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(CRIT_INT));

/// Java `Limits.sqrt2`.
pub const SQRT2: f64 = core::f64::consts::SQRT_2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crit_int_is_two_to_the_twenty_fifth() {
        assert_eq!(CRIT_INT, 33_554_432);
        assert_eq!(CRIT_INT, 1 << 25);
    }

    #[test]
    fn crit_double_is_two_to_the_fifty_third() {
        assert_eq!(CRIT_DOUBLE, 9_007_199_254_740_992.0);
        assert_eq!(CRIT_DOUBLE, 2f64.powi(53));
    }

    #[test]
    fn crit_int_big_mirrors_crit_int() {
        assert_eq!(&*CRIT_INT_BIG, &BigInt::from(33_554_432));
    }

    #[test]
    fn sqrt2_is_the_double_square_root_of_two() {
        // Math.sqrt(2) is the correctly-rounded double, as is this constant.
        assert_eq!(SQRT2, 2.0f64.sqrt());
        assert_ne!(SQRT2, 0.0f64);
    }
}
