//! Java `ViaOptimizer.java` (733 l) — the per-via relocation optimizer
//! the changed-area fixpoint drives (`TraceTightener.java:160-165`,
//! `optViaLocation(board, via, traceCosts, minTranslateDist, 10)`,
//! gated on `traceCosts != null` — with null costs the via arm is dead,
//! which is why no pre-T5 capture ever saw a via move).
//!
//! Contract (Java-exact):
//! * A via relocates to reduce the weighted layer-trace cost of its
//!   contact geometry; [`opt_via_location`] (`:33-158`) accepts vias
//!   with exactly 1 contact (plane/fanout face) or exactly 2 trace
//!   contacts (the cost arm), then relocates along/across the contact
//!   geometry through [`reposition_via_move`]'s corridor check +
//!   mover-approved bisection.
//! * **Recursion budget** — the production caller passes **10**
//!   (`TraceTightener.java:165`); the port pins that constant
//!   ([`VIA_RELOCATION_RECURSION_DEPTH`]). The post-move via pick
//!   recurses on the FIRST picked via (`:150-156` — `break` after one)
//!   with `maxRecursionDepth - 1`; the depth guards (`:42-45`,
//!   `:163-166`) carry Java's "probably endless loop" debug note.
//! * **Insertion** — `DrillItemMover.insert(via, delta, 9, 9, null)`
//!   (`:136`, `:282`) — budgets 9/9, the [`crate::drill_item_mover`]
//!   substrate; every pre-check is the mover `check` with budgets 0/0
//!   (`:244`, `:338`, `:428`).
//! * **The four `checkTraceSegment` sites use the POINTS overload**
//!   (`:246-253`, `:316-324`, `:401-409`, `:415-423`) — the banked
//!   SEAM note; the port routes all four through
//!   [`crate::routing_board_search::check_trace_segment_points`].
//!   No shape overload exists in this class.
//! * **Plane face (M6 boundary, loud)** — Java's plane-containment
//!   arm (`:264-280`: pick CONDUCTION at the new location, require
//!   the contact plane among the picks) needs copper pours, which no
//!   tier fixture carries until M6. On a pour-free board Java's pick
//!   finds nothing and Java answers `false` — so the port answers
//!   `false` at the containment point ([`opt_plane_or_fanout_via`]':
//!   `contact_plane_detected` gate) after running the REAL fanout
//!   geometry: observationally identical for the pure-plane via
//!   (never reaches it — `contactTrace == null` returns at `:188-190`)
//!   and the mixed plane+trace via (containment fails). A future
//!   plane-aware fixture flip fails loudly at this note instead of
//!   silently (the T4 any-angle stub pattern). The FANOUT face (a
//!   single trace contact) is REAL and fully ported.
//! * `isWithinTolerance` (`:719-732`) is the Manhattan distance
//!   `<= tolerance` with `tolerance = (int) (via.minWidth() / 2) + 1`
//!   (`:87`, `:194` — `minWidth` reads the minimum pad bounding-box
//!   side in DBU, i.e. the DIAMETER of a circular pad).
//! * **Point equality** — Java `newLocation.equals(viaCenter)`
//!   (`:132`) and `newViaLocation.equals(checkCorner)` (`:292`) are
//!   VALUE comparisons; the port's `PartialEq` matches.
//! * Java `PolylineTrace.firstCorner()/lastCorner()` read
//!   `polyline.corner(0)` / `corner(cornerCount - 1)`; the
//!   near-corner ladder (`:89-106`, `:197-211`) reads
//!   `corner(1)` / `corner(cornerCount - 2)` — see
//!   [`tolerance_ladder`].

use crate::board::Board;
use crate::id::ItemId;
use crate::items::ItemData;
use crate::rules_surf::AngleRestriction;
use crate::tree_manager::SearchTreeManager;
use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;
use epic_geometry::side::Side;

use super::{TraceCostFactor, TraceTightener};

/// Java `TraceTightener.java:165` — the production recursion budget
/// (`optViaLocation(…, 10)`).
pub(crate) const VIA_RELOCATION_RECURSION_DEPTH: i32 = 10;

/// Java `:87` / `:194`: `tolerance = (int) (via.minWidth() / 2) + 1`
/// (the `(int)` cast truncates the double toward zero).
fn via_tolerance(board: &mut Board, via_id: ItemId) -> i32 {
    let min_width = board
        .drill_min_width(via_id)
        .expect("live via min width (Java NPE)");
    ((min_width / 2.0) as i32) + 1
}

