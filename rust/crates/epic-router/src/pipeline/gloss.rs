//! M8-T3: the GLOSS stage family home — the parallel-bus GROUP
//! detector and the hug/spread re-spacing pass, both behind
//! `router.gloss.bus` (tri-state, default OFF — the two-regime law:
//! default runs never reach this module, so default output stays
//! byte-identical; ON runs are the improvement regime, golden-captured
//! at first landing).
//!
//! **Group detection (advisory-first, the islands precedent).** Over
//! ROUTED nets (at least one on-board trace; plane nets never
//! participate): two nets GROUP when their traces share at least one
//! same-layer AXIS-ALIGNED parallel span — same orientation (both
//! horizontal or both vertical), perpendicular offset within the
//! coupling window ([`COUPLING_WINDOW_DBU`], the tied constant,
//! inclusive), and an axis-projection overlap STRICTLY longer than
//! [`BUS_SPAN_THRESHOLD_DBU`]. Transitively closed over union-find;
//! deterministic: nets iterate in number order, pairs in (a, b)
//! lexicographic order, groups report members ascending and order by
//! min member. CROSSING IS NOT PARALLEL: a perpendicular crossing
//! (distance zero at the X) differs in orientation and never
//! qualifies — pinned. Scope limit, documented honestly: the detector
//! and the pass are AXIS-ALIGNED only; 45-degree-diagonal parallel
//! runs are T4 flow territory — neither grouped nor moved.
//!
//! **The hug/spread pass (per detected group).** CORRIDOR vote: per
//! (layer, orientation) key, the distinct member nets holding a
//! qualifying pair on the key; the corridor is the key with the MOST
//! distinct nets (count tie: horizontal before vertical, then the
//! lowest layer — the scan is deterministic). Each corridor
//! participant contributes its LONGEST qualifying-on-the-key segment
//! (tie: trace id ascending, then segment start) as its SPAN; the
//! span's perpendicular coordinate is the member's POSITION. SPREAD
//! re-spaces the ladder to a uniform pitch: target pitch = the MEDIAN
//! existing pitch (odd count: the middle element; even: the
//! round-average of the two middle elements — f64 `.round()`, half
//! away from zero, integer result), anchored on the median-position
//! member (index (n-1)/2 of the position-sorted ladder — the LOWER
//! middle for even n), targets `anchor + (i - anchor_index) *
//! median_pitch`. HUG is the clearance-permitting approach ladder
//! toward that target: at most TWO whole-candidate relocations per
//! member — the FULL delta, then HALF the delta (i64 division,
//! truncating toward zero) — so a member lands as far toward the
//! corridor as clearance permits; a rejected candidate does not land
//! and the member's ladder stops there (the honest-stop face,
//! recorded in the sidecar).
//!
//! **The relocation candidate.** A member moves by rewriting its
//! ANCHOR TRACE's polyline: every INTERIOR corner whose perpendicular
//! coordinate equals the span position shifts by the step along the
//! perpendicular axis; the trace's two END corners (the connection
//! faces) never move, and every other corner is preserved verbatim
//! (a non-integer span corner cannot shift exactly — the candidate is
//! rejected, never a silent partial shift). A trace with no interior
//! span corner (a straight pin-to-pin run) has no candidate and
//! records REJECTED. USER_FIXED and SYSTEM_FIXED traces never move
//! (recorded, never guessed).
//!
//! **Acceptance (the meander-engine discipline).** EVERY candidate
//! passes, in order: (a) the fixed-state gate; (b) the length guard —
//! the candidate's piece-sum length may not exceed the current length
//! by more than [`BUS_MAX_LENGTH_GAIN_DBU`] (equality allowed; hug
//! must shorten or hold — the AM2 direction predicates); (c) the REAL
//! clearance probe — every polyline piece probed at full length via
//! `epic_board::routing_board_search::check_trace_segment_points`
//! (exact corners; same-net items are exempt by the tree query, the
//! `wave_is_clear` precedent), one failed piece rejecting the WHOLE
//! candidate. A landed candidate writes through
//! `Board::replace_trace_geometry` (the tree-replacement face). The
//! stage emits NO events and NO stage transition (the meander-stage
//! precedent).
//!
//! **Determinism.** Group order by min net id; the sweep order is
//! participant NET order (an earlier member's landed geometry is
//! visible to later probes — the board is the single source of
//! truth); no threads, no hash-order iteration.
//!
//! **Two-regime safety.** The ONLY caller gate is the resolved
//! `router.gloss.bus` flag (`BatchSettings::bus_active`, default
//! false); an OFF run never constructs a report and never touches the
//! board. The report rides the `--dump-aesthetics` SIDECAR ONLY (the
//! `bus_groups` block rendered by epic-cli — NEVER the manifest: the
//! canary cf607714 pins manifest bytes).

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::FixedState;
use epic_board::routing_board_search::check_trace_segment_points;
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

use crate::pipeline::pairs::COUPLING_WINDOW_DBU;

/// The bus SPAN THRESHOLD, in board DBU: two nets group only when they
/// share a parallel span STRICTLY longer than this (the plan's
/// "longer than a named threshold" — equality does NOT group; the
/// plus/minus 1 DBU boundary worlds and the both-direction mutations
/// pin the strict edge). Value: 100_000 DBU = 10 mm at the um-10
/// transform.
pub const BUS_SPAN_THRESHOLD_DBU: i64 = 100_000;

/// The candidate LENGTH-GAIN guard, in board DBU: a relocation whose
/// piece-sum length exceeds the member's current length by more than
/// this is rejected (equality allowed). Hug must shorten or hold (the
/// AM2 length-excess direction predicate); spread re-spacing around
/// the median anchor mostly shortens, and a jog stretch above this
/// bound is refused. Value: 40_000 DBU = 4 mm = two coarsest meander
/// wave granularities. The exact edge is pinned at [`length_guard_ok`]
/// (old + GAIN lands, old + GAIN + 1 rejects) with both-direction
/// mutations (DNR-16).
pub const BUS_MAX_LENGTH_GAIN_DBU: i64 = 40_000;

