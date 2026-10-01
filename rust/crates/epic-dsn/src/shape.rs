//! Port of `io.specctra.parser.Shape` and its leaf classes `Rectangle`,
//! `Polygon`, `Circle`, `PolylinePath` and `PolygonPath` (the `Path` base):
//! the DSN shape IR, the five scope readers, the `transformToBoard[_rel]`
//! board conversions and the `boundingBox`/`union` helpers.
//!
//! Java hierarchy notes the port flattens:
//! - `Shape` is abstract with a public `layer` field; every leaf keeps its
//!   own layer. `PolylinePath.layer` is [`Option`]`<Layer>` because its
//!   reader lacks the null-layer check its siblings have
//!   (`Shape.java:107-162` has no `if (layer == null) return null`, unlike
//!   the polygon-path reader `:479-481`) — an unknown layer yields a
//!   PolylinePath with a NULL layer (jar session
//!   `/tmp/epic-t3-shape-readers.jsh`, output
//!   `/tmp/epic-t3-shape-readers.out`, case PL2).
//! - The board-side result type [`BoardShape`] mirrors the concrete
//!   `geometry.planar.Shape` subclasses the Java transforms construct:
//!   `IntBox`/`IntOctagon`/`Simplex` ([`TileShape`]), `PolygonShape` and
//!   `geometry.planar.Circle`.
//! - `TileShape extends PolylineShape` (TileShape.java:14): in
//!   `CoordinateTransform.boardToDsn(Shape, Layer)` the `instanceof
//!   PolylineShape` branch therefore also swallows octagons and simplexes
//!   (via `cornerApproxArr`), and the final warn-null branch is DEAD for
//!   every existing class. See
//!   [`CoordinateTransform::board_to_dsn_shape`].
//!
//! Layer resolution ([`get_layer`], `Shape.java:78-104`): the pseudo-layer
//! names `"pcb"`/`"signal"` are matched BEFORE the layer structure is
//! consulted, so they resolve even with no structure; anything else needs
//! `LayerStructure::get_no` in range. The canonical pseudo-layers come from
//! [`Layer::pcb`] / [`Layer::signal`] — Java compares shape layers by
//! identity (`==`, `Shape.java:82-84`), the port by value, which stays
//! equivalent as long as every site uses those constructors.
//!
//! Lexical-state note: the five shape keywords leave the scanner in the
//! LAYER_NAME state where only `pcb`/`signal` remain keywords; this is why
//! [`read_polygon_scope`] can test for the `pcb`/`signal` KEYWORD tokens
//! (`Shape.java:309-312`) while the other readers read the layer via
//! `next_string`.
//!
//! T31: a `(path ...)` scope with fewer than 5 tokens (width + <2 points)
//! is skipped (`Shape.java:470-478`); jar cases PP2a (4 tokens → null) and
//! PP2b (exactly 5 → valid) pin the exact boundary.
//!
//! T44: `PolygonPath.boundingBox` (`PolygonPath.java:111-130`) puts the
//! x-max offset OUTSIDE the `Math.max` — `bounds[2] =
//! Math.max(bounds[2], coordinateArr[i]) + offset` — so the offset COMPOUNDS
//! once per new maximum, while the y-max has it inside the max. Ported
//! bug-compatibly and pinned where the orderings flip (jar session
//! `/tmp/epic-t3-transforms.jsh`, output `/tmp/epic-t3-transforms.out`,
//! case T44: x-max 14 vs 12 for the correct-inside form).
//!
//! Deliberate divergence (unpinnable): the polyline/polygon-path corner
//! loops of `Shape.java:113-122` and `:456-467` have no end-of-file check —
//! a truncated scope appends the null token forever and the Java reader
//! hangs. The port returns `None` at [`Token::Eof`]. A hang cannot be
//! jar-captured, so there is no pin; every captured case terminates before
//! EOF.
//!
//! Known limitation, flagged for Task 9/12: a `Token::Error` (i32 overflow
//! token) aborts the whole Java parse through an uncaught
//! `NumberFormatException` (the shape readers catch `IOException` only),
//! while these `Option`-based readers collapse it into a skipped shape. The
//! abort semantics must be re-established where the scope readers consume
//! the same scanner (see the `Token::Error` doc in `lexer.rs`).
//!
//! Deferred (documented consumers): `Shape.readAreaScope` +
//! `transformAreaToBoard[_rel]` land with the keepout/plane readers
//! (M1b Task 6); `writeScope`/`writeScopeInt` of the five leaves land with
//! the SES writer (Task 13). Java `Math.min/max` differ from
//! [`f64::min`]/[`f64::max`] only on NaN (Java propagates it, Rust returns
//! the other operand) — unreachable here because the DSN number scanners
//! cannot produce NaN tokens.

use crate::coordinate_transform::CoordinateTransform;
use crate::keyword::{Keyword, skip_scope};
use crate::layer_structure::{Layer, LayerStructure};
use crate::lexer::{Scanner, Token};
use crate::sink::AreaIr;
use crate::state::AreaScopeResult;
use epic_geometry::circle::Circle as BoardCircle;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polygon_shape::PolygonShape;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::rounding::java_round;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;

/// Java `parser/Rectangle` (`Rectangle.java:11-22`): `coor` is lower-left x,
/// lower-left y, upper-right x, upper-right y (DSN doubles, unrounded).
#[derive(Clone, Debug, PartialEq)]
pub struct Rectangle {
    /// Java `layer`.
    pub layer: Layer,
    /// Java `coor`.
    pub coor: [f64; 4],
}

/// Java `parser/Polygon` (`Polygon.java:12-23`): `coor` is
/// x0, y0, x1, y1, ... (2 * point_count entries).
#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    /// Java `layer`.
    pub layer: Layer,
    /// Java `coor`.
    pub coor: Vec<f64>,
}

/// Java `parser/Circle` (`Circle.java:13-23`): `coor[0]` is the diameter as
/// written in the DSN, `coor[1]`/`coor[2]` the center x/y.
#[derive(Clone, Debug, PartialEq)]
pub struct Circle {
    /// Java `layer`.
    pub layer: Layer,
    /// Java `coor`.
    pub coor: [f64; 3],
}

/// Java `parser/PolylinePath` (`PolylinePath.java:11-16`): a path given as
/// line segments, 4 coordinates per line. The transforms are null stubs
/// (`PolylinePath.java:56-72`).
#[derive(Clone, Debug, PartialEq)]
pub struct PolylinePath {
    /// Java `layer` — `None` (Java null) survives an unknown layer because
    /// the reader has no null check (module docs).
    pub layer: Option<Layer>,
    /// Java `width`.
    pub width: f64,
    /// Java `coordinateArr`.
    pub coordinate_arr: Vec<f64>,
}

/// Java `parser/PolygonPath` (`PolygonPath.java:13-18`): a path given as a
/// corner sequence, the hot shape for reference-routed wires.
#[derive(Clone, Debug, PartialEq)]
pub struct PolygonPath {
    /// Java `layer`.
    pub layer: Layer,
    /// Java `width`.
    pub width: f64,
    /// Java `coordinateArr`.
    pub coordinate_arr: Vec<f64>,
}

/// Java `Shape` (`Shape.java:21-27`): the five leaf shapes a shape scope
/// can produce.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Java `Rectangle`.
    Rectangle(Rectangle),
    /// Java `Polygon`.
    Polygon(Polygon),
    /// Java `Circle`.
    Circle(Circle),
    /// Java `PolylinePath`.
    PolylinePath(PolylinePath),
    /// Java `PolygonPath` (DSN keyword `path`).
    PolygonPath(PolygonPath),
}

/// The concrete `geometry.planar.Shape` subclasses the Java
/// `transformToBoard`/`transformToBoardRel` methods and the
/// `boardToDsn(Shape, Layer)` overloads traffic in.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardShape {
    /// `IntBox` / `IntOctagon` / `Simplex` (Java virtual `TileShape`).
    Tile(TileShape),
    /// Java `geometry.planar.PolygonShape`.
    PolygonShape(PolygonShape),
    /// Java `geometry.planar.Circle`.
    Circle(BoardCircle),
}

impl BoardShape {
    /// Java `Area.dimension()` over the concrete board shapes: 0 for
    /// degenerate tiles/polygons, 2 for real areas. The Circle arm IS
    /// reachable from Task 6 — a single-shape circle keepout survives
    /// `transformAreaToBoard` uncast (the `(PolylineShape)` casts only
    /// guard createBoard and the PolylineArea constructor), and Java
    /// `Circle.dimension` (`Circle.java:36-42`) is 2 unless the radius is
    /// 0. Jar `/tmp/epic-t6-board.out` t6-keep-circle: the r=1500 circle
    /// keepout is INSERTED on both layers, not degenerate-skipped.
    pub(crate) fn dimension(&self) -> i32 {
        match self {
            BoardShape::Tile(tile) => tile.dimension(),
            BoardShape::PolygonShape(polygon) => polygon.dimension(),
            BoardShape::Circle(circle) => {
                if circle.radius == 0 {
                    0
                } else {
                    2
                }
            }
        }
    }

    /// Java `Circle.boundingBox` (`Circle.java:99-105`) for the Circle arm:
    /// center +/- radius on both axes (jar t6-keep-circle: the r=1500
    /// circle at (50000, 40000) dumps bbox 48500 38500 .. 51500 41500).
    pub(crate) fn bounding_box(&self) -> IntBox {
        match self {
            BoardShape::Tile(tile) => tile.bounding_box(),
            BoardShape::PolygonShape(polygon) => polygon.bounding_box(),
            BoardShape::Circle(circle) => IntBox::new(
                IntPoint::new(
                    circle.center.x - circle.radius,
                    circle.center.y - circle.radius,
                ),
                IntPoint::new(
                    circle.center.x + circle.radius,
                    circle.center.y + circle.radius,
                ),
            ),
        }
    }

