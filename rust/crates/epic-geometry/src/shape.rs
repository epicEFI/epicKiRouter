//! Ports of the Java shape *interfaces* `app.freerouting.geometry.planar.
//! {Area, Shape, ConvexShape}` plus the `ShapeBoundingDirections` family.
//!
//! Java models the shape hierarchy with interfaces implemented across
//! `IntBox` / `IntOctagon` / `Simplex` (and, in Task 8, `Circle` /
//! `PolylineShape`-based shapes); cross-type operations that the Java
//! virtual machine cannot dispatch in parameter position are resolved
//! with package-private overloads. In Rust the sealed enums
//! [`crate::tile_shape::TileShape`] and
//! [`crate::regular_tile_shape::RegularTileShape`] carry that dispatch;
//! the traits below exist to mirror the Java interface surfaces and are
//! implemented by the three Task 7 leaves and by [`ShapeRef`].
//!
//! Where a Java method returns the covariant `Area` / `Shape` /
//! `ConvexShape` types, the traits return the concrete
//! [`crate::tile_shape::TileShape`] (the closed set of line-bounded
//! convex shapes; Circle joins in Task 8 and will widen the enum then).
//!
//! Deferral ledger:
//! - `Area.getBorder` / `Area.getHoles` -> Task 8 (return `Shape`;
//!   meaningful only once the non-convex `Shape` family exists).
//! - `Area.rotateApprox` -> Task 8 (needs `Polygon`).
//! - `Shape.intersects(Circle)` / `Shape.cutout(Polyline)` -> Task 8
//!   (needs `Circle` / `Polyline`).
//! - `Shape.getBounds`-style members taking `Polygon` -> Task 8.

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

/// Java `Area`: functionality of an area. Implemented by [`IntBox`],
/// [`IntOctagon`], [`Simplex`] and [`ShapeRef`].
pub trait Area {
    /// Returns true, if this area is empty.
    fn is_empty(&self) -> bool;
    /// Returns true, if this area is bounded.
    fn is_bounded(&self) -> bool;
    /// The dimension of this area.
    fn dimension(&self) -> i32;
    /// Returns true, if this area is completely contained in box.
    fn is_contained_in_box(&self, r#box: &IntBox) -> bool;
    /// Returns the smallest box with integer coordinates containing this
    /// area.
    fn bounding_box(&self) -> IntBox;
    /// Returns the smallest octagon with integer coordinates containing
    /// this area.
    fn bounding_octagon(&self) -> IntOctagon;
    /// Returns true, if point is contained in this area.
    fn contains_point(&self, point: &Point) -> bool;
    /// Returns true, if point is contained in this area (float variant
    /// with tolerance 0).
    fn contains_float(&self, point: &FloatPoint) -> bool;
    /// Returns an approximation of the point in this area which has the
    /// smallest distance to from_point.
    fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint;
    /// Turns this area by factor times 90 degrees around pole.
    fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape;
    /// Returns the affine translation of this area by vector.
    fn translate_by(&self, vector: &Vector) -> TileShape;
    /// Mirrors this area at the vertical line through pole.
    fn mirror_vertical(&self, pole: &IntPoint) -> TileShape;
    /// Mirrors this area at the horizontal line through pole.
    fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape;
    /// Returns approximations of all corners of this area.
    fn corner_approx_arr(&self) -> Vec<FloatPoint>;
    /// Splits this area into convex pieces.
    fn split_to_convex(&self) -> Vec<TileShape>;
}

/// Java `Shape`: functionality of a shape. Adds the geometric measures
/// and predicates over [`Area`].
pub trait Shape: Area {
    /// Returns the cumulative border length of the shape.
    fn circumference(&self) -> f64;
    /// Returns the content of the area of the shape.
    fn area(&self) -> f64;
    /// Returns the arithmetic middle of the corners of the shape.
    fn centre_of_gravity(&self) -> FloatPoint;
    /// Returns true, if point is neither in the inside nor on the edge of
    /// the shape.
    fn is_outside(&self, point: &Point) -> bool;
    /// Returns true, if point is contained in the shape, but not on an
    /// edge line.
    fn contains_inside(&self, point: &Point) -> bool;
    /// Returns true, if point lies exactly on the boundary of the shape.
    fn contains_on_border(&self, point: &Point) -> bool;
    /// Returns the distance between point and its nearest point on the
    /// shape.
    fn distance(&self, point: &FloatPoint) -> f64;
    /// Returns the distance between point and its nearest point on the
    /// edge of the shape.
    fn border_distance(&self, point: &FloatPoint) -> f64;
    /// The bounding tile of this shape.
    fn bounding_tile(&self) -> TileShape;
    /// Returns the bounding RegularTileShape with the fixed directions.
    /// `None` where Java returns null.
    fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape>;
    /// Returns the maximal radius of the shape around its centre of
    /// gravity.
    fn smallest_radius(&self) -> f64;
    /// Returns this shape enlarged by offset.
    fn enlarge(&self, offset: f64) -> TileShape;
    /// Returns the maximum of the edge widths of the shape.
    fn max_width(&self) -> f64;
    /// Returns the minimum of the edge widths of the shape.
    fn min_width(&self) -> f64;
}

/// Java `ConvexShape`: a convex `Shape` with offset support.
pub trait ConvexShape: Shape {
    /// Returns this shape offsetted by width. If width > 0, the offset is
    /// to the outside, else to the inside.
    fn offset(&self, width: f64) -> TileShape;
    /// Returns this shape shrunk by width (Java `shrink`).
    fn shrink(&self, width: f64) -> TileShape;
}

// ---------------------------------------------------------------------------
// ShapeBoundingDirections
// ---------------------------------------------------------------------------

/// Port of Java `ShapeBoundingDirections` and its two singletons
/// `FortyfiveDegreeBoundingDirections` (8 directions) and
/// `OrthogonalBoundingDirections` (4 directions). The Java singletons
/// become enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeBoundingDirections {
    /// The 8 45-degree directions (`FortyfiveDegreeBoundingDirections`).
    FortyfiveDegree,
    /// The 4 orthogonal directions (`OrthogonalBoundingDirections`).
    Orthogonal,
}

