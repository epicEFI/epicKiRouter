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
//! **M7-T5: the MATCH contract (match groups).** The stage's work list
//! is grouped by NET CLASS — a MATCH GROUP is the nets of one
//! constrained class (the flat net→class table; T2's no-inheritance
//! audit). Per group a TARGET and a TOLERANCE are resolved:
//!
//! - **Both bounds declared** (`min > 0 && max > 0`): target = `max`,
//!   tolerance = `max − min`, and the per-net GOAL is `min` exactly
//!   (the window arithmetic `max − (max − min)` collapses) — the T4
//!   face bit for bit: deficit toward min, headroom `max − current`,
//!   the equality landing AT max allowed, so the declared-window
//!   worlds behave exactly as before.
//! - **Min-only** (`min > 0`, `max == 0`): target = the LONGEST routed
//!   length among the group's members, ties broken deterministically
//!   by net number ASC (the walk order — the first of a tie wins);
//!   tolerance = [`MATCH_TOLERANCE_DBU`]. The per-net GOAL is
//!   `max(min, target − tolerance)` — the floor keeps the honoring
//!   contract (no net is driven to less than its own min).
//!
//! Every net ends within the group tolerance of the target, or the
//! honest-stop face records the deficit; a net already AT or ABOVE its
//! goal is untouched (never shortened — the T3 honoring contract
//! holds; above-`max` nets are the report-only face, never truncated).
//! A max-only class (`min == 0`) forms no group: the work list is the
//! under-min population, so nothing is actionable there — the
//! over-max rows belong to the report face alone.
//!
//! **MSDTW median-trace disposition (design :73, DECLINED on
//! measurement at T5).** The technique keys a corridor-sharing group's
//! meanders off the group's median trace. Measured on the committed
//! population it would face: all four tuning fixtures route each
//! tuned net on its OWN corridor (separate pin pairs, no shared
//! corridor between members of any class), and the pin worlds are the
//! same shape — a median trace does not exist for them (single-member
//! groups) or coincides with each member's own straight run (the
//! multi fixture's members are parallel, non-adjacent runs). With no
//! population case where a median differs from the per-net
//! accommodation, the technique's benefit set is EMPTY here; the
//! per-net deterministic accommodation is strictly simpler and
//! already pins-verified. Declined on measurement (the honest-stop
//! discipline), not dropped: a future milestone that charts
//! shared-corridor groups inherits this note.
//!
//! **MSDTW disposition at T6 (reopened per AMENDMENT 5 §5, declined
//! again on measurement).** The T6 pair face DID create a
//! shared-corridor population (the declared pairs). Measured on it:
//! a pair is a TWO-member population — the median of two traces is
//! not defined as a central tendency distinct from the members
//! themselves, and the pair contract anchors the match on the LONGER
//! member (the `target_net` pattern of [`MatchGroupReport
//! ::target_net`], carried by [`crate::pipeline::pairs
//! ::PairStageOutcome::anchor_net`]) — a per-net deterministic anchor,
//! not a median. On every pair world the median-trace accommodation
//! either coincides with the per-net one (the anchor IS the longer
//! member) or is undefined (two-member groups); the benefit set
//! remains EMPTY. Declined on measurement again, with the pair face
//! as the recorded reopening candidate: a future charter with
//! ≥3-member shared corridors (bus groups) is the population where a
//! median could differ.
//!
//! **M7-T6: the PAIR delta.** The pair face charters its own tighter
//! delta ([`crate::pipeline::pairs::PAIR_DELTA_DBU`], 20_000 DBU —
//! deliberately tighter than [`MATCH_TOLERANCE_DBU`], never inherited
//! from a class window; AMENDMENT 5 §6) and drives this module's
//! insertion engine directly ([`meander_net`], the per-net loop, with
//! goal = the anchor member's length).

use std::collections::BTreeMap;

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

/// The M7-T5 group MATCH tolerance, in board DBU: when a constrained
/// class declares NO max (a min-only class), the group's target is the
/// longest routed member length and every member must end within this
/// distance of that target. NAMED CONSTANT by design (the T2/T4
/// family): the class's own min–max window owns the tolerance when
/// both bounds are declared; this constant owns it otherwise. Value:
/// 40_000 DBU — four coarsest-dent lengths (the amplitude ladder's
/// granularity is 2·A per dent with A up to 10_000), comfortably above
/// one wave granularity so the ladder's own step size alone cannot
/// strand a landed net outside tolerance.
pub const MATCH_TOLERANCE_DBU: f64 = 40_000.0;

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

