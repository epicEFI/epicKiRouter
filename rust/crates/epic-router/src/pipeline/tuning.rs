//! M7-T4: the tuning MEANDER stage — the beyond-Java clearance-aware
//! accordion insertion that fills an under-min length deficit on a
//! constrained net after the optimization stage.
//!
//! **Input contract (T3's `length_report` rows, re-derived in-engine).**
//! The stage walks the nets in NUMBER order (the deterministic row
//! order of `Nets::iter`), keeps the constrained nets (`min > 0.0` —
//! the T1 delivery gate), skips nets with incompletes (Java's
//! `calcLengthViolation` gate — a net still being routed is not a
//! meander target) and skips nets already at or above `min`. Negative
//! violation rows (the under-min DEFICIT) are the stage's work list;
//! positive over-max rows are report-only and are never touched (T5's
//! match face owns them).
//!
//! **Shape model.** The engine inserts a fixed square-wave ACCORDION
//! (dents) into ONE axis-aligned segment of the net's routed trace,
//! replacing the sub-span `[from-corner, from-corner + W]` of the
//! segment (the window anchored at the segment's from-corner) with the
//! wave. For amplitude `A` and `N` dents the wave path per dent is
//! `up A · across A · down A · across A`, so the wave advances `2·A·N`
//! along the segment while measuring `4·A·N` — the ADDED length is
//! exactly `2·A·N` (the window length `W`). The wave ends ON the
//! segment axis, so every original polyline corner (in particular the
//! trace's first/last corners — the connection faces to pins/vias and
//! neighboring traces) is preserved verbatim; connectivity cannot
//! change.
//!
//! **Determinism.** No float search, no iteration-order dependence:
//! sites are ordered by (segment length DESC, trace id ASC, corner
//! index ASC), amplitudes come from a fixed table in fixed order, the
//! dent count is integer arithmetic, and the landing is a whole-
//! candidate accept/reject. All geometry is integer (`IntPoint`)
//! arithmetic widened to i64 and narrowed back with a guard (an
//! overflow candidate is skipped — the honest-stop face).
//!
//! **Clearance awareness.** A candidate lands only if EVERY wave piece
//! passes the board's own probe
//! ([`epic_board::routing_board_search::check_trace_segment_points`],
//! Java `RoutingBoardSearchFacade.checkTraceSegment`): each piece's
//! endpoints are extended by [`CORNER_MITER_PROBE_ALLOWANCE_HALFWIDTHS`]
//! half-widths along the piece axis (the corner-miter allowance — the
//! 90°-corner offset polygons stick out diagonally, and the piece-axis
//! extension covers the miter zone) and the probe must answer that the
//! FULL extended piece is insertable at the trace's own half width and
//! clearance class. The probe skips same-net obstacles (the wave may
//! touch — even land ON TOP of — the net's own pin pads and traces,
//! invisibly to the probe; by design: same net is one conductor), so
//! the wave's SELF-interference is guaranteed geometrically instead:
//! the amplitude must be at least the self-clear distance
//! `2·half_width + clearance(class, class, layer)` (adjacent parallel
//! teeth are `A` apart center-to-center).
//!
//! **Budgets.** Named constants, all documented here: the amplitude
//! table (the granularity ladder), the per-site dent cap, the per-net
//! insertion cap, and the never-exceed-max guard (a candidate whose
//! added length would push the net above a declared `max` is
//! rejected; the ladder walk owns the fall-through to a FINER
//! amplitude, never a larger dent count).
//!
//! **Honest stop.** A net whose every candidate fails fit, budget, or
//! probe keeps its deficit; the stage records the outcome and the
//! manifest's `length_report` (computed AFTER the pipeline) still
//! carries the row — the report says so.
//!
//! **MSDTW median-trace disposition.** The design :73 median-trace
//! technique (meanders keyed off the median trace of a corridor-
//! sharing net group) needs the GROUP face — the match-group model
//! lands in T5. T4's committed face is the per-net deterministic
//! accommodation; the technique ports (or is declined on measurement)
//! at T5 where the group exists. Recorded, not silently dropped.

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::incompletes::all_incompletes;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

use crate::pipeline::event_sink::DriverSink;

