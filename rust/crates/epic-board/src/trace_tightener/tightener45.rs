//! Java `board/optimize/TraceTightener45.java` (674 l) — the 45°
//! pull-tight variant: the fixpoint (`pullTight` `:35-45`) runs
//! reduce-corner → smoothen-corner → reposition until stable (the 90°
//! variant runs skip-second-corner → skip-corners → reposition
//! instead), the corner-count reducer (`reduceCorners` `:52-221`)
//! with its two clip-gated translation arms, and the REAL smoothen
//! bodies (`smoothenCorners` `:227-267`, `smoothenSharpCorner`
//! `:275-310`, `smoothenNonIntegerCorner` `:316-368`, `smoothenCorner`
//! `:376-459`, `smoothenStartCornerAtTrace` `:462-565`,
//! `smoothenEndCornerAtTrace` `:568-673`) that the 90° variant only
//! stubs.
//!
//! The `pullTight` loop condition `newResult != prevResult` is a Java
//! REFERENCE comparison; the port compares values. Every no-change
//! stage returns its input object, so reference inequality with VALUE
//! equality would mean the stage rebuilt the same geometry — the
//! next loop would rebuild it again forever under Java semantics, so
//! stopping at value equality is the terminating face of the same
//! verdict (the T3 documented proxy, module docs of [`super`]).
//!
//! Tie faces (cerebrum mode 11/16), with their MEASURED mutant
//! verdicts (logs/M4-T4/evidence/): `smoothenCorner`'s
//! `prevDist <= nextDist` picks the PREV corner on an exact tie
//! (`:392`) — the flip mutant fires once on the symmetric chamfer
//! world and HEALS to the same final (banked healer T4-M2,
//! `mut_T4-M2_smoothen_tie_reach_1hit.log`); `reposition_line`'s
//! strict nearer-corner pick likewise heals — fires 11x across the
//! 45° pin suite, converges identically (banked healer T4-M1). The
//! equality break `translateDist == maxTranslateDist` (`:436`) is
//! INTENTIONAL exact-float equality — the overshoot arm mutates both
//! by the same `shortenValue`, so the equality still detects the
//! first-iteration face (the T3-Q4 biggest-change break; re-probed on
//! the full 45° pin suite in T4, STILL heals — banked, see
//! `reprobe_T3-Q4_first_time_break_removed.log`). Killed mutants: the
//! dx/dy
//! swap of the sharp-corner shave anchor (T4-M3, termination face)
//! and the composite 45°→90° mis-dispatch (T4-M4).

use epic_geometry::direction::Direction;
use epic_geometry::int_point::IntPoint;
use epic_geometry::limits::SQRT2;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::side::Side;

use crate::board::Board;
use crate::forced_pad_router::check_trace_shape;
use crate::items::ItemData;
use crate::trace_ops::is_shove_fixed;
use crate::tree_manager::SearchTreeManager;

use super::TraceTightener;

/// Java `TraceTightener45.pullTight` (`:35-45`): acid-trap identity,
/// then the fixed point reduce-corner → smoothen-corner → reposition
/// until nothing changes (or the stop face fires).
pub(crate) fn pull_tight_45(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: Polyline,
) -> Polyline {
    let mut new_result = state.avoid_acid_traps(manager, board, polyline);
    let mut prev_result: Option<Polyline> = None;
    // Java `while (newResult != prevResult && !this.isStopRequested())`
    // — null prevResult makes the first iteration unconditional (the
    // value-equality proxy, module docs).
    while prev_result.as_ref() != Some(&new_result) && !state.is_stop_requested() {
        prev_result = Some(new_result.clone());
        let prev = prev_result.as_ref().expect("just set");
        let tmp1 = reduce_corners(state, manager, board, prev);
        let tmp2 = smoothen_corners(state, manager, board, &tmp1);
        new_result = state.reposition_lines(manager, board, &tmp2);
    }
    new_result
}

