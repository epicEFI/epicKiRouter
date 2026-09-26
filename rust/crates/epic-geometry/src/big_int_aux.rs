//! Port of Java `app.freerouting.datastructures.BigIntAux`.
//!
//! Also carries three JDK `BigInteger` shims (`big_integer_hash_code`,
//! `big_integer_int_value`, `big_integer_double_value`) that have no Java
//! `BigIntAux` counterpart but are needed for bit-parity of the point/vector
//! factories and hashes, plus `big_abs`/`big_gcd` stand-ins for the
//! `num-traits`/`num-integer` trait methods (which are deliberately not
//! direct dependencies of this crate).

use num_bigint::{BigInt, Sign};

/// Calculates the determinant of the vectors (x1, y1) and (x2, y2):
/// `x1*y2 - x2*y1`, in Java's operand order (`tmp1 = x1*y2; tmp2 = x2*y1;
/// return tmp1.subtract(tmp2)`).
pub fn determinant(x1: &BigInt, y1: &BigInt, x2: &BigInt, y2: &BigInt) -> BigInt {
    let tmp1 = x1 * y2;
    let tmp2 = x2 * y1;
    tmp1 - tmp2
}

/// Java `BigIntAux.addRationalCoordinates`: adds two rational coordinates
/// carried as `[numerator_1, numerator_2, denominator]` triples. Equal
/// denominators are kept; otherwise the product of the denominators is used
/// ("taking the least common multiple would be optimal", per the Java
/// comment). The result is never gcd-reduced.
pub fn add_rational_coordinates(first: &[BigInt; 3], second: &[BigInt; 3]) -> [BigInt; 3] {
    if first[2] == second[2] {
        // both rational numbers have the same denominator
        [
            &first[0] + &second[0],
            &first[1] + &second[1],
            first[2].clone(),
        ]
    } else {
        // multiply both denominators for the new denominator
        [
            &first[0] * &second[2] + &second[0] * &first[2],
            &first[1] * &second[2] + &second[1] * &first[2],
            &first[2] * &second[2],
        ]
    }
}

/// Java `BigIntAux.binaryGcd`: GCD of `a` and `b` interpreted as unsigned
/// 32-bit integers.
///
/// Semantically identical to the JDK algorithm (which Freerouting copied from
/// private `java.math`): the byte-wise strip + `trailingZeroTable` lookup is a
/// trailing-zero count, and Java's unsigned-compare trick
/// `(a + 0x80000000) > (b + 0x80000000)` is plain `a > b` on `u32`. The final
/// `a << t` cannot exceed `min(a, b)` of the inputs, so it fits in `u32`.
pub fn binary_gcd(mut a: u32, mut b: u32) -> u32 {
    if b == 0 {
        return a;
    }
    if a == 0 {
        return b;
    }

    let shift_a = a.trailing_zeros();
    a >>= shift_a;

    let shift_b = b.trailing_zeros();
    b >>= shift_b;

    let shift = shift_a.min(shift_b);

    while a != b {
        if a > b {
            a -= b;
            a >>= a.trailing_zeros();
        } else {
            b -= a;
            b >>= b.trailing_zeros();
        }
    }
    a << shift
}

/// Java `BigInteger.abs()` (always non-negative). Stand-in for the
/// `num_traits::Signed::abs` trait method, which would require a direct
/// `num-traits` dependency.
pub(crate) fn big_abs(v: &BigInt) -> BigInt {
    if v.sign() == Sign::Minus {
        -v
    } else {
        v.clone()
    }
}

/// Java `BigInteger.gcd(other)`: gcd of the absolute values, always
/// non-negative, with gcd(0, 0) = 0. Value-equal to the JDK's binary gcd
/// (the algorithm differs, the result does not). Stand-in for the
/// `num_integer::Integer::gcd` trait method.
pub(crate) fn big_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let mut a = big_abs(a);
    let mut b = big_abs(b);
    while b.sign() != Sign::NoSign {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}