/// One AXIS-ALIGNED trace segment (the detector/pass working unit):
/// layer, orientation, the perpendicular coordinate, and the
/// along-axis interval [lo, hi] (ascending; len = hi - lo).
#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisSegment {
    layer: i32,
    horizontal: bool,
    perp: i64,
    lo: i64,
    hi: i64,
    trace_id: ItemId,
}

impl AxisSegment {
    fn len(&self) -> i64 {
        self.hi - self.lo
    }
}

/// The qualifying-span predicate between two axis segments: same
/// layer, same orientation, perpendicular offset within the (tied)
/// coupling window (inclusive), and an axis-projection overlap
/// STRICTLY longer than [`BUS_SPAN_THRESHOLD_DBU`].
fn qualifying_overlap(a: &AxisSegment, b: &AxisSegment) -> Option<i64> {
    if a.layer != b.layer || a.horizontal != b.horizontal {
        return None;
    }
    let offset = (a.perp - b.perp).abs();
    if offset as f64 > COUPLING_WINDOW_DBU {
        return None;
    }
    let overlap = a.hi.min(b.hi) - a.lo.max(b.lo);
    if overlap > BUS_SPAN_THRESHOLD_DBU {
        Some(overlap)
    } else {
        None
    }
}

fn as_int(point: &Point) -> Option<IntPoint> {
    match point {
        Point::Int(int_point) => Some(*int_point),
        Point::Rational(_) => None,
    }
}

/// The net's axis-aligned segments over all on-board traces
/// (deterministic: trace id ascending, corner ascending — the
/// `collect_corridor_segments` walk order).
fn net_axis_segments(board: &Board, net: i32) -> Vec<AxisSegment> {
    let mut trace_ids: Vec<_> = board
        .get_connectable_items(net)
        .into_iter()
        .filter(|&id| board.is_on_the_board(id) && board.trace_polyline(id).is_some())
        .collect();
    trace_ids.sort();
    let mut out = Vec::new();
    for trace_id in trace_ids {
        let Some(lines) = board.trace_polyline(trace_id) else {
            continue;
        };
        let Some(layer) = board.trace_layer(trace_id) else {
            continue;
        };
        for pair in lines.corners().windows(2) {
            let (Some(a), Some(b)) = (as_int(&pair[0]), as_int(&pair[1])) else {
                continue;
            };
            let dx = i64::from(b.x) - i64::from(a.x);
            let dy = i64::from(b.y) - i64::from(a.y);
            let (horizontal, perp, lo, hi) = if dx != 0 && dy == 0 {
                (
                    true,
                    i64::from(a.y),
                    i64::from(a.x).min(i64::from(b.x)),
                    i64::from(a.x).max(i64::from(b.x)),
                )
            } else if dx == 0 && dy != 0 {
                (
                    false,
                    i64::from(a.x),
                    i64::from(a.y).min(i64::from(b.y)),
                    i64::from(a.y).max(i64::from(b.y)),
                )
            } else {
                continue; // diagonal or zero-length: outside the axis-aligned scope
            };
            out.push(AxisSegment {
                layer,
                horizontal,
                perp,
                lo,
                hi,
                trace_id,
            });
        }
    }
    out
}

/// One detected bus group (the report face): member net numbers
/// ascending with their resolved names, the chosen corridor, and the
/// pass's honest per-member rows.
#[derive(Clone, Debug, PartialEq)]
pub struct BusGroupOutcome {
    /// Member net numbers, ascending.
    pub members: Vec<i32>,
    /// Member net names, in member order (the sidecar face).
    pub member_names: Vec<String>,
    /// The corridor layer.
    pub layer: i32,
    /// The corridor orientation (true = horizontal).
    pub horizontal: bool,
    /// Members whose relocation LANDED (full or half step).
    pub moves_landed: u32,
    /// Members whose ladder ended with NO landing (both candidates
    /// rejected, or no movable candidate existed).
    pub moves_rejected: u32,
    /// Per-moved-member rows (net order ascending; members already at
    /// their target do not appear — no candidate, no row).
    pub rows: Vec<BusMoveRow>,
}

/// One member's relocation attempt (the honest-stop face).
#[derive(Clone, Debug, PartialEq)]
pub struct BusMoveRow {
    pub net: i32,
    pub net_name: String,
    /// The span position at stage entry, board DBU.
    pub from_pos: i64,
    /// The uniform-pitch target, board DBU.
    pub target_pos: i64,
    /// Some(landed position) when a candidate landed; None when the
    /// ladder stopped without landing (every candidate rejected).
    pub landed: Option<i64>,
}

/// The gloss BUS stage report (the sidecar's `bus_groups` block
/// source). Default = empty = the OFF face (never serialized).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlossBusReport {
    pub groups: Vec<BusGroupOutcome>,
}

impl GlossBusReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

fn has_on_board_traces(board: &Board, net: i32) -> bool {
    board
        .get_connectable_items(net)
        .into_iter()
        .any(|id| board.is_on_the_board(id) && board.trace_polyline(id).is_some())
}

