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
//! coupling window ([`epic_board::aesthetics::AESTHETICS_PARALLELISM_WINDOW_DBU`],
//! the tied constant's definition site,
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
//! **Complexity (quality-r1 MINOR-2 note).** The detector is a brute
//! O(N^2 * S^2) net-pair x segment-pair scan with a same-component
//! skip (an already-connected pair never re-scans); there is NO
//! spatial prefilter — a SegmentIndex-style bucket pass was considered
//! and REJECTED for this round (an optimization with its own bucket-
//! boundary correctness surface needs measured justification first):
//! the gloss-ON battery's wall-clock column at T8 decides whether it
//! becomes an M9+ carry-forward.
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
//!
//! ===========================================================================
//! M8-T4: the FLOW pass — 45° jog/stub elimination + miter/recorner
//! ===========================================================================
//!
//! **The tightener AUDIT (recorded decision).** The change-acceptance
//! faces of `epic_board::trace_tightener` are (a)
//! `polyline_trace_pull_tight` → the 6-arg `pull_tight` dispatch (the
//! fixpoint body, gated by the M7-T3 min-length gate), and (b) the
//! `smoothen_end_corners_at_trace` chain — both driven PER TRACE at
//! route time through `PullTightSeam` (`opt_changed_area` and the
//! `insertForcedTracePolyline` per-trace face), i.e. the tightener
//! runs INSIDE the routing/optimization flow on the changed area
//! only, and it is a PARITY surface (Java `TraceTightener`: the byte
//! gates pin its outputs). A PRE-tightener normalizer would have to
//! hook the seam mid-flow — an invasive edit to a frozen parity
//! surface whose every intermediate geometry is oracle-pinned. So the
//! flow pass is a POST-TIGHTENER pass: it runs in the gloss stage
//! slot (post-optimizer, post-meander, post-pair, post-BUS — flow
//! after spread, the AM2 composition law), cleaning exactly what the
//! tightener leaves: the tightener shortens and smoothens but never
//! converts an axis-aligned 90° jog pair into 45° transition
//! geometry (its 45° faces reduce/smoothen within the existing
//! corridor; they do not manufacture new 45° bridges). The stage
//! order in `pipeline/full.rs` is the audit's implementation face.
//!
//! **Site detection.** A site is a corner window
//! `(c[i], c[i+1], c[i+2], c[i+3])` where the three consecutive
//! segments S1 = c[i]→c[i+1], S2 = c[i+1]→c[i+2], S3 = c[i+2]→c[i+3]
//! are all AXIS-ALIGNED, S1 ∥ S3 (same orientation), and S2 ⊥ both —
//! the staircase/Z artifact. `stub = len(S2)` is the SEPARATION of
//! the 90° corner pair (c[i+1], c[i+2]). The site's KIND: a JOG when
//! S3 has a continuation after c[i+3]; a STUB when S3 is the trace's
//! final segment (it ends at a via/pin face with no continuation —
//! the elimination relocates only the interior corner, the END
//! anchor never moves).
//!
//! **The two faces (in attempt order, jog first).** Both rewrite the
//! window to `[.., c[i], q, c[i+3], ..]` (exactly one corner fewer —
//! the bend-count monotonicity face) and differ only in where q
//! comes from:
//! - **JOG (the exact drop):** eligible when
//!   `stub < JOG_MAX_SEGMENT_FACTOR × trace width` (width = 2 × half
//!   width). q is an EXISTING corner: the flanking corner farther
//!   from the longer flank is dropped (resolve onto the LONGER
//!   flank's line — forward when `len(S3) >= len(S1)`: drop c[i+1],
//!   the merged segment is c[i]→c[i+2]; backward: drop c[i+2], the
//!   merged segment is c[i+1]→c[i+3]). The merged segment must be
//!   angle-exact (axis or exactly 45°) — when the existing geometry
//!   happens to make the drop 45°-exact, the two 90° bends become a
//!   single 45° transition pair with no invented coordinates.
//! - **MITER (the recorner bridge):** eligible when
//!   `stub < MITER_MAX_STUB_DBU`. q is INVENTED on the resolution
//!   line so the bridge is 45° by construction: forward — q sits on
//!   S3's line at one stub-length past c[i] along travel; backward —
//!   q sits on S1's line one stub-length before c[i+3]. This is the
//!   Specctra recorner shape: the 90° pair separated by a short
//!   stub becomes a single 45° miter pair (the two 45° bends at the
//!   ends of one diagonal). When the bridge lands exactly ON the
//!   dropped corner's position (the `stub == flank length` edge),
//!   the bridge degenerates to the jog-drop form — the two faces
//!   agree there, and the row's kind records which face fired.
//!
//! A site eligible for NEITHER threshold is not a candidate and
//! produces NO row (silent, the T3 no-candidate face); a site where
//! a face PASSED its threshold but the candidate was rejected
//! (angle/guards/clearance) records its HONEST row (`landed: false`).
//!
//! **Acceptance (the T3 meander-engine discipline, whole-candidate).**
//! In order: (a) the fixed-state gate — USER_FIXED and SYSTEM_FIXED
//! traces never move; (b) the angle-validity check on every NEW or
//! CHANGED segment (axis-aligned or exactly 45° — the pass only ever
//! runs on FORTYFIVE_DEGREE boards, its second gate below); (c) the
//! bend-count monotonicity guard — the candidate's corner count may
//! not exceed the current count (both faces remove exactly one
//! corner; this is the AM2 bend predicate
//! `bend_to_length_ratio` never increases per candidate); (d) the
//! length budget — the piece-sum length may not grow by more than
//! [`FLOW_MAX_LENGTH_GAIN_DBU`] (both faces strictly shorten by
//! construction — `Δ·(2−√2)` for the dropped window — so the budget
//! is the guard, never the driver); (e) the REAL clearance probe —
//! every piece of the NEW polyline at full length via
//! `check_trace_segment_points` (exact corners; same-net items
//! exempt by the tree query). One failed check rejects the WHOLE
//! candidate; a landed candidate writes through
//! `Board::replace_trace_geometry`. End anchors NEVER move: the drop
//! keeps both window ends as segment endpoints (it removes an
//! interior corner), the bridge inserts q strictly between them, and
//! the stub face's terminal corner is c[i+3] itself — untouched.
//!
//! **Determinism.** Nets in number order, traces in id order, corner
//! windows in ascending index; an acceptance RESTARTS the trace scan
//! (every acceptance shrinks the corner list by one, so the scan
//! terminates); no threads, no hash-order iteration.
//!
//! **Stage gates (in order).** (1) the board's angle restriction must
//! be FORTYFIVE_DEGREE — the pass manufactures 45° geometry, so on
//! 90°/any-angle boards it is a documented no-op (empty report, no
//! rows); (2) the incompletes gate (the T3 bus-stage precedent): a
//! net with incompletes is still being routed — the stage
//! short-circuits with `gated = true`, and epic-cli emits the
//! DISTINCT sibling key `gloss_flow_gated` in the sidecar (the T3
//! fix-round lesson: never conflate a gated hold with a group-free
//! board — the distinct marker exists from day one here); and (3) the
//! ROUTED-NET filter — plane nets (`contains_plane`) are NEVER flow
//! targets, the bus-stage/D5 precedent and the Issue 093/152
//! plane-clearance precaution (plane-routing mode is outside the
//! gloss family's scope — pinned at F13).
//!
//! **F3 DECISION — 45°-diagonal parallel bus grouping: DECLINED.**
//! `detect_bus_groups` stays axis-aligned-only. Rationale (the M7
//! declined-inherit precedent): (a) the T3 detector/pass semantics
//! are axis-projection based — the corridor vote, the perpendicular
//! ladder and the uniform-pitch targets all reduce to ONE perpendicular
//! coordinate per member; a diagonal extension is not an extension
//! but a re-derivation (projection-basis selection, cross-layer
//! diagonal corridors, pitch on the oblique) with its own threshold
//! discipline — a milestone of its own, not a T4 rider; (b) T8-first
//! measurement: no committed measurement shows diagonal-parallel
//! buses exist in the 21-board sample (the parallelism_ratio face is
//! direction-gated per AM2 — a speculative win cannot justify the
//! surface); (c) the coupling-window semantics for diagonals
//! (perpendicular distance vs projection overlap) is a definitional
//! question owned by a re-charter. REOPEN TRIGGER: a T8 measurement
//! face showing ≥ 1 diagonal-parallel bus group in the sample corpus
//! AND bend/length headroom on the same boards. The scope limit the
//! T3 module doc recorded (diagonal parallels "T4 flow territory")
//! is hereby dispositioned: they are NEITHER grouped NOR moved, and
//! the flow pass does not create any (both faces preserve the
//! axis-aligned flanks; only the bridge segment is diagonal).
//!
//! **Two-regime safety (flow).** The ONLY caller gate is the resolved
//! `router.gloss.flow` flag (`BatchSettings::flow_active`, default
//! false) plus the two stage gates above; an OFF run never
//! constructs a report and never touches the board. The report rides
//! the `--dump-aesthetics` SIDECAR ONLY (the `gloss_flow` block
//! rendered by epic-cli — NEVER the manifest: the canary cf607714
//! pins manifest bytes).

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::FixedState;
use epic_board::routing_board_search::check_trace_segment_points;
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

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
    // The integer-domain compare (quality-r1 NIT): both magnitudes are
    // exact either way — this keeps the predicate in the domain it
    // reasons about (the i64 window constant's own type).
    if offset > epic_board::aesthetics::AESTHETICS_PARALLELISM_WINDOW_DBU {
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
    /// rejected, or no movable candidate existed). ASYMMETRY NOTE
    /// (quality-r1 NIT): the sweep's rejections each carry a
    /// [`BusMoveRow`], but the DEGENERATE paths (no corridor, fewer
    /// than two participants, a pitch <= 0 ladder) bump this counter
    /// with ZERO rows — synthetic rows would need a fabricated
    /// `target_pos` for members that never got a participant frame,
    /// so the count-only form is the honest one.
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
    /// The INCOMPLETES-GATE marker (quality-r1 F6): true when the
    /// stage short-circuited because some net still has incompletes.
    /// Byte-identical to "nothing grouped" in the `bus_groups` array,
    /// but the sidecar can then distinguish the honest hold (still
    /// routing) from a genuinely group-free board via the sibling
    /// `bus_groups_gated` key epic-cli emits for this face. Default
    /// false = the OFF/default semantics unchanged.
    pub gated: bool,
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
            // Same-component skip (quality-r1 MINOR-2): once a and b
            // share a root, the union would be a no-op (attach-larger-
            // root-to-smaller is idempotent) — skip the whole segment
            // scan. Semantics-free.
            let mut ra = a;
            while parent[ra] != ra {
                ra = parent[ra];
            }
            let mut rb = b;
            while parent[rb] != rb {
                rb = parent[rb];
            }
            if ra == rb {
                continue;
            }
            let qualifies = segments[a].1.iter().any(|sa| {
                segments[b]
                    .1
                    .iter()
                    .any(|sb| qualifying_overlap(sa, sb).is_some())
            });
            if qualifies {
                if ra < rb {
                    parent[rb] = ra;
                } else {
                    parent[ra] = rb;
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
    let mut outcome = BusGroupOutcome {
        members: members.to_vec(),
        member_names: members
            .iter()
            .map(|net| participant_name(board, *net))
            .collect(),
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
    // ties; the lowest layer survives full ties (scan order). The
    // pure-fn extraction (quality-r1 MINOR-3a) is the pin surface.
    let Some(corridor_key) = corridor_pick(&votes) else {
        outcome.moves_rejected = member_reject_count(members.len());
        return outcome;
    };
    outcome.layer = corridor_key.0;
    outcome.horizontal = corridor_key.1;
    // The corridor's voter nets, derived from the vote map (quality-r1
    // NIT fold — no second overlap walk), sorted ASC.
    let mut voter_list = votes.get(&corridor_key).cloned().unwrap_or_default();
    voter_list.sort_unstable();
    build_participants_and_sweep(
        manager,
        board,
        &member_segments,
        corridor_key,
        &voter_list,
        &mut outcome,
    );
    outcome
}

/// The degenerate-ladder reject count (saturating).
fn member_reject_count(members: usize) -> u32 {
    u32::try_from(members).unwrap_or(u32::MAX)
}

/// The CORRIDOR PICK over the vote map: the (layer, orientation) key
/// with the most distinct member nets; a count tie breaks to the
/// horizontal orientation — INCLUDING across layers ((1, horizontal)
/// beats (0, vertical) at equal count); a full tie (same orientation,
/// different layers) survives on the LOWEST layer (the BTreeMap scan
/// is (layer ASC, false < true), and only a strictly-better key
/// replaces the incumbent). The `anchor_index_of` precedent: a pure
/// fn so the tie faces pin without a board (quality-r1 MINOR-3a; the
/// M-TIE mutant dies on the cross-layer face).
#[must_use]
fn corridor_pick(votes: &std::collections::BTreeMap<(i32, bool), Vec<i32>>) -> Option<(i32, bool)> {
    let mut best: Option<((i32, bool), usize)> = None;
    for (key, nets) in votes {
        let better = match &best {
            None => true,
            Some((best_key, best_count)) => {
                nets.len() > *best_count || (nets.len() == *best_count && key.1 && !best_key.1)
            }
        };
        if better {
            best = Some((*key, nets.len()));
        }
    }
    best.map(|(key, _)| key)
}

/// The participants, the position ladder, and the relocation sweep
/// (the pass body after the corridor is chosen).
fn build_participants_and_sweep(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    member_segments: &[(i32, Vec<AxisSegment>)],
    corridor_key: (i32, bool),
    voter_list: &[i32],
    outcome: &mut BusGroupOutcome,
) {
    // Participants: voters in net order; each contributes its longest
    // qualifying-on-the-key segment (tie: trace id, then start).
    let mut participants: Vec<Participant> = Vec::new();
    for &net_number in voter_list {
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
        // `index` IS the participant's position in the position-sorted
        // ladder (participants are unique by net, so the sweep holds
        // each exactly once — quality-r1 NIT: no O(n) re-lookup).
        let target = anchor_pos + (index as i64 - anchor_index as i64) * median_pitch;
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
        return GlossBusReport {
            gated: true,
            ..Default::default()
        };
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

// ===========================================================================
// M8-T4: the FLOW pass — jog/stub elimination + miter/recorner
// ===========================================================================

/// The JOG threshold FACTOR: a corner window's stub (the perpendicular
/// segment separating a 90° corner pair) is jog-eligible only when
/// `stub < JOG_MAX_SEGMENT_FACTOR × trace width` (width = 2 × half
/// width). Value: 6.0 — a jog shorter than six trace widths is the
/// staircase artifact; longer separations are real routing intent.
/// The exact edge (`<`, strict) is pinned at [`jog_threshold_ok`] and
/// at the ±1 world pins with BOTH-direction mutations (DNR-16): a
/// factor mutant of either sign flips exactly one of the 11_999 /
/// 12_001 verdicts in `jog_miter_threshold_boundary_worlds`.
pub const JOG_MAX_SEGMENT_FACTOR: f64 = 6.0;

/// The MITER threshold, in board DBU: a 90° corner pair separated by
/// a stub STRICTLY shorter than this is miter-eligible (the recorner
/// bridge). Value: 30_000 DBU = 3 mm at the um-10 transform — the
/// stub must be small enough that the 45° bridge is visually a
/// recorner, not a reroute. The exact edge is pinned at the
/// 29_999 / 30_001 world pins with both-direction mutations (DNR-16).
pub const MITER_MAX_STUB_DBU: i64 = 30_000;

/// The candidate LENGTH-GAIN guard, in board DBU: a flow candidate
/// whose piece-sum length exceeds the trace's current length by more
/// than this is rejected (equality allowed). Both faces strictly
/// shorten by construction (`Δ·(2−√2)` per landing), so the guard is
/// the budget law, never the driver. Value: 40_000 DBU = 4 mm, the
/// T3 [`BUS_MAX_LENGTH_GAIN_DBU`] mirror. The exact edge is pinned at
/// [`flow_length_guard_ok`] with both-direction mutations (DNR-16).
pub const FLOW_MAX_LENGTH_GAIN_DBU: i64 = 40_000;

/// The flow candidate's KIND (the sidecar face): the exact jog/stub
/// drop, or the invented-q recorner bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowKind {
    /// The exact corner drop (existing geometry is 45°-exact).
    Jog,
    /// The drop at a trace end (the terminal segment ends at a
    /// via/pin face with no continuation; the END anchor never moves).
    Stub,
    /// The invented-q 45° bridge (the Specctra recorner shape).
    Miter,
}

impl FlowKind {
    /// The sidecar label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FlowKind::Jog => "jog",
            FlowKind::Stub => "stub",
            FlowKind::Miter => "miter",
        }
    }
}

/// One flow candidate's row (the honest-stop face): the site locator
/// is the stub's FIRST corner (c[i+1] of the window — stable under
/// the landing, which removes a corner after it), and `landed` is the
/// whole-candidate verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowCandidateRow {
    pub net: i32,
    pub net_name: String,
    pub kind: FlowKind,
    /// The stub segment's first corner (board DBU).
    pub corner_x: i64,
    /// The stub segment's first corner (board DBU).
    pub corner_y: i64,
    /// Whether the candidate LANDED (the geometry was written).
    pub landed: bool,
}

/// The gloss FLOW stage report (the sidecar's `gloss_flow` block
/// source). Default = empty = the OFF face (never serialized).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlossFlowReport {
    /// Per-candidate rows in landing/scan order (net, trace, corner
    /// window order — deterministic).
    pub rows: Vec<FlowCandidateRow>,
    /// The INCOMPLETES-GATE marker (the T3 `bus_groups_gated` lesson —
    /// the distinct sibling key exists from day one): true when the
    /// stage short-circuited because some net still has incompletes.
    /// Default false = the OFF/default semantics unchanged.
    pub gated: bool,
}

