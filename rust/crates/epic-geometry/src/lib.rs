//! Exact 45-degree integer geometry kernel: Point/Vector families
//! (IntPoint, RationalPoint, FloatPoint, IntVector, RationalVector),
//! IntBox, IntOctagon, Line/LineSegment, TileShape/Simplex, Polygon/
//! PolygonShape/Polyline.
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
//! `boundingOctagon`) with IntOctagon, and the Task 6 deferrals
//! (`sideOf(Line)` / `perpendicularProjection` /
//! `perpendicularDirection` / `projectionApprox`) with the Line family.
//!
//! M1a complete: bit-parity port of all 34 `geometry.planar` classes
//! (incl. TileShape, Polyline), green against the committed 5000-case
//! differential corpus (`cargo run -p epic-harness -- corpus compare`).
//!
//! The port is transliteration-complete, not consumer-complete: a handful
//! of pub items (e.g. `RegularTileShape::as_int_box`, `Simplex::
//! remove_border_line`) have no callers yet — they are ported Java API
//! surface awaiting M2+ consumers (board/searchtree), not dead code.

pub mod big_int_aux;
pub mod big_int_direction;
pub mod bounding;
pub mod circle;
pub mod delaunay;
pub mod direction;
pub mod ellipse;
pub mod float_line;
pub mod float_point;
pub mod fortyfive_degree_direction;
pub mod int_box;
pub mod int_direction;
pub mod int_octagon;
pub mod int_point;
pub mod int_vector;
pub mod java_random;
pub mod limits;
pub mod line;
pub mod line_segment;
pub mod point;
pub mod polygon;
pub mod polygon_shape;
pub mod polyline;
pub mod polyline_area;
pub mod polyline_shape;
pub mod rational_point;
pub mod rational_vector;
pub mod regular_tile_shape;
pub mod rounding;
pub mod shape;
pub mod side;
pub mod simplex;
pub mod tile_shape;
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
