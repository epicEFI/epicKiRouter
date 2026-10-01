//! Trace geometry — the `PolylineTraceGeometry` surface (M2 Task 4).
//!
//! Java anchor: `board/trace/PolylineTraceGeometry.java` — a
//! stateless collaborator of `PolylineTrace` ("This collaborator
//! deliberately has no board or trace state", `:12-18`). The port
//! keeps exactly that shape: FREE FUNCTIONS over the item's
//! `(lines, half_width)` pair, no `&mut`, no board handle. The
//! polyline surgery surface (split/combine/normalize) is Tasks 11-13
//! and deliberately absent here.
//!
//! The exact Java accessor set (`:23-66`) is
//! `firstCorner`/`lastCorner`/`cornerCount`/`length`/`boundingBox`/
//! `tileShapeCount`/`connectionShape` — there is NO middle-corner
//! accessor, and the port adds none. The remaining `PolylineTraceGeometry`
//! members (`translate`/`turn90Degree`/`rotateApprox`/`mirrorVertical`,
//! `:47-61`) are thin delegates to the [`Polyline`] methods epic-geometry
//! already exposes 1:1; they are not re-wrapped here (the transform
//! dispatch would be a pure re-export).
//!
//! Evidence rule: the underlying epic-geometry operations
//! (`Polyline::corner`/`corner_count`/`length_approx_total`/
//! `bounding_box_total`/`offset_shape`, `LineSegment::to_simplex`,
//! `TileShape::simplify`) are pinned against the M1b geometry corpus
//! (committed goldens; `epic-harness corpus compare`). The tests here
//! pin the COMPOSITION — the half-width inflating the bounding box,
//! the `lines - 2` counts, the 3-line `LineSegment` windowing — with
//! self-enforcing arithmetic that includes forms where wrong and
//! right differ (pin failure mode 4: e.g. a bounding box computed
//! without the offset fails the `+half_width` assertion).

use epic_geometry::int_box::IntBox;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;

/// Java `PolylineTraceGeometry.firstCorner(lines)`
/// (`PolylineTraceGeometry.java:23-25` = `lines.corner(0)`).
#[must_use]
pub fn first_corner(lines: &Polyline) -> Option<Point> {
    lines.corner(0)
}

/// Java `PolylineTraceGeometry.lastCorner(lines)`
/// (`PolylineTraceGeometry.java:27-29` = `lines.corner(lines.length
/// - 2)`): the LINE-array index is `lines.len() - 2`, which is the
/// last CORNER index because `corner_count == lines.len() - 1`.
#[must_use]
pub fn last_corner(lines: &Polyline) -> Option<Point> {
    lines.corner(lines.corner_count() as i32 - 1)
}

/// Java `PolylineTraceGeometry.cornerCount(lines)`
/// (`PolylineTraceGeometry.java:31-33` = `lines.length - 1`).
#[must_use]
pub fn corner_count(lines: &Polyline) -> usize {
    lines.corner_count()
}

/// Java `PolylineTraceGeometry.length(lines)`
/// (`PolylineTraceGeometry.java:35-37` = `lines.lengthApprox()` —
/// the NO-ARG overload, i.e. the total over all segments).
#[must_use]
pub fn length(lines: &Polyline) -> f64 {
    lines.length_approx_total()
}

/// Java `PolylineTraceGeometry.boundingBox(lines, halfWidth)`
/// (`PolylineTraceGeometry.java:39-41`): the polyline's total
/// bounding box INFLATED by the trace half width on all four sides —
/// the clearance-corrected bounds a trace actually occupies.
#[must_use]
pub fn bounding_box(lines: &Polyline, half_width: i32) -> IntBox {
    lines.bounding_box_total().offset(f64::from(half_width))
}

/// Java `PolylineTraceGeometry.tileShapeCount(lines)`
/// (`PolylineTraceGeometry.java:43-45`): one tile shape per interior
/// line window (`max(lines - 2, 0)`; the two-segment single-corner
/// polyline has one, a plain segment has ZERO).
#[must_use]
pub fn tile_shape_count(lines: &Polyline) -> usize {
    lines.lines.len().saturating_sub(2)
}