impl ShapeBoundingDirections {
    /// The number of directions of this set.
    pub fn count(&self) -> i32 {
        match self {
            ShapeBoundingDirections::FortyfiveDegree => 8,
            ShapeBoundingDirections::Orthogonal => 4,
        }
    }

    /// Returns a RegularTileShape with the bounds of box.
    pub fn bounds_box(&self, r#box: &IntBox) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::FortyfiveDegree => {
                RegularTileShape::IntOctagon(r#box.to_int_octagon())
            }
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(*r#box),
        }
    }

    /// Returns a RegularTileShape with the bounds of octagon.
    pub fn bounds_octagon(&self, octagon: &IntOctagon) -> RegularTileShape {
        match self {
            ShapeBoundingDirections::FortyfiveDegree => RegularTileShape::IntOctagon(*octagon),
            ShapeBoundingDirections::Orthogonal => RegularTileShape::IntBox(octagon.bounding_box()),
        }
    }

    /// Returns a RegularTileShape with the bounds of simplex. `None` if
    /// the simplex is unbounded enough for the 45-degree bounding octagon
    /// (Java returns null).
    pub fn bounds_simplex(&self, simplex: &Simplex) -> Option<RegularTileShape> {
        match self {
            ShapeBoundingDirections::FortyfiveDegree => {
                simplex.bounding_octagon().map(RegularTileShape::IntOctagon)
            }
            ShapeBoundingDirections::Orthogonal => {
                Some(RegularTileShape::IntBox(simplex.bounding_box()))
            }
        }
    }

    /// Returns a RegularTileShape with the bounds of the given regular
    /// shape.
    pub fn bounds_regular(&self, regular: &RegularTileShape) -> RegularTileShape {
        match regular {
            RegularTileShape::IntBox(b) => self.bounds_box(b),
            RegularTileShape::IntOctagon(o) => self.bounds_octagon(o),
        }
    }

    /// Returns a RegularTileShape with the bounds of shape. `None` where
    /// Java returns null (unbounded simplex with the 45-degree
    /// directions).
    pub fn bounds_tile(&self, shape: &TileShape) -> Option<RegularTileShape> {
        match shape {
            TileShape::RegularTileShape(r) => Some(self.bounds_regular(r)),
            TileShape::Simplex(s) => self.bounds_simplex(s),
        }
    }
}

// ---------------------------------------------------------------------------
// ShapeRef
// ---------------------------------------------------------------------------

/// A borrowed view of one of the Task 7 leaf shapes, mirroring Java code
/// that hands around `Shape` / `Area` references. `Circle` (and the other
/// non-line-bounded shapes) join in Task 8 as further variants.
#[derive(Debug, Clone, Copy)]
pub enum ShapeRef<'a> {
    IntBox(&'a IntBox),
    IntOctagon(&'a IntOctagon),
    Simplex(&'a Simplex),
}

impl<'a> From<&'a TileShape> for ShapeRef<'a> {
    fn from(value: &'a TileShape) -> Self {
        match value {
            TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => ShapeRef::IntBox(b),
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(o)) => ShapeRef::IntOctagon(o),
            TileShape::Simplex(s) => ShapeRef::Simplex(s),
        }
    }
}

