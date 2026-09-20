//! The item model scaffold: kind dispatch, per-kind payloads, and the
//! epic-board mirrors of the shape/area/fixed-state values items carry.
//!
//! Java anchors: `board/model/items/Item.java` (fields :38-67,
//! `getBoardItemType` :117-146 — the kind dispatch order below mirrors
//! the instanceof chain), `board/model/items/BoardItemType.java` (the
//! TEN-variant enum, declaration order mirrored exactly — ordinal order
//! is treated as load-bearing, T70), and the concrete item classes
//! (`PolylineTrace.java:42` `lines`, `Trace` `halfWidth`/`layer`,
//! `Via`, `Pin`, `ObstacleArea`, `ConductionArea`,
//! `ComponentOutline.java:27-37`, `structure/BoardOutline.java:29-48`).
//!
//! M2 trap T70: the three obstacle kinds stay DISTINCT
//! (`OBSTACLE_AREA` / `VIA_OBSTACLE_AREA` / `COMPONENT_OBSTACLE_AREA`).
//! Java models them as three `ObstacleArea` subclasses chosen by the
//! readers (`Structure.java:926-933`: plain keepout -> `ObstacleArea`,
//! `via_keepout` -> `ViaObstacleArea`, `place_keepout` ->
//! `ComponentObstacleArea`; package keepouts `Network.java:1080-1149`
//! the same way). The port keeps one [`ItemData::ObstacleArea`] variant
//! carrying an [`ObstacleKind`] discriminator — the KIND ENUM still has
//! all three variants, so no kind is erased.
//!
//! The per-kind geometry surface (M2 Task 4) lives in the submodules:
//! [`trace`] (the `PolylineTraceGeometry` accessors), [`drill`] (the
//! `DrillItem` precalculated spans), [`pin`] (the Pin item wrapping
//! Task 3's placement resolution), [`obstacle`] (the T49 absolute-area
//! transform chain + `ConductionArea`), and [`outline`]
//! (`ComponentOutline` + `BoardOutline` keepout derivation).

pub mod drill;
pub mod obstacle;
pub mod outline;
pub mod pin;
pub mod trace;

use epic_geometry::circle::Circle;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polygon_shape::PolygonShape;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;

/// Java `BoardItemType` (`board/model/items/BoardItemType.java`) —
/// declaration order mirrored exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoardItemType {
    /// Java `TRACE`.
    Trace,
    /// Java `PIN`.
    Pin,
    /// Java `VIA`.
    Via,
    /// Java `OBSTACLE_AREA`.
    ObstacleArea,
    /// Java `VIA_OBSTACLE_AREA`.
    ViaObstacleArea,
    /// Java `CONDUCTION_AREA`.
    ConductionArea,
    /// Java `COMPONENT_OBSTACLE_AREA`.
    ComponentObstacleArea,
    /// Java `BOARD_OUTLINE`.
    BoardOutline,
    /// Java `COMPONENT_OUTLINE`.
    ComponentOutline,
    /// Java `OTHER` — the fallback for item classes outside the nine
    /// named kinds; no parse-time item maps to it.
    Other,
}

/// Which of the three Java `ObstacleArea` subclasses an
/// [`ItemData::ObstacleArea`] stands for (module docs, T70).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObstacleKind {
    /// Java `ObstacleArea` — a plain `(keepout ...)` / structure /
    /// outline-hole / package keepout area.
    ObstacleArea,
    /// Java `ViaObstacleArea` — a `(via_keepout ...)` / package via
    /// keepout (blocks vias, not traces).
    ViaObstacleArea,
    /// Java `ComponentObstacleArea` — a `(place_keepout ...)` /
    /// package place keepout (blocks component placement).
    ComponentObstacleArea,
}

