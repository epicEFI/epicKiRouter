//! Java `board/optimize/TraceTightener.java` — the changed-area
//! pull-tight layer the oracle runs after EVERY routed connection,
//! after tail removal, and after each fanout escape
//! (`RoutingBoardOperations.optChangedArea`, the [`PullTightSeam`]
//! consumption). M4-T3 ports the base class plus the 90° variant
//! ([`tightener90`]); M4-T4 adds the 45° variant ([`tightener45`]) —
//! the angle restriction of every tier fixture — and defers any-angle.
//!
//! ## The fixpoint (`TraceTightener.optChangedArea`, `:121-169`)
//!
//! `while somethingChanged` over the layers of the LIVE marking
//! session: per layer, take the region, `setEmpty` it BEFORE
//! processing (so the geometry mutations re-mark it for the next
//! sweep), enlarge by
//! `1.5 * (clearanceMatrix.maxValue(layer) + 2 * rules.maxTraceHalfWidth)`
//! — the RULES field, NOT the `BasicBoard` accumulator — query the
//! default tree, and per overlapping trace run
//! `PolylineTrace.pullTight` / `smoothenEndCornersAtTrace` (with the
//! break discipline: a keep-point split breaks the item loop; a
//! successful smoothen breaks it "because items may be removed").
//!
//! ## Named divergences (all disclosed in the M4-T3 report)
//!
//! * **Dead acid-trap body** — Java `avoidAcidTraps` opens with
//!   `if (true) return polyline;` (`:518-520`); the spring-over body
//!   is DEAD CODE and acid traps are NEVER avoided by the oracle.
//!   Ported as the identity verdict; the dead body is quoted, not
//!   ported ([`TraceTightener::avoid_acid_traps`]).
//! * **Wall→tick budget** — Java's `timeLimit` is a wall-clock
//!   [`TimeLimit`]; behind the `deterministic_budgets` profile the
//!   port spends the same limit as `is_stop_requested` consultation
//!   ticks with the same strict `>` (the T12 [`crate::engine`-side
//!   `RouteBudget`] pattern). Construction keeps Java's
//!   `timeLimit > 0` gate.
//! * **Reference vs value equality** — the 90° `pullTight` loop
//!   condition and `PolylineTrace.pullTight`'s change test are Java
//!   REFERENCE comparisons; the port compares values. Every no-change
//!   path returns the input object, so equal-value fresh objects are
//!   argued unreachable; the proxy is documented at each site.
//! * **`changeEntries` collapse** — Java `PolylineTrace.change`
//!   reuses search-tree leaves for performance
//!   (`ShapeSearchTree.change_entries`); the port reaches the same
//!   end state through the remove+reinsert collapse
//!   (`replace_geometry`, the `reuse_entries_after_cutout` precedent).
//!   The `keepAtStartCount`/`keepAtEndCount` scan results are dead in
//!   the collapse; the diff scans are kept for their two early-out
//!   faces ("both polylines equal, no change necessary").
//! * **`additionalUpdateAfterChange`** — EMPTY in `BasicBoard`
//!   (`:1228`), a GUI-subclass hook; nothing to port.
//! * **`notifyChanged` observer** — no headless consumer (bug-118
//!   convention); the change is broadcast through the tree manager
//!   only.
//! * **`normalize` try/catch** — Java swallows a normalization
//!   exception into `FRLogger.error` and keeps the changed polyline;
//!   the port's `normalize` returns a bool and cannot throw, so the
//!   catch is unrepresentable (the result is consumed the same way:
//!   the change stands either way).
//! * **ViaOptimizer hook (T5)** — Java relocates vias inside the
//!   changed region when `traceCosts != null`
//!   (`ViaOptimizer.optViaLocation`, `:160-165`); the port lives in
//!   [`via_optimizer`] behind [`opt_via_location_seam`] (recursion
//!   budget 10, the production call site literal). The plane face of
//!   `optPlaneOrFanoutVia` is the M6 stub (no pours on tier fixtures);
//!   the fanout and 2-trace cost faces are real.
//! * **45° variant (T4)** — `TraceTightener45` (674 l) is ported in
//!   [`tightener45`]: the diagonal `pullTight` fixpoint
//!   (reduce-corner → smoothen → reposition), the clip-gated
//!   corner-reduction arms, the REAL smoothen bodies the 90° variant
//!   only stubs, and the at-trace start/end corner overrides.
//!   [`TraceTightenerSeam`] ACTIVATES on 45° and 90° boards.
//! * **Any-angle variant (deferred)** — Java `getInstance` dispatches
//!   `AngleRestriction.NONE` → `TraceTightenerAnyAngle` (~800 l with
//!   its own reposition/avoid-acid faces and the `c_max_cos_angle`
//!   consumer); the census found 0 any-angle fixtures (the parse
//!   default is FORTYFIVE_DEGREE), so T4 defers it: [`active_for`]
//!   keeps the no-op gate on `NONE` and the `pull_tight_polyline`
//!   identity fallback is unreachable through the seam. A future
//!   any-angle fixture fails loudly at the dispatch pin.
//! * **The `PolylineTrace.pullTight` pin-connection tail (LANDED, T6)** —
//!   Java `swapConnectionToPin`/`correctConnectionToPin`
//!   (`PolylineTrace.java:842-855`, gated
//!   `angleRestriction != NINETY_DEGREE && pinEdgeToTurnDist > 0`) is
//!   LIVE on every 45° board (the parser defaults `pinEdgeToTurnDist`
//!   to `minTraceHalfWidth`, `Structure.java:667-668`). Deferred in T4
//!   (the tail-deferral counter-witness pin isolated the divergence to
//!   exactly this face); the port lives in [`pin_tail`], and that pin
//!   now asserts the jar's split geometry instead of the byte-exact
//!   parse.
//! * **`joinGraphicsUpdateBox`** — GUI update box, dropped (the
//!   headless D-divergence documented in
//!   [`crate::routing_board_insert`]).
//! * **`contactPins` leak** — Java's `smoothenEndCornersAtTrace1`
//!   returns true from the keep-point split WITHOUT restoring the
//!   saved `contactPins` (`:461-463` — the `return` skips the
//!   restore at `:468`). The leak is real but unobservable (every
//!   later consumer re-sets the field first); the port reproduces it
//!   exactly ([`TraceTightener::smoothen_end_corners_at_trace1`]).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::side::Side;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::forced_pad_router::check_trace_shape;
use crate::id::ItemId;
use crate::items::ItemData;
use crate::normalize_all::normalize_traces_of_net;
use crate::routing_board_insert::PullTightSeam;
use crate::rules_surf::AngleRestriction;
use crate::time_limit::TimeLimit;
use crate::trace_ops::{
    change_trace_geometry, insert_trace_without_cleaning, is_shove_fixed, nets_equal, nets_normal,
    remove_item_through_repository,
};
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::clearance_compensation_value;

