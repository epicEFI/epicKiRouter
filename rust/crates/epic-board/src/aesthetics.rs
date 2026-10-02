//! The M8 aesthetics measurer (design :86 — the four metrics): ONE
//! pure, java-free function over a live [`Board`] plus its rules, so
//! the identical code measures professional PCBench routes and our own
//! routed output (the M8-T1 charter).
//!
//! ## The four metrics (the plan's Key-facts definitions — the contract)
//!
//! * `mean_length_excess` — the mean over qualifying nets of
//!   `(routed_length − mst_lb) / mst_lb`, where a net QUALIFIES when it
//!   carries ≥ 2 connectable endpoints AND a routed length > 0;
//!   `mst_lb` is the per-net Euclidean MST over the endpoint centers,
//!   in board DBU. The endpoint (terminal) set mirrors
//!   `epic-router/src/pipeline/board_statistics_bounds.rs`
//!   (`getTerminals` + `toTerminal`): pins (shape-corrected center,
//!   non-empty signal-layer set) and conduction areas (border center
//!   of gravity). The Java parity source for that terminal face is
//!   `core/scoring/BoardStatisticsBoundsCalculator.getTerminalItems`
//!   (a terminal = `containsNet && !isRoutable()` item — pins +
//!   conduction areas); the METRIC here differs deliberately: the
//!   bounds calculator's MST is Manhattan in mm (the V2 score face),
//!   this one is Euclidean in DBU (the design :86 face). Nets with an
//!   all-zero-MST (coincident terminals) are skipped — a zero lower
//!   bound divides by zero; the skip is part of the definition.
//! * `via_density` = `via_count / (total_length_mm / 100)` — vias per
//!   100 mm. The `via_count` / `total_length_mm` faces come from a
//!   [`BoardTally`] walk that REPLICATES the `BoardStatistics`
//!   constructor's counting semantics
//!   (`epic-router/src/pipeline/board_statistics.rs` — the ctor lives
//!   behind `&mut` + the tree manager there and is NOT moved to
//!   epic-board; the replication is pinned equal on a crafted world
//!   and on the bm08 tier fixture in that crate's tests). Zero-length
//!   boards answer 0.0 for both densities (an empty denominator is
//!   the no-routed-copper face, not an error).
//! * `bend_to_length_ratio` = `bend_count / (total_length_mm / 100)`
//!   — bends per 100 mm; bends per the `BendsCounts` semantics
//!   (`cornerCount − 2` per trace, |angle − 90| < 1 / |angle − 45| < 1
//!   or |angle − 135| < 1).
//! * `parallelism_ratio` — the fraction of total trace length lying
//!   within [`AESTHETICS_PARALLELISM_WINDOW_DBU`] of a DIFFERENT net's
//!   trace on the same layer. Per SEGMENT: a segment's length counts
//!   ONCE when ANY different-net same-layer trace comes within the
//!   window of it (min point-to-segment distance over the other
//!   trace's segments, `dist <= window`, equality included — the
//!   pairs.rs `delta <=` convention). Crossing segments count (a
//!   crossing IS within the window at distance 0) — the plan's
//!   definition is the contract; the parallel-vs-crossing distinction
//!   is T3's group detector's, not the measurer's. Deterministic
//!   reduction: segments iterate in (layer, trace-id, segment-index)
//!   order, f64 accumulates in that order. The window queries run
//!   through a purpose-built uniform-grid [`SegmentIndex`] (cell size
//!   = the window): the Java-parity searchtree is a mutation-support
//!   structure (`&mut Board` + `SearchTreeManager` reinsert), which a
//!   pure `&Board` measurer cannot touch — the grid gives the same
//!   window-query semantics read-only.
//!
//! ## Serialization edge
//!
//! [`AestheticsMetrics::render_json`] is the ONLY rendering face: the
//! four metrics, f64s rounded to 3 decimals at this edge only (never
//! in the stored fields), serde_json pretty print with the BTreeMap
//! (alphabetical) key order — byte-stable for a fixed value set. The
//! reconciliation/aggregate faces ([`AestheticsMetrics::routed_length_dbu`]
//! and friends) never serialize.

use std::collections::BTreeMap;

use epic_dsn::state::Unit;
use epic_geometry::float_point::FloatPoint;

use crate::board::Board;
use crate::id::ItemId;
use crate::items::{BoardShape, ItemData};
use crate::rules_surf::BoardRules;

/// The parallelism window, in board DBU: a trace segment counts as
/// coupled when a DIFFERENT net's same-layer trace lies within this
/// distance of it. THE SINGLE DEFINITION of the number (the M8-T3
/// window-constant TIE): this is the definition site — epic-router's
/// `COUPLING_WINDOW_DBU` (pipeline/pairs.rs) and the gloss group
/// detector's window are DERIVED from it (epic-router may import from
/// epic-board; the reverse may not, so the definition lives in the
/// lower crate). Value 50_000 board DBU.
pub const AESTHETICS_PARALLELISM_WINDOW_DBU: i64 = 50_000;

