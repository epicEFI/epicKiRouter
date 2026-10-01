//! The forced-insertion board surface (Java
//! `board/facade/RoutingBoard.java` insert/check methods, ported as the
//! free-function surface the M3 router calls).
//!
//! Java anchors (RoutingBoard.java):
//!
//! * `checkForcedTracePolyline` `:408-448` — the shove-feasibility
//!   probe ([`check_forced_trace_polyline`]),
//! * `insertForcedTracePolyline` `:456-876` — the money path
//!   ([`insert_forced_trace_polyline`]),
//! * `connectToTrace` `:1116-1170` ([`connect_to_trace`]),
//! * `removeTraceTails` `:1193-1238` ([`remove_trace_tails`]) — plus
//!   the marking trio from `RoutingBoardOperations.java`
//!   (`startMarkingChangedArea` `:26-31`, `joinChangedArea` `:33-37`,
//!   `markAllChangedArea` `:39-50`) and the `optChangedArea` skeleton
//!   `:52-79` (the [`PullTightSeam`] surface).
//!
//! Call-graph edges: `insert_forced_trace_polyline` drives
//! [`crate::trace_shover::spring_over_obstacles`], the shove
//! `check`/`insert` pair,
//! [`crate::trace_ops::insert_trace_without_cleaning`] and
//! [`crate::trace_ops::combine`], the changed-area session and the
//! pull-tight seam; `connect_to_trace` drives
//! [`check_polyline_trace`] and [`crate::trace_ops::insert_trace`];
//! `remove_trace_tails` drives the contact walk and
//! [`crate::trace_shover::combine_traces`].
//!
//! ## The return-null contract
//!
//! Java `insertForcedTracePolyline` returns `null` ("the database may
//! be damaged and an undo necessary") from exactly two arms: a failed
//! `shoveTraceAlgo.insert` in the main shape loop (`:616-618`) and in
//! the sampling retry (`:742-745`). The port models this as
//! [`insert_forced_trace_polyline`]'s `None`. Java additionally NPEs
//! unguarded at `:756` (`newTrace.combine()`) when
//! `insertTraceWithoutCleaning` returned null for an empty polyline —
//! a JVM crash the pipeline never reaches from real callers; the port
//! maps it onto the same `None` (see the `// T10c` marker at the site).
//!
//! ## The deferred segment wrapper
//!
//! Java's `insertForcedTraceSegment` (`:381-405`) — the per-segment
//! wrapper T11's `FoundConnectionInserter` loop calls — is deferred to
//! T11 with its consumer. Parity subtlety to carry: Java maps the
//! insert result via REFERENCE equality (`okPoint ==
//! insertPolyline.firstCorner()`, `RoutingBoard.java:393-401`;
//! likewise FoundConnectionInserter's `candidateOkPoint ==
//! targetPoint` tie-break), which Rust value equality reproduces only
//! in the absence of value-coincident distinct points.
//!
//! ## The pull-tight seam (M4-T3 fill)
//!
//! Java runs the `TraceTightener` both from `insertForcedTracePolyline`
//! (the per-segment `pullTight` at `:860-862`) and from
//! `RoutingBoardOperations.optChangedArea` (`:52-79`). T10c ported the
//! SKELETON behind [`PullTightSeam`]; M4-T3 fills the seam with the
//! [`crate::trace_tightener::TraceTightenerSeam`] production
//! implementation — ACTIVE on `NINETY_DEGREE` boards only, because
//! Java's `getInstance` dispatches the 45°/any-angle variants that are
//! T4 scope (`crate::trace_tightener` module docs carry the full
//! divergence list; [`crate::trace_tightener::active_for`] is the
//! dispatch face). [`NoPullTight`] stays for the test/oracle paths.
//! On non-90° boards the production seam keeps the T10c no-op
//! behavior, so the T10c-era divergences survive THERE:
//!
//! 1. **Post-route corner coordinates** — on 45°/any-angle boards
//!    Java's pull-tight moves corners inside the tidy region, so
//!    routed geometry after this seam can differ from Java's until T4.
//! 2. **Trace-id deltas from tightener splits** — on 45°/any-angle
//!    boards the tightener's own splitting changes the id inventory;
//!    capture rows taken after `pullTight` (`after_pull_tight`) have
//!    no comparable Rust values.
//! 3. **Missing `after_pull_tight` counterparts** — on 45°/any-angle
//!    boards the probe prints those rows from the Java run; the Rust
//!    pins consciously skip them (the `before_pull_tight` rows remain
//!    comparable). On 90° boards the seam is live and the rows are
//!    comparable.
//!
//! `splitTracesAtKeepPoint` (`TraceTightener.java:474-491`) is NOT
//! tightener-internal (a pick + split driver over the board surface)
//! and is ported here in full:
//! [`split_traces_at_keep_point`].

use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTreeVariant;

use crate::board::Board;
use crate::contacts;
use crate::id::ItemId;
use crate::items::trace::{first_corner, last_corner, tile_shape_count};
use crate::items::{FixedState, ItemData};
use crate::rules_surf::AngleRestriction;
use crate::shape_entry_side::ShapeEntrySide;
use crate::time_limit::TimeLimit;
use crate::trace_ops::StopConnectionOption;
use crate::trace_shover::{combine_traces, get_trace_tail, remove_items};
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::clearance_compensation_value;

// ---------------------------------------------------------------------------
// The changed-area session (RoutingBoardOperations.java:26-50)
// ---------------------------------------------------------------------------

/// Java `RoutingBoardOperations.startMarkingChangedArea` (`:26-31`):
/// create the session only when none is active — nested starts
/// accumulate into the existing marker.
pub fn start_marking_changed_area(board: &mut Board) {
    if board.changed_area.is_none() {
        board.changed_area = Some(crate::changed_area::ChangedArea::new(
            board.layers().layers.len(),
        ));
    }
}

/// Java `RoutingBoardOperations.joinChangedArea` (`:33-37`): grow the
/// session marker (a no-op while no session is active — Java's null
/// guard, the seam sites rely on it).
pub fn join_changed_area(board: &mut Board, point: &FloatPoint, layer: i32) {
    if let Some(area) = board.changed_area.as_mut() {
        area.join_point(point, layer);
    }
}

/// Java `RoutingBoardOperations.markAllChangedArea` (`:39-50`): start
/// a session and join the four bounding-box corners of the board on
/// EVERY layer.
///
/// No consumer in Rust or in Java outside the facade wrapper (the
/// T10c quality review verified both) — ported for trio completeness
/// with [`start_marking_changed_area`] / [`join_changed_area`].
pub fn mark_all_changed_area(board: &mut Board) {
    start_marking_changed_area(board);
    let bounding_box = board.bounding_box().expect("post-parse bounding box");
    let board_corners = [
        bounding_box.ll.to_float(),
        FloatPoint::new(f64::from(bounding_box.ur.x), f64::from(bounding_box.ll.y)),
        bounding_box.ur.to_float(),
        FloatPoint::new(f64::from(bounding_box.ll.x), f64::from(bounding_box.ur.y)),
    ];
    let layer_count = board.layers().layers.len() as i32;
    for layer in 0..layer_count {
        for corner in &board_corners {
            join_changed_area(board, corner, layer);
        }
    }
}

// ---------------------------------------------------------------------------
// The pull-tight seam (M4 fill; RoutingBoardOperations.optChangedArea :52-79)
// ---------------------------------------------------------------------------

/// The pull-tight SEAM trait — the T10c boundary in front of the
/// `TraceTightener` port, filled at M4-T3 by
/// [`crate::trace_tightener::TraceTightenerSeam`]; the no-op
/// [`NoPullTight`] stays for tests and oracle-comparison paths.
pub trait PullTightSeam {
    /// Java `TraceTightener.optChangedArea(traceCosts)`
    /// (`TraceTightener.java:113+` via
    /// `RoutingBoardOperations.optChangedArea` `:65-74`): optimize the
    /// routed surface inside the marked area. The marking session is
    /// read LIVE off `board.changed_area` (M4-T3 signature change: the
    /// T10c skeleton passed a CLONE, but the fixpoint must see the
    /// joins its own geometry mutations make — a stale snapshot would
    /// end the sweep after one round).
    ///
    /// The widened tail is the M4-T3 planned break: `trace_costs`
    /// (Java feeds the via arm; the port's via hook is T5),
    /// `stoppable` (Java `stoppableThread`), `time_limit_millis`
    /// (Java `timeLimit` — wall clock in Java, a deterministic tick
    /// budget behind `deterministic_budgets`, the `RouteBudget`
    /// pattern), and `deterministic_budgets` (the profile switch).
    /// `clip_shape` is `Option` because Java's `clipShape == null`
    /// means UNBOUNDED (the gate still runs) — `None` here.
    #[allow(clippy::too_many_arguments)] // the Java read set, kept flat
    fn opt_changed_area(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        only_net_no_arr: &[i32],
        clip_shape: Option<&IntOctagon>,
        accuracy: i32,
        keep_point: Option<&Point>,
        keep_point_layer: i32,
        trace_costs: Option<&[crate::trace_tightener::TraceCostFactor]>,
        stoppable: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
        time_limit_millis: i32,
        deterministic_budgets: bool,
    );

    /// Java `PolylineTrace.pullTight(TraceTightener)` at
    /// `RoutingBoard.java:860-862`: tighten the freshly inserted trace
    /// around its end corner. See the module docs for the named
    /// divergences of the no-op face.
    fn pull_tight_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
        algo: &PullTightAlgo,
    );
}

/// The T10c no-op seam, kept for TEST and oracle-comparison paths
/// (the production call sites route through
/// [`crate::trace_tightener::TraceTightenerSeam`] since M4-T3).
pub struct NoPullTight;

impl PullTightSeam for NoPullTight {
    fn opt_changed_area(
        &mut self,
        _manager: &mut SearchTreeManager,
        _board: &mut Board,
        _only_net_no_arr: &[i32],
        _clip_shape: Option<&IntOctagon>,
        _accuracy: i32,
        _keep_point: Option<&Point>,
        _keep_point_layer: i32,
        _trace_costs: Option<&[crate::trace_tightener::TraceCostFactor]>,
        _stoppable: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
        _time_limit_millis: i32,
        _deterministic_budgets: bool,
    ) {
        // The no-op face: on 90° boards this forgoes Java's
        // TraceTightener sweep (post-route coordinate + id-inventory
        // divergence, module docs); on 45°/any boards it MATCHES the
        // production seam (T4 scope there).
    }

    fn pull_tight_trace(
        &mut self,
        _manager: &mut SearchTreeManager,
        _board: &mut Board,
        _trace_id: ItemId,
        _algo: &PullTightAlgo,
    ) {
        // T10c: PolylineTrace.pullTight is M4 scope — no-op. Java's
        // call would move corners (post-route coordinate divergence),
        // split traces (id deltas) and change the picked-at-end-corner
        // counts the probe reports after the call.
    }
}

/// The `TraceTightener.getInstance` descriptor (`TraceTightener.java:76-113`):
/// the fields the seam consumers read. `getInstance` NEVER returns
/// null — it picks the concrete class from the angle restriction and
/// clamps `minTranslateDist` to at least 100 — so the port models it
/// as this plain value, not an `Option`.
#[derive(Debug, Clone)]
pub struct PullTightAlgo {
    /// Java `onlyNetNoArr`: the nets the tightener may touch (the
    /// `optNetNoArr` decision at `:777-782`: the full net array only
    /// when `maxRecursionDepth <= 0`, else empty).
    pub only_nets: Vec<i32>,
    /// Java `currentClipShape` (the `tidyRegion`): `None` for Java
    /// null, i.e. unbounded optimizing.
    pub clip_shape: Option<IntOctagon>,
    /// Java `minTranslateDist = Math.max(pullTightAccuracy, 100)`.
    pub min_translate_dist: i32,
    /// Java `keepPoint` — traces containing it must keep containing it.
    pub keep_point: Option<Point>,
    /// Java `keepPointLayer`.
    pub keep_point_layer: i32,
}

impl PullTightAlgo {
    /// Java `TraceTightener.getInstance(board, onlyNetNoArr, clipShape,
    /// minTranslateDist, stoppable, timeLimit, keepPoint, keepLayer)`
    /// — the field projection of the concrete-instance construction
    /// (the angle restriction selects the class; all classes store the
    /// same fields the seam reads).
    #[must_use]
    pub fn get_instance(
        only_nets: &[i32],
        clip_shape: Option<IntOctagon>,
        min_translate_dist: i32,
        keep_point: Option<Point>,
        keep_point_layer: i32,
    ) -> PullTightAlgo {
        PullTightAlgo {
            only_nets: only_nets.to_vec(),
            clip_shape,
            min_translate_dist: min_translate_dist.max(100),
            keep_point,
            keep_point_layer,
        }
    }
}