impl<'a> ShapeRef<'a> {
    /// Clones the referenced shape into an owned [`TileShape`].
    pub fn to_tile(&self) -> TileShape {
        match self {
            // `b` / `o` are `&&IntBox` / `&&IntOctagon` (ShapeRef stores
            // references to the caller's references).
            ShapeRef::IntBox(b) => TileShape::RegularTileShape(RegularTileShape::IntBox(**b)),
            ShapeRef::IntOctagon(o) => {
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(**o))
            }
            ShapeRef::Simplex(s) => TileShape::Simplex(Box::new((*s).clone())),
        }
    }

    /// The no-th border line of the referenced shape.
    pub fn border_line(&self, no: i32) -> Line {
        match self {
            ShapeRef::IntBox(b) => b.border_line(no),
            ShapeRef::IntOctagon(o) => o.border_line(no),
            ShapeRef::Simplex(s) => s.border_line(no),
        }
    }

    /// The number of border lines of the referenced shape.
    pub fn border_line_count(&self) -> usize {
        match self {
            ShapeRef::IntBox(_) => 4,
            ShapeRef::IntOctagon(_) => 8,
            ShapeRef::Simplex(s) => s.border_line_count(),
        }
    }

    /// The no-th corner of the referenced shape.
    pub fn corner(&self, no: i32) -> Point {
        match self {
            ShapeRef::IntBox(b) => Point::int(b.corner(no)),
            ShapeRef::IntOctagon(o) => Point::int(o.corner(no)),
            ShapeRef::Simplex(s) => s.corner(no),
        }
    }

    /// An approximation of the no-th corner of the referenced shape.
    /// `None` for the empty simplex (Java null).
    pub fn corner_approx(&self, no: i32) -> Option<FloatPoint> {
        match self {
            ShapeRef::IntBox(b) => Some(b.corner(no).to_float()),
            ShapeRef::IntOctagon(o) => Some(o.corner(no).to_float()),
            ShapeRef::Simplex(s) => s.corner_approx(no),
        }
    }

    /// True if the referenced shape is a `Simplex` with a dimension below
    /// 2, or an empty regular shape.
    pub fn is_empty(&self) -> bool {
        match self {
            ShapeRef::IntBox(b) => b.is_empty(),
            ShapeRef::IntOctagon(o) => o.is_empty(),
            ShapeRef::Simplex(s) => s.is_empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait implementations for the leaves
// ---------------------------------------------------------------------------

/// Shared PolylineShape generic code used by the trait impls: cumulative
/// border length from corner approximations (Java
/// `PolylineShape.circumference`).
fn generic_circumference<F>(corner_count: usize, corner_approx: F) -> f64
where
    F: Fn(i32) -> FloatPoint,
{
    let mut result = 0.0f64;
    let mut prev_corner = corner_approx(corner_count as i32 - 1);
    for i in 0..corner_count as i32 {
        let current_corner = corner_approx(i);
        result += current_corner.distance(&prev_corner);
        prev_corner = current_corner;
    }
    result
}

/// Shared TileShape:669 border-line rotation used for the shapes without
/// a leaf override.
pub(crate) fn turn_90_degree_via_border_lines<F>(
    line_count: usize,
    border_line: F,
    factor: i32,
    pole: &IntPoint,
) -> TileShape
where
    F: Fn(i32) -> Line,
{
    let new_lines: Vec<Line> = (0..line_count as i32)
        .map(|i| border_line(i).turn_90_degree(factor, pole))
        .collect();
    TileShape::get_instance(&new_lines)
}

/// Shared TileShape:705/714 border-line mirroring (no leaf overrides).
pub(crate) fn mirror_via_border_lines<F>(
    line_count: usize,
    border_line: F,
    pole: &IntPoint,
    vertical: bool,
) -> TileShape
where
    F: Fn(i32) -> Line,
{
    let new_lines: Vec<Line> = (0..line_count as i32)
        .map(|i| {
            if vertical {
                border_line(i).mirror_vertical(pole)
            } else {
                border_line(i).mirror_horizontal(pole)
            }
        })
        .collect();
    TileShape::get_instance(&new_lines)
}

impl Area for IntBox {
    fn is_empty(&self) -> bool {
        IntBox::is_empty(self)
    }
    fn is_bounded(&self) -> bool {
        // IntBox.isBounded() returns true unconditionally.
        true
    }
    fn dimension(&self) -> i32 {
        IntBox::dimension(self)
    }
    fn is_contained_in_box(&self, r#box: &IntBox) -> bool {
        self.is_contained_in(r#box)
    }
    fn bounding_box(&self) -> IntBox {
        *self
    }
    fn bounding_octagon(&self) -> IntOctagon {
        self.to_int_octagon()
    }
    fn contains_point(&self, point: &Point) -> bool {
        // TileShape.contains(Point) == !isOutside(Point); IntBox has no
        // corner-loop shortcut, so use the shared TileShape logic via the
        // border lines.
        tile_contains_point(&ShapeRef::IntBox(self), point)
    }
    fn contains_float(&self, point: &FloatPoint) -> bool {
        tile_contains_float(&ShapeRef::IntBox(self), point, 0.0)
    }
    fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        tile_nearest_point_approx(&ShapeRef::IntBox(self), from_point)
    }
    fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape {
        // IntBox overrides turn90Degree with its own coordinate swap.
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::turn_90_degree(
            self, factor, pole,
        )))
    }
    fn translate_by(&self, vector: &Vector) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::translate_by(self, vector)))
    }
    fn mirror_vertical(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(4, |i| self.border_line(i), pole, true)
    }
    fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(4, |i| self.border_line(i), pole, false)
    }
    fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..4).map(|i| self.corner(i).to_float()).collect()
    }
    fn split_to_convex(&self) -> Vec<TileShape> {
        vec![TileShape::RegularTileShape(RegularTileShape::IntBox(*self))]
    }
}