/// One MATCH GROUP's report row (the stage's group face, in
/// class-index ASC order). `target_net` is `Some` only on the
/// min-only target derivation (the routed-length target has a
/// DEFINING net; the declared-max target does not).
#[derive(Clone, Debug, PartialEq)]
pub struct MatchGroupReport {
    /// The class index (the flat net→class table's key).
    pub class_index: i32,
    /// The class name.
    pub class_name: String,
    /// The group's member net numbers (net-number ASC — the walk
    /// order).
    pub members: Vec<i32>,
    /// The group target length (board DBU): the declared `max`, or the
    /// longest routed member length on a min-only class.
    pub target: f64,
    /// The group tolerance (board DBU): `max − min` when both bounds
    /// are declared, else [`MATCH_TOLERANCE_DBU`].
    pub tolerance: f64,
    /// The net that DEFINES the target (`None` when the target is the
    /// declared `max`).
    pub target_net: Option<i32>,
}

/// The meander stage's full report: the per-net outcomes (the T4 face,
/// in group-then-member order — one group per constrained class,
/// members in net-number order) plus the T5 match-group rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeanderStageReport {
    /// One row per processed under-goal net (the T4 outcome face).
    pub outcomes: Vec<MeanderNetOutcome>,
    /// One row per match group (class-index ASC).
    pub match_groups: Vec<MatchGroupReport>,
}

/// One group's resolved contract: the per-net GOAL length plus the
/// report row. The T5 closure shape (`goal_of: Box<dyn Fn>`) is
/// COLLAPSED (the T6 bank discharge, AMENDMENT 5 §7a): both arms were
/// current-independent, and the T6 pair face drives the per-net loop
/// directly with a plain goal too ([`crate::pipeline::pairs
/// ::run_pair_stage`] → [`meander_net`]) — no per-net goal consumer
/// exists, so the closure is dead weight.
struct GroupContract {
    goal: f64,
    report: MatchGroupReport,
}

/// Resolves one match group's contract (the target/tolerance/goal
/// semantics; the module docs own the derivation).
fn resolve_group_contract(
    board: &Board,
    class_index: i32,
    class_name: String,
    members: &[(i32, String, f64, f64)],
) -> GroupContract {
    let (_, _, min, max) = members[0];
    if max > 0.0 {
        // Both bounds declared: the window owns the tolerance and the
        // per-net goal is exactly `min` — the T4 arithmetic, bit for
        // bit (deficit toward min; headroom = max − current; the
        // equality landing AT max allowed).
        GroupContract {
            goal: min,
            report: MatchGroupReport {
                class_index,
                class_name,
                members: members.iter().map(|(net, _, _, _)| *net).collect(),
                target: max,
                tolerance: max - min,
                target_net: None,
            },
        }
    } else {
        // Min-only: the target is the longest routed member length,
        // ties broken by net number ASC (the walk order — the FIRST of
        // a tie wins, so `>` is strict: an equal later member never
        // displaces the incumbent).
        let mut target_net = members[0].0;
        let mut target = board.net_trace_length(target_net);
        for (net, _, _, _) in &members[1..] {
            let length = board.net_trace_length(*net);
            if length > target {
                target = length;
                target_net = *net;
            }
        }
        let goal_floor = (target - MATCH_TOLERANCE_DBU).max(min);
        GroupContract {
            goal: goal_floor,
            report: MatchGroupReport {
                class_index,
                class_name,
                members: members.iter().map(|(net, _, _, _)| *net).collect(),
                target,
                tolerance: MATCH_TOLERANCE_DBU,
                target_net: Some(target_net),
            },
        }
    }
}