/// Java `RoutingBoardOperations.optChangedArea` (`:52-79`) — the
/// session-consumption skeleton. The `TraceTightener` arm runs through
/// the [`PullTightSeam`] (M4-T3 fills it); the rest is verbatim:
///
/// * the null-session early-out (`:56-58`) — the session is KEPT,
/// * the `clipShape != IntOctagon.EMPTY` gate around the tightener
///   call (`:59-74`) — Java's `clipShape == null` SATISFIES the gate
///   (`null != EMPTY`), so `None` here means unbounded optimizing and
///   the call RUNS,
/// * the graphics update box join (`:77`) — NOT ported: Java joins
///   `surroundingBox()` into the board's GUI `updateBox`
///   (`BasicBoard.joinGraphicsUpdateBox`), an observer surface the
///   headless port does not carry (the box is computed and dropped
///   here),
/// * the session teardown `board.changedArea = null` (`:78`).
///
/// M4-T3: the T10c marker CLONE is gone — the seam reads the session
/// LIVE off `board.changed_area`, because the fixpoint's own joins
/// must feed its next sweep (a snapshot would terminate after one
/// round).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn opt_changed_area<S: PullTightSeam>(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    seam: &mut S,
    only_net_no_arr: &[i32],
    clip_shape: Option<&IntOctagon>,
    accuracy: i32,
    keep_point: Option<&Point>,
    keep_point_layer: i32,
    trace_costs: Option<&[crate::trace_tightener::TraceCostFactor]>,
    stoppable: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    time_limit_millis: i32,
    deterministic_budgets: bool,
) {
    // Java guards on the null session and keeps it on the early-out.
    if board.changed_area.is_none() {
        return;
    }
    // Java `:59`: `if (clipShape != IntOctagon.EMPTY)` — a null
    // clipShape runs the sweep (unbounded).
    if clip_shape.is_none_or(|oct| *oct != IntOctagon::EMPTY) {
        seam.opt_changed_area(
            manager,
            board,
            only_net_no_arr,
            clip_shape,
            accuracy,
            keep_point,
            keep_point_layer,
            trace_costs,
            stoppable,
            time_limit_millis,
            deterministic_budgets,
        );
    }
    // Java `:77`: joinGraphicsUpdateBox(changedArea.surroundingBox())
    // — the GUI update box does not exist on the headless port; the
    // box value is intentionally dropped (D-divergence, module docs).
    if let Some(marker) = board.changed_area.as_ref() {
        let _ = marker.surrounding_box();
    }
    board.changed_area = None;
}

// ---------------------------------------------------------------------------
// checkForcedTracePolyline (RoutingBoard.java:408-448)
// ---------------------------------------------------------------------------

/// Java `checkForcedTracePolyline` (`:408-448`): checks if a trace
/// polyline with the input parameters can be inserted while shoving
/// aside obstacle traces and vias — WITHOUT changing the board.
/// Consumer: M3-T11 `FoundConnectionInserter` (per-segment
/// feasibility probe ahead of the forced insert).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn check_forced_trace_polyline(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
    half_width: i32,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    max_spring_over_recursion_depth: i32,
) -> bool {
    let tree = manager.default_tree();
    let tree_class = tree.compensated_clearance_class;
    let compensated_half_width = half_width
        + clearance_compensation_value(board.rules(), clearance_class_index, tree_class, layer);
    let trace_shapes =
        polyline.offset_shapes(compensated_half_width, 0, polyline.lines.len() as i32 - 1);
    let orthogonal_mode = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;
    for (i, shape) in trace_shapes.iter().enumerate() {
        let mut current_trace_shape = shape.clone();
        if orthogonal_mode {
            current_trace_shape = box_tile(&current_trace_shape);
        }
        let from_side = ShapeEntrySide::from_entry_no(polyline, i as i32 + 1, &current_trace_shape);
        let check_shove_ok = crate::trace_shover::check(
            manager,
            board,
            &current_trace_shape,
            Some(from_side),
            None,
            layer,
            net_numbers,
            clearance_class_index,
            max_recursion_depth,
            max_via_recursion_depth,
            max_spring_over_recursion_depth,
            None, // Java passes the null TimeLimit literal
        );
        if !check_shove_ok {
            return false;
        }
    }
    true
}

/// The picked-trace geometry the Java combine sites read (bug 177).
///
/// Java `RoutingBoard.insertForcedTracePolyline` picks a same-net
/// trace OBJECT at `:489-517` and reads `pickedTrace.polyline()` —
/// the object's CURRENT `lines` field — at both combine sites
/// (`:536-542` and, in the sampling retry, `:682`). The reference
/// stays readable across the whole shove dance even when the dance
/// REMOVES the picked trace from the board: a read after removal
/// answers the geometry AS OF REMOVAL (the JVM object dies with its
/// last field write, which the removal paths do not perform —
/// removals only unregister).
///
/// The id-based port models the reference as the id plus the
/// PICK-TIME geometry snapshot: re-read the id while it still
/// resolves (the port's in-place geometry writes keep the id, so the
/// current field is observable there — the same answer Java's live
/// reference gives); once the id is dead, answer the snapshot — equal
/// to Java's removal-time geometry in every death the current engine
/// produces (all removal paths unregister WITHOUT changing geometry
/// first). The residual corner — an in-place geometry write FOLLOWED
/// by removal between pick and re-read — would make Java answer the
/// changed (removal-time) geometry where the port answers the
/// pick-time snapshot; no current code path writes-then-removes a
/// picked trace inside the dance (in-place writes are combine
/// receivers, which stay live), so the corner is documented, not
/// handled (SEAM T17b).
///
/// Vacuity disclosure (T17b mutation analysis): both orderings are
/// behaviorally indistinguishable in reachable worlds — a live,
/// CHANGED picked trace cannot occur before either site (site 1 sits
/// behind the board-read-only `spring_over_obstacles`; the loop's
/// in-place writes are combine receivers that keep the id AND the
/// change would need to precede the site re-read), and at site 2 the
/// recombined value propagates only through `shape_index =
/// lines - 3`, the entry-side corner VALUE at that index, and the
/// `< 3 lines` gate — all invariant under prepending the picked
/// chain (the pick forces the chain to attach at the front corner,
/// `shorten` keeps the front). The fix's observable content is the
/// no-panic + Java's VALUE; a skip-when-dead shape would be
/// equivalent today, and the port still computes Java's value (the
/// contract shape) rather than skipping.
fn picked_trace_lines(board: &Board, picked: &(ItemId, Polyline)) -> Polyline {
    board
        .trace_polyline(picked.0)
        .cloned()
        .unwrap_or_else(|| picked.1.clone())
}

// ---------------------------------------------------------------------------
// insertForcedTracePolyline (RoutingBoard.java:456-876)
// ---------------------------------------------------------------------------