impl Shape for IntBox {
    fn circumference(&self) -> f64 {
        IntBox::circumference(self)
    }
    fn area(&self) -> f64 {
        IntBox::area(self)
    }
    fn centre_of_gravity(&self) -> FloatPoint {
        let mut x = 0.0f64;
        let mut y = 0.0f64;
        for i in 0..4 {
            let current = self.corner(i).to_float();
            x += current.x;
            y += current.y;
        }
        x /= 4.0;
        y /= 4.0;
        FloatPoint::new(x, y)
    }
    fn is_outside(&self, point: &Point) -> bool {
        tile_is_outside(&ShapeRef::IntBox(self), point)
    }
    fn contains_inside(&self, point: &Point) -> bool {
        tile_contains_inside(&ShapeRef::IntBox(self), point)
    }
    fn contains_on_border(&self, point: &Point) -> bool {
        tile_contains_on_border(&ShapeRef::IntBox(self), point)
    }
    fn distance(&self, point: &FloatPoint) -> f64 {
        self.nearest_point_approx(point).distance(point)
    }
    fn border_distance(&self, point: &FloatPoint) -> f64 {
        shape_nearest_border_point_approx(&ShapeRef::IntBox(self), point).distance(point)
    }
    fn bounding_tile(&self) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(*self))
    }
    fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        Some(dirs.bounds_box(self))
    }
    fn smallest_radius(&self) -> f64 {
        self.border_distance(&self.centre_of_gravity())
    }
    fn enlarge(&self, offset: f64) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntBox::enlarge(self, offset)))
    }
    fn max_width(&self) -> f64 {
        IntBox::max_width(self)
    }
    fn min_width(&self) -> f64 {
        IntBox::min_width(self)
    }
}

impl ConvexShape for IntBox {
    fn offset(&self, width: f64) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::offset(self, width)))
    }
    fn shrink(&self, width: f64) -> TileShape {
        tile_shrink(&ShapeRef::IntBox(self), width)
    }
}

impl Area for IntOctagon {
    fn is_empty(&self) -> bool {
        IntOctagon::is_empty(self)
    }
    fn is_bounded(&self) -> bool {
        // IntOctagon.isBounded() returns true unconditionally.
        true
    }
    fn dimension(&self) -> i32 {
        IntOctagon::dimension(self)
    }
    fn is_contained_in_box(&self, r#box: &IntBox) -> bool {
        IntOctagon::is_contained_in_box(self, r#box)
    }
    fn bounding_box(&self) -> IntBox {
        IntOctagon::bounding_box(self)
    }
    fn bounding_octagon(&self) -> IntOctagon {
        *self
    }
    fn contains_point(&self, point: &Point) -> bool {
        tile_contains_point(&ShapeRef::IntOctagon(self), point)
    }
    fn contains_float(&self, point: &FloatPoint) -> bool {
        tile_contains_float(&ShapeRef::IntOctagon(self), point, 0.0)
    }
    fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        tile_nearest_point_approx(&ShapeRef::IntOctagon(self), from_point)
    }
    fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape {
        // IntOctagon does not override turn90Degree (Task 5 ledger):
        // TileShape.java:669 border-line path.
        turn_90_degree_via_border_lines(8, |i| self.border_line(i), factor, pole)
    }
    fn translate_by(&self, vector: &Vector) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::translate_by(
            self, vector,
        )))
    }
    fn mirror_vertical(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(8, |i| self.border_line(i), pole, true)
    }
    fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(8, |i| self.border_line(i), pole, false)
    }
    fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..8).map(|i| self.corner(i).to_float()).collect()
    }
    fn split_to_convex(&self) -> Vec<TileShape> {
        vec![TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            *self,
        ))]
    }
}

