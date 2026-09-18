//! Port of Java `app.freerouting.autoroute.path.FoundConnectionLocator45Degree`
//! — the 45-degree corner synthesis dispatch of the found-connection
//! locator (T9). Used for BOTH the ninety-degree and the
//! fortyfive-degree restriction (Java `getInstance`); the restriction
//! flows into [`super::locator::calculate_additional_corner`].
//!
//! Java types are preserved: `traceHalfwidth` and `traceHalfwidthAdd`
//! are INTS (`AutorouteEngine.TRACE_WIDTH_TOLERANCE` is an int
//! constant), `IntBox.width()/height()` are int subtractions
//! (`wrapping_sub`), and every `int <= double` comparison promotes the
//! int side exactly like Java. The net-gated `FRLogger` hooks
//! (`NO_MORE_DOORS` / `TARGET_DOOR` / `EXPANSION_DOOR` /
//! `room_shrink`, nets 33/66/67) are the spike capture format — not
//! ported (module doc in `locator.rs`).

use epic_geometry::float_point::FloatPoint;

use crate::control::{AngleRestriction, AutorouteControl};
use crate::expansion::NeighbourEngine;
use crate::maze::list_element::ExpandableObject;
use crate::maze::locator_access::LocatorAccess;

use super::locator::{
    LocatorState, NextCorner, TRACE_WIDTH_TOLERANCE, calculate_additional_corner,
};

/// Java `Signum.of(double)` — `>0 → 1`, `<0 → −1`, else 0 (NaN → 0).
fn signum(value: f64) -> i32 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// Java `IntBox.width()/height()` — int subtractions (wrapping like
/// Java's two's-complement int arithmetic).
fn box_width(bb: &epic_geometry::int_box::IntBox) -> i32 {
    bb.ur.x.wrapping_sub(bb.ll.x)
}

fn box_height(bb: &epic_geometry::int_box::IntBox) -> i32 {
    bb.ur.y.wrapping_sub(bb.ll.y)
}

/// Java `roundToInteger` (`:40-42`).
fn round_to_integer(point: FloatPoint) -> FloatPoint {
    point.round().to_float()
}

/// Java `calcHorizontalFirstFromDoor` (`:48-101`) verdict core — the
/// FromDoor face of the horizontal-first mirror pair. The pair is
/// COMPLEMENT OFF-TIE, AGREEMENT ON TIE: on all five comparator arms the
/// two faces return the SAME verdict at the tie (`height >= width` vs
/// `height <= width` — both TRUE at height == width; each diagonal
/// strict `<`/`>` vs its mirror — both FALSE at |dx| == |dy|), so the
/// faces are NOT strict negations of each other and are ported as two
/// SEPARATE verbatim verdict functions ([`horizontal_first_core`] and
/// [`horizontal_first_to_door_core`]), never as core-plus-negation
/// (bug-139: the `!core` structuralization flipped all five tie
/// verdicts).
pub(crate) fn horizontal_first_core(
    shape: &epic_geometry::tile_shape::TileShape,
    dimension: i32,
    from_point: &FloatPoint,
    to_point: &FloatPoint,
) -> bool {
    let from_door_box = shape.bounding_box();
    if dimension != 1 {
        return box_height(&from_door_box) >= box_width(&from_door_box);
    }

    let door_line_segment = shape
        .diagonal_corner_segment()
        .expect("a 1-dimensional door shape has a diagonal corner segment");
    // tie `a.x == b.x && a.y <= b.y` keeps a as the left corner.
    let (left_corner, right_corner) = if door_line_segment.a.x < door_line_segment.b.x
        || (door_line_segment.a.x == door_line_segment.b.x
            && door_line_segment.a.y <= door_line_segment.b.y)
    {
        (door_line_segment.a, door_line_segment.b)
    } else {
        (door_line_segment.b, door_line_segment.a)
    };
    let door_dx = right_corner.x - left_corner.x;
    let door_dy = right_corner.y - left_corner.y;
    let abs_door_dy = door_dy.abs();
    let door_max_width = door_dx.max(abs_door_dy);
    let door_half_max_width = 0.5 * door_max_width;
    if f64::from(box_width(&from_door_box)) <= door_half_max_width {
        // door is about vertical
        true
    } else if f64::from(box_height(&from_door_box)) <= door_half_max_width {
        // door is about horizontal
        false
    } else {
        let dx = to_point.x - from_point.x;
        let dy = to_point.y - from_point.y;
        if left_corner.y < right_corner.y {
            // door is about right diagonal
            if signum(dx) == signum(dy) {
                dx.abs() > dy.abs()
            } else {
                dx.abs() < dy.abs()
            }
        } else {
            // door is about left diagonal
            if signum(dx) == signum(dy) {
                dx.abs() < dy.abs()
            } else {
                dx.abs() > dy.abs()
            }
        }
    }
}