/// Java `reduceCorners` (`:52-221`): sweep the polyline with a 4-slot
/// corner window and try to remove each middle corner by translating
/// its adjacent line toward the previous or the next corner. Both
/// arms are clearance-checked through `checkTraceShape` on the
/// offset shape of the would-be deleted corner; the exact-corner
/// retention of the polyline ends rides the window (corners 0 and the
/// last two are appended verbatim at `:216-219`).
fn reduce_corners(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
) -> Polyline {
    let len = polyline.lines.len();
    // Java `:53-55`.
    if len <= 4 {
        return polyline.clone();
    }
    // Java `:56-62`: the first four corners must be exact integer
    // points, else the input wins. `Polyline.corner` would only miss
    // past the end (Java NPE face) — every index here is in range.
    let mut corner: [Point; 4] = [
        polyline.corner(0).expect("corner 0 of a live polyline"),
        polyline.corner(1).expect("corner 1 of a live polyline"),
        polyline.corner(2).expect("corner 2 of a live polyline"),
        polyline.corner(3).expect("corner 3 of a live polyline"),
    ];
    for c in &corner {
        if !matches!(c, Point::Int(_)) {
            return polyline.clone();
        }
    }
    // Java `:63-71`: the per-corner clip flags (null clip = all in).
    let in_clip = |p: &Point| -> bool {
        match &state.current_clip_shape {
            None => true,
            // Java `TileShape.isOutside(Point)`; the T3 proxy
            // convention (`!clip.contains(point.toFloat())`).
            Some(clip) => !clip.contains(&p.to_float()),
        }
    };
    let mut clip: [bool; 4] = [true; 4];
    for (i, c) in corner.iter().enumerate() {
        clip[i] = in_clip(c);
    }

    let mut polyline_changed = false;
    // Java `:74-77`: newCorners[0] = corner[0], count starts at 1.
    let mut new_corners: Vec<Point> = Vec::with_capacity(len - 3);
    new_corners.push(corner[0].clone());
    let mut new_corner: Option<Point> = None;
    let mut corner_index: i32 = 3;
    // Java `:80` `while (cornerIndex < polyline.lines.length - 1)`.
    while corner_index < len as i32 - 1 {
        corner[3] = polyline.corner(corner_index).expect("corner in range");
        if !matches!(corner[3], Point::Int(_)) {
            return polyline.clone();
        }
        // Java `:85-87` precedence: `a || (b && c)` — corners in the
        // middle of a line can be skipped.
        if corner[1] == corner[2]
            || (corner_index < len as i32 - 2
                && corner[3].side_of(&corner[1], &corner[2]) == Side::Collinear)
        {
            // Java `:89-98`: shift and RELOAD corner[3] — unless the
            // shifted index is the loop end, where corner[3] keeps
            // the skipped value (== corner[2]; the removal arm below
            // then fires on the equal-corner face, Java-exact).
            corner_index += 1;
            corner[2] = corner[3].clone();
            clip[2] = clip[3];
            if corner_index < len as i32 - 1 {
                corner[3] = polyline.corner(corner_index).expect("corner in range");
                if !matches!(corner[3], Point::Int(_)) {
                    return polyline.clone();
                }
            }
            polyline_changed = true;
        }
        // Java `:100-101`.
        clip[3] = in_clip(&corner[3]);
        let mut corner_removed = false;
        if clip[1] && clip[2] && clip[3] {
            // Java `:103-147`: translate the line from corner[2] to
            // corner[1] so it runs toward corner[3].
            let delta = corner[3].difference_by(&corner[2]);
            let candidate = corner[1].translate_by(&delta);
            new_corner = Some(candidate.clone());
            if corner[3] == corner[2] {
                // just remove multiple corner
                corner_removed = true;
            } else if candidate.side_of(&corner[0], &corner[1]) == Side::Collinear {
                let mut check_points = [candidate, corner[1].clone()];
                let check_polyline = Polyline::from_points(&check_points);
                if check_polyline.lines.len() == 3 {
                    let shape_to_check = check_polyline
                        .offset_shape(state.current_half_width, 0)
                        .expect("2-point polyline has offset shape 0");
                    if check_trace_shape(
                        manager,
                        board,
                        &shape_to_check,
                        state.current_layer,
                        &state.current_net_numbers,
                        state.current_clearance_class_index,
                        state.contact_pins.as_ref(),
                    ) {
                        check_points[1] = corner[3].clone();
                        if check_points[0] == check_points[1] {
                            corner_removed = true;
                        } else {
                            let check_polyline = Polyline::from_points(&check_points);
                            if check_polyline.lines.len() == 3 {
                                let shape_to_check = check_polyline
                                    .offset_shape(state.current_half_width, 0)
                                    .expect("2-point polyline has offset shape 0");
                                corner_removed = check_trace_shape(
                                    manager,
                                    board,
                                    &shape_to_check,
                                    state.current_layer,
                                    &state.current_net_numbers,
                                    state.current_clearance_class_index,
                                    state.contact_pins.as_ref(),
                                );
                            } else {
                                corner_removed = true;
                            }
                        }
                    }
                } else {
                    // the would-be corner coincides with corner[1] —
                    // removal without a check (Java `:143-145`).
                    corner_removed = true;
                }
            }
        }
        if !corner_removed && clip[0] && clip[1] && clip[2] {
            // Java `:148-190`: the first try has failed — translate
            // the line from corner[2] to corner[1] toward corner[0].
            let delta = corner[0].difference_by(&corner[1]);
            let candidate = corner[2].translate_by(&delta);
            new_corner = Some(candidate.clone());
            if corner[0] == corner[1] {
                // just remove multiple corner
                corner_removed = true;
            } else if candidate.side_of(&corner[2], &corner[3]) == Side::Collinear {
                let mut check_points = [candidate, corner[0].clone()];
                let check_polyline = Polyline::from_points(&check_points);
                if check_polyline.lines.len() == 3 {
                    let shape_to_check = check_polyline
                        .offset_shape(state.current_half_width, 0)
                        .expect("2-point polyline has offset shape 0");
                    if check_trace_shape(
                        manager,
                        board,
                        &shape_to_check,
                        state.current_layer,
                        &state.current_net_numbers,
                        state.current_clearance_class_index,
                        state.contact_pins.as_ref(),
                    ) {
                        check_points[1] = corner[2].clone();
                        let check_polyline = Polyline::from_points(&check_points);
                        if check_polyline.lines.len() == 3 {
                            let shape_to_check = check_polyline
                                .offset_shape(state.current_half_width, 0)
                                .expect("2-point polyline has offset shape 0");
                            corner_removed = check_trace_shape(
                                manager,
                                board,
                                &shape_to_check,
                                state.current_layer,
                                &state.current_net_numbers,
                                state.current_clearance_class_index,
                                state.contact_pins.as_ref(),
                            );
                        } else {
                            corner_removed = true;
                        }
                    }
                } else {
                    corner_removed = true;
                }
            }
        }
        if corner_removed {
            // Java `:191-200`.
            polyline_changed = true;
            let nc = new_corner.clone().expect("the removing arm set newCorner");
            corner[1] = nc;
            clip[1] = in_clip(&corner[1]);
            if board.changed_area.is_some() {
                crate::routing_board_insert::join_changed_area(
                    board,
                    &corner[1].to_float(),
                    state.current_layer,
                );
                crate::routing_board_insert::join_changed_area(
                    board,
                    &corner[1].to_float(),
                    state.current_layer,
                );
                crate::routing_board_insert::join_changed_area(
                    board,
                    &corner[2].to_float(),
                    state.current_layer,
                );
            }
        } else {
            // Java `:201-208`: keep the corner, slide the window.
            new_corners.push(corner[1].clone());
            corner[0] = corner[1].clone();
            corner[1] = corner[2].clone();
            clip[0] = clip[1];
            clip[1] = clip[2];
        }
        corner[2] = corner[3].clone();
        clip[2] = clip[3];
        corner_index += 1;
    }
    // Java `:213-215`.
    if !polyline_changed {
        return polyline.clone();
    }
    // Java `:216-220`: kept corners + the two window survivors.
    new_corners.push(corner[1].clone());
    new_corners.push(corner[2].clone());
    Polyline::from_points(&new_corners)
}

