//! Board outline — keepout derivations (M2 Task 4).
//!
//! Java anchor: `board/model/structure/BoardOutline.java` — the id-1
//! item. The class carries TWO alternative keepout forms:
//!
//! - `getKeepoutArea()` (`:182-190`) — the AREA outside the outline
//!   curves ("The outline curves are holes of the keepoutArea"):
//!   `new PolylineArea(this.board.boundingBox, shapes.clone())`. The
//!   border is the board bounding box (the outline bounds enlarged by
//!   1000 at `create_board`, `Structure.java:1207-1208`, T43) and the
//!   outline's own shapes become the HOLES.
//! - the LINE keepout — what the search tree actually inserts while
//!   `keepoutOutsideOutline` is false (the parse-time state): each
//!   border line inflated into a tile by `offsetShape(HALF_WIDTH +
//!   clearanceCompensation, 0)`. The HALF_WIDTH constant
//!   (`BoardOutline.java:28`, **100**) is consumed ONLY here — by the
//!   line branch of `ShapeSearchTree.calculateTreeShapes(BoardOutline)`
//!   (`ShapeSearchTree.java:964-990`) via `getHalfWidth()`
//!   (`BoardOutline.java:256-258`).
//!
//! `getKeepoutLines()` (`BoardOutline.java:192-197`) is JAVA DEAD
//! CODE: it lazily assigns `new TileShape[0]` and nothing ever fills
//! the array — the line keepout is computed tree-side, not stored.
//! The port reproduces the empty result and documents it
//! ([`keepout_lines`]).
//!
//! Evidence (jar spike `rust/harness/oracle/ItemGeometrySpike.java`):
//! the outline keepout pins run on **DAC2020_bm08.dsn** — the
//! SMALLEST tier-A fixture with a boundary (5501 bytes, a closed
//! 4-corner pcb path, 2 signal layers, an all-integer tile set; the
//! smaller tier-A boards have no outline, and
//! `complex_hierarchy`'s diagonal outline produces RATIONAL tile
//! corners, unsuitable for exact pins). Capture
//! `/tmp/epic-t4-items-bm08.out`, `OUTLINE`/`BOARD_BBOX`/
//! `OUTLINE_SHAPE`/`OUTLINE_KEEPOUT`/`OUTLINE_HOLE`/`OUTLINE_TILES`
//! lines.

use epic_geometry::int_box::IntBox;
use epic_geometry::line::Line;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;

use crate::items::{Area, BoardShape, ItemData};

/// Java `BoardOutline.HALF_WIDTH` (`BoardOutline.java:28`) — the
/// fixed inflation half width of the outline LINE keepout. 100 in
/// internal board units, independent of any clearance rule.
pub const HALF_WIDTH: i32 = 100;

/// Java `BoardOutline.getHalfWidth()` (`BoardOutline.java:256-258`)
/// — the constant [`HALF_WIDTH`].
#[must_use]
pub fn half_width() -> i32 {
    HALF_WIDTH
}

/// Java `BoardOutline.lineCount()` (`BoardOutline.java:246-252`) —
/// "the sum of the lines of all outline polygons":
/// `borderLineCount()` summed over the shapes. The CIRCLE arm is
/// unreachable from a parsed boundary (the reader's
/// `(PolylineShape)` cast rejects circles before the outline item
/// exists, `structure.rs` outline loop) and returns 0 like Java's
/// stub in the same position.
#[must_use]
pub fn line_count(shapes: &[BoardShape]) -> usize {
    shapes
        .iter()
        .map(|shape| match shape {
            BoardShape::Tile(tile) => tile.border_line_count(),
            BoardShape::PolygonShape(polygon) => polygon.border_line_count(),
            BoardShape::Circle(_) => 0,
        })
        .sum()
}

/// Java `BoardOutline.getKeepoutArea()` (`BoardOutline.java:182-190`):
/// the outside-the-outline area — the board bounding box as the
/// border, the outline shapes as the holes (`shapes.clone()` into
/// `new PolylineArea(board.boundingBox, holeArr)`). Java memoizes in
/// the `keepoutArea` field (cleared by the geometry mutators
/// `:117-152`); the port computes on demand — same values, and the
/// mutators are not part of the M2 surface.
#[must_use]
pub fn keepout_area(bounding_box: IntBox, shapes: &[BoardShape]) -> Area {
    Area {
        border: BoardShape::Tile(TileShape::RegularTileShape(
            epic_geometry::regular_tile_shape::RegularTileShape::IntBox(bounding_box),
        )),
        holes: shapes.to_vec(),
    }
}