impl Shape for IntOctagon {
    fn circumference(&self) -> f64 {
        generic_circumference(8, |i| self.corner(i).to_float())
    }
    fn area(&self) -> f64 {
        IntOctagon::area(self)
    }
    fn centre_of_gravity(&self) -> FloatPoint {
        let mut x = 0.0f64;
        let mut y = 0.0f64;
        for i in 0..8 {
            let current = self.corner(i).to_float();
            x += current.x;
            y += current.y;
        }
        x /= 8.0;
        y /= 8.0;
        FloatPoint::new(x, y)
    }
    fn is_outside(&self, point: &Point) -> bool {
        tile_is_outside(&ShapeRef::IntOctagon(self), point)
    }
    fn contains_inside(&self, point: &Point) -> bool {
        tile_contains_inside(&ShapeRef::IntOctagon(self), point)
    }
    fn contains_on_border(&self, point: &Point) -> bool {
        tile_contains_on_border(&ShapeRef::IntOctagon(self), point)
    }
    fn distance(&self, point: &FloatPoint) -> f64 {
        self.nearest_point_approx(point).distance(point)
    }
    fn border_distance(&self, point: &FloatPoint) -> f64 {
        shape_nearest_border_point_approx(&ShapeRef::IntOctagon(self), point).distance(point)
    }
    fn bounding_tile(&self) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(*self))
    }
    fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        Some(dirs.bounds_octagon(self))
    }
    fn smallest_radius(&self) -> f64 {
        self.border_distance(&self.centre_of_gravity())
    }
    fn enlarge(&self, offset: f64) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::enlarge(
            self, offset,
        )))
    }
    fn max_width(&self) -> f64 {
        IntOctagon::max_width(self)
    }
    fn min_width(&self) -> f64 {
        IntOctagon::min_width(self)
    }
}

impl ConvexShape for IntOctagon {
    fn offset(&self, width: f64) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::offset(
            self, width,
        )))
    }
    fn shrink(&self, width: f64) -> TileShape {
        tile_shrink(&ShapeRef::IntOctagon(self), width)
    }
}

impl Area for Simplex {
    fn is_empty(&self) -> bool {
        Simplex::is_empty(self)
    }
    fn is_bounded(&self) -> bool {
        Simplex::is_bounded(self)
    }
    fn dimension(&self) -> i32 {
        Simplex::dimension(self)
    }
    fn is_contained_in_box(&self, r#box: &IntBox) -> bool {
        // PolylineShape.isContainedIn(box) == box.contains(boundingBox());
        // the ported IntBox has no public contains(IntBox), so use the
        // equivalent flipped is_contained_in.
        self.bounding_box().is_contained_in(r#box)
    }
    fn bounding_box(&self) -> IntBox {
        Simplex::bounding_box(self)
    }
    fn bounding_octagon(&self) -> IntOctagon {
        // Trait surface is infallible; Simplex::bounding_octagon returns
        // Option in the port. The unbounded case produces the sentinel
        // box Java would compute before its null return: an unbounded
        // simplex has no valid octagon, so return the empty octagon.
        // Callers needing the Java null use Simplex::bounding_octagon.
        Simplex::bounding_octagon(self).unwrap_or(IntOctagon::EMPTY)
    }
    fn contains_point(&self, point: &Point) -> bool {
        tile_contains_point(&ShapeRef::Simplex(self), point)
    }
    fn contains_float(&self, point: &FloatPoint) -> bool {
        tile_contains_float(&ShapeRef::Simplex(self), point, 0.0)
    }
    fn nearest_point_approx(&self, from_point: &FloatPoint) -> FloatPoint {
        tile_nearest_point_approx(&ShapeRef::Simplex(self), from_point)
    }
    fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> TileShape {
        turn_90_degree_via_border_lines(
            self.border_line_count(),
            |i| self.border_line(i),
            factor,
            pole,
        )
    }
    fn translate_by(&self, vector: &Vector) -> TileShape {
        TileShape::Simplex(Box::new(Simplex::translate_by(self, vector)))
    }
    fn mirror_vertical(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(
            self.border_line_count(),
            |i| self.border_line(i),
            pole,
            true,
        )
    }
    fn mirror_horizontal(&self, pole: &IntPoint) -> TileShape {
        mirror_via_border_lines(
            self.border_line_count(),
            |i| self.border_line(i),
            pole,
            false,
        )
    }
    fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        Simplex::corner_approx_arr(self)
    }
    fn split_to_convex(&self) -> Vec<TileShape> {
        vec![TileShape::Simplex(Box::new(self.clone()))]
    }
}