/// Java `insertForcedTracePolyline` (`:456-876`): inserts a trace
/// polyline while shoving obstacles aside, with the sampling retry for
/// a failing last shape. Returns the last corner the shove reached
/// (`Some`), or `None` where Java returns null — the database may be
/// damaged and an undo necessary (module docs, "The return-null
/// contract").
///
/// Consumer: M3-T11 `FoundConnectionInserter` (per-segment forced
/// inserts; also the deferred `insert_forced_trace_segment` wrapper —
/// module docs, "The deferred segment wrapper").
///
/// Step ladder (the probe's `step=` rows map 1:1 onto these blocks):
///
/// 1. `clearShoveFailingObstacle` (`:469`)
/// 2. degenerate null-corner bail → `None` (`:472-480`)
/// 3. `fromCorner == toCorner` → `toCorner` (`:481-483`)
/// 4. IntPoint gate → `Some(from_corner)` (`:484-487`)
/// 5. `startMarkingChangedArea` (`:488`)
/// 6. picked-trace candidate: `pickedItems.size()==1` +
///    netsEqual/halfWidth/class gate (`:489-517`)
/// 7. compensated width (`:518-520`)
/// 8. springOver, null → `fromCorner` (`:525-535`)
/// 9. combine + `lines < 3` → `fromCorner` (`:536-553`)
/// 10. `startShapeNo` + tail `offsetShapes` (`:554-558`)
/// 11. the shove loop: entry-side ShapeEntrySide index (`:567-571`),
///     withCheck-break → `lastShapeNo` (`:585-588`), insert fail →
///     null (`:616-618`)
/// 12. the sampling retry (`:634-746`): `sampleWidth` (`:641`),
///     `>100×` → `fromCorner` (`:645`), `shorten` (`:659`), IntPoint
///     bail, recombine, `shapeIndex` (`:685`), offset+re-check,
///     insert fail → null (`:742-745`)
/// 13. per-corner join (`:747-750`) + `insertTraceWithoutCleaning`
///     (`:752-754`) + `newTrace.combine()` (`:756`)
/// 14. tidy region + pull-tight algo (`:773-785`) + the swallowing
///     try: normalize (`:791`) → `splitTracesAtKeepPoint` (`:809`) →
///     re-pick (`:824-833`)
/// 15. the `pullTight` gate (`:860-862`) + return `newCorner` (`:875`)
///
/// `net_numbers` models Java's nullable `int[]`: Java's probe logs
/// gate on `!= null && length > 0` (plus a net-94 fail-row filter) —
/// pure oracle-side logging the port does not carry (D12). The engine
/// itself is only ever fed real arrays by the pipeline callers; a
/// literal `None` would NPE inside Java's picked-trace gate — the
/// port substitutes the empty slice there and at the engine calls,
/// the conservative reading of that unreachable defensive path.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn insert_forced_trace_polyline<S: PullTightSeam>(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    seam: &mut S,
    polyline: &Polyline,
    half_width: i32,
    layer: i32,
    net_numbers: Option<&[i32]>,
    clearance_class_index: i32,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    max_spring_over_recursion_depth: i32,
    tidy_width: i32,
    pull_tight_accuracy: i32,
    with_check: bool,
    time_limit: Option<&TimeLimit>,
) -> Option<Point> {
    let net_slice: &[i32] = net_numbers.unwrap_or(&[]);

    board.clear_shove_failing_obstacle();
    let from_corner = polyline.first_corner();
    let to_corner = polyline.last_corner();
    let (Some(from_corner), Some(to_corner)) = (from_corner, to_corner) else {
        // Java `:472-480`: a degenerate polyline has no well-defined
        // first/last corner; the trace cannot be inserted and the
        // caller treats this segment as not inserted.
        return None;
    };
    if from_corner == to_corner {
        return Some(to_corner);
    }
    let (Point::Int(_), Point::Int(_)) = (&from_corner, &to_corner) else {
        // Java `:484-487`: FRLogger.warn "only implemented for
        // IntPoints"; returns fromCorner.
        return Some(from_corner);
    };
    start_marking_changed_area(board);

    // Java `:489-517`: check if an item of the same net ends at
    // fromCorner — its geometry will be used to cut off dog ears of
    // the check shape. The picked item must be a polyline trace with
    // equal nets, half width and clearance class.
    //
    // Java holds the picked POLYLINE TRACE OBJECT (a live reference)
    // across the whole method; both combine sites read
    // `pickedTrace.polyline()` — the object's CURRENT `lines` field.
    // The object stays readable even after the shove dance REMOVES it
    // from the board (memory-alive: the read answers the geometry as
    // of removal). The id-based port models this as the id plus the
    // PICK-TIME polyline snapshot: a live re-read while the id still
    // names the trace (in-place geometry writes are id-preserving
    // here, so the current field is observable), the snapshot once
    // the id is dead. See `picked_trace_lines` + the bug-177 SEAM row.
    let picked_items = pick_traces_at(manager, board, &from_corner, layer);
    let mut picked_trace: Option<(ItemId, Polyline)> = None;
    if let [candidate] = picked_items[..] {
        // Qualify + snapshot in one pass: the snapshot is the `lines`
        // field the Java reference would capture — the tombstone half
        // of the contract below.
        let snapshot = match board.get(candidate) {
            Some(entry) => match &entry.data {
                ItemData::Trace { lines, .. }
                    if crate::trace_ops::nets_equal(&entry.nets, net_slice)
                        && board.trace_half_width(candidate) == Some(half_width)
                        && board.item_clearance_class(candidate) == Some(clearance_class_index) =>
                {
                    Some(lines.clone())
                }
                _ => None,
            },
            None => None,
        };
        if let Some(snapshot) = snapshot {
            picked_trace = Some((candidate, snapshot));
        }
    }

    let tree = manager.default_tree();
    let tree_class = tree.compensated_clearance_class;
    let compensated_half_width = half_width
        + clearance_compensation_value(board.rules(), clearance_class_index, tree_class, layer);

    // Java `:522-535`: wrap the polyline around the obstacles.
    let Some(mut new_polyline) = crate::trace_shover::spring_over_obstacles(
        manager,
        board,
        polyline,
        compensated_half_width,
        layer,
        net_slice,
        clearance_class_index,
        None,
    ) else {
        // Java `:525-535`: the fail row (net-94-gated, probe-side).
        return Some(from_corner);
    };

    // Java `:536-542`: combine with the picked trace's geometry.
    let mut combined_polyline = match &picked_trace {
        None => new_polyline.clone(),
        Some(picked) => new_polyline.combine(Some(&picked_trace_lines(board, picked))),
    };
    if combined_polyline.lines.len() < 3 {
        // Java `:543-553`: the fail row (net-94-gated, probe-side).
        return Some(from_corner);
    }
    let start_shape_no = combined_polyline.lines.len() as i32 - new_polyline.lines.len() as i32;
    // Java `:555-559`: calculate the last shapes of the combined
    // polyline for checking.
    let trace_shapes = combined_polyline.offset_shapes(
        compensated_half_width,
        start_shape_no,
        combined_polyline.lines.len() as i32 - 1,
    );
    let trace_shape_count = trace_shapes.len() as i32;
    let mut last_shape_no = trace_shape_count;
    let orthogonal_mode = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;

    // Java `:561-619`: the shove loop.
    for (i, shape) in trace_shapes.iter().enumerate() {
        let mut current_trace_shape = shape.clone();
        if orthogonal_mode {
            current_trace_shape = box_tile(&current_trace_shape);
        }
        let from_side = ShapeEntrySide::from_entry_no(
            &combined_polyline,
            combined_polyline.corner_count() as i32 - trace_shape_count - 1 + i as i32,
            &current_trace_shape,
        );
        if with_check {
            let check_shove_ok = crate::trace_shover::check(
                manager,
                board,
                &current_trace_shape,
                Some(from_side),
                None,
                layer,
                net_slice,
                clearance_class_index,
                max_recursion_depth,
                max_via_recursion_depth,
                max_spring_over_recursion_depth,
                time_limit,
            );
            if !check_shove_ok {
                last_shape_no = i as i32;
                break;
            }
        }
        let insert_ok = crate::trace_shover::insert(
            manager,
            board,
            &current_trace_shape,
            Some(from_side),
            layer,
            net_slice,
            clearance_class_index,
            &[],
            max_recursion_depth,
            max_via_recursion_depth,
            max_spring_over_recursion_depth,
        );
        if !insert_ok {
            // Java `:616-618`: return null — the database may be
            // damaged.
            return None;
        }
    }

    let mut new_corner = to_corner;
    // Java `:634-746`: the sampling retry for the failing last shape.
    if last_shape_no < trace_shape_count {
        let mut last_trace_shape = trace_shapes[last_shape_no as usize].clone();
        if orthogonal_mode {
            last_trace_shape = box_tile(&last_trace_shape);
        }
        let sample_width = 2 * board.min_trace_half_width();
        let last_corner_approx = new_polyline.corner_approx(last_shape_no + 1);
        let prev_last_corner = new_polyline.corner_approx(last_shape_no);
        let last_segment_length = last_corner_approx.distance(&prev_last_corner);
        if last_segment_length > f64::from(100 * sample_width) {
            // Java `:645-656`: too many cycles to sample; the fail row
            // (net-94-gated, probe-side).
            return Some(from_corner);
        }
        let mut shape_index =
            combined_polyline.corner_count() as i32 - trace_shape_count - 1 + last_shape_no;
        if last_segment_length > f64::from(sample_width) {
            // Java `:659-674`: sample the shove line to a shorter
            // shove distance and try again.
            new_polyline = new_polyline.shorten(
                new_polyline.lines.len() as i32 - (trace_shape_count - last_shape_no - 1),
                f64::from(sample_width),
            );
            let Some(Point::Int(current_last_corner)) = new_polyline.last_corner() else {
                // Java `:663-674`: "IntPoint expected"; the fail row
                // (net-94-gated, probe-side).
                return Some(from_corner);
            };
            new_corner = Point::Int(current_last_corner);
            combined_polyline = match &picked_trace {
                None => new_polyline.clone(),
                Some(picked) => new_polyline.combine(Some(&picked_trace_lines(board, picked))),
            };
            if combined_polyline.lines.len() < 3 {
                // Java `:682-684`: silent newCorner return (no fail
                // row on this arm).
                return Some(new_corner);
            }
            shape_index = combined_polyline.lines.len() as i32 - 3;
            let offset_shape = combined_polyline
                .offset_shape(compensated_half_width, shape_index)
                .expect("shape index in range after the < 3 lines gate");
            last_trace_shape = if orthogonal_mode {
                box_tile(&offset_shape)
            } else {
                offset_shape
            };
        }
        let from_side =
            ShapeEntrySide::from_entry_no(&combined_polyline, shape_index, &last_trace_shape);
        let check_shove_ok = crate::trace_shover::check(
            manager,
            board,
            &last_trace_shape,
            Some(from_side),
            None,
            layer,
            net_slice,
            clearance_class_index,
            max_recursion_depth,
            max_via_recursion_depth,
            max_spring_over_recursion_depth,
            time_limit,
        );
        if !check_shove_ok {
            // Java `:704-730`: the fail row (+ the obstacle row)
            // (net-94-gated, probe-side).
            return Some(from_corner);
        }
        let insert_ok = crate::trace_shover::insert(
            manager,
            board,
            &last_trace_shape,
            Some(from_side),
            layer,
            net_slice,
            clearance_class_index,
            &[],
            max_recursion_depth,
            max_via_recursion_depth,
            max_spring_over_recursion_depth,
        );
        if !insert_ok {
            // Java `:742-745`: return null — the database may be
            // damaged.
            return None;
        }
    }

    // Java `:747-750`: mark every corner of the new polyline BEFORE
    // inserting it.
    for i in 0..new_polyline.corner_count() as i32 {
        let corner = new_polyline.corner_approx(i);
        join_changed_area(board, &corner, layer);
    }
    // Java `:752-754`: insert with the UNcompensated half width.
    let new_trace = crate::trace_ops::insert_trace_without_cleaning(
        manager,
        board,
        new_polyline.clone(),
        layer,
        half_width,
        net_slice,
        clearance_class_index,
        FixedState::Unfixed,
    );
    let Some(new_trace) = new_trace else {
        // T10c divergence: Java `:756` dereferences newTrace
        // unguarded (`newTrace.combine()`) and the NPE propagates OUT
        // of insertForcedTracePolyline — a JVM crash reachable only
        // with an empty polyline (springOver cannot produce one
        // through real pipeline inputs). The port maps the crash onto
        // the method's own damage contract: None.
        return None;
    };
    let _ = crate::trace_ops::combine(manager, board, new_trace);

    // Java `:773-785`: the pull-tight algorithm construction (never
    // null; `minTranslateDist` clamped to >= 100). NOTE the trap: a
    // `tidyWidth` of `Integer.MAX_VALUE` still fires the pull-tight
    // call below — only the tidy REGION construction is skipped.
    let tidy_region = if tidy_width < i32::MAX {
        Some(
            new_corner
                .surrounding_octagon()
                .enlarge(f64::from(tidy_width)),
        )
    } else {
        None
    };
    let opt_net_no_arr: &[i32] = if max_recursion_depth <= 0 {
        net_slice
    } else {
        &[]
    };
    let pull_tight_algo = PullTightAlgo::get_instance(
        opt_net_no_arr,
        tidy_region,
        pull_tight_accuracy,
        Some(new_corner.clone()),
        layer,
    );

    // Java `:787-842`: the swallowing try block — normalize against
    // the DIRECT changedArea dereference, then split at the keep point
    // and re-pick the new trace at newCorner. All three failure faces
    // (normalize false / a thrown normalize exception / the session
    // NPE at :791) land in the catch and skip the split+re-pick. The
    // clip is extracted BEFORE the call, mirroring Java's
    // `changedArea.getArea(layer)` dereference. The no-session face is
    // UNREACHABLE in this function — step 4 starts the session and
    // nothing tears it down mid-function — and were it reachable, Rust
    // would run `normalize(.., None)` (the UNBOUNDED face: it may
    // return true and split), not Java's NPE-swallowed skip. The port
    // keeps Java's extraction order; only the two reachable failure
    // faces behave identically.
    let normalize_clip = board.changed_area.as_ref().map(|area| area.get_area(layer));
    let normalize_result =
        crate::trace_ops::normalize(manager, board, new_trace, normalize_clip.as_ref());
    // The trace object held for the pull-tight gate: Java's newTrace
    // variable — re-picked (or nulled) when the normalize branch ran,
    // the inserted trace otherwise.
    let trace_for_seam = if normalize_result {
        // Java `:809`: pullTightAlgo.splitTracesAtKeepPoint() — the
        // in-scope pick+split driver.
        split_traces_at_keep_point(
            manager,
            board,
            pull_tight_algo.keep_point.as_ref(),
            pull_tight_algo.keep_point_layer,
        );
        // Java `:822-833`: re-pick at newCorner — the keep-point split
        // may have replaced the inserted trace, so the end-corner item
        // must be looked up again (only the FIRST picked item counts,
        // the descending-id head of the pick list; an empty pick sets
        // newTrace = null).
        pick_traces_at(manager, board, &new_corner, layer)
            .first()
            .copied()
    } else {
        Some(new_trace)
    };
    // Java `:860-862`: `if (tidyWidth > 0 && newTrace != null)
    // newTrace.pullTight(pullTightAlgo);` — the pull-tight seam.
    if tidy_width > 0
        && let Some(trace_id) = trace_for_seam
    {
        seam.pull_tight_trace(manager, board, trace_id, &pull_tight_algo);
    }
    Some(new_corner)
}

// ---------------------------------------------------------------------------
// insertForcedTraceSegment (RoutingBoard.java:361-402)
// ---------------------------------------------------------------------------

/// Java `RoutingBoard.insertForcedTraceSegment` (`:361-402`): the
/// per-segment wrapper the M3-T11 `FoundConnectionInserter` loop
/// calls — wraps the two corners into a `Polyline`, runs the forced
/// polyline insert, and maps the result corner back onto the INPUT
/// endpoints. `Some(to_corner)` for equal corners (Java returns the
/// toCorner object verbatim); `None` passes the damage contract of
/// [`insert_forced_trace_polyline`] through (Java `:398` `result =
/// okPoint` with a null `okPoint`).
///
/// Parity note (the anchors' reference-`==` analysis, CONSIDERED):
/// Java maps via REFERENCE equality against `insertPolyline`'s
/// first/last corner objects. Rust compares by value. `==` true
/// implies value-equal, and `==` false with value-equal points falls
/// into Java's `result = okPoint` arm — which yields the SAME VALUE —
/// so the two mappings are indistinguishable at the type surface.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn insert_forced_trace_segment<S: PullTightSeam>(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    seam: &mut S,
    from_corner: &Point,
    to_corner: &Point,
    half_width: i32,
    layer: i32,
    net_numbers: Option<&[i32]>,
    clearance_class_index: i32,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    max_spring_over_recursion_depth: i32,
    tidy_width: i32,
    pull_tight_accuracy: i32,
    with_check: bool,
    time_limit: Option<&TimeLimit>,
) -> Option<Point> {
    if from_corner == to_corner {
        return Some(to_corner.clone());
    }
    let insert_polyline = Polyline::from_two_corners(from_corner, to_corner);
    let ok_point = insert_forced_trace_polyline(
        manager,
        board,
        seam,
        &insert_polyline,
        half_width,
        layer,
        net_numbers,
        clearance_class_index,
        max_recursion_depth,
        max_via_recursion_depth,
        max_spring_over_recursion_depth,
        tidy_width,
        pull_tight_accuracy,
        with_check,
        time_limit,
    );
    Some(match ok_point? {
        ok if Some(&ok) == insert_polyline.first_corner().as_ref() => from_corner.clone(),
        ok if Some(&ok) == insert_polyline.last_corner().as_ref() => to_corner.clone(),
        ok => ok,
    })
}

// ---------------------------------------------------------------------------
// connectToTrace (RoutingBoard.java:1116-1170)
// ---------------------------------------------------------------------------