/// The pull-tight tails (`:140-149`, `:286-291`): Java picks with a
/// TRACES filter at the location and calls
/// `((PolylineTrace) it).pullTight(true, tracePullTightAccuracy, null)`
/// — the FRESH-algo face (own-net only-net list, null clip, the call's
/// accuracy, no budget), NOT the fixpoint's state. The port swaps the
/// state for the duration through
/// [`TraceTightener::with_fresh_algo_face`].
fn pull_tight_picked_traces(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    location: &Point,
    layer: i32,
    trace_pull_tight_accuracy: i32,
) {
    for current_item in manager.pick_items(board, location, layer) {
        // Java picks with a TRACES filter; the unfiltered pick plus
        // this kind check yields the same set.
        if !matches!(
            board.get(current_item).map(|entry| &entry.data),
            Some(ItemData::Trace { .. })
        ) {
            continue;
        }
        let own_nets = board
            .get(current_item)
            .map(|entry| entry.nets.clone())
            .unwrap_or_default();
        // Java discards the boolean result.
        let _ = state.with_fresh_algo_face(&own_nets, trace_pull_tight_accuracy, |state| {
            super::polyline_trace_pull_tight(state, manager, board, current_item)
        });
    }
}

/// Java `:89-106` / `:196-211`: the near-corner ladder. Tests the
/// trace's FIRST corner (`corner(0)`) against the via center, then the
/// LAST corner (`corner(cornerCount - 1)`); the from-corner is the
/// NEXT corner inward (`corner(1)` / `corner(cornerCount - 2)`).
/// Returns the from-corner plus the at-first-corner flag (the fanout
/// face's projection arm reads it). `None` = "via is not connected at
/// trace endpoints — skip optimization" (`:94`/`:104`/`:202`).
fn tolerance_ladder(
    board: &Board,
    trace_id: ItemId,
    via_center: &Point,
    tolerance: i32,
) -> Option<(Point, bool)> {
    let polyline = board.trace_polyline(trace_id)?;
    let corner_count = polyline.corner_count() as i32;
    let first_corner = polyline
        .corner(0)
        .expect("corner 0 (traces have >= 2 corners)");
    let last_corner = polyline
        .corner(corner_count - 1)
        .expect("corner cornerCount - 1");
    if is_within_tolerance(&first_corner, via_center, tolerance) {
        Some((polyline.corner(1).expect("corner 1"), true))
    } else if is_within_tolerance(&last_corner, via_center, tolerance) {
        Some((
            polyline
                .corner(corner_count - 2)
                .expect("corner cornerCount - 2"),
            false,
        ))
    } else {
        None
    }
}

