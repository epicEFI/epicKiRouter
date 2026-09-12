//! Port of Java `app.freerouting.geometry.planar.Circle` — a circle shape
//! in the plane with an [`IntPoint`] center and an `i32` radius.
//!
//! Bit-parity notes:
//! - All containment predicates mix the float `distanceSquare` with the
//!   int radius: `fp.distance_square(center) > (f64) radius * radius` —
//!   the multiplication is `f64 * f64` exactly as in Java.
//! - `bounding_octagon` uses `sqrt(2) - 1` corner displacements with
//!   Java `Math.ceil` / `Math.floor` and `(int)` narrowing casts.
//! - `intersects(Circle)` squares the radius SUM first
//!   (`radiusSumSquare = radius + other.radius; *= itself`) — order kept.
//! - Java implements `ConvexShape`; in this crate the `Shape` trait
//!   family is closed over the line-bounded `TileShape`s (its transform
//!   methods return `TileShape`), so `Circle` mirrors the Java surface
//!   with inherent methods instead. `ShapeRef::Circle` carries it in the
//!   cross-shape dispatch where Java passes `Shape` references.
//! - `toString` is ported as [`Display`] (Java `toString(Locale.ENGLISH)`;
//!   note there is deliberately NO space between center and radius part).

use crate::direction::Direction;
use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::int_vector::IntVector;
use crate::limits::SQRT2;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::rounding::java_round;
use crate::shape::ShapeBoundingDirections;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;
use std::fmt;

/// Java `Circle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Circle {
    /// The center (Java public final field).
    pub center: IntPoint,
    /// The radius (Java public final field; never negative).
    pub radius: i32,
}

impl Circle {
    /// Creates a new instance of Circle. A negative radius is negated
    /// (Java logs a warning).
    pub fn new(center: IntPoint, radius: i32) -> Circle {
        if radius < 0 {
            Circle {
                center,
                radius: -radius,
            }
        } else {
            Circle { center, radius }
        }
    }

    /// Java `isEmpty()`: always false.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Java `isBounded()`: always true.
    pub fn is_bounded(&self) -> bool {
        true
    }

    /// Java `dimension()`: 0 for a radius-0 point circle, else 2.
    pub fn dimension(&self) -> i32 {
        if self.radius == 0 {
            // circle is reduced to a point
            return 0;
        }
        2
    }

    /// Java `circumference()`.
    pub fn circumference(&self) -> f64 {
        2.0 * std::f64::consts::PI * self.radius as f64
    }

    /// Java `area()`.
    pub fn area(&self) -> f64 {
        (std::f64::consts::PI * self.radius as f64) * self.radius as f64
    }

    /// Java `centreOfGravity()`.
    pub fn centre_of_gravity(&self) -> FloatPoint {
        self.center.to_float()
    }

