//! Exact 45-degree integer geometry kernel: IntPoint, IntBox, IntOctagon,
//! TileShape, Polyline.
//!
//! Port discipline (design doc §9, decision D1): bit-parity port of Java
//! `geometry/planar` — i32 with Java wraparound (`wrapping_*`), double-for-
//! double predicates, `num-bigint` fallback; i128 widening deferred to M5
//! behind the differential corpus. No `mul_add`; operation order identical to
//! the Java original.

pub mod big_int_aux;
pub mod java_random;
pub mod limits;
pub mod rounding;
pub mod side;

#[cfg(test)]
mod tests {
    /// The crate name matches the package metadata.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-geometry");
    }
}