/// Java `BoardOutline.getKeepoutLines()` (`BoardOutline.java:192-197`)
/// — ALWAYS EMPTY. Dead code in Java (the lazily-assigned
/// `new TileShape[0]` is never replaced; the real line keepout is
/// computed by [`line_keepout_tiles`], the search-tree branch). The
/// port reproduces the empty result so callers mirror Java exactly; a
/// fresh `Vec` per call mirrors Java's lazy `new TileShape[0]`
/// allocation (never a shared static).
#[must_use]
pub fn keepout_lines() -> Vec<TileShape> {
    Vec::new()
}

/// The border lines of an outline shape in Java's order.
fn shape_border_lines(shape: &BoardShape) -> Vec<Line> {
    match shape {
        BoardShape::Tile(tile) => (0..tile.border_line_count() as i32)
            .map(|no| tile.border_line(no))
            .collect(),
        BoardShape::PolygonShape(polygon) => (0..polygon.border_line_count() as i32)
            .map(|no| polygon.border_line(no))
            .collect(),
        BoardShape::Circle(_) => Vec::new(),
    }
}

/// The LINE keepout of a `keepoutOutsideOutline == false` outline —
/// `ShapeSearchTree.calculateTreeShapes(BoardOutline)`, line branch
/// (`ShapeSearchTree.java:964-990`), verbatim loop shape:
///
/// - the result is LAYER-MAJOR (`lineCount * layerCount` tiles:
///   every line of every shape once per layer, layer 0 first),
/// - per border line `i` the THREE-LINE window is
///   `(border_line(i - 1), border_line(i), border_line((i + 1) % n))`
///   — the previous/next lines wrap around the closed polygon
///   (`:975` seeds `currentLineArr[0] = borderLine(n - 1)`;
///   `:984` carries `currentLineArr[0] = currentLineArr[1]`),
/// - the window becomes a canonical [`Polyline`] (the ctor skips
///   degenerate corners — the M1b dsn-0151 core) and each tile is
///   `offsetShape(half_width + cmp(layer), 0)`.
///
/// `cmp` is `clearanceCompensationValue(clearanceClassIndex, layer)`
/// (`ShapeSearchTree.java:104-114` — 0 whenever the class is <= 0;
/// the bm08 capture pins `class=1 cmp=0` on both layers). The task-7
/// search tree supplies the real rules-driven closure; here it is a
/// parameter so the geometry stays rule-free.
///
/// The CIRCLE arm contributes nothing (unreachable from a parsed
/// boundary — see [`line_count`]).
#[must_use]
pub fn line_keepout_tiles(
    shapes: &[BoardShape],
    layer_count: usize,
    half_width: i32,
    cmp: impl Fn(i32) -> i32,
) -> Vec<Option<TileShape>> {
    let mut result = Vec::with_capacity(line_count(shapes) * layer_count);
    for layer_index in 0..layer_count {
        let cmp_value = cmp(layer_index as i32);
        for shape in shapes {
            let lines = shape_border_lines(shape);
            let border_line_count = lines.len();
            if border_line_count == 0 {
                continue;
            }
            // The wraparound window: prev starts at the LAST line.
            let mut prev = lines[border_line_count - 1].clone();
            for i in 0..border_line_count {
                let current = lines[i].clone();
                let next = lines[(i + 1) % border_line_count].clone();
                let window = Polyline::new(vec![prev, current.clone(), next]);
                result.push(window.offset_shape(half_width + cmp_value, 0));
                prev = current;
            }
        }
    }
    result
}

