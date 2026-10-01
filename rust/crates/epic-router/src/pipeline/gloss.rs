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
//! version-blind manifest canary pins manifest bytes; the raw
//! cf607714… retired at M10-T5).
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
//! rendered by epic-cli — NEVER the manifest: the version-blind manifest canary pins manifest bytes; the raw
//! cf607714… retired at M10-T5).
//!
//! ===========================================================================
//! M8-T5: the VIA PLACE pass — return-path-aware via placement
//! ===========================================================================
//!
//! **The ViaOptimizer AUDIT (recorded decision — the AMENDMENT 4
//! requirement).** `epic_board::trace_tightener::via_optimizer` is the
//! ported Java `ViaOptimizer`: a per-via relocation optimizer that
//! minimizes the WEIGHTED LAYER-TRACE COST of a via's contact geometry
//! (`opt_via_location`, recursion depth 10, `DrillItemMover` insertion
//! budgets 9/9, the four `check_trace_segment_points` POINTS sites),
//! driven by the changed-area fixpoint at ROUTE time
//! (`TraceTightener.java:160-165`). Its plane arm is the M6
//! CONTAINMENT gate (`opt_plane_or_fanout_via` answers false at the
//! containment point — a legality face, not a score). It has NO
//! return-path objective, NO alignment objective, and NO whole-board
//! post-route sweep; it is a frozen parity surface whose outputs the
//! byte gates pin. THE OVERLAP FACE: both mechanisms relocate an
//! existing via within clearance legality, and a cost-arm landing can
//! coincidentally look like an alignment move. THIS PASS IS DISTINCT
//! on all three axes the audit names: (a) the RETURN-PATH objective —
//! for a via whose net is a plane net, candidates rank by how much
//! the stitch distance to the net's OWN conduction region DROPS (the
//! M6 pour machinery: `rules_surf` plane lookups + the
//! `ConductionArea` faces; ViaOptimizer's plane arm scores nothing);
//! (b) the ALIGNMENT objective — a candidate is accepted ONLY when
//! BOTH adjoining trace arms re-corner to SINGLE angle-legal segments
//! into the new center (the via-on-the-bend elimination face), ranked
//! by shorter total arm length (ViaOptimizer MINIMIZES TRACE COST
//! AROUND the contact geometry, it never requires straightness); (c)
//! the REGIME — a whole-board POST-route gloss sweep behind
//! `router.gloss.via_place` (default OFF, honest per-via rows),
//! whereas ViaOptimizer runs inside the route-time tightener fixpoint.
//! A pass that merely re-implemented the cost arm would fail this
//! spec; none of the cost arm is ported here.
//!
//! **The pass.** For each via — deterministic order: net number
//! ascending, item id ascending within the net — with EXACTLY TWO
//! trace contacts whose polylines END at the via center (the
//! `drill_normal_contacts` endpoint-equality face; plane contacts of
//! the via's OWN net do not disqualify — a via sitting on its own
//! pour is the pass's PRIMARY subject). Candidate relocations = the
//! 45°-LATTICE points within [`VIA_PLACE_RADIUS_DBU`] of the current
//! center — displacements that are multiples of [`VIA_PLACE_STEP_DBU`]
//! on both axes, angle-legal under the board's own restriction, in an
//! inclusive Chebyshev radius — PLUS the ALIGNMENT-DERIVED points:
//! the intersections of the two arms' 45°-legal direction lines
//! through their far anchors (the exact in-line positions; arbitrary
//! DBU coordinates). Deduplicated; enumerated in the documented order:
//! Chebyshev displacement ascending, then dx, then dy (both families
//! merged). Score = (a) return-path continuity — the net's plane
//! stitch-distance drop (neutral 0 for non-plane nets and plane nets
//! with no in-span conduction area); (b) alignment — shorter total
//! re-cornered arm length. Rank: score (a) DESC, (b) ASC, then the
//! enumeration order. Probe-and-land-FIRST-FIT in rank order: the
//! first candidate passing ALL of the following lands, nothing
//! partial: (1) the fixed-state gate (USER/SYSTEM_FIXED via or arm
//! never moves); (2) both arms angle-legal as single segments; (3)
//! the net's trace-length budget — the net's total trace length may
//! grow by at most [`VIA_PLACE_MAX_LENGTH_GAIN_DBU`] (equality
//! allowed; the T3/T4 length-budget mirror); (4) the REAL via probe —
//! `drill_item_mover::check` at budgets 0/0, the same
//! `DrillItemMover.check(via, delta, 0, 0)` legality face ViaOptimizer
//! itself uses as its pre-check (the via's shape at the candidate; NO
//! shoving at 0/0; same-net copper is the mover's own-net face) — the
//! audited existing REAL via-at-a-point check, NOT a new probe; (5)
//! the REAL arm probes — every new arm segment at full length via
//! `check_trace_segment_points` (exact corners; same-net exempt by
//! the tree query, the T3/T4 face). PLUS the IMPROVEMENT gate: a
//! candidate must IMPROVE the via — a strictly positive return-path
//! drop, OR a strictly smaller total arm length, OR a strictly
//! smaller total corner count (a via already in-line with no pour
//! does not move; the nearest acceptable candidate alone never moves
//! a via). The sweep is an ORDER-DEPENDENT SINGLE-PASS improver, not
//! a fixed point: a landing never re-examines an earlier-attempted
//! via; each attempt simply re-reads its arms from the current
//! polylines at attempt time (the safety face — pinned by V11's
//! no-restart world). Landing writes
//! `Board::set_via_center` (board.rs:1068 — the RAW center write, the
//! caller owns the acceptance face) inside the tree remove /
//! `clear_derived_data` / re-insert sequence (the `drill_move_by`
//! geometry-write face, minus the undo save and the connector-trace
//! insertion — the gloss family never touches undo and the arms are
//! rewritten wholesale below), then each arm through
//! `replace_trace_geometry`. Via COUNT never increases — the pass
//! only relocates; the pin asserts count invariance across the stage.
//! Every attempted via records its honest row (`landed:false` when no
//! candidate passed); vias outside the exactly-two-arm scope record
//! NO row (the T3 no-candidate face).
//!
//! **Stage gates (in order).** (1) the incompletes gate (the T3/T4
//! precedent): some net still has incompletes → the stage
//! short-circuits with `gated = true`, surfaced as the DISTINCT
//! `gloss_via_place_gated` sidecar key (from day one — the T3
//! fix-round lesson). (2) The ROUTED-BOARD reality: plane nets ARE in
//! scope — a DELIBERATE divergence from the T3/T4 routed-net filter
//! (`!net.contains_plane`), because the return-path arm is ABOUT
//! plane-net vias; the Issue 093/152 precaution covers plane ROUTING
//! mode (stub+via insertion into a live plane), and this pass neither
//! routes nor modifies any pour — it only relocates vias over the
//! STATIC parsed copper. No angle gate: the pass adapts to the
//! board's own restriction (Ninety/Fortyfive lattice + alignment
//! families; a None-restriction board gets the full step lattice and
//! no alignment family — the in-line objective needs directions).
//!
//! **Slot decision (recorded, the charter's order question).** The
//! pass runs in the gloss stage slot AFTER flow. A via relocation
//! CREATES flow candidates in principle (new arm corners), but the
//! pass's arms land as SINGLE segments — zero new corners, nothing
//! for a later flow pass to clean — so via-then-flow composes
//! identically to flow-then-via on the via side. The reverse order
//! loses: the pass REPLACES both arm polylines WHOLESALE, so any flow
//! landing INSIDE an arm of an attempted via is discarded work, and
//! the flow rows would diverge from the flow-only ablation face
//! (attribution stays clean only if the earlier pass's geometry is
//! final before the later pass reads it). Terminal slot wins.
//!
//! **Two-regime safety (via_place).** The ONLY caller gate is the
//! resolved `router.gloss.via_place` flag
//! (`BatchSettings::via_place_active`, default false); an OFF run
//! never constructs a report and never touches the board. The report
//! rides the `--dump-aesthetics` SIDECAR ONLY (the `gloss_via_place`
//! block rendered by epic-cli — NEVER the manifest: the version-blind manifest canary pins manifest bytes; the raw
//! cf607714… retired at M10-T5).

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

// ===========================================================================
// M8-T5: the VIA PLACE pass — return-path-aware via placement
// ===========================================================================

/// The candidate RADIUS, in board DBU: a relocation is enumerated only
/// when its CHEBYSHEV displacement from the current center
/// (`max(|dx|, |dy|)`) is at most this (INCLUSIVE — the exact edge is
/// pinned: a candidate at exactly the radius is enumerated, one at
/// radius+1 is not; the boundary worlds and the both-direction
/// mutations pin it at [`via_radius_allows`], DNR-16). Value:
/// 60_000 DBU = 6 mm at the um-10 transform — a via should reach a
/// better return-path or in-line slot within a component-pitch
/// neighborhood, not across the board.
pub const VIA_PLACE_RADIUS_DBU: i64 = 60_000;

/// The lattice STEP, in board DBU: the 45°-lattice family enumerates
/// displacements that are integer multiples of this on both axes
/// (angle-legal ones). Granularity, not a boundary predicate — the
/// enumeration-order pin ([`via_lattice_candidates`]) pins the step's
/// effect on the candidate set. Value: 10_000 DBU = 1 mm — a
/// via-pad-scale move quantum.
pub const VIA_PLACE_STEP_DBU: i64 = 10_000;

/// The candidate LENGTH-GAIN budget, in board DBU: a relocation whose
/// NET's total trace length exceeds the net's current total by more
/// than this is rejected (equality allowed). Value: 40_000 DBU = 4 mm,
/// the T3 [`BUS_MAX_LENGTH_GAIN_DBU`]/T4 [`FLOW_MAX_LENGTH_GAIN_DBU`]
/// mirror. The exact edge is pinned at [`via_length_guard_ok`]
/// (old + GAIN lands, old + GAIN + 1 rejects) with both-direction
/// mutations (DNR-16).
pub const VIA_PLACE_MAX_LENGTH_GAIN_DBU: i64 = 40_000;

/// One via relocation's row (the honest-stop face): the via locator
/// is the item id, `from`/`to` the current/landed center.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaPlaceRow {
    pub via_id: u32,
    pub net: i32,
    pub net_name: String,
    /// The center at stage entry, board DBU.
    pub from_x: i64,
    /// The center at stage entry, board DBU.
    pub from_y: i64,
    /// The landed center (`from` when no candidate landed), board DBU.
    pub to_x: i64,
    /// The landed center (`from` when no candidate landed), board DBU.
    pub to_y: i64,
    /// Whether a candidate LANDED (the geometry was written).
    pub landed: bool,
}

/// The gloss VIA PLACE stage report (the sidecar's `gloss_via_place`
/// block source). Default = empty = the OFF face (never serialized).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlossViaPlaceReport {
    /// Per-attempted-via rows in sweep order (net asc, via id asc).
    pub rows: Vec<ViaPlaceRow>,
    /// The INCOMPLETES-GATE marker (the T3 `bus_groups_gated` lesson —
    /// the distinct sibling key exists from day one): true when the
    /// stage short-circuited because some net still has incompletes.
    /// Default false = the OFF/default semantics unchanged.
    pub gated: bool,
}

impl GlossViaPlaceReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The displacement ADMITTER, pinnable in isolation: `dx`/`dy` nonzero,
/// angle-legal under the board's restriction (None: any displacement;
/// NinetyDegree: axis-aligned only; FortyfiveDegree: axis or exactly
/// 45° — the [`flow_angle_ok`] face).
#[must_use]
pub fn via_displacement_ok(
    restriction: epic_board::rules_surf::AngleRestriction,
    dx: i64,
    dy: i64,
) -> bool {
    if dx == 0 && dy == 0 {
        return false;
    }
    match restriction {
        epic_board::rules_surf::AngleRestriction::None => true,
        epic_board::rules_surf::AngleRestriction::NinetyDegree => dx == 0 || dy == 0,
        epic_board::rules_surf::AngleRestriction::FortyfiveDegree => flow_angle_ok(dx, dy),
    }
}

/// The RADIUS admitter, pinnable in isolation (the DNR-16 exact edge:
/// displacement at exactly the radius is admitted, radius+1 is not —
/// both-direction mutations verified).
#[must_use]
pub fn via_radius_allows(dx: i64, dy: i64) -> bool {
    dx.abs().max(dy.abs()) <= VIA_PLACE_RADIUS_DBU
}

/// The via LENGTH-GUARD predicate, pinnable in isolation (the DNR-16
/// exact edge: old + GAIN lands, old + GAIN + 1 rejects —
/// both-direction mutations verified).
#[must_use]
pub fn via_length_guard_ok(old_len: f64, new_len: f64) -> bool {
    new_len <= old_len + VIA_PLACE_MAX_LENGTH_GAIN_DBU as f64
}