/// Java `smoothenCorners` (`:227-267`): insert a chamfer line at every
/// 90°-or-sharper corner (greedy `smoothenCorner` first, the
/// unchecked `smoothenSharpCorner` as the fallback), rebuilding the
/// polyline once per PASS (the inserts grow the live array; the loop
/// bound re-reads the grown length every iteration).
fn smoothen_corners(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
) -> Polyline {
    let mut result = polyline.clone();
    let mut polyline_changed = true;
    while polyline_changed {
        // Java `:231-233` — the guard re-runs at every pass top.
        if result.lines.len() < 4 {
            return result;
        }
        polyline_changed = false;
        let mut lines: Vec<Line> = result.lines.clone();
        let mut i: i32 = 1;
        // Java `:238` `for (int i = 1; i < lines.length - 2; i++)` —
        // `lines` is reassigned on insert, so the bound moves.
        while i < lines.len() as i32 - 2 {
            let d1 = lines[i as usize].direction().clone();
            let d2 = lines[(i + 1) as usize].direction().clone();
            if d1.is_multiple_of_45_degree()
                && d2.is_multiple_of_45_degree()
                && d1.projection(&d2) != Side::Positive
            {
                // there is a 90 degree or sharper angle
                let new_line = smoothen_corner(state, manager, board, &lines, i).or_else(|| {
                    // the greedy smoothening couldn't change the polyline
                    smoothen_sharp_corner(state, manager, board, &lines, i)
                });
                if let Some(new_line) = new_line {
                    polyline_changed = true;
                    // add the new line into the line array (Java
                    // `:253-257` arraycopy insert at i+1) and skip
                    // past it (`++i` plus the loop `i++`).
                    lines.insert((i + 1) as usize, new_line);
                    i += 1;
                }
            }
            i += 1;
        }
        if polyline_changed {
            result = Polyline::new(lines);
        }
    }
    result
}

