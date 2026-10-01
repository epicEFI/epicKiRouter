//! Port of Java
//! `app.freerouting.autoroute.path.FoundConnectionLocatorAnyAngle` —
//! the any-angle corner synthesis dispatch of the found-connection
//! locator (T9): a visibility-range sweep over the remaining doors of
//! the backtrack list, constructing a maximum-length straight trace
//! line, followed by a clearance-correction pass over the last doors.
//!
//! Reference-semantics mapping (module doc in `locator.rs`):
//! * Java `resultCorner` is a nullable local the base class filters
//!   with `resultCorner != null && resultCorner != currentFromPoint`
//!   (`:329`). The port models it as [`ResultCorner`]: `None` is Java
//!   null (an intersection miss — Java NPEs at the clearance-check
//!   line construction, so the port panics there), `FromPoint` is the
//!   turn-helper fallback arms `return fromCorner` (the ADDED corner
//!   would be reference-equal to `currentFromPoint` and is dropped by
//!   the base filter — so the port never emits it), `Point` is a fresh
//!   point (always emitted).
//! * The two "door completely passed" arms (`:98-103`, `:178-186`)
//!   increment `currentToDoorIndex` and return the `currentFromPoint`
//!   reference ITSELF as a NON-EMPTY singleton — the base filter drops
//!   that alias, but the NON-empty result keeps the trace loop running
//!   from the advanced index, so the SAME trace covers the remaining
//!   doors. The port models the singleton as
//!   [`NextCorner::FromPointAlias`] (`locator.rs`); returning an empty
//!   list here would be Java's trace-END signal and would truncate the
//!   trace at the passed door (the bug the fix round closed).
//! * Java's `FloatPoint.rightTangentialPoint`/`leftTangentialPoint`
//!   return null when `toPoint` is null (null check at the method
//!   head) or the point lies inside the circle; the port chains
//!   `Option::and_then` and applies Java's
//!   `doorCorner != null && tangent == null → tangent = doorCorner`
//!   fallbacks verbatim.

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;
use epic_geometry::side::Side;

use crate::expansion::NeighbourEngine;
use crate::maze::locator_access::LocatorAccess;

use super::locator::{BacktrackElement, LocatorState, NextCorner, other_room};

/// Java `cTolerance` (`:26`).
const C_TOLERANCE: f64 = 1.0;

/// The Java `resultCorner` local (module doc).
#[derive(Clone, Copy, Debug)]
enum ResultCorner {
    /// Java null — an intersection miss.
    None,
    /// The turn-helper fallback `return fromCorner` — the added corner
    /// is the from-point ITSELF and the base reference filter drops it.
    FromPoint,
    /// A fresh point object.
    Point(FloatPoint),
}

/// Java `calcDoorLeftCorner` (`:43-49`) — the left-most corner of the
/// door shape seen from the centre of gravity of the common room.
/// Java NPEs when `otherRoom` answers null (an incomplete room); the
/// port panics there (expect-panic discipline).
fn calc_door_left_corner<A: LocatorAccess>(access: &A, to_info: &BacktrackElement) -> FloatPoint {
    let room_key = other_room(access, &to_info.door, to_info.next_room)
        .expect("calcDoorLeftCorner: the other room is a complete room (Java NPE otherwise)");
    let pole = access.engine().room_shape(room_key).centre_of_gravity();
    let door_shape = access.expandable_object_shape(&to_info.door);
    let left_most_corner_no = door_shape.index_of_left_most_corner(&pole);
    door_shape
        .corner_approx(left_most_corner_no)
        .expect("a non-empty door shape has the corner")
}

/// Java `calcDoorRightCorner` (`:55-61`).
fn calc_door_right_corner<A: LocatorAccess>(access: &A, to_info: &BacktrackElement) -> FloatPoint {
    let room_key = other_room(access, &to_info.door, to_info.next_room)
        .expect("calcDoorRightCorner: the other room is a complete room (Java NPE otherwise)");
    let pole = access.engine().room_shape(room_key).centre_of_gravity();
    let door_shape = access.expandable_object_shape(&to_info.door);
    let right_most_corner_no = door_shape.index_of_right_most_corner(&pole);
    door_shape
        .corner_approx(right_most_corner_no)
        .expect("a non-empty door shape has the corner")
}