/// The 45°-LATTICE candidate family: displacements that are integer
/// multiples of [`VIA_PLACE_STEP_DBU`] on both axes, angle-legal under
/// `restriction`, nonzero, within [`VIA_PLACE_RADIUS_DBU`] (Chebyshev,
/// inclusive). The enumeration order of the RETURNED list is
/// documented and deterministic: Chebyshev displacement ascending,
/// then dx ascending, then dy ascending (the merged candidate order —
/// pinned).
#[must_use]
pub fn via_lattice_candidates(
    restriction: epic_board::rules_surf::AngleRestriction,
) -> Vec<(i64, i64)> {
    let max_k = VIA_PLACE_RADIUS_DBU / VIA_PLACE_STEP_DBU;
    let mut out = Vec::new();
    for da in -max_k..=max_k {
        for db in -max_k..=max_k {
            let dx = da * VIA_PLACE_STEP_DBU;
            let dy = db * VIA_PLACE_STEP_DBU;
            if !via_radius_allows(dx, dy) || !via_displacement_ok(restriction, dx, dy) {
                continue;
            }
            out.push((dx, dy));
        }
    }
    out.sort_by_key(|&(dx, dy)| (dx.abs().max(dy.abs()), dx, dy));
    out
}

/// One arm of an attempted via: the contact trace, its far anchor (the
/// endpoint NOT at the via center), and the per-probe parameters.
struct ViaArm {
    trace_id: ItemId,
    far: IntPoint,
    layer: i32,
    half_width: i32,
    clearance_class: i32,
    current_length: f64,
    corner_count: usize,
}

/// One scored candidate: the displacement, the new center, and the
/// ranking key (return-path drop DESC, total arm length ASC, then the
/// enumeration order).
struct ViaCandidate {
    dx: i64,
    dy: i64,
    new_center: IntPoint,
    ret_drop: f64,
    arm_total: f64,
}

/// The STITCH DISTANCE from a point to one conduction area: 0 inside
/// (border points count as contained — the `Area::contains_point`
/// face), else the distance to the border shape (Tile/Circle exact;
/// PolygonShape falls back to its bounding-box tile distance — the
/// documented approximation, the parse census carries no polygon
/// pours).
fn stitch_distance(area: &epic_board::items::Area, point: &IntPoint) -> f64 {
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;
    if area.contains_point(&Point::Int(*point)) {
        return 0.0;
    }
    let float = point.to_float();
    match &area.border {
        epic_board::items::BoardShape::Tile(tile) => tile.distance(&float),
        epic_board::items::BoardShape::Circle(circle) => circle.distance(&float),
        epic_board::items::BoardShape::PolygonShape(polygon) => {
            TileShape::RegularTileShape(RegularTileShape::IntBox(polygon.bounding_box()))
                .distance(&float)
        }
    }
}

/// The via's PLANE STITCH distances: `Some` only when the via's net is
/// a plane net carrying at least one conduction area within the via's
/// padstack layer span. The value at a point = the MINIMUM stitch
/// distance over those areas.
fn plane_stitch(board: &mut Board, via_id: ItemId, net: i32, point: &IntPoint) -> Option<f64> {
    let rules = board.rules();
    if !rules.nets.get(net)?.contains_plane {
        return None;
    }
    let first = board.drill_first_layer(via_id)?;
    let last = board.drill_last_layer(via_id)?;
    let mut best: Option<f64> = None;
    for id in board.get_connectable_items(net) {
        let Some(area) = board.conduction_area(id) else {
            continue;
        };
        let Some(layer) = board.area_layer(id) else {
            continue;
        };
        if layer < first || layer > last {
            continue;
        }
        let distance = stitch_distance(&area, point);
        best = Some(best.map_or(distance, |current: f64| current.min(distance)));
    }
    best
}

/// The stage entry (the pipeline's post-flow slot — the terminal gloss
/// pass, the recorded slot decision; the caller gates on
/// `BatchSettings::via_place_active` — the flag is the only caller
/// gate). Emits NO events and NO stage transition (the meander-stage
/// precedent). An empty report = no attempted via, nothing moved.
pub fn run_gloss_via_place_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> GlossViaPlaceReport {
    // The incompletes gate (the T3/T4 precedent): a net with
    // incompletes is not a gloss target (still being routed).
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    if incompletes.iter().any(|row| row.incomplete_count > 0) {
        return GlossViaPlaceReport {
            gated: true,
            ..Default::default()
        };
    }
    let mut report = GlossViaPlaceReport::default();
    // The sweep order: net number ascending, via item id ascending
    // within the net (`get_connectable_items` returns descending ids —
    // sorted; the seen-set keeps a multi-net via single-visited).
    let max_net = board.rules().nets.max_net_number();
    let mut seen: std::collections::BTreeSet<ItemId> = std::collections::BTreeSet::new();
    for net in 1..=max_net {
        let mut via_ids: Vec<ItemId> = board
            .get_connectable_items(net)
            .into_iter()
            .filter(|&id| {
                matches!(
                    board.get(id).map(|entry| &entry.data),
                    Some(epic_board::items::ItemData::Via { .. })
                ) && seen.insert(id)
            })
            .collect();
        via_ids.sort();
        for via_id in via_ids {
            via_place_attempt(manager, board, net, via_id, &mut report.rows);
        }
    }
    report
}

/// One via's attempt: the candidate enumeration, the rank, the
/// probe-and-land-first-fit sweep, and the honest row.
fn via_place_attempt(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net: i32,
    via_id: ItemId,
    rows: &mut Vec<ViaPlaceRow>,
) {
    let Some(center) = board.drill_center(via_id) else {
        return;
    };
    let center = match center {
        Point::Int(point) => point,
        Point::Rational(_) => return,
    };
    // The fixed-state gate on the VIA (silent — no row, the no-candidate
    // face; the arms carry their own gate at acceptance).
    if board
        .get(via_id)
        .is_some_and(|entry| matches!(entry.fixed, FixedState::UserFixed | FixedState::SystemFixed))
    {
        return;
    }
    // The exactly-two-arm scope: trace contacts whose polylines END at
    // the center. Plane contacts of the via's OWN net do not
    // disqualify (a via on its own pour is the pass's subject).
    let contact_traces: Vec<ItemId> =
        epic_board::contacts::drill_normal_contacts(manager, board, via_id)
            .into_iter()
            .filter(|&id| {
                matches!(
                    board.get(id).map(|entry| &entry.data),
                    Some(epic_board::items::ItemData::Trace { .. })
                )
            })
            .collect();
    if contact_traces.len() != 2 {
        return;
    }
    let mut arms = Vec::with_capacity(2);
    for trace_id in contact_traces {
        let Some(lines) = board.trace_polyline(trace_id) else {
            return;
        };
        let Some(layer) = board.trace_layer(trace_id) else {
            return;
        };
        let Some(half_width) = board.trace_half_width(trace_id) else {
            return;
        };
        let Some(clearance_class) = board.item_clearance_class(trace_id) else {
            return;
        };
        let corner_count = lines.corner_count();
        if corner_count < 2 {
            return;
        }
        let Some(first) = lines.corner(0) else {
            return;
        };
        let Some(last) = lines.corner(i32::try_from(lines.corner_count() - 1).unwrap_or(0)) else {
            return;
        };
        let (Some(first), Some(last)) = (as_int(&first), as_int(&last)) else {
            return;
        };
        let far = if first == center {
            last
        } else if last == center {
            first
        } else {
            return; // not endpoint-connected: outside the scope
        };
        if far == center {
            return; // degenerate closed trace: outside the scope
        }
        arms.push(ViaArm {
            trace_id,
            far,
            layer,
            half_width,
            clearance_class,
            current_length: piece_length_sum(&lines.corners()),
            corner_count,
        });
    }
    // The net's CURRENT total trace length (the budget's base) and the
    // plane-stitch distance at the CURRENT center.
    let net_len_before = net_trace_length(board, net);
    let stitch_before = plane_stitch(board, via_id, net, &center);
    // The candidate enumeration: the lattice family + the
    // alignment-derived intersections, deduplicated, in the documented
    // order.
    let restriction = board.rules().trace_angle_restriction;
    let mut candidates: Vec<ViaCandidate> = Vec::new();
    let mut seen: std::collections::BTreeSet<(i64, i64)> = std::collections::BTreeSet::new();
    let mut push_candidate =
        |dx: i64,
         dy: i64,
         candidates: &mut Vec<ViaCandidate>,
         seen: &mut std::collections::BTreeSet<(i64, i64)>| {
            if !via_radius_allows(dx, dy) || !via_displacement_ok(restriction, dx, dy) {
                return;
            }
            if !seen.insert((dx, dy)) {
                return;
            }
            let new_center = IntPoint::new(
                i32::try_from(i64::from(center.x) + dx).unwrap_or(center.x),
                i32::try_from(i64::from(center.y) + dy).unwrap_or(center.y),
            );
            let arm_total = arms
                .iter()
                .map(|arm| arm_length(&arm.far, &new_center))
                .sum();
            let ret_drop = stitch_before.map_or(0.0, |before| {
                plane_stitch(board, via_id, net, &new_center).map_or(before, |after| before - after)
            });
            candidates.push(ViaCandidate {
                dx,
                dy,
                new_center,
                ret_drop,
                arm_total,
            });
        };
    for &(dx, dy) in &via_lattice_candidates(restriction) {
        push_candidate(dx, dy, &mut candidates, &mut seen);
    }
    for (dx, dy) in alignment_candidates(&arms, &center, restriction) {
        push_candidate(dx, dy, &mut candidates, &mut seen);
    }
    // The RANK: return-path drop DESC, total arm length ASC, then the
    // enumeration order (dx, dy ASC — the enumeration order's key).
    // FLOAT NOTE: with diagonal arms the f64 totals carry √2 factors —
    // exactly-equal exact-math totals can compare float-distinct and
    // near-ties float-equal; `total_cmp` then decides by float bits,
    // deterministically.
    candidates.sort_by(|a, b| {
        b.ret_drop
            .total_cmp(&a.ret_drop)
            .then(a.arm_total.total_cmp(&b.arm_total))
            .then_with(|| {
                (a.dx.abs().max(a.dy.abs()), a.dx, a.dy).cmp(&(
                    b.dx.abs().max(b.dy.abs()),
                    b.dx,
                    b.dy,
                ))
            })
    });
    // The probe-and-land-first-fit sweep.
    let net_name = participant_name(board, net);
    let mut row = ViaPlaceRow {
        via_id: via_id.get(),
        net,
        net_name,
        from_x: i64::from(center.x),
        from_y: i64::from(center.y),
        to_x: i64::from(center.x),
        to_y: i64::from(center.y),
        landed: false,
    };
    for candidate in &candidates {
        // The IMPROVEMENT gate (the module-doc face): strictly
        // positive return-path drop, strictly smaller total arm
        // length, or strictly fewer total corners — else the via
        // stays.
        let new_arm_total: f64 = arms
            .iter()
            .map(|arm| arm_length(&arm.far, &candidate.new_center))
            .sum();
        let new_corner_total: usize = arms.iter().map(|_| 2).sum();
        let improves = candidate.ret_drop > 0.0
            || new_arm_total < arms.iter().map(|arm| arm.current_length).sum::<f64>()
            || new_corner_total < arms.iter().map(|arm| arm.corner_count).sum::<usize>();
        if !improves {
            continue;
        }
        let delta = epic_geometry::vector::Vector::get_instance(
            i32::try_from(candidate.dx).unwrap_or(0),
            i32::try_from(candidate.dy).unwrap_or(0),
        );
        if !via_candidate_accepts(
            manager,
            board,
            via_id,
            net,
            net_len_before,
            &arms,
            &candidate.new_center,
            &delta,
        ) {
            continue;
        }
        land_via_candidate(manager, board, via_id, &arms, &candidate.new_center);
        row.to_x = i64::from(candidate.new_center.x);
        row.to_y = i64::from(candidate.new_center.y);
        row.landed = true;
        break;
    }
    rows.push(row);
}

/// The net's total on-board TRACE length (piece-sum, the T3 face).
fn net_trace_length(board: &Board, net: i32) -> f64 {
    board
        .get_connectable_items(net)
        .into_iter()
        .filter_map(|id| board.trace_polyline(id).map(|lines| (id, lines)))
        .filter(|(id, _)| {
            matches!(
                board.get(*id).map(|entry| &entry.data),
                Some(epic_board::items::ItemData::Trace { .. })
            )
        })
        .map(|(_, lines)| piece_length_sum(&lines.corners()))
        .sum()
}

/// The arm length between a far anchor and a candidate center (f64
/// Euclidean).
fn arm_length(far: &IntPoint, center: &IntPoint) -> f64 {
    let dx = i64::from(far.x) - i64::from(center.x);
    let dy = i64::from(far.y) - i64::from(center.y);
    ((dx * dx + dy * dy) as f64).sqrt()
}

