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

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
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

/// The shape's OWN border corners as float points — Java
/// `PolylineShape.cornerApprox(no)` (`PolylineShape.java:68-70`) over
/// the `borderLineCount()` corners. Tiles go through
/// [`TileShape::corner_approx`]: a simplex corner where adjacent
/// border lines are parallel answers the UNBOUNDED arm (Java's
/// MAX_VALUE coordinates — the point lands outside every outline
/// shape, so its distance is 0, the conservative direction) instead
/// of panicking the way the exact [`TileShape::corner`] face does;
/// polygon corners are stored points, exact as in Java. Empty for
/// circles (they carry no border corners; the pin-gap consumers
/// route them through the bounding box instead —
/// [`pad_corner_points`]).
fn own_corner_points(shape: &BoardShape) -> Vec<FloatPoint> {
    match shape {
        BoardShape::Tile(tile) => (0..tile.border_line_count() as i32)
            .filter_map(|no| tile.corner_approx(no))
            .collect(),
        BoardShape::PolygonShape(polygon) => (0..polygon.border_line_count() as i32)
            .map(|no| polygon.corner(no).to_float())
            .collect(),
        BoardShape::Circle(_) => Vec::new(),
    }
}

/// The corners a PAD is sampled at for
/// [`Board::outline_minimum_pin_gap`] — Java `BoardOutline.cornerGap`
/// (a917044ff, upstream #935). Polygonal and tile pads sample their
/// OWN corners: the upstream branch is `instanceof PolylineShape`,
/// which catches `PolygonShape` AND `TileShape` (both extend it;
/// the upstream review patch initially broke exactly this arm by
/// sending only polygons through it — "only round pads fall back to
/// the bounding box"). Round pads sample the bounding-box corners —
/// conservative (never larger than the true gap) for round copper.
/// The Java `isEmpty`/`!isBounded` skip is structural here: every
/// [`BoardShape`] is non-empty and bounded by construction.
fn pad_corner_points(pad_shape: &BoardShape) -> Vec<FloatPoint> {
    if matches!(pad_shape, BoardShape::Circle(_)) {
        let bounds = pad_shape.bounding_box();
        return (0..4).map(|no| bounds.corner(no).to_float()).collect();
    }
    own_corner_points(pad_shape)
}

/// The outline-contains face of Java `BoardOutline.contains` —
/// inside ANY outline shape (the any-shape loop,
/// `BoardOutline.java:266-289`; NOT the `keepoutOutsideOutline`
/// inversion, which only the tree shapes apply).
fn shape_contains_point(shape: &BoardShape, point: &FloatPoint) -> bool {
    match shape {
        BoardShape::Tile(tile) => tile.contains_float(point),
        BoardShape::PolygonShape(polygon) => polygon.contains_float(point),
        BoardShape::Circle(circle) => circle.contains_float(point),
    }
}

/// Java `BoardOutline.distanceToOutline(FloatPoint)` (a917044ff,
/// upstream #935) — 0 for points that are not inside the outline;
/// otherwise the smallest distance to any outline border SEGMENT
/// (upstream measures the corner-to-corner segments directly because
/// `PolygonShape.borderDistance` is not implemented there either).
/// A CIRCLE outline shape contributes nothing — unreachable from a
/// parsed boundary (see [`line_count`]); Java cannot even express it
/// in the `PolylineShape[]` field.
fn distance_to_outline(outline_shapes: &[BoardShape], point: FloatPoint) -> f64 {
    if !outline_shapes
        .iter()
        .any(|shape| shape_contains_point(shape, &point))
    {
        return 0.0;
    }
    let mut result = f64::INFINITY;
    for shape in outline_shapes {
        let corners = own_corner_points(shape);
        let corner_count = corners.len();
        for i in 0..corner_count {
            let segment = FloatLine::new(corners[i], corners[(i + 1) % corner_count]);
            result = result.min(segment.segment_distance(&point));
        }
    }
    result
}

