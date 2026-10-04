//! Pour-island detection — the M6-T6 BEYOND-JAVA advisory face (design
//! :14/:72: upstream has no split/island/fragmentation face at all; the
//! pour is a single Connectable `ConductionArea` item and its copper is
//! never geometrically partitioned). Pure analysis over the board
//! model: NO board mutation, NO search tree, NO route decision. The
//! advisory output feeds the run manifest's `pour_islands` face and the
//! M6-T6 connectivity clamp (`router.plane_island_clamp`, default OFF —
//! dead code at defaults, its route effect measured at T8/T9).
//!
//! ## Geometric contract (M6-T6; there is NO Java oracle for any of it)
//!
//! For every on-board ConductionArea with `is_filled` (a `false` value
//! is a routing border only — nothing to fill, no islands) the
//! detector samples the pour's metal on the INTEGER lattice at 1-unit
//! rows over the pour border's bounding box:
//!
//! * a lattice point (x, y) is POUR METAL iff the pour area contains
//!   it (border-inclusive; a window hole excludes its strict interior
//!   — the `Area::contains_point` face) and no foreign copper carve
//!   covers it. Foreign = an on-board item on the SAME LAYER with nets
//!   disjoint from the pour's: traces, pins, vias, obstacle areas
//!   (keepouts), and other pours — every FOREIGN area carves; a
//!   same-net area never carves (the `copper_sources` net filter —
//!   reconciled with the code at M6-T8, E-10). The carve is
//!   RAW COPPER — no clearance halo (documented v1 face).
//! * stroke model: an axis-aligned trace segment carves its EXACT
//!   rectangle face (half-width h: rows [y1-h, y2+h], columns
//!   [x1-h, x2+h] per orientation); a diagonal segment carves the
//!   per-row band [x(y)-h, x(y)+h] over the segment's own rows,
//!   WITHOUT cap extensions beyond them.
//! * CORNER ASYMMETRY (documented, E-12): the polygon scanline
//!   carries rational corners through an exact-rational (or, mixed,
//!   f64) crossing arithmetic, while the stroke model truncates a
//!   rational trace corner to i64 BEFORE the sweep (`trace_source`)
//!   — a rational corner is thus exact in the area face and
//!   truncating in the stroke face. Parsed boards carry `Int`
//!   corners end to end, so both faces agree there.
//! * regions = 4-CONNECTED components of the metal lattice, built as
//!   an interval union-find over rows. Two consecutive rows bridge
//!   where their metal intervals overlap by at least MIN_BRIDGE_WIDTH
//!   columns — THE boundary constant: a metal channel exactly 1 unit
//!   wide connects; zero width does not; a diagonal 1-wide staircase
//!   does NOT connect under 4-connectivity (the conservative
//!   routability reading; the constant is mutation-verified BOTH
//!   directions at M6-T6 on the `island-spike` crafted worlds).
//! * seeds = same-net pin/via/trace copper overlapping a region; a
//!   metal region without any seed is a FLOATING ISLAND (unreachable
//!   from the net's pins/vias and same-net routing). A component made
//!   only of seed copper outside the pour metal is a SEED-ONLY region:
//!   it carries `has_seed` in the digest, but `region_count` counts
//!   METAL regions only (the digest-includes / region_count-excludes
//!   nuance, E-12).
//!
//! The per-pour digest is the SHA-256 of the canonically ordered
//! region rows (regions in scan order — row ascending, left-to-right;
//! each region carries its bbox, cell count and seed flag), so the
//! same board always yields the same digest bytes regardless of thread
//! count or item iteration order (the manifest-canary face).

use std::collections::{BTreeMap, BTreeSet};

use epic_geometry::circle::Circle;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::point::Point;
use epic_geometry::polygon_shape::PolygonShape;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use sha2::{Digest, Sha256};

use crate::board::{Board, ItemEntry};
use crate::id::ItemId;
use crate::items::{Area, BoardShape, ItemData};

/// The connectivity boundary constant (DNR-16 exact-boundary pin): the
/// minimum integer overlap width (in lattice columns) at which two
/// consecutive rows' intervals union. `1` = a single shared metal
/// column bridges; `0` would additionally bridge merely-TOUCHING
/// intervals (diagonal point contact — killed by the `island-spike`
/// staircase world); `2` would cut the exact 1-unit channel (killed by
/// the gap1 world). Mutation-verified BOTH directions at M6-T6.
const MIN_BRIDGE_WIDTH: i64 = 1;

/// One floating island: the bbox (inclusive lattice coordinates) and
/// the metal cell count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Island {
    /// Inclusive bbox, x0.
    pub x0: i64,
    /// Inclusive bbox, y0.
    pub y0: i64,
    /// Inclusive bbox, x1.
    pub x1: i64,
    /// Inclusive bbox, y1.
    pub y1: i64,
    /// Metal lattice cells in the region.
    pub cells: i64,
}

/// One METAL region's seed attribution — the 152-H
/// `isolated_island_unconnected` face's input: which same-net items'
/// copper claims the region. Upstream (`3011e6e60`) maps items onto
/// AWT islands by CENTER containment; this port maps by LATTICE
/// overlap (a seed item's copper unioned into a region claims it),
/// so a region here is a maximal same-layer same-net copper union —
/// a region severed under this model is severed in any same-layer
/// topology (see the `zone_islands` module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSeeds {
    /// Inclusive lattice bbox, x0.
    pub x0: i64,
    /// Inclusive lattice bbox, y0.
    pub y0: i64,
    /// Inclusive lattice bbox, x1.
    pub x1: i64,
    /// Inclusive lattice bbox, y1.
    pub y1: i64,
    /// The claiming seed item ids, ascending, deduplicated. Empty =
    /// a floating region (the dead-copper arm's subject).
    pub items: Vec<u32>,
}

/// One pour's island face: the region partition summary + the floating
/// islands + the deterministic geometry digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PourIslands {
    /// The ConductionArea item id.
    pub pour_item_id: u32,
    /// The pour's first net's name (empty when the net row is absent).
    pub net: String,
    /// The pour's first net NUMBER (0 when the pour carries no net —
    /// upstream `ca.netCount() > 0 ? ca.getNetNumber(0) : 0`).
    pub net_number: i32,
    /// The pour's 0-based layer.
    pub layer: i32,
    /// Number of metal regions (0 = the pour is fully covered).
    pub region_count: usize,
    /// Number of floating islands (regions without any seed).
    pub island_count: usize,
    /// The floating islands, in canonical region order.
    pub islands: Vec<Island>,
    /// Every METAL region's seed attribution, canonical scan order
    /// (the same enumeration the digest's `R{idx}` rows use, seed-only
    /// components skipped). For a metal region, `items` is non-empty
    /// iff the region is seeded (the `has_seed` invariant — pinned).
    pub region_seeds: Vec<RegionSeeds>,
    /// SHA-256 hex over the canonical per-region rows (module docs).
    pub digest: String,
}

/// An inclusive integer x-interval at one lattice row.
type Interval = (i64, i64);

// ---------------------------------------------------------------------------
// row-interval primitives (the lattice-membership face)
// ---------------------------------------------------------------------------