/// The ALIGNMENT-DERIVED candidate family: the intersections of the
/// two arms' 45°-legal direction lines through their far anchors —
/// the exact in-line positions (arbitrary DBU coordinates). Each arm
/// contributes the direction set its board restriction allows (4
/// directions on NinetyDegree, 8 on FortyfiveDegree); an intersection
/// must be exactly integer. A None-restriction board yields NO
/// alignment family (the in-line objective needs directions). The
/// returned displacements are relative to `center`; the radius and
/// angle filters apply at the push site.
#[must_use]
fn alignment_candidates(
    arms: &[ViaArm],
    center: &IntPoint,
    restriction: epic_board::rules_surf::AngleRestriction,
) -> Vec<(i64, i64)> {
    use epic_board::rules_surf::AngleRestriction;
    let directions: Vec<(i64, i64)> = match restriction {
        AngleRestriction::NinetyDegree => vec![(1, 0), (-1, 0), (0, 1), (0, -1)],
        AngleRestriction::FortyfiveDegree => vec![
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ],
        AngleRestriction::None => return Vec::new(),
    };
    if arms.len() != 2 {
        return Vec::new();
    }
    // Opposite direction pairs generate the SAME line, so the raw
    // double loop yields each intersection multiple times — the
    // DEDUP+sort makes the family itself deterministic.
    let mut unique: std::collections::BTreeSet<(i64, i64)> = std::collections::BTreeSet::new();
    for &(d1x, d1y) in &directions {
        for &(d2x, d2y) in &directions {
            let a1 = &arms[0].far;
            let a2 = &arms[1].far;
            // p = a1 + t*d1 = a2 + s*d2; solve exactly in i64.
            let rx = i64::from(a2.x) - i64::from(a1.x);
            let ry = i64::from(a2.y) - i64::from(a1.y);
            let det = d1x * d2y - d1y * d2x;
            if det == 0 {
                continue; // parallel direction lines: no intersection
            }
            // t = cross(r, d2)/det, s = cross(r, d1)/det.
            let t_num = rx * d2y - ry * d2x;
            let s_num = rx * d1y - ry * d1x;
            if t_num % det != 0 || s_num % det != 0 {
                continue; // non-integer intersection: not a candidate
            }
            let t = t_num / det;
            let px = i64::from(a1.x) + t * d1x;
            let py = i64::from(a1.y) + t * d1y;
            unique.insert((px - i64::from(center.x), py - i64::from(center.y)));
        }
    }
    let mut out: Vec<(i64, i64)> = unique.into_iter().collect();
    out.sort_by_key(|&(dx, dy)| (dx.abs().max(dy.abs()), dx, dy));
    out
}

/// The whole-candidate acceptance (nothing partial): the arms' fixed
/// gates, both arms angle-legal as single segments, the net length
/// budget, the REAL via probe (`drill_item_mover::check` 0/0 — the
/// audited ViaOptimizer pre-check face, no shoving), and the REAL arm
/// probes (`check_trace_segment_points`, same-net exempt).
#[allow(clippy::too_many_arguments)] // the acceptance read set, kept flat
fn via_candidate_accepts(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    net: i32,
    net_len_before: f64,
    arms: &[ViaArm],
    new_center: &IntPoint,
    delta: &epic_geometry::vector::Vector,
) -> bool {
    // The arms' fixed-state gate.
    for arm in arms {
        if board.get(arm.trace_id).is_some_and(|entry| {
            matches!(entry.fixed, FixedState::UserFixed | FixedState::SystemFixed)
        }) {
            return false;
        }
    }
    // Both arms angle-legal as SINGLE segments into the new center.
    for arm in arms {
        let dx = i64::from(new_center.x) - i64::from(arm.far.x);
        let dy = i64::from(new_center.y) - i64::from(arm.far.y);
        if !via_displacement_ok(board.rules().trace_angle_restriction, dx, dy) {
            return false;
        }
    }
    // The net's trace-length budget (equality allowed).
    let new_arm_total: f64 = arms
        .iter()
        .map(|arm| arm_length(&arm.far, new_center))
        .sum();
    let old_arm_total: f64 = arms.iter().map(|arm| arm.current_length).sum();
    if !via_length_guard_ok(
        net_len_before,
        net_len_before + new_arm_total - old_arm_total,
    ) {
        return false;
    }
    // The degenerate-arm guard: an arm of length zero (the candidate
    // ON a far anchor) is not a landing. DEFENSE-IN-DEPTH, provably
    // redundant for integer inputs: the angle gate above already
    // rejects a zero arm (via_displacement_ok answers false for
    // (0,0), and arm_length <= 0.0 holds iff far == new_center iff
    // the displacement is (0,0)) — the guard can never be the
    // decisive rejector; the far-anchor observable is pinned by V11's
    // anchor-coincident candidate (enumerated, probed, rejected
    // upstream).
    if arms
        .iter()
        .any(|arm| arm_length(&arm.far, new_center) <= 0.0)
    {
        return false;
    }
    // The REAL via probe: the mover legality check at budgets 0/0.
    let mut ignore_items = Vec::new();
    if !epic_board::drill_item_mover::check(
        manager,
        board,
        via_id,
        delta,
        0,
        0,
        &mut ignore_items,
        None,
    ) {
        return false;
    }
    // The REAL arm probes: each new single-segment arm at full length.
    for arm in arms {
        let insertable = check_trace_segment_points(
            manager,
            board,
            &Point::Int(arm.far),
            &Point::Int(*new_center),
            arm.layer,
            &[net],
            arm.half_width,
            arm.clearance_class,
            false,
        );
        if insertable + 1e-6 < arm_length(&arm.far, new_center) {
            return false;
        }
    }
    true
}

/// The landing write: tree remove → `set_via_center` (board.rs:1068,
/// the RAW center write — the caller owns the acceptance face, the
/// `replace_trace_geometry` analog) → `clear_derived_data` → tree
/// re-insert, then each arm's polyline replaced WHOLESALE with the
/// single straight segment (the whole-candidate write; no undo save
/// and no connector-trace insertion — the gloss family never touches
/// undo, and the arms carry the connection geometry themselves).
fn land_via_candidate(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    arms: &[ViaArm],
    new_center: &IntPoint,
) {
    manager.remove(board, via_id);
    board.set_via_center(via_id, *new_center);
    board.clear_derived_data(via_id);
    manager.insert(board, via_id);
    for arm in arms {
        board.replace_trace_geometry(
            manager,
            arm.trace_id,
            Polyline::from_two_corners(&Point::Int(arm.far), &Point::Int(*new_center)),
        );
    }
}

/// ===========================================================================
/// M8-T6: the TEARDROPS pass — graded-width wires at trace/pad junctions
/// ===========================================================================
///
/// **The representation (the T6 investigation record,
/// `logs/M8-T6/investigation.md`).** A teardrop built as a polygon
/// conduction item has a REAL session face — the SES writer emits
/// `(wire (polygon ...))` (`epic_dsn::ses::writer::write_conduction_area`,
/// the Java `SesWriter.writeConductionArea` mirror) — but NO consumer
/// round-trips it: KiCad's `FromSESSION` explicitly ignores `T_polygon`
/// wires ("Wire polygons are zone fills ... ignore the session pour
/// geometry"), and the parity oracle's own `SesReader.processWireScope`
/// silently skips every wire scope without a `polygon_path` ("conduction
/// areas have no polygon_path — silently skip"). A polygon teardrop is
/// DRC-correct in-engine and INVISIBLE downstream — dishonest. So the
/// representation is GRADED-WIDTH WIRES: short overlapping `(wire (path
/// ...))` traces of increasing width along the trace axis into the pad —
/// the session's native currency, imported as tracks by every consumer.
/// Same-net overlap is clearance-exempt in the REAL counter (the tree
/// query filters obstacles per-net — an item sharing the probe's net is
/// not an obstacle for it), so the overlaps with the pad, the base
/// trace, and each other are legal; only the taper's OUTER boundary
/// faces foreign nets, and that boundary is probed with the REAL
/// counter before anything lands.
///
/// **The junction.** A trace ENDPOINT (either end) EXACTLY at a
/// same-net pin/via pad center (the endpoint-connected via-place face).
/// The pad diameter = the MAX `shape_max_width` over ALL the
/// padstack's shapes (a circle's is 2·radius) — the T6 investigation's
/// locked consequence 2. A per-trace-LAYER diameter selection is a
/// KNOWN DEFERRED improvement (never shipped; the AM6 note) — the
/// pass's only caller uses the max-over-shapes face. The
/// ANATOMY gate (the named ratio threshold): the trace width must be
/// strictly less than [`TEARDROP_MAX_TRACE_RATIO_PCT`] percent of the
/// pad diameter — the exact edge (width == diameter → NO teardrop,
/// one unit narrower → teardrop) is pinned and DNR-16
/// mutation-verified BOTH directions.
///
/// **The geometry (literal, integer).** Wires run along the trace's
/// END SEGMENT, widest at the pad. The end segment must be
/// axis-aligned or exact 45° ([`teardrop_axis_ok`]) so the back-offsets
/// stay ON the segment. Wire k (k = 1..=[`TEARDROP_WIRE_COUNT`]) runs
/// from `min(k·[`TEARDROP_STEP_DBU`], seg_len_cheb)` back from the pad
/// center to the center, with half-width `hw + (r−hw)·k/N` (integer
/// division; k = N is exactly the pad radius r). The plan is
/// [`graded_wire_plan`] — pinnable in isolation. Degenerate plans
/// (r ≤ hw: no taper possible) skip the junction silently.
///
/// **Acceptance (whole-candidate, the via-place discipline).** ALL
/// graded wires pass their REAL clearance probes
/// (`check_trace_segment_points`, same-net exempt) at full length
/// BEFORE any write; a single shortfall → the junction is skipped with
/// an honest `"landed": false` row. A teardrop NEVER introduces a
/// violation — pinned at the clearance boundary (exactly-at → lands
/// with zero board violations, one unit tighter → no landing).
///
/// **The landing.** Wires insert k-ascending through
/// `epic_board::trace_ops::insert_trace_without_cleaning` (the raw
/// insert — no normalization combine can merge the teardrop into the
/// base trace), `FixedState::ShoveFixed` (survives any later shove;
/// the SES writer skips only `SystemFixed`, so teardrops emit).
///
/// **Determinism.** Sweep: net ascending, trace id ascending, start
/// corner before end corner; per-junction wires k ascending (ids
/// allocated in that order). No threads, no hash-order iteration.
///
/// **Two-regime safety.** The ONLY caller gate is the resolved
/// `router.gloss.teardrops` flag (`BatchSettings::teardrops_active`,
/// default false); an OFF run never constructs a report and never
/// touches the board. The report rides the `--dump-aesthetics` SIDECAR
/// ONLY (the `gloss_teardrops` block rendered by epic-cli — NEVER the
/// manifest: the version-blind manifest canary pins manifest bytes;
/// the raw cf607714… retired at M10-T5); the incompletes
/// gate surfaces as the DISTINCT `gloss_teardrops_gated` sibling key.
/// The stage is the TERMINAL gloss slot (after via-place — that pass
/// moves vias, teardrops attach at the moved positions) and emits NO
/// events and NO stage transition (the meander-stage precedent).
//
// -- the named constants (the DNR-16 family discipline) ---------------------
/// The ANATOMY boundary, as a percentage: a junction qualifies only
/// when the trace width is STRICTLY less than this percentage of the
/// pad diameter (integer cross-multiplication — no float). Value: 100
/// — the plan's anatomy condition verbatim ("trace width < the pad
/// diameter"). The exact edge is pinned at [`teardrop_anatomy_ok`]:
/// width == diameter rejects, one unit narrower accepts, and BOTH ±1
/// constant mutations (100 → 99 and 100 → 101) are killed by the
/// boundary pins (DNR-16).
pub const TEARDROP_MAX_TRACE_RATIO_PCT: i64 = 100;

/// The graded-wire COUNT: the taper's step count N. Value: 3 — the
/// plan's "2–3 short overlapping wires" band at its wide end: three
/// steps give a visible taper without a wire-count tax on big boards.
/// Granularity, not a boundary predicate — the exact-geometry pin
/// ([`graded_wire_plan`]) pins its effect (the via-place step
/// precedent).
pub const TEARDROP_WIRE_COUNT: usize = 3;

/// The per-wire LENGTH step along the trace axis, in board DBU: wire k
/// starts `min(k·STEP, seg_len_cheb)` back from the pad center. Value:
/// 4_000 DBU = 0.4 mm at the um-10 transform — a pad-scale taper
/// quantum (the [`VIA_PLACE_STEP_DBU`] mirror). Granularity, not a
/// boundary predicate — the exact-geometry pin pins its effect.
pub const TEARDROP_STEP_DBU: i64 = 4_000;

/// The per-board landing BUDGET: at most this many teardrops land per
/// stage run (deterministic — the sweep order decides which). Value:
/// 1024 — far above any corpus fixture's junction count (the T5
/// lattice census peaks at dozens), so the budget is a runaway guard,
/// not a shaping force. The boundary is pinned at the budgeted entry
/// (0 = no junction attempted at all, 1 = exactly the first landing)
/// rather than by a ±1 constant mutation: 1024 vs 1023 is
/// behaviorally identical on every board with fewer than 1023
/// junctions — there is no discriminating world at that edge, and the
/// budgeted-entry pins own the boundary behavior itself. The budget
/// counts inserted WIRES (`*landings` increments once per graded wire
/// at the landing loop), so a full taper costs
/// [`TEARDROP_WIRE_COUNT`] (3) and the nominal per-board taper cap is
/// 1024/3 ≈ 341 — but the budget check is JUNCTION-level, so the cap
/// is SOFT: a junction admitted at 1023 landings writes its full 3
/// wires (up to 1026 wires / 342 tapers at budget 1024). The
/// budgeted-entry pins (0/1) own the junction-level boundary behavior
/// and are unaffected.
pub const TEARDROP_BUDGET_PER_BOARD: usize = 1024;

/// The ANATOMY admitter, pinnable in isolation (the DNR-16 exact edge:
/// width == diameter rejects, one unit narrower accepts — both ±1
/// constant mutations verified against these pins). Integer
/// cross-multiplication; no float anywhere.
#[must_use]
pub fn teardrop_anatomy_ok(trace_width: i64, pad_diameter: i64) -> bool {
    trace_width * 100 < TEARDROP_MAX_TRACE_RATIO_PCT * pad_diameter
}

