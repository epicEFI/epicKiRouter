//! Port of Java `app.freerouting.geometry.planar.FloatLine`.
//!
//! Defines a line in the plane by two FloatPoints. Calculations with
//! FloatLines are generally not exact; collinear is not defined for
//! FloatLines. Every operation is a double-for-double transliteration in
//! the Java operation order — including the 0.01 tolerance literals of
//! `segmentDistance` / `nearestSegmentPoint` and the `Limits.CRIT_INT`
//! bounds of the projection functions.
//!
//! `intersection` returns `None` where Java returns null (parallel
//! lines); `segmentProjection` / `segmentProjection2` likewise.
//!
//! Also lands the Task 6 deferral from `point.rs` /
//! `float_point.rs`: `FloatPoint.projectionApprox(Line)` (see
//! [`FloatPoint::projection_approx`]).

use crate::float_point::FloatPoint;
use crate::limits::CRIT_INT;
use crate::line::Line;

/// Defines a line in the plane by two FloatPoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatLine {
    /// The first point of the directed line (Java public final field).
    pub a: FloatPoint,
    /// The second point of the directed line (Java public final field).
    pub b: FloatPoint,
}

impl FloatLine {
    /// Creates a line from two FloatPoints. (Java logs a debug message
    /// for null endpoints; Rust has no nulls.)
    pub fn new(a: FloatPoint, b: FloatPoint) -> FloatLine {
        FloatLine { a, b }
    }

    /// Returns the FloatLine with swapped end points.
    pub fn opposite(&self) -> FloatLine {
        FloatLine::new(self.b, self.a)
    }

    /// Adjusts this line's direction to match the orientation of another
    /// line.
    pub fn adjust_direction(&self, other: &FloatLine) -> FloatLine {
        if self.b.side_of(&self.a, &other.a) == other.b.side_of(&self.a, &other.a) {
            *self
        } else {
            self.opposite()
        }
    }

    /// Calculates the intersection of this line with other. Returns None
    /// if the lines are parallel (Java null).
    pub fn intersection(&self, other: &FloatLine) -> Option<FloatPoint> {
        let d1x = self.b.x - self.a.x;
        let d1y = self.b.y - self.a.y;
        let d2x = other.b.x - other.a.x;
        let d2y = other.b.y - other.a.y;
        let det1 = self.a.x * self.b.y - self.a.y * self.b.x;
        let det2 = other.a.x * other.b.y - other.a.y * other.b.x;
        let det = d2x * d1y - d2y * d1x;
        if det == 0.0 {
            return None;
        }
        Some(FloatPoint::new(
            (d2x * det1 - d1x * det2) / det,
            (d2y * det1 - d1y * det2) / det,
        ))
    }

    /// Translates the line perpendicular at about dist. If dist > 0, the
    /// line will be translated to the left, else to the right.
    pub fn translate(&self, dist: f64) -> FloatLine {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        let dxdx = dx * dx;
        let dydy = dy * dy;
        let length = (dxdx + dydy).sqrt();
        let new_a = if dxdx <= dydy {
            // translate along the x axis
            let rel_x = dist * length / dy;
            FloatPoint::new(self.a.x - rel_x, self.a.y)
        } else {
            // translate along the y axis
            let rel_y = dist * length / dx;
            FloatPoint::new(self.a.x, self.a.y + rel_y)
        };
        let new_b = FloatPoint::new(new_a.x + dx, new_a.y + dy);
        FloatLine::new(new_a, new_b)
    }

    /// Returns the signed distance of this line from point. Positive, if
    /// the line is on the left of the point, else negative.
    pub fn signed_distance(&self, point: &FloatPoint) -> f64 {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        let det = dy * (point.x - self.a.x) - dx * (point.y - self.a.y);
        // area of the parallelogramm spanned by the 3 points
        let length = (dx * dx + dy * dy).sqrt();
        det / length
    }