    /// Java `PolylineShape.splitToConvex()`: tiles decompose to themselves
    /// (`TileShape.java:890-894`); the polygon split can fail (Java null).
    /// A Circle is NOT a `PolylineShape` (a Java cast here throws) — the
    /// `None` arm stands in for that; every current caller rejects circles
    /// before splitting.
    pub(crate) fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        match self {
            BoardShape::Tile(tile) => Some(vec![tile.clone()]),
            BoardShape::PolygonShape(polygon) => polygon.split_to_convex(),
            BoardShape::Circle(_) => None,
        }
    }

    /// Java `PolylineShape.borderLineCount()`. A Circle is not a
    /// `PolylineShape` (the Java cast would throw) — stub 0.
    pub(crate) fn border_line_count(&self) -> i32 {
        match self {
            BoardShape::Tile(tile) => tile.border_line_count() as i32,
            BoardShape::PolygonShape(polygon) => polygon.border_line_count() as i32,
            BoardShape::Circle(_) => 0,
        }
    }

    /// Java `PolylineShape.corner(i)`. A Circle is not a `PolylineShape`
    /// (the Java cast would throw) — stub origin.
    pub(crate) fn corner(&self, no: i32) -> Point {
        match self {
            BoardShape::Tile(tile) => tile.corner(no),
            BoardShape::PolygonShape(polygon) => polygon.corner(no),
            BoardShape::Circle(_) => Point::Int(IntPoint::new(0, 0)),
        }
    }

    /// Java `Area.turn90Degree(factor, pole)` over the concrete classes:
    /// `TileShape` rotates the border lines (`TileShape.java:663-671`),
    /// `PolylineShape`/`Circle` rotate the center/corners and keep the kind
    /// (`Circle.java:192-195`, `PolygonShape.turn90Degree`). T49: used by
    /// [`crate::ses_board::SesBoard::obstacle_absolute_area`] for the
    /// `ObstacleArea.getArea()` placement transform.
    pub(crate) fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> BoardShape {
        match self {
            BoardShape::Tile(tile) => tile_shape(tile.turn_90_degree(factor, pole)),
            BoardShape::PolygonShape(polygon) => {
                BoardShape::PolygonShape(polygon.turn_90_degree(factor, pole))
            }
            BoardShape::Circle(circle) => BoardShape::Circle(circle.turn_90_degree(factor, pole)),
        }
    }

    /// Java `Area.rotateApprox(angle, pole)`: the approximate rotation.
    /// `TileShape` rotates the corner approximations and rebuilds the
    /// smallest containing tile (`TileShape.java:673-698` — a box rotated
    /// by 45° comes back as an octagon/simplex), polygons/circles keep
    /// their kind (`Circle.java:197-200` re-centers and keeps the radius).
    pub(crate) fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> BoardShape {
        match self {
            BoardShape::Tile(tile) => tile_shape(tile.rotate_approx(angle, pole)),
            BoardShape::PolygonShape(polygon) => {
                BoardShape::PolygonShape(polygon.rotate_approx(angle, pole))
            }
            BoardShape::Circle(circle) => BoardShape::Circle(circle.rotate_approx(angle, pole)),
        }
    }

    /// Java `Area.mirrorVertical(pole)` — the mirror at the VERTICAL line
    /// through the pole (x reflected about pole.x; `point.rs`
    /// `mirror_at_y_axis`). `TileShape.java:700-707`, `Circle.java:207-210`,
    /// `PolygonShape.mirrorVertical`.
    pub(crate) fn mirror_vertical(&self, pole: &IntPoint) -> BoardShape {
        match self {
            BoardShape::Tile(tile) => tile_shape(tile.mirror_vertical(pole)),
            BoardShape::PolygonShape(polygon) => {
                BoardShape::PolygonShape(polygon.mirror_vertical(pole))
            }
            BoardShape::Circle(circle) => BoardShape::Circle(circle.mirror_vertical(pole)),
        }
    }

    /// Java `Area.translateBy(vector)` (`PolylineShape.java:44`,
    /// `Circle.java:245-251`, `PolygonShape.java:242-249`).
    pub(crate) fn translate_by(&self, vector: &Vector) -> BoardShape {
        match self {
            BoardShape::Tile(tile) => tile_shape(tile.translate_by(vector)),
            BoardShape::PolygonShape(polygon) => {
                BoardShape::PolygonShape(polygon.translate_by(vector))
            }
            BoardShape::Circle(circle) => BoardShape::Circle(circle.translate_by(vector)),
        }
    }
}

/// The `IntBox` board shape constructor (used by the structure reader's
/// power-plane fallback too).
pub(crate) fn box_shape(r#box: IntBox) -> BoardShape {
    BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(r#box)))
}

fn octagon_shape(octagon: IntOctagon) -> BoardShape {
    BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntOctagon(
        octagon,
    )))
}

fn tile_shape(tile: TileShape) -> BoardShape {
    BoardShape::Tile(tile)
}

/// Java `PolylineShape.cornerApproxArr()` over the flattened port: Java
/// `TileShape extends PolylineShape`, so boxes, octagons and simplexes all
/// expose their border-corner approximations (4, 8 and `borderLineCount`
/// corners respectively — an EMPTY simplex has 0, jar case B5).
pub fn tile_corner_approx_arr(tile: &TileShape) -> Vec<FloatPoint> {
    match tile {
        TileShape::RegularTileShape(regular) => regular.corner_approx_arr(),
        TileShape::Simplex(simplex) => simplex.corner_approx_arr(),
    }
}

/// Java `PolylineShape.cornerApproxArr()` for `PolygonShape` — the only
/// PolylineShape subclass outside the tile hierarchy (`PolygonShape.java`
/// inherits the corner-approximation loop).
pub fn polygon_shape_corner_approx_arr(polygon_shape: &PolygonShape) -> Vec<FloatPoint> {
    (0..polygon_shape.border_line_count() as i32)
        .map(|i| polygon_shape.corner(i).to_float())
        .collect()
}

impl Shape {
    /// Java `Shape.transformToBoard` dispatch (`PolylinePath` warn-null
    /// stub, `PolylinePath.java:62-66`).
    pub fn transform_to_board(
        &self,
        coordinate_transform: &CoordinateTransform,
    ) -> Option<BoardShape> {
        match self {
            Shape::Rectangle(shape) => Some(shape.transform_to_board(coordinate_transform)),
            Shape::Polygon(shape) => Some(shape.transform_to_board(coordinate_transform)),
            Shape::Circle(shape) => Some(shape.transform_to_board(coordinate_transform)),
            // FRLogger.warn is log-only — not a parity-warnings surface (D12)
            Shape::PolylinePath(_) => None,
            Shape::PolygonPath(shape) => Some(shape.transform_to_board(coordinate_transform)),
        }
    }

    /// Java `Shape.transformToBoardRel` dispatch (`PolylinePath.java:56-60`).
    pub fn transform_to_board_rel(
        &self,
        coordinate_transform: &CoordinateTransform,
    ) -> Option<BoardShape> {
        match self {
            Shape::Rectangle(shape) => Some(shape.transform_to_board_rel(coordinate_transform)),
            Shape::Polygon(shape) => Some(shape.transform_to_board_rel(coordinate_transform)),
            Shape::Circle(shape) => Some(shape.transform_to_board_rel(coordinate_transform)),
            Shape::PolylinePath(_) => None,
            Shape::PolygonPath(shape) => Some(shape.transform_to_board_rel(coordinate_transform)),
        }
    }

    /// Java `Shape.boundingBox` dispatch (`PolylinePath.java:68-72` warn-null
    /// stub).
    pub fn bounding_box(&self) -> Option<Rectangle> {
        match self {
            Shape::Rectangle(shape) => Some(shape.bounding_box()),
            Shape::Polygon(shape) => Some(shape.bounding_box()),
            Shape::Circle(shape) => Some(shape.bounding_box()),
            Shape::PolylinePath(_) => None,
            Shape::PolygonPath(shape) => Some(shape.bounding_box()),
        }
    }
}

impl Rectangle {
    /// Java `boundingBox` (`Rectangle.java:24-27`) returns `this`; the port
    /// clones (value-equal). The clone is cheap (a `Layer` String plus four
    /// doubles) and cannot be avoided without handing out `&self`, so the
    /// hot loops of later M1b tasks (Task 4 board model, Task 13 SES writer)
    /// should hoist the call out of inner loops instead of reaching for a
    /// borrowing variant.
    pub fn bounding_box(&self) -> Rectangle {
        self.clone()
    }

    /// Java `union` (`Rectangle.java:29-37`): the smallest rectangle
    /// containing both; keeps THIS rectangle's layer.
    pub fn union(&self, other: &Rectangle) -> Rectangle {
        Rectangle {
            layer: self.layer.clone(),
            coor: [
                self.coor[0].min(other.coor[0]),
                self.coor[1].min(other.coor[1]),
                self.coor[2].max(other.coor[2]),
                self.coor[3].max(other.coor[3]),
            ],
        }
    }

    /// Java `transformToBoardRel` (`Rectangle.java:39-56`): per-coordinate
    /// `(int) Math.round(dsnToBoard(..))` — pure scaling, the base plays no
    /// role here (only `dsnToBoard(double[])` subtracts it) — then the two
    /// `IntBox` constructor orders for the y-ordered and y-flipped inputs.
    pub fn transform_to_board_rel(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let mut box_coor = [0i32; 4];
        for (i, value) in box_coor.iter_mut().enumerate() {
            *value = java_round(coordinate_transform.dsn_to_board_value(self.coor[i])) as i32;
        }
        let r#box = if box_coor[1] <= box_coor[3] {
            // boxCoor describe lower left and upper right corner
            IntBox::new(
                IntPoint::new(box_coor[0], box_coor[1]),
                IntPoint::new(box_coor[2], box_coor[3]),
            )
        } else {
            // boxCoor describe upper left and lower right corner
            IntBox::new(
                IntPoint::new(box_coor[0], box_coor[3]),
                IntPoint::new(box_coor[2], box_coor[1]),
            )
        };
        box_shape(r#box)
    }

