//! The coarse-grid congestion map (M6-T7) — an occupancy/overflow
//! estimator over the board's SIGNAL layers (design §4.2 stage 2, the
//! M9 heatmap's substrate, design :360).
//!
//! ## The model (beyond-Java: no oracle exists, nothing here is
//! parity-checked)
//!
//! A square grid of cell side `cell = max(1, max(bbox_width,
//! bbox_height) / 128)` covers the board's bounding box. Every item's
//! placed copper is rasterized onto the cells its bounding box touches,
//! per signal layer it spans, as a DISTINCT-NET occupant set (a net
//! whose copper crosses a cell occupies one routing track through it;
//! netless copper and keepouts occupy one track as the `Netless`
//! occupant). The extent is the PARSE box
//! ([`Board::parse_bounding_box`], the outline-derived board box) —
//! NOT the route-head-grown bounding box (M11-T2): items entirely
//! outside the grid extent occupy NO cells (the far-outside netless
//! outlines the grown box covers route nothing and must not
//! recalibrate the cell size or the border occupancy — the T2
//! finding on interf_u). Cell capacity on a signal layer is the estimated track
//! count `cap = max(1, cell / pitch)` with
//! `pitch = 2 * max_trace_half_width + max_clearance` — the densest
//! legal packing of the board's widest net-class trace at the layer's
//! maximum clearance value. OVERFLOW of a cell is
//! `max(0, occupancy - capacity)`: the number of tracks demanded beyond
//! what the cell can carry.
//!
//! Rasterization is the item's PLACED bounding box per layer (bbox
//! granularity, not exact-shape): traces widen their centerline bbox by
//! the stored half width; drill items (pins/vias) take each span
//! layer's padstack shape bbox; areas take the placed border bbox
//! (window holes ignored — an occupancy OVERestimate only). Component
//! outlines, board outlines, and unfilled areas are not occupancy.
//!
//! ## Determinism contract
//!
//! The map is a pure function of board state: fixed iteration orders
//! (ascending item id, ascending layers, sorted occupant sets), no
//! HashMap anywhere, integer arithmetic only (i64 intermediates,
//! saturating boundary clamps), and the digest is a SHA-256 over
//! canonically ordered occupancy rows. The `&mut Board` the build
//! takes fills the drill-span memo caches (`Board::drill_first_layer`)
//! — memoization is an answer-neutral cache (the equality face between
//! the memoizing and the pure-span read is pinned in epic-board), so
//! the build stays a pure function of board state. Thread-count
//! invariant by construction.

use epic_board::board::Board;
use epic_board::items::{BoardShape, ItemData};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use sha2::{Digest as _, Sha256};

/// One occupant of a cell: a net's copper (by net number) or copper
/// with no net / a keepout (blocks every net equally).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Occupant {
    /// A netless obstruction (keepout, netless copper).
    Netless,
    /// The copper of this net number.
    Net(i32),
}

/// The coarse-grid congestion map over the board's signal layers.
///
/// Built once per planning face ([`crate::global::plan`]) and per
/// pattern-route consult ([`crate::global::pattern`]) — a pure
/// function of board state (module docs).
#[derive(Debug, Clone)]
pub struct CongestionMap {
    /// The grid origin (the bounding box lower-left).
    ll: IntPoint,
    /// The square cell side (>= 1, in board units).
    cell: i64,
    /// Grid width (cells per row).
    nx: usize,
    /// Grid height (rows).
    ny: usize,
    /// Per signal layer: the cell capacity (tracks per cell).
    cap: Vec<i64>,
    /// Per signal layer, per cell (row-major `iy * nx + ix`): the
    /// sorted distinct occupant set.
    occupants: Vec<Vec<Vec<Occupant>>>,
    /// Per signal layer: the total overflow (the overflow vector).
    total_overflow: Vec<u64>,
}

/// The placed bounding box of a board shape.
fn shape_bbox(shape: &BoardShape) -> IntBox {
    match shape {
        BoardShape::Tile(tile) => tile.bounding_box(),
        BoardShape::PolygonShape(polygon) => polygon.bounding_box(),
        BoardShape::Circle(circle) => circle.bounding_box(),
    }
}