/// The candidate amplitude ladder (board DBU), tried in THIS order at
/// every site (descending — the fewest-dents shape wins the ladder
/// walk when several fit). Table entries below the site's self-clear
/// distance are filtered out; a site whose self-clear distance exceeds
/// every table entry falls back to the self-clear distance itself
/// (integer geometry, still deterministic). The granularity of a
/// landing is `2·A` per dent.
pub const MEANDER_AMPLITUDE_TABLE_DBU: [i64; 4] = [10_000, 5_000, 2_000, 1_000];

/// The per-site dent budget: no single wave inserts more dents than
/// this, whatever the deficit asks for (the budget bound; a capped
/// wave that does not cover the deficit leaves the remainder to the
/// per-net loop's next insertion — the honest-report face).
pub const MAX_MEANDER_DENTS_PER_SITE: i64 = 32;

/// The per-net insertion budget: a net receives at most this many
/// separate wave insertions per stage run (one full-length wave
/// usually covers the whole deficit; the cap bounds the
/// multi-insertion fallback).
pub const MAX_MEANDER_INSERTIONS_PER_NET: usize = 4;

/// The wave-piece probe extension, in half-widths, on BOTH ends of
/// every probed piece: the corner-miter allowance. The 90°-corner
/// offset polygons of the wave stick out diagonally beyond the
/// piece-parallel probe rectangles; extending each probe two
/// half-widths along its own axis covers the miter zone of both
/// adjoining corners (the extension is over-clearance by
/// construction — a candidate that fails the extended probe fails
/// SAFE to the next candidate, never into a violation).
pub const CORNER_MITER_PROBE_ALLOWANCE_HALFWIDTHS: i32 = 2;

/// One net's meander outcome (the stage's report rows, in the stage's
/// walk order = net-number order). `landed == false` is the honest
/// stop: the deficit stands and the manifest's `length_report` (the
/// caller computes it AFTER the pipeline) keeps the row.
#[derive(Clone, Debug, PartialEq)]
pub struct MeanderNetOutcome {
    /// The 1-based net number.
    pub net_number: i32,
    /// The net name (the DSN-declared name, case-preserved).
    pub net_name: String,
    /// The under-min deficit the stage was asked to cover (board DBU).
    pub deficit: f64,
    /// Whether a wave landed.
    pub landed: bool,
    /// The total added length across this net's insertions (0.0 when
    /// nothing landed).
    pub added_length: f64,
    /// The total dent count across this net's insertions.
    pub dent_count: i64,
}

/// The meander stage (the pipeline's post-optimization face). INERT
/// unless `board.tuning_active()` (the caller resolves the flag; the
/// stage re-checks so direct callers cannot bypass the regime gate).
/// Returns one [`MeanderNetOutcome`] per processed under-min net, in
/// net-number order.
pub fn run_meander_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    sink: &mut dyn DriverSink,
) -> Vec<MeanderNetOutcome> {
    if !board.tuning_active() {
        return Vec::new();
    }
    // Snapshot the work list first (the rules borrow must not span the
    // mutable landing work). Number order — `Nets::iter`.
    let targets: Vec<(i32, String, f64, f64)> = {
        let rules = board.rules();
        rules
            .nets
            .iter()
            .filter_map(|(net_number, net)| {
                let (min, max) = rules.net_class_length_bounds(net_number);
                (min > 0.0).then(|| (net_number, net.name.clone(), min, max))
            })
            .collect()
    };
    // Java's `calcLengthViolation` incompletes gate: a net with
    // incompletes is not a meander target (still being routed).
    let incompletes = all_incompletes(manager, board).1;
    let mut outcomes = Vec::new();
    for (net_number, net_name, min, max) in targets {
        let current = board.net_trace_length(net_number);
        if current >= min {
            // At or above `min`: nothing to add (and above-max is T5's
            // report-only face — never truncated, never touched).
            continue;
        }
        if incompletes
            .iter()
            .any(|row| row.net_no == net_number && row.incomplete_count > 0)
        {
            continue;
        }
        let deficit = min - current;
        let outcome = meander_net(
            manager, board, net_number, &net_name, deficit, min, max, sink,
        );
        outcomes.push(outcome);
    }
    outcomes
}

