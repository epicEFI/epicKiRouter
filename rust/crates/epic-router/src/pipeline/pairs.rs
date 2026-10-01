//! M7-T6: the differential-PAIR face — the pair DECLARATION resolution,
//! the post-routing MATCH stage (the shorter member meandered up to
//! the longer, anchored on the longer member), the advisory coupled-
//! corridor measure, and the maze COUPLING preference the follower's
//! routing consults.
//!
//! **Declaration face (the fromto audit outcome).** Java's `(fromto`
//! consumption is a SUBNET-DIVISION face: `Network.readNetScope`
//! (`Network.java:1381-1386`) reads each `(fromto` scope as one
//! subnet's pin list and splits the net into independently routed
//! subnets (`Network.java:1401-1462`) — the parse face is already
//! ported Java-exact (`epic-dsn/src/scope/network.rs:708`). Every
//! fromto scope's pins belong to ONE net, so fromto CANNOT declare a
//! cross-net pair; there is NO fromto→pair mapping (the conservative
//! mapping rule is: none exists — any would be an invented semantics
//! that would perturb the parity face of fromto-bearing boards). The
//! sole pair declaration face is the explicit settings list
//! `router.tuning.pairs` (net-name pairs — the KiCad-export reality:
//! KiCad DSN export declares no pairs; committed corpus: zero fromto
//! scopes, grep-verified at T6; a name resolving to several subnet
//! nets answers UNRESOLVED — recorded, never guessed).
//!
//! **The pair delta (the T6-OWNED constant; AMENDMENT 5 §6).** With
//! the T5 class match's goal=min, a both-declared pair could legally
//! end at delta = the class window — too loose for a pair. The pair
//! face charters its OWN tighter delta, [`PAIR_DELTA_DBU`], deliberately
//! NOT inherited from the class window and NOT
//! [`crate::pipeline::tuning::MATCH_TOLERANCE_DBU`]: the LONGER member
//! is the anchor (the `target_net` pattern), the SHORTER member is
//! meandered up toward it through the T4 insertion engine, and the
//! report's `matched` verdict is `delta <= PAIR_DELTA_DBU` (equality
//! allowed — the T4 equality-at-max precedent).
//!
//! **Two-regime safety.** The face's activation input is the resolved
//! declaration list ALONE (the third activation input; it does NOT
//! require the `router.tuning` regime): an empty list ⇒ zero
//! activation ⇒ byte-identical routing. Structurally: every consumer —
//! the pass leader-first order, the maze cost term, the match stage —
//! gates on the resolved list being non-empty (the `meander_active`
//! pattern, Default empty), and the maze cost term additionally
//! requires the follower's per-attempt preference to be `Some`.
//!
//! **Dubins A\* disposition (design :73, DECLINED on measurement +
//! realizability at T6; the MSDTW precedent shape).** The design
//! charters the pose-based A\* with Dubins heuristic as the pair
//! corridor's QUALITY mechanism; the plan's honest either-or let the
//! simpler coupled-maze face go first. Measured on the committed pair
//! population, the simple face already meets the contract: the
//! pair_SPLIT fixture's advisory row reads `coupled_length` 381 482.0
//! DBU of shared corridor, delta 10 270.24 ≤ [`PAIR_DELTA_DBU`], and
//! the pair_COUPLIED row reads `coupled_length` 0.0 — no
//! within-window parallel span at all, the advisory honestly reporting
//! the decoupled face while the members still match by meander, delta
//! 15 147.19 ≤ [`PAIR_DELTA_DBU`] — both matched, REAL-DRC 0. No
//! quality gap for a better corridor search to close. (ERRATUM
//! APPLIED, design `:637-642`: the pre-fix text attributed the 381 482
//! / 10 270 row to the coupled fixture and split world — the two rows
//! were label-swapped; the committed goldens were always right.) The
//! realizability premise fails independently: the maze
//! emits ANGLE-RESTRICTED polylines (45°/90°; the
//! `expand_to_door`/section face in `maze/search_engine.rs`) with no
//! arc primitive, and a Dubins path is G1-continuous arc geometry —
//! the heuristic's output cannot be landed by this engine. Re-charter
//! trigger: a geometry milestone that adds continuous-curvature
//! (arc-bearing) route output inherits this note and may re-face the
//! pair corridor search on it.

use std::collections::BTreeMap;

use epic_board::board::Board;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;

use crate::pipeline::batch::BatchSettings;
use crate::pipeline::event_sink::DriverSink;
use crate::pipeline::tuning;

/// The M7-T6 pair DELTA, in board DBU (the T6-OWNED constant,
/// AMENDMENT 5 §6): a declared pair is MATCHED when its two members'
/// routed lengths end within this distance. Deliberately TIGHTER than
/// the T5 class-match tolerance ([`tuning::MATCH_TOLERANCE_DBU`],
/// 40_000 DBU) and never inherited from a class min–max window. Value:
/// 20_000 DBU — exactly ONE coarsest-dent wave granularity (the T4
/// amplitude ladder's coarsest amplitude is 10_000 DBU; the added
/// length per dent is 2·A), so any successful wave landing puts the
/// remaining delta below this bound for every integer deficit (delta
/// = added − deficit < 2·A when any single candidate covers the
/// deficit; multi-wave top-ups land finer). The bound BINDS on the
/// honest-stop face (no candidate landed — the deficit stands) and on
/// the report verdict — equality allowed (the T4 equality-at-max
/// precedent).
pub const PAIR_DELTA_DBU: f64 = 20_000.0;