    /// Returns an approximation of the perpendicular projection of point
    /// onto this line.
    pub fn perpendicular_projection(&self, point: &FloatPoint) -> FloatPoint {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        if dx == 0.0 && dy == 0.0 {
            return self.a;
        }

        let dxdx = dx * dx;
        let dydy = dy * dy;
        let dxdy = dx * dy;
        let denominator = dxdx + dydy;
        let det = self.a.x * self.b.y - self.b.x * self.a.y;

        let x = (point.x * dxdx + point.y * dxdy + det * dy) / denominator;
        let y = (point.x * dxdy + point.y * dydy - det * dx) / denominator;

        FloatPoint::new(x, y)
    }

    /// Returns the distance of point to the nearest point of this line
    /// between this.a and this.b.
    pub fn segment_distance(&self, point: &FloatPoint) -> f64 {
        let projection = self.perpendicular_projection(point);
        if projection.is_contained_in_box(&self.a, &self.b, 0.01) {
            point.distance(&projection)
        } else {
            point.distance(&self.a).min(point.distance(&self.b))
        }
    }

    /// Returns the perpendicular projection of lineSegment onto this
    /// oriented line segment; None, if the projection is empty.
    pub fn segment_projection(&self, line_segment: &FloatLine) -> Option<FloatLine> {
        if self.b.scalar_product(&self.a, &line_segment.a) < 0.0 {
            return None;
        }
        if self.a.scalar_product(&self.b, &line_segment.b) < 0.0 {
            return None;
        }
        let projected_a = if self.a.scalar_product(&self.b, &line_segment.a) < 0.0 {
            self.a
        } else {
            let projected = self.perpendicular_projection(&line_segment.a);
            if projected.x.abs() >= CRIT_INT as f64 || projected.y.abs() >= CRIT_INT as f64 {
                return None;
            }
            projected
        };
        let projected_b = if self.b.scalar_product(&self.a, &line_segment.b) < 0.0 {
            self.b
        } else {
            self.perpendicular_projection(&line_segment.b)
        };
        if projected_b.x.abs() >= CRIT_INT as f64 || projected_b.y.abs() >= CRIT_INT as f64 {
            return None;
        }
        Some(FloatLine::new(projected_a, projected_b))
    }

    /// Returns the projection of lineSegment onto this oriented line
    /// segment by moving lineSegment perpendicular into the direction of
    /// this line segment; returns None if the projection is empty or
    /// lineSegment.a == lineSegment.b.
    pub fn segment_projection2(&self, line_segment: &FloatLine) -> Option<FloatLine> {
        if line_segment.a.scalar_product(&line_segment.b, &self.b) <= 0.0 {
            return None;
        }
        if line_segment.b.scalar_product(&line_segment.a, &self.a) <= 0.0 {
            return None;
        }
        let projected_a = if line_segment.a.scalar_product(&line_segment.b, &self.a) < 0.0 {
            let current_perpendicular_line = FloatLine::new(
                line_segment.a,
                line_segment.b.turn_90_degree_around(1, &line_segment.a),
            );
            match current_perpendicular_line.intersection(self) {
                Some(projected) if self.within_crit_int(&projected) => projected,
                _ => return None,
            }
        } else {
            self.a
        };

        let projected_b = if line_segment.b.scalar_product(&line_segment.a, &self.b) < 0.0 {
            let current_perpendicular_line = FloatLine::new(
                line_segment.b,
                line_segment.a.turn_90_degree_around(1, &line_segment.b),
            );
            match current_perpendicular_line.intersection(self) {
                Some(projected) if self.within_crit_int(&projected) => projected,
                _ => return None,
            }
        } else {
            self.b
        };
        Some(FloatLine::new(projected_a, projected_b))
    }

    /// Java's `projectedA == null || Math.abs(projectedA.x) >=
    /// Limits.CRIT_INT || Math.abs(projectedA.y) >= Limits.CRIT_INT`
    /// check.
    fn within_crit_int(&self, point: &FloatPoint) -> bool {
        point.x.abs() < CRIT_INT as f64 && point.y.abs() < CRIT_INT as f64
    }

    /// Shrinks this line on both sides by value. The result will contain
    /// at least the midpoint of the line.
    pub fn shrink_segment(&self, offset: f64) -> FloatLine {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        if dx == 0.0 && dy == 0.0 {
            return *self;
        }
        let length = (dx * dx + dy * dy).sqrt();
        let effective_offset = offset.min(length / 2.0);
        let new_a = FloatPoint::new(
            self.a.x + dx * effective_offset / length,
            self.a.y + dy * effective_offset / length,
        );
        let new_length = length - effective_offset;
        let new_b = FloatPoint::new(
            self.a.x + dx * new_length / length,
            self.a.y + dy * new_length / length,
        );
        FloatLine::new(new_a, new_b)
    }