/// Java `calcHorizontalFirstFromDoor` (`:48-101`) — calculates if the
/// next 45-degree angle should be horizontal first when coming from
/// `from_point` on `from_door`.
pub(crate) fn calc_horizontal_first_from_door<A: LocatorAccess>(
    access: &A,
    from_door: &ExpandableObject,
    from_point: &FloatPoint,
    to_point: &FloatPoint,
) -> bool {
    let door_shape = access.expandable_object_shape(from_door);
    horizontal_first_core(&door_shape, from_door.dimension(), from_point, to_point)
}

/// Java `calcHorizontalFirstToDoor` (`:304-356`) verdict core — the
/// ToDoor face of the mirror pair, ported VERBATIM (not as a negation
/// of [`horizontal_first_core`]; the faces agree at the five comparator
/// ties, see the mirror-pair note there). The corner-classification
/// block (`:311-328`) is deliberately duplicated from the FromDoor face
/// — in Java the two blocks are byte-identical, but the RESULT arms
/// below are per-face literals so no shared structuralization can
/// silently fix a tie comparator for both faces.
pub(crate) fn horizontal_first_to_door_core(
    shape: &epic_geometry::tile_shape::TileShape,
    dimension: i32,
    from_point: &FloatPoint,
    to_point: &FloatPoint,
) -> bool {
    let from_door_box = shape.bounding_box();
    if dimension != 1 {
        return box_height(&from_door_box) <= box_width(&from_door_box);
    }

    let door_line_segment = shape
        .diagonal_corner_segment()
        .expect("a 1-dimensional door shape has a diagonal corner segment");
    // tie `a.x == b.x && a.y <= b.y` keeps a as the left corner.
    let (left_corner, right_corner) = if door_line_segment.a.x < door_line_segment.b.x
        || (door_line_segment.a.x == door_line_segment.b.x
            && door_line_segment.a.y <= door_line_segment.b.y)
    {
        (door_line_segment.a, door_line_segment.b)
    } else {
        (door_line_segment.b, door_line_segment.a)
    };
    let door_dx = right_corner.x - left_corner.x;
    let door_dy = right_corner.y - left_corner.y;
    let abs_door_dy = door_dy.abs();
    let door_max_width = door_dx.max(abs_door_dy);
    let door_half_max_width = 0.5 * door_max_width;
    if f64::from(box_width(&from_door_box)) <= door_half_max_width {
        // door is about vertical
        false
    } else if f64::from(box_height(&from_door_box)) <= door_half_max_width {
        // door is about horizontal
        true
    } else {
        let dx = to_point.x - from_point.x;
        let dy = to_point.y - from_point.y;
        if left_corner.y < right_corner.y {
            // door is about right diagonal
            if signum(dx) == signum(dy) {
                dx.abs() < dy.abs()
            } else {
                dx.abs() > dy.abs()
            }
        } else {
            // door is about left diagonal
            if signum(dx) == signum(dy) {
                dx.abs() > dy.abs()
            } else {
                dx.abs() < dy.abs()
            }
        }
    }
}