/// The exact integer x covered by an IntBox at row `y`.
fn int_box_row_interval(box_: &IntBox, y: i64) -> Option<Interval> {
    if y < i64::from(box_.ll.y) || y > i64::from(box_.ur.y) {
        return None;
    }
    Some((i64::from(box_.ll.x), i64::from(box_.ur.x)))
}

/// The exact integer x interval of a convex IntOctagon at row `y`: the
/// intersection of the 8 half-planes, in the Java x-axis intercept
/// convention (a -45 degree border is `x + y = c`, a +45 degree border
/// is `x - y = c`).
fn int_octagon_row_interval(oct: &IntOctagon, y: i64) -> Option<Interval> {
    let left = i64::from(oct.left_x);
    let bottom = i64::from(oct.bottom_y);
    let right = i64::from(oct.right_x);
    let top = i64::from(oct.top_y);
    if y < bottom || y > top {
        return None;
    }
    let ul = i64::from(oct.upper_left_diagonal_x);
    let lr = i64::from(oct.lower_right_diagonal_x);
    let ll = i64::from(oct.lower_left_diagonal_x);
    let ur = i64::from(oct.upper_right_diagonal_x);
    // Left bounds: -45 degree upper-left border `x >= ul - y`; +45
    // degree lower-left border `x >= ll + y`.
    let lo = left.max(ul - y).max(ll + y);
    // Right bounds: -45 degree lower-right border `x <= lr - y`; +45
    // degree upper-right border `x <= ur + y`.
    let hi = right.min(lr - y).min(ur + y);
    if lo > hi { None } else { Some((lo, hi)) }
}

/// One polygon corner flattened for the scanline: exact-integer when
/// the source point is Point::Int (the f64 view is then exact too); a
/// rational corner carries its f64 view (deterministic IEEE
/// arithmetic; the exact-integer path is the one the boundary worlds
/// and every real pour polygon exercise).
#[derive(Debug, Clone, Copy)]
struct Corner {
    x: i64,
    y: i64,
    x_f: f64,
    y_f: f64,
    exact: bool,
}

fn flatten_corner(point: &Point) -> Corner {
    match point {
        Point::Int(p) => Corner {
            x: i64::from(p.x),
            y: i64::from(p.y),
            x_f: f64::from(p.x),
            y_f: f64::from(p.y),
            exact: true,
        },
        Point::Rational(r) => {
            let f = r.to_float();
            Corner {
                x: f.x as i64,
                y: f.y as i64,
                x_f: f.x,
                y_f: f.y,
                exact: false,
            }
        }
    }
}

/// An exact rational crossing value (num/den, den > 0, i128
/// arithmetic — board coordinates never come near the overflow face).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rational {
    num: i128,
    den: i128,
}

impl Rational {
    fn floor(self) -> i64 {
        i64::try_from(self.num.div_euclid(self.den)).unwrap_or(0)
    }

    fn ceil(self) -> i64 {
        let floored = self.num.div_euclid(self.den);
        let rem = self.num.rem_euclid(self.den);
        let value = if rem == 0 { floored } else { floored + 1 };
        i64::try_from(value).unwrap_or(0)
    }
}

/// Even-odd scanline over a closed polygon given as a corner slice
/// (NOT repeated first at the end). Returns the covered integer x
/// intervals at row `y` under point-sampling semantics: a lattice
/// point on the polygon border counts as covered (the
/// border-inclusive `contains` face) — the covered span is
/// [ceil(left crossing), floor(right crossing)]. Horizontal edges are
/// skipped (their endpoints arrive via the adjacent edges' crossings);
/// non-horizontal edges cross on the half-open span [y_lo, y_hi) PLUS
/// the global-max row (closed there — otherwise the polygon's top
/// border row would come out empty while its border points count as
/// contained). Exact-integer when every corner is integral; a single
/// rational corner switches the whole polygon to the deterministic f64
/// face.
fn polygon_row_intervals(corners: &[Corner], y: i64) -> Vec<Interval> {
    if corners.len() < 3 {
        return Vec::new();
    }
    let poly_max_y = corners.iter().map(|corner| corner.y).max().unwrap_or(0);
    let poly_max_f = corners
        .iter()
        .map(|corner| corner.y_f)
        .fold(f64::NEG_INFINITY, f64::max);
    let exact = corners.iter().all(|corner| corner.exact);
    let mut crossings_f: Vec<f64> = Vec::new();
    let mut crossings_r: Vec<Rational> = Vec::new();
    for i in 0..corners.len() {
        let a = corners[i];
        let b = corners[(i + 1) % corners.len()];
        if exact {
            if a.y == b.y {
                continue;
            }
            let (y_lo, y_hi) = if a.y <= b.y { (a.y, b.y) } else { (b.y, a.y) };
            let on_span = y >= y_lo && y < y_hi;
            let on_closed_max = y == y_hi && y_hi == poly_max_y;
            if !on_span && !on_closed_max {
                continue;
            }
            let num = i128::from(b.x - a.x) * i128::from(y - a.y);
            let den = i128::from(b.y - a.y);
            let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
            crossings_r.push(Rational {
                num: i128::from(a.x) * den + num,
                den,
            });
        } else {
            if a.y_f == b.y_f {
                continue;
            }
            let (y_lo, y_hi) = if a.y_f <= b.y_f {
                (a.y_f, b.y_f)
            } else {
                (b.y_f, a.y_f)
            };
            let yf = y as f64;
            let on_span = yf >= y_lo && yf < y_hi;
            let on_closed_max = yf == y_hi && y_hi == poly_max_f;
            if !on_span && !on_closed_max {
                continue;
            }
            let t = (yf - a.y_f) / (b.y_f - a.y_f);
            crossings_f.push(a.x_f + t * (b.x_f - a.x_f));
        }
    }
    if !exact {
        // The float face (any rational corner switches the whole
        // polygon; deterministic IEEE arithmetic).
        crossings_f.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::with_capacity(crossings_f.len() / 2);
        for pair in crossings_f.chunks_exact(2) {
            let lo = pair[0].ceil() as i64;
            let hi = pair[1].floor() as i64;
            if lo <= hi {
                out.push((lo, hi));
            }
        }
        return out;
    }
    crossings_r.sort_by(|a, b| (a.num * b.den).cmp(&(b.num * a.den)));
    let mut out = Vec::with_capacity(crossings_r.len() / 2);
    for pair in crossings_r.chunks_exact(2) {
        let lo = pair[0].ceil();
        let hi = pair[1].floor();
        if lo <= hi {
            out.push((lo, hi));
        }
    }
    out
}

/// The covered integer x intervals of a PolygonShape at row `y`.
fn polygon_shape_row_intervals(polygon: &PolygonShape, y: i64) -> Vec<Interval> {
    let n = polygon.border_line_count();
    let mut corners = Vec::with_capacity(n);
    for i in 0..n {
        let idx = i32::try_from(i).unwrap_or(0);
        corners.push(flatten_corner(&polygon.corner(idx)));
    }
    polygon_row_intervals(&corners, y)
}

/// The covered integer x intervals of a Simplex at row `y` (its 3
/// corners through the same scanline).
fn simplex_row_intervals(simplex: &Simplex, y: i64) -> Vec<Interval> {
    let n = simplex.border_line_count();
    let mut corners = Vec::with_capacity(n);
    for i in 0..n {
        let idx = i32::try_from(i).unwrap_or(0);
        corners.push(flatten_corner(&simplex.corner(idx)));
    }
    polygon_row_intervals(&corners, y)
}