    /// Java `transformToBoard` (`Rectangle.java:58-69`): the min/max
    /// coordinate swap happens on the DSN doubles, both corners go through
    /// `dsnToBoard(double[])`, then each is rounded (`FloatPoint.round` —
    /// Java `Math.round`, half away from zero).
    pub fn transform_to_board(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let lower_left = coordinate_transform.dsn_to_board_tuple([
            self.coor[0].min(self.coor[2]),
            self.coor[1].min(self.coor[3]),
        ]);
        let upper_right = coordinate_transform.dsn_to_board_tuple([
            self.coor[0].max(self.coor[2]),
            self.coor[1].max(self.coor[3]),
        ]);
        box_shape(IntBox::new(lower_left.round(), upper_right.round()))
    }
}

impl Polygon {
    /// Java `transformToBoard` (`Polygon.java:25-36`): each corner through
    /// `dsnToBoard(double[]).round()`, then `PolygonShape` (which normalizes
    /// clockwise input away).
    ///
    /// CRASH PARITY (Task 3 review): `(polygon signal 0)` parses to an
    /// EMPTY `coor: []` and the transform then panics indexing corner 0 —
    /// exactly Java, where the same input reaches
    /// `geometry.planar.PolygonShape.<init>` and throws
    /// `ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0`
    /// at `PolygonShape.java:72` (jar session `/tmp/epic-t3-part-b.jsh`,
    /// output `/tmp/epic-t3-part-b.out`, EMPTY case: reader coorLen=0, then
    /// the AIOOBE). The port panics the same way through
    /// [`epic_geometry::polygon_shape::PolygonShape::new`]; whatever feeds
    /// reader output into `transformAreaToBoard` (M1b Task 6) must treat
    /// this as a parse failure BEFORE calling the transform, mirroring how
    /// the Java parse dies on the same input.
    pub fn transform_to_board(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let mut points: Vec<Point> = Vec::with_capacity(self.coor.len() / 2);
        for i in 0..self.coor.len() / 2 {
            let point =
                coordinate_transform.dsn_to_board_tuple([self.coor[2 * i], self.coor[2 * i + 1]]);
            points.push(Point::Int(point.round()));
        }
        BoardShape::PolygonShape(PolygonShape::new(&points))
    }

    /// Java `transformToBoardRel` (`Polygon.java:38-51`): an empty
    /// coordinate array yields `Simplex.EMPTY`; otherwise per-corner
    /// `(int) Math.round(dsnToBoard(double))` — NO base.
    pub fn transform_to_board_rel(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        if self.coor.len() < 2 {
            return tile_shape(TileShape::Simplex(Box::new(Simplex::empty())));
        }
        let mut points: Vec<Point> = Vec::with_capacity(self.coor.len() / 2);
        for i in 0..self.coor.len() / 2 {
            let x = java_round(coordinate_transform.dsn_to_board_value(self.coor[2 * i])) as i32;
            let y =
                java_round(coordinate_transform.dsn_to_board_value(self.coor[2 * i + 1])) as i32;
            points.push(Point::Int(IntPoint::new(x, y)));
        }
        BoardShape::PolygonShape(PolygonShape::new(&points))
    }

    /// Java `boundingBox` (`Polygon.java:53-72`): plain min/max, NO width
    /// offset (unlike [`PolygonPath::bounding_box`]).
    pub fn bounding_box(&self) -> Rectangle {
        let mut bounds = [
            f64::from(i32::MAX),
            f64::from(i32::MAX),
            f64::from(i32::MIN),
            f64::from(i32::MIN),
        ];
        for (i, &value) in self.coor.iter().enumerate() {
            if i % 2 == 0 {
                bounds[0] = bounds[0].min(value);
                bounds[2] = bounds[2].max(value);
            } else {
                bounds[1] = bounds[1].min(value);
                bounds[3] = bounds[3].max(value);
            }
        }
        Rectangle {
            layer: self.layer.clone(),
            coor: bounds,
        }
    }
}

impl Circle {
    /// Java `transformToBoard` (`Circle.java:33-42`): center through
    /// `dsnToBoard(double[]).round()`; radius
    /// `(int) Math.round(dsnToBoard(diameter) / 2)` — the diameter halves
    /// AFTER scaling.
    pub fn transform_to_board(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let center = coordinate_transform
            .dsn_to_board_tuple([self.coor[1], self.coor[2]])
            .round();
        let radius = java_round(coordinate_transform.dsn_to_board_value(self.coor[0]) / 2.0) as i32;
        BoardShape::Circle(BoardCircle::new(center, radius))
    }

    /// Java `transformToBoardRel` (`Circle.java:44-54`): same halving order,
    /// per-coordinate scaling without base.
    pub fn transform_to_board_rel(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let radius = java_round(coordinate_transform.dsn_to_board_value(self.coor[0]) / 2.0) as i32;
        let center = IntPoint::new(
            java_round(coordinate_transform.dsn_to_board_value(self.coor[1])) as i32,
            java_round(coordinate_transform.dsn_to_board_value(self.coor[2])) as i32,
        );
        BoardShape::Circle(BoardCircle::new(center, radius))
    }

    /// Java `boundingBox` (`Circle.java:56-64`): center +/- diameter.
    pub fn bounding_box(&self) -> Rectangle {
        Rectangle {
            layer: self.layer.clone(),
            coor: [
                self.coor[1] - self.coor[0],
                self.coor[2] - self.coor[0],
                self.coor[1] + self.coor[0],
                self.coor[2] + self.coor[0],
            ],
        }
    }
}

impl PolygonPath {
    /// Java `transformToBoard` (`PolygonPath.java:58-82`): corners through
    /// `dsnToBoard(double[])`; `offset = dsnToBoard(width) / 2`; at most two
    /// corners become the bounding octagon enlarged by the offset; more
    /// corners become a `PolygonShape` of the rounded corners, itself
    /// replaced by `boundingTile().enlarge(offset)` iff `offset > 0`.
    pub fn transform_to_board(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let mut corners: Vec<FloatPoint> = Vec::with_capacity(self.coordinate_arr.len() / 2);
        for i in 0..self.coordinate_arr.len() / 2 {
            corners.push(
                coordinate_transform.dsn_to_board_tuple([
                    self.coordinate_arr[2 * i],
                    self.coordinate_arr[2 * i + 1],
                ]),
            );
        }
        let offset = coordinate_transform.dsn_to_board_value(self.width) / 2.0;
        if corners.len() <= 2 {
            return octagon_shape(FloatPoint::bounding_octagon(&corners).enlarge(offset));
        }
        let rounded: Vec<Point> = corners.iter().map(|c| Point::Int(c.round())).collect();
        let polygon = PolygonShape::new(&rounded);
        if offset > 0.0 {
            tile_shape(polygon.bounding_tile().enlarge(offset))
        } else {
            BoardShape::PolygonShape(polygon)
        }
    }

    /// Java `transformToBoardRel` (`PolygonPath.java:84-108`): the same
    /// structure with relative (base-free) corner scaling.
    pub fn transform_to_board_rel(&self, coordinate_transform: &CoordinateTransform) -> BoardShape {
        let mut corners: Vec<FloatPoint> = Vec::with_capacity(self.coordinate_arr.len() / 2);
        for i in 0..self.coordinate_arr.len() / 2 {
            corners.push(coordinate_transform.dsn_to_board_rel_tuple([
                self.coordinate_arr[2 * i],
                self.coordinate_arr[2 * i + 1],
            ]));
        }
        let offset = coordinate_transform.dsn_to_board_value(self.width) / 2.0;
        if corners.len() <= 2 {
            return octagon_shape(FloatPoint::bounding_octagon(&corners).enlarge(offset));
        }
        let rounded: Vec<Point> = corners.iter().map(|c| Point::Int(c.round())).collect();
        let polygon = PolygonShape::new(&rounded);
        if offset > 0.0 {
            tile_shape(polygon.bounding_tile().enlarge(offset))
        } else {
            BoardShape::PolygonShape(polygon)
        }
    }

    /// Java `boundingBox` (`PolygonPath.java:110-130`) — carries the T44
    /// offset paren bug: the x-max offset is OUTSIDE the `Math.max` and
    /// compounds per new maximum, the y-max offset is inside. Ported
    /// bug-compatibly (module docs).
    pub fn bounding_box(&self) -> Rectangle {
        let offset = self.width / 2.0;
        let mut bounds = [
            f64::from(i32::MAX),
            f64::from(i32::MAX),
            f64::from(i32::MIN),
            f64::from(i32::MIN),
        ];
        for (i, &value) in self.coordinate_arr.iter().enumerate() {
            if i % 2 == 0 {
                bounds[0] = bounds[0].min(value - offset);
                // T44 (PolygonPath.java:122) — offset OUTSIDE the max, kept.
                bounds[2] = bounds[2].max(value) + offset;
            } else {
                bounds[1] = bounds[1].min(value - offset);
                bounds[3] = bounds[3].max(value + offset);
            }
        }
        Rectangle {
            layer: self.layer.clone(),
            coor: bounds,
        }
    }
}

/// Java `Shape.getLayer` (`Shape.java:78-104`): the pcb/signal pseudo-layer
/// names match BEFORE the structure is consulted (jar cases R1/R3: both
/// resolve with a NULL structure); any other name needs
/// `LayerStructure::get_no` in range. The FRLogger.warn calls are
/// diagnostics only (omitted).
fn get_layer(layer_structure: Option<&LayerStructure>, layer_name: &str) -> Option<Layer> {
    if layer_name == Layer::PCB_NAME {
        return Some(Layer::pcb());
    }
    if layer_name == Layer::SIGNAL_NAME {
        return Some(Layer::signal());
    }
    let structure = layer_structure?;
    let layer_index = structure.get_no(layer_name);
    if !(0..structure.layers.len() as i32).contains(&layer_index) {
        return None;
    }
    Some(structure.layers[layer_index as usize].clone())
}

/// Java `instanceof Double || instanceof Integer` on a token
/// (Shape.java:270-273 et al.).
pub(crate) fn number_value(token: &Token) -> Option<f64> {
    match token {
        Token::Double(value) => Some(*value),
        Token::Int(value) => Some(f64::from(*value)),
        _ => None,
    }
}