/// The END-SEGMENT ANGLE gate: the trace's end segment must be
/// axis-aligned (`dx == 0` or `dy == 0`) or exact 45° (`|dx| == |dy|`)
/// so the wires' Chebyshev back-offsets stay ON the segment. The
/// degenerate zero segment (`dx == 0 && dy == 0`) is rejected here
/// too — the caller guards it first, but the gate is total.
#[must_use]
pub fn teardrop_axis_ok(dx: i64, dy: i64) -> bool {
    if dx == 0 && dy == 0 {
        return false;
    }
    dx == 0 || dy == 0 || dx.abs() == dy.abs()
}

/// The GRADED-WIRE PLAN (pinnable in isolation): for k =
/// 1..=`wire_count`, the back-offset `min(k·step, seg_len_cheb)` and
/// the half-width `hw + (r−hw)·k/N` (integer division). The returned
/// order is k ascending — the landing order. Degenerate plans, both
/// documented: (a) the CLAMP COLLAPSE — when `seg_len_cheb < N·step`
/// the inner wires share the clamped back (duplicate backs, still-
/// graded widths; the same-net overlap is legal and the silhouette is
/// still tapered); (b) the DUPLICATE-WIDTH plan — when `r − hw < N`
/// several wires share widths near the base trace. The exact-geometry
/// pins assert this plan verbatim (incl. the clamp face).
#[must_use]
pub fn graded_wire_plan(
    trace_half_width: i64,
    pad_radius: i64,
    seg_len_cheb: i64,
    wire_count: usize,
    step: i64,
) -> Vec<(i64, i64)> {
    let n = wire_count.max(1) as i64;
    (1..=n)
        .map(|k| {
            let back = (k * step).min(seg_len_cheb);
            let half_width = trace_half_width + (pad_radius - trace_half_width) * k / n;
            (back, half_width)
        })
        .collect()
}

/// One teardrop junction's row (the honest-stop face): the base trace
/// id, the junction (pad center), and whether the taper LANDED.
#[derive(Clone, Debug, PartialEq)]
pub struct TeardropRow {
    pub trace_id: u32,
    pub net: i32,
    pub net_name: String,
    /// The junction (the pad center), board DBU.
    pub at_x: i64,
    /// The junction (the pad center), board DBU.
    pub at_y: i64,
    /// The pad diameter the anatomy gate measured, board DBU.
    pub pad_diameter: i64,
    /// Whether the graded taper LANDED (the wires were written).
    pub landed: bool,
}

/// The gloss TEARDROPS stage report (the sidecar's `gloss_teardrops`
/// block source). Default = empty = the OFF face (never serialized).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlossTeardropsReport {
    /// Per-attempted-junction rows in sweep order (net asc, trace id
    /// asc, start corner before end corner).
    pub rows: Vec<TeardropRow>,
    /// The INCOMPLETES-GATE marker (the distinct sibling key from day
    /// one — the T3 lesson): true when the stage short-circuited
    /// because some net still has incompletes. Default false.
    pub gated: bool,
}

impl GlossTeardropsReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The pad diameter: the MAX `shape_max_width` over ALL the
/// padstack's shapes (a circle's is 2·radius) — the investigation's
/// locked consequence 2. `None` for a non-drill item or a shapeless
/// padstack.
fn pad_diameter_at(board: &mut Board, id: ItemId) -> Option<i64> {
    let count = board.drill_tile_shape_count(id)?;
    let mut any_face: Option<i64> = None;
    for index in 0..count {
        let shape = board.drill_shape(id, index)?;
        let width = epic_board::items::shape_max_width(&shape).round() as i64;
        any_face = Some(any_face.map_or(width, |current: i64| current.max(width)));
    }
    any_face
}

/// The stage entry (the pipeline's terminal gloss slot — after
/// via-place, the recorded slot decision in the module docs; the
/// caller gates on `BatchSettings::teardrops_active` — the flag is the
/// only caller gate). Emits NO events and NO stage transition (the
/// meander-stage precedent). An empty report = no attempted junction,
/// nothing landed.
pub fn run_gloss_teardrops_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> GlossTeardropsReport {
    run_gloss_teardrops_stage_budgeted(manager, board, TEARDROP_BUDGET_PER_BOARD)
}

/// The budgeted entry (the budget boundary's pin face): identical to
/// [`run_gloss_teardrops_stage`] with the landing budget a parameter.
fn run_gloss_teardrops_stage_budgeted(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    budget: usize,
) -> GlossTeardropsReport {
    // The incompletes gate (the T3/T4/T5 precedent): a net with
    // incompletes is not a gloss target (still being routed).
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    if incompletes.iter().any(|row| row.incomplete_count > 0) {
        return GlossTeardropsReport {
            gated: true,
            ..Default::default()
        };
    }
    let mut report = GlossTeardropsReport::default();
    let mut landings = 0usize;
    // The junction DEDUP: one RECORDED row per pad center. A center
    // is marked done only when an attempt returns `Some(row)` (landed
    // OR honest probe-stop); a GATE-REJECTED endpoint (anatomy/angle →
    // `None`, no row) leaves the junction open for a later qualifying
    // sibling endpoint at the same center. Duplicate stacked tapers
    // are never built (the td8 pin).
    let mut done: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
    let max_net = board.rules().nets.max_net_number();
    for net in 1..=max_net {
        if landings >= budget {
            break;
        }
        // The junction pad table for this net: pad center → diameter
        // (pins and vias; a later same-center pad wins the entry — a
        // via stacked exactly on a pin center is the same copper).
        let mut pads: std::collections::BTreeMap<(i32, i32), i64> =
            std::collections::BTreeMap::new();
        for id in board.get_connectable_items(net) {
            let is_pad = matches!(
                board.get(id).map(|entry| &entry.data),
                Some(epic_board::items::ItemData::Pin { .. })
                    | Some(epic_board::items::ItemData::Via { .. })
            );
            if !is_pad {
                continue;
            }
            let Some(Point::Int(center)) = board.drill_center(id) else {
                continue;
            };
            if let Some(diameter) = pad_diameter_at(board, id) {
                pads.insert((center.x, center.y), diameter);
            }
        }
        let mut trace_ids: Vec<ItemId> = board
            .get_connectable_items(net)
            .into_iter()
            .filter(|&id| {
                matches!(
                    board.get(id).map(|entry| &entry.data),
                    Some(epic_board::items::ItemData::Trace { .. })
                )
            })
            .collect();
        trace_ids.sort();
        for trace_id in trace_ids {
            if landings >= budget {
                break;
            }
            let Some(lines) = board.trace_polyline(trace_id) else {
                continue;
            };
            let count = lines.corner_count();
            if count < 2 {
                continue;
            }
            let Some(layer) = board.trace_layer(trace_id) else {
                continue;
            };
            // The two endpoints, START CORNER FIRST (the sweep order).
            // Copied OWNED before the attempt (the attempt needs &mut
            // board; the polyline borrow must end first).
            let endpoints: Vec<(IntPoint, IntPoint)> = [(0usize, 1usize), (count - 1, count - 2)]
                .iter()
                .filter_map(|&(end_index, other_index)| {
                    let end = lines.corner(i32::try_from(end_index).unwrap_or(0))?;
                    let other = lines.corner(i32::try_from(other_index).unwrap_or(0))?;
                    Some((as_int(&end)?, as_int(&other)?))
                })
                .collect();
            for (end, other) in endpoints {
                let Some(&diameter) = pads.get(&(end.x, end.y)) else {
                    continue;
                };
                if landings >= budget {
                    break;
                }
                if done.contains(&(end.x, end.y)) {
                    continue;
                }
                if let Some(row) = teardrop_attempt(
                    manager,
                    board,
                    net,
                    trace_id,
                    layer,
                    end,
                    other,
                    diameter,
                    &mut landings,
                ) {
                    // Mark the center consumed ONLY on a RECORDED
                    // attempt (landed or honest probe-stop); a
                    // gate-rejected endpoint leaves the junction open.
                    done.insert((end.x, end.y));
                    report.rows.push(row);
                }
            }
        }
    }
    report
}