/// The GROUP DETECTOR: routed nets, qualifying shared parallel spans,
/// transitive closure (union-find), deterministic order. Components
/// with fewer than two nets are dropped.
#[must_use]
pub fn detect_bus_groups(board: &Board) -> Vec<Vec<i32>> {
    let routed: Vec<i32> = {
        let rules = board.rules();
        rules
            .nets
            .iter()
            .filter(|(number, net)| !net.contains_plane && has_on_board_traces(board, *number))
            .map(|(number, _)| number)
            .collect()
    };
    let segments: Vec<(i32, Vec<AxisSegment>)> = routed
        .iter()
        .map(|net| (*net, net_axis_segments(board, *net)))
        .collect();
    let mut parent: Vec<usize> = (0..routed.len()).collect();
    for a in 0..routed.len() {
        for b in (a + 1)..routed.len() {
            let qualifies = segments[a].1.iter().any(|sa| {
                segments[b]
                    .1
                    .iter()
                    .any(|sb| qualifying_overlap(sa, sb).is_some())
            });
            if qualifies {
                let mut ra = a;
                while parent[ra] != ra {
                    ra = parent[ra];
                }
                let mut rb = b;
                while parent[rb] != rb {
                    rb = parent[rb];
                }
                if ra != rb {
                    if ra < rb {
                        parent[rb] = ra;
                    } else {
                        parent[ra] = rb;
                    }
                }
            }
        }
    }
    let mut components: std::collections::BTreeMap<usize, Vec<i32>> =
        std::collections::BTreeMap::new();
    for (index, net) in routed.iter().enumerate() {
        let mut root = index;
        while parent[root] != root {
            root = parent[root];
        }
        components.entry(root).or_default().push(*net);
    }
    components
        .into_values()
        .filter(|members| members.len() >= 2)
        .collect()
}

/// One corridor participant's working frame.
struct Participant {
    net: i32,
    net_name: String,
    /// The anchor trace (the relocation target).
    trace_id: ItemId,
    /// The anchor segment (the member's span).
    span: AxisSegment,
}

/// The per-group HUG/SPREAD pass (members ascending).
fn run_group_pass(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    members: &[i32],
) -> BusGroupOutcome {
    let net_name = |board: &Board, net: i32| -> String {
        board
            .rules()
            .nets
            .get(net)
            .map_or_else(|| format!("net {net}"), |row| row.name.clone())
    };
    let mut outcome = BusGroupOutcome {
        members: members.to_vec(),
        member_names: members.iter().map(|net| net_name(board, *net)).collect(),
        layer: 0,
        horizontal: true,
        moves_landed: 0,
        moves_rejected: 0,
        rows: Vec::new(),
    };
    let member_segments: Vec<(i32, Vec<AxisSegment>)> = members
        .iter()
        .map(|net| (*net, net_axis_segments(board, *net)))
        .collect();
    // Corridor key vote: per (layer, orientation), the DISTINCT member
    // nets holding at least one qualifying pair on the key. BTreeMap
    // order = (layer ascending, false < true) — deterministic.
    let mut votes: std::collections::BTreeMap<(i32, bool), Vec<i32>> =
        std::collections::BTreeMap::new();
    for a in 0..member_segments.len() {
        for b in (a + 1)..member_segments.len() {
            for sa in &member_segments[a].1 {
                for sb in &member_segments[b].1 {
                    if qualifying_overlap(sa, sb).is_some() {
                        let key = (sa.layer, sa.horizontal);
                        let entry = votes.entry(key).or_default();
                        if !entry.contains(&member_segments[a].0) {
                            entry.push(member_segments[a].0);
                        }
                        if !entry.contains(&member_segments[b].0) {
                            entry.push(member_segments[b].0);
                        }
                    }
                }
            }
        }
    }
    // Pick the corridor: most distinct nets; horizontal breaks count
    // ties; the lowest layer survives full ties (scan order).
    let mut corridor: Option<((i32, bool), usize)> = None;
    for (key, nets) in &votes {
        let better = match &corridor {
            None => true,
            Some((best_key, best_count)) => {
                nets.len() > *best_count || (nets.len() == *best_count && key.1 && !best_key.1)
            }
        };
        if better {
            corridor = Some((*key, nets.len()));
        }
    }
    let Some((corridor_key, _)) = corridor else {
        outcome.moves_rejected = member_reject_count(members.len());
        return outcome;
    };
    outcome.layer = corridor_key.0;
    outcome.horizontal = corridor_key.1;
    build_participants_and_sweep(manager, board, &member_segments, corridor_key, &mut outcome);
    outcome
}

/// The degenerate-ladder reject count (saturating).
fn member_reject_count(members: usize) -> u32 {
    u32::try_from(members).unwrap_or(u32::MAX)
}

/// The participants, the position ladder, and the relocation sweep
/// (the pass body after the corridor is chosen).
fn build_participants_and_sweep(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    member_segments: &[(i32, Vec<AxisSegment>)],
    corridor_key: (i32, bool),
    outcome: &mut BusGroupOutcome,
) {
    // Participants: voters in net order; each contributes its longest
    // qualifying-on-the-key segment (tie: trace id, then start).
    let mut participants: Vec<Participant> = Vec::new();
    for net_number in voter_nets(member_segments, corridor_key) {
        let own: Vec<AxisSegment> = member_segments
            .iter()
            .find(|(member, _)| *member == net_number)
            .map_or_else(Vec::new, |(_, segments)| segments.clone());
        let mut on_key: Vec<AxisSegment> = own
            .iter()
            .copied()
            .filter(|segment| {
                segment.layer == corridor_key.0
                    && segment.horizontal == corridor_key.1
                    && member_segments
                        .iter()
                        .filter(|(member, _)| *member != net_number)
                        .any(|(_, other)| {
                            other
                                .iter()
                                .any(|candidate| qualifying_overlap(segment, candidate).is_some())
                        })
            })
            .collect();
        on_key.sort_by_key(|segment| {
            (
                std::cmp::Reverse(segment.len()),
                segment.trace_id,
                segment.lo,
            )
        });
        let Some(span) = on_key.first().copied() else {
            continue;
        };
        participants.push(Participant {
            net: net_number,
            net_name: participant_name(board, net_number),
            trace_id: span.trace_id,
            span,
        });
    }
    if participants.len() < 2 {
        outcome.moves_rejected = member_reject_count(outcome.members.len());
        return;
    }
    // The position ladder (position ascending, net id breaking ties).
    participants.sort_by_key(|p| (p.span.perp, p.net));
    let positions: Vec<i64> = participants.iter().map(|p| p.span.perp).collect();
    let mut pitches: Vec<i64> = positions.windows(2).map(|w| w[1] - w[0]).collect();
    if pitches.iter().any(|pitch| *pitch <= 0) {
        outcome.moves_rejected = member_reject_count(outcome.members.len());
        return;
    }
    pitches.sort_unstable();
    let median_pitch = median_of(&pitches);
    let anchor_index = anchor_index_of(participants.len());
    let anchor_pos = positions[anchor_index];
    // The relocation sweep: participant NET order (an earlier landing
    // is visible to later probes — the deterministic acceptance order).
    let mut sweep: Vec<usize> = (0..participants.len()).collect();
    sweep.sort_by_key(|index| participants[*index].net);
    for index in sweep {
        let participant = &participants[index];
        let sorted_index = participants
            .iter()
            .position(|p| p.net == participant.net)
            .unwrap_or(0);
        let target = anchor_pos + (sorted_index as i64 - anchor_index as i64) * median_pitch;
        let delta = target - participant.span.perp;
        if delta == 0 {
            continue;
        }
        let mut landed: Option<i64> = None;
        for step in hug_candidates(delta) {
            if relocate_span(manager, board, participant, participant.span.perp, step) {
                landed = Some(participant.span.perp + step);
                break;
            }
        }
        if landed.is_some() {
            outcome.moves_landed += 1;
        } else {
            outcome.moves_rejected += 1;
        }
        outcome.rows.push(BusMoveRow {
            net: participant.net,
            net_name: participant.net_name.clone(),
            from_pos: participant.span.perp,
            target_pos: target,
            landed,
        });
    }
}