pub(crate) mod pin_tail;
pub(crate) mod tightener45;
pub(crate) mod tightener90;
pub(crate) mod via_optimizer;
pub(crate) use tightener45::{
    pull_tight_45, smoothen_end_corner_at_trace_45, smoothen_start_corner_at_trace_45,
};
pub(crate) use tightener90::pull_tight_90;

/// Java `TraceTightener.c_min_corner_dist_square` (`:36`) — "with
/// angles too close to 180 degree the algorithm becomes numerically
/// unstable". (`c_max_cos_angle` (`:33`) has no 90°/45°-variant
/// consumer — only `TraceTightenerAnyAngle.java:405` reads it; it
/// lands with the any-angle port. CORRECTION, T4: the T3 comment
/// claimed it lands with T4's 45° port — wrong; TraceTightener45
/// never reads it.)
const C_MIN_CORNER_DIST_SQUARE: f64 = 0.9;

/// The epic-board mirror of Java's
/// `AutorouteControl.ExpansionCostFactor` — the trace-cost pair the
/// fixpoint's via arm would consume (`optViaLocation(board, via,
/// traceCosts, ...)`). `epic_router::control::ExpansionCostFactor`
/// carries the same two fields; the mirror keeps epic-board
/// router-crate-free (T5 may unify when the via hook lands).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceCostFactor {
    /// Java `ExpansionCostFactor.horizontal`.
    pub horizontal: f64,
    /// Java `ExpansionCostFactor.vertical`.
    pub vertical: f64,
}

/// The deterministic port face of Java's `timeLimit` construction
/// (`TraceTightener` ctor `:73-77`:
/// `timeLimit > 0 ? new TimeLimit(timeLimit) : null`). `Wall` is the
/// Java face; `Deterministic` spends the same limit as consultation
/// ticks (the `RouteBudget` pattern, strict `>`).
#[derive(Debug)]
pub(crate) enum PullTightBudget {
    /// Java `timeLimit == null` — never stops.
    None,
    /// Java `new TimeLimit(milliSeconds)` — wall clock.
    Wall(TimeLimit),
    /// The deterministic profile: `limit` ticks, one per
    /// `is_stop_requested` consultation; stops when `spent > limit`.
    Deterministic { limit: u64, spent: u64 },
}

/// The ported `TraceTightener` state — the Java fields of the base
/// class (`:37-60`) with the concrete variant collapsed into a tag
/// (Java subclasses; Rust dispatches). Constructed only through
/// [`TraceTightener::get_instance`].
pub(crate) struct TraceTightener {
    /// The selected variant (Java's concrete class). T3 constructs
    /// only the 90° instance; the tag keeps the dispatch total for
    /// the T4 variants.
    variant: AngleRestriction,
    /// Java `onlyNetNoArr` — only these nets are optimized (empty =
    /// all).
    pub(crate) only_net_no_arr: Vec<i32>,
    /// Java `stoppableThread` — the shared stop flag.
    stoppable_flag: Option<Arc<AtomicBool>>,
    /// Java `timeLimit` through the deterministic profile face.
    budget: PullTightBudget,
    /// Java `keepPoint` — traces containing it must keep containing
    /// it.
    keep_point: Option<Point>,
    /// Java `keepPointLayer`.
    keep_point_layer: i32,
    /// Java `currentLayer`.
    current_layer: i32,
    /// Java `currentHalfWidth` — WITH the default tree's clearance
    /// compensation on the `pullTight` face, without it on the
    /// smoothen face (the two Java setters differ).
    current_half_width: i32,
    /// Java `currentNetNumbers`.
    current_net_numbers: Vec<i32>,
    /// Java `currentClearanceClassIndex`.
    current_clearance_class_index: i32,
    /// Java `currentClipShape` — `None` for Java null (unbounded).
    current_clip_shape: Option<IntOctagon>,
    /// Java `contactPins` — the pins at the end corners of the
    /// polyline under work; other pins are obstacles even when they
    /// carry an own net (the acid-trap face).
    contact_pins: Option<BTreeSet<ItemId>>,
    /// Java `minTranslateDist` — the binary-search accuracy, clamped
    /// to at least 100 at construction.
    min_translate_dist: i32,
}

impl TraceTightener {
    /// Java `TraceTightener.getInstance` (`:87-114`): the angle
    /// restriction picks the variant, the clip shape and the clamped
    /// translate distance are set on the instance, and `timeLimit > 0`
    /// builds the budget (deterministic profile: the same limit as
    /// ticks). `board` is read for the rules; the tightener carries no
    /// board handle (the Rust port passes `(manager, board)` per
    /// call — Java's `board` field is the same handle the whole
    /// facade shares).
    #[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
    pub(crate) fn get_instance(
        _manager: &SearchTreeManager,
        board: &Board,
        only_net_no_arr: &[i32],
        clip_shape: Option<IntOctagon>,
        min_translate_dist: i32,
        stoppable_flag: Option<Arc<AtomicBool>>,
        time_limit_millis: i32,
        deterministic_budgets: bool,
        keep_point: Option<Point>,
        keep_point_layer: i32,
    ) -> TraceTightener {
        let variant = board.rules().trace_angle_restriction;
        // Java `:73-77`: `timeLimit > 0 ? new TimeLimit(timeLimit) :
        // null`, with the deterministic profile spending the same
        // limit as ticks.
        let budget = if time_limit_millis > 0 {
            if deterministic_budgets {
                PullTightBudget::Deterministic {
                    limit: u64::try_from(time_limit_millis).unwrap_or(0),
                    spent: 0,
                }
            } else {
                PullTightBudget::Wall(TimeLimit::new(i64::from(time_limit_millis)))
            }
        } else {
            PullTightBudget::None
        };
        TraceTightener {
            variant,
            only_net_no_arr: only_net_no_arr.to_vec(),
            stoppable_flag,
            budget,
            keep_point,
            keep_point_layer,
            current_layer: 0,
            current_half_width: 0,
            current_net_numbers: Vec::new(),
            current_clearance_class_index: 0,
            current_clip_shape: clip_shape,
            contact_pins: None,
            min_translate_dist: min_translate_dist.max(100),
        }
    }

