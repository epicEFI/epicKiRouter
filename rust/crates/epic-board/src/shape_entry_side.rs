//! The shove entry side: on which border line of an obstacle shape a
//! polyline enters, and where exactly it crosses (Java
//! `board/model/structure/ShapeEntrySide.java`, 161 lines, ported in
//! full). The shove machinery reads [`ShapeEntrySide::no`] as the
//! edge index of the entry border and
//! [`ShapeEntrySide::border_intersection`] as the crossing point —
//! the shove direction is derived from both.

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;

/// Java `ShapeEntrySide` — value struct; the four Java constructors
/// become the named constructors below. The four Java signatures are
/// pairwise distinct — the split into named constructors is purely
/// because Rust has no constructor overloading.
///
/// Java's `NOT_CALCULATED` singleton is [`ShapeEntrySide::NOT_CALCULATED`]
/// (`:16`); `borderIntersection` is a mutable field in Java (the ctor-C
/// fallback writes it mid-loop) but the writes are dead — the final
/// value is always overwritten at `:143` — so the port keeps the
/// struct immutable without observable divergence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeEntrySide {
    /// The number of the border line of the shape (Java `no`); -1 in
    /// the [`ShapeEntrySide::NOT_CALCULATED`] state.
    pub no: i32,
    /// The crossing point with that border line (Java
    /// `borderIntersection`); `None` where Java stores null.
    pub border_intersection: Option<FloatPoint>,
}

impl ShapeEntrySide {
    /// Java's `NOT_CALCULATED` singleton (`:16`): `no = -1`, no
    /// crossing point. Java shares ONE instance; the port hands out a
    /// value per call (the struct is a small `Copy`).
    pub const NOT_CALCULATED: ShapeEntrySide = ShapeEntrySide {
        no: -1,
        border_intersection: None,
    };

    /// Values already calculated — just create an instance (Java ctor
    /// `:157-160`).
    #[must_use]
    pub fn new_precomputed(no: i32, border_intersection: Option<FloatPoint>) -> Self {
        Self {
            no,
            border_intersection,
        }
    }

    /// Calculates the number of the edge line of `shape` where
    /// `polyline` enters (Java ctor `:26-62`). Used in the push-trace
    /// algorithm to determine the shove direction; `no` is expected
    /// between 1 and `polyline.lineCount - 2` inclusive.
    ///
    /// The walk starts at line `no` and moves DOWN; the first line
    /// segment with a border intersection wins. Fallback (`:41-59`,
    /// the first corner is inside the shape): the nearest intersection
    /// of `polyline.lines[1]` with any border line, STRICTLY closer
    /// each time (`<` — equals never replace the incumbent, so the
    /// FIRST minimal candidate wins ties).
    #[must_use]
    pub fn from_entry_no(polyline: &Polyline, no: i32, shape: &TileShape) -> Self {
        let mut fromside_no = -1;
        let mut intersection: Option<FloatPoint> = None;
        let mut border_intersection_found = false;
        // calculate the edgeIndex of shape, where polyline enters
        for current_no in (1..=no).rev() {
            let current_seg = LineSegment::from_polyline(polyline, current_no);
            let intersections = current_seg.border_intersections(shape);
            if !intersections.is_empty() {
                fromside_no = intersections[0];
                intersection = Some(
                    current_seg
                        .get_line()
                        .intersection_approx(&shape.border_line(fromside_no)),
                );
                border_intersection_found = true;
                break;
            }
        }
        if !border_intersection_found {
            // The first corner of polyline is inside shape.
            // Calculate the nearest intersection point of
            // polyline.lines[1] with the border of shape to the first
            // corner of polyline.
            let from_point = polyline.corner_approx(0);
            let check_line = &polyline.lines[1];
            let mut min_dist = f64::MAX;
            let edge_count = shape.border_line_count();
            for i in 0..edge_count {
                let current_line = shape.border_line(i as i32);
                let current_intersection = check_line.intersection_approx(&current_line);
                let current_distance = current_intersection.distance(&from_point).abs();
                if current_distance < min_dist {
                    fromside_no = i as i32;
                    intersection = Some(current_intersection);
                    min_dist = current_distance;
                }
            }
        }
        Self {
            no: fromside_no,
            border_intersection: intersection,
        }
    }

    /// Calculates the nearest border side of `shape` to `from_point`
    /// (Java ctor `:68-75`, taking a `Point`). Used in the
    /// shove-drill-item algorithm to determine the shove direction.
    /// Java warns (log-only, D12) when the projection lands on no
    /// border line.
    #[must_use]
    pub fn from_point(from_point: Point, shape: &TileShape) -> Self {
        let border_projection = shape
            .nearest_border_point(&from_point)
            .expect("nearest border point exists for a bounded shape");
        let no = shape.contains_on_border_line_no(&border_projection);
        // Java warns through FRLogger when `no < 0` ("CalcFromSide:
        // this.no >= 0 expected") — a log-only site (D12), silent here.
        let _ = no;
        Self {
            no,
            border_intersection: Some(border_projection.to_float()),
        }
    }