/// Java `smoothenSharpCorner` (`:275-310`): shave a corner by a line
/// parallel to the mean of the two corner directions at distance
/// `(sqrt2 - 1) * halfWidth` — so small that NO clearance check runs.
fn smoothen_sharp_corner(
    state: &mut TraceTightener,
    _manager: &mut SearchTreeManager, // unused: the shave is check-free (Java `:275-310`)
    board: &mut Board,
    lines: &[Line],
    no: i32,
) -> Option<Line> {
    let current_corner = lines[no as usize].intersection_approx(&lines[(no + 1) as usize]);
    if current_corner.x != (current_corner.x as i32) as f64 {
        // intersection of 2 diagonal lines is not integer — only x is
        // tested (Java `:277`), the y face rides along.
        if let Some(result) = smoothen_non_integer_corner(lines, no) {
            return Some(result);
        }
    }
    let prev_corner = lines[no as usize].intersection_approx(&lines[(no - 1) as usize]);
    let next_corner = lines[(no + 1) as usize].intersection_approx(&lines[(no + 2) as usize]);
    let prev_dir = lines[no as usize].direction();
    let next_dir = lines[(no + 1) as usize].direction();
    let new_line_dir = Direction::get_instance(&prev_dir.get_vector().add(&next_dir.get_vector()));
    let translate_line = Line::get_instance(Point::Int(current_corner.round()), new_line_dir);
    let mut translate_dist = (SQRT2 - 1.0) * f64::from(state.current_half_width);
    let prev_dist = translate_line.signed_distance(&prev_corner).abs();
    let next_dist = translate_line.signed_distance(&next_corner).abs();
    translate_dist = translate_dist.min(prev_dist);
    translate_dist = translate_dist.min(next_dist);
    if translate_dist < 0.99 {
        return None;
    }
    translate_dist = (translate_dist - 1.0).max(1.0);
    if translate_line.side_of_float_zero(&next_corner) == Side::Positive {
        translate_dist = -translate_dist;
    }
    let result = translate_line.translate(translate_dist);
    if board.changed_area.is_some() {
        crate::routing_board_insert::join_changed_area(board, &current_corner, state.current_layer);
    }
    Some(result)
}

/// Java `smoothenNonIntegerCorner` (`:316-368`): remove a
/// non-integer corner of two diagonals with a short axis-parallel
/// line; the ceil/floor branch picks the lattice point toward the
/// polyline's interior side. Null when the four branches all miss.
fn smoothen_non_integer_corner(lines: &[Line], no: i32) -> Option<Line> {
    let prev_line = &lines[no as usize];
    let next_line = &lines[(no + 1) as usize];
    if prev_line.is_equal_or_opposite(next_line) {
        return None;
    }
    if !(prev_line.is_diagonal() && next_line.is_diagonal()) {
        return None;
    }
    let current_corner = prev_line.intersection_approx(next_line);
    let prev_corner = prev_line.intersection_approx(&lines[(no - 1) as usize]);
    let next_corner = next_line.intersection_approx(&lines[(no + 2) as usize]);
    let mut new_x: i32 = 0;
    let mut new_y: i32 = 0;
    let mut new_line_is_vertical = false;
    let mut new_line_is_horizontal = false;
    if prev_corner.x > current_corner.x && next_corner.x > current_corner.x {
        new_x = current_corner.x.ceil() as i32;
        new_y = current_corner.y.ceil() as i32;
        new_line_is_vertical = true;
    } else if prev_corner.x < current_corner.x && next_corner.x < current_corner.x {
        new_x = current_corner.x.floor() as i32;
        new_y = current_corner.y.floor() as i32;
        new_line_is_vertical = true;
    } else if prev_corner.y > current_corner.y && next_corner.y > current_corner.y {
        new_x = current_corner.x.ceil() as i32;
        new_y = current_corner.y.ceil() as i32;
        new_line_is_horizontal = true;
    } else if prev_corner.y < current_corner.y && next_corner.y < current_corner.y {
        new_x = current_corner.x.floor() as i32;
        new_y = current_corner.y.floor() as i32;
        new_line_is_horizontal = true;
    }
    let new_line_dir: Direction;
    if new_line_is_vertical {
        if prev_corner.y < next_corner.y {
            new_line_dir = Direction::UP;
        } else {
            new_line_dir = Direction::DOWN;
        }
    } else if new_line_is_horizontal {
        if prev_corner.x < next_corner.x {
            new_line_dir = Direction::RIGHT;
        } else {
            new_line_dir = Direction::LEFT;
        }
    } else {
        return None;
    }
    Some(Line::get_instance(
        Point::Int(IntPoint::new(new_x, new_y)),
        new_line_dir,
    ))
}