/// The meander stage (the pipeline's post-optimization face). INERT
/// unless `board.tuning_active()` (the caller resolves the flag; the
/// stage re-checks so direct callers cannot bypass the regime gate).
/// Returns the stage report: one [`MeanderNetOutcome`] per processed
/// under-goal net and one [`MatchGroupReport`] per constrained class.
pub fn run_meander_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    sink: &mut dyn DriverSink,
) -> MeanderStageReport {
    if !board.tuning_active() {
        return MeanderStageReport::default();
    }
    // Snapshot the work list first (the rules borrow must not span the
    // mutable landing work). Number order — `Nets::iter`.
    let targets: Vec<(i32, String, f64, f64, i32)> = {
        let rules = board.rules();
        rules
            .nets
            .iter()
            .filter_map(|(net_number, net)| {
                let (min, max) = rules.net_class_length_bounds(net_number);
                (min > 0.0).then(|| {
                    // The T6 bank discharge (AMENDMENT 5 §7c): the old
                    // `rules.nets.get(net_number).map_or(0, …)` fallback
                    // was DEAD — every walked net answers `get`, so the
                    // class index reads off the iteration face directly.
                    let class_index = net.net_class;
                    (net_number, net.name.clone(), min, max, class_index)
                })
            })
            .collect()
    };
    // The MATCH GROUPS: one per constrained class (class-index ASC —
    // the BTreeMap order), members in net-number order (the walk
    // order). A member tuple: (net number, name, min, max).
    let mut groups: BTreeMap<i32, Vec<(i32, String, f64, f64)>> = BTreeMap::new();
    for (net_number, net_name, min, max, class_index) in targets {
        groups
            .entry(class_index)
            .or_default()
            .push((net_number, net_name, min, max));
    }
    // Java's `calcLengthViolation` incompletes gate: a net with
    // incompletes is not a meander target (still being routed).
    let incompletes = all_incompletes(manager, board).1;
    let mut report = MeanderStageReport::default();
    for (class_index, members) in groups {
        let class_name = {
            let rules = board.rules();
            rules.net_class(class_index).map_or_else(
                || format!("class {class_index}"),
                |class| class.name.clone(),
            )
        };
        let contract = resolve_group_contract(board, class_index, class_name, &members);
        report.match_groups.push(contract.report);
        for (net_number, net_name, _min, max) in &members {
            let current = board.net_trace_length(*net_number);
            let goal = contract.goal;
            if current >= goal {
                // At or above the GOAL: nothing to add (and above-max
                // is the report-only face — never truncated, never
                // touched).
                continue;
            }
            if incompletes
                .iter()
                .any(|row| row.net_no == *net_number && row.incomplete_count > 0)
            {
                continue;
            }
            let deficit = goal - current;
            let outcome = meander_net(
                manager,
                board,
                *net_number,
                net_name,
                deficit,
                goal,
                *max,
                sink,
            );
            report.outcomes.push(outcome);
        }
    }
    report
}