impl GlossFlowReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The ANGLE-VALIDITY predicate for one new/changed segment: axis-
/// aligned or exactly 45° (the pass only ever runs on
/// FORTYFIVE_DEGREE boards, so this is exactly the board's angle
/// restriction). Pinnable in isolation.
#[must_use]
pub fn flow_angle_ok(dx: i64, dy: i64) -> bool {
    dx == 0 || dy == 0 || dx.abs() == dy.abs()
}

/// The flow LENGTH-GUARD predicate, pinnable in isolation (the DNR-16
/// exact edge: old + GAIN lands, old + GAIN + 1 rejects —
/// both-direction mutations verified).
#[must_use]
pub fn flow_length_guard_ok(old_len: f64, new_len: f64) -> bool {
    new_len <= old_len + FLOW_MAX_LENGTH_GAIN_DBU as f64
}

/// The BEND-MONOTONICITY guard (the AM2 bend predicate): a candidate
/// may never introduce more bends than it removes — the corner count
/// strictly non-increasing per candidate. Both faces remove exactly
/// one corner, so the guard is the pinned invariant, not the driver.
#[must_use]
pub fn bend_monotonic_ok(before: usize, after: usize) -> bool {
    after <= before
}

/// The JOG-threshold predicate, pinnable in isolation: strict `<`
/// (stub == threshold is NOT a jog). The constant is read at the pin —
/// a factor mutant of either sign flips exactly one of the two
/// boundary verdicts (DNR-16).
#[must_use]
pub fn jog_threshold_ok(stub: i64, width: i64) -> bool {
    (stub as f64) < JOG_MAX_SEGMENT_FACTOR * width as f64
}

