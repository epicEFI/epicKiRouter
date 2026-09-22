//! Java `board/optimize/TraceTightener90.java` — the 90° pull-tight
//! variant (`pullTight` loop `:26-36`, `trySkipSecondCorner` `:39-70`,
//! `trySkipCorners` `:73-158`). The smoothen-corner overrides
//! (`:160-168`) answer null and live on the base struct as the
//! always-None bodies ([`super::TraceTightener::
//! smoothen_start_corner_at_trace`] /
//! [`super::TraceTightener::smoothen_end_corner_at_trace`]) — the
//! whole smoothen arm is inert on 90° boards.
//!
//! The `pullTight` loop condition `newResult != prevResult` is a Java
//! REFERENCE comparison; the port compares values. Every no-change
//! stage returns its input object, so reference inequality with VALUE
//! equality would mean the stage rebuilt the same geometry — the
//! next loop would rebuild it again forever under Java semantics, so
//! stopping at value equality is the terminating face of the same
//! verdict (documented proxy, module docs of [`super`]).

use epic_geometry::line::Line;
use epic_geometry::polyline::Polyline;

use crate::board::Board;
use crate::forced_pad_router::check_trace_shape;
use crate::tree_manager::SearchTreeManager;

use super::TraceTightener;

/// Java `TraceTightener90.pullTight` (`:26-36`): acid-trap identity,
/// then the fixed point skip-corner → skip-corners → reposition until
/// nothing changes (or the stop face fires).
pub(crate) fn pull_tight_90(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: Polyline,
) -> Polyline {
    let mut new_result = state.avoid_acid_traps(manager, board, polyline);
    let mut prev_result: Option<Polyline> = None;
    // Java `while (newResult != prevResult && !this.isStopRequested())`
    // — null prevResult makes the first iteration unconditional.
    while prev_result.as_ref() != Some(&new_result) && !state.is_stop_requested() {
        prev_result = Some(new_result.clone());
        let prev = prev_result.as_ref().expect("just set");
        let tmp1 = try_skip_second_corner(state, manager, board, prev);
        let tmp2 = try_skip_corners(state, manager, board, &tmp1);
        new_result = state.reposition_lines(manager, board, &tmp2);
    }
    new_result
}

/// Java `TraceTightener90.trySkipSecondCorner` (`:39-70`): try to
/// skip the second corner by swapping the first two lines and
/// dropping the third. The check polyline `[lines[1], lines[0],
/// lines[3], lines[4]]` turns the corner-1 geometry inside out; both
/// offset shapes 0 and 1 must clear (Java's loop bound `i < 2`).
fn try_skip_second_corner(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
) -> Polyline {
    let line_arr = &polyline.lines;
    if line_arr.len() < 5 {
        return polyline.clone();
    }
    // Java `:48-53`: the FIRST TWO lines are swapped — the new trace
    // starts with the old second line extended backward.
    let check_lines = vec![
        line_arr[1].clone(),
        line_arr[0].clone(),
        line_arr[3].clone(),
        line_arr[4].clone(),
    ];
    let check_polyline = Polyline::new(check_lines);
    // Java `:57-61` precedence: `(len != 4) || (clip != null && !contains)`.
    if check_polyline.lines.len() != 4
        || state
            .current_clip_shape
            .as_ref()
            .is_some_and(|clip| !clip.contains(&check_polyline.corner_approx(1)))
    {
        return polyline.clone();
    }
    for i in 0..2 {
        // Java's offsetShape null would be an unreachable NPE (the
        // 4-line guard keeps every index in range).
        let curr_shape = check_polyline
            .offset_shape(state.current_half_width, i)
            .expect("4-line polyline has offset shape in range");
        if !check_trace_shape(
            manager,
            board,
            &curr_shape,
            state.current_layer,
            &state.current_net_numbers,
            state.current_clearance_class_index,
            state.contact_pins.as_ref(),
        ) {
            return polyline.clone();
        }
    }
    // Java `:66-70`: drop line 2 — `[lines[1], lines[0], lines[3..]]`.
    let mut new_lines = Vec::with_capacity(line_arr.len() - 1);
    new_lines.push(line_arr[1].clone());
    new_lines.push(line_arr[0].clone());
    new_lines.extend_from_slice(&line_arr[3..]);
    Polyline::new(new_lines)
}