/// Java `BoardOutline.cornerGap(Shape)` (a917044ff, upstream #935) —
/// the smallest outline distance over the pad's sampled corners
/// ([`pad_corner_points`]).
fn corner_gap(pad_shape: &BoardShape, outline_shapes: &[BoardShape]) -> f64 {
    pad_corner_points(pad_shape)
        .into_iter()
        .map(|corner| distance_to_outline(outline_shapes, corner))
        .fold(f64::INFINITY, f64::min)
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

    /// Java `BoardOutline.minimumPinGap()` (a917044ff, upstream #935)
    /// — the smallest gap, in board units, between the copper of any
    /// pin and the outline lines: 0 if a pin touches or crosses the
    /// outline, `+∞` if the board has no pins. Pad corners are
    /// sampled ([`corner_gap`]) — exact for polygonal/tile pads,
    /// conservative (never larger than the true gap) for round ones.
    /// Java's `this.board == null` early return is structural here
    /// (the method lives ON the board); a non-outline id answers
    /// `+∞` the same way Java's null board does.
    ///
    /// `&mut self` because the per-pin layer span resolves through
    /// [`Board::item_first_layer`]/[`Board::item_last_layer`].
    pub fn outline_minimum_pin_gap(&mut self, id: crate::id::ItemId) -> f64 {
        let outline_shapes: Vec<BoardShape> = match self.get(id).map(|entry| &entry.data) {
            Some(ItemData::BoardOutline { shapes, .. }) => shapes.clone(),
            _ => return f64::INFINITY,
        };
        let pin_ids: Vec<crate::id::ItemId> = self
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        let mut result = f64::INFINITY;
        for pin_id in pin_ids {
            let (Some(first_layer), Some(last_layer)) =
                (self.item_first_layer(pin_id), self.item_last_layer(pin_id))
            else {
                continue;
            };
            for layer in first_layer..=last_layer {
                // Java `pin.getShape(layer - pin.firstLayer())` — the
                // null shape slot is the skip.
                let Some(pad_shape) = self.pin_shape(pin_id, layer - first_layer) else {
                    continue;
                };
                result = result.min(corner_gap(&pad_shape, &outline_shapes));
                if result <= 0.0 {
                    return 0.0;
                }
            }
        }
        result
    }

    /// Java `BasicBoard.getOutline()` — the board's outline item, if
    /// it has one (a parsed boundary always yields exactly one).
    #[must_use]
    pub fn outline_id(&self) -> Option<crate::id::ItemId> {
        self.iter_descending()
            .find(|entry| matches!(entry.data, ItemData::BoardOutline { .. }))
            .map(|entry| entry.id)
    }

    /// M11-T6 (upstream #931) — Java `BoardOutline.getEdgePinNets`'s
    /// computation, made EAGER at the Java invalidation seams (see
    /// [`crate::board::Board::edge_pin_nets`]): every net carried by
    /// any pin that is an EDGE or OUTSIDE pin. A pin is edge when its
    /// CENTER is outside every outline shape, or when ANY tile pad
    /// shape on ANY of its layers has a corner outside every outline
    /// shape (`BoardOutline.java:94-126`, the `pre-t6` tree): the
    /// center test is `BoardOutline.contains` (any shape,
    /// [`shape_contains_point`]), the corner test only samples
    /// `TileShape` pads (`instanceof TileShape` — polygon and circle
    /// pads contribute their center verdict only, exactly like Java),
    /// and the pin's WHOLE net list joins the set.
    ///
    /// `&mut self` because the per-pin layer span resolves through
    /// [`Board::item_first_layer`]/[`Board::item_last_layer`] (the
    /// same constraint as [`Board::outline_minimum_pin_gap`], whose
    /// walk this mirrors).
    pub(crate) fn recompute_edge_pin_nets(&mut self) {
        let mut set = std::collections::BTreeSet::new();
        let Some(outline_id) = self.outline_id() else {
            self.edge_pin_nets = set;
            self.edge_pin_nets_dirty = false;
            return;
        };
        let outline_shapes: Vec<BoardShape> = match self.get(outline_id).map(|e| &e.data) {
            Some(ItemData::BoardOutline { shapes, .. }) => shapes.clone(),
            _ => {
                self.edge_pin_nets = set;
                self.edge_pin_nets_dirty = false;
                return;
            }
        };
        let contains = |point: FloatPoint| {
            outline_shapes
                .iter()
                .any(|shape| shape_contains_point(shape, &point))
        };
        let pin_ids: Vec<crate::id::ItemId> = self
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        for pin_id in pin_ids {
            // Java: a non-null center outside the outline decides
            // immediately; a null center OR an inside center falls
            // through to the pad-corner walk.
            let mut is_edge_or_outside = match self.pin_center(pin_id) {
                Some(center) => !contains(center.to_float()),
                None => false,
            };
            if !is_edge_or_outside {
                let (Some(first_layer), Some(last_layer)) =
                    (self.item_first_layer(pin_id), self.item_last_layer(pin_id))
                else {
                    continue;
                };
                'layers: for layer in first_layer..=last_layer {
                    let Some(pad_shape) = self.pin_shape(pin_id, layer - first_layer) else {
                        continue;
                    };
                    // `shape instanceof TileShape` — only tile pads
                    // sample their corners.
                    let BoardShape::Tile(tile) = &pad_shape else {
                        continue;
                    };
                    for no in 0..tile.border_line_count() as i32 {
                        // Java: `corner(c)` yields null on the unbounded
                        // (parallel-line) arm and `contains(null)` answers
                        // false — a degenerate corner classifies the pin
                        // OUTSIDE. `corner_is_bounded` is Java's
                        // `cornerIsBounded` guard for exactly that arm
                        // (the exact `corner` face panics where Java
                        // answers null).
                        if !tile.corner_is_bounded(no) || !contains(tile.corner(no).to_float()) {
                            is_edge_or_outside = true;
                            break 'layers;
                        }
                    }
                }
            }
            if is_edge_or_outside && let Some(entry) = self.get(pin_id) {
                set.extend(entry.nets.iter().copied());
            }
        }
        self.edge_pin_nets = set;
        self.edge_pin_nets_dirty = false;
    }

    /// Java `BoardOutline.blocksNets(int[])` (M11-T6, upstream #931,
    /// `BoardOutline.java:149-159`): the outline blocks a net list
    /// unless EVERY net of the list is an edge-pin net — "a tie trace
    /// that also carries an ordinary net stays blocked". A null/empty
    /// list blocks (there is no net to exempt). Non-outline ids keep
    /// the base net-obstacle verdict (conservative; unreachable
    /// through the seams, where Java would fail the cast).
    #[must_use]
    pub fn outline_blocks_nets(&self, outline_id: crate::id::ItemId, nets: &[i32]) -> bool {
        if nets.is_empty() {
            return true;
        }
        nets.iter()
            .any(|&net| self.item_is_trace_obstacle(outline_id, net))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::id::ItemId;
    use crate::items::ItemData;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_geometry::circle::Circle;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polygon_shape::PolygonShape;

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

    // -------------------------------------------------------------------
    // The minimum-pin-gap corner-sampling pins (a917044ff, upstream
    // #935). World: a right-triangle outline — legs on the axes,
    // hypotenuse x + y = 2000 — with pads placed so the OWN-corner and
    // bounding-box answers genuinely differ: a diamond polygon whose
    // bbox corner (1000,1000) sits exactly ON the hypotenuse (bbox
    // answer 0) while its own nearest corner is 200 above the y=0
    // leg; and a circle whose bbox corners measure 400/sqrt(2) to the
    // hypotenuse while the round copper is genuinely farther.
    // -------------------------------------------------------------------

    /// The right-triangle outline: (0,0), (2000,0), (0,2000).
    fn triangle_outline() -> Vec<BoardShape> {
        vec![BoardShape::PolygonShape(PolygonShape::new(&[
            Point::Int(IntPoint::new(0, 0)),
            Point::Int(IntPoint::new(2000, 0)),
            Point::Int(IntPoint::new(0, 2000)),
        ]))]
    }

    /// The diamond pad: corners (600,200), (1000,600), (600,1000),
    /// (200,600) — every own corner strictly inside the triangle.
    fn diamond_pad() -> BoardShape {
        BoardShape::PolygonShape(PolygonShape::new(&[
            Point::Int(IntPoint::new(600, 200)),
            Point::Int(IntPoint::new(1000, 600)),
            Point::Int(IntPoint::new(600, 1000)),
            Point::Int(IntPoint::new(200, 600)),
        ]))
    }

    /// **The own-corners pin** — polygonal pads sample their OWN
    /// corners, not the bounding box. The diamond's nearest own corner
    /// is (600,200), 200 above the y=0 leg (the other three corners
    /// measure 400/√2 ≈ 282.84, 400/√2, and 200 to the x=0 leg), so
    /// the own-corner gap is exactly 200. The BOUNDING BOX would
    /// answer 0: its corner (1000,1000) lies exactly ON the
    /// hypotenuse (the upstream review patch that routed polygons
    /// through the bbox produced exactly this class of wrong answer).
    #[test]
    fn corner_gap_samples_polygon_corners_not_the_bounding_box() {
        let outline = triangle_outline();
        assert_eq!(
            corner_gap(&diamond_pad(), &outline),
            200.0,
            "own corners: 200 from (600,200) to the y=0 leg"
        );
    }

    /// A pad corner outside every outline shape measures 0 — the
    /// `distanceToOutline` outside arm (and, through it, the
    /// minimumPinGap early return for pads that poke out).
    #[test]
    fn corner_gap_answers_zero_for_pads_outside_the_outline() {
        let outline = triangle_outline();
        let outside = BoardShape::PolygonShape(PolygonShape::new(&[
            Point::Int(IntPoint::new(2600, 200)),
            Point::Int(IntPoint::new(3000, 600)),
            Point::Int(IntPoint::new(2600, 1000)),
            Point::Int(IntPoint::new(2200, 600)),
        ]));
        assert_eq!(
            corner_gap(&outside, &outline),
            0.0,
            "every corner of the shifted diamond is outside the triangle"
        );
    }

    /// **The round-pad bbox pin** — circles have no border corners, so
    /// they fall back to the bounding box (the upstream final
    /// semantics: "only round pads fall back"). Circle center (600,600)
    /// radius 200: the bbox corner (800,800) measures 400/√2 ≈ 282.84
    /// to the hypotenuse, CONSERVATIVE against the true round-copper
    /// gap (600·√2-ish 565.69 − 200 ≈ 365.69) — never larger, exactly
    /// as the Java doc comment promises.
    #[test]
    fn corner_gap_falls_back_to_the_bounding_box_for_round_pads() {
        let outline = triangle_outline();
        let circle = BoardShape::Circle(Circle::new(IntPoint::new(600, 600), 200));
        let expected = 400.0 / 2.0_f64.sqrt();
        assert!(
            (corner_gap(&circle, &outline) - expected).abs() < 1e-9,
            "bbox corner (800,800) -> 400/sqrt(2) = {expected}"
        );
    }

    /// The TILE arm of the own-corner walk — a box tile's own corners
    /// ARE its bbox corners (400,400)-(800,800), so the box measures
    /// the same 400/√2 at (800,800) here; the pin covers the
    /// `TileShape` branch of `own_corner_points` (Java: `TileShape
    /// extends PolylineShape`, the branch the broken upstream review
    /// patch missed).
    #[test]
    fn corner_gap_samples_tile_corners_through_the_polyline_branch() {
        let outline = triangle_outline();
        let box_tile = BoardShape::Tile(TileShape::RegularTileShape(
            epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(400, 400),
                IntPoint::new(800, 800),
            )),
        ));
        let expected = 400.0 / 2.0_f64.sqrt();
        assert!(
            (corner_gap(&box_tile, &outline) - expected).abs() < 1e-9,
            "box corner (800,800) -> 400/sqrt(2) = {expected}"
        );
    }

    // -------------------------------------------------------------------
    // The M11-T6 edge-pin-net pins (upstream #931, cluster A).
    // World: `harness/fixtures/t6/t6-edge-pins.dsn` — boundary rect
    // [0,0,250000,140000], one component at (125000,70000) with three
    // ±5000 rect pads. PIN_IN abs (95000,70000) fully interior;
    // PIN_EDGE abs (247500,70000) center-INSIDE with the pad corner at
    // x 252500 protruding past 250000 (the castellated discriminator —
    // a center-only port misses it); PIN_OUT abs (325000,70000)
    // center-outside. Nets: N_IN on the interior pin, N_EDGE on the
    // corner-protruding pin, N_OUT+N_MIX on the center-outside pin,
    // N_MIX ALSO on the interior pin (a pin carries several nets, and
    // ALL nets of an EDGE pin join the set — while the same net on an
    // INTERIOR pin contributes nothing).
    // -------------------------------------------------------------------

    /// The T6 fixture path.
    const T6_EDGE_PINS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../rust/harness/fixtures/t6/t6-edge-pins.dsn"
    );

    /// Parses the T6 fixture through the epic-dsn reader (the
    /// `bm08_board` pattern).
    fn t6_board() -> Board {
        let bytes = std::fs::read(T6_EDGE_PINS).expect("t6 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// The net NUMBER for a net name (names are unique in the craft).
    fn net_no(board: &Board, name: &str) -> i32 {
        board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name == name)
            .map(|(number, _)| number)
            .unwrap_or_else(|| panic!("net {name} present"))
    }

    /// The pin whose net list contains `needle` (each of N_IN/N_EDGE/
    /// N_OUT names exactly one pin; N_MIX names two, so the mixed-pin
    /// tests resolve pins by their UNIQUE discriminators).
    fn pin_carrying(board: &Board, needle: i32) -> ItemId {
        board
            .iter_descending()
            .find(|entry| {
                matches!(entry.data, ItemData::Pin { .. }) && entry.nets.contains(&needle)
            })
            .map(|entry| entry.id)
            .unwrap_or_else(|| panic!("a pin carrying net {needle}"))
    }

    /// **The classification pin** — the edge set is exactly ⋃ nets of
    /// the EDGE and OUT pins: the corner-protruding pad (center
    /// inside, one corner outside) IS an edge pin, the center-outside
    /// pad IS, the interior pad is NOT, and N_MIX joins through the OUT
    /// pin while N_IN stays out (the interior pin carries N_MIX too —
    /// that membership contributes nothing). Kills the center-test-only
    /// mutant (N_EDGE drops), the corner-walk-dropped mutant (N_EDGE
    /// drops — its center is inside), and the first-net-only mutant
    /// (N_MIX drops).
    #[test]
    fn t6_edge_pin_set_classifies_center_corner_and_inside_pins() {
        let board = t6_board();
        let n_edge = net_no(&board, "N_EDGE");
        let n_out = net_no(&board, "N_OUT");
        let n_mix = net_no(&board, "N_MIX");
        let expected: std::collections::BTreeSet<i32> =
            [n_edge, n_out, n_mix].into_iter().collect();
        assert_eq!(
            board.edge_pin_nets, expected,
            "exactly the EDGE and OUT pins' nets; N_IN never joins"
        );
        assert!(
            !board.edge_pin_nets_dirty,
            "from_ses_board filled the cache (no pending recompute)"
        );
    }

    /// **The blocksNets / isTraceObstacle semantics pin** — `blocksNets`
    /// answers `any(isTraceObstacle)` (BLOCKED unless EVERY net is an
    /// edge-pin net — a tie trace that also carries an ordinary net
    /// stays blocked), empty blocks, and the net guard: only `net > 0`
    /// edge-pin membership exempts (net 0 and negatives never do).
    #[test]
    fn t6_outline_blocks_nets_and_trace_obstacle_semantics() {
        let board = t6_board();
        let outline = outline_id(&board);
        let n_in = net_no(&board, "N_IN");
        let n_edge = net_no(&board, "N_EDGE");
        let n_out = net_no(&board, "N_OUT");
        let n_mix = net_no(&board, "N_MIX");
        assert!(
            board.outline_blocks_nets(outline, &[]),
            "empty net list blocks"
        );
        assert!(!board.outline_blocks_nets(outline, &[n_edge]));
        assert!(
            !board.outline_blocks_nets(outline, &[n_out, n_mix]),
            "an ALL-edge net list passes"
        );
        assert!(
            board.outline_blocks_nets(outline, &[n_in]),
            "ordinary net blocks"
        );
        assert!(
            board.outline_blocks_nets(outline, &[n_edge, n_in]),
            "mixed list stays blocked (the any-semantics, not all-of-mine)"
        );
        assert!(!board.item_is_trace_obstacle(outline, n_edge));
        assert!(board.item_is_trace_obstacle(outline, n_in));
        assert!(
            board.item_is_trace_obstacle(outline, 0),
            "net 0 never joins the edge set (the net > 0 guard)"
        );
        assert!(
            board.item_is_trace_obstacle(outline, -1),
            "negative nets never join either"
        );
    }

    /// **The set_item_nets recompute pin** (Java `changeNet` tail,
    /// `Item.java:1049-1053`): re-netting the CENTER-OUTSIDE pin moves
    /// its nets out of the edge set and the new net in (a port without
    /// the hook keeps the stale N_OUT/N_MIX membership); re-netting the
    /// INTERIOR pin changes nothing (its nets were never in the set).
    #[test]
    fn t6_set_item_nets_recomputes_the_edge_pin_set() {
        let mut board = t6_board();
        let n_in = net_no(&board, "N_IN");
        let n_edge = net_no(&board, "N_EDGE");
        let n_out = net_no(&board, "N_OUT");
        let n_mix = net_no(&board, "N_MIX");
        let out_pin = pin_carrying(&board, n_out);
        let in_pin = pin_carrying(&board, n_in);

        board.set_item_nets(out_pin, vec![999]);
        assert!(
            !board.edge_pin_nets.contains(&n_out) && !board.edge_pin_nets.contains(&n_mix),
            "the re-netted edge pin's old nets left the set"
        );
        assert!(board.edge_pin_nets.contains(&999), "the new net joined");
        assert!(
            board.edge_pin_nets.contains(&n_edge),
            "the untouched edge pin keeps its net in the set"
        );
        assert!(!board.edge_pin_nets_dirty);

        board.set_item_nets(in_pin, vec![42]);
        assert!(
            !board.edge_pin_nets.contains(&42),
            "an interior pin's nets never join — even after a re-net"
        );
        assert_eq!(
            board.edge_pin_nets,
            [n_edge, 999].into_iter().collect(),
            "the interior re-net is a no-op on the set"
        );
    }

    // -------------------------------------------------------------------
    // The M11-T2 pins (upstream #931): the board bounding box grows
    // to cover every item at the route head
    // (`BasicBoard.expandBoundingBoxToIncludeAllItems`,
    // BasicBoard.java:592-609 of the post tree; the call site is
    // HeadlessBoardManager.startRouting:835).
    // -------------------------------------------------------------------

    /// Renders a box like the bm08 pins (`ll.x ll.y ur.x ur.y`).
    fn box_str(box_: &epic_geometry::int_box::IntBox) -> String {
        format!("{} {} {} {}", box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y)
    }

    /// **The growth + idempotence pin** — on the T6 fixture the parse
    /// box is the outline ± the 1000 parse margin (T43), which does
    /// NOT contain the center-outside pin's pad (x up to 330000) or
    /// the corner-protruding pad (252500): the expand walk unions
    /// them, then re-applies the 1000 margin. A second expand changes
    /// nothing (every box contained — an unconditional-offset mutant
    /// grows the box again and dies here). The outline's own
    /// `item_bounding_box` is the boundary rect itself (the shapes'
    /// union, no margin).
    #[test]
    fn t2_expand_grows_over_protruding_pins_and_is_idempotent() {
        let mut board = t6_board();
        let outline = outline_id(&board);
        assert_eq!(
            box_str(&board.bounding_box().expect("parse box")),
            "-1000 -1000 251000 141000",
            "the T43 parse box: outline + 1000"
        );
        assert_eq!(
            box_str(&board.item_bounding_box(outline).expect("outline box")),
            "0 0 250000 140000",
            "the outline item's own box is the boundary rect"
        );
        let edge_set_before = board.edge_pin_nets.clone();
        board.expand_bounding_box_to_include_all_items();
        assert_eq!(
            box_str(&board.bounding_box().expect("grown box")),
            "-2000 -2000 331000 142000",
            "union with the OUT pad (330000) then offset(1000)"
        );
        assert_eq!(
            board.edge_pin_nets, edge_set_before,
            "the change-path recompute lands the same set (the outline did not move)"
        );
        assert!(!board.edge_pin_nets_dirty);
        board.expand_bounding_box_to_include_all_items();
        assert_eq!(
            box_str(&board.bounding_box().expect("still the grown box")),
            "-2000 -2000 331000 142000",
            "idempotent: the no-change walk does not re-offset"
        );
    }

    /// **The trace-arm pin** — a Trace's box is the polyline box
    /// OFFSET BY THE HALF WIDTH (`PolylineTraceGeometry.java:39-41`):
    /// a trace to x 400000 at half width 125 grows the box to
    /// 400125 + the 1000 margin (a mutant dropping the
    /// `.offset(half_width)` lands on 400000 and dies). The insert of
    /// a TRACE never dirties the edge-pin cache (the dirty hooks fire
    /// on Pin/BoardOutline only).
    #[test]
    fn t2_trace_boxes_offset_by_the_half_width() {
        let mut board = t6_board();
        let trace_id = board.alloc_id();
        board.insert_item(crate::board::ItemEntry {
            id: trace_id,
            data: ItemData::Trace {
                layer: 0,
                half_width: 125,
                lines: epic_geometry::polyline::Polyline::from_two_corners(
                    &Point::Int(IntPoint::new(0, 0)),
                    &Point::Int(IntPoint::new(400_000, 0)),
                ),
            },
            nets: vec![net_no(&board, "N_IN")],
            clearance_class: 1,
            component_id: 0,
            fixed: crate::items::FixedState::Unfixed,
            on_the_board: false,
        });
        assert_eq!(
            box_str(&board.item_bounding_box(trace_id).expect("trace box")),
            "-125 -125 400125 125",
            "polyline box [0,0,400000,0] offset by half width 125"
        );
        assert!(
            !board.edge_pin_nets_dirty,
            "a trace insert never marks the edge-pin cache dirty"
        );
        board.expand_bounding_box_to_include_all_items();
        assert_eq!(
            box_str(&board.bounding_box().expect("grown box")),
            "-2000 -2000 401125 142000",
            "the trace's 400125 drives the union"
        );
    }

    // -------------------------------------------------------------------
    // The T6 degenerate-pad pin (the dsn-corpus Issue179 finding,
    // 2026-10-02): padstack p7 of Issue179-Autorouter_PCB1 declares a
    // TWO-POINT polygon pad — a zero-width line segment — which the
    // reader normalizes into a 2-line Simplex (two parallel half-planes,
    // NO finite corner). Java's corner(c) yields null on that arm and
    // contains(null) answers false, so the pin classifies OUTSIDE and
    // its net joins the edge set; the port's exact corner() PANICS there
    // (the from_ses_board eager recompute crashed the dsn-corpus walk
    // until corner_is_bounded guarded the arm).
    // -------------------------------------------------------------------

    /// The degenerate-pad fixture path.
    const T6_DEGENERATE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../rust/harness/fixtures/t6/t6-degenerate-pad.dsn"
    );

    /// **The unbounded-corner pin** — the two-point-polygon pad parses
    /// into a 2-line Simplex whose every corner is unbounded: the pin's
    /// CENTER is inside the outline (the center test alone would NOT
    /// classify it), the guarded corner walk classifies it OUTSIDE
    /// (Java null-corner semantics), and the parse does not panic —
    /// the regression face the dsn corpus caught. A port dropping the
    /// corner_is_bounded guard panics here at parse.
    #[test]
    fn t6_degenerate_pad_classifies_outside_without_panicking() {
        let bytes = std::fs::read(T6_DEGENERATE).expect("degenerate fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        // The eager recompute rides the conversion — a panic here is
        // the regression (unguarded exact corner on the parallel lines).
        let mut board = Board::from_ses_board(&ses);
        let n_rect = net_no(&board, "N_RECT");
        let n_line = net_no(&board, "N_LINE");
        let line_pin = pin_carrying(&board, n_line);
        // The discriminator precondition: the LINE pin's center is
        // INSIDE (the unbounded corner, not the center, decides).
        let center = board
            .pin_center(line_pin)
            .expect("center exists")
            .to_float();
        assert!(
            center.x > 0.0 && center.x < 250_000.0 && center.y > 0.0 && center.y < 140_000.0,
            "the degenerate pin sits interior (center {center:?})"
        );
        assert_eq!(
            board.edge_pin_nets,
            [n_line]
                .into_iter()
                .collect::<std::collections::BTreeSet<i32>>(),
            "only the degenerate pad's net joins — the rect pad stays interior"
        );
        assert!(!board.edge_pin_nets_dirty);
        let outline = outline_id(&board);
        assert!(
            !board.outline_blocks_nets(outline, &[n_line]),
            "the degenerate pad's net gains the edge exemption"
        );
        assert!(
            board.outline_blocks_nets(outline, &[n_rect]),
            "the ordinary interior pad's net stays blocked"
        );
        // set_item_nets on the degenerate pin recomputes through the
        // guarded walk too (the hook path, no panic).
        board.set_item_nets(line_pin, vec![777]);
        assert_eq!(
            board.edge_pin_nets,
            [777]
                .into_iter()
                .collect::<std::collections::BTreeSet<i32>>(),
            "the re-net tracks the degenerate pin's nets"
        );
    }
}
