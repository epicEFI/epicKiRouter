//! Obstacle areas — the T49 absolute-area transform chain (M2 Task 4).
//!
//! Java anchors: `board/model/items/ObstacleArea.java:119-144`
//! (`getArea`) and `board/model/items/ComponentOutline.java:192-216`
//! (`getArea`) — the SAME chain with a different mirror predicate
//! (`sideChanged` vs `!isFront`). Transform order, verbatim:
//!
//! 1. `mirrorVertical(Point.ZERO)` when the mirror flag is set AND the
//!    flip style is NOT rotate-first (`ObstacleArea.java:127-129`),
//! 2. the rotation — exact 90-multiples via
//!    `turn90Degree(((int) rotation) / 90, Point.ZERO)`, everything
//!    else `rotateApprox(Math.toRadians(rotation), FloatPoint.ZERO)`
//!    (`:130-137`),
//! 3. the mirror when the flag is set AND the style IS rotate-first
//!    (`:138-140`),
//! 4. `translateBy(translation)` (`:141`, relative to `Point.ZERO`).
//!
//! The reference implementation of this chain is
//! `epic_dsn::ses_board::SesBoard::obstacle_absolute_area` (M1b Task
//! 12, corpus-proven at 1332/1332). This module is the ITEM-SIDE
//! port over epic-board's [`Area`]/[`BoardShape`] types — the
//! duplication is deliberate until M10 (the crates' shape mirrors
//! stay separate; calling epic-dsn from item internals would couple
//! the live model back to the parse IR).
//!
//! ## Memoization divergence (documented, unobservable)
//!
//! Java caches the result in `precalculatedAbsoluteArea`
//! (`ObstacleArea.java:121`/`:140-141`, cleared by `clearDerivedData`
//! `:218-219`); the port computes on demand. The two differ only when
//! the component table or flip-style flag mutates BETWEEN reads of
//! the same item — never through any M2 gate (the flag is parse-set).
//!
//! ## ConductionArea
//!
//! Java `ConductionArea` (`ConductionArea.java`) EXTENDS
//! `ObstacleArea` and adds `isObstacle` (`:29`) and `isFilled`
//! (`:30`) — the flags live on [`crate::items::ItemData::ConductionArea`].
//! **NOT PORTED** (deliberate omission): Java's `getArea()`
//! AWT-FILL-CACHE (`ConductionArea.java`, the `fillShape`/
//! `drawArea` machinery feeding `ShapeSearchTree` through
//! `java.awt.geom.Area`) — an AWT rendering artifact with no board
//! read of its own; the port's [`Area`] carries the border + holes
//! directly. Also: a parsed `ConductionArea` always carries
//! translation ZERO / rotation 0 / no side change
//! (`BasicBoard.insertConductionArea`), so its "absolute area" is its
//! stored area verbatim — the transform chain is not exercised for
//! conduction areas at parse time.

use epic_geometry::int_point::IntPoint;
use epic_geometry::vector::Vector;

use crate::components::{
    java_to_radians, shape_mirror_vertical, shape_rotate_approx, shape_translate_by,
    shape_turn_90_degree,
};
use crate::id::ItemId;
use crate::items::{Area, BoardShape, ItemData};

/// Applies one shape's T49 chain step (mirror predicate already
/// resolved by the caller): mirror-early -> rotate -> mirror-late ->
/// translate.
fn transform_shape(
    shape: &BoardShape,
    translation: IntPoint,
    rotation: f64,
    mirror: bool,
    flip_style_rotate_first: bool,
) -> BoardShape {
    // Owned working copy — the arms below reassign it wholesale.
    let mut turned = shape.clone();
    if mirror && !flip_style_rotate_first {
        // ObstacleArea.java:127-129 (ComponentOutline.java:199-201).
        turned = shape_mirror_vertical(turned);
    }
    if rotation != 0.0 {
        if rotation % 90.0 == 0.0 {
            // Java `((int) rotation) / 90` — truncating cast first.
            turned = shape_turn_90_degree(turned, (rotation as i32) / 90);
        } else {
            // Math.toRadians parity: divide-first (components.rs docs).
            turned = shape_rotate_approx(turned, java_to_radians(rotation));
        }
    }
    if mirror && flip_style_rotate_first {
        // ObstacleArea.java:138-140 (ComponentOutline.java:210-212).
        turned = shape_mirror_vertical(turned);
    }
    let vector = Vector::Int(epic_geometry::int_vector::IntVector::new(
        translation.x,
        translation.y,
    ));
    shape_translate_by(turned, &vector)
}