impl ObstacleKind {
    /// The Java `getBoardItemType()` result for the subclass.
    #[must_use]
    pub fn board_item_type(self) -> BoardItemType {
        match self {
            ObstacleKind::ObstacleArea => BoardItemType::ObstacleArea,
            ObstacleKind::ViaObstacleArea => BoardItemType::ViaObstacleArea,
            ObstacleKind::ComponentObstacleArea => BoardItemType::ComponentObstacleArea,
        }
    }
}

/// Java `FixedState` (`board/model/structure/FixedState.java`): variant
/// order mirrors the Java ordinal order (UNFIXED, SHOVE_FIXED,
/// USER_FIXED, SYSTEM_FIXED) — order is load-bearing (T36, the
/// closed-trace drop guard compares `ordinal() < USER_FIXED`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixedState {
    /// Java `UNFIXED`.
    Unfixed,
    /// Java `SHOVE_FIXED` (set by the router).
    ShoveFixed,
    /// Java `USER_FIXED` — `(type route)` wires, promoted planes.
    UserFixed,
    /// Java `SYSTEM_FIXED` — `(type fix)` wires, keepouts, outlines.
    SystemFixed,
}

/// The concrete `geometry.planar.Shape` subclasses board areas are built
/// from — epic-board's own mirror of the parse IR's `BoardShape` (the
/// crates' types stay separate; the conversion lives in
/// [`crate::board::Board::from_ses_board`]). Variants and names mirror
/// `epic_dsn::shape::BoardShape` 1:1 (`TileShape` = Java's virtual
/// `TileShape` family IntBox/IntOctagon/Simplex, `PolygonShape`,
/// `Circle`).
#[derive(Clone, Debug, PartialEq)]
pub enum BoardShape {
    /// `IntBox` / `IntOctagon` / `Simplex` (Java virtual `TileShape`).
    Tile(TileShape),
    /// Java `geometry.planar.PolygonShape`.
    PolygonShape(PolygonShape),
    /// Java `geometry.planar.Circle`.
    Circle(Circle),
}

/// Java `ConvexShape.maxWidth()` dispatched over the board shape kinds:
/// the tile family carries its own `maxWidth`, a polygon's is its
/// bounding box's, a circle's twice its radius. The shared epic-board
/// home — consumers include epic-router's `control.rs` (via-cost
/// arithmetic) and the drill subsystem's `default_via_diameter` /
/// via-radius arithmetic. (`TileShape`/`Circle` implement maxWidth
/// natively; `PolygonShape` is NOT a Java `ConvexShape` — padstack
/// shapes never carry it, the parse converts — so the bounding-box
/// fallback is defensive only.)
#[must_use]
pub fn shape_max_width(shape: &BoardShape) -> f64 {
    match shape {
        BoardShape::Tile(tile) => tile.max_width(),
        BoardShape::PolygonShape(polygon) => polygon.bounding_box().max_width(),
        BoardShape::Circle(circle) => circle.max_width(),
    }
}

/// The `border + holes` area epic-board items carry — the epic-board
/// mirror of the IR's `AreaIr` (the `(window ...)` holes are the rest of
/// a Java `PolylineArea`; `holes: []` is a hole-free area).
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    /// The border shape (board coordinates).
    pub border: BoardShape,
    /// The `(window ...)` holes (board coordinates), in file order.
    pub holes: Vec<BoardShape>,
}

impl Area {
    /// A hole-free area (the IR's `AreaIr::simple`).
    #[must_use]
    pub fn simple(border: BoardShape) -> Self {
        Self {
            border,
            holes: Vec::new(),
        }
    }