    /// Java `isOutside(Point)`.
    pub fn is_outside(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) > self.radius as f64 * self.radius as f64
    }

    /// Java `contains(Point)`.
    pub fn contains_point(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    /// Java `contains(FloatPoint)`.
    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        point.distance_square(&self.center.to_float()) <= self.radius as f64 * self.radius as f64
    }

    /// Java `containsInside(Point)`.
    pub fn contains_inside(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) < self.radius as f64 * self.radius as f64
    }

    /// Java `containsOnBorder(Point)`.
    pub fn contains_on_border(&self, point: &Point) -> bool {
        let fp = point.to_float();
        fp.distance_square(&self.center.to_float()) == self.radius as f64 * self.radius as f64
    }

    /// Java `distance(FloatPoint)`: max(distance to center - radius, 0).
    pub fn distance(&self, point: &FloatPoint) -> f64 {
        let d = point.distance(&self.center.to_float()) - self.radius as f64;
        d.max(0.0)
    }

    /// Java `smallestRadius()`.
    pub fn smallest_radius(&self) -> f64 {
        self.radius as f64
    }

    /// Java `boundingBox()`.
    pub fn bounding_box(&self) -> IntBox {
        let lower_left_x = self.center.x.wrapping_sub(self.radius);
        let upper_right_x = self.center.x.wrapping_add(self.radius);
        let lower_left_y = self.center.y.wrapping_sub(self.radius);
        let upper_right_y = self.center.y.wrapping_add(self.radius);
        IntBox::new(
            IntPoint::new(lower_left_x, lower_left_y),
            IntPoint::new(upper_right_x, upper_right_y),
        )
    }

    /// Java `boundingOctagon()`.
    pub fn bounding_octagon(&self) -> IntOctagon {
        let left_x = self.center.x.wrapping_sub(self.radius);
        let right_x = self.center.x.wrapping_add(self.radius);
        let bottom_y = self.center.y.wrapping_sub(self.radius);
        let top_y = self.center.y.wrapping_add(self.radius);

        let sqrt2_minus_1 = SQRT2 - 1.0;
        let ceil_corner_value = (sqrt2_minus_1 * self.radius as f64).ceil() as i32;
        let floor_corner_value = (sqrt2_minus_1 * self.radius as f64).floor() as i32;

        let upper_left_diagonal_x =
            left_x.wrapping_sub(self.center.y.wrapping_add(floor_corner_value));
        let lower_right_diagonal_x =
            right_x.wrapping_sub(self.center.y.wrapping_sub(ceil_corner_value));
        let lower_left_diagonal_x =
            left_x.wrapping_add(self.center.y.wrapping_sub(floor_corner_value));
        let upper_right_diagonal_x =
            right_x.wrapping_add(self.center.y.wrapping_add(ceil_corner_value));
        IntOctagon::new(
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        )
    }

    /// Java `boundingTile()`: the bounding octagon (the Java comment
    /// documents that the finer approximation caused problems with the
    /// spring_over algorithm and is dead).
    pub fn bounding_tile(&self) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(self.bounding_octagon()))
    }

    /// Creates a bounding tile shape around this circle, so that the length
    /// of the line segments of the tile is at most `max_segment_length`
    /// (Java `boundingTile(int)`).
    pub fn bounding_tile_with_max_segment(&self, max_segment_length: i32) -> TileShape {
        let quadrant_division_count = self.radius / max_segment_length + 1;
        if quadrant_division_count <= 2 {
            return self.bounding_tile();
        }
        let mut tangent_line_arr: Vec<Line> =
            Vec::with_capacity(quadrant_division_count as usize * 4);
        tangent_line_arr.resize(
            quadrant_division_count as usize * 4,
            Line::new(Point::Int(IntPoint::ZERO), Point::Int(IntPoint::ZERO)),
        );
        for i in 0..quadrant_division_count {
            // calculate the tangential points in the first quadrant
            let border_delta: crate::vector::Vector = if i == 0 {
                crate::vector::Vector::Int(IntVector::new(self.radius, 0))
            } else {
                let current_angle =
                    i as f64 * std::f64::consts::PI / (2.0 * quadrant_division_count as f64);
                let current_x = (current_angle.sin() * self.radius as f64).ceil() as i32;
                let current_y = (current_angle.cos() * self.radius as f64).ceil() as i32;
                crate::vector::Vector::Int(IntVector::new(current_x, current_y))
            };
            let center_point = Point::Int(self.center);
            let current_a = center_point.translate_by(&border_delta);
            let current_b = current_a.turn_90_degree(1, &center_point);
            let current_direction =
                Direction::get_instance(&current_b.difference_by(&center_point));
            let current_tangent = Line::get_instance(current_a, current_direction);
            let q = quadrant_division_count as usize;
            tangent_line_arr[q + i as usize] = current_tangent.clone();
            tangent_line_arr[2 * q + i as usize] = current_tangent.turn_90_degree(1, &self.center);
            tangent_line_arr[3 * q + i as usize] = current_tangent.turn_90_degree(2, &self.center);
            tangent_line_arr[i as usize] = current_tangent.turn_90_degree(3, &self.center);
        }
        TileShape::get_instance(&tangent_line_arr)
    }

    /// Java `isContainedIn(IntBox)`.
    pub fn is_contained_in(&self, r#box: &IntBox) -> bool {
        if r#box.ll.x > self.center.x.wrapping_sub(self.radius) {
            return false;
        }
        if r#box.ll.y > self.center.y.wrapping_sub(self.radius) {
            return false;
        }
        if r#box.ur.x < self.center.x.wrapping_add(self.radius) {
            return false;
        }
        r#box.ur.y >= self.center.y.wrapping_add(self.radius)
    }

    /// Java `turn90Degree(int, IntPoint)`.
    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> Circle {
        let new_center = int_point_turn_90(self.center, factor, pole);
        Circle::new(new_center, self.radius)
    }

    /// Java `rotateApprox(double, FloatPoint)`.
    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> Circle {
        let new_center = self.center.to_float().rotate(angle, pole).round();
        Circle::new(new_center, self.radius)
    }

    /// Java `mirrorVertical(IntPoint)`.
    pub fn mirror_vertical(&self, pole: &IntPoint) -> Circle {
        let x = pole.x.wrapping_sub(self.center.x.wrapping_sub(pole.x));
        Circle::new(IntPoint::new(x, self.center.y), self.radius)
    }

    /// Java `mirrorHorizontal(IntPoint)`.
    pub fn mirror_horizontal(&self, pole: &IntPoint) -> Circle {
        let y = pole.y.wrapping_sub(self.center.y.wrapping_sub(pole.y));
        Circle::new(IntPoint::new(self.center.x, y), self.radius)
    }

    /// Java `maxWidth()`.
    pub fn max_width(&self) -> f64 {
        2.0 * self.radius as f64
    }

    /// Java `minWidth()`.
    pub fn min_width(&self) -> f64 {
        2.0 * self.radius as f64
    }

    /// Java `boundingShape(ShapeBoundingDirections)`.
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_circle(self)
    }

    /// Java `offset(double)`.
    pub fn offset(&self, offset: f64) -> Circle {
        let new_radius = self.radius as f64 + offset;
        Circle::new(self.center, java_round(new_radius) as i32)
    }

    /// Java `shrink(double)`: the radius never drops below 1.
    pub fn shrink(&self, offset: f64) -> Circle {
        let new_radius = self.radius as f64 - offset;
        Circle::new(self.center, (java_round(new_radius) as i32).max(1))
    }

    /// Java `translateBy(Vector)` — only implemented for `IntVector`s in
    /// Java (other vectors return `this` with a warning).
    pub fn translate_by(&self, vector: &Vector) -> Circle {
        if *vector == Vector::ZERO {
            return *self;
        }
        match vector {
            Vector::Int(v) => Circle::new(
                IntPoint::new(
                    self.center.x.wrapping_add(v.x),
                    self.center.y.wrapping_add(v.y),
                ),
                self.radius,
            ),
            // Java: FRLogger.warn("Circle.translate_by only implemented for
            // IntVectors till now"); returns this.
            _ => *self,
        }
    }

    /// Java `borderDistance(FloatPoint)`.
    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        let d = point.distance(&self.center.to_float()) - self.radius as f64;
        d.abs()
    }

    /// Java `enlarge(double)`.
    pub fn enlarge(&self, offset: f64) -> Circle {
        if offset == 0.0 {
            return *self;
        }
        let new_radius = self.radius.wrapping_add(java_round(offset) as i32);
        Circle::new(self.center, new_radius)
    }

    /// Java `intersects(Shape)` resolved through the other shape: every
    /// leaf's `intersects(Circle)` delegates back to the circle, so the
    /// dispatch here mirrors the double dispatch by enum match.
    pub fn intersects_tile(&self, other: &crate::shape::ShapeRef) -> bool {
        match other {
            crate::shape::ShapeRef::IntBox(b) => self.intersects_box(b),
            crate::shape::ShapeRef::IntOctagon(o) => self.intersects_octagon(o),
            crate::shape::ShapeRef::Simplex(s) => self.intersects_simplex(s),
            crate::shape::ShapeRef::Circle(c) => self.intersects_circle(c),
        }
    }

    /// Java `intersects(Circle)`: `center.distanceSquare(other.center) <=
    /// (radius + other.radius)^2` with the square computed as a float
    /// multiply of the int sum.
    pub fn intersects_circle(&self, other: &Circle) -> bool {
        let mut radius_sum_square = (self.radius.wrapping_add(other.radius)) as f64;
        radius_sum_square *= radius_sum_square;
        self.center
            .to_float()
            .distance_square(&other.center.to_float())
            <= radius_sum_square
    }

    /// Java `intersects(IntBox)`: `box.distance(center) <= radius`.
    pub fn intersects_box(&self, r#box: &IntBox) -> bool {
        r#box.distance(&self.center.to_float()) <= self.radius as f64
    }

    /// Java `intersects(IntOctagon)`: `oct.distance(center) <= radius`.
    pub fn intersects_octagon(&self, oct: &IntOctagon) -> bool {
        oct.intersects_circle(self)
    }

    /// Java `intersects(Simplex)`: `simplex.distance(center) <= radius`.
    pub fn intersects_simplex(&self, simplex: &Simplex) -> bool {
        crate::shape::Shape::distance(simplex, &self.center.to_float()) <= self.radius as f64
    }

    /// Java `splitToConvex()`: the single bounding tile.
    pub fn split_to_convex(&self) -> Vec<TileShape> {
        vec![self.bounding_tile()]
    }

    /// Java `getBorder()`: circles have no holes, the border is the circle.
    pub fn get_border(&self) -> Circle {
        *self
    }

    /// Java `getHoles()`: always empty.
    pub fn get_holes(&self) -> Vec<crate::shape::ShapeRef<'static>> {
        Vec::new()
    }

    /// Java `cornerApproxArr()`: circles have no corners.
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        Vec::new()
    }
}