/// JDK `BigInteger.hashCode()`: folds the big-endian 32-bit magnitude words
/// with `h = 31*h + word` (int wraparound) and multiplies the result by the
/// signum, so `hashCode(-x) == -hashCode(x)`. Bit-exact with the JDK, which
/// the `RationalPoint` hash pins rely on.
pub(crate) fn big_integer_hash_code(v: &BigInt) -> i32 {
    let mut h: i32 = 0;
    for word in v.magnitude().to_u32_digits().iter().rev() {
        h = h.wrapping_mul(31).wrapping_add(*word as i32);
    }
    let sign = match v.sign() {
        Sign::Plus => 1,
        Sign::NoSign => 0,
        Sign::Minus => -1,
    };
    h.wrapping_mul(sign)
}

/// Java `BigInteger.intValue()`: the low 32 bits of the value with
/// wraparound (sign * low magnitude word, modulo 2^32 — identical to the
/// JDK two's-complement truncation).
pub(crate) fn big_integer_int_value(v: &BigInt) -> i32 {
    // The zero-alloc LE digit iterator (slice C): `first()` answers the
    // same least-significant word `to_u32_digits().first()` answered
    // (both strip the zero words above the low word; both yield
    // nothing for zero) — identical truncation, no temp `Vec`.
    let low = match v.magnitude().iter_u32_digits().next() {
        Some(word) => word as i32,
        None => 0,
    };
    match v.sign() {
        Sign::Plus => low,
        Sign::Minus => low.wrapping_neg(),
        Sign::NoSign => 0,
    }
}