/// One axis-aligned segment view (the flow window's working unit).
struct FlowAxisSeg {
    horizontal: bool,
    perp: i64,
    lo: i64,
    hi: i64,
}

impl FlowAxisSeg {
    fn len(&self) -> i64 {
        self.hi - self.lo
    }
    /// The along-axis coordinate of a point on this segment's line.
    fn along(&self, p: &IntPoint) -> i64 {
        if self.horizontal {
            i64::from(p.x)
        } else {
            i64::from(p.y)
        }
    }
}

/// The axis view of a corner pair; `None` when not axis-aligned.
fn flow_axis_seg(a: &IntPoint, b: &IntPoint) -> Option<FlowAxisSeg> {
    let (ax, ay, bx, by) = (
        i64::from(a.x),
        i64::from(a.y),
        i64::from(b.x),
        i64::from(b.y),
    );
    if ax != bx && ay == by {
        Some(FlowAxisSeg {
            horizontal: true,
            perp: ay,
            lo: ax.min(bx),
            hi: ax.max(bx),
        })
    } else if ax == bx && ay != by {
        Some(FlowAxisSeg {
            horizontal: false,
            perp: ax,
            lo: ay.min(by),
            hi: ay.max(by),
        })
    } else {
        None
    }
}

/// The site verdict of one corner-window attempt.
enum SiteVerdict {
    /// The candidate landed (the geometry was written); `window` is
    /// the landing's corner-window index for the resume scan.
    Landed { window: usize },
    /// A face passed its threshold but the candidate was rejected
    /// (angle/guards/clearance) — the honest row.
    Rejected,
    /// No candidate at this window (advance).
    None,
}

/// The per-trace flow pass: corner-window scan with RESUME-on-
/// acceptance (every landing shrinks the corner list by one, so the
/// scan terminates). `rows` — every RECORDED attempt (landed or
/// honest-stop) is pushed in scan order.
///
/// RESUME SAFETY (quality-r1 Q1a): a landing at window `j` changes
/// only corners `[j, j+2]` (the drop removes one of `j+1`/`j+2`, the
/// bridge replaces `j+2`); a window starting at `k` reads corners
/// `k..=k+3`, so NO window with `k < j - 3` can see a changed corner —
/// the scan resumes at `j.saturating_sub(3)` instead of restarting at
/// 0, which provably changes no verdict while eliminating (a) the
/// O(C) re-probes of unchanged rejected windows per landing (the
/// latent O(C^3) face) and (b) their DUPLICATE `landed: false` rows
/// (the sidecar-honesty face: a rejected window's row is recorded
/// EXACTLY once, at its first encounter — no duplicate rows can reach
/// the sidecar; the committed flow goldens carry none, which is why
/// they re-derive byte-identically across this refactor).
fn flow_trace_pass(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net: i32,
    trace_id: ItemId,
    rows: &mut Vec<FlowCandidateRow>,
) {
    // The loop-invariant fixed-state gate (quality-r1 NIT hoist): a
    // USER_FIXED / SYSTEM_FIXED trace never moves — identical
    // zero-rows semantics to the in-window gate this replaces (the
    // window gate returned `None` before any row push).
    if board
        .get(trace_id)
        .is_some_and(|entry| matches!(entry.fixed, FixedState::UserFixed | FixedState::SystemFixed))
    {
        return;
    }
    let mut resume = 0;
    'restart: loop {
        let Some(lines) = board.trace_polyline(trace_id) else {
            return;
        };
        let corners = lines.corners();
        let mut index = resume;
        while index + 3 < corners.len() {
            match flow_window(manager, board, net, trace_id, &corners, index, rows) {
                SiteVerdict::Landed { window } => {
                    resume = window.saturating_sub(3);
                    continue 'restart;
                }
                SiteVerdict::Rejected => index += 1,
                SiteVerdict::None => index += 1,
            }
        }
        return;
    }
}

/// The row builder (the site locator = the stub's first corner).
fn landed_row(
    board: &Board,
    net: i32,
    kind: FlowKind,
    at: &IntPoint,
    landed: bool,
) -> FlowCandidateRow {
    FlowCandidateRow {
        net,
        net_name: participant_name(board, net),
        kind,
        corner_x: i64::from(at.x),
        corner_y: i64::from(at.y),
        landed,
    }
}