    /// Calculates the nearest point on this line to fromPoint between
    /// this.a and this.b.
    pub fn nearest_segment_point(&self, from_point: &FloatPoint) -> FloatPoint {
        let projection = self.perpendicular_projection(from_point);
        if projection.is_contained_in_box(&self.a, &self.b, 0.01) {
            return projection;
        }
        // Now the projection is outside the line segment.
        if from_point.distance_square(&self.a) <= from_point.distance_square(&self.b) {
            self.a
        } else {
            self.b
        }
    }

    /// Divides this line segment into count line segments of nearly equal
    /// length (Java `divideSegmentIntoSections`; at most maxSectionLength
    /// per the javadoc).
    ///
    /// ORPHANED-DELEGATE DISCLOSURE (spec round NIT-5, the T4 option-(a)
    /// precedent): this owned Java-parity form has NO production caller
    /// since slice C — the production path is the appending core
    /// [`Self::divide_segment_into_sections_into`] via
    /// `ExpansionDoor::get_section_segments_into`. The owned form is
    /// kept deliberately as the parity-reading surface of the Java
    /// method and as the reuse pin's comparison oracle; wrapper pins
    /// over it are identity tautologies (cerebrum 11) and are not
    /// added.
    pub fn divide_segment_into_sections(&self, count: i32) -> Vec<FloatLine> {
        let mut result = Vec::new();
        self.divide_segment_into_sections_into(count, &mut result);
        result
    }

    /// The appending core of [`Self::divide_segment_into_sections`]
    /// (slice C): appends the identical sections in the identical
    /// order to `out` (cleared first), so the caller's buffer is
    /// reused across calls.
    pub fn divide_segment_into_sections_into(&self, count: i32, out: &mut Vec<FloatLine>) {
        out.clear();
        if count == 0 {
            return;
        }
        if count == 1 {
            out.push(*self);
            return;
        }
        let line_length = self.b.distance(&self.a);
        out.reserve(count as usize);
        let section_length = line_length / count as f64;
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        let mut current_a = self.a;
        for i in 0..count {
            let current_b = if i == count - 1 {
                self.b
            } else {
                let current_distance = (i + 1) as f64 * section_length;
                FloatPoint::new(
                    self.a.x + dx * current_distance / line_length,
                    self.a.y + dy * current_distance / line_length,
                )
            };
            out.push(FloatLine::new(current_a, current_b));
            current_a = current_b;
        }
    }
}