/// The four M8 aesthetics metrics plus the reconciliation faces. The
/// four metric fields are the serialized face
/// ([`Self::render_json`]); everything else exists for the DNR-18
/// reconciliation pins (Σ per-net vs the board-level sums) and the
/// `BoardStatistics`-equality pins — never serialized.
#[derive(Clone, Debug, PartialEq)]
pub struct AestheticsMetrics {
    /// The mean routed-length excess over qualifying nets.
    pub mean_length_excess: f64,
    /// Vias per 100 mm of routed trace.
    pub via_density: f64,
    /// Bends per 100 mm of routed trace.
    pub bend_to_length_ratio: f64,
    /// The fraction of total trace length within the parallelism
    /// window of a different net's same-layer trace.
    pub parallelism_ratio: f64,
    /// Σ per-net routed length, DBU (the DNR-18 face — reconciles
    /// against [`BoardTally::total_length`] pre-f32-rounding).
    pub routed_length_dbu: f64,
    /// Σ per-net MST lower bounds, DBU (the DNR-18 face — bounded by
    /// the Manhattan bounds total: Euclidean ≤ Manhattan per net).
    pub mst_lb_dbu: f64,
    /// The number of nets that qualified (≥2 endpoints, routed
    /// length > 0, mst_lb > 0) — the mean's denominator.
    pub nets_measured: u32,
    /// The [`BoardTally`] this run's densities came from (the
    /// `BoardStatistics`-equality pin surface).
    pub tally: BoardTally,
}

/// The minimal per-board tally, REPLICATING the `BoardStatistics`
/// constructor's counting semantics over the same ascending-id live
/// walk (`epic-router/src/pipeline/board_statistics.rs` — the
/// traces/bends/vias faces; pinned field-equal against the real walk
/// in that crate's tests). The measurer lives in epic-board and the
/// `BoardStatistics` ctor needs `&mut Board` + `SearchTreeManager`,
/// so the walk is reimplemented here against the same semantics —
/// the parity pin is the honest glue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardTally {
    /// `TracesCounts.total_count`.
    pub trace_count: i32,
    /// `TracesCounts.total_length` (the f64 sum rounded to f32).
    pub total_length: f32,
    /// `TracesCounts.total_length_mm` (`(float)(f32 total_length *
    /// unit_to_mm_factor)`).
    pub total_length_mm: Option<f32>,
    /// `BendsCounts.total_count`.
    pub bend_total: i32,
    /// `BendsCounts.ninety_degree_count`.
    pub bend_ninety: i32,
    /// `BendsCounts.forty_five_degree_count`.
    pub bend_forty_five: i32,
    /// `BendsCounts.other_angle_count`.
    pub bend_other: i32,
    /// `ViasCounts.total_count`.
    pub via_total: i32,
    /// `ViasCounts.through_hole_count`.
    pub via_through: i32,
    /// `ViasCounts.blind_count`.
    pub via_blind: i32,
    /// `ViasCounts.buried_count`.
    pub via_buried: i32,
}

/// One trace SEGMENT (two consecutive polyline corners) in the
/// parallelism walk. f64 coordinates (the `corner_approx` face —
/// reference DSNs carry rational corners).
#[derive(Clone, Debug)]
struct Segment {
    layer: i32,
    trace: ItemId,
    /// The net set of the owning trace (almost always exactly one —
    /// the disjointness check is against ALL of them).
    nets: Vec<i32>,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    /// This segment's length (the `length_approx` term — the same
    /// summand the trace's `length_approx_total` uses, so the coupled
    /// fraction is a true fraction of the total).
    len: f64,
}

/// The uniform-grid spatial index over the trace segments (module
/// docs): cell side = the window; a segment is inserted into every
/// cell its bounding box covers, and a query gathers the cells its
/// window-expanded box covers. Deterministic by construction (the
/// predicate is order-independent; the accumulation order comes from
/// the sorted segment list, not the index).
struct SegmentIndex {
    cell: i64,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
}

impl SegmentIndex {
    fn build(segments: &[Segment], window: f64) -> Self {
        let cell = if window >= 1.0 { window as i64 } else { 1 };
        let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
        for (index, segment) in segments.iter().enumerate() {
            let min_x = segment.ax.min(segment.bx).floor() as i64;
            let max_x = segment.ax.max(segment.bx).floor() as i64;
            let min_y = segment.ay.min(segment.by).floor() as i64;
            let max_y = segment.ay.max(segment.by).floor() as i64;
            for cx in min_x / cell..=max_x / cell {
                for cy in min_y / cell..=max_y / cell {
                    cells.entry((cx, cy)).or_default().push(index);
                }
            }
        }
        Self { cell, cells }
    }