/// The T49 absolute-area chain — `ObstacleArea.getArea()`
/// (`ObstacleArea.java:119-144`) over an image-relative area:
/// every border AND every hole goes through the same transform
/// (`PolylineArea.mirrorVertical/turn90Degree/rotateApprox/translateBy`
/// reach the holes; pinned by `t49_placement_transform_reaches_holes`
/// in the epic-dsn reference tests and re-pinned here).
///
/// `mirror` is Java's `sideChanged` for obstacle areas and
/// `!isFront` for component outlines — both call sites feed this one
/// function.
#[must_use]
pub fn placement_absolute_area(
    area: &Area,
    translation: IntPoint,
    rotation: f64,
    mirror: bool,
    flip_style_rotate_first: bool,
) -> Area {
    let transform = |shape: &BoardShape| {
        transform_shape(
            shape,
            translation,
            rotation,
            mirror,
            flip_style_rotate_first,
        )
    };
    Area {
        border: transform(&area.border),
        holes: area.holes.iter().map(transform).collect(),
    }
}

impl crate::board::Board {
    /// `ObstacleArea.getArea()` for an obstacle item on this board —
    /// the T49 chain with the board's flip-style flag
    /// (`components.getFlipStyleRotateFirst()`, parse-set). `None` for
    /// a non-obstacle item or a foreign id — including
    /// `ConductionArea` ids: Java's `ConductionArea` EXTENDS
    /// `ObstacleArea` and inherits `getArea`, but a parse-time
    /// conduction area always carries identity placement fields
    /// (translation zero, rotation 0, no side change — module docs),
    /// so its Java `getArea()` equals its stored area verbatim; read
    /// [`ItemData::ConductionArea`]'s `area` directly for those.
    /// Computes on demand (the Java `precalculatedAbsoluteArea` cache
    /// is a performance artifact — module docs).
    #[must_use]
    pub fn obstacle_area(&self, id: ItemId) -> Option<Area> {
        let entry = self.get(id)?;
        let ItemData::ObstacleArea {
            area,
            translation,
            rotation,
            side_changed,
            ..
        } = &entry.data
        else {
            return None;
        };
        let flip_first = self.components().flip_style_rotate_first();
        Some(placement_absolute_area(
            area,
            *translation,
            *rotation,
            *side_changed,
            flip_first,
        ))
    }