/// One junction's attempt: the anatomy + angle + taper gates, the
/// whole-candidate REAL probes, and the honest row. Returns `None` for
/// the SILENT skips (anatomy failure = the no-teardrop world,
/// degenerate/angle-inapplicable segments — the no-candidate face);
/// `Some(row)` for every ATTEMPTED junction (probes decide `landed`).
#[allow(clippy::too_many_arguments)] // the attempt read set, kept flat
fn teardrop_attempt(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net: i32,
    trace_id: ItemId,
    layer: i32,
    end: IntPoint,
    other: IntPoint,
    pad_diameter: i64,
    landings: &mut usize,
) -> Option<TeardropRow> {
    let net_name = participant_name(board, net);
    let mut row = TeardropRow {
        trace_id: trace_id.get(),
        net,
        net_name,
        at_x: i64::from(end.x),
        at_y: i64::from(end.y),
        pad_diameter,
        landed: false,
    };
    let half_width = board.trace_half_width(trace_id)?;
    let clearance_class = board.item_clearance_class(trace_id)?;
    // The ANATOMY gate — silent on failure (the no-teardrop world).
    let trace_width = 2 * i64::from(half_width);
    if !teardrop_anatomy_ok(trace_width, pad_diameter) {
        return None;
    }
    // The end-segment gates — silent on failure (the no-candidate
    // face): degenerate, or not axis/45° (the back-offsets would
    // leave the segment).
    let dx = i64::from(other.x) - i64::from(end.x);
    let dy = i64::from(other.y) - i64::from(end.y);
    let seg_len_cheb = dx.abs().max(dy.abs());
    if seg_len_cheb == 0 || !teardrop_axis_ok(dx, dy) {
        return None;
    }
    let pad_radius = pad_diameter / 2;
    if pad_radius <= i64::from(half_width) {
        return None; // no taper possible
    }
    let plan = graded_wire_plan(
        i64::from(half_width),
        pad_radius,
        seg_len_cheb,
        TEARDROP_WIRE_COUNT,
        TEARDROP_STEP_DBU,
    );
    // The Chebyshev unit step along the segment (axis: the one nonzero
    // axis; 45°: both signs).
    let step = (dx.signum(), dy.signum());
    let wires: Vec<(IntPoint, i64)> = plan
        .iter()
        .map(|&(back, half_width)| {
            let start = IntPoint::new(
                end.x + i32::try_from(step.0 * back).unwrap_or(end.x),
                end.y + i32::try_from(step.1 * back).unwrap_or(end.y),
            );
            (start, half_width)
        })
        .collect();
    // WHOLE-CANDIDATE acceptance: every wire's REAL probe at full
    // length before ANY write (same-net exempt by the tree query —
    // the pad, the base trace, and the sibling wires are legal).
    let all_clear = wires.iter().all(|&(start, half)| {
        let length = arm_length(&start, &end);
        let insertable = check_trace_segment_points(
            manager,
            board,
            &Point::Int(start),
            &Point::Int(end),
            layer,
            &[net],
            i32::try_from(half).unwrap_or(i32::MAX),
            clearance_class,
            false,
        );
        insertable + 1e-6 >= length
    });
    if !all_clear {
        return Some(row); // the honest stop, recorded
    }
    let mut inserted = 0usize;
    for &(start, half) in &wires {
        let Some(_landed) = epic_board::trace_ops::insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(&Point::Int(start), &Point::Int(end)),
            layer,
            i32::try_from(half).unwrap_or(i32::MAX),
            &[net],
            clearance_class,
            FixedState::ShoveFixed,
        ) else {
            continue;
        };
        inserted += 1;
        *landings += 1;
    }
    // The landing invariant, self-announcing (the two-crate face:
    // `from_two_corners` yields a 3-line polyline for distinct
    // corners; `back >= 1` guarantees distinct corners;
    // `insert_trace_without_cleaning` fails only on <3 lines / closed
    // traces — so a recorded landing is ALWAYS a full taper).
    debug_assert_eq!(inserted, wires.len());
    row.landed = true;
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse;
    use epic_dsn::ses_board::{ItemIr, SesBoard};
    use epic_dsn::sink::{ClearanceIr, CreateBoardIr, FixedStateIr, NetClassIr, NetIr, TraceIr};
    use epic_geometry::int_box::IntBox;

    /// A td10 test-local row: (start, end, half_width) of one taper wire.
    type DiagWire = ((i64, i64), (i64, i64), i32);

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

    // -- the VIA PLACE pass (M8-T5) ---------------------------------

    use epic_board::rules_surf::AngleRestriction;
    use epic_dsn::shape::BoardShape as DsnBoardShape;
    use epic_dsn::sink::FixedStateIr as DsnFixedStateIr;
    use epic_dsn::sink::{AreaIr, ConductionAreaIr, KeepoutIr, KeepoutKindIr, PadstackIr, ViaIr};
    use epic_geometry::circle::Circle as DsnCircle;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// The via world's VIA padstack (radius-100 circles on both layers).
    fn via_padstack() -> PadstackIr {
        PadstackIr {
            name: "VIA".to_string(),
            shapes: vec![
                Some(DsnBoardShape::Circle(DsnCircle::new(
                    IntPoint::new(0, 0),
                    100,
                ))),
                Some(DsnBoardShape::Circle(DsnCircle::new(
                    IntPoint::new(0, 0),
                    100,
                ))),
            ],
            drillable: true,
            placed_absolute: false,
        }
    }

    /// The OFF-BEND VIA world (NinetyDegree): net 1 "sig" — arm 1
    /// straight F.Cu from (30_000,150_000) to the via at
    /// (200_000,150_000); arm 2 a STAIRCASE on B.Cu from the via
    /// (200_000,150_000) → (200_000,250_000) → (260_000,250_000). The
    /// unique in-radius both-arms-straight position is (260_000,
    /// 150_000) — a Chebyshev displacement of EXACTLY
    /// [`VIA_PLACE_RADIUS_DBU`] (60_000): the at-radius boundary world.
    /// `far2_x` shifts arm 2's far anchor (the radius+1 world sets it
    /// to 260_001); `pour_min_x` adds a SAME-NET plane pour (net 1 is
    /// a plane net with a rect conduction area on F.Cu x ≥ pour_min_x)
    /// when `Some`.
    fn via_world(far2_x: i64, pour_min_x: Option<i64>) -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        ses.padstacks.push(via_padstack());
        register_nets(&mut ses, &[("sig", pour_min_x.is_some())]);
        // arm 1: F.Cu, straight into the via center.
        straight_trace(
            &mut ses,
            3,
            1,
            vec![
                IntPoint::new(30_000, 150_000),
                IntPoint::new(200_000, 150_000),
            ],
        );
        // arm 2: B.Cu, staircase out of the via center.
        ses.push_routed_item(staircase_trace(far2_x));
        // the via at the bend (200_000, 150_000).
        ses.push_routed_item(ItemIr::Via {
            id: 5,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(200_000, 150_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        if let Some(pour_min_x) = pour_min_x {
            ses.push_routed_item(ItemIr::ConductionArea {
                id: 6,
                area: ConductionAreaIr {
                    layer_no: 0,
                    area: AreaIr::simple(DsnBoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(i32::try_from(pour_min_x).unwrap_or(0), 100_000),
                            IntPoint::new(500_000, 200_000),
                        )),
                    ))),
                    nets: vec![1],
                    clearance_class: 1,
                    fixed: DsnFixedStateIr::Unfixed,
                },
            });
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    fn staircase_corners(far2_x: i64) -> Vec<IntPoint> {
        vec![
            IntPoint::new(200_000, 150_000),
            IntPoint::new(200_000, 250_000),
            IntPoint::new(i32::try_from(far2_x).unwrap_or(0), 250_000),
        ]
    }

    /// The SPLIT-RUN VIA world: net 1 "sig" (plane when the pour is
    /// present) — arm 1 F.Cu (30_000,150_000)→(200_000,150_000); arm 2
    /// B.Cu (200_000,150_000)→(260_000,150_000); the via at
    /// (200_000,150_000) mid-run. All 12 on-line lattice candidates
    /// (dx ±10_000..±60_000, dy = 0) are both-arms-straight with a
    /// constant arm total — the return-path rank alone discriminates.
    /// `pour_min_x` adds the same-net F.Cu rect pour x ≥ pour_min_x.
    fn via_world_split(pour_min_x: Option<i64>) -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        ses.padstacks.push(via_padstack());
        register_nets(&mut ses, &[("sig", pour_min_x.is_some())]);
        straight_trace(
            &mut ses,
            3,
            1,
            vec![
                IntPoint::new(30_000, 150_000),
                IntPoint::new(200_000, 150_000),
            ],
        );
        straight_trace(
            &mut ses,
            4,
            1,
            vec![
                IntPoint::new(200_000, 150_000),
                IntPoint::new(260_000, 150_000),
            ],
        );
        ses.push_routed_item(ItemIr::Via {
            id: 5,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(200_000, 150_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        if let Some(pour_min_x) = pour_min_x {
            ses.push_routed_item(ItemIr::ConductionArea {
                id: 6,
                area: ConductionAreaIr {
                    layer_no: 0,
                    area: AreaIr::simple(DsnBoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(i32::try_from(pour_min_x).unwrap_or(0), 100_000),
                            IntPoint::new(500_000, 200_000),
                        )),
                    ))),
                    nets: vec![1],
                    clearance_class: 1,
                    fixed: DsnFixedStateIr::Unfixed,
                },
            });
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// The trace IR for arm 2 (a B.Cu staircase polyline).
    fn staircase_trace(far2_x: i64) -> ItemIr {
        let corners = staircase_corners(far2_x);
        ItemIr::Trace {
            id: 4,
            trace: TraceIr {
                layer_no: 1,
                half_width: 2000,
                corners: corners.clone(),
                polyline: TraceIr::polyline_of_corners(&corners),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        }
    }

    /// The net-1 via count (the COUNT-INVARIANCE face's counter).
    fn via_count(board: &Board) -> usize {
        board
            .get_connectable_items(1)
            .into_iter()
            .filter(|&id| {
                matches!(
                    board.get(id).map(|entry| &entry.data),
                    Some(epic_board::items::ItemData::Via { .. })
                )
            })
            .count()
    }

    /// **V1 (the lattice enumeration order + step/radius granularity).**
    /// NinetyDegree: the axis-only multiples ±10_000..±60_000, x before
    /// y, ascending magnitude: the EXACT 24-element list. A step mutant
    /// of EITHER sign changes the list (the enumeration pin); the
    /// radius edge keeps ring 6 (`6×10_000 == VIA_PLACE_RADIUS_DBU`,
    /// the inclusive edge) — an R−1 mutant drops it.
    #[test]
    fn v1_lattice_enumeration_order() {
        let got = via_lattice_candidates(AngleRestriction::NinetyDegree);
        // LITERAL expectation (the DNR-16 discipline: a pin built from
        // the constant follows the constant — a step mutant could never
        // flip it). The 24 axis multiples, x before y, ascending
        // magnitude.
        let want: Vec<(i64, i64)> = vec![
            (-10_000, 0),
            (0, -10_000),
            (0, 10_000),
            (10_000, 0),
            (-20_000, 0),
            (0, -20_000),
            (0, 20_000),
            (20_000, 0),
            (-30_000, 0),
            (0, -30_000),
            (0, 30_000),
            (30_000, 0),
            (-40_000, 0),
            (0, -40_000),
            (0, 40_000),
            (40_000, 0),
            (-50_000, 0),
            (0, -50_000),
            (0, 50_000),
            (50_000, 0),
            (-60_000, 0),
            (0, -60_000),
            (0, 60_000),
            (60_000, 0),
        ];
        assert_eq!(got, want);
        // FortyfiveDegree adds the on-diagonal rings (48 total).
        let got45 = via_lattice_candidates(AngleRestriction::FortyfiveDegree);
        assert_eq!(got45.len(), 48);
        // None: the full grid minus the center (168 candidates).
        let got_none = via_lattice_candidates(AngleRestriction::None);
        assert_eq!(got_none.len(), 168);
    }

    /// The alignment-family probe over a plain anchor pair (no board):
    /// anchors (30_000,150_000) and (260_000,250_000) with the center
    /// (200_000,150_000) — the at-radius world's frames. The in-radius
    /// both-arms-straight intersections: EXACTLY the in-line
    /// (60_000, 0); the other intersection (30_000,250_000) is out of
    /// radius and the current position is not an anchor-line
    /// intersection.
    #[test]
    fn v1b_alignment_family_world() {
        let arms = [
            via_arm(1, IntPoint::new(30_000, 150_000), 0),
            via_arm(2, IntPoint::new(260_000, 250_000), 1),
        ];
        assert_eq!(
            alignment_candidates(
                &arms,
                &IntPoint::new(200_000, 150_000),
                AngleRestriction::NinetyDegree
            ),
            // Deduped and in the family order; the radius filter applies
            // at the push site, so the out-of-radius intersection is in
            // the family list (and is dropped before any probe).
            vec![(60_000, 0), (-170_000, 100_000)]
        );
    }

    /// The BLOCKED world builder: the at-radius world plus a plain
    /// structure keepout covering the in-line landing
    /// (250_000..270_000 × 140_000..160_000) on F.Cu — the via pad and
    /// arm 1's extension both collide → every candidate rejected, the
    /// honest `landed: false` row, geometry verbatim, via count
    /// invariant.
    fn via_world_blocked() -> (SearchTreeManager, Board) {
        let mut ses = base_ses();
        ses.padstacks.push(via_padstack());
        register_nets(&mut ses, &[("sig", false)]);
        straight_trace(
            &mut ses,
            3,
            1,
            vec![
                IntPoint::new(30_000, 150_000),
                IntPoint::new(200_000, 150_000),
            ],
        );
        ses.push_routed_item(staircase_trace(260_000));
        ses.push_routed_item(ItemIr::Via {
            id: 5,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(200_000, 150_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        ses.push_routed_item(ItemIr::Keepout {
            id: 7,
            keepout: KeepoutIr {
                kind: KeepoutKindIr::Keepout,
                layer_no: 0,
                area: AreaIr::simple(DsnBoardShape::Tile(TileShape::RegularTileShape(
                    RegularTileShape::IntBox(IntBox::new(
                        IntPoint::new(250_000, 140_000),
                        IntPoint::new(270_000, 160_000),
                    )),
                ))),
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
                component_id: 0,
                rotation: 0.0,
                side_changed: false,
                name: None,
                translation: IntPoint::new(0, 0),
            },
        });
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (manager, board)
    }

    /// **V6 (the blocked world).** No candidate passes → the honest
    /// `landed: false` row, the via and both arms VERBATIM, the via
    /// count invariant.
    #[test]
    fn v6_blocked_world_honest_row() {
        let (mut manager, mut board) = via_world_blocked();
        let before_center = board.drill_center(ItemId::new(5));
        let before_arm1 = board
            .trace_polyline(ItemId::new(3))
            .map(|l| l.corners().len());
        let before_arm2 = board
            .trace_polyline(ItemId::new(4))
            .map(|l| l.corners().len());
        let before_count = via_count(&board);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1);
        assert!(!report.rows[0].landed, "no candidate passes the keepout");
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (200_000, 150_000),
            "the row's to == from (the honest stop)"
        );
        assert_eq!(board.drill_center(ItemId::new(5)), before_center);
        assert_eq!(
            board
                .trace_polyline(ItemId::new(3))
                .map(|l| l.corners().len()),
            before_arm1
        );
        assert_eq!(
            board
                .trace_polyline(ItemId::new(4))
                .map(|l| l.corners().len()),
            before_arm2
        );
        assert_eq!(via_count(&board), before_count, "count invariant");
    }

    /// **V7 (the no-via board).** Flag ON, zero rows — the sweep has
    /// nothing to attempt.
    #[test]
    fn v7_no_via_board_zero_rows() {
        let (manager, mut board) = world();
        let mut manager = manager;
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(!report.gated);
        assert!(report.rows.is_empty(), "no vias, no rows");
    }

    /// **V8 (determinism ×2).** Fresh worlds, byte-equal reports.
    #[test]
    fn v8_determinism_times_two() {
        let (mut m1, mut b1) = via_world(260_000, Some(240_000));
        let (mut m2, mut b2) = via_world(260_000, Some(240_000));
        let r1 = run_gloss_via_place_stage(&mut m1, &mut b1);
        let r2 = run_gloss_via_place_stage(&mut m2, &mut b2);
        assert_eq!(r1, r2, "two fresh runs agree");
    }

    /// **V9 (the gated world).** The GATED_WORLD (an open net at stage
    /// entry) short-circuits: `gated = true`, zero rows.
    #[test]
    fn v9_incompletes_gate_marks_the_hold() {
        let (mut manager, mut board) = parse(GATED_WORLD);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(report.gated, "the hold is marked");
        assert!(report.rows.is_empty());
    }

    /// **V10 (the ARM_TOTAL rank arm, spec r1 NIT-5).** The 45° world:
    /// arm 1 F.Cu straight (140_000,200_000)→(200_000,200_000); arm 2
    /// B.Cu staircase (200_000,200_000)→(260_000,200_000)→(260_000,
    /// 260_000); the via at cur (200_000,200_000). The alignment
    /// family yields five in-radius legal candidates; THREE are
    /// keepout-blocked — (140_000,140_000) (box 130_000..145_000 ²
    /// covers the via pad and the y=x diagonal arm into it),
    /// (140_000,260_000) (box 130_000..150_000 × 250_000..270_000
    /// covers arm 1's vertical extension), and (260_000,200_000) (box
    /// 250_000..270_000 × 190_000..210_000 covers arm 1's horizontal
    /// extension; the box also clips arm 2's pre-existing horizontal
    /// tail — a harmless keepout/trace overlap the pass never probes).
    /// EXACTLY TWO candidates pass every acceptance gate, both at
    /// ret_drop 0 (pour-free — the neutral face), with DIFFERENT arm
    /// totals: P_win = (200_000,260_000) (disp (0,60_000), total
    /// √2·60_000 + 60_000 ≈ 144_853) and P_short-displacement =
    /// (170_000,170_000) (disp (−30_000,−30_000), total √2·30_000 +
    /// √2·90_000 = √2·120_000 ≈ 169_706). The rank's arm_total arm
    /// alone decides:
    /// P_win lands — the ENUMERATION order would take (170_000,
    /// 170_000) first (Chebyshev 30_000 < 60_000), which is exactly
    /// what the rank-swap mutant produces, so the mutant dies on this
    /// pin's literal to_x/to_y. None of the keepouts touches the two
    /// survivors' arms or pads.
    #[test]
    fn v10_arm_total_rank_discriminates() {
        let mut ses = base_ses();
        // The FORTYFIVE_DEGREE restriction: the diagonal candidates of
        // the alignment family are enumerated on this board.
        ses.rules.trace_angle_restriction = epic_dsn::state::AngleRestriction::FortyfiveDegree;
        ses.padstacks.push(via_padstack());
        register_nets(&mut ses, &[("sig", false)]);
        straight_trace(
            &mut ses,
            3,
            1,
            vec![
                IntPoint::new(140_000, 200_000),
                IntPoint::new(200_000, 200_000),
            ],
        );
        ses.push_routed_item(ItemIr::Trace {
            id: 4,
            trace: TraceIr {
                layer_no: 1,
                half_width: 2000,
                corners: vec![
                    IntPoint::new(200_000, 200_000),
                    IntPoint::new(260_000, 200_000),
                    IntPoint::new(260_000, 260_000),
                ],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(200_000, 200_000),
                    IntPoint::new(260_000, 200_000),
                    IntPoint::new(260_000, 260_000),
                ]),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        });
        ses.push_routed_item(ItemIr::Via {
            id: 5,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(200_000, 200_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        for (id, x0, y0, x1, y1) in [
            (7u32, 130_000i64, 130_000i64, 145_000i64, 145_000i64),
            (8, 130_000, 250_000, 150_000, 270_000),
            (9, 250_000, 190_000, 270_000, 210_000),
        ] {
            ses.push_routed_item(ItemIr::Keepout {
                id: i32::try_from(id).unwrap_or(0),
                keepout: KeepoutIr {
                    kind: KeepoutKindIr::Keepout,
                    layer_no: 0,
                    area: AreaIr::simple(DsnBoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(
                                i32::try_from(x0).unwrap_or(0),
                                i32::try_from(y0).unwrap_or(0),
                            ),
                            IntPoint::new(
                                i32::try_from(x1).unwrap_or(0),
                                i32::try_from(y1).unwrap_or(0),
                            ),
                        )),
                    ))),
                    clearance_class: 1,
                    fixed: FixedStateIr::Unfixed,
                    component_id: 0,
                    rotation: 0.0,
                    side_changed: false,
                    name: None,
                    translation: IntPoint::new(0, 0),
                },
            });
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1, "exactly one attempted via");
        assert!(report.rows[0].landed, "a candidate lands");
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (200_000, 260_000),
            "the SHORTER-arm candidate wins the equal-ret_drop rank"
        );
        assert_eq!(via_count(&board), 1, "via count invariant");
    }

    /// **V11 (the TWO-VIA world: corner-arm sole trigger + the
    /// no-restart sweep + the far-anchor observable, quality r1 F-3).**
    /// One CONNECTED net "sig", two vias, sweep order id 5 then id 6:
    /// - via A (id 5) at (200_000,200_000): arm 1 F.Cu straight
    ///   (140_000,200_000)->(200_000,200_000); arm 2 B.Cu staircase
    ///   (200_000,200_000)->(260_000,200_000)->(260_000,260_000).
    ///   Current total 60_000+120_000 = 180_000, corners 2+3 = 5. A's
    ///   only angle-passing candidate is (260_000,200_000) (disp
    ///   (60_000,0)): new total 120_000+60_000 = 180_000 - EQUAL (the
    ///   arm-total arm does NOT fire), ret_drop 0 (pour-free), corners
    ///   4 < 5 - the CORNER-COUNT arm is the SOLE improvement trigger.
    ///   (The anchor-coincident candidate (140_000,200_000) ranks
    ///   FIRST (total |(120_000,60_000)| ~ 134_164 < 180_000) and is
    ///   probed first - and rejected at the zero-arm angle gate;
    ///   via_displacement_ok answers false for (0,0), so the
    ///   degenerate-arm guard behind it is defense-in-depth, redundant
    ///   for integer inputs by that ordering - see the guard comment.)
    ///   A lands at (260_000,200_000), length-neutral.
    /// - via B (id 6) at (260_000,150_000): arm 1 F.Cu
    ///   (140_000,200_000)->(140_000,150_000)->(260_000,150_000) (its
    ///   (140_000,200_000) endpoint is arm 1's far anchor - the world
    ///   is ONE connected component, so the incompletes gate holds);
    ///   arm 2 B.Cu (260_000,260_000)->(320_000,260_000)->(320_000,
    ///   150_000)->(260_000,150_000) (its (260_000,260_000) endpoint is
    ///   arm 2's far anchor). B's ONLY angle-passing candidate is the
    ///   SAME position A just landed on - (260_000,200_000) (disp
    ///   (0,50_000); new total 120_000+60_000 = 180_000 vs current
    ///   170_000+230_000 = 400_000 - the arm arm fires). A's landed VIA
    ///   PAD now occupies that point, so B's drill check (copper
    ///   sharing off - an overlapping via is an obstacle) REJECTS it:
    ///   B records the honest landed:false with to == from. That is
    ///   the observable SINGLE-PASS/NO-RESTART semantics: B is
    ///   attempted AFTER A, reads the post-A board, and its outcome
    ///   depends on A's landing.
    #[test]
    fn v11_two_via_world_sweep_order_and_corner_arm() {
        let mut ses = base_ses();
        ses.padstacks.push(via_padstack());
        register_nets(&mut ses, &[("sig", false)]);
        // via A's arms.
        straight_trace(
            &mut ses,
            3,
            1,
            vec![
                IntPoint::new(140_000, 200_000),
                IntPoint::new(200_000, 200_000),
            ],
        );
        ses.push_routed_item(ItemIr::Trace {
            id: 4,
            trace: TraceIr {
                layer_no: 1,
                half_width: 2000,
                corners: vec![
                    IntPoint::new(200_000, 200_000),
                    IntPoint::new(260_000, 200_000),
                    IntPoint::new(260_000, 260_000),
                ],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(200_000, 200_000),
                    IntPoint::new(260_000, 200_000),
                    IntPoint::new(260_000, 260_000),
                ]),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        });
        ses.push_routed_item(ItemIr::Via {
            id: 5,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(200_000, 200_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        // via B's arms - the shared endpoints (arm 1's far anchor
        // (140_000,200_000) with via A's arm 1; arm 2's far anchor
        // (260_000,260_000) with via A's arm 2) keep the net ONE
        // connected component, so the incompletes gate holds.
        ses.push_routed_item(ItemIr::Trace {
            id: 7,
            trace: TraceIr {
                layer_no: 0,
                half_width: 2000,
                corners: vec![
                    IntPoint::new(140_000, 200_000),
                    IntPoint::new(140_000, 150_000),
                    IntPoint::new(260_000, 150_000),
                ],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(140_000, 200_000),
                    IntPoint::new(140_000, 150_000),
                    IntPoint::new(260_000, 150_000),
                ]),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        });
        ses.push_routed_item(ItemIr::Trace {
            id: 8,
            trace: TraceIr {
                layer_no: 1,
                half_width: 2000,
                corners: vec![
                    IntPoint::new(260_000, 260_000),
                    IntPoint::new(320_000, 260_000),
                    IntPoint::new(320_000, 150_000),
                    IntPoint::new(260_000, 150_000),
                ],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(260_000, 260_000),
                    IntPoint::new(320_000, 260_000),
                    IntPoint::new(320_000, 150_000),
                    IntPoint::new(260_000, 150_000),
                ]),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        });
        ses.push_routed_item(ItemIr::Via {
            id: 6,
            via: ViaIr {
                padstack_no: 1,
                location: IntPoint::new(260_000, 150_000),
                nets: vec![1],
                clearance_class: 1,
                fixed: DsnFixedStateIr::Unfixed,
                attach_smd_allowed: false,
            },
        });
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(!report.gated, "the connected world is ungated");
        assert_eq!(report.rows.len(), 2, "both vias attempted, one row each");
        // Row 1 = via A (id 5): the corner-count arm is the SOLE
        // improvement (ret_drop 0, new total == current total), and the
        // landing is length-neutral.
        assert_eq!(report.rows[0].via_id, 5, "id-ascending sweep order");
        assert!(report.rows[0].landed, "A's corners-sole candidate lands");
        assert_eq!(
            (report.rows[0].from_x, report.rows[0].from_y),
            (200_000, 200_000)
        );
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (260_000, 200_000),
            "A lands on the corner-arm alone (equal totals)"
        );
        // Row 2 = via B (id 6): its sole candidate is A's landing spot;
        // the post-A board rejects it (the drill check vs A's pad) -
        // the honest landed:false, and the no-restart observable.
        assert_eq!(report.rows[1].via_id, 6, "id-ascending sweep order");
        assert!(
            !report.rows[1].landed,
            "B's sole candidate is occupied by A's landed pad"
        );
        assert_eq!(
            (report.rows[1].to_x, report.rows[1].to_y),
            (260_000, 150_000),
            "B's row records to == from (the honest stop)"
        );
        assert_eq!(via_count(&board), 2, "via count invariant");
    }

    fn via_arm(trace_id: u32, far: IntPoint, layer: i32) -> ViaArm {
        ViaArm {
            trace_id: ItemId::new(trace_id),
            far,
            layer,
            half_width: 2000,
            clearance_class: 1,
            current_length: 0.0,
            corner_count: 2,
        }
    }

    /// **V2 (the pure predicates, the DNR-16 exact edges).**
    /// Displacement legality per restriction; the radius inclusive edge
    /// (exactly 60_000 in, 60_001 out); the length-guard exact edge
    /// (old + GAIN lands, old + GAIN + 1 rejects).
    #[test]
    fn v2_predicates_exact_edges() {
        use epic_board::rules_surf::AngleRestriction as AR;
        assert!(via_displacement_ok(AR::None, 3, 7));
        assert!(!via_displacement_ok(AR::None, 0, 0));
        assert!(via_displacement_ok(AR::NinetyDegree, 60_000, 0));
        assert!(!via_displacement_ok(AR::NinetyDegree, 60_000, 10_000));
        assert!(via_displacement_ok(AR::FortyfiveDegree, 30_000, 30_000));
        assert!(!via_displacement_ok(AR::FortyfiveDegree, 30_000, 60_000));
        assert!(via_radius_allows(60_000, 0));
        assert!(via_radius_allows(0, 60_000));
        assert!(!via_radius_allows(60_001, 0));
        assert!(!via_radius_allows(0, 60_001));
        // LITERAL boundary (DNR-16): the guard reads the constant, the
        // pin reads 40_000/40_001 — a gain mutant of either sign flips
        // exactly one verdict.
        assert!(via_length_guard_ok(1_000_000.0, 1_040_000.0));
        assert!(!via_length_guard_ok(1_000_000.0, 1_040_001.0));
    }

    /// **V3 (the OFF-BEND world, the at-radius landing).** The in-line
    /// candidate (260_000, 150_000) — Chebyshev displacement EXACTLY
    /// [`VIA_PLACE_RADIUS_DBU`] — lands; the exact landing geometry is
    /// asserted; the via COUNT is invariant.
    #[test]
    fn v3_off_bend_lands_at_radius_edge() {
        let (mut manager, mut board) = via_world(260_000, None);
        let before = via_count(&board);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(!report.gated);
        assert_eq!(report.rows.len(), 1, "exactly one attempted via");
        let row = &report.rows[0];
        assert!(row.landed, "the in-line candidate lands");
        assert_eq!((row.from_x, row.from_y), (200_000, 150_000));
        assert_eq!((row.to_x, row.to_y), (260_000, 150_000));
        assert_eq!(
            board.drill_center(ItemId::new(5)),
            Some(Point::Int(IntPoint::new(260_000, 150_000))),
            "the via center moved to the in-line position"
        );
        assert_eq!(
            board
                .trace_polyline(ItemId::new(3))
                .map(|lines| lines.corners().len()),
            Some(2),
            "arm 1 straightened to a single segment"
        );
        let arm2 = board.trace_polyline(ItemId::new(4)).expect("arm 2 live");
        assert_eq!(arm2.corners().len(), 2, "arm 2 straightened");
        assert_eq!(via_count(&board), before, "via COUNT never changes");
    }

    /// **V4 (the radius+1 world).** The in-line position sits ONE DBU
    /// past the radius — NOT enumerated, no landing, geometry
    /// verbatim. Kills the R+1 mutant (the candidate becomes
    /// enumerated and would land); the R−1 mutant is killed by V3.
    #[test]
    fn v4_radius_plus_one_not_enumerated() {
        let (mut manager, mut board) = via_world(260_001, None);
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 1);
        assert!(!report.rows[0].landed, "radius+1 is outside the lattice");
        assert_eq!(
            board.drill_center(ItemId::new(5)),
            Some(Point::Int(IntPoint::new(200_000, 150_000))),
            "the via did not move"
        );
    }

    /// **V5 (the return-path scoring, the strict face + the ±1 DBU
    /// boundary triple).** The SPLIT-RUN plane world: both anchors on
    /// y = 150_000 (arm 1 (30_000,150_000)→(200_000,150_000); arm 2
    /// (200_000,150_000)→(260_000,150_000)), so all 12 on-line lattice
    /// candidates are both-arms-straight with a CONSTANT arm total
    /// (230_000) — the return-path rank alone discriminates. Pour left
    /// edge b (F.Cu rect x ≥ b, y in [100_000, 200_000]): the
    /// ret_drop of a dx candidate is max(0, b−200_000) when
    /// 200_000+dx ≥ b, else dx. Strict face (b = 240_000): the
    /// plane-proximal dx=40_000 wins (ret_drop 40_000) over the
    /// enumeration-cheb rank that would take dx=10_000 — the scoring
    /// pin. Tie face (b = 230_000): dx ≥ 30_000 tie at ret_drop
    /// 30_000 → the Chebyshev tiebreak takes dx=30_000 → (230_000,
    /// 150_000). +1 face (b = 230_001): dx=30_000's ret_drop drops to
    /// 30_000, dx ≥ 40_000 tie at 30_001 → the winner FLIPS to
    /// (240_000, 150_000) — one DBU of pour geometry flips the
    /// verdict (the DNR-16 exact-boundary triple). −1 face
    /// (b = 229_999): dx ≥ 30_000 tie at 29_999 → (230_000, 150_000)
    /// — the tie face's winner holds inside the boundary.
    #[test]
    fn v5_return_path_scoring_and_tie_boundary() {
        // The STRICT face (b = 240_000): the plane-proximal candidate
        // wins over the nearest candidate.
        let (mut manager, mut board) = via_world_split(Some(240_000));
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(report.rows[0].landed, "strict face lands");
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (240_000, 150_000),
            "the plane-proximal candidate wins"
        );
        // The TIE face (b = 230_000).
        let (mut manager, mut board) = via_world_split(Some(230_000));
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(report.rows[0].landed);
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (230_000, 150_000)
        );
        // The +1 face (b = 230_001): the winner flips by one DBU of
        // pour geometry.
        let (mut manager, mut board) = via_world_split(Some(230_001));
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(report.rows[0].landed);
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (240_000, 150_000),
            "+1 DBU flips the tie"
        );
        // The −1 face (b = 229_999): the tie face's winner holds.
        let (mut manager, mut board) = via_world_split(Some(229_999));
        let report = run_gloss_via_place_stage(&mut manager, &mut board);
        assert!(report.rows[0].landed);
        assert_eq!(
            (report.rows[0].to_x, report.rows[0].to_y),
            (230_000, 150_000),
            "−1 DBU holds the tie winner"
        );
    }

    // -- M8-T6: the TEARDROPS pass (literal DSN worlds, the P10
    //    parse face) ----------------------------------------------

    /// The TD world (NinetyDegree): net "sig" — pin A1 (3000,15000) →
    /// wire to the junction via at (15000,15000) → wire on to pin A2
    /// (45000,15000), all on F.Cu, trace width 200 DSN (board
    /// half-width 1000). The junction via carries the BIG padstack
    /// (radius 1200 DSN → board diameter 24_000); the pins carry the
    /// small PAD padstack (radius 1000 DSN → board diameter 20_000).
    /// Junctions (deduped): A1, the via, A2 — the via center is the
    /// endpoint of BOTH wires and the first attempt wins. Net "other"
    /// (when present) is B1 → B2, a straight F.Cu run whose y sits
    /// `delta_dsn` DSN above the via taper's widest edge — the
    /// clearance-boundary family: at gap exactly 200 DSN (the
    /// clearance) the probe blocks, at +1 DSN (10 board DBU of slack)
    /// it clears.
    fn td_world(delta_dsn: Option<i64>) -> (SearchTreeManager, Board) {
        let delta = delta_dsn.unwrap_or(10_000);
        let dsn = format!(
            r#"
(pcb td_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 600000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 3000 15000 Front 0.000000))
    (component "IA" (place "A2" 45000 15000 Front 0.000000))
    (component "IB" (place "B1" 12500 {delta} Front 0.000000))
    (component "IB" (place "B2" 48000 {delta} Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (image "IB" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
      (attach off)
    )
    (padstack "TVIA"
      (shape (circle F.Cu 1200 0 0))
      (shape (circle B.Cu 1200 0 0))
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
    (net "sig" (pins "A1"-"P" "A2"-"P"))
    (net "other" (pins "B1"-"P" "B2"-"P"))
    (class kicad_default "sig" "other" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 3000 15000 15000 15000) (net "sig"))
    (wire (path F.Cu 200 15000 15000 45000 15000) (net "sig"))
    (via TVIA 15000 15000 (net "sig"))
    (wire (path F.Cu 200 12500 {delta} 48000 {delta}) (net "other"))
  )
)
"#
        );
        let (manager, board) = parse(&dsn);
        // The via's BIG padstack comes from the DSN's `via TVIA ...`
        // line — no in-test padstack swap is needed.
        (manager, board)
    }

    /// **TD1 (the junction world, the exact-geometry + DRC-clean
    /// pin).** Five junctions land (A1, the via, A2 for "sig" — the
    /// via's second endpoint is deduped — and B1, B2 for "other"), and
    /// each landed taper is the [`graded_wire_plan`] VERBATIM (backs
    /// 4000/8000/12_000; pin tapers hw 1000 → 5000, the via taper hw
    /// 1000 → 6000). The board carries ZERO clearance violations after
    /// the pass (the DRC-clean law).
    #[test]
    fn td1_junction_world_exact_geometry_and_drc_clean() {
        let (mut manager, mut board) = td_world(None);
        let before = epic_drc::clearance::all_clearance_violations(&mut manager, &mut board).0;
        assert_eq!(before, 0, "the world parses clean");
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        assert!(!report.gated);
        // Net 1 "sig": A1, the via, A2 (the via's second endpoint is
        // deduped); net 2 "other": B1, B2. All five land in the clean
        // world; the sweep order is net asc.
        assert_eq!(report.rows.len(), 5, "3 sig junctions + 2 other");
        assert!(report.rows.iter().all(|row| row.landed));
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| (row.net, row.at_x, row.at_y))
                .collect::<Vec<_>>(),
            [
                (1, 30_000, 150_000),
                (1, 150_000, 150_000),
                (1, 450_000, 150_000),
                (2, 125_000, 100_000),
                (2, 480_000, 100_000),
            ]
        );
        // The exact tapers, per junction, read off the board: every
        // new ShoveFixed trace is one graded wire.
        let mut tapers: Vec<(i64, i64, i64, i64)> = Vec::new(); // (x1,y1,x2,y2)
        let mut widths: Vec<i32> = Vec::new();
        for entry in board.iter_ascending() {
            if !matches!(&entry.data, epic_board::items::ItemData::Trace { .. }) {
                continue;
            }
            let id = entry.id;
            let Some(epic_board::items::FixedState::ShoveFixed) = board.get(id).map(|e| e.fixed)
            else {
                continue; // the parsed wires are Unfixed
            };
            let lines = board.trace_polyline(id).expect("trace polyline");
            let corners = lines.corners();
            assert_eq!(corners.len(), 2, "one graded wire = one segment");
            let (a, b) = (corners[0].clone(), corners[1].clone());
            let point = |c: &epic_geometry::point::Point| match c {
                epic_geometry::point::Point::Int(p) => (i64::from(p.x), i64::from(p.y)),
                _ => panic!("int corner"),
            };
            let (mut p, mut q) = (point(&a), point(&b));
            if p > q {
                std::mem::swap(&mut p, &mut q);
            }
            tapers.push((p.0, p.1, q.0, q.1));
            widths.push(board.trace_half_width(id).expect("half width"));
        }
        tapers.sort();
        widths.sort();
        // Pin junctions: A1 (30_000,150_000) taper runs +x; A2
        // (450_000,150_000) taper runs −x; via junction (150_000,
        // 150_000) runs +x (wire 2's start loses the dedup).
        let expected = [
            (34_000, 150_000, 30_000, 150_000),   // A1 k=1
            (38_000, 150_000, 30_000, 150_000),   // A1 k=2
            (42_000, 150_000, 30_000, 150_000),   // A1 k=3
            (146_000, 150_000, 150_000, 150_000), // via k=1 (wire 1's endpoint wins the dedup: toward A1)
            (142_000, 150_000, 150_000, 150_000), // via k=2
            (138_000, 150_000, 150_000, 150_000), // via k=3
            (446_000, 150_000, 450_000, 150_000), // A2 k=1
            (442_000, 150_000, 450_000, 150_000), // A2 k=2
            (438_000, 150_000, 450_000, 150_000), // A2 k=3
            (129_000, 100_000, 125_000, 100_000), // B1 k=1
            (133_000, 100_000, 125_000, 100_000), // B1 k=2
            (137_000, 100_000, 125_000, 100_000), // B1 k=3
            (468_000, 100_000, 480_000, 100_000), // B2 k=3
            (472_000, 100_000, 480_000, 100_000), // B2 k=2
            (476_000, 100_000, 480_000, 100_000), // B2 k=1
        ];
        let mut expected = expected.to_vec();
        for (x1, y1, x2, y2) in &mut expected {
            if (*x1, *y1) > (*x2, *y2) {
                std::mem::swap(x1, x2);
                std::mem::swap(y1, y2);
            }
        }
        expected.sort();
        assert_eq!(tapers, expected, "the exact taper geometry");
        // The graded half-widths, sorted. The DSN circle values are
        // DIAMETERS (the reader divides by two): pin pads D = 10_000
        // board (r = 5_000), the via pad D = 12_000 (r = 6_000). Pin
        // tapers run hw 1000 → 5000 ({2333, 3666, 5000} each, four
        // junctions); the via taper 1000 → 6000 ({2666, 4333, 6000}).
        assert_eq!(
            widths,
            vec![
                2_333, 2_333, 2_333, 2_333, 2_666, 3_666, 3_666, 3_666, 3_666, 4_333, 5_000, 5_000,
                5_000, 5_000, 6_000
            ]
        );
        let after = epic_drc::clearance::all_clearance_violations(&mut manager, &mut board).0;
        assert_eq!(after, 0, "a teardrop never introduces a violation");
    }

    /// The FLAT world: net "sig" — pin A1 (3000,15000) → A2
    /// (45000,15000), trace width 1000 DSN (board W = 10_000) into pin
    /// pads of board diameter exactly 10_000: W == D at every junction
    /// — the anatomy edge — so the ON stage is a NO-OP (the
    /// no-teardrop world).
    const TD_FLAT_WORLD: &str = r#"