    /// The candidate segment indices within the window of the query
    /// segment's bounding box (dedup via the visited stamp — the
    /// candidate set is a SET, the caller's predicate is
    /// order-independent).
    fn window_candidates(
        &self,
        segment: &Segment,
        window: f64,
    ) -> impl Iterator<Item = usize> + '_ {
        let min_x = (segment.ax.min(segment.bx) - window).floor() as i64;
        let max_x = (segment.ax.max(segment.bx) + window).floor() as i64;
        let min_y = (segment.ay.min(segment.by) - window).floor() as i64;
        let max_y = (segment.ay.max(segment.by) + window).floor() as i64;
        let mut seen = std::collections::BTreeSet::new();
        for cx in min_x / self.cell..=max_x / self.cell {
            for cy in min_y / self.cell..=max_y / self.cell {
                if let Some(bucket) = self.cells.get(&(cx, cy)) {
                    seen.extend(bucket.iter().copied());
                }
            }
        }
        seen.into_iter()
    }
}

/// Java `Unit.scale(1.0, unit, MM) / (resolution > 0 ? resolution :
/// 1)` — the same f64 factor the `BoardStatistics` ctor and the
/// bounds calculator both derive.
fn board_unit_to_mm_factor(board: &Board) -> f64 {
    let communication = board.communication();
    let resolution = i64::from(communication.resolution);
    let resolution = if resolution > 0 { resolution } else { 1 };
    Unit::scale(1.0, communication.unit, Unit::Mm) / resolution as f64
}

/// The MICROMETRE twin of [`board_unit_to_mm_factor`] — upstream
/// `Unit.scale(1.0, unit, UM) / max(1, resolution)` — pub because the
/// epic-drc #925a shortfall-tolerance gate (upstream `14b28b6ff`)
/// compares clearance shortfalls against a µm tolerance and needs the
/// board-unit→µm conversion without a direct epic-dsn dependency edge
/// (epic-dsn is dev-only there).
#[must_use]
pub fn board_unit_to_um_factor(board: &Board) -> f64 {
    let communication = board.communication();
    let resolution = i64::from(communication.resolution);
    let resolution = if resolution > 0 { resolution } else { 1 };
    Unit::scale(1.0, communication.unit, Unit::Um) / resolution as f64
}