/// Java `Shape.readScope` (`Shape.java:33-45`): overreads one optional open
/// bracket, then dispatches on the shape keyword. The IOException catch site
/// has no counterpart (the Rust scanner cannot fail).
pub fn read_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<Shape> {
    let mut token = scanner.next_token();
    if token == Token::Open {
        // overread the open bracket
        token = scanner.next_token();
    }
    read_scope_from_keyword(scanner, &token, layer_structure)
}

/// Java `Shape.readScopeFromKeyword` (`Shape.java:50-69`): dispatch on the
/// already-scanned shape keyword; anything else skip-scopes and yields None.
pub fn read_scope_from_keyword(
    scanner: &mut Scanner,
    keyword: &Token,
    layer_structure: Option<&LayerStructure>,
) -> Option<Shape> {
    match keyword {
        Token::Keyword(Keyword::Rectangle) => {
            read_rectangle_scope(scanner, layer_structure).map(Shape::Rectangle)
        }
        Token::Keyword(Keyword::Polygon) => {
            read_polygon_scope(scanner, layer_structure).map(Shape::Polygon)
        }
        Token::Keyword(Keyword::Circle) => {
            read_circle_scope(scanner, layer_structure).map(Shape::Circle)
        }
        Token::Keyword(Keyword::PolygonPath) => {
            read_polygon_path_scope(scanner, layer_structure).map(Shape::PolygonPath)
        }
        Token::Keyword(Keyword::PolylinePath) => {
            read_polyline_path_scope(scanner, layer_structure).map(Shape::PolylinePath)
        }
        _ => {
            skip_scope(scanner);
            None
        }
    }
}

/// Java `Shape.readRectangleScope` (`Shape.java:257-298`): the SIGNAL
/// fallback (`:261-263`) rescues a failed layer lookup — with the name match
/// in [`get_layer`] it even fires with a NULL structure (jar R3) — but the
/// reader still fails when the fallback also misses (jar: unreachable for
/// "signal", which always matches).
pub fn read_rectangle_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<Rectangle> {
    let layer_name = scanner.next_string();
    let mut layer = get_layer(layer_structure, &layer_name);
    if layer.is_none() {
        layer = get_layer(layer_structure, Layer::SIGNAL_NAME);
    }

    let mut coor = [0.0; 4];
    for value in &mut coor {
        // fail fast on the first non-number (Shape.java:268-281)
        *value = number_value(&scanner.next_token())?;
    }
    // overread the closing bracket (Shape.java:284-289)
    if scanner.next_token() != Token::Close {
        return None;
    }
    // Shape.java:290-292 — unreachable in practice: the SIGNAL fallback
    // makes a None layer impossible for a matching name
    Some(Rectangle {
        layer: layer?,
        coor,
    })
}

/// Java `Shape.readPolygonScope` (`Shape.java:304-391`): the ONLY reader
/// that tests the pcb/signal KEYWORD tokens (`:309-312`, the LAYER_NAME
/// lexical state keeps them keywords); a failed named-layer lookup sets
/// `layerOk = false` and the scope is STILL fully consumed before it fails
/// (`:367-369`).
pub fn read_polygon_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<Polygon> {
    let mut layer: Option<Layer> = None;
    let mut layer_ok = true;
    let token = scanner.next_token();
    if token == Token::Keyword(Keyword::Pcb) {
        layer = Some(Layer::pcb());
    } else if token == Token::Keyword(Keyword::Signal) {
        layer = Some(Layer::signal());
    } else {
        // Shape.java:314-320: without a structure only pcb/signal exist
        let structure = layer_structure?;
        // Shape.java:321-327: a layer-name STRING is required
        let Token::Str(layer_name) = &token else {
            return None;
        };
        let layer_index = structure.get_no(layer_name);
        if !(0..structure.layers.len() as i32).contains(&layer_index) {
            layer_ok = false; // the scope below is consumed regardless
        } else {
            layer = Some(structure.layers[layer_index as usize].clone());
        }
    }

    // overread the aperture width (Shape.java:342-343)
    scanner.next_token();

    let mut coor_tokens: Vec<Token> = Vec::new();
    loop {
        let mut token = scanner.next_token();
        if token == Token::Eof {
            // Shape.java:350-356
            return None;
        }
        if token == Token::Open {
            // unknown embedded scope (Shape.java:357-361)
            skip_scope(scanner);
            token = scanner.next_token();
        }
        if token == Token::Close {
            break;
        }
        coor_tokens.push(token);
    }
    if !layer_ok {
        return None; // Shape.java:367-369
    }
    let mut coor = Vec::with_capacity(coor_tokens.len());
    for token in &coor_tokens {
        coor.push(number_value(token)?);
    }
    Some(Polygon {
        layer: layer?,
        coor,
    })
}

/// Java `Shape.readCircleScope` (`Shape.java:394-444`): a failed layer only
/// WARNS mid-read (`:399-406`) and fails at the END (`:436-438`) — there is
/// NO signal fallback for circles (jar C3).
pub fn read_circle_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<Circle> {
    let layer_name = scanner.next_string();
    let layer = get_layer(layer_structure, &layer_name);

    let mut coor = [0.0; 3];
    let mut current_index = 0usize;
    loop {
        let token = scanner.next_token();
        if token == Token::Close {
            break;
        }
        if current_index > 2 {
            // Shape.java:417-423: a fourth number fails the read
            return None;
        }
        coor[current_index] = number_value(&token)?;
        current_index += 1;
    }
    Some(Circle {
        layer: layer?,
        coor,
    })
}

/// Java `Shape.readPolygonPathScope` (`Shape.java:447-516`): the corner
/// token list holds width + coordinate pairs; fewer than 5 tokens fails
/// BEFORE the layer check (`:470-481`, T31) — and unknown embedded scopes
/// are skipped inside the loop (`:458-462`).
pub fn read_polygon_path_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<PolygonPath> {
    let layer_name = scanner.next_string();
    let layer = get_layer(layer_structure, &layer_name);

    let mut corner_tokens: Vec<Token> = Vec::new();
    loop {
        let mut token = scanner.next_token();
        if token == Token::Open {
            skip_scope(scanner);
            token = scanner.next_token();
        }
        if token == Token::Eof {
            // Java appends the null token forever here (no check);
            // deliberate divergence — terminate instead (module docs).
            return None;
        }
        if token == Token::Close {
            break;
        }
        corner_tokens.push(token);
    }

    if corner_tokens.len() < 5 {
        // T31: single-point paths are not valid traces (Shape.java:470-478;
        // FRLogger.debug there is log-only)
        return None;
    }
    let layer = layer?; // Shape.java:479-481 — AFTER the size check
    let mut values = Vec::with_capacity(corner_tokens.len());
    for token in &corner_tokens {
        values.push(number_value(token)?);
    }
    Some(PolygonPath {
        layer,
        width: values[0],
        coordinate_arr: values[1..].to_vec(),
    })
}

/// Java `Shape.readPolylinePathScope` (`Shape.java:107-162`): like the
/// polygon-path reader but WITHOUT the embedded-scope skip in the loop and
/// WITHOUT the null-layer check at the end (jar PL2 — the layer stays null).
pub fn read_polyline_path_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
) -> Option<PolylinePath> {
    let layer_name = scanner.next_string();
    let layer = get_layer(layer_structure, &layer_name);

    let mut corner_tokens: Vec<Token> = Vec::new();
    loop {
        let token = scanner.next_token();
        if token == Token::Close {
            break;
        }
        if token == Token::Eof {
            // Java appends the null token forever (no check); deliberate
            // divergence — terminate instead (module docs).
            return None;
        }
        corner_tokens.push(token);
    }
    if corner_tokens.len() < 5 {
        // Shape.java:123-129 (T31)
        return None;
    }
    let mut values = Vec::with_capacity(corner_tokens.len());
    for token in &corner_tokens {
        values.push(number_value(token)?);
    }
    Some(PolylinePath {
        layer,
        width: values[0],
        coordinate_arr: values[1..].to_vec(),
    })
}

/// Java `Shape.readAreaScope` (`Shape.java:169-251`): one area scope — an
/// optional NAME string, the border shape, then `(window ...)` holes
/// (unless `skip_window_scopes`, set when the host is Allegro —
/// `Plane.readScope` — whose cutouts would fragment the conduction area),
/// `(clearance_class ...)` and unknown scopes. `None` mirrors the Java
/// null return: a failed FIRST shape (`resultOk = false`), end of file or
/// a scan error anywhere, or a missing bracket after a window shape.
///
/// Bug-compat notes:
/// - a failed WINDOW shape is appended as `None` (Java `shapeList.add(null)`)
///   and does NOT fail the read — the failure only surfaces at insertion
///   time, when `transform_area_to_board` NPEs on the null hole;
/// - the scope identifier is set to the area name (Java
///   `scanner.setScopeIdentifier`), which only feeds FRLogger warning
///   texts — no Rust counterpart.
pub(crate) fn read_area_scope(
    scanner: &mut Scanner,
    layer_structure: Option<&LayerStructure>,
    skip_window_scopes: bool,
) -> Option<AreaScopeResult> {
    let mut area_name = None;
    let mut clearance_class_name = None;
    let mut shapes: Vec<Option<Shape>> = Vec::new();
    let mut result_ok = true;

    let first_token = scanner.next_token();
    if let Token::Str(name) = first_token
        && !name.is_empty()
    {
        area_name = Some(name.to_string());
    }
    let current_shape = read_scope(scanner, layer_structure);
    if current_shape.is_none() {
        // Java warns "Shape.read_area_scope: could not read shape"
        result_ok = false;
    }
    shapes.push(current_shape);

    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file" and returns null
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            if next_token == Token::Keyword(Keyword::Window) && !skip_window_scopes {
                let hole_shape = read_scope(scanner, layer_structure);
                shapes.push(hole_shape);
                // overread the closing bracket
                if scanner.next_token() != Token::Close {
                    // Java warns "closed bracket expected"
                    return None;
                }
            } else if next_token == Token::Keyword(Keyword::ClearanceClass) {
                clearance_class_name = Some(crate::scope::structure::read_string_scope(scanner));
            } else {
                // skip unknown scope
                skip_scope(scanner);
            }
        }
        prev_token = Some(next_token);
    }
    if !result_ok {
        return None;
    }
    Some(AreaScopeResult {
        area_name,
        clearance_class: clearance_class_name,
        shapes,
    })
}