/// Java `rightTurnNextCorner` (`:365-383`): first line = LEFT side
/// tangent from `from_corner` to the circle around `to_corner`; second
/// line = RIGHT side tangent from `to_corner` to the circle around
/// `next_corner` (radius `2 * dist + cTolerance`), translated by
/// `+dist`; the result is the intersection (Java null →
/// [`ResultCorner::None`]).
fn right_turn_next_corner(
    from_corner: &FloatPoint,
    dist: f64,
    to_corner: &FloatPoint,
    next_corner: &FloatPoint,
) -> ResultCorner {
    let Some(first_tangential_point) = from_corner.left_tangential_point(to_corner, dist) else {
        return ResultCorner::FromPoint;
    };
    let first_line = FloatLine::new(*from_corner, first_tangential_point);
    let Some(second_tangential_point) =
        to_corner.right_tangential_point(next_corner, 2.0 * dist + C_TOLERANCE)
    else {
        return ResultCorner::FromPoint;
    };
    let second_line = FloatLine::new(*to_corner, second_tangential_point).translate(dist);
    match first_line.intersection(&second_line) {
        Some(point) => ResultCorner::Point(point),
        None => ResultCorner::None,
    }
}

/// Java `leftTurnNextCorner` (`:391-408`) — the mirrored variant with
/// a `−dist` translation.
fn left_turn_next_corner(
    from_corner: &FloatPoint,
    dist: f64,
    to_corner: &FloatPoint,
    next_corner: &FloatPoint,
) -> ResultCorner {
    let Some(first_tangential_point) = from_corner.right_tangential_point(to_corner, dist) else {
        return ResultCorner::FromPoint;
    };
    let first_line = FloatLine::new(*from_corner, first_tangential_point);
    let Some(second_tangential_point) =
        to_corner.left_tangential_point(next_corner, 2.0 * dist + C_TOLERANCE)
    else {
        return ResultCorner::FromPoint;
    };
    let second_line = FloatLine::new(*to_corner, second_tangential_point).translate(-dist);
    match first_line.intersection(&second_line) {
        Some(point) => ResultCorner::Point(point),
        None => ResultCorner::None,
    }
}

/// Java `rightLeftTangentialPoint` (`:414-431`): the RIGHT tangential
/// line from `from_point` and the LEFT tangential line from `to_point`
/// to the circle around `center` (a Java null `center` nulls both
/// tangential points → `None`); the intersection of the two lines.
fn right_left_tangential_point(
    from_point: &FloatPoint,
    to_point: &FloatPoint,
    center: Option<&FloatPoint>,
    dist: f64,
) -> Option<FloatPoint> {
    let center = center?;
    let first_tangential_point = from_point.right_tangential_point(center, dist)?;
    let first_line = FloatLine::new(*from_point, first_tangential_point);
    let second_tangential_point = to_point.left_tangential_point(center, dist)?;
    let second_line = FloatLine::new(*to_point, second_tangential_point);
    first_line.intersection(&second_line)
}

/// Java `leftRightTangentialPoint` (`:437-454`) — the mirrored
/// variant.
fn left_right_tangential_point(
    from_point: &FloatPoint,
    to_point: &FloatPoint,
    center: Option<&FloatPoint>,
    dist: f64,
) -> Option<FloatPoint> {
    let center = center?;
    let first_tangential_point = from_point.left_tangential_point(center, dist)?;
    let first_line = FloatLine::new(*from_point, first_tangential_point);
    let second_tangential_point = to_point.right_tangential_point(center, dist)?;
    let second_line = FloatLine::new(*to_point, second_tangential_point);
    first_line.intersection(&second_line)
}