/// The covered integer x interval of a Circle at row `y` (f64 face —
/// deterministic per build; circles appear as via padstacks).
fn circle_row_interval(circle: &Circle, y: i64) -> Option<Interval> {
    let dy = y - i64::from(circle.center.y);
    let r = i64::from(circle.radius);
    if dy.abs() > r {
        return None;
    }
    let half = ((r * r - dy * dy) as f64).sqrt();
    let cx = i64::from(circle.center.x);
    Some((cx - half.floor() as i64, cx + half.floor() as i64))
}

/// The covered integer x intervals of a board shape at row `y`.
fn shape_row_intervals(shape: &BoardShape, y: i64) -> Vec<Interval> {
    match shape {
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(box_))) => {
            int_box_row_interval(box_, y).map_or_else(Vec::new, |iv| vec![iv])
        }
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct))) => {
            int_octagon_row_interval(oct, y).map_or_else(Vec::new, |iv| vec![iv])
        }
        BoardShape::Tile(TileShape::Simplex(simplex)) => simplex_row_intervals(simplex, y),
        BoardShape::PolygonShape(polygon) => polygon_shape_row_intervals(polygon, y),
        BoardShape::Circle(circle) => {
            circle_row_interval(circle, y).map_or_else(Vec::new, |iv| vec![iv])
        }
    }
}

/// The area's covered intervals at row `y`: the border (border points
/// included) minus each hole's STRICT interior (a lattice point on a
/// hole's border stays in the area — the `Area::contains_point` face).
fn area_row_intervals(area: &Area, y: i64) -> Vec<Interval> {
    let mut intervals = shape_row_intervals(&area.border, y);
    for hole in &area.holes {
        for hole_iv in shape_row_intervals(hole, y) {
            // Strict interior: integer x with lo < x < hi.
            let strict = (hole_iv.0 + 1, hole_iv.1 - 1);
            if strict.0 > strict.1 {
                continue;
            }
            intervals = subtract_interval(&intervals, strict);
        }
    }
    intervals
}

/// Subtracts one interval from a sorted, disjoint interval list.
fn subtract_interval(intervals: &[Interval], cut: Interval) -> Vec<Interval> {
    let mut out = Vec::with_capacity(intervals.len() + 1);
    for &(lo, hi) in intervals {
        if hi < cut.0 || lo > cut.1 {
            out.push((lo, hi));
            continue;
        }
        if lo < cut.0 {
            out.push((lo, cut.0 - 1));
        }
        if hi > cut.1 {
            out.push((cut.1 + 1, hi));
        }
    }
    out
}

/// Subtracts a sorted interval list from another (both sorted,
/// disjoint).
fn subtract_intervals(intervals: &[Interval], cuts: &[Interval]) -> Vec<Interval> {
    let mut out = intervals.to_vec();
    for &cut in cuts {
        out = subtract_interval(&out, cut);
    }
    out
}

/// A trace stroke's covered intervals at row `y` (the module-docs
/// stroke model): axis-aligned segments contribute their exact
/// rectangle face; diagonal segments the per-row band [x(y)-h,
/// x(y)+h] over the segment's own rows (no cap extensions).
fn trace_row_intervals(xs: &[i64], ys: &[i64], half_width: i64, y: i64) -> Vec<Interval> {
    let mut out = Vec::new();
    for i in 0..xs.len().saturating_sub(1) {
        let (x1, y1) = (xs[i], ys[i]);
        let (x2, y2) = (xs[i + 1], ys[i + 1]);
        let (x_lo, x_hi) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        let (y_lo, y_hi) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        if y1 == y2 {
            // Horizontal: the exact rectangle face rows [y1-h, y1+h].
            if y < y1 - half_width || y > y1 + half_width {
                continue;
            }
            out.push((x_lo - half_width, x_hi + half_width));
        } else if x1 == x2 {
            // Vertical: the exact rectangle face rows [y1-h, y2+h].
            if y < y_lo - half_width || y > y_hi + half_width {
                continue;
            }
            out.push((x1 - half_width, x1 + half_width));
        } else {
            // Diagonal: the band over the segment's own rows.
            if y < y_lo || y > y_hi {
                continue;
            }
            let xf = f64::from(x1 as i32)
                + (f64::from(y as i32) - f64::from(y1 as i32))
                    * (f64::from(x2 as i32) - f64::from(x1 as i32))
                    / (f64::from(y2 as i32) - f64::from(y1 as i32));
            let lo = (xf - half_width as f64).floor() as i64;
            let hi = (xf + half_width as f64).floor() as i64;
            out.push((lo.min(hi), lo.max(hi)));
        }
    }
    out.sort_unstable();
    out
}

// ---------------------------------------------------------------------------
// the detector
// ---------------------------------------------------------------------------

/// One copper source resolved for a pour scan (foreign obstacles carve,
/// same-net pins/vias/traces seed).
struct CopperSource {
    /// Bounding box — the pre-filter against the pour's scan range
    /// (`detect_pour_islands_for`'s `ordered` step; E-12 doc: there is
    /// no active-set sweep, the bbox is the only pre-filter).
    x_lo: i64,
    y_lo: i64,
    x_hi: i64,
    y_hi: i64,
    shapes: SourceShapes,
    /// True = same-net seed copper (marks regions reachable), false =
    /// foreign copper (carves).
    seed: bool,
    /// The contributing item (seed sources only — the 152-H
    /// unconnected-island face's attribution input; obstacles carry
    /// `None`).
    item: Option<ItemId>,
}

enum SourceShapes {
    /// A trace stroke (flattened corners + half width).
    Trace {
        xs: Vec<i64>,
        ys: Vec<i64>,
        half_width: i64,
    },
    /// An area (border + holes).
    Area(Area),
    /// Drill/padstack shapes (absolute).
    Shapes(Vec<BoardShape>),
}

impl CopperSource {
    fn covers_row(&self, y: i64) -> bool {
        y >= self.y_lo && y <= self.y_hi
    }

    fn covered(&self, y: i64) -> Vec<Interval> {
        match &self.shapes {
            SourceShapes::Trace { xs, ys, half_width } => {
                trace_row_intervals(xs, ys, *half_width, y)
            }
            SourceShapes::Area(area) => area_row_intervals(area, y),
            SourceShapes::Shapes(shapes) => {
                let mut out = Vec::new();
                for shape in shapes {
                    out.extend(shape_row_intervals(shape, y));
                }
                out.sort_unstable();
                out
            }
        }
    }
}

fn shape_bbox(shape: &BoardShape) -> (i64, i64, i64, i64) {
    let bbox: IntBox = shape.bounding_box();
    (
        i64::from(bbox.ll.x),
        i64::from(bbox.ll.y),
        i64::from(bbox.ur.x),
        i64::from(bbox.ur.y),
    )
}