/// Java `(IntPoint) point.turn90Degree(factor, pole)` — the Point-level
/// rotation narrowed back to an `IntPoint`.
fn int_point_turn_90(center: IntPoint, factor: i32, pole: &IntPoint) -> IntPoint {
    match Point::Int(center).turn_90_degree(factor, &Point::Int(*pole)) {
        Point::Int(p) => p,
        Point::Rational(_) => panic!("turn90Degree of an IntPoint is an IntPoint"),
    }
}

impl fmt::Display for Circle {
    /// Java `toString(Locale.ENGLISH)` — `NumberFormat` for an int is its
    /// decimal form; note the missing space before `radius` (oracle parity).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = String::from("Circle: ");
        if self.center != IntPoint::ZERO {
            result.push_str(&format!("center {}", self.center));
        }
        result.push_str(&format!("radius {}", self.radius));
        write!(f, "{result}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::Line;
    use crate::point::Point;
    use crate::regular_tile_shape::RegularTileShape;
    use crate::shape::ShapeRef;
    use crate::simplex::Simplex;
    use crate::tile_shape::TileShape;

    fn c(x: i32, y: i32, r: i32) -> Circle {
        Circle::new(IntPoint::new(x, y), r)
    }

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// Negative radii are normalized by the constructor.
    #[test]
    fn negative_radius_normalized() {
        assert_eq!(Circle::new(IntPoint::new(1, 2), -5).radius, 5);
    }

    /// Dimension 0 for point circles, 2 otherwise.
    #[test]
    fn dimension_and_measures() {
        assert_eq!(c(0, 0, 0).dimension(), 0);
        assert_eq!(c(0, 0, 7).dimension(), 2);
        assert!(!c(3, 4, 5).is_empty());
        assert!(c(3, 4, 5).is_bounded());
        let circle = c(0, 0, 10);
        assert!((circle.area() - (std::f64::consts::PI * 10.0) * 10.0).abs() < 1e-9);
        assert!((circle.circumference() - 2.0 * std::f64::consts::PI * 10.0).abs() < 1e-9);
        assert_eq!(circle.max_width(), 20.0);
        assert_eq!(circle.min_width(), 20.0);
    }

    /// The float/int distance mix: a point at distance exactly sqrt(r^2+r^2)
    /// is inside a radius-1 circle only when the float square is <= 1.
    #[test]
    fn containment_predicates() {
        let circle = c(0, 0, 5);
        // 3-4-5 triangle: exactly on the border.
        let on_border = Point::Int(IntPoint::new(3, 4));
        assert!(circle.contains_point(&on_border));
        assert!(circle.contains_on_border(&on_border));
        assert!(!circle.contains_inside(&on_border));
        assert!(!circle.is_outside(&on_border));
        let outside = Point::Int(IntPoint::new(4, 4));
        assert!(circle.is_outside(&outside));
        assert!(!circle.contains_point(&outside));
        assert!(circle.contains_float(&FloatPoint::new(5.0, 0.0)));
        assert!(!circle.contains_float(&FloatPoint::new(5.1, 0.0)));
        assert_eq!(circle.distance(&FloatPoint::new(10.0, 0.0)), 5.0);
        assert_eq!(circle.distance(&FloatPoint::new(3.0, 0.0)), 0.0);
        assert_eq!(circle.border_distance(&FloatPoint::new(10.0, 0.0)), 5.0);
    }

    /// Bounding box and octagon (jshell-captured oracle below in the
    /// integration pins; here the structural checks).
    #[test]
    fn bounding_shapes_structure() {
        let circle = c(10, 20, 5);
        assert_eq!(
            circle.bounding_box(),
            IntBox::new(IntPoint::new(5, 15), IntPoint::new(15, 25))
        );
        let oct = circle.bounding_octagon();
        // sqrt(2)-1 ~ 0.4142; 0.4142*5 = 2.07 => ceil 3, floor 2.
        assert_eq!(oct.left_x, 5);
        assert_eq!(oct.bottom_y, 15);
        assert_eq!(oct.right_x, 15);
        assert_eq!(oct.top_y, 25);
        assert_eq!(oct.upper_left_diagonal_x, 5 - (20 + 2));
        assert_eq!(oct.lower_right_diagonal_x, 15 - (20 - 3));
        assert_eq!(oct.lower_left_diagonal_x, 5 + (20 - 2));
        assert_eq!(oct.upper_right_diagonal_x, 15 + (20 + 3));
    }

    /// Offset / shrink / enlarge round the radius with java_round; shrink
    /// never drops below radius 1.
    #[test]
    fn offset_family() {
        let circle = c(0, 0, 10);
        assert_eq!(circle.offset(2.6).radius, 13);
        assert_eq!(circle.offset(-0.4).radius, 10);
        assert_eq!(circle.shrink(3.7).radius, 6);
        assert_eq!(c(0, 0, 2).shrink(5.0).radius, 1);
        assert_eq!(circle.enlarge(1.5).radius, 12);
        assert_eq!(circle.enlarge(0.0), circle);
    }

    /// Display mirrors Java toString(Locale.ENGLISH): no space between the
    /// center part and `radius`; ZERO centers are omitted.
    #[test]
    fn display_matches_java_to_string() {
        // jshell: new Circle(new IntPoint(0,0), 5) -> "Circle: radius 5"
        assert_eq!(format!("{}", c(0, 0, 5)), "Circle: radius 5");
        // jshell: new Circle(new IntPoint(3,-4), 7)
        //   -> "Circle: center (3,-4)radius 7" (no space anywhere)
        assert_eq!(format!("{}", c(3, -4, 7)), "Circle: center (3,-4)radius 7");
    }

    /// Transformations keep the radius.
    #[test]
    fn transforms() {
        let circle = c(5, 0, 3);
        let turned = circle.turn_90_degree(1, &IntPoint::new(0, 0));
        assert_eq!(turned.center, IntPoint::new(0, 5));
        let mirrored = circle.mirror_vertical(&IntPoint::new(2, 0));
        assert_eq!(mirrored.center, IntPoint::new(-1, 0));
        let moved = circle.translate_by(&Vector::Int(IntVector::new(1, 1)));
        assert_eq!(moved.center, IntPoint::new(6, 1));
        assert_eq!(moved.radius, 3);
        // non-int vectors return this (Java warns) — a RationalVector
        // exercises the warn arm (Vector::ZERO would early-return above).
        let rational = circle.translate_by(&Vector::Rational(Box::new(
            crate::rational_vector::RationalVector::new(
                num_bigint::BigInt::from(1),
                num_bigint::BigInt::from(2),
                num_bigint::BigInt::from(3),
            ),
        )));
        assert_eq!(rational, circle);
    }

    /// jshell pin `PDISP1`: `Circle.toString()` is
    /// `"Circle: center (3,-4)radius 7"` — IntPoint.toString prints
    /// `"(x,y)"` without a space and there is no space before `radius`.
    #[test]
    fn pdisp1_display_matches_java_to_string() {
        let circle = Circle::new(IntPoint::new(3, -4), 7);
        assert_eq!(format!("{}", circle), "Circle: center (3,-4)radius 7");
        assert_eq!(format!("{}", IntPoint::new(0, 0)), "(0,0)");
    }

    /// jshell pins `PIC1`..`PIC4` — `Shape.intersects(Circle)` double
    /// dispatch, all through the ShapeRef table:
    /// - box-octagon (0,0,10,10) vs circle ((3,-3), 3) TOUCHES the border:
    ///   true; radius 2: false (CO_OCT).
    /// - IntBox (0,0)-(10,10) vs circle ((13,5), r): touching at (10,5)
    ///   with r=3: true; r=2: false (CO_BOX).
    /// - 4-line Simplex of the same box vs circle ((13,5), r) (CO_SIMP).
    /// - circle-circle ((0,0),5) vs ((8,0), r): radius sum 8 = distance,
    ///   r=3: true; r=2: false (CO_CC).
    #[test]
    fn pic1_intersects_circle_pins() {
        let circle3 = Circle::new(IntPoint::new(3, -3), 3);
        let circle2 = Circle::new(IntPoint::new(3, -3), 2);
        let oct = TileShape::from_4_ints(0, 0, 10, 10);
        assert!(
            circle3.intersects_tile(&ShapeRef::from(&TileShape::RegularTileShape(
                RegularTileShape::IntOctagon(oct)
            ),))
        );
        assert!(
            !circle2.intersects_tile(&ShapeRef::from(&TileShape::RegularTileShape(
                RegularTileShape::IntOctagon(oct)
            ),))
        );

        let r#box = IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 10));
        let c13r3 = Circle::new(IntPoint::new(13, 5), 3);
        let c13r2 = Circle::new(IntPoint::new(13, 5), 2);
        assert!(ShapeRef::IntBox(&r#box).intersects_circle(&c13r3));
        assert!(!ShapeRef::IntBox(&r#box).intersects_circle(&c13r2));

        let border_lines = [
            Line::new(p(0, 0), p(10, 0)),
            Line::new(p(10, 0), p(10, 10)),
            Line::new(p(10, 10), p(0, 10)),
            Line::new(p(0, 10), p(0, 0)),
        ];
        let simplex = Simplex::get_instance(&border_lines);
        assert!(ShapeRef::Simplex(&simplex).intersects_circle(&c13r3));
        assert!(!ShapeRef::Simplex(&simplex).intersects_circle(&c13r2));

        let cc_base = Circle::new(IntPoint::new(0, 0), 5);
        let cc_touch = Circle::new(IntPoint::new(8, 0), 3);
        let cc_apart = Circle::new(IntPoint::new(8, 0), 2);
        assert!(ShapeRef::Circle(&cc_base).intersects_circle(&cc_touch));
        assert!(!ShapeRef::Circle(&cc_base).intersects_circle(&cc_apart));
    }
}
