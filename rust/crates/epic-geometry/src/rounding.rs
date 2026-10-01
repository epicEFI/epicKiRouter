//! Java `Math.round` / `Math.rint` semantics for `f64`.

/// Java 7+ `Math.round(double)` (JDK-6430675): closest integer with ties
/// toward +infinity, computed without the double-rounding artifact of the
/// naive `floor(d + 0.5)` formula (which mis-rounds values 1 ulp below a
/// half, e.g. 0.49999999999999994). `d - d.floor()` is exact for finite
/// doubles (fractional part of a double is always representable), so the
/// comparison `frac >= 0.5` is the exact tie test. NaN casts to 0 and
/// out-of-range values (including ±infinity) saturate, matching Java's
/// `(long)` cast.
pub fn java_round(d: f64) -> i64 {
    let floor = d.floor();
    let frac = d - floor;
    if frac >= 0.5 {
        (floor + 1.0) as i64
    } else {
        floor as i64
    }
}

/// Java `Math.rint`: nearest integral value, halves to even.
pub fn java_rint(d: f64) -> f64 {
    d.round_ties_even()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_round_matches_floor_plus_half_semantics() {
        assert_eq!(java_round(2.5), 3); // toward +inf
        assert_eq!(java_round(-2.5), -2); // NOT Rust round()'s -3
        assert_eq!(java_round(-2.6), -3);
        assert_eq!(java_round(2.4), 2);
    }

    /// JDK-6430675: the only double for which the naive `floor(d + 0.5)`
    /// diverges from JDK 7+ `Math.round` — the addition rounds up to exactly
    /// 1.0, while the exact fractional part is below the half. Pinned against
    /// JDK 25 jshell: `Math.round(0.49999999999999994) == 0` while
    /// `(long)Math.floor(0.49999999999999994 + 0.5) == 1`.
    #[test]
    fn java_round_rounds_down_one_ulp_below_half() {
        assert_eq!(java_round(0.49999999999999994), 0);
    }

    /// Further JDK 25 jshell pins. No scaled divergence exists: for every
    /// half-integer h >= 1.5 the exact sum `h - 1 ulp + 0.5` is the largest
    /// double below the next integer, hence representable, so the naive
    /// formula agrees with `Math.round` away from 0.49999999999999994. These
    /// cases pin that agreement (and the tie/saturation/NaN edges).
    #[test]
    fn java_round_jshell_pinned_scaled_and_edge_cases() {
        assert_eq!(java_round(2.4999999999999996), 2); // 2.5 - 1 ulp; naive also 2
        // Exact tie at the 2^23 scale (the suggested literal
        // 8388608.499999999999999 parses to this same double): rounds up.
        assert_eq!(java_round(8_388_608.5), 8_388_609);
        // Fractional part 0.9999999990686774: rounds up in both formulas.
        assert_eq!(java_round(8388607.999999999), 8388608);
        assert_eq!(java_round(-0.5), 0);
        // Saturation and NaN match Java's `(long)` cast semantics
        // (jshell: Math.round(-Inf) == Math.round(-Double.MAX_VALUE) ==
        // Long.MIN_VALUE).
        assert_eq!(java_round(9.223372036854776e18), i64::MAX);
        assert_eq!(java_round(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(java_round(f64::MIN), i64::MIN);
        assert_eq!(java_round(f64::NAN), 0);
    }

    #[test]
    fn java_rint_rounds_halves_to_even() {
        assert_eq!(java_rint(2.5), 2.0);
        assert_eq!(java_rint(3.5), 4.0);
        assert_eq!(java_rint(-2.5), -2.0);
    }
}