/// Java `connectToTrace` (`:1116-1170`): inserts a trace from
/// `from_point` to the nearest point on `to_trace`; false when that is
/// not possible without a clearance violation. The port takes the
/// target trace by id — the Java `instanceof PolylineTrace` gate is
/// the trace-kind check (all ported traces are polyline traces).
/// Consumer: M3-T11 `FoundConnectionInserter` (the post-locator
/// target connection).
pub fn connect_to_trace(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    from_point: &IntPoint,
    to_trace: ItemId,
    pen_half_width: i32,
    clearance_class_index: i32,
) -> bool {
    let Some(entry) = board.get(to_trace) else {
        return false; // Java would NPE; unreachable through the seam
    };
    if !matches!(entry.data, ItemData::Trace { .. }) {
        return false; // "not yet implemented" for non-polyline traces
    }
    let trace_lines = board
        .trace_polyline(to_trace)
        .expect("live trace polyline")
        .clone();
    let trace_layer = board.trace_layer(to_trace).expect("live trace layer");
    let trace_nets = board.get(to_trace).expect("checked above").nets.clone();

    let from = Point::Int(*from_point);
    if trace_lines.contains(&from) {
        // no connection line necessary
        return true;
    }
    let Some(projection_line) = trace_lines.projection_line(&from) else {
        return false;
    };
    let connection_line = projection_line.to_polyline();
    if connection_line.lines.len() != 3 {
        // Java also tests `connectionLine == null` — toPolyline never
        // returns null.
        return false;
    }
    if !check_polyline_trace(
        manager,
        board,
        &connection_line,
        trace_layer,
        pen_half_width,
        &trace_nets,
        clearance_class_index,
    ) {
        return false;
    }
    // Java `:1140-1144`: the DIRECT board-field join (not the guarded
    // joinChangedArea wrapper — same effect).
    if let Some(area) = board.changed_area.as_mut() {
        for i in 0..connection_line.corner_count() as i32 {
            let corner = connection_line.corner_approx(i);
            area.join_point(&corner, trace_layer);
        }
    }
    let _ = crate::trace_ops::insert_trace(
        manager,
        board,
        connection_line,
        trace_layer,
        pen_half_width,
        &trace_nets,
        clearance_class_index,
        FixedState::Unfixed,
    );
    // Java `:1154-1168`: remove unconnected tails at the target
    // trace's end corners (the connection may have replaced them),
    // skipping a corner that IS the connection point.
    let first = first_corner(&trace_lines);
    let last = last_corner(&trace_lines);
    for corner in [first, last] {
        let Some(corner) = corner else {
            continue;
        };
        if from != corner
            && let Some(tail) = get_trace_tail(manager, board, &corner, trace_layer, &trace_nets)
        {
            let user_fixed = matches!(
                board.get(tail).map(|entry| &entry.fixed),
                Some(FixedState::UserFixed)
            );
            if !user_fixed {
                crate::trace_ops::remove_item_through_repository(manager, board, tail);
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// removeTraceTails (RoutingBoard.java:1193-1238)
// ---------------------------------------------------------------------------

/// Java `removeTraceTails` (`:1193-1238`): removes all trace tails of
/// the input net (`net_number <= 0`: all nets); true when something
/// was removed. The stub walk, the Via stop-option faces and the
/// descending-id sets are kept verbatim (module docs of
/// [`crate::trace_ops::get_connection_items`] for the walk).
/// Consumer: M3-T11 `FoundConnectionInserter` (tail cleanup around a
/// re-connected target).
pub fn remove_trace_tails(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    net_number: i32,
    stop_connection_option: StopConnectionOption,
) -> bool {
    // Java `this.getItems()` — the repository walk, DESCENDING id.
    // Java interleaves the cheap filters with the (board-mutating)
    // tail/contact queries; the port collects the cheap-filter
    // candidates FIRST (the `item_is_tail` probe needs `&mut board`,
    // which cannot alias the iterator) and then applies the tail +
    // Via faces in the same descending order — all predicates are
    // pure, so the reorder is semantics-preserving.
    let candidates: Vec<ItemId> = board
        .iter_descending()
        .filter(|entry| {
            // Java `currentItem.isRoutable()` — the Item base +
            // Trace/Via override (`!isUserFixed && netCount > 0`; a
            // pin is NEVER routable).
            crate::trace_ops::is_routable(board, entry.id)
                && entry.nets.len() == 1
                && (net_number <= 0 || entry.nets[0] == net_number)
        })
        .map(|entry| entry.id)
        .collect();
    let mut stub_set: Vec<ItemId> = Vec::new();
    for id in candidates {
        if !contacts::item_is_tail(manager, board, id) {
            continue;
        }
        let is_via = matches!(
            board.get(id).map(|entry| &entry.data),
            Some(ItemData::Via { .. })
        );
        if is_via {
            if stop_connection_option == StopConnectionOption::Via {
                continue;
            }
            if stop_connection_option == StopConnectionOption::FanoutVia
                && item_is_fanout_via(manager, board, id, None)
            {
                continue;
            }
        }
        stub_set.push(id);
    }
    // Java collects into a TreeSet (descending under Item.compareTo).
    stub_set.sort_unstable_by(|a, b| b.cmp(a));
    stub_set.dedup();

    let mut stub_connections: Vec<ItemId> = Vec::new();
    for &stub in &stub_set {
        let contact_count = crate::trace_ops::item_normal_contacts(manager, board, stub).len();
        if contact_count == 1 {
            let connection_items = crate::trace_ops::get_connection_items(
                manager,
                board,
                stub,
                stop_connection_option,
            );
            stub_connections.extend(connection_items);
        } else {
            // the connected items are no stubs — for example a via
            // connected on 1 layer but to several traces.
            stub_connections.push(stub);
        }
    }
    // TreeSet semantics: descending + dedup.
    stub_connections.sort_unstable_by(|a, b| b.cmp(a));
    stub_connections.dedup();
    if stub_connections.is_empty() {
        return false;
    }
    remove_items(manager, board, &stub_connections);
    combine_traces(manager, board, net_number);
    true
}

/// Java `Item.isFanoutVia(Set<Item> ignoreItems)` (`Item.java:1250-1291`):
/// true when the item touches (directly or through a short contact
/// trace) an SMD pin — a fanout via is protected from removal. The
/// length gate is `getLength() >= 400 * getHalfWidth()`
/// (`PROTECT_FANOUT_LENGTH`); a pin qualifies when single-layer with
/// at most one normal contact (a PIN's `getNormalContacts` is the
/// Item default — empty — so any single-layer pin qualifies).
pub(crate) fn item_is_fanout_via(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    id: ItemId,
    ignore_items: Option<&[ItemId]>,
) -> bool {
    let contact_list = crate::trace_ops::item_normal_contacts(manager, board, id);
    for contact in contact_list {
        // Classify WITHOUT holding the entry borrow — the layer reads
        // below take `&mut board` (`item_first_layer` is `&mut self`,
        // the component-placement memo).
        let (is_pin, is_trace) = match board.get(contact) {
            Some(entry) => (
                matches!(entry.data, ItemData::Pin { .. }),
                matches!(entry.data, ItemData::Trace { .. }),
            ),
            None => (false, false),
        };
        if is_pin {
            let first_layer = board.item_first_layer(contact);
            let last_layer = board.item_last_layer(contact);
            let pin_contacts = crate::trace_ops::item_normal_contacts(manager, board, contact);
            if first_layer == last_layer && pin_contacts.len() <= 1 {
                return true;
            }
        } else if is_trace {
            if ignore_items.is_some_and(|items| items.contains(&contact)) {
                continue;
            }
            let length = board
                .trace_polyline(contact)
                .map(|lines| lines.length_approx_total())
                .unwrap_or(0.0);
            let half_width = board.trace_half_width(contact).unwrap_or(0);
            if length >= 400.0 * f64::from(half_width) {
                continue;
            }
            let trace_contact_list =
                crate::trace_ops::item_normal_contacts(manager, board, contact);
            for tmp_contact in trace_contact_list {
                let (tmp_is_pin, tmp_is_fixed_exit) = match board.get(tmp_contact) {
                    Some(entry) => (
                        matches!(entry.data, ItemData::Pin { .. }),
                        // look for shove-fixed exit traces of SMD pins
                        matches!(entry.data, ItemData::Trace { .. })
                            && matches!(entry.fixed, FixedState::ShoveFixed)
                            && board
                                .trace_polyline(tmp_contact)
                                .is_some_and(|lines| lines.corner_count() == 2),
                    ),
                    None => (false, false),
                };
                if tmp_is_pin {
                    let first_layer = board.item_first_layer(tmp_contact);
                    let last_layer = board.item_last_layer(tmp_contact);
                    let pin_contacts =
                        crate::trace_ops::item_normal_contacts(manager, board, tmp_contact);
                    if first_layer == last_layer && pin_contacts.len() <= 1 {
                        return true;
                    }
                } else if tmp_is_fixed_exit {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// checkPolylineTrace (BasicBoard.java:1054-1075) + its helpers
// ---------------------------------------------------------------------------

/// Java `Trace.touchingPinsAtEndCorners` (`Trace.java:390-410`), on the
/// UNINSERTED trace (`lines`, `layer`, `half_width`, `net_numbers`,
/// `clearance_class` stand in for the tmp object): the pins of the own
/// net touching either end corner's enlarged surrounding octagon
/// ("acid traps"). Java's `TreeSet<Pin>` → the port's `BTreeSet`
/// (ascending id; the consumer only tests membership). `pub(crate)`
/// since M4-T3: the tightener's smoothen arm re-derives the pin set
/// for the ADJUSTED polyline (`TraceTightener90`'s end-corner faces
/// and `PolylineTrace.pullTight`'s contact read).
pub(crate) fn touching_pins_at_end_corners(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    lines: &Polyline,
    layer: i32,
    half_width: i32,
    net_numbers: &[i32],
    clearance_class: i32,
) -> std::collections::BTreeSet<ItemId> {
    let mut result = std::collections::BTreeSet::new();
    let mut current_end_point = first_corner(lines);
    for i in 0..2 {
        let Some(end_point) = current_end_point else {
            break;
        };
        let current_oct = end_point
            .surrounding_octagon()
            .enlarge(f64::from(half_width));
        let oct_shape = TileShape::RegularTileShape(RegularTileShape::IntOctagon(current_oct));
        let overlaps = manager.overlapping_items_with_clearance(
            board,
            0,
            &oct_shape,
            layer,
            &[],
            clearance_class,
        );
        for item in overlaps {
            let Some(entry) = board.get(item) else {
                continue;
            };
            if matches!(entry.data, ItemData::Pin { .. })
                && crate::trace_ops::nets_equal(&entry.nets, net_numbers)
            {
                result.insert(item);
            }
        }
        current_end_point = if i == 0 { last_corner(lines) } else { None };
    }
    result
}

/// Java `BasicBoard.checkPolylineTrace` (`:1054-1075`): checks if a
/// trace with the input parameters can be inserted WITHOUT shoving —
/// the tmp trace is never added to the board; its tile shapes are the
/// default-tree decomposition (the lazy `getTileShape` of the
/// uninserted item, `Item.java:195-238`), i.e. the compensated offset
/// shapes ([`crate::tree_shapes`] trace arm semantics: offset boxes
/// under the 90-degree tree, octagon-cut offset shapes otherwise).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn check_polyline_trace(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
    layer: i32,
    pen_half_width: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
) -> bool {
    // Java `BasicBoard.checkPolylineTrace` (`:1057-1065`) constructs a
    // tmp PolylineTrace to reuse its geometry helpers — and the Item
    // CONSTRUCTOR assigns the id (`Item.java:87`,
    // `idGenerator.newId()`), so every check call burns one id from the
    // generator even though the tmp is never inserted. The port works
    // on the raw polyline, so the burn must be made explicit to keep
    // the id stream 1:1 with Java: the T10c connect ladder depends on
    // it (c3's failed check burns 113, c2's check burns 114, the c2
    // connection lands on 115 — the capture's `after_c2 new=[115]`).
    let _tmp_trace_id = board.alloc_id();
    let tree = manager.default_tree();
    let tree_class = tree.compensated_clearance_class;
    let variant = tree.variant; // copied out: `manager` is re-borrowed below
    let offset_width = pen_half_width
        + clearance_compensation_value(board.rules(), clearance_class_index, tree_class, layer);
    let contact_pins = touching_pins_at_end_corners(
        manager,
        board,
        polyline,
        layer,
        pen_half_width,
        net_numbers,
        clearance_class_index,
    );
    let count = tile_shape_count(polyline);
    for index in 0..count {
        let index = index as i32;
        let shape: Option<TileShape> = match variant {
            SearchTreeVariant::NinetyDegree => polyline
                .offset_box(offset_width, index)
                .map(|b| TileShape::RegularTileShape(RegularTileShape::IntBox(b))),
            // The base tree's offsetShape == the 45-degree subclass's.
            _ => polyline.offset_shape(offset_width, index),
        };
        let Some(shape) = shape else {
            // Java would NPE on the null shape (out-of-range index);
            // unreachable: index < tileShapeCount <= len - 2.
            continue;
        };
        if !crate::forced_pad_router::check_trace_shape(
            manager,
            board,
            &shape,
            layer,
            net_numbers,
            clearance_class_index,
            Some(&contact_pins),
        ) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Pick + split helpers
// ---------------------------------------------------------------------------

/// Java `pickItems(location, layer, filter)` with the TRACES filter
/// (`BasicBoard.java:1102+`): the descending-id pick list restricted
/// to trace items (Java applies the filter after the query; the
/// descending order is the `TreeSet` under `Item.compareTo`).
pub(crate) fn pick_traces_at(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    location: &Point,
    layer: i32,
) -> Vec<ItemId> {
    manager
        .pick_items(board, location, layer)
        .into_iter()
        .filter(|&id| {
            matches!(
                board.get(id).map(|entry| &entry.data),
                Some(ItemData::Trace { .. })
            )
        })
        .collect()
}

/// Java `TraceTightener.splitTracesAtKeepPoint` (`TraceTightener.java:474-491`):
/// splits the traces containing the keep point; true when something
/// was split. The pick order (descending id) is observable — the
/// FIRST trace that actually splits wins.
pub fn split_traces_at_keep_point(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    keep_point: Option<&Point>,
    keep_point_layer: i32,
) -> bool {
    let Some(keep_point) = keep_point else {
        return false;
    };
    let picked_items = pick_traces_at(manager, board, keep_point, keep_point_layer);
    for trace_id in picked_items {
        if crate::trace_ops::split_at_point(manager, board, trace_id, keep_point).is_some() {
            return true;
        }
    }
    false
}

/// The `TileShape` box projection of Java's `shape.boundingBox()`
/// (the orthogonal-mode arm).
fn box_tile(shape: &TileShape) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(shape.bounding_box()))
}

#[cfg(test)]
mod tests {
    //! Capture pins for the forced-insertion surface. Jar oracle:
    //! `rust/harness/oracle/ForcedInsertProbe.java` on the
    //! `locator-spike/t9_locator45.dsn` fixture — captures
    //! `logs/M3-T10c/captures/forced_insert_rows_run1.jsonl` +
    //! `_run2.jsonl`, double-run byte-identical. Every world below is a
    //! verbatim replay of a probe world (same fixture parse, same seed
    //! order so the ids replay 105+, same call parameters); every
    //! literal (ids, corners, moved centers, changed-area octagon
    //! fields) is read off the capture rows.
    //!
    //! The native `compare_trace_*` rows the probe taps are emitted by
    //! the Java ENGINE; the Rust replay reproduces their EFFECTS — the
    //! id deltas, the moved via, the wrapped substitute piece — through
    //! the same code paths this module pins.

    use super::*;
    use crate::components::BoardPadstack;
    use crate::drill_item_mover::insert_via;
    use crate::items::BoardShape;
    use crate::test_util::parse_board_from_path;
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;

    /// The probe's net-94 selection: the native fail-row gate compares
    /// the net NUMBER (`netNumbers[0] == 94`), and on this fixture the
    /// NAME-to-number mapping is off by one (N094 has number 94+1).
    const NET: i32 = 94;
    const FOREIGN_NET: i32 = 2;
    const THIRD_NET: i32 = 3;
    const FIFTH_NET: i32 = 96;

    /// Java `Limits.CRIT_INT` — the `IntOctagon.EMPTY` sentinel fields
    /// the fail world's untouched session reports (`after_fail` rows:
    /// all eight fields +-33554432, the surrounding box inverted).
    const CRIT_INT: i32 = 33_554_432;

    fn p(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// A fresh fixture parse + tree reinsert (the probe's `parse`).
    fn parse_fixture() -> Board {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        board
    }

    /// The probe's +-250 through-padstack (`addViaPadstack`).
    fn add_via_padstack(board: &mut Board, name: &str) -> i32 {
        let rect = || {
            Some(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(-250, -250),
                    IntPoint::new(250, 250),
                )),
            )))
        };
        let layer_count = board.library().padstacks[0].shapes.len();
        board.library_mut().padstacks.push(BoardPadstack {
            name: name.to_string(),
            shapes: vec![rect(); layer_count],
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        board.library().padstacks.len() as i32
    }

    /// All live item ids, ascending (the probe's inventory rows sort by
    /// id).
    fn live_ids(board: &Board) -> Vec<i32> {
        let mut ids: Vec<i32> = board.iter_descending().map(|e| e.id.get() as i32).collect();
        ids.sort_unstable();
        ids
    }

    fn trace_is(board: &Board, id: ItemId) -> bool {
        board
            .get(id)
            .is_some_and(|e| matches!(e.data, ItemData::Trace { .. }))
    }

    /// `assert_corners`: the literal corner chain of a live trace
    /// (corner_approx values as i32 pairs).
    #[track_caller]
    fn assert_corners(board: &Board, id: ItemId, expected: &[(i32, i32)]) {
        let lines = board.trace_polyline(id).expect("live trace polyline");
        assert_eq!(
            lines.corner_count(),
            expected.len(),
            "corner count of {id:?}"
        );
        for (i, (x, y)) in expected.iter().enumerate() {
            let c = lines.corner_approx(i as i32);
            assert_eq!(
                (c.x, c.y),
                (f64::from(*x), f64::from(*y)),
                "corner {i} of {id:?}"
            );
        }
    }

    /// The 8 changed-area octagon fields in capture-row order:
    /// leftX, bottomY, rightX, topY, upperLeft, lowerRight, lowerLeft,
    /// upperRight diagonal x.
    #[track_caller]
    fn assert_area(board: &Board, layer: i32, e: [i32; 8]) {
        let area = board
            .changed_area
            .as_ref()
            .expect("changed-area session active");
        let oct = area.get_area(layer);
        assert_eq!(oct.left_x, e[0], "L{layer} leftX");
        assert_eq!(oct.bottom_y, e[1], "L{layer} bottomY");
        assert_eq!(oct.right_x, e[2], "L{layer} rightX");
        assert_eq!(oct.top_y, e[3], "L{layer} topY");
        assert_eq!(
            oct.upper_left_diagonal_x, e[4],
            "L{layer} upperLeftDiagonalX"
        );
        assert_eq!(
            oct.lower_right_diagonal_x, e[5],
            "L{layer} lowerRightDiagonalX"
        );
        assert_eq!(
            oct.lower_left_diagonal_x, e[6],
            "L{layer} lowerLeftDiagonalX"
        );
        assert_eq!(
            oct.upper_right_diagonal_x, e[7],
            "L{layer} upperRightDiagonalX"
        );
    }

    /// The probe's `findNet94TraceAt`: the single-layer-0 net-94 trace
    /// containing the point.
    fn find_net_trace_at(board: &Board, at: &Point, net: i32) -> ItemId {
        for entry in board.iter_descending() {
            if let ItemData::Trace { layer, lines, .. } = &entry.data
                && entry.nets == vec![net]
                && *layer == 0
                && lines.contains(at)
            {
                return entry.id;
            }
        }
        panic!("no net-{net} trace at {at:?}");
    }

    /// The probe's insert world (W1 `insert_tidy` / W2
    /// `insert_notidy`): own net-94 trace 105 ending at the center
    /// (500000,300000), foreign N002 vertical trace 106 crossing at
    /// 501000, N003 via 107 at (500600,300000).
    struct InsertWorld {
        #[allow(dead_code)] // capture world-row literals
        own: ItemId,
        #[allow(dead_code)] // capture world-row literals
        cross: ItemId,
        via: ItemId,
    }

    fn build_insert_world() -> (SearchTreeManager, Board, InsertWorld) {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let padstack = add_via_padstack(&mut board, "insert_via");

        let own = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(498000, 300000), &p(500000, 300000)),
            0,
            100,
            &[NET],
            0,
            FixedState::Unfixed,
        )
        .expect("probe own trace");
        assert_eq!(own.get(), 105, "capture ownTrace id");
        let cross = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(501000, 298000), &p(501000, 302000)),
            0,
            100,
            &[FOREIGN_NET],
            0,
            FixedState::Unfixed,
        )
        .expect("probe cross trace");
        assert_eq!(cross.get(), 106, "capture crossTrace id");
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(500600, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(via.get(), 107, "capture corridorVia id");
        assert_eq!(live_ids(&board).len(), 107, "capture before-inventory size");
        (manager, board, InsertWorld { own, cross, via })
    }

    /// The W2 success ladder (`insert_notidy` capture rows): the forced
    /// segment (500000,300000)->(502000,300000) shoves the corridor via
    /// north (to (500600,300368), its old +-250 box joined on BOTH
    /// layers), wraps the cross trace around the corridor's compensated
    /// end cap (substitute piece 111, the 7-line circuit — the
    /// `compare_trace_shove_shape` row's idBefore 108 / idAfter 111 /
    /// delta 3 + the transient 4-id shove loop delta), combines with
    /// the own trace (112 = the merged 498000->502000 line; the
    /// `insert_and_combine` row), normalize does NOT split
    /// (`result=false`, delta 0). The changed-area session stays ACTIVE
    /// (Java tears it down only in `optChangedArea`) with the capture's
    /// L0/L1 octagons — the join web (piece corners 502217 on L0, the
    /// via's OLD box on L1, the P corners, the combine start corner)
    /// verified field by field.
    #[track_caller]
    fn run_and_pin_insert_world() -> (SearchTreeManager, Board, InsertWorld) {
        let (mut manager, mut board, w) = build_insert_world();
        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(&p(500000, 300000), &p(502000, 300000)),
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            10,
            2,
            0, // tidyWidth: the W2 face
            100,
            true,
            None,
        );
        assert_eq!(
            result,
            Some(p(502000, 300000)),
            "capture insert_result (the to corner)"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            None,
            "the shove ladder succeeded"
        );
        // after_insert inventory: 105 (own, absorbed by combine) and
        // 106 (cross, replaced by the wrapped substitute) gone; 111 +
        // 112 new; the via 107 moved north by 368.
        let ids = live_ids(&board);
        assert_eq!(ids.len(), 107, "capture after-inventory size (-2 +2)");
        assert!(!ids.contains(&105), "own trace absorbed");
        assert!(!ids.contains(&106), "cross trace replaced");
        assert!(ids.contains(&111), "capture substitute piece id");
        assert!(ids.contains(&112), "capture merged trace id");
        let via_center = board.drill_center(w.via).expect("live via");
        assert_eq!(
            via_center,
            p(500600, 300368),
            "capture moved via center (shoveDodgedVia)"
        );
        assert_corners(
            &board,
            ItemId::new(111),
            &[
                (501000, 298000),
                (501000, 299783),
                (502089, 299783),
                (502217, 299911),
                (502217, 300089),
                (502089, 300217),
                (501000, 300217),
                (501000, 302000),
            ],
        );
        assert_corners(
            &board,
            ItemId::new(112),
            &[(498000, 300000), (502000, 300000)],
        );
        assert!(
            trace_is(&board, ItemId::new(112)),
            "112 is the surviving net-94 trace"
        );
        // The changed-area octagons (capture after_insert rows, L0+L1).
        assert_area(
            &board,
            0,
            [
                500000, 299750, 502217, 300250, 200000, 202306, 800000, 802306,
            ],
        );
        assert_area(
            &board,
            1,
            [
                500350, 299750, 500850, 300250, 200100, 201100, 800100, 801100,
            ],
        );
        let box_ = board
            .changed_area
            .as_ref()
            .expect("session")
            .surrounding_box();
        assert_eq!(
            (box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y),
            (500000, 299750, 502217, 300250),
            "capture surroundingBox"
        );
        (manager, board, w)
    }

    /// The W2 replay (tidyWidth 0) — the full success ladder pins.
    #[test]
    fn t10c_insert_notidy_success_ladder() {
        run_and_pin_insert_world();
    }

    /// The W1 face (`insert_tidy`, tidyWidth 400): Java's pullTight RUNS
    /// (tidyWidth > 0, the seam gate fires) but is a NO-OP on the
    /// straight trace — the capture's W1 rows are IDENTICAL to W2's
    /// (pickedAtEndCorner 1 before and after, same inventory, same
    /// octagons). The port's no-op seam therefore produces the same
    /// board, and this test pins exactly that equality: the entire
    /// W2 pin set replays green at tidyWidth 400. (The divergence the
    /// no-op seam costs on pull-tight-sensitive geometry is a
    /// documented M4 bank — module docs, "The pull-tight seam".)
    #[test]
    fn t10c_insert_tidy_noop_seam_matches_capture() {
        let (mut manager, mut board, _w) = build_insert_world();
        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(&p(500000, 300000), &p(502000, 300000)),
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            10,
            2,
            400, // tidyWidth: the W1 face
            100,
            true,
            None,
        );
        assert_eq!(result, Some(p(502000, 300000)));
        // The W1 after_insert rows equal W2's — spot-pins from the same
        // capture block (the full set is W2's test).
        let ids = live_ids(&board);
        assert_eq!(ids.len(), 107);
        assert!(!ids.contains(&105) && !ids.contains(&106));
        assert!(ids.contains(&111) && ids.contains(&112));
        assert_area(
            &board,
            0,
            [
                500000, 299750, 502217, 300250, 200000, 202306, 800000, 802306,
            ],
        );
    }

    /// `TraceTightener.getInstance` (`TraceTightener.java:76-113` via
    /// the [`PullTightAlgo`] projection): never null, `minTranslateDist`
    /// clamped to at least 100 — the clamp mutant `.max(1000)` (or the
    /// unclamped pass-through) is killed by the 42 accuracy.
    #[test]
    fn t10c_pull_tight_algo_clamp() {
        let algo = PullTightAlgo::get_instance(&[], None, 42, None, 0);
        assert_eq!(algo.min_translate_dist, 100, "the >= 100 clamp");
        assert_eq!(
            PullTightAlgo::get_instance(&[], None, 250, None, 0).min_translate_dist,
            250,
            "a larger accuracy passes through"
        );
    }

    /// The W2 connectToTrace ladder (`c1_contains` / `c3_blocked` /
    /// `c2_insert` capture rows, in that order — c2 LAST because its
    /// tail removal deletes the target).
    ///
    /// * c1 (500500,300000) ON the merged trace 112: the contains
    ///   short-circuit — true, NO insert, NO area growth.
    /// * c3 (500500,301200): the 3-line connection would cross the
    ///   shoved corridor (via 107 now at (500600,300368)) — the
    ///   checkPolylineTrace gate answers false, NO insert.
    /// * c2 (500200,301200): the insert face — connection 115
    ///   (500200,301200)->(500200,300000) inserted, then the end-corner
    ///   tail walk finds 112 unconnected at BOTH original ends (its
    ///   mid-span contact with 115 is not a normal contact) and
    ///   removes the WHOLE target — the capture's
    ///   `after_c2` gone=[112] new=[115] + the area growth
    ///   (topY 300250 -> 301200, upperLeft 200000 -> 199000).
    #[test]
    fn t10c_connect_ladder_contains_blocked_insert() {
        let (mut manager, mut board, _w) = run_and_pin_insert_world();
        let target = find_net_trace_at(&board, &p(500500, 300000), NET);
        assert_eq!(target.get(), 112, "the capture target id");

        // c1: contains short-circuit.
        let c1 = connect_to_trace(
            &mut manager,
            &mut board,
            &IntPoint::new(500500, 300000),
            target,
            100,
            0,
        );
        assert!(c1, "capture c1_contains=true");
        let ids_after_c1 = live_ids(&board);
        assert!(!ids_after_c1.contains(&115), "c1 inserts nothing");
        // capture after_c1: no growth.
        assert_area(
            &board,
            0,
            [
                500000, 299750, 502217, 300250, 200000, 202306, 800000, 802306,
            ],
        );

        // c3: check-blocked face.
        let c3 = connect_to_trace(
            &mut manager,
            &mut board,
            &IntPoint::new(500500, 301200),
            target,
            100,
            0,
        );
        assert!(!c3, "capture c3_blocked=false");
        let ids_after_c3 = live_ids(&board);
        assert_eq!(ids_after_c3, ids_after_c1, "c3 changes nothing");
        // capture after_c3: no growth.
        assert_area(
            &board,
            0,
            [
                500000, 299750, 502217, 300250, 200000, 202306, 800000, 802306,
            ],
        );

        // c2: the insert face, LAST.
        let c2 = connect_to_trace(
            &mut manager,
            &mut board,
            &IntPoint::new(500200, 301200),
            target,
            100,
            0,
        );
        assert!(c2, "capture c2_insert=true");
        let ids = live_ids(&board);
        assert!(
            !ids.contains(&112),
            "capture after_c2 gone=[112]: the target removed by the tail walk"
        );
        assert!(ids.contains(&115), "capture after_c2 new=[115]");
        // Capture `after_c2` inventory ids >= 105 = [107, 111, 115]: Java's
        // normalize split the target 112 into transient pieces 116/117 and
        // the end-corner tail walk removed BOTH. Asserting their absence (and
        // the total inventory) is what witnesses the tail-walk removal —
        // `!contains(&112)` alone is already true from the normalize split.
        assert_eq!(
            ids.len(),
            107,
            "capture after_c2 inventory: 104 fixture items + [107, 111, 115]"
        );
        assert!(
            !ids.contains(&116) && !ids.contains(&117),
            "the transient normalize-split pieces are gone: the end-corner tail walk removed them"
        );
        assert_corners(
            &board,
            ItemId::new(115),
            &[(500200, 301200), (500200, 300000)],
        );
        assert!(
            matches!(
                board.get(ItemId::new(115)).map(|e| e.fixed),
                Some(FixedState::Unfixed)
            ),
            "the connection is inserted UNFIXED"
        );
        assert_area(
            &board,
            0,
            [
                500000, 299750, 502217, 301200, 199000, 202306, 800000, 802306,
            ],
        );
        assert_area(
            &board,
            1,
            [
                500350, 299750, 500850, 300250, 200100, 201100, 800100, 801100,
            ],
        );
        let box_ = board
            .changed_area
            .as_ref()
            .expect("session")
            .surrounding_box();
        assert_eq!(
            (box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y),
            (500000, 299750, 502217, 301200),
            "capture after_c2 surroundingBox"
        );
    }

    /// The post-c2 ID-STREAM position (oracle `c2debug` run — the
    /// throwaway probe world that replays the c2 steps with inventory
    /// dumps between them; not a capture row, so it lives in its own
    /// test). Java ground truth: c3's failed `checkPolylineTrace` burns
    /// the tmp-trace id 113, c2's check burns 114, the connection
    /// inserts as 115, and the connection's normalize SPLITS the target
    /// 112 at the touch point — `PolylineTrace.split(int, Line)`
    /// (`:713-756`) removes 112 and constructs BOTH pieces through
    /// `insertTraceWithoutCleaning`, burning 116 (west) and 117 (east);
    /// the end-corner tail walk then removes both pieces. Live
    /// inventory is 115 only, but the generator watermark is 117: the
    /// port must have burned the same two transient ids, i.e. its
    /// normalize found-trace split must have FIRED (a whole-item tail
    /// removal without the split would end at watermark 115).
    #[test]
    fn t10c_c2_id_stream_transient_split_pieces() {
        let (mut manager, mut board, _w) = run_and_pin_insert_world();
        let target = find_net_trace_at(&board, &p(500500, 300000), NET);
        // c3: the check-blocked face (burns the tmp id 113).
        assert!(!connect_to_trace(
            &mut manager,
            &mut board,
            &IntPoint::new(500500, 301200),
            target,
            100,
            0,
        ));
        // c2: the insert face (burns the check tmp 114, inserts the
        // connection 115, splits 112 into transient 116/117).
        assert!(connect_to_trace(
            &mut manager,
            &mut board,
            &IntPoint::new(500200, 301200),
            target,
            100,
            0,
        ));
        assert_eq!(
            board.max_generated_id(),
            117,
            "oracle c2debug: the transient split pieces burned 116+117"
        );
    }

    /// Java `removeTraceTails(p_net_no, ...)` net gate
    /// (`RoutingBoard.java:1203-1206`): `p_net_no <= 0` matches ALL
    /// nets — net 0 included (structural: the capture worlds only call
    /// with -1; the `<=` vs `<` boundary at 0 is Java-verbatim and
    /// pinned here). A net-0 call must remove everything, exactly like
    /// the -1 NONE face.
    #[test]
    fn t10c_remove_tails_net_zero_matches_all_nets() {
        let (mut manager, mut board, _w) = build_tails_world();
        let removed = remove_trace_tails(&mut manager, &mut board, 0, StopConnectionOption::None);
        assert!(removed, "net 0 is the all-nets face");
        let ids = live_ids(&board);
        for gone in [105, 106, 107, 108] {
            assert!(!ids.contains(&gone), "net 0 removes {gone}");
        }
    }

    /// The `optChangedArea` skeleton (`RoutingBoardOperations.java
    /// :52-79`) without the M4 tightener: the null-session early-out
    /// keeps no session, the EMPTY-clip gate skips the seam arm, and
    /// the teardown (`:78`) runs in BOTH gated and ungated faces — the
    /// session is consumed exactly once. (The EMPTY-gate inversion is
    /// unobservable through the no-op seam — M4 pins the seam arm; the
    /// teardown + early-out faces are the T10c surface.)
    #[test]
    fn t10c_opt_changed_area_skeleton_faces() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);

        // No session: early-out, nothing to tear down.
        opt_changed_area(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &[],
            Some(&IntOctagon::EMPTY),
            100,
            None,
            0,
            None,
            None,
            0,
            false,
        );
        assert!(board.changed_area.is_none(), "early-out keeps no session");

        // Session + EMPTY clip: the seam arm is skipped, the teardown
        // still consumes the session.
        start_marking_changed_area(&mut board);
        join_changed_area(&mut board, &FloatPoint::new(0.0, 0.0), 0);
        opt_changed_area(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &[],
            Some(&IntOctagon::EMPTY),
            100,
            None,
            0,
            None,
            None,
            0,
            false,
        );
        assert!(board.changed_area.is_none(), "EMPTY-clip face tears down");

        // Session + non-EMPTY clip: the (no-op) seam arm runs, the
        // teardown consumes the session.
        start_marking_changed_area(&mut board);
        join_changed_area(&mut board, &FloatPoint::new(1000.0, 1000.0), 1);
        let clip = board.changed_area.as_ref().expect("session").get_area(1);
        opt_changed_area(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &[],
            Some(&clip),
            100,
            None,
            0,
            None,
            None,
            0,
            false,
        );
        assert!(board.changed_area.is_none(), "ungated face tears down");
    }

    /// The W3 fail world (`insert_fail`): a straight 400000-long
    /// corridor from (500000,300000) through the UNFIXED N003 via 105
    /// at (510300,300000) with `maxViaRecursionDepth` 0 — the via
    /// ladder burns out immediately, the main-loop check fails on shape
    /// 0 (`lastShapeNo=0`, `traceShapes.length=7`: springOver wrapped
    /// the corridor around a fixture obstacle), the wrapped first
    /// segment is short so the sampling retry SKIPS the shorten arm and
    /// re-checks shape 0 — failing again: Java emits the native
    /// `compare_trace_insert_forced_fail` +
    /// `compare_trace_insert_forced_obstacle` rows
    /// (`failing obstacle=via: ... Obstacle=Via#105`) and returns
    /// fromCorner. Pins: result = from corner, the failing obstacle IS
    /// via 105, the inventory UNDAMAGED (105 items, via unmoved), and
    /// the changed-area session ACTIVE-but-EMPTY — all eight octagon
    /// fields at the +-CRIT_INT sentinels on both layers, the
    /// surrounding box inverted (the fail path joins nothing and never
    /// tears the session down).
    #[test]
    fn t10c_insert_fail_via_budget_zero() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let padstack = add_via_padstack(&mut board, "fail_via");
        assert_eq!(
            board.min_trace_half_width(),
            10000,
            "capture minTraceHalfWidth"
        );
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(510300, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(via.get(), 105, "capture blockingVia id");
        let before = live_ids(&board);
        assert_eq!(before.len(), 105);

        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(&p(500000, 300000), &p(900000, 300000)),
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            0, // maxViaRecursionDepth: the budget-0 face
            2,
            0,
            100,
            true,
            None,
        );
        assert_eq!(
            result,
            Some(p(500000, 300000)),
            "capture insert_result: fromCorner"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(via),
            "capture failing_obstacle Via#105"
        );
        assert_eq!(
            live_ids(&board),
            before,
            "the fail path leaves the board UNDAMAGED"
        );
        assert_eq!(
            board.drill_center(via),
            Some(p(510300, 300000)),
            "the blocking via unmoved"
        );
        // The active-but-empty session (after_fail rows).
        assert!(
            board.changed_area.is_some(),
            "the session survives the fail path"
        );
        let empty = [
            CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT,
        ];
        assert_area(&board, 0, empty);
        assert_area(&board, 1, empty);
        let box_ = board
            .changed_area
            .as_ref()
            .expect("session")
            .surrounding_box();
        assert_eq!(
            (box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y),
            (CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT),
            "capture after_fail surroundingBox (inverted)"
        );
    }

    /// `checkForcedTracePolyline` (`:408-448`): the no-change probe.
    /// Structural pins (the probe never calls it; the machinery is the
    /// insert path's check loop, capture-pinned above): shoveable
    /// corridor on the W2 pre-state answers true; the W3 corridor at
    /// via budget 0 answers false with via 105 reported.
    #[test]
    fn t10c_check_forced_trace_polyline_faces() {
        // true face: the W2 corridor pre-insert.
        let (mut manager, mut board, _w) = build_insert_world();
        assert!(check_forced_trace_polyline(
            &mut manager,
            &mut board,
            &Polyline::from_two_corners(&p(500000, 300000), &p(502000, 300000)),
            100,
            0,
            &[NET],
            0,
            10,
            10,
            2,
        ));
        assert_eq!(live_ids(&board).len(), 107, "the check changes nothing");
        assert_eq!(board.shove_failing_obstacle(), None);

        // false face: the W3 corridor at via budget 0.
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let padstack = add_via_padstack(&mut board, "check_fail_via");
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(510300, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert!(!check_forced_trace_polyline(
            &mut manager,
            &mut board,
            &Polyline::from_two_corners(&p(500000, 300000), &p(900000, 300000)),
            100,
            0,
            &[NET],
            0,
            10,
            0,
            2,
        ));
        // Unlike the insert path (whose springOver wrap detours the raw
        // corridor around the fixture obstacles, so the first failing
        // shape reports the via — capture-pinned above), the BARE check
        // offset-shapes the unwrapped corridor and fails on the first
        // raw obstacle it meets. The probe never calls the bare check,
        // so no capture row pins WHICH item — only the verdict and the
        // fact that an obstacle is reported.
        assert!(
            board.shove_failing_obstacle().is_some(),
            "an obstacle is reported"
        );
        let _ = via;
    }

    /// The W4 tails world (`tails_*` capture worlds): floating net-94
    /// stub A (105), N096 via Vn (106) + stub C (107) at the
    /// center-east, and the net-94 fanout via Vf (108) ON the SMD pin
    /// (capture pin94: id 97 at (663500,20000), layers 0:0).
    struct TailsWorld {
        #[allow(dead_code)] // capture world-row literals
        stub_a: ItemId,
        vn: ItemId,
        stub_c: ItemId,
        vf: ItemId,
    }

    fn build_tails_world() -> (SearchTreeManager, Board, TailsWorld) {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let padstack = add_via_padstack(&mut board, "tails_via");

        // The capture's net-94 SMD pin: id 97 at (663500,20000).
        let pin97 = board
            .iter_descending()
            .find(|e| {
                matches!(e.data, ItemData::Pin { .. }) && e.nets == vec![NET] && e.id.get() == 97
            })
            .map(|e| e.id)
            .expect("capture pin94 id 97 on the net");
        assert_eq!(
            board.drill_center(pin97),
            Some(p(663500, 20000)),
            "capture pin94Center"
        );

        let stub_a = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(496000, 300000), &p(498000, 300000)),
            0,
            100,
            &[NET],
            0,
            FixedState::Unfixed,
        )
        .expect("stub A");
        assert_eq!(stub_a.get(), 105, "capture stubA id");
        let vn = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(503000, 300000),
            &[FIFTH_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(vn.get(), 106, "capture nonFanoutVia id");
        let stub_c = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(503000, 300000), &p(504000, 300000)),
            0,
            100,
            &[FIFTH_NET],
            0,
            FixedState::Unfixed,
        )
        .expect("stub C");
        assert_eq!(stub_c.get(), 107, "capture stubC id");
        let vf = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(663500, 20000),
            &[NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(vf.get(), 108, "capture fanoutVia id");
        assert_eq!(live_ids(&board).len(), 108, "capture before-inventory size");
        (
            manager,
            board,
            TailsWorld {
                stub_a,
                vn,
                stub_c,
                vf,
            },
        )
    }

    /// `removeTraceTails(-1, NONE)` (`tails_none` capture): everything
    /// is a tail under no stop protection — the whole seed set
    /// {105,106,107,108} goes (the fanout via TOO: NONE protects
    /// nothing), result true. Vn+stubC die as one connection component.
    #[test]
    fn t10c_remove_tails_none_removes_everything() {
        let (mut manager, mut board, w) = build_tails_world();
        let removed = remove_trace_tails(&mut manager, &mut board, -1, StopConnectionOption::None);
        assert!(removed, "capture remove_tails result=true");
        let ids = live_ids(&board);
        for gone in [105, 106, 107, 108] {
            assert!(!ids.contains(&gone), "NONE removes {gone}");
        }
        let _ = (w.stub_a, w.vn, w.stub_c, w.vf);
    }

    /// `removeTraceTails(-1, VIA)` (`tails_via` capture): vias are
    /// skipped as stubs AND stop the connection walk — only the trace
    /// stubs {105, 107} die; Vn (106) and Vf (108) survive.
    #[test]
    fn t10c_remove_tails_via_spares_vias() {
        let (mut manager, mut board, w) = build_tails_world();
        let removed = remove_trace_tails(&mut manager, &mut board, -1, StopConnectionOption::Via);
        assert!(removed, "capture remove_tails result=true");
        let ids = live_ids(&board);
        assert!(!ids.contains(&105), "stub A removed");
        assert!(!ids.contains(&107), "stub C removed");
        assert!(ids.contains(&106), "Vn survives the VIA option");
        assert!(
            ids.contains(&(w.vf.get() as i32)),
            "Vf survives the VIA option"
        );
    }

    /// `removeTraceTails(-1, FANOUT_VIA)` (`tails_fanout_via` capture):
    /// the fanout via Vf (108, touching the single-layer net-94 SMD
    /// pin) is PROTECTED, but the plain via Vn (106 — no SMD pin in its
    /// short-contact reach) is not: {105,106,107} die, 108 survives.
    #[test]
    fn t10c_remove_tails_fanout_via_protects_only_fanout() {
        let (mut manager, mut board, w) = build_tails_world();
        assert!(
            item_is_fanout_via(&mut manager, &mut board, w.vf, None),
            "Vf is a fanout via (SMD pin contact)"
        );
        assert!(
            !item_is_fanout_via(&mut manager, &mut board, w.vn, None),
            "Vn is NOT a fanout via (no SMD pin reachable)"
        );
        let removed = remove_trace_tails(
            &mut manager,
            &mut board,
            -1,
            StopConnectionOption::FanoutVia,
        );
        assert!(removed, "capture remove_tails result=true");
        let ids = live_ids(&board);
        for gone in [105, 106, 107] {
            assert!(!ids.contains(&gone), "FANOUT_VIA removes {gone}");
        }
        assert!(
            ids.contains(&(w.vf.get() as i32)),
            "the fanout via survives"
        );
    }

    // ---- fix-round pins (spec review 1: MINOR-1/2/3) ----

    /// Spec-review MINOR-1: the sampling-retry SHORTEN arm, observed on
    /// SUCCESS (capture `forced_insert_rows_fix_run{1,2}`, world
    /// `insert_retry`; `cmp`-identical double run). Bare fixture +
    /// blocking via 105 at (560000,300000): the straight corridor
    /// (500000,300000)->(900000,300000) at via budget 0 fails the
    /// shape-0 check on the via (`lastShapeNo=0 <
    /// traceShapes.length=7`), and the last segment (400000) lies
    /// strictly between sampleWidth (20000 = 2*minTraceHalfWidth) and
    /// 100*sampleWidth — so the shorten arm clips the corridor to
    /// (520000,300000), the re-check on the SHORTENED shape PASSES, and
    /// the returned newCorner is the sampled IntPoint, not the
    /// toCorner (unlike the W3 fail world, whose re-check failed and
    /// discarded it). Discriminates: halving `2*` moves the sampled
    /// corner to (510000,300000); dropping the shorten arm re-fails the
    /// full-shape check (fromCorner, no trace); a 10x cap instead of
    /// 100x flips to the too-many-cycles arm (400000 > 200000).
    #[test]
    fn t10c_retry_shorten_arm_samples_the_corner() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let padstack = add_via_padstack(&mut board, "retry_via");
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(560000, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(via.get(), 105, "capture blockingVia id");

        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(&p(500000, 300000), &p(900000, 300000)),
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            0, // maxViaRecursionDepth: the budget-0 face
            2,
            0,
            100,
            true,
            None,
        );
        assert_eq!(
            result,
            Some(p(520000, 300000)),
            "capture insert_result: the SAMPLED corner (shorten arm), not the toCorner"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(via),
            "capture failing_obstacle Via#105 (set by the loop's failed check; the successful retry does not clear it)"
        );
        assert_eq!(
            board.drill_center(via),
            Some(p(560000, 300000)),
            "the blocking via unmoved"
        );
        // after_retry inventory: the shortened trace 106 is the only
        // new item; the retry's check/recombine burned nothing beyond
        // it.
        let ids = live_ids(&board);
        assert_eq!(ids.len(), 106, "capture after-inventory size (+1)");
        assert!(ids.contains(&106), "capture shortened-trace id");
        assert_corners(
            &board,
            ItemId::new(106),
            &[(500000, 300000), (520000, 300000)],
        );
        assert_eq!(
            board.max_generated_id(),
            106,
            "stream watermark: no hidden id burns past the trace"
        );
        // after_retry octagons: the shortened polyline's two corners on
        // L0; the session exists but nothing joined on L1.
        assert_area(
            &board,
            0,
            [
                500000, 300000, 520000, 300000, 200000, 220000, 800000, 820000,
            ],
        );
        let empty = [
            CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT,
        ];
        assert_area(&board, 1, empty);
        let box_ = board
            .changed_area
            .as_ref()
            .expect("session")
            .surrounding_box();
        assert_eq!(
            (box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y),
            (500000, 300000, 520000, 300000),
            "capture after_retry surroundingBox"
        );
    }

    /// Spec-review MINOR-2: the forced_pad / drill-join RUN faces under
    /// a LIVE marking session (capture `forced_insert_rows_fix_run{1,2}`,
    /// world `drill_move`). Via 105 (single-layer +-250 padstack, net 3)
    /// sits at the board center; the N002 vertical trace 106 crosses the
    /// move target at x=504000. With the session started,
    /// `DrillItemMover.insert` (Java :110-167) shoves the trace aside at
    /// the target — ForcedPadRouter re-inserts the substitute pieces and
    /// joins their corners (:434-436), then normalizes each with the
    /// live clip (:440-450) — and joins the four corners of the
    /// UNTRANSLATED +-250 box (:159-162). The two join webs sit ~4000
    /// apart, so the pinned octagon discriminates either join dropped
    /// INDEPENDENTLY (box leftX 499750 vs piece rightX 504367).
    #[test]
    fn t10c_drill_move_run_faces_join_under_live_session() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        // The probe's single-layer padstack: shape slot 0 only
        // (`padstacks.add(shape, 0, 0)`), so the per-layer join loop
        // runs layer 0 alone and L1 stays untouched.
        let layer_count = board.library().padstacks[0].shapes.len();
        let mut shapes: Vec<Option<BoardShape>> = (0..layer_count).map(|_| None).collect();
        shapes[0] = Some(BoardShape::Tile(TileShape::RegularTileShape(
            RegularTileShape::IntBox(IntBox::new(
                IntPoint::new(-250, -250),
                IntPoint::new(250, 250),
            )),
        )));
        board.library_mut().padstacks.push(BoardPadstack {
            name: "move_via".to_string(),
            shapes,
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        let padstack = board.library().padstacks.len() as i32;
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(500000, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(via.get(), 105, "capture movedVia id");
        let cross = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(504000, 297000), &p(504000, 303000)),
            0,
            100,
            &[FOREIGN_NET],
            0,
            FixedState::Unfixed,
        )
        .expect("probe cross trace");
        assert_eq!(cross.get(), 106, "capture crossTrace id");
        start_marking_changed_area(&mut board);

        let vector = Point::int(IntPoint::new(504000, 300000))
            .difference_by(&Point::int(IntPoint::new(500000, 300000)));
        assert!(
            crate::drill_item_mover::insert(&mut manager, &mut board, via, &vector, 10, 10, None),
            "capture move_result true"
        );
        assert_eq!(
            board.drill_center(via),
            Some(p(504000, 300000)),
            "capture after_move via center"
        );
        // The shoved trace re-formed as 109 (106 replaced; 107/108 the
        // transient shove ids), wrapped east around the moved via.
        let ids = live_ids(&board);
        assert_eq!(ids.len(), 106, "capture after_move inventory size");
        assert!(!ids.contains(&106), "the crossed trace replaced");
        assert!(ids.contains(&109), "capture re-formed trace id");
        assert_eq!(board.max_generated_id(), 109, "stream watermark");
        assert_corners(
            &board,
            ItemId::new(109),
            &[
                (504000, 297000),
                (504000, 299633),
                (504298, 299633),
                (504367, 299702),
                (504367, 300298),
                (504298, 300367),
                (504000, 300367),
                (504000, 303000),
            ],
        );
        // The fused join webs on L0: the drill join's box corner 499750
        // + the forced-pad piece extremes 504367/299633/300367. The
        // single-layer padstack never joins L1 (still EMPTY).
        assert_area(
            &board,
            0,
            [
                499750, 299633, 504367, 300367, 199500, 204665, 799500, 804665,
            ],
        );
        let empty = [
            CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT,
        ];
        assert_area(&board, 1, empty);
        let box_ = board
            .changed_area
            .as_ref()
            .expect("session")
            .surrounding_box();
        assert_eq!(
            (box_.ll.x, box_.ll.y, box_.ur.x, box_.ur.y),
            (499750, 299633, 504367, 300367),
            "capture after_move surroundingBox"
        );
    }

    /// Spec-review MINOR-3: the nested-start semantic — Java
    /// `RoutingBoardOperations.startMarkingChangedArea` (:27-29) creates
    /// the session only when none is active, so a second start KEEPS the
    /// live marker instead of resetting it.
    #[test]
    fn t10c_nested_start_keeps_the_live_session() {
        let mut board = parse_fixture();
        start_marking_changed_area(&mut board);
        join_changed_area(&mut board, &FloatPoint::new(500000.0, 300000.0), 0);
        let before = board.changed_area.as_ref().expect("session").get_area(0);
        assert_eq!(
            (before.left_x, before.bottom_y, before.right_x, before.top_y),
            (500000, 300000, 500000, 300000),
            "the join landed"
        );
        // The nested start: the joined point SURVIVES (a reset would
        // hand back the +-CRIT_INT EMPTY octagon).
        start_marking_changed_area(&mut board);
        let after = board.changed_area.as_ref().expect("session").get_area(0);
        assert_eq!(
            (after.left_x, after.bottom_y, after.right_x, after.top_y),
            (500000, 300000, 500000, 300000),
            "nested start keeps the live marker (Java :27-29)"
        );
    }

    // ---- quality-review-1 pins (M-1: the normalize-true branch) ----

    /// Quality review M-1: the normalize-true branch —
    /// `split_traces_at_keep_point` + the post-split re-pick (Java
    /// :809-833) — observed on a REAL keep-point split (capture
    /// `forced_insert_rows_m1_run{1,2}`, world `keep_point`;
    /// `cmp`-identical double run; probe gate `m1run`). Bare fixture +
    /// two net-94 seeds: a vertical trace crossing the corridor MIDDLE
    /// at (502000,300000) and a COLLINEAR continuation from the
    /// corridor end (504000,300000) to (508000,300000). Flow, per the
    /// capture step rows: `pickedSize=0`; `combined=true` — the :756
    /// combine merges the continuation, so the keep point becomes the
    /// merged trace's INTERIOR (this is what makes the keep-point split
    /// real instead of an end-corner no-op); `normalize result=true,
    /// idBefore=107, idAfter=111, delta=4` — split_clip found-splits
    /// the crossing (105 -> 108/109) and own-splits the merged trace
    /// (107 -> 110/111), so `result = pieces != 1` is TRUE; then
    /// `split_at_keep idBefore=111, idAfter=113, delta=2` — the
    /// keep-point split cuts piece 111 at (504000,300000) into
    /// 112 + 113, and the re-pick (`pickedAtEndCorner=2`,
    /// `new_trace_null=false`) runs over {112, 113} (its picked value
    /// feeds only the pull-tight seam, dead here at tidyWidth 0 — the
    /// M4 arm). The keep-point split is the ONLY step that replaces
    /// 111: a world without it cannot produce the observed id set.
    /// Single real-split candidate: with one interior trace at the keep
    /// point, the pick order is forced — the two-candidate design is
    /// foreclosed because normalize's own found-splits pre-split every
    /// same-net crossing ON the inserted polyline (and an end-abutting
    /// second candidate is combine-eaten, the M-2 masking mechanism).
    #[test]
    fn t10c_keep_point_split_splits_and_re_picks() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let crossing = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(502000, 297000), &p(502000, 303000)),
            0,
            100,
            &[NET],
            0,
            FixedState::Unfixed,
        )
        .expect("probe crossingTrace");
        assert_eq!(crossing.get(), 105, "capture crossingTrace id");
        let continuation = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(504000, 300000), &p(508000, 300000)),
            0,
            100,
            &[NET],
            0,
            FixedState::Unfixed,
        )
        .expect("probe continuationTrace");
        assert_eq!(continuation.get(), 106, "capture continuationTrace id");

        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(&p(500000, 300000), &p(504000, 300000)),
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            0,
            2,
            0,
            100,
            true,
            None,
        );
        assert_eq!(result, Some(p(504000, 300000)), "capture insert_result");
        // after inventory ids >= 105 = [108, 109, 110, 112, 113]: 106
        // consumed by the combine, 107 replaced by the normalize split
        // (108/109 = the crossing's pieces, 110/111 = the merged
        // trace's), 111 replaced by the KEEP-POINT SPLIT (112/113).
        let ids = live_ids(&board);
        assert_eq!(
            ids.iter()
                .filter(|id| **id >= 105)
                .copied()
                .collect::<Vec<_>>(),
            vec![108, 109, 110, 112, 113],
            "capture after-inventory: the keep-point split replaced 111 with 112+113"
        );
        assert_corners(
            &board,
            ItemId::new(108),
            &[(502000, 297000), (502000, 300000)],
        );
        assert_corners(
            &board,
            ItemId::new(109),
            &[(502000, 300000), (502000, 303000)],
        );
        assert_corners(
            &board,
            ItemId::new(110),
            &[(500000, 300000), (502000, 300000)],
        );
        assert_corners(
            &board,
            ItemId::new(112),
            &[(502000, 300000), (504000, 300000)],
        );
        assert_corners(
            &board,
            ItemId::new(113),
            &[(504000, 300000), (508000, 300000)],
        );
        assert_eq!(
            board.max_generated_id(),
            113,
            "stream watermark: the keep-point split's second piece"
        );
        // after octagons: the corridor's two corners on L0; nothing on
        // L1 (single-layer world).
        assert_area(
            &board,
            0,
            [
                500000, 300000, 504000, 300000, 200000, 204000, 800000, 804000,
            ],
        );
        let empty = [
            CRIT_INT, CRIT_INT, -CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT, CRIT_INT, -CRIT_INT,
        ];
        assert_area(&board, 1, empty);
    }

    /// T17b (bug 177): the picked-trace TOMBSTONE contract at the
    /// forced-insertion face. The picked same-net trace A (105) is
    /// picked ALIVE at `:489-517`, then DIES mid-shove-loop — during
    /// insert(shape 0) the foreign via V1 dodges north (the
    /// W2-captured nearest-border tie-break) and its pad lands on A's
    /// east leg; the forcedPad replacement removes A's id (the
    /// `drill_move`-captured mechanism) — and the sampling-retry
    /// recombine at `:682` must still answer the picked geometry.
    /// Java holds the live `PolylineTrace` reference: it reads the
    /// object's CURRENT `lines` field and stays readable after board
    /// removal (memory-alive — the read answers removal-time
    /// geometry, equal to pick-time here because removal never
    /// rewrote A). The port models it as the id + pick-time snapshot
    /// ([`picked_trace_lines`]). RED witness: pre-fix, the id re-read
    /// panicked `picked trace polyline` at the site-2 `expect` (this
    /// exact world, caught under `catch_unwind` during the probe —
    /// the panic IS the red).
    ///
    /// The pinned board discriminates the surviving mutants:
    /// * result = the SAMPLED corner (521000,299800) — the shorten
    ///   arm ran and the retry SUCCEEDED (a fail would answer
    ///   fromCorner (500000,300000); the `>100x` arm the same);
    /// * 116 = T's 45°-wrap substitute around the RETRIED east
    ///   segment — reachable only when the site-2 recombine used the
    ///   snapshot: the combine sets `shapeIndex = lines-3` to the
    ///   LONG east segment, whose insert shoves T (a skip-when-dead
    ///   mutant re-checks the SHORT south segment instead: no wrap);
    /// * 117 = the merged route + A's surviving substitute lineage —
    ///   the west end keeps A's shape (cut at 501133 by the dodge
    ///   pad), the south end the sampled corner;
    /// * V1 (106) dodged to (501500,300368), U (108) untouched.
    #[test]
    fn t17b_picked_trace_tombstone_recombines_after_midloop_death() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        // A: contains from_corner at its first corner, rises NE, then
        // runs east UNDER V1's dodge landing pad.
        let a = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_points(&[p(500000, 300000), p(500600, 300600), p(501600, 300600)]),
            0,
            100,
            &[NET],
            0,
            FixedState::Unfixed,
        )
        .expect("A");
        assert_eq!(a.get(), 105, "the picked trace id");
        let route =
            Polyline::from_points(&[p(500000, 300000), p(521000, 300000), p(521000, 290000)]);
        let padstack = add_via_padstack(&mut board, "t17b_via");
        // V1 on shape 0's band: dodges north (the W2-captured
        // nearest-border tie-break), its pad landing on A's east leg.
        let v1 = insert_via(
            &mut manager,
            &mut board,
            padstack,
            IntPoint::new(501500, 300000),
            &[THIRD_NET],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(v1.get(), 106, "the dodging via id");
        // T crosses shape 1's band below shape 0's floor; U crosses
        // T's substitute wrap path — the stack-depth pair that fails
        // check(shape 1) and gates the sampling retry.
        let t = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(520500, 299700), &p(521500, 299700)),
            0,
            100,
            &[FOREIGN_NET],
            0,
            FixedState::Unfixed,
        )
        .expect("T");
        assert_eq!(t.get(), 107, "the wrapped trace id");
        let u = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(520500, 295000), &p(521500, 295000)),
            0,
            100,
            &[FIFTH_NET],
            0,
            FixedState::Unfixed,
        )
        .expect("U");
        assert_eq!(u.get(), 108, "the untouched trace id");
        assert_eq!(live_ids(&board).len(), 108, "the seed inventory");

        let result = insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &route,
            100,
            0,
            Some(&[NET][..]),
            0,
            10,
            1,
            2,
            0,
            100,
            true,
            None,
        );
        assert_eq!(
            result,
            Some(p(521000, 299800)),
            "the SAMPLED corner: the shorten arm + a successful retry"
        );
        // The picked trace DIED (the dodge-pad replacement removed it)
        // and T died (replaced by its wrap substitute); V1 and U live.
        let ids = live_ids(&board);
        assert!(!ids.contains(&105), "the picked trace died mid-loop");
        assert!(!ids.contains(&107), "T replaced by the wrap substitute");
        assert_eq!(
            ids.iter()
                .filter(|id| **id >= 105)
                .copied()
                .collect::<Vec<_>>(),
            vec![106, 108, 116, 117],
            "the post-death inventory"
        );
        assert_eq!(
            board.drill_center(v1),
            Some(p(501500, 300368)),
            "V1 dodged north (the tie-break capture)"
        );
        assert_corners(&board, u, &[(520500, 295000), (521500, 295000)]);
        // The wrap substitute — the retried east segment's shove.
        assert_corners(
            &board,
            ItemId::new(116),
            &[
                (520500, 299700),
                (520794, 299700),
                (520911, 299583),
                (521089, 299583),
                (521206, 299700),
                (521500, 299700),
            ],
        );
        // The final net-94 trace: A's substitute lineage west, the
        // sampled route east.
        assert_corners(
            &board,
            ItemId::new(117),
            &[
                (501133, 300600),
                (500600, 300600),
                (500000, 300000),
                (521000, 300000),
                (521000, 299800),
            ],
        );
        assert_eq!(
            board.get(ItemId::new(117)).map(|e| e.nets.clone()),
            Some(vec![NET]),
            "117 is the surviving net-94 trace"
        );
    }
}