(pcb td_flat_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 600000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 3000 15000 Front 0.000000))
    (component "IA" (place "A2" 45000 15000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
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
    (net "sig" (pins "A1"-"P" "A2"-"P"))
    (class kicad_default "sig" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 1000 3000 15000 45000 15000) (net "sig"))
  )
)
"#;

    /// The GATED world: the TD junctions plus an OPEN net (two pins,
    /// no wire) — an incomplete at stage entry short-circuits the pass
    /// into the `gated` hold.
    const TD_GATED_WORLD: &str = r#"
(pcb td_gated_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 600000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 3000 15000 Front 0.000000))
    (component "IA" (place "A2" 45000 15000 Front 0.000000))
    (component "IO" (place "O1" 3000 25000 Front 0.000000))
    (component "IO" (place "O2" 45000 25000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (image "IO" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
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
    (net "sig" (pins "A1"-"P" "A2"-"P"))
    (net "open" (pins "O1"-"P" "O2"-"P"))
    (class kicad_default "sig" "open" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 3000 15000 45000 15000) (net "sig"))
  )
)
"#;

    /// **TD2 (the DNR-16 anatomy edge, BOTH directions).** The pure-fn
    /// pins: width == diameter rejects, ONE unit narrower accepts, one
    /// unit wider rejects; both ±1 constant mutations die here (100 →
    /// 99 kills the narrow-accepts pin, 100 → 101 kills the
    /// equal-rejects pin). The plan fn is pinned verbatim too. The
    /// BOARD face of the edge: the FLAT world (W == D everywhere) is a
    /// no-op — empty report, trace count unchanged — and a 20-board-DBU
    /// narrower trace (the DSN grid's one-step-narrower face) tapers.
    #[test]
    fn td2_anatomy_edge_both_directions() {
        // The exact ±1 faces (pure fn, no grid constraint).
        assert!(
            teardrop_anatomy_ok(1_999, 2_000),
            "one unit narrower accepts"
        );
        assert!(
            !teardrop_anatomy_ok(2_000, 2_000),
            "width == diameter rejects"
        );
        assert!(!teardrop_anatomy_ok(2_001, 2_000), "wider rejects");
        // The plan fn verbatim (the granularity constants' pin).
        assert_eq!(
            graded_wire_plan(1_000, 5_000, 100_000, 3, 4_000),
            vec![(4_000, 2_333), (8_000, 3_666), (12_000, 5_000)]
        );
        // The CLAMP face: `seg_len_cheb < N·step` collapses the inner
        // wires' backs to the segment length (duplicate backs,
        // still-graded widths — the same-net overlap is legal, the
        // silhouette still tapers).
        assert_eq!(
            graded_wire_plan(1_000, 5_000, 5_000, 3, 4_000),
            vec![(4_000, 2_333), (5_000, 3_666), (5_000, 5_000)]
        );
        // The board face: W == D → the no-teardrop world.
        let (mut manager, mut board) = parse(TD_FLAT_WORLD);
        let traces_before = board
            .iter_ascending()
            .filter(|entry| matches!(&entry.data, epic_board::items::ItemData::Trace { .. }))
            .count();
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        assert!(report.is_empty() && !report.gated, "the no-op world");
        let traces_after = board
            .iter_ascending()
            .filter(|entry| matches!(&entry.data, epic_board::items::ItemData::Trace { .. }))
            .count();
        assert_eq!(traces_before, traces_after, "no wire written at the edge");
    }

    /// **TD3 (the REAL-counter clearance boundary).** The foreign run
    /// ("other") hovers over the via junction's widest taper wire. The
    /// probe's own verdict IS the boundary (no hand-rolled formula).
    /// Calibrated by sweep: the taper BLOCKS at a foreign-edge gap of
    /// clearance + 10 board DBU and LANDS at clearance + 20 — the
    /// octagon offset arithmetic of the plain tree puts the accept
    /// edge there, and exactly-at-clearance is still blocked. Both
    /// worlds carry ZERO clearance violations in BOTH faces (the
    /// DRC-clean law: the landed taper sits strictly inside the
    /// probe's own legal envelope).
    #[test]
    fn td3_clearance_boundary_worlds() {
        // The LAND face: gap == clearance + 20 board DBU (centerline
        // 15_902 DSN = taper edge 15_600 + clearance 200 + foreign
        // half 100 + 2 DSN of slack).
        let (mut manager, mut board) = td_world(Some(15_902));
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        let via_row = report
            .rows
            .iter()
            .find(|row| row.net == 1 && row.at_x == 150_000 && row.at_y == 150_000)
            .expect("the via junction row");
        assert!(via_row.landed, "the calibrated accept face lands");
        assert_eq!(
            epic_drc::clearance::all_clearance_violations(&mut manager, &mut board).0,
            0,
            "the landed taper introduces no violation"
        );
        // The BLOCKED face: one DSN grid step tighter (gap ==
        // clearance + 10 board DBU).
        let (mut manager, mut board) = td_world(Some(15_901));
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        let via_row = report
            .rows
            .iter()
            .find(|row| row.net == 1 && row.at_x == 150_000 && row.at_y == 150_000)
            .expect("the via junction row");
        assert!(!via_row.landed, "one grid step tighter blocks the taper");
        assert_eq!(
            epic_drc::clearance::all_clearance_violations(&mut manager, &mut board).0,
            0,
            "blocked world stays clean"
        );
    }

    /// **TD4 (the budget boundary at the budgeted entry).** Budget 0:
    /// no junction attempted (no rows, no wires); budget 1: exactly
    /// the sweep's first junction lands (A1), every later junction
    /// silently skipped.
    #[test]
    fn td4_budget_boundary_worlds() {
        let (mut manager, mut board) = td_world(None);
        let traces_before = board
            .iter_ascending()
            .filter(|entry| matches!(&entry.data, epic_board::items::ItemData::Trace { .. }))
            .count();
        let report = run_gloss_teardrops_stage_budgeted(&mut manager, &mut board, 0);
        assert!(report.is_empty(), "budget 0: nothing attempted");
        assert_eq!(
            board
                .iter_ascending()
                .filter(|entry| matches!(&entry.data, epic_board::items::ItemData::Trace { .. }))
                .count(),
            traces_before,
            "budget 0: no wire written"
        );
        let (mut manager, mut board) = td_world(None);
        let report = run_gloss_teardrops_stage_budgeted(&mut manager, &mut board, 1);
        assert_eq!(report.rows.len(), 1, "budget 1: only the first junction");
        assert!(report.rows[0].landed);
        assert_eq!(
            (report.rows[0].at_x, report.rows[0].at_y),
            (30_000, 150_000),
            "the first sweep junction is A1"
        );
    }

    /// **TD6 (the gated marker).** An open net at stage entry
    /// short-circuits into the DISTINCT gated hold (the T3 sibling-key
    /// lesson) — no rows, the pass never runs. (There is no unit td5:
    /// the flag-OFF board-level byte-invariance face is folded into the
    /// harness pin `teardrop_default_face_rotates_nothing` plus the
    /// seven default-face compares — a stronger witness than a unit
    /// pin.)
    #[test]
    fn td6_gated_world_marks_the_hold() {
        let (mut manager, mut board) = parse(TD_GATED_WORLD);
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        assert!(report.gated, "the hold is marked");
        assert!(report.rows.is_empty(), "the gate short-circuits first");
        assert!(!GlossTeardropsReport::default().gated);
    }

    /// **TD7 (determinism ×2).** Two fresh parses, two stage runs —
    /// identical reports and identical landed geometry.
    #[test]
    fn td7_determinism_x2() {
        let run = || {
            let (mut manager, mut board) = td_world(None);
            let report = run_gloss_teardrops_stage(&mut manager, &mut board);
            let mut tapers: Vec<String> = Vec::new();
            for entry in board.iter_ascending() {
                if !matches!(&entry.data, epic_board::items::ItemData::Trace { .. }) {
                    continue;
                }
                if !matches!(
                    board.get(entry.id).map(|e| e.fixed),
                    Some(epic_board::items::FixedState::ShoveFixed)
                ) {
                    continue;
                }
                let lines = board.trace_polyline(entry.id).expect("polyline");
                let half = board.trace_half_width(entry.id).expect("half width");
                tapers.push(format!("{:?} w={half}", lines.corners()));
            }
            tapers.sort();
            (report.rows, tapers)
        };
        let (rows1, tapers1) = run();
        let (rows2, tapers2) = run();
        assert_eq!(rows1, rows2, "rows deterministic");
        assert_eq!(tapers1, tapers2, "geometry deterministic");
        assert_eq!(tapers1.len(), 15, "all five junctions' wires present");
    }

    /// The SIBLING world (net "sig"): a WIDE trace (width 2000 DSN →
    /// board W = 20_000) and a NARROW trace (width 200 DSN → board W =
    /// 2_000) BOTH run from the empty point (5000,5000) to the junction
    /// via (15000,15000, TVIA: board D = 12_000); the junction's other
    /// wire continues to pin A1 (25000,15000). Parse order puts the
    /// WIDE trace at the LOWER id → the sweep attempts it at the via
    /// FIRST: its anatomy gate fails SILENTLY (W ≥ D, no row) — and
    /// with the fix-round-2 semantics a gate-rejected endpoint does
    /// NOT consume the junction, so the NARROW trace's endpoint at the
    /// same center still lands. A1 (PAD, board D = 10_000) teardrops
    /// off the narrow trace too.
    const TD_SIBLING_WORLD: &str = r#"