/// The COUPLING WINDOW, in board DBU: a follower maze step counts as
/// IN-corridor when its section middle point lies within this distance
/// of the leader's routed real copper (same layer). The window is a
/// preference radius, never a clamp — geometry outside it routes
/// normally (the split-world face).
///
/// The T1 window-constant TIE (the T1 quality-review bank, discharged
/// by AM2 to this task): this is the DERIVED face — the SINGLE
/// definition of the number lives in
/// [`epic_board::aesthetics::AESTHETICS_PARALLELISM_WINDOW_DBU`]
/// (epic-router may import from epic-board; the reverse may not, so
/// the definition sits in the lower crate), and both sites cite it.
/// Value unchanged: 50_000 board DBU.
pub const COUPLING_WINDOW_DBU: f64 =
    epic_board::aesthetics::AESTHETICS_PARALLELISM_WINDOW_DBU as f64;

/// The COUPLING DISCOUNT: the fraction of the maze's incremental
/// weighted-distance step cost waived while in-corridor. A discount —
/// not a subtracted bonus — keeps every step cost non-negative and the
/// best-first order well-formed; the `None`-coupling path never
/// touches the term (the cost face is bit-identical).
pub const COUPLING_DISCOUNT: f64 = 0.5;

/// One RESOLVED pair declaration (net numbers; the leader is the LOWER
/// net number — the deterministic lead rule: the lower id routes
/// first, the follower's routing carries the coupling preference).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PairSpec {
    /// The leader net number (the lower of the two).
    pub leader: i32,
    /// The follower net number (the higher of the two).
    pub follower: i32,
}

/// One leader-corridor segment (axis-aligned, board DBU): the maze
/// coupling preference's geometry source (the leader's on-board
/// traces, extracted fresh per follower routing attempt).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorridorSegment {
    pub layer: i32,
    /// The segment endpoints (x, y), board DBU.
    pub ax: i64,
    pub ay: i64,
    pub bx: i64,
    pub by: i64,
}

impl CorridorSegment {
    /// The segment axis: `true` = horizontal (`ay == by`).
    #[must_use]
    pub fn is_horizontal(&self) -> bool {
        self.ay == self.by
    }
}

/// The follower's COUPLING preference: the leader's routed corridors
/// plus the window/discount parameters. Rides
/// [`crate::control::AutorouteControl::coupling`] (RUST-ONLY, `None` at
/// default — the OFF path never consults it, byte-identical).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CouplingPreference {
    pub segments: Vec<CorridorSegment>,
    pub window: f64,
    pub discount: f64,
}

impl CouplingPreference {
    /// The distance from a point to the nearest same-layer corridor
    /// segment (Euclid; `f64::MAX` when the layer carries no segment).
    #[must_use]
    pub fn distance_to(&self, x: f64, y: f64, layer: i32) -> f64 {
        let mut best = f64::MAX;
        for segment in &self.segments {
            if segment.layer != layer {
                continue;
            }
            let (ax, ay) = (segment.ax as f64, segment.ay as f64);
            let (bx, by) = (segment.bx as f64, segment.by as f64);
            let dx = bx - ax;
            let dy = by - ay;
            let len_sq = dx * dx + dy * dy;
            let t = if len_sq > 0.0 {
                (((x - ax) * dx) + ((y - ay) * dy)) / len_sq
            } else {
                0.0
            };
            let t = t.clamp(0.0, 1.0);
            let px = ax + (t * dx);
            let py = ay + (t * dy);
            let ex = x - px;
            let ey = y - py;
            let dist = (ex * ex + ey * ey).sqrt();
            if dist < best {
                best = dist;
            }
        }
        best
    }

    /// The in-corridor predicate: the point lies within the coupling
    /// window of the leader's copper on this layer (the INCLUSIVE
    /// edge is in — the DNR-16 boundary faces pin window ± 1).
    #[must_use]
    pub fn in_corridor(&self, x: f64, y: f64, layer: i32) -> bool {
        self.distance_to(x, y, layer) <= self.window
    }
}

/// Extracts the follower's coupling preference: `Some` iff `net` is
/// the FOLLOWER of a resolved pair whose LEADER has on-board routed
/// traces (the leader's geometry is the corridor source). An unrouted
/// leader answers `None` — inert (the preference never fabricates
/// geometry). MULTI-FOLLOWER edge: a leader with SEVERAL declared
/// followers couples each of them INDEPENDENTLY (the first matching
/// pair spec supplies the corridor; there is no leader-side cap and
/// no follower-to-follower effect — the discount is per-follower).
#[must_use]
pub fn coupling_preference(
    board: &Board,
    settings: &BatchSettings,
    net: i32,
) -> Option<CouplingPreference> {
    if settings.pairs.is_empty() {
        return None;
    }
    let leader = settings
        .pairs
        .iter()
        .find(|pair| pair.follower == net)
        .map(|pair| pair.leader)?;
    let mut segments = Vec::new();
    collect_corridor_segments(board, leader, &mut segments);
    if segments.is_empty() {
        return None;
    }
    Some(CouplingPreference {
        segments,
        window: COUPLING_WINDOW_DBU,
        discount: COUPLING_DISCOUNT,
    })
}