/// Java `ViaOptimizer.optViaLocation` (`:33-158`): optimizes the
/// location of a via connected to at most 2 traces according to the
/// trace costs on the layers of the connected traces. Returns false
/// if the via was not changed.
pub(crate) fn opt_via_location(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    trace_costs: Option<&[TraceCostFactor]>,
    trace_pull_tight_accuracy: i32,
    max_recursion_depth: i32,
) -> bool {
    if crate::trace_ops::is_shove_fixed(board, via_id) {
        return false;
    }
    if max_recursion_depth <= 0 {
        // Java `:43`: FRLogger.debug("OptViaAlgo.opt_via_location:
        // probably endless loop").
        return false;
    }
    let contacts = crate::contacts::drill_normal_contacts(manager, board, via_id);
    let mut is_plane_or_fanout_via = contacts.len() == 1;
    let mut first_trace: Option<ItemId> = None;
    let mut second_trace: Option<ItemId> = None;
    if !is_plane_or_fanout_via {
        if contacts.len() != 2 {
            // Java `:51-53`.
            return false;
        }
        // Java iterates the contact `TreeSet` in DESCENDING id; the
        // first-iterated contact is `firstTrace` (`:54-74`).
        // [`crate::contacts::drill_normal_contacts`] returns descending
        // ids, so index 0 is Java's first.
        for contact in [contacts[0], contacts[1]] {
            let is_shove = crate::trace_ops::is_shove_fixed(board, contact);
            let is_trace = matches!(
                board.get(contact).map(|entry| &entry.data),
                Some(ItemData::Trace { .. })
            );
            if is_shove || !is_trace {
                if matches!(
                    board.get(contact).map(|entry| &entry.data),
                    Some(ItemData::ConductionArea { .. })
                ) {
                    is_plane_or_fanout_via = true;
                } else {
                    return false;
                }
            } else if first_trace.is_none() {
                first_trace = Some(contact);
            } else {
                second_trace = Some(contact);
            }
        }
    }
    if is_plane_or_fanout_via {
        return opt_plane_or_fanout_via(
            state,
            manager,
            board,
            via_id,
            trace_pull_tight_accuracy,
            max_recursion_depth,
        );
    }
    let Some(first_trace) = first_trace else {
        // Java-unreachable: the scan above filled both slots when
        // !is_plane_or_fanout_via (Java dereferences at `:80`).
        return false;
    };
    let Some(second_trace) = second_trace else {
        return false;
    };
    let via_center = board
        .drill_center(via_id)
        .expect("live via center (Java NPE at `:79`)");
    let first_layer = board
        .trace_layer(first_trace)
        .expect("live trace layer (Java NPE at `:80`)");
    let second_layer = board
        .trace_layer(second_trace)
        .expect("live trace layer (Java NPE at `:81`)");
    let tolerance = via_tolerance(board, via_id);
    let Some((first_trace_from_corner, _)) =
        tolerance_ladder(board, first_trace, &via_center, tolerance)
    else {
        // Java `:94`/`:104`: via not connected at trace endpoints.
        return false;
    };
    let Some((second_trace_from_corner, _)) =
        tolerance_ladder(board, second_trace, &via_center, tolerance)
    else {
        return false;
    };

    // Java `:108-116`: per-layer costs, or the shared uniform (1, 1)
    // (`secondLayerTraceCosts = firstLayerTraceCosts` — one object).
    let (first_layer_trace_costs, second_layer_trace_costs) = match trace_costs {
        Some(costs) => (
            costs[usize::try_from(first_layer).expect("layer index")],
            costs[usize::try_from(second_layer).expect("layer index")],
        ),
        None => {
            let uniform = TraceCostFactor {
                horizontal: 1.0,
                vertical: 1.0,
            };
            (uniform, uniform)
        }
    };

    let first_half_width = board
        .trace_half_width(first_trace)
        .expect("live trace half width");
    let first_cl_class = board
        .item_clearance_class(first_trace)
        .expect("live trace class");
    let second_half_width = board
        .trace_half_width(second_trace)
        .expect("live trace half width");
    let second_cl_class = board
        .item_clearance_class(second_trace)
        .expect("live trace class");

    let new_location = reposition_via_cost(
        manager,
        board,
        via_id,
        first_half_width,
        first_cl_class,
        first_layer,
        first_layer_trace_costs,
        &first_trace_from_corner,
        second_half_width,
        second_cl_class,
        second_layer,
        second_layer_trace_costs,
        &second_trace_from_corner,
    );
    // Java `:132-134`: `newLocation == null || newLocation.equals(
    // viaCenter)` → false.
    let Some(new_location) = new_location.filter(|location| *location != via_center) else {
        return false;
    };
    let delta = new_location.difference_by(&via_center);
    // Java `:136`: DrillItemMover.insert(via, delta, 9, 9, null, board).
    if !crate::drill_item_mover::insert(manager, board, via_id, &delta, 9, 9, None) {
        // Java `:137`: FRLogger.warn("OptViaAlgo.opt_via_location:
        // move via failed").
        return false;
    }
    // Java `:140-149`: pull tight the picked TRACES at the new location
    // on both contact layers.
    pull_tight_picked_traces(
        state,
        manager,
        board,
        &new_location,
        first_layer,
        trace_pull_tight_accuracy,
    );
    pull_tight_picked_traces(
        state,
        manager,
        board,
        &new_location,
        second_layer,
        trace_pull_tight_accuracy,
    );
    // Java `:150-156`: recurse on the FIRST picked via (descending pick
    // order; `break` after exactly one) with depth - 1. The just-moved
    // via sits at the new location, so the walk continues through it —
    // the collinear-world witness exercises this.
    for current_item in manager.pick_items(board, &new_location, first_layer) {
        if matches!(
            board.get(current_item).map(|entry| &entry.data),
            Some(ItemData::Via { .. })
        ) {
            opt_via_location(
                state,
                manager,
                board,
                current_item,
                trace_costs,
                trace_pull_tight_accuracy,
                max_recursion_depth - 1,
            );
            break;
        }
    }
    true
}