/// The three ways Java `Shape.transformAreaToBoard` (`Shape.java:522-555`)
/// can fail. The keepout and plane call sites treat the null-return and
/// the in-loop throw DIFFERENTLY, so they must not be folded into one
/// `None`:
///
/// - [`TransformedArea::Area`]: success — including the uncast
///   single-shape `Circle` (`Circle.dimension()` is 2, `Circle.java:36-42`;
///   jar t6-keep-circle inserts it).
/// - [`TransformedArea::Null`]: Java returned null — the empty list
///   (`:527`), a non-`PolylineShape` boundary (`:538-541`), or a hole
///   failing the `instanceof PolylineShape` check (a `Circle` or a null
///   transform, `:546-548`). Keepout site: the null NPEs at
///   `Structure.java:864` — parse dies (jar t6-keep-circle-hole). Plane
///   site: `insertConductionArea(null)` warns log-only and the parse
///   CONTINUES without the plane (`BasicBoard.java:553-556`, jar
///   t6-plane-circwin / t6-plane-mixed-holes).
/// - [`TransformedArea::Threw`]: Java THREW, uncaught, parse-fatal at
///   BOTH call sites — a null hole ENTRY dereferenced at
///   `it.next().transformToBoard` (`:544-545`, jar t6-plane-nullhole /
///   t6-keep-nullhole / t6-plane-mixed2-holes THROWN lines) or a null
///   boundary entry at `boundary.transformToBoard` (`:530-532`).
///
/// The hole loop stops at the FIRST failing hole, so the flavors are
/// ORDER-SENSITIVE: `[circle, null-entry]` is [`TransformedArea::Null`]
/// (jar t6-plane-mixed-holes: Success, plane skipped) while
/// `[null-entry, circle]` throws (jar t6-plane-mixed2-holes) — an
/// order-blind scan cannot replace this walk.
pub(crate) enum TransformedArea {
    Area(AreaIr),
    Null,
    Threw,
}

/// Java `Shape.transformAreaToBoard` (`Shape.java:522-555`): transforms
/// the area's shape list to board coordinates. With no holes the
/// transformed border IS the area; with holes the border must be a Java
/// `PolylineShape` (tiles and `PolygonShape` — NOT a `Circle`) and the
/// result is a `PolylineArea` ([`AreaIr`]). See [`TransformedArea`] for
/// the failure-flavor contract.
pub(crate) fn transform_area_to_board(
    shapes: &[Option<Shape>],
    coordinate_transform: &CoordinateTransform,
) -> TransformedArea {
    if shapes.is_empty() {
        // Java: warn "area.size() > 0 expected" + null
        return TransformedArea::Null;
    }
    let Some(boundary) = shapes[0].as_ref() else {
        // Java: null boundary -> NPE in boundary.transformToBoard
        return TransformedArea::Threw;
    };
    let Some(boundary_shape) = boundary.transform_to_board(coordinate_transform) else {
        // Java: a null boundary TRANSFORM (PolylinePath stub) makes the
        // result null — the null-return flavor, not a throw
        return TransformedArea::Null;
    };
    if shapes.len() == 1 {
        return TransformedArea::Area(AreaIr::simple(boundary_shape));
    }
    // area with holes: the border must be a PolylineShape — a Circle
    // triggers the "PolylineShape expected" warn + null (jar:
    // t6-keep-circle-hole THROWN "keepoutArea is null")
    if matches!(boundary_shape, BoardShape::Circle(_)) {
        return TransformedArea::Null;
    }
    let mut holes = Vec::with_capacity(shapes.len() - 1);
    for hole in &shapes[1..] {
        let Some(shape) = hole else {
            // Java: NPE at `it.next().transformToBoard` (`:544-545`,
            // jar t6-plane-nullhole / t6-keep-nullhole THROWN)
            return TransformedArea::Threw;
        };
        let Some(hole_shape) = shape.transform_to_board(coordinate_transform) else {
            // Java: null transform -> the instanceof check fails
            // ("PolylineShape expected" warn + null)
            return TransformedArea::Null;
        };
        if matches!(hole_shape, BoardShape::Circle(_)) {
            // Java: warn "PolylineShape expected" + null
            return TransformedArea::Null;
        }
        holes.push(hole_shape);
    }
    TransformedArea::Area(AreaIr {
        border: boundary_shape,
        holes,
    })
}

/// The `transformAreaToBoardRel` mirror of [`TransformedArea`] — same
/// three flavors, same call-site split: the image-keepout conversion
/// (`Library.java:382-390`) stores a `TransformedAreaRel::Null` result as
/// a keepout with a NULL area and the parse SURVIVES, while
/// `TransformedAreaRel::Threw` (null WINDOW entry dereferenced at
/// `it.next().transformToBoardRel`) is parse-fatal — the enclosing
/// `Library.readScope` has no try/catch. Structurally the SAME three-state
/// enum as [`TransformedArea`] (the Area/Null/Threw split lives entirely at
/// the call sites), so it is a type alias; the duplicated transform bodies
/// are kept Java-verbatim per flavor.
pub(crate) type TransformedAreaRel = TransformedArea;