/// The net's axis-aligned trace segments (all layers, on-board traces
/// only; deterministic order: trace id ASC, corner ASC).
fn collect_corridor_segments(board: &Board, net: i32, out: &mut Vec<CorridorSegment>) {
    let mut trace_ids: Vec<_> = board
        .get_connectable_items(net)
        .into_iter()
        .filter(|&id| board.is_on_the_board(id) && board.trace_polyline(id).is_some())
        .collect();
    trace_ids.sort();
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
            if (dx != 0 && dy != 0) || (dx == 0 && dy == 0) {
                continue; // the corridor model is axis-aligned only
            }
            out.push(CorridorSegment {
                layer,
                ax: i64::from(a.x),
                ay: i64::from(a.y),
                bx: i64::from(b.x),
                by: i64::from(b.y),
            });
        }
    }
}

fn as_int(point: &Point) -> Option<IntPoint> {
    match point {
        Point::Int(int_point) => Some(*int_point),
        Point::Rational(_) => None,
    }
}

/// The shared-corridor measure: the total length of the FOLLOWER's
/// axis-aligned segments that run PARALLEL to (same layer, same axis)
/// and WITHIN `window` perpendicular offset of a leader segment, with
/// a positive axis overlap — per follower segment the MAX overlap
/// counts (overlapping several leader segments counts once). Board
/// DBU. The advisory report's coupling face and the split world's
/// honest decoupled-segment record.
#[must_use]
pub fn coupled_length(board: &Board, leader: i32, follower: i32, window: f64) -> f64 {
    let mut leader_segments = Vec::new();
    collect_corridor_segments(board, leader, &mut leader_segments);
    if leader_segments.is_empty() {
        return 0.0;
    }
    let mut follower_segments = Vec::new();
    collect_corridor_segments(board, follower, &mut follower_segments);
    let mut total = 0.0f64;
    for seg in &follower_segments {
        let mut best = 0.0f64;
        for leader_seg in &leader_segments {
            if leader_seg.layer != seg.layer || leader_seg.is_horizontal() != seg.is_horizontal() {
                continue;
            }
            let (offset, seg_lo, seg_hi, other_lo, other_hi) = if seg.is_horizontal() {
                (
                    (seg.ay - leader_seg.ay).abs(),
                    seg.ax.min(seg.bx),
                    seg.ax.max(seg.bx),
                    leader_seg.ax.min(leader_seg.bx),
                    leader_seg.ax.max(leader_seg.bx),
                )
            } else {
                (
                    (seg.ax - leader_seg.ax).abs(),
                    seg.ay.min(seg.by),
                    seg.ay.max(seg.by),
                    leader_seg.ay.min(leader_seg.by),
                    leader_seg.ay.max(leader_seg.by),
                )
            };
            if (offset as f64) > window {
                continue;
            }
            let overlap = seg_hi.min(other_hi) - seg_lo.max(other_lo);
            if overlap > 0 {
                let overlap_f = overlap as f64;
                if overlap_f > best {
                    best = overlap_f;
                }
            }
        }
        total += best;
    }
    total
}

/// One pair's MATCH-stage outcome (the engine-side evidence rows; the
/// manifest's advisory rows are computed AFTER the pipeline from the
/// final board — the M6-T6 island-detector pattern).
#[derive(Clone, Debug, PartialEq)]
pub struct PairStageOutcome {
    /// The leader net number (the lower of the pair).
    pub leader: i32,
    /// The follower net number (the higher of the pair).
    pub follower: i32,
    /// Lengths at stage entry (board DBU).
    pub leader_length: f64,
    pub follower_length: f64,
    /// The anchor: the LONGER member's net number (the `target_net`
    /// pattern; a tie answers the FOLLOWER — no work either way:
    /// deficit 0).
    pub anchor_net: i32,
    /// Whether a match wave landed on the shorter member.
    pub landed: bool,
    /// The total added length across the member's insertions.
    pub added_length: f64,
    /// The total dent count across the member's insertions.
    pub dent_count: i64,
}

/// The post-routing MATCH stage: per resolved pair, the SHORTER member
/// is meandered up toward the LONGER member's current length (the
/// anchor; never shortened — the T3 honoring contract holds for the
/// pair face too). The meandered member's own class `max` still
/// bounds every candidate (the T4 never-exceed-max guard — the pair
/// goal never overrides a declared max), and a member with
/// incompletes is not a meander target (the T4 gate). Ordering:
/// resolved pairs in (leader, follower) ASC. The stage runs AFTER the
/// class meander/match stage (the class faces own the class goals
/// first; the pair face tops the pair delta up).
pub fn run_pair_stage(
    manager: &mut epic_board::tree_manager::SearchTreeManager,
    board: &mut Board,
    pairs: &[PairSpec],
    sink: &mut dyn DriverSink,
) -> Vec<PairStageOutcome> {
    let mut outcomes = Vec::new();
    // The incompletes gate (the T4 face): a member still being routed
    // is not a meander target.
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    for pair in pairs {
        let leader_length = board.net_trace_length(pair.leader);
        let follower_length = board.net_trace_length(pair.follower);
        // The anchor: the LONGER member; the SHORTER member rises to
        // the anchor's length (strict `>` — a tie answers the
        // follower and has no work anyway: deficit 0).
        let (anchor_net, anchor_length, meander_net_no, deficit) =
            if leader_length > follower_length {
                (
                    pair.leader,
                    leader_length,
                    pair.follower,
                    leader_length - follower_length,
                )
            } else {
                (
                    pair.follower,
                    follower_length,
                    pair.leader,
                    follower_length - leader_length,
                )
            };
        let mut outcome = PairStageOutcome {
            leader: pair.leader,
            follower: pair.follower,
            leader_length,
            follower_length,
            anchor_net,
            landed: false,
            added_length: 0.0,
            dent_count: 0,
        };
        let meander_blocked = incompletes
            .iter()
            .any(|row| row.net_no == meander_net_no && row.incomplete_count > 0);
        if deficit > 0.0 && !meander_blocked {
            let max = {
                let rules = board.rules();
                rules.net_class_length_bounds(meander_net_no).1
            };
            let name = board
                .rules()
                .nets
                .get(meander_net_no)
                .map_or_else(|| format!("net#{meander_net_no}"), |net| net.name.clone());
            // The meandered member drives toward the anchor's length
            // (goal = anchor length); the T4 engine's loop owns the
            // honest stop and the never-exceed-max guard.
            let result = tuning::meander_net(
                manager,
                board,
                meander_net_no,
                &name,
                deficit,
                anchor_length,
                max,
                sink,
            );
            outcome.landed = result.landed;
            outcome.added_length = result.added_length;
            outcome.dent_count = result.dent_count;
        }
        outcomes.push(outcome);
    }
    outcomes
}