(pcb td_sibling_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 600000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 25000 15000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
      (attach off)
    )
    (padstack "TVIA"
      (shape (circle F.Cu 1200 0 0))
      (shape (circle B.Cu 1200 0 0))
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
    (net "sig" (pins "A1"-"P"))
    (class kicad_default "sig" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 2000 5000 5000 15000 15000) (net "sig"))
    (wire (path F.Cu 200 5000 5000 15000 15000) (net "sig"))
    (wire (path F.Cu 200 15000 15000 25000 15000) (net "sig"))
    (via TVIA 15000 15000 (net "sig"))
  )
)
"#;

    /// **TD8 (the dedup consumes the junction only on a RECORDED
    /// attempt).** Exactly ONE row at the via center (the narrow
    /// trace's, landed), plus A1's own row — the wide trace's
    /// gate-rejected endpoints produced NO rows and consumed NOTHING.
    /// On the pre-fix semantics this pin dies (the wide attempt's
    /// done.insert would suppress the narrow landing).
    #[test]
    fn td8_gate_fail_sibling_lands() {
        let (mut manager, mut board) = parse(TD_SIBLING_WORLD);
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        assert_eq!(
            report.rows.len(),
            2,
            "the narrow trace's via + A1 rows — the wide trace silent"
        );
        assert!(report.rows.iter().all(|row| row.landed));
        let via_row = report
            .rows
            .iter()
            .find(|row| row.at_x == 150_000 && row.at_y == 150_000)
            .expect("the via junction row exists");
        assert!(via_row.landed, "the sibling landing at the shared center");
        // The teardrop is the NARROW trace's: via taper hw {2666,
        // 4333, 6000} (r = 6_000), A1 taper hw {2333, 3666, 5000}.
        let mut widths: Vec<i32> = Vec::new();
        for entry in board.iter_ascending() {
            if !matches!(&entry.data, epic_board::items::ItemData::Trace { .. }) {
                continue;
            }
            if !matches!(
                board.get(entry.id).map(|e| e.fixed),
                Some(epic_board::items::FixedState::ShoveFixed)
            ) {
                continue;
            }
            widths.push(board.trace_half_width(entry.id).expect("half width"));
        }
        widths.sort();
        assert_eq!(
            widths,
            vec![2_333, 2_666, 3_666, 4_333, 5_000, 6_000],
            "the narrow trace's two tapers — nothing from the wide one"
        );
    }

    /// **TD9 (the angle gate's edge table, pure fn).** The axis faces
    /// and BOTH directions of the exact-45° edge, plus sign variants:
    /// (0,0) rejects (the total gate), axis segments accept, exact
    /// diagonals accept, ONE-OFF diagonals reject in all four
    /// quadrants.
    #[test]
    fn td9_axis_gate_edges() {
        assert!(!teardrop_axis_ok(0, 0), "(0,0) rejects");
        assert!(teardrop_axis_ok(10, 0), "axis accepts");
        assert!(teardrop_axis_ok(0, 10), "axis accepts");
        assert!(teardrop_axis_ok(7, 7), "exact diagonal accepts");
        assert!(!teardrop_axis_ok(7, 8), "one-off diagonal rejects (+,+)");
        assert!(!teardrop_axis_ok(8, 7), "one-off diagonal rejects (+,+)");
        assert!(teardrop_axis_ok(-7, 7), "exact diagonal accepts (−,+)");
        assert!(teardrop_axis_ok(7, -7), "exact diagonal accepts (+,−)");
        assert!(teardrop_axis_ok(-7, -7), "exact diagonal accepts (−,−)");
        assert!(!teardrop_axis_ok(-7, -8), "one-off diagonal rejects (−,−)");
    }

    /// The DIAGONAL world (net "sig"): the wire A1 (5000,15000) →
    /// (9000,19000) is an EXACT 45° end segment (board dx = dy =
    /// 40_000); it continues horizontally through the junction via at
    /// (21000,19000) to A2 (45000,19000). A1's taper steps
    /// Chebyshev-along the DIAGONAL — both axis signs stepped — and
    /// lands; zero violations after.
    const TD_DIAGONAL_WORLD: &str = r#"