impl Shape for Simplex {
    fn circumference(&self) -> f64 {
        if !self.is_bounded() {
            return 2147483647.0;
        }
        generic_circumference(self.border_line_count(), |i| {
            self.corner_approx(i).expect("bounded simplex has corners")
        })
    }
    fn area(&self) -> f64 {
        tile_area(&ShapeRef::Simplex(self))
    }
    fn centre_of_gravity(&self) -> FloatPoint {
        // The corner-average formula; garbage (NaN) on the empty simplex
        // exactly like Java.
        let corner_count = self.border_line_count();
        let mut x = 0.0f64;
        let mut y = 0.0f64;
        for i in 0..corner_count as i32 {
            let current = self.corner_approx(i).expect("non-empty in loop");
            x += current.x;
            y += current.y;
        }
        x /= corner_count as f64;
        y /= corner_count as f64;
        FloatPoint::new(x, y)
    }
    fn is_outside(&self, point: &Point) -> bool {
        tile_is_outside(&ShapeRef::Simplex(self), point)
    }
    fn contains_inside(&self, point: &Point) -> bool {
        tile_contains_inside(&ShapeRef::Simplex(self), point)
    }
    fn contains_on_border(&self, point: &Point) -> bool {
        tile_contains_on_border(&ShapeRef::Simplex(self), point)
    }
    fn distance(&self, point: &FloatPoint) -> f64 {
        self.nearest_point_approx(point).distance(point)
    }
    fn border_distance(&self, point: &FloatPoint) -> f64 {
        shape_nearest_border_point_approx(&ShapeRef::Simplex(self), point).distance(point)
    }
    fn bounding_tile(&self) -> TileShape {
        Simplex::bounding_tile(self)
    }
    fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        Simplex::bounding_shape(self, dirs)
    }
    fn smallest_radius(&self) -> f64 {
        self.border_distance(&self.centre_of_gravity())
    }
    fn enlarge(&self, offset: f64) -> TileShape {
        TileShape::Simplex(Box::new(Simplex::enlarge(self, offset)))
    }
    fn max_width(&self) -> f64 {
        Simplex::max_width(self)
    }
    fn min_width(&self) -> f64 {
        Simplex::min_width(self)
    }
}

impl ConvexShape for Simplex {
    fn offset(&self, width: f64) -> TileShape {
        TileShape::Simplex(Box::new(Simplex::offset(self, width)))
    }
    fn shrink(&self, width: f64) -> TileShape {
        tile_shrink(&ShapeRef::Simplex(self), width)
    }
}

// ---------------------------------------------------------------------------
// Shared TileShape generic algorithms (Java TileShape concrete methods),
// operating on a ShapeRef view so leaves and TileShape dispatch reuse them.
// ---------------------------------------------------------------------------

/// Java `TileShape.isOutside(Point)`.
pub(crate) fn tile_is_outside(shape: &ShapeRef, point: &Point) -> bool {
    let line_count = shape.border_line_count();
    if line_count == 0 {
        return true;
    }
    for i in 0..line_count as i32 {
        if shape.border_line(i).side_of(point) == Side::Positive {
            return true;
        }
    }
    false
}

/// Java `TileShape.contains(Point)`: the negation of `isOutside`.
pub(crate) fn tile_contains_point(shape: &ShapeRef, point: &Point) -> bool {
    !tile_is_outside(shape, point)
}

/// Java `TileShape.contains(FloatPoint, double)`.
pub(crate) fn tile_contains_float(shape: &ShapeRef, point: &FloatPoint, tolerance: f64) -> bool {
    let line_count = shape.border_line_count();
    if line_count == 0 {
        return false;
    }
    for i in 0..line_count as i32 {
        if shape.border_line(i).side_of_float(point, tolerance) != Side::Negative {
            return false;
        }
    }
    true
}