fn participant_name(board: &Board, net: i32) -> String {
    board
        .rules()
        .nets
        .get(net)
        .map_or_else(|| format!("net {net}"), |row| row.name.clone())
}

/// The corridor's voter nets (ASC).
fn voter_nets(member_segments: &[(i32, Vec<AxisSegment>)], corridor_key: (i32, bool)) -> Vec<i32> {
    let mut nets: Vec<i32> = Vec::new();
    for (a, sa_set) in member_segments {
        for (b, sb_set) in member_segments {
            if a >= b {
                continue;
            }
            if sa_set.iter().any(|sa| {
                sa.layer == corridor_key.0
                    && sa.horizontal == corridor_key.1
                    && sb_set.iter().any(|sb| qualifying_overlap(sa, sb).is_some())
            }) {
                if !nets.contains(a) {
                    nets.push(*a);
                }
                if !nets.contains(b) {
                    nets.push(*b);
                }
            }
        }
    }
    nets.sort_unstable();
    nets
}

/// The ANCHOR of the re-spacing ladder: index (n-1)/2 of the
/// position-sorted participants — the exact middle for odd n, the
/// LOWER middle for even n. Pinned for both parities (a `n/2` mutant
/// dies at n=4: 2 vs 1).
fn anchor_index_of(n: usize) -> usize {
    (n - 1) / 2
}

/// The median of an ascending-sorted pitch list: odd count — the
/// middle element; even — the round-average of the two middle
/// elements (f64 `.round()`, half away from zero, integer result).
fn median_of(sorted: &[i64]) -> i64 {
    if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        ((sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) as f64 / 2.0).round() as i64
    }
}

/// Builds and (on full acceptance) lands one relocation candidate: the
/// anchor trace's interior corners at `current_perp` shift by `step`
/// along the perpendicular axis. Returns true when the geometry was
/// written. Acceptance: the fixed-state gate, the length guard, and
/// the REAL clearance probe over every piece (whole-candidate
/// rejection — nothing lands partially).
fn relocate_span(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    participant: &Participant,
    current_perp: i64,
    step: i64,
) -> bool {
    let trace_id = participant.trace_id;
    // The fixed-state gate: USER_FIXED and SYSTEM_FIXED traces never
    // move (the router-set SHOVE_FIXED may — the ripup regime moves
    // it too).
    if board
        .get(trace_id)
        .is_some_and(|entry| matches!(entry.fixed, FixedState::UserFixed | FixedState::SystemFixed))
    {
        return false;
    }
    let Some(lines) = board.trace_polyline(trace_id) else {
        return false;
    };
    let Some(layer) = board.trace_layer(trace_id) else {
        return false;
    };
    let Some(half_width) = board.trace_half_width(trace_id) else {
        return false;
    };
    let Some(clearance_class) = board.item_clearance_class(trace_id) else {
        return false;
    };
    let corners = lines.corners();
    if corners.len() < 3 {
        return false; // no interior corners: no movable candidate
    }
    // Build the shifted corner list. The end corners never move; a
    // non-integer span corner cannot shift exactly — the candidate is
    // rejected (never a silent partial shift).
    let horizontal = participant.span.horizontal;
    let mut points: Vec<Point> = Vec::with_capacity(corners.len());
    for (index, corner) in corners.iter().enumerate() {
        let interior = index > 0 && index + 1 < corners.len();
        let shift = interior
            .then(|| as_int(corner))
            .flatten()
            .filter(|point| i64::from(if horizontal { point.y } else { point.x }) == current_perp);
        match shift {
            Some(point) => {
                let (x, y) = (i64::from(point.x), i64::from(point.y));
                let shifted = if horizontal {
                    i32::try_from(y + step).ok().map(|y2| (point.x, y2))
                } else {
                    i32::try_from(x + step).ok().map(|x2| (x2, point.y))
                };
                let Some((x, y)) = shifted else {
                    return false; // overflow: reject, never wrap
                };
                points.push(Point::Int(IntPoint::new(x, y)));
            }
            None => points.push(corner.clone()),
        }
    }
    let new_lines = Polyline::from_points(&points);
    if new_lines.lines.len() != lines.lines.len() {
        return false; // a degenerate construction is not a landing
    }
    // The length guard (hug must shorten or hold — the AM2 predicate;
    // equality at the gain bound allowed).
    let old_len = piece_length_sum(&lines.corners());
    let new_len = piece_length_sum(&new_lines.corners());
    if !length_guard_ok(old_len, new_len) {
        return false;
    }
    // The REAL clearance probe: every piece of the NEW polyline at
    // full length (exact corners; same-net items are exempt by the
    // tree query — the wave_is_clear precedent). One conflict rejects
    // the whole candidate.
    for pair in new_lines.corners().windows(2) {
        let (Some(a), Some(b)) = (as_int(&pair[0]), as_int(&pair[1])) else {
            return false;
        };
        let dx = i64::from(b.x) - i64::from(a.x);
        let dy = i64::from(b.y) - i64::from(a.y);
        let len = ((dx * dx + dy * dy) as f64).sqrt();
        if len <= 0.0 {
            continue;
        }
        let insertable = check_trace_segment_points(
            manager,
            board,
            &Point::Int(a),
            &Point::Int(b),
            layer,
            &[participant.net],
            half_width,
            clearance_class,
            false,
        );
        if insertable + 1e-6 < len {
            return false;
        }
    }
    board.replace_trace_geometry(manager, trace_id, new_lines);
    true
}