/// Java `smoothenCorner` (`:376-459`): the GREEDY chamfer — bisect a
/// cut line toward the nearer of the two neighbor corners until the
/// clearance check accepts. The tie `prevDist <= nextDist` picks the
/// PREV corner (`:392`); the exact-float break `translateDist ==
/// maxTranslateDist` fires only on the first (biggest-change)
/// acceptance because the overshoot arm mutates both by the same
/// `shortenValue` (`:436`, `:444-449` — unlike the base
/// `repositionLine`, `maxTranslateDist` IS re-read here).
fn smoothen_corner(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    lines: &[Line],
    no: i32,
) -> Option<Line> {
    let no_us = no as usize;
    let prev_corner = lines[no_us].intersection_approx(&lines[no_us - 1]);
    let current_corner = lines[no_us].intersection_approx(&lines[no_us + 1]);
    let next_corner = lines[no_us + 1].intersection_approx(&lines[no_us + 2]);
    let prev_dir = lines[no_us].direction();
    let next_dir = lines[no_us + 1].direction();
    let new_line_dir = Direction::get_instance(&prev_dir.get_vector().add(&next_dir.get_vector()));
    let translate_line = Line::get_instance(Point::Int(current_corner.round()), new_line_dir);
    let prev_dist = translate_line.signed_distance(&prev_corner).abs();
    let next_dist = translate_line.signed_distance(&next_corner).abs();
    // Java `:387-389`.
    if prev_dist == 0.0 || next_dist == 0.0 {
        return None;
    }
    // Java `:390-398` — the tie face: `<=` picks PREV.
    let (mut max_translate_dist, nearest_corner) = if prev_dist <= next_dist {
        (prev_dist, prev_corner)
    } else {
        (next_dist, next_corner)
    };
    // Java `:399-401`.
    if max_translate_dist < 1.0 {
        return None;
    }
    // Java `:402-405`.
    max_translate_dist = (max_translate_dist - 1.0).max(1.0);
    if translate_line.side_of_float_zero(&next_corner) == Side::Positive {
        max_translate_dist = -max_translate_dist;
    }
    let mut check_lines: [Line; 3] = [
        lines[no_us].clone(),
        lines[no_us].clone(), // placeholder; overwritten in the loop
        lines[no_us + 1].clone(),
    ];
    let mut translate_dist = max_translate_dist;
    let mut delta_dist = max_translate_dist;
    let side_of_nearest_corner = translate_line.side_of_float_zero(&nearest_corner);
    let sign = Side::as_int(max_translate_dist);
    let mut result: Option<Line> = None;
    // Java `:414`.
    while delta_dist.abs() > f64::from(state.min_translate_dist) {
        let mut check_ok = false;
        let new_line = translate_line.translate(translate_dist);
        let new_line_side_of_nearest_corner = new_line.side_of_float_zero(&nearest_corner);
        if new_line_side_of_nearest_corner == side_of_nearest_corner
            || new_line_side_of_nearest_corner == Side::Collinear
        {
            check_lines[1] = new_line;
            let tmp = Polyline::new(check_lines.to_vec());
            if tmp.lines.len() == 3 {
                let shape_to_check = tmp
                    .offset_shape(state.current_half_width, 0)
                    .expect("3-line polyline has offset shape 0");
                check_ok = check_trace_shape(
                    manager,
                    board,
                    &shape_to_check,
                    state.current_layer,
                    &state.current_net_numbers,
                    state.current_clearance_class_index,
                    state.contact_pins.as_ref(),
                );
            }
            // Java `:433`: the halving lives INSIDE the side-ok branch.
            delta_dist /= 2.0;
            if check_ok {
                result = Some(check_lines[1].clone());
                if translate_dist == max_translate_dist {
                    // biggest possible change — the exact-float
                    // equality is the Java face; the overshoot arm
                    // mutates both equally so it tracks first-accept.
                    break;
                }
                translate_dist += delta_dist;
            } else {
                translate_dist -= delta_dist;
            }
        } else {
            // moved a little bit too far at the first time because of
            // numerical inaccuracy (Java `:444-449` — maxTranslateDist
            // IS re-read by the `==` break).
            let shorten_value = f64::from(sign) * 0.5;
            max_translate_dist -= shorten_value;
            translate_dist -= shorten_value;
            delta_dist -= shorten_value;
        }
    }
    if let Some(accepted) = &result
        && board.changed_area.is_some()
    {
        let new_prev_corner = check_lines[0].intersection_approx(accepted);
        let new_next_corner = check_lines[2].intersection_approx(accepted);
        crate::routing_board_insert::join_changed_area(
            board,
            &new_prev_corner,
            state.current_layer,
        );
        crate::routing_board_insert::join_changed_area(
            board,
            &new_next_corner,
            state.current_layer,
        );
        crate::routing_board_insert::join_changed_area(board, &current_corner, state.current_layer);
    }
    result
}

