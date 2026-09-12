//! Java `Math.round` / `Math.rint` semantics for `f64`.

/// Java `Math.round(d)`: floor(d + 0.5) — halves go toward +infinity.
///
/// Note: for arguments exactly one ulp below a half-integer the JDK's
/// `Math.round` implementation rounds down where this expression rounds up
/// (the classic `0.49999999999999994` class of cases). The spec formula is
/// used per plan decision D1; revisit behind the M5 differential corpus if it
/// ever matters.
pub fn java_round(d: f64) -> i64 {
    (d + 0.5).floor() as i64
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

    #[test]
    fn java_rint_rounds_halves_to_even() {
        assert_eq!(java_rint(2.5), 2.0);
        assert_eq!(java_rint(3.5), 4.0);
        assert_eq!(java_rint(-2.5), -2.0);
    }
}