/// Resolves the DECLARATION list (net-name pairs) against the board's
/// netlist: exact case-sensitive name → number; a name resolving to
/// MULTIPLE nets (a fromto subnet split) or to NONE leaves the pair
/// unresolved (recorded honestly, never guessed); a resolved pair
/// orders (leader, follower) = (min, max) of the two numbers. A
/// declared SELF-pair (`NET_A:NET_A` — the same net on both sides) is
/// silently DROPPED here: never resolved, never a report row (a
/// one-net "pair" has no delta to match — the honest face is the
/// absence, not a row). Returns the resolved specs in (leader,
/// follower) ASC order (deduplicated)
/// plus the unresolved rows in declaration order (the advisory face).
#[must_use]
pub fn resolve_pairs(
    board: &Board,
    declarations: &[(String, String)],
) -> (Vec<PairSpec>, Vec<(String, String)>) {
    let mut by_name: BTreeMap<&str, Vec<i32>> = BTreeMap::new();
    let rules = board.rules();
    for (net_number, net) in rules.nets.iter() {
        by_name
            .entry(net.name.as_str())
            .or_default()
            .push(net_number);
    }
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for (name_a, name_b) in declarations {
        let nets_a = by_name.get(name_a.as_str());
        let nets_b = by_name.get(name_b.as_str());
        match (nets_a, nets_b) {
            (Some(a), Some(b)) if a.len() == 1 && b.len() == 1 => {
                let (leader, follower) = if a[0] < b[0] {
                    (a[0], b[0])
                } else {
                    (b[0], a[0])
                };
                if leader != follower {
                    resolved.push(PairSpec { leader, follower });
                }
            }
            _ => unresolved.push((name_a.clone(), name_b.clone())),
        }
    }
    resolved.sort_unstable();
    resolved.dedup();
    (resolved, unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::pipeline::batch::BatchSettings;
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::pipeline::full::{self, PipelineOutcome};
    use crate::pipeline::optimizer::OptimizerSettingsIr;
    use crate::test_util::parse;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_drc::clearance::all_clearance_violation_depths;

    /// A routable pair world: net 1 (`pa`, the LEADER by the net-number
    /// rule) and net 2 (`pb`, the follower) run left-to-right, the
    /// follower's far pin offset UPWARD (y 24000 -> y 30000) — the
    /// shortest path is a single-bend L whose bend position is a cost
    /// TIE (bend costs 0; the vertical span is fixed either way). The
    /// coupling discount is the tie's only bias toward staying in the
    /// leader's corridor (P1's load-bearing face; the discount mutant
    /// flips it). `sol` (net 3) is UNDECLARED and lives in a separate
    /// region — the P3 invariance face.
    const PAIR_WORLD: &str = r#"
(pcb pair_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 60000 40000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "CA1" (place "PA1" 2000 20000 Front 0.000000))
    (component "CA2" (place "PA2" 48000 20000 Front 0.000000))
    (component "CB1" (place "PB1" 2000 24000 Front 0.000000))
    (component "CB2" (place "PB2" 48000 30000 Front 0.000000))
    (component "CS1" (place "PS1" 2000 38000 Front 0.000000))
    (component "CS2" (place "PS2" 48000 38000 Front 0.000000))
  )
  (library
    (image "CA1" (pin "PAD" "P" 0 0))
    (image "CA2" (pin "PAD" "P" 0 0))
    (image "CB1" (pin "PAD" "P" 0 0))
    (image "CB2" (pin "PAD" "P" 0 0))
    (image "CS1" (pin "PAD" "P" 0 0))
    (image "CS2" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
    (padstack "VIA_PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
  )
  (network
    (via VT VIA_PAD kicad_default)
    (net "pa" (pins "PA1"-"P" "PA2"-"P"))
    (net "pb" (pins "PB1"-"P" "PB2"-"P"))
    (net "sol" (pins "PS1"-"P" "PS2"-"P"))
    (class kicad_default "pa" "pb" "sol" (rule (clearance 200)))
  )
)
"#;

    /// The P2 SPLIT world: the pair world plus an F.Cu keepout wall,
    /// x ∈ [20000, 22000], y from 20400 up to 36000 (all DSN um;
    /// internal DBU are ×10). The wall bottom sits 4000 internal DBU
    /// above the leader's y = 20000 center line — a gap the follower
    /// (hw 1000 + clearance 2016 to the leader, 3016 to the wall)
    /// cannot route through — so across the wall's x-span the follower
    /// must leave its y = 24000 pin line (the engine's route squeezes
    /// below, between the leader and the wall). The decoupled segment
    /// is recorded honestly in the coupled-length measure and the
    /// lengths still match.
    const SPLIT_WORLD: &str = r#"
(pcb split_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 60000 60000))
    (keepout ""
      (polygon F.Cu 0  20000 20400  22000 20400  22000 36000  20000 36000  20000 20400)
    )
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "CA1" (place "PA1" 2000 20000 Front 0.000000))
    (component "CA2" (place "PA2" 48000 20000 Front 0.000000))
    (component "CB1" (place "PB1" 2000 24000 Front 0.000000))
    (component "CB2" (place "PB2" 48000 24000 Front 0.000000))
  )
  (library
    (image "CA1" (pin "PAD" "P" 0 0))
    (image "CA2" (pin "PAD" "P" 0 0))
    (image "CB1" (pin "PAD" "P" 0 0))
    (image "CB2" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
    (padstack "VIA_PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
  )
  (network
    (via VT VIA_PAD kicad_default)
    (net "pa" (pins "PA1"-"P" "PA2"-"P"))
    (net "pb" (pins "PB1"-"P" "PB2"-"P"))
    (class kicad_default "pa" "pb" (rule (clearance 200)))
  )
)
"#;

    fn settings_ir() -> RouterSettingsIr {
        RouterSettingsIr {
            trace_costs: vec![
                crate::control::ExpansionCostFactor {
                    horizontal: 1.0,
                    vertical: 2.7,
                },
                crate::control::ExpansionCostFactor {
                    horizontal: 1.6,
                    vertical: 1.0,
                },
            ],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![0.0, 0.0],
            layer_active: vec![true, true],
            automatic_neckdown: false,
            start_ripup_costs: 1,
            fanout: Default::default(),
        }
    }

    fn opt_settings() -> OptimizerSettingsIr {
        OptimizerSettingsIr {
            algorithm: "freerouting-optimizer".to_string(),
            max_passes: Some(1),
            max_items: None,
            improvement_threshold: Some(2.5),
            enable_preflight_guards: Some(false),
            max_consecutive_failures: Some(50),
            max_consecutive_failures_pass1: Some(12),
            additional_ripup_cost_factor_at_start: 10,
            trace_ripup_cost_factor: 0.6,
            max_autoroute_passes: 6,
            timeout_string: None,
        }
    }

    fn run_pipeline(
        manager: &mut SearchTreeManager,
        board: &mut epic_board::board::Board,
        pairs: Vec<PairSpec>,
    ) -> PipelineOutcome {
        run_pipeline_sink(manager, board, pairs).0
    }

    fn run_pipeline_sink(
        manager: &mut SearchTreeManager,
        board: &mut epic_board::board::Board,
        pairs: Vec<PairSpec>,
    ) -> (PipelineOutcome, String) {
        let mut settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        settings.fanout_enabled = false;
        settings.pairs = pairs;
        let mut sink = CaptureDriverSink::default();
        let outcome = full::run(
            manager,
            board,
            settings,
            opt_settings(),
            false,
            crate::pipeline::batch::StopFace::default(),
            &mut sink,
        );
        let info = sink.joined("info");
        (outcome, info)
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

    /// **P1** — the pair world routes coupled: the follower shares the
    /// leader's corridor (the parallel-within-window measure over the
    /// world's span), the lengths match within the pair-delta
    /// constant, and the lead order is deterministic (the lower net
    /// number leads — the coupling preference rides the follower's
    /// routing only). DRC-clean on the REAL counter.
    #[test]
    fn p1_pair_world_routes_coupled_and_matched() {
        let (mut manager, mut board) = parse(PAIR_WORLD);
        let outcome = run_pipeline(
            &mut manager,
            &mut board,
            vec![PairSpec {
                leader: 1,
                follower: 2,
            }],
        );
        assert!(
            matches!(&outcome.routing, Ok(true)),
            "the pair world routes to completion: {:?}",
            outcome.routing
        );
        let (violations, _) = all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "a coupled pair route is DRC-clean");
        let leader_length = board.net_trace_length(1);
        let follower_length = board.net_trace_length(2);
        let delta = (leader_length - follower_length).abs();
        assert!(
            delta <= PAIR_DELTA_DBU,
            "the pair delta {delta} must sit within the pair-delta constant (stage: {:?})",
            outcome.pair_stage
        );
        let coupled = coupled_length(&board, 1, 2, COUPLING_WINDOW_DBU);
        assert!(
            coupled >= 200_000.0,
            "the follower shares the corridor: coupled length {coupled} over the open span"
        );
        // The match stage ran for the pair and its anchor is the longer
        // member.
        assert_eq!(outcome.pair_stage.len(), 1, "one pair, one stage row");
        assert_eq!(outcome.pair_stage[0].leader, 1);
        assert_eq!(outcome.pair_stage[0].follower, 2);
    }

    /// **P1 (the load-bearing face)** — the coupling discount is what
    /// holds the follower in the corridor: with the declared pair, the
    /// follower's route stays within the coupling window of the leader
    /// for the coupled span; the same board with NO declaration routes
    /// the follower OUT of the corridor (the cost tie resolves
    /// differently) — the in-corridor span differs. (The discount
    /// mutant dies on this pin's coupled-span assert.)
    #[test]
    fn p1b_coupling_holds_the_follower_in_the_corridor() {
        let run = |pairs: Vec<PairSpec>| -> (f64, f64) {
            let (mut manager, mut board) = parse(PAIR_WORLD);
            let _outcome = run_pipeline(&mut manager, &mut board, pairs);
            (
                coupled_length(&board, 1, 2, COUPLING_WINDOW_DBU),
                board.net_trace_length(2),
            )
        };
        let (coupled_on, _) = run(vec![PairSpec {
            leader: 1,
            follower: 2,
        }]);
        let (coupled_off, _) = run(Vec::new());
        assert!(
            coupled_on > coupled_off,
            "the declared pair's follower stays in the corridor: ON {coupled_on} > OFF {coupled_off}"
        );
    }

    /// **P2** — the split world: the keepout wall forces the follower
    /// out of the corridor; the decoupled face is recorded honestly
    /// (the coupled measure is SMALLER than the split-free world's)
    /// and the lengths still match within the pair delta.
    #[test]
    fn p2_split_world_records_decoupled_and_still_matches() {
        let (mut manager, mut board) = parse(SPLIT_WORLD);
        let (outcome, sink) = run_pipeline_sink(
            &mut manager,
            &mut board,
            vec![PairSpec {
                leader: 1,
                follower: 2,
            }],
        );
        assert!(
            matches!(&outcome.routing, Ok(true)),
            "the split world routes to completion: {:?}\nsink:\n{sink}",
            outcome.routing
        );
        let (violations, _) = all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "a split pair route is DRC-clean");
        let leader_length = board.net_trace_length(1);
        let follower_length = board.net_trace_length(2);
        let delta = (leader_length - follower_length).abs();
        assert!(
            delta <= PAIR_DELTA_DBU,
            "the split pair still matches: delta {delta} within the constant"
        );
        let coupled = coupled_length(&board, 1, 2, COUPLING_WINDOW_DBU);
        // The bound is world-derived, not magic: the follower's un-split
        // in-corridor face is its straight pin-to-pin span
        // (48000 - 2000 DSN x 10 = 460000 internal DBU), and across the
        // wall's own x-span ([200000, 220000] internal = 20000 DBU) the
        // follower is OFF the corridor (climbing over the wall /
        // squeezing below it), so the coupled overlap cannot cover it.
        // Bound = 460000 - 20000; a wall-free straight world measures
        // the full 460000 and dies on this assert (the split/coupled
        // discriminator). The measured value sits well under the bound
        // (the squeeze's vertical work is uncounted by the measure).
        const P2_STRAIGHT_SPAN_DBU: f64 = 460_000.0;
        const P2_WALL_SPAN_DBU: f64 = 20_000.0;
        assert!(
            coupled <= P2_STRAIGHT_SPAN_DBU - P2_WALL_SPAN_DBU,
            "the wall decouples a segment: coupled {coupled} must sit at or below \
             the straight face {P2_STRAIGHT_SPAN_DBU} minus the wall span {P2_WALL_SPAN_DBU}"
        );
        assert!(coupled > 0.0, "the un-split spans still share the corridor");
    }

    /// **P3** — the invariance face: the UNDECLARED net's routes are
    /// GEOMETRY-identical with the pair face ON vs OFF on the same
    /// board world. Geometry-level is the correct contract, deliberately
    /// NOT byte-level: the SES carries no item ids, and wire order
    /// follows the internal insert-order ids — which the pair
    /// leader-first reorder legitimately shifts — so byte equality
    /// would pin incidental ordering, not routing behavior (the spec
    /// review's Q2 answer, adopted).
    #[test]
    fn p3_undeclared_net_geometry_identical_on_vs_off() {
        let run_sol = |pairs: Vec<PairSpec>| -> Vec<String> {
            let (mut manager, mut board) = parse(PAIR_WORLD);
            let _outcome = run_pipeline(&mut manager, &mut board, pairs);
            net_traces(&board, 3)
        };
        let sol_on = run_sol(vec![PairSpec {
            leader: 1,
            follower: 2,
        }]);
        assert!(!sol_on.is_empty(), "the undeclared net routed");
        let sol_off = run_sol(Vec::new());
        assert_eq!(
            sol_on, sol_off,
            "the undeclared net routes identically with the pair face ON vs OFF"
        );
    }

    /// **P4** — determinism ×2: fresh worlds; the pair stage report
    /// and the final geometry (every net) agree byte-for-byte.
    #[test]
    fn p4_determinism_two_fresh_runs() {
        let run = || -> (Vec<PairStageOutcome>, String) {
            let (mut manager, mut board) = parse(PAIR_WORLD);
            let outcome = run_pipeline(
                &mut manager,
                &mut board,
                vec![PairSpec {
                    leader: 1,
                    follower: 2,
                }],
            );
            (
                outcome.pair_stage.clone(),
                format!(
                    "{:?}|{:?}|{:?}",
                    net_traces(&board, 1),
                    net_traces(&board, 2),
                    net_traces(&board, 3)
                ),
            )
        };
        let (stage_a, geom_a) = run();
        let (stage_b, geom_b) = run();
        assert_eq!(stage_a, stage_b, "the pair stage reports agree");
        assert_eq!(geom_a, geom_b, "the final geometry agrees");
    }

    /// **P6 (the declaration faces)** — the fromto face is a NEGATIVE
    /// resolution (the audit: fromto splits ONE net into subnets, all
    /// sharing the name — the conservative resolver answers AMBIGUOUS,
    /// recorded, never guessed) and the explicit list resolves
    /// (leader/follower ordered by net number).
    #[test]
    fn p6_declaration_faces_fromto_negative_explicit_positive() {
        let (manager, board) = parse(FROMTO_WORLD);
        let _ = manager;
        // The fromto world's net 4 ("ft") parsed into TWO subnet nets
        // sharing the name (the Java-parity split) — the ambiguity
        // face; `pa`/`pb` are unique.
        let declarations = vec![
            ("ft".to_string(), "pa".to_string()),
            ("pa".to_string(), "pb".to_string()),
        ];
        let (resolved, unresolved) = resolve_pairs(&board, &declarations);
        assert_eq!(unresolved.len(), 1, "the ambiguous name stays unresolved");
        assert_eq!(unresolved[0].0, "ft");
        assert_eq!(resolved.len(), 1, "the explicit pair resolves");
        assert_eq!(resolved[0].leader, 1, "the lower net number leads");
        assert_eq!(resolved[0].follower, 2);
    }

    const FROMTO_WORLD: &str = r#"