/// Java `FloatPoint.projectionApprox(Line)`: an approximation of the
/// perpendicular projection of this point onto line. Defined here (Task
/// 6) because it needs the [`FloatLine`] and [`Line`] types.
impl FloatPoint {
    pub fn projection_approx(&self, line: &Line) -> FloatPoint {
        let float_line = FloatLine::new(line.a.to_float(), line.b.to_float());
        float_line.perpendicular_projection(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::Line;

    fn fp(x: f64, y: f64) -> FloatPoint {
        FloatPoint::new(x, y)
    }

    fn fl(ax: f64, ay: f64, bx: f64, by: f64) -> FloatLine {
        FloatLine::new(fp(ax, ay), fp(bx, by))
    }

    /// jshell pins `FL1`-`FL3`: intersection of f1 = (0,0)-(4,2) with
    /// f2 = (0,3)-(4,0) is (0x1.3333333333333p1, 0x1.3333333333333p0) =
    /// (2.4, 1.2); a parallel line yields null -> None; the
    /// perpendicular projection of (2, 3) onto the x axis segment is
    /// (2, 0).
    #[test]
    fn intersection_and_projection() {
        let f1 = fl(0.0, 0.0, 4.0, 2.0);
        let f2 = fl(0.0, 3.0, 4.0, 0.0);
        assert_eq!(f1.intersection(&f2), Some(fp(2.4, 1.2))); // FL1
        assert_eq!(f1.intersection(&fl(0.0, 0.0, 2.0, 1.0)), None); // FL2
        let fh = fl(0.0, 0.0, 4.0, 0.0);
        assert_eq!(fh.perpendicular_projection(&fp(2.0, 3.0)), fp(2.0, 0.0)); // FL3
        // Degenerate zero-length lines project to a.
        assert_eq!(
            fl(1.0, 1.0, 1.0, 1.0).perpendicular_projection(&fp(3.0, 3.0)),
            fp(1.0, 1.0)
        );
    }

    /// jshell pins `FL4`-`FL8`: signedDistance; the 0.01-tolerance box
    /// check of segmentDistance — (2, 0.005) projects (distance 0.005 =
    /// 0x1.47ae147ae147bp-8) while (4.02, 0) is rejected as outside the
    /// box and charged the endpoint distance 0.02 (bit pattern
    /// 0x1.47ae147ae14p-6, one ulp off the 0.02 literal); translate
    /// upwards; shrinkSegment.
    #[test]
    fn segment_distance_tolerance_boundary() {
        let fh = fl(0.0, 0.0, 4.0, 0.0);
        assert_eq!(fh.signed_distance(&fp(2.0, 3.0)), -3.0); // FL4 = -0x1.8p1
        // FL5: 0x1.47ae147ae147bp-8 — the canonical double 0.005.
        assert_eq!(fh.segment_distance(&fp(2.0, 0.005)), 0.005);
        // FL6: 0x1.47ae147ae14p-6 — NOT the 0.02 literal; the value is
        // pinned by raw bits (0x3F947AE147AE1400).
        assert_eq!(
            fh.segment_distance(&fp(4.02, 0.0)).to_bits(),
            0x3F94_7AE1_47AE_1400
        );
        // FL7: fh.translate(2.0) = (0,2)-(4,2).
        assert_eq!(fh.translate(2.0), fl(0.0, 2.0, 4.0, 2.0));
        // FL8: fh.shrinkSegment(1.0) = (1,0)-(3,0).
        assert_eq!(fh.shrink_segment(1.0), fl(1.0, 0.0, 3.0, 0.0));
        // Oversized shrink keeps at least the midpoint.
        assert_eq!(fh.shrink_segment(100.0), fl(2.0, 0.0, 2.0, 0.0));
    }

    /// jshell pins `FL9`-`FL12`: nearestSegmentPoint clamps to (4, 0) /
    /// (2, 0); segmentProjection of (3,1)-(7,3) is (3,0)-(4,0);
    /// segmentProjection2 of (3,1)-(7,2) is (0x1.ap1, 0)-(4,0) =
    /// (3.25, 0)-(4, 0); divideSegmentIntoSections(3) splits at 4/3 and
    /// 8/3.
    #[test]
    fn projections_onto_segments_and_sections() {
        let fh = fl(0.0, 0.0, 4.0, 0.0);
        // FL9: nearest points (4, 0) and (2, 0).
        assert_eq!(fh.nearest_segment_point(&fp(6.0, 3.0)), fp(4.0, 0.0));
        assert_eq!(fh.nearest_segment_point(&fp(2.0, 3.0)), fp(2.0, 0.0));
        // FL10: (3,0)-(4,0).
        assert_eq!(
            fh.segment_projection(&fl(3.0, 1.0, 7.0, 3.0)),
            Some(fl(3.0, 0.0, 4.0, 0.0))
        );
        // FL11: (3.25,0)-(4,0).
        assert_eq!(
            fh.segment_projection2(&fl(3.0, 1.0, 7.0, 2.0)),
            Some(fl(3.25, 0.0, 4.0, 0.0))
        );
        // An empty projection is None.
        assert_eq!(fh.segment_projection(&fl(-3.0, 1.0, -7.0, 3.0)), None);
        assert_eq!(fh.segment_projection2(&fl(-3.0, 1.0, -7.0, 2.0)), None);
        // FL12: sections at 0x1.5555555555555p0 / 0x1.5555555555555p1.
        let secs = fh.divide_segment_into_sections(3);
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0], fl(0.0, 0.0, 4.0 / 3.0, 0.0));
        assert_eq!(secs[1], fl(4.0 / 3.0, 0.0, 8.0 / 3.0, 0.0));
        assert_eq!(secs[2], fl(8.0 / 3.0, 0.0, 4.0, 0.0));
        // count 0 -> empty, count 1 -> the segment itself.
        assert!(fh.divide_segment_into_sections(0).is_empty());
        assert_eq!(fh.divide_segment_into_sections(1), vec![fh]);
    }