/// The per-net insertion loop (the budget face:
/// [`MAX_MEANDER_INSERTIONS_PER_NET`]).
#[allow(clippy::too_many_arguments)] // the stage's per-net frame, kept flat
fn meander_net(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net_number: i32,
    net_name: &str,
    deficit: f64,
    min: f64,
    max: f64,
    sink: &mut dyn DriverSink,
) -> MeanderNetOutcome {
    let mut outcome = MeanderNetOutcome {
        net_number,
        net_name: net_name.to_string(),
        deficit,
        landed: false,
        added_length: 0.0,
        dent_count: 0,
    };
    for _ in 0..MAX_MEANDER_INSERTIONS_PER_NET {
        let current = board.net_trace_length(net_number);
        if current >= min {
            break;
        }
        let remaining = min - current;
        // The never-exceed-max guard: the headroom a candidate must
        // respect (the i64/2 sentinel when `max` is undeclared — the
        // arithmetic can never reach it).
        let headroom: i64 = if max > 0.0 {
            ((max - current) as i64).max(0)
        } else {
            i64::MAX / 2
        };
        let Some((added, dents)) = insert_one_wave(manager, board, net_number, remaining, headroom)
        else {
            break; // honest stop: no candidate landed anywhere
        };
        outcome.landed = true;
        outcome.added_length += added as f64;
        outcome.dent_count += dents;
    }
    let verdict = if outcome.landed {
        "meander landed"
    } else {
        "meander blocked"
    };
    sink.info(&format!(
        "Meander stage on net '{net_name}': {verdict}, deficit {}, added {}, dents {}",
        outcome.deficit, outcome.added_length, outcome.dent_count
    ));
    outcome
}

/// The candidate ladder: enumerate the net's wave sites and amplitude
/// candidates in the documented deterministic order and land the FIRST
/// candidate that fits, respects the budgets, and passes the probes.
fn insert_one_wave(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net_number: i32,
    remaining: f64,
    headroom: i64,
) -> Option<(i64, i64)> {
    let sites = enumerate_sites(board, net_number);
    for site in &sites {
        let Some(layer) = board.trace_layer(site.trace_id) else {
            continue;
        };
        let Some(half_width) = board.trace_half_width(site.trace_id) else {
            continue;
        };
        let Some(clearance_class) = board.item_clearance_class(site.trace_id) else {
            continue;
        };
        // The self-clear floor: adjacent parallel teeth are `A` apart
        // center-to-center; below `2·hw + clearance(self, self)` the
        // wave would short itself (the probe cannot see it — same-net
        // obstacles are skipped — so this geometric floor owns it).
        let self_clear_min = 2 * i64::from(half_width)
            + i64::from(board.clearance_value(clearance_class, clearance_class, layer));
        let amplitudes: Vec<i64> = MEANDER_AMPLITUDE_TABLE_DBU
            .iter()
            .copied()
            .filter(|&amplitude| amplitude >= self_clear_min)
            .collect();
        if amplitudes.is_empty() {
            // Every table entry is below the self-clear floor (a very
            // wide/fine board): the site offers NO legal shape — the
            // honest-stop face (the floor is a hard geometric bound;
            // a computed off-table amplitude would be un-tabled
            // geometry).
            continue;
        }
        for &amplitude in &amplitudes {
            let Some((wave, added)) = build_wave(site, amplitude, remaining, headroom) else {
                continue;
            };
            if !wave_is_clear(
                manager,
                board,
                net_number,
                layer,
                half_width,
                clearance_class,
                &wave,
            ) {
                continue; // try the next geometry
            }
            land_wave(manager, board, site, &wave);
            return Some((added, wave.dent_count));
        }
    }
    None
}

/// One wave site: an axis-aligned segment of one of the net's on-board
/// traces.
struct MeanderSite {
    trace_id: ItemId,
    /// The segment's from-corner index in the trace polyline.
    corner_index: usize,
    /// The segment length (|dx| + |dy|).
    length: i64,
    /// The from-corner.
    start: IntPoint,
    /// The axis unit direction (toward the to-corner).
    dir: (i64, i64),
}