    /// Java `Area.contains(Point)` — the `ConductionArea` acceptance
    /// test of the contacts seam (`Trace.getNormalContacts`,
    /// `Trace.java:196-197`). Two shapes of "area" meet here:
    ///
    /// * a HOLE-FREE area is the boundary shape ITSELF (Java `Shape
    ///   extends Area`; the parser returns `boundaryShape` directly
    ///   when `holeCount == 0`, `io/specctra/parser/Shape.java:537-541`),
    ///   so containment is the border's own `contains` — BORDER
    ///   POINTS COUNT AS CONTAINED;
    /// * a HOLED area is a `PolylineArea` (`PolylineArea.java:92-101`):
    ///   the border must contain the point AND no hole may
    ///   CONTAIN-INSIDE it — a point ON a hole's border stays IN the
    ///   area (`containsInside`, not `contains`, on the holes).
    #[must_use]
    pub fn contains_point(&self, point: &Point) -> bool {
        let border_contains = match &self.border {
            BoardShape::Tile(tile) => tile.contains_point(point),
            BoardShape::PolygonShape(polygon) => polygon.contains_point(point),
            BoardShape::Circle(circle) => circle.contains_point(point),
        };
        if !border_contains {
            return false;
        }
        self.holes.iter().all(|hole| !match hole {
            BoardShape::Tile(tile) => tile.contains_inside(point),
            BoardShape::PolygonShape(polygon) => polygon.contains_inside(point),
            BoardShape::Circle(circle) => circle.contains_inside(point),
        })
    }

    /// Java `PolylineArea.cornerApproxArr()` — the border's corners,
    /// then each hole's (`PolylineArea.java:135`). A hole-free area
    /// (the parser returns the boundary shape itself) is exactly the
    /// border's array. The `ConductionArea` ratsnest consumer rounds
    /// each corner to a grid point (`ConductionArea.java:370-375`).
    #[must_use]
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        let mut result = self.border.corner_approx_arr();
        for hole in &self.holes {
            result.extend(hole.corner_approx_arr());
        }
        result
    }
}

impl BoardShape {
    /// Java `Shape.boundingBox()` over the shape arms — the one
    /// geometry accessor every kind shares (`DrillItem.java:380`'s
    /// width/height reads, the outline keepout bounds checks).
    #[must_use]
    pub fn bounding_box(&self) -> IntBox {
        match self {
            BoardShape::Tile(tile) => tile.bounding_box(),
            BoardShape::PolygonShape(polygon) => polygon.bounding_box(),
            BoardShape::Circle(circle) => circle.bounding_box(),
        }
    }

    /// Java `Shape.boundingOctagon()` over the arms (M2 Task 6: the
    /// FORTYFIVE drill dispatch and the 45-degree tree insert hull).
    /// `IntBox.boundingOctagon` = `toIntOctagon()` (cut-corner octagon,
    /// `IntBox.java:252-254`); `None` is Java's null (an unbounded
    /// Simplex only — never reachable from a padstack shape).
    #[must_use]
    pub fn bounding_octagon(&self) -> Option<TileShape> {
        let octagon = match self {
            BoardShape::Tile(tile) => return tile.bounding_octagon().map(octagon_tile),
            BoardShape::PolygonShape(polygon) => polygon.bounding_octagon(),
            BoardShape::Circle(circle) => circle.bounding_octagon(),
        };
        Some(octagon_tile(octagon))
    }

    /// Java `Shape.boundingTile()` over the arms (the no-restriction
    /// drill dispatch arm).
    #[must_use]
    pub fn bounding_tile(&self) -> TileShape {
        match self {
            BoardShape::Tile(tile) => tile.bounding_tile(),
            BoardShape::PolygonShape(polygon) => polygon.bounding_tile(),
            BoardShape::Circle(circle) => circle.bounding_tile(),
        }
    }

    /// Java `Shape.translateBy(Vector)` over the arms — every shape
    /// kind shares the virtual (the forced-via inserter moves padstack
    /// shapes from origin-relative to location-absolute coordinates,
    /// `ForcedViaInserter.java:156`).
    #[must_use]
    pub fn translate_by(&self, vector: &Vector) -> BoardShape {
        match self {
            BoardShape::Tile(tile) => BoardShape::Tile(tile.translate_by(vector)),
            BoardShape::PolygonShape(polygon) => {
                BoardShape::PolygonShape(polygon.translate_by(vector))
            }
            BoardShape::Circle(circle) => BoardShape::Circle(circle.translate_by(vector)),
        }
    }