/// The M8 aesthetics measurer — the four metrics over one board (the
/// module docs carry the metric definitions; the plan's Key facts is
/// the contract). Pure: reads the board, mutates nothing.
#[must_use]
pub fn aesthetics_metrics(board: &Board, rules: &BoardRules) -> AestheticsMetrics {
    // `rules` is the plan-named parameter; the walk reads everything
    // through the board (which owns the same rules). Kept in the
    // signature so the call sites state their measurement surface
    // explicitly (the plan's `aesthetics_metrics(board, rules)`).
    let _ = rules;
    let window = AESTHETICS_PARALLELISM_WINDOW_DBU as f64;
    let mm_factor = board_unit_to_mm_factor(board);

    // ---- the counting walk (ascending id, the BoardStatistics face)
    let mut tally = BoardTally::default();
    let mut total_length_f64 = 0.0f64;
    // Per-net routed lengths (ascending trace id within the net — the
    // traces arrive in ascending-id order from the walk).
    let mut length_per_net: BTreeMap<i32, f64> = BTreeMap::new();
    // Per-net terminal positions (pins + conduction areas, the
    // board_statistics_bounds terminal face).
    let mut terminals_per_net: BTreeMap<i32, Vec<(f64, f64)>> = BTreeMap::new();
    let mut segments: Vec<Segment> = Vec::new();

    let layer_count = i32::try_from(board.layers().layers.len()).unwrap_or(i32::MAX);
    for entry in board.iter_ascending() {
        if !entry.on_the_board {
            continue;
        }
        match &entry.data {
            ItemData::Trace { .. } => {
                let Some(polyline) = board.trace_polyline(entry.id) else {
                    continue;
                };
                let Some(layer) = board.trace_layer(entry.id) else {
                    continue;
                };
                let trace_length = polyline.length_approx_total();
                tally.trace_count += 1;
                total_length_f64 += trace_length;
                for &net in &entry.nets {
                    *length_per_net.entry(net).or_insert(0.0) += trace_length;
                }
                let corner_count = polyline.corner_count();
                if corner_count >= 3 {
                    tally.bend_total = tally
                        .bend_total
                        .saturating_add(i32::try_from(corner_count - 2).unwrap_or(i32::MAX));
                    let corners = polyline.corners();
                    for i in 1..(corner_count - 1) {
                        let prev = corners[i - 1].to_float();
                        let current = corners[i].to_float();
                        let next = corners[i + 1].to_float();
                        let dx1 = current.x - prev.x;
                        let dy1 = current.y - prev.y;
                        let dx2 = next.x - current.x;
                        let dy2 = next.y - current.y;
                        let mut angle = (dy2.atan2(dx2) - dy1.atan2(dx1)).to_degrees().abs();
                        angle = angle.min(360.0 - angle);
                        if angle > 180.0 {
                            angle = 360.0 - angle;
                        }
                        if (angle - 90.0).abs() < 1.0 {
                            tally.bend_ninety += 1;
                        } else if (angle - 45.0).abs() < 1.0 || (angle - 135.0).abs() < 1.0 {
                            tally.bend_forty_five += 1;
                        } else {
                            tally.bend_other += 1;
                        }
                    }
                }
                // The segment extraction (the pairs.rs
                // collect_corridor_segments shape, without the
                // axis-aligned filter — the metric is over ALL
                // segments).
                for index in 0..corner_count.saturating_sub(1) {
                    let Some(a) = polyline.corner(index as i32) else {
                        continue;
                    };
                    let Some(b) = polyline.corner((index + 1) as i32) else {
                        continue;
                    };
                    let af = a.to_float();
                    let bf = b.to_float();
                    let len = polyline.length_approx(index as i32, (index + 1) as i32);
                    segments.push(Segment {
                        layer,
                        trace: entry.id,
                        nets: entry.nets.clone(),
                        ax: af.x,
                        ay: af.y,
                        bx: bf.x,
                        by: bf.y,
                        len,
                    });
                }
            }
            ItemData::Via { padstack_no, .. } => {
                tally.via_total += 1;
                let Some(padstack) = board.library().padstack(*padstack_no) else {
                    continue;
                };
                let first = padstack.from_layer() as i32;
                let last = padstack.to_layer();
                let last_layer_index = layer_count - 1;
                if first == 0 && last == last_layer_index {
                    tally.via_through += 1;
                } else if first == 0 || last == last_layer_index {
                    tally.via_blind = tally.via_blind.saturating_add(1);
                } else {
                    tally.via_buried = tally.via_buried.saturating_add(1);
                }
            }
            ItemData::Pin { .. } => {
                // The terminal face (pins with a non-empty signal
                // layer set — the bounds calculator's skip).
                let Some(center) = board.pin_center(entry.id) else {
                    continue;
                };
                if pin_signal_layers(board, entry.id).is_empty() {
                    continue;
                }
                let float = center.to_float();
                for &net in &entry.nets {
                    terminals_per_net
                        .entry(net)
                        .or_default()
                        .push((float.x, float.y));
                }
            }
            ItemData::ConductionArea { layer, .. } => {
                if !layer_is_signal(board, *layer) {
                    continue;
                }
                let Some(area) = board.conduction_area(entry.id) else {
                    continue;
                };
                let gravity = conduction_gravity(&area.border);
                for &net in &entry.nets {
                    terminals_per_net
                        .entry(net)
                        .or_default()
                        .push((gravity.x, gravity.y));
                }
            }
            ItemData::ObstacleArea { .. }
            | ItemData::BoardOutline { .. }
            | ItemData::ComponentOutline { .. }
            | ItemData::Other => {}
        }
    }

    // The mm normalization — the BoardStatistics face (f64 sum → f32
    // → × factor → f32).
    tally.total_length = total_length_f64 as f32;
    tally.total_length_mm = Some((f64::from(tally.total_length) * mm_factor) as f32);

    // ---- the per-net MST lower bounds
    let mut excess_sum = 0.0f64;
    let mut mst_lb_total = 0.0f64;
    let mut routed_total = 0.0f64;
    let mut nets_measured = 0u32;
    for (&net, &routed) in &length_per_net {
        routed_total += routed;
        let Some(terminals) = terminals_per_net.get(&net) else {
            continue;
        };
        if terminals.len() < 2 || routed <= 0.0 {
            continue;
        }
        let mst_lb = prim_mst_length(terminals);
        if mst_lb <= 0.0 {
            continue;
        }
        excess_sum += (routed - mst_lb) / mst_lb;
        mst_lb_total += mst_lb;
        nets_measured += 1;
    }
    // Nets with terminals but NO traces never entered
    // `length_per_net` — their (unqualified) absence is the skip the
    // definition asks for; nets with traces but no terminals
    // (route-through nets) are skipped by the `terminals.len() < 2`
    // guard above.
    let mean_length_excess = if nets_measured > 0 {
        excess_sum / f64::from(nets_measured)
    } else {
        0.0
    };

    // ---- the densities
    let total_length_mm = f64::from(tally.total_length_mm.unwrap_or(0.0));
    let length_per_100mm = total_length_mm / 100.0;
    let via_density = if length_per_100mm > 0.0 {
        f64::from(tally.via_total) / length_per_100mm
    } else {
        0.0
    };
    let bend_to_length_ratio = if length_per_100mm > 0.0 {
        f64::from(tally.bend_total) / length_per_100mm
    } else {
        0.0
    };

    // ---- the parallelism ratio (module docs: the segment walk, the
    // grid index, the deterministic reduction)
    let mut sorted = segments;
    // A STABLE sort by (layer, trace id): segments were pushed in
    // polyline order per trace, and the stable sort preserves that
    // order within a trace — the (layer, trace, polyline-index) walk
    // the module docs name.
    sorted.sort_by(|left, right| {
        left.layer
            .cmp(&right.layer)
            .then(left.trace.cmp(&right.trace))
    });
    let index = SegmentIndex::build(&sorted, window);
    let mut coupled_length = 0.0f64;
    for segment in &sorted {
        let coupled = index
            .window_candidates(segment, window)
            .any(|candidate_index| {
                let candidate = &sorted[candidate_index];
                candidate.trace != segment.trace
                    && candidate.layer == segment.layer
                    && nets_disjoint(&segment.nets, &candidate.nets)
                    && segment_distance(segment, candidate) <= window
            });
        if coupled {
            coupled_length += segment.len;
        }
    }
    let parallelism_ratio = if total_length_f64 > 0.0 {
        coupled_length / total_length_f64
    } else {
        0.0
    };

    AestheticsMetrics {
        mean_length_excess,
        via_density,
        bend_to_length_ratio,
        parallelism_ratio,
        routed_length_dbu: routed_total,
        mst_lb_dbu: mst_lb_total,
        nets_measured,
        tally,
    }
}