/// The bbox of a polyline's corner span (the centerline; the caller
/// widens by the trace half width). Rational corners (unreachable for
/// parsed polylines, which carry `Int` corners end to end) are skipped.
fn polyline_bbox(lines: &epic_geometry::polyline::Polyline) -> IntBox {
    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;
    for corner in lines.corners() {
        if let Point::Int(point) = corner {
            min_x = min_x.min(i64::from(point.x));
            min_y = min_y.min(i64::from(point.y));
            max_x = max_x.max(i64::from(point.x));
            max_y = max_y.max(i64::from(point.y));
        }
    }
    if min_x > max_x {
        return IntBox {
            ll: IntPoint { x: 0, y: 0 },
            ur: IntPoint { x: 0, y: 0 },
        };
    }
    IntBox {
        ll: IntPoint {
            x: min_x as i32,
            y: min_y as i32,
        },
        ur: IntPoint {
            x: max_x as i32,
            y: max_y as i32,
        },
    }
}

/// Widens a bbox symmetrically (saturating) — the trace half width
/// face and the plan's region expansion.
pub(crate) fn widen(bbox: &IntBox, by: i32) -> IntBox {
    IntBox {
        ll: IntPoint {
            x: bbox.ll.x.saturating_sub(by),
            y: bbox.ll.y.saturating_sub(by),
        },
        ur: IntPoint {
            x: bbox.ur.x.saturating_add(by),
            y: bbox.ur.y.saturating_add(by),
        },
    }
}

/// The placed bounding box of an item area: the border's bbox with the
/// ObstacleArea placement transform applied (translation always;
/// rotation by rotating the border bbox's four corners — a bbox of a
/// rotated bbox, conservative for non-90-degree rotations, EXACT for
/// the axis-aligned parses; window holes are ignored, which can only
/// OVERestimate occupancy).
fn area_bbox(area: &epic_board::items::Area, translation: IntPoint, rotation_deg: f64) -> IntBox {
    let raw = shape_bbox(&area.border);
    let bx = if rotation_deg == 0.0 {
        raw
    } else {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let corners = [
            (i64::from(raw.ll.x), i64::from(raw.ll.y)),
            (i64::from(raw.ur.x), i64::from(raw.ll.y)),
            (i64::from(raw.ur.x), i64::from(raw.ur.y)),
            (i64::from(raw.ll.x), i64::from(raw.ur.y)),
        ];
        let mut min_x = i64::MAX;
        let mut min_y = i64::MAX;
        let mut max_x = i64::MIN;
        let mut max_y = i64::MIN;
        for (x, y) in corners {
            // Java's rotation convention (positive = counterclockwise):
            // x' = x cos - y sin, y' = x sin + y cos, rounded — the
            // M1b T49 transform chain's face.
            let rx = (x as f64 * cos - y as f64 * sin).round() as i64;
            let ry = (x as f64 * sin + y as f64 * cos).round() as i64;
            min_x = min_x.min(rx);
            min_y = min_y.min(ry);
            max_x = max_x.max(rx);
            max_y = max_y.max(ry);
        }
        IntBox {
            ll: IntPoint {
                x: min_x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                y: min_y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            },
            ur: IntPoint {
                x: max_x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                y: max_y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            },
        }
    };
    IntBox {
        ll: IntPoint {
            x: bx.ll.x.saturating_add(translation.x),
            y: bx.ll.y.saturating_add(translation.y),
        },
        ur: IntPoint {
            x: bx.ur.x.saturating_add(translation.x),
            y: bx.ur.y.saturating_add(translation.y),
        },
    }
}