/// Collects the drill/padstack shapes of a pin/via on `layer`
/// (absolute shapes). COUNT-BASED (the E-12 fix of the hard `0..256`
/// cap): a padstack's shape count is its drill span `last - first + 1`
/// — read through `item_shape_layer_read`, whose pin/via arm CLAMPS
/// the index into `[0, last - first]` (it never answers None for a
/// drill item, so any unbounded walk is wrong; index 0 answers
/// `first`, `i32::MAX` answers `last`).
fn drill_shapes_on_layer(board: &Board, id: ItemId, layer: i32) -> Vec<BoardShape> {
    let Some(first) = board.item_shape_layer_read(id, 0) else {
        return Vec::new();
    };
    let Some(last) = board.item_shape_layer_read(id, i32::MAX) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..=(last - first) {
        if board.item_shape_layer_read(id, index) == Some(layer)
            && let Some(shape) = board.drill_shape(id, index)
        {
            out.push(shape);
        }
    }
    out
}

/// True when the item shares a net with the pour.
fn is_same_net(entry: &ItemEntry, pour_nets: &[i32]) -> bool {
    entry.nets.iter().any(|net| pour_nets.contains(net))
}

/// Builds the copper sources for a pour scan: foreign copper carves,
/// same-net pins/vias/traces seed. `seed_filter` = `Some(ids)` restricts
/// the SEED face to those items (a same-net item outside the filter is
/// ignored entirely — neither seed nor carve; foreign copper carves
/// regardless). Outline items are deliberately NOT sources (the v1
/// contract: generated outline keepouts are a router action, not parse
/// state).
fn copper_sources(
    board: &Board,
    pour_id: ItemId,
    pour_nets: &[i32],
    layer: i32,
    seed_filter: Option<&[ItemId]>,
) -> Vec<CopperSource> {
    let mut sources = Vec::new();
    for entry in board.iter_ascending() {
        if !entry.on_the_board || entry.id == pour_id {
            continue;
        }
        let same_net = is_same_net(entry, pour_nets);
        let foreign = !same_net;
        match &entry.data {
            ItemData::Trace { .. } => {
                let in_seed_filter =
                    seed_filter.is_none() || seed_filter.is_some_and(|ids| ids.contains(&entry.id));
                if same_net && !in_seed_filter {
                    continue;
                }
                if let Some(mut source) = trace_source(board, entry, layer, same_net) {
                    if same_net {
                        source.item = Some(entry.id);
                    }
                    sources.push(source);
                }
            }
            ItemData::ObstacleArea {
                area,
                layer: item_layer,
                ..
            } => {
                if *item_layer == layer && foreign {
                    sources.push(area_source(area, foreign));
                }
            }
            ItemData::ConductionArea {
                area,
                layer: item_layer,
                ..
            } => {
                if *item_layer == layer && foreign {
                    sources.push(area_source(area, foreign));
                }
            }
            ItemData::Pin { .. } | ItemData::Via { .. } => {
                let in_seed_filter =
                    seed_filter.is_none() || seed_filter.is_some_and(|ids| ids.contains(&entry.id));
                if same_net && !in_seed_filter {
                    continue;
                }
                let shapes = drill_shapes_on_layer(board, entry.id, layer);
                if !shapes.is_empty() {
                    let mut source = drill_source(shapes, same_net);
                    if same_net {
                        source.item = Some(entry.id);
                    }
                    sources.push(source);
                }
            }
            _ => {}
        }
    }
    sources
}

/// The trace stroke source (None: wrong layer or missing faces).
fn trace_source(board: &Board, entry: &ItemEntry, layer: i32, seed: bool) -> Option<CopperSource> {
    let polyline = board.trace_polyline(entry.id)?;
    let half_width = i64::from(board.trace_half_width(entry.id)?);
    if board.trace_layer(entry.id)? != layer {
        return None;
    }
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for corner in polyline.corners() {
        match corner {
            Point::Int(p) => {
                xs.push(i64::from(p.x));
                ys.push(i64::from(p.y));
            }
            Point::Rational(r) => {
                let f = r.to_float();
                xs.push(f.x as i64);
                ys.push(f.y as i64);
            }
        }
    }
    Some(build_source(
        SourceShapes::Trace { xs, ys, half_width },
        seed,
    ))
}

/// The area source (keepout or foreign pour). Foreign areas carve;
/// seed stays false (same-net areas are skipped by the caller).
fn area_source(area: &Area, _foreign: bool) -> CopperSource {
    build_source(SourceShapes::Area(area.clone()), false)
}

/// The drill source (a pin or via's shapes on the pour's layer).
fn drill_source(shapes: Vec<BoardShape>, seed: bool) -> CopperSource {
    build_source(SourceShapes::Shapes(shapes), seed)
}

/// Builds a source with its bbox computed from the shapes.
fn build_source(shapes: SourceShapes, seed: bool) -> CopperSource {
    let (mut x_lo, mut y_lo, mut x_hi, mut y_hi) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    match &shapes {
        SourceShapes::Trace { xs, ys, half_width } => {
            for (&x, &y) in xs.iter().zip(ys.iter()) {
                x_lo = x_lo.min(x - half_width);
                x_hi = x_hi.max(x + half_width);
                y_lo = y_lo.min(y - half_width);
                y_hi = y_hi.max(y + half_width);
            }
        }
        SourceShapes::Area(area) => {
            let (bx0, by0, bx1, by1) = shape_bbox(&area.border);
            x_lo = x_lo.min(bx0);
            y_lo = y_lo.min(by0);
            x_hi = x_hi.max(bx1);
            y_hi = y_hi.max(by1);
        }
        SourceShapes::Shapes(shapes) => {
            for shape in shapes {
                let (bx0, by0, bx1, by1) = shape_bbox(shape);
                x_lo = x_lo.min(bx0);
                y_lo = y_lo.min(by0);
                x_hi = x_hi.max(bx1);
                y_hi = y_hi.max(by1);
            }
        }
    }
    CopperSource {
        x_lo,
        y_lo,
        x_hi,
        y_hi,
        shapes,
        seed,
        item: None,
    }
}

/// The union-find over interval nodes (path-halving; deterministic:
/// the lower root wins).
struct Uf {
    parent: Vec<usize>,
}

impl Uf {
    fn new() -> Self {
        Self { parent: Vec::new() }
    }

    fn push(&mut self) -> usize {
        let idx = self.parent.len();
        self.parent.push(idx);
        idx
    }

    fn find(&mut self, mut idx: usize) -> usize {
        while self.parent[idx] != idx {
            self.parent[idx] = self.parent[self.parent[idx]];
            idx = self.parent[idx];
        }
        idx
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra < rb {
            self.parent[rb] = ra;
        } else if rb < ra {
            self.parent[ra] = rb;
        }
    }
}

/// One node's per-row record.
#[derive(Debug, Clone)]
struct RowNode {
    interval: Interval,
    seed: bool,
    uf_idx: usize,
}

/// Bridges two consecutive rows where the intervals' overlap width is
/// at least MIN_BRIDGE_WIDTH.
fn bridge_rows(uf: &mut Uf, prev: &[RowNode], cur: &[RowNode]) {
    for a in prev {
        for b in cur {
            let overlap = a.interval.1.min(b.interval.1) - a.interval.0.max(b.interval.0) + 1;
            if overlap >= MIN_BRIDGE_WIDTH {
                uf.union(a.uf_idx, b.uf_idx);
            }
        }
    }
}