/// Java `calcHorizontalFirstToDoor` (`:304-356`) — the access wrapper:
/// resolves the door shape through the live board and delegates to the
/// verbatim [`horizontal_first_to_door_core`].
pub(crate) fn calc_horizontal_first_to_door<A: LocatorAccess>(
    access: &A,
    to_door: &ExpandableObject,
    from_point: &FloatPoint,
    to_point: &FloatPoint,
) -> bool {
    let door_shape = access.expandable_object_shape(to_door);
    horizontal_first_to_door_core(&door_shape, to_door.dimension(), from_point, to_point)
}

/// Java `calculateNextTraceCorners` (`:104-298`) — calculates a list
/// with the next corners of the trace under construction; an EMPTY
/// result ends the current trace. The warn arms return their partial
/// result WITHOUT advancing `currentToDoorIndex`, exactly like Java.
pub(crate) fn calculate_next_trace_corners<A: LocatorAccess>(
    st: &mut LocatorState,
    access: &A,
    ctrl: &AutorouteControl,
    angle_restriction: AngleRestriction,
) -> Vec<NextCorner> {
    let mut result: Vec<NextCorner> = Vec::new();
    if st.current_to_door_index > st.current_target_door_index {
        return result;
    }

    let current_from_info = st.backtrack_array[(st.current_to_door_index - 1) as usize].clone();

    let Some(next_room_key) = current_from_info.next_room else {
        return result;
    };
    let room_shape = access.engine().room_shape(next_room_key);

    let trace_halfwidth = ctrl.compensated_trace_half_width[st.current_trace_layer as usize];
    // add some tolerance for free space expansion rooms (`:138-141`).
    let trace_halfwidth_add = trace_halfwidth + TRACE_WIDTH_TOLERANCE;
    let shrink_offset = if access.engine().room_is_obstacle(next_room_key) {
        trace_halfwidth
    } else {
        trace_halfwidth_add
    };

    let mut shrinked_room_shape = room_shape.offset(-f64::from(shrink_offset));
    // net-33/66/67 room_shrink hook — capture-only.
    if !shrinked_room_shape.is_empty() {
        // enter the shrunk room shape by a 45-degree angle first
        // (`:175-186`). The horizontal-first decision reads the
        // UNROUNDED nearest point; the added corners use the rounded
        // one. Java updates `currentFromPoint` here but NOT
        // `previousFromPoint`.
        let nearest_room_point = shrinked_room_shape.nearest_point_approx(
            st.current_from_point
                .as_ref()
                .expect("the trace loop runs with the from point set"),
        );
        let horizontal_first = calc_horizontal_first_from_door(
            access,
            &current_from_info.door,
            st.current_from_point.as_ref().expect("set above"),
            &nearest_room_point,
        );
        let nearest_room_point = round_to_integer(nearest_room_point);
        let from = st.current_from_point.expect("set above");
        result.push(NextCorner::Fresh(calculate_additional_corner(
            &from,
            &nearest_room_point,
            horizontal_first,
            angle_restriction,
        )));
        result.push(NextCorner::Fresh(nearest_room_point));
        st.current_from_point = Some(nearest_room_point);
    } else {
        shrinked_room_shape = room_shape;
    }

    if st.current_to_door_index == st.current_target_door_index {
        // the next trace leads to the final target (`:191-226`).
        let from = st.current_from_point.expect("set above");
        let nearest_point = round_to_integer(
            st.current_target_shape
                .as_ref()
                .expect("the ctor set the target shape")
                .nearest_point_approx(&from),
        );
        let mut add_corner =
            calculate_additional_corner(&from, &nearest_point, true, angle_restriction);
        // Java `shrinkedRoomShape.contains(addCorner)` is the VIRTUAL
        // one-arg dispatch: for an IntOctagon room shape it selects the
        // border-INCLUDED `IntOctagon.contains(FloatPoint)` override
        // (`IntOctagon.java:332-344`), for box/simplex the strict
        // `TileShape.contains(FloatPoint, 0)` loop. The port's
        // `contains_float` mirrors the dispatch.
        if !shrinked_room_shape.contains_float(&add_corner) {
            add_corner =
                calculate_additional_corner(&from, &nearest_point, false, angle_restriction);
        }
        result.push(NextCorner::Fresh(add_corner));
        result.push(NextCorner::Fresh(nearest_point));
        st.current_to_door_index += 1;
        // net-33/66/67 TARGET_DOOR hook — capture-only.
        return result;
    }

    let current_to_info = st.backtrack_array[st.current_to_door_index as usize].clone();
    let ExpandableObject::RoomDoor(current_to_door) = &current_to_info.door else {
        return result;
    };

    let nearest_to_door_point;
    if current_to_door.dimension == 2 {
        // May not happen in free angle routing mode because then
        // corners are cut off (`:236-242`).
        let to_door_shape = access.expandable_object_shape(&current_to_info.door);
        let shrinked_to_door_shape = to_door_shape.shrink(f64::from(shrink_offset));
        nearest_to_door_point = shrinked_to_door_shape.nearest_point_approx(
            st.current_from_point
                .as_ref()
                .expect("the trace loop runs with the from point set"),
        );
    } else {
        // Java's 1-arg `getSectionSegments(traceHalfwidth)`: the Rust
        // 5-arg version adds the tolerance internally and reproduces
        // the whole 1-arg dispatch (door.rs `:137`).
        let first_key = access
            .engine()
            .room_key_of_id(current_to_door.first_room_id);
        let second_key = access
            .engine()
            .room_key_of_id(current_to_door.second_room_id);
        let first_shape = access
            .engine()
            .room_shape(first_key.expect("a live room door carries its endpoint rooms"));
        let second_shape = access
            .engine()
            .room_shape(second_key.expect("a live room door carries its endpoint rooms"));
        let both_complete_free_space = first_key
            .is_some_and(|k| access.engine().room_is_complete_free_space(k))
            && second_key.is_some_and(|k| access.engine().room_is_complete_free_space(k));
        let door_shape =
            crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape);
        let (_section_count, line_sections) = current_to_door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            f64::from(trace_halfwidth),
        );
        if current_to_info.section_no_of_door >= line_sections.len() as i32 {
            return result;
        }
        let current_line_section = &line_sections[current_to_info.section_no_of_door as usize];
        let mut point = current_line_section.nearest_segment_point(
            st.current_from_point
                .as_ref()
                .expect("the trace loop runs with the from point set"),
        );

        let mut nearest_to_door_point_ok = true;
        if let Some(to_next_room_key) = current_to_info.next_room {
            // Java routes through `toSimplex()` — regular tile border
            // lines include zero-length edges that would change the
            // candidate set (Simplex::nearest_border_points_approx doc).
            let next_room_shape = access.engine().room_shape(to_next_room_key).to_simplex();
            let nearest_points = next_room_shape.nearest_border_points_approx(&point, 2);
            if nearest_points.len() >= 2 {
                nearest_to_door_point_ok =
                    nearest_points[1].distance(&point) >= f64::from(trace_halfwidth_add);
            }
        }
        if !nearest_to_door_point_ok {
            // may be the room has an acute (45 degree) angle at a
            // corner of the door (`:264-267`).
            point = current_line_section.a.middle_point(&current_line_section.b);
        }
        nearest_to_door_point = point;
    }
    let nearest_to_door_point = round_to_integer(nearest_to_door_point);
    let horizontal_first = calc_horizontal_first_to_door(
        access,
        &current_to_info.door,
        st.current_from_point
            .as_ref()
            .expect("the trace loop runs with the from point set"),
        &nearest_to_door_point,
    );
    let from = st.current_from_point.expect("set above");
    result.push(NextCorner::Fresh(calculate_additional_corner(
        &from,
        &nearest_to_door_point,
        horizontal_first,
        angle_restriction,
    )));
    result.push(NextCorner::Fresh(nearest_to_door_point));
    st.current_to_door_index += 1;
    // net-33/66/67 EXPANSION_DOOR hook — capture-only.
    result
}