impl CongestionMap {
    /// Builds the map over the board's signal layers. An empty board
    /// (no bounding box) yields the empty map (every query answers 0 /
    /// clear).
    #[must_use]
    pub fn build(board: &mut Board) -> Self {
        // The PARSE box, not the grown box: `expand_bounding_box_`
        // `to_include_all_items` (M11-T2, both route heads) grows
        // `bounding_box` to cover every item — including netless
        // ComponentOutlines (fab slivers) far outside the outline,
        // whose growth recalibrated this grid's cell size on interf_u
        // (9045→9596) and degraded the pathfinder. The grid derives
        // from where routing happens — the outline interior (the T2
        // finding). Fallback = the live box for hand-built worlds
        // that never carried a parse box.
        let Some(bbox) = board.parse_bounding_box().or_else(|| board.bounding_box()) else {
            return Self::empty();
        };
        let signal_layer_count = board.layers().signal_layer_count().max(0) as usize;
        if signal_layer_count == 0 {
            return Self::empty();
        }

        // The square cell side: the dominant extent axis (the parse
        // box above) over a 128-cell target resolution (module docs).
        let width = i64::from(bbox.ur.x) - i64::from(bbox.ll.x);
        let height = i64::from(bbox.ur.y) - i64::from(bbox.ll.y);
        let cell = (width.max(height) / 128).max(1);
        let nx = (width / cell + 1).max(1) as usize;
        let ny = (height / cell + 1).max(1) as usize;

        // Capacity per signal layer: `max(1, cell / pitch)` with the
        // widest net-class trace half width and the layer's maximum
        // clearance value (module docs).
        let mut cap = Vec::with_capacity(signal_layer_count);
        for signal_no in 0..signal_layer_count {
            let layer_no = board.layers().get_layer_no(signal_no as i32);
            let layer_no = if layer_no < 0 { 0 } else { layer_no } as usize;
            let mut half_width_max = 0i64;
            for net_class in &board.rules().net_classes {
                half_width_max = half_width_max.max(i64::from(
                    net_class.trace_half_width(layer_no as i32).max(0),
                ));
            }
            let clearance_max = i64::from(
                board
                    .rules()
                    .clearance
                    .max_value_on_layer(layer_no as i32)
                    .max(0),
            );
            let pitch = (2 * half_width_max + clearance_max).max(1);
            cap.push((cell / pitch).max(1));
        }

        let mut occupants: Vec<Vec<Vec<Occupant>>> =
            vec![vec![Vec::new(); nx * ny]; signal_layer_count];

        // The rasterization walk (ascending item id — the canonical
        // order; the occupant sets get sorted + deduplicated before any
        // read face, so the walk order cannot leak into a query). The
        // id list is collected first so the per-item reads (some
        // memo-filling `&mut` calls) never hold an iterator borrow.
        let ids: Vec<epic_board::id::ItemId> = board
            .iter_ascending()
            .filter(|entry| entry.on_the_board)
            .map(|entry| entry.id)
            .collect();
        for id in ids {
            let Some(entry) = board.get(id) else {
                continue;
            };
            let nets = entry.nets.clone();
            let mut push_bbox = |signal_no: usize, item_bbox: &IntBox| {
                // Outside the grid extent: NO cells. `cell_range`
                // clamps out-of-range coordinates onto the border
                // cells (the right semantics for point queries), so
                // an explicit disjointness gate is needed before the
                // walk — without it the far-outside netless outlines
                // the parse-box extent excludes would smear onto the
                // border as phantom occupancy.
                if item_bbox.intersection(&bbox).is_empty() {
                    return;
                }
                let (ix0, ix1, iy0, iy1) = Self::cell_range(item_bbox, &bbox, cell, nx, ny);
                for iy in iy0..=iy1 {
                    for ix in ix0..=ix1 {
                        let slot = &mut occupants[signal_no][iy * nx + ix];
                        if nets.is_empty() {
                            slot.push(Occupant::Netless);
                        } else {
                            for &net in &nets {
                                slot.push(Occupant::Net(net));
                            }
                        }
                    }
                }
            };
            match &entry.data {
                ItemData::Trace {
                    layer,
                    lines,
                    half_width,
                } => {
                    let Some(signal_no) = Self::signal_ordinal(board, *layer) else {
                        continue;
                    };
                    let item_bbox = widen(&polyline_bbox(lines), *half_width);
                    push_bbox(signal_no, &item_bbox);
                }
                ItemData::Pin { .. } | ItemData::Via { .. } => {
                    // Rasterize the drill item's per-layer padstack
                    // shape over its span (the memoized span reads —
                    // answer-neutral caches, module docs).
                    let Some(first) = board.drill_first_layer(id) else {
                        continue;
                    };
                    let Some(last) = board.drill_last_layer(id) else {
                        continue;
                    };
                    for layer in first..=last {
                        let Some(signal_no) = Self::signal_ordinal(board, layer) else {
                            continue;
                        };
                        let Some(shape) = board.drill_shape(id, layer - first) else {
                            continue;
                        };
                        push_bbox(signal_no, &shape_bbox(&shape));
                    }
                }
                ItemData::ObstacleArea {
                    area,
                    translation,
                    rotation,
                    ..
                } => {
                    let Some(layer) = board.area_layer(id) else {
                        continue;
                    };
                    let Some(signal_no) = Self::signal_ordinal(board, layer) else {
                        continue;
                    };
                    push_bbox(signal_no, &area_bbox(area, *translation, *rotation));
                }
                ItemData::ConductionArea { layer, area, .. } => {
                    // A FILLED area is metal (occupancy) regardless of
                    // the isObstacle flag — foreign traces may route
                    // through a non-obstacle pour, but the pour's own
                    // tracks are still demanded capacity. An unfilled
                    // area is a routing border only — not occupancy.
                    let filled = matches!(
                        board.get(id).map(|entry| &entry.data),
                        Some(ItemData::ConductionArea {
                            is_filled: true,
                            ..
                        })
                    );
                    if !filled {
                        continue;
                    }
                    let Some(signal_no) = Self::signal_ordinal(board, *layer) else {
                        continue;
                    };
                    // Conduction areas parse with a zero transform
                    // (`items/mod.rs` docs) — the direct bbox.
                    push_bbox(signal_no, &area_bbox(area, IntPoint { x: 0, y: 0 }, 0.0));
                }
                // Component outlines (no copper, no routing
                // obstruction), board outlines, and anything else: not
                // occupancy (module docs).
                _ => {}
            }
        }

        // Sort + dedup every touched occupant set, then the overflow
        // vector (layer-major, row-major — the canonical order).
        let mut total_overflow = vec![0u64; signal_layer_count];
        for (signal_no, layer_cells) in occupants.iter_mut().enumerate() {
            let mut sum = 0u64;
            for cell_occupants in layer_cells.iter_mut() {
                if cell_occupants.is_empty() {
                    continue;
                }
                cell_occupants.sort_unstable();
                cell_occupants.dedup();
                let occupancy = cell_occupants.len() as i64;
                sum += (occupancy - cap[signal_no]).max(0) as u64;
            }
            total_overflow[signal_no] = sum;
        }

        Self {
            ll: bbox.ll,
            cell,
            nx,
            ny,
            cap,
            occupants,
            total_overflow,
        }
    }

