//! Exact 45-degree integer geometry kernel: Point/Vector families
//! (IntPoint, RationalPoint, FloatPoint, IntVector, RationalVector),
//! IntBox, IntOctagon; later TileShape, Polyline.
//!
//! Port discipline (design doc §9, decision D1): bit-parity port of Java
//! `geometry/planar` — i32 with Java wraparound (`wrapping_*`), double-for-
//! double predicates, `num-bigint` fallback; i128 widening deferred to M5
//! behind the differential corpus. No `mul_add`; operation order identical to
//! the Java original.
//!
//! Task 2 (point + vector families) ported every public Java method
//! except those whose parameter or return types belong to later M1a
//! tasks; the deferrals are documented on each module. The Task 4
//! deferrals (`surroundingBox` / `boundingBox` / `isContainedIn`) landed
//! with IntBox, the Task 5 deferrals (`surroundingOctagon` /
//! `boundingOctagon`) with IntOctagon.
//! - `sideOf(Line)` / `perpendicularProjection` /
//!   `perpendicularDirection` / `projectionApprox` -> Task 6 (Line /
//!   FloatLine)

pub mod big_int_aux;
pub mod big_int_direction;
pub mod direction;
pub mod float_point;
pub mod fortyfive_degree_direction;
pub mod int_box;
pub mod int_direction;
pub mod int_octagon;
pub mod int_point;
pub mod int_vector;
pub mod java_random;
pub mod limits;
pub mod point;
pub mod rational_point;
pub mod rational_vector;
pub mod rounding;
pub mod side;
pub mod vector;

#[cfg(test)]
mod tests {
    /// The crate name matches the package metadata.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-geometry");
    }
}