/// Java `TraceTightener90.trySkipCorners` (`:73-158`): walk the
/// polyline and skip every corner whose direct connection clears.
/// `newLines[newLineIndex]` is the last KEPT line; a successful skip
/// advances `i` twice (Java's `++i` inside the branch plus the loop
/// increment) and does NOT advance `newLineIndex`, so the next
/// candidate jumps from the same kept line.
fn try_skip_corners(
    state: &mut TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
) -> Polyline {
    let line_arr = &polyline.lines;
    let len = line_arr.len() as i32;
    // Java allocates `new Line[line_arr.length]`; the arithmetic never
    // writes past `len - 1` (init 2 + at most one non-skip per i + a
    // 2-or-3-line tail).
    let mut new_lines: Vec<Option<Line>> = Vec::with_capacity(line_arr.len());
    new_lines.push(Some(line_arr[0].clone()));
    new_lines.push(Some(line_arr[1].clone()));
    new_lines.resize(line_arr.len(), None);
    let mut new_line_index: usize = 1;
    let mut polyline_changed = false;
    let mut second_last_corner_skipped = false;
    let mut check_lines: [Line; 4] = [
        line_arr[0].clone(),
        line_arr[1].clone(),
        line_arr[1].clone(),
        line_arr[1].clone(),
    ];
    let mut i: i32 = 5;
    while i <= len {
        let mut skip_lines = false;
        // Java `:82`: `inClipShape = clip == null || clip.contains(
        // cornerApprox(i - 3))`.
        let in_clip_shape = state
            .current_clip_shape
            .as_ref()
            .is_none_or(|clip| clip.contains(&polyline.corner_approx(i - 3)));
        if in_clip_shape {
            check_lines[0] = new_lines[new_line_index - 1]
                .clone()
                .expect("written by init or a previous non-skip");
            check_lines[1] = new_lines[new_line_index]
                .clone()
                .expect("written by init or a previous non-skip");
            check_lines[2] = line_arr[(i - 1) as usize].clone();
            check_lines[3] = if i < len {
                line_arr[i as usize].clone()
            } else {
                line_arr[(i - 2) as usize].clone()
            };
            let check_polyline = Polyline::new(check_lines.to_vec());
            // Java `TraceTightener90.trySkipCorners`, the `skipLines`
            // length/clip test (`:96-99`; `:91-92` is the concluding-line
            // else-arm).
            skip_lines = check_polyline.lines.len() == 4
                && state
                    .current_clip_shape
                    .as_ref()
                    .is_none_or(|clip| clip.contains(&check_polyline.corner_approx(1)));
            if skip_lines {
                let shape = check_polyline
                    .offset_shape(state.current_half_width, 0)
                    .expect("4-line polyline has offset shape 0");
                skip_lines = check_trace_shape(
                    manager,
                    board,
                    &shape,
                    state.current_layer,
                    &state.current_net_numbers,
                    state.current_clearance_class_index,
                    state.contact_pins.as_ref(),
                );
            }
            if skip_lines {
                let shape = check_polyline
                    .offset_shape(state.current_half_width, 1)
                    .expect("4-line polyline has offset shape 1");
                skip_lines = check_trace_shape(
                    manager,
                    board,
                    &shape,
                    state.current_layer,
                    &state.current_net_numbers,
                    state.current_clearance_class_index,
                    state.contact_pins.as_ref(),
                );
            }
        }
        if skip_lines {
            if i == len {
                second_last_corner_skipped = true;
            }
            if board.changed_area.is_some() {
                // Java `:99-106`: mark the changed area at the new
                // corner and at the skipped corner.
                let new_corner = check_lines[1].intersection_approx(&check_lines[2]);
                crate::routing_board_insert::join_changed_area(
                    board,
                    &new_corner,
                    state.current_layer,
                );
                let skipped_corner =
                    line_arr[(i - 2) as usize].intersection_approx(&line_arr[(i - 3) as usize]);
                crate::routing_board_insert::join_changed_area(
                    board,
                    &skipped_corner,
                    state.current_layer,
                );
            }
            polyline_changed = true;
            // Java `++i` inside the branch — the next candidate starts
            // two positions ahead.
            i += 1;
        } else {
            new_line_index += 1;
            new_lines[new_line_index] = Some(line_arr[(i - 3) as usize].clone());
        }
        i += 1;
    }
    if !polyline_changed {
        return polyline.clone();
    }
    // Java `:114-126`: the tail. If the second-last corner was
    // skipped, the last two lines close the polyline; otherwise all
    // last three lines append.
    if second_last_corner_skipped {
        new_line_index += 1;
        new_lines[new_line_index] = Some(line_arr[(len - 1) as usize].clone());
        new_line_index += 1;
        new_lines[new_line_index] = Some(line_arr[(len - 2) as usize].clone());
    } else {
        let mut k = 3;
        while k > 0 {
            new_line_index += 1;
            new_lines[new_line_index] = Some(line_arr[(len - k) as usize].clone());
            k -= 1;
        }
    }
    // Java `:128-131`: copy `[0..=newLineIndex]` into the result —
    // the constructor's parallel-line filter collapses a duplicate
    // produced by skip-then-tail adjacency.
    let cleaned: Vec<Line> = new_lines
        .into_iter()
        .take(new_line_index + 1)
        .flatten()
        .collect();
    Polyline::new(cleaned)
}