/// Java `BigInteger.doubleValue()`: the closest double (correctly rounded);
/// magnitudes beyond the double range saturate to ±infinity.
/// `str::parse::<f64>` is correctly rounded exactly like the JDK conversion,
/// so this is bit-exact. (The `num-traits::ToPrimitive` route would need a
/// new direct dependency, which the port rules forbid.)
pub(crate) fn big_integer_double_value(v: &BigInt) -> f64 {
    v.to_string()
        .parse::<f64>()
        .expect("decimal BigInt string always parses as f64")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    #[test]
    fn determinant_is_x1y2_minus_x2y1() {
        // 2*5 - 4*3
        assert_eq!(determinant(&b(2), &b(3), &b(4), &b(5)), b(-2));
        // Vectors (x1,y1)=(4,3), (x2,y2)=(5,2): 4*2 - 5*3 = -7.
        assert_eq!(determinant(&b(4), &b(3), &b(5), &b(2)), b(-7));
        // Exact beyond i64: x1*y2 - x2*y1 = big*big - 2 with big = 10^30.
        let big = BigInt::from(10u32).pow(30);
        assert_eq!(determinant(&big, &b(2), &b(1), &big), &big * &big - b(2));
    }

    #[test]
    fn add_rational_coordinates_keeps_equal_denominator() {
        let first = [b(1), b(2), b(5)];
        let second = [b(2), b(3), b(5)];
        assert_eq!(
            add_rational_coordinates(&first, &second),
            [b(3), b(5), b(5)]
        );
    }

    #[test]
    fn add_rational_coordinates_multiplies_denominators_without_reduction() {
        // (1/2) + (1/3) = (5/6), stored as numerator 5 over denominator 6.
        let first = [b(1), b(1), b(2)];
        let second = [b(1), b(1), b(3)];
        assert_eq!(
            add_rational_coordinates(&first, &second),
            [b(5), b(5), b(6)]
        );

        // (1/2) + (1/4) = (6/8), NOT the reduced (3/4).
        let first = [b(1), b(0), b(2)];
        let second = [b(1), b(1), b(4)];
        assert_eq!(
            add_rational_coordinates(&first, &second),
            [b(6), b(2), b(8)]
        );
    }

    #[test]
    fn binary_gcd_matches_mathematical_gcd_on_positives() {
        assert_eq!(binary_gcd(48, 18), 6);
        assert_eq!(binary_gcd(18, 48), 6);
        assert_eq!(binary_gcd(680, 612), 68);
        assert_eq!(binary_gcd(1_000_000, 999_999), 1);
        assert_eq!(binary_gcd(1, 1), 1);
        assert_eq!(binary_gcd(0, 7), 7);
        assert_eq!(binary_gcd(7, 0), 7);
    }

    #[test]
    fn binary_gcd_interprets_inputs_as_unsigned() {
        // 0x80000000 and 0xC0000000 are negative as signed ints; as unsigned
        // their gcd is 0x40000000.
        assert_eq!(binary_gcd(0x8000_0000, 0xC000_0000), 0x4000_0000);
        assert_eq!(binary_gcd(0x8000_0000, 0x8000_0000), 0x8000_0000);
        assert_eq!(binary_gcd(0xFFFF_FFFF, 0x0000_FFFF), 0x0000_FFFF);
    }

    #[test]
    fn big_abs_is_non_negative() {
        assert_eq!(big_abs(&b(-7)), b(7));
        assert_eq!(big_abs(&b(7)), b(7));
        assert_eq!(big_abs(&b(0)), b(0));
    }

    #[test]
    fn big_gcd_matches_java_gcd_contract() {
        // Always non-negative, gcd of absolute values, gcd(0, 0) = 0.
        assert_eq!(big_gcd(&b(-4), &b(6)), b(2));
        assert_eq!(big_gcd(&b(0), &b(0)), b(0));
        assert_eq!(big_gcd(&b(0), &b(7)), b(7));
        // Beyond u64: gcd(2^70, 2^40) = 2^40.
        let big = BigInt::from(2u32).pow(70);
        let small = BigInt::from(2u32).pow(40);
        assert_eq!(big_gcd(&big, &small), small);
    }

    /// Pinned against JDK 25 jshell: `new BigInteger("2").hashCode() == 2`,
    /// `new BigInteger("-2").hashCode() == -2`, and multi-word values fold
    /// `31*h + word` over big-endian magnitude words times the sign.
    #[test]
    fn big_integer_hash_code_matches_jdk() {
        assert_eq!(big_integer_hash_code(&b(0)), 0);
        assert_eq!(big_integer_hash_code(&b(2)), 2);
        assert_eq!(big_integer_hash_code(&b(-2)), -2);
        // 2^40 + 5: words big-endian [2^8, 5] -> h = 31*256 + 5 = 7941.
        let v = (BigInt::from(1u32) << 40u32) + b(5);
        assert_eq!(big_integer_hash_code(&v), 31 * 256 + 5);
        assert_eq!(big_integer_hash_code(&-v), -(31 * 256 + 5));
    }

    /// Java `BigInteger.intValue()` truncates to the low 32 bits of the
    /// two's-complement value with wraparound. -(2^32+1) mod 2^32 is
    /// 0xFFFFFFFF = -1, and the low word of 2^32 is 0.
    #[test]
    fn big_integer_int_value_truncates_to_low_32_bits() {
        assert_eq!(big_integer_int_value(&b(0)), 0);
        assert_eq!(big_integer_int_value(&b(33_554_432)), 33_554_432);
        assert_eq!(big_integer_int_value(&b(-2)), -2);
        // -(2^32 + 1): low 32 bits of two's complement are 0xFFFFFFFF.
        let v = -(BigInt::from(1u32) << 32u32) - b(1);
        assert_eq!(big_integer_int_value(&v), -1);
        // 2^32: low word is 0.
        let v = BigInt::from(1u32) << 32u32;
        assert_eq!(big_integer_int_value(&v), 0);
    }

    #[test]
    fn big_integer_double_value_is_correctly_rounded_and_saturates() {
        assert_eq!(big_integer_double_value(&b(0)), 0.0);
        assert_eq!(big_integer_double_value(&b(-3)), -3.0);
        assert_eq!(
            big_integer_double_value(&b(1_000_000_000_000_000_000)),
            1e18
        );
        // Beyond the double range: saturates to infinity like the JDK.
        let huge = BigInt::from(10u32).pow(400);
        assert_eq!(big_integer_double_value(&huge), f64::INFINITY);
        assert_eq!(big_integer_double_value(&-huge), f64::NEG_INFINITY);
    }
}