/// Java `ViaOptimizer.optPlaneOrFanoutVia` (`:161-296`): optimisations
/// for vias with only 1 connected trace (plane or fanout vias). The
/// PLANE face is the M6 stub (module docs); the FANOUT face is real.
fn opt_plane_or_fanout_via(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    trace_pull_tight_accuracy: i32,
    max_recursion_depth: i32,
) -> bool {
    if max_recursion_depth <= 0 {
        // Java `:164`: FRLogger.debug("OptViaAlgo.opt_plane_or_fanout_via:
        // probably endless loop").
        return false;
    }
    let contact_list = crate::contacts::drill_normal_contacts(manager, board, via_id);
    if contact_list.is_empty() {
        // Java `:168-170`.
        return false;
    }
    let mut contact_plane_detected = false;
    let mut contact_trace: Option<ItemId> = None;
    for current_contact in contact_list {
        match board.get(current_contact).map(|entry| &entry.data) {
            Some(ItemData::ConductionArea { .. }) => {
                if contact_plane_detected {
                    // Java `:175-177`: a second plane contact → false.
                    return false;
                }
                // Java `:176` records the plane and CONTINUES the scan
                // (a mixed plane+trace via still runs the fanout face).
                contact_plane_detected = true;
            }
            Some(ItemData::Trace { .. }) => {
                if crate::trace_ops::is_shove_fixed(board, current_contact)
                    || contact_trace.is_some()
                {
                    // Java `:180-182`.
                    return false;
                }
                contact_trace = Some(current_contact);
            }
            _ => {
                // Java `:184-186`.
                return false;
            }
        }
    }
    let Some(contact_trace) = contact_trace else {
        // Java `:188-190`: contactTrace == null (a pure plane via —
        // unreachable through the stub above, kept for shape parity).
        return false;
    };
    let via_center = board
        .drill_center(via_id)
        .expect("live via center (Java NPE at `:191`)");
    let tolerance = via_tolerance(board, via_id);
    let Some((check_corner, at_first_corner)) =
        tolerance_ladder(board, contact_trace, &via_center, tolerance)
    else {
        // Java `:202`: via not connected at trace endpoints.
        return false;
    };
    let rounded_check_corner = Point::Int(check_corner.to_float().round());
    let trace_half_width = board
        .trace_half_width(contact_trace)
        .expect("live trace half width");
    let trace_layer = board.trace_layer(contact_trace).expect("live trace layer");
    let trace_cl_class_no = board
        .item_clearance_class(contact_trace)
        .expect("live trace class");
    // Java `:216-217`: the 6-arg repositionVia toward the near corner.
    let mut new_via_location = reposition_via_move(
        manager,
        board,
        via_id,
        &rounded_check_corner,
        trace_half_width,
        trace_layer,
        trace_cl_class_no,
    );
    if new_via_location.is_none()
        && board
            .trace_polyline(contact_trace)
            .map(|polyline| polyline.corner_count())
            >= Some(3)
    {
        // Java `:218-260`: try to project the via to the previous line.
        let polyline = board
            .trace_polyline(contact_trace)
            .expect("live trace polyline");
        let corner_count = polyline.corner_count() as i32;
        let prev_corner = if at_first_corner {
            polyline.corner(2).expect("corner 2")
        } else {
            polyline
                .corner(corner_count - 3)
                .expect("corner cornerCount - 3")
        };
        let float_check_corner = check_corner.to_float();
        let float_via_center = via_center.to_float();
        let float_prev_corner = prev_corner.to_float();
        if float_check_corner.scalar_product(&float_via_center, &float_prev_corner) != 0.0 {
            let current_line = FloatLine::new(float_check_corner, float_prev_corner);
            let projection = Point::Int(
                current_line
                    .perpendicular_projection(&float_via_center)
                    .round(),
            );
            let diff_vector = projection.difference_by(&via_center);
            let mut projection_ok = true;
            let angle_restriction = board.rules().trace_angle_restriction;
            if projection == via_center
                || angle_restriction == AngleRestriction::NinetyDegree
                    && !diff_vector.is_orthogonal()
                || angle_restriction == AngleRestriction::FortyfiveDegree
                    && !diff_vector.is_multiple_of_45_degree()
            {
                projection_ok = false;
            }
            if projection_ok {
                // Java `:244`: DrillItemMover.check(via, diffVector, 0, 0, …).
                let mut ignore_items = Vec::new();
                if crate::drill_item_mover::check(
                    manager,
                    board,
                    via_id,
                    &diff_vector,
                    0,
                    0,
                    &mut ignore_items,
                    None,
                ) {
                    // Java `:246-253`: the POINTS overload (SEAM note).
                    let nets = board
                        .get(via_id)
                        .map(|entry| entry.nets.clone())
                        .unwrap_or_default();
                    let ok_length = crate::routing_board_search::check_trace_segment_points(
                        manager,
                        board,
                        &via_center,
                        &projection,
                        trace_layer,
                        &nets,
                        trace_half_width,
                        trace_cl_class_no,
                        false,
                    );
                    if ok_length >= f64::from(i32::MAX) {
                        new_via_location = Some(projection);
                    }
                }
            }
        }
    }
    // Java `:261-263`.
    let Some(new_via_location) = new_via_location else {
        return false;
    };
    // Java `:264-280` — the plane-containment arm (`if (contactPlane
    // != null)`: pick CONDUCTION at the new location on the plane's
    // layer, require the contact plane among the picks, else false).
    // M6 BOUNDARY (loud, the T4 any-angle stub pattern): the pick
    // needs a copper pour — no tier fixture carries a ConductionArea
    // until M6. On a pour-free board Java's pick finds nothing, so
    // `contactOk` stays false and Java answers false — this stub is
    // observationally IDENTICAL there, for both the pure-plane via
    // (which never gets here: `contactTrace == null` returned at
    // `:188-190`) and the mixed plane+trace via (full fanout geometry
    // above, containment fails). A future plane-aware fixture flip
    // fails HERE, loudly, instead of silently.
    if contact_plane_detected {
        return false;
    }
    let diff_vector = new_via_location.difference_by(&via_center);
    // Java `:282`: DrillItemMover.insert(via, diffVector, 9, 9, null).
    if !crate::drill_item_mover::insert(manager, board, via_id, &diff_vector, 9, 9, None) {
        // Java `:283`: FRLogger.warn("OptViaAlgo.opt_plane_or_fanout_via:
        // move via failed").
        return false;
    }
    // Java `:286-291`: pull tight the picked traces at the new location.
    pull_tight_picked_traces(
        state,
        manager,
        board,
        &new_via_location,
        trace_layer,
        trace_pull_tight_accuracy,
    );
    if new_via_location == check_corner {
        // Java `:292-294`: the fanout walk — recurse when the via
        // landed exactly on the check corner (the result is discarded).
        opt_plane_or_fanout_via(
            state,
            manager,
            board,
            via_id,
            trace_pull_tight_accuracy,
            max_recursion_depth - 1,
        );
    }
    true
}