/// One corner-window attempt: detect the site, then try the JOG face
/// (exact drop) and the MITER face (invented-q bridge) in order. Every
/// RECORDED attempt (landed or honest-stop) is pushed to `rows`; a
/// rejected attempt ADVANCES the scan (the same window is never
/// re-attempted), a landing RESTARTS it.
#[allow(clippy::too_many_arguments)]
fn flow_window(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net: i32,
    trace_id: ItemId,
    corners: &[Point],
    index: usize,
    rows: &mut Vec<FlowCandidateRow>,
) -> SiteVerdict {
    let corner_count = corners.len();
    let (Some(c0), Some(c1), Some(c2), Some(c3)) = (
        as_int(&corners[index]),
        as_int(&corners[index + 1]),
        as_int(&corners[index + 2]),
        as_int(&corners[index + 3]),
    ) else {
        return SiteVerdict::None;
    };
    let (Some(s1), Some(s2), Some(s3)) = (
        flow_axis_seg(&c0, &c1),
        flow_axis_seg(&c1, &c2),
        flow_axis_seg(&c2, &c3),
    ) else {
        return SiteVerdict::None;
    };
    // S1 ∥ S3 (same orientation), S2 ⊥ both.
    if s1.horizontal != s3.horizontal || s2.horizontal == s1.horizontal {
        return SiteVerdict::None;
    }
    let stub = s2.len();
    // Defense-in-depth only: PROVABLY UNREACHABLE — `flow_axis_seg`
    // answers `None` for a degenerate (a == b) corner pair, so every
    // `Some` segment carries a positive length (quality-r1 NIT).
    if stub <= 0 {
        return SiteVerdict::None;
    }
    let kind = if index + 3 == corner_count - 1 {
        FlowKind::Stub
    } else {
        FlowKind::Jog
    };
    // The JOG face (exact drop): eligible when the stub clears the
    // factor×width threshold. An angle-INEXACT drop forms NO jog
    // candidate (no row — the window simply is not a jog candidate);
    // the miter face below still gets its own attempt.
    //
    // WIDTHLESS-TRACE SEMANTICS (spec-r1 NIT, doc-only): a trace with
    // NO half width reads as width 0 here, so `stub < 0 × factor` is
    // never true and the JOG face silently SKIPS the window — while
    // the MITER face (a DBU threshold, width-independent) stays live.
    // That asymmetry is the documented behavior, not an oversight:
    // the jog threshold is meaningless without a width to scale, and
    // the pin worlds (F1/F4/F12) all carry width-2000 traces, so the
    // pinned semantics are untouched by this note. No behavior change.
    if jog_threshold_ok(
        stub,
        i64::from(2 * board.trace_half_width(trace_id).unwrap_or(0)),
    ) {
        let forward = s3.len() >= s1.len();
        let merged = if forward { (c0, c2) } else { (c1, c3) };
        let drop_index = if forward { index + 1 } else { index + 2 };
        let dx = i64::from(merged.1.x) - i64::from(merged.0.x);
        let dy = i64::from(merged.1.y) - i64::from(merged.0.y);
        if flow_angle_ok(dx, dy) {
            let landed =
                apply_flow_candidate(manager, board, trace_id, net, corners, drop_index, None);
            rows.push(landed_row(board, net, kind, &c1, landed));
            return if landed {
                SiteVerdict::Landed { window: index }
            } else {
                SiteVerdict::Rejected
            };
        }
    }
    // The MITER face (invented-q bridge).
    if stub < MITER_MAX_STUB_DBU {
        let forward = s3.len() >= s1.len();
        // Travel sign along the flank axis.
        let travel = s1.along(&c3) - s1.along(&c0);
        let sign = i64::signum(travel);
        if sign == 0 {
            return SiteVerdict::None;
        }
        let (q_along, q_perp) = if forward {
            // q on S3's line, one stub-length past c0 along travel.
            let reach = s1.len();
            if stub > reach || sign * (s1.along(&c3) - (s1.along(&c0) + sign * stub)) <= 0 {
                // The bridge overshoots the corner or leaves no
                // positive S3' — the honest stop.
                rows.push(landed_row(board, net, FlowKind::Miter, &c1, false));
                return SiteVerdict::Rejected;
            }
            (s1.along(&c0) + sign * stub, s3.perp)
        } else {
            // q on S1's line, one stub-length before c3 along travel.
            let reach = s3.len();
            if stub > reach || sign * ((s1.along(&c3) - sign * stub) - s1.along(&c0)) <= 0 {
                rows.push(landed_row(board, net, FlowKind::Miter, &c1, false));
                return SiteVerdict::Rejected;
            }
            (s1.along(&c3) - sign * stub, s1.perp)
        };
        let q_some = if s1.horizontal {
            i32::try_from(q_along)
                .ok()
                .zip(i32::try_from(q_perp).ok())
                .map(|(x, y)| IntPoint::new(x, y))
        } else {
            i32::try_from(q_perp)
                .ok()
                .zip(i32::try_from(q_along).ok())
                .map(|(x, y)| IntPoint::new(x, y))
        };
        let Some(q) = q_some else {
            return SiteVerdict::None; // overflow: never a candidate, never a wrap
        };
        // The bridge's new/changed segments: c0→q (the diagonal) and
        // q→c3 (the flank extension) — angle-validated explicitly
        // (the construction is 45°-by-design, the check is the law).
        let diag_dx = i64::from(q.x) - i64::from(c0.x);
        let diag_dy = i64::from(q.y) - i64::from(c0.y);
        let flank_dx = i64::from(c3.x) - i64::from(q.x);
        let flank_dy = i64::from(c3.y) - i64::from(q.y);
        if !flow_angle_ok(diag_dx, diag_dy) || !flow_angle_ok(flank_dx, flank_dy) {
            rows.push(landed_row(board, net, FlowKind::Miter, &c1, false));
            return SiteVerdict::Rejected;
        }
        let landed = apply_flow_candidate(
            manager,
            board,
            trace_id,
            net,
            corners,
            index + 1,
            Some((index + 2, q)),
        );
        rows.push(landed_row(board, net, FlowKind::Miter, &c1, landed));
        return if landed {
            SiteVerdict::Landed { window: index }
        } else {
            SiteVerdict::Rejected
        };
    }
    SiteVerdict::None
}