/// The serialization VALUE face (the gloss sidecar's insertion face):
/// the four metrics as a `serde_json::Value` object, 3-decimal
/// rounding applied HERE only. The M8-T3 `bus_groups` block is
/// inserted into this object by epic-cli when the gloss report is
/// non-empty (serde-skip-when-empty — the default face carries the
/// four keys alone, byte-identical to the T1 goldens).
#[must_use]
pub fn render_json_value(metrics: &AestheticsMetrics) -> serde_json::Value {
    fn round3(value: f64) -> f64 {
        (value * 1000.0).round() / 1000.0
    }
    serde_json::json!({
        "bend_to_length_ratio": round3(metrics.bend_to_length_ratio),
        "mean_length_excess": round3(metrics.mean_length_excess),
        "parallelism_ratio": round3(metrics.parallelism_ratio),
        "via_density": round3(metrics.via_density),
    })
}

/// The serialization edge (the ONLY one): the four metrics, 3-decimal
/// rounding applied HERE only, serde_json pretty with the BTreeMap
/// (alphabetical) key order — byte-stable for a fixed value set.
#[must_use]
pub fn render_json(metrics: &AestheticsMetrics) -> String {
    let object = render_json_value(metrics);
    // Infallible in practice (serializing a `serde_json::Value` of
    // finite f64s and string keys cannot fail — no IO, no non-string
    // map keys, no NaN guards needed at these values); loud if the
    // impossible ever happens, never a silent `"{}"`.
    serde_json::to_string_pretty(&object).expect("serde_json Value serialization cannot fail")
}

/// The per-net Euclidean MST length — Prim seeded at terminal 0 with
/// the bounds calculator's comparison shape (first-minimum-wins scan,
/// strict-`<` relax); the MST WEIGHT is order-invariant (the multiset
/// of edge weights is unique), so the seeding order only fixes the f64
/// accumulation order — deterministic either way.
fn prim_mst_length(terminals: &[(f64, f64)]) -> f64 {
    let count = terminals.len();
    let mut used = vec![false; count];
    let mut distances = vec![f64::INFINITY; count];
    distances[0] = 0.0;
    let mut total = 0.0f64;
    for _ in 0..count {
        let mut current: Option<usize> = None;
        for (index, &is_used) in used.iter().enumerate() {
            if is_used {
                continue;
            }
            let better = match current {
                None => true,
                Some(current) => distances[index] < distances[current],
            };
            if better {
                current = Some(index);
            }
        }
        let Some(current) = current else { break };
        used[current] = true;
        if distances[current].is_finite() {
            total += distances[current];
        }
        let (cx, cy) = terminals[current];
        for index in 0..count {
            if used[index] {
                continue;
            }
            let (tx, ty) = terminals[index];
            let dx = cx - tx;
            let dy = cy - ty;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < distances[index] {
                distances[index] = distance;
            }
        }
    }
    total
}

/// `nets_disjoint` — no shared net number between the two traces.
fn nets_disjoint(left: &[i32], right: &[i32]) -> bool {
    left.iter().all(|l| !right.contains(l))
}

/// The min distance between the two segments: an EXACT proper-crossing
/// predicate first (interior-interior X crossings are distance 0 — the
/// module docs' crossing contract, which the endpoint-distance arms
/// alone do NOT deliver: all four distances are positive for a proper
/// X), then the four endpoint-to-other-segment distances as the
/// fallback (which cover endpoint touches and collinear overlaps —
/// those land at ~0 through the projection arms).
fn segment_distance(a: &Segment, b: &Segment) -> f64 {
    if segments_properly_cross(a, b) {
        return 0.0;
    }
    let d1 = point_segment_distance(a.ax, a.ay, b.ax, b.ay, b.bx, b.by);
    let d2 = point_segment_distance(a.bx, a.by, b.ax, b.ay, b.bx, b.by);
    let d3 = point_segment_distance(b.ax, b.ay, a.ax, a.ay, a.bx, a.by);
    let d4 = point_segment_distance(b.bx, b.by, a.ax, a.ay, a.bx, a.by);
    d1.min(d2).min(d3).min(d4)
}