/// Java `smoothenStartCornerAtTrace` (`:462-565`): reshape the trace's
/// FIRST corner around a touching trace — the acute arm adds a 45°
/// shave line toward the contact, the bend arm repositions the first
/// segment using the contact's lines as context.
pub(crate) fn smoothen_start_corner_at_trace_45(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: crate::id::ItemId,
) -> Option<Polyline> {
    let mut acute_angle = false;
    let mut bend = false;
    let mut other_trace_corner_approx: Option<epic_geometry::float_point::FloatPoint> = None;
    let mut other_trace_line: Option<Line> = None;
    let mut other_prev_trace_line: Option<Line> = None;
    // Java `:468`: the live trace's polyline — the receiver cannot be
    // absent on this face (the base checked `is_on_the_board`).
    let trace_polyline = board
        .trace_polyline(trace_id)
        .cloned()
        .expect("live trace polyline");
    // Java `:469` — exact corner 0.
    let current_end_corner = trace_polyline.corner(0).expect("corner 0");
    // Java `:471-473`.
    if let Some(clip) = &state.current_clip_shape
        && !clip.contains(&current_end_corner.to_float())
    {
        return None;
    }
    // Java `:475-478`.
    let current_prev_end_corner = trace_polyline.corner(1).expect("corner 1");
    let mut prev_corner_side: Option<Side> = None;
    let line_direction = trace_polyline.lines[1].direction().clone();
    let prev_line_direction = trace_polyline.lines[2].direction().clone();
    // Java `:480`.
    let contact_list = crate::contacts::start_contacts(manager, board, trace_id);
    for contact in contact_list {
        // Java `:482`: `instanceof PolylineTrace && !isShoveFixed()`,
        // else the WHOLE arm returns null.
        let is_candidate = matches!(
            board.get(contact).map(|entry| &entry.data),
            Some(ItemData::Trace { .. })
        ) && !is_shove_fixed(board, contact);
        if !is_candidate {
            return None;
        }
        let contact_polyline = board
            .trace_polyline(contact)
            .cloned()
            .expect("live contact trace");
        let (current_other_corner_approx, current_other_line, current_other_prev_line) =
            if contact_polyline.first_corner().as_ref() == Some(&current_end_corner) {
                (
                    contact_polyline.corner_approx(1),
                    contact_polyline.lines[1].clone(),
                    contact_polyline.lines[2].clone(),
                )
            } else {
                let current_corner_no = contact_polyline.corner_count() as i32 - 2;
                (
                    contact_polyline.corner_approx(current_corner_no),
                    // Java `:494`: the contact's far-end line REVERSED.
                    contact_polyline.lines[(current_corner_no + 1) as usize].opposite(),
                    contact_polyline.lines[current_corner_no as usize].clone(),
                )
            };
        let current_prev_corner_side = current_prev_end_corner.side_of_line(&current_other_line);
        let current_projection = line_direction.projection(current_other_line.direction());
        let mut other_trace_found = false;
        if current_projection == Side::Positive && current_prev_corner_side != Side::Collinear {
            if current_other_line.direction().is_orthogonal() {
                acute_angle = true;
                other_trace_found = true;
            }
        } else if current_projection == Side::Collinear
            && trace_polyline.corner_count() > 2
            && prev_line_direction.projection(current_other_line.direction()) == Side::Positive
        {
            bend = true;
            other_trace_found = true;
        }
        if other_trace_found {
            other_trace_corner_approx = Some(current_other_corner_approx);
            other_trace_line = Some(current_other_line);
            prev_corner_side = Some(current_prev_corner_side);
            other_prev_trace_line = Some(current_other_prev_line);
        }
    }
    if acute_angle {
        // Java `:522-549`.
        let other_line = other_trace_line.as_ref().expect("acute arm has a contact");
        let new_line_dir = if prev_corner_side == Some(Side::Positive) {
            other_line.direction().turn_45_degree(2)
        } else {
            other_line.direction().turn_45_degree(6)
        };
        let translate_line = Line::get_instance(
            Point::Int(current_end_corner.to_float().round()),
            new_line_dir,
        );
        let mut translate_dist = (SQRT2 - 1.0) * f64::from(state.current_half_width);
        let prev_corner_dist = translate_line
            .signed_distance(&current_prev_end_corner.to_float())
            .abs();
        let other_dist = translate_line
            .signed_distance(&other_trace_corner_approx.expect("acute arm corner approx"))
            .abs();
        translate_dist = translate_dist.min(prev_corner_dist);
        translate_dist = translate_dist.min(other_dist);
        if translate_dist >= 0.99 {
            translate_dist = (translate_dist - 1.0).max(1.0);
            if translate_line.side_of(&current_prev_end_corner) == Side::Positive {
                translate_dist = -translate_dist;
            }
            let add_line = translate_line.translate(translate_dist);
            // construct the new trace polyline — the trace's FIRST
            // line is replaced by [otherTraceLine, addLine] (Java
            // `:544-548` arraycopy keeps lines[1..]).
            let mut new_lines: Vec<Line> = Vec::with_capacity(trace_polyline.lines.len() + 1);
            new_lines.push(other_line.clone());
            new_lines.push(add_line);
            new_lines.extend_from_slice(&trace_polyline.lines[1..]);
            return Some(Polyline::new(new_lines));
        }
    } else if bend {
        // Java `:550-563`.
        let other_line = other_trace_line.as_ref().expect("bend arm has a contact");
        let other_prev_line = other_prev_trace_line
            .as_ref()
            .expect("bend arm has a prev contact line");
        let mut check_line_arr: Vec<Line> = Vec::with_capacity(trace_polyline.lines.len() + 1);
        check_line_arr.push(other_prev_line.clone());
        check_line_arr.push(other_line.clone());
        check_line_arr.extend_from_slice(&trace_polyline.lines[1..]);
        if let Some(new_line) = state.reposition_line(manager, board, &check_line_arr, 2) {
            let mut new_lines: Vec<Line> = Vec::with_capacity(trace_polyline.lines.len());
            new_lines.push(other_line.clone());
            new_lines.push(new_line);
            new_lines.extend_from_slice(&trace_polyline.lines[2..]);
            return Some(Polyline::new(new_lines));
        }
    }
    None
}