impl crate::board::Board {
    /// `BoardOutline.getKeepoutArea()` for the outline item with the
    /// given id — [`keepout_area`] over the board's own bounding box
    /// and the item's shapes. `None` for a non-outline item or before
    /// the bounding box exists (never after a successful parse).
    #[must_use]
    pub fn outline_keepout_area(&self, id: crate::id::ItemId) -> Option<Area> {
        let entry = self.get(id)?;
        let ItemData::BoardOutline { shapes, .. } = &entry.data else {
            return None;
        };
        let bounding_box = self.bounding_box()?;
        Some(keepout_area(bounding_box, shapes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::id::ItemId;
    use crate::items::ItemData;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;

    /// The bm08 fixture path — the smallest tier-A fixture WITH an
    /// outline (module docs).
    const BM08: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
    );

    /// Parses bm08 through the epic-dsn reader and converts it.
    fn bm08_board() -> Board {
        let bytes = std::fs::read(BM08).expect("bm08 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// Renders a tile's corners like the spike's `corners()` helper
    /// (`x,y;...`, exact ints only — bm08's tiles are all-integer).
    fn corners(tile: &TileShape) -> String {
        (0..tile.border_line_count())
            .map(|no| match tile.corner(no as i32) {
                Point::Int(p) => format!("{},{}", p.x, p.y),
                other => panic!("expected an integer corner, got {other:?}"),
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The id of the board's outline item (id 1 on every parse).
    fn outline_id(board: &Board) -> ItemId {
        board
            .iter_descending()
            .find(|entry| matches!(entry.data, ItemData::BoardOutline { .. }))
            .map(|entry| entry.id)
            .expect("the outline item exists")
    }

    /// The outline header capture: `OUTLINE id=1 shapes=1 lineCount=4
    /// halfWidth=100 keepoutOutside=false clearanceClass=1` and
    /// `OUTLINE_SHAPE s=0 lines=4 bbox=1382520 -1119380 1587500
    /// -980694`. lineCount SUMS the border lines of the shapes (4
    /// from one 4-corner polygon); an implementation counting SHAPES
    /// (1) or corners fails.
    #[test]
    fn bm08_outline_header_and_line_count_match_the_capture() {
        let board = bm08_board();
        let id = outline_id(&board);
        let entry = board.get(id).expect("outline");
        let ItemData::BoardOutline {
            shapes,
            keepout_outside_outline,
        } = &entry.data
        else {
            panic!("the outline payload");
        };
        assert_eq!(id.get(), 1, "OUTLINE id=1");
        assert_eq!(shapes.len(), 1, "OUTLINE shapes=1");
        assert!(!keepout_outside_outline, "keepoutOutside=false at parse");
        assert_eq!(line_count(shapes), 4, "OUTLINE lineCount=4");
        assert_eq!(half_width(), 100, "OUTLINE halfWidth=100");
        assert_eq!(entry.clearance_class, 1, "clearanceClass=1");
        let bounds = shapes[0].bounding_box();
        assert_eq!(
            (bounds.ll.x, bounds.ll.y, bounds.ur.x, bounds.ur.y),
            (1_382_520, -1_119_380, 1_587_500, -980_694),
            "OUTLINE_SHAPE s=0 bbox"
        );
    }

    /// The keepout AREA capture: `OUTLINE_KEEPOUT border=IntBox
    /// borderBBox=1381520 -1120380 1588500 -979694 holes=1` with
    /// `OUTLINE_HOLE h=0 lines=4 bbox=1382520 -1119380 1587500
    /// -980694`. The border is the BOARD bounding box (the outline
    /// bounds + the 1000 margin, `BOARD_BBOX 1381520 -1120380
    /// 1588500 -979694`), and the outline shape itself is the single
    /// hole — an implementation using the outline bbox as the border
    /// misses by the margin on all four sides.
    #[test]
    fn bm08_outline_keepout_area_matches_the_capture() {
        let board = bm08_board();
        let id = outline_id(&board);
        // The board bounding box is wired from the parse (T43).
        let bbox = board.bounding_box().expect("BOARD_BBOX exists");
        assert_eq!(
            (bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y),
            (1_381_520, -1_120_380, 1_588_500, -979_694),
            "BOARD_BBOX"
        );
        let keepout = board.outline_keepout_area(id).expect("keepout area");
        match &keepout.border {
            BoardShape::Tile(tile) => {
                let bounds = tile.bounding_box();
                assert_eq!(
                    (bounds.ll.x, bounds.ll.y, bounds.ur.x, bounds.ur.y),
                    (1_381_520, -1_120_380, 1_588_500, -979_694),
                    "OUTLINE_KEEPOUT borderBBox = BOARD_BBOX, not the outline bbox"
                );
            }
            other => panic!("the border is the bounding IntBox, got {other:?}"),
        }
        assert_eq!(keepout.holes.len(), 1, "OUTLINE_KEEPOUT holes=1");
        let hole_bounds = keepout.holes[0].bounding_box();
        assert_eq!(
            (
                hole_bounds.ll.x,
                hole_bounds.ll.y,
                hole_bounds.ur.x,
                hole_bounds.ur.y
            ),
            (1_382_520, -1_119_380, 1_587_500, -980_694),
            "OUTLINE_HOLE h=0 bbox = the outline shape"
        );
        assert_ne!(
            hole_bounds, bbox,
            "anchor-blind: hole bounds and border bounds differ by the 1000 margin"
        );
    }

    /// `getKeepoutLines()` is Java dead code — ALWAYS EMPTY, and the
    /// real line keepout is [`line_keepout_tiles`].
    #[test]
    fn keepout_lines_is_always_empty() {
        assert!(keepout_lines().is_empty());
    }

    /// **The LINE keepout capture — the mutation-(b) pin.**
    /// `OUTLINE_TILES n=8` with the exact corner sequences of tiles
    /// 0-3 (layer 0; 4-7 are the layer-1 repetition, identical because
    /// the captured compensation is 0 on both layers:
    /// `OUTLINE_CMP layer=0 class=1 cmp=0` / `layer=1 class=1 cmp=0`).
    /// Every tile is the 100-unit inflation of one border line's
    /// 3-line window — a port with HALF_WIDTH 0 (or inflated inward,
    /// or without the wraparound neighbor lines) produces different
    /// corners and fails here. The anchor-blind arithmetic: tile 0's
    /// leftmost x 1382420 = the hole's left edge 1382520 - 100.
    #[test]
    fn bm08_outline_line_keepout_tiles_match_the_capture() {
        let board = bm08_board();
        let id = outline_id(&board);
        let entry = board.get(id).expect("outline");
        let ItemData::BoardOutline { shapes, .. } = &entry.data else {
            panic!("the outline payload");
        };
        let layer_count = board.layers().layers.len();
        assert_eq!(layer_count, 2, "BOARD layers=2");
        let tiles = line_keepout_tiles(shapes, layer_count, half_width(), |_| 0);
        assert_eq!(tiles.len(), 8, "OUTLINE_TILES n=8 (4 lines x 2 layers)");

        let expected_layer0 = [
            "1382479,-1119480;1587541,-1119480;1587600,-1119421;1587600,-1119339;1587541,-1119280;1382479,-1119280;1382420,-1119339;1382420,-1119421",
            "1587459,-1119480;1587541,-1119480;1587600,-1119421;1587600,-980653;1587541,-980594;1587459,-980594;1587400,-980653;1587400,-1119421",
            "1382479,-980794;1587541,-980794;1587600,-980735;1587600,-980653;1587541,-980594;1382479,-980594;1382420,-980653;1382420,-980735",
            "1382479,-1119480;1382561,-1119480;1382620,-1119421;1382620,-980653;1382561,-980594;1382479,-980594;1382420,-980653;1382420,-1119421",
        ];
        for (i, expected) in expected_layer0.iter().enumerate() {
            let tile = tiles[i].as_ref().expect("tile is present");
            assert_eq!(
                corners(tile),
                *expected,
                "OUTLINE_TILE i={i} (layer 0), capture verbatim"
            );
        }
        // Layer 1 repeats layer 0 verbatim (cmp=0 on both layers).
        for i in 0..4 {
            let a = tiles[i].as_ref().expect("layer 0 tile");
            let b = tiles[4 + i].as_ref().expect("layer 1 tile");
            assert_eq!(corners(a), corners(b), "layer-1 tile {i} repeats layer 0");
        }

        // Anchor-blind arithmetic: the top-left tile's leftmost x is
        // exactly the outline's left edge minus HALF_WIDTH.
        let tile0 = tiles[0].as_ref().expect("tile 0");
        let bounds = tile0.bounding_box();
        assert_eq!(bounds.ll.x, 1_382_520 - 100, "1382520 - HALF_WIDTH");
    }

    /// A NON-ZERO compensation flows into the inflation width —
    /// `offsetShape(half_width + cmp, 0)` with cmp 250 widens every
    /// tile by 250 more units (the capture's cmp=0 form is the
    /// anchor-blind-complement: here the +cmp and the bare-100
    /// answers genuinely differ).
    #[test]
    fn line_keepout_tiles_add_the_clearance_compensation() {
        let board = bm08_board();
        let id = outline_id(&board);
        let entry = board.get(id).expect("outline");
        let ItemData::BoardOutline { shapes, .. } = &entry.data else {
            panic!("the outline payload");
        };
        let zero = line_keepout_tiles(shapes, 1, half_width(), |_| 0);
        let compensated = line_keepout_tiles(shapes, 1, half_width(), |_| 250);
        let zero_tile = zero[0].as_ref().expect("cmp=0 tile");
        let comp_tile = compensated[0].as_ref().expect("cmp=250 tile");
        assert_eq!(
            comp_tile.bounding_box().ll.x,
            zero_tile.bounding_box().ll.x - 250,
            "half_width + cmp inflates outward by exactly the compensation"
        );
        assert_ne!(
            corners(comp_tile),
            corners(zero_tile),
            "the compensation must be observable"
        );
    }

    /// `keepout_area` is pure data assembly — the shapes are CLONED
    /// into holes (Java `shapes.clone()`, `BoardOutline.java:186`) and
    /// the input slice is untouched.
    #[test]
    fn keepout_area_clones_the_shapes_into_holes() {
        let shape = BoardShape::Tile(TileShape::RegularTileShape(
            epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(0, 0),
                IntPoint::new(1000, 1000),
            )),
        ));
        let shapes = vec![shape.clone()];
        let area = keepout_area(
            IntBox::new(IntPoint::new(-100, -100), IntPoint::new(1100, 1100)),
            &shapes,
        );
        assert_eq!(area.holes, vec![shape], "the hole IS the outline shape");
        assert_eq!(shapes.len(), 1, "the input is untouched");
    }
}