/// Java `PolylineTraceGeometry.connectionShape(lines, index)`
/// (`PolylineTraceGeometry.java:63-66`): the convex connection shape
/// of the `index`-th line segment — the `LineSegment` over the THREE
/// consecutive lines `(lines[index], lines[index+1], lines[index+2])`,
/// simplified to its minimal tile shape. `index` runs `0..tile_shape_count`
/// (out-of-range yields `None`, where Java would array-bound-throw).
#[must_use]
pub fn connection_shape(lines: &Polyline, index: i32) -> Option<TileShape> {
    let count = tile_shape_count(lines);
    if usize::try_from(index).ok()? >= count {
        return None;
    }
    let start = index as usize;
    let segment = LineSegment::new(
        lines.lines[start].clone(),
        lines.lines[start + 1].clone(),
        lines.lines[start + 2].clone(),
    );
    Some(segment.to_simplex().simplify())
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_point::IntPoint;

    /// A 3-corner orthogonal polyline: (0,0) -> (10000,0) ->
    /// (10000,5000). The ctor stores FOUR lines (n corners -> n+1
    /// lines: the two perpendicular end cut-lines plus one line per
    /// segment), so `cornerCount = lines - 1 = 3` and
    /// `tileShapeCount = lines - 2 = 2`.
    fn sample_trace() -> Polyline {
        Polyline::from_points(&[
            Point::Int(IntPoint::new(0, 0)),
            Point::Int(IntPoint::new(10_000, 0)),
            Point::Int(IntPoint::new(10_000, 5_000)),
        ])
    }

    /// A single-segment polyline (3 lines: two perpendicular ends +
    /// the segment line).
    fn segment_trace() -> Polyline {
        Polyline::from_two_corners(
            &Point::Int(IntPoint::new(0, 0)),
            &Point::Int(IntPoint::new(7_000, 0)),
        )
    }

    /// The corner accessors (`:23-29`): first/last pick the ENDS, and
    /// last is NOT first (anchor-blind guard — an implementation
    /// returning `corner(0)` for both fails the `last` assertion).
    #[test]
    fn first_and_last_corner_are_the_two_ends() {
        let lines = sample_trace();
        assert_eq!(
            first_corner(&lines),
            Some(Point::Int(IntPoint::new(0, 0))),
            "firstCorner = corner(0)"
        );
        assert_eq!(
            last_corner(&lines),
            Some(Point::Int(IntPoint::new(10_000, 5_000))),
            "lastCorner = corner(lines.length - 2)"
        );
        assert_ne!(first_corner(&lines), last_corner(&lines));
        // A single-segment trace: 3 lines -> 2 corners, still distinct.
        let segment = segment_trace();
        assert_eq!(corner_count(&segment), 2, "3 lines - 1");
        assert_eq!(
            first_corner(&segment),
            Some(Point::Int(IntPoint::new(0, 0)))
        );
        assert_eq!(
            last_corner(&segment),
            Some(Point::Int(IntPoint::new(7_000, 0)))
        );
        assert_ne!(first_corner(&segment), last_corner(&segment));
    }

    /// `cornerCount` (`:31-33`): lines - 1 for every shape — 3 corners
    /// here (4 lines), 2 for a segment (3 lines), and the empty
    /// polyline keeps Java's `-1` as the port's documented wrapping
    /// `usize::MAX` (epic-geometry `corner_count` docs).
    #[test]
    fn corner_count_is_lines_minus_one() {
        assert_eq!(corner_count(&sample_trace()), 3);
        assert_eq!(corner_count(&segment_trace()), 2);
        assert_eq!(
            corner_count(&Polyline::from_points(&[])),
            usize::MAX,
            "Java's -1 as the wrapping usize"
        );
    }

    /// `length` (`:35-37`): the total approximation = the sum of the
    /// segment lengths (10_000 + 5_000 for the sample, EXACT in f64).
    #[test]
    fn length_is_the_total_over_all_segments() {
        assert_eq!(length(&sample_trace()), 15_000.0);
        // A diagonal segment keeps the Euclidean approximation
        // (3-4-5 triangle): NOT the Manhattan 7_000.
        let diagonal = Polyline::from_two_corners(
            &Point::Int(IntPoint::new(0, 0)),
            &Point::Int(IntPoint::new(3_000, 4_000)),
        );
        assert_eq!(length(&diagonal), 5_000.0);
        assert_ne!(length(&diagonal), 7_000.0, "Manhattan would be wrong");
    }

    /// `boundingBox` (`:39-41`): the raw polyline box `(0,0)-(10000,
    /// 5000)` inflated by half_width on ALL FOUR sides — the
    /// anchor-blind form asserts the exact inflated box, which a
    /// no-offset implementation (or an offset of the full width, or a
    /// one-sided offset) misses.
    #[test]
    fn bounding_box_inflates_by_half_width_on_all_sides() {
        let lines = sample_trace();
        assert_eq!(
            bounding_box(&lines, 0),
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(10_000, 5_000)),
            "half_width 0 is the raw total box"
        );
        assert_eq!(
            bounding_box(&lines, 125),
            IntBox::new(IntPoint::new(-125, -125), IntPoint::new(10_125, 5_125)),
            "125 on every side"
        );
        assert_ne!(
            bounding_box(&lines, 125),
            bounding_box(&lines, 0),
            "the offset must be observable"
        );
    }

    /// `tileShapeCount` (`:43-45`): lines - 2 — the count of INTERIOR
    /// lines (the real segments): 2 for the 3-corner sample (4
    /// lines), 1 for a plain segment (3 lines — its single interior
    /// line is the segment itself), saturating at 0.
    #[test]
    fn tile_shape_count_is_interior_windows() {
        assert_eq!(tile_shape_count(&sample_trace()), 2);
        assert_eq!(tile_shape_count(&segment_trace()), 1, "3 lines - 2");
        assert_eq!(tile_shape_count(&Polyline::from_points(&[])), 0);
    }

    /// `connectionShape` (`:63-66`): the convex tile of the 3-line
    /// window `(lines[index], lines[index+1], lines[index+2])`. For
    /// the orthogonal sample every window degenerates to the FLAT box
    /// of one segment (the middle line bounded by its neighbors):
    /// window 0 spans the first segment `(0,0)-(10000,0)`, window 1
    /// the second `(10000,0)-(10000,5000)`; out-of-range indices
    /// yield `None` (Java throws). A plain segment HAS one connection
    /// shape (its single interior line).
    #[test]
    fn connection_shape_windows_three_consecutive_lines() {
        let lines = sample_trace();
        let shape0 = connection_shape(&lines, 0).expect("window 0");
        let bounds0 = shape0.bounding_box();
        assert_eq!(bounds0.ll, IntPoint::new(0, 0));
        assert_eq!(bounds0.ur, IntPoint::new(10_000, 0), "the first segment");
        let shape1 = connection_shape(&lines, 1).expect("window 1");
        let bounds1 = shape1.bounding_box();
        assert_eq!(bounds1.ll, IntPoint::new(10_000, 0));
        assert_eq!(
            bounds1.ur,
            IntPoint::new(10_000, 5_000),
            "the second segment"
        );
        // The two windows genuinely differ — anchor-blind form.
        assert_ne!(bounds0, bounds1);
        assert_eq!(connection_shape(&lines, 2), None, "index past count");
        assert_eq!(connection_shape(&lines, -1), None, "negative index");
        // A plain segment has exactly its one connection shape.
        let segment = segment_trace();
        let shape = connection_shape(&segment, 0).expect("the single window");
        assert_eq!(shape.bounding_box().ur, IntPoint::new(7_000, 0));
        assert_eq!(connection_shape(&segment, 1), None);
    }
}