    /// jshell pins `FL13`/`FL14` and the adjustDirection pins: opposite
    /// swaps the endpoints; adjustDirection keeps the orientation of
    /// fb45 = (0,0)-(1,0) against the same-direction (2,1)-(5,1) and
    /// flips nothing for the mixed (2,1)-(-1,1) partner here (both
    /// endpoints of the partner are on the same side of fb45).
    #[test]
    fn opposite_and_adjust_direction() {
        let fb45 = fl(0.0, 0.0, 1.0, 0.0);
        let fb45b = fl(2.0, 1.0, 5.0, 1.0);
        assert_eq!(fb45.opposite(), fl(1.0, 0.0, 0.0, 0.0)); // FL14
        assert_eq!(fb45.adjust_direction(&fb45b), fb45); // FL13a
        // FL13b: the mixed partner (2,1)-(-1,1) puts this.b and other.b
        // on DIFFERENT sides of the a -> other.a ray, so Java returns
        // this.opposite() = (1,0)-(0,0).
        let adj = fb45.adjust_direction(&fl(2.0, 1.0, -1.0, 1.0));
        assert_eq!(adj, fb45.opposite()); // FL13b
        // A partner whose a lies on this line's own ray gives collinear
        // == collinear on both sides, so Java keeps *self (no flip even
        // though the partner runs opposite).
        assert_eq!(fb45.adjust_direction(&fb45.opposite()), fb45);
    }

    /// jshell pin landing the Task 6 deferral:
    /// FloatPoint(3, 5).projectionApprox(Line(0, 1, 2, 1)) — the FloatLine
    /// algorithm (correct sign, unlike the RationalPoint projection) maps
    /// the point to (3, 1).
    #[test]
    fn float_point_projection_approx() {
        let line = Line::from_int_coords(0, 1, 2, 1);
        assert_eq!(fp(3.0, 5.0).projection_approx(&line), fp(3.0, 1.0));
    }

    /// PIN (M5 slice C) — the appending core's reuse is invisible:
    /// repeated `_into` calls into ONE buffer reproduce the owned
    /// form's sections exactly across all three count arms (0, 1,
    /// many), and a shorter re-fill drops the previous call's tail
    /// (the clear-before-fill contract). The missing-`out.clear()`
    /// mutant survives the value faces (identical sections appended
    /// after the stale ones) but dies on the LENGTH face — which is
    /// why the length asserts are separate from the equality asserts.
    #[test]
    fn divide_segment_into_sections_reuse_is_invisible() {
        let seg = fl(0.0, 0.0, 10.0, 0.0);
        let mut buf: Vec<FloatLine> = Vec::new();
        // The many-section arm.
        seg.divide_segment_into_sections_into(3, &mut buf);
        assert_eq!(buf, seg.divide_segment_into_sections(3));
        assert_eq!(buf.len(), 3);
        // The single-section arm, refilling the NON-EMPTY buffer: the
        // stale two-section tail must be gone.
        seg.divide_segment_into_sections_into(1, &mut buf);
        assert_eq!(buf, seg.divide_segment_into_sections(1));
        assert_eq!(buf.len(), 1, "stale tail dropped on re-fill");
        // The zero arm: empty result, buffer emptied.
        seg.divide_segment_into_sections_into(0, &mut buf);
        assert!(buf.is_empty());
        // Cross-check the many-arm VALUES against the jshell capture
        // convention: 3 sections of a 10-long segment at 1/3 marks.
        seg.divide_segment_into_sections_into(3, &mut buf);
        assert_eq!(buf[0].b.x, 10.0 / 3.0);
        assert_eq!(buf[1].a.x, 10.0 / 3.0);
        assert_eq!(buf[1].b.x, 20.0 / 3.0);
        assert_eq!(buf[2].b.x, 10.0);
    }
}