/// Unions this row's seed nodes with every node they overlap (metal
/// and seed alike; metal intervals are disjoint by construction, so
/// only seeds can bridge in-row).
fn union_row_seeds(uf: &mut Uf, row: &[RowNode]) {
    for i in 0..row.len() {
        if !row[i].seed {
            continue;
        }
        for j in 0..row.len() {
            if i == j {
                continue;
            }
            let (a, b) = (row[i].interval, row[j].interval);
            let overlap = a.1.min(b.1) - a.0.max(b.0) + 1;
            if overlap >= MIN_BRIDGE_WIDTH {
                uf.union(row[i].uf_idx, row[j].uf_idx);
            }
        }
    }
}

/// Detects the floating islands of EVERY filled pour on the board
/// (parse-time or post-route — the detector is pure over the live
/// board). Returns pours in the board's canonical (descending-id)
/// enumeration order.
#[must_use]
pub fn detect_pour_islands(board: &Board) -> Vec<PourIslands> {
    let mut out = Vec::new();
    for entry in board.iter_ascending() {
        if !entry.on_the_board {
            continue;
        }
        if let ItemData::ConductionArea {
            layer,
            area,
            is_filled: true,
            ..
        } = &entry.data
            && let Some((face, _seeded)) =
                detect_pour_islands_for(board, entry.id, *layer, area, None)
        {
            out.push(face);
        }
    }
    out
}

/// True when the pour has metal regions and EVERY region is floating
/// (the M6-T6 pour-level clamp predicate; the REGION-LEVEL gate lives
/// in [`pour_region_seeded_by`]). A covered pour (no regions), a pour
/// with at least one seeded region, and a non-pour id are all `false`.
#[must_use]
pub fn pour_fully_floating(board: &Board, pour_id: ItemId) -> bool {
    let Some(entry) = board.get(pour_id) else {
        return false;
    };
    let ItemData::ConductionArea {
        layer,
        area,
        is_filled,
        ..
    } = &entry.data
    else {
        return false;
    };
    if !is_filled {
        return false;
    }
    let Some((face, _seeded)) = detect_pour_islands_for(board, pour_id, *layer, area, None) else {
        return false;
    };
    face.region_count > 0 && face.island_count == face.region_count
}

/// The REGION-LEVEL clamp predicate (M6-T8): true when at least one of
/// the pour's regions carries seed copper FROM `seed_items` — the
/// connected set's own pins/vias/traces overlapping uncarved pour
/// metal. A pour REGION not carrying the net's seed copper cannot
/// answer CONNECTED_TO_PLANE (`connection_router::plane_connected_gate`,
/// behind `router.plane_island_clamp`). A covered pour (no regions at
/// all), an unfilled area, and a non-pour id answer `false`.
pub fn pour_region_seeded_by(board: &Board, pour_id: ItemId, seed_items: &[ItemId]) -> bool {
    let Some(entry) = board.get(pour_id) else {
        return false;
    };
    let ItemData::ConductionArea {
        layer,
        area,
        is_filled: true,
        ..
    } = &entry.data
    else {
        return false;
    };
    let Some((_face, seeded)) =
        detect_pour_islands_for(board, pour_id, *layer, area, Some(seed_items))
    else {
        return false;
    };
    seeded > 0
}

/// The per-pour scan: the row sweep + interval union-find + the
/// canonical digest. Returns the face plus the number of regions
/// carrying seed copper (seed-only regions included; the M6-T8
/// region-level clamp's input).
fn detect_pour_islands_for(
    board: &Board,
    pour_id: ItemId,
    layer: i32,
    area: &Area,
    seed_filter: Option<&[ItemId]>,
) -> Option<(PourIslands, usize)> {
    let entry = board.get(pour_id)?;
    let pour_nets = entry.nets.clone();
    let net_name = pour_nets
        .first()
        .and_then(|net_no| board.rules().nets.get(*net_no))
        .map_or_else(String::new, |net| net.name.clone());
    let bbox = area.border.bounding_box();
    let (x_min, y_min) = (i64::from(bbox.ll.x), i64::from(bbox.ll.y));
    let (x_max, y_max) = (i64::from(bbox.ur.x), i64::from(bbox.ur.y));
    if x_min > x_max || y_min > y_max {
        return None;
    }
    let sources = copper_sources(board, pour_id, &pour_nets, layer, seed_filter);
    let mut ordered: Vec<&CopperSource> = sources
        .iter()
        .filter(|source| source.x_hi >= x_min && source.x_lo <= x_max)
        .collect();
    ordered.sort_by(|a, b| (a.y_lo, a.x_lo, a.seed).cmp(&(b.y_lo, b.x_lo, b.seed)));
    let mut uf = Uf::new();
    let mut prev_row: Vec<RowNode> = Vec::new();
    let mut nodes_meta: Vec<(Interval, bool, i64)> = Vec::new();
    // Seed-node attribution (152-H): (uf index, contributing item) per
    // final seed node. `seed_cuts` merges all sources' intervals per
    // row, losing per-source identity; intersecting each source's raw
    // intervals with the final (obstacle-carved) nodes recovers it
    // exactly — a source's copper cell either lands in a seed node or
    // was carved away.
    let mut seed_item_nodes: Vec<(usize, ItemId)> = Vec::new();
    for y in y_min..=y_max {
        let mut obstacle_cuts: Vec<Interval> = Vec::new();
        let mut seed_cuts: Vec<Interval> = Vec::new();
        for source in &ordered {
            if !source.covers_row(y) {
                continue;
            }
            if source.seed {
                seed_cuts.extend(source.covered(y));
            } else {
                obstacle_cuts.extend(source.covered(y));
            }
        }
        obstacle_cuts.sort_unstable();
        seed_cuts.sort_unstable();
        let metal = subtract_intervals(&area_row_intervals(area, y), &obstacle_cuts);
        let seeds = subtract_intervals(&seed_cuts, &obstacle_cuts);
        if metal.is_empty() && seeds.is_empty() {
            prev_row.clear();
            continue;
        }
        let mut cur_row: Vec<RowNode> = Vec::with_capacity(metal.len() + seeds.len());
        for &(lo, hi) in &metal {
            let uf_idx = uf.push();
            cur_row.push(RowNode {
                interval: (lo, hi),
                seed: false,
                uf_idx,
            });
        }
        for &(lo, hi) in &seeds {
            let uf_idx = uf.push();
            cur_row.push(RowNode {
                interval: (lo, hi),
                seed: true,
                uf_idx,
            });
            for source in &ordered {
                let Some(item) = source.item else { continue };
                if !source.covers_row(y) {
                    continue;
                }
                let touches = source
                    .covered(y)
                    .iter()
                    .any(|&(slo, shi)| slo.max(lo) <= shi.min(hi));
                if touches {
                    seed_item_nodes.push((uf_idx, item));
                }
            }
        }
        union_row_seeds(&mut uf, &cur_row);
        bridge_rows(&mut uf, &prev_row, &cur_row);
        for node in &cur_row {
            nodes_meta.push((node.interval, node.seed, y));
        }
        prev_row = cur_row;
    }
    Some(classify_and_digest(
        pour_id,
        &net_name,
        pour_nets.first().copied().unwrap_or(0),
        layer,
        &mut uf,
        nodes_meta,
        seed_item_nodes,
    ))
}