    /// `ComponentOutline.getArea()` (`ComponentOutline.java:192-216`)
    /// for an outline item — the same chain with `!isFront` in the
    /// mirror role. `None` for a non-component-outline item.
    #[must_use]
    pub fn component_outline_area(&self, id: ItemId) -> Option<Area> {
        let entry = self.get(id)?;
        let ItemData::ComponentOutline {
            area,
            translation,
            rotation,
            is_front,
            ..
        } = &entry.data
        else {
            return None;
        };
        let flip_first = self.components().flip_style_rotate_first();
        Some(placement_absolute_area(
            area,
            *translation,
            *rotation,
            !*is_front,
            flip_first,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, ItemEntry};
    use crate::items::FixedState;
    use epic_geometry::circle::Circle;
    use epic_geometry::point::Point;
    use epic_geometry::polygon_shape::PolygonShape;

    /// A circle shape (`circle(x,y,r)` in the capture shorthand).
    fn circle(x: i32, y: i32, r: i32) -> BoardShape {
        BoardShape::Circle(Circle::new(IntPoint::new(x, y), r))
    }

    /// A rectangle polygon in the capture corner order.
    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> BoardShape {
        BoardShape::PolygonShape(PolygonShape::new(&[
            Point::Int(IntPoint::new(x0, y0)),
            Point::Int(IntPoint::new(x1, y0)),
            Point::Int(IntPoint::new(x1, y1)),
            Point::Int(IntPoint::new(x0, y1)),
        ]))
    }

    /// **T49 pins — the full jar-capture row set.** Every row's input
    /// AND expected output was captured from
    /// `build/libs/freerouting-current-executable.jar` by the one-time
    /// jshell probe `/tmp/epic-t12-probe.jsh` (reflection off
    /// `ObstacleArea`, expected = the item's `getArea()` output); the
    /// rows are the SAME rows that pin the epic-dsn reference
    /// (`ses_board.rs::t49_obstacle_absolute_area_matches_jar_captures`,
    /// corpus-proven at 1332/1332), rerun here through the
    /// EPIC-BOARD path — the port must reproduce them IDENTICALLY.
    /// The rows cover every `getArea()` branch: translation-only,
    /// exact 90-degree turns x1/x-1/x2/x3 under both flip styles,
    /// `rotateApprox` at an odd angle, the default-order mirror
    /// (mirror -> rotate -> translate), and the rotate-first ordering
    /// (rotate -> mirror -> translate).
    #[test]
    fn t49_absolute_area_matches_the_jar_captures() {
        struct Case {
            name: &'static str,
            provenance: &'static str,
            flip_style: bool,
            border: BoardShape,
            translation: (i32, i32),
            rotation: f64,
            side_changed: bool,
            expected_border: BoardShape,
        }
        let cases = vec![
            // T49 BRANCH flipFirst=false sideChanged=false rot0 (count=2572).
            // A no-translate bug pins x at 39500 (vs 1370841).
            Case {
                name: "translation_only",
                provenance: "dsn-0002 DAC2020_bm02.dsn keepout id=362 (T49B probe)",
                flip_style: false,
                border: circle(39500, 0, 10000),
                translation: (1331341, -1036066),
                rotation: 0.0,
                side_changed: false,
                expected_border: circle(1370841, -1036066, 10000),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x1 (count=57).
            // turn90Degree(1) is CCW: (109000,37000) -> (-37000,109000).
            Case {
                name: "front_rot90_ccw",
                provenance: "dsn-0012 DAC2020_bm05.dsn keepout id=162",
                flip_style: false,
                border: circle(109000, 37000, 11000),
                translation: (1384311, -1233536),
                rotation: 90.0,
                side_changed: false,
                expected_border: circle(1347311, -1124536, 11000),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x-1 (count=66).
            // turn90Degree(-1) is CW: (50800,-88900) -> (-88900,-50800).
            Case {
                name: "front_rot_minus90_cw",
                provenance: "dsn-0019 1-Wire_Wing unrouted.dsn keepout id=170",
                flip_style: false,
                border: circle(50800, -88900, 18750),
                translation: (1193160, -1089020),
                rotation: -90.0,
                side_changed: false,
                expected_border: circle(1104260, -1139820, 18750),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x2 (count=536).
            // turn90Degree(2) negates both: (35800,0) -> (-35800,0).
            Case {
                name: "front_rot180",
                provenance: "dsn-0022 CM5_MINIMA_3.dsn keepout id=655",
                flip_style: false,
                border: circle(35800, 0, 6000),
                translation: (861500, -622500),
                rotation: 180.0,
                side_changed: false,
                expected_border: circle(825700, -622500, 6000),
            },
            // T49 BRANCH flipFirst=true sideChanged=false rot90x3 (count=12):
            // rotate_first metadata alone must not mirror a front keepout.
            // turn90Degree(3): (0,-220000) -> (-220000,0).
            Case {
                name: "front_rot270_under_rotate_first",
                provenance: "dsn-0051 Issue143-rpi_splitter.dsn keepout id=33",
                flip_style: true,
                border: circle(0, -220000, 146600),
                translation: (1016000, 3556000),
                rotation: 270.0,
                side_changed: false,
                expected_border: circle(796000, 3556000, 146600),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot0 — the pure
            // mirror branch with a NON-DEGENERATE center (x=28800 != 0; an
            // x=0 center cancels the mirror and is anchor-blind).
            // Mirror: (28800,12900) -> (-28800,12900).
            Case {
                name: "back_rot0_mirror_moves_x",
                provenance: "dsn-0083 Issue297-myboard.dsn keepout id=682 (T49B MIRROR-ABS)",
                flip_style: false,
                border: circle(28800, 12900, 5750),
                translation: (1365223, -1614442),
                rotation: 0.0,
                side_changed: true,
                expected_border: circle(1336423, -1601542, 5750),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x1 — the
            // mirror-PROVING pin: mirror-then-turn90 gives (5600,-15000);
            // a mirror-skipping bug gives (5600,15000) and abs y -713800
            // instead of -743800.
            Case {
                name: "back_rot90_mirror_proving",
                provenance: "dsn-0022 CM5_MINIMA_3.dsn keepout id=1267",
                flip_style: false,
                border: circle(15000, -5600, 6500),
                translation: (1301800, -728800),
                rotation: 90.0,
                side_changed: true,
                expected_border: circle(1307400, -743800, 6500),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x2 — mirror,
            // then turn90Degree(2): (-38100,-25400) -> (38100,-25400) ->
            // (-38100,25400).
            Case {
                name: "back_rot180_mirror_then_turn",
                provenance: "dsn-0088 corney_island_wireless.dsn keepout id=25",
                flip_style: false,
                border: circle(-38100, -25400, 17500),
                translation: (1950000, -786250),
                rotation: 180.0,
                side_changed: true,
                expected_border: circle(1911900, -760850, 17500),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x3 — the
            // polygon branch. The expected corner SEQUENCE is the jar's
            // `getArea()` dump verbatim (Java re-normalizes the polygon's
            // start corner under the transform; hand-permuting rel corners
            // gives the same corner set in a DIFFERENT order — pinned as
            // captured, not recomputed).
            Case {
                name: "back_rot270_polygon",
                provenance: "dsn-0155 Issue732-RoyalBlue54L-Feather.dsn keepout id=961",
                flip_style: false,
                border: rect(-12700, -6350, 12700, 6350),
                translation: (1286100, -1049725),
                rotation: 270.0,
                side_changed: true,
                expected_border: BoardShape::PolygonShape(PolygonShape::new(&[
                    Point::Int(IntPoint::new(1279750, -1062425)),
                    Point::Int(IntPoint::new(1292450, -1062425)),
                    Point::Int(IntPoint::new(1292450, -1037025)),
                    Point::Int(IntPoint::new(1279750, -1037025)),
                ])),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rotOdd:330.0 —
            // the rotateApprox branch (cos330 * -55000 = -47631.397 ->
            // -47631). The next row shares this exact input with
            // side_changed=true + rotate_first, so the two rows together
            // discriminate BOTH the mirror and the ordering (wrong order
            // -> y -1369250 here).
            Case {
                name: "front_rot330_approx",
                provenance: "dsn-0033 Issue054-tairakb.dsn keepout id=1479",
                flip_style: false,
                border: circle(-55000, 0, 9000),
                translation: (1742920, -1341750),
                rotation: 330.0,
                side_changed: false,
                expected_border: circle(1695289, -1314250, 9000),
            },
            // rotate-first ordering discriminator (T49B probe): rotateApprox
            // rounds FIRST (-47631,27500), then the mirror flips to
            // (47631,27500): abs (1790551,-1314250). The default order on
            // the same input is the row above (1695289,-1314250) — the
            // ordering swap moves BOTH coordinates.
            Case {
                name: "flip_first_rotate_then_mirror",
                provenance: "T49B FLIPFIRST-CIRC probe on dsn-0033 keepout id=1479 input",
                flip_style: true,
                border: circle(-55000, 0, 9000),
                translation: (1742920, -1341750),
                rotation: 330.0,
                side_changed: true,
                expected_border: circle(1790551, -1314250, 9000),
            },
        ];
        for case in cases {
            let area = Area::simple(case.border.clone());
            let absolute = placement_absolute_area(
                &area,
                IntPoint::new(case.translation.0, case.translation.1),
                case.rotation,
                case.side_changed,
                case.flip_style,
            );
            assert_eq!(
                absolute.border, case.expected_border,
                "T49 row {} ({})",
                case.name, case.provenance
            );
        }
    }

    /// The T49 rounding pin (`/tmp/epic-t49c.out` `T49C ROUNDDISC`):
    /// 10 degrees on a circle at (55000, 0). The Y coordinate is the
    /// round-vs-truncate discriminator — `sin(10 deg) * 55000 =
    /// 9550.6498...`, so round-half-up gives 9551 while
    /// truncate/floor give 9550 (X does NOT discriminate:
    /// `cos(10) * 55000 = 54164.4264` rounds to 54164 either way).
    #[test]
    fn t49_rotate_approx_rounds_half_up_not_truncates() {
        let area = Area::simple(circle(55000, 0, 9000));
        let absolute =
            placement_absolute_area(&area, IntPoint::new(1742920, -1341750), 10.0, false, false);
        assert_eq!(
            absolute.border,
            circle(1797084, -1332199, 9000),
            "y=-1341750+9551 (round) not -1341750+9550 (truncate)"
        );
    }

    /// The T49 HOLES arm pin (`/tmp/epic-t49c.out` `T49C HOLES`): a
    /// holed area must apply the SAME default-order transform
    /// (mirror -> turn90(1) -> translate) to every hole as to the
    /// border. No corpus keepout carries `(window)` holes, so this is
    /// the synthetic jar composition: border
    /// rect(-20000,-10000,20000,10000), hole
    /// rect(-5000,-2500,5000,2500), t(1301800,-728800), rot 90,
    /// side changed. Expected corner SEQUENCES are the jar dump
    /// verbatim. A border-only implementation leaves the hole at its
    /// relative position — off by the full mirror+turn here.
    #[test]
    fn t49_placement_transform_reaches_holes() {
        let area = Area {
            border: rect(-20000, -10000, 20000, 10000),
            holes: vec![rect(-5000, -2500, 5000, 2500)],
        };
        let absolute =
            placement_absolute_area(&area, IntPoint::new(1301800, -728800), 90.0, true, false);
        assert_eq!(
            absolute.border,
            rect(1291800, -748800, 1311800, -708800),
            "the transformed border (jar corner order survives)"
        );
        assert_eq!(
            absolute.holes,
            vec![BoardShape::PolygonShape(PolygonShape::new(&[
                Point::Int(IntPoint::new(1299300, -733800)),
                Point::Int(IntPoint::new(1304300, -733800)),
                Point::Int(IntPoint::new(1304300, -723800)),
                Point::Int(IntPoint::new(1299300, -723800)),
            ]))],
            "the transformed hole (jar corner order)"
        );
    }

    /// The synthetic ComponentOutline captures
    /// (`ItemGeometrySpike.java` `syntheticComponentOutlines`,
    /// `/tmp/epic-t4-items-bm08.out`) — `ComponentOutline.getArea`
    /// is the SAME chain with `!isFront` in the mirror role, inserted
    /// through the real board API:
    /// ```text
    /// CO_BACK id=45 isFront=false courtyard=true fabrication=false closed=true
    ///   border=IntBox bbox=1299300 -733800 1304300 -723800
    /// CO_APPROX id=46 isFront=true bbox=1737340 -1346415 1748500 -1337085
    ///   border=1746000,-1346415;1748500,-1342085;1739840,-1337085;1737340,-1341415
    /// ```
    /// CO_BACK (relative IntBox (-5000,-2500,5000,2500), back side,
    /// rot 90, t (1301800,-728800)) lands on the SAME rectangle as
    /// the holes pin's hole — mirror + turn90 of the box; the
    /// CO_APPROX corner sequence is the capture verbatim.
    #[test]
    fn component_outline_area_matches_the_spike_captures() {
        let relative = Area::simple(BoardShape::Tile(
            epic_geometry::tile_shape::TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                    epic_geometry::int_box::IntBox::new(
                        IntPoint::new(-5000, -2500),
                        IntPoint::new(5000, 2500),
                    ),
                ),
            ),
        ));
        // CO_BACK: back side -> mirror, default (mirror-first) style.
        let back = placement_absolute_area(
            &relative,
            IntPoint::new(1301800, -728800),
            90.0,
            true,
            false,
        );
        match &back.border {
            BoardShape::Tile(tile) => {
                let bounds = tile.bounding_box();
                assert_eq!(
                    (bounds.ll.x, bounds.ll.y, bounds.ur.x, bounds.ur.y),
                    (1299300, -733800, 1304300, -723800),
                    "CO_BACK bbox"
                );
            }
            other => panic!("CO_BACK border stays a tile, got {other:?}"),
        }
        // CO_APPROX: FRONT side (no mirror), rot 330 -> rotateApprox.
        let approx = placement_absolute_area(
            &relative,
            IntPoint::new(1742920, -1341750),
            330.0,
            false,
            false,
        );
        match &approx.border {
            BoardShape::Tile(tile) => {
                let bounds = tile.bounding_box();
                assert_eq!(
                    (bounds.ll.x, bounds.ll.y, bounds.ur.x, bounds.ur.y),
                    (1737340, -1346415, 1748500, -1337085),
                    "CO_APPROX bbox"
                );
                let corner_string = (0..tile.border_line_count())
                    .map(|no| match tile.corner(no as i32) {
                        Point::Int(p) => format!("{},{}", p.x, p.y),
                        other => panic!("integer corner, got {other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join(";");
                assert_eq!(
                    corner_string,
                    "1746000,-1346415;1748500,-1342085;1739840,-1337085;1737340,-1341415",
                    "CO_APPROX border corners, capture verbatim"
                );
            }
            other => panic!("CO_APPROX border is a simplex tile, got {other:?}"),
        }
    }

    /// **I-1 board-path pin — `Board::obstacle_area`.** The T49 rows
    /// above call the FREE function with hand-supplied flags; this one
    /// drives the wrapper through a real inserted entry
    /// (`insert_item` -> `board.obstacle_area(id)`), pinning the
    /// entry-field -> predicate wiring the free-function rows cannot
    /// see. Geometry: the mirror-proving T49B circle (NON-origin
    /// centered, so the mirror is observable) at rot 90,
    /// t(1301800,-728800), default mirror-first style:
    /// `side_changed=false` -> turn90 only -> y -713800;
    /// `side_changed=true` -> mirror-then-turn, the captured
    /// `back_rot90_mirror_proving` row -> y -743800. A wrapper that
    /// drops or inverts the `side_changed` read swaps the two
    /// outcomes.
    #[test]
    fn obstacle_area_wrapper_reads_side_changed_from_the_entry() {
        let mut board = Board::new();
        assert!(
            !board.components().flip_style_rotate_first(),
            "the default style is mirror-first"
        );
        let make_entry = |id: crate::id::ItemId, side_changed: bool| ItemEntry {
            id,
            data: ItemData::ObstacleArea {
                kind: crate::items::ObstacleKind::ObstacleArea,
                layer: 0,
                area: Area::simple(circle(15_000, -5_600, 6_500)),
                translation: IntPoint::new(1_301_800, -728_800),
                rotation: 90.0,
                side_changed,
                name: None,
            },
            nets: Vec::new(),
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::SystemFixed,
            on_the_board: false,
        };
        let front_id = board.alloc_id();
        board.insert_item(make_entry(front_id, false));
        let back_id = board.alloc_id();
        board.insert_item(make_entry(back_id, true));

        let front = board.obstacle_area(front_id).expect("front keepout");
        assert_eq!(
            front.border,
            circle(1_307_400, -713_800, 6_500),
            "no mirror: turn90(15000,-5600) = (5600,15000), + t"
        );
        let back = board.obstacle_area(back_id).expect("back keepout");
        assert_eq!(
            back.border,
            circle(1_307_400, -743_800, 6_500),
            "the T49 back_rot90_mirror_proving row (dsn-0022 id=1267)"
        );
        // The discriminator: the two outcomes differ, so a swapped or
        // dropped predicate cannot satisfy both assertions.
        assert_ne!(front.border, back.border);
        // A foreign id stays None.
        assert!(board.obstacle_area(crate::id::ItemId::new(9_999)).is_none());
    }

    /// **I-1 board-path pin — `Board::component_outline_area`.** The
    /// same outline geometry inserted twice with ONLY `is_front`
    /// differing — the wrapper's `!is_front` mirror predicate (the
    /// reviewer-prescribed discriminating form; the port reads the
    /// flag per call, so two entries on one board are safe). Expected
    /// values are the same turn90 arithmetic as the obstacle pin:
    /// front (no mirror) -> (1307400,-713800); back (mirror-then-turn)
    /// -> (1307400,-743800). An inverted predicate (`is_front` instead
    /// of `!is_front`) swaps the two and fails.
    #[test]
    fn component_outline_area_wrapper_mirrors_only_back_side_outlines() {
        let mut board = Board::new();
        let make_entry = |id: crate::id::ItemId, is_front: bool| ItemEntry {
            id,
            data: ItemData::ComponentOutline {
                layer: 0,
                area: Area::simple(circle(15_000, -5_600, 6_500)),
                translation: IntPoint::new(1_301_800, -728_800),
                rotation: 90.0,
                is_front,
                is_courtyard: true,
                is_fabrication: false,
                is_closed: true,
            },
            nets: Vec::new(),
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::SystemFixed,
            on_the_board: false,
        };
        let front_id = board.alloc_id();
        board.insert_item(make_entry(front_id, true));
        let back_id = board.alloc_id();
        board.insert_item(make_entry(back_id, false));

        let front = board
            .component_outline_area(front_id)
            .expect("front outline");
        assert_eq!(
            front.border,
            circle(1_307_400, -713_800, 6_500),
            "is_front=true -> !is_front=false -> no mirror"
        );
        let back = board.component_outline_area(back_id).expect("back outline");
        assert_eq!(
            back.border,
            circle(1_307_400, -743_800, 6_500),
            "is_front=false -> !is_front=true -> mirror-then-turn"
        );
        assert_ne!(front.border, back.border, "the predicate is observable");
    }
}