    /// The empty map (an empty board): every query answers 0 / clear.
    fn empty() -> Self {
        Self {
            ll: IntPoint { x: 0, y: 0 },
            cell: 1,
            nx: 0,
            ny: 0,
            cap: Vec::new(),
            occupants: Vec::new(),
            total_overflow: Vec::new(),
        }
    }

    /// True when the map carries no grid (the empty-board face).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nx == 0 || self.ny == 0 || self.occupants.is_empty()
    }

    /// The square cell side (board units).
    #[must_use]
    pub fn cell_size(&self) -> i64 {
        self.cell
    }

    /// The grid origin (the board bbox lower-left).
    #[must_use]
    pub fn grid_origin(&self) -> (i64, i64) {
        (i64::from(self.ll.x), i64::from(self.ll.y))
    }

    /// The grid dimensions `(nx, ny)`.
    #[must_use]
    pub fn grid_dims(&self) -> (usize, usize) {
        (self.nx, self.ny)
    }

    /// The cell capacity on a signal layer (0 = no such layer).
    #[must_use]
    pub fn capacity(&self, signal_layer: usize) -> i64 {
        self.cap.get(signal_layer).copied().unwrap_or(0)
    }

    /// The signal-layer ordinal of a physical layer index (None when
    /// the layer is not a signal layer).
    pub(crate) fn signal_ordinal(board: &Board, layer_index: i32) -> Option<usize> {
        if layer_index < 0 {
            return None;
        }
        let index = layer_index as usize;
        // The layer itself must BE a signal layer:
        // `get_signal_layer_no` only counts the signal layers strictly
        // BEFORE the index, so a non-signal layer would alias the NEXT
        // signal layer's ordinal — a filled pour on a power layer then
        // rasterized into that signal row's occupancy and suppressed
        // the pattern fast path (witnessed by the g5 mixed-layer pin
        // once its POWER layer carried a synthesized plane; buglog-233).
        if !board
            .layers()
            .layers
            .get(index)
            .is_some_and(|layer| layer.is_signal)
        {
            return None;
        }
        let ordinal = board.layers().get_signal_layer_no(index);
        if ordinal < 0 {
            None
        } else {
            Some(ordinal as usize)
        }
    }

    /// The inclusive cell range of a bbox against this map's grid
    /// (the guide-region and corridor faces).
    #[must_use]
    pub fn cell_range_of(&self, item_bbox: &IntBox) -> (usize, usize, usize, usize) {
        Self::cell_range(item_bbox, &self.grid_bbox(), self.cell, self.nx, self.ny)
    }

    /// The inclusive cell range a bbox touches, clamped to the grid.
    fn cell_range(
        item_bbox: &IntBox,
        grid_bbox: &IntBox,
        cell: i64,
        nx: usize,
        ny: usize,
    ) -> (usize, usize, usize, usize) {
        let clamp_ix = |x: i64| -> usize {
            ((x - i64::from(grid_bbox.ll.x)) / cell)
                .clamp(0, nx as i64 - 1)
                .max(0) as usize
        };
        let clamp_iy = |y: i64| -> usize {
            ((y - i64::from(grid_bbox.ll.y)) / cell)
                .clamp(0, ny as i64 - 1)
                .max(0) as usize
        };
        (
            clamp_ix(i64::from(item_bbox.ll.x)),
            clamp_ix(i64::from(item_bbox.ur.x)),
            clamp_iy(i64::from(item_bbox.ll.y)),
            clamp_iy(i64::from(item_bbox.ur.y)),
        )
    }

    /// The cell index of a point, clamped to the grid.
    #[must_use]
    pub fn cell_of(&self, point: IntPoint) -> (usize, usize) {
        if self.is_empty() {
            return (0, 0);
        }
        let degenerate = IntBox {
            ll: point,
            ur: point,
        };
        let grid_bbox = self.grid_bbox();
        let (ix, _ix1, iy, _iy1) =
            Self::cell_range(&degenerate, &grid_bbox, self.cell, self.nx, self.ny);
        (ix, iy)
    }

    /// The grid's covering bbox (the query-time reconstruction of the
    /// build-time grid extent).
    fn grid_bbox(&self) -> IntBox {
        IntBox {
            ll: self.ll,
            ur: IntPoint {
                x: self.ll.x.saturating_add((self.nx as i64 - 1) as i32),
                y: self.ll.y.saturating_add((self.ny as i64 - 1) as i32),
            },
        }
    }

    /// The occupancy of one cell on one signal layer, excluding the
    /// given net's own copper (the routing-net read).
    #[must_use]
    pub fn occupancy(
        &self,
        ix: usize,
        iy: usize,
        signal_layer: usize,
        exclude_net: Option<i32>,
    ) -> i64 {
        if self.is_empty() || signal_layer >= self.occupants.len() || ix >= self.nx || iy >= self.ny
        {
            return 0;
        }
        self.occupants[signal_layer][iy * self.nx + ix]
            .iter()
            .filter(|occupant| match (occupant, exclude_net) {
                (Occupant::Net(net), Some(exclude)) => *net != exclude,
                _ => true,
            })
            .count() as i64
    }

    /// The overflow of one cell on one signal layer:
    /// `max(0, occupancy(exclude_net) - capacity)`.
    #[must_use]
    pub fn overflow(
        &self,
        ix: usize,
        iy: usize,
        signal_layer: usize,
        exclude_net: Option<i32>,
    ) -> i64 {
        (self.occupancy(ix, iy, signal_layer, exclude_net) - self.capacity(signal_layer)).max(0)
    }

    /// The overflow vector (per signal layer, the total overflow).
    #[must_use]
    pub fn total_overflow(&self) -> &[u64] {
        &self.total_overflow
    }

    /// The coarse corridor check for pattern routing: every cell the
    /// axis-aligned segment touches has spare capacity when the given
    /// net's own copper is excluded (`occupancy < capacity` — the net
    /// could still lay one more track). An empty map answers clear.
    #[must_use]
    pub fn corridor_clear(
        &self,
        from: IntPoint,
        to: IntPoint,
        signal_layer: usize,
        net: Option<i32>,
    ) -> bool {
        if self.is_empty() {
            return true;
        }
        if signal_layer >= self.occupants.len() {
            return false;
        }
        let seg = IntBox {
            ll: IntPoint {
                x: from.x.min(to.x),
                y: from.y.min(to.y),
            },
            ur: IntPoint {
                x: from.x.max(to.x),
                y: from.y.max(to.y),
            },
        };
        let grid_bbox = self.grid_bbox();
        let (ix0, ix1, iy0, iy1) = Self::cell_range(&seg, &grid_bbox, self.cell, self.nx, self.ny);
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                if self.occupancy(ix, iy, signal_layer, net) >= self.capacity(signal_layer) {
                    return false;
                }
            }
        }
        true
    }

    /// The occupancy digest: SHA-256 over canonically ordered rows
    /// `l<signal_no> x<ix> y<iy> n<occupancy>` for every NON-EMPTY cell
    /// (layer-major, y-major, x-minor — the islands.rs digest
    /// discipline). A pure function of board state.
    #[must_use]
    pub fn occupancy_digest(&self) -> String {
        let mut hasher = Sha256::new();
        for (signal_no, layer_cells) in self.occupants.iter().enumerate() {
            for (cell, occupants) in layer_cells.iter().enumerate() {
                if occupants.is_empty() {
                    continue;
                }
                let ix = cell % self.nx;
                let iy = cell / self.nx;
                let row = format!("l{signal_no} x{ix} y{iy} n{}", occupants.len());
                hasher.update(row.as_bytes());
                hasher.update(b"\n");
            }
        }
        format!("{:x}", hasher.finalize())
    }
}