/// A region accumulator (canonical scan-order classification).
struct RegionAcc {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    cells: i64,
    has_metal: bool,
    has_seed: bool,
    /// Seed item attribution (152-H) — `has_seed` carries the copper
    /// truth, this carries WHO.
    items: BTreeSet<u32>,
}

/// Classifies the union-find into regions and builds the canonical
/// digest (regions in scan order; seed-only components included).
/// Node k in `nodes_meta` was the k-th union-find push, so its
/// uf index is k. The digest input carries the `v1` schema prefix
/// (E-11: the version token — verified rotation-free for the gate set
/// at M6-T8: both gate fixtures' manifests carry an EMPTY pour_islands
/// face, and no committed baseline or events-golden carries any islands
/// digest). `seed_item_nodes` is folded into `region_seeds` (the
/// digest stays geometry-only — attribution NEVER touches it).
fn classify_and_digest(
    pour_id: ItemId,
    net_name: &str,
    net_number: i32,
    layer: i32,
    uf: &mut Uf,
    nodes_meta: Vec<(Interval, bool, i64)>,
    seed_item_nodes: Vec<(usize, ItemId)>,
) -> (PourIslands, usize) {
    let mut root_to_region: BTreeMap<usize, usize> = BTreeMap::new();
    let mut regions: Vec<RegionAcc> = Vec::new();
    for (node_idx, (interval, seed, y)) in nodes_meta.iter().enumerate() {
        let root = uf.find(node_idx);
        let region = *root_to_region.entry(root).or_insert_with(|| {
            regions.push(RegionAcc {
                x0: interval.0,
                y0: *y,
                x1: interval.1,
                y1: *y,
                cells: 0,
                has_metal: false,
                has_seed: false,
                items: BTreeSet::new(),
            });
            regions.len() - 1
        });
        let acc = &mut regions[region];
        acc.x0 = acc.x0.min(interval.0);
        acc.y0 = acc.y0.min(*y);
        acc.x1 = acc.x1.max(interval.1);
        acc.y1 = acc.y1.max(*y);
        if *seed {
            acc.has_seed = true;
        } else {
            acc.has_metal = true;
            acc.cells += interval.1 - interval.0 + 1;
        }
    }
    for (uf_idx, item) in seed_item_nodes {
        if let Some(&region) = root_to_region.get(&uf.find(uf_idx)) {
            regions[region].items.insert(item.get());
        }
    }
    let region_count = regions.iter().filter(|r| r.has_metal).count();
    let islands: Vec<Island> = regions
        .iter()
        .filter(|r| r.has_metal && !r.has_seed)
        .map(|r| Island {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
            cells: r.cells,
        })
        .collect();
    let region_seeds: Vec<RegionSeeds> = regions
        .iter()
        .filter(|r| r.has_metal)
        .map(|r| RegionSeeds {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
            items: r.items.iter().copied().collect(),
        })
        .collect();
    let mut digest_input = format!(
        "v1\npour={} net={net_name} layer={layer} regions={region_count}\n",
        pour_id.get()
    );
    for (idx, r) in regions.iter().enumerate() {
        digest_input.push_str(&format!(
            "R{idx} bbox={},{},{},{} cells={} metal={} seed={}\n",
            r.x0, r.y0, r.x1, r.y1, r.cells, r.has_metal, r.has_seed
        ));
    }
    let digest = format!("{:x}", Sha256::digest(digest_input.as_bytes()));
    let seeded_regions = regions.iter().filter(|r| r.has_seed).count();
    (
        PourIslands {
            pour_item_id: pour_id.get(),
            net: net_name.to_string(),
            net_number,
            layer,
            region_count,
            island_count: islands.len(),
            islands,
            region_seeds,
            digest,
        },
        seeded_regions,
    )
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_point::IntPoint;

    /// The interval primitives' contract, checked against the shapes'
    /// own contains_point on exhaustive small-world samples.
    #[test]
    fn row_intervals_agree_with_contains_point() {
        let box_ = IntBox::new(IntPoint::new(-10, -5), IntPoint::new(20, 15));
        let box_tile = TileShape::RegularTileShape(RegularTileShape::IntBox(box_));
        for y in -6..=16 {
            let (lo, hi) = int_box_row_interval(&box_, y).unwrap_or((1, 0));
            for x in -12..=22 {
                let contained = box_tile.contains_point(&Point::Int(IntPoint::new(x, y as i32)));
                assert_eq!(
                    contained,
                    i64::from(x) >= lo && i64::from(x) <= hi,
                    "IntBox row {y} col {x}"
                );
            }
        }
        let oct = IntOctagon::new(0, 0, 40, 40, 10, 30, 10, 30);
        let oct_tile = TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct));
        for y in -1..=41 {
            let (lo, hi) = int_octagon_row_interval(&oct, y).unwrap_or((1, 0));
            for x in -2..=42 {
                let contained = oct_tile.contains_point(&Point::Int(IntPoint::new(x, y as i32)));
                assert_eq!(
                    contained,
                    i64::from(x) >= lo && i64::from(x) <= hi,
                    "IntOctagon row {y} col {x}"
                );
            }
        }
    }

    /// The integer polygon scanline on a rectangle and a triangle.
    #[test]
    fn polygon_scanline_matches_point_sampling() {
        let rect = [
            Corner {
                x: 0,
                y: 0,
                x_f: 0.0,
                y_f: 0.0,
                exact: true,
            },
            Corner {
                x: 30,
                y: 0,
                x_f: 30.0,
                y_f: 0.0,
                exact: true,
            },
            Corner {
                x: 30,
                y: 20,
                x_f: 30.0,
                y_f: 20.0,
                exact: true,
            },
            Corner {
                x: 0,
                y: 20,
                x_f: 0.0,
                y_f: 20.0,
                exact: true,
            },
        ];
        for y in 0..=20 {
            assert_eq!(
                polygon_row_intervals(&rect, y),
                vec![(0, 30)],
                "rect row {y}"
            );
        }
        assert!(polygon_row_intervals(&rect, -1).is_empty());
        assert!(polygon_row_intervals(&rect, 21).is_empty());
        let tri = [
            Corner {
                x: 0,
                y: 0,
                x_f: 0.0,
                y_f: 0.0,
                exact: true,
            },
            Corner {
                x: 20,
                y: 0,
                x_f: 20.0,
                y_f: 0.0,
                exact: true,
            },
            Corner {
                x: 0,
                y: 20,
                x_f: 0.0,
                y_f: 20.0,
                exact: true,
            },
        ];
        for y in 0..20 {
            assert_eq!(
                polygon_row_intervals(&tri, y),
                vec![(0, 20 - y)],
                "tri row {y}"
            );
        }
        assert_eq!(polygon_row_intervals(&tri, 20), vec![(0, 0)], "apex row");
    }

    /// The circle row interval covers every exactly-contained sample
    /// (the exact containment face dx^2 + dy^2 <= r^2; the interval's
    /// floor faces bracket it).
    #[test]
    fn circle_row_interval_covers_contained_samples() {
        let circle = Circle::new(IntPoint::new(100, 200), 30);
        for y in 168..=232 {
            let (lo, hi) = circle_row_interval(&circle, y).unwrap_or((1, 0));
            for x in 68..=132 {
                let dy = y - i64::from(circle.center.y);
                let dx = x - i64::from(circle.center.x);
                if dx * dx + dy * dy <= i64::from(circle.radius) * i64::from(circle.radius) {
                    assert!(x >= lo && x <= hi, "circle row {y} col {x}");
                }
            }
        }
    }

    /// The subtract primitives: disjointness and order invariants.
    #[test]
    fn interval_subtraction_keeps_sorted_disjoint() {
        let base = vec![(0, 10), (20, 30), (40, 50)];
        assert_eq!(
            subtract_interval(&base, (5, 22)),
            vec![(0, 4), (23, 30), (40, 50)]
        );
        assert_eq!(
            subtract_intervals(&base, &[(25, 45), (48, 60)]),
            vec![(0, 10), (20, 24), (46, 47)]
        );
    }

    /// The stroke model: axis-aligned segments carve their exact
    /// rectangle face; the diagonal band stays within its own rows.
    #[test]
    fn trace_stroke_intervals_match_the_contract() {
        let ivs = trace_row_intervals(&[0, 50], &[100, 100], 3, 100);
        assert_eq!(ivs, vec![(-3, 53)]);
        assert!(trace_row_intervals(&[0, 50], &[100, 100], 3, 96).is_empty());
        assert!(trace_row_intervals(&[0, 50], &[100, 100], 3, 104).is_empty());
        for y in [-2, 0, 20, 40, 42] {
            let ivs = trace_row_intervals(&[10, 10], &[0, 40], 2, y);
            assert_eq!(ivs, vec![(8, 12)], "vertical row {y}");
        }
        assert!(trace_row_intervals(&[10, 10], &[0, 40], 2, -3).is_empty());
        let ivs = trace_row_intervals(&[0, 10], &[0, 10], 1, 5);
        assert_eq!(ivs, vec![(4, 6)]);
        assert!(trace_row_intervals(&[0, 10], &[0, 10], 1, -1).is_empty());
    }
}