/// Java `ViaOptimizer.repositionVia` 6-arg (`:302-365`): tries to move
/// the via into the direction of `to_location` as far as possible.
/// Returns the new location, or None if no move was possible.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn reposition_via_move(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    to_location: &Point,
    trace_half_width: i32,
    trace_layer: i32,
    trace_cl_class: i32,
) -> Option<Point> {
    let from_location = board
        .drill_center(via_id)
        .expect("live via center (Java NPE at `:310`)");
    if from_location == *to_location {
        // Java `:312-314`.
        return None;
    }
    let nets = board
        .get(via_id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default();
    // Java `:316-324`: the POINTS checkTraceSegment overload (SEAM note).
    let mut ok_length = crate::routing_board_search::check_trace_segment_points(
        manager,
        board,
        &from_location,
        to_location,
        trace_layer,
        &nets,
        trace_half_width,
        trace_cl_class,
        false,
    );
    if ok_length <= 0.0 {
        // Java `:325-327`.
        return None;
    }
    let float_from_location = from_location.to_float();
    let float_to_location = to_location.to_float();
    let new_float_to_location = if ok_length >= f64::from(i32::MAX) {
        float_to_location
    } else {
        float_from_location.change_length(&float_to_location, ok_length)
    };
    let new_to_location = Point::Int(new_float_to_location.round());
    let delta = new_to_location.difference_by(&from_location);
    // Java `:338`: DrillItemMover.check(via, delta, 0, 0, …).
    let mut ignore_items = Vec::new();
    let check_ok = crate::drill_item_mover::check(
        manager,
        board,
        via_id,
        &delta,
        0,
        0,
        &mut ignore_items,
        None,
    );
    if check_ok {
        // Java `:340-342`.
        return Some(new_to_location);
    }

    // Java `:344-364`: the bisection — longest mover-approved prefix of
    // the corridor, halving from okLength/2 down to 0.3 * halfWidth + 1.
    let min_length = 0.3 * f64::from(trace_half_width) + 1.0;
    ok_length = ok_length.min(float_from_location.distance(&float_to_location));
    let mut current_length = ok_length / 2.0;
    ok_length = 0.0;
    let mut result = None;
    while current_length >= min_length {
        let check_point = Point::Int(
            float_from_location
                .change_length(&float_to_location, ok_length + current_length)
                .round(),
        );
        let delta = check_point.difference_by(&from_location);
        let mut ignore_items = Vec::new();
        if crate::drill_item_mover::check(
            manager,
            board,
            via_id,
            &delta,
            0,
            0,
            &mut ignore_items,
            None,
        ) {
            ok_length += current_length;
            result = Some(check_point);
        }
        current_length /= 2.0;
    }
    result
}

/// Java `ViaOptimizer.repositionVia` 10-arg boolean (`:367-429`): the
/// two-leg corridor check (via → toLocation on the FIRST trace's
/// parameters, toLocation → connectLocation on the SECOND's) plus the
/// mover check. No board mutation.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn reposition_via_two_leg_check(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    to_location: &Point,
    trace_half_width1: i32,
    trace_layer1: i32,
    trace_cl_class1: i32,
    connect_location: &Point,
    trace_half_width2: i32,
    trace_layer2: i32,
    trace_cl_class2: i32,
) -> bool {
    let from_location = board
        .drill_center(via_id)
        .expect("live via center (Java NPE at `:379`)");
    if from_location == *to_location {
        // Java `:382`: FRLogger.trace("OptViaAlgo.reposition_via:
        // fromLocation equal toLocation").
        return false;
    }
    let delta = to_location.difference_by(&from_location);
    if board.rules().trace_angle_restriction == AngleRestriction::None
        && delta.length_approx() <= 1.5
    {
        // Java `:388-397`: TraceTightenerAnyAngle.reduce_corners may
        // not be able to remove the new generated overlap (numerical
        // stability) — that would result in an endless loop.
        return false;
    }
    let nets = board
        .get(via_id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default();
    // Java `:401-409`: the POINTS overload (SEAM note).
    let ok_length = crate::routing_board_search::check_trace_segment_points(
        manager,
        board,
        &from_location,
        to_location,
        trace_layer1,
        &nets,
        trace_half_width1,
        trace_cl_class1,
        false,
    );
    if ok_length < f64::from(i32::MAX) {
        // Java `:411-413`.
        return false;
    }
    // Java `:415-423`: the POINTS overload (SEAM note).
    let ok_length = crate::routing_board_search::check_trace_segment_points(
        manager,
        board,
        to_location,
        connect_location,
        trace_layer2,
        &nets,
        trace_half_width2,
        trace_cl_class2,
        false,
    );
    if ok_length < f64::from(i32::MAX) {
        // Java `:425-427`.
        return false;
    }
    // Java `:428`: DrillItemMover.check(via, delta, 0, 0, …).
    let mut ignore_items = Vec::new();
    crate::drill_item_mover::check(
        manager,
        board,
        via_id,
        &delta,
        0,
        0,
        &mut ignore_items,
        None,
    )
}

/// Java `ViaOptimizer.repositionVia` 11-arg (`:435-713`): tries to
/// reposition the via to a better location according to the trace
/// costs. Returns None if no better location was found. Arm order is
/// Java-exact: COLLINEAR (unconditional return) → wd(first corner) →
/// wd(second corner) → acute angle → axis-parallel decomposition
/// (first delta's two axis candidates, then second delta's).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
fn reposition_via_cost(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_id: ItemId,
    first_trace_half_width: i32,
    first_trace_cl_class: i32,
    first_trace_layer: i32,
    first_trace_costs: TraceCostFactor,
    first_trace_from_corner: &Point,
    second_trace_half_width: i32,
    second_trace_cl_class: i32,
    second_trace_layer: i32,
    second_trace_costs: TraceCostFactor,
    second_trace_from_corner: &Point,
) -> Option<Point> {
    let via_location = board
        .drill_center(via_id)
        .expect("live via center (Java NPE at `:448`)");
    let first_delta = first_trace_from_corner.difference_by(&via_location);
    let second_delta = second_trace_from_corner.difference_by(&via_location);
    let scalar_product = first_delta.scalar_product(&second_delta);

    let float_via_location = via_location.to_float();
    let float_first_trace_from_corner = first_trace_from_corner.to_float();
    let float_second_trace_from_corner = second_trace_from_corner.to_float();
    let first_trace_from_corner_distance =
        float_via_location.distance(&float_first_trace_from_corner);
    let second_trace_from_corner_distance =
        float_via_location.distance(&float_second_trace_from_corner);
    let rounded_first_trace_from_corner = Point::Int(float_first_trace_from_corner.round());
    let rounded_second_trace_from_corner = Point::Int(float_second_trace_from_corner.round());

    // Java `:462-482`: handle the case of overlapping lines first —
    // the COLLINEAR arm returns UNCONDITIONALLY (both sub-branches
    // return the 6-arg result even when it is null).
    if via_location.side_of(first_trace_from_corner, second_trace_from_corner) == Side::Collinear
        && scalar_product > 0.0
    {
        if second_trace_from_corner_distance < first_trace_from_corner_distance {
            return reposition_via_move(
                manager,
                board,
                via_id,
                &rounded_second_trace_from_corner,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
            );
        }
        return reposition_via_move(
            manager,
            board,
            via_id,
            &rounded_first_trace_from_corner,
            second_trace_half_width,
            second_trace_layer,
            second_trace_cl_class,
        );
    }

    // Java `:485-505`: the FIRST weighted-distance arm.
    let mut current_weighted_distance1 = float_via_location.weighted_distance(
        &float_first_trace_from_corner,
        first_trace_costs.horizontal,
        first_trace_costs.vertical,
    );
    let mut current_weighted_distance2 = float_via_location.weighted_distance(
        &float_first_trace_from_corner,
        second_trace_costs.horizontal,
        second_trace_costs.vertical,
    );

    if current_weighted_distance1 > current_weighted_distance2 {
        // Java `:493`: try to move the via in direction of
        // firstTraceFromCorner — with the SECOND trace's parameters.
        if let Some(found) = reposition_via_move(
            manager,
            board,
            via_id,
            &rounded_first_trace_from_corner,
            second_trace_half_width,
            second_trace_layer,
            second_trace_cl_class,
        ) {
            return Some(found);
        }
    }

    // Java `:507-527`: the SECOND weighted-distance arm (mirrored).
    current_weighted_distance1 = float_via_location.weighted_distance(
        &float_second_trace_from_corner,
        second_trace_costs.horizontal,
        second_trace_costs.vertical,
    );
    current_weighted_distance2 = float_via_location.weighted_distance(
        &float_second_trace_from_corner,
        first_trace_costs.horizontal,
        first_trace_costs.vertical,
    );

    if current_weighted_distance1 > current_weighted_distance2 {
        // Java `:515`: try to move the via in direction of
        // secondTraceFromCorner — with the FIRST trace's parameters.
        if let Some(found) = reposition_via_move(
            manager,
            board,
            via_id,
            &rounded_second_trace_from_corner,
            first_trace_half_width,
            first_trace_layer,
            first_trace_cl_class,
        ) {
            return Some(found);
        }
    }

    if scalar_product > 0.0
        && board.rules().trace_angle_restriction != AngleRestriction::NinetyDegree
    {
        // Java `:528-579`: the acute-angle arm — walk the nearer
        // corner to the farther corner's distance, then try both
        // directions in cost order.
        let to_point1;
        let to_point2;
        let float_to_point1;
        let float_to_point2;
        if first_trace_from_corner_distance < second_trace_from_corner_distance {
            // Java holds these as immutable references; the clones are
            // the value-semantics equivalent (the decomposition arms
            // below still need the originals).
            to_point1 = rounded_first_trace_from_corner.clone();
            float_to_point1 = float_first_trace_from_corner;
            float_to_point2 = float_via_location.change_length(
                &float_second_trace_from_corner,
                first_trace_from_corner_distance,
            );
            to_point2 = Point::Int(float_to_point2.round());
        } else {
            float_to_point1 = float_via_location.change_length(
                &float_first_trace_from_corner,
                second_trace_from_corner_distance,
            );
            to_point1 = Point::Int(float_to_point1.round());
            to_point2 = rounded_second_trace_from_corner.clone();
            float_to_point2 = float_second_trace_from_corner;
        }
        current_weighted_distance1 = float_to_point1.weighted_distance(
            &float_to_point2,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );
        current_weighted_distance2 = float_to_point1.weighted_distance(
            &float_to_point2,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );

        let result = if current_weighted_distance1 > current_weighted_distance2 {
            // Java `:556`: try moving the via first into the direction
            // of toPoint1 (SECOND parameters), then toPoint2 (FIRST).
            reposition_via_move(
                manager,
                board,
                via_id,
                &to_point1,
                second_trace_half_width,
                second_trace_layer,
                second_trace_cl_class,
            )
            .or_else(|| {
                reposition_via_move(
                    manager,
                    board,
                    via_id,
                    &to_point2,
                    first_trace_half_width,
                    first_trace_layer,
                    first_trace_cl_class,
                )
            })
        } else {
            // Java `:566`: toPoint2 (FIRST parameters) first, then
            // toPoint1 (SECOND).
            reposition_via_move(
                manager,
                board,
                via_id,
                &to_point2,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
            )
            .or_else(|| {
                reposition_via_move(
                    manager,
                    board,
                    via_id,
                    &to_point1,
                    second_trace_half_width,
                    second_trace_layer,
                    second_trace_cl_class,
                )
            })
        };
        if let Some(found) = result {
            return Some(found);
        }
    }

    // Java `:581-643`: try decomposition in axis-parallel parts — the
    // FIRST delta's two axis candidates.
    if !first_delta.is_orthogonal() {
        let float_check_location = FloatPoint {
            x: float_via_location.x,
            y: float_first_trace_from_corner.y,
        };

        current_weighted_distance1 = float_via_location.weighted_distance(
            &float_first_trace_from_corner,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );
        current_weighted_distance2 = float_via_location.weighted_distance(
            &float_check_location,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );
        let current_weighted_distance3 = float_check_location.weighted_distance(
            &float_first_trace_from_corner,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );

        if current_weighted_distance1 > current_weighted_distance2 + current_weighted_distance3 {
            let check_location = Point::Int(float_check_location.round());
            let check_ok = reposition_via_two_leg_check(
                manager,
                board,
                via_id,
                &check_location,
                second_trace_half_width,
                second_trace_layer,
                second_trace_cl_class,
                &rounded_first_trace_from_corner,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
            );
            if check_ok {
                return Some(check_location);
            }
        }

        let float_check_location = FloatPoint {
            x: float_first_trace_from_corner.x,
            y: float_via_location.y,
        };

        // Java `:618-625`: `currentWeightedDistance1` is REUSED from
        // the first candidate (via → firstCorner with FIRST costs).
        current_weighted_distance2 = float_via_location.weighted_distance(
            &float_check_location,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );
        let current_weighted_distance3 = float_check_location.weighted_distance(
            &float_first_trace_from_corner,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );

        if current_weighted_distance1 > current_weighted_distance2 + current_weighted_distance3 {
            let check_location = Point::Int(float_check_location.round());
            let check_ok = reposition_via_two_leg_check(
                manager,
                board,
                via_id,
                &check_location,
                second_trace_half_width,
                second_trace_layer,
                second_trace_cl_class,
                &rounded_first_trace_from_corner,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
            );
            if check_ok {
                return Some(check_location);
            }
        }
    }

    // Java `:645-711`: the SECOND delta's two axis candidates.
    if !second_delta.is_orthogonal() {
        let float_check_location = FloatPoint {
            x: float_via_location.x,
            y: float_second_trace_from_corner.y,
        };

        current_weighted_distance1 = float_via_location.weighted_distance(
            &float_second_trace_from_corner,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );
        current_weighted_distance2 = float_via_location.weighted_distance(
            &float_check_location,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );
        let current_weighted_distance3 = float_check_location.weighted_distance(
            &float_second_trace_from_corner,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );

        if current_weighted_distance1 > current_weighted_distance2 + current_weighted_distance3 {
            let check_location = Point::Int(float_check_location.round());
            let check_ok = reposition_via_two_leg_check(
                manager,
                board,
                via_id,
                &check_location,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
                &rounded_second_trace_from_corner,
                second_trace_half_width,
                second_trace_layer,
                second_trace_cl_class,
            );
            if check_ok {
                return Some(check_location);
            }
        }

        let float_check_location = FloatPoint {
            x: float_second_trace_from_corner.x,
            y: float_via_location.y,
        };

        // Java `:684-691`: `currentWeightedDistance1` is REUSED from
        // the first candidate (via → secondCorner with SECOND costs).
        current_weighted_distance2 = float_via_location.weighted_distance(
            &float_check_location,
            first_trace_costs.horizontal,
            first_trace_costs.vertical,
        );
        let current_weighted_distance3 = float_check_location.weighted_distance(
            &float_second_trace_from_corner,
            second_trace_costs.horizontal,
            second_trace_costs.vertical,
        );

        if current_weighted_distance1 > current_weighted_distance2 + current_weighted_distance3 {
            let check_location = Point::Int(float_check_location.round());
            let check_ok = reposition_via_two_leg_check(
                manager,
                board,
                via_id,
                &check_location,
                first_trace_half_width,
                first_trace_layer,
                first_trace_cl_class,
                &rounded_second_trace_from_corner,
                second_trace_half_width,
                second_trace_layer,
                second_trace_cl_class,
            );
            if check_ok {
                return Some(check_location);
            }
        }
    }
    // Java `:712`.
    None
}

/// Java `ViaOptimizer.isWithinTolerance` (`:719-732`): the Manhattan
/// distance `|x1-x2| + |y1-y2| <= tolerance`, matching the
/// connectivity-detection logic in `DrillItem.getNormalContacts()`.
fn is_within_tolerance(p1: &Point, p2: &Point, tolerance: i32) -> bool {
    let fp1 = p1.to_float();
    let fp2 = p2.to_float();
    let dx = (fp1.x - fp2.x).abs();
    let dy = (fp1.y - fp2.y).abs();
    dx + dy <= f64::from(tolerance)
}