/// Java `TileShape.containsInside(Point)`.
pub(crate) fn tile_contains_inside(shape: &ShapeRef, point: &Point) -> bool {
    let line_count = shape.border_line_count();
    if line_count == 0 {
        return false;
    }
    for i in 0..line_count as i32 {
        if shape.border_line(i).side_of(point) != Side::Negative {
            return false;
        }
    }
    true
}

/// Java `TileShape.containsOnBorderLineNo(Point)`.
pub(crate) fn tile_contains_on_border_line_no(shape: &ShapeRef, point: &Point) -> i32 {
    let line_count = shape.border_line_count();
    if line_count == 0 {
        return -1;
    }
    let mut containing_line_no = -1;
    for i in 0..line_count as i32 {
        let side_of = shape.border_line(i).side_of(point);
        if side_of == Side::Positive {
            // point outside the convex shape
            return -1;
        }
        if side_of == Side::Collinear {
            containing_line_no = i;
        }
    }
    containing_line_no
}

/// Java `TileShape.containsOnBorder(Point)`.
pub(crate) fn tile_contains_on_border(shape: &ShapeRef, point: &Point) -> bool {
    tile_contains_on_border_line_no(shape, point) >= 0
}

/// Java `TileShape.area()` (shoelace over the corner approximations).
pub(crate) fn tile_area(shape: &ShapeRef) -> f64 {
    if !shape_is_bounded(shape) {
        return f64::MAX;
    }
    if tile_dimension(shape) < 2 {
        return 0.0;
    }
    let corner_count = shape.border_line_count();
    let corner_approx = |i: i32| -> FloatPoint {
        match shape.corner_approx(i) {
            Some(p) => p,
            None => {
                // Java would have dereferenced the null cornerApprox of the
                // empty simplex here; dimension() < 2 already returned, so
                // this is unreachable.
                unreachable!("bounded shape with dimension >= 2 has corners")
            }
        }
    };
    let mut prev_corner = corner_approx(corner_count as i32 - 2);
    let mut current_corner = corner_approx(corner_count as i32 - 1);
    let mut result = 0.0f64;
    for i in 0..corner_count as i32 {
        let next_corner = corner_approx(i);
        result += current_corner.x * (next_corner.y - prev_corner.y);
        prev_corner = current_corner;
        current_corner = next_corner;
    }
    0.5 * result.abs()
}

/// `isBounded` on the ShapeRef view (regular shapes are always bounded).
fn shape_is_bounded(shape: &ShapeRef) -> bool {
    match shape {
        ShapeRef::IntBox(_) | ShapeRef::IntOctagon(_) => true,
        ShapeRef::Simplex(s) => s.is_bounded(),
    }
}

/// `dimension` on the ShapeRef view.
pub(crate) fn tile_dimension(shape: &ShapeRef) -> i32 {
    match shape {
        ShapeRef::IntBox(b) => b.dimension(),
        ShapeRef::IntOctagon(o) => o.dimension(),
        ShapeRef::Simplex(s) => s.dimension(),
    }
}

/// Java `TileShape.nearestPointApprox(FloatPoint)`.
pub(crate) fn tile_nearest_point_approx(shape: &ShapeRef, from_point: &FloatPoint) -> FloatPoint {
    if tile_contains_float(shape, from_point, 0.0) {
        return *from_point;
    }
    shape_nearest_border_point_approx(shape, from_point)
}

/// Java `TileShape.nearestBorderPointApprox(FloatPoint)`.
pub(crate) fn shape_nearest_border_point_approx(
    shape: &ShapeRef,
    from_point: &FloatPoint,
) -> FloatPoint {
    let nearest = shape_nearest_border_points_approx(shape, from_point, 1);
    match nearest.first() {
        Some(p) => *p,
        None => {
            // Java returns null for the empty shape; callers dereference.
            panic!("nearestBorderPointApprox on empty shape (Java returns null)")
        }
    }
}