/// Builds and (on full acceptance) lands one flow candidate.
/// `drop_index` — the corner to REMOVE (both faces). `bridge` —
/// `Some((replaced_index, q))` for the miter face: the corner at
/// `replaced_index` is replaced by `q` (in addition to the drop),
/// giving the window rewrite `[.., c0, q, c3, ..]`. Whole-candidate
/// acceptance: fixed gate, bend guard, length guard, REAL clearance
/// probe, then `replace_trace_geometry`.
#[allow(clippy::too_many_arguments)]
fn apply_flow_candidate(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    net: i32,
    corners: &[Point],
    drop_index: usize,
    bridge: Option<(usize, IntPoint)>,
) -> bool {
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
    // Build the new corner list: drop `drop_index`, optionally
    // replacing `bridge.0` with q (the bridge tuple's corner).
    let mut points: Vec<Point> = Vec::with_capacity(corners.len() - 1);
    for (position, corner) in corners.iter().enumerate() {
        if position == drop_index {
            continue;
        }
        if let Some((replaced, q)) = bridge
            && position == replaced
        {
            points.push(Point::Int(q));
            continue;
        }
        points.push(corner.clone());
    }
    let new_lines = Polyline::from_points(&points);
    // Exactly one corner fewer, or the construction collapsed
    // something (degenerate) — not a landing.
    if new_lines.lines.len() != lines.lines.len() - 1 {
        return false;
    }
    // The bend-count monotonicity guard (the AM2 bend predicate).
    if !bend_monotonic_ok(lines.lines.len(), new_lines.lines.len()) {
        return false;
    }
    // The length budget (both faces shorten; the guard is the law).
    let old_len = piece_length_sum(&lines.corners());
    let new_len = piece_length_sum(&new_lines.corners());
    if !flow_length_guard_ok(old_len, new_len) {
        return false;
    }
    // The REAL clearance probe: every piece of the NEW polyline at
    // full length (exact corners; same-net items exempt by the tree
    // query). One conflict rejects the whole candidate.
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
            &[net],
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

/// The stage entry (the pipeline's post-bus slot — flow after spread,
/// the AM2 composition law; the caller gates on
/// `BatchSettings::flow_active` — the flag is the only caller gate).
/// Stage gates IN ORDER: the FORTYFIVE_DEGREE angle gate (the pass
/// manufactures 45° geometry; a documented no-op on 90°/any-angle
/// boards), then the incompletes gate (the T3 precedent — the hold is
/// marked via `gated`, surfaced as the DISTINCT `gloss_flow_gated`
/// sidecar key). The sweep covers ROUTED non-plane nets only — a
/// `contains_plane` net is never a flow target (the bus-stage/D5
/// precedent, the Issue 093/152 plane-clearance precaution; pinned at
/// F13). Emits NO events and NO stage transition (the meander-stage
/// precedent). An empty report = nothing eligible, nothing moved.
pub fn run_gloss_flow_stage(manager: &mut SearchTreeManager, board: &mut Board) -> GlossFlowReport {
    if board.rules().trace_angle_restriction
        != epic_board::rules_surf::AngleRestriction::FortyfiveDegree
    {
        return GlossFlowReport::default();
    }
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    if incompletes.iter().any(|row| row.incomplete_count > 0) {
        return GlossFlowReport {
            gated: true,
            ..Default::default()
        };
    }
    let mut report = GlossFlowReport::default();
    let routed: Vec<i32> = {
        let rules = board.rules();
        rules
            .nets
            .iter()
            .filter(|(number, net)| !net.contains_plane && has_on_board_traces(board, *number))
            .map(|(number, _)| number)
            .collect()
    };
    for net in routed {
        let mut trace_ids: Vec<_> = board
            .get_connectable_items(net)
            .into_iter()
            .filter(|&id| board.is_on_the_board(id) && board.trace_polyline(id).is_some())
            .collect();
        trace_ids.sort();
        for trace_id in trace_ids {
            flow_trace_pass(manager, board, net, trace_id, &mut report.rows);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse;
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
        // Net 1's delta is nonzero in this world (100_000 -> 97_500),
        // so the row MUST exist — a dropped row is a regression, not a
        // pass (quality-r1: the disjunctive form could mask it).
        let row1 = group
            .rows
            .iter()
            .find(|row| row.net == 1)
            .expect("net 1 row must exist (its delta is nonzero)");
        assert!(
            row1.landed.is_none(),
            "the fixed member never lands: {row1:?}"
        );
        assert!(net_traces(&board, 1)[0].contains("100000"));
    }

    /// **P10 (the gated marker, quality-r1 F6)** — a world with an
    /// incomplete net at stage entry (the routed bus + an open net):
    /// the stage returns `gated == true` with EMPTY groups — the
    /// honest hold is distinguishable from a genuinely group-free
    /// board via the marker (and epic-cli's sibling sidecar key).
    #[test]
    fn p10_gated_world_marks_the_hold() {
        let (mut manager, mut board) = parse(GATED_WORLD);
        let report = run_stage(&mut manager, &mut board);
        assert!(report.gated, "the hold is marked");
        assert!(
            report.groups.is_empty(),
            "the gate short-circuits before the detector"
        );
        // The OFF/default face is unchanged: the marker is false.
        assert!(!GlossBusReport::default().gated);
    }

    /// The GATED world: the routed 3-net bus + an open net (two pins,
    /// no wire) — an incomplete at stage entry. The pass never runs
    /// (the gate fires first); the DSN string exercises the
    /// test_util parse face.
    const GATED_WORLD: &str = r#"
(pcb gated_world.dsn
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
    (component "IA" (place "A1" 1500 3000 Front 0.000000))
    (component "IA" (place "A2" 54500 3000 Front 0.000000))
    (component "IB" (place "B1" 1500 6000 Front 0.000000))
    (component "IB" (place "B2" 54500 6000 Front 0.000000))
    (component "IC" (place "C1" 1500 9000 Front 0.000000))
    (component "IC" (place "C2" 54500 9000 Front 0.000000))
    (component "IO" (place "O1" 2000 24000 Front 0.000000))
    (component "IO" (place "O2" 50000 24000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (image "IB" (pin "PAD" "P" 0 0))
    (image "IC" (pin "PAD" "P" 0 0))
    (image "IO" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 500 0 0))
      (attach off)
    )
    (padstack "VIA"
      (shape (circle F.Cu 100 0 0))
      (shape (circle B.Cu 100 0 0))
      (attach off)
    )
  )
  (network
    (via V VIA kicad_default)
    (net "bus_a" (pins "A1"-"P" "A2"-"P"))
    (net "bus_b" (pins "B1"-"P" "B2"-"P"))
    (net "bus_c" (pins "C1"-"P" "C2"-"P"))
    (net "open" (pins "O1"-"P" "O2"-"P"))
    (class kicad_default "bus_a" "bus_b" "bus_c" "open" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 1500 3000 3200 3000 3200 10000 51000 10000 51000 3000 54500 3000) (net "bus_a"))
    (wire (path F.Cu 200 1500 6000 2600 6000 2600 14000 52000 14000 52000 6000 54500 6000) (net "bus_b"))
    (wire (path F.Cu 200 1500 9000 1800 9000 1800 18500 53000 18500 53000 9000 54500 9000) (net "bus_c"))
  )
)
"#;

    /// **P11 (the corridor PICK tie faces, quality-r1 MINOR-3a)** —
    /// the pure-fn surface: count beats horizontal preference;
    /// horizontal beats vertical ACROSS layers at equal count (the
    /// M-TIE kill face); the lowest layer survives a full
    /// same-orientation tie.
    #[test]
    fn p11_corridor_pick_tie_faces() {
        let votes = |entries: Vec<((i32, bool), Vec<i32>)>| {
            entries
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        // Strictly-greater count beats the horizontal preference.
        assert_eq!(
            corridor_pick(&votes(vec![
                ((0, true), vec![1, 2]),
                ((1, false), vec![1, 2, 3]),
            ])),
            Some((1, false)),
            "count wins over the horizontal tie-break"
        );
        // Count tie: horizontal beats vertical ACROSS layers — (1,
        // true) beats (0, false). The M-TIE mutant (preference
        // negated) flips this verdict.
        assert_eq!(
            corridor_pick(&votes(vec![
                ((0, false), vec![1, 2]),
                ((1, true), vec![1, 2]),
            ])),
            Some((1, true)),
            "the horizontal preference crosses layers"
        );
        // Full tie (same orientation, different layers): the lowest
        // layer survives (both orientations pinned).
        assert_eq!(
            corridor_pick(&votes(vec![
                ((0, true), vec![1, 2]),
                ((1, true), vec![1, 2]),
            ])),
            Some((0, true)),
            "lowest layer on a horizontal tie"
        );
        assert_eq!(
            corridor_pick(&votes(vec![
                ((0, false), vec![1, 2]),
                ((1, false), vec![1, 2]),
            ])),
            Some((0, false)),
            "lowest layer on a vertical tie"
        );
        // Empty vote map: no corridor.
        assert_eq!(corridor_pick(&votes(vec![])), None);
    }

    /// **P12 (the sweep-order world, quality-r1 MINOR-3b)** — net
    /// order != position order AND the outcome depends on the order:
    /// the four-member ladder 92_000 / 100_000 / 116_000 / 124_000
    /// carries nets (1, 2, 4, 3) — the member at the TOP rung (net 3)
    /// moves DOWN into net 4's CURRENT slot (116_000, distance zero),
    /// so the NET-ordered sweep rejects it (and its half step sits at
    /// the exact-clearance edge, also rejected) while net 4 lands
    /// first and vacates. A POSITION-ordered sweep moves net 4 out of
    /// the way first and net 3 LANDS — the assertions fail under the
    /// mutant (M-SWEEP's kill face). Literal geometry throughout.
    #[test]
    fn p12_sweep_order_outcome_depends_on_net_order() {
        let (mut manager, mut board) = sweep_order_world();
        let report = run_stage(&mut manager, &mut board);
        assert_eq!(report.groups.len(), 1);
        let group = &report.groups[0];
        assert_eq!(group.members, vec![1, 2, 3, 4]);
        assert_eq!(group.moves_landed, 1, "only net 4 lands");
        assert_eq!(group.moves_rejected, 1, "net 3 is the honest stop");
        let row3 = group
            .rows
            .iter()
            .find(|row| row.net == 3)
            .expect("net 3 row");
        assert_eq!(row3.from_pos, 124_000);
        assert_eq!(row3.target_pos, 116_000);
        assert_eq!(row3.landed, None, "net 3 is blocked by net 4's slot");
        let row4 = group
            .rows
            .iter()
            .find(|row| row.net == 4)
            .expect("net 4 row");
        assert_eq!(row4.from_pos, 116_000);
        assert_eq!(row4.target_pos, 108_000);
        assert_eq!(row4.landed, Some(108_000), "net 4 lands first (net order)");
        let (violations, _) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "the post-sweep board is DRC-clean");
    }

    /// The sweep-order world: the Z template, four members, nets
    /// (1, 2, 4, 3) at spans (92_000, 100_000, 116_000, 124_000) —
    /// pitches 8_000 / 16_000 / 8_000, median 8_000, anchor net 2.
    fn sweep_order_world() -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[
                ("bus_a", false),
                ("bus_b", false),
                ("bus_c", false),
                ("bus_d", false),
            ],
        );
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
        member(&mut ses, 500, 1, 30_000, 92_000, 32_000, 510_000);
        member(&mut ses, 501, 2, 60_000, 100_000, 26_000, 520_000);
        // NOTE the swapped net assignment: net 4 at 116_000, net 3 at
        // 124_000 — the ladder's top two rungs carry DESCENDING nets.
        member(&mut ses, 503, 4, 90_000, 116_000, 18_000, 530_000);
        member(&mut ses, 502, 3, 120_000, 124_000, 10_000, 540_000);
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    // ===================================================================
    // M8-T4: the FLOW pins (literal geometry throughout — the bug-212
    // rule: worlds never derive from the constants under test)
    // ===================================================================

    /// The 45° flow base: the T3 base with the FORTYFIVE_DEGREE angle
    /// restriction (the flow stage's angle gate) — same clearance
    /// matrix (2000 pair), widths (half 1000), transform.
    fn flow_base_ses() -> SesBoard {
        use epic_dsn::coordinate_transform::CoordinateTransform;
        use epic_dsn::layer_structure::{Layer, LayerStructure};
        use epic_dsn::sink::BoardSink;
        use epic_dsn::state::AngleRestriction as DsnAngleRestriction;

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
                trace_angle_restriction: DsnAngleRestriction::FortyfiveDegree,
                default_trace_half_widths: vec![1000],
                min_trace_half_width: 1000,
                max_trace_half_width: 1000,
                pin_edge_to_turn_dist: 0.0,
                default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            },
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        for name in ["flow_a", "flow_b", "flow_c"] {
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

    /// An arbitrary-layer trace (the stub world needs the B.Cu
    /// completion wire; `straight_trace` is layer-0-only).
    fn trace_on(ses: &mut SesBoard, id: i32, net: i32, layer: i32, corners: Vec<IntPoint>) {
        ses.push_routed_item(ItemIr::Trace {
            id,
            trace: TraceIr {
                layer_no: layer,
                half_width: 1000,
                corners: corners.clone(),
                polyline: TraceIr::polyline_of_corners(&corners),
                nets: vec![net],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        });
    }

    /// The FLOW WORLD: three nets, one of each candidate kind.
    /// - net 1 (flow_a): the double-jog staircase — BOTH drops are
    ///   45°-exact (crafted |dx| == dy windows).
    /// - net 2 (flow_b): the miter site — stub 16_000 (jog-INELIGIBLE:
    ///   16_000 >= 6.0 x 2000; miter-eligible: 16_000 < 30_000).
    /// - net 3 (flow_c): the stub — a trace ENDING at a via (no
    ///   continuation past it), completed on B.Cu through the via.
    fn flow_world() -> (SearchTreeManager, Board) {
        use epic_dsn::sink::BoardSink;
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        // net 1: 6 corners; the two staircase jogs are 4_000 DBU.
        straight_trace(
            &mut ses,
            500,
            1,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(24_000, 200_000),
                IntPoint::new(24_000, 204_000),
                IntPoint::new(28_000, 204_000),
                IntPoint::new(28_000, 200_000),
                IntPoint::new(480_000, 200_000),
            ],
        );
        // net 2: the Z with the 16_000 vertical stub.
        straight_trace(
            &mut ses,
            501,
            2,
            vec![
                IntPoint::new(20_000, 100_000),
                IntPoint::new(200_000, 100_000),
                IntPoint::new(200_000, 116_000),
                IntPoint::new(420_000, 116_000),
            ],
        );
        // net 3: the stub trace on F.Cu (ends AT the via) + the B.Cu
        // completion + the via itself (a real 2-layer padstack).
        trace_on(
            &mut ses,
            502,
            3,
            0,
            vec![
                IntPoint::new(20_000, 300_000),
                IntPoint::new(96_000, 300_000),
                IntPoint::new(96_000, 304_000),
                IntPoint::new(100_000, 304_000),
            ],
        );
        ses.append_padstack(epic_dsn::sink::PadstackIr {
            name: "VIA".to_string(),

            shapes: vec![
                Some(epic_dsn::shape::BoardShape::Tile(
                    epic_geometry::tile_shape::TileShape::RegularTileShape(
                        epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(98_500, 302_500),
                            IntPoint::new(101_500, 305_500),
                        )),
                    ),
                )),
                Some(epic_dsn::shape::BoardShape::Tile(
                    epic_geometry::tile_shape::TileShape::RegularTileShape(
                        epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(98_500, 302_500),
                            IntPoint::new(101_500, 305_500),
                        )),
                    ),
                )),
            ],
            drillable: true,
            placed_absolute: false,
        });
        ses.insert_via(epic_dsn::sink::ViaIr {
            padstack_no: 1,
            location: IntPoint::new(100_000, 304_000),
            nets: vec![3],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
            attach_smd_allowed: false,
        });
        trace_on(
            &mut ses,
            504,
            3,
            1,
            vec![
                IntPoint::new(100_000, 304_000),
                IntPoint::new(200_000, 304_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// The flow world's expected F1 rows (the single factored truth
    /// for F1 and F10).
    fn expected_flow_rows() -> Vec<FlowCandidateRow> {
        vec![
            FlowCandidateRow {
                net: 1,
                net_name: "flow_a".to_string(),
                kind: FlowKind::Jog,
                corner_x: 24_000,
                corner_y: 200_000,
                landed: true,
            },
            FlowCandidateRow {
                net: 1,
                net_name: "flow_a".to_string(),
                // The SECOND staircase jog is a TAIL jog (the staircase
                // descends to the end anchor, so its S3 is the trace's
                // final segment) — the honest kind label is Stub, not
                // Jog.
                kind: FlowKind::Stub,
                corner_x: 28_000,
                corner_y: 204_000,
                landed: true,
            },
            FlowCandidateRow {
                net: 2,
                net_name: "flow_b".to_string(),
                kind: FlowKind::Miter,
                corner_x: 200_000,
                corner_y: 100_000,
                landed: true,
            },
            FlowCandidateRow {
                net: 3,
                net_name: "flow_c".to_string(),
                kind: FlowKind::Stub,
                corner_x: 96_000,
                corner_y: 300_000,
                landed: true,
            },
        ]
    }

    /// **F1 (the flow world, EXACT expected geometry)** — all three
    /// kinds land in one stage run: net 1's double jog (4 bends → 2),
    /// net 2's miter bridge (2x90 -> 2x45), net 3's stub drop (the END
    /// corner at the via is verbatim). DRC-clean on the REAL counter.
    #[test]
    fn f1_flow_world_exact_geometry() {
        let (mut manager, mut board) = flow_world();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert_eq!(report.rows, expected_flow_rows(), "the row set");
        assert!(!report.gated);
        // The EXACT final geometry, net by net.
        let p = |x: i32, y: i32| Point::Int(IntPoint::new(x, y));
        assert_eq!(
            net_traces(&board, 1),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(20_000, 200_000),
                    p(24_000, 204_000),
                    p(28_000, 200_000),
                    p(480_000, 200_000),
                ])
            )],
            "net 1: both jogs dropped, 4 corners -> 2 bends"
        );
        assert_eq!(
            net_traces(&board, 2),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(20_000, 100_000),
                    p(36_000, 116_000),
                    p(420_000, 116_000),
                ])
            )],
            "net 2: the 90-degree pair became the 45-degree miter pair"
        );
        assert_eq!(net_traces(&board, 3).len(), 2, "net 3 keeps both wires");
        let traces3 = net_traces(&board, 3);
        let stub = traces3
            .iter()
            .find(|t| t.contains("300000"))
            .expect("the F.Cu stub wire");
        assert!(
            stub.contains("96000") && stub.contains("100000") && stub.contains("304000"),
            "the F.Cu stub wire was rewritten to the 45-degree drop: {stub}"
        );
        // The END corner (the via face) never moved: the B.Cu wire is
        // untouched (it carries the via face (100_000, 304_000)).
        let completion = traces3
            .iter()
            .find(|t| !t.contains("300000"))
            .expect("the B.Cu completion wire");
        assert!(
            completion.contains("100000") && completion.contains("200000"),
            "the B.Cu completion wire is verbatim: {completion}"
        );
        let (violations, _) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "the post-flow board is DRC-clean");
    }

    /// **F2 (the blocked world, the honest-stop face)** — a keepout on
    /// the NEW miter corridor (x [100_000, 110_000], y [114_000,
    /// 118_000]: inside net 2's would-be bridge extension, clear of
    /// every CURRENT trace's clearance zone) blocks net 2's bridge:
    /// the honest rejected row, geometry untouched — while nets 1 and
    /// 3 still land (not all-or-nothing).
    #[test]
    fn f2_blocked_world_honest_row() {
        let mut ses = flow_base_ses_with_members();
        ses.items.push(ItemIr::Keepout {
            id: 600,
            keepout: epic_dsn::sink::KeepoutIr {
                kind: epic_dsn::sink::KeepoutKindIr::Keepout,
                layer_no: 0,
                area: epic_dsn::sink::AreaIr::simple(epic_dsn::shape::BoardShape::Tile(
                    epic_geometry::tile_shape::TileShape::RegularTileShape(
                        epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(100_000, 114_000),
                            IntPoint::new(110_000, 118_000),
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
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        // Net 2's miter is the honest stop; nets 1 and 3 still land.
        let net2_rows: Vec<&FlowCandidateRow> =
            report.rows.iter().filter(|row| row.net == 2).collect();
        assert_eq!(net2_rows.len(), 1, "net 2 has exactly one row");
        assert_eq!(net2_rows[0].kind, FlowKind::Miter);
        assert!(!net2_rows[0].landed, "net 2's bridge is the honest stop");
        assert_eq!(
            report.rows.iter().filter(|row| row.landed).count(),
            3,
            "nets 1 and 3 still land (3 landed rows)"
        );
        // Net 2's geometry is untouched (the Z stands).
        let traces = net_traces(&board, 2);
        assert!(
            traces[0].contains("200000") && traces[0].contains("116000"),
            "net 2's Z corner is verbatim: {traces:?}"
        );
        let (violations, _) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(violations, 0, "the blocked world stays DRC-clean");
    }

    /// The flow world's members without the via apparatus (the
    /// blocked-world builder base — keepouts are appended to a fresh
    /// SesBoard).
    fn flow_base_ses_with_members() -> SesBoard {
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        straight_trace(
            &mut ses,
            500,
            1,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(24_000, 200_000),
                IntPoint::new(24_000, 204_000),
                IntPoint::new(28_000, 204_000),
                IntPoint::new(28_000, 200_000),
                IntPoint::new(480_000, 200_000),
            ],
        );
        straight_trace(
            &mut ses,
            501,
            2,
            vec![
                IntPoint::new(20_000, 100_000),
                IntPoint::new(200_000, 100_000),
                IntPoint::new(200_000, 116_000),
                IntPoint::new(420_000, 116_000),
            ],
        );
        // net 3 WITHOUT the via: the tail triple is geometrically the
        // same candidate (the pass reads geometry, not topology).
        trace_on(
            &mut ses,
            502,
            3,
            0,
            vec![
                IntPoint::new(20_000, 300_000),
                IntPoint::new(96_000, 300_000),
                IntPoint::new(96_000, 304_000),
                IntPoint::new(100_000, 304_000),
            ],
        );
        ses
    }

    /// **F4 (the jog/miter threshold boundary, DNR-16, BOTH
    /// directions)** — stub 11_999 resolves as a JOG (factor edge in),
    /// stub 12_001 resolves as a MITER (factor edge out; the miter
    /// constant 30_000 is untouched at both stubs — the geometry is
    /// otherwise identical). A JOG_MAX_SEGMENT_FACTOR mutant of either
    /// sign flips exactly one verdict; the kind label is the kill
    /// face. The world geometry is LITERAL (bug-212: no reference to
    /// the factor).
    #[test]
    fn f4_jog_miter_threshold_boundary_worlds() {
        // 11_999 (in): the exact drop fires, kind = Jog. The trailing
        // corner (200_000, 250_000) gives S3 a continuation — a
        // window's S3 being the trace's LAST segment would make the
        // kind Stub (the F4 worlds pin the JOG kind, not the stub
        // kind).
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        trace_on(
            &mut ses,
            500,
            1,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(31_999, 200_000),
                IntPoint::new(31_999, 211_999),
                IntPoint::new(200_000, 211_999),
                IntPoint::new(200_000, 250_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].kind, FlowKind::Jog, "11_999 is a jog");
        assert!(report.rows[0].landed);
        let p = |x: i32, y: i32| Point::Int(IntPoint::new(x, y));
        assert_eq!(
            net_traces(&board, 1),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(20_000, 200_000),
                    p(31_999, 211_999),
                    p(200_000, 211_999),
                    p(200_000, 250_000),
                ])
            )]
        );
        // 12_001 (out of the jog, inside the miter): the bridge fires,
        // kind = Miter, and q lands EXACTLY on the old corner's
        // position (the stub == flank-length edge of the bridge). The
        // trailing corner gives S3 its continuation (kind = Jog, not
        // Stub).
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        trace_on(
            &mut ses,
            500,
            1,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(32_001, 200_000),
                IntPoint::new(32_001, 212_001),
                IntPoint::new(200_000, 212_001),
                IntPoint::new(200_000, 250_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].kind, FlowKind::Miter, "12_001 is a miter");
        assert!(report.rows[0].landed);
        assert_eq!(
            net_traces(&board, 1),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(20_000, 200_000),
                    p(32_001, 212_001),
                    p(200_000, 212_001),
                    p(200_000, 250_000),
                ])
            )]
        );
    }

    /// **F5 (the miter threshold boundary, DNR-16, BOTH directions)** —
    /// stub 29_999 bridges (kind = Miter), the EXACT-EDGE stub 30_000
    /// produces NO row (strict `<`: a 30_001-threshold mutant fires
    /// here and flips the verdict — the exact-edge world is the one
    /// that kills the +1 direction) and the geometry is untouched. A
    /// MITER_MAX_STUB_DBU mutant of either sign flips exactly one
    /// verdict.
    #[test]
    fn f5_miter_threshold_boundary_worlds() {
        // 29_999 (in): the bridge fires.
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        trace_on(
            &mut ses,
            500,
            1,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(50_000, 200_000),
                IntPoint::new(50_000, 229_999),
                IntPoint::new(200_000, 229_999),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].kind, FlowKind::Miter);
        assert!(report.rows[0].landed);
        let p = |x: i32, y: i32| Point::Int(IntPoint::new(x, y));
        assert_eq!(
            net_traces(&board, 1),
            vec![format!(
                "{:?}",
                Polyline::from_points(&[
                    p(20_000, 200_000),
                    p(49_999, 229_999),
                    p(200_000, 229_999),
                ])
            )]
        );
        // 30_000 (the exact edge, strict <: declines).
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        trace_on(
            &mut ses,
            500,
            1,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(50_000, 200_000),
                IntPoint::new(50_000, 230_000),
                IntPoint::new(200_000, 230_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let before = net_traces(&board, 1).clone();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(
            report.rows.is_empty(),
            "the exact-edge stub is not a candidate: no row — {:?}",
            report.rows
        );
        assert_eq!(net_traces(&board, 1), before, "geometry untouched");
    }

    /// **F7 (the no-op invariance world)** — flag ON, no eligible
    /// sites: a straight trace and a wide Z (stub 100_000 clears BOTH
    /// thresholds): zero rows, geometry byte-identical, no gated mark.
    #[test]
    fn f7_noop_world_no_rows_no_change() {
        let mut ses = flow_base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        trace_on(
            &mut ses,
            500,
            1,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(400_000, 200_000),
            ],
        );
        trace_on(
            &mut ses,
            501,
            2,
            0,
            vec![
                IntPoint::new(20_000, 100_000),
                IntPoint::new(120_000, 100_000),
                IntPoint::new(120_000, 200_000),
                IntPoint::new(300_000, 200_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let before_1 = net_traces(&board, 1).clone();
        let before_2 = net_traces(&board, 2).clone();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(report.rows.is_empty(), "no candidates: {report:?}");
        assert!(!report.gated);
        assert_eq!(net_traces(&board, 1), before_1);
        assert_eq!(net_traces(&board, 2), before_2);
    }

    /// **F8 (the angle gate)** — the SAME staircase world on a
    /// NINETY_DEGREE board (the T3 base): the stage is a documented
    /// no-op (it manufactures 45° geometry), zero rows.
    #[test]
    fn f8_angle_gate_ninety_board_is_noop() {
        let mut ses = base_ses();
        register_nets(
            &mut ses,
            &[("flow_a", false), ("flow_b", false), ("flow_c", false)],
        );
        straight_trace(
            &mut ses,
            500,
            1,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(24_000, 200_000),
                IntPoint::new(24_000, 204_000),
                IntPoint::new(200_000, 204_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let before = net_traces(&board, 1).clone();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(report.rows.is_empty(), "the 90-degree board never flows");
        assert!(!report.gated);
        assert_eq!(net_traces(&board, 1), before, "geometry untouched");
    }

    /// **F9 (the gated marker)** — a FORTYFIVE_DEGREE board with an
    /// incomplete net (the routed staircase + an open net): the stage
    /// short-circuits with `gated == true`, ZERO rows — the honest
    /// hold, surfaced by epic-cli as the DISTINCT `gloss_flow_gated`
    /// sidecar key.
    #[test]
    fn f9_gated_world_marks_the_hold() {
        let (mut manager, mut board) = parse(FLOW_GATED_WORLD);
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(report.gated, "the hold is marked");
        assert!(
            report.rows.is_empty(),
            "the gate short-circuits before the scan"
        );
        assert!(!GlossFlowReport::default().gated);
    }

    /// The F9 world: the flow staircase + an open net (two pins, no
    /// wire) on a FORTYFIVE_DEGREE board. The gate fires before any
    /// geometry work — load-time tightening of the crafted corners is
    /// irrelevant here.
    const FLOW_GATED_WORLD: &str = r#"
(pcb flow_gated_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 500000 300000))
    (snap_angle fortyfive_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 2000 20000 Front 0.000000))
    (component "IA" (place "A2" 48000 20000 Front 0.000000))
    (component "IO" (place "O1" 2000 24000 Front 0.000000))
    (component "IO" (place "O2" 50000 24000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (image "IO" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 500 0 0))
      (attach off)
    )
    (padstack "VIA"
      (shape (circle F.Cu 100 0 0))
      (shape (circle B.Cu 100 0 0))
      (attach off)
    )
  )
  (network
    (via V VIA kicad_default)
    (net "flow_a" (pins "A1"-"P" "A2"-"P"))
    (net "open" (pins "O1"-"P" "O2"-"P"))
    (class kicad_default "flow_a" "open" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200
      2000 20000
      2400 20000
      2400 20400
      20000 20400
    ) (net "flow_a"))
  )
)
"#;

    /// **F10 (determinism x2)** — two fresh flow worlds through the
    /// stage: the reports and every net's final geometry agree exactly.
    #[test]
    fn f10_determinism_two_fresh_runs() {
        let run = || -> (GlossFlowReport, Vec<String>) {
            let (mut manager, mut board) = flow_world();
            let report = run_gloss_flow_stage(&mut manager, &mut board);
            let geom: Vec<String> = (1..=3).flat_map(|net| net_traces(&board, net)).collect();
            (report, geom)
        };
        let (report_a, geom_a) = run();
        let (report_b, geom_b) = run();
        assert_eq!(report_a, report_b);
        assert_eq!(geom_a, geom_b);
    }

    /// **F11 (the pure predicate faces)** — flow_angle_ok (axis and
    /// the exact-45° faces, and the rejects), the flow length-guard
    /// exact edge (old + GAIN lands / +1 rejects, DNR-16 both
    /// directions — literal arithmetic, no constant reference), the
    /// jog threshold strict edge at the documented factor (12_000
    /// declines, 11_999 passes), and the bend monotonicity guard.
    #[test]
    fn f11_pure_predicate_faces() {
        // angle: axis faces, the exact 45 faces, the rejects.
        assert!(flow_angle_ok(0, 5));
        assert!(flow_angle_ok(5, 0));
        assert!(flow_angle_ok(7, 7));
        assert!(flow_angle_ok(-7, 7));
        assert!(!flow_angle_ok(7, 6));
        assert!(
            flow_angle_ok(0, 0),
            "the degenerate segment is excluded UPSTREAM (stub > 0); \
             the predicate itself only classifies direction"
        );
        // flow length guard: the GAIN bound is 40_000 (old 540_000).
        assert!(flow_length_guard_ok(540_000.0, 580_000.0), "equality lands");
        assert!(!flow_length_guard_ok(540_000.0, 580_001.0), "+1 rejects");
        // jog threshold: strict < (stub == threshold declines).
        assert!(!jog_threshold_ok(12_000, 2_000), "equality declines");
        assert!(jog_threshold_ok(11_999, 2_000), "-1 passes");
        // bend monotonicity: never more, fewer fine, equal fine.
        assert!(bend_monotonic_ok(4, 2));
        assert!(bend_monotonic_ok(4, 4));
        assert!(!bend_monotonic_ok(4, 5));
    }

    /// **F12 (the fixed-state gate)** — net 1's staircase promoted to
    /// USER_FIXED: no net-1 rows (the window gate is silent — not a
    /// candidate), geometry untouched; nets 2 and 3 still land.
    #[test]
    fn f12_fixed_member_does_not_flow() {
        let mut ses = flow_base_ses_with_members();
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
        let before = net_traces(&board, 1).clone();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(
            report.rows.iter().all(|row| row.net != 1),
            "the fixed trace has no rows: {report:?}"
        );
        assert_eq!(
            net_traces(&board, 1),
            before,
            "the fixed trace is untouched"
        );
        assert_eq!(
            report.rows.iter().filter(|row| row.landed).count(),
            2,
            "nets 2 and 3 still land"
        );
    }

    /// **F13 (the plane-net exclusion, quality-r1 MINOR)** — a
    /// `contains_plane` net carrying a TEXTBOOK staircase (the exact
    /// geometry that lands in F1's net 1): ZERO rows and unchanged
    /// final geometry. The bus stage documents AND pins the identical
    /// filter (D5, the `plane_net2` world helper); before this pin the
    /// F-series passed with the filter deleted. Mirrors the D5
    /// `two_net_world` plane-flag scaffolding.
    #[test]
    fn f13_plane_net_is_never_a_flow_target() {
        let mut ses = flow_base_ses();
        register_nets(&mut ses, &[("flow_a", false), ("plane_net", true)]);
        // The plane net's staircase: F1 net 1's exact geometry.
        trace_on(
            &mut ses,
            500,
            2,
            0,
            vec![
                IntPoint::new(20_000, 200_000),
                IntPoint::new(24_000, 200_000),
                IntPoint::new(24_000, 204_000),
                IntPoint::new(28_000, 204_000),
                IntPoint::new(28_000, 200_000),
                IntPoint::new(480_000, 200_000),
            ],
        );
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let before = net_traces(&board, 2).clone();
        let report = run_gloss_flow_stage(&mut manager, &mut board);
        assert!(
            report.rows.is_empty(),
            "a plane net is never a flow target: {report:?}"
        );
        assert!(!report.gated);
        assert_eq!(net_traces(&board, 2), before, "geometry untouched");
    }
}