/// The piece-sum Euclidean length of a corner run (f64; axis-aligned
/// corners sum exactly — the length-guard boundary arithmetic).
fn piece_length_sum(corners: &[Point]) -> f64 {
    corners
        .windows(2)
        .map(|pair| {
            let (Some(a), Some(b)) = (as_int(&pair[0]), as_int(&pair[1])) else {
                return 0.0;
            };
            let dx = i64::from(b.x) - i64::from(a.x);
            let dy = i64::from(b.y) - i64::from(a.y);
            ((dx * dx + dy * dy) as f64).sqrt()
        })
        .sum()
}

/// The stage entry (the pipeline's post-tuning, pre-report slot; the
/// caller gates on `BatchSettings::bus_active` — the flag is the only
/// gate). Runs the group detector, then the pass per group (min net
/// id order). Emits NO events and NO stage transition (the
/// meander-stage precedent). An empty report = nothing grouped,
/// nothing moved.
pub fn run_gloss_bus_stage(manager: &mut SearchTreeManager, board: &mut Board) -> GlossBusReport {
    // The incompletes gate (the meander/pairs precedent): a net with
    // incompletes is not a gloss target (still being routed).
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    if incompletes.iter().any(|row| row.incomplete_count > 0) {
        return GlossBusReport::default();
    }
    let mut report = GlossBusReport::default();
    for members in detect_bus_groups(board) {
        let outcome = run_group_pass(manager, board, &members);
        report.groups.push(outcome);
    }
    report
}

/// The candidate LADDER face (the HUG approach), pinnable in
/// isolation: the full delta, then HALF the delta (i64 division,
/// truncating toward zero), deduplicated (a plus/minus 1 delta halves
/// to zero — no zero-step candidate).
#[must_use]
pub fn hug_candidates(delta: i64) -> Vec<i64> {
    let half = delta / 2;
    if half == 0 || half == delta {
        vec![delta]
    } else {
        vec![delta, half]
    }
}

