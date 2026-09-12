//! Port of Java `app.freerouting.datastructures.BigIntAux`.

use num_bigint::BigInt;

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
}