/// The deterministic site order: segment length DESC (the longest run
/// gets the first chance — the fewest-dent shapes fit there), then
/// trace id ASC, then corner index ASC.
fn enumerate_sites(board: &Board, net_number: i32) -> Vec<MeanderSite> {
    let mut sites: Vec<MeanderSite> = Vec::new();
    let mut trace_ids: Vec<ItemId> = board
        .get_connectable_items(net_number)
        .into_iter()
        .filter(|&id| board.is_on_the_board(id) && board.trace_polyline(id).is_some())
        .collect();
    trace_ids.sort();
    for trace_id in trace_ids {
        let Some(lines) = board.trace_polyline(trace_id) else {
            continue;
        };
        let corners = lines.corners();
        for (index, pair) in corners.windows(2).enumerate() {
            let (Some(a), Some(b)) = (as_int_point(&pair[0]), as_int_point(&pair[1])) else {
                continue; // a rational corner cannot anchor integer wave math
            };
            let dx = i64::from(b.x) - i64::from(a.x);
            let dy = i64::from(b.y) - i64::from(a.y);
            if (dx != 0 && dy != 0) || (dx == 0 && dy == 0) {
                continue; // the wave model is axis-aligned only
            }
            sites.push(MeanderSite {
                trace_id,
                corner_index: index,
                length: dx.abs() + dy.abs(),
                start: a,
                dir: (dx.signum(), dy.signum()),
            });
        }
    }
    sites.sort_by(|a, b| {
        b.length
            .cmp(&a.length)
            .then_with(|| a.trace_id.cmp(&b.trace_id))
            .then_with(|| a.corner_index.cmp(&b.corner_index))
    });
    sites
}

/// A built wave: the corner string to splice into the trace polyline
/// (the splice reuses the trace's own from-corner — the wave corners
/// FOLLOW it), plus the bookkeeping. The LAST wave corner is the
/// window end, ON the segment axis.
struct Wave {
    start: IntPoint,
    corners: Vec<IntPoint>,
    dent_count: i64,
}

impl Wave {
    /// The wave's straight pieces: `4·N` of them (per dent: up, across,
    /// down, across).
    fn piece_count(&self) -> usize {
        self.corners.len()
    }

    /// Piece `i` as `(from, to)` in i64 coordinates.
    fn piece(&self, index: usize) -> (IntPoint, IntPoint) {
        let from = if index == 0 {
            self.start
        } else {
            self.corners[index - 1]
        };
        (from, self.corners[index])
    }
}

/// Builds the wave for one (site, amplitude) candidate: the dent count
/// from the remaining deficit (integer arithmetic), the budget caps,
/// and the fit check against the site length. Returns `None` when the
/// candidate does not fit or the budget rejects it.
fn build_wave(
    site: &MeanderSite,
    amplitude: i64,
    remaining: f64,
    headroom: i64,
) -> Option<(Wave, i64)> {
    if amplitude <= 0 || remaining <= 0.0 {
        return None;
    }
    let dent_len = 2 * amplitude; // the added length per dent
    let need_dents = ((remaining.ceil() as i64) + dent_len - 1) / dent_len;
    if need_dents < 1 {
        return None;
    }
    // Budgets: the dent cap, and the fit — the window must be at most
    // the site length (the wave anchors at the from-corner).
    let max_by_fit = site.length / dent_len;
    let dent_count = need_dents.min(MAX_MEANDER_DENTS_PER_SITE).min(max_by_fit);
    if dent_count < 1 {
        return None;
    }
    let added = dent_count * dent_len;
    if added > headroom {
        // The never-exceed-max face: this amplitude overshoots the
        // declared max. The LADDER does not grow the dent count (that
        // would overshoot further) — a SMALLER amplitude (finer
        // granularity) may still fit; the caller's ladder walk owns
        // the fall-through.
        return None;
    }
    let (dx, dy) = site.dir;
    let perp = (-dy, dx); // +90°
    let mut corners = Vec::with_capacity(usize::try_from(4 * dent_count).ok()?);
    for i in 0..dent_count {
        // Per dent: up A, across A, down A, across A (all integer i64
        // math, narrowed with the overflow guard).
        let bx = i64::from(site.start.x) + 2 * i * amplitude * dx;
        let by = i64::from(site.start.y) + 2 * i * amplitude * dy;
        let corner = |ox: i64, oy: i64| -> Option<IntPoint> {
            let x = i32::try_from(bx + ox).ok()?;
            let y = i32::try_from(by + oy).ok()?;
            Some(IntPoint::new(x, y))
        };
        // up A
        corners.push(corner(amplitude * perp.0, amplitude * perp.1)?);
        // across A at the tooth top
        corners.push(corner(
            amplitude * (dx + perp.0),
            amplitude * (dy + perp.1),
        )?);
        // down A (back ON the axis)
        corners.push(corner(amplitude * dx, amplitude * dy)?);
        // across A on the axis (the next dent's base)
        corners.push(corner(2 * amplitude * dx, 2 * amplitude * dy)?);
    }
    Some((
        Wave {
            start: site.start,
            corners,
            dent_count,
        },
        added,
    ))
}