/// The LENGTH-GUARD predicate, pinnable in isolation (the DNR-16
/// exact edge: old + GAIN lands, old + GAIN + 1 rejects —
/// both-direction mutations verified).
#[must_use]
pub fn length_guard_ok(old_len: f64, new_len: f64) -> bool {
    new_len <= old_len + BUS_MAX_LENGTH_GAIN_DBU as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_dsn::ses_board::{ItemIr, SesBoard};
    use epic_dsn::sink::{ClearanceIr, CreateBoardIr, FixedStateIr, NetClassIr, NetIr, TraceIr};
    use epic_geometry::int_box::IntBox;

    // -- the world builders (SesBoard-built, no routing; the
    //    pairs.rs prerouted-world pattern) --------------------------

    fn clearance_matrix_2layer(pair: i32) -> ClearanceIr {
        ClearanceIr {
            names: vec!["null".to_string(), "bussed".to_string()],
            values: vec![
                vec![vec![pair, pair], vec![pair, pair]],
                vec![vec![pair, pair], vec![pair, pair]],
            ],
        }
    }

    fn base_ses() -> SesBoard {
        use epic_dsn::coordinate_transform::CoordinateTransform;
        use epic_dsn::layer_structure::{Layer, LayerStructure};
        use epic_dsn::sink::BoardSink;

        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(600_000, 300_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("null".to_string()),
            rules: epic_dsn::sink::BoardRulesIr {
                clearance: clearance_matrix_2layer(2000),
                trace_angle_restriction: epic_dsn::state::AngleRestriction::NinetyDegree,
                default_trace_half_widths: vec![1000],
                min_trace_half_width: 1000,
                max_trace_half_width: 1000,
                pin_edge_to_turn_dist: 0.0,
                default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            },
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        for name in ["bus_a", "bus_b", "bus_c"] {
            ses.net_classes.push(NetClassIr {
                name: name.to_string(),
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
        }
        ses
    }

    fn register_nets(ses: &mut SesBoard, names: &[(&str, bool)]) {
        for (index, (name, plane)) in names.iter().enumerate() {
            ses.nets.push(NetIr {
                name: (*name).to_string(),
                subnet_number: 1,
                contains_plane: *plane,
                net_class: index as i32 + 1,
            });
        }
    }

    fn straight_trace(ses: &mut SesBoard, id: i32, net: i32, corners: Vec<IntPoint>) {
        ses.push_routed_item(ItemIr::Trace {
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
        });
    }

    fn world() -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[("bus_a", false), ("bus_b", false), ("bus_c", false)],
        );
        push_members(&mut ses);
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// The three Z members (the shared template of `world` and
    /// `blocked_world`). Bottom runs ascend with the span (a run at
    /// PY_k never crosses another net's vertical, whose y-range starts
    /// at PY >= PY_k); verticals and runs staggered >= 8_000 (mutual
    /// clearance needs 2*1000 + 2000 = 4_000).
    fn push_members(ses: &mut SesBoard) {
        let member = |ses: &mut SesBoard, id: i32, net: i32, py: i64, s: i64, xl: i64, xr: i64| {
            let p = |x: i64, y: i64| IntPoint::new(x as i32, y as i32);
            straight_trace(
                ses,
                id,
                net,
                vec![
                    p(0, py),
                    p(xl, py),
                    p(xl, s),
                    p(xr, s),
                    p(xr, py),
                    p(540_000, py),
                ],
            );
        };
        member(ses, 500, 1, 40_000, 100_000, 30_000, 510_000);
        member(ses, 501, 2, 50_000, 140_000, 20_000, 520_000);
        member(ses, 502, 3, 60_000, 185_000, 10_000, 530_000);
    }

    /// The net's on-board trace corner strings (sorted; the geometry
    /// equality face for the invariance/determinism pins).
    fn net_traces(board: &Board, net: i32) -> Vec<String> {
        let mut rows: Vec<String> = board
            .get_connectable_items(net)
            .into_iter()
            .filter(|&id| board.is_on_the_board(id))
            .filter_map(|id| board.trace_polyline(id).map(|lines| format!("{lines:?}")))
            .collect();
        rows.sort();
        rows
    }

    fn two_net_world(net2_corners: Vec<IntPoint>, plane_net2: bool) -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        register_nets(&mut ses, &[("bus_a", false), ("bus_b", plane_net2)]);
        straight_trace(
            &mut ses,
            500,
            1,
            vec![IntPoint::new(0, 100_000), IntPoint::new(300_000, 100_000)],
        );
        straight_trace(&mut ses, 501, 2, net2_corners);
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    fn run_stage(manager: &mut SearchTreeManager, board: &mut Board) -> GlossBusReport {
        run_gloss_bus_stage(manager, board)
    }

    /// The PASS WORLD plus the BLOCKER: a keepout band at y
    /// [179_000, 181_000] over x [200_000, 260_000] — between net 3's
    /// ladder candidates and its current span. Blocks the full step
    /// (probe band [179_500, 185_500] overlaps) and the half step
    /// (probe band [180_750, 186_750] overlaps), stays CLEAR of the
    /// current span's clearance zone ([182_000, 188_000]) and of every
    /// conductor (a keepout never counts in the REAL clearance
    /// counter).
    fn blocked_world() -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[("bus_a", false), ("bus_b", false), ("bus_c", false)],
        );
        push_members(&mut ses);
        ses.items.push(ItemIr::Keepout {
            id: 600,
            keepout: epic_dsn::sink::KeepoutIr {
                kind: epic_dsn::sink::KeepoutKindIr::Keepout,
                layer_no: 0,
                area: epic_dsn::sink::AreaIr::simple(epic_dsn::shape::BoardShape::Tile(
                    epic_geometry::tile_shape::TileShape::RegularTileShape(
                        epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(200_000, 179_000),
                            IntPoint::new(260_000, 181_000),
                        )),
                    ),
                )),
                clearance_class: 1,
                fixed: FixedStateIr::SystemFixed,
                component_id: 0,
                translation: IntPoint::new(0, 0),
                rotation: 0.0,
                side_changed: false,
                name: None,
            },
        });
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    // -- the DETECTION pins ----------------------------------------

    /// **D1 (the bus world, one group)** — the three Z members group
    /// transitively: (1,2) offset 40_000 and (2,3) offset 45_000 are
    /// within the window, spans overlap far beyond the threshold; the
    /// members ascend, one group.
    #[test]
    fn d1_bus_world_one_transitive_group() {
        let (manager, board) = world();
        let groups = detect_bus_groups(&board);
        assert_eq!(groups, vec![vec![1, 2, 3]], "one group, members ASC");
        let _ = manager;
    }

    /// **D2 (crossing is not parallel)** — a perpendicular crossing
    /// sits at distance zero but differs in orientation: no group.
    #[test]
    fn d2_crossing_is_not_parallel() {
        let (manager, board) = two_net_world(
            vec![
                IntPoint::new(150_000, 50_000),
                IntPoint::new(150_000, 150_000),
            ],
            false,
        );
        let groups = detect_bus_groups(&board);
        assert!(groups.is_empty(), "a crossing never groups: {groups:?}");
        let _ = manager;
    }

    /// **D3 (the span-threshold boundary, DNR-16)** — an overlap of
    /// EXACTLY [`BUS_SPAN_THRESHOLD_DBU`] does NOT group (strict `>`),
    /// one DBU more does. The pins read the CONSTANT (never a
    /// literal — the M-F shadowing lesson): a threshold mutant flips
    /// one of the two verdicts.
    #[test]
    fn d3_span_threshold_boundary_strict_both_directions() {
        // Overlap exactly TH: [100_000, 200_000] on the probe net
        // against the same span below — offset 20_000 (in-window).
        let (manager, board) = two_net_world(
            vec![
                IntPoint::new(100_000, 120_000),
                IntPoint::new(200_000, 120_000),
            ],
            false,
        );
        let groups = detect_bus_groups(&board);
        assert!(
            groups.is_empty(),
            "overlap == TH is NOT a group (strict >): {groups:?}"
        );
        drop((manager, board));
        // One DBU more: a group.
        let (_manager, board) = two_net_world(
            vec![
                IntPoint::new(100_000, 120_000),
                IntPoint::new(200_001, 120_000),
            ],
            false,
        );
        let groups = detect_bus_groups(&board);
        assert_eq!(groups, vec![vec![1, 2]], "overlap == TH + 1 groups");
    }

    /// **D4 (the window boundary, DNR-16)** — the perpendicular
    /// offset edge is INCLUSIVE (the tied window constant read at
    /// both pins): offset == window groups, offset == window + 1
    /// does not.
    #[test]
    fn d4_window_boundary_inclusive_plus_one_out() {
        // World geometry carries the LITERAL edge (50_000 / 50_001 —
        // the documented value of the tied window constant); the
        // verdicts read the constant. A window mutant of either sign
        // flips exactly one verdict (the bug-212 no-shadowing rule:
        // geometry must NOT derive from the constant under test).
        let (manager, board) = two_net_world(
            vec![IntPoint::new(0, 150_000), IntPoint::new(300_000, 150_000)],
            false,
        );
        let groups = detect_bus_groups(&board);
        assert_eq!(
            groups,
            vec![vec![1, 2]],
            "offset == 50_000 groups (inclusive)"
        );
        drop((manager, board));
        let (_manager, board) = two_net_world(
            vec![IntPoint::new(0, 150_001), IntPoint::new(300_000, 150_001)],
            false,
        );
        let groups = detect_bus_groups(&board);
        assert!(
            groups.is_empty(),
            "offset == 50_001 does not group: {groups:?}"
        );
    }

    /// **D5 (the plane-net exclusion)** — a contains_plane net never
    /// joins a group (the plane-routing mode is outside the gloss
    /// family's scope).
    #[test]
    fn d5_plane_net_excluded() {
        let (manager, board) = two_net_world(
            vec![IntPoint::new(0, 120_000), IntPoint::new(300_000, 120_000)],
            true,
        );
        let groups = detect_bus_groups(&board);
        assert!(groups.is_empty(), "a plane net never groups: {groups:?}");
        let _ = manager;
    }

    /// **D6 (the report faces on the pass world)** — the group row
    /// carries members, names, corridor (layer 0 horizontal), and
    /// starts clean.
    #[test]
    fn d6_report_faces_members_names_corridor() {
        let (mut manager, mut board) = world();
        let report = run_stage(&mut manager, &mut board);
        assert_eq!(report.groups.len(), 1);
        let group = &report.groups[0];
        assert_eq!(group.members, vec![1, 2, 3]);
        assert_eq!(
            group.member_names,
            vec![
                "bus_a".to_string(),
                "bus_b".to_string(),
                "bus_c".to_string()
            ]
        );
        assert_eq!(group.layer, 0);
        assert!(group.horizontal);
    }

    // -- the PASS pins ---------------------------------------------

    /// **P1 (the crafted bus world, EXACT expected geometry)** — the
    /// ragged ladder 100_000 / 140_000 / 185_000 (pitches 40_000,
    /// 45_000; median 42_500 round-averaged) re-spaces around the
    /// anchor (net 2, the lower-middle at index 1): targets
    /// 97_500 / 140_000 / 182_500. Both movers shorten (the AM2 hug
    /// direction), land on the REAL probe, and the board stays
    /// DRC-clean on the REAL counter.
    #[test]
    fn p1_pass_world_exact_geometry() {
        let (mut manager, mut board) = world();
        let report = run_stage(&mut manager, &mut board);
        assert_eq!(report.groups.len(), 1);
        let group = &report.groups[0];
        assert_eq!(group.moves_landed, 2);
        assert_eq!(group.moves_rejected, 0);
        assert_eq!(group.rows.len(), 2);
        let row1 = &group.rows[0];
        assert_eq!(row1.net, 1);
        assert_eq!(row1.from_pos, 100_000);
        assert_eq!(row1.target_pos, 97_500);
        assert_eq!(row1.landed, Some(97_500));
        let row3 = &group.rows[1];
        assert_eq!(row3.net, 3);
        assert_eq!(row3.from_pos, 185_000);
        assert_eq!(row3.target_pos, 182_500);
        assert_eq!(row3.landed, Some(182_500));
        // The EXACT final geometry.
        let p = |x: i32, y: i32| Point::Int(IntPoint::new(x, y));
        assert_eq!(
            net_traces(&board, 1),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(0, 40_000),
                    p(30_000, 40_000),
                    p(30_000, 97_500),
                    p(510_000, 97_500),
                    p(510_000, 40_000),
                    p(540_000, 40_000),
                ])
            )]
        );
        assert_eq!(
            net_traces(&board, 2),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(0, 50_000),
                    p(20_000, 50_000),
                    p(20_000, 140_000),
                    p(520_000, 140_000),
                    p(520_000, 50_000),
                    p(540_000, 50_000),
                ])
            )]
        );
        assert_eq!(
            net_traces(&board, 3),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(0, 60_000),
                    p(10_000, 60_000),
                    p(10_000, 182_500),
                    p(530_000, 182_500),
                    p(530_000, 60_000),
                    p(540_000, 60_000),
                ])
            )]
        );
        let (violations, _) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "the re-spaced bus is DRC-clean");
    }

    /// **P2 (the blocked world, the honest-stop face)** — a keepout
    /// band blocks BOTH ladder steps of net 3's relocation (the full
    /// step to 182_500 and the half step to 183_750): no move lands,
    /// the row records `landed: None`, and net 1's move still lands
    /// (the pass is not all-or-nothing). The keepout sits BETWEEN the
    /// candidates and the current position, clear of every CURRENT
    /// trace's clearance zone.
    #[test]
    fn p2_blocked_world_no_move_honest_row() {
        let (mut manager, mut board) = blocked_world();
        let report = run_stage(&mut manager, &mut board);
        assert_eq!(report.groups.len(), 1);
        let group = &report.groups[0];
        assert_eq!(group.members, vec![1, 2, 3]);
        assert_eq!(group.moves_landed, 1, "net 1 still lands");
        assert_eq!(group.moves_rejected, 1, "net 3 is the honest stop");
        let row3 = group
            .rows
            .iter()
            .find(|row| row.net == 3)
            .expect("net 3 has a row");
        assert_eq!(row3.from_pos, 185_000);
        assert_eq!(row3.target_pos, 182_500);
        assert_eq!(row3.landed, None, "both steps blocked: no move");
        // Net 3's geometry is untouched.
        assert!(net_traces(&board, 3)[0].contains("185000"));
    }

    /// **P3 (the isolation world, the criterion-2 invariance pin, the
    /// pairs.rs P3 pattern)** — a fourth net routed AWAY from the bus
    /// (no qualifying span with anyone) keeps its GEOMETRY identical
    /// with the pass ON vs a no-op run, and appears in NO group.
    #[test]
    fn p3_non_group_net_identical_on_vs_off() {
        let run = || -> (Vec<String>, Vec<Vec<i32>>) {
            let (mut manager, mut board) = world_with_lone_net();
            let groups = detect_bus_groups(&board);
            let _report = run_stage(&mut manager, &mut board);
            (net_traces(&board, 4), groups)
        };
        let (lone_on, groups) = run();
        assert_eq!(groups, vec![vec![1, 2, 3]], "the lone net is in NO group");
        let input = vec![format!(
            "{:?}",
            Polyline::from_points(&[
                Point::Int(IntPoint::new(0, 20_000)),
                Point::Int(IntPoint::new(120_000, 20_000)),
            ])
        )];
        assert_eq!(lone_on, input, "the lone net's geometry is untouched");
    }

    /// The pass world plus a fourth net routed far below the bus: no
    /// qualifying span with anyone (window-parallel to net 1's bottom
    /// run at 20_000 offset, but the x-overlap 30_000 sits far below
    /// the span threshold — recorded, not grouped).
    fn world_with_lone_net() -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[
                ("bus_a", false),
                ("bus_b", false),
                ("bus_c", false),
                ("lone", false),
            ],
        );
        push_members(&mut ses);
        straight_trace(
            &mut ses,
            503,
            4,
            vec![IntPoint::new(0, 20_000), IntPoint::new(120_000, 20_000)],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// **P4 (determinism x2)** — two fresh worlds through the stage:
    /// the reports and every net's final geometry agree exactly.
    #[test]
    fn p4_determinism_two_fresh_runs() {
        let run = || -> (GlossBusReport, Vec<String>) {
            let (mut manager, mut board) = world();
            let report = run_stage(&mut manager, &mut board);
            let geom: Vec<String> = (1..=3).flat_map(|net| net_traces(&board, net)).collect();
            (report, geom)
        };
        let (report_a, geom_a) = run();
        let (report_b, geom_b) = run();
        assert_eq!(report_a, report_b);
        assert_eq!(geom_a, geom_b);
    }

    /// **P5 (the ladder pin)** — the HUG candidates: full delta then
    /// half (i64 truncation toward zero), deduplicated at the ±1
    /// boundary (a ±1 delta has NO half step — one candidate only).
    #[test]
    fn p5_hug_ladder_faces() {
        assert_eq!(hug_candidates(-30_000), vec![-30_000, -15_000]);
        assert_eq!(hug_candidates(30_001), vec![30_001, 15_000]);
        assert_eq!(hug_candidates(-1), vec![-1]);
        assert_eq!(hug_candidates(1), vec![1]);
        assert_eq!(hug_candidates(-2), vec![-2, -1]);
    }

    /// **P6 (the length-guard boundary, DNR-16 exact edge)** — old +
    /// GAIN lands (equality allowed), old + GAIN + 1 rejects. The
    /// constants are read at both pins — a GAIN mutant flips one of
    /// the two verdicts.
    #[test]
    fn p6_length_guard_exact_edge_both_directions() {
        // Literal lengths (old 540_000; the gain bound is 40_000):
        // 580_000 lands (equality), 580_001 rejects. No reference to
        // the constant in the arithmetic — a GAIN mutant of either
        // sign flips exactly one verdict (bug-212 rule).
        let old_len = 540_000.0;
        assert!(length_guard_ok(old_len, 580_000.0));
        assert!(!length_guard_ok(old_len, 580_001.0));
    }

    /// **P8 (the anchor face)** — the ladder anchor is (n-1)/2: the
    /// exact middle for odd n, the LOWER middle for even n. A `n/2`
    /// mutant dies at n=4 (2 vs 1).
    #[test]
    fn p8_anchor_face() {
        assert_eq!(anchor_index_of(2), 0);
        assert_eq!(anchor_index_of(3), 1);
        assert_eq!(anchor_index_of(4), 1);
        assert_eq!(anchor_index_of(5), 2);
    }

    /// **P7 (the median face)** — odd count: the middle element; even:
    /// the round-average (the .5 half-away face pinned exactly).
    #[test]
    fn p7_median_faces() {
        assert_eq!(median_of(&[42_500]), 42_500);
        assert_eq!(median_of(&[40_000, 45_000]), 42_500);
        assert_eq!(median_of(&[40_000, 45_001]), 42_501, "the .5 rounds away");
        assert_eq!(median_of(&[10_000, 20_000, 90_000]), 20_000);
    }

    /// **P9 (the fixed-state gate)** — a USER_FIXED member trace does
    /// not move (recorded rejected), while unfixed members do.
    #[test]
    fn p9_fixed_member_does_not_move() {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[("bus_a", false), ("bus_b", false), ("bus_c", false)],
        );
        push_members(&mut ses);
        // Promote net 1's trace to USER_FIXED in place.
        ses.items = ses
            .items
            .into_iter()
            .map(|item| match item {
                ItemIr::Trace { id, trace } if id == 500 => ItemIr::Trace {
                    id,
                    trace: TraceIr {
                        fixed: FixedStateIr::UserFixed,
                        ..trace
                    },
                },
                other => other,
            })
            .collect();
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_stage(&mut manager, &mut board);
        let group = &report.groups[0];
        let row1 = group.rows.iter().find(|row| row.net == 1);
        assert!(
            row1.is_none() || row1.expect("row").landed.is_none(),
            "the fixed member never lands"
        );
        assert!(net_traces(&board, 1)[0].contains("100000"));
    }
}