// ---------------------------------------------------------------------------
// the crafted-world pins (island-spike DSN worlds; the T3-M5 discipline)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod world_tests {
    use super::*;
    use crate::board::Board;

    fn fixture(name: &str) -> (Board, Vec<PourIslands>) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../harness/fixtures/island-spike/{name}"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("crafted world {name} present: {error}"));
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        let result = epic_dsn::reader::read_board(text.as_bytes(), &mut ses);
        assert!(
            matches!(result, epic_dsn::reader::DsnReadResult::Success { .. }),
            "crafted world {name} must parse cleanly, got {result:?}"
        );
        let board = Board::from_ses_board(&ses);
        let face = detect_pour_islands(&board);
        (board, face)
    }

    /// The EXACT disconnect boundary (metal channel width exactly 1
    /// unit): the 1-unit channel bridges — ONE region, ZERO islands.
    /// Kills the MIN_BRIDGE_WIDTH = 2 mutant.
    #[test]
    fn island_gap1_boundary_world_is_connected() {
        let (_, face) = fixture("t11_island_gap1.dsn");
        assert_eq!(face.len(), 1, "exactly the PLANE pour");
        assert_eq!(face[0].region_count, 1, "the 1-unit channel bridges");
        assert_eq!(face[0].island_count, 0);
    }

    /// One unit BELOW the boundary (channel width 0): the corridors
    /// abut, the slab has no metal — TWO regions, the seedless half
    /// floating.
    #[test]
    fn island_gap0_world_disconnects() {
        let (_, face) = fixture("t11_island_gap0.dsn");
        assert_eq!(face.len(), 1);
        assert_eq!(face[0].region_count, 2);
        assert_eq!(face[0].island_count, 1, "the seedless half floats");
        assert_eq!(face[0].islands.len(), 1);
    }

    /// One unit ABOVE the boundary (channel width 2): connected.
    #[test]
    fn island_gap2_world_is_connected() {
        let (_, face) = fixture("t11_island_gap2.dsn");
        assert_eq!(face.len(), 1);
        assert_eq!(face[0].region_count, 1);
        assert_eq!(face[0].island_count, 0);
    }

    /// The diagonal staircase slab (a 1-wide diagonal metal staircase):
    /// under 4-connectivity the staircase does NOT bridge — 11 regions
    /// (top, bottom, 9 isolated single-cell steps) with 10 floating.
    /// Kills the MIN_BRIDGE_WIDTH = 0 mutant (touching intervals must
    /// not bridge).
    #[test]
    fn island_diagonal_staircase_does_not_connect() {
        let (_, face) = fixture("t11_island_diagonal.dsn");
        assert_eq!(face.len(), 1);
        assert_eq!(face[0].region_count, 11);
        assert_eq!(face[0].island_count, 10);
    }

    /// A clean connected pour: one region, zero islands.
    #[test]
    fn island_clean_pour_has_zero_islands() {
        let (_, face) = fixture("t11_island_clean.dsn");
        assert_eq!(face.len(), 1);
        assert_eq!(face[0].region_count, 1);
        assert_eq!(face[0].island_count, 0);
        assert_eq!(face[0].net, "PLANE");
    }

    /// The fully-covered pour face: zero metal regions, zero islands
    /// (nothing to connect), and the clamp predicate is false (there
    /// is no copper to exclude).
    #[test]
    fn island_covered_pour_has_zero_regions() {
        let (board, face) = fixture("t11_island_covered.dsn");
        assert_eq!(face.len(), 1);
        assert_eq!(face[0].region_count, 0);
        assert_eq!(face[0].island_count, 0);
        assert!(
            !pour_fully_floating(&board, ItemId::new(face[0].pour_item_id)),
            "a covered pour has no regions, so the clamp predicate is false"
        );
    }

    /// The clamp predicate on a PARTLY floating pour: the gap0 world's
    /// pour has one seeded region, so it is NOT fully floating (the
    /// clamp only excludes pours whose entire copper is unreachable).
    #[test]
    fn clamp_predicate_is_false_for_a_partly_floating_pour() {
        let (board, face) = fixture("t11_island_gap0.dsn");
        assert_eq!(face[0].island_count, 1);
        assert_eq!(face[0].region_count, 2);
        assert!(!pour_fully_floating(
            &board,
            ItemId::new(face[0].pour_item_id)
        ));
    }

    /// Determinism of the digest: two independent parses of the same
    /// world produce identical digest bytes (64 hex chars).
    #[test]
    fn island_digest_is_parse_deterministic() {
        let (_, a) = fixture("t11_island_gap0.dsn");
        let (_, b) = fixture("t11_island_gap0.dsn");
        assert_eq!(a[0].digest, b[0].digest);
        assert_eq!(a[0].digest.len(), 64, "sha256 hex");
    }

    /// E-11 (M6-T8): the gap0 digest LITERAL pin, with the `v1` schema
    /// prefix in the digest input (the version token — verified
    /// rotation-free for the gate set: bm08/bm06 manifests carry an
    /// EMPTY pour_islands face, and no committed artifact carries any
    /// islands digest). A mutant dropping the token rotates the literal
    /// and dies here.
    #[test]
    fn island_gap0_digest_v1_literal_pin() {
        let (_, face) = fixture("t11_island_gap0.dsn");
        assert_eq!(
            face[0].digest, "640decf3eccedf8d9a5165eaa667fb654a760864952fb92e128dc3b9578d8c5d",
            "the v1-prefixed gap0 digest literal (probe-captured)"
        );
    }

    /// The M6-T8 REGION-LEVEL clamp helper (`pour_region_seeded_by`):
    /// seeds are the GIVEN set's copper, not the board's same-net
    /// population. Faces: the floating world's pin (outside the pour
    /// metal) seeds nothing; the gap0 pin seeds the slab region; the
    /// EMPTY set seeds nothing even though the board-wide population
    /// does (the difference face vs the old pour-level verdict — the
    /// mutant reverting to `pour_fully_floating` answers true here and
    /// dies); and the covered pour answers false.
    #[test]
    fn region_seeded_helper_faces_and_difference() {
        let (board, _face) = fixture("t11_island_floating.dsn");
        let (pour, pin) = pour_pin_ids(&board);
        assert!(
            !pour_region_seeded_by(&board, pour, &[pin]),
            "the floating world's pin sits outside the pour metal"
        );

        let (board, _face) = fixture("t11_island_gap0.dsn");
        let (pour, pin) = pour_pin_ids(&board);
        assert!(
            pour_region_seeded_by(&board, pour, &[pin]),
            "the gap0 pin overlaps the seeded slab region"
        );
        assert!(
            !pour_region_seeded_by(&board, pour, &[]),
            "the EMPTY set seeds nothing — region-level, not pour-level"
        );
        assert!(
            !pour_fully_floating(&board, pour),
            "difference face: the pour-level verdict says the pour is seeded"
        );

        // M6-T9 bank (T8 quality MINOR-Q2): the COVERED world — the
        // one face where the pour-level and region-level verdicts
        // genuinely diverge. The covering pin is OBSTACLE copper: it
        // carves its region out of the pour metal, so a fully covered
        // pour carries NO regions at all and the region-level
        // predicate answers FALSE (the fn doc above, `:823-824`)
        // even though `pour_fully_floating` calls the pour seeded.
        // The T8 form of this arm re-selected the gap0 board (a stale
        // `fixture` binding) and asserted TRUE with an inverted
        // message — the erratum of record is the T8 report's quality
        // block.
        let (board, _face) = fixture("t11_island_covered.dsn");
        let covered_pour = board
            .iter_ascending()
            .find(|entry| matches!(entry.data, ItemData::ConductionArea { .. }))
            .map(|entry| entry.id)
            .expect("covered world has a pour");
        assert!(
            !pour_region_seeded_by(&board, covered_pour, &[pin_covered(&board)]),
            "the covering pin CARVES its region out of the pour — a covered pour has no seeded region (false)"
        );
        assert!(
            !pour_fully_floating(&board, covered_pour),
            "divergence face: the pour-level verdict still calls the covered pour seeded"
        );
    }

    /// The floating/gap0 worlds' (pour, pin) id pair.
    fn pour_pin_ids(board: &Board) -> (ItemId, ItemId) {
        let mut pour = None;
        let mut pin = None;
        for entry in board.iter_ascending() {
            match &entry.data {
                ItemData::ConductionArea { .. } if pour.is_none() => pour = Some(entry.id),
                ItemData::Pin { .. } if pin.is_none() => pin = Some(entry.id),
                _ => {}
            }
        }
        (pour.expect("pour"), pin.expect("pin"))
    }

    /// The covered world's COVERING pin — the pin whose metal overlays
    /// (and carves) the pour. It is OBSTACLE copper for the region
    /// predicate, not seed copper: `pour_region_seeded_by` answers
    /// false through it (the covered arm above).
    fn pin_covered(board: &Board) -> ItemId {
        board
            .iter_ascending()
            .find(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .expect("covered world has a pin")
    }

    /// The severed world's TWO net-PLANE pins by center y (lower
    /// first; the HOT pin sits on another net and is excluded).
    /// Relative-id discipline: found by net membership + geometry,
    /// never by raw id.
    fn plane_pins_by_y(board: &Board, net_number: i32) -> (ItemId, ItemId) {
        let mut pins: Vec<(i64, ItemId)> = Vec::new();
        for entry in board.iter_ascending() {
            if matches!(entry.data, ItemData::Pin { .. }) && entry.nets.contains(&net_number) {
                let shapes = drill_shapes_on_layer(board, entry.id, 0);
                let (_, y0, _, y1) =
                    shape_bbox(shapes.first().expect("F.Cu pad has shapes on layer 0"));
                pins.push(((y0 + y1) / 2, entry.id));
            }
        }
        assert_eq!(
            pins.len(),
            2,
            "the severed world carries exactly two PLANE pins"
        );
        pins.sort_by_key(|&(y, _)| y);
        (pins[0].1, pins[1].1)
    }

    /// 152-H per-region seed attribution on the severed world: the
    /// two metal regions each carry EXACTLY their own pin (the sweep
    /// runs y-ascending, so scan-order region 0 is the LOWER slab),
    /// `net_number` resolves to the net row the face's name carries,
    /// and the face invariant holds — `region_seeds` enumerates every
    /// metal region (`region_count`) with the empty-item count equal
    /// to the floating-island count (items-nonempty ⟺ seeded).
    #[test]
    fn severed_world_attributes_each_region_its_own_pin() {
        let (board, face) = fixture("t11_island_severed.dsn");
        let pour = &face[0];
        assert_eq!(pour.region_count, 2, "the carve splits the pour in two");
        assert_eq!(pour.region_seeds.len(), pour.region_count);
        assert_eq!(pour.net, "PLANE");
        let net = board
            .rules()
            .nets
            .get(pour.net_number)
            .expect("net_number resolves to a net row");
        assert_eq!(net.name, "PLANE");
        let (lower_pin, upper_pin) = plane_pins_by_y(&board, pour.net_number);
        assert_eq!(pour.region_seeds[0].items, vec![lower_pin.get()]);
        assert_eq!(pour.region_seeds[1].items, vec![upper_pin.get()]);
        // Scan order: region 0 is the lower slab, the carve sits
        // strictly between the two regions' bboxes.
        assert!(pour.region_seeds[0].y1 < pour.region_seeds[1].y0);
        // Invariant: floating metal regions == empty-item regions.
        let empty = pour
            .region_seeds
            .iter()
            .filter(|region| region.items.is_empty())
            .count();
        assert_eq!(empty, pour.island_count);
        assert_eq!(pour.island_count, 0, "both halves are seeded");
    }

    /// 152-H control: the bridged world's F.Cu partition is IDENTICAL
    /// to the severed one (the B.Cu PLANE wire is not a source for the
    /// F.Cu scan — wrong layer), so the attribution is byte-identical
    /// too; only the connectivity walk downstream differs.
    #[test]
    fn bridged_world_keeps_the_severed_partition() {
        let (board, severed) = fixture("t11_island_severed.dsn");
        let (_bridged_board, bridged) = fixture("t11_island_bridged.dsn");
        assert_eq!(bridged[0].region_seeds, severed[0].region_seeds);
        assert_eq!(bridged[0].digest, severed[0].digest);
        let (lower_pin, upper_pin) = plane_pins_by_y(&board, severed[0].net_number);
        assert_eq!(bridged[0].region_seeds[0].items, vec![lower_pin.get()]);
        assert_eq!(bridged[0].region_seeds[1].items, vec![upper_pin.get()]);
    }
}