    /// Calculates the side of `shape` at the start of `line_segment`;
    /// if `shove_to_the_left` the entry index is decremented by 2,
    /// else it is increased by 2 (Java ctor `:81-154`).
    ///
    /// The main walk compares the SIGN of each shape corner against
    /// the segment line; the first sign flip whose crossing point is
    /// nearer to the segment START than to its END is the front side.
    /// The fallback (`:108-145`, reached when no corner-sign flip
    /// qualifies) projects the start point onto each border line and
    /// keeps the strictly nearest projection contained in the side's
    /// box (tolerance 0.01) — its mid-loop
    /// `borderIntersection = projection` write (`:128`) is DEAD, the
    /// middle-point write below always overwrites it; the port keeps
    /// only the live value.
    #[must_use]
    pub fn from_line_segment(
        line_segment: &LineSegment,
        shape: &TileShape,
        shove_to_the_left: bool,
    ) -> Self {
        let start_corner = line_segment.start_point_approx();
        let end_corner = line_segment.end_point_approx();
        let border_line_count = shape.border_line_count();
        let check_line = line_segment.get_line();
        let first_corner = shape
            .corner_approx(0)
            .expect("a shove entry shape has at least one corner");
        let mut prev_side = check_line.side_of_float_zero(&first_corner);
        let mut front_side_no: i32 = -1;

        for i in 1..=border_line_count {
            let next_corner = if i == border_line_count {
                first_corner
            } else {
                shape
                    .corner_approx(i as i32)
                    .expect("corner index within border count")
            };
            let next_side = check_line.side_of_float_zero(&next_corner);
            if prev_side != next_side {
                let current_intersection = shape
                    .border_line(i as i32 - 1)
                    .intersection_approx(check_line);
                if current_intersection.distance_square(&start_corner)
                    < current_intersection.distance_square(&end_corner)
                {
                    front_side_no = i as i32 - 1;
                    break;
                }
            }
            prev_side = next_side;
        }
        if front_side_no < 0 {
            // Fallback: find the nearest side of the shape to the
            // start point of the line segment.
            let mut min_distance = f64::MAX;
            let mut nearest_side: i32 = 0;

            // Check each side of the shape.
            for i in 0..border_line_count {
                let border_line = shape.border_line(i as i32);
                let float_border =
                    FloatLine::new(border_line.a.to_float(), border_line.b.to_float());
                let projection = float_border.perpendicular_projection(&start_corner);

                // Only consider if projection is on the line segment.
                let side_start = shape
                    .corner_approx(i as i32)
                    .expect("corner index within border count");
                let side_end = shape
                    .corner_approx(((i + 1) % border_line_count) as i32)
                    .expect("corner index within border count");
                if projection.is_contained_in_box(&side_start, &side_end, 0.01) {
                    let distance = start_corner.distance(&projection);
                    if distance < min_distance {
                        min_distance = distance;
                        nearest_side = i as i32;
                    }
                }
            }

            // Apply the same shove direction logic as the original
            // code.
            let no = if shove_to_the_left {
                (nearest_side + 2) % border_line_count as i32
            } else {
                (nearest_side + border_line_count as i32 - 2) % border_line_count as i32
            };
            // Update border intersection to be the middle of the
            // chosen side (this is ALSO the fallback's final
            // borderIntersection — the mid-loop projection write is
            // dead in Java).
            let prev_corner = shape
                .corner_approx(no)
                .expect("entry no within border count");
            let next_corner = shape
                .corner_approx((no + 1) % border_line_count as i32)
                .expect("entry no + 1 within border count");
            return Self {
                no,
                border_intersection: Some(prev_corner.middle_point(&next_corner)),
            };
        }
        let no = if shove_to_the_left {
            (front_side_no + 2) % border_line_count as i32
        } else {
            (front_side_no + border_line_count as i32 - 2) % border_line_count as i32
        };
        let prev_corner = shape
            .corner_approx(no)
            .expect("entry no within border count");
        let next_corner = shape
            .corner_approx((no + 1) % border_line_count as i32)
            .expect("entry no + 1 within border count");
        Self {
            no,
            border_intersection: Some(prev_corner.middle_point(&next_corner)),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Literal captures from the jar (jshell against
    //! `build/libs/freerouting-current-executable.jar`, JDK 25; capture
    //! log `logs/M3-T10a/captures/shape_entry_side_caps.txt`). Every
    //! case pins the (no, borderIntersection) pair through the
    //! production named constructors.

    use super::*;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_octagon::IntOctagon;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    fn unit_box() -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(0, 0),
            IntPoint::new(1000, 1000),
        )))
    }

    fn assert_side(side: &ShapeEntrySide, want_no: i32, want_x: f64, want_y: f64, label: &str) {
        assert_eq!(side.no, want_no, "{label} no");
        let bi = side
            .border_intersection
            .expect("{label} borderIntersection");
        assert!(
            (bi.x - want_x).abs() < 1e-6 && (bi.y - want_y).abs() < 1e-6,
            "{label} bi ({},{}) vs ({want_x},{want_y})",
            bi.x,
            bi.y
        );
    }

    /// ctor A (`:26-62`), captured A1/A1b/A2: walk hits on segment 2 /
    /// segment 1, and the first-corner-inside fallback (nearest border
    /// intersection of lines[1], strictly-closer tie rule).
    #[test]
    fn ctor_a_walk_and_fallback_match_the_jar() {
        let box_shape = unit_box();
        let pa = Polyline::from_points(&[p(-500, 500), p(500, 500), p(500, 1500)]);
        let a1 = ShapeEntrySide::from_entry_no(&pa, 2, &box_shape);
        assert_side(&a1, 2, 500.0, 1000.0, "A1-no2");
        let a1b = ShapeEntrySide::from_entry_no(&pa, 1, &box_shape);
        assert_side(&a1b, 3, 0.0, 500.0, "A1b-no1");
        let pa2 = Polyline::from_points(&[p(200, 600), p(600, 200), p(600, 600)]);
        let a2 = ShapeEntrySide::from_entry_no(&pa2, 2, &box_shape);
        assert_side(&a2, 3, 0.0, 800.0, "A2-fallback");
    }

    /// ctor B (`:68-75`), captured B1/B2/B3: interior point projects to
    /// the bottom border, exterior point to the right border, and the
    /// diagonal-exterior point lands on the corner (assigned to the top
    /// border by the jar's containsOnBorderLineNo).
    #[test]
    fn ctor_b_point_projection_matches_the_jar() {
        let box_shape = unit_box();
        let b1 = ShapeEntrySide::from_point(p(200, 100), &box_shape);
        assert_side(&b1, 0, 200.0, 0.0, "B1-inside");
        let b2 = ShapeEntrySide::from_point(p(1200, 500), &box_shape);
        assert_side(&b2, 1, 1000.0, 500.0, "B2-outside");
        let b3 = ShapeEntrySide::from_point(p(1100, 1100), &box_shape);
        assert_side(&b3, 2, 1000.0, 1000.0, "B3-corner");
    }

    /// ctor C (`:81-154`), captured C1/C2/C3. C1: the main walk fires
    /// on the 4-side box and the +2/-2 arithmetic coincides (both
    /// directions no=3). C2 on the 8-border octagon the two directions
    /// DIFFER (7 vs 3) — the discriminating case. C3: segment above the
    /// shape, no corner-sign flip qualifies, the nearest-projection
    /// fallback fires identically in both directions.
    #[test]
    fn ctor_c_segment_sides_and_fallback_match_the_jar() {
        let box_shape = unit_box();
        let pc1 = Polyline::from_points(&[p(1500, 300), p(-500, 300)]);
        let c1 = LineSegment::from_polyline(&pc1, 1);
        let c1_left = ShapeEntrySide::from_line_segment(&c1, &box_shape, true);
        assert_side(&c1_left, 3, 0.0, 500.0, "C1-box-left");
        let c1_right = ShapeEntrySide::from_line_segment(&c1, &box_shape, false);
        assert_side(&c1_right, 3, 0.0, 500.0, "C1-box-right");

        let oct = TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::new(
            100, 100, 900, 900, 200, 800, 200, 800,
        )));
        let pc2 = Polyline::from_points(&[p(1200, 500), p(-200, 500)]);
        let c2 = LineSegment::from_polyline(&pc2, 1);
        let c2_left = ShapeEntrySide::from_line_segment(&c2, &oct, true);
        assert_side(&c2_left, 7, 100.0, 100.0, "C2-oct-left");
        let c2_right = ShapeEntrySide::from_line_segment(&c2, &oct, false);
        assert_side(&c2_right, 3, 400.0, 400.0, "C2-oct-right");

        let pc3 = Polyline::from_points(&[p(100, 1200), p(900, 1200)]);
        let c3 = LineSegment::from_polyline(&pc3, 1);
        let c3_left = ShapeEntrySide::from_line_segment(&c3, &box_shape, true);
        assert_side(&c3_left, 0, 500.0, 0.0, "C3-fallback-left");
        let c3_right = ShapeEntrySide::from_line_segment(&c3, &box_shape, false);
        assert_side(&c3_right, 0, 500.0, 0.0, "C3-fallback-right");
    }
}