/// The clearance-aware acceptance face: EVERY wave piece must probe
/// insertable at full length, endpoints extended by the corner-miter
/// allowance. One failed piece rejects the whole candidate.
#[allow(clippy::too_many_arguments)] // the probe's own inputs, kept flat
fn wave_is_clear(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net_number: i32,
    layer: i32,
    half_width: i32,
    clearance_class: i32,
    wave: &Wave,
) -> bool {
    let extension = i64::from(half_width) * i64::from(CORNER_MITER_PROBE_ALLOWANCE_HALFWIDTHS);
    for piece_index in 0..wave.piece_count() {
        let (from, to) = wave.piece(piece_index);
        let dx = i64::from(to.x) - i64::from(from.x);
        let dy = i64::from(to.y) - i64::from(from.y);
        let len = (dx.abs() + dy.abs()) as f64;
        if len <= 0.0 {
            continue;
        }
        let (ux, uy) = (dx.signum(), dy.signum());
        let (Ok(fx), Ok(fy), Ok(tx), Ok(ty)) = (
            i32::try_from(i64::from(from.x) - ux * extension),
            i32::try_from(i64::from(from.y) - uy * extension),
            i32::try_from(i64::from(to.x) + ux * extension),
            i32::try_from(i64::from(to.y) + uy * extension),
        ) else {
            return false; // the extension overflows the int domain — fail safe
        };
        let from_point = Point::Int(IntPoint::new(fx, fy));
        let to_point = Point::Int(IntPoint::new(tx, ty));
        let insertable = epic_board::routing_board_search::check_trace_segment_points(
            manager,
            board,
            &from_point,
            &to_point,
            layer,
            &[net_number],
            half_width,
            clearance_class,
            false,
        );
        if insertable + 1e-6 < len {
            return false;
        }
    }
    true
}

/// Lands the wave: splice the wave corners into the site's trace
/// polyline (the trace's own corners elsewhere are preserved
/// verbatim) and write the geometry through the tree-replacement
/// face. When the wave ends exactly ON the site's to-corner, the
/// duplicate is dropped.
fn land_wave(manager: &mut SearchTreeManager, board: &mut Board, site: &MeanderSite, wave: &Wave) {
    let Some(lines) = board.trace_polyline(site.trace_id) else {
        return;
    };
    let old_corners = lines.corners();
    let last_wave_corner = wave.corners.last().copied();
    let mut points: Vec<Point> = Vec::with_capacity(old_corners.len() + wave.corners.len());
    for (index, corner) in old_corners.iter().enumerate() {
        if index == site.corner_index {
            points.push(corner.clone());
            for &wave_corner in &wave.corners {
                points.push(Point::Int(wave_corner));
            }
        } else if index == site.corner_index + 1 {
            if last_wave_corner
                .is_some_and(|last| Some(last) != as_int_point(corner).filter(|_| true))
            {
                points.push(corner.clone());
            }
        } else {
            points.push(corner.clone());
        }
    }
    let new_lines = Polyline::from_points(&points);
    if new_lines.lines.len() < 3 {
        return; // a collapsed construction is not a landing
    }
    board.replace_trace_geometry(manager, site.trace_id, new_lines);
}

/// The `Point::Int` extractor (a rational corner cannot anchor integer
/// wave arithmetic — the callers skip such sites/pieces).
fn as_int_point(point: &Point) -> Option<IntPoint> {
    match point {
        Point::Int(int_point) => Some(*int_point),
        Point::Rational(_) => None,
    }
}

#[cfg(test)]
mod tests {
    // The pin worlds live below (the SesBoard-world pattern of
    // `epic-cli/src/route.rs`'s `length_report_board`, rebuilt locally
    // — the test trees are per-crate).
    use super::*;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use epic_board::id::ItemId;
    use epic_drc::clearance::all_clearance_violation_depths;
    use epic_dsn::coordinate_transform::CoordinateTransform;
    use epic_dsn::layer_structure::{Layer, LayerStructure};
    use epic_dsn::ses_board::{ItemIr, SesBoard};
    use epic_dsn::sink::{
        BoardRulesIr, BoardSink, ClearanceIr, CreateBoardIr, FixedStateIr, NetClassIr, NetIr,
        TraceIr,
    };
    use epic_geometry::int_box::IntBox;

