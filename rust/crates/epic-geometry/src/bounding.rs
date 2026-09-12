//! Port of the Java `ShapeBoundingDirections` family:
//! `ShapeBoundingDirections.java`,
//! `FortyfiveDegreeBoundingDirections.java` and
//! `OrthogonalBoundingDirections.java`. The Java interface + two
//! singletons become one enum; the `bounds(ConvexShape)` entry point
//! becomes per-type methods (the Java overload set: IntBox, IntOctagon,
//! Simplex, Circle, PolygonShape). These feed the M2 search trees.
//!
//! Re-exported from [`crate::shape`] so the Task 7 import paths keep
//! working.

use crate::circle::Circle;
use crate::polygon_shape::PolygonShape;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape::ShapeBoundingDirections as Directions;

impl Directions {
    /// Java `bounds(ConvexShape)` with a `Circle` argument.
    pub fn bounds_circle(&self, circle: &Circle) -> RegularTileShape {
        match self {
            Directions::FortyfiveDegree => RegularTileShape::IntOctagon(circle.bounding_octagon()),
            Directions::Orthogonal => RegularTileShape::IntBox(circle.bounding_box()),
        }
    }

    /// Java `bounds(ConvexShape)` with a `PolygonShape` argument.
    pub fn bounds_polygon(&self, polygon: &PolygonShape) -> RegularTileShape {
        match self {
            Directions::FortyfiveDegree => RegularTileShape::IntOctagon(polygon.bounding_octagon()),
            Directions::Orthogonal => RegularTileShape::IntBox(polygon.bounding_box()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::float_point::FloatPoint;
    use crate::int_box::IntBox;
    use crate::int_octagon::IntOctagon;
    use crate::int_point::IntPoint;
    use crate::point::Point;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    /// jshell pin `PBD1`: the 45-degree bounding octagon of the box
    /// (0,0)-(10,20) is (0,0,10,20,-20,10,0,30); the orthogonal bounding
    /// shape is the box itself. Oracle:
    /// `FortyfiveDegreeBoundingDirections.INSTANCE.bounds(
    ///   new IntBox(new IntPoint(0,0), new IntPoint(10,20)))`.
    #[test]
    fn pbd1_box_directions() {
        let r#box = IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 20));
        assert_eq!(
            Directions::FortyfiveDegree.bounds_box(&r#box),
            RegularTileShape::IntOctagon(IntOctagon::new(0, 0, 10, 20, -20, 10, 0, 30))
        );
        assert_eq!(
            Directions::Orthogonal.bounds_box(&r#box),
            RegularTileShape::IntBox(r#box)
        );
        assert_eq!(Directions::FortyfiveDegree.count(), 8);
        assert_eq!(Directions::Orthogonal.count(), 4);
    }

    /// The polygon bounds delegate to the polygon's own bounding shapes
    /// (same values as `bounds_polygon` on an L-shaped polygon).
    #[test]
    fn polygon_directions() {
        let l_shape =
            PolygonShape::new(&[p(0, 0), p(10, 0), p(10, 10), p(5, 10), p(5, 5), p(0, 5)]);
        let oct = Directions::FortyfiveDegree.bounds_polygon(&l_shape);
        assert_eq!(
            oct,
            RegularTileShape::IntOctagon(l_shape.bounding_octagon())
        );
        let r#box = Directions::Orthogonal.bounds_polygon(&l_shape);
        assert_eq!(r#box, RegularTileShape::IntBox(l_shape.bounding_box()));
    }

    /// The circle bounds delegate to the circle's bounding shapes.
    #[test]
    fn circle_directions() {
        let circle = Circle::new(IntPoint::new(10, 20), 5);
        assert_eq!(
            Directions::FortyfiveDegree.bounds_circle(&circle),
            RegularTileShape::IntOctagon(circle.bounding_octagon())
        );
        assert_eq!(
            Directions::Orthogonal.bounds_circle(&circle),
            RegularTileShape::IntBox(circle.bounding_box())
        );
        // silence unused import warnings for test-only helpers
        let _ = FloatPoint::new(0.0, 0.0);
    }
}