    /// Java `TraceTightener.optChangedArea(traceCosts)` (`:121-169`)
    /// — the fixpoint over the LIVE marking session.
    pub(crate) fn opt_changed_area(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_costs: Option<&[TraceCostFactor]>,
    ) {
        // Java `:122-124` — the null-session guard; the free
        // [`crate::routing_board_insert::opt_changed_area`] skeleton
        // already guarded, but the sweep re-checks because it drives
        // the session through `board.changed_area` mutably.
        if board.changed_area.is_none() {
            return;
        }
        // Java `:126-128`: "starting with curr_min_translate_dist big
        // is a try to avoid fine approximation at the beginning to
        // avoid problems with dog ears" — the comment rides the
        // FIELD; the loop below has no per-sweep mutation of it.
        let mut something_changed = true;
        while something_changed {
            something_changed = false;
            let layer_count = board.layers().layers.len() as i32;
            for layer in 0..layer_count {
                // Java `:132`: `board.changedArea.getArea(i)` — the
                // LIVE session, not a snapshot: the geometry
                // mutations below join back into it for the next
                // sweep.
                let changed_region = board
                    .changed_area
                    .as_ref()
                    .expect("session guarded above")
                    .get_area(layer);
                if changed_region.is_empty() {
                    continue;
                }
                // Java `:136`: setEmpty BEFORE processing — the
                // mutations re-mark the region for the next sweep.
                board
                    .changed_area
                    .as_mut()
                    .expect("session guarded above")
                    .set_empty(layer);
                // Java `:137`: joinGraphicsUpdateBox — the GUI update
                // box, dropped (headless D-divergence).
                // Java `:138-141`: `1.5 * (maxValue(i) + 2 *
                // rules.getMaxTraceHalfWidth())` — INT arithmetic
                // inside the parens (wrapping keeps the overflow
                // face), then a double multiply.
                let inner = board
                    .rules()
                    .clearance
                    .max_value_on_layer(layer)
                    .wrapping_add(2i32.wrapping_mul(board.rules().max_trace_half_width));
                let changed_area_offset = 1.5 * f64::from(inner);
                let changed_region = changed_region.enlarge(changed_area_offset);
                // Java `:145`: `board.overlappingObjects(changedRegion, i)`
                // — the DEFAULT tree, the `TreeSet` descending-id order.
                let query_shape =
                    TileShape::RegularTileShape(RegularTileShape::IntOctagon(changed_region));
                let items = manager.overlapping_objects(board, 0, &query_shape, layer, &[]);
                for item_id in items {
                    if self.is_stop_requested() {
                        return;
                    }
                    let item_kind = board.get(item_id).and_then(|entry| entry.data.kind_tag());
                    match item_kind {
                        Some(ItemKindTag::Trace) => {
                            if polyline_trace_pull_tight(self, manager, board, item_id) {
                                something_changed = true;
                                if self.split_traces_at_keep_point(manager, board) {
                                    break;
                                }
                            } else if self.smoothen_end_corners_at_trace(manager, board, item_id) {
                                something_changed = true;
                                // because items may be removed
                                break;
                            }
                        }
                        Some(ItemKindTag::Via) if trace_costs.is_some() => {
                            // Java `:160-165`:
                            // `ViaOptimizer.optViaLocation(board, via,
                            // traceCosts, minTranslateDist, 10)` —
                            // landed (T5); the seam delegates with the
                            // production depth budget 10.
                            let accuracy = self.min_translate_dist;
                            if opt_via_location_seam(
                                self,
                                manager,
                                board,
                                item_id,
                                trace_costs,
                                accuracy,
                            ) {
                                something_changed = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Java `PolylineTrace.pullTight(boolean, int, Stoppable)`
    /// (`PolylineTrace.java:869-886`), the face the ViaOptimizer arm
    /// drives: it builds a FRESH tightener (`getInstance` with the
    /// picked trace's OWN nets as the only-net list, a NULL clip, the
    /// call's `pullTightAccuracy` as the accuracy, and `timeLimit = -1`
    /// — no budget) and runs the SAME 1-arg `pullTight` body on it,
    /// leaving the fixpoint's algo object untouched. ViaOptimizer calls
    /// it with `stoppableThread = null` (`ViaOptimizer.java:144/:148/:290`),
    /// so the fresh face ALSO clears the stop flag: a via-arm
    /// pull-tight never stops mid-tighten in Java (M4-T5 spec-review
    /// MINOR-2). The port swaps the FIVE state fields for the duration
    /// of `body` and restores them after (Java's fresh algo is a
    /// separate object; the fixpoint state must survive the via arm).
    /// The accuracy carries Java's `getInstance` clamp
    /// `Math.max(minTranslateDist, 100)` (`TraceTightener.java:112`).
    ///
    /// RETAINED fields and why each is safe today (the enumeration the
    /// spec-review stop-flag divergence was found by — keep it current):
    /// - `keep_point`/`keep_point_layer`: Java's fresh tightener has
    ///   `keepPoint = null`, `keepPointLayer = -1`; the port RETAINS the
    ///   fixpoint's values across the closure. Same-value-by-construction
    ///   today — every production `opt_changed_area` caller passes
    ///   `None, 0` — and observable the moment a caller passes a real
    ///   keep point AND a via-arm fresh pull-tight reaches a
    ///   smoothen-split (`split_traces_at_keep_point` reads them below):
    ///   the fresh face would split at the fixpoint's keep point where
    ///   Java's fresh object would not — the spec-review stop-flag
    ///   divergence's class, one field over. Flip condition: the FIRST
    ///   caller passing a keep point — swap the pair here in the same
    ///   commit.
    /// - `contact_pins` and the `current_*` trace state: safe —
    ///   re-derived/re-set before every read by each `pull_tight`
    ///   dispatch (spec-round adjudication).
    /// - `variant`: safe — the same angle restriction on both faces.
    ///
    /// Observability note (quality review T5-Q2): the stop-flag swap is
    /// Java-exact but UNOBSERVABLE until a stoppable-carrying caller
    /// exists — every current test seam passes `stoppable = None`, so
    /// the flag is `None` under either shape. Green here is not
    /// coverage; the kill needs a stopper firing while a via-arm fresh
    /// pull-tight is mid-body.
    pub(crate) fn with_fresh_algo_face<T>(
        &mut self,
        own_nets: &[i32],
        accuracy: i32,
        body: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved_only_net = std::mem::replace(&mut self.only_net_no_arr, own_nets.to_vec());
        let saved_clip = self.current_clip_shape.take();
        let saved_budget = std::mem::replace(&mut self.budget, PullTightBudget::None);
        let saved_min_translate_dist =
            std::mem::replace(&mut self.min_translate_dist, accuracy.max(100));
        let saved_stoppable_flag = self.stoppable_flag.take();
        let result = body(self);
        self.only_net_no_arr = saved_only_net;
        self.current_clip_shape = saved_clip;
        self.budget = saved_budget;
        self.min_translate_dist = saved_min_translate_dist;
        self.stoppable_flag = saved_stoppable_flag;
        result
    }

    /// Java `TraceTightener.pullTight` (6-arg, `:175-190`): set the
    /// current-trace state (the half width WITH the default tree's
    /// clearance compensation), then dispatch to the variant.
    #[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
    pub(crate) fn pull_tight(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        polyline: Polyline,
        layer: i32,
        half_width: i32,
        net_numbers: &[i32],
        clearance_class_index: i32,
        contact_pins: Option<BTreeSet<ItemId>>,
    ) -> Polyline {
        self.current_layer = layer;
        // Java `:183-185`: `searchTree.clearanceCompensationValue(clIdx,
        // layer)` — the TREE-level formula (matrix value toward the
        // tree's class minus the matrix compensation, floored at 0,
        // 0 for a non-positive class), NOT the matrix-level (c,c)+1/2
        // read of the same Java method name.
        let tree_class = manager.default_tree().compensated_clearance_class;
        self.current_half_width = half_width.wrapping_add(clearance_compensation_value(
            board.rules(),
            clearance_class_index,
            tree_class,
            layer,
        ));
        self.current_net_numbers = net_numbers.to_vec();
        self.current_clearance_class_index = clearance_class_index;
        self.contact_pins = contact_pins;
        self.pull_tight_polyline(manager, board, polyline)
    }

    /// Java `public abstract Polyline pullTight(Polyline)` (`:192`) —
    /// the variant dispatch. T3 ports the 90° arm, T4 the 45° arm;
    /// any-angle keeps the identity fallback (the variant is deferred —
    /// census 0 any-angle fixtures — and unreachable through the seam,
    /// which [`active_for`] gates to the two built variants).
    fn pull_tight_polyline(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        polyline: Polyline,
    ) -> Polyline {
        match self.variant {
            AngleRestriction::NinetyDegree => pull_tight_90(self, manager, board, polyline),
            AngleRestriction::FortyfiveDegree => pull_tight_45(self, manager, board, polyline),
            // Java `TraceTightenerAnyAngle` — deferred (T4 boundary,
            // SEAM); never constructed while `active_for` gates None.
            AngleRestriction::None => polyline,
        }
    }

    /// Java `isStopRequested` (`:195-212`). The wall-clock exceeded
    /// face logs a debug row (log-only, dropped — the bug-118
    /// no-digest convention; the `board == null` error face is
    /// unrepresentable — the reference is never null).
    fn is_stop_requested(&mut self) -> bool {
        if let Some(flag) = &self.stoppable_flag
            && flag.load(Ordering::Relaxed)
        {
            return true;
        }
        match &mut self.budget {
            PullTightBudget::None => false,
            PullTightBudget::Wall(time_limit) => time_limit.limit_exceeded(),
            PullTightBudget::Deterministic { limit, spent } => {
                *spent += 1;
                *spent > *limit
            }
        }
    }

    /// Java `repositionLines` (`:215-230`): try to shorten the
    /// polyline by relocating its middle lines; the first accepted
    /// relocation wins (rebuild + `skipSegmentsOfLength0`).
    pub(crate) fn reposition_lines(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        polyline: &Polyline,
    ) -> Polyline {
        if polyline.lines.len() < 5 {
            return polyline.clone();
        }
        let len = polyline.lines.len() as i32;
        for no in 2..len - 2 {
            if let Some(new_line) = self.reposition_line(manager, board, &polyline.lines, no) {
                let mut lines = polyline.lines.clone();
                lines[no as usize] = new_line;
                let result = Polyline::new(lines);
                return self.skip_segments_of_length_0(manager, board, &result);
            }
        }
        polyline.clone()
    }

    /// Java `repositionLine` (`:235-331`): the binary-search
    /// translation of line `no` toward the nearer of its neighbor
    /// corners, clearance-checked through `checkTraceShape` on the
    /// 3-line offset shape.
    fn reposition_line(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        lines: &[Line],
        no: i32,
    ) -> Option<Line> {
        // Java `:236-238`.
        if lines.len() as i32 - no < 3 {
            return None;
        }
        let no_us = no as usize;
        // Java `:239-247`: with a clip shape, both corners of the
        // line must be inside. Consecutive polyline lines always
        // intersect (the constructor filters parallel runs), so the
        // Java null corner would be an NPE on a broken input — the
        // expect keeps that loud.
        if let Some(clip_shape) = &self.current_clip_shape {
            for i in -1..1 {
                let corner = lines[(no + i) as usize]
                    .intersection(&lines[(no + i + 1) as usize])
                    .expect("consecutive polyline lines intersect");
                // Java `isOutside(Point)` == `!contains(point.toFloat())`.
                if !clip_shape.contains(&corner.to_float()) {
                    return None;
                }
            }
        }
        let translate_line = &lines[no_us];
        let prev_corner = lines[no_us - 2]
            .intersection(&lines[no_us - 1])
            .expect("consecutive polyline lines intersect");
        let next_corner = lines[no_us + 1]
            .intersection(&lines[no_us + 2])
            .expect("consecutive polyline lines intersect");
        let prev_dist = translate_line.signed_distance(&prev_corner.to_float());
        let next_dist = translate_line.signed_distance(&next_corner.to_float());
        if Side::of(prev_dist) != Side::of(next_dist) {
            // the 2 corners are at different sides of translateLine
            return None;
        }
        let (nearest_point, max_translate_dist) = if prev_dist.abs() < next_dist.abs() {
            (prev_corner, prev_dist)
        } else {
            (next_corner, next_dist)
        };
        let mut translate_dist = max_translate_dist;
        let mut delta_dist = max_translate_dist;
        let side_of_nearest_point = translate_line.side_of(&nearest_point);
        let sign = Side::as_int(max_translate_dist);
        let mut new_line: Option<Line> = None;
        let mut check_lines: [Line; 3] = [
            lines[no_us - 1].clone(),
            lines[no_us].clone(),
            lines[no_us + 1].clone(),
        ];
        let mut first_time = true;
        while first_time || delta_dist.abs() > f64::from(self.min_translate_dist) {
            if first_time && matches!(nearest_point, Point::Int(_)) {
                check_lines[1] =
                    Line::get_instance(nearest_point.clone(), translate_line.direction().clone());
            } else {
                check_lines[1] = translate_line.translate(-translate_dist);
            }
            // Java `:281`: `checkLines[1].equals(translateLine)` —
            // Line.equals is same-infinite-line + direction, exactly
            // the port's `fast_equals`.
            if check_lines[1].fast_equals(translate_line) {
                // may happen at first time if nearestPoint is not an IntPoint
                return None;
            }
            let new_line_side_of_nearest_point = check_lines[1].side_of(&nearest_point);
            if new_line_side_of_nearest_point != side_of_nearest_point
                && new_line_side_of_nearest_point != Side::Collinear
            {
                // moved a little bit too far at the first time because
                // of numerical inaccuracy; may happen if nearestPoint
                // is not an IntPoint
                let shorten_value = f64::from(sign) * 0.5;
                // Java also runs `maxTranslateDist -= shortenValue`
                // here, but that field is never read again (its last
                // read is `sign = Signum.asInt(...)` before the loop)
                // — a dead store, dropped from the port.
                translate_dist -= shorten_value;
                delta_dist -= shorten_value;
                continue;
            }
            let tmp = Polyline::new(check_lines.to_vec());
            let mut check_ok = false;
            if tmp.lines.len() == 3 {
                // Java `:301`: `tmp.offsetShape(currentHalfWidth, 0)`
                // — null only on a range error; the 3-line guard
                // makes index 0 in-range, so the Java null is an
                // unreachable NPE.
                let shape_to_check = tmp
                    .offset_shape(self.current_half_width, 0)
                    .expect("3-line polyline has offset shape 0");
                check_ok = check_trace_shape(
                    manager,
                    board,
                    &shape_to_check,
                    self.current_layer,
                    &self.current_net_numbers,
                    self.current_clearance_class_index,
                    self.contact_pins.as_ref(),
                );
            }
            delta_dist /= 2.0;
            if check_ok {
                new_line = Some(check_lines[1].clone());
                if first_time {
                    // biggest possible change
                    break;
                }
                translate_dist += delta_dist;
            } else {
                translate_dist -= delta_dist;
            }
            first_time = false;
        }
        if let Some(accepted) = &new_line
            && board.changed_area.is_some()
        {
            // mark the changed area (Java `:323-329`) — the LIVE
            // session feeds the fixpoint's next sweep.
            join_approx(board, &check_lines[0], accepted, self.current_layer);
            join_approx(board, &check_lines[2], accepted, self.current_layer);
            join_approx(board, &lines[no_us - 1], &lines[no_us], self.current_layer);
            join_approx(board, &lines[no_us], &lines[no_us + 1], self.current_layer);
        }
        new_line
    }

    /// Java `skipSegmentsOfLength0` (`:338-399`): skip zero-length
    /// segments; a check is necessary before skipping because new dog
    /// ears may occur.
    pub(crate) fn skip_segments_of_length_0(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        polyline: &Polyline,
    ) -> Polyline {
        let mut polyline_changed = false;
        let mut current_polyline = polyline.clone();
        let mut i: i32 = 1;
        // Java's for-condition re-reads the CURRENT polyline length
        // every iteration (the accepted skips shrink it).
        while i < current_polyline.lines.len() as i32 - 1 {
            let try_skip = if i == 1 || i == current_polyline.lines.len() as i32 - 2 {
                // the position of the first corner and the last corner
                // must be retained exactly — exact point equality.
                match (current_polyline.corner(i), current_polyline.corner(i - 1)) {
                    (Some(current_corner), Some(prev_corner)) => current_corner == prev_corner,
                    // Java would NPE; a polyline in the loop has >= 3
                    // lines, so both corners exist.
                    _ => false,
                }
            } else {
                let prev_corner = current_polyline.corner_approx(i - 1);
                let current_corner = current_polyline.corner_approx(i);
                current_corner.distance_square(&prev_corner) < C_MIN_CORNER_DIST_SQUARE
            };
            if try_skip {
                // check, if skipping the line of length 0 does not
                // result in a clearance violation
                let mut current_lines = current_polyline.lines.clone();
                current_lines.remove(i as usize);
                let tmp = Polyline::new(current_lines);
                // Java `:362`: `tmp.lines.length == currentLines.length`
                // — the constructor collapses degenerate inputs to the
                // empty polyline, and that collapse vetoes the skip.
                let mut check_ok = tmp.lines.len() == current_polyline.lines.len() - 1;
                if check_ok && !current_polyline.lines[i as usize].is_multiple_of_45_degree() {
                    // no check necessary for skipping 45 degree lines,
                    // because the check is performance critical and the
                    // line shapes are intersected with the bounding
                    // octagon anyway.
                    if i > 1 {
                        // Java null on a range error is unreachable:
                        // `i - 2` is in range for the shrunk polyline.
                        let shape_to_check = tmp
                            .offset_shape(self.current_half_width, i - 2)
                            .expect("offset shape i-2 in range");
                        check_ok = check_trace_shape(
                            manager,
                            board,
                            &shape_to_check,
                            self.current_layer,
                            &self.current_net_numbers,
                            self.current_clearance_class_index,
                            self.contact_pins.as_ref(),
                        );
                    }
                    if check_ok && i < current_polyline.lines.len() as i32 - 2 {
                        let shape_to_check = tmp
                            .offset_shape(self.current_half_width, i - 1)
                            .expect("offset shape i-1 in range");
                        check_ok = check_trace_shape(
                            manager,
                            board,
                            &shape_to_check,
                            self.current_layer,
                            &self.current_net_numbers,
                            self.current_clearance_class_index,
                            self.contact_pins.as_ref(),
                        );
                    }
                }
                if check_ok {
                    polyline_changed = true;
                    current_polyline = tmp;
                    // re-test the same index (Java `--i` + the loop
                    // `++i`)
                    i -= 1;
                }
            }
            i += 1;
        }
        if polyline_changed {
            current_polyline
        } else {
            polyline.clone()
        }
    }

    /// Java `smoothenEndCornersAtTrace` (`:402-411`): set the
    /// current-trace state (the RAW half width — no compensation on
    /// this face), then run the smoothen chain. The current state is
    /// read off the (possibly removed) entry — Java reads the live
    /// object's fields, and the port's arena keeps removed entries.
    pub(crate) fn smoothen_end_corners_at_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> bool {
        let nets = board
            .get(trace_id)
            .map(|entry| entry.nets.clone())
            .unwrap_or_default();
        if !self.only_net_no_arr.is_empty() && !nets_equal(&nets, &self.only_net_no_arr) {
            return false;
        }
        self.current_layer = board.trace_layer(trace_id).unwrap_or(0);
        self.current_half_width = board.trace_half_width(trace_id).unwrap_or(0);
        self.current_net_numbers = nets;
        self.current_clearance_class_index = board
            .get(trace_id)
            .map(|entry| entry.clearance_class)
            .unwrap_or(0);
        self.smoothen_end_corners_at_trace1(manager, board, trace_id)
    }

    /// Java `smoothenEndCornersAtTrace1` (`:414-470`). The double
    /// `removeItem` at `:433`/`:445` is Java-exact (the repository
    /// remove is harmless when absent); the keep-point early return
    /// leaves `contactPins` null — the Java leak, kept (module docs).
    fn smoothen_end_corners_at_trace1(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> bool {
        // try to improve the connection to other traces
        if is_shove_fixed(board, trace_id) {
            return false;
        }
        let saved_contact_pins = self.contact_pins.take();
        // to allow the trace to slide to the end point of a contact
        // trace, if the contact trace ends at a pin.
        let mut result = false;
        let mut connection_to_trace_improved = true;
        let mut current_trace = trace_id;
        while connection_to_trace_improved {
            connection_to_trace_improved = false;
            let Some(adjusted_polyline) =
                self.smoothen_end_corners_at_trace2(manager, board, current_trace)
            else {
                continue;
            };
            // Read the re-insert facts BEFORE the removal (Java
            // `:430-432`): the entry stays readable after the remove,
            // but the nets list is captured up front because the
            // insert below needs the REMOVED trace's nets.
            let trace_layer = board.trace_layer(current_trace).unwrap_or(0);
            let current_cl_class = board
                .get(current_trace)
                .map(|entry| entry.clearance_class)
                .unwrap_or(0);
            let current_fixed_state = board.get(current_trace).map(|entry| entry.fixed);
            let current_net_numbers = board
                .get(current_trace)
                .map(|entry| entry.nets.clone())
                .unwrap_or_default();
            remove_item_through_repository(manager, board, current_trace);
            let adj_ins_trace = insert_trace_without_cleaning(
                manager,
                board,
                adjusted_polyline.clone(),
                trace_layer,
                self.current_half_width,
                &current_net_numbers,
                current_cl_class,
                current_fixed_state.unwrap_or(crate::items::FixedState::Unfixed),
            );
            if let Some(new_trace) = adj_ins_trace {
                result = true;
                connection_to_trace_improved = true;
                // Java `:445`: the second removeItem — harmless when
                // the first removed (the repository guard).
                remove_item_through_repository(manager, board, current_trace);
                current_trace = new_trace;
                for &net in &current_net_numbers {
                    let first_corner = adjusted_polyline
                        .first_corner()
                        .expect("adjusted polyline has a first corner");
                    let last_corner = adjusted_polyline
                        .last_corner()
                        .expect("adjusted polyline has a last corner");
                    crate::drill_item_mover::split_traces(
                        manager,
                        board,
                        &first_corner,
                        trace_layer,
                        net,
                    );
                    crate::drill_item_mover::split_traces(
                        manager,
                        board,
                        &last_corner,
                        trace_layer,
                        net,
                    );
                    // Java `:451-459` wraps the normalization in
                    // try/catch → FRLogger.error + continue; the
                    // port's normalize returns a bool and cannot
                    // throw, so the catch is unrepresentable.
                    let _ = normalize_traces_of_net(manager, board, net);
                    // Java `:461-463`: `return true` WITHOUT the
                    // contactPins restore at `:468` — the leak kept.
                    if self.split_traces_at_keep_point(manager, board) {
                        return true;
                    }
                }
            }
        }
        self.contact_pins = saved_contact_pins;
        result
    }

    /// Java `splitTracesAtKeepPoint` (`:476-491`) — the board-surface
    /// pick+split driver, already ported in
    /// [`crate::routing_board_insert::split_traces_at_keep_point`].
    pub(crate) fn split_traces_at_keep_point(
        &self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
    ) -> bool {
        crate::routing_board_insert::split_traces_at_keep_point(
            manager,
            board,
            self.keep_point.as_ref(),
            self.keep_point_layer,
        )
    }

    /// Java `smoothenEndCornersAtTrace2` (`:494-514`): try the start
    /// corner, then the end corner; join the moved corner into the
    /// live session; refresh `contactPins` from the trace's end
    /// corners; run the zero-length skip.
    fn smoothen_end_corners_at_trace2(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> Option<Polyline> {
        if !board.is_on_the_board(trace_id) {
            return None;
        }
        let mut result = self.smoothen_start_corner_at_trace(manager, board, trace_id);
        if result.is_none() {
            result = self.smoothen_end_corner_at_trace(manager, board, trace_id);
            if let Some(adjusted) = &result {
                // mark the changed area — the LAST corner of the
                // end-smoothed polyline (Java `:503`).
                let corner_no = adjusted.corner_count() as i32 - 1;
                let corner = adjusted.corner_approx(corner_no);
                crate::routing_board_insert::join_changed_area(board, &corner, self.current_layer);
            }
        } else if let Some(adjusted) = &result {
            // mark the changed area — the FIRST corner (Java `:507`).
            let corner = adjusted.corner_approx(0);
            crate::routing_board_insert::join_changed_area(board, &corner, self.current_layer);
        }
        result.map(|adjusted| {
            // Java `:510-511`: refresh the contact pins from the
            // ADJUSTED polyline's end corners, then the zero-length
            // skip.
            self.contact_pins = Some(crate::routing_board_insert::touching_pins_at_end_corners(
                manager,
                board,
                &adjusted,
                self.current_layer,
                self.current_half_width,
                &self.current_net_numbers,
                self.current_clearance_class_index,
            ));
            self.skip_segments_of_length_0(manager, board, &adjusted)
        })
    }

    /// Java `smoothenStartCornerAtTrace` (`:544`, abstract) — the 90°
    /// variant answers null (`TraceTightener90.java:161-163`); the 45°
    /// variant carries the REAL body (`TraceTightener45.java:462-565`,
    /// ported in [`tightener45`]). Any-angle stays deferred.
    fn smoothen_start_corner_at_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> Option<Polyline> {
        match self.variant {
            AngleRestriction::FortyfiveDegree => {
                smoothen_start_corner_at_trace_45(self, manager, board, trace_id)
            }
            _ => None,
        }
    }

    /// Java `smoothenEndCornerAtTrace` (`:546`, abstract) — the 90°
    /// variant answers null (`TraceTightener90.java:166-168`); the 45°
    /// variant carries the REAL body (`TraceTightener45.java:568-673`).
    fn smoothen_end_corner_at_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> Option<Polyline> {
        match self.variant {
            AngleRestriction::FortyfiveDegree => {
                smoothen_end_corner_at_trace_45(self, manager, board, trace_id)
            }
            _ => None,
        }
    }

    /// Java `avoidAcidTraps` (`:517-542`) — verbatim the identity:
    ///
    /// ```java
    /// protected Polyline avoidAcidTraps(Polyline polyline) {
    ///     if (true) {
    ///         return polyline;
    ///     }
    ///     Polyline result = polyline;
    ///     TraceShover shoveTraceAlgo = new TraceShover(this.board);
    ///     Polyline newPolyline = shoveTraceAlgo.springOverObstacles(
    ///         polyline, currentHalfWidth, currentLayer,
    ///         currentNetNumbers, currentClearanceClassIndex, contactPins);
    ///     if (newPolyline != null && newPolyline != polyline) {
    ///         if (this.board.checkPolylineTrace(newPolyline,
    ///                 currentLayer, currentHalfWidth, currentNetNumbers,
    ///                 currentClearanceClassIndex)) {
    ///             result = newPolyline;
    ///         }
    ///     }
    ///     return result;
    /// }
    /// ```
    ///
    /// The `if (true)` makes the spring-over body DEAD CODE — the
    /// oracle NEVER wraps around pins to avoid acid traps. The port
    /// keeps the identity verdict (and the parameters, so a
    /// mutation-test activation of the dead body compiles).
    pub(crate) fn avoid_acid_traps(
        &mut self,
        _manager: &mut SearchTreeManager,
        _board: &mut Board,
        polyline: Polyline,
    ) -> Polyline {
        polyline
    }
}

/// The item-kind discriminator the fixpoint's loop matches on (Java's
/// `instanceof PolylineTrace` / `instanceof Via` chain). A missing id
/// reads as "neither" (the entry was removed under the snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKindTag {
    Trace,
    Via,
}

impl ItemData {
    /// `None` for every kind outside the fixpoint's `instanceof` pair
    /// (pins, keepouts, outlines — Java falls through them silently).
    fn kind_tag(&self) -> Option<ItemKindTag> {
        match self {
            ItemData::Trace { .. } => Some(ItemKindTag::Trace),
            ItemData::Via { .. } => Some(ItemKindTag::Via),
            _ => None,
        }
    }
}

/// Java `PolylineTrace.pullTight(TraceTightener)` (`PolylineTrace.java
/// :810-859`) — the gate ladder + the 6-arg `pullTight` dispatch + the
/// `change` consumption. The `swapConnectionToPin` /
/// `correctConnectionToPin` tail (`:842-855`) is gated on
/// `angleRestriction != NINETY_DEGREE && pinEdgeToTurnDist > 0`:
/// unreachable on 90° boards, LIVE on 45° boards, and DEFERRED in T4
/// (module docs + the tail comment below).
pub(crate) fn polyline_trace_pull_tight(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> bool {
    // `:812-816`: "This trace may have been deleted in a trace split
    // for example".
    if !board.is_on_the_board(trace_id) {
        return false;
    }
    if is_shove_fixed(board, trace_id) {
        return false;
    }
    let entry_nets = board
        .get(trace_id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default();
    let layer = board.trace_layer(trace_id).unwrap_or(0);
    let half_width = board.trace_half_width(trace_id).unwrap_or(0);
    let clearance_class = board
        .get(trace_id)
        .map(|entry| entry.clearance_class)
        .unwrap_or(0);
    if !nets_normal(&entry_nets) {
        return false;
    }
    if !state.only_net_no_arr.is_empty() && !nets_equal(&entry_nets, &state.only_net_no_arr) {
        return false;
    }
    // Java `:823-828`: the FIRST net's class gate. A missing net or
    // class is Java-unreachable (parse-valid nets) and reads as
    // not-pull-tight.
    if let Some(&first_net) = entry_nets.first() {
        let pull_tight_allowed = board
            .rules()
            .nets
            .get(first_net)
            .and_then(|net| board.rules().net_class(net.net_class))
            .is_some_and(|net_class| net_class.pull_tight);
        if !pull_tight_allowed {
            return false;
        }
    }
    let lines = board.trace_polyline(trace_id).cloned().unwrap_or_else(|| {
        // Java-unreachable (the receiver is a PolylineTrace); the
        // empty polyline keeps the gate ladder total.
        Polyline::new(Vec::new())
    });
    // Java `:830`: `touchingPinsAtEndCorners()` — the acid-trap pin
    // set of THIS trace's end corners.
    let contact_pins = crate::routing_board_insert::touching_pins_at_end_corners(
        manager,
        board,
        &lines,
        layer,
        half_width,
        &entry_nets,
        clearance_class,
    );
    let new_lines = state.pull_tight(
        manager,
        board,
        lines.clone(),
        layer,
        half_width,
        &entry_nets,
        clearance_class,
        Some(contact_pins),
    );
    // Java `:838`: `if (newLines != lines)` — a REFERENCE test. The
    // value proxy is safe: every no-change path of the algorithm
    // returns the input polyline object itself, so an equal-value
    // fresh object (the only way value and reference disagree) is
    // unreachable.
    if new_lines != lines {
        // M7-T3 (beyond-Java): the min-length honoring gate — when the
        // tuning regime is ON (`Board::tuning_active`), a constrained
        // net's trace (resolved `min > 0` via
        // `BoardRules::net_class_length_bounds`, the T2 query surface)
        // is never shortened below `min` by the tightener: a candidate
        // polyline shorter than `min` is rejected WHOLE (the old
        // geometry stands; the fixpoint then behaves exactly like the
        // no-change path — it falls through to the pin tail). The gate
        // is INERT when the flag is false or the net carries no `min`
        // (`min <= 0.0` — the T1 delivery gate), so the parity regime
        // is untouched (the byte-invariance rule). Java has no
        // counterpart: the oracle shortens unconstrained. Both tightener
        // change-acceptance faces gate here or at the pin tail
        // (`pin_tail::pin_connection_tail` — the module's only other
        // `change_trace_geometry` site); the via arm re-lands
        // length-preserving split pieces through this same function, so
        // the gate covers it too.
        if min_length_gate_allows(board, trace_id, &new_lines) {
            change_trace_geometry(manager, board, trace_id, new_lines);
            return true;
        }
    }
    // Java `:840-855`: the swap/correct pin-connection tail —
    // `angleRestriction != NINETY_DEGREE && pinEdgeToTurnDist > 0`.
    // LIVE as of M4-T6 (the T4 deferral is retired — see
    // [`pin_tail`]): LIVE on every 45° board (the DSN parser defaults
    // pinEdgeToTurnDist to minTraceHalfWidth,
    // `Structure.java:667-668`, and no tier fixture carries
    // `smd_to_turn_gap`); inert on 90° boards by the first gate half.
    pin_tail::pin_connection_tail(state, manager, board, trace_id)
}

/// M7-T3 (beyond-Java): the min-length honoring gate — the single
/// predicate every tightener change-acceptance face consults. TRUE when
/// the candidate polyline may land: either the tuning regime is OFF
/// (the parity regime, `Board::tuning_active` false — the derived
/// board Default), the net carries no `min` (the T1 delivery gate,
/// `min <= 0.0`), or the candidate is not a shortening below `min`
/// (NEW length `>= min`; landing AT `min` is allowed — the contract is
/// "never below", the `min + ε → min` tightening face). The old
/// geometry always stands on a forbidden candidate (a WHOLE-candidate
/// rejection, never a partial adjust — deterministic and order-stable,
/// a pure function of the board state and the candidate).
#[must_use]
pub(crate) fn min_length_gate_allows(
    board: &Board,
    trace_id: ItemId,
    new_lines: &Polyline,
) -> bool {
    if !board.tuning_active() {
        return true;
    }
    let Some(first_net) = board
        .get(trace_id)
        .map(|entry| entry.nets.first().copied())
        .unwrap_or(None)
    else {
        return true;
    };
    let (min, _max) = board.rules().net_class_length_bounds(first_net);
    if min <= 0.0 {
        return true;
    }
    new_lines.length_approx_total() >= min
}

/// The fixpoint's per-via relocation seam (Java `ViaOptimizer.
/// optViaLocation(board, via, traceCosts, minTranslateDist, 10)`,
/// `TraceTightener.java:160-165`) — LANDED in [`via_optimizer`] (T5);
/// this wrapper carries the production recursion budget
/// [`via_optimizer::VIA_RELOCATION_RECURSION_DEPTH`] (Java's literal
/// `10` at the `TraceTightener.java:165` call site) so the fixpoint
/// arm keeps a single call shape.
fn opt_via_location_seam(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    trace_costs: Option<&[TraceCostFactor]>,
    min_translate_dist: i32,
) -> bool {
    via_optimizer::opt_via_location(
        state,
        manager,
        board,
        via_id,
        trace_costs,
        min_translate_dist,
        via_optimizer::VIA_RELOCATION_RECURSION_DEPTH,
    )
}

/// The changed-area join of `Line::intersection_approx` (Java
/// `board.changedArea.join(lineA.intersectionApprox(lineB), layer)`).
fn join_approx(board: &mut Board, a: &Line, b: &Line, layer: i32) {
    let point = a.intersection_approx(b);
    crate::routing_board_insert::join_changed_area(board, &point, layer);
}

// ---------------------------------------------------------------------------
// The production seam
// ---------------------------------------------------------------------------

/// The M4 production [`PullTightSeam`] — Java's
/// `RoutingBoardOperations.optChangedArea` tightener arm and the
/// `insertForcedTracePolyline` per-trace face, behind the
/// [`active_for`] activation gate (45° + 90° live as of T4; any-angle
/// keeps the T10c no-op face until its variant lands).
pub struct TraceTightenerSeam;

impl PullTightSeam for TraceTightenerSeam {
    fn opt_changed_area(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        only_net_no_arr: &[i32],
        clip_shape: Option<&IntOctagon>,
        accuracy: i32,
        keep_point: Option<&Point>,
        keep_point_layer: i32,
        trace_costs: Option<&[TraceCostFactor]>,
        stoppable: Option<&Arc<AtomicBool>>,
        time_limit_millis: i32,
        deterministic_budgets: bool,
    ) {
        if !active_for(board.rules().trace_angle_restriction) {
            return;
        }
        let mut tightener = TraceTightener::get_instance(
            manager,
            board,
            only_net_no_arr,
            clip_shape.cloned(),
            accuracy,
            stoppable.cloned(),
            time_limit_millis,
            deterministic_budgets,
            keep_point.cloned(),
            keep_point_layer,
        );
        tightener.opt_changed_area(manager, board, trace_costs);
    }

    fn pull_tight_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
        algo: &crate::routing_board_insert::PullTightAlgo,
    ) {
        // Java `RoutingBoard.insertForcedTracePolyline` (`:783-793`)
        // builds the tightener with `stoppable = null`,
        // `timeLimit = -1` (no budget), the tidy region as clip, the
        // new corner as keep point.
        if !active_for(board.rules().trace_angle_restriction) {
            return;
        }
        let mut tightener = TraceTightener::get_instance(
            manager,
            board,
            &algo.only_nets,
            algo.clip_shape,
            algo.min_translate_dist,
            None,
            -1,
            false,
            algo.keep_point.clone(),
            algo.keep_point_layer,
        );
        let _ = polyline_trace_pull_tight(&mut tightener, manager, board, trace_id);
    }
}

/// The T4 activation face of the variant dispatch: Java `getInstance`
/// (`:96-110`) selects TraceTightener90 on NINETY_DEGREE boards,
/// TraceTightener45 on FORTYFIVE_DEGREE boards, and
/// TraceTightenerAnyAngle otherwise. The port carries the first two
/// variants; any-angle stays DEFERRED (census: 0 any-angle fixtures —
/// the parse default is FORTYFIVE_DEGREE, `ReadScopeParameter.java:59`),
/// so the seam keeps the T10c no-op face only on `NONE` and a future
/// any-angle fixture fails loudly at the dispatch pin, not silently.
#[must_use]
pub fn active_for(angle_restriction: AngleRestriction) -> bool {
    !matches!(angle_restriction, AngleRestriction::None)
}

#[cfg(test)]
mod pins;