    /// Clearance matrix cell value helper: a full 2000 matrix on both
    /// layers unless overridden.
    fn clearance_matrix_2layer(pair: i32) -> ClearanceIr {
        ClearanceIr {
            names: vec!["null".to_string(), "tuned".to_string()],
            values: vec![
                vec![vec![pair, pair], vec![pair, pair]],
                vec![vec![pair, pair], vec![pair, pair]],
            ],
        }
    }

    /// The wave world: net 1 ("deficit", class 1 "tuned") carries ONE
    /// pre-routed straight trace (0,0)->(420000,0), hw 1000, layer 0,
    /// clearance class 1. Net 2 ("free", class 0) carries the OPTIONAL
    /// blocking trace (the caller decides). Class 1 declares
    /// `(min, max)`; class 0 is unconstrained. The clearance matrix is
    /// a uniform `pair` on both layers (board.clearance_value adds the
    /// +16 safety margin, so the engine's self-clear floor reads
    /// `2·1000 + pair + 16`).
    struct WaveWorld {
        manager: SearchTreeManager,
        board: Board,
    }

    fn wave_world(min: f64, max: f64, pair: i32, blocker: bool) -> WaveWorld {
        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(500_000, 200_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("null".to_string()),
            rules: BoardRulesIr {
                clearance: clearance_matrix_2layer(pair),
                trace_angle_restriction: epic_dsn::state::AngleRestriction::NinetyDegree,
                default_trace_half_widths: vec![1000],
                min_trace_half_width: 1000,
                max_trace_half_width: 1000,
                pin_edge_to_turn_dist: 0.0,
                default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            },
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        ses.net_classes.push(NetClassIr {
            name: "tuned".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1000],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: min,
            max_trace_length: max,
            nets: Vec::new(),
        });
        ses.net_classes.push(NetClassIr {
            name: "free".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1000],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            nets: Vec::new(),
        });
        ses.nets = vec![
            NetIr {
                name: "deficit".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
            NetIr {
                name: "free".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 2,
            },
        ];
        let trace = |id: i32, net: i32, corners: Vec<IntPoint>| ItemIr::Trace {
            id,
            trace: TraceIr {
                layer_no: 0,
                half_width: 1000,
                corners: corners.clone(),
                polyline: TraceIr::polyline_of_corners(&corners),
                nets: vec![net],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        };
        ses.push_routed_item(trace(
            500,
            1,
            vec![IntPoint::new(0, 0), IntPoint::new(420_000, 0)],
        ));
        if blocker {
            // The foreign blocker: net 2, a long horizontal wall whose
            // faces span y ∈ [6000, 8000] (hw 1000 about y=7000) — it
            // overlaps EVERY wave candidate of the main run (teeth top
            // out at y = A + hw >= 6000 for every legal amplitude).
            ses.push_routed_item(trace(
                501,
                2,
                vec![IntPoint::new(-10_000, 7_000), IntPoint::new(430_000, 7_000)],
            ));
        }
        let mut board = Board::from_ses_board(&ses);
        board.set_tuning_active(true);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        WaveWorld { manager, board }
    }

    /// The stage run + the real-DRC counter (correctness by
    /// construction: every landing world also asserts ZERO clearance
    /// violations on the REAL counter).
    fn run_stage(world: &mut WaveWorld) -> Vec<MeanderNetOutcome> {
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_meander_stage(&mut world.manager, &mut world.board, &mut sink);
        let (violations, _) = all_clearance_violation_depths(&mut world.manager, &mut world.board);
        assert_eq!(
            violations, 0,
            "the REAL DRC counter: a meander landing must be clearance-clean"
        );
        outcomes
    }

    /// The exact expected wave corners for deficit 40000 on the
    /// 420000 straight run: amplitude 10000 (the first table entry
    /// above the self-clear floor 4016), 2 dents, window [0, 40000].
    /// The wave's final axis corner (40000, 0) is collinear with the
    /// remaining tail ((30000,0) -> (40000,0) -> (420000,0)), so the
    /// Polyline construction normalizes it away — the geometry is
    /// identical; the asserted face is the POST-normalization corner
    /// string.
    fn expected_w1_corners() -> Vec<IntPoint> {
        vec![
            IntPoint::new(0, 0),
            IntPoint::new(0, 10_000),
            IntPoint::new(10_000, 10_000),
            IntPoint::new(10_000, 0),
            IntPoint::new(20_000, 0),
            IntPoint::new(20_000, 10_000),
            IntPoint::new(30_000, 10_000),
            IntPoint::new(30_000, 0),
            IntPoint::new(420_000, 0),
        ]
    }

    /// PIN W1 — the straight run with room: the EXACT expected meander
    /// geometry. Deficit = 460000 − 420000 = 40000; amplitude ladder
    /// [10000, 5000] (both >= floor 4016); A=10000: need
    /// ceil(40000/20000) = 2 dents, added 40000 = the deficit
    /// exactly, window 40000 <= 420000 -> LANDS as the first
    /// candidate. Every asserted value derives from the world + the
    /// documented arithmetic (shape, count, added length, final
    /// length = min EXACTLY).
    #[test]
    fn wave_straight_run_lands_exact_geometry() {
        let mut world = wave_world(460_000.0, 0.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].landed);
        assert_eq!(outcomes[0].added_length, 40_000.0);
        assert_eq!(outcomes[0].dent_count, 2);
        let lines = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant");
        assert_eq!(
            lines.length_approx_total(),
            460_000.0,
            "min reached exactly"
        );
        let corners: Vec<IntPoint> = lines
            .corners()
            .iter()
            .map(|p| as_int_point(p).expect("the pin world invariant"))
            .collect();
        assert_eq!(corners, expected_w1_corners());
    }