/// The EXACT orientation of the triangle (o, a, b): positive =
/// counter-clockwise, negative = clockwise, 0 = collinear. i128
/// products when every coordinate is an integral f64 in i64 range —
/// the common face (IntPoint corners land here exactly; the
/// epic-geometry checked-arithmetic precedent) — the f64 product
/// otherwise (rational corners; the approximation only affects sign
/// calls at |cross| within f64 rounding of zero).
fn orientation(ox: f64, oy: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let to_i128 = |v: f64| -> Option<i128> {
        if v.fract() == 0.0 && v.abs() < 9.0e15 {
            Some(v as i128)
        } else {
            None
        }
    };
    if let (Some(ox), Some(oy), Some(ax), Some(ay), Some(bx), Some(by)) = (
        to_i128(ox),
        to_i128(oy),
        to_i128(ax),
        to_i128(ay),
        to_i128(bx),
        to_i128(by),
    ) {
        let cross = (ax - ox) * (by - oy) - (ay - oy) * (bx - ox);
        if cross > 0 {
            1.0
        } else if cross < 0 {
            -1.0
        } else {
            0.0
        }
    } else {
        let cross = (ax - ox) * (by - oy) - (ay - oy) * (bx - ox);
        if cross > 0.0 {
            1.0
        } else if cross < 0.0 {
            -1.0
        } else {
            0.0
        }
    }
}

