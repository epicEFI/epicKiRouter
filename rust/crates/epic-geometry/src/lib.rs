//! Exact 45-degree integer geometry kernel: IntPoint, IntBox, IntOctagon,
//! TileShape, Polyline.
//!
//! Port discipline (design doc §9): i128 fast path with checked arithmetic and
//! BigRational fallback; no `mul_add`; operation order identical to the Java
//! `geometry/planar` package it replaces.

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-geometry");
    }
}