/// Java `calculateNextTraceCorners` (`:68-356`) — an empty result ends
/// the current trace. The any-angle dispatch NEVER mutates
/// `currentFromPoint` (all updates happen in the base loop).
pub(crate) fn calculate_next_trace_corners<A: LocatorAccess>(
    st: &mut LocatorState,
    access: &A,
    ctrl: &crate::control::AutorouteControl,
) -> Vec<NextCorner> {
    let mut result: Vec<NextCorner> = Vec::new();
    let from = st
        .current_from_point
        .expect("the trace loop runs with the from point set");
    let previous = st
        .previous_from_point
        .expect("the base ctor set the previous from point");

    if st.current_to_door_index >= st.current_target_door_index {
        if st.current_to_door_index == st.current_target_door_index {
            // the final target arm uses the EXACT nearest point (the
            // 45-degree dispatch uses `nearestPointApprox`).
            let nearest_point = st
                .current_target_shape
                .as_ref()
                .expect("the ctor set the target shape")
                .nearest_point(&Point::int(from.round()))
                .expect("the target shape is non-empty (Java NPE otherwise)")
                .to_float();
            st.current_to_door_index += 1;
            result.push(NextCorner::Fresh(nearest_point));
        }
        return result;
    }

    let trace_halfwidth_exact =
        f64::from(ctrl.compensated_trace_half_width[st.current_trace_layer as usize]);
    let trace_halfwidth_max =
        trace_halfwidth_exact + f64::from(super::locator::TRACE_WIDTH_TOLERANCE);
    let trace_halfwidth_middle = trace_halfwidth_exact + C_TOLERANCE;

    let current_to_info = st.backtrack_array[st.current_to_door_index as usize].clone();
    let mut door_left_corner: Option<FloatPoint> =
        Some(calc_door_left_corner(access, &current_to_info));
    let mut door_right_corner: Option<FloatPoint> =
        Some(calc_door_right_corner(access, &current_to_info));
    let door_already_crossed = from.side_of(
        &door_left_corner.expect("calc gives a point"),
        &door_right_corner.expect("calc gives a point"),
    ) != Side::Negative;
    if door_already_crossed
        && from.scalar_product(&previous, &door_left_corner.expect("calc gives a point")) >= 0.0
    {
        // Also the left corner of the door is passed. That may not
        // be the case if the door line is crossed almost parallel.
        door_left_corner = None;
    }
    if door_already_crossed
        && from.scalar_product(&previous, &door_right_corner.expect("calc gives a point")) >= 0.0
    {
        // Also the right corner of the door is passed.
        door_right_corner = None;
    }
    if door_already_crossed && door_left_corner.is_none() && door_right_corner.is_none() {
        // The door is completely passed (`:98-103`): Java advances the
        // index and returns the from-point reference ITSELF as a
        // NON-EMPTY singleton. The base reference filter
        // (`FoundConnectionLocator.java:432`) drops that alias, but the
        // non-empty result keeps the trace loop RUNNING — the next
        // dispatch call continues from the advanced index within the
        // SAME trace. An empty return here would be Java's trace-end
        // signal (`:428-429`) and would truncate the trace at the
        // passed door.
        st.current_to_door_index += 1;
        result.push(NextCorner::FromPointAlias);
        return result;
    }

    // Calculate the visibility range for a trace line from
    // currentFromPoint through the interval from
    // left_most_visible_point to right_most_visible_point, by
    // advancing the door index as far as possible, so that still
    // something is visible (`:106-117`).
    let mut end_of_trace = false;
    let mut left_tangent_point: Option<FloatPoint>;
    let mut right_tangent_point: Option<FloatPoint>;
    let mut new_door_ind = st.current_to_door_index;
    let mut left_ind = new_door_ind;
    let mut right_ind = new_door_ind;
    let mut current_door_ind = st.current_to_door_index + 1;
    let mut result_corner = ResultCorner::None;

    // construct a maximum length straight line through the doors
    loop {
        left_tangent_point = door_left_corner
            .and_then(|corner| from.right_tangential_point(&corner, trace_halfwidth_max));
        if let Some(corner) = door_left_corner
            && left_tangent_point.is_none()
        {
            left_tangent_point = Some(corner);
        }
        right_tangent_point = door_right_corner
            .and_then(|corner| from.left_tangential_point(&corner, trace_halfwidth_max));
        if let Some(corner) = door_right_corner
            && right_tangent_point.is_none()
        {
            right_tangent_point = Some(corner);
        }
        if let (Some(lt), Some(rt)) = (&left_tangent_point, &right_tangent_point)
            && rt.side_of(&from, lt) != Side::Negative
        {
            // The gap between the most visible points is too
            // small for a trace with the current half width
            // (`:140-157`). Both door corners are non-null here (a
            // null corner nulls its tangent, failing the guard).
            let left_corner = door_left_corner.expect("non-null above");
            let right_corner = door_right_corner.expect("non-null above");
            let left_corner_distance = left_corner.distance(&from);
            let right_corner_distance = right_corner.distance(&from);
            if left_corner_distance <= right_corner_distance {
                new_door_ind = left_ind;
                result_corner =
                    left_turn_next_corner(&from, trace_halfwidth_max, &left_corner, &right_corner);
            } else {
                new_door_ind = right_ind;
                result_corner =
                    right_turn_next_corner(&from, trace_halfwidth_max, &right_corner, &left_corner);
            }
            break;
        }
        if current_door_ind >= st.current_target_door_index {
            end_of_trace = true;
            break;
        }
        let next_to_info: BacktrackElement = st.backtrack_array[current_door_ind as usize].clone();
        let next_left_corner_raw = calc_door_left_corner(access, &next_to_info);
        let next_right_corner_raw = calc_door_right_corner(access, &next_to_info);
        let mut next_left_corner: Option<FloatPoint> = Some(next_left_corner_raw);
        let mut next_right_corner: Option<FloatPoint> = Some(next_right_corner_raw);
        let next_door_crossed =
            from.side_of(&next_left_corner_raw, &next_right_corner_raw) != Side::Negative;
        if next_door_crossed
            && door_left_corner.is_none()
            && from.scalar_product(&previous, &next_left_corner_raw) >= 0.0
        {
            next_left_corner = None;
        }
        if next_door_crossed
            && door_right_corner.is_none()
            && from.scalar_product(&previous, &next_right_corner_raw) >= 0.0
        {
            next_right_corner = None;
        }
        if next_door_crossed && next_left_corner.is_none() && next_right_corner.is_none() {
            // The door is completely passed. Should not happen because
            // the previous door was not passed completely (`:178-186`):
            // Java advances the index and returns the from-point
            // reference ITSELF (non-empty singleton) — the base filter
            // drops the alias and the trace loop continues, exactly
            // like the `:98-103` arm above.
            st.current_to_door_index += 1;
            result.push(NextCorner::FromPointAlias);
            return result;
        }
        if let (Some(dl), Some(dr)) = (&door_left_corner, &door_right_corner) {
            // otherwise the following sideOf conditions may not be
            // correct even if all parameter points are defined
            // (`:188-208`). A next corner is only null-ed when the
            // CURRENT corner of the same side is null, so both are
            // defined inside this block.
            let next_left = next_left_corner
                .expect("next-left stays non-null when the current left corner does");
            if next_left.side_of(&from, dr) == Side::Negative {
                // bend to the right
                new_door_ind = right_ind + 1;
                result_corner = right_turn_next_corner(&from, trace_halfwidth_max, dr, &next_left);
                break;
            }
            let next_right = next_right_corner
                .expect("next-right stays non-null when the current right corner does");
            if next_right.side_of(&from, dl) == Side::Positive {
                // bend to the left
                new_door_ind = left_ind + 1;
                result_corner = left_turn_next_corner(&from, trace_halfwidth_max, dl, &next_right);
                break;
            }
        }
        // The visibility updates (`:209-242`) — Java assigns the
        // (possibly null-ed) next corner to the current corner.
        let mut visibility_range_gets_smaller_on_the_right_side = door_right_corner.is_none();
        if let Some(dr) = &door_right_corner
            && next_right_corner_raw.side_of(&from, dr) != Side::Negative
            && let Some(current_tangential_point) =
                from.left_tangential_point(&next_right_corner_raw, trace_halfwidth_max)
        {
            let check_line = FloatLine::new(from, current_tangential_point);
            if check_line.segment_distance(dr) >= trace_halfwidth_max {
                visibility_range_gets_smaller_on_the_right_side = true;
            }
        }
        if visibility_range_gets_smaller_on_the_right_side {
            // The visibility range gets smaller on the right side.
            door_right_corner = next_right_corner;
            right_ind = current_door_ind;
        }
        let mut visibility_range_gets_smaller_on_the_left_side = door_left_corner.is_none();
        if let Some(dl) = &door_left_corner
            && next_left_corner_raw.side_of(&from, dl) != Side::Positive
            && let Some(current_tangential_point) =
                from.right_tangential_point(&next_left_corner_raw, trace_halfwidth_max)
        {
            let check_line = FloatLine::new(from, current_tangential_point);
            if check_line.segment_distance(dl) >= trace_halfwidth_max {
                visibility_range_gets_smaller_on_the_left_side = true;
            }
        }
        if visibility_range_gets_smaller_on_the_left_side {
            // The visibility range gets smaller on the left side.
            door_left_corner = next_left_corner;
            left_ind = current_door_ind;
        }
        current_door_ind += 1;
    }

    if end_of_trace {
        let target_shape = st
            .current_target_shape
            .as_ref()
            .expect("the ctor set the target shape");
        let nearest_point = target_shape
            .nearest_point(&Point::int(from.round()))
            .expect("the target shape is non-empty (Java NPE otherwise)")
            .to_float();
        result_corner = ResultCorner::Point(nearest_point);
        if let Some(lt) = &left_tangent_point
            && nearest_point.side_of(&from, lt) == Side::Positive
        {
            // The nearest target point is to the left of the
            // visible range, add another corner (`:252-263`).
            new_door_ind = left_ind + 1;
            let target_right_corner = target_shape
                .corner_approx(target_shape.index_of_right_most_corner(&from))
                .expect("a non-empty target shape has the corner");
            if let Some(current_corner) = right_left_tangential_point(
                &from,
                &target_right_corner,
                door_left_corner.as_ref(),
                trace_halfwidth_max,
            ) {
                result_corner = ResultCorner::Point(current_corner);
                end_of_trace = false;
            }
        } else if let Some(rt) = &right_tangent_point
            && nearest_point.side_of(&from, rt) == Side::Negative
        {
            // The nearest target point is to the right of the
            // visible range, add another corner (`:264-277`).
            let target_left_corner = target_shape
                .corner_approx(target_shape.index_of_left_most_corner(&from))
                .expect("a non-empty target shape has the corner");
            new_door_ind = right_ind + 1;
            if let Some(current_corner) = left_right_tangential_point(
                &from,
                &target_left_corner,
                door_right_corner.as_ref(),
                trace_halfwidth_max,
            ) {
                result_corner = ResultCorner::Point(current_corner);
                end_of_trace = false;
            }
        }
    }
    if end_of_trace {
        new_door_ind = st.current_target_door_index;
    }

    // Check clearance violation with the previous door shapes and
    // correct them in this case (`:284-326`).
    let result_corner_point = match result_corner {
        ResultCorner::Point(point) => point,
        // The turn-helper fallbacks hand back the from-point OBJECT;
        // FloatLine(from, from) is the faithful zero-length line.
        ResultCorner::FromPoint => from,
        ResultCorner::None => {
            panic!("Java NPE: resultCorner is null at the clearance check (parallel tangent lines)")
        }
    };
    let check_line = FloatLine::new(from, result_corner_point);
    let check_from_door_index =
        std::cmp::max(st.current_to_door_index - 5, st.current_from_door_index + 1);
    let mut corrected_result: Option<(i32, FloatPoint)> = None;
    for i in check_from_door_index..new_door_ind {
        let info = &st.backtrack_array[i as usize];
        let current_left_corner = calc_door_left_corner(access, info);
        let current_distance = check_line.segment_distance(&current_left_corner);
        if current_distance.abs() < trace_halfwidth_middle
            && let Some(current_corrected_result) = right_left_tangential_point(
                &check_line.a,
                &check_line.b,
                Some(&current_left_corner),
                trace_halfwidth_max,
            )
        {
            let take = match &corrected_result {
                None => true,
                Some((_, corrected)) => {
                    current_corrected_result.side_of(&from, corrected) == Side::Negative
                }
            };
            if take {
                corrected_result = Some((i, current_corrected_result));
            }
        }
        let current_right_corner = calc_door_right_corner(access, info);
        let current_distance = check_line.segment_distance(&current_right_corner);
        if current_distance.abs() < trace_halfwidth_middle
            && let Some(current_corrected_result) = left_right_tangential_point(
                &check_line.a,
                &check_line.b,
                Some(&current_right_corner),
                trace_halfwidth_max,
            )
        {
            let take = match &corrected_result {
                None => true,
                Some((_, corrected)) => {
                    current_corrected_result.side_of(&from, corrected) == Side::Positive
                }
            };
            if take {
                corrected_result = Some((i, current_corrected_result));
            }
        }
    }
    if let Some((corrected_door_ind, corrected)) = corrected_result {
        result_corner = ResultCorner::Point(corrected);
        new_door_ind = std::cmp::max(corrected_door_ind, st.current_to_door_index);
    }

    st.current_to_door_index = new_door_ind;
    // Java `:329`: `resultCorner != null && resultCorner != this
    // .currentFromPoint` — only a FRESH point object is added; the
    // turn-helper fallbacks that hand back the from-point OBJECT emit
    // NOTHING here (an empty list = the trace-end signal).
    if let ResultCorner::Point(point) = result_corner {
        result.push(NextCorner::Fresh(point));
    }
    // net-33/66/67 hook — capture-only.
    result
}