(pcb fromto_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 500000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "CA1" (place "PA1" 20000 60000 Front 0.000000))
    (component "CA2" (place "PA2" 480000 60000 Front 0.000000))
    (component "CB1" (place "PB1" 20000 120000 Front 0.000000))
    (component "CB2" (place "PB2" 480000 120000 Front 0.000000))
    (component "CF1" (place "PF1" 20000 180000 Front 0.000000))
    (component "CF2" (place "PF2" 240000 180000 Front 0.000000))
  )
  (library
    (image "CA1" (pin "PAD" "P" 0 0))
    (image "CA2" (pin "PAD" "P" 0 0))
    (image "CB1" (pin "PAD" "P" 0 0))
    (image "CB2" (pin "PAD" "P" 0 0))
    (image "CF1" (pin "PAD" "P" 0 0))
    (image "CF2" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
      (attach off)
    )
    (padstack "VIA_PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
  )
  (network
    (via VT VIA_PAD kicad_default)
    (net "pa" (pins "PA1"-"P" "PA2"-"P"))
    (net "pb" (pins "PB1"-"P" "PB2"-"P"))
    (net "ft" (fromto "PF1"-"P" "PF2"-"P") (fromto "PF2"-"P" "PF1"-"P"))
    (class kicad_default "pa" "pb" "ft" (rule (clearance 200)))
  )
)
"#;

    /// **P6 (the advisory delta arithmetic, world-derived)** — a
    /// pre-routed pair board: the advisory row's delta is exactly the
    /// two members' routed-length difference and the matched verdict
    /// follows [`PAIR_DELTA_DBU`] (equality allowed).
    #[test]
    fn p6_advisory_delta_arithmetic_world_derived() {
        let world = prerouted_pair_world(200_000.0, 160_000.0);
        let (mut manager, mut board) = world;
        let pairs = vec![PairSpec {
            leader: 1,
            follower: 2,
        }];
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_pair_stage(&mut manager, &mut board, &pairs, &mut sink);
        assert_eq!(outcomes.len(), 1);
        let leader_length = board.net_trace_length(1);
        let follower_length = board.net_trace_length(2);
        // The stage landed: the shorter member rose to the anchor's
        // length plus at most one wave granularity.
        assert!(outcomes[0].landed);
        let delta = (leader_length - follower_length).abs();
        assert!(
            delta <= PAIR_DELTA_DBU,
            "the matched delta {delta} sits within the constant"
        );
        // World-derived arithmetic: leader 200000, follower 160000 +
        // the coarse wave's 20000 added.
        assert_eq!(leader_length, 200_000.0);
        assert_eq!(
            follower_length, 200_000.0,
            "the shorter member rose to the anchor"
        );
        assert_eq!(
            outcomes[0].added_length, 40_000.0,
            "deficit 40000 = two coarse dents"
        );
        assert_eq!(outcomes[0].dent_count, 2);
        assert_eq!(outcomes[0].anchor_net, 1, "the longer member anchors");
        // The advisory measure: the straight members share the corridor
        // over the follower's whole span.
        let coupled = coupled_length(&board, 1, 2, COUPLING_WINDOW_DBU);
        assert!(coupled > 0.0, "the parallel members measure coupled");
    }

    /// **P5 (the pair-delta boundary, DNR-16 both directions)** — the
    /// closed-form boundary is the constant ITSELF on the honest-stop
    /// face: a blocked world whose deficit is EXACTLY
    /// [`PAIR_DELTA_DBU`] ends matched (equality allowed); a blocked
    /// world one unit past ends UNmatched. The ±1 mutants each kill
    /// exactly one world (the mutation log owns the runs).
    #[test]
    fn p5_pair_delta_boundary_equality_and_one_past() {
        // Deficit EXACTLY PAIR_DELTA_DBU (20000), every wave candidate
        // blocked: delta stays 20000 → matched (equality).
        let (mut manager, mut board) = prerouted_blocked_pair_world(20_000.0);
        let pairs = vec![PairSpec {
            leader: 1,
            follower: 2,
        }];
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_pair_stage(&mut manager, &mut board, &pairs, &mut sink);
        assert!(!outcomes[0].landed, "the blocker stops every candidate");
        let delta = (board.net_trace_length(1) - board.net_trace_length(2)).abs();
        assert_eq!(delta, PAIR_DELTA_DBU, "the deficit stands at the boundary");
        assert!(
            delta <= PAIR_DELTA_DBU,
            "equality AT the pair-delta constant is matched"
        );
        // One unit PAST the boundary: unmatched.
        let (mut manager, mut board) = prerouted_blocked_pair_world(20_001.0);
        let mut sink = CaptureDriverSink::default();
        let outcomes = run_pair_stage(&mut manager, &mut board, &pairs, &mut sink);
        assert!(!outcomes[0].landed);
        let delta = (board.net_trace_length(1) - board.net_trace_length(2)).abs();
        assert_eq!(delta, 20_001.0);
        assert!(
            delta > PAIR_DELTA_DBU,
            "one unit past the constant is unmatched"
        );
    }

    // -- the pre-routed world builders (the WaveWorld pattern of
    //    tuning.rs's tests; SesBoard-built, no routing) --------------

    /// Clearance matrix cell value helper: a uniform matrix on both
    /// layers unless overridden (copied from the tuning.rs test face).
    fn clearance_matrix_2layer(pair: i32) -> epic_dsn::sink::ClearanceIr {
        epic_dsn::sink::ClearanceIr {
            names: vec!["null".to_string(), "paired".to_string()],
            values: vec![
                vec![vec![pair, pair], vec![pair, pair]],
                vec![vec![pair, pair], vec![pair, pair]],
            ],
        }
    }

    fn base_ses() -> epic_dsn::ses_board::SesBoard {
        use epic_dsn::coordinate_transform::CoordinateTransform;
        use epic_dsn::layer_structure::{Layer, LayerStructure};
        use epic_dsn::ses_board::SesBoard;
        use epic_dsn::sink::BoardSink;
        use epic_dsn::sink::{CreateBoardIr, NetClassIr};
        use epic_geometry::int_box::IntBox;
        use epic_geometry::int_point::IntPoint;

        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(500_000, 200_000)),
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
        for name in ["lead", "follow"] {
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

    /// A pre-routed pair: the leader straight at y=120000 (length
    /// `leader_len`), the follower straight at y=0 (length
    /// `follower_len`) — both class-free (headroom unbounded), no
    /// blocker.
    fn prerouted_pair_world(leader_len: f64, follower_len: f64) -> (SearchTreeManager, Board) {
        use epic_dsn::ses_board::ItemIr;
        use epic_dsn::sink::{FixedStateIr, NetIr, TraceIr};
        use epic_geometry::int_point::IntPoint;

        let mut ses = base_ses();
        ses.nets = vec![
            NetIr {
                name: "lead".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
            NetIr {
                name: "follow".to_string(),
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
            vec![
                IntPoint::new(0, 80_000),
                IntPoint::new(leader_len as i32, 80_000),
            ],
        ));
        ses.push_routed_item(trace(
            501,
            2,
            vec![
                IntPoint::new(0, 40_000),
                IntPoint::new(follower_len as i32, 40_000),
            ],
        ));
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// The BLOCKED pair world (the tuning.rs WaveWorld blocker face):
    /// the follower runs at y=0 (length `500000 - deficit`), the
    /// leader at y=120000 (length 500000), and a foreign wall at
    /// y=7000 overlaps EVERY wave candidate of the follower (teeth top
    /// out at y = A + hw >= 6000 for every legal amplitude) — the
    /// honest stop, the deficit stands.
    fn prerouted_blocked_pair_world(deficit: f64) -> (SearchTreeManager, Board) {
        use epic_dsn::ses_board::ItemIr;
        use epic_dsn::sink::{FixedStateIr, NetIr, TraceIr};
        use epic_geometry::int_point::IntPoint;

        let mut ses = base_ses();
        ses.nets = vec![
            NetIr {
                name: "lead".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
            NetIr {
                name: "follow".to_string(),
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
        let follower_len = 500_000.0 - deficit;
        ses.push_routed_item(trace(
            500,
            1,
            vec![IntPoint::new(0, 120_000), IntPoint::new(500_000, 120_000)],
        ));
        ses.push_routed_item(trace(
            501,
            2,
            vec![IntPoint::new(0, 0), IntPoint::new(follower_len as i32, 0)],
        ));
        // The foreign blocker: net-less geometry is not allowed — the
        // blocker is the LEADER's second trace? No: a foreign net's
        // wall (net 3 unregistered would break the netlist) — the
        // blocker rides a third registered net.
        ses.nets.push(NetIr {
            name: "wall".to_string(),
            subnet_number: 1,
            contains_plane: false,
            net_class: 2,
        });
        ses.push_routed_item(trace(
            502,
            3,
            vec![IntPoint::new(-10_000, 7_000), IntPoint::new(510_000, 7_000)],
        ));
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// The corridor-window boundary (DNR-16): the INCLUSIVE edge is
    /// in-corridor, one unit out is not — on the unit-level
    /// [`CouplingPreference`] predicate.
    #[test]
    fn coupling_window_boundary_inclusive_plus_one_out() {
        // The window rides the CONSTANT (not a literal): a ±1 drift of
        // [`COUPLING_WINDOW_DBU`] must flip the verdicts below (the
        // M-F/M-G mutants of the mutation log die here).
        let pref = CouplingPreference {
            segments: vec![CorridorSegment {
                layer: 0,
                ax: 0,
                ay: 0,
                bx: 400_000,
                by: 0,
            }],
            window: COUPLING_WINDOW_DBU,
            discount: COUPLING_DISCOUNT,
        };
        assert!(
            pref.in_corridor(200_000.0, 50_000.0, 0),
            "at the window edge: IN"
        );
        assert!(
            !pref.in_corridor(200_000.0, 50_001.0, 0),
            "one unit past the window edge: OUT"
        );
        assert!(
            pref.in_corridor(200_000.0, -50_000.0, 0),
            "the negative side edge is IN too"
        );
        assert!(
            !pref.in_corridor(200_000.0, 50_000.0, 1),
            "other layer: no corridor"
        );
    }
}