    /// Java `Shape.borderDistance(FloatPoint)` over the arms — the
    /// copper-radius input of `drillHoleClearanceDelta`
    /// (`ShapeSearchTree.java:1058`).
    ///
    /// QUIRK PORTED VERBATIM: `PolygonShape.borderDistance` is NOT
    /// IMPLEMENTED in Java — it logs "not yet implemented" and returns
    /// 0 (`PolygonShape.java:184-187`), which the delta caller then
    /// treats as `copperRadius <= 0` and re-reads the raw padstack
    /// shape. `PolygonShape` is not a `ConvexShape`, so a PADSTACK
    /// shape can never be one (the parse census confirms: pad shapes
    /// arrive as Circle/IntBox/IntOctagon only) — the arm exists for
    /// the OTHER item kinds' shapes (obstacle areas, Task 7).
    #[must_use]
    pub fn border_distance(&self, point: &FloatPoint) -> f64 {
        match self {
            BoardShape::Tile(tile) => tile.border_distance(point),
            BoardShape::PolygonShape(_polygon) => 0.0,
            BoardShape::Circle(circle) => circle.border_distance(point),
        }
    }

    /// Java `Shape.cornerApproxArr()` over the arms — the
    /// `ConductionArea` ratsnest input (`ConductionArea.java:370`
    /// calls it on `getArea()`, then rounds each corner). Circle
    /// returns EMPTY (Java `Circle.cornerApproxArr`: circles have no
    /// corners) — a circular conduction area contributes ZERO
    /// Delaunay corners, the same disconnect mechanism as the
    /// both-ends-contacted trace.
    #[must_use]
    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        match self {
            BoardShape::Tile(tile) => tile.corner_approx_arr(),
            BoardShape::PolygonShape(polygon) => polygon.corner_approx_arr(),
            BoardShape::Circle(_circle) => Vec::new(),
        }
    }
}

/// Wraps an [`IntOctagon`] as the [`TileShape`] the Java
/// `boundingOctagon()` sites assign into their `TileShape` locals.
fn octagon_tile(octagon: epic_geometry::int_octagon::IntOctagon) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntOctagon(octagon))
}