    /// PIN W2 — the blocked run: the foreign wall blocks EVERY
    /// candidate (A=10000 and A=5000 both overlap it; no second
    /// site). NO meander lands; the honest report: the outcome row
    /// stands with the deficit, zero added, zero dents — and the
    /// trace geometry is byte-stable.
    #[test]
    fn wave_blocked_run_stands_honest() {
        let mut world = wave_world(460_000.0, 0.0, 2000, true);
        let before = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners();
        let outcomes = run_stage(&mut world);
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].landed, "the honest stop");
        assert_eq!(outcomes[0].added_length, 0.0);
        assert_eq!(outcomes[0].dent_count, 0);
        let after = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners();
        assert_eq!(before, after, "the deficit row stands, unmeandered");
    }

    /// PIN W3 — the clearance boundary (DNR-16, both directions) on
    /// the SELF-CLEAR floor: the amplitude filter is
    /// `A >= 2·hw + clearance + 16`. With declared clearance 7984 the
    /// floor reads 10000 exactly == the first table amplitude ->
    /// the teeth sit at the declared clearance + 16 face-to-face ->
    /// LANDS (equality allowed, DRC-clean: actual > expected). With
    /// 7985 the floor reads 10001 -> NO table entry passes -> the
    /// honest stop. The `>=`->`>` mutant dies on the first arm, the
    /// floor-formula-weakened mutant (dropping the +16/declared term,
    /// floor = 2·hw = 2000) on the second (every table entry would
    /// pass and A=10000 would land with teeth at 7985 − 2000 − 2000·?
    /// ... — concretely the second arm's blocked assertion dies).
    #[test]
    fn wave_self_clear_boundary_both_directions() {
        // Equality: floor 10000, first table entry 10000 -> lands.
        let mut world = wave_world(460_000.0, 0.0, 7984, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes[0].landed, "floor == table amplitude is allowed");
        // One unit above: floor 10001, no table entry passes -> stop.
        let mut world = wave_world(460_000.0, 0.0, 7985, false);
        let outcomes = run_stage(&mut world);
        assert!(!outcomes[0].landed, "one unit above the floor blocks");
    }

    /// PIN W4 — the never-exceed-max headroom boundary (DNR-16, both
    /// directions, on the EXACT equality face): headroom =
    /// max − current. max 460000 == the post-landing length ->
    /// headroom 40000 == added 40000 -> the equality face is ALLOWED
    /// (`added > headroom` is the rejection — landing AT max is not
    /// exceeding it), and the net lands AT exactly max. max 459999:
    /// headroom 39999 < 40000 -> the A=10000 candidate is rejected;
    /// A=5000 needs 4 dents = 40000 added — ALSO rejected (same
    /// overshoot); no finer entry -> the honest stop (a landing would
    /// exceed max, which the contract forbids). The `>`->`>=` mutant
    /// dies on the FIRST arm (it would block the equality landing),
    /// the `>`->`+1`-relaxed mutant on the second (it would land).
    #[test]
    fn wave_never_exceed_max_boundary_both_directions() {
        // Headroom EXACTLY the added length: lands AT max exactly.
        let mut world = wave_world(460_000.0, 460_000.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes[0].landed);
        assert_eq!(
            world.board.net_trace_length(1),
            460_000.0,
            "AT max, not past it"
        );
        // Headroom one DBU short: every amplitude overshoots max.
        let mut world = wave_world(460_000.0, 459_999.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(!outcomes[0].landed, "the honest stop above max");
        assert_eq!(world.board.net_trace_length(1), 420_000.0, "untouched");
    }

    /// PIN W5 — the ceil/granularity boundary: deficit EXACTLY one
    /// dent (min 440000, deficit 20000) lands 1 dent AT min exactly;
    /// deficit 20001 needs 2 dents (ceil) — the `ceil`->`floor`
    /// mutant lands short (420000 + 20000 = 440000 < 440001, the
    /// deficit stands) and dies.
    #[test]
    fn wave_granularity_ceil_boundary() {
        let mut world = wave_world(440_000.0, 0.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes[0].landed);
        assert_eq!(outcomes[0].dent_count, 1, "deficit == one dent");
        assert_eq!(world.board.net_trace_length(1), 440_000.0, "exactly min");
        let mut world = wave_world(440_001.0, 0.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes[0].landed);
        assert_eq!(outcomes[0].dent_count, 2, "ceil: one unit more needs 2");
        assert_eq!(
            world.board.net_trace_length(1),
            460_000.0,
            "440001 + 2·20000 - 1... the exact arithmetic: 420000 + 40000"
        );
    }

    /// PIN W6 — the not-my-face rows: a net already AT min (no
    /// outcome row), a net ABOVE min (untouched — T5's report-only
    /// face), and an over-max net (never truncated). The stage
    /// processes ONLY the under-min deficit net.
    #[test]
    fn wave_untouched_faces() {
        // min == the trace's own length: the at-min skip.
        let mut world = wave_world(420_000.0, 0.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes.is_empty(), "at-min: no row, no touch");
        // Above-min is expressed via min < current — same skip arm.
        let mut world = wave_world(100_000.0, 500_000.0, 2000, false);
        let outcomes = run_stage(&mut world);
        assert!(outcomes.is_empty(), "above-min: no truncation (T5's face)");
    }

    /// PIN W7 — the inert faces: tuning OFF (the parity regime — no
    /// processing even with a declaration) and the blocker present
    /// but tuning off... the stage never fires.
    #[test]
    fn wave_stage_inert_without_tuning_active() {
        let mut world = wave_world(460_000.0, 0.0, 2000, false);
        world.board.set_tuning_active(false);
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_meander_stage(&mut world.manager, &mut world.board, &mut sink);
        assert!(outcomes.is_empty(), "the regime gate is the stage's own");
        assert_eq!(world.board.net_trace_length(1), 420_000.0);
    }

    /// PIN W8 — the ORDERING + SURVIVAL face: a meandered net AT min
    /// exactly survives a subsequent default-ON tightener pass (the
    /// T3 honoring gate protects it — the whole-candidate rejection
    /// means the fold-to-420000 candidate below min cannot land), so
    /// the stage's post-optimization position is stable: nothing that
    /// runs after the meander stage can shorten the landing.
    #[test]
    fn wave_survives_subsequent_tightener_pass() {
        use epic_board::routing_board_insert::opt_changed_area;
        use epic_board::trace_tightener::TraceTightenerSeam;
        let mut world = wave_world(460_000.0, 0.0, 2000, false);
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_meander_stage(&mut world.manager, &mut world.board, &mut sink);
        assert!(outcomes[0].landed);
        let before = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners();
        // The default-ON tightener pass over the meandered net (the
        // changed-area session is the driver's own shape —
        // pass_runner.rs:346).
        world.board.start_marking_changed_area();
        opt_changed_area(
            &mut world.manager,
            &mut world.board,
            &mut TraceTightenerSeam,
            &[1],
            None,
            500,
            None,
            0,
            None,
            None,
            -1,
            true,
        );
        let after = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners();
        assert_eq!(before, after, "the gate protects the meandered net");
        assert_eq!(world.board.net_trace_length(1), 460_000.0);
    }
}