/// The per-net insertion loop (the budget face:
/// [`MAX_MEANDER_INSERTIONS_PER_NET`]). `pub(crate)` for the T6 pair
/// stage ([`crate::pipeline::pairs::run_pair_stage`]), which drives
/// the same loop with the pair anchor as the goal.
#[allow(clippy::too_many_arguments)] // the stage's per-net frame, kept flat
pub(crate) fn meander_net(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net_number: i32,
    net_name: &str,
    deficit: f64,
    goal: f64,
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
        if current >= goal {
            break;
        }
        let remaining = goal - current;
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
            if !land_wave(manager, board, site, &wave) {
                continue; // a phantom landing is not a landing: next candidate
            }
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
/// duplicate is dropped. Returns TRUE when the geometry was actually
/// written: the two early-return arms are defensive (an absent
/// trace; a collapsed Polyline construction) and unreachable on the
/// ladder's own candidates — but a silent no-op there would let the
/// caller report a PHANTOM landing (added length credited, no
/// geometry), so the outcome is observable and the caller continues
/// its ladder on `false`.
fn land_wave(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    site: &MeanderSite,
    wave: &Wave,
) -> bool {
    let Some(lines) = board.trace_polyline(site.trace_id) else {
        return false;
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
        return false; // a collapsed construction is not a landing
    }
    board.replace_trace_geometry(manager, site.trace_id, new_lines);
    true
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
    fn run_stage(world: &mut WaveWorld) -> MeanderStageReport {
        let mut sink = CaptureDriverSink::default();
        let report = run_meander_stage(&mut world.manager, &mut world.board, &mut sink);
        let (violations, _) = all_clearance_violation_depths(&mut world.manager, &mut world.board);
        assert_eq!(
            violations, 0,
            "the REAL DRC counter: a meander landing must be clearance-clean"
        );
        report
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
        let outcomes = run_stage(&mut world).outcomes;
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
        let outcomes = run_stage(&mut world).outcomes;
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
    /// floor = 2·hw = 2000) on the second: every table entry would
    /// pass the filter and the 7985 world's teeth would land with an
    /// edge-to-edge gap of 10000 − 2·1000 = 8000 against the expected
    /// 7985 + 16 = 8001 — a 1-unit shortfall the stage's real-DRC-clean
    /// assertion catches (run_stage asserts zero violations on every
    /// landing world).
    #[test]
    fn wave_self_clear_boundary_both_directions() {
        // Equality: floor 10000, first table entry 10000 -> lands.
        let mut world = wave_world(460_000.0, 0.0, 7984, false);
        let outcomes = run_stage(&mut world).outcomes;
        assert!(outcomes[0].landed, "floor == table amplitude is allowed");
        // One unit above: floor 10001, no table entry passes -> stop.
        let mut world = wave_world(460_000.0, 0.0, 7985, false);
        let outcomes = run_stage(&mut world).outcomes;
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
        let outcomes = run_stage(&mut world).outcomes;
        assert!(outcomes[0].landed);
        assert_eq!(
            world.board.net_trace_length(1),
            460_000.0,
            "AT max, not past it"
        );
        // Headroom one DBU short: every amplitude overshoots max.
        let mut world = wave_world(460_000.0, 459_999.0, 2000, false);
        let outcomes = run_stage(&mut world).outcomes;
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
        let outcomes = run_stage(&mut world).outcomes;
        assert!(outcomes[0].landed);
        assert_eq!(outcomes[0].dent_count, 1, "deficit == one dent");
        assert_eq!(world.board.net_trace_length(1), 440_000.0, "exactly min");
        let mut world = wave_world(440_001.0, 0.0, 2000, false);
        let outcomes = run_stage(&mut world).outcomes;
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
        let outcomes = run_stage(&mut world).outcomes;
        assert!(outcomes.is_empty(), "at-min: no row, no touch");
        // Above-min is expressed via min < current — same skip arm.
        let mut world = wave_world(100_000.0, 500_000.0, 2000, false);
        let outcomes = run_stage(&mut world).outcomes;
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
        let outcomes = run_meander_stage(&mut world.manager, &mut world.board, &mut sink).outcomes;
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
        let outcomes = run_meander_stage(&mut world.manager, &mut world.board, &mut sink).outcomes;
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

    // ------------------------------------------------------------------
    // M7-T5: the MATCH pins (the group face; the wave worlds above own
    // the per-net arithmetic — these own the target/tolerance/group
    // semantics)
    // ------------------------------------------------------------------

    /// The match world: `tuned.len()` tuned-class nets (class 1, the
    /// ONE constrained class) each carrying a straight trace of the
    /// requested length at its own row (row i at y = i·40000 — the
    /// spacing clears every legal tooth: neighbor teeth top out at
    /// y + 11000 < 40000 − clearance), plus the OPTIONAL free net
    /// (class 2) and the OPTIONAL row-0 blocker (net free+1, the W2
    /// wall shape at y = 7000). All values world-derived; the
    /// clearance matrix / widths are the W-world's.
    fn match_world(
        min: f64,
        max: f64,
        tuned: &[f64],
        free: Option<f64>,
        blocker: bool,
    ) -> WaveWorld {
        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(600_000, 300_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("null".to_string()),
            rules: BoardRulesIr {
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
        let mut nets = Vec::new();
        for i in 0..tuned.len() {
            nets.push(NetIr {
                name: format!("m{}", i + 1),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            });
        }
        if free.is_some() {
            nets.push(NetIr {
                name: "free".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 2,
            });
        }
        if blocker {
            nets.push(NetIr {
                name: "wall".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 2,
            });
        }
        ses.nets = nets;
        let trace = |id: i32, net: i32, y: i32, length: i32| ItemIr::Trace {
            id,
            trace: TraceIr {
                layer_no: 0,
                half_width: 1000,
                corners: vec![IntPoint::new(0, y), IntPoint::new(length, y)],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(0, y),
                    IntPoint::new(length, y),
                ]),
                nets: vec![net],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        };
        let mut next_id = 500;
        for (i, length) in tuned.iter().enumerate() {
            let y = 40_000 * i as i32;
            ses.push_routed_item(trace(next_id, i as i32 + 1, y, *length as i32));
            next_id += 1;
        }
        if let Some(free_length) = free {
            let y = 40_000 * tuned.len() as i32;
            ses.push_routed_item(trace(
                next_id,
                tuned.len() as i32 + 1,
                y,
                free_length as i32,
            ));
            next_id += 1;
        }
        if blocker {
            // The W2 wall shape, overlapping ROW 0's tooth band
            // (y = 7000: the row-0 trace itself (y ∈ [-1000, 1000])
            // stays 5000 clear of it; every row-0 wave candidate
            // overlaps it). Own net (the last one), own row-free
            // geometry.
            let wall_net = ses.nets.len() as i32;
            ses.push_routed_item(trace(next_id, wall_net, 7_000, 600_000));
        }
        let mut board = Board::from_ses_board(&ses);
        board.set_tuning_active(true);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        WaveWorld { manager, board }
    }

    /// PIN M1 — the 3-net class world, target = the declared max: two
    /// under-min nets meander toward the target (goal = min, the
    /// window arithmetic), the third sits AT the target -> untouched.
    /// Exact outcomes per net on world-derived values:
    /// m1 420000: deficit 40000 -> 2 dents A=10000 -> +40000 -> AT
    /// goal/min 460000 exactly. m2 430000: deficit 30000 ->
    /// ceil(30000/20000) = 2 dents -> +40000 -> 470000 (inside the
    /// window [460000, 500000]). m3 500000 == target -> no row, no
    /// touch. The group row: target 500000, tolerance max−min = 40000,
    /// target_net None (declared, not derived).
    #[test]
    fn match_three_net_class_meanders_to_the_declared_max_target() {
        let mut world = match_world(
            460_000.0,
            500_000.0,
            &[420_000.0, 430_000.0, 500_000.0],
            None,
            false,
        );
        let report = run_stage(&mut world);
        assert_eq!(report.match_groups.len(), 1, "one constrained class");
        let group = &report.match_groups[0];
        assert_eq!(group.class_name, "tuned");
        assert_eq!(group.members, vec![1, 2, 3]);
        assert_eq!(group.target, 500_000.0, "target = the declared max");
        assert_eq!(group.tolerance, 40_000.0, "tolerance = max − min");
        assert_eq!(
            group.target_net, None,
            "declared target has no defining net"
        );
        assert_eq!(report.outcomes.len(), 2, "only the two under-goal nets");
        let m1 = &report.outcomes[0];
        assert_eq!(m1.net_number, 1);
        assert!(m1.landed);
        assert_eq!(m1.deficit, 40_000.0);
        assert_eq!(m1.added_length, 40_000.0);
        assert_eq!(m1.dent_count, 2);
        let m2 = &report.outcomes[1];
        assert_eq!(m2.net_number, 2);
        assert!(m2.landed);
        assert_eq!(m2.deficit, 30_000.0);
        assert_eq!(world.board.net_trace_length(1), 460_000.0);
        assert_eq!(world.board.net_trace_length(2), 470_000.0);
        assert_eq!(world.board.net_trace_length(3), 500_000.0);
        // The at-target net: byte-stable geometry (never touched).
        let corners: Vec<_> = world
            .board
            .trace_polyline(ItemId::new(502))
            .expect("the pin world invariant")
            .corners()
            .to_vec();
        assert_eq!(
            corners,
            vec![
                Point::Int(IntPoint::new(0, 80_000)),
                Point::Int(IntPoint::new(500_000, 80_000))
            ]
        );
    }

    /// PIN M2 — the min-only class world + the TIE-BREAK pin: two
    /// members tie at the longest routed length (300000) -> the
    /// deterministic net-id-ASC target choice (target_net = 1, the
    /// FIRST of the tie — a `>=` mutant flips it to 2 and dies); the
    /// short member (200000) meanders to the goal
    /// max(min, 300000 − 40000) = 260000 exactly (3 dents A=10000,
    /// +60000; AT the goal — the inclusive skip boundary allows it).
    #[test]
    fn match_min_only_tie_break_and_longest_target() {
        let mut world = match_world(
            50_000.0,
            0.0,
            &[300_000.0, 300_000.0, 200_000.0],
            None,
            false,
        );
        let report = run_stage(&mut world);
        assert_eq!(report.match_groups.len(), 1);
        let group = &report.match_groups[0];
        assert_eq!(
            group.target, 300_000.0,
            "target = the longest routed length"
        );
        assert_eq!(
            group.target_net,
            Some(1),
            "the tie breaks to the LOWER net id (walk order: first of a tie wins)"
        );
        assert_eq!(group.tolerance, 40_000.0, "the named constant");
        assert_eq!(group.tolerance, MATCH_TOLERANCE_DBU);
        assert_eq!(report.outcomes.len(), 1, "only the short member");
        let m3 = &report.outcomes[0];
        assert_eq!(m3.net_number, 3);
        assert!(m3.landed);
        assert_eq!(m3.deficit, 60_000.0);
        assert_eq!(m3.added_length, 60_000.0);
        assert_eq!(m3.dent_count, 3);
        assert_eq!(world.board.net_trace_length(3), 260_000.0);
        // The tied members: untouched, byte-stable.
        assert_eq!(world.board.net_trace_length(1), 300_000.0);
        assert_eq!(world.board.net_trace_length(2), 300_000.0);
    }

    /// PIN M3 — the impossible world: the short member (200000, row 0)
    /// sits under the blocker wall — EVERY candidate is rejected, so
    /// the match goal (260000) is unreachable. Honest report: the
    /// outcome row stands with the deficit and zero added; the
    /// geometry is byte-stable; no forced insertion.
    #[test]
    fn match_impossible_world_honest_stop() {
        let mut world = match_world(50_000.0, 0.0, &[200_000.0, 300_000.0], None, true);
        let before = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners()
            .to_vec();
        let report = run_stage(&mut world);
        let group = &report.match_groups[0];
        assert_eq!(group.target, 300_000.0);
        assert_eq!(
            group.target_net,
            Some(2),
            "the other member defines the target"
        );
        assert_eq!(report.outcomes.len(), 1);
        let row = &report.outcomes[0];
        assert_eq!(row.net_number, 1);
        assert!(!row.landed, "the honest stop: no candidate landed");
        assert_eq!(row.deficit, 60_000.0);
        assert_eq!(row.added_length, 0.0);
        assert_eq!(row.dent_count, 0);
        let after = world
            .board
            .trace_polyline(ItemId::new(500))
            .expect("the pin world invariant")
            .corners()
            .to_vec();
        assert_eq!(before, after, "geometry byte-stable under the honest stop");
    }

    /// PIN M4 — the tolerance boundary (DNR-16, both directions): the
    /// goal is target − tolerance (floor min). World A: the short
    /// member sits EXACTLY at target − tolerance (300000 − 40000 =
    /// 260000) -> WITHIN the group tolerance -> untouched, no row (the
    /// inclusive skip). World B: one unit below (259999) -> OUTSIDE ->
    /// processed: 1 dent A=10000 (+20000, the granularity overshoot)
    /// -> 279999 (259999 + one 20000 dent), inside tolerance
    /// (20001 <= 40000). The
    /// tolerance+1/−1 mutants and the skip-comparison mutant each die
    /// on the world whose arm they bend (documented in the mutation
    /// log).
    #[test]
    fn match_tolerance_boundary_both_directions() {
        // World A: exactly AT the tolerance boundary -> untouched.
        let mut world = match_world(50_000.0, 0.0, &[300_000.0, 260_000.0], None, false);
        let report = run_stage(&mut world);
        assert!(
            report.outcomes.is_empty(),
            "AT target − tolerance is WITHIN the group tolerance: no row, no touch"
        );
        assert_eq!(world.board.net_trace_length(2), 260_000.0);
        // World B: one unit below the boundary -> processed.
        let mut world = match_world(50_000.0, 0.0, &[300_000.0, 259_999.0], None, false);
        let report = run_stage(&mut world);
        assert_eq!(report.outcomes.len(), 1, "one unit below is outside");
        let row = &report.outcomes[0];
        assert_eq!(row.net_number, 2);
        assert!(row.landed);
        assert_eq!(row.deficit, 1.0);
        assert_eq!(row.added_length, 20_000.0, "one dent at the coarsest A");
        assert_eq!(
            world.board.net_trace_length(2),
            279_999.0,
            "259999 + one 20000 dent: the granularity overshoot, 20001 inside tolerance"
        );
    }

    /// PIN M5 — the over-max/above-target REPORT-ONLY face: a net
    /// ABOVE the declared max is never truncated (no row, geometry
    /// byte-stable) while the T3 report face carries its POSITIVE
    /// violation row; the other member meanders normally.
    #[test]
    fn match_over_max_never_truncated_reported_only() {
        let mut world = match_world(400_000.0, 500_000.0, &[550_000.0, 380_000.0], None, false);
        let report = run_stage(&mut world);
        assert_eq!(report.outcomes.len(), 1, "only the under-min member");
        assert_eq!(report.outcomes[0].net_number, 2);
        assert!(report.outcomes[0].landed);
        assert_eq!(
            world.board.net_trace_length(1),
            550_000.0,
            "never truncated"
        );
        assert_eq!(
            world.board.net_trace_length(2),
            400_000.0,
            "deficit 20000 -> one dent -> AT min exactly"
        );
        // The report face (T3's calcLengthViolation port): net 1's row
        // is POSITIVE (over-max), net 2's is zero (within the window).
        let rules = world.board.rules();
        assert_eq!(
            rules.length_violation(1, 550_000.0, false),
            50_000.0,
            "the honest over-max row"
        );
        assert_eq!(rules.length_violation(2, 400_000.0, false), 0.0);
    }

    /// PIN M6 — the mixed-board byte-invariance face (criterion 3, at
    /// the engine level): the constraint-free net's geometry is
    /// byte-identical across tuning ON/OFF on the SAME board, while
    /// the tuned member lands its wave under ON.
    #[test]
    fn match_mixed_board_free_net_byte_identical_on_off() {
        let build = || match_world(460_000.0, 0.0, &[420_000.0], Some(150_000.0), false);
        let mut on = build();
        let report = run_stage(&mut on);
        assert!(report.outcomes[0].landed, "the tuned member lands under ON");
        let mut off = build();
        off.board.set_tuning_active(false);
        let mut sink = CaptureDriverSink::default();
        let report_off = run_meander_stage(&mut off.manager, &mut off.board, &mut sink);
        assert!(report_off.outcomes.is_empty(), "OFF: the stage is inert");
        let free_corners = |world: &WaveWorld| -> Vec<Point> {
            world
                .board
                .trace_polyline(ItemId::new(501))
                .expect("the pin world invariant")
                .corners()
                .to_vec()
        };
        assert_eq!(
            free_corners(&on),
            free_corners(&off),
            "the free net byte-identical ON vs OFF"
        );
        assert_eq!(on.board.net_trace_length(2), off.board.net_trace_length(2),);
    }

    /// PIN M7 — determinism x2 on a match-heavy world: two freshly
    /// built M1-shaped worlds run the stage and agree on the full
    /// report AND the final geometry, byte for byte.
    #[test]
    fn match_stage_determinism_times_two() {
        let run = || {
            let mut world = match_world(
                460_000.0,
                500_000.0,
                &[420_000.0, 430_000.0, 500_000.0],
                Some(150_000.0),
                false,
            );
            let report = run_stage(&mut world);
            let corners = |world: &WaveWorld| -> Vec<String> {
                (0..4)
                    .filter_map(|net| world.board.trace_polyline(ItemId::new(500 + net)))
                    .map(|lines| format!("{:?}", lines.corners()))
                    .collect()
            };
            (report, corners(&world))
        };
        let (report_a, corners_a) = run();
        let (report_b, corners_b) = run();
        assert_eq!(report_a, report_b, "the full report is deterministic");
        assert_eq!(corners_a, corners_b, "the final geometry is deterministic");
    }
}