/// The per-kind payload of a board item — one variant per
/// [`BoardItemType`] kind (T70: the obstacle kinds share a variant but
/// keep the [`ObstacleKind`] discriminator), carrying that kind's
/// primary fields. The item-common fields (nets, clearance class,
/// component id, fixed state, on-the-board flag) live on
/// [`crate::board::ItemEntry`], mirroring the Java `Item` base class vs
/// subclass split (`Item.java:38-67`). The per-kind geometry surface
/// (M2 Task 4, delivered) lives in the submodules — module docs above.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemData {
    /// Java `PolylineTrace` (`trace/PolylineTrace.java`): the canonical
    /// corner polyline (`lines`), the 0-based layer, and the half width.
    /// The `Polyline` is built by the caller from the corner list —
    /// see [`crate::board::Board::from_ses_board`] for the exact
    /// constructor contract.
    Trace {
        /// Java `Trace.get_layer()` (0-based).
        layer: i32,
        /// Java `Trace.get_half_width()`.
        half_width: i32,
        /// Java `PolylineTrace.lines`.
        lines: Polyline,
    },
    /// Java `Pin` (`model/items/Pin.java`): a component pin. The center
    /// is DERIVED from the component placement (`Pin.getCenter`) and is
    /// not stored — placement resolution is delivered (M2 Task 3):
    /// [`crate::components::pin_shape`] / [`crate::components::pin_center`]
    /// and the item-side wrappers of [`pin`].
    Pin {
        /// Java `Pin.pin_index` (0-based position in the package image).
        pin_index: i32,
        /// The resolved padstack number (1-based).
        padstack_no: i32,
    },
    /// Java `Via` (`model/items/Via.java`): a drill item with its own
    /// stored center. Padstack-derived spans (first/last layer, min
    /// width) and the per-layer shape are delivered (M2 Task 4) in
    /// [`drill`]; the item-side wrappers live on
    /// [`crate::board::Board`] (`via_shape`/`drill_*`).
    Via {
        /// Java `DrillItem.center`.
        center: IntPoint,
        /// The resolved padstack number (1-based).
        padstack_no: i32,
        /// Java `Via.attach_allowed`.
        attach_smd_allowed: bool,
    },
    /// Java `ObstacleArea` and its two subclasses (module docs, T70):
    /// the image-relative area plus the placement transform fields
    /// applied LAZILY (`ObstacleArea.getArea`, `ObstacleArea.java:119-144`;
    /// the M1b T49 transform chain is the reference).
    ObstacleArea {
        /// Which `ObstacleArea` subclass this stands for.
        kind: ObstacleKind,
        /// 0-based layer.
        layer: i32,
        /// Java `ObstacleArea.relativeArea` (border + window holes).
        area: Area,
        /// Java `ObstacleArea.translation`.
        translation: IntPoint,
        /// Java `ObstacleArea.rotationInDegree` — the RAW (un-normalized)
        /// placement rotation; see the `KeepoutIr.rotation` docs for why
        /// normalizing here would diverge `getArea()`.
        rotation: f64,
        /// Java `ObstacleArea.side_changed`.
        side_changed: bool,
        /// Java `ObstacleArea.name` — `Some` only for package keepouts
        /// whose image keepout carried a name.
        name: Option<String>,
    },
    /// Java `ConductionArea` (`model/items/ConductionArea.java` — an
    /// `ObstacleArea` subclass with two extra flags): a plane or
    /// rectangle wire. Parse-time inserts always pass `isObstacle=false`
    /// (`Structure.java:1108`, `:562`, `Wiring.java:485`) and the ctor
    /// default `isFilled=true` (`ConductionArea.java:30`) — the flags
    /// exist for the router to flip (`setIsObstacle`/`setIsFilled`).
    ConductionArea {
        /// 0-based layer.
        layer: i32,
        /// Java `ConductionArea.relativeArea` (border + window holes);
        /// the parse inserts with translation ZERO / rotation 0 / no
        /// side change (`BasicBoard.insertConductionArea:558-562`).
        area: Area,
        /// Java `ConductionArea.isObstacle` (`:29`, `getIsObstacle`
        /// `:388`) — false at parse; when false, foreign-net traces and
        /// vias may route through the area.
        is_obstacle: bool,
        /// Java `ConductionArea.isFilled` (`:30`) — true at parse; a
        /// false value turns the area into a routing border only.
        is_filled: bool,
    },
    /// Java `ComponentOutline` (`model/items/ComponentOutline.java:27-37`):
    /// a package outline item (inserted when a package outline has more
    /// than one shape). Burns an id, mirrors Java field-for-field.
    ComponentOutline {
        /// 0-based layer (carried from the IR; Java derives placement
        /// geometry from the area itself).
        layer: i32,
        /// Java `ComponentOutline.relativeArea`.
        area: Area,
        /// Java `ComponentOutline.translation`.
        translation: IntPoint,
        /// Java `ComponentOutline.rotationInDegree` (raw, like
        /// [`ItemData::ObstacleArea`]).
        rotation: f64,
        /// Java `ComponentOutline.isFront`.
        is_front: bool,
        /// Java `ComponentOutline.isCourtyard`.
        is_courtyard: bool,
        /// Java `ComponentOutline.isFabrication`.
        is_fabrication: bool,
        /// Java `ComponentOutline.isClosed`.
        is_closed: bool,
    },
    /// Java `BoardOutline` (`model/structure/BoardOutline.java:29-48`):
    /// the id-1 item of every parse. The keepout derivations live in
    /// [`outline`] (keepout area/lines, `HALF_WIDTH = 100`).
    BoardOutline {
        /// Java `BoardOutline.shapes` — the outline's inner shapes.
        shapes: Vec<BoardShape>,
        /// Java `BoardOutline.keepoutOutsideOutline` (`:44`) — false at
        /// parse (`generateKeepoutOutside` is a GUI/router action);
        /// while false the outline's tree keepout is the LINE shapes
        /// inflated by [`outline::HALF_WIDTH`], not the outside area.
        keepout_outside_outline: bool,
    },
    /// Java `BoardItemType.OTHER` — no parse-time item produces this;
    /// present so the kind enum round-trips every Java kind.
    Other,
}