/// Java `TileShape.nearestBorderPointsApprox(FloatPoint, int)` — the
/// strict-`<` insertion top-k over bounded corners first and then the
/// border-line projections.
pub(crate) fn shape_nearest_border_points_approx(
    shape: &ShapeRef,
    from_point: &FloatPoint,
    count: i32,
) -> Vec<FloatPoint> {
    if count <= 0 {
        return Vec::new();
    }
    let line_count = shape.border_line_count();
    if line_count == 0 {
        return Vec::new();
    }
    if line_count == 1 {
        return vec![from_point.projection_approx(&shape.border_line(0))];
    }
    if tile_dimension(shape) == 0 {
        return vec![
            shape
                .corner_approx(0)
                .expect("dimension 0 shape has corners"),
        ];
    }
    let result_count = (count as usize).min(line_count);
    let mut nearest_points = vec![FloatPoint::new(0.0, 0.0); result_count];
    let mut min_dists = vec![f64::MAX; result_count];

    // calculate the distances to the nearest corners first
    for i in 0..line_count as i32 {
        if shape_corner_is_bounded(shape, i) {
            let current_corner = shape
                .corner_approx(i)
                .expect("bounded corner has an approximation");
            let current_distance = current_corner.distance_square(from_point);
            for j in 0..result_count {
                if current_distance < min_dists[j] {
                    let mut k = j + 1;
                    while k < result_count {
                        min_dists[k] = min_dists[k - 1];
                        nearest_points[k] = nearest_points[k - 1];
                        k += 1;
                    }
                    min_dists[j] = current_distance;
                    nearest_points[j] = current_corner;
                    break;
                }
            }
        }
    }

    let mut prev_ind = line_count as i32 - 2;
    let mut current_ind = line_count as i32 - 1;

    for next_ind in 0..line_count as i32 {
        let projection = from_point.projection_approx(&shape.border_line(current_ind));
        if (!shape_corner_is_bounded(shape, current_ind)
            || shape.border_line(prev_ind).side_of_float_zero(&projection) == Side::Negative)
            && (!shape_corner_is_bounded(shape, next_ind)
                || shape.border_line(next_ind).side_of_float_zero(&projection) == Side::Negative)
        {
            let current_distance = projection.distance_square(from_point);
            for j in 0..result_count {
                if current_distance < min_dists[j] {
                    let mut k = j + 1;
                    while k < result_count {
                        min_dists[k] = min_dists[k - 1];
                        nearest_points[k] = nearest_points[k - 1];
                        k += 1;
                    }
                    min_dists[j] = current_distance;
                    nearest_points[j] = projection;
                    break;
                }
            }
        }
        prev_ind = current_ind;
        current_ind = next_ind;
    }
    nearest_points
}

/// `cornerIsBounded` on the ShapeRef view.
fn shape_corner_is_bounded(shape: &ShapeRef, no: i32) -> bool {
    match shape {
        ShapeRef::IntBox(_) | ShapeRef::IntOctagon(_) => true,
        ShapeRef::Simplex(s) => s.corner_is_bounded(no),
    }
}

/// The arithmetic middle of the corner approximations (Java
/// `PolylineShape.centreOfGravity`, shared by all TileShapes). On an empty
/// shape this divides 0 by 0 — Java produces the same NaN garbage.
pub(crate) fn shape_centre_of_gravity(shape: &ShapeRef) -> FloatPoint {
    let corner_count = shape.border_line_count();
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    for i in 0..corner_count as i32 {
        let current = shape
            .corner_approx(i)
            .expect("loop is empty for count == 0");
        x += current.x;
        y += current.y;
    }
    x /= corner_count as f64;
    y /= corner_count as f64;
    FloatPoint::new(x, y)
}

/// Java `TileShape.shrink(double)`.
pub(crate) fn tile_shrink(shape: &ShapeRef, offset: f64) -> TileShape {
    // Java passes the NEGATED argument to offset: this.offset(-offset).
    let result = tile_offset(shape, -offset);
    if !result.is_empty() {
        return result;
    }
    let gravity = shape_centre_of_gravity(shape);
    let centre_box = gravity.bounding_box();
    // Java: this.intersection(centreBox)
    shape_intersection_tile(
        shape,
        &TileShape::RegularTileShape(RegularTileShape::IntBox(centre_box)),
    )
}

/// Java `offset` dispatch on the ShapeRef view.
pub(crate) fn tile_offset(shape: &ShapeRef, width: f64) -> TileShape {
    match shape {
        ShapeRef::IntBox(b) => {
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::offset(b, width)))
        }
        ShapeRef::IntOctagon(o) => {
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(IntOctagon::offset(o, width)))
        }
        ShapeRef::Simplex(s) => TileShape::Simplex(Box::new(Simplex::offset(s, width))),
    }
}

/// Java `intersection(TileShape)` dispatch on the ShapeRef view.
pub(crate) fn shape_intersection_tile(shape: &ShapeRef, other: &TileShape) -> TileShape {
    shape.to_tile().intersection(other)
}