/// Java `Shape.transformAreaToBoardRel` (`Shape.java:561-596`): the
/// image-relative twin of [`transform_area_to_board`], transforming every
/// shape with `transformToBoardRel`. Structure is Java-verbatim except
/// the per-shape transform call; note the Java quirk that the HOLE loop's
/// warn still says `transform_area_to_board` (copy-paste upstream,
/// `:585`) while the boundary warn says `transform_area_to_board_rel`
/// (`:572`) — both log-only.
pub(crate) fn transform_area_to_board_rel(
    shapes: &[Option<Shape>],
    coordinate_transform: &CoordinateTransform,
) -> TransformedAreaRel {
    if shapes.is_empty() {
        // Java: warn "area.size() > 0 expected" + null
        return TransformedAreaRel::Null;
    }
    let Some(boundary) = shapes[0].as_ref() else {
        // Java cannot reach this: readAreaScope only returns results whose
        // first shape is non-null (a failed FIRST shape fails the whole
        // read). Handled as the Threw flavor (a Java NPE = parse death).
        return TransformedAreaRel::Threw;
    };
    let Some(boundary_shape) = boundary.transform_to_board_rel(coordinate_transform) else {
        // Java: a null boundary TRANSFORM (PolylinePath stub) fails the
        // instanceof PolylineShape check -> warn + null. Jar t7-image: the
        // keepout list never saw this (rect keepouts), but the OUTLINE
        // path did (OUTLINE 3 null) — same stub.
        return TransformedAreaRel::Null;
    };
    if shapes.len() == 1 {
        return TransformedAreaRel::Area(AreaIr::simple(boundary_shape));
    }
    // area with holes: the border must be a PolylineShape
    if matches!(boundary_shape, BoardShape::Circle(_)) {
        return TransformedAreaRel::Null;
    }
    let mut holes = Vec::with_capacity(shapes.len() - 1);
    for hole in &shapes[1..] {
        let Some(shape) = hole else {
            // Java: NPE at `it.next().transformToBoardRel` — parse death
            // (Library.readScope has no catch).
            return TransformedAreaRel::Threw;
        };
        let Some(hole_shape) = shape.transform_to_board_rel(coordinate_transform) else {
            // Java: null transform -> the instanceof check fails
            return TransformedAreaRel::Null;
        };
        if matches!(hole_shape, BoardShape::Circle(_)) {
            // Java: warn "PolylineShape expected" + null
            return TransformedAreaRel::Null;
        }
        holes.push(hole_shape);
    }
    TransformedAreaRel::Area(AreaIr {
        border: boundary_shape,
        holes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_box::IntBox;

    /// Jar session `/tmp/epic-t3-transforms.jsh` (repo root, JDK 25 jar),
    /// output `/tmp/epic-t3-transforms.out`, transform (10, 3.5, -2.25).
    fn ct() -> CoordinateTransform {
        CoordinateTransform::new(10.0, 3.5, -2.25)
    }

    /// Jar session `/tmp/epic-t3-shape-readers.jsh`: layer structure
    /// `[TopLayer, BottomLayer]`.
    fn two_layers() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("TopLayer", 0, true),
            Layer::new("BottomLayer", 1, true),
        ])
    }

    fn read(input: &str, layers: Option<&LayerStructure>) -> Option<Shape> {
        let mut scanner = Scanner::new(input.as_bytes());
        read_scope(&mut scanner, layers)
    }

    fn as_rectangle(shape: Option<Shape>) -> Rectangle {
        match shape.expect("shape expected") {
            Shape::Rectangle(rectangle) => rectangle,
            other => panic!("expected Rectangle, got {other:?}"),
        }
    }

    fn as_polygon(shape: Option<Shape>) -> Polygon {
        match shape.expect("shape expected") {
            Shape::Polygon(polygon) => polygon,
            other => panic!("expected Polygon, got {other:?}"),
        }
    }

    fn as_circle(shape: Option<Shape>) -> Circle {
        match shape.expect("shape expected") {
            Shape::Circle(circle) => circle,
            other => panic!("expected Circle, got {other:?}"),
        }
    }

    fn as_polygon_path(shape: Option<Shape>) -> PolygonPath {
        match shape.expect("shape expected") {
            Shape::PolygonPath(path) => path,
            other => panic!("expected PolygonPath, got {other:?}"),
        }
    }

    fn as_polyline_path(shape: Option<Shape>) -> PolylinePath {
        match shape.expect("shape expected") {
            Shape::PolylinePath(path) => path,
            other => panic!("expected PolylinePath, got {other:?}"),
        }
    }

    fn as_box(shape: &BoardShape) -> &IntBox {
        match shape {
            BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(r#box))) => r#box,
            other => panic!("expected IntBox, got {other:?}"),
        }
    }

    fn as_octagon(shape: &BoardShape) -> &IntOctagon {
        match shape {
            BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntOctagon(
                octagon,
            ))) => octagon,
            other => panic!("expected IntOctagon, got {other:?}"),
        }
    }

    fn as_board_polygon(shape: &BoardShape) -> &PolygonShape {
        match shape {
            BoardShape::PolygonShape(polygon) => polygon,
            other => panic!("expected PolygonShape, got {other:?}"),
        }
    }

    fn as_board_circle(shape: &BoardShape) -> &BoardCircle {
        match shape {
            BoardShape::Circle(circle) => circle,
            other => panic!("expected Circle, got {other:?}"),
        }
    }

    /// `IntOctagon` fields in the Java print order
    /// leftX, bottomY, rightX, topY, upperLeftDiagonalX,
    /// lowerRightDiagonalX, lowerLeftDiagonalX, upperRightDiagonalX.
    fn oct(octagon: &IntOctagon) -> [i32; 8] {
        [
            octagon.left_x,
            octagon.bottom_y,
            octagon.right_x,
            octagon.top_y,
            octagon.upper_left_diagonal_x,
            octagon.lower_right_diagonal_x,
            octagon.lower_left_diagonal_x,
            octagon.upper_right_diagonal_x,
        ]
    }

    fn int_corners(polygon: &PolygonShape) -> Vec<(i32, i32)> {
        (0..polygon.border_line_count() as i32)
            .map(|i| match polygon.corner(i) {
                Point::Int(point) => (point.x, point.y),
                other => panic!("expected int corner, got {other:?}"),
            })
            .collect()
    }

    // ---- readers: rectangle (jar /tmp/epic-t3-shape-readers.out) -------

    /// R1 "Rectangle layer=signal/-1/true (pcb=false,signal=true)
    /// coor=[0.0, 0.0, 100.0, 100.0]" — the signal pseudo-layer resolves
    /// with a NULL structure, and the value equality to [`Layer::signal`]
    /// stands in for the Java identity check `l == Layer.SIGNAL` (the
    /// Part-A constructors guarantee it). R3/R3b: an UNKNOWN layer falls
    /// back to signal — even with a null structure, because the name match
    /// precedes the structure check in `getLayer` — a port without the
    /// fallback returns None here and fails this test.
    #[test]
    fn rectangle_reader_signal_fallback() {
        let r1 = as_rectangle(read("(rectangle signal 0 0 100 100)", None));
        assert_eq!(r1.layer, Layer::signal());
        assert_ne!(r1.layer, Layer::pcb());
        assert_eq!(r1.coor, [0.0, 0.0, 100.0, 100.0]);

        let r3 = as_rectangle(read("(rectangle NoSuch 1 2 3 4)", None));
        assert_eq!(r3.layer, Layer::signal());
        assert_eq!(r3.coor, [1.0, 2.0, 3.0, 4.0]);

        let r3b = as_rectangle(read("(rectangle NoSuch 1 2 3 4)", Some(&two_layers())));
        assert_eq!(r3b.layer, Layer::signal());
        assert_eq!(r3b.coor, [1.0, 2.0, 3.0, 4.0]);
    }

    /// R2 "Rectangle layer=TopLayer/0/true coor=[-100.0, -50.0, 200.0,
    /// 300.0]" — named layer via the structure; R6 "coor=[0.5, -1.25, 2.75,
    /// 3.5]" — doubles preserved exactly.
    #[test]
    fn rectangle_reader_named_layer_and_doubles() {
        let r2 = as_rectangle(read(
            "(rectangle TopLayer -100 -50 200 300)",
            Some(&two_layers()),
        ));
        assert_eq!(
            r2.layer,
            Layer {
                name: "TopLayer".to_string(),
                no: 0,
                is_signal: true,
                net_names: Vec::new()
            }
        );
        assert_eq!(r2.coor, [-100.0, -50.0, 200.0, 300.0]);

        let r6 = as_rectangle(read("(rectangle signal 0.5 -1.25 2.75 3.5)", None));
        assert_eq!(r6.coor, [0.5, -1.25, 2.75, 3.5]);
    }

    /// R4 "(rectangle signal 0 0 100 100" -> null (the close-bracket
    /// overread fails, Shape.java:284-289); R5 "abc" as third coordinate ->
    /// null (fail-fast inline conversion, Shape.java:274-280).
    #[test]
    fn rectangle_reader_error_forms() {
        assert!(read("(rectangle signal 0 0 100 100", None).is_none());
        assert!(read("(rectangle signal 0 0 abc 100)", None).is_none());
    }

    // ---- readers: circle ------------------------------------------------

    /// C1 "Circle layer=signal/-1/true coor=[10.0, 5.0, 6.0]" (radius, x, y
    /// order); C4 "(circle signal 10 5)" -> "coor=[10.0, 5.0, 0.0]" — the
    /// missing third number stays 0.0.
    #[test]
    fn circle_reader_success_forms() {
        let c1 = as_circle(read("(circle signal 10 5 6)", None));
        assert_eq!(c1.layer, Layer::signal());
        assert_eq!(c1.coor, [10.0, 5.0, 6.0]);

        let c4 = as_circle(read("(circle signal 10 5)", None));
        assert_eq!(c4.coor, [10.0, 5.0, 0.0]);
    }

    /// C2 "(circle signal 10 5 6 7)" -> null (fourth number,
    /// Shape.java:417-423); C3 "(circle NoSuch 10 5 6)" WITH structure ->
    /// null: circles have NO signal fallback (the discriminating counterpart
    /// of `rectangle_reader_signal_fallback`).
    #[test]
    fn circle_reader_error_forms() {
        assert!(read("(circle signal 10 5 6 7)", None).is_none());
        assert!(read("(circle NoSuch 10 5 6)", Some(&two_layers())).is_none());
    }

    // ---- readers: polygon ----------------------------------------------

    /// P1 "Polygon layer=signal coor=[10.0, 20.0, 30.0, 40.0, 50.0]" (width
    /// 0 overread, 5 coords kept); P2 "pcb" keyword identity; P4 "(polygon
    /// signal 0 (window) 1 2 3 4)" -> "coor=[1.0, 2.0, 3.0, 4.0]" — the
    /// embedded scope is skipped INSIDE the coordinate loop (Shape.java:
    /// 357-361); P6 "(polygon signal 0.5 1.5 2.5 3.5)" -> "coor=[1.5, 2.5,
    /// 3.5]" — the width token is a double and is overread.
    #[test]
    fn polygon_reader_success_forms() {
        let p1 = as_polygon(read("(polygon signal 0 10 20 30 40 50)", None));
        assert_eq!(p1.layer, Layer::signal());
        assert_eq!(p1.coor, vec![10.0, 20.0, 30.0, 40.0, 50.0]);

        let p2 = as_polygon(read("(polygon pcb 5 1 2 3 4)", None));
        assert_eq!(p2.layer, Layer::pcb());
        assert_eq!(p2.coor, vec![1.0, 2.0, 3.0, 4.0]);

        let p4 = as_polygon(read("(polygon signal 0 (window) 1 2 3 4)", None));
        assert_eq!(p4.coor, vec![1.0, 2.0, 3.0, 4.0]);

        let p6 = as_polygon(read("(polygon signal 0.5 1.5 2.5 3.5)", None));
        assert_eq!(p6.coor, vec![1.5, 2.5, 3.5]);
    }

    /// P3 "(polygon NoSuch 0 1 2 3 4)" WITH structure -> null (layerOk
    /// false, consumed then failed); P5 EOF -> null (Shape.java:350-356);
    /// P7 "(polygon 42 0 1 2)" with NULL structure -> null — the
    /// structure-null branch fires before the string check (Shape.java:
    /// 314-320), a NUMBER is not a layer string.
    #[test]
    fn polygon_reader_error_forms() {
        assert!(read("(polygon NoSuch 0 1 2 3 4)", Some(&two_layers())).is_none());
        assert!(read("(polygon signal 0 1 2", None).is_none());
        assert!(read("(polygon 42 0 1 2)", None).is_none());
    }

    // ---- readers: polygon path -----------------------------------------

    /// PP2a "(path signal 10 0 0 100)" (width + 3 coords = 4 tokens) -> null
    /// vs PP2b "(path signal 10 0 0 100 100)" (exactly 5) -> valid with
    /// "width=10.0 coor=[0.0, 0.0, 100.0, 100.0]" — the T31 boundary pair,
    /// each side fails/passes where the other agrees (anchor-blind).
    #[test]
    fn polygon_path_reader_t31_token_boundary() {
        assert!(read("(path signal 10 0 0 100)", None).is_none());
        let pp2b = as_polygon_path(read("(path signal 10 0 0 100 100)", None));
        assert_eq!(pp2b.layer, Layer::signal());
        assert_eq!(pp2b.width, 10.0);
        assert_eq!(pp2b.coordinate_arr, vec![0.0, 0.0, 100.0, 100.0]);
    }

    /// PP1 "width=10.0 coor=[0.0, 0.0, 100.0, 100.0, 200.0, 0.0]"; PP4
    /// "(path signal 10.5 ...)" -> "width=10.5" (double width preserved);
    /// PP5 "(path signal 10 (rule 5) 0 0 100 100)" -> valid — the embedded
    /// scope is skipped in the corner loop (Shape.java:458-462).
    #[test]
    fn polygon_path_reader_success_forms() {
        let pp1 = as_polygon_path(read("(path signal 10 0 0 100 100 200 0)", None));
        assert_eq!(pp1.coordinate_arr, vec![0.0, 0.0, 100.0, 100.0, 200.0, 0.0]);

        let pp4 = as_polygon_path(read("(path signal 10.5 0 0 100 100)", None));
        assert_eq!(pp4.width, 10.5);

        let pp5 = as_polygon_path(read("(path signal 10 (rule 5) 0 0 100 100)", None));
        assert_eq!(pp5.coordinate_arr, vec![0.0, 0.0, 100.0, 100.0]);
    }

    /// PP3 "(path NoSuch 10 0 0 100 100)" WITH structure -> null: the size
    /// check runs BEFORE the layer check (Shape.java:470-481), so a 5-token
    /// scope fails on the layer, not the size.
    #[test]
    fn polygon_path_reader_unknown_layer() {
        assert!(read("(path NoSuch 10 0 0 100 100)", Some(&two_layers())).is_none());
    }

    // ---- readers: polyline path ----------------------------------------

    /// PL1 "PolylinePath layer=signal width=10.0 coor=[0.0, 0.0, 100.0,
    /// 0.0, 100.0, 100.0]" (4 values per line); PL3 4 tokens -> null.
    #[test]
    fn polyline_path_reader_success_forms() {
        let pl1 = as_polyline_path(read("(polyline_path signal 10 0 0 100 0 100 100)", None));
        assert_eq!(pl1.layer, Some(Layer::signal()));
        assert_eq!(pl1.width, 10.0);
        assert_eq!(pl1.coordinate_arr, vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0]);

        assert!(read("(polyline_path signal 10 0 0 100)", None).is_none());
    }

    /// PL2 "(polyline_path NoSuch 10 0 0 100 0 100 100)" WITH structure ->
    /// a PolylinePath whose LAYER IS NULL (Shape.java:107-162 has no null
    /// check) — the quirk the `Option<Layer>` field preserves; a wrong port
    /// returning None fails here.
    #[test]
    fn polyline_path_reader_null_layer_quirk() {
        let pl2 = as_polyline_path(read(
            "(polyline_path NoSuch 10 0 0 100 0 100 100)",
            Some(&two_layers()),
        ));
        assert_eq!(pl2.layer, None);
        assert_eq!(pl2.width, 10.0);
    }

    // ---- transforms: rectangle (jar /tmp/epic-t3-transforms.out) --------

    /// T1 "rect.toBoard=IntBox 7,42 .. 1385,443" for
    /// `[142, 42, 4.2, 1.95]`: BOTH min/max swap branches run (x0 > x2 and
    /// y0 > y1), dsnToBoard(4.2).x = 7.000000000000002 rounds to 7, and
    /// 442.5 rounds half-up to 443. T2 "rect.toBoardRel=IntBox 42,20 ..
    /// 1000,424" for `[4.2, 42.4, 100, 1.95]`: the y-flip IntBox ctor order
    /// AND no base subtraction (4.2 * 10 = 42 — a port using the
    /// base-subtracting `dsn_to_board_tuple` yields negative-offset junk
    /// and fails).
    #[test]
    fn rectangle_transforms_swap_flip_and_round() {
        let r1 = Rectangle {
            layer: Layer::signal(),
            coor: [142.0, 42.0, 4.2, 1.95],
        };
        let board = r1.transform_to_board(&ct());
        let r#box = as_box(&board);
        assert_eq!(
            (r#box.ll.x, r#box.ll.y, r#box.ur.x, r#box.ur.y),
            (7, 42, 1385, 443)
        );

        let r2 = Rectangle {
            layer: Layer::signal(),
            coor: [4.2, 42.4, 100.0, 1.95],
        };
        let board_rel = r2.transform_to_board_rel(&ct());
        let box_rel = as_box(&board_rel);
        assert_eq!(
            (box_rel.ll.x, box_rel.ll.y, box_rel.ur.x, box_rel.ur.y),
            (42, 20, 1000, 424)
        );
    }

    // ---- transforms: polygon -------------------------------------------

    /// T3 "poly.toBoard=PolygonShape corners=1,44 101,44 101,144 1,144" for
    /// the DSN square [3.6, 2.15, 13.6, 2.15, 13.6, 12.15, 3.6, 12.15]
    /// ((3.6-3.5)*10 = 1.0000000000000009 -> 1, (2.15+2.25)*10 =
    /// 44.00000000000001 -> 44). T4a "[5]" -> "Simplex dim=-1"
    /// (Simplex.EMPTY); T4b "[5, 5]" -> the single corner (50,50); T4c
    /// "[5, 5, 7, 9]" -> (50,50) (70,90) — base-free rel scaling.
    #[test]
    fn polygon_transforms() {
        let p1 = Polygon {
            layer: Layer::signal(),
            coor: vec![3.6, 2.15, 13.6, 2.15, 13.6, 12.15, 3.6, 12.15],
        };
        assert_eq!(
            int_corners(as_board_polygon(&p1.transform_to_board(&ct()))),
            vec![(1, 44), (101, 44), (101, 144), (1, 144)]
        );

        let p2 = Polygon {
            layer: Layer::signal(),
            coor: vec![5.0],
        };
        assert_eq!(
            p2.transform_to_board_rel(&ct()),
            BoardShape::Tile(TileShape::Simplex(Box::new(Simplex::empty())))
        );

        let p3 = Polygon {
            layer: Layer::signal(),
            coor: vec![5.0, 5.0],
        };
        assert_eq!(
            int_corners(as_board_polygon(&p3.transform_to_board_rel(&ct()))),
            vec![(50, 50)]
        );

        let p4 = Polygon {
            layer: Layer::signal(),
            coor: vec![5.0, 5.0, 7.0, 9.0],
        };
        assert_eq!(
            int_corners(as_board_polygon(&p4.transform_to_board_rel(&ct()))),
            vec![(50, 50), (70, 90)]
        );
    }

    // ---- transforms: circle --------------------------------------------

    /// T5 "circ.toBoard=Circle 7,42 r=50" — radius (int)
    /// Math.round(dsnToBoard(10) / 2) = 50 (halving AFTER scaling), center
    /// dsnToBoard([4.2, 1.95]).round() = (7,42); T5r "circ.toBoardRel=
    /// Circle 42,20 r=50".
    #[test]
    fn circle_transforms() {
        let c1 = Circle {
            layer: Layer::signal(),
            coor: [10.0, 4.2, 1.95],
        };
        let board_shape = c1.transform_to_board(&ct());
        let board = as_board_circle(&board_shape);
        assert_eq!((board.center.x, board.center.y, board.radius), (7, 42, 50));

        let rel_shape = c1.transform_to_board_rel(&ct());
        let rel = as_board_circle(&rel_shape);
        assert_eq!((rel.center.x, rel.center.y, rel.radius), (42, 20, 50));
    }

    /// T5b (Task 3 review carryover): ODD diameter 15 at scale 10 — the
    /// halving ORDER becomes observable. Java halves AFTER scaling:
    /// `(int) Math.round(dsnToBoard(15) / 2)` = round(150 / 2) = 75. A port
    /// that halves (with rounding) in DSN space first computes
    /// `Math.round(15 / 2)` = 8 and yields 80 — the T5 diameter-10 pin
    /// cannot see that swap (both orders give 50). Jar session
    /// `/tmp/epic-t3-part-b.jsh`, output `/tmp/epic-t3-part-b.out`:
    /// "T5b circ15.toBoard=Circle: center (7,42)radius 75" and
    /// "T5b circ15.toBoardRel=Circle: center (42,20)radius 75" (and the
    /// named-layer form "T5b circ15Top.toBoard=... radius 75" — the base
    /// plays no role in the radius). T5c: non-integer odd diameter 1.5 →
    /// "T5c circ1p5.toBoard=... radius 8" — round(dsnToBoard(1.5)/2) =
    /// round(7.5) = 8, the Math.round half-up boundary on the radius path.
    #[test]
    fn circle_radius_halves_after_scaling_odd_diameter() {
        let c15 = Circle {
            layer: Layer::signal(),
            coor: [15.0, 4.2, 1.95],
        };
        let board_shape = c15.transform_to_board(&ct());
        let board = as_board_circle(&board_shape);
        assert_eq!((board.center.x, board.center.y, board.radius), (7, 42, 75));

        let rel_shape = c15.transform_to_board_rel(&ct());
        let rel = as_board_circle(&rel_shape);
        assert_eq!((rel.center.x, rel.center.y, rel.radius), (42, 20, 75));

        // The halve-in-DSN-space wrong form differs exactly here: 80, not 75.
        assert_ne!(75, 10 * java_round(15.0 / 2.0) as i32);

        let c1p5 = Circle {
            layer: Layer::signal(),
            coor: [1.5, 0.0, 0.0],
        };
        let board_shape = c1p5.transform_to_board(&ct());
        let board = as_board_circle(&board_shape);
        assert_eq!(board.radius, 8);
    }

    // ---- transforms: polygon path ---------------------------------------

    /// T6 "pp.toBoard.le2=IntOctagon -19,24,121,164,-71,-15,17,273" for 2
    /// corners [3.6, 2.15, 13.6, 12.15] and width 4 (offset 2): the <=2
    /// branch — bounding octagon + enlarge, NOT a PolygonShape. T9
    /// "pp.toBoardRel.le2=IntOctagon 16,1,156,142,-14,43,29,286".
    #[test]
    fn polygon_path_transform_le2_corners_is_enlarged_octagon() {
        let pp = PolygonPath {
            layer: Layer::signal(),
            width: 4.0,
            coordinate_arr: vec![3.6, 2.15, 13.6, 12.15],
        };
        assert_eq!(
            oct(as_octagon(&pp.transform_to_board(&ct()))),
            [-19, 24, 121, 164, -71, -15, 17, 273]
        );
        assert_eq!(
            oct(as_octagon(&pp.transform_to_board_rel(&ct()))),
            [16, 1, 156, 142, -14, 43, 29, 286]
        );
    }

    /// T7 "pp.toBoard.gt2pos=IntOctagon -19,24,121,164,-71,85,17,273" for 3
    /// corners and offset 2: boundingTile().enlarge(offset). T8
    /// "pp.toBoard.gt2zero=PolygonShape corners=1,44 101,44 101,144 1,144"
    /// for width 0: the offset>0 guard is FALSE so the PolygonShape itself
    /// comes back — a port enlarging unconditionally fails here. T10
    /// "pp.toBoardRel.gt2=IntOctagon 16,2,156,142,-14,142,30,286".
    #[test]
    fn polygon_path_transform_gt2_corners_offset_guard() {
        let pp_pos = PolygonPath {
            layer: Layer::signal(),
            width: 4.0,
            coordinate_arr: vec![3.6, 2.15, 13.6, 2.15, 13.6, 12.15],
        };
        assert_eq!(
            oct(as_octagon(&pp_pos.transform_to_board(&ct()))),
            [-19, 24, 121, 164, -71, 85, 17, 273]
        );
        assert_eq!(
            oct(as_octagon(&pp_pos.transform_to_board_rel(&ct()))),
            [16, 2, 156, 142, -14, 142, 30, 286]
        );

        let pp_zero = PolygonPath {
            layer: Layer::signal(),
            width: 0.0,
            coordinate_arr: vec![3.6, 2.15, 13.6, 2.15, 13.6, 12.15],
        };
        assert_eq!(
            int_corners(as_board_polygon(&pp_zero.transform_to_board(&ct()))),
            vec![(1, 44), (101, 44), (101, 144)]
        );
    }

    /// T11 "plp.toBoard=null toBoardRel=null" — the PolylinePath transforms
    /// are warn-null stubs (PolylinePath.java:56-66); the Shape dispatch
    /// maps them to None.
    #[test]
    fn polyline_path_transforms_are_null_stubs() {
        let plp = PolylinePath {
            layer: Some(Layer::signal()),
            width: 4.0,
            coordinate_arr: vec![0.0, 0.0, 100.0, 100.0],
        };
        assert!(
            Shape::PolylinePath(plp.clone())
                .transform_to_board(&ct())
                .is_none()
        );
        assert!(
            Shape::PolylinePath(plp)
                .transform_to_board_rel(&ct())
                .is_none()
        );
    }

    // ---- bounding boxes -------------------------------------------------

    /// T44 THE paren-bug pin: "T44 pp.bbox=[3.0, -2.0, 14.0, 5.0]" for
    /// width 4 (offset 2) and corners [10, 0], [5, 3]. Trace x-max:
    /// max(MIN,10)+2 = 12, then max(12,5)+2 = 14 — the correct-inside form
    /// (max(10,5)+2 = 12) differs, so a "fixed" port fails. y-max (offset
    /// INSIDE): max(0+2)=2 then max(2,3+2)=5. The single-point anchor T44b
    /// "[8.0, -2.0, 12.0, 2.0]" is the form where both orderings AGREE
    /// (x-max = 10+2 either way).
    #[test]
    fn polygon_path_bounding_box_t44_paren_bug() {
        let bb1 = PolygonPath {
            layer: Layer::signal(),
            width: 4.0,
            coordinate_arr: vec![10.0, 0.0, 5.0, 3.0],
        };
        assert_eq!(bb1.bounding_box().coor, [3.0, -2.0, 14.0, 5.0]);

        let bb2 = PolygonPath {
            layer: Layer::signal(),
            width: 4.0,
            coordinate_arr: vec![10.0, 0.0],
        };
        assert_eq!(bb2.bounding_box().coor, [8.0, -2.0, 12.0, 2.0]);
    }

    /// T44c "poly.bbox=[5.0, 0.0, 10.0, 3.0]" — Polygon has NO offset
    /// (discriminates against copying the path's bbox); T44d
    /// "circ.bbox=[-5.8, -8.05, 14.2, 11.95]" (center +/- diameter);
    /// T44e "rect.identity=[0.0, 0.0, 10.0, 10.0] union=[0.0, 0.0, 30.0,
    /// 10.0]".
    #[test]
    fn polygon_circle_rectangle_bounding_boxes() {
        let poly = Polygon {
            layer: Layer::signal(),
            coor: vec![10.0, 0.0, 5.0, 3.0],
        };
        assert_eq!(poly.bounding_box().coor, [5.0, 0.0, 10.0, 3.0]);

        let circle = Circle {
            layer: Layer::signal(),
            coor: [10.0, 4.2, 1.95],
        };
        assert_eq!(circle.bounding_box().coor, [-5.8, -8.05, 14.2, 11.95]);

        let rect = Rectangle {
            layer: Layer::signal(),
            coor: [0.0, 0.0, 10.0, 10.0],
        };
        assert_eq!(rect.bounding_box().coor, [0.0, 0.0, 10.0, 10.0]);
        let other = Rectangle {
            layer: Layer::pcb(),
            coor: [20.0, 5.0, 30.0, 6.0],
        };
        // union keeps THIS layer (signal), not the other's
        let united = rect.union(&other);
        assert_eq!(united.coor, [0.0, 0.0, 30.0, 10.0]);
        assert_eq!(united.layer, Layer::signal());
    }

    /// The Shape-level dispatch: bounding_box/bBox of a PolylinePath is the
    /// null stub, every other leaf yields a Rectangle.
    #[test]
    fn shape_bounding_box_dispatch() {
        let shape = Shape::PolylinePath(PolylinePath {
            layer: Some(Layer::signal()),
            width: 4.0,
            coordinate_arr: vec![0.0, 0.0, 100.0, 100.0],
        });
        assert!(shape.bounding_box().is_none());

        let shape = Shape::Rectangle(Rectangle {
            layer: Layer::signal(),
            coor: [1.0, 2.0, 3.0, 4.0],
        });
        assert_eq!(
            shape.bounding_box().expect("rect bbox").coor,
            [1.0, 2.0, 3.0, 4.0]
        );
    }

    // ---- unknown shape keyword -----------------------------------------

    /// `read_scope` dispatches on the five keywords; anything else
    /// skip-scopes the rest and yields None (Shape.java:67-68). `(layer
    /// Top)` is not a shape scope.
    #[test]
    fn unknown_scope_is_skipped() {
        assert!(read("(layer TopLayer)", Some(&two_layers())).is_none());
        assert!(read("(window 1 2 3)", None).is_none());
    }

    // ---- Task 6: read_area_scope / transform_area_to_board -------------

    /// `read_area_scope` null arms (`Shape.java:169-251`): EOF inside the
    /// scope, and a failed FIRST shape (resultOk = false -> null at the
    /// end even though the rest of the scope drained fine).
    #[test]
    fn area_scope_null_forms() {
        // EOF: the rectangle never closes
        let mut scanner = Scanner::new(b"(rectangle signal 0 0 10 10");
        assert!(read_area_scope(&mut scanner, None, false).is_none());

        // unknown first shape: consumed via skip, then nulled
        let mut scanner = Scanner::new(b"(nosuch 1))");
        assert!(read_area_scope(&mut scanner, None, false).is_none());
    }

    /// `read_area_scope` capture arms: the optional NAME (empty string is
    /// Java null -> None), the `(clearance_class ...)` value and the
    /// window as a second (hole) entry.
    #[test]
    fn area_scope_captures_name_class_and_window() {
        let mut scanner = Scanner::new(
            b"zone (rectangle signal 0 0 10 10) (window (rectangle signal 1 1 2 2)) (clearance_class power))",
        );
        let area = read_area_scope(&mut scanner, None, false).expect("area");
        assert_eq!(area.area_name.as_deref(), Some("zone"));
        assert_eq!(area.clearance_class.as_deref(), Some("power"));
        assert_eq!(area.shapes.len(), 2);
        assert!(area.shapes[0].is_some());
        assert!(area.shapes[1].is_some());

        // an empty name string is treated as Java null (Shape.java:180-184)
        let mut scanner = Scanner::new(b"\"\" (rectangle signal 0 0 10 10))");
        let area = read_area_scope(&mut scanner, None, false).expect("area");
        assert_eq!(area.area_name, None);
    }

    /// `transform_area_to_board` (`Shape.java:522-555`): the empty-list
    /// arm, the flavor split between null-return and in-loop throw, the
    /// single-shape passthrough (a Circle survives — no PolylineArea
    /// constructor involved) and the order-sensitivity of the hole loop.
    #[test]
    fn transform_area_to_board_null_and_circle_arms() {
        use TransformedArea::{Area, Null, Threw};

        // empty list: the holeCount <= -1 warn-null arm
        assert!(matches!(transform_area_to_board(&[], &ct()), Null));

        // null boundary ENTRY -> the iterator NPE flavor, not a null
        // return (Shape.java:530-532)
        assert!(matches!(transform_area_to_board(&[None], &ct()), Threw));

        // single circle passes through untouched
        let circle = Shape::Circle(Circle {
            layer: Layer::signal(),
            coor: [300.0, 5000.0, 4000.0],
        });
        let Area(area) = transform_area_to_board(&[Some(circle.clone())], &ct()) else {
            panic!("single shape must pass through");
        };
        assert!(area.holes.is_empty());
        assert!(matches!(area.border, BoardShape::Circle(_)));

        // circle border + window: the instanceof check fails -> null
        // return (jar t6-keep-circle-hole / t6-plane-circwin
        // "PolylineShape expected")
        let window = Shape::Rectangle(Rectangle {
            layer: Layer::signal(),
            coor: [4900.0, 3900.0, 5100.0, 4100.0],
        });
        assert!(matches!(
            transform_area_to_board(&[Some(circle), Some(window)], &ct()),
            Null
        ));
    }

    /// The hole flavors split on ENTRY null vs transform failure, and the
    /// loop stops at the FIRST failing hole (jar `/tmp/epic-t6-nullhole.
    /// out`: circle-then-bad survives as the null-return flavor while
    /// bad-then-circle NPEs — Shape.java:543-548).
    #[test]
    fn transform_area_to_board_hole_flavors_are_order_sensitive() {
        use TransformedArea::{Null, Threw};

        let border = Shape::Rectangle(Rectangle {
            layer: Layer::signal(),
            coor: [1000.0, 1000.0, 5000.0, 5000.0],
        });
        let circle_hole = Shape::Circle(Circle {
            layer: Layer::signal(),
            coor: [100.0, 2000.0, 2000.0],
        });

        // null hole ENTRY -> the NPE flavor at it.next().transformToBoard
        assert!(matches!(
            transform_area_to_board(&[Some(border.clone()), None], &ct()),
            Threw
        ));

        // circle hole BEFORE the null entry: the instanceof failure at
        // hole 0 returns null first — the null-entry hole is never reached
        assert!(matches!(
            transform_area_to_board(
                &[Some(border.clone()), Some(circle_hole.clone()), None],
                &ct()
            ),
            Null
        ));

        // null entry BEFORE the circle hole: the NPE at hole 0 wins
        assert!(matches!(
            transform_area_to_board(&[Some(border), None, Some(circle_hole)], &ct()),
            Threw
        ));
    }
}