/// Java `smoothenEndCornerAtTrace` (`:568-673`): the END-corner mirror
/// of [`smoothen_start_corner_at_trace_45`] — the turn-45 choices are
/// SWAPPED (`:632-635`), and the polyline splices at the last corner
/// (the trace's last segment line + end cap are replaced).
pub(crate) fn smoothen_end_corner_at_trace_45(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: crate::id::ItemId,
) -> Option<Polyline> {
    let mut acute_angle = false;
    let mut bend = false;
    let mut other_trace_corner_approx: Option<epic_geometry::float_point::FloatPoint> = None;
    let mut other_trace_line: Option<Line> = None;
    let mut other_prev_trace_line: Option<Line> = None;
    let trace_polyline = board
        .trace_polyline(trace_id)
        .cloned()
        .expect("live trace polyline");
    let len = trace_polyline.lines.len();
    // Java `:575` — exact LAST corner.
    let current_end_corner = trace_polyline.last_corner().expect("last corner");
    // Java `:577-579`.
    if let Some(clip) = &state.current_clip_shape
        && !clip.contains(&current_end_corner.to_float())
    {
        return None;
    }
    // Java `:581-586`.
    let current_prev_end_corner = trace_polyline
        .corner(trace_polyline.corner_count() as i32 - 2)
        .expect("second-to-last corner");
    let mut prev_corner_side: Option<Side> = None;
    let line_direction = trace_polyline.lines[len - 2].direction().opposite();
    let prev_line_direction = trace_polyline.lines[len - 3].direction().opposite();
    // Java `:588`.
    let contact_list = crate::contacts::end_contacts(manager, board, trace_id);
    for contact in contact_list {
        // Java `:590`.
        let is_candidate = matches!(
            board.get(contact).map(|entry| &entry.data),
            Some(ItemData::Trace { .. })
        ) && !is_shove_fixed(board, contact);
        if !is_candidate {
            return None;
        }
        let contact_polyline = board
            .trace_polyline(contact)
            .cloned()
            .expect("live contact trace");
        let (current_other_corner_approx, current_other_line, current_other_prev_line) =
            if contact_polyline.first_corner().as_ref() == Some(&current_end_corner) {
                (
                    contact_polyline.corner_approx(1),
                    contact_polyline.lines[1].clone(),
                    contact_polyline.lines[2].clone(),
                )
            } else {
                let current_corner_no = contact_polyline.corner_count() as i32 - 2;
                (
                    contact_polyline.corner_approx(current_corner_no),
                    contact_polyline.lines[(current_corner_no + 1) as usize].opposite(),
                    contact_polyline.lines[current_corner_no as usize].clone(),
                )
            };
        let current_prev_corner_side = current_prev_end_corner.side_of_line(&current_other_line);
        let current_projection = line_direction.projection(current_other_line.direction());
        let mut other_trace_found = false;
        if current_projection == Side::Positive && current_prev_corner_side != Side::Collinear {
            if current_other_line.direction().is_orthogonal() {
                acute_angle = true;
                other_trace_found = true;
            }
        } else if current_projection == Side::Collinear
            && trace_polyline.corner_count() > 2
            && prev_line_direction.projection(current_other_line.direction()) == Side::Positive
        {
            bend = true;
            other_trace_found = true;
        }
        if other_trace_found {
            other_trace_corner_approx = Some(current_other_corner_approx);
            other_trace_line = Some(current_other_line);
            prev_corner_side = Some(current_prev_corner_side);
            other_prev_trace_line = Some(current_other_prev_line);
        }
    }
    if acute_angle {
        // Java `:630-657` — the turn-45 choices SWAPPED vs the start.
        let other_line = other_trace_line.as_ref().expect("acute arm has a contact");
        let new_line_dir = if prev_corner_side == Some(Side::Positive) {
            other_line.direction().turn_45_degree(6)
        } else {
            other_line.direction().turn_45_degree(2)
        };
        let translate_line = Line::get_instance(
            Point::Int(current_end_corner.to_float().round()),
            new_line_dir,
        );
        let mut translate_dist = (SQRT2 - 1.0) * f64::from(state.current_half_width);
        let prev_corner_dist = translate_line
            .signed_distance(&current_prev_end_corner.to_float())
            .abs();
        let other_dist = translate_line
            .signed_distance(&other_trace_corner_approx.expect("acute arm corner approx"))
            .abs();
        translate_dist = translate_dist.min(prev_corner_dist);
        translate_dist = translate_dist.min(other_dist);
        if translate_dist >= 0.99 {
            translate_dist = (translate_dist - 1.0).max(1.0);
            if translate_line.side_of(&current_prev_end_corner) == Side::Positive {
                translate_dist = -translate_dist;
            }
            let add_line = translate_line.translate(translate_dist);
            // construct the new trace polyline — the trace's last
            // SEGMENT line and end cap are replaced by [addLine,
            // otherTraceLine] (Java `:652-656` arraycopy keeps
            // lines[0..len-1]).
            let mut new_lines: Vec<Line> = Vec::with_capacity(len + 1);
            new_lines.extend_from_slice(&trace_polyline.lines[..len - 1]);
            new_lines.push(add_line);
            new_lines.push(other_line.clone());
            return Some(Polyline::new(new_lines));
        }
    } else if bend {
        // Java `:658-671`.
        let other_line = other_trace_line.as_ref().expect("bend arm has a contact");
        let other_prev_line = other_prev_trace_line
            .as_ref()
            .expect("bend arm has a prev contact line");
        let mut check_line_arr: Vec<Line> = Vec::with_capacity(len + 1);
        check_line_arr.extend_from_slice(&trace_polyline.lines[..len - 1]);
        check_line_arr.push(other_line.clone());
        check_line_arr.push(other_prev_line.clone());
        if let Some(new_line) =
            state.reposition_line(manager, board, &check_line_arr, len as i32 - 2)
        {
            let mut new_lines: Vec<Line> = Vec::with_capacity(len);
            new_lines.extend_from_slice(&trace_polyline.lines[..len - 2]);
            new_lines.push(new_line);
            new_lines.push(other_line.clone());
            return Some(Polyline::new(new_lines));
        }
    }
    None
}