(pcb td_diagonal_world.dsn
  (parser (string_quote ") (space_in_quoted_tokens on))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 600000 300000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (placement
    (component "IA" (place "A1" 5000 15000 Front 0.000000))
    (component "IA" (place "A2" 45000 19000 Front 0.000000))
  )
  (library
    (image "IA" (pin "PAD" "P" 0 0))
    (padstack "PAD"
      (shape (circle F.Cu 1000 0 0))
      (shape (circle B.Cu 1000 0 0))
      (attach off)
    )
    (padstack "TVIA"
      (shape (circle F.Cu 1200 0 0))
      (shape (circle B.Cu 1200 0 0))
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
    (net "sig" (pins "A1"-"P" "A2"-"P"))
    (class kicad_default "sig" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 5000 15000 9000 19000) (net "sig"))
    (wire (path F.Cu 200 9000 19000 21000 19000) (net "sig"))
    (wire (path F.Cu 200 21000 19000 45000 19000) (net "sig"))
    (via TVIA 21000 19000 (net "sig"))
  )
)
"#;

    /// **TD10 (the 45° board world).** A1's taper steps along the
    /// diagonal — starts at (54_000,154_000), (58_000,158_000),
    /// (62_000,162_000), all ending at A1 (50_000,150_000), half-widths
    /// {2333, 3666, 5000} — lands with ZERO clearance violations; the
    /// via and A2 junctions land too (3 rows).
    #[test]
    fn td10_diagonal_world_lands() {
        let (mut manager, mut board) = parse(TD_DIAGONAL_WORLD);
        let report = run_gloss_teardrops_stage(&mut manager, &mut board);
        assert_eq!(report.rows.len(), 3, "A1 (diagonal), the via, A2");
        assert!(report.rows.iter().all(|row| row.landed));
        // A1's diagonal taper, corner-verbatim.
        let mut diag: Vec<DiagWire> = Vec::new();
        for entry in board.iter_ascending() {
            if !matches!(&entry.data, epic_board::items::ItemData::Trace { .. }) {
                continue;
            }
            if !matches!(
                board.get(entry.id).map(|e| e.fixed),
                Some(epic_board::items::FixedState::ShoveFixed)
            ) {
                continue;
            }
            let lines = board.trace_polyline(entry.id).expect("polyline");
            let corners = lines.corners();
            let p = match &corners[0] {
                epic_geometry::point::Point::Int(pt) => (i64::from(pt.x), i64::from(pt.y)),
                _ => panic!("int corner"),
            };
            let q = match &corners[1] {
                epic_geometry::point::Point::Int(pt) => (i64::from(pt.x), i64::from(pt.y)),
                _ => panic!("int corner"),
            };
            let half = board.trace_half_width(entry.id).expect("half width");
            diag.push(((p.0, p.1), (q.0, q.1), half));
        }
        // Expect exactly the three DIAGONAL wires (via/A2 tapers are
        // axis-aligned — filter to the diagonal faces).
        let mut diag: Vec<DiagWire> = diag
            .into_iter()
            .filter(|(p, q, _)| p.0 != q.0 && p.1 != q.1)
            .collect();
        diag.sort();
        assert_eq!(
            diag,
            vec![
                ((54_000, 154_000), (50_000, 150_000), 2_333),
                ((58_000, 158_000), (50_000, 150_000), 3_666),
                ((62_000, 162_000), (50_000, 150_000), 5_000),
            ],
            "the diagonal taper, corner-verbatim"
        );
        assert_eq!(
            epic_drc::clearance::all_clearance_violations(&mut manager, &mut board).0,
            0,
            "zero violations after the diagonal landing"
        );
    }
}