impl ItemData {
    /// Java `Item.getBoardItemType()` (`Item.java:117-146`): the
    /// instanceof chain mirrored. All TEN kinds are reachable (T70 —
    /// the three obstacle kinds via [`ObstacleKind`]).
    #[must_use]
    pub fn board_item_type(&self) -> BoardItemType {
        match self {
            ItemData::Trace { .. } => BoardItemType::Trace,
            ItemData::Pin { .. } => BoardItemType::Pin,
            ItemData::Via { .. } => BoardItemType::Via,
            ItemData::ObstacleArea { kind, .. } => kind.board_item_type(),
            ItemData::ConductionArea { .. } => BoardItemType::ConductionArea,
            ItemData::ComponentOutline { .. } => BoardItemType::ComponentOutline,
            ItemData::BoardOutline { .. } => BoardItemType::BoardOutline,
            ItemData::Other => BoardItemType::Other,
        }
    }

    /// Java `item.getClass().getSimpleName()` for the driver's log rows
    /// (the batch queue row, the ripped-item row, and the net-94 dump):
    /// the class simple names of the routed kinds. The obstacle kinds
    /// fold to `"ObstacleArea"` (Java's concrete subclasses differ, but
    /// obstacle items are never connectable, so they can never reach a
    /// row that renders this face); `Other` falls back to `"Item"` the
    /// same way the callers' null arms do.
    #[must_use]
    pub fn java_simple_name(&self) -> &'static str {
        match self {
            ItemData::Pin { .. } => "Pin",
            ItemData::Trace { .. } => "PolylineTrace",
            ItemData::Via { .. } => "Via",
            ItemData::ConductionArea { .. } => "ConductionArea",
            ItemData::ObstacleArea { .. } => "ObstacleArea",
            ItemData::ComponentOutline { .. } => "ComponentOutline",
            ItemData::BoardOutline { .. } => "BoardOutline",
            ItemData::Other => "Item",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::point::Point;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    /// A minimal non-degenerate tile area for kind-dispatch tests.
    fn test_area() -> Area {
        Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
            RegularTileShape::IntBox(IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 10))),
        )))
    }

    /// The dispatch mirrors `Item.getBoardItemType()`
    /// (`Item.java:117-146`): every kind maps to its own
    /// `BoardItemType`, and the three obstacle kinds stay DISTINCT
    /// (T70) — a port that collapses them into one kind fails here.
    #[test]
    fn all_ten_board_item_kinds_are_reachable_and_distinct() {
        let data = [
            ItemData::Trace {
                layer: 0,
                half_width: 100,
                lines: Polyline::from_two_corners(
                    &Point::Int(IntPoint::new(0, 0)),
                    &Point::Int(IntPoint::new(100, 0)),
                ),
            },
            ItemData::Pin {
                pin_index: 0,
                padstack_no: 1,
            },
            ItemData::Via {
                center: IntPoint::new(0, 0),
                padstack_no: 1,
                attach_smd_allowed: false,
            },
            ItemData::ObstacleArea {
                kind: ObstacleKind::ObstacleArea,
                layer: 0,
                area: test_area(),
                translation: IntPoint::ZERO,
                rotation: 0.0,
                side_changed: false,
                name: None,
            },
            ItemData::ObstacleArea {
                kind: ObstacleKind::ViaObstacleArea,
                layer: 0,
                area: test_area(),
                translation: IntPoint::ZERO,
                rotation: 0.0,
                side_changed: false,
                name: None,
            },
            ItemData::ConductionArea {
                layer: 0,
                area: test_area(),
                is_obstacle: false,
                is_filled: true,
            },
            ItemData::ObstacleArea {
                kind: ObstacleKind::ComponentObstacleArea,
                layer: 0,
                area: test_area(),
                translation: IntPoint::ZERO,
                rotation: 0.0,
                side_changed: false,
                name: None,
            },
            ItemData::BoardOutline {
                shapes: Vec::new(),
                keepout_outside_outline: false,
            },
            ItemData::ComponentOutline {
                layer: 0,
                area: test_area(),
                translation: IntPoint::ZERO,
                rotation: 0.0,
                is_front: true,
                is_courtyard: true,
                is_fabrication: false,
                is_closed: true,
            },
            ItemData::Other,
        ];
        let kinds: Vec<BoardItemType> = data.iter().map(ItemData::board_item_type).collect();
        assert_eq!(
            kinds,
            vec![
                BoardItemType::Trace,
                BoardItemType::Pin,
                BoardItemType::Via,
                BoardItemType::ObstacleArea,
                BoardItemType::ViaObstacleArea,
                BoardItemType::ConductionArea,
                BoardItemType::ComponentObstacleArea,
                BoardItemType::BoardOutline,
                BoardItemType::ComponentOutline,
                BoardItemType::Other,
            ],
            "all ten kinds, Java BoardItemType declaration order"
        );
        // The obstacle discriminator maps to three distinct kinds.
        assert_ne!(kinds[3], kinds[4]);
        assert_ne!(kinds[4], kinds[6]);
        assert_ne!(kinds[3], kinds[6]);
    }

    /// `FixedState` mirrors the Java ordinal order (`FixedState.java`,
    /// the sink IR docs): derived `Ord` must agree, the closed-trace
    /// guard and the T37 plane heuristic compare ordinals.
    #[test]
    fn fixed_state_order_mirrors_the_java_ordinals() {
        let mut states = [
            FixedState::SystemFixed,
            FixedState::Unfixed,
            FixedState::UserFixed,
            FixedState::ShoveFixed,
        ];
        states.sort();
        assert_eq!(
            states,
            [
                FixedState::Unfixed,
                FixedState::ShoveFixed,
                FixedState::UserFixed,
                FixedState::SystemFixed,
            ]
        );
    }

    /// Java `ConvexShape.maxWidth()` dispatch (`M3-T5`): the tile
    /// family is its own max box dimension, a polygon's its bounding
    /// box's, a circle's its diameter — all three variants pinned so a
    /// collapsed dispatch fails.
    #[test]
    fn shape_max_width_dispatches_over_the_three_kinds() {
        let tile = BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(
            IntBox::new(IntPoint::new(0, 0), IntPoint::new(300, 200)),
        )));
        assert_eq!(super::shape_max_width(&tile), 300.0, "tile: max dimension");

        let polygon = BoardShape::PolygonShape(PolygonShape::new(&[
            Point::Int(IntPoint::new(0, 0)),
            Point::Int(IntPoint::new(400, 0)),
            Point::Int(IntPoint::new(400, 150)),
        ]));
        assert_eq!(
            super::shape_max_width(&polygon),
            400.0,
            "polygon: bounding box max dimension"
        );

        let circle = BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 90));
        assert_eq!(super::shape_max_width(&circle), 180.0, "circle: diameter");
    }
}