/// The CLRS proper-crossing test: the two segments' endpoints are
/// STRICTLY on opposite sides of each other's line (all four
/// orientations nonzero with opposite signs). Endpoint touches and
/// collinear overlaps need no predicate — the distance fallback arms
/// already answer ~0 for them.
fn segments_properly_cross(a: &Segment, b: &Segment) -> bool {
    let d1 = orientation(b.ax, b.ay, b.bx, b.by, a.ax, a.ay);
    let d2 = orientation(b.ax, b.ay, b.bx, b.by, a.bx, a.by);
    let d3 = orientation(a.ax, a.ay, a.bx, a.by, b.ax, b.ay);
    let d4 = orientation(a.ax, a.ay, a.bx, a.by, b.bx, b.by);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// The min distance from the point (px, py) to the segment
/// (ax, ay)-(bx, by).
fn point_segment_distance(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        let ex = px - ax;
        let ey = py - ay;
        return (ex * ex + ey * ey).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    let ex = px - (ax + t * dx);
    let ey = py - (ay + t * dy);
    (ex * ex + ey * ey).sqrt()
}

/// The conduction-area terminal position — the bounds calculator's
/// gravity arms (Tile / PolygonShape / Circle).
fn conduction_gravity(border: &BoardShape) -> FloatPoint {
    match border {
        BoardShape::Tile(tile) => tile.centre_of_gravity(),
        BoardShape::PolygonShape(polygon) => {
            let count = polygon.border_line_count();
            let mut x = 0.0f64;
            let mut y = 0.0f64;
            for i in 0..count {
                let current = polygon.corner(i as i32).to_float();
                x += current.x;
                y += current.y;
            }
            FloatPoint::new(x / count as f64, y / count as f64)
        }
        BoardShape::Circle(circle) => circle.centre_of_gravity(),
    }
}

/// The signal-layer probe for the terminal skip (`add_signal_layer`
/// semantics: in range AND signal).
fn layer_is_signal(board: &Board, layer: i32) -> bool {
    let layers = board.layers();
    layer >= 0
        && layer < i32::try_from(layers.layers.len()).unwrap_or(i32::MAX)
        && layers.layers[layer as usize].is_signal
}

/// The pin's signal-layer set probe (the bounds calculator's
/// `pin_signal_layers`): the padstack span, filtered to signal
/// layers with a non-null shape. Empty set → the pin is NOT a
/// terminal (the getTerminals skip).
fn pin_signal_layers(board: &Board, id: ItemId) -> Vec<i32> {
    let mut result = Vec::new();
    let layer_count = board.layers().layers.len() as i32;
    let Some(first) = (0..layer_count).find(|&layer| board.pin_is_on_layer(id, layer)) else {
        return result;
    };
    let Some(last) = (0..layer_count)
        .rev()
        .find(|&layer| board.pin_is_on_layer(id, layer))
    else {
        return result;
    };
    for layer in first..=last {
        if board.pin_shape(id, layer - first).is_some() && layer_is_signal(board, layer) {
            result.push(layer);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{parse_board_from_path, parse_board_from_text};

    // The crafted-world DSN header shared by the pins below (2
    // signal layers, resolution um 1 — 1 DBU = 0.001 mm).
    const HEADER: &str = r#"(pcb PIN.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 400000 300000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 120000 30000 front 0))
    (component CMP_C (place CMP_C 20000 120000 front 0))
    (component CMP_D (place CMP_D 120000 120000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_TH
      (shape (circle F.Cu 500 0 0))
      (shape (circle B.Cu 500 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 801 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
    (image CMP_C (pin PAD_SMD P1 0 0))
    (image CMP_D (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
"#;

    /// Parses a full DSN (the shared header + the network tail + the
    /// wiring section).
    fn parse_full(network: &str, wiring: &str) -> Board {
        // Scope order: (pcb ... (network <nets>) (wiring <wires>)).
        let text = format!("{HEADER}{network}  )\n{wiring})\n");
        parse_board_from_text(&text)
    }

    /// The known-MST world (pin a): net NET1, two pins 100_000 DBU
    /// apart on one axis (the MST over two terminals IS the center
    /// distance), routed with an L path of exactly 120_000 DBU →
    /// excess exactly 0.2. NET2 carries pins but NO traces → NOT
    /// qualified (routed length 0 — the skip face).
    #[test]
    fn t1_known_mst_world_exact_excess() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n    (net NET2 (pins CMP_C-P1 CMP_D-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 95000 30000 95000 50000 120000 50000)(net NET1)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.nets_measured, 1);
        assert_eq!(metrics.routed_length_dbu, 120_000.0);
        assert_eq!(metrics.mst_lb_dbu, 100_000.0);
        assert!((metrics.mean_length_excess - 0.2).abs() < 1e-12);
    }

    /// The proper-X world (Q1-1): two same-layer different-net
    /// segments crossing at interior points, ALL four
    /// endpoint-to-other-segment distances 65_000-100_000 DBU — OUTSIDE
    /// the 50_000 window — so the crossing arm is the ONLY coupling
    /// path. A crossing IS distance 0 (the module docs / the plan's
    /// contract): both segments couple → the whole routed length
    /// (330_000 DBU) is coupled → ratio exactly 1.0. Mutation-verified:
    /// removing the intersection arm rotates this world to 0.0
    /// (fix2-mutation-crossing.log).
    #[test]
    fn t1_proper_x_world_coupled_at_distance_zero() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n    (net NET2 (pins CMP_C-P1 CMP_D-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 150000 150000 150000)(net NET1)(type route))
    (wire (path F.Cu 200 85000 250000 85000 50000)(net NET2)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.trace_count, 2);
        // All four endpoint distances sit OUTSIDE the window (the
        // crossing arm is load-bearing, not redundant):
        // (85000,50000)→NET1 = 100_000; (85000,250000)→NET1 = 100_000;
        // (20000,150000)→NET2 = 65_000; (150000,150000)→NET2 = 65_000.
        assert!(
            (metrics.parallelism_ratio - 1.0).abs() < 1e-12,
            "a proper X is distance 0 → both segments couple: {:?}",
            metrics.parallelism_ratio
        );
    }

    /// The parallelism boundary (pin b), BOTH directions: two
    /// same-layer traces of different nets at EXACTLY the window
    /// (50_000 DBU apart) are coupled → ratio 1.0; one DBU past the
    /// window NOTHING couples (ratio 0). DNR-16: the exact edge
    /// inclusive, one step out excluded, mutation-verified both
    /// directions (the report's transcript carries the apply → test →
    /// revert rounds).
    #[test]
    fn t1_parallelism_at_exactly_window_coupled() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n    (net NET2 (pins CMP_C-P1 CMP_D-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 120000 30000)(net NET1)(type route))
    (wire (path F.Cu 200 20000 80000 120000 80000)(net NET2)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.trace_count, 2);
        // BOTH segments couple (each lies exactly at the window from
        // the other) → the whole routed length is coupled: ratio 1.0.
        assert!((metrics.parallelism_ratio - 1.0).abs() < 1e-12);
    }

    /// The +1 world: the same geometry one DBU past the window —
    /// nothing couples (ratio 0.0). The mutant `<=` → `<` kills on
    /// the exactly-at world; the mutant `<=` → `>` kills on BOTH.
    #[test]
    fn t1_parallelism_one_past_window_uncoupled() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n    (net NET2 (pins CMP_C-P1 CMP_D-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 120000 30000)(net NET1)(type route))
    (wire (path F.Cu 200 20000 80001 120000 80001)(net NET2)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.parallelism_ratio, 0.0);
    }

    /// The via-density world (pin c): one through via + exactly
    /// 100_000 DBU of routed trace → 100 mm → 1.0 via per 100 mm.
    #[test]
    fn t1_via_density_world() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 70000 30000)(net NET1)(type route))
    (wire (path B.Cu 200 70000 39000 120000 39000)(net NET1)(type route))
    (via PAD_VIA 70000 34500 (net NET1)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.via_total, 1);
        assert_eq!(metrics.tally.via_through, 1);
        let length_mm = metrics
            .tally
            .total_length_mm
            .expect("a routed world has an mm face");
        assert!((length_mm - 100.0).abs() < 1e-3);
        assert!((metrics.via_density - 1.0).abs() < 1e-9);
    }

    /// The bends world (pin d): one 5-corner trace (3 bends) — a 90°
    /// corner at (70000, 30000), then two 45° corners — classified per
    /// the BendsCounts semantics (`cornerCount − 2` per trace,
    /// |angle − 90| < 1 / |angle − 45| < 1 or |angle − 135| < 1).
    #[test]
    fn t1_bends_world_ninety_and_forty_five() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 70000 30000 70000 75000 92500 97500 120000 97500)(net NET1)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.bend_total, 3);
        assert_eq!(metrics.tally.bend_ninety, 1);
        assert_eq!(metrics.tally.bend_forty_five, 2);
        assert_eq!(metrics.tally.bend_other, 0);
    }

    /// The band-edge + 135° world (Q4-1): one trace, five corners — a
    /// 90° (exact), a 91.2° (OUTSIDE the ninety band by 0.2° → other),
    /// a 135° exact (the |angle−135| < 1 arm of the forty-five bucket),
    /// a 44.5° (INSIDE the forty-five band), and a 43.6° (OUTSIDE by
    /// 1.4° → other). The exact ±1.0° edge itself is f64-unstable at
    /// the line-intersection rounding scale (~1e-6°), so the edges are
    /// pinned with a 0.2° margin — a classification mutant that moves
    /// any band boundary by more than that dies here; a boundary shift
    /// of exactly 0 (`<` vs `<=` at the edge) is not f64-discriminable,
    /// and that residual is covered by the 1Bitsy field-equal parity on
    /// real data.
    #[test]
    fn t1_bends_band_edges_and_135() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 70000 30000 70000 80000 20011 78952.9 56099 44345.7 106095 44956.6 141880 79877.3)(net NET1)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.bend_total, 5);
        assert_eq!(metrics.tally.bend_ninety, 1, "90° exact: ninety");
        assert_eq!(
            metrics.tally.bend_forty_five, 2,
            "135° exact (the 135 arm) + 44.5° (inside)"
        );
        assert_eq!(
            metrics.tally.bend_other, 2,
            "91.2° and 43.6°: outside their bands"
        );
    }

    /// The DNR-18 reconciliation on a 2-net world: Σ per-net routed ==
    /// the tally total (f64, exact on integer DBU), the mean excess is
    /// the mean of the per-net excesses.
    #[test]
    fn t1_reconciliation_per_net_equals_board_total() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n    (net NET2 (pins CMP_C-P1 CMP_D-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 95000 30000 95000 50000 120000 50000)(net NET1)(type route))
    (wire (path F.Cu 200 20000 120000 120000 120000)(net NET2)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.routed_length_dbu, 220_000.0);
        assert_eq!(f64::from(metrics.tally.total_length), 220_000.0);
        assert_eq!(metrics.nets_measured, 2);
        assert!((metrics.mean_length_excess - 0.1).abs() < 1e-12);
    }

    /// A Tier A fixture parse (bm08): the measurer runs over a real
    /// parsed board; the pins-only face (no traces → zero densities,
    /// no qualifying nets, but a positive terminal bound).
    #[test]
    fn t1_bm08_fixture_parse_runs() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
        );
        let board = parse_board_from_path(path);
        let metrics = aesthetics_metrics(&board, board.rules());
        assert_eq!(metrics.tally.trace_count, 0, "bm08 is a pins-only board");
        assert_eq!(metrics.via_density, 0.0);
        assert_eq!(metrics.mean_length_excess, 0.0, "no routed nets qualify");
        assert_eq!(metrics.nets_measured, 0);
        // `mst_lb_dbu` sums the QUALIFYING nets only — no traces, no
        // qualified net, so the sum is 0 even though all 40 pins
        // resolve as terminals (the per-net walk never opens a net
        // without routed length).
        assert_eq!(metrics.mst_lb_dbu, 0.0);
    }

    /// The serialization-edge pin: render_json is byte-stable, carries
    /// the four keys, and rounds to 3 decimals (a value like 1/3
    /// renders "0.333", never 17 digits).
    #[test]
    fn t1_render_json_stable_and_rounded() {
        let board = parse_full(
            "    (net NET1 (pins CMP_A-P1 CMP_B-P1))\n",
            r#"(wiring
    (wire (path F.Cu 200 20000 30000 70000 30000)(net NET1)(type route))
  )
"#,
        );
        let metrics = aesthetics_metrics(&board, board.rules());
        let first = render_json(&metrics);
        let second = render_json(&metrics);
        assert_eq!(first, second);
        assert!(first.contains("bend_to_length_ratio"));
        assert!(first.contains("mean_length_excess"));
        assert!(first.contains("parallelism_ratio"));
        assert!(first.contains("via_density"));
        // No f64 full-precision digit runs: every number is 3-decimal.
        assert!(!first.contains("0.3333333333"));
    }
}
